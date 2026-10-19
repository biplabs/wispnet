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
use wispnet_core::{
    identity::EndpointId,
    pairing::{self, Hello, Request},
};

pub(crate) async fn serve_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    peer: EndpointId,
    authority: wispnet_registry::host::Authority,
) {
    let result = timeout(HANDSHAKE_TIMEOUT, async {
        let bytes = zeroize::Zeroizing::new(read_frame(&mut recv, pairing::MAX_REQUEST).await?);
        let request: Request = serde_json::from_slice(&bytes)
            .map_err(|_| TunnelError::Transport("invalid pairing request"))?;
        let bytes = match request {
            Request::Status { pairing } => match authority.pairing_status(peer, pairing).await {
                Ok(ack) => serde_json::to_vec(&ack),
                Err(_) => serde_json::to_vec(&serde_json::json!({"error":"denied"})),
            },
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
            Request::ProveMutualSession { proof, bootstrap } => match authority
                .pairing_prove_session(peer, proof, Some(bootstrap), true)
                .await
            {
                Ok(pending) => serde_json::to_vec(&pending),
                Err(_) => serde_json::to_vec(&serde_json::json!({"error":"denied"})),
            },
            Request::ProveMutual { proof, bootstrap } => match authority
                .pairing_prove_mutual(peer, proof, Some(bootstrap))
                .await
            {
                Ok(pending) => serde_json::to_vec(&pending),
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
    invitation: &wispnet_core::pairing::Invitation,
    expected: EndpointId,
    config: &wispnet_core::config::TransportConfig,
    now: u64,
) -> Result<EndpointAddr, TunnelError> {
    let bootstrap = invitation
        .bootstrap(expected, now)
        .ok_or(TunnelError::Transport("invalid or expired host invitation"))?;
    let relay = config
        .relay_url
        .as_deref()
        .and_then(|url| wispnet_core::config::controller_origin(url).ok())
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
) -> Result<wispnet_core::pairing::Invitation, TunnelError> {
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
    decode_invitation(&bytes)
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

/// A denial/timeout is never interpreted as deletion. Only a pinned, explicit status is.
pub async fn pairing_status(
    endpoint: &Endpoint,
    target: EndpointAddr,
    pairing: &str,
) -> Result<pairing::Status, TunnelError> {
    let expected = target.id;
    let bytes = rpc(
        endpoint,
        target,
        &Request::Status {
            pairing: pairing.to_owned(),
        },
    )
    .await?;
    let status: pairing::Status = serde_json::from_slice(&bytes)
        .map_err(|_| TunnelError::Transport("invalid pairing status"))?;
    if status.host != expected || status.peer != endpoint.id() || status.pairing != pairing {
        return Err(TunnelError::Transport("pairing status identity mismatch"));
    }
    Ok(status)
}

/// Explicit two-step proof request. Never retries or grants service access.
pub async fn request_enrollment(
    endpoint: &Endpoint,
    expected: EndpointId,
    invitation: &pairing::Invitation,
    config: &wispnet_core::config::TransportConfig,
    key: &wispnet_core::identity::SecretKey,
    name: &str,
) -> Result<pairing::Pending, TunnelError> {
    request_enrollment_with_bootstrap(endpoint, expected, invitation, config, key, name, None).await
}

pub async fn request_enrollment_with_bootstrap(
    endpoint: &Endpoint,
    expected: EndpointId,
    invitation: &pairing::Invitation,
    config: &wispnet_core::config::TransportConfig,
    key: &wispnet_core::identity::SecretKey,
    name: &str,
    bootstrap: Option<pairing::Bootstrap>,
) -> Result<pairing::Pending, TunnelError> {
    let now = wispnet_registry::enrollment::now();
    let target = invitation_target(invitation, expected, config, now)?;
    if endpoint.id() != key.public() || !wispnet_core::enrollment::valid_name(name) {
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
        wispnet_registry::enrollment::now(),
    ) {
        return Err(TunnelError::Transport("pairing challenge mismatch"));
    }
    let signature = key.sign(&challenge.signing_bytes());
    let proof = pairing::Proof {
        challenge,
        signature,
    };
    // The reverse bootstrap travels on the same identity-authenticated channel as
    // the single-use proof. It is stored pending, and activated only by the owner.
    let request = match bootstrap {
        Some(bootstrap) => Request::ProveMutualSession { proof, bootstrap },
        None => Request::Prove { proof },
    };
    let bytes = rpc(endpoint, target, &request).await?;
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

/// Camera-free transport of the same target-bound contract. No URL is fetched.
pub fn decode_invitation(bytes: &[u8]) -> Result<wispnet_core::pairing::Invitation, TunnelError> {
    use base64::Engine;
    let denied = || TunnelError::Transport("invalid host invitation document");
    if bytes.len() > 8192 {
        return Err(denied());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| denied())?.trim();
    let decoded = zeroize::Zeroizing::new(if let Some(code) = text.strip_prefix("wispnet1:") {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(code)
            .map_err(|_| denied())?
    } else {
        text.as_bytes().to_vec()
    });
    serde_json::from_slice(&decoded).map_err(|_| denied())
}
pub fn encode_invitation(
    invitation: &wispnet_core::pairing::Invitation,
) -> Result<zeroize::Zeroizing<String>, serde_json::Error> {
    use base64::Engine;
    let bytes = zeroize::Zeroizing::new(serde_json::to_vec(invitation)?);
    Ok(zeroize::Zeroizing::new(format!(
        "wispnet1:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
    )))
}
#[cfg(test)]
mod code_tests {
    use super::*;
    #[test]
    fn code_and_json_share_strict_target_contract_and_bounds() {
        let text = serde_json::json!({"version":1,"host":wispnet_core::identity::SecretKey::generate().public(),"relay_url":"https://relay.example","invitation":"1".repeat(64),"code":"2".repeat(64),"scope":"request_access","expires_at":200});
        let json = serde_json::to_vec(&text).unwrap();
        let invitation = decode_invitation(&json).unwrap();
        let code = encode_invitation(&invitation).unwrap();
        let decoded = decode_invitation(code.as_bytes()).unwrap();
        assert_eq!(decoded.host, invitation.host);
        assert_eq!(decoded.code, invitation.code);
        assert!(decoded.bootstrap(decoded.host, 100).is_some());
        assert!(decoded.bootstrap(decoded.host, 200).is_none());
        assert!(
            decoded
                .bootstrap(wispnet_core::identity::SecretKey::generate().public(), 100)
                .is_none()
        );
        for invalid in ["wispnet2:abc", "wispnet1:!!", &"x".repeat(8193)] {
            assert!(decode_invitation(invalid.as_bytes()).is_err());
        }
        let mut invalid = text;
        invalid["admin"] = serde_json::json!(true);
        assert!(decode_invitation(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
}
