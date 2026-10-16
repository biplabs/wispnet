//! Controller bootstrap uses a preconfigured HTTPS URL, never the service directory.
use crate::control::Directory;
use wipsnet_core::identity::EndpointId;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidController,
    Transport,
    Rejected,
    InvalidResponse,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidController => "controller must be an HTTPS origin",
            Self::Transport => "controller connection or certificate validation failed",
            Self::Rejected => "controller rejected request",
            Self::InvalidResponse => "invalid controller response",
        })
    }
}
impl std::error::Error for Error {}
/// No Debug implementation, credentials in memory are zeroized on drop.
pub struct Client {
    http: reqwest::Client,
    origin: reqwest::Url,
    secret: Zeroizing<String>,
}
impl Client {
    pub fn new(origin: &str, secret: Zeroizing<String>) -> Result<Self, Error> {
        let origin = wipsnet_core::config::controller_origin(origin)
            .map_err(|_| Error::InvalidController)?;
        if !crate::control::valid_token(&secret) {
            return Err(Error::InvalidController);
        }
        let http = http_builder().build().map_err(|_| Error::Transport)?;
        Ok(Self {
            http,
            origin,
            secret,
        })
    }
    pub async fn directory(&self, expected_device: Option<EndpointId>) -> Result<Directory, Error> {
        let url = self
            .origin
            .join("v1/directory")
            .map_err(|_| Error::InvalidController)?;
        let mut response = self
            .http
            .get(url)
            .bearer_auth(&*self.secret)
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::Rejected);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(Error::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        let directory: Directory =
            serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)?;
        if directory.device != expected_device
            || !wipsnet_core::protocol::valid_service_id(&directory.mesh)
            || directory.revision >= i64::MAX as u64
            || directory.devices.len() > 1024
            || directory.services.len() > 1024
            || directory
                .services
                .iter()
                .map(|s| s.grants.len())
                .sum::<usize>()
                > 16384
            || directory
                .services
                .iter()
                .any(|s| !wipsnet_core::protocol::valid_service_id(&s.id) || s.grants.len() > 1024)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(directory)
    }
}

fn http_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::{
        TlsAcceptor,
        rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
    };
    async fn tls_server(
        response: String,
    ) -> (
        String,
        reqwest::Certificate,
        tokio::task::JoinHandle<()>,
        String,
    ) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let root = reqwest::Certificate::from_der(cert.cert.der()).unwrap();
        let pem = cert.cert.pem();
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.cert.der().clone()],
                PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()).into(),
            )
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!(
            "https://localhost:{}",
            listener.local_addr().unwrap().port()
        );
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            if let Ok(mut tls) = TlsAcceptor::from(Arc::new(config)).accept(tcp).await {
                let mut data = Vec::new();
                let mut chunk = [0u8; 1024];
                loop {
                    let count = tls.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    data.extend_from_slice(&chunk[..count]);
                    if data.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                    assert!(data.len() < 4096);
                }
                let _ = tls.write_all(response.as_bytes()).await;
                let _ = tls.shutdown().await;
            }
        });
        (origin, root, task, pem)
    }
    fn body() -> String {
        let data = r#"{"mesh":"test","revision":1,"device":null,"devices":[],"services":[]}"#;
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{data}",
            data.len()
        )
    }
    #[test]
    fn insecure_and_ambiguous_origins_are_rejected() {
        for origin in [
            "http://localhost",
            "https://owner:secret@example.com",
            "https://example.com/private",
            "https://example.com/?secret=x",
            "https://example.com/#x",
        ] {
            assert!(matches!(
                Client::new(origin, crate::control::token()),
                Err(Error::InvalidController)
            ));
        }
    }
    #[tokio::test]
    async fn controller_certificate_and_hostname_are_validated() {
        let (origin, _root, task, _pem) = tls_server(body()).await;
        let client = Client::new(&origin, crate::control::token()).unwrap();
        assert!(matches!(
            client.directory(None).await,
            Err(Error::Transport)
        ));
        task.await.unwrap();
        let (origin, root, task, _pem) = tls_server(body()).await;
        let mut client = Client::new(&origin, crate::control::token()).unwrap();
        client.http = http_builder().add_root_certificate(root).build().unwrap();
        assert_eq!(client.directory(None).await.unwrap().revision, 1);
        task.await.unwrap();
        let (origin, root, task, _pem) = tls_server(body()).await;
        let mut client = Client::new(
            &origin.replace("localhost", "127.0.0.1"),
            crate::control::token(),
        )
        .unwrap();
        client.http = http_builder().add_root_certificate(root).build().unwrap();
        assert!(matches!(
            client.directory(None).await,
            Err(Error::Transport)
        ));
        task.await.unwrap();
    }
    #[tokio::test]
    async fn a_device_cannot_accept_a_directory_for_another_identity() {
        let (origin, root, task, _pem) = tls_server(body()).await;
        let mut client = Client::new(&origin, crate::control::token()).unwrap();
        client.http = http_builder().add_root_certificate(root).build().unwrap();
        assert!(matches!(
            client
                .directory(Some(
                    wipsnet_core::identity::Identity::generate().endpoint_id()
                ))
                .await,
            Err(Error::InvalidResponse)
        ));
        task.await.unwrap();
    }
    #[tokio::test]
    async fn owner_tool_writes_enrollment_credential_without_printing_it() {
        use std::os::unix::fs::PermissionsExt;
        let secret = crate::control::token();
        let data = serde_json::json!({"revision":1,"token":&*secret}).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{data}",
            data.len()
        );
        let (origin, _root, task, pem) = tls_server(response).await;
        let dir = tempfile::tempdir().unwrap();
        let owner = dir.path().join("owner.token");
        let certificate = dir.path().join("root.pem");
        let mutation = dir.path().join("enroll.json");
        let output = dir.path().join("device.token");
        std::fs::write(&owner, &*crate::control::token()).unwrap();
        std::fs::set_permissions(&owner, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&certificate, pem).unwrap();
        std::fs::write(
            &mutation,
            serde_json::json!({"op":"enroll","expected_revision":0,"device":wipsnet_core::identity::Identity::generate().endpoint_id()}).to_string(),
        )
        .unwrap();
        let destination = output.clone();
        let result = tokio::task::spawn_blocking(move || {
            std::process::Command::new("python3")
                .arg(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../scripts/registry_admin.py"),
                )
                .args(["--origin", &origin, "--owner-token"])
                .arg(owner)
                .arg("--mutation")
                .arg(mutation)
                .arg("--credential-out")
                .arg(destination)
                .env("SSL_CERT_FILE", certificate)
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!String::from_utf8_lossy(&result.stdout).contains(&*secret));
        assert_eq!(std::fs::read_to_string(&output).unwrap(), *secret);
        assert_eq!(
            std::fs::metadata(output).unwrap().permissions().mode() & 0o777,
            0o600
        );
        task.await.unwrap();
    }
    #[tokio::test]
    async fn redirects_never_forward_credentials_or_downgrade() {
        let trap = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            trap.local_addr().unwrap()
        );
        let (origin, root, task, _pem) = tls_server(response).await;
        let mut client = Client::new(&origin, crate::control::token()).unwrap();
        client.http = http_builder().add_root_certificate(root).build().unwrap();
        assert!(matches!(client.directory(None).await, Err(Error::Rejected)));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), trap.accept())
                .await
                .is_err()
        );
        task.await.unwrap();
    }
}
