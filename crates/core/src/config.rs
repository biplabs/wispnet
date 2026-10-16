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
        let config: Self = toml::from_str(text).map_err(|_| ConfigError("invalid TOML or schema; expected schema_version, device_name, state_dir, log_level (see config/example.toml)"))?;
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
