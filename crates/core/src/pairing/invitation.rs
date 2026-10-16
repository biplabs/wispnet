//! Host-scoped enrollment transcript; neither a display group nor a relay is authority.
use crate::{
    config::controller_origin,
    enrollment::{valid_code, valid_name},
    identity::EndpointId,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    RequestAccess,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Request,
    Complete,
}

/// Secret capability. Deliberately not Debug/Clone; never print or persist as a pin.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub version: u16,
    pub host: EndpointId,
    pub relay_url: String,
    pub invitation: String,
    pub code: String,
    pub scope: Scope,
    pub expires_at: u64,
    #[serde(default)]
    pub direct_hint: Option<SocketAddr>,
}
impl Drop for Invitation {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.code.zeroize();
    }
}
impl Invitation {
    pub fn valid(&self, now: u64) -> bool {
        self.version == super::VERSION
            && controller_origin(&self.relay_url).is_ok()
            && valid_code(&self.invitation)
            && valid_code(&self.code)
            && self.expires_at > now
            && self.expires_at <= now.saturating_add(900)
            && self.expires_at <= i64::MAX as u64
            && self.direct_hint.is_none_or(|hint| {
                hint.port() != 0
                    && !hint.ip().is_unspecified()
                    && !hint.ip().is_multicast()
                    && hint.ip() != std::net::Ipv4Addr::BROADCAST
                    && !matches!(hint, SocketAddr::V6(a) if a.scope_id() != 0 || a.flowinfo() != 0)
            })
    }
    /// Bootstrap/pin persistence must not retain the invitation ID or capability.
    pub fn bootstrap(&self, expected: EndpointId, now: u64) -> Option<Bootstrap> {
        if !self.valid(now) || self.host != expected {
            return None;
        }
        Some(Bootstrap {
            version: super::VERSION,
            host: self.host,
            relay_url: controller_origin(&self.relay_url).ok()?.to_string(),
            direct_hint: self.direct_hint,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bootstrap {
    pub version: u16,
    pub host: EndpointId,
    pub relay_url: String,
    pub direct_hint: Option<SocketAddr>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub version: u16,
    pub host: EndpointId,
    pub invitation: String,
    pub device: EndpointId,
    pub name: String,
    pub scope: Scope,
    pub nonce: String,
    pub purpose: Purpose,
    pub expires_at: u64,
}
impl Challenge {
    pub fn valid(&self, now: u64) -> bool {
        self.version == super::VERSION
            && valid_code(&self.invitation)
            && valid_code(&self.nonce)
            && valid_name(&self.name)
            && self.expires_at > now
            && self.expires_at <= now.saturating_add(60)
            && self.expires_at <= i64::MAX as u64
    }
    pub fn matches(
        &self,
        invitation: &Invitation,
        device: EndpointId,
        name: &str,
        purpose: Purpose,
        now: u64,
    ) -> bool {
        invitation.valid(now)
            && self.valid(now)
            && self.host == invitation.host
            && self.invitation == invitation.invitation
            && self.scope == invitation.scope
            && self.device == device
            && self.name == name
            && self.purpose == purpose
            && self.expires_at <= invitation.expires_at
    }
    /// Domain-separated canonical bytes; relay location does not confer trust.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut bytes = b"wipsnet/host-pairing/proof/1\0".to_vec();
        bytes.extend_from_slice(&self.version.to_be_bytes());
        bytes.extend_from_slice(self.host.as_bytes());
        bytes.extend_from_slice(self.device.as_bytes());
        for field in [&self.invitation, &self.name, &self.nonce] {
            bytes.extend_from_slice(&(field.len() as u32).to_be_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes.extend_from_slice(match self.scope {
            Scope::RequestAccess => b"request_access\0",
        });
        bytes.extend_from_slice(match self.purpose {
            Purpose::Request => b"request\0",
            Purpose::Complete => b"complete\0",
        });
        bytes.extend_from_slice(&self.expires_at.to_be_bytes());
        bytes
    }
    /// Does not consume a nonce or grant access: the target must do so atomically.
    pub fn verify(
        &self,
        signature: &iroh_base::Signature,
        authenticated_peer: EndpointId,
        now: u64,
    ) -> bool {
        self.valid(now)
            && self.device == authenticated_peer
            && authenticated_peer
                .verify(&self.signing_bytes(), signature)
                .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::SecretKey;
    fn invite(host: EndpointId) -> Invitation {
        Invitation {
            version: 1,
            host,
            relay_url: "https://relay.example".into(),
            invitation: "1".repeat(64),
            code: "2".repeat(64),
            scope: Scope::RequestAccess,
            expires_at: 200,
            direct_hint: None,
        }
    }
    #[test]
    fn invitations_bound_target_expiry_origin_hints_and_separate_secrets() {
        let host = SecretKey::generate().public();
        let mut invitation = invite(host);
        assert!(invitation.valid(100));
        invitation.version = 2;
        assert!(!invitation.valid(100));
        invitation.version = 1;
        assert!(
            invitation
                .bootstrap(SecretKey::generate().public(), 100)
                .is_none()
        );
        assert!(!invitation.valid(200));
        let bootstrap = invitation.bootstrap(host, 100).unwrap();
        let bytes = serde_json::to_string(&bootstrap).unwrap();
        assert!(!bytes.contains(&invitation.code));
        assert!(!bytes.contains(&invitation.invitation));
        assert_eq!(bootstrap.relay_url, "https://relay.example/");
        invitation.expires_at = 1001;
        assert!(!invitation.valid(100));
        invitation.expires_at = 200;
        for url in [
            "http://relay.example",
            "https://u:secret@relay.example",
            "https://relay.example/path",
            "https://relay.example?code=secret",
        ] {
            invitation.relay_url = url.into();
            assert!(!invitation.valid(100));
        }
        invitation.relay_url = "https://relay.example".into();
        for hint in [
            "0.0.0.0:1",
            "127.0.0.1:0",
            "255.255.255.255:1",
            "224.0.0.1:1",
            "[ff02::1]:1",
            "[fe80::1%2]:1",
        ] {
            invitation.direct_hint = Some(hint.parse().unwrap());
            assert!(!invitation.valid(100));
        }
        invitation.direct_hint = Some("127.0.0.1:1234".parse().unwrap());
        assert!(invitation.valid(100));
        let mut value = serde_json::to_value(&invitation).unwrap();
        value["scope"] = serde_json::json!("admin");
        assert!(serde_json::from_value::<Invitation>(value).is_err());
    }
    #[test]
    fn proofs_bind_every_authority_field_and_authenticated_enrolling_key() {
        let host = SecretKey::generate().public();
        let device = SecretKey::generate();
        let invitation = invite(host);
        let challenge = Challenge {
            version: 1,
            host,
            invitation: invitation.invitation.clone(),
            device: device.public(),
            name: "태블릿".into(),
            scope: Scope::RequestAccess,
            nonce: "3".repeat(64),
            purpose: Purpose::Request,
            expires_at: 150,
        };
        assert!(challenge.matches(
            &invitation,
            device.public(),
            "태블릿",
            Purpose::Request,
            100
        ));
        assert!(!challenge.matches(
            &invitation,
            device.public(),
            "태블릿",
            Purpose::Complete,
            100
        ));
        let signature = device.sign(&challenge.signing_bytes());
        assert!(challenge.verify(&signature, device.public(), 100));
        let legacy = crate::enrollment::Challenge {
            version: 1,
            controller: "https://relay.example/".into(),
            mesh: "test".into(),
            invitation: invitation.invitation.clone(),
            device: device.public(),
            name: "태블릿".into(),
            nonce: challenge.nonce.clone(),
            expires_at: 150,
            purpose: crate::enrollment::Purpose::Request,
        };
        assert!(!challenge.verify(&device.sign(&legacy.signing_bytes()), device.public(), 100));
        assert!(!challenge.verify(&signature, SecretKey::generate().public(), 100));
        assert!(!challenge.verify(&signature, device.public(), 150));
        for field in 0..9 {
            let mut altered = challenge.clone();
            match field {
                0 => altered.version = 2,
                1 => altered.host = SecretKey::generate().public(),
                2 => altered.device = SecretKey::generate().public(),
                3 => altered.invitation = "4".repeat(64),
                4 => altered.name = "different".into(),
                5 => altered.nonce = "5".repeat(64),
                6 => altered.purpose = Purpose::Complete,
                7 => altered.expires_at += 1,
                _ => altered.name = "태블릿\n".into(),
            }
            assert!(
                !altered.verify(&signature, device.public(), 100),
                "field {field}"
            );
        }
        let mut other_host = invite(SecretKey::generate().public());
        assert!(!challenge.matches(
            &other_host,
            device.public(),
            "태블릿",
            Purpose::Request,
            100
        ));
        other_host.host = host;
        other_host.expires_at = 140;
        assert!(!challenge.matches(
            &other_host,
            device.public(),
            "태블릿",
            Purpose::Request,
            100
        ));
        let mut long = challenge.clone();
        long.expires_at = 161;
        assert!(!long.valid(100));
    }
}
