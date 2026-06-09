# PH36 — Merkle checkpoints + verify_chain + reproduce()

**Stage:** S7 — Ledger Provenance  ·  **Crate:** `calyx-ledger`  ·
**PRD roadmap:** P7  ·  **Axioms:** A15, A16

## Objective

Build tamper detection and claim-replay on top of the PH35 hash chain. Periodic
Merkle roots over `[seq_a, seq_b]` provide compact, exportable attestations
(optionally Ed25519-signed). `verify_chain(range)` walks every entry and
re-checks the hash chain; if it finds a broken link it returns
`CALYX_LEDGER_CHAIN_BROKEN` and quarantines the range (fail-closed — it never
silently continues). `reproduce(answer_id)` re-measures the recorded inputs
with the recorded content-addressed frozen lenses/weights, re-runs the recorded
fusion, and asserts the result is within numerical tolerance using Forge
determinism mode — proving the answer was measured, not fabricated. Together
these make Calyx auditable to the byte (PRD `PROVENANCE` predicate, `11 §3/§5`).

## Dependencies

- **Phases:** PH35 (hash-chain append-only CF — all entry types and the chain
  structure must exist before we can walk or checkpoint it)
- **Provides for:** PH61 (crypto-shred must verify chain integrity before
  and after erasure), PH67 (DR restore verifies chain after backup+restore),
  PH63 (calyx-mcp exposes `verify_chain` + `get_answer_trace`), PH70
  (intelligence FSV uses `reproduce` as the honesty gate)

## Current state (build off what exists)

`calyx-ledger` has its entry/append/group-commit layer after PH35.
`calyx-core/src/model/signal.rs` has `LedgerRef { seq, hash }`.
`calyx-aster/src/cf/key.rs` has `ledger_range(start, end)`.
`calyx-registry` will have frozen, content-addressed lenses (PH18) by the time
reproduce is called; PH36 depends on that contract existing.
`calyx-forge` (PH13) provides the CUDA determinism mode required by reproduce.
`merkle.rs` is implemented and FSV-signed-off through #249/#347/#348.
`verify.rs` is implemented and FSV-signed-off through #250 with Aster manifest
quarantine. `checkpoint.rs` is implemented and FSV-signed-off through #251 with
same-WAL-batch Admin checkpoint rows. `reproduce.rs` now covers the
content-addressed lens lookup, Forge determinism activation, input-hash
verification, and slot re-measure half through #252; fusion replay/drift result
assembly remains T05.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-ledger/src/merkle.rs` | Range Merkle tree over ledger entries; `merkle_root(range)`; Ed25519-signed export bundle |
| `crates/calyx-ledger/src/verify.rs` | `verify_chain(vault, range) -> VerifyResult`; `CALYX_LEDGER_CHAIN_BROKEN`; quarantine flag write |
| `crates/calyx-ledger/src/reproduce.rs` | `reproduce(answer_id) -> ReproduceResult`; content-addressed lens/weight lookup; Forge determinism mode invocation; drift assertion |
| `crates/calyx-ledger/src/checkpoint.rs` | Periodic checkpoint scheduler; checkpoint record written to ledger CF as `EntryKind::Admin`; cadence config |
| `crates/calyx-ledger/src/lib.rs` | Re-exports; API surface `get_provenance`, `get_answer_trace`, `verify_chain`, `merkle_root`, `reproduce`, `audit` |
| `crates/calyx-ledger/src/tests/` | Unit + proptest + FSV-support tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `merkle.rs`: range root + leaf hashing + Ed25519-signed export | — |
| T02 | `verify.rs`: `verify_chain(range)` + `CALYX_LEDGER_CHAIN_BROKEN` + quarantine | T01 |
| T03 | Checkpoint scheduler: periodic Merkle root written as Admin entry (done #251) | T01 |
| T04 | `reproduce.rs`: content-addressed lens lookup + re-measure + Forge determinism (done #252) | T02 |
| T05 | `reproduce.rs`: re-run fusion + drift assertion + `ReproduceResult` | T04 |
| T06 | Audit query surface: `get_provenance`, `get_answer_trace`, `audit(filter)` | T02 |
| T07 | FSV integration: flip-byte tamper test + reproduce bit-parity test | T05, T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Two proofs, both byte-level on aiwonder:

1. **Tamper detection:** flip one byte in a stored ledger entry (raw CF edit
   with `xxd` + `dd`); run `calyx verify-chain --vault <vault> --range 0..100`;
   confirm the output prints `CALYX_LEDGER_CHAIN_BROKEN at seq=<n>` where `<n>`
   is exactly the sequence number of the corrupted entry — not before, not after.

2. **Reproduce:** run `calyx reproduce --answer <answer_id>`; observe output
   `{ reproduced: true, max_drift: <f64> }` where `max_drift ≤ 1e-3` (bit-parity
   within tolerance); read both the original and reproduced answer rows from the
   `ledger` CF and confirm the score vectors differ by ≤ 1e-3 per element.

## Risks / landmines

- **Quarantine is fail-closed (A16):** `verify_chain` must write a quarantine
  tombstone to the vault manifest (not to the `ledger` CF itself, which is
  append-only) so that subsequent reads from the affected range return
  `CALYX_LEDGER_CHAIN_BROKEN` rather than serving potentially tampered data.
- **Ed25519 key management:** signing key is vault-local; never hardcoded; if
  absent, `merkle_root` returns unsigned root (still valid for local audit); sign
  is opt-in for export.
- **Reproduce requires frozen lenses (PH18):** if the lens referenced in the
  ledger entry has been retired without a frozen content-addressed snapshot,
  `reproduce` returns `CALYX_LENS_FROZEN_VIOLATION` (not a false positive
  drift — the failure reason is distinct and explicit).
- **Forge determinism mode (PH13):** `reproduce` must set the Forge determinism
  seed from the recorded session seed in the ledger payload; if the seed is
  absent, `reproduce` returns `CALYX_REPRODUCE_NONDETERMINISTIC` rather than
  silently accepting drift.
- **≤500-line hard limit:** `reproduce.rs` may need to be split into
  `reproduce/lens.rs` and `reproduce/fusion.rs` if fusion re-run logic grows.
