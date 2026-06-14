use super::{AsterVault, encode};
use crate::timetravel::RetentionHorizon;
use calyx_core::{CalyxError, Clock, Result};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use serde_json::json;

const RETENTION_HORIZON_SUBJECT: &[u8] = b"timetravel_retention_horizon";

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn retention_horizon(&self) -> RetentionHorizon {
        self.retention_horizon
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn set_retention_horizon(&self, horizon: RetentionHorizon) -> Result<()> {
        horizon.validate()?;
        self.with_durable_commit_lock(|| {
            let old = self.retention_horizon();
            if old == horizon {
                return Ok(());
            }
            if let Some(durable) = &self.durable {
                durable.write_retention_horizon_manifest(&horizon)?;
            }
            if let Err(error) = self.commit_retention_horizon_ledger(&old, &horizon) {
                if let Some(durable) = &self.durable
                    && let Err(rollback) = durable.write_retention_horizon_manifest(&old)
                {
                    eprintln!("calyx retention horizon manifest rollback failed: {rollback}");
                }
                return Err(error);
            }
            self.replace_retention_horizon(horizon.clone())
        })
    }

    pub(crate) fn replace_retention_horizon(&self, horizon: RetentionHorizon) -> Result<()> {
        *self
            .retention_horizon
            .lock()
            .map_err(|_| retention_lock_error())? = horizon;
        Ok(())
    }

    fn commit_retention_horizon_ledger(
        &self,
        old: &RetentionHorizon,
        new: &RetentionHorizon,
    ) -> Result<()> {
        let payload = serde_json::to_vec(&json!({
            "event": "RETENTION_HORIZON_CHANGED",
            "old": old,
            "new": new,
            "changed_at_millis": self.clock_now(),
        }))
        .map_err(|error| {
            CalyxError::aster_corrupt_shard(format!("encode retention horizon ledger: {error}"))
        })?;
        self.commit_rows_with_ledger_entry_locked(
            Vec::<encode::WriteRow>::new(),
            EntryKind::Admin,
            SubjectId::Guard(RETENTION_HORIZON_SUBJECT.to_vec()),
            payload,
            ActorId::System,
        )?;
        Ok(())
    }
}

fn retention_lock_error() -> CalyxError {
    CalyxError::backpressure("retention horizon lock poisoned")
}
