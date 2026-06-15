# PH62 · T06 — Provenance, verify-chain, anneal-status

| Field | Value |
|---|---|
| **Phase** | PH62 — calyx-cli (vault/lens/ingest/search/readback) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/cmd/provenance.rs` (≤500) |
| **Depends on** | T04, PH35 (hash-chain Ledger CF), PH36 (Merkle verify/reproduce), PH43 (tripwires/anneal) |
| **Axioms** | A15, A17 |
| **PRD** | `dbprdplans/14 §2` (provenance/ops group), `dbprdplans/18 §4` |

## Goal

Implement the provenance and ops subcommands that expose the Ledger hash-chain
and Anneal self-optimization state. An agent uses `provenance` to get full
lineage for a constellation, `verify-chain` to check tamper-evidence, and
`anneal-status` to inspect self-optimization proposals and tripwire state. All
provenance commands default to including chain hashes in their output (A15).

## Build (checklist of concrete, code-level steps)

- [ ] `cmd/provenance.rs` — `provenance <vault> <cx_id>`: calls
  `Calyx::provenance(vault, cx_id)` → prints `Lineage` JSON:
  `{"cx_id":"…","ingest_seq":12,"ledger_chain_hash":"<hex64>",
   "lens_measures":[{"slot":0,"lens_id":"<hex16>","measured_at":"<ts>"}],
   "anchors":[{"kind":"test_pass","ledger_seq":13}]}`
- [ ] `cmd/provenance.rs` — `verify-chain <vault> [--from <seq>] [--to <seq>]`:
  calls `Calyx::verify_chain(vault, range)` → prints
  `{"status":"ok"|"broken","checked":<n>,"break_at":null|<seq>}`;
  `CALYX_LEDGER_CHAIN_BROKEN` on tamper detection → remediation
  `"quarantine range, investigate"` on stderr
- [ ] `cmd/provenance.rs` — (stub, wires to full impl at PH36):
  `reproduce <vault> <answer_id>`: calls `Calyx::reproduce(answer_id)` →
  prints `{"bit_parity":true|false,"original_hash":"…","reproduced_hash":"…"}`;
  divergence → structured error with both hashes in message
- [ ] `cmd/provenance.rs` — `anneal-status <vault>`: calls
  `Calyx::anneal_status(vault)` → prints `AnnealStatus` JSON:
  `{"phase":"stable"|"healing"|"tuning","tripwires":[{"name":"…","state":"armed"|
  "tripped"}],"proposals":[{"type":"add_lens","rationale":"…"}],
   "last_soak_at":"<ts>","p99_latency_ms":42}`
- [ ] All commands route `CalyxError` through `CliError` → JSON stderr; notably
  `CALYX_LEDGER_CHAIN_BROKEN` from verify-chain and `CALYX_STALE_DERIVED` from
  a stale anneal status

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `provenance` output for a known seeded `CxId` contains the exact
  `ingest_seq` and `ledger_chain_hash` written by that ingest operation
- [ ] unit: `verify-chain` on an unmodified vault → `{"status":"ok","break_at":null}`
- [ ] unit: `verify-chain` after flipping a byte in a Ledger file → `break_at`
  equals the tampered sequence number; `CALYX_LEDGER_CHAIN_BROKEN` on stderr
- [ ] unit: `anneal-status` JSON contains all required fields (`phase`, `tripwires`,
  `proposals`) and is valid JSON (parse round-trip)
- [ ] edge: `provenance` for non-existent `cx_id` → `CALYX_VAULT_ACCESS_DENIED`;
  `verify-chain --from 999 --to 1` (invalid range) → `CALYX_CLI_USAGE_ERROR`;
  `reproduce` for non-existent answer → structured error, not panic
- [ ] fail-closed: `reproduce` with hash mismatch → prints both hashes in message,
  exit 2; never silently returns `bit_parity:true` on a mismatch

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the raw Ledger CF entries at `<vault.calyx>/ledger/<seq_hex>` written
  during ingest and anchor operations
- **Readback:** `calyx readback --ledger <vault.calyx>/ledger/<seq_hex>` → prints
  raw Ledger entry bytes; `calyx verify-chain aiwonder-test` → `"status":"ok"`;
  `xxd <vault.calyx>/ledger/<seq_hex>` on aiwonder cross-checks the hash bytes
- **Prove:** ledger entry bytes for seq=1 are present and contain a non-zero
  chain hash; `verify-chain` on the intact vault returns `ok`; after
  deliberately corrupting one byte in a ledger file, `verify-chain` returns
  `broken` at the exact tampered seq; `provenance <cx_id>` output's
  `ledger_chain_hash` matches the bytes read by `readback --ledger`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ [ FSV evidence (readback output / screenshot) attached to the PH62 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
