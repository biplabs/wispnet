//! Fixed HTTPS-only egress/relay preflight. No credentials or configurable URLs.
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn get(host: &'static str, path: &'static str) -> Result<String, Box<dyn std::error::Error>> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect((host, 443)).await?;
    let mut tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(host.try_into()?, tcp)
        .await?;
    tls.write_all(
        format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await?;
    let mut bytes = Vec::new();
    tls.take(8192).read_to_end(&mut bytes).await?;
    let text = String::from_utf8(bytes)?;
    if !text.starts_with("HTTP/1.1 200 ") && !text.starts_with("HTTP/1.0 200 ") {
        return Err("HTTPS preflight requires status 200".into());
    }
    Ok(text)
}
fn public_ip(response: &str) -> Option<IpAddr> {
    let (_, body) = response.split_once("\r\n\r\n")?;
    body.lines()
        .find_map(|line| line.strip_prefix("ip=").and_then(|ip| ip.parse().ok()))
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        let response = get("www.cloudflare.com", "/cdn-cgi/trace").await?;
        let ip = public_ip(&response).ok_or("egress address missing")?;
        get("relay.wispnet.app", "/ping").await?;
        Ok::<_, Box<dyn std::error::Error>>(ip)
    })
    .await??;
    println!(
        "{}",
        serde_json::json!({"event":"network_preflight","public_ip":result.to_string(),"relay_https":true})
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn egress_is_read_only_from_a_valid_response_body() {
        assert_eq!(
            public_ip("HTTP/1.1 200 OK\r\nip=192.0.2.1\r\n\r\nip=198.51.100.2\n"),
            Some("198.51.100.2".parse().unwrap())
        );
        assert_eq!(public_ip("ip=198.51.100.2"), None);
        assert_eq!(
            public_ip("HTTP/1.1 200 OK\r\n\r\nip=not-an-address\n"),
            None
        );
    }
}
