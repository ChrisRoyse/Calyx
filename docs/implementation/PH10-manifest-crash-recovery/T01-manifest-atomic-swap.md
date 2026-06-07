# PH10 · T01 — Manifest atomic swap + version guard

| Field | Value |
|---|---|
| **Phase** | PH10 — Manifest + atomic swap + crash recovery |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/manifest/mod.rs` (≤500), `crates/calyx-aster/src/manifest/tests.rs` (≤500) |
| **Depends on** | PH04 (CalyxError), PH09 (vault structure) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §7` |

## Goal

Harden the existing `ManifestStore::write_current` / `load_current` with a full
test suite proving the atomic swap is correct under concurrent writes, that an
unknown-major version fails closed, that a CURRENT pointing at a non-existent or
corrupt manifest file fails closed, and that `ImmutableRef` path validation
rejects all traversal attacks. These are already implemented; this card adds the
missing test coverage.

## Build (checklist of concrete, code-level steps)

- [ ] Add test: `write_current(manifest)` produces three files: `CURRENT`,
  `MANIFEST`, `manifest-<seq>.json`; `load_current()` returns the same manifest.
- [ ] Add test: two sequential `write_current` calls with `manifest_seq = 1` then
  `manifest_seq = 2`; `load_current()` returns seq=2; both `manifest-*.json` files
  exist (old one not deleted — immutable artifact).
- [ ] Add test: `VaultManifest` with `version.major = 2` (unknown major) fails
  `validate()` with `CALYX_ASTER_CORRUPT_SHARD`.
- [ ] Add test: `load_current()` when `CURRENT` file is absent →
  `CALYX_DISK_PRESSURE` (not found).
- [ ] Add test: `load_current()` when `CURRENT` points at a non-existent
  `manifest-*.json` → `CALYX_DISK_PRESSURE`.
- [ ] Add test: `CURRENT` file containing arbitrary garbage (not a valid manifest
  filename) → `CALYX_ASTER_CORRUPT_SHARD`.
- [ ] Add test: `ImmutableRef::new` with path containing `../` → `CALYX_ASTER_CORRUPT_SHARD`.
- [ ] Add test: `ImmutableRef::new` with path starting with `/` →
  `CALYX_ASTER_CORRUPT_SHARD`.
- [ ] Add proptest: for any valid `VaultManifest`, `encode_manifest` +
  `decode_manifest` round-trips byte-exact.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: two-write sequence; `load_current` returns latest.
- [ ] unit: bad major version → `CALYX_ASTER_CORRUPT_SHARD`.
- [ ] proptest: encode/decode round-trip.
- [ ] edge (≥3): (1) CURRENT absent → Err; (2) CURRENT points at missing file →
  Err; (3) path traversal in ImmutableRef → Err.
- [ ] fail-closed: corrupt MANIFEST JSON bytes → `CALYX_ASTER_CORRUPT_SHARD` on
  `decode_manifest`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `vault/CURRENT` and `vault/manifest-00000000000000000001.json` files.
- **Readback:**
  ```
  xxd /home/croyse/calyx/test-vault/CURRENT
  cat /home/croyse/calyx/test-vault/manifest-00000000000000000001.json
  ```
- **Prove:** `CURRENT` contains the text `manifest-00000000000000000001.json`;
  the JSON file has `"manifest_seq": 1`, `"durable_seq": <N>`, correct panel and
  codebook refs. Screenshot posted to PH10 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH10 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
