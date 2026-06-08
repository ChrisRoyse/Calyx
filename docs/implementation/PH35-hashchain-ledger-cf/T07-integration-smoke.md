# PH35 - T07 - Integration smoke: PH09 constellation write -> chained ledger entry in WAL

| Field | Value |
|---|---|
| **Phase** | PH35 - Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 - Ledger Provenance |
| **Crate** | `calyx-aster` integration over `calyx-ledger` |
| **Files** | `crates/calyx-aster/src/vault/ledger_integration_tests.rs` (<=500), `crates/calyx-aster/src/vault.rs` |
| **Depends on** | T05, T06 (this phase) - PH09 |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 Section 1`, `11 Section 6`, `04 Section 5` |

## Goal

Run a complete end-to-end smoke test: write N constellations through the PH09
path with the group-commit hook active, then read back the WAL and the `ledger`
CF to prove every accepted write produced a chained entry, the chain links are
byte-exact, and no entry stores a secret value. This is the primary FSV evidence
for GitHub issue #248.

## Build (checklist of concrete, code-level steps)

- [x] Add ignored aiwonder FSV test
  `ph35_ledger_integration_smoke_aiwonder_fsv` in the Aster vault test module
  so it uses the real PH09 `AsterVault::put` path and durable WAL.
- [x] Before writing, read `ledger_key(0)` at snapshot 0 and prove no ledger row
  is already present.
- [x] Write 100 unique synthetic `Constellation` values through `vault.put`, then
  read the real `ledger` CF rows and verify `prev_hash` / `entry_hash` linkage
  for every row.
- [x] Replay the durable WAL and assert each record with a base-CF write also
  contains one ledger-CF write, with the ledger row staged before the base row.
- [x] Decode ledger payload bytes and assert they contain none of `"secret"`,
  `"password"`, or `"token"`; persist a JSON readback for manual FSV.

## Tests (synthetic, deterministic - known input -> known bytes/number)

- [x] Unit/supporting: existing PH35 tests cover single append, sequential
  hash-chain append, restart recovery, payload redaction, actor stamping, and
  group-commit staging.
- [x] Integration FSV: `n=100` unique PH09 constellation writes produce ledger
  seq 0..99, `EntryKind::Ingest`, and `SubjectId::Cx` matching the stored base
  constellation rows.
- [x] Edge: before-read at snapshot 0 proves an empty ledger; PH09 duplicate
  same-byte ingest remains an idempotent no-op by design and is covered by the
  existing Aster test; the smoke uses unique constellations to prove one ledger
  row per accepted data mutation.
- [x] Fail-closed support: existing WAL and Ledger tests cover torn/corrupt
  record rejection, decode corruption, stale tips, gap detection, and payload
  secret rejection.

## FSV (read the bytes on aiwonder - the truth gate)

- **SoT:** `ledger` CF rows, durable WAL records, and SST files under
  `/home/croyse/calyx/data/fsv-issue248-ledger-integration-smoke-20260608/ledger-integration-smoke/vault`
- **Readback:**
  1. `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue248-ledger-integration-smoke-20260608 cargo test -p calyx-aster ph35_ledger_integration_smoke_aiwonder_fsv -- --ignored --nocapture`
     prints `"chain OK: 100 entries, all links verified"` and writes
     `ledger-integration-smoke-readback.json`.
  2. `cargo run -q -p calyx-cli -- readback --cf ledger --vault <vault>` prints
     ledger CF rows; seq 0-4 show byte-exact `prev_hash` -> previous
     `entry_hash` linkage.
  3. `cargo run -q -p calyx-cli -- readback --wal --vault <vault>` plus `xxd`
     readback proves 100 WAL records, 100 ledger rows, 100 base rows, and each
     record stages ledger before base.
  4. `grep -RIna -e secret -e password -e token <vault>/cf/ledger <ledger-readback>`
     writes an empty `08-secret-grep.out` and `09-secret-grep-count.out` reads
     `0`.
- **Prove:** before: no ledger row at seq 0; after: 100 accepted constellation
  writes produce 100 chained ledger rows, WAL co-location with data rows, and no
  raw secret strings in ledger payload/readback bytes.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] File(s) <= 500 lines (line-count gate passes).
- [x] FSV evidence attached to GitHub issue #248.
- [x] No anti-pattern: no flatten / no `C(N,2)` past DPI / nothing "trusted"
      without grounding / no frozen-lens mutation / no harness-as-FSV.
