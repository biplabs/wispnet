//! Native binary Files helpers. Only job metadata crosses the owner-local API.
//! Resume journals are private, durable, and bound to both endpoints and content.
use crate::Client;
use iroh::endpoint::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite},
    sync::{Semaphore, mpsc, watch},
    task::JoinHandle,
};
use wispnet_core::identity::EndpointId;
use wispnet_files::{
    FileInfo, FileService, MAX_CHUNK, Request, Response, TransferState, read_response,
    write_request,
};

const PIPELINE: usize = 8;
const MAX_RETRIES: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTarget {
    pub host: EndpointId,
    pub service: String,
    pub incarnation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferProgress {
    pub phase: String,
    pub verified_bytes: u64,
    pub size_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_offset: Option<u64>,
}
impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for TransferError {}
impl TransferError {
    fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            expected_offset: None,
        }
    }
    fn io(error: io::Error) -> Self {
        let code = match error.raw_os_error() {
            Some(libc::ENOSPC) | Some(libc::EDQUOT) => "NO_SPACE",
            _ => match error.kind() {
                io::ErrorKind::NotFound => "NOT_FOUND",
                io::ErrorKind::AlreadyExists => "CONFLICT",
                io::ErrorKind::PermissionDenied => "PERMISSION_DENIED",
                _ => "UNAVAILABLE",
            },
        };
        Self::new(code, "File transfer I/O failed")
    }
    fn response(response: Response) -> Result<Response> {
        if let Response::Error {
            code,
            message,
            expected_offset,
        } = response
        {
            let code = serde_json::to_value(code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "UNAVAILABLE".into());
            Err(Self {
                code,
                message,
                expected_offset,
            })
        } else {
            Ok(response)
        }
    }
}
type Result<T> = std::result::Result<T, TransferError>;
pub type ProgressCallback = Arc<dyn Fn(TransferProgress) + Send + Sync>;

#[derive(Clone)]
pub struct FileCancellation(watch::Sender<bool>);
impl Default for FileCancellation {
    fn default() -> Self {
        Self(watch::channel(false).0)
    }
}
impl FileCancellation {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        loop {
            if *receiver.borrow_and_update() {
                return;
            }
            if receiver.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}
struct CancelOnDrop {
    cancellation: FileCancellation,
    armed: bool,
}
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if self.armed {
            self.cancellation.cancel();
        }
    }
}

#[derive(Clone)]
pub struct TransferOptions {
    pub resume: bool,
    pub progress: Option<ProgressCallback>,
    pub cancellation: FileCancellation,
}
impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            resume: true,
            progress: None,
            cancellation: FileCancellation::default(),
        }
    }
}
impl TransferOptions {
    fn report(&self, phase: &str, verified_bytes: u64, size_bytes: u64) {
        if let Some(progress) = &self.progress {
            progress(TransferProgress {
                phase: phase.into(),
                verified_bytes,
                size_bytes,
            });
        }
    }
}

struct Job {
    host: EndpointId,
    cancellation: FileCancellation,
    status: Mutex<Value>,
    finished: Mutex<Option<Instant>>,
}
pub(crate) struct Files {
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
    slots: Arc<Semaphore>,
    local: OnceLock<FileService>,
}
impl Default for Files {
    fn default() -> Self {
        Self {
            jobs: Mutex::new(BTreeMap::new()),
            slots: Arc::new(Semaphore::new(2)),
            local: OnceLock::new(),
        }
    }
}
impl Files {
    pub(crate) fn cancel_host(&self, host: Option<EndpointId>) {
        for job in self.jobs.lock().unwrap_or_else(|e| e.into_inner()).values() {
            if host.is_none_or(|host| job.host == host) {
                job.cancellation.cancel();
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Binding {
    version: u8,
    own: String,
    target: FileTarget,
    workspace: String,
    remote_path: String,
    local_path: PathBuf,
    direction: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
// Older upload journals include unused download fields; accept those when resuming.
struct Journal {
    binding: Binding,
    size_bytes: u64,
    sha256: String,
    idempotency_key: String,
    transfer_id: Option<String>,
    committed_offset: u64,
}
struct ResumeFiles {
    journal: PathBuf,
    _lock: Arc<File>,
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
type BoxStream = Box<dyn Stream>;
#[derive(Default)]
struct Resources {
    connection: Option<Connection>,
    local: Option<JoinHandle<()>>,
}
impl Drop for Resources {
    fn drop(&mut self) {
        if let Some(task) = &self.local {
            task.abort();
        }
        if let Some(connection) = &self.connection {
            connection.close(0u32.into(), b"file transfer stream closed");
        }
    }
}

impl Client {
    /// Start a resumable upload. Dropping the future or cancelling preserves the
    /// acknowledged remote prefix; the same call can be repeated after restart.
    pub async fn put_file(
        self: &Arc<Self>,
        target: FileTarget,
        workspace: &str,
        local_path: &Path,
        remote_path: &str,
        options: TransferOptions,
    ) -> Result<FileInfo> {
        let mut drop_guard = CancelOnDrop {
            cancellation: options.cancellation.clone(),
            armed: true,
        };
        let result = tokio::select! {
            biased;
            _ = options.cancellation.cancelled() => Err(TransferError::new("CANCELLED", "Transfer cancelled; resumable data retained")),
            result = self.put_file_inner(target, workspace, local_path, remote_path, &options) => result,
        };
        drop_guard.armed = false;
        result
    }
    pub(crate) async fn files_command(
        self: &Arc<Self>,
        value: &Value,
    ) -> std::result::Result<Value, String> {
        match value["op"].as_str().unwrap_or("") {
            "file_status" | "file_cancel" => {
                let id = crate::argument(value, "transfer")?;
                let job = self
                    .files
                    .jobs
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(id)
                    .cloned()
                    .ok_or("File transfer handle unavailable; repeat put to resume")?;
                if value["op"] == "file_cancel" {
                    job.cancellation.cancel();
                }
                let status = job.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
                Ok(status)
            }
            "file_put" => {
                let target = FileTarget {
                    host: crate::peer(value)?,
                    service: value["service"].as_str().unwrap_or("files").into(),
                    incarnation: crate::argument(value, "incarnation")?.into(),
                };
                validate_target(&target).map_err(|e| e.to_string())?;
                let workspace = crate::argument(value, "workspace")?.to_owned();
                let remote_path = crate::argument(value, "remote_path")?.to_owned();
                let local_path = PathBuf::from(crate::argument(value, "local_path")?);
                if !local_path.is_absolute() {
                    return Err("Local file path must be absolute".into());
                }
                let resume = value
                    .get("resume")
                    .map(|v| v.as_bool().ok_or("resume must be a boolean"))
                    .transpose()?
                    .unwrap_or(true);
                let permit = self
                    .files
                    .slots
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| "RESOURCE_LIMIT: two file transfers are already active")?;
                let cancellation = FileCancellation::default();
                let id = crate::token();
                let job = Arc::new(Job {
                    host: target.host,
                    cancellation: cancellation.clone(),
                    status: Mutex::new(
                        json!({"transfer":id,"state":"running","phase":"preparing","verified_bytes":0,"size_bytes":0}),
                    ),
                    finished: Mutex::new(None),
                });
                {
                    let mut jobs = self.files.jobs.lock().unwrap_or_else(|e| e.into_inner());
                    jobs.retain(|_, job| {
                        job.finished
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .is_none_or(|at| at.elapsed() < Duration::from_secs(3600))
                    });
                    if jobs.len() >= 64 {
                        let oldest = jobs
                            .iter()
                            .filter_map(|(id, job)| {
                                job.finished
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .map(|at| (id.clone(), at))
                            })
                            .min_by_key(|(_, at)| *at)
                            .map(|(id, _)| id);
                        if let Some(id) = oldest {
                            jobs.remove(&id);
                        } else {
                            return Err("RESOURCE_LIMIT: file transfer history is full".into());
                        }
                    }
                    jobs.insert(id, job.clone());
                }
                let job_progress = job.clone();
                let options = TransferOptions {
                    resume,
                    cancellation,
                    progress: Some(Arc::new(move |progress| {
                        let mut status = job_progress
                            .status
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        status["phase"] = json!(progress.phase);
                        status["verified_bytes"] = json!(progress.verified_bytes);
                        status["size_bytes"] = json!(progress.size_bytes);
                    })),
                };
                let client = self.clone();
                let started = Instant::now();
                let operation_job = job.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let result = client
                        .put_file(target, &workspace, &local_path, &remote_path, options)
                        .await;
                    let mut status = operation_job
                        .status
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    status["duration_ms"] = json!(started.elapsed().as_millis() as u64);
                    match result {
                        Ok(file) => {
                            status["state"] = json!("complete");
                            status["phase"] = json!("complete");
                            status["verified_bytes"] = json!(file.size_bytes);
                            status["size_bytes"] = json!(file.size_bytes);
                            status["file"] = json!(file);
                        }
                        Err(error) => {
                            status["state"] = json!(if error.code == "CANCELLED" {
                                "cancelled"
                            } else {
                                "failed"
                            });
                            status["error"] = json!(error);
                        }
                    }
                    *operation_job
                        .finished
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                });
                Ok(job.status.lock().unwrap_or_else(|e| e.into_inner()).clone())
            }
            _ => Err("Unsupported file operation".into()),
        }
    }
    async fn open_file_stream(
        self: &Arc<Self>,
        target: &FileTarget,
        resources: &mut Resources,
    ) -> Result<BoxStream> {
        validate_target(target)?;
        if target.host == self.endpoint.id() {
            let authority = self
                .local_authority
                .as_ref()
                .ok_or_else(|| TransferError::new("UNAVAILABLE", "Local host unavailable"))?;
            let service = authority
                .services()
                .await
                .map_err(|_| TransferError::new("UNAVAILABLE", "Local service unavailable"))?
                .into_iter()
                .find(|s| {
                    s.id == target.service && s.kind == wispnet_core::host::ServiceKind::Files
                })
                .ok_or_else(|| {
                    TransferError::new("PERMISSION_DENIED", "Files service unavailable")
                })?;
            if service.incarnation != target.incarnation {
                return Err(TransferError::new(
                    "SERVICE_CHANGED",
                    "Files service changed; refresh its catalog",
                ));
            }
            let exports = authority
                .file_exports()
                .await
                .map_err(|_| TransferError::new("UNAVAILABLE", "File exports unavailable"))?;
            if self.files.local.get().is_none() {
                let manager = FileService::new(authority.state_dir().join("files"))
                    .map_err(TransferError::io)?;
                let _ = self.files.local.set(manager);
            }
            let manager = self.files.local.get().unwrap().clone();
            let (stream, server) = tokio::io::duplex(MAX_CHUNK * 2);
            let own = self.endpoint.id().to_string();
            let incarnation = target.incarnation.clone();
            let authority = authority.clone();
            resources.local = Some(tokio::spawn(async move {
                let (reader, writer) = tokio::io::split(server);
                let guard = async {
                    loop {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        if !authority.services().await.is_ok_and(|services| {
                            services.iter().any(|s| {
                                s.id == "files"
                                    && s.kind == wispnet_core::host::ServiceKind::Files
                                    && s.incarnation == incarnation
                            })
                        }) {
                            break;
                        }
                    }
                };
                tokio::select! { _ = guard => {}, _ = manager.serve(&own, &incarnation, exports, reader, writer) => {} }
            }));
            return Ok(Box::new(stream));
        }
        if self
            .saved_host(target.host)
            .await
            .map_err(|_| TransferError::new("UNAVAILABLE", "Cannot inspect pairing"))?
            .is_none()
        {
            return Err(TransferError::new(
                "PERMISSION_DENIED",
                "Pair with this host first",
            ));
        }
        let catalog = self
            .catalog(target.host)
            .await
            .map_err(|_| TransferError::new("UNAVAILABLE", "Cannot read host catalog"))?;
        let selected = catalog["services"]
            .as_array()
            .and_then(|services| {
                services
                    .iter()
                    .find(|s| s["id"] == target.service && s["kind"] == "files")
            })
            .ok_or_else(|| {
                TransferError::new(
                    "PERMISSION_DENIED",
                    "Files service unavailable or not granted",
                )
            })?;
        if selected["incarnation"] != target.incarnation {
            return Err(TransferError::new(
                "SERVICE_CHANGED",
                "Files service changed; refresh its catalog",
            ));
        }
        let connection = wispnet_agent::tunnel::connect_target(
            &self.endpoint,
            self.target(target.host)
                .await
                .map_err(|_| TransferError::new("UNAVAILABLE", "Host unavailable"))?,
        )
        .await
        .map_err(|_| TransferError::new("UNAVAILABLE", "Cannot connect to Files service"))?;
        resources.connection = Some(connection.clone());
        let stream = wispnet_agent::tunnel::open_service_bound(
            &connection,
            &target.service,
            &target.incarnation,
        )
        .await
        .map_err(|error| match error {
            wispnet_agent::tunnel::TunnelError::Status(_) => {
                TransferError::new("PERMISSION_DENIED", "Files service access denied")
            }
            wispnet_agent::tunnel::TunnelError::Transport(_) => {
                TransferError::new("UNAVAILABLE", "Files service connection interrupted")
            }
        })?;
        if self
            .saved_host(target.host)
            .await
            .map_err(|_| TransferError::new("UNAVAILABLE", "Cannot inspect pairing"))?
            .is_none()
        {
            return Err(TransferError::new(
                "PERMISSION_DENIED",
                "Pairing was removed",
            ));
        }
        Ok(Box::new(stream))
    }
    async fn file_stream(self: &Arc<Self>, target: &FileTarget) -> Result<(BoxStream, Resources)> {
        let mut resources = Resources::default();
        let stream = tokio::time::timeout(
            Duration::from_secs(15),
            self.open_file_stream(target, &mut resources),
        )
        .await
        .map_err(|_| TransferError::new("UNAVAILABLE", "Files connection setup timed out"))??;
        Ok((stream, resources))
    }
    async fn prepare_resume(&self, binding: Binding) -> Result<ResumeFiles> {
        let directory = self.state.join("file-client");
        for attempt in 0..20 {
            let state = directory.clone();
            let binding = binding.clone();
            match blocking(move || resume_files(&state, &binding)).await {
                Err(error) if error.code == "CONFLICT" && attempt < 19 => {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                result => return result,
            }
        }
        unreachable!()
    }
    async fn put_file_inner(
        self: &Arc<Self>,
        target: FileTarget,
        workspace: &str,
        local_path: &Path,
        remote_path: &str,
        options: &TransferOptions,
    ) -> Result<FileInfo> {
        validate_target(&target)?;
        let local_path = absolute_file_path(local_path)?;
        let binding = Binding {
            version: 1,
            own: self.endpoint.id().to_string(),
            target: target.clone(),
            workspace: workspace.into(),
            remote_path: remote_path.into(),
            local_path: local_path.clone(),
            direction: "put".into(),
        };
        let resume = self.prepare_resume(binding.clone()).await?;
        options.report("hashing", 0, 0);
        let source_path = local_path.clone();
        let cancellation = options.cancellation.clone();
        let (size_bytes, sha256) = blocking(move || {
            let mut file = safe_open(&source_path, false, false)?;
            let metadata = file.metadata().map_err(TransferError::io)?;
            if !metadata.is_file() {
                return Err(TransferError::new(
                    "INVALID_ARGUMENT",
                    "Upload source must be a regular file",
                ));
            }
            Ok((
                metadata.len(),
                hash_reader_cancelled(&mut file, Some(&cancellation))?,
            ))
        })
        .await?;
        let journal_path = resume.journal.clone();
        let old = blocking(move || load_journal(&journal_path)).await?;
        let mut journal = if options.resume {
            old.filter(|j| j.binding == binding && j.size_bytes == size_bytes && j.sha256 == sha256)
        } else {
            None
        }
        .unwrap_or_else(|| Journal {
            binding,
            size_bytes,
            sha256: sha256.clone(),
            idempotency_key: crate::token(),
            transfer_id: None,
            committed_offset: 0,
        });
        persist(&resume.journal, &journal, &resume._lock).await?;
        for attempt in 0..MAX_RETRIES {
            let result = self
                .upload_attempt(
                    &target,
                    &local_path,
                    &resume.journal,
                    &resume._lock,
                    &mut journal,
                    options,
                )
                .await;
            match result {
                Ok(file) => return Ok(file),
                Err(error) if retryable(&error) && attempt + 1 < MAX_RETRIES => {
                    options.report("reconnecting", journal.committed_offset, size_bytes);
                    tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))).await;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
    async fn upload_attempt(
        self: &Arc<Self>,
        target: &FileTarget,
        local_path: &Path,
        journal_path: &Path,
        resume_lock: &Arc<File>,
        journal: &mut Journal,
        options: &TransferOptions,
    ) -> Result<FileInfo> {
        let (mut stream, _resources) = self.file_stream(target).await?;
        // UploadBegin recovers the peer-bound manifest, including a committed
        // transfer whose response was lost. The receive-only grant cannot stat
        // arbitrary destination files.
        write_request(
            &mut stream,
            &Request::UploadBegin {
                workspace: journal.binding.workspace.clone(),
                path: journal.binding.remote_path.clone(),
                size_bytes: journal.size_bytes,
                sha256: journal.sha256.clone(),
                idempotency_key: journal.idempotency_key.clone(),
            },
        )
        .await
        .map_err(TransferError::io)?;
        let (transfer_id, offset, chunk_size) = match response(&mut stream).await? {
            Response::Upload {
                transfer_id,
                state: TransferState::Committed,
                file: Some(file),
                ..
            } if file.sha256 == journal.sha256 && file.size_bytes == journal.size_bytes => {
                journal.transfer_id = Some(transfer_id);
                return Ok(file);
            }
            Response::Upload {
                transfer_id,
                state: TransferState::Uploading,
                committed_offset,
                max_chunk_bytes,
                size_bytes,
                sha256,
                ..
            } if size_bytes == journal.size_bytes
                && sha256 == journal.sha256
                && committed_offset <= size_bytes
                && max_chunk_bytes > 0 =>
            {
                (
                    transfer_id,
                    committed_offset,
                    (max_chunk_bytes as usize).min(MAX_CHUNK),
                )
            }
            Response::Upload {
                state: TransferState::Expired,
                ..
            } => {
                return Err(TransferError::new(
                    "TRANSFER_EXPIRED",
                    "Upload expired; repeat with resume disabled",
                ));
            }
            _ => return Err(protocol_error()),
        };
        journal.transfer_id = Some(transfer_id.clone());
        journal.committed_offset = offset;
        persist(journal_path, journal, resume_lock).await?;
        options.report("transferring", offset, journal.size_bytes);
        let mut source = tokio::fs::File::from_std(safe_open(local_path, false, false)?);
        source
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(TransferError::io)?;
        let (mut reader, mut writer) = tokio::io::split(stream);
        let slots = Arc::new(Semaphore::new(PIPELINE));
        let (queued, mut expected) = mpsc::channel(PIPELINE);
        let size = journal.size_bytes;
        let send = async {
            let mut at = offset;
            while at < size {
                let permit = slots
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(|_| protocol_error())?;
                let mut data = vec![0; (size - at).min(chunk_size as u64) as usize];
                source
                    .read_exact(&mut data)
                    .await
                    .map_err(TransferError::io)?;
                let (data, digest) = blocking(move || {
                    let digest = wispnet_files::sha256(&data);
                    Ok((data, digest))
                })
                .await?;
                let length = data.len() as u64;
                write_request(
                    &mut writer,
                    &Request::UploadWrite {
                        transfer_id: transfer_id.clone(),
                        offset: at,
                        chunk_sha256: digest,
                        data,
                    },
                )
                .await
                .map_err(TransferError::io)?;
                at += length;
                queued
                    .send((at, permit))
                    .await
                    .map_err(|_| protocol_error())?;
            }
            drop(queued);
            Ok::<_, TransferError>(())
        };
        let receive = async {
            while let Some((end, permit)) = expected.recv().await {
                match response(&mut reader).await? {
                    Response::Written { committed_offset }
                        if committed_offset >= end && committed_offset <= size =>
                    {
                        journal.committed_offset = committed_offset;
                        options.report("transferring", committed_offset, size);
                    }
                    _ => return Err(protocol_error()),
                }
                drop(permit);
            }
            Ok::<_, TransferError>(())
        };
        tokio::try_join!(send, receive)?;
        options.report("verifying", size, size);
        write_request(&mut writer, &Request::UploadCommit { transfer_id })
            .await
            .map_err(TransferError::io)?;
        let file = match response(&mut reader).await? {
            Response::File { file }
                if file.size_bytes == journal.size_bytes && file.sha256 == journal.sha256 =>
            {
                file
            }
            _ => return Err(protocol_error()),
        };
        options.report("complete", size, size);
        Ok(file)
    }
}

fn validate_target(target: &FileTarget) -> Result<()> {
    if target.service != "files" || !wispnet_core::enrollment::valid_code(&target.incarnation) {
        return Err(TransferError::new(
            "INVALID_ARGUMENT",
            "Use the built-in Files service and its current incarnation",
        ));
    }
    Ok(())
}
fn protocol_error() -> TransferError {
    TransferError::new("INVALID_ARGUMENT", "Invalid Files protocol response")
}
fn retryable(error: &TransferError) -> bool {
    error.code == "UNAVAILABLE"
}
async fn response(reader: &mut (impl AsyncRead + Unpin)) -> Result<Response> {
    let response = read_response(reader)
        .await
        .map_err(TransferError::io)?
        .ok_or_else(|| {
            TransferError::new(
                "UNAVAILABLE",
                "Files connection interrupted; transfer remains resumable",
            )
        })?;
    TransferError::response(response)
}
async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|_| TransferError::new("UNAVAILABLE", "File operation interrupted"))?
}
fn absolute_file_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(TransferError::new(
            "INVALID_ARGUMENT",
            "Local file path must be absolute",
        ));
    }
    let parent = std::fs::canonicalize(path.parent().unwrap()).map_err(TransferError::io)?;
    Ok(parent.join(path.file_name().unwrap()))
}
fn safe_open(path: &Path, write: bool, create: bool) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .create(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(TransferError::io)?;
    if !file.metadata().map_err(TransferError::io)?.is_file() {
        return Err(TransferError::new(
            "PATH_DENIED",
            "File transfer requires a regular file",
        ));
    }
    Ok(file)
}
fn resume_files(directory: &Path, binding: &Binding) -> Result<ResumeFiles> {
    std::fs::create_dir_all(directory).map_err(TransferError::io)?;
    let meta = std::fs::symlink_metadata(directory).map_err(TransferError::io)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(TransferError::new(
            "PATH_DENIED",
            "Unsafe transfer state directory",
        ));
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
        .map_err(TransferError::io)?;
    // Persist both the directory's metadata and its entry in the existing
    // owner state directory before any resumable offset can be journaled.
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(TransferError::io)?;
    sync_parent(directory)?;
    let key = wispnet_files::sha256(&serde_json::to_vec(binding).map_err(|_| protocol_error())?);
    let lock = safe_open(&directory.join(format!("{key}.lock")), true, true)?;
    let metadata = lock.metadata().map_err(TransferError::io)?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o077 != 0 {
        return Err(TransferError::new(
            "PATH_DENIED",
            "Transfer lock must be private to its owner",
        ));
    }
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(TransferError::new(
            "CONFLICT",
            "This file transfer is already active",
        ));
    }
    Ok(ResumeFiles {
        journal: directory.join(format!("{key}.json")),
        _lock: Arc::new(lock),
    })
}
fn load_journal(path: &Path) -> Result<Option<Journal>> {
    let file = match safe_open(path, false, false) {
        Ok(file) => file,
        Err(e) if e.code == "NOT_FOUND" => return Ok(None),
        Err(e) => return Err(e),
    };
    if file.metadata().map_err(TransferError::io)?.len() > 65536 {
        return Err(TransferError::new(
            "INVALID_ARGUMENT",
            "Transfer journal exceeds its size limit",
        ));
    }
    serde_json::from_reader(file)
        .map(Some)
        .map_err(|_| TransferError::new("INVALID_ARGUMENT", "Saved transfer state is invalid"))
}
async fn persist(path: &Path, journal: &Journal, resume_lock: &Arc<File>) -> Result<()> {
    let path = path.to_owned();
    let journal = journal.clone();
    let resume_lock = resume_lock.clone();
    blocking(move || {
        let _resume_lock = resume_lock;
        let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or_else(protocol_error)?)
            .map_err(TransferError::io)?;
        serde_json::to_writer(temp.as_file_mut(), &journal).map_err(|_| protocol_error())?;
        temp.as_file_mut().flush().map_err(TransferError::io)?;
        temp.as_file().sync_all().map_err(TransferError::io)?;
        temp.persist(&path)
            .map_err(|e| TransferError::io(e.error))?;
        sync_parent(&path)
    })
    .await
}
fn sync_parent(path: &Path) -> Result<()> {
    File::open(path.parent().ok_or_else(protocol_error)?)
        .and_then(|f| f.sync_all())
        .map_err(TransferError::io)
}
fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn hash_reader_cancelled(
    reader: &mut impl Read,
    cancellation: Option<&FileCancellation>,
) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        if cancellation.is_some_and(FileCancellation::is_cancelled) {
            return Err(TransferError::new(
                "CANCELLED",
                "Transfer cancelled; resumable data retained",
            ));
        }
        let n = reader.read(&mut buffer).map_err(TransferError::io)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex_digest(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> FileTarget {
        FileTarget {
            host: wispnet_core::identity::SecretKey::generate().public(),
            service: "files".into(),
            incarnation: "a".repeat(64),
        }
    }
    fn binding(path: PathBuf) -> Binding {
        Binding {
            version: 1,
            own: "owner".into(),
            target: target(),
            workspace: "training".into(),
            remote_path: "input.bin".into(),
            local_path: path,
            direction: "put".into(),
        }
    }
    #[tokio::test]
    async fn resume_binding_changes_with_target_identity_and_incarnation() {
        let directory = tempfile::tempdir().unwrap();
        let b = binding(directory.path().join("output"));
        let first = resume_files(&directory.path().join("state"), &b).unwrap();
        let mut changed = b.clone();
        changed.target.incarnation = "b".repeat(64);
        let second = resume_files(&directory.path().join("state"), &changed).unwrap();
        assert_ne!(first.journal, second.journal);
        assert!(resume_files(&directory.path().join("state"), &b).is_err());
        drop(first);
        assert!(resume_files(&directory.path().join("state"), &b).is_ok());
    }
    #[tokio::test]
    async fn private_journal_round_trips_and_preserves_upload_identity() {
        let directory = tempfile::tempdir().unwrap();
        let b = binding(directory.path().join("output"));
        let resume = resume_files(&directory.path().join("state"), &b).unwrap();
        let journal = Journal {
            binding: b.clone(),
            size_bytes: 5,
            sha256: wispnet_files::sha256(b"hello"),
            idempotency_key: "key".into(),
            transfer_id: Some("id".into()),
            committed_offset: 3,
        };
        persist(&resume.journal, &journal, &resume._lock)
            .await
            .unwrap();
        let loaded = load_journal(&resume.journal).unwrap().unwrap();
        assert_eq!(loaded.binding, b);
        assert_eq!(loaded.committed_offset, 3);
        assert_eq!(
            std::fs::metadata(&resume.journal)
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        assert_eq!(loaded.idempotency_key, "key");
        assert_eq!(loaded.transfer_id.as_deref(), Some("id"));
        // Preserve resume support for journals written before the service
        // became receive-only, without retaining download state in new journals.
        let mut legacy = serde_json::to_value(&journal).unwrap();
        legacy["file_version"] = json!(null);
        legacy["prefix_sha256"] = json!("");
        let restored: Journal = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.idempotency_key, journal.idempotency_key);
    }
    #[test]
    fn source_hashing_rejects_symlinks_and_observes_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(
            hash_reader_cancelled(&mut safe_open(&path, false, false).unwrap(), None).unwrap(),
            wispnet_files::sha256(b"hello")
        );
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(safe_open(&link, false, false).is_err());
        let cancellation = FileCancellation::default();
        cancellation.cancel();
        assert_eq!(
            hash_reader_cancelled(&mut b"hello".as_slice(), Some(&cancellation))
                .unwrap_err()
                .code,
            "CANCELLED"
        );
    }
    #[tokio::test]
    async fn cancellation_is_observed_even_before_waiting() {
        let token = FileCancellation::default();
        token.cancel();
        tokio::time::timeout(Duration::from_millis(10), token.cancelled())
            .await
            .unwrap();
    }
}
