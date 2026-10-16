//! Iroh 1.3 wire probe with Sites' private service header. Token comes from stdin.
//! This deliberately does not pretend the stock Iroh client supports that header.
use iroh_base::SecretKey;
use n0_future::{SinkExt, StreamExt};
use std::io::{self, BufRead, Write};
use tokio_websockets::{ClientBuilder, Message};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3
        || !args[1].starts_with("wss://")
        || !["listen", "send"].contains(&args[2].as_str())
    {
        return Err("usage: sites_relay_probe wss://SITE/relay listen|send; token and optional destination on stdin".into());
    }
    let mut lines = io::stdin().lock().lines();
    let token = zeroize::Zeroizing::new(lines.next().ok_or("missing token")??);
    let destination = if args[2] == "send" {
        Some(
            lines
                .next()
                .ok_or("missing destination")??
                .parse::<iroh_base::PublicKey>()?,
        )
    } else {
        None
    };
    let secret = SecretKey::generate();
    let builder = ClientBuilder::new()
        .uri(&args[1])?
        .add_header(
            http::header::SEC_WEBSOCKET_PROTOCOL,
            "iroh-relay-v2".parse()?,
        )?
        .add_header(
            http::HeaderName::from_static("oai-sites-authorization"),
            format!("Bearer {}", &*token).parse()?,
        )?;
    let (mut socket, response) =
        match tokio::time::timeout(std::time::Duration::from_secs(15), builder.connect()).await? {
            Ok(value) => value,
            Err(_) => {
                println!("{}", serde_json::json!({"event":"upgrade_failed"}));
                return Err("WebSocket upgrade failed (details redacted)".into());
            }
        };
    if response
        .headers()
        .get(http::header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|h| h.to_str().ok())
        != Some("iroh-relay-v2")
    {
        return Err("subprotocol mismatch".into());
    }
    let first = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
        .await?
        .ok_or("missing challenge")??;
    let challenge = first.as_payload();
    if !first.is_binary() || challenge.len() != 17 || challenge[0] != 0 {
        return Err("invalid challenge".into());
    }
    let signature = secret.sign(&blake3::derive_key(
        "iroh-relay handshake v1 challenge signature",
        &challenge[1..],
    ));
    let mut auth = vec![1];
    auth.extend_from_slice(secret.public().as_bytes());
    auth.push(64);
    auth.extend_from_slice(&signature.to_bytes());
    socket.send(Message::binary(auth)).await?;
    let confirmation = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
        .await?
        .ok_or("missing confirmation")??;
    if confirmation.as_payload().as_ref() != [2] {
        println!(
            "{}",
            serde_json::json!({"event":"authentication_failed","binary":confirmation.is_binary(),"close":confirmation.is_close()})
        );
        return Err("authentication denied".into());
    }
    println!(
        "{}",
        serde_json::json!({"event":"authenticated","peer":secret.public().to_string()})
    );
    io::stdout().flush()?;
    if let Some(destination) = destination {
        let mut frame = vec![4];
        frame.extend_from_slice(destination.as_bytes());
        frame.push(0);
        frame.extend_from_slice(b"wispnet-sites-compatibility");
        socket.send(Message::binary(frame)).await?;
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let message = tokio::time::timeout_at(deadline, socket.next())
            .await?
            .ok_or("relay closed")??;
        if !message.is_binary() {
            continue;
        }
        let frame = message.as_payload();
        match frame.first() {
            Some(9) if frame.len() == 9 => {
                let mut pong = frame.to_vec();
                pong[0] = 10;
                socket.send(Message::binary(pong)).await?;
            }
            Some(6) if frame.len() >= 34 && &frame[34..] == b"wispnet-sites-compatibility" => {
                if let Some(expected) = destination {
                    if &frame[1..33] != expected.as_bytes() {
                        return Err("wrong sender".into());
                    }
                } else {
                    let mut reply = frame.to_vec();
                    reply[0] = 4;
                    socket.send(Message::binary(reply)).await?;
                }
                println!("{}", serde_json::json!({"event":"packet_roundtrip"}));
                break;
            }
            Some(8) => {
                println!("{}", serde_json::json!({"event":"destination_unavailable"}));
                return Err("isolate-local route unavailable".into());
            }
            _ => {}
        }
    }
    socket.close().await?;
    Ok(())
}
