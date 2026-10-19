//! Owner-local durable acknowledgements. No remote peer acquires admin rights.
use crate::{
    control::{self, ControlError},
    host::{Mutation, apply_mutation},
    store::Store,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use wispnet_core::identity::EndpointId;

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Approve { peer: EndpointId, name: String },
    Revoke { peer: EndpointId },
}
#[derive(Deserialize, Serialize, Debug, PartialEq)]
pub struct Receipt {
    pub state: String,
    pub revision: Option<i64>,
    pub reason: Option<String>,
}
impl Store {
    pub fn host_apply_operation(
        &mut self,
        host: EndpointId,
        secret: &str,
        id: &str,
        action: Action,
    ) -> Result<Receipt, ControlError> {
        self.host_check(host, Some(secret))?;
        if !control::valid_token(id) {
            return Err(ControlError::Invalid);
        }
        let payload =
            control::hash(&serde_json::to_string(&action).map_err(|_| ControlError::Invalid)?);
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = tx
            .query_row(
                "SELECT payload,state,revision,reason FROM host_operation_receipts WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        Receipt {
                            state: r.get(1)?,
                            revision: r.get(2)?,
                            reason: r.get(3)?,
                        },
                    ))
                },
            )
            .optional()?;
        if let Some((saved, receipt)) = previous {
            if saved != payload {
                return Err(ControlError::Conflict);
            }
            return Ok(receipt);
        }
        let count: i64 = tx.query_row("SELECT count(*) FROM host_operation_receipts", [], |r| {
            r.get(0)
        })?;
        // Never evict replay protection and then silently re-execute an old intent.
        if count >= 10000 {
            return Err(ControlError::RateLimited);
        }
        let revision: i64 =
            tx.query_row("SELECT value FROM revision WHERE id=1", [], |r| r.get(0))?;
        let operation = match action {
            Action::Approve { peer, name } => Mutation::Approve {
                expected_revision: revision as u64,
                peer,
                name,
            },
            Action::Revoke { peer } => Mutation::Revoke {
                expected_revision: revision as u64,
                peer,
            },
        };
        tx.execute_batch("SAVEPOINT intent;")?;
        let outcome = if revision == i64::MAX {
            Err(ControlError::Invalid)
        } else {
            apply_mutation(&tx, operation)
        };
        let receipt = match outcome {
            Ok(()) => {
                tx.execute("UPDATE revision SET value=value+1 WHERE id=1", [])?;
                Receipt {
                    state: "applied".into(),
                    revision: Some(revision + 1),
                    reason: None,
                }
            }
            Err(error) => {
                tx.execute_batch("ROLLBACK TO intent;")?;
                Receipt {
                    state: "rejected".into(),
                    revision: None,
                    reason: Some(error.to_string()),
                }
            }
        };
        tx.execute_batch("RELEASE intent;")?;
        tx.execute(
            "INSERT INTO host_operation_receipts VALUES(?1,?2,?3,?4,?5)",
            params![id, payload, receipt.state, receipt.revision, receipt.reason],
        )?;
        tx.commit()?;
        Ok(receipt)
    }
}
