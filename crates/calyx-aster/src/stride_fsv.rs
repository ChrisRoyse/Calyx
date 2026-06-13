//! STRIDE defense FSV proofs for PH61 T06.

use calyx_core::{CalyxError, Result};

/// Module-local fail-closed code for non-allowlisted external commands.
pub const CALYX_EXTERNAL_CMD_NOT_ALLOWED: &str = "CALYX_EXTERNAL_CMD_NOT_ALLOWED";

/// Minimal external-command allowlist gate for lens sandbox execution.
pub fn run_external_cmd(cmd: &str, allowlist: &[&str]) -> Result<()> {
    if allowlist.contains(&cmd) {
        Ok(())
    } else {
        Err(CalyxError {
            code: CALYX_EXTERNAL_CMD_NOT_ALLOWED,
            message: format!("external command {cmd:?} is not in the allowlist"),
            remediation: "register the command in the explicit lens sandbox allowlist before use",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::grant::AuditEvent;
    use crate::vault::{CALYX_QUOTA_EXCEEDED, QuotaConfig, QuotaGuard, VaultContext};
    use calyx_core::{
        AuthN, CALYX_AUTHN_REQUIRED, CalyxErrorCode, CxId, FixedClock, VaultId, no_anonymous_write,
    };
    use calyx_ledger::{
        ActorId, EntryKind, LedgerAppender, LedgerCfStore, LedgerEntry, MemoryLedgerStore,
        SubjectId, VerifyResult, decode, encode, verify_chain,
    };
    use ulid::Ulid;

    const T: u64 = 1_785_600_000_000;
    const T_NS: u64 = 1_000_000_000;

    #[test]
    fn stride_s_spoofing_anonymous_write_denied() {
        let err = no_anonymous_write(None).unwrap_err();
        println!("STRIDE_S_BEFORE authn=None");
        println!("STRIDE_S_AFTER anonymous_write=Err({})", err.code);
        assert_eq!(err.code, CALYX_AUTHN_REQUIRED);

        let authn = AuthN::InProcess {
            host_app_id: "stride-fsv-host".to_string(),
        };
        println!(
            "STRIDE_S_EDGE authn=InProcess result={:?}",
            no_anonymous_write(Some(&authn))
        );
        assert!(no_anonymous_write(Some(&authn)).is_ok());
        println!("[STRIDE S] anonymous_write = Err(CALYX_AUTHN_REQUIRED) ✓");
    }

    #[test]
    fn stride_t_tampering_ledger_chain_detected() {
        let mut store = ledger_store(5);
        let intact = verify_chain(&store, 0..5).unwrap();
        let (before, after, before_payload, after_payload) =
            flip_ledger_payload_byte(&mut store, 2);
        let tampered = verify_chain(&store, 0..5).unwrap();

        println!("STRIDE_T_BEFORE verify_chain={intact:?}");
        println!(
            "STRIDE_T_RAW_CF seq=2 before={} after={}",
            hex16(&before),
            hex16(&after)
        );
        println!(
            "STRIDE_T_PAYLOAD seq=2 before={} after={}",
            hex(&before_payload),
            hex(&after_payload)
        );
        println!("STRIDE_T_AFTER verify_chain={tampered:?}");
        assert_eq!(intact, VerifyResult::Intact { count: 5 });
        assert!(matches!(tampered, VerifyResult::Broken { at_seq: 2, .. }));
        assert_eq!(tampered.quarantine_seq(), Some(2));
        println!(
            "[STRIDE T] verify_chain after tamper = Err({} at seq=2) ✓",
            CalyxErrorCode::LedgerChainBroken.code()
        );
    }

    #[test]
    fn stride_r_repudiation_ledger_immutable() {
        let mut appender =
            LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(T)).unwrap();
        appender
            .append(
                EntryKind::Admin,
                SubjectId::Cx(CxId::from_bytes([0xA7; 16])),
                b"actor-stamped".to_vec(),
                ActorId::Agent("actor-a".to_string()),
            )
            .unwrap();
        let rows = appender.store().scan().unwrap();
        let stored = decode(&rows[0].bytes).unwrap();
        let mut store = appender.into_store();
        let overwrite = store.put_new(0, b"overwrite-attempt").unwrap_err();

        println!(
            "STRIDE_R_BEFORE actor={:?} rows={}",
            stored.actor,
            rows.len()
        );
        println!("STRIDE_R_AFTER overwrite=Err({})", overwrite.code);
        assert_eq!(stored.actor, ActorId::Agent("actor-a".to_string()));
        assert_eq!(overwrite.code, "CALYX_LEDGER_APPEND_ONLY_VIOLATION");
        println!(
            "[STRIDE R] ledger append-only overwrite = Err(CALYX_LEDGER_APPEND_ONLY_VIOLATION) ✓"
        );
    }

    #[test]
    fn stride_i_info_disclosure_cross_vault_denied() {
        let src = vault(0xA1);
        let dst = vault(0xB2);
        let ctx = VaultContext::new(
            src,
            b"stride-master-key-000000",
            QuotaConfig::default(),
            "tank/calyx",
        )
        .unwrap();
        let actor = ActorId::Agent("stride-agent".to_string());
        let before = ctx.grants().read().unwrap().audit_events(8);
        let err = ctx
            .check_cross_vault_read(dst, actor.clone(), T)
            .unwrap_err();
        let after = ctx.grants().read().unwrap().audit_events(8);

        println!("STRIDE_I_BEFORE audit_events={before:?}");
        println!(
            "STRIDE_I_AFTER cross_vault_read=Err({}) audit_events={after:?}",
            err.code
        );
        assert_eq!(err.code, "CALYX_VAULT_ACCESS_DENIED");
        assert!(after.iter().any(|event| {
            matches!(
                event,
                AuditEvent::Denied {
                    src_vault,
                    dst_vault,
                    actor: denied_actor,
                    at
                } if *src_vault == src && *dst_vault == dst && denied_actor == &actor && *at == T
            )
        }));
        println!(
            "[STRIDE I] cross_vault_read = Err(CALYX_VAULT_ACCESS_DENIED), audit_event = Denied ✓"
        );
    }

    #[test]
    fn stride_d_dos_quota_backpressure() {
        let guard = QuotaGuard::new(
            vault(0xD0),
            QuotaConfig {
                max_ingest_cx_per_sec: 100,
                ..QuotaConfig::default()
            },
        );
        let before = guard.counters();
        let err = guard.charge_ingest(101, T_NS).unwrap_err();
        let after = guard.counters();
        let reset = guard
            .charge_ingest(100, T_NS + crate::vault::quota::WINDOW_NS)
            .is_ok();

        println!("STRIDE_D_BEFORE counters={before:?}");
        println!(
            "STRIDE_D_AFTER charge_ingest(101)=Err({}) counters={after:?}",
            err.code
        );
        println!(
            "STRIDE_D_EDGE rollover_charge_100_ok={reset} counters={:?}",
            guard.counters()
        );
        assert_eq!(err.code, CALYX_QUOTA_EXCEEDED);
        assert_eq!(after.0, 101);
        assert!(reset);
        assert_eq!(guard.counters().0, 100);
        println!("[STRIDE D] charge_ingest(101) at T = Err(CALYX_QUOTA_EXCEEDED) ✓");
    }

    #[test]
    fn stride_e_elevation_no_external_cmd() {
        let allowed = run_external_cmd("calyx-readback", &["calyx-readback"]).is_ok();
        let err = run_external_cmd("rm -rf /", &["calyx-readback"]).unwrap_err();

        println!("STRIDE_E_BEFORE allowlist=[calyx-readback]");
        println!("STRIDE_E_EDGE allowed_cmd_ok={allowed}");
        println!("STRIDE_E_AFTER external_cmd=Err({})", err.code);
        assert!(allowed);
        assert_eq!(err.code, CALYX_EXTERNAL_CMD_NOT_ALLOWED);
        println!("[STRIDE E] external_cmd not allowlisted = Err(CALYX_EXTERNAL_CMD_NOT_ALLOWED) ✓");
    }

    fn ledger_store(count: u8) -> MemoryLedgerStore {
        let mut appender =
            LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(T)).unwrap();
        for seq in 0..count {
            appender
                .append(
                    EntryKind::Ingest,
                    SubjectId::Cx(CxId::from_bytes([seq; 16])),
                    format!("stride-payload-{seq}").into_bytes(),
                    ActorId::Service("stride-fsv".to_string()),
                )
                .unwrap();
        }
        appender.into_store()
    }

    fn flip_ledger_payload_byte(
        store: &mut MemoryLedgerStore,
        seq: u64,
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let row = store
            .scan()
            .unwrap()
            .into_iter()
            .find(|row| row.seq == seq)
            .unwrap();
        let before = row.bytes;
        let mut entry: LedgerEntry = decode(&before).unwrap();
        let before_payload = entry.payload.clone();
        entry.payload[0] ^= 1;
        let after_payload = entry.payload.clone();
        let after = encode(&entry);
        store.insert_raw(seq, after.clone());
        (before, after, before_payload, after_payload)
    }

    fn vault(byte: u8) -> VaultId {
        VaultId::from_ulid(Ulid::from_bytes([byte; 16]))
    }

    fn hex16(bytes: &[u8]) -> String {
        hex(&bytes[..bytes.len().min(16)])
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
