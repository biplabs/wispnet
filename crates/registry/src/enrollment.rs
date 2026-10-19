//! Persistent invitation state machine. Authenticate owner operations at API boundary.
use crate::{
    control::{self, ControlError},
    store::Store,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use wispnet_core::{
    config::controller_origin,
    enrollment::{Challenge, Claim, Invitation, Proof, Purpose, valid_code, valid_name},
    identity::EndpointId,
};

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Admin {
    Invite {
        controller: String,
        ttl_seconds: u64,
    },
    Approve {
        invitation: String,
        device: EndpointId,
        name: String,
        expected_revision: u64,
    },
    Reject {
        invitation: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminReply {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invitation: Option<Invitation>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenge: Option<Challenge>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}
impl Drop for Outcome {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.token.zeroize();
    }
}
impl Outcome {
    fn state(state: &str) -> Self {
        Self {
            state: state.into(),
            challenge: None,
            revision: None,
            token: None,
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct InvitationRecord {
    pub invitation: String,
    pub state: String,
    pub device: Option<EndpointId>,
    pub name: Option<String>,
    pub expires_at: u64,
}
#[derive(Serialize, Deserialize)]
pub struct DeviceRecord {
    pub device: EndpointId,
    pub name: String,
    pub state: String,
    pub enrolled_at: u64,
    pub revoked_at: Option<u64>,
}
#[derive(Serialize, Deserialize)]
pub struct Listing {
    pub revision: u64,
    pub invitations: Vec<InvitationRecord>,
    pub devices: Vec<DeviceRecord>,
}
fn expire(tx: &Transaction<'_>, now: u64) -> Result<(), ControlError> {
    tx.execute("UPDATE invitations SET state='expired',nonce=NULL WHERE expires<=?1 AND state IN ('issued','pending','approved')",[now as i64])?;
    Ok(())
}
struct Record {
    origin: String,
    expires: u64,
    state: String,
    device: Option<String>,
    name: Option<String>,
    nonce: Option<String>,
    challenge_expires: Option<u64>,
    purpose: Option<String>,
}
fn record(tx: &Transaction<'_>, id: &str) -> Result<Record, ControlError> {
    tx.query_row("SELECT origin,expires,state,device,name,nonce,challenge_expires,purpose FROM invitations WHERE id=?1",[id],|r|Ok(Record{origin:r.get(0)?,expires:r.get::<_,i64>(1)? as u64,state:r.get(2)?,device:r.get(3)?,name:r.get(4)?,nonce:r.get(5)?,challenge_expires:r.get::<_,Option<i64>>(6)?.map(|i|i as u64),purpose:r.get(7)?})).map_err(|_|ControlError::Forbidden)
}
fn mesh(tx: &Transaction<'_>) -> Result<String, ControlError> {
    Ok(tx.query_row("SELECT mesh FROM controller WHERE id=1", [], |r| r.get(0))?)
}
// Persist per-invitation budgets even when the requested operation fails.
fn attempt(store: &mut Store, id: &str, code: &str, now: u64) -> Result<(), ControlError> {
    if !valid_code(id) || !control::valid_token(code) {
        return Err(ControlError::Forbidden);
    }
    let tx = store
        .0
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let counters: Option<(i64, i64)> = tx
        .query_row(
            "SELECT window,attempts FROM invitations WHERE id=?1 AND hash=?2",
            params![id, control::hash(code)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((window, count)) = counters else {
        return Err(ControlError::Forbidden);
    };
    if now < (window as u64).saturating_add(60) && count >= 12 {
        return Err(ControlError::RateLimited);
    }
    let (window, count) = if now >= (window as u64).saturating_add(60) {
        (now as i64, 1)
    } else {
        (window, count + 1)
    };
    tx.execute(
        "UPDATE invitations SET window=?2,attempts=?3 WHERE id=?1",
        params![id, window, count],
    )?;
    expire(&tx, now)?;
    tx.commit()?;
    Ok(())
}
impl Store {
    pub fn enrollment_admin(
        &mut self,
        operation: Admin,
        now: u64,
    ) -> Result<AdminReply, ControlError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire(&tx, now)?;
        let reply = match operation {
            Admin::Invite {
                controller,
                ttl_seconds,
            } => {
                if !(30..=900).contains(&ttl_seconds) || now > i64::MAX as u64 - 900 {
                    return Err(ControlError::Invalid);
                }
                let controller = controller_origin(&controller)
                    .map_err(|_| ControlError::Invalid)?
                    .to_string();
                let active:i64=tx.query_row("SELECT count(*) FROM invitations WHERE state IN ('issued','pending','approved')",[],|r|r.get(0))?;
                if active >= 128 {
                    return Err(ControlError::RateLimited);
                }
                // Retain at most 4096 recent invitations; device history is separate.
                tx.execute("DELETE FROM invitations WHERE id IN (SELECT id FROM invitations WHERE state NOT IN ('issued','pending','approved') ORDER BY rowid DESC LIMIT -1 OFFSET 3967)",[])?;
                let id = control::token();
                let code = control::token();
                let expires = now + ttl_seconds;
                tx.execute("INSERT INTO invitations(id,hash,origin,expires,state) VALUES(?1,?2,?3,?4,'issued')",params![&*id,control::hash(&code),controller,expires as i64])?;
                AdminReply {
                    state: "issued".into(),
                    invitation: Some(Invitation {
                        version: 1,
                        controller,
                        mesh: mesh(&tx)?,
                        invitation: id.to_string(),
                        code: code.to_string(),
                        expires_at: expires,
                    }),
                }
            }
            Admin::Approve {
                invitation,
                device,
                name,
                expected_revision,
            } => {
                let revision: i64 =
                    tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
                if expected_revision != revision as u64 {
                    return Err(ControlError::Conflict);
                }
                let row = record(&tx, &invitation)?;
                if row.state != "pending"
                    || row.device.as_deref() != Some(&device.to_string())
                    || row.name.as_deref() != Some(&name)
                {
                    return Err(ControlError::Forbidden);
                }
                tx.execute(
                    "UPDATE invitations SET state='approved',nonce=NULL WHERE id=?1",
                    [invitation],
                )?;
                AdminReply {
                    state: "approved".into(),
                    invitation: None,
                }
            }
            Admin::Reject { invitation } => {
                let row = record(&tx, &invitation)?;
                if !matches!(row.state.as_str(), "issued" | "pending" | "approved") {
                    return Err(ControlError::Forbidden);
                }
                tx.execute(
                    "UPDATE invitations SET state='rejected',nonce=NULL WHERE id=?1",
                    [invitation],
                )?;
                AdminReply {
                    state: "rejected".into(),
                    invitation: None,
                }
            }
        };
        tx.commit()?;
        Ok(reply)
    }
    pub fn enrollments(&mut self, now: u64) -> Result<Listing, ControlError> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire(&tx, now)?;
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        let mut invitations = Vec::new();
        let mut devices = Vec::new();
        {
            let mut stmt = tx.prepare(
                "SELECT id,state,device,name,expires FROM invitations ORDER BY rowid DESC",
            )?;
            for row in stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })? {
                let (id, state, device, name, expires) = row?;
                invitations.push(InvitationRecord {
                    invitation: id,
                    state,
                    device: device
                        .map(|s| s.parse().map_err(|_| ControlError::Storage))
                        .transpose()?,
                    name,
                    expires_at: expires as u64,
                });
            }
            let mut stmt=tx.prepare("SELECT device,name,state,enrolled_at,revoked_at FROM device_records ORDER BY device")?;
            for row in stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                ))
            })? {
                let (device, name, state, enrolled, revoked) = row?;
                devices.push(DeviceRecord {
                    device: device.parse().map_err(|_| ControlError::Storage)?,
                    name,
                    state,
                    enrolled_at: enrolled as u64,
                    revoked_at: revoked.map(|i| i as u64),
                });
            }
        }
        tx.commit()?;
        Ok(Listing {
            revision: revision as u64,
            invitations,
            devices,
        })
    }
    pub fn enrollment_challenge(
        &mut self,
        code: &str,
        claim: Claim,
        now: u64,
    ) -> Result<Outcome, ControlError> {
        attempt(self, &claim.invitation, code, now)?;
        if !valid_name(&claim.name) {
            return Err(ControlError::Invalid);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = record(&tx, &claim.invitation)?;
        if row
            .device
            .as_deref()
            .is_some_and(|d| d != claim.device.to_string())
            || row.name.as_ref().is_some_and(|n| n != &claim.name)
        {
            return Err(ControlError::Forbidden);
        }
        if row.state == "pending" {
            return Ok(Outcome::state("pending"));
        }
        if !matches!(row.state.as_str(), "issued" | "approved") {
            return Err(ControlError::Forbidden);
        }
        let purpose = if row.state == "approved" {
            Purpose::Complete
        } else {
            Purpose::Request
        };
        let nonce = control::token();
        let expires = (now + 120).min(row.expires);
        let challenge = Challenge {
            version: 1,
            controller: row.origin,
            mesh: mesh(&tx)?,
            invitation: claim.invitation.clone(),
            device: claim.device,
            name: claim.name.clone(),
            nonce: nonce.to_string(),
            expires_at: expires,
            purpose: purpose.clone(),
        };
        tx.execute("UPDATE invitations SET device=?2,name=?3,nonce=?4,challenge_expires=?5,purpose=?6 WHERE id=?1",params![claim.invitation,claim.device.to_string(),claim.name,&*nonce,expires as i64,match purpose{Purpose::Request=>"request",Purpose::Complete=>"complete"}])?;
        tx.commit()?;
        Ok(Outcome {
            state: row.state,
            challenge: Some(challenge),
            revision: None,
            token: None,
        })
    }
    pub fn enrollment_proof(
        &mut self,
        code: &str,
        proof: Proof,
        now: u64,
    ) -> Result<Outcome, ControlError> {
        attempt(self, &proof.invitation, code, now)?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = record(&tx, &proof.invitation)?;
        if !matches!(row.state.as_str(), "issued" | "approved")
            || row.device.as_deref() != Some(&proof.device.to_string())
            || row.name.as_deref() != Some(&proof.name)
            || row.nonce.as_deref() != Some(&proof.nonce)
            || row.challenge_expires.is_none_or(|e| e <= now)
        {
            return Err(ControlError::Forbidden);
        }
        let purpose = match (row.state.as_str(), row.purpose.as_deref()) {
            ("issued", Some("request")) => Purpose::Request,
            ("approved", Some("complete")) => Purpose::Complete,
            _ => return Err(ControlError::Forbidden),
        };
        let challenge = Challenge {
            version: 1,
            controller: row.origin,
            mesh: mesh(&tx)?,
            invitation: proof.invitation.clone(),
            device: proof.device,
            name: proof.name.clone(),
            nonce: proof.nonce,
            expires_at: row.challenge_expires.ok_or(ControlError::Forbidden)?,
            purpose: purpose.clone(),
        };
        if proof
            .device
            .verify(&challenge.signing_bytes(), &proof.signature)
            .is_err()
        {
            return Err(ControlError::Forbidden);
        }
        let outcome = match purpose {
            Purpose::Request => {
                tx.execute(
                    "UPDATE invitations SET state='pending',nonce=NULL WHERE id=?1",
                    [proof.invitation],
                )?;
                Outcome::state("pending")
            }
            Purpose::Complete => {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1)",
                    [proof.device.to_string()],
                    |r| r.get(0),
                )?;
                let count: i64 = tx.query_row("SELECT count(*) FROM devices", [], |r| r.get(0))?;
                if exists || count >= 1024 {
                    return Err(ControlError::Conflict);
                }
                let revision: i64 =
                    tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
                if revision == i64::MAX {
                    return Err(ControlError::Conflict);
                }
                let secret = control::token();
                tx.execute("INSERT INTO devices VALUES(?1)", [proof.device.to_string()])?;
                tx.execute(
                    "INSERT INTO device_tokens VALUES(?1,?2)",
                    params![proof.device.to_string(), control::hash(&secret)],
                )?;
                tx.execute("INSERT INTO device_records VALUES(?1,?2,'enrolled',?3,NULL) ON CONFLICT(device) DO UPDATE SET name=excluded.name,state='enrolled',enrolled_at=excluded.enrolled_at,revoked_at=NULL",params![proof.device.to_string(),proof.name,now as i64])?;
                tx.execute(
                    "UPDATE invitations SET state='enrolled',nonce=NULL WHERE id=?1",
                    [proof.invitation],
                )?;
                tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
                Outcome {
                    state: "enrolled".into(),
                    challenge: None,
                    revision: Some(revision as u64 + 1),
                    token: Some(secret.to_string()),
                }
            }
        };
        tx.commit()?;
        Ok(outcome)
    }
}
