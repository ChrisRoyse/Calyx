# PH71 · T06 — Production Vault calyx-only + control-plane-identical FSV + PostgreSQL-untouched proof (V2 gate)

| Field | Value |
|---|---|
| **Phase** | PH71 — V0 shadow → V1 flip → V2 calyx-only |
| **Stage** | S19 — Leapable Vault Swap |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/leapable/production_fsv.rs` (≤500) |
| **Depends on** | T05 (shadow removed, CalyxOnly mode), PH35/PH36 (Ledger provenance chain) |
| **Axioms** | A18, A15, A16 |
| **PRD** | `dbprdplans/15 §1`, `15 §4`, `15 §5 V2` |

## Goal

Deliver the **V2 FSV gate** — the phase completion proof. A real production Vault
on aiwonder runs Calyx-only with full provenance (every Ask returns a
`LedgerRef`-cited result that is reproducible via `reproduce()`). The
**control-plane queries/billing/listing for that Vault return identical results**
before and after. **PostgreSQL is verified untouched** (`pg_dump` diff shows zero
rows changed; every `psql` control-plane query returns byte-identical responses).
This card produces `ProductionFSV`, the mechanical proof tool that records all
evidence for the PH71 GitHub issue.

## Build (checklist of concrete, code-level steps)

- [ ] `ProductionFSV::snapshot_pg_state(pg_conn: &PgConn, vault_name: &str) ->
      Result<PgSnapshot, CalyxError>`: queries the PostgreSQL control-plane tables
      relevant to this Vault (`creator_databases` WHERE `database_name = vault_name`,
      `queries` table recent rows, billing summary row, `marketplace`, and
      `outbox`) and hashes the result set to
      `PgSnapshot { tables: Vec<TableHash>, taken_at: Timestamp }`. Uses
      **read-only** queries only. Any write attempt → compile error (connection is
      opened read-only). `CALYX_PG_WRITE_ATTEMPTED` if the driver reports a write
      op somehow triggered.
- [ ] `ProductionFSV::verify_pg_unchanged(before: &PgSnapshot, after: &PgSnapshot)
      -> Result<PgUnchangedProof, CalyxError>`: compares table hashes; any
      difference → `CALYX_PG_STATE_CHANGED { table, before_hash, after_hash }`.
      `PgUnchangedProof { matched_tables: usize, vault_name: String }`.
- [ ] `ProductionFSV::run_full_ask_cycle(vault: &ShadowVault, query_vec: &[f32],
      top_k: usize) -> Result<AskProof, CalyxError>`: runs Ask on the `CalyxOnly`
      Vault; asserts every `Hit` carries a `LedgerRef`; calls `ledger.reproduce(hit.
      ledger_ref)` and asserts the reproduced Constellation matches the Hit content
      (byte-exact on `chunk_id`, `text_hash`). Returns `AskProof { hits: Vec<Hit>,
      all_ledger_refs_valid: bool, reproduced_byte_exact: bool }`.
- [ ] `ProductionFSV::verify_control_plane_contract(vault_name: &str, pg_conn:
      &PgConn) -> Result<ContractProof, CalyxError>`: executes the same query set
      the Leapable backend issues for this Vault (`/api/databases/<vault_name>`
      equivalent, `leapable_db_*` SQL calls per PRD `15 §4`); compares response
      structure and values to the baseline captured in `PgSnapshot`. Any column
      name, type, or value change → `CALYX_PG_CONTRACT_VIOLATION`.
- [ ] `ProductionFSV::emit_evidence(ask_proof, pg_unchanged, contract_proof) ->
      Result<EvidenceBundle, CalyxError>`: serializes all proofs to
      `ph71_v2_evidence.json` on disk. The bundle is the artifact attached to the
      GitHub issue. Includes: `database_name` (verbatim), `calyx_only_at_seq`,
      `pg_snapshot_before_hash`, `pg_snapshot_after_hash`, `all_hashes_match: bool`,
      `all_ledger_refs_valid: bool`, `reproduced_byte_exact: bool`.
- [ ] `calyx leapable production-fsv` CLI subcommand: runs all four functions in
      order; prints summary; exits non-zero if any proof fails.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `verify_pg_unchanged` with two identical `PgSnapshot`s (same table
      hashes, seed 0xPG_SAFE) → `PgUnchangedProof.matched_tables == N`; no error.
- [ ] unit: `verify_pg_unchanged` with one differing table hash →
      `CALYX_PG_STATE_CHANGED` with correct `table` name and both hashes in the
      error.
- [ ] unit: `run_full_ask_cycle` on a `CalyxOnly` fixture Vault (5 chunks, known
      vectors) → `AskProof.all_ledger_refs_valid == true`;
      `AskProof.reproduced_byte_exact == true` (reproduce returns same `chunk_id`
      and `text_hash` as the original Hit).
- [ ] unit: `emit_evidence` with all proofs passing → `ph71_v2_evidence.json` on
      disk; JSON contains `all_hashes_match: true`, `all_ledger_refs_valid: true`,
      `reproduced_byte_exact: true`.
- [ ] proptest: for any `database_name` (ASCII, 1–64 chars, seed 0xDB_NAME_42),
      `ContractProof.database_name` matches input byte-exact (no normalization).
- [ ] edge (≥3):
      (a) `snapshot_pg_state` with a Vault name not in `creator_databases` →
          `CALYX_VAULT_NOT_IN_PG { vault_name }` (the Vault doesn't exist in the
          control plane — wrong name passed);
      (b) `run_full_ask_cycle` on a Vault still in `Calyx` mode (not `CalyxOnly`)
          → `CALYX_VAULT_NOT_CALYX_ONLY`;
      (c) `reproduce()` returns a constellation with a different `text_hash` than
          the Hit (corrupted Ledger) → `CALYX_REPRODUCE_MISMATCH`, `AskProof.
          reproduced_byte_exact == false`.
- [ ] fail-closed: `snapshot_pg_state` with a write-capable connection →
      `CALYX_PG_WRITE_ATTEMPTED` (connection is typed as read-only at the Rust type
      level; this error fires only if the type guard is somehow bypassed — belt and
      suspenders).

## FSV (read the bytes on aiwonder — the truth gate)

> This is the **V2 FSV gate** and the **Phase 71 completion proof**. Every item
> below must be byte-proven on a real production Vault on aiwonder. PostgreSQL on
> aiwonder must not be touched during this run — the `before` snapshot is taken
> from the existing leapable/postgres state (read-only), and the `after` snapshot
> must be identical.

- **SoT:** `ph71_v2_evidence.json` on aiwonder; `pg_dump` diff; `calyx readback`
  output; `LedgerRef` chain verification output.
- **Readback:**
  ```
  # 1. Take PostgreSQL snapshot BEFORE (read-only):
  calyx leapable production-fsv snapshot-pg \
      --vault-name <real_vault_name> \
      --pg-conn "host=localhost port=5432 dbname=leapable user=readonly" \
      --out pg_before.json

  # 2. Run the full Calyx-only Ask cycle + evidence emission:
  calyx leapable production-fsv run \
      --vault vault_v2.calyx \
      --vault-name <real_vault_name> \
      --pg-conn "host=localhost port=5432 dbname=leapable user=readonly" \
      --out ph71_v2_evidence.json

  # 3. Take PostgreSQL snapshot AFTER (read-only):
  calyx leapable production-fsv snapshot-pg \
      --vault-name <real_vault_name> \
      --pg-conn "host=localhost port=5432 dbname=leapable user=readonly" \
      --out pg_after.json

  # 4. Verify PostgreSQL untouched:
  calyx leapable production-fsv verify-pg-unchanged \
      --before pg_before.json --after pg_after.json
  # must print: all_hashes_match=true, vault_name=<expected>, PASS

  # 5. Byte-readback on Vault constellations + Ledger chain:
  calyx readback --vault vault_v2.calyx --verify-ledger
  # must print: N constellations, ledger chain valid, all LedgerRefs present

  # 6. Control-plane contract check:
  calyx leapable production-fsv verify-contract \
      --vault-name <real_vault_name> \
      --pg-conn "host=localhost port=5432 dbname=leapable user=readonly"
  # must print: ContractProof PASS, database_name=<expected verbatim>

  # 7. Inspect evidence bundle:
  cat ph71_v2_evidence.json | jq '{all_hashes_match, all_ledger_refs_valid, reproduced_byte_exact}'
  # must print: {"all_hashes_match": true, "all_ledger_refs_valid": true, "reproduced_byte_exact": true}
  ```
- **Prove:** `pg_before.json` and `pg_after.json` hashes are identical for all
  tables (PostgreSQL **untouched**). `ph71_v2_evidence.json` shows all three proof
  flags `true`. `calyx readback --verify-ledger` confirms full provenance on every
  constellation. `database_name` in `ContractProof` matches the source `.db`
  metadata row verbatim. This is the **V2 FSV gate** and the **Stage 19 exit**.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence bundle `ph71_v2_evidence.json` attached to the PH71 GitHub
      issue, showing all three proof flags `true`, on a real production Vault on
      aiwonder
- [ ] `pg_before.json` / `pg_after.json` diff showing **zero hash differences**
      (PostgreSQL untouched) attached to the same issue
- [ ] `calyx readback --verify-ledger` output confirming full LedgerRef coverage
      attached to the same issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
- [ ] `CALYX_PG_WRITE_ATTEMPTED` confirmed to be the only exit path if a write
      op is attempted through the read-only PG connection (grep confirms)
- [ ] Stage 19 exit condition satisfied: every Leapable Vault is a multi-lens,
      kernel-grounded, guarded, provenanced constellation store; `sqlite-vec`
      retired; PostgreSQL control plane exactly as it was (`03_PHASE_MAP Stage 19
      exit`, `29_STAGE19_LEAPABLE.md §Stage 19 exit`)
