//! Target-local issuance, pending proofs and owner decisions. No global membership.
use crate::{
    control::{self, ControlError},
    host::Authority,
    store::Store,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Serialize;
use wispnet_core::{
    enrollment::{valid_code, valid_name},
    identity::EndpointId,
    pairing::{Challenge, Claim, Invitation, Pending, Proof, Purpose, Scope, VERSION},
};

type InvitationRow = (String, i64, String, Option<String>, Option<String>);

#[derive(Serialize)]
pub struct Entry {
    pub invitation: String,
    pub state: String,
    pub expires_at: u64,
    pub peer: Option<EndpointId>,
    pub name: Option<String>,
    pub mutual: bool,
}
impl Store {
    /// Owner-local consent to recognize the scanned host. Existing grants are preserved;
    /// a newly recognized host receives no grants.
    pub fn pairing_accept_host(
        &mut self,
        host: EndpointId,
        owner: &str,
        bootstrap: &wispnet_core::pairing::Bootstrap,
        name: &str,
    ) -> Result<(), ControlError> {
        self.pairing_accept_session(host, owner, bootstrap, name, None)
    }
    pub fn pairing_accept_session(
        &mut self,
        host: EndpointId,
        owner: &str,
        bootstrap: &wispnet_core::pairing::Bootstrap,
        name: &str,
        pairing: Option<&str>,
    ) -> Result<(), ControlError> {
        self.host_check(host, Some(owner))?;
        if pairing.is_some_and(|p| !valid_code(p)) {
            return Err(ControlError::Invalid);
        }
        if !bootstrap.valid(bootstrap.host) || bootstrap.host == host || !valid_name(name) {
            return Err(ControlError::Invalid);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let peer = bootstrap.host.to_string();
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM host_clients WHERE id!=?1",
            [&peer],
            |r| r.get(0),
        )?;
        if count >= 1024 {
            return Err(ControlError::RateLimited);
        }
        tx.execute(
            "INSERT INTO host_clients VALUES(?1,?2) ON CONFLICT(id) DO NOTHING",
            params![peer, name],
        )?;
        tx.execute("INSERT INTO host_pairings(peer,bootstrap,pairing) VALUES(?1,?2,coalesce(?3,'')) ON CONFLICT(peer) DO UPDATE SET bootstrap=excluded.bootstrap,pairing=coalesce(?3,host_pairings.pairing)",
            params![peer, serde_json::to_string(bootstrap).map_err(|_| ControlError::Invalid)?, pairing])?;
        tx.execute("DELETE FROM host_pairing_blocks WHERE peer=?1", [&peer])?;
        tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
        tx.commit()?;
        Ok(())
    }
    pub fn paired_hosts(
        &self,
        host: EndpointId,
    ) -> Result<Vec<wispnet_core::pairing::Bootstrap>, ControlError> {
        self.host_check(host, None)?;
        let mut stmt = self
            .0
            .prepare("SELECT peer,bootstrap FROM host_pairings ORDER BY peer")?;
        let mut hosts = Vec::new();
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (peer, text) = row?;
            let peer = peer.parse().map_err(|_| ControlError::Storage)?;
            let record: wispnet_core::pairing::Bootstrap =
                serde_json::from_str(&text).map_err(|_| ControlError::Storage)?;
            if !record.valid(peer) {
                return Err(ControlError::Storage);
            }
            hosts.push(record);
        }
        Ok(hosts)
    }
    pub fn pairing_forget_host(
        &mut self,
        host: EndpointId,
        owner: &str,
        peer: EndpointId,
    ) -> Result<(), ControlError> {
        self.host_check(host, Some(owner))?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Removing a mutual relationship also revokes all grants on this machine.
        if tx.execute("DELETE FROM host_clients WHERE id=?1 AND EXISTS(SELECT 1 FROM host_pairings WHERE peer=?1)", [peer.to_string()])? > 0 {
            tx.execute("UPDATE host_invitations SET state='revoked',challenge=NULL WHERE peer=?1 AND state IN ('issued','challenged','pending')", [peer.to_string()])?;
            tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn pairing_invite(
        &mut self,
        host: EndpointId,
        owner: &str,
        relay_url: &str,
        now: u64,
        ttl: u64,
    ) -> Result<Invitation, ControlError> {
        self.host_check(host, Some(owner))?;
        if !(30..=900).contains(&ttl) || now > i64::MAX as u64 - ttl {
            return Err(ControlError::Invalid);
        }
        let relay = wispnet_core::config::controller_origin(relay_url)
            .map_err(|_| ControlError::Invalid)?
            .to_string();
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire(&tx, now)?;
        tx.execute(
            "DELETE FROM host_invitations WHERE expires<=?1",
            [now as i64],
        )?;
        let count: i64 = tx.query_row("SELECT count(*) FROM host_invitations", [], |r| r.get(0))?;
        if count >= 128 {
            return Err(ControlError::RateLimited);
        }
        let invitation = Invitation {
            version: VERSION,
            host,
            host_name: None,
            relay_url: relay,
            invitation: control::token().to_string(),
            code: control::token().to_string(),
            scope: Scope::RequestAccess,
            expires_at: now + ttl,
            direct_hint: None,
        };
        tx.execute(
            "INSERT INTO host_invitations(id,hash,expires,state) VALUES(?1,?2,?3,'issued')",
            params![
                invitation.invitation,
                control::hash(&invitation.code),
                invitation.expires_at as i64
            ],
        )?;
        tx.commit()?;
        Ok(invitation)
    }
    pub fn pairing_list(
        &mut self,
        host: EndpointId,
        owner: &str,
        now: u64,
    ) -> Result<Vec<Entry>, ControlError> {
        self.host_check(host, Some(owner))?;
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire(&tx, now)?;
        let entries = {
            let mut stmt =
                tx.prepare("SELECT id,state,expires,peer,name FROM host_invitations ORDER BY id")?;
            let mut entries = Vec::new();
            for row in stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get(4)?,
                ))
            })? {
                let (invitation, state, expires, peer, name) = row?;
                let mutual = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM host_pairing_offers WHERE invitation=?1)",
                    [&invitation],
                    |r| r.get(0),
                )?;
                entries.push(Entry {
                    invitation,
                    state,
                    expires_at: u64::try_from(expires).map_err(|_| ControlError::Storage)?,
                    peer: peer
                        .map(|p| p.parse())
                        .transpose()
                        .map_err(|_| ControlError::Storage)?,
                    name,
                    mutual,
                });
            }
            entries
        };
        tx.commit()?;
        Ok(entries)
    }
    pub fn pairing_challenge(
        &mut self,
        host: EndpointId,
        peer: EndpointId,
        claim: &Claim,
        now: u64,
    ) -> Result<Challenge, ControlError> {
        self.host_check(host, None)?;
        if !valid_code(&claim.invitation) || !valid_code(&claim.code) || !valid_name(&claim.name) {
            return Err(ControlError::Invalid);
        }
        // Commit the attempt budget separately so later rejection cannot roll it back.
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire(&tx, now)?;
        let record: Option<InvitationRow> = tx
            .query_row(
                "SELECT hash,expires,state,peer,name FROM host_invitations WHERE id=?1",
                [&claim.invitation],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((digest, expires, state, stored_peer, name)) = record else {
            return Err(ControlError::Forbidden);
        };
        if digest != control::hash(&claim.code)
            || !matches!(state.as_str(), "issued" | "challenged")
            || stored_peer
                .as_deref()
                .is_some_and(|p| p != peer.to_string())
            || name.as_deref().is_some_and(|n| n != claim.name)
        {
            return Err(ControlError::Forbidden);
        }
        attempt(&tx, &claim.invitation, now)?;
        tx.commit()?;
        let challenge = Challenge {
            version: VERSION,
            host,
            invitation: claim.invitation.clone(),
            device: peer,
            name: claim.name.clone(),
            scope: Scope::RequestAccess,
            nonce: control::token().to_string(),
            purpose: Purpose::Request,
            expires_at: (expires as u64).min(now.saturating_add(60)),
        };
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Recheck state after releasing the attempt transaction (another local writer may act).
        let changed = tx.execute(
            "UPDATE host_invitations SET state='challenged',peer=?1,name=?2,challenge=?3
            WHERE id=?4 AND state IN ('issued','challenged') AND expires>?5
            AND (peer IS NULL OR peer=?1) AND (name IS NULL OR name=?2)",
            params![
                peer.to_string(),
                claim.name,
                serde_json::to_string(&challenge).map_err(|_| ControlError::Storage)?,
                claim.invitation,
                now as i64
            ],
        )?;
        if changed != 1 {
            return Err(ControlError::Conflict);
        }
        tx.commit()?;
        Ok(challenge)
    }
    pub fn pairing_prove(
        &mut self,
        host: EndpointId,
        peer: EndpointId,
        proof: &Proof,
        now: u64,
    ) -> Result<Pending, ControlError> {
        self.pairing_prove_with_bootstrap(host, peer, proof, None, now)
    }
    pub fn pairing_prove_with_bootstrap(
        &mut self,
        host: EndpointId,
        peer: EndpointId,
        proof: &Proof,
        bootstrap: Option<&wispnet_core::pairing::Bootstrap>,
        now: u64,
    ) -> Result<Pending, ControlError> {
        self.pairing_prove_session(host, peer, proof, bootstrap, now, false)
    }
    pub fn pairing_prove_session(
        &mut self,
        host: EndpointId,
        peer: EndpointId,
        proof: &Proof,
        bootstrap: Option<&wispnet_core::pairing::Bootstrap>,
        now: u64,
        session: bool,
    ) -> Result<Pending, ControlError> {
        self.host_check(host, None)?;
        if bootstrap.is_some_and(|b| !b.valid(peer) || peer == host) {
            return Err(ControlError::Invalid);
        }
        if !valid_code(&proof.challenge.invitation) || !proof.challenge.valid(now) {
            return Err(ControlError::Invalid);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        expire(&tx, now)?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT challenge FROM host_invitations
            WHERE id=?1 AND state='challenged' AND peer=?2",
                params![proof.challenge.invitation, peer.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let Some(stored) = stored else {
            return Err(ControlError::Forbidden);
        };
        attempt(&tx, &proof.challenge.invitation, now)?;
        tx.commit()?;
        let challenge: Challenge =
            serde_json::from_str(&stored).map_err(|_| ControlError::Storage)?;
        if challenge.signing_bytes() != proof.challenge.signing_bytes()
            || challenge.host != host
            || !challenge.verify(&proof.signature, peer, now)
        {
            return Err(ControlError::Forbidden);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE host_invitations SET state='pending',challenge=NULL
            WHERE id=?1 AND state='challenged' AND peer=?2 AND challenge=?3 AND expires>?4",
            params![challenge.invitation, peer.to_string(), stored, now as i64],
        )?;
        if changed != 1 {
            return Err(ControlError::Conflict);
        }
        if let Some(bootstrap) = bootstrap {
            tx.execute(
                "INSERT INTO host_pairing_offers(invitation,bootstrap,pairing) VALUES(?1,?2,?3)",
                params![
                    challenge.invitation,
                    serde_json::to_string(bootstrap).map_err(|_| ControlError::Invalid)?,
                    if session {
                        challenge.invitation.as_str()
                    } else {
                        ""
                    }
                ],
            )?;
        }
        tx.commit()?;
        Ok(Pending {
            host,
            peer,
            invitation: challenge.invitation,
        })
    }
}
fn expire(tx: &Transaction<'_>, now: u64) -> Result<(), ControlError> {
    if now > i64::MAX as u64 {
        return Err(ControlError::Invalid);
    }
    tx.execute(
        "UPDATE host_invitations SET state='expired',challenge=NULL
        WHERE expires<=?1 AND state IN ('issued','challenged','pending')",
        [now as i64],
    )?;
    Ok(())
}
fn attempt(tx: &Transaction<'_>, id: &str, now: u64) -> Result<(), ControlError> {
    let (window, attempts): (i64, i64) = tx.query_row(
        "SELECT window,attempts FROM host_invitations WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let reset = now >= (window as u64).saturating_add(60);
    if !reset && attempts >= 12 {
        return Err(ControlError::RateLimited);
    }
    tx.execute(
        "UPDATE host_invitations SET window=?1,attempts=?2 WHERE id=?3",
        params![
            if reset { now as i64 } else { window },
            if reset { 1 } else { attempts + 1 },
            id
        ],
    )?;
    Ok(())
}
pub(crate) fn approve(tx: &Transaction<'_>, id: &str, now: u64) -> Result<String, ControlError> {
    expire(tx, now)?;
    let pending: Option<(String, String)> = tx
        .query_row(
            "SELECT peer,name FROM host_invitations WHERE id=?1 AND state='pending'",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((peer, name)) = pending else {
        return Err(ControlError::Forbidden);
    };
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM host_clients WHERE id!=?1",
        [&peer],
        |r| r.get(0),
    )?;
    if count >= 1024 {
        return Err(ControlError::RateLimited);
    }
    tx.execute(
        "INSERT INTO host_clients VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET name=excluded.name",
        params![peer, name],
    )?;
    tx.execute(
        "INSERT INTO host_pairings(peer,bootstrap,pairing)
        SELECT ?1,bootstrap,pairing FROM host_pairing_offers WHERE invitation=?2
        ON CONFLICT(peer) DO UPDATE SET bootstrap=excluded.bootstrap,pairing=excluded.pairing",
        params![peer, id],
    )?;
    tx.execute("DELETE FROM host_pairing_blocks WHERE peer=?1", [&peer])?;
    tx.execute("DELETE FROM host_pairing_offers WHERE invitation=?1", [id])?;
    tx.execute(
        "UPDATE host_invitations SET state='spent',challenge=NULL WHERE id=?1",
        [id],
    )?;
    Ok(peer)
}
pub(crate) fn reject(tx: &Transaction<'_>, id: &str) -> Result<(), ControlError> {
    if !valid_code(id)
        || tx.execute(
            "UPDATE host_invitations SET state='rejected',challenge=NULL
        WHERE id=?1 AND state IN ('issued','challenged','pending')",
            [id],
        )? != 1
    {
        return Err(ControlError::Forbidden);
    }
    Ok(())
}
impl Authority {
    /// Display-only state for the authenticated requester; this grants no access.
    pub async fn pairing_pending(&self, peer: EndpointId) -> Result<bool, ControlError> {
        let store = self.store.clone().lock_owned().await;
        let host = self.host;
        tokio::task::spawn_blocking(move || {
            store.host_check(host, None)?;
            Ok(store.0.query_row(
                "SELECT EXISTS(SELECT 1 FROM host_invitations WHERE peer=?1 AND state='pending' AND expires>?2)",
                params![peer.to_string(), crate::enrollment::now() as i64],
                |row| row.get(0),
            )?)
        }).await.map_err(|_| ControlError::Storage)?
    }
    pub async fn paired_hosts(
        &self,
    ) -> Result<Vec<wispnet_core::pairing::Bootstrap>, ControlError> {
        let store = self.store.clone().lock_owned().await;
        let host = self.host;
        tokio::task::spawn_blocking(move || store.paired_hosts(host))
            .await
            .map_err(|_| ControlError::Storage)?
    }
    pub async fn pairing_challenge(
        &self,
        peer: EndpointId,
        claim: Claim,
    ) -> Result<Challenge, ControlError> {
        let mut store = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            self.store.clone().lock_owned(),
        )
        .await
        .map_err(|_| ControlError::RateLimited)?;
        let host = self.host;
        tokio::task::spawn_blocking(move || {
            store.pairing_challenge(host, peer, &claim, crate::enrollment::now())
        })
        .await
        .map_err(|_| ControlError::Storage)?
    }
    pub async fn pairing_prove(
        &self,
        peer: EndpointId,
        proof: Proof,
    ) -> Result<Pending, ControlError> {
        self.pairing_prove_mutual(peer, proof, None).await
    }
    pub async fn pairing_prove_mutual(
        &self,
        peer: EndpointId,
        proof: Proof,
        bootstrap: Option<wispnet_core::pairing::Bootstrap>,
    ) -> Result<Pending, ControlError> {
        self.pairing_prove_session(peer, proof, bootstrap, false)
            .await
    }
    pub async fn pairing_prove_session(
        &self,
        peer: EndpointId,
        proof: Proof,
        bootstrap: Option<wispnet_core::pairing::Bootstrap>,
        session: bool,
    ) -> Result<Pending, ControlError> {
        let mut store = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            self.store.clone().lock_owned(),
        )
        .await
        .map_err(|_| ControlError::RateLimited)?;
        let host = self.host;
        tokio::task::spawn_blocking(move || {
            store.pairing_prove_session(
                host,
                peer,
                &proof,
                bootstrap.as_ref(),
                crate::enrollment::now(),
                session,
            )
        })
        .await
        .map_err(|_| ControlError::Storage)?
    }
}
