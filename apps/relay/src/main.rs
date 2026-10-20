//! Standalone packet relay: no application/node identity or persistent registry.
use iroh_relay::server::{
    Access, AccessControl, AcmeConfig, CertConfig, ClientRateLimit, ClientRequest, ConnectionId,
    RelayConfig, Server, ServerConfig, TlsConfig,
};
use rustls_pki_types::pem::PemObject;
use serde::Deserialize;
use std::{
    fs::OpenOptions,
    io::Read,
    net::SocketAddr,
    num::NonZeroU32,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    listen: SocketAddr,
    #[serde(default)]
    http_only: bool,
    certificate: Option<PathBuf>,
    private_key: Option<PathBuf>,
    acme_hostname: Option<String>,
    acme_cache: Option<PathBuf>,
    max_connections: usize,
}

fn read_protected(path: &Path, secret: bool) -> Result<Vec<u8>, &'static str> {
    if !path.is_absolute() {
        return Err("relay file paths must be absolute");
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| "relay file unavailable")?;
    let m = file
        .metadata()
        .map_err(|_| "relay file metadata unavailable")?;
    // SAFETY: geteuid takes no pointers and has no preconditions.
    let uid = unsafe { libc::geteuid() };
    if !m.is_file()
        || m.nlink() != 1
        || (m.uid() != uid && m.uid() != 0)
        || m.mode() & if secret { 0o077 } else { 0o022 } != 0
        || m.len() > 65536
    {
        return Err("relay file must be protected, regular, singly linked and bounded");
    }
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "relay file read failed")?;
    if bytes.len() > 65536 {
        return Err("relay file too large");
    }
    Ok(bytes)
}

#[derive(Debug)]
struct Admission {
    current: AtomicUsize,
    maximum: usize,
}
impl Admission {
    fn admit(&self) -> bool {
        self.current
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.maximum).then_some(n + 1)
            })
            .is_ok()
    }
}
impl AccessControl for Admission {
    async fn on_connect(&self, _request: &ClientRequest) -> Access {
        if self.admit() {
            Access::Allow
        } else {
            Access::Deny {
                reason: Some("relay capacity".into()),
            }
        }
    }
    fn on_disconnect(&self, _endpoint: iroh_base::EndpointId, _connection: ConnectionId) {
        self.current.fetch_sub(1, Ordering::AcqRel);
    }
}

fn certificate_config(config: &Config) -> Result<CertConfig, &'static str> {
    let tls_builder = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "relay TLS configuration failed")?
    .with_no_client_auth();
    let result = match (
        &config.certificate,
        &config.private_key,
        &config.acme_hostname,
        &config.acme_cache,
    ) {
        (Some(certificate), Some(private_key), None, None) => {
            let certificates = read_protected(certificate, false)?;
            let certificates = rustls_pki_types::CertificateDer::pem_slice_iter(&certificates)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "invalid relay certificate")?;
            let key = Zeroizing::new(read_protected(private_key, true)?);
            let key = rustls_pki_types::PrivateKeyDer::from_pem_slice(&key)
                .map_err(|_| "invalid relay private key")?;
            CertConfig::Manual {
                server_config: tls_builder
                    .with_single_cert(certificates, key)
                    .map_err(|_| "relay certificate/key mismatch")?,
            }
        }
        (None, None, Some(hostname), Some(cache)) => {
            if !hostname.contains('.')
                || hostname.len() > 253
                || !hostname
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
                || !cache.is_absolute()
            {
                return Err("invalid ACME hostname/cache");
            }
            let meta = std::fs::symlink_metadata(cache).map_err(|_| "ACME cache unavailable")?;
            // SAFETY: geteuid has no preconditions.
            if !meta.is_dir()
                || meta.uid() != unsafe { libc::geteuid() }
                || meta.mode() & 0o077 != 0
            {
                return Err("unsafe ACME cache");
            }
            CertConfig::LetsEncrypt {
                acme_config: AcmeConfig::letsencrypt(true)
                    .domains(vec![hostname.clone()])
                    .cache_path(cache.clone()),
                server_config_builder: tls_builder,
            }
        }
        _ => return Err("choose exactly one complete manual or ACME TLS configuration"),
    };
    Ok(result)
}

async fn serve(path: &Path) -> Result<(), &'static str> {
    let bytes = Zeroizing::new(read_protected(path, true)?);
    let text = std::str::from_utf8(&bytes).map_err(|_| "invalid relay configuration")?;
    let config: Config = toml::from_str(text).map_err(|_| "invalid relay configuration")?;
    if config.listen.port() == 0 || !(1..=1024).contains(&config.max_connections) {
        return Err("invalid relay listener or connection limit");
    }
    if config.http_only
        && (!config.listen.ip().is_loopback()
            || config.certificate.is_some()
            || config.private_key.is_some()
            || config.acme_hostname.is_some()
            || config.acme_cache.is_some())
    {
        return Err(
            "HTTP-only packet relay must be loopback-only with TLS configured at its front end",
        );
    }
    let certificate_config = if config.http_only {
        None
    } else {
        Some(certificate_config(&config)?)
    };
    // Auxiliary HTTP stays loopback-only on an ephemeral port. QUIC and metrics
    // listeners remain disabled; only the configured TLS port is exposed.
    let mut relay = RelayConfig::new(if config.http_only {
        config.listen
    } else {
        "127.0.0.1:0"
            .parse::<SocketAddr>()
            .expect("constant address")
    });
    if let Some(certificate_config) = certificate_config {
        relay.tls = Some(TlsConfig::new(config.listen, certificate_config));
    }
    relay.access = Arc::new(Admission {
        current: AtomicUsize::new(0),
        maximum: config.max_connections,
    });
    let mut rate = ClientRateLimit::new(NonZeroU32::new(1024 * 1024).expect("nonzero"));
    rate.max_burst_bytes = NonZeroU32::new(2 * 1024 * 1024);
    relay.limits.client_rx = Some(rate);
    let mut server_config = ServerConfig::default();
    server_config.relay = Some(relay);
    let server = Server::spawn(server_config)
        .await
        .map_err(|_| "relay listener startup failed")?;
    println!(
        "{{\"event\":\"started\",\"role\":\"packet_relay\",\"listen\":\"{}\"}}",
        config.listen
    );
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "relay signal setup failed")?;
    tokio::select! { _=term.recv()=>{}, _=tokio::signal::ctrl_c()=>{} }
    server
        .shutdown()
        .await
        .map_err(|_| "relay shutdown failed")?;
    println!("{{\"event\":\"stopped\",\"role\":\"packet_relay\"}}");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = if args.len() == 2 && args[0] == "--config" {
        serve(Path::new(&args[1])).await
    } else {
        Err("usage: wispnet-relay --config /absolute/relay.toml")
    };
    if let Err(message) = result {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_is_bounded_and_capacity_can_be_reused() {
        let gate = Admission {
            current: AtomicUsize::new(0),
            maximum: 2,
        };
        assert!(gate.admit());
        assert!(gate.admit());
        assert!(!gate.admit());
        gate.current.fetch_sub(1, Ordering::AcqRel);
        assert!(gate.admit());
        assert!(!gate.admit());
    }
    #[test]
    fn private_key_file_rejects_shared_permissions_links_and_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let key = root.path().join("key.pem");
        std::fs::write(&key, b"not logged key material").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_protected(&key, true).is_ok());
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_protected(&key, true).is_err());
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        let alias = root.path().join("alias");
        symlink(&key, &alias).unwrap();
        assert!(read_protected(&alias, true).is_err());
        let hard = root.path().join("hard");
        std::fs::hard_link(&key, &hard).unwrap();
        assert!(read_protected(&key, true).is_err());
    }
    #[test]
    fn tls_mode_never_silently_falls_back_or_accepts_mixed_settings() {
        let mut config = Config {
            http_only: false,
            listen: "127.0.0.1:8443".parse().unwrap(),
            max_connections: 2,
            certificate: None,
            private_key: None,
            acme_hostname: None,
            acme_cache: None,
        };
        assert_eq!(
            certificate_config(&config).unwrap_err(),
            "choose exactly one complete manual or ACME TLS configuration"
        );
        config.certificate = Some("/not-read.pem".into());
        assert_eq!(
            certificate_config(&config).unwrap_err(),
            "choose exactly one complete manual or ACME TLS configuration"
        );
        config.private_key = Some("/not-read.key".into());
        config.acme_hostname = Some("relay.example.test".into());
        config.acme_cache = Some("/not-read".into());
        assert_eq!(
            certificate_config(&config).unwrap_err(),
            "choose exactly one complete manual or ACME TLS configuration"
        );
        config.certificate = None;
        config.private_key = None;
        config.acme_cache = Some("relative/cache".into());
        assert_eq!(
            certificate_config(&config).unwrap_err(),
            "invalid ACME hostname/cache"
        );
    }
}
