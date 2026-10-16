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

pub(crate) async fn serve_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    peer: EndpointId,
    authority: wipsnet_registry::host::Authority,
) {
    let result = timeout(HANDSHAKE_TIMEOUT, async {
        let bytes = zeroize::Zeroizing::new(read_frame(&mut recv, pairing::MAX_REQUEST).await?);
        let request: Request = serde_json::from_slice(&bytes)
            .map_err(|_| TunnelError::Transport("invalid pairing request"))?;
        let bytes = match request {
            Request::Hello {} => {
                if bytes.len() > 256 {
                    return Err(TunnelError::Transport("pairing hello too large"));
                }
                serde_json::to_vec(&Hello {
                    version: pairing::VERSION,
                    host: authority.host(),
                })
            }
            Request::Challenge { claim } => match authority.pairing_challenge(peer, claim).await {
                Ok(challenge) => serde_json::to_vec(&challenge),
                Err(_) => serde_json::to_vec(&serde_json::json!({"error":"denied"})),
            },
            Request::Prove { proof } => match authority.pairing_prove(peer, proof).await {
                Ok(pending) => serde_json::to_vec(&pending),
                Err(_) => serde_json::to_vec(&serde_json::json!({"error":"denied"})),
            },
        }
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
        let bytes = read_frame(&mut recv, 512).await?;
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

/// Validate untrusted invitation data against the independently expected host and
/// locally configured relay before binding/dialing. Never substitutes relay trust.
pub fn invitation_target(
    invitation: &wipsnet_core::pairing::Invitation,
    expected: EndpointId,
    config: &wipsnet_core::config::TransportConfig,
    now: u64,
) -> Result<EndpointAddr, TunnelError> {
    let bootstrap = invitation
        .bootstrap(expected, now)
        .ok_or(TunnelError::Transport("invalid or expired host invitation"))?;
    let relay = config
        .relay_url
        .as_deref()
        .and_then(|url| wipsnet_core::config::controller_origin(url).ok())
        .ok_or(TunnelError::Transport(
            "invitation requires a configured relay",
        ))?;
    if relay.as_str() != bootstrap.relay_url {
        return Err(TunnelError::Transport(
            "invitation relay does not match local configuration",
        ));
    }
    crate::tunnel::peer_target(expected, Some(config), bootstrap.direct_hint)
}

/// Capabilities are read only from a bounded private regular file, never stdin,
/// command arguments or diagnostics. Reject symlinks, hardlinks and special files.
pub fn read_invitation(
    path: &std::path::Path,
) -> Result<wipsnet_core::pairing::Invitation, TunnelError> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let denied = || TunnelError::Transport("cannot read protected host invitation");
    if !path.is_absolute()
        || path
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir))
    {
        return Err(denied());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| denied())?;
    let metadata = file.metadata().map_err(|_| denied())?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.len() > 8192
    {
        return Err(denied());
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| denied())?;
    if bytes.len() > 8192 {
        return Err(denied());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| TunnelError::Transport("invalid host invitation document"))
}

async fn rpc(
    endpoint: &Endpoint,
    target: EndpointAddr,
    request: &Request,
) -> Result<Vec<u8>, TunnelError> {
    let expected = target.id;
    let connection = timeout(HANDSHAKE_TIMEOUT, endpoint.connect(target, pairing::ALPN))
        .await
        .map_err(|_| crate::tunnel::connection_failure(endpoint, "pairing connection timeout"))?
        .map_err(|_| {
            crate::tunnel::connection_failure(endpoint, "pairing identity or connection failed")
        })?;
    let connection = Close(connection);
    if connection.0.remote_id() != expected || connection.0.alpn() != pairing::ALPN {
        return Err(TunnelError::Transport(
            "pairing target or protocol mismatch",
        ));
    }
    timeout(HANDSHAKE_TIMEOUT, async {
        let (mut send, mut recv) = connection
            .0
            .open_bi()
            .await
            .map_err(|_| TunnelError::Transport("pairing stream failed"))?;
        let bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(request)
                .map_err(|_| TunnelError::Transport("invalid pairing request"))?,
        );
        if bytes.len() > pairing::MAX_REQUEST {
            return Err(TunnelError::Transport("pairing request too large"));
        }
        write_frame(&mut send, &bytes).await?;
        let bytes = read_frame(&mut recv, pairing::MAX_RESPONSE).await?;
        if serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|v| v.get("error").and_then(|v| v.as_str()) == Some("denied"))
        {
            return Err(TunnelError::Transport("pairing request denied"));
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| TunnelError::Transport("pairing request timeout"))?
}

/// Explicit two-step proof request. Never retries or grants service access.
pub async fn request_enrollment(
    endpoint: &Endpoint,
    expected: EndpointId,
    invitation: &pairing::Invitation,
    config: &wipsnet_core::config::TransportConfig,
    key: &wipsnet_core::identity::SecretKey,
    name: &str,
) -> Result<pairing::Pending, TunnelError> {
    let now = wipsnet_registry::enrollment::now();
    let target = invitation_target(invitation, expected, config, now)?;
    if endpoint.id() != key.public() || !wipsnet_core::enrollment::valid_name(name) {
        return Err(TunnelError::Transport("invalid enrolling identity or name"));
    }
    let request = Request::Challenge {
        claim: pairing::Claim {
            invitation: invitation.invitation.clone(),
            code: invitation.code.clone(),
            name: name.to_owned(),
        },
    };
    let bytes = rpc(endpoint, target.clone(), &request).await?;
    let challenge: pairing::Challenge = serde_json::from_slice(&bytes)
        .map_err(|_| TunnelError::Transport("invalid pairing challenge"))?;
    if !challenge.matches(
        invitation,
        key.public(),
        name,
        pairing::Purpose::Request,
        wipsnet_registry::enrollment::now(),
    ) {
        return Err(TunnelError::Transport("pairing challenge mismatch"));
    }
    let signature = key.sign(&challenge.signing_bytes());
    let bytes = rpc(
        endpoint,
        target,
        &Request::Prove {
            proof: pairing::Proof {
                challenge,
                signature,
            },
        },
    )
    .await?;
    let pending: pairing::Pending = serde_json::from_slice(&bytes)
        .map_err(|_| TunnelError::Transport("invalid pairing pending response"))?;
    if pending.host != expected
        || pending.peer != key.public()
        || pending.invitation != invitation.invitation
    {
        return Err(TunnelError::Transport("pairing pending identity mismatch"));
    }
    Ok(pending)
}
