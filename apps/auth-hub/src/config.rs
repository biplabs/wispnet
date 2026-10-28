use crate::{Error, Result};
use std::path::PathBuf;

/// Never Debug: this configuration contains the GitHub application secret.
#[derive(Clone)]
pub struct Config {
    pub base_url: String,
    pub state_dir: PathBuf,
    pub public_domain: String,
    pub github_client_id: String,
    pub github_client_secret: String,
    /// Programmatic fixture configuration only; production never reads this from the environment.
    pub test_mode: bool,
}
impl Config {
    pub fn from_env() -> Result<Self> {
        Self {
            base_url: std::env::var("WISPNET_COMMAND_URL")
                .unwrap_or_else(|_| "https://cmd.wispnet.app".into()),
            state_dir: std::env::var("WISPNET_COMMAND_STATE")
                .unwrap_or_default()
                .into(),
            public_domain: std::env::var("WISPNET_PUBLIC_DOMAIN")
                .unwrap_or_else(|_| "wispnet.app".into()),
            github_client_id: std::env::var("WISPNET_GITHUB_CLIENT_ID").unwrap_or_default(),
            github_client_secret: std::env::var("WISPNET_GITHUB_CLIENT_SECRET").unwrap_or_default(),
            test_mode: false,
        }
        .validate()
    }
    pub fn validate(mut self) -> Result<Self> {
        let url = url::Url::parse(&self.base_url)
            .map_err(|_| Error::new(400, "Command center requires an HTTPS origin"))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || url.host_str().is_none()
            || !(url.scheme() == "https"
                || self.test_mode
                    && url.scheme() == "http"
                    && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")))
        {
            return Err(Error::new(400, "Command center requires an HTTPS origin"));
        }
        if !self.state_dir.is_absolute() || !valid_domain(&self.public_domain) {
            return Err(Error::new(400, "Invalid command center state or domain"));
        }
        if !self.test_mode
            && (self.github_client_id.is_empty() || self.github_client_secret.is_empty())
        {
            return Err(Error::new(
                400,
                "Configure WISPNET_GITHUB_CLIENT_ID and WISPNET_GITHUB_CLIENT_SECRET before starting GitHub registration",
            ));
        }
        self.base_url = url.origin().ascii_serialization();
        Ok(self)
    }
}
fn valid_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}
