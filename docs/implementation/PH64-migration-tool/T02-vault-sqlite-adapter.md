# PH64 · T02 — VaultSqliteAdapter: row → constellation (1-slot)

| Field | Value |
|---|---|
| **Phase** | PH64 — Migration tool (sqlite→calyx vault) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/migrate/adapter.rs` (≤500) |
| **Depends on** | T01 (ChunkRow type), PH09 (VaultStore + CxId), PH18 (LensId content-addressing) |
| **Axioms** | A1, A4, A16, A18 |
| **PRD** | `dbprdplans/15 §2` (vault concern mapping), `dbprdplans/15 §4` (invariants), `dbprdplans/15 §5` |

## Goal

Implement `VaultSqliteAdapter` — the struct that converts a `ChunkRow` into a
Calyx `Constellation` with exactly one slot (the 768-d GTE vector), preserving
`chunk_id` and `database_name` as metadata, assigning the correct content-
addressed `LensId` for the GTE model, and never persisting candidate text. This
adapter is the Rust equivalent of the `vault-sqlite.ts` storage interface — it
implements the same semantic contract in Rust.

## Build (checklist of concrete, code-level steps)

- [ ] `migrate/adapter.rs`: `struct VaultSqliteAdapter { gte_lens_id: LensId,
  slot_id: SlotId }` initialized with the GTE model's content-addressed `LensId`
  (passed in at construction; the migration CLI arg `--gte-lens-id <hex>` supplies
  it, or it is derived from the GTE model hash if the vault already has the lens
  registered)
- [ ] `fn from_chunk_row(&self, row: ChunkRow) -> Result<Constellation, CliError>`:
  - `CxId`: derive as `blake3(row.content ‖ panel_ver=1u32.to_be_bytes() ‖ salt=
    [0u8;16])` so it is deterministic and content-addressed (same content →
    same `CxId` on re-migration)
  - `Constellation.slots`: one entry at `self.slot_id` → `SlotVector::Dense(
    row.embedding)` — the raw GTE float32 vector
  - `Constellation.input_ref`: `InputRef::ContentHash(blake3(row.content))` —
    stores the content hash, not the cleartext (never persist candidate text,
    PRD 15 §4)
  - `Constellation.scalars`: numeric source measurements only; never encode
    `chunk_id`/`database_name` here because those identifiers are strings.
  - `Constellation.metadata`: `chunk_id` and `database_name` stored verbatim
    (string keys matching the
    `vault-sqlite.ts` contract names)
  - `CALYX_LENS_DIM_MISMATCH` if `row.embedding.len() != 768`
  - `CALYX_LENS_NUMERICAL_INVARIANT` if any embedding value is NaN or Inf

- [ ] `fn gte_lens_id_for_hash(model_weights_hash: &[u8; 32]) -> LensId`:
  content-addressed `LensId([0..16] of blake3(model_weights_hash))` — same hash
  = same LensId across vaults; different model = different LensId (never mix
  vectors across models, PRD 15 §4)

- [ ] Port the `vault-sqlite.ts` allowed-direct-import test invariants as doc-
  comments on `VaultSqliteAdapter`: list the 4 invariants that the TypeScript
  tests enforce and assert them in the Rust unit tests (see Tests below)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `from_chunk_row` with `chunk_id="abc123"`, `database_name="mydb"`,
  `content=b"hello"`, `embedding=[1.0f32; 768]` → `cx_id` is the deterministic
  blake3 hash; metadata contains `chunk_id:"abc123"` and `database_name:"mydb"`
  verbatim
- [ ] unit: same `content` bytes → same `CxId` on two calls (content-addressed
  idempotency invariant from `vault-sqlite.ts`)
- [ ] unit: `input_ref` is `ContentHash(blake3(b"hello"))`, not the cleartext
  `b"hello"` (never persist candidate text)
- [ ] unit: two `from_chunk_row` calls with different `gte_lens_id` values but same
  content → different `slots` keys (different `SlotId`) — LensId content-
  addressing enforces no vector mixing
- [ ] unit: `embedding` with one `f32::NAN` → `CALYX_LENS_NUMERICAL_INVARIANT`,
  `code` matches exactly
- [ ] proptest: for any 768-element `Vec<f32>` of finite values, `from_chunk_row`
  succeeds and `slots[slot_id]` is `SlotVector::Dense(v)` where `v` equals the
  input embedding
- [ ] edge: `embedding.len() == 767` → `CALYX_LENS_DIM_MISMATCH`; empty
  `database_name` → preserved as empty string in metadata, not an error; very
  long `chunk_id` (1000 chars) → stored without truncation

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the base CF row at `<vault.calyx>/cf/base/<cx_id_hex>` after a
  migration of a single known row from `sample-leapable.db`
- **Readback:** `calyx readback --cf-row <vault.calyx> --cf base --key <cx_id_hex>`
  → hex dump; manually verify that the `chunk_id` string bytes appear in the
  metadata section of the dump (use `grep`/`xxd` on the bytes to find the
  `"abc123"` string pattern in the constellation serialization)
- **Prove:** the `CxId` for a known `content` value matches the deterministic
  blake3 computation (compute manually: `echo -n "hello" | b3sum --no-names |
  cut -c1-32` and compare to the `cx_id_hex`); `chunk_id` string is present in
  the CF row bytes

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH64 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
