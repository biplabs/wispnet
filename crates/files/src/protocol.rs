use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_CHUNK: usize = 4 * 1024 * 1024;
pub const MAX_METADATA: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub file_version: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    Uploading,
    Committed,
    Aborted,
    Expired,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Unavailable,
    PermissionDenied,
    ServiceChanged,
    NotFound,
    PathDenied,
    Conflict,
    InvalidArgument,
    InvalidOffset,
    HashMismatch,
    StaleVersion,
    NoSpace,
    ResourceLimit,
    TransferExpired,
    AlreadyCommitted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    UploadBegin {
        workspace: String,
        path: String,
        size_bytes: u64,
        sha256: String,
        idempotency_key: String,
    },
    UploadWrite {
        transfer_id: String,
        offset: u64,
        chunk_sha256: String,
        #[serde(skip)]
        data: Vec<u8>,
    },
    UploadStatus {
        transfer_id: String,
    },
    UploadCommit {
        transfer_id: String,
    },
    UploadAbort {
        transfer_id: String,
    },
    FileStat {
        workspace: String,
        path: String,
    },
    FileReadRange {
        workspace: String,
        path: String,
        expected_version: String,
        offset: u64,
        max_bytes: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Upload {
        transfer_id: String,
        state: TransferState,
        committed_offset: u64,
        size_bytes: u64,
        sha256: String,
        max_chunk_bytes: u64,
        expires_at: u64,
        file: Option<FileInfo>,
    },
    Written {
        committed_offset: u64,
    },
    File {
        file: FileInfo,
    },
    Aborted,
    Range {
        file_version: String,
        offset: u64,
        next_offset: u64,
        eof: bool,
        chunk_sha256: String,
        #[serde(skip)]
        data: Vec<u8>,
    },
    Error {
        code: ErrorCode,
        message: String,
        expected_offset: Option<u64>,
    },
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid Files frame")
}

async fn read_frame(
    reader: &mut (impl AsyncRead + Unpin),
) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut first = [0];
    if reader.read(&mut first).await? == 0 {
        return Ok(None);
    }
    let mut rest = [0; 3];
    reader.read_exact(&mut rest).await?;
    let metadata_len = u32::from_be_bytes([first[0], rest[0], rest[1], rest[2]]) as usize;
    let data_len = reader.read_u32().await? as usize;
    if metadata_len == 0 || metadata_len > MAX_METADATA || data_len > MAX_CHUNK {
        return Err(invalid());
    }
    let mut metadata = vec![0; metadata_len];
    reader.read_exact(&mut metadata).await?;
    let mut data = vec![0; data_len];
    reader.read_exact(&mut data).await?;
    Ok(Some((metadata, data)))
}

async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    metadata: Vec<u8>,
    data: &[u8],
) -> io::Result<()> {
    if metadata.is_empty() || metadata.len() > MAX_METADATA || data.len() > MAX_CHUNK {
        return Err(invalid());
    }
    writer.write_u32(metadata.len() as u32).await?;
    writer.write_u32(data.len() as u32).await?;
    writer.write_all(&metadata).await?;
    writer.write_all(data).await?;
    writer.flush().await
}

pub async fn read_request(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Option<Request>> {
    let Some((metadata, data)) = read_frame(reader).await? else {
        return Ok(None);
    };
    let mut request = serde_json::from_slice(&metadata).map_err(|_| invalid())?;
    if let Request::UploadWrite { data: target, .. } = &mut request {
        *target = data;
    } else if !data.is_empty() {
        return Err(invalid());
    }
    Ok(Some(request))
}

pub async fn write_request(
    writer: &mut (impl AsyncWrite + Unpin),
    request: &Request,
) -> io::Result<()> {
    let data = match request {
        Request::UploadWrite { data, .. } => data.as_slice(),
        _ => &[],
    };
    write_frame(
        writer,
        serde_json::to_vec(request).map_err(|_| invalid())?,
        data,
    )
    .await
}

pub async fn read_response(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Option<Response>> {
    let Some((metadata, data)) = read_frame(reader).await? else {
        return Ok(None);
    };
    let mut response = serde_json::from_slice(&metadata).map_err(|_| invalid())?;
    if let Response::Range { data: target, .. } = &mut response {
        *target = data;
    } else if !data.is_empty() {
        return Err(invalid());
    }
    Ok(Some(response))
}

pub async fn write_response(
    writer: &mut (impl AsyncWrite + Unpin),
    response: &Response,
) -> io::Result<()> {
    let data = match response {
        Response::Range { data, .. } => data.as_slice(),
        _ => &[],
    };
    write_frame(
        writer,
        serde_json::to_vec(response).map_err(|_| invalid())?,
        data,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn binary_payload_is_bounded_and_never_json_encoded() {
        let request = Request::UploadWrite {
            transfer_id: "a".repeat(64),
            offset: 0,
            chunk_sha256: "b".repeat(64),
            data: vec![0, 255, 1, 128],
        };
        let mut bytes = Vec::new();
        write_request(&mut bytes, &request).await.unwrap();
        assert_eq!(
            read_request(&mut bytes.as_slice()).await.unwrap(),
            Some(request)
        );
        assert!(read_response(&mut bytes.as_slice()).await.is_err());
        assert!(read_request(&mut [255; 8].as_slice()).await.is_err());
        assert!(
            read_request(&mut bytes[..bytes.len() - 1].as_ref())
                .await
                .is_err()
        );
    }
}
