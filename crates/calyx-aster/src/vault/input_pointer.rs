use super::{AsterVault, encode, ledger_hook};
use crate::cf::{ColumnFamily, base_key};
use crate::retained_input::validate_vault_input_pointer;
use calyx_core::{CalyxError, Clock, CxId, InputRef, LedgerRef, Result, VaultStore};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};

pub const CALYX_INPUT_POINTER_IDENTITY_MISMATCH: &str = "CALYX_INPUT_POINTER_IDENTITY_MISMATCH";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputPointerBackfill {
    AlreadyPresent { ledger_ref: LedgerRef },
    Backfilled { ledger_ref: LedgerRef },
}

impl InputPointerBackfill {
    pub fn changed(&self) -> bool {
        matches!(self, Self::Backfilled { .. })
    }

    pub fn ledger_ref(&self) -> &LedgerRef {
        match self {
            Self::AlreadyPresent { ledger_ref } | Self::Backfilled { ledger_ref } => ledger_ref,
        }
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Stamps an exact retained pointer onto a legacy hash-only Base row and Ledger atomically.
    pub fn backfill_input_pointer(
        &self,
        id: CxId,
        expected: &InputRef,
    ) -> Result<InputPointerBackfill> {
        if expected.redacted {
            return Err(identity_error(
                id,
                "incoming retained input is marked redacted",
            ));
        }
        let pointer = expected
            .pointer
            .as_deref()
            .ok_or_else(|| identity_error(id, "incoming retained input has no pointer"))?;
        validate_vault_input_pointer(pointer)?;
        self.with_durable_commit_lock(|| {
            let mut stored = self.get(id, self.snapshot())?;
            if stored.input_ref.hash != expected.hash {
                return Err(identity_error(
                    id,
                    "stored and incoming input hashes differ",
                ));
            }
            if stored.input_ref.redacted != expected.redacted {
                return Err(identity_error(
                    id,
                    "stored and incoming input redaction states differ",
                ));
            }
            match stored.input_ref.pointer.as_deref() {
                Some(existing) if existing == pointer => {
                    return Ok(InputPointerBackfill::AlreadyPresent {
                        ledger_ref: stored.provenance,
                    });
                }
                Some(_) => {
                    return Err(identity_error(
                        id,
                        "stored input pointer conflicts with the incoming canonical pointer",
                    ));
                }
                None => {}
            }

            let payload = pointer_backfill_payload(id, expected, pointer)?;
            let actor = ActorId::Service("calyx-aster".to_string());
            let subject = SubjectId::Cx(id);
            let mut rows = Vec::new();
            let mut hook_guard = match &self.ledger_hook {
                Some(hook) => Some(ledger_hook::lock_hook(hook)?),
                None => None,
            };
            let staged = if let Some(hook) = hook_guard.as_deref() {
                let staged = ledger_hook::stage_entry_payload(
                    hook,
                    &mut rows,
                    EntryKind::Migrate,
                    subject,
                    payload,
                    actor,
                )?;
                let ledger_ref = staged
                    .first()
                    .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
                    .ledger_ref();
                Some((staged, ledger_ref))
            } else {
                let ledger_ref = self.stage_raw_ledger_entry_locked(
                    &mut rows,
                    EntryKind::Migrate,
                    subject,
                    payload,
                    actor,
                )?;
                stored.input_ref.pointer = Some(pointer.to_string());
                stored.provenance = ledger_ref.clone();
                stored.validate_schema()?;
                rows.push(base_row(id, &stored)?);
                self.commit_rows_locked(&rows)?;
                return Ok(InputPointerBackfill::Backfilled { ledger_ref });
            };

            let (staged, ledger_ref) = staged.expect("hook branch returns staged rows");
            stored.input_ref.pointer = Some(pointer.to_string());
            stored.provenance = ledger_ref.clone();
            stored.validate_schema()?;
            rows.push(base_row(id, &stored)?);
            self.commit_rows_locked(&rows)?;
            if let Some(hook) = hook_guard.as_deref_mut() {
                ledger_hook::commit_staged(hook, &staged)?;
            }
            Ok(InputPointerBackfill::Backfilled { ledger_ref })
        })
    }
}

fn base_row(id: CxId, stored: &calyx_core::Constellation) -> Result<encode::WriteRow> {
    Ok(encode::WriteRow {
        cf: ColumnFamily::Base,
        key: base_key(id),
        value: encode::encode_constellation_base(stored)?,
    })
}

fn pointer_backfill_payload(id: CxId, input_ref: &InputRef, pointer: &str) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(&serde_json::json!({
        "mode": "retained-input-pointer-backfill",
        "cx_id": id.to_string(),
        "input_hash": hex(&input_ref.hash),
        "pointer_hash": hex(blake3::hash(pointer.as_bytes()).as_bytes()),
    }))
    .map_err(|error| {
        CalyxError::ledger_group_commit_failed(format!(
            "encode retained input pointer backfill payload: {error}"
        ))
    })?;
    RedactionPolicy::check_payload(&payload)?;
    Ok(payload)
}

fn identity_error(id: CxId, reason: &str) -> CalyxError {
    CalyxError {
        code: CALYX_INPUT_POINTER_IDENTITY_MISMATCH,
        message: format!("cannot backfill retained input pointer for {id}: {reason}"),
        remediation: "supply the exact authoritative bytes for this hash-only constellation; never replace a conflicting retained pointer",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use calyx_core::{Constellation, CxFlags, FixedClock, LedgerRef, Modality, VaultId};
    use calyx_ledger::decode;

    use super::*;
    use crate::cf::ledger_key;

    #[test]
    fn pointerless_base_and_migrate_ledger_commit_together() {
        let vault = vault();
        let (cx, expected) = pointerless_constellation(&vault, b"pointer backfill");
        let id = cx.cx_id;
        vault.put(cx).unwrap();

        let outcome = vault.backfill_input_pointer(id, &expected).unwrap();
        let stored = vault.get(id, vault.snapshot()).unwrap();
        let ledger_ref = outcome.ledger_ref();
        let ledger_bytes = vault
            .read_cf_at(
                vault.snapshot(),
                ColumnFamily::Ledger,
                &ledger_key(ledger_ref.seq),
            )
            .unwrap()
            .expect("migration ledger row");
        let ledger = decode(&ledger_bytes).unwrap();

        assert!(outcome.changed());
        assert_eq!(stored.input_ref, expected);
        assert_eq!(&stored.provenance, ledger_ref);
        assert_eq!(ledger.kind, EntryKind::Migrate);
        assert_eq!(ledger.subject, SubjectId::Cx(id));
    }

    #[test]
    fn matching_pointer_is_idempotent_without_new_commit() {
        let vault = vault();
        let (mut cx, expected) = pointerless_constellation(&vault, b"already retained");
        cx.input_ref = expected.clone();
        vault.put(cx).unwrap();
        let before = vault.snapshot();

        let outcome = vault
            .backfill_input_pointer(outcome_id(&vault, b"already retained"), &expected)
            .unwrap();

        assert!(!outcome.changed());
        assert_eq!(vault.snapshot(), before);
    }

    #[test]
    fn conflicting_pointer_and_hash_fail_without_mutation() {
        let vault = vault();
        let (mut cx, expected) = pointerless_constellation(&vault, b"identity conflict");
        cx.input_ref.pointer = Some("calyx-vault://inputs/original.bin".to_string());
        let id = cx.cx_id;
        vault.put(cx).unwrap();
        let before = vault.snapshot();

        let pointer_error = vault.backfill_input_pointer(id, &expected).unwrap_err();
        let mut wrong_hash = expected;
        wrong_hash.hash = [9; 32];
        let hash_error = vault.backfill_input_pointer(id, &wrong_hash).unwrap_err();

        assert_eq!(pointer_error.code, CALYX_INPUT_POINTER_IDENTITY_MISMATCH);
        assert_eq!(hash_error.code, CALYX_INPUT_POINTER_IDENTITY_MISMATCH);
        assert_eq!(vault.snapshot(), before);
    }

    fn vault() -> AsterVault<FixedClock> {
        AsterVault::with_clock(vault_id(), b"issue1423-salt".to_vec(), FixedClock::new(42))
    }

    fn pointerless_constellation(
        vault: &AsterVault<FixedClock>,
        bytes: &[u8],
    ) -> (Constellation, InputRef) {
        let hash = *blake3::hash(bytes).as_bytes();
        let expected = InputRef {
            hash,
            pointer: Some(crate::retained_input::canonical_text_pointer(&hash)),
            redacted: false,
        };
        let id = vault.cx_id_for_input(bytes, 1);
        (
            Constellation {
                cx_id: id,
                vault_id: vault_id(),
                panel_version: 1,
                created_at: 42,
                input_ref: InputRef {
                    hash,
                    pointer: None,
                    redacted: false,
                },
                modality: Modality::Text,
                slots: BTreeMap::new(),
                scalars: BTreeMap::new(),
                metadata: BTreeMap::new(),
                anchors: Vec::new(),
                provenance: LedgerRef {
                    seq: 1,
                    hash: [0; 32],
                },
                flags: CxFlags::default(),
            },
            expected,
        )
    }

    fn outcome_id(vault: &AsterVault<FixedClock>, bytes: &[u8]) -> CxId {
        vault.cx_id_for_input(bytes, 1)
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
    }
}
