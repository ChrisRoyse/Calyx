# T-010 — calyx-core: IDs + content-addressing

**Phase:** PH03 · **Dep:** T-008 · **Sudo:** no

## Objective
The dependency-free identity vocabulary every crate uses: `VaultId`, `LensId`,
`CxId`, `SlotId`, with content-addressing that makes ingest idempotent and
provenance hashes stable.

## Preconditions
- T-008 (workspace), T-015 (test conventions) ideally in parallel.

## Steps
1. `crates/calyx-core/src/ids.rs` (≤500 lines):
   - `VaultId(Ulid)` — stable per vault.
   - `LensId([u8;16])` = `blake3(name‖weights_sha256‖corpus_hash‖output_shape)[..16]`
     — content-addressed (identical lens ⇒ identical id across vaults).
   - `CxId([u8;16])` = `blake3(input_bytes‖panel_version‖vault_salt)[..16]`.
   - `SlotId(u16)` + a stable `slot_key: String`.
   - `Display`/`FromStr`/serde for each; helper `content_address(parts) -> [u8;16]`.
2. Deps: only `blake3`, `ulid`, `serde` (no I/O).
3. proptest round-trips (`parse(display(x)) == x`, serde round-trip); determinism
   test (same inputs → same 16 bytes).

## Deliverables
- `ids.rs` with all four IDs + content-addressing helper + round-trip tests.

## FSV gate
`cargo test -p calyx-core` green; the content-addressing is deterministic —
the same `(input, panel_version, salt)` yields the **same `CxId` bytes** across
runs (assert on the raw bytes, not a Display string); two identical lens specs
yield the same `LensId` (read both).

## Done
IDs implemented, content-addressed, round-trip + determinism tested.

## Refs
PRD `03 §2`, `18 §2`, A1.
