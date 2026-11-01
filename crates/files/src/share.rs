//! Read-only shares use distinct services and grants from incoming Files.
use crate::{
    ErrorCode, FileInfo, MAX_CHUNK, MAX_METADATA, Request, Response, ShareEntry, encode_hash,
    read_request, write_response,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    os::fd::{AsRawFd, FromRawFd, IntoRawFd},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Semaphore,
};

const PAGE_ENTRIES: usize = 32;
const MAX_DIRECTORY_ENTRIES: usize = 100_000;
type Result<T> = std::result::Result<T, ErrorCode>;
fn io_code(error: io::Error) -> ErrorCode {
    match error.raw_os_error() {
        Some(libc::ENOENT) => ErrorCode::NotFound,
        Some(libc::ELOOP | libc::ENOTDIR | libc::EACCES | libc::EPERM) => ErrorCode::PathDenied,
        _ => ErrorCode::Unavailable,
    }
}
fn parts(path: &str) -> Result<Vec<&str>> {
    if path.len() > 4096 || path.contains(['\0', '\\']) {
        return Err(ErrorCode::PathDenied);
    }
    if path.is_empty() {
        return Ok(vec![]);
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.iter().any(|part| {
        part.is_empty()
            || matches!(*part, "." | "..")
            || part.len() > 255
            || part.starts_with(".wispnet-")
    }) {
        return Err(ErrorCode::PathDenied);
    }
    Ok(parts)
}
fn open_at(directory: &File, name: &std::ffi::OsStr, is_directory: bool) -> Result<File> {
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(name.as_bytes()).map_err(|_| ErrorCode::PathDenied)?;
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK
                | if is_directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    if fd < 0 {
        return Err(io_code(io::Error::last_os_error()));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(io_code)?;
    if !meta.is_dir() && !meta.is_file() {
        return Err(ErrorCode::PathDenied);
    }
    Ok(file)
}
fn open(root: &Path, path: &str) -> Result<File> {
    let components = parts(path)?;
    let mut file = crate::server::open_directory(root).map_err(io_code)?;
    for (index, name) in components.iter().enumerate() {
        file = open_at(
            &file,
            std::ffi::OsStr::new(name),
            index + 1 < components.len(),
        )?;
    }
    Ok(file)
}
fn entry(path: &str, name: &str, file: &File) -> Result<ShareEntry> {
    let meta = file.metadata().map_err(io_code)?;
    Ok(ShareEntry {
        name: name.into(),
        path: path.into(),
        is_dir: meta.is_dir(),
        size_bytes: if meta.is_file() { meta.len() } else { 0 },
    })
}
struct Directory(*mut libc::DIR);
impl Drop for Directory {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}
fn list(root: &Path, path: &str, cursor: Option<String>, cancel: &AtomicBool) -> Result<Response> {
    if cursor
        .as_ref()
        .is_some_and(|cursor| parts(cursor).is_err() || cursor.contains('/') || cursor.is_empty())
    {
        return Err(ErrorCode::InvalidArgument);
    }
    let directory = open(root, path)?;
    if !directory.metadata().map_err(io_code)?.is_dir() {
        return Err(ErrorCode::PathDenied);
    }
    // fdopendir owns a separate descriptor. No path is re-resolved for listing.
    let duplicate = directory.try_clone().map_err(io_code)?.into_raw_fd();
    let raw = unsafe { libc::fdopendir(duplicate) };
    if raw.is_null() {
        unsafe {
            libc::close(duplicate);
        }
        return Err(io_code(io::Error::last_os_error()));
    }
    let handle = Directory(raw);
    let mut candidates = BTreeMap::new();
    let mut count = 0;
    let mut more = false;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(ErrorCode::Unavailable);
        }
        let raw_entry = unsafe { libc::readdir(handle.0) };
        if raw_entry.is_null() {
            break;
        }
        count += 1;
        if count > MAX_DIRECTORY_ENTRIES {
            return Err(ErrorCode::ResourceLimit);
        }
        let name = unsafe { CStr::from_ptr((*raw_entry).d_name.as_ptr()) };
        let Ok(name) = name.to_str() else {
            continue;
        };
        if parts(name).is_err()
            || name.is_empty()
            || cursor.as_deref().is_some_and(|cursor| name <= cursor)
        {
            continue;
        }
        let child_path = if path.is_empty() {
            name.into()
        } else {
            format!("{path}/{name}")
        };
        if parts(&child_path).is_err() {
            continue;
        }
        let child = match open_at(&directory, std::ffi::OsStr::new(name), false) {
            Ok(file) => file,
            Err(ErrorCode::PathDenied | ErrorCode::NotFound) => continue,
            Err(error) => return Err(error),
        };
        candidates.insert(name.to_owned(), entry(&child_path, name, &child)?);
        if candidates.len() > PAGE_ENTRIES {
            candidates.pop_last();
            more = true;
        }
    }
    let mut entries: Vec<_> = candidates.into_values().collect();
    // Long relative paths still fit the fixed metadata frame ceiling.
    loop {
        let next_cursor = if more {
            entries.last().map(|entry| entry.name.clone())
        } else {
            None
        };
        let response = Response::Entries {
            entries: entries.clone(),
            next_cursor,
        };
        if serde_json::to_vec(&response)
            .map_err(|_| ErrorCode::Unavailable)?
            .len()
            <= MAX_METADATA
        {
            return Ok(response);
        }
        entries.pop();
        more = true;
        if entries.is_empty() {
            return Err(ErrorCode::ResourceLimit);
        }
    }
}
fn stat(root: &Path, path: &str, cancel: &AtomicBool) -> Result<Response> {
    let mut file = open(root, path)?;
    let meta = file.metadata().map_err(io_code)?;
    if !meta.is_file() {
        return Err(ErrorCode::PathDenied);
    }
    let before = crate::server::fingerprint(&meta);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut remaining = meta.len();
    while remaining > 0 {
        if cancel.load(Ordering::Acquire) {
            return Err(ErrorCode::Unavailable);
        }
        let length = remaining.min(buffer.len() as u64) as usize;
        let count = file.read(&mut buffer[..length]).map_err(io_code)?;
        if count == 0 {
            return Err(ErrorCode::StaleVersion);
        }
        remaining -= count as u64;
        hasher.update(&buffer[..count]);
    }
    if crate::server::fingerprint(&file.metadata().map_err(io_code)?) != before {
        return Err(ErrorCode::StaleVersion);
    }
    let hash = encode_hash(hasher.finalize());
    Ok(Response::File {
        file: FileInfo {
            path: path.into(),
            size_bytes: meta.len(),
            file_version: format!("{hash}:{before}"),
            sha256: hash,
        },
    })
}
fn read(
    root: &Path,
    path: &str,
    expected_version: &str,
    offset: u64,
    max_bytes: u64,
    cancel: &AtomicBool,
) -> Result<Response> {
    if max_bytes == 0 || max_bytes > MAX_CHUNK as u64 {
        return Err(ErrorCode::InvalidArgument);
    }
    let Some((hash, expected)) = expected_version.split_once(':') else {
        return Err(ErrorCode::StaleVersion);
    };
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(ErrorCode::StaleVersion);
    }
    let mut file = open(root, path)?;
    let meta = file.metadata().map_err(io_code)?;
    if !meta.is_file() {
        return Err(ErrorCode::PathDenied);
    }
    if crate::server::fingerprint(&meta) != expected {
        return Err(ErrorCode::StaleVersion);
    }
    if offset > meta.len() {
        return Err(ErrorCode::InvalidOffset);
    }
    if cancel.load(Ordering::Acquire) {
        return Err(ErrorCode::Unavailable);
    }
    file.seek(SeekFrom::Start(offset)).map_err(io_code)?;
    let mut data = vec![0; (meta.len() - offset).min(max_bytes) as usize];
    file.read_exact(&mut data).map_err(io_code)?;
    if crate::server::fingerprint(&file.metadata().map_err(io_code)?) != expected {
        return Err(ErrorCode::StaleVersion);
    }
    Ok(Response::Data { offset, data })
}
struct Cancel(Arc<AtomicBool>);
impl Drop for Cancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) async fn serve(
    root: PathBuf,
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    bulk: Arc<Semaphore>,
) -> io::Result<()> {
    let cancel = Arc::new(AtomicBool::new(false));
    let _guard = Cancel(cancel.clone());
    loop {
        let Some(request) =
            tokio::time::timeout(Duration::from_secs(300), read_request(&mut reader))
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "File share idle timeout")
                })??
        else {
            return Ok(());
        };
        let _permit = bulk
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| io::Error::other("File share unavailable"))?;
        let root = root.clone();
        let cancel = cancel.clone();
        let response = tokio::task::spawn_blocking(move || {
            let _permit = _permit;
            match request {
                Request::ShareList { path, cursor } => list(&root, &path, cursor, &cancel),
                Request::ShareStat { path } => stat(&root, &path, &cancel),
                Request::ShareReadRange {
                    path,
                    expected_version,
                    offset,
                    max_bytes,
                } => read(&root, &path, &expected_version, offset, max_bytes, &cancel),
                _ => Err(ErrorCode::PermissionDenied),
            }
        })
        .await
        .map_err(|_| io::Error::other("File share unavailable"))?;
        let response = response.unwrap_or_else(|code| Response::Error {
            code,
            message: "File share request failed".into(),
            expected_offset: None,
        });
        tokio::time::timeout(
            Duration::from_secs(300),
            write_response(&mut writer, &response),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "File share write timeout"))??;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileService, read_response, sha256, write_request};
    use std::fs;

    fn info(root: &Path, path: &str) -> FileInfo {
        match stat(root, path, &AtomicBool::new(false)).unwrap() {
            Response::File { file } => file,
            _ => panic!("file response required"),
        }
    }
    #[test]
    fn share_ranges_pin_versions_and_reject_source_replacement() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("step-1.pt"), b"checkpoint bytes").unwrap();
        let file = info(root.path(), "step-1.pt");
        assert_eq!(file.sha256, sha256(b"checkpoint bytes"));
        assert!(
            matches!(read(root.path(), "step-1.pt", &file.file_version, 3, 4, &AtomicBool::new(false)).unwrap(), Response::Data { offset: 3, data } if data == b"ckpo")
        );
        fs::write(root.path().join("new"), b"checkpoint bytes").unwrap();
        fs::rename(root.path().join("new"), root.path().join("step-1.pt")).unwrap();
        assert_eq!(
            read(
                root.path(),
                "step-1.pt",
                &file.file_version,
                0,
                4,
                &AtomicBool::new(false)
            )
            .unwrap_err(),
            ErrorCode::StaleVersion
        );
        let file = info(root.path(), "step-1.pt");
        fs::write(root.path().join("step-1.pt"), b"changed in place").unwrap();
        assert_eq!(
            read(
                root.path(),
                "step-1.pt",
                &file.file_version,
                0,
                4,
                &AtomicBool::new(false)
            )
            .unwrap_err(),
            ErrorCode::StaleVersion
        );
    }
    #[test]
    fn share_paths_and_listings_cannot_escape_or_expose_partial_files() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"private").unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        symlink(outside.path().join("secret"), root.path().join("linked")).unwrap();
        fs::write(root.path().join(".wispnet-partial"), b"partial").unwrap();
        fs::write(root.path().join("safe"), b"safe").unwrap();
        let fifo = CString::new(root.path().join("fifo").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        for path in [
            "/secret",
            "../secret",
            "escape/secret",
            "linked",
            "fifo",
            ".wispnet-partial",
            "a//b",
            "a\\b",
        ] {
            assert!(
                stat(root.path(), path, &AtomicBool::new(false)).is_err(),
                "{path}"
            );
        }
        assert!(
            matches!(list(root.path(), "", None, &AtomicBool::new(false)).unwrap(), Response::Entries { entries, next_cursor: None } if entries.len() == 1 && entries[0].name == "safe")
        );
        let alias = outside.path().join("alias");
        symlink(root.path(), &alias).unwrap();
        assert!(stat(&alias, "safe", &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn configured_folder_replaced_by_file_or_symlink_cannot_be_read() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("selected");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("model"), b"weights").unwrap();
        assert_eq!(info(&root, "model").size_bytes, 7);
        fs::remove_dir_all(&root).unwrap();
        fs::write(&root, b"unexpected file").unwrap();
        assert_eq!(
            list(&root, "", None, &AtomicBool::new(false)).unwrap_err(),
            ErrorCode::PathDenied
        );
        assert_eq!(
            stat(&root, "", &AtomicBool::new(false)).unwrap_err(),
            ErrorCode::PathDenied
        );
        fs::remove_file(&root).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"private").unwrap();
        std::os::unix::fs::symlink(outside.path(), &root).unwrap();
        assert_eq!(
            list(&root, "", None, &AtomicBool::new(false)).unwrap_err(),
            ErrorCode::PathDenied
        );
        assert_eq!(
            stat(&root, "secret", &AtomicBool::new(false)).unwrap_err(),
            ErrorCode::PathDenied
        );
    }
    #[test]
    fn listings_are_bounded_paginated_and_complete() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..79 {
            fs::write(root.path().join(format!("step-{index:03}")), b"weights").unwrap();
        }
        let mut cursor = None;
        let mut names = vec![];
        loop {
            let response = list(root.path(), "", cursor, &AtomicBool::new(false)).unwrap();
            assert!(serde_json::to_vec(&response).unwrap().len() <= MAX_METADATA);
            let Response::Entries {
                entries,
                next_cursor,
            } = response
            else {
                panic!("entries expected");
            };
            assert!(entries.len() <= PAGE_ENTRIES);
            names.extend(entries.into_iter().map(|entry| entry.name));
            cursor = next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(names.len(), 79);
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
    }
    #[tokio::test]
    async fn binary_share_stream_reads_but_cannot_upload() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        fs::write(root.path().join("model"), b"\0\xffweights").unwrap();
        let service = FileService::new(state.path().into()).unwrap();
        let (mut client, server) = tokio::io::duplex(MAX_CHUNK * 2);
        let (reader, writer) = tokio::io::split(server);
        let path = root.path().to_owned();
        let task = tokio::spawn(async move {
            service
                .serve_share("peer", "incarnation", path, reader, writer)
                .await
                .unwrap();
        });
        write_request(
            &mut client,
            &Request::ShareStat {
                path: "model".into(),
            },
        )
        .await
        .unwrap();
        let Some(Response::File { file }) = read_response(&mut client).await.unwrap() else {
            panic!("file expected");
        };
        write_request(
            &mut client,
            &Request::ShareReadRange {
                path: "model".into(),
                expected_version: file.file_version,
                offset: 0,
                max_bytes: MAX_CHUNK as u64,
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(read_response(&mut client).await.unwrap(), Some(Response::Data { offset: 0, data }) if data == b"\0\xffweights")
        );
        write_request(
            &mut client,
            &Request::UploadBegin {
                workspace: "".into(),
                path: "injected".into(),
                size_bytes: 3,
                sha256: sha256(b"bad"),
                idempotency_key: "upload".into(),
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            read_response(&mut client).await.unwrap(),
            Some(Response::Error {
                code: ErrorCode::PermissionDenied,
                ..
            })
        ));
        drop(client);
        task.await.unwrap();
        assert!(!root.path().join("injected").exists());
    }
}
