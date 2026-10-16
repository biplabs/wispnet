//! Small, bounded open-service protocol. No caller identity or upstream is on the wire.
pub const ALPN: &[u8] = b"wipsnet/service/1";
pub const VERSION: u16 = 1;
pub const MAX_FRAME: usize = 128;
pub const MAX_SERVICE_ID: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Ok = 0,
    Denied = 1,
    UnknownService = 2,
    BackendUnavailable = 3,
    Busy = 4,
    Malformed = 5,
    UnsupportedVersion = 6,
    Timeout = 7,
}
impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "access denied",
            Self::UnknownService => "unknown service",
            Self::BackendUnavailable => "backend unavailable",
            Self::Busy => "tunnel capacity reached",
            Self::Malformed => "malformed handshake",
            Self::UnsupportedVersion => "unsupported protocol version",
            Self::Timeout => "handshake timed out",
        }
    }
}
pub fn valid_service_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SERVICE_ID
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
        && id.as_bytes()[0].is_ascii_alphanumeric()
}
pub fn request(id: &str) -> Result<Vec<u8>, Status> {
    if !valid_service_id(id) {
        return Err(Status::Malformed);
    }
    let mut frame = Vec::with_capacity(4 + id.len());
    frame.extend_from_slice(&((2 + id.len()) as u16).to_be_bytes());
    frame.extend_from_slice(&VERSION.to_be_bytes());
    frame.extend_from_slice(id.as_bytes());
    Ok(frame)
}
pub fn decode_request(payload: &[u8]) -> Result<&str, Status> {
    if payload.len() < 3 || payload.len() > MAX_FRAME {
        return Err(Status::Malformed);
    }
    if u16::from_be_bytes([payload[0], payload[1]]) != VERSION {
        return Err(Status::UnsupportedVersion);
    }
    let id = std::str::from_utf8(&payload[2..]).map_err(|_| Status::Malformed)?;
    if !valid_service_id(id) {
        return Err(Status::Malformed);
    }
    Ok(id)
}
pub fn response(status: Status) -> [u8; 5] {
    let version = VERSION.to_be_bytes();
    [0, 3, version[0], version[1], status as u8]
}
pub fn decode_response(payload: &[u8]) -> Result<Status, Status> {
    if payload.len() != 3 {
        return Err(Status::Malformed);
    }
    if u16::from_be_bytes([payload[0], payload[1]]) != VERSION {
        return Err(Status::UnsupportedVersion);
    }
    match payload[2] {
        0 => Ok(Status::Ok),
        1 => Ok(Status::Denied),
        2 => Ok(Status::UnknownService),
        3 => Ok(Status::BackendUnavailable),
        4 => Ok(Status::Busy),
        5 => Ok(Status::Malformed),
        6 => Ok(Status::UnsupportedVersion),
        7 => Ok(Status::Timeout),
        _ => Err(Status::Malformed),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_wire_format_rejects_addresses_forgery_and_versions() {
        let frame = request("website").unwrap();
        assert_eq!(decode_request(&frame[2..]), Ok("website"));
        for id in [
            "",
            "http://127.0.0.1:80",
            "../website",
            "website\0caller-key",
            "UPPER",
        ] {
            assert_eq!(request(id), Err(Status::Malformed));
        }
        assert!(request(&"a".repeat(MAX_SERVICE_ID + 1)).is_err());
        assert_eq!(
            decode_request(&[0, 2, b'a']),
            Err(Status::UnsupportedVersion)
        );
        assert_eq!(decode_request(&[0, 1, 255]), Err(Status::Malformed));
        assert_eq!(decode_request(&[0; MAX_FRAME + 1]), Err(Status::Malformed));
        assert_eq!(decode_response(&[0, 1, 255]), Err(Status::Malformed));
        for status in [
            Status::Ok,
            Status::Denied,
            Status::UnknownService,
            Status::BackendUnavailable,
            Status::Busy,
            Status::Malformed,
            Status::UnsupportedVersion,
            Status::Timeout,
        ] {
            assert_eq!(decode_response(&response(status)[2..]), Ok(status));
        }
    }
}
