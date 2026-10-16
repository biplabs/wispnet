use serde::Deserialize;
use std::io::Read;
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

/// Configuration is intentionally not Debug: future credentials must not leak.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u16,
    pub device_name: String,
    pub state_dir: PathBuf,
    pub log_level: LogLevel,
    #[serde(default)]
    pub transport: Option<TransportConfig>,
    #[serde(default)]
    pub registry: Option<RegistryConfig>,
    #[serde(default)]
    pub controller: Option<ControllerConfig>,
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
}

/// Only a trusted local HTTPS reverse proxy may reach this listener.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryConfig {
    pub bind_addr: std::net::SocketAddr,
    #[serde(default = "default_mesh")]
    pub mesh: String,
}

fn default_mesh() -> String {
    "default".into()
}
/// Out-of-band controller origin and device credential, never a service route.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerConfig {
    pub origin: String,
    pub credential_file: PathBuf,
    /// Optional independently provisioned trust root for a private HTTPS controller.
    #[serde(default)]
    pub ca_certificate_file: Option<PathBuf>,
}

/// Parse an out-of-band controller origin without accepting embedded authority.
pub fn controller_origin(origin: &str) -> Result<url::Url, ConfigError> {
    let url =
        url::Url::parse(origin).map_err(|_| ConfigError("invalid controller HTTPS origin"))?;
    if origin.len() > 2048
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port() == Some(0)
    {
        return Err(ConfigError(
            "controller must be an HTTPS origin without credentials, path, query or fragment",
        ));
    }
    Ok(url)
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportConfig {
    pub bind_addr: std::net::SocketAddr,
    #[serde(default = "default_connections")]
    pub max_connections: usize,
    #[serde(default = "default_streams")]
    pub max_streams: usize,
    #[serde(default = "default_per_connection")]
    pub max_streams_per_connection: usize,
}
fn default_connections() -> usize {
    16
}
fn default_streams() -> usize {
    64
}
fn default_per_connection() -> usize {
    8
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    pub id: String,
    pub backend: std::net::SocketAddr,
    pub allow: Vec<crate::identity::EndpointId>,
}
impl TransportConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(1..=128).contains(&self.max_connections)
            || !(1..=256).contains(&self.max_streams)
            || !(1..=32).contains(&self.max_streams_per_connection)
        {
            return Err(ConfigError(
                "transport limits must be connections 1-128, streams 1-256, streams per connection 1-32",
            ));
        }
        Ok(())
    }
}
pub fn validate_services(services: &[ServiceConfig]) -> Result<(), ConfigError> {
    if services.len() > 128 {
        return Err(ConfigError("at most 128 services may be configured"));
    }
    let mut ids = std::collections::HashSet::new();
    for service in services {
        if !crate::protocol::valid_service_id(&service.id) || !ids.insert(&service.id) {
            return Err(ConfigError(
                "service IDs must be unique lowercase identifiers of 1-64 bytes",
            ));
        }
        if !service.backend.ip().is_loopback() || service.backend.port() == 0 {
            return Err(ConfigError(
                "service backend must be an explicit loopback IP with a nonzero port",
            ));
        }
        if service.allow.len() > 128 {
            return Err(ConfigError(
                "at most 128 peer grants may be configured per service",
            ));
        }
        let unique: std::collections::HashSet<_> = service.allow.iter().collect();
        if unique.len() != service.allow.len() {
            return Err(ConfigError("duplicate peer grant"));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        }
    }
}

/// Never retain parser messages: they may include raw configuration or secrets.
#[derive(Debug)]
pub struct ConfigError(pub &'static str);
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ConfigError {}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.is_absolute() {
            return Err(ConfigError("--config must be an absolute path"));
        }
        let file = fs::File::open(path).map_err(|_| {
            ConfigError("cannot read configuration; check --config and file permissions")
        })?;
        let metadata = file
            .metadata()
            .map_err(|_| ConfigError("cannot inspect configuration"))?;
        if !metadata.is_file() || metadata.len() > 65536 {
            return Err(ConfigError(
                "configuration must be a regular file of at most 64 KiB",
            ));
        }
        let mut text = String::new();
        file.take(65537).read_to_string(&mut text).map_err(|_| {
            ConfigError("cannot read UTF-8 configuration; check file permissions and encoding")
        })?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        if text.len() > 65536 {
            return Err(ConfigError("configuration exceeds 64 KiB"));
        }
        let config: Self = toml::from_str(text).map_err(|_| ConfigError("invalid TOML or schema; expected base fields and optional transport/services (see config/example.toml)"))?;
        if config.schema_version != 1 {
            return Err(ConfigError("schema_version must be 1"));
        }
        if config.device_name.trim().is_empty()
            || config.device_name.len() > 128
            || config.device_name.chars().any(char::is_control)
        {
            return Err(ConfigError(
                "device_name must contain 1-128 bytes without control characters",
            ));
        }
        if !config.state_dir.is_absolute()
            || config
                .state_dir
                .components()
                .any(|p| matches!(p, std::path::Component::ParentDir))
            || config.state_dir.parent().is_none()
        {
            return Err(ConfigError(
                "state_dir must be an absolute non-root path without .. components",
            ));
        }
        if let Some(transport) = &config.transport {
            transport.validate()?;
        }
        if config.transport.is_none() && !config.services.is_empty() {
            return Err(ConfigError(
                "services require an explicit transport section",
            ));
        }
        if let Some(registry) = &config.registry
            && (!registry.bind_addr.ip().is_loopback()
                || registry.bind_addr.port() == 0
                || !crate::protocol::valid_service_id(&registry.mesh))
        {
            return Err(ConfigError(
                "registry must bind an explicit loopback IP and nonzero port",
            ));
        }
        if let Some(controller) = &config.controller
            && (controller_origin(&controller.origin).is_err()
                || controller.ca_certificate_file.as_ref().is_some_and(|p| {
                    !p.is_absolute()
                        || p.components()
                            .any(|c| matches!(c, std::path::Component::ParentDir))
                })
                || !controller.credential_file.is_absolute()
                || controller
                    .credential_file
                    .components()
                    .any(|p| matches!(p, std::path::Component::ParentDir)))
        {
            return Err(ConfigError(
                "controller requires an HTTPS origin and absolute credential_file",
            ));
        }
        validate_services(&config.services)?;
        Ok(config)
    }

    /// Called only after all configuration has been validated.
    pub fn prepare_state(&self) -> Result<(), ConfigError> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.state_dir).map_err(|_| {
            ConfigError("cannot create state_dir; check its parent and permissions")
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::symlink_metadata(&self.state_dir)
                .map_err(|_| ConfigError("cannot inspect state_dir"))?;
            if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
                return Err(ConfigError(
                    "state_dir must be a real private directory (chmod 700); do not use a shared directory",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> String {
        "schema_version = 1
device_name = 'laptop'
state_dir = '/tmp/wipsnet-test'
log_level = 'info'"
            .into()
    }
    #[test]
    fn rejects_bad_configuration() {
        assert!(Config::parse(&valid()).is_ok());
        for text in [
            valid().replace("= 1", "= 2"),
            valid().replace("/tmp/wipsnet-test", "relative"),
            valid().replace("/tmp/wipsnet-test", "/"),
            valid().replace("/tmp/wipsnet-test", "/tmp/../secret"),
            valid().replace("laptop", ""),
            valid().replace("'info'", "'trace'"),
            format!(
                "{}
unknown = 'value'",
                valid()
            ),
        ] {
            assert!(Config::parse(&text).is_err());
        }
    }
    #[test]
    fn invalid_network_configuration_fails_before_startup() {
        let peer = crate::identity::SecretKey::generate().public().to_string();
        let transport = "\n[transport]\nbind_addr='127.0.0.1:0'\n";
        let service =
            format!("\n[[services]]\nid='website'\nbackend='127.0.0.1:8080'\nallow=['{peer}']\n");
        assert!(Config::parse(&format!("{}{transport}{service}", valid())).is_ok());
        for text in [
            format!("{}{service}", valid()),
            format!("{}{transport}max_connections=0\n", valid()),
            format!("{}{transport}max_streams=257\n", valid()),
            format!("{}{transport}max_streams_per_connection=0\n", valid()),
            format!("{}{transport}max_streams_per_connection=33\n", valid()),
            format!(
                "{}{transport}{}",
                valid(),
                service.replace("127.0.0.1:8080", "192.168.1.1:8080")
            ),
            format!(
                "{}{transport}{}",
                valid(),
                service.replace("127.0.0.1:8080", "127.0.0.1:0")
            ),
            format!(
                "{}{transport}{}",
                valid(),
                service.replace("'website'", "'http://upstream'")
            ),
            format!(
                "{}{transport}{}",
                valid(),
                service.replace(&peer, "SECRET-SENTINEL")
            ),
            format!("{}{transport}{service}{service}", valid()),
            format!(
                "{}{transport}{}",
                valid(),
                service.replace(&format!("['{peer}']"), &format!("['{peer}','{peer}']"))
            ),
        ] {
            let error = Config::parse(&text)
                .err()
                .expect("invalid network configuration accepted");
            assert!(!error.to_string().contains("SECRET-SENTINEL"));
        }
    }

    #[test]
    fn registry_binds_and_controller_origins_fail_closed() {
        for address in ["0.0.0.0:7780", "[::]:7780", "192.0.2.1:7780", "127.0.0.1:0"] {
            assert!(
                Config::parse(&format!("{}\n[registry]\nbind_addr='{address}'", valid())).is_err()
            );
        }
        assert!(
            Config::parse(&format!(
                "{}\n[registry]\nbind_addr='127.0.0.1:7780'",
                valid()
            ))
            .is_ok()
        );
        for origin in [
            "http://controller.example",
            "https://user:SECRET-SENTINEL@controller.example",
            "https://controller.example/path",
            "https://controller.example/?token=SECRET-SENTINEL",
            "https://controller.example/#x",
            "https://controller.example:0",
        ] {
            let text = format!(
                "{}\n[controller]\norigin='{origin}'\ncredential_file='/private/device.token'",
                valid()
            );
            let error = Config::parse(&text).err().unwrap();
            assert!(!error.to_string().contains("SECRET-SENTINEL"));
        }
    }
    #[test]
    fn errors_never_echo_sensitive_input() {
        for field in [
            "private_key",
            "enrollment_token",
            "browser_token",
            "http_payload",
        ] {
            let text = format!(
                "{}
{field} = 'SECRET-SENTINEL'",
                valid()
            );
            let error = Config::parse(&text).err().unwrap();
            assert!(!format!("{error:?} {error}").contains("SECRET-SENTINEL"));
        }
        let error = Config::parse("device_name = 'SECRET-SENTINEL")
            .err()
            .unwrap();
        assert!(!error.to_string().contains("SECRET-SENTINEL"));
    }
}
