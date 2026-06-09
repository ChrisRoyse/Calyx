# PH18 · T01 — LensId content-addressing (blake3)

| Field | Value |
|---|---|
| **Phase** | PH18 — Frozen contract + content-addressed LensId |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/lens_id.rs` (≤500) |
| **Depends on** | PH17 T01 (LensSpec exists) |
| **Axioms** | A4 |
| **PRD** | `dbprdplans/05 §4`, `dbprdplans/03 §2` |

## Goal

Implement the content-addressing formula `LensId = blake3(name ‖ weights_sha256
‖ corpus_hash ‖ output_shape)` so that any two registries — on separate
machines, separate vaults, separate runs — always produce the same `LensId`
for the same frozen lens specification. This is the identity invariant the
frozen contract depends on.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn compute_lens_id(spec: &LensSpec) -> LensId`:
  - start a `blake3::Hasher`.
  - feed `spec.name.as_bytes()` (no length prefix; name uniqueness is the
    caller's responsibility).
  - feed `spec.weights_sha256` (32 bytes).
  - feed `spec.corpus_hash` (32 bytes).
  - feed `serde_json::to_vec(&spec.output).expect("SlotShape is always serializable")`
    — this pins the canonical encoding of `SlotShape`.
  - finalize to 32 bytes; take the first 16 bytes as the `LensId` (matching
    `LensId::from_bytes([u8; 16])`).
- [x] `LensSpec::compute_id(&self) -> LensId` convenience wrapper.
- [x] Expose `compute_lens_id` from `calyx_registry::lens_id`.
- [x] Document the exact concatenation order in a `# Canonical form` doc
  comment so it cannot be silently changed.
- [x] Add a compile-time static test vector: `name = "test"`, all-zero hashes,
  `SlotShape::Dense(768)` → known 16-byte `LensId` (pre-compute and hard-code
  in test).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `compute_lens_id` on the static test vector produces the exact
  pre-computed 16-byte `LensId` (byte-exact assertion).
- [x] unit: changing `name` by one byte → different `LensId`.
- [x] unit: changing `weights_sha256[0]` by one bit → different `LensId`.
- [x] proptest: `compute_lens_id(spec) == compute_lens_id(spec)` for arbitrary
  specs (pure function, deterministic).
- [x] edge (≥3): (1) empty `name` string → `LensId` still computes without
  panic; (2) `SlotShape::Sparse(0)` → stable serialization; (3) two specs
  differing only in `corpus_hash` → different `LensId` values.
- [x] fail-closed: N/A — this function is total and infallible; assert in test
  that it never panics on any input.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output on aiwonder; the hardcoded static golden bytes
- **Readback:** `cargo test -p calyx-registry lens_id -- --nocapture 2>&1`
- **Prove:** test prints `compute_lens_id golden: <16 hex bytes>` matching the
  pre-committed value; two independent `compute_lens_id` calls with identical
  spec print identical hex; attached to PH18 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH18 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
