# PH36 · T04 — `reproduce.rs`: content-addressed lens lookup + re-measure + Forge determinism

| Field | Value |
|---|---|
| **Phase** | PH36 — Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/reproduce.rs` (≤500) |
| **Depends on** | T02 (this phase) · PH18 (frozen content-addressed lenses) · PH13 (Forge determinism mode) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §3`, `11 §5` |

## Goal

Implement the lens-lookup and re-measure half of `reproduce(answer_id)`. Given
an `answer_id`, find its `Answer` ledger entry, extract the recorded
`(cx_id, slot_id, lens_id, weights_sha256, input_hash, corpus_shard_hash)`
for each measured slot, retrieve the frozen content-addressed lens snapshot
matching `weights_sha256`, activate Forge determinism mode with the recorded
seed, and re-embed each input to produce the re-measured slot vectors. This
half owns everything up to "I have the re-measured vectors"; T05 owns the
fusion re-run and drift assertion.

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct ReproduceContext { answer_id: QueryId, ledger_entries: Vec<LedgerEntry>, recorded_slots: Vec<RecordedSlot> }` —
  `RecordedSlot` = `{ cx_id, slot_id, lens_id, weights_sha256: [u8;32], input_hash: [u8;32], forge_seed: u64 }`.
- [ ] `fn build_reproduce_context(cf_reader, answer_id) -> Result<ReproduceContext>` —
  reads the `Answer` ledger entry for `answer_id`; reads back-linked `Measure`
  entries for each slot; populates `RecordedSlot` fields from payload.
- [ ] `fn lookup_frozen_lens(registry: &dyn LensRegistry, lens_id: LensId, weights_sha256: &[u8;32]) -> Result<Box<dyn Lens>>` —
  asks the registry for the lens; verifies `lens.weights_hash() == weights_sha256`;
  if mismatch → `CALYX_LENS_FROZEN_VIOLATION`.
  Add `CALYX_REPRODUCE_NONDETERMINISTIC` to error catalog (remediation:
  `"no determinism seed in ledger entry — cannot guarantee reproduce fidelity"`).
- [ ] `fn activate_forge_determinism(forge: &mut dyn ForgeBackend, seed: u64)` —
  sets the PRNG seed for the Forge CUDA/CPU backend; required before any
  re-embedding call.
- [ ] `fn remeasure_slots(ctx: &ReproduceContext, registry: &dyn LensRegistry, forge: &mut dyn ForgeBackend) -> Result<Vec<RemeasuredSlot>>` —
  for each `RecordedSlot`: look up frozen lens, activate determinism, call
  `lens.embed(input_hash_ref)` (uses content-addressed input pointer from
  `InputRef`), returns `RemeasuredSlot { slot_id, vector: Vec<f32> }`.
- [ ] If `forge_seed` is absent from the entry payload → `CALYX_REPRODUCE_NONDETERMINISTIC`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: build a synthetic `ReproduceContext` with 2 slots; mock registry
  returns a fixed deterministic lens; assert `remeasure_slots` produces the
  same vector on two calls with the same seed (idempotency of re-measure).
- [ ] unit: mock lens returns `weights_hash != weights_sha256` → `CALYX_LENS_FROZEN_VIOLATION`;
  assert the error code is exact.
- [ ] unit: `forge_seed=0` in context → `activate_forge_determinism(forge, 0)`
  is called; assert a subsequent embed call is bit-identical to a second call
  with the same seed.
- [ ] edge (≥3): `recorded_slots` empty → `remeasure_slots` returns `Ok(vec![])`;
  lens retired but frozen snapshot present → succeeds; lens retired without
  frozen snapshot → `CALYX_LENS_FROZEN_VIOLATION`.
- [ ] fail-closed: no `forge_seed` field in payload → `CALYX_REPRODUCE_NONDETERMINISTIC`
  (not a panic or silent success with undefined drift).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** re-measured slot vectors on aiwonder after calling reproduce on a
  real ingested constellation
- **Readback:** `cargo test -p calyx-ledger -- --nocapture reproduce_remeasure 2>&1`
  prints the original vector and the re-measured vector for slot 0; assert
  element-wise diff ≤ 1e-3.
- **Prove:** before: no reproduce path; after: re-measure produces a vector
  within 1e-3 of the original (CPU↔GPU bit-parity inherited from Forge PH13);
  `CALYX_LENS_FROZEN_VIOLATION` fires when the weights hash doesn't match
  (not a silent success).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the re-measured golden set (Forge determinism mode)
- [ ] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
