# PH67 · T03 — `calyx verify-restore` byte-verification tool

| Field | Value |
|---|---|
| **Phase** | PH67 — restic backup + DR drill |
| **Stage** | S16 — Server & Deployment |
| **Crate** | `calyxd` |
| **Files** | `crates/calyxd/src/verify.rs` (≤500), `crates/calyx-cli/src/main.rs` (add subcommand) |
| **Depends on** | PH35 T01 (Ledger chain), PH09 (Constellation CRUD + CxId), PH05 (WAL) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/16 §7` |

## Goal

Implement `calyx verify-restore --vault <path>` — a CLI command that opens a
restored vault directory (does NOT require `calyxd` to be running), reads back
every constellation CxId stored in the base CF, reads back every anchor, walks
the full ledger chain from genesis to tip verifying each hash link, and reports
byte counts + the chain tip hash. Exits 0 only if the chain is fully intact and
at least one constellation and one anchor are readable. This is the mechanical
proof used in the DR drill FSV — not a "restored:true" flag.

## Build (checklist of concrete, code-level steps)

- [ ] `VerifyRestoreReport` struct (serde-serializable):
  ```rust
  pub struct VerifyRestoreReport {
      pub vault_path: PathBuf,
      pub constellation_count: u64,
      pub anchor_count: u64,
      pub ledger_entry_count: u64,
      pub ledger_tip_hash: String,    // hex of the last chain hash
      pub chain_intact: bool,
      pub wal_bytes_present: u64,     // byte count of WAL files
      pub first_cx_id: Option<String>,   // hex of the first CxId found
      pub error: Option<String>,         // CALYX_* code if chain_intact==false
  }
  ```
- [ ] `fn verify_restore(vault_path: &Path) -> Result<VerifyRestoreReport,
  DaemonError>`:
  1. Open the Aster vault at `vault_path` in read-only mode (no writes, no WAL
     replay that would modify state)
  2. Scan the `base` CF: count every `CxId` key; read the first one back
     completely (all slot columns); assert the constellation bytes are non-empty
  3. Scan the `anchors` CF: count all anchor entries
  4. Walk the `ledger` CF from seq=0 to tip: for each entry, verify
     `entry.prev_hash == hash(previous_entry)`; on any mismatch →
     `chain_intact = false`, `error = Some("CALYX_LEDGER_CHAIN_BROKEN")`
  5. Stat all `wal/*.wal` files; sum their sizes into `wal_bytes_present`
  6. Return the complete report; exit code 0 iff `chain_intact == true` and
     `constellation_count > 0` and `anchor_count > 0` and `wal_bytes_present > 0`
- [ ] `calyx verify-restore --vault <path> [--json]` CLI subcommand: prints the
  report as JSON (with `--json`) or as human-readable text; exits 1 on any
  failure or chain break
- [ ] Read-only vault open: `CalyxVault::open_readonly(path)` — opens the
  manifest and CFs in read-only mode; no compaction, no background writes, no
  WAL recovery that would alter state. Assert no write operations are called.
- [ ] `verify.rs` must NOT require ANN indexes, kernel indexes, or guard state
  to be present (those are excluded from the restic backup). If their
  directories are absent, the tool proceeds without them and notes their absence
  in a log line.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `verify_restore` on a freshly-created test vault with 3 known
  constellations and a valid ledger chain → `chain_intact == true`,
  `constellation_count == 3`, ledger tip hash matches known value (seeded)
- [ ] unit: flip one byte in the 5th ledger entry of the test vault →
  `verify_restore` returns `chain_intact == false` with
  `error == Some("CALYX_LEDGER_CHAIN_BROKEN")` and exit code 1
- [ ] unit: vault with 0 constellations → exits 1 (`constellation_count == 0`)
- [ ] unit: `wal_bytes_present == 0` (WAL files absent) → exits 1
- [ ] unit: ANN index directory absent → proceeds without error (just skips);
  report does not mention ANN
- [ ] edge: `--vault <path>` does not exist → exits 1 with
  `CALYX_DAEMON_CONFIG_INVALID` naming the missing path
- [ ] edge: vault is open (locked) by a running `calyxd` → read-only open
  succeeds (no exclusive lock on read-only open); or if exclusive lock required
  by Aster, document and return a clear error code
- [ ] fail-closed: partial ledger (last entry truncated) → chain walk fails at
  the truncated entry; `chain_intact == false` with the seq number of the
  break in `error`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `VerifyRestoreReport` JSON output from a restored vault on aiwonder
- **Readback:**
  ```bash
  # On aiwonder — against a seeded test vault before the DR drill:
  cargo test -p calyxd verify -- --nocapture 2>&1 | tail -30
  # All unit tests pass

  # Pre-drill smoke test (against live vault, read-only):
  calyx verify-restore --vault /zfs/hot/calyx --json | python3 -m json.tool
  # Must show: chain_intact=true, constellation_count>0, wal_bytes_present>0
  ```
- **Prove:** unit tests pass (chain break detected at right seq, count correct);
  live vault smoke test shows `chain_intact: true` and non-zero counts. Both
  outputs attached to PH67 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH67 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
