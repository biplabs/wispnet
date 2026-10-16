//! Pinned pre-approval hello over the same endpoint as control and services.
use crate::{
    control::{read_frame, write_frame},
    tunnel::{HANDSHAKE_TIMEOUT, TunnelError},
};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream},
};
use tokio::time::timeout;
use wipsnet_core::{
    identity::EndpointId,
    pairing::{self, Hello, Request},
};

pub(crate) async fn serve_stream(mut send: SendStream, mut recv: RecvStream, host: EndpointId) {
    let result = timeout(HANDSHAKE_TIMEOUT, async {
        let bytes = read_frame(&mut recv, pairing::MAX_REQUEST).await?;
        let Request::Hello {} = serde_json::from_slice(&bytes)
            .map_err(|_| TunnelError::Transport("invalid pairing request"))?;
        let bytes = serde_json::to_vec(&Hello {
            version: pairing::VERSION,
            host,
        })
        .map_err(|_| TunnelError::Transport("invalid pairing response"))?;
        write_frame(&mut send, &bytes).await?;
        let _ = send.stopped().await;
        Ok::<_, TunnelError>(())
    })
    .await;
    if !matches!(result, Ok(Ok(()))) {
        let _ = recv.stop(1u32.into());
        let _ = send.reset(1u32.into());
    }
}

/// Contains no invitation capability, policy metadata or owner credential.
pub async fn hello(endpoint: &Endpoint, target: EndpointAddr) -> Result<Hello, TunnelError> {
    let expected = target.id;
    let connection = timeout(HANDSHAKE_TIMEOUT, endpoint.connect(target, pairing::ALPN))
        .await
        .map_err(|_| crate::tunnel::connection_failure(endpoint, "pairing connection timeout"))?
        .map_err(|_| {
            crate::tunnel::connection_failure(endpoint, "pairing identity or connection failed")
        })?;
    hello_on(&connection, expected).await
}

struct Close(Connection);
impl Drop for Close {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"pairing hello complete");
    }
}

pub async fn hello_on(connection: &Connection, expected: EndpointId) -> Result<Hello, TunnelError> {
    if connection.remote_id() != expected || connection.alpn() != pairing::ALPN {
        return Err(TunnelError::Transport(
            "pairing target or protocol mismatch",
        ));
    }
    let connection = Close(connection.clone());
    timeout(HANDSHAKE_TIMEOUT, async {
        let (mut send, mut recv) = connection
            .0
            .open_bi()
            .await
            .map_err(|_| TunnelError::Transport("pairing stream failed"))?;
        let bytes = serde_json::to_vec(&Request::Hello {})
            .map_err(|_| TunnelError::Transport("invalid pairing request"))?;
        write_frame(&mut send, &bytes).await?;
        let bytes = read_frame(&mut recv, pairing::MAX_RESPONSE).await?;
        let hello: Hello = serde_json::from_slice(&bytes)
            .map_err(|_| TunnelError::Transport("invalid pairing response"))?;
        if hello.host != expected || hello.version != pairing::VERSION {
            return Err(TunnelError::Transport("pairing host or version mismatch"));
        }
        Ok(hello)
    })
    .await
    .map_err(|_| TunnelError::Transport("pairing request timeout"))?
}
