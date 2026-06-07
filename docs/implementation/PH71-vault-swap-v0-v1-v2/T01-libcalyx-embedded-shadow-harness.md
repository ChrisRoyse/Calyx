# PH71 · T01 — `libcalyx` embedded shadow harness (V0)

| Field | Value |
|---|---|
| **Phase** | PH71 — V0 shadow → V1 flip → V2 calyx-only |
| **Stage** | S19 — Leapable Vault Swap |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/leapable/shadow_harness.rs` (≤500), `crates/calyx-cli/src/leapable/mod.rs` (≤100) |
| **Depends on** | PH64 (migration tool + `calyx migrate vault`), PH63 (calyx-mcp) |
| **Axioms** | A18, A15, A16 |
| **PRD** | `dbprdplans/15 §1`, `15 §4`, `15 §5 V0` |

## Goal

Embed `libcalyx` as a **shadow index** alongside the existing `sqlite-vec` Vault.
The harness opens both the existing `.db` (SQLite/`sqlite-vec`) and a new
`vault.calyx` directory side-by-side, wires the `ShadowVault` abstraction that
exposes the same `vault-sqlite.ts` contract to the control plane, and ensures that
the PostgreSQL side cannot distinguish this build from the previous one. This is
the V0 entry-point: everything else in PH71 builds on this harness.

## Build (checklist of concrete, code-level steps)

- [ ] Define `ShadowVault` struct holding `SqliteHandle` (read/write path to
      existing `.db`) and `CalyxHandle` (write path only in V0 — `VaultRef` from
      PH64). `ShadowVault::open(sqlite_path, calyx_dir) -> Result<Self, CalyxError>`.
- [ ] Implement `ShadowVault::close()` that flushes both handles and syncs both
      WALs; any partial state → `CALYX_VAULT_SYNC_FAILED` with remediation message.
- [ ] Expose `ShadowVault::vault_name() -> &str` returning `database_name` verbatim
      from the SQLite metadata row — this name is a code-contract identifier
      (PRD `15 §4`) and must not be transformed.
- [ ] Wire a `VaultMode` enum: `Shadow` (V0), `Calyx` (V1+), `CalyxOnly` (V2).
      `ShadowVault::mode() -> VaultMode`. Changing mode is a one-way ratchet:
      `Shadow → Calyx → CalyxOnly`; reverse → `CALYX_VAULT_MODE_ROLLBACK_DENIED`.
- [ ] Implement `ShadowVault::verify_pg_contract()` — reads the same SQL query set
      the control plane issues (`creator_databases`, `queries` table) against the
      local SQLite and asserts the responses are structurally unchanged; returns
      `CALYX_PG_CONTRACT_VIOLATION` if any column name or type differs.
- [ ] `mod.rs`: re-export `ShadowVault`, `VaultMode`, and the sub-module tree for
      T02–T06. Keep ≤100 lines.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: open `ShadowVault` on a fixture `.db` (3 chunks, known `database_name =
      "test_vault"`) → `vault_name()` returns `"test_vault"` byte-exact; `mode()`
      returns `Shadow`.
- [ ] unit: `close()` after partial write to Calyx handle → both WALs flushed;
      re-open succeeds; no data loss on either side.
- [ ] proptest: `vault_name()` is pure (identical across 100 open/close cycles on
      the same fixture, seed 0xDEAD_BEEF).
- [ ] edge (≥3):
      (a) missing `.db` → `CALYX_VAULT_NOT_FOUND`;
      (b) corrupted Calyx dir (truncated manifest) → `CALYX_MANIFEST_CORRUPT`,
          SQLite side unaffected;
      (c) `database_name` row absent from SQLite → `CALYX_CONTRACT_NAME_MISSING`.
- [ ] fail-closed: attempt `Shadow → Shadow` mode transition (no-op) → returns
      `Ok(())`; attempt `CalyxOnly → Shadow` → `CALYX_VAULT_MODE_ROLLBACK_DENIED`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the Calyx Vault directory (`vault.calyx/`) created by the harness on
  aiwonder; the existing `.db` file (unmodified in V0); and the manifest file
  `vault.calyx/MANIFEST` showing mode=`Shadow`.
- **Readback:**
  ```
  calyx readback --vault vault.calyx --show-manifest
  # must print: mode=Shadow, database_name=<expected>, chunk_count=0 (pre-ingest)
  xxd vault.calyx/MANIFEST | head -4
  # confirm magic bytes present and mode byte = 0x00 (Shadow)
  ```
- **Prove:** before this card: no `vault.calyx/` exists. After: `vault.calyx/MANIFEST`
  exists with `mode=Shadow`, `database_name` matches the source `.db` row verbatim,
  Calyx WAL is empty (no constellations yet — ingest is T02). The `.db` file `stat`
  `mtime` must be **unchanged** (read-only in V0 harness open).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH71 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
- [ ] `.db` `mtime` confirmed unchanged after `ShadowVault::open()` in V0 mode
      (byte-level proof that PostgreSQL-side contract is intact)
