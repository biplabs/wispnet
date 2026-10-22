//! Opt-in public relay smoke check using disposable identities and no device state.
use iroh::Watcher;
use std::time::Duration;
use wispnet_agent::tunnel::{bind_configured_client, connect_target, peer_target};
use wispnet_core::{config::TransportConfig, identity::SecretKey, protocol};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let relay = std::env::args()
        .nth(1)
        .ok_or("usage: relay_failover_probe BACKUP_HTTPS_ORIGIN")?;
    let config: TransportConfig = serde_json::from_value(serde_json::json!({
        "bind_addr":"0.0.0.0:0", "relay_only":true,
        "relay_url":"https://127.0.0.1:9", "backup_relay_urls":[relay],
    }))?;
    config.validate()?;
    let server_key = SecretKey::generate();
    let client_key = SecretKey::generate();
    let server = bind_configured_client(&server_key, Some(&config)).await?;
    server.set_alpns(vec![protocol::ALPN.to_vec()]);
    let client = bind_configured_client(&client_key, Some(&config)).await?;
    let result = tokio::time::timeout(Duration::from_secs(45), async {
        tokio::join!(server.online(), client.online());
        let target = peer_target(server.id(), Some(&config), None)?;
        let receive = async {
            let incoming = server.accept().await.ok_or("no incoming connection")?;
            let connection = incoming.await?;
            if connection.remote_id() != client.id() {
                return Err("wrong client identity".into());
            }
            let (mut send, mut recv) = connection.accept_bi().await?;
            let bytes = recv.read_to_end(128).await?;
            send.write_all(&bytes).await?;
            send.finish()?;
            let _ = send.stopped().await;
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        let transmit = async {
            let connection = connect_target(&client, target).await?;
            let (mut send, mut recv) = connection.open_bi().await?;
            send.write_all(b"wispnet backup relay").await?;
            send.finish()?;
            let bytes = recv.read_to_end(128).await?;
            if bytes != b"wispnet backup relay" || connection.paths().iter().any(|p| !p.is_relay())
            {
                return Err("echo or relay-only path check failed".into());
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        };
        tokio::try_join!(receive, transmit)?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
    .await;
    let statuses = client.home_relay_status().get();
    client.close().await;
    server.close().await;
    result??;
    println!(
        "{}",
        serde_json::json!({"echo":"ok","direct_transports":false,"primary":"unreachable","relays":statuses.iter().map(|s|serde_json::json!({"url":s.url(),"connected":s.is_connected()})).collect::<Vec<_>>()})
    );
    Ok(())
}
