//! Contact-driven removal. No outgoing notices, delivery queue or retry worker.
use crate::{
    control::{self, ControlError},
    host::Authority,
    store::Store,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use wispnet_core::{
    identity::EndpointId,
    pairing::{Bootstrap, PairingState, Status},
};
fn legacy(host: EndpointId, peer: EndpointId) -> String {
    let mut ids = [host.to_string(), peer.to_string()];
    ids.sort();
    control::hash(&format!("wispnet/legacy-pairing/1:{}:{}", ids[0], ids[1]))
}
fn session(
    tx: &Transaction<'_>,
    host: EndpointId,
    peer: EndpointId,
) -> Result<Option<String>, ControlError> {
    let active: Option<String> = tx
        .query_row(
            "SELECT pairing FROM host_pairings WHERE peer=?1",
            [peer.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(active) = active {
        return Ok(Some(if active.is_empty() {
            legacy(host, peer)
        } else {
            active
        }));
    }
    let known:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM host_clients WHERE id=?1 UNION ALL SELECT 1 FROM host_legacy_pins WHERE peer=?1)",[peer.to_string()],|r|r.get(0))?;
    Ok(known.then(|| legacy(host, peer)))
}
fn remove(tx: &Transaction<'_>, host: EndpointId, peer: EndpointId) -> Result<(), ControlError> {
    let pairing = session(tx, host, peer)?.unwrap_or_else(|| legacy(host, peer));
    tx.execute("DELETE FROM host_clients WHERE id=?1", [peer.to_string()])?;
    tx.execute(
        "DELETE FROM host_legacy_pins WHERE peer=?1",
        [peer.to_string()],
    )?;
    tx.execute(
        "INSERT INTO host_pairing_blocks(peer,pairing) VALUES(?1,?2) ON CONFLICT(peer) DO UPDATE SET pairing=excluded.pairing",
        params![peer.to_string(),pairing],
    )?;
    tx.execute("UPDATE host_invitations SET state='revoked',challenge=NULL WHERE peer=?1 AND state IN ('issued','challenged','pending')",[peer.to_string()])?;
    tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
    Ok(())
}
impl Store {
    /// Index verified legacy file pins, without adding clients or widening grants.
    pub fn index_legacy_pin(
        &mut self,
        host: EndpointId,
        owner: &str,
        peer: EndpointId,
    ) -> Result<(), ControlError> {
        self.host_check(host, Some(owner))?;
        self.0.execute("INSERT OR IGNORE INTO host_legacy_pins SELECT ?1 WHERE NOT EXISTS(SELECT 1 FROM host_pairing_blocks WHERE peer=?1)",[peer.to_string()])?;
        Ok(())
    }
    pub fn delete_pairing(
        &mut self,
        host: EndpointId,
        owner: &str,
        peer: EndpointId,
        bootstrap: Option<&Bootstrap>,
        expected: Option<u64>,
    ) -> Result<(), ControlError> {
        self.host_check(host, Some(owner))?;
        if host == peer || bootstrap.is_some_and(|b| !b.valid(peer)) {
            return Err(ControlError::Invalid);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        if expected.is_some_and(|e| e != revision as u64) {
            return Err(ControlError::Conflict);
        }
        if session(&tx, host, peer)?.is_none() && bootstrap.is_none() {
            return Ok(());
        }
        remove(&tx, host, peer)?;
        tx.commit()?;
        Ok(())
    }
    pub fn pairing_blocked(
        &self,
        host: EndpointId,
        peer: EndpointId,
    ) -> Result<bool, ControlError> {
        self.host_check(host, None)?;
        Ok(self.0.query_row(
            "SELECT EXISTS(SELECT 1 FROM host_pairing_blocks WHERE peer=?1)",
            [peer.to_string()],
            |r| r.get(0),
        )?)
    }
    pub fn pairing_session(
        &mut self,
        host: EndpointId,
        peer: EndpointId,
    ) -> Result<Option<String>, ControlError> {
        self.host_check(host, None)?;
        let tx = self.0.transaction()?;
        let current = session(&tx, host, peer)?;
        tx.commit()?;
        Ok(current)
    }
    /// Read-only status for the transport-authenticated requester. Empty grants and
    /// unknown identities are not removals. A pending new request takes precedence.
    pub fn pairing_status(
        &mut self,
        host: EndpointId,
        peer: EndpointId,
        pairing: &str,
    ) -> Result<Status, ControlError> {
        self.host_check(host, None)?;
        if host == peer || !control::valid_token(pairing) {
            return Err(ControlError::Invalid);
        }
        let tx = self.0.transaction()?;
        let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM host_invitations WHERE id=?1 AND peer=?2 AND state IN ('challenged','pending') AND expires>?3)",params![pairing,peer.to_string(),crate::enrollment::now() as i64],|r|r.get(0))?;
        let current = session(&tx, host, peer)?;
        let blocked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM host_pairing_blocks WHERE peer=?1 AND pairing=?2)",
            params![peer.to_string(), pairing],
            |r| r.get(0),
        )?;
        let unapproved:bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM host_invitations WHERE id=?1 AND peer=?2 AND state IN ('issued','challenged','pending','expired'))",params![pairing,peer.to_string()],|r|r.get(0))?;
        let state = if pending {
            PairingState::Pending
        } else if unapproved {
            PairingState::Unknown
        } else if current.as_deref() == Some(pairing) {
            PairingState::Paired
        } else if current.is_some() || blocked {
            PairingState::Removed
        } else {
            PairingState::Unknown
        };
        tx.commit()?;
        Ok(Status {
            host,
            peer,
            pairing: pairing.to_owned(),
            state,
        })
    }
    /// Apply an authenticated removal response only to the session we queried.
    /// A concurrent fresh pairing makes a delayed response harmless.
    pub fn apply_remote_removal(
        &mut self,
        host: EndpointId,
        owner: &str,
        peer: EndpointId,
        pairing: &str,
    ) -> Result<bool, ControlError> {
        self.host_check(host, Some(owner))?;
        if host == peer || !control::valid_token(pairing) {
            return Err(ControlError::Invalid);
        }
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if session(&tx, host, peer)?.as_deref() != Some(pairing) {
            return Ok(false);
        }
        remove(&tx, host, peer)?;
        tx.commit()?;
        Ok(true)
    }
}
impl Authority {
    pub async fn pairing_blocked(&self, peer: EndpointId) -> Result<bool, ControlError> {
        let store = self.store.clone().lock_owned().await;
        let host = self.host;
        tokio::task::spawn_blocking(move || store.pairing_blocked(host, peer))
            .await
            .map_err(|_| ControlError::Storage)?
    }
    pub async fn pairing_session(&self, peer: EndpointId) -> Result<Option<String>, ControlError> {
        let mut store = self.store.clone().lock_owned().await;
        let host = self.host;
        tokio::task::spawn_blocking(move || store.pairing_session(host, peer))
            .await
            .map_err(|_| ControlError::Storage)?
    }
    pub async fn pairing_status(
        &self,
        peer: EndpointId,
        pairing: String,
    ) -> Result<Status, ControlError> {
        let mut store = self.store.clone().lock_owned().await;
        let host = self.host;
        tokio::task::spawn_blocking(move || store.pairing_status(host, peer, &pairing))
            .await
            .map_err(|_| ControlError::Storage)?
    }
}
