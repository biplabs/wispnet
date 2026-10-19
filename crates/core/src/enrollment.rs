//! Portable invitation and proof contract shared by Linux and Android clients.
use crate::{config::controller_origin, identity::EndpointId, protocol::valid_service_id};
pub use iroh_base::Signature;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub version: u16,
    pub controller: String,
    pub mesh: String,
    pub invitation: String,
    pub code: String,
    pub expires_at: u64,
}
impl Drop for Invitation {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.code.zeroize();
    }
}
impl Invitation {
    pub fn valid(&self) -> bool {
        self.version == 1
            && controller_origin(&self.controller).is_ok()
            && valid_service_id(&self.mesh)
            && valid_code(&self.invitation)
            && valid_code(&self.code)
    }
}
pub fn valid_code(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Request,
    Complete,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub version: u16,
    pub controller: String,
    pub mesh: String,
    pub invitation: String,
    pub device: EndpointId,
    pub name: String,
    pub nonce: String,
    pub expires_at: u64,
    pub purpose: Purpose,
}
impl Challenge {
    /// Domain-separated, length-delimited canonical bytes. No secret key on wire.
    pub fn signing_bytes(&self) -> Vec<u8> {
        // Frozen signing domain for compatibility with existing client proofs.
        let mut bytes = b"wipsnet/enrollment/proof/1\0".to_vec();
        bytes.extend_from_slice(&self.version.to_be_bytes());
        for field in [
            &self.controller,
            &self.mesh,
            &self.invitation,
            &self.name,
            &self.nonce,
        ] {
            bytes.extend_from_slice(&(field.len() as u32).to_be_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes.extend_from_slice(self.device.as_bytes());
        bytes.extend_from_slice(&self.expires_at.to_be_bytes());
        bytes.extend_from_slice(match self.purpose {
            Purpose::Request => b"request",
            Purpose::Complete => b"complete",
        });
        bytes
    }
    pub fn matches(
        &self,
        invitation: &Invitation,
        device: EndpointId,
        name: &str,
        now: u64,
    ) -> bool {
        invitation.valid()
            && self.version == 1
            && self.controller == invitation.controller
            && self.mesh == invitation.mesh
            && self.invitation == invitation.invitation
            && self.device == device
            && self.name == name
            && valid_name(name)
            && valid_code(&self.nonce)
            && self.expires_at > now
            && self.expires_at <= invitation.expires_at
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    pub invitation: String,
    pub device: EndpointId,
    pub name: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proof {
    pub invitation: String,
    pub device: EndpointId,
    pub name: String,
    pub nonce: String,
    pub signature: Signature,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_and_capabilities_are_bounded() {
        assert!(valid_name("Linux laptop"));
        assert!(valid_name("태블릿"));
        for name in ["", "  ", "tablet\n", "\u{1b}[0m"] {
            assert!(!valid_name(name));
        }
        assert!(!valid_name(&"a".repeat(129)));
        assert!(valid_code(&"a".repeat(64)));
        assert!(!valid_code(&"A".repeat(64)));
        assert!(!valid_code(&"a".repeat(65)));
    }
    #[test]
    fn signing_transcript_separates_purpose_and_fields() {
        let key = crate::identity::SecretKey::generate();
        let mut challenge = Challenge {
            version: 1,
            controller: "https://controller.example/".into(),
            mesh: "test".into(),
            invitation: "1".repeat(64),
            device: key.public(),
            name: "laptop".into(),
            nonce: "2".repeat(64),
            expires_at: 120,
            purpose: Purpose::Request,
        };
        let signature = key.sign(&challenge.signing_bytes());
        assert!(
            key.public()
                .verify(&challenge.signing_bytes(), &signature)
                .is_ok()
        );
        challenge.purpose = Purpose::Complete;
        assert!(
            key.public()
                .verify(&challenge.signing_bytes(), &signature)
                .is_err()
        );
        challenge.purpose = Purpose::Request;
        challenge.name = "laptop2".into();
        assert!(
            key.public()
                .verify(&challenge.signing_bytes(), &signature)
                .is_err()
        );
    }
}
