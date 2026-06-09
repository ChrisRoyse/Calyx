# PH18 · T06 — Cross-vault LensId stability test

| Field | Value |
|---|---|
| **Phase** | PH18 — Frozen contract + content-addressed LensId |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/tests/cross_vault.rs` (≤500) |
| **Depends on** | T01, T05 (this phase) |
| **Axioms** | A4 |
| **PRD** | `dbprdplans/05 §4`, `dbprdplans/03 §2`, `13_STAGE3_REGISTRY.md §PH18 FSV gate` |

## Goal

Prove the cross-vault `LensId` stability invariant from the PH18 FSV gate:
register the same `LensSpec` in two independent `Registry` instances and
confirm both produce the identical `LensId`. This is the identity contract
that allows the same lens used in one vault to be recognised as the same
instrument in another vault without a lookup service.

## Build (checklist of concrete, code-level steps)

- [x] Test `cross_vault_same_lensid`:
  - build `LensSpec` with fixed `name = "test-gte-768"`, fixed
    `weights_sha256 = [0xab; 32]`, fixed `corpus_hash = [0xcd; 32]`,
    `output = SlotShape::Dense(768)`.
  - create `Registry::default()` → `r1`.
  - create `Registry::default()` → `r2`.
  - register the identical spec + a mock lens in both.
  - assert `r1.get_spec(id1).unwrap().lens_id == r2.get_spec(id2).unwrap().lens_id`.
  - print both ids with `println!("{:x?}", id)`.
- [x] Test `cross_vault_different_weights_different_id`:
  - build two specs differing only in `weights_sha256`; register in separate
    registries; assert `id1 != id2`.
- [x] Test `lensid_is_deterministic_across_process_restarts`:
  - hard-code the expected bytes from a previous run of `compute_lens_id` with
    the canonical test vector; assert equality on every run.
- [x] Assert that `LensId` derives `PartialEq`, `Eq`, `Hash`, `Debug`, and
  `serde::{Serialize, Deserialize}` (compile-time check).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: two independent registries produce identical `LensId` for identical
  spec (assertion is byte-for-byte equality).
- [x] unit: specs differing by one bit in `weights_sha256` → different `LensId`.
- [x] unit: canonical test vector → expected pre-committed 16-byte hex value
  (from T01).
- [x] edge (≥3): (1) registering in different order in each registry does not
  affect `LensId` computation; (2) `corpus_hash` all-ones vs all-zeros → two
  distinct ids; (3) two specs with same name but different `output` shape →
  different ids.
- [x] fail-closed: N/A — `compute_lens_id` is total; assert no panic on any
  spec.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test file `cross_vault.rs` output on aiwonder
- **Readback:** `cargo test -p calyx-registry cross_vault -- --nocapture 2>&1`
- **Prove:** output shows `r1.lens_id = r2.lens_id = <same 16 hex bytes>`;
  also shows `id1 != id2` for the differing-weights case; screenshot and hex
  bytes attached to PH18 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH18 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
