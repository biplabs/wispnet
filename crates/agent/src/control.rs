//! Bounded unary catalog RPC. No remote owner API, backend address, or caller claim.
use crate::tunnel::{HANDSHAKE_TIMEOUT, TunnelError};
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream},
};
use std::net::SocketAddr;
use tokio::time::timeout;
use wipsnet_core::{
    host::{self, Catalog, Request},
    identity::EndpointId,
};

pub(crate) async fn read_frame(
    recv: &mut RecvStream,
    limit: usize,
) -> Result<Vec<u8>, TunnelError> {
    let mut length = [0; 4];
    recv.read_exact(&mut length)
        .await
        .map_err(|_| TunnelError::Transport("control frame truncated"))?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > limit {
        return Err(TunnelError::Transport("control frame too large"));
    }
    let mut bytes = zeroize::Zeroizing::new(vec![0; length]);
    recv.read_exact(&mut bytes)
        .await
        .map_err(|_| TunnelError::Transport("control frame truncated"))?;
    let mut extra = [0];
    if recv
        .read(&mut extra)
        .await
        .map_err(|_| TunnelError::Transport("control stream failed"))?
        .is_some()
    {
        return Err(TunnelError::Transport("control trailing data"));
    }
    Ok(std::mem::take(&mut *bytes))
}
pub(crate) async fn write_frame(send: &mut SendStream, bytes: &[u8]) -> Result<(), TunnelError> {
    send.write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .map_err(|_| TunnelError::Transport("control stream failed"))?;
    send.write_all(bytes)
        .await
        .map_err(|_| TunnelError::Transport("control stream failed"))?;
    send.finish()
        .map_err(|_| TunnelError::Transport("control stream failed"))?;
    Ok(())
}
pub(crate) async fn serve_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    peer: EndpointId,
    authority: wipsnet_registry::host::Authority,
    counts: crate::tunnel::ActiveCounts,
) {
    let outcome = timeout(HANDSHAKE_TIMEOUT, async {
        let request = read_frame(&mut recv, host::MAX_REQUEST).await?;
        let Request::Catalog {} = serde_json::from_slice(&request)
            .map_err(|_| TunnelError::Transport("invalid control request"))?;
        let catalog = match authority.catalog(peer).await {
            Ok(catalog) => catalog,
            Err(wipsnet_registry::control::ControlError::Forbidden) => {
                write_frame(&mut send, br#"{"error":"access_denied"}"#).await?;
                let _ = send.stopped().await;
                return Ok(());
            }
            Err(_) => return Err(TunnelError::Transport("control unavailable")),
        };
        counts.record_activity(peer);
        let bytes =
            serde_json::to_vec(&catalog).map_err(|_| TunnelError::Transport("invalid catalog"))?;
        if bytes.len() > host::MAX_RESPONSE {
            return Err(TunnelError::Transport("catalog too large"));
        }
        write_frame(&mut send, &bytes).await?;
        let _ = send.stopped().await;
        Ok::<_, TunnelError>(())
    })
    .await;
    if !matches!(outcome, Ok(Ok(()))) {
        let _ = recv.stop(1u32.into());
        let _ = send.reset(1u32.into());
    }
}
pub async fn connect(
    endpoint: &Endpoint,
    expected: EndpointId,
    address: SocketAddr,
) -> Result<Connection, TunnelError> {
    connect_target(endpoint, EndpointAddr::new(expected).with_ip_addr(address)).await
}
pub async fn connect_target(
    endpoint: &Endpoint,
    target: EndpointAddr,
) -> Result<Connection, TunnelError> {
    timeout(HANDSHAKE_TIMEOUT, endpoint.connect(target, host::ALPN))
        .await
        .map_err(|_| crate::tunnel::connection_failure(endpoint, "control connection timeout"))?
        .map_err(|_| {
            crate::tunnel::connection_failure(endpoint, "control identity or connection failed")
        })
}
pub async fn catalog_on(
    connection: &Connection,
    expected: EndpointId,
) -> Result<Catalog, TunnelError> {
    // A reused connection must still match this operation's saved host pin.
    // Check before opening a stream or disclosing even the request type.
    if connection.remote_id() != expected {
        return Err(TunnelError::Transport("control target identity mismatch"));
    }
    let result = timeout(HANDSHAKE_TIMEOUT, async {
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|_| TunnelError::Transport("control stream failed"))?;
        let request = serde_json::to_vec(&Request::Catalog {})
            .map_err(|_| TunnelError::Transport("invalid control request"))?;
        write_frame(&mut send, &request).await?;
        let bytes = read_frame(&mut recv, host::MAX_RESPONSE).await?;
        if bytes == br#"{"error":"access_denied"}"# {
            return Err(TunnelError::Status(wipsnet_core::protocol::Status::Denied));
        }
        let catalog: Catalog = serde_json::from_slice(&bytes)
            .map_err(|_| TunnelError::Transport("invalid catalog"))?;
        if catalog.host != expected
            || connection.remote_id() != expected
            || catalog.revision > i64::MAX as u64
            || catalog.services.len() > 128
            || catalog
                .services
                .iter()
                .map(|s| &s.id)
                .collect::<std::collections::HashSet<_>>()
                .len()
                != catalog.services.len()
            || !wipsnet_core::enrollment::valid_code(&catalog.generation)
            || catalog.services.iter().any(|s| {
                !wipsnet_core::protocol::valid_service_id(&s.id)
                    || !wipsnet_core::enrollment::valid_code(&s.incarnation)
            })
        {
            return Err(TunnelError::Transport("catalog host or schema mismatch"));
        }
        Ok(catalog)
    })
    .await
    .map_err(|_| TunnelError::Transport("control request timeout"))?;
    if result.is_err()
        && matches!(connection.close_reason(), Some(iroh::endpoint::ConnectionError::ApplicationClosed(ref close)) if close.error_code == (wipsnet_core::protocol::Status::Denied as u32).into())
    {
        return Err(TunnelError::Status(wipsnet_core::protocol::Status::Denied));
    }
    result
}
