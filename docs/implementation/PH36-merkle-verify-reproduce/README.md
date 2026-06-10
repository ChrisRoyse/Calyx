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
same-WAL-batch Admin checkpoint rows. `reproduce.rs` covers the
content-addressed lens lookup, Forge determinism activation, input-hash
verification, slot re-measure half through #252, and fusion replay/drift result
assembly through #253. `audit.rs` is implemented and FSV-signed-off through
#254 with quarantine-aware provenance, answer trace, audit filtering, linked
Kernel/Guard trace rows, and unprovenanced partial answer protection.
The PH36 exit FSV integration bundle is implemented and FSV-signed-off through
#255 with flip-byte tamper detection at seq 11, manifest quarantine, and
reproduce bit-parity readback.
Stage 7 exit rollup #256 is also FSV-signed-off, covering PH35-PH36 end to end
with group-commit atomicity, all 10 `EntryKind` values, redaction, Admin
checkpoints, tamper quarantine, reproduce bit-parity, and audit trace readback.
#349 signs off the residual PH36 audit-query quarantine filter hardening without
reopening the #249-#256 FSV closeouts. Filtered audit queries ignore unrelated
quarantined rows outside the requested result set, still fail closed for
requested ranges or matching/relevant quarantined rows, reject physical ledger
row-key mismatches, and use typed `cx` provenance fields instead of arbitrary
payload string matching.
#651 hardens `verify_chain` itself for physical row failures: missing Ledger
rows, truncated row payloads, and key/encoded-sequence mismatches now return
structured `VerifyResult::Corrupt` results. `calyx verify-chain --vault` reports
`CALYX_LEDGER_CORRUPT at seq=<n>` and still writes the fail-closed manifest
quarantine record.

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
| T05 | `reproduce.rs`: re-run fusion + drift assertion + `ReproduceResult` (done #253) | T04 |
| T06 | Audit query surface: `get_provenance`, `get_answer_trace`, `audit(filter)` (done #254) | T02 |
| T07 | FSV integration: flip-byte tamper test + reproduce bit-parity test (done #255) | T05, T06 |

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

Latest reproduce evidence (#253): ledger API FSV at
`/home/croyse/calyx/data/fsv-issue253-reproduce-fusion-20260609` wrote
`happy-ledger-cf/0000000000000003.ledger` with payload tag `reproduce_v1`,
`reproduced=true`, `max_drift=0.0`, and intact chain readback. Readback JSON
SHA-256: `97dd9a65f4b1c4421b437247b1b2fb89d99975eae720be4521615713702bd994`.
CLI surfacing for provenance/answer-trace/audit is covered by #254; the final
tamper + reproduce integration bundle is covered by #255.

Latest verify-chain physical-row evidence (#651): aiwonder FSV at
`/home/croyse/calyx/data/fsv-issue651-verify-chain-physical-20260610` wrote
real Aster vaults, removed WAL segments, mutated physical `cf/ledger` SST rows,
and read back `MANIFEST` bytes. Missing row, truncated payload, and
key/encoded-sequence mismatch each failed with `CALYX_LEDGER_CORRUPT at seq=1`,
advanced manifest quarantine state, and caused subsequent ledger readback for
seq 1 to fail closed with `CALYX_LEDGER_CHAIN_BROKEN`. The intact vault still
reported `CHAIN_INTACT count=4`.

Latest audit-query evidence (#254, #349): CLI and Lodestar FSV at
`/home/croyse/calyx/data/fsv-issue254-audit-query-20260609` wrote:
`audit-query-surface/audit-query-readback.json` with SHA-256
`c72fd19bb132533ffdf613d6ca4563e97e458bd54ac4074937f07fea1c94c09d`, and
`ph36-audit-mid-hop-failure/ph36-audit-mid-hop-failure-readback.json` with
SHA-256 `5948a107fff864195659b9cffe89ae4475a21d04afb943efcc438860fb731c25`.
Readback proved provenance count 5, answer trace `complete=true` with linked
Kernel/Guard rows and no warnings, audit ingest count 3, quarantined Answer seq
8 fail-closed with `CALYX_LEDGER_CHAIN_BROKEN`, partial hop rows
`complete=false`, and injected mid-hop failure leaving one Answer row that
`get_answer_trace` marks `Unprovenanced` and not trusted.
#349 adds filter-aware audit quarantine hardening at
`/home/croyse/calyx/data/fsv-issue349-audit-query-hardening-20260609-5697553`:
durable Ledger SST rows were read back with seqs `[0,1,2,3,4]`; manifest
quarantine readback proved `1..2`; `audit --kind ingest` returned seqs `[0,2]`;
`audit --kind measure` failed with `CALYX_LEDGER_CHAIN_BROKEN`; and
`get-provenance` returned typed/explicit cx rows `[0,4]` while ignoring arbitrary
comment/note strings. The same root contains `sha256-manifest.txt`.

Latest PH36 exit integration evidence (#255): aiwonder FSV at
`/home/croyse/calyx/data/fsv-issue255-ph36-integration-20260609` wrote
`ph36-exit-fsv/ph36-fsv-integration-readback.json` with SHA-256
`006ef67bdb9db189b1142c6d4bb45c1181f8b6b31d1fb2cd8a51392553993fea`.
The FSV log SHA-256 is
`9ca1c532d305c8b45f2141e7bb5513c7e03796ca03d56ac9b717085fe02eb403`, and the
xxd log SHA-256 is
`e54e93b614538e45b03e8914e24cdfa31981c02923fd70031c7dcbf108862cff`.
Readback proved `CALYX_LEDGER_CHAIN_BROKEN at seq=11`, manifest quarantine
`0..20` with `broken_at_seq=11`, denied readback for ledger seq 11, and
reproduce `reproduced=true`, `max_drift=0.0`, identical original/reproduced
score bytes `4f71c93c` and `8c31c63c`, with an intact four-row reproduce ledger.

Latest Stage 7 exit rollup evidence (#256): aiwonder FSV at
`/home/croyse/calyx/data/fsv-issue256-stage7-exit-20260609-nomock` wrote
`stage7-exit.log` with SHA-256
`3c9b2e9d5ca2c925bca52f6d0d0f3fcf0377900e56728c0c85f3c2e81505ad5e`,
`ledger-appender-readback.json` with SHA-256
`f6bc6713f91eb93be892c468c09deaa57678ece07f1ffcea77018758e9b72299`,
`ph36-exit-fsv/ph36-fsv-integration-readback.json` with SHA-256
`c53bda82248727fe8f79334a2cf180890082929153e429bebae2ab1ce779af57`,
and `audit-query-surface/audit-query-readback.json` with SHA-256
`153aab69eabd70801d7d7c7a542dc46178a189c1ec340487a6dd9dfef51a52f2`.
Manual physical readback also captured appender ledger row `xxd`, reproduce
Admin row `xxd`, real `calyx-registry` lens IDs, real `calyx-forge`
TurboQuant deterministic seed IDs, group-commit SST/WAL `xxd`, clean redaction
grep, and seq 11 readback failure with `CALYX_LEDGER_CHAIN_BROKEN`.

## Risks / landmines

- **Quarantine is fail-closed (A16):** `verify_chain` must write a quarantine
  tombstone to the vault manifest (not to the `ledger` CF itself, which is
  append-only) so that subsequent reads from the affected range return
  `CALYX_LEDGER_CHAIN_BROKEN` rather than serving potentially tampered data.
- **Physical row failures quarantine too:** missing ledger rows, undecodable row
  bytes, and key/encoded-seq mismatches are `VerifyResult::Corrupt`, not plain
  unquarantined errors.
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
