# 04. Core Foundation (calyx-core)

`calyx-core` is the S0 foundation crate that every other Calyx crate depends on. It owns the stable
identifier types, the closed `CALYX_*` error catalog, the shared enum vocabulary, the constellation
data model, the engine trait boundaries, the injected clock abstraction, the bounded-allocation and
bounded-cache primitives, and the security/consent/cold-start/temporal contract types. It has no
intra-workspace dependencies (its `Cargo.toml` lists only `blake3`, `rand`, `rand_chacha`, `serde`,
`thiserror`, `tracing`, `ulid`), which keeps it dependency-free at the bottom of the build graph and
lets sibling crates (e.g. `calyx-aster`, `calyxd`) share these types without a cycle.

> All claims below are derived from the crate source as of this writing. Where a fact could not be
> established from source it is marked **Not determined from source**.

## Source files covered:

- `crates/calyx-core/src/lib.rs` — crate root, module list, public re-exports
- `crates/calyx-core/src/ids.rs` — `VaultId`, `LensId`, `CxId`, `SlotId`, `SlotKey`, `ParseIdError`, `content_address`
- `crates/calyx-core/src/error.rs` — `CalyxError`, `CalyxErrorCode`, `CalyxWarning`, the closed PRD-18 catalog
- `crates/calyx-core/src/enums.rs` — `Modality`, `SlotShape`, `Asymmetry`, `QuantPolicy`, `AnchorKind`, `SlotState`, `AbsentReason`
- `crates/calyx-core/src/traits.rs` — `Input`, `Lens`, `Index`, `VaultStore`, `Estimator`
- `crates/calyx-core/src/time.rs` — `Seq`, `Ts`, `Clock`, `SystemClock`, `FixedClock`
- `crates/calyx-core/src/cosine.rs` — `GuardTauProfile`, `dense_cosine`
- `crates/calyx-core/src/temporal.rs` — temporal policy contracts and `CALYX_TEMPORAL_*` codes
- `crates/calyx-core/src/security.rs` — `AuthN`, `TlsConfig`, `MtlsConfig`, `no_anonymous_write`
- `crates/calyx-core/src/consent.rs` — `LawfulBasis`, `Purpose`, `ConsentTag`, `check_consent`
- `crates/calyx-core/src/cold_start.rs` — `VaultTrustState`, `ColdStartGuard`
- `crates/calyx-core/src/model/mod.rs` — model re-exports
- `crates/calyx-core/src/model/constellation.rs` — `Constellation`, metadata keys
- `crates/calyx-core/src/model/signal.rs` — `Signal`, `ConfidenceInterval`, `InputRef`, `LedgerRef`, `CxFlags`
- `crates/calyx-core/src/model/slot.rs` — `Slot`, `Panel`, `SlotResource`, `LensCost`, `Placement`
- `crates/calyx-core/src/model/anchor.rs` — `Anchor`, `AnchorValue`
- `crates/calyx-core/src/model/vector.rs` — `SlotVector`, `SparseEntry`
- `crates/calyx-core/src/model/validation.rs` — `CALYX_RECORD_SCHEMA_VIOLATION`, `record_schema_error`
- `crates/calyx-core/src/alloc/mod.rs` — `AllocStats`, `CALYX_ALLOC_CAP_EXCEEDED`
- `crates/calyx-core/src/alloc/arena.rs` — `Arena`, `ArenaVec`, `ARENA_BASE_ALIGN`
- `crates/calyx-core/src/alloc/slab.rs` — `SlabPool`, `PageAlignedSlabPool`, `AnnNode`, pool aliases/constants
- `crates/calyx-core/src/cache/mod.rs` — cache re-exports
- `crates/calyx-core/src/cache/lru_ttl.rs` — `LruTtlCache`, `InsertResult`, `CALYX_CACHE_EVICTED`
- `crates/calyx-core/src/cache/lru_ttl/tests.rs` — cache behavior tests
- `crates/calyx-core/tests/fusion_weights_validation_fsv.rs` — fusion-weight validation FSV harness

---

## 1. Identifier Types (`ids.rs`)

All identifiers are stable, serde-serialized as strings, and round-trip through `Display`/`FromStr`.
The crate fixes `ID_BYTES = 16` and `HEX_CHARS = 32` (`ids.rs`).

### 1.1 ID type catalog

| Type | Underlying repr | Display form | Construction | Source |
|------|-----------------|--------------|--------------|--------|
| `VaultId` | `Ulid` (newtype `pub VaultId(pub Ulid)`) | ULID Crockford-base32 string (`self.0` Display) | `from_ulid(Ulid)`, `FromStr` parses a ULID | `ids.rs:42` |
| `LensId` | `[u8; 16]` (via `hex_id!` macro) | 32 lowercase hex chars | `from_bytes`, `from_parts(name, weights_sha256, corpus_hash, output_shape)` | `ids.rs:163` |
| `CxId` | `[u8; 16]` (via `hex_id!` macro) | 32 lowercase hex chars | `from_bytes`, `from_input(input_bytes, panel_version: u32, vault_salt)` | `ids.rs:188` |
| `SlotId` | `u16` (`#[serde(transparent)]`) | decimal | `new(u16)`, `get() -> u16`, `with_key(...)` | `ids.rs:206` |
| `SlotKey` | `{ id: SlotId, key: String }` | n/a (struct) | `new(SlotId, key)`; accessors `id()`, `key()` | `ids.rs:245` |

The `hex_id!` macro (`ids.rs:98`) generates, for `LensId` and `CxId`: `from_bytes`, `to_bytes`,
`as_bytes`, `Debug`, `Display` (lowercase hex via `hex_lower`), `FromStr` (via `parse_hex_16`),
`Serialize` (as string), and `Deserialize` (via `StringIdVisitor`). Derived traits on all four hex/ulid
ids: `Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash`.

### 1.2 Content addressing

`content_address<I, P>(parts) -> [u8; 16]` (`ids.rs:275`) computes a **16-byte truncated BLAKE3** hash
over **length-delimited** ordered byte parts. Each part is prefixed with its length as a big-endian
`u64` before being hashed, then the first 16 bytes of the BLAKE3 digest are returned:

```rust
let mut hasher = blake3::Hasher::new();
for part in parts {
    hasher.update(&(part.len() as u64).to_be_bytes()); // length delimiter
    hasher.update(part);
}
// out = finalize()[..16]
```

Length-delimiting prevents boundary ambiguity (the test `content_address_is_length_delimited` proves
`["ab","c"] != ["a","bc"]`, `ids.rs:407`).

Derived ID formulas (both built on `content_address`):

| ID | Inputs (in order) | Source |
|----|-------------------|--------|
| `LensId::from_parts` | `name.as_bytes()`, `weights_sha256`, `corpus_hash`, `output_shape` | `ids.rs:173` |
| `CxId::from_input` | `input_bytes`, `panel_version.to_be_bytes()`, `vault_salt` | `ids.rs:195` |

Both are deterministic on identical raw input bytes (tests at `ids.rs:415` and `ids.rs:426`); changing
the panel version or output shape changes the id.

### 1.3 `ParseIdError`

| Variant | Meaning | Source |
|---------|---------|--------|
| `InvalidLength { expected, actual }` | input had wrong byte length | `ids.rs:17` |
| `InvalidHex { index }` | non-hex byte at byte index | `ids.rs:19` |
| `InvalidUlid` | not a valid ULID string | `ids.rs:21` |
| `InvalidSlotId` | not a valid `u16` slot id | `ids.rs:23` |

Implements `Display` and `std::error::Error`. Hex parsing (`parse_hex_16`, `ids.rs:334`) accepts upper
and lower case (`hex_value`, `ids.rs:353`).

---

## 2. Error Catalog (`error.rs`)

### 2.1 Core error types

`CalyxError` (`error.rs:13`) is the structured failure payload used across APIs, MCP, and agent
remediation. It is `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]` with
`#[error("{code}: {message}")]`:

| Field | Type | Meaning |
|-------|------|---------|
| `code` | `&'static str` | stable `CALYX_*` wire code |
| `message` | `String` | concrete failure details |
| `remediation` | `&'static str` | stable remediation text |

`pub type Result<T> = std::result::Result<T, CalyxError>` (`error.rs:23`).

`CalyxWarning` (`error.rs:28`) is a non-fatal warning, serde-tagged on `code` with snake_case rename.
Sole variant: `Unprovenanced { surface: String }` (constructor `CalyxWarning::unprovenanced`). Used by
surfaces that must not be labeled trusted.

`CalyxErrorCode` (`error.rs:61`) is a closed enum generated by the `error_catalog!` macro
(`error.rs:51`). The macro derives `Debug, Clone, Copy, PartialEq, Eq, Hash` and generates per-code:
`code() -> &'static str`, `meaning() -> &'static str`, `remediation() -> &'static str`,
`error(message) -> CalyxError`, plus a named constructor on `CalyxError` (e.g.
`CalyxError::lens_dim_mismatch(...)`). `CALYX_ERROR_CODES` (`error.rs:69`) is the ordered slice of all
codes; a test pins it to PRD-18 order exactly (`catalog_matches_prd_18_exactly`, `error.rs:296`).

`CalyxError::from_code(code, message)` builds an error, attaching the code's PRD-18 remediation.

### 2.2 Closed PRD-18 catalog (`CALYX_ERROR_CODES`)

These are the only codes in the closed catalog, in order. "Meaning" is the doc/`meaning()` text;
"Remediation" is the `remediation()` text. All from `error.rs:112`.

| # | Code | Meaning | Remediation |
|---|------|---------|-------------|
| 1 | `CALYX_LENS_FROZEN_VIOLATION` | weights hash != registered | re-register as new LensId |
| 2 | `CALYX_LENS_DIM_MISMATCH` | output dim != Slot.shape | fix lens or slot shape |
| 3 | `CALYX_LENS_NUMERICAL_INVARIANT` | NaN/Inf/non-unit output | check lens runtime/normalize |
| 4 | `CALYX_LENS_UNREACHABLE` | runtime endpoint down | restore lens service |
| 5 | `CALYX_REGISTRY_DUPLICATE` | lens id already registered | reuse existing LensId or register a distinct frozen spec |
| 6 | `CALYX_REGISTRY_UNAVAILABLE` | lens registry unavailable | restore registry before guarded anneal update |
| 7 | `CALYX_ASSAY_INSUFFICIENT_SAMPLES` | < quorum (50) anchors | anchor more outcomes |
| 8 | `CALYX_ASSAY_LOW_SIGNAL` | lens < 0.05 bits | park/retire lens |
| 9 | `CALYX_ASSAY_REDUNDANT` | pair corr > 0.6 | drop duplicate lens |
| 10 | `CALYX_KERNEL_UNGROUNDED` | kernel over ungrounded graph | add anchors (grounding_gaps) |
| 11 | `CALYX_GUARD_PROVISIONAL` | tau not calibrated | calibrate before high-stakes use |
| 12 | `CALYX_GUARD_OOD` | query/output outside trusted region | new-region or reject per policy |
| 13 | `CALYX_FORGE_NUMERICAL_INVARIANT` | kernel NaN/Inf | numerical fail-closed |
| 14 | `CALYX_FORGE_DEVICE_UNAVAILABLE` | CUDA init failed (server mode) | fix driver (reboot per gotcha) |
| 15 | `CALYX_ASTER_CORRUPT_SHARD` | base shard hash mismatch | restore from restic/snapshot |
| 16 | `CALYX_ASTER_TORN_WAL` | torn tail on recovery | auto-discarded; logged |
| 17 | `CALYX_LEDGER_CHAIN_BROKEN` | hash-chain verify failed | quarantine range, investigate |
| 18 | `CALYX_LEDGER_CORRUPT` | ledger CF integrity violation | ledger CF integrity violation — run verify_chain to identify range |
| 19 | `CALYX_LEDGER_APPEND_ONLY_VIOLATION` | ledger CF append-only invariant violated | ledger CF is append-only; deletes and tombstones are forbidden |
| 20 | `CALYX_LEDGER_SECRET_IN_PAYLOAD` | ledger payload contains secret-like material | ledger payload must store hashes/ids only — redact before writing |
| 21 | `CALYX_LEDGER_ACTOR_TOO_LONG` | ledger actor id exceeds 64 UTF-8 bytes | actor id must be <= 64 bytes UTF-8 |
| 22 | `CALYX_LEDGER_GROUP_COMMIT_FAILED` | ledger hook failed during group commit | ledger hook failed — group-commit rolled back; retry the write |
| 23 | `CALYX_REPRODUCE_NONDETERMINISTIC` | reproduce ledger entry lacks determinism seed | no determinism seed in ledger entry - cannot guarantee reproduce fidelity |
| 24 | `CALYX_REPRODUCE_DRIFT_EXCEEDED` | reproduce max_drift exceeded tolerance | reproduce max_drift exceeded 1e-3 - possible lens drift or fusion parameter change |
| 25 | `CALYX_VAULT_ACCESS_DENIED` | cross-vault read without grant | request grant |
| 26 | `CALYX_ERASE_ALREADY_TOMBSTONED` | erase scope already has an erasure tombstone | treat as idempotent erasure or inspect ledger tombstone |
| 27 | `CALYX_STALE_DERIVED` | fresh required, rebuild pending | retry or accept StaleOk |
| 28 | `CALYX_ORACLE_INSUFFICIENT` | I(panel;oracle) < H(Y) - panel can't predict | add outcome/execution lens (propose_lens) |
| 29 | `CALYX_FORGE_VRAM_BUDGET` | dispatch exceeds VRAM budget | split batch / raise budget / wait |
| 30 | `CALYX_BACKPRESSURE` | write/query queue at high-water | retry with backoff |
| 31 | `CALYX_DISK_PRESSURE` | hotpool near full | free/spill to archive; writes fail-closed |
| 32 | `CALYX_QUANT_INTELLIGENCE_LOSS` | quant level would drop bits/cosine/FAR beyond bound | use a gentler level (A25) |
| 33 | `CALYX_READER_LEASE_EXPIRED` | long reader aborted to release MVCC version | re-issue with bounded-staleness snapshot |
| 34 | `CALYX_DATASET_NOT_FOUND` | dataset dir or MANIFEST row missing | acquire + register via scripts/acquire_datasets.sh |
| 35 | `CALYX_DATASET_CHECKSUM_MISMATCH` | recomputed sha256 != recorded value | re-acquire at the pinned revision; never edit dataset bytes in place |
| 36 | `CALYX_DATASET_ROWCOUNT_MISMATCH` | recomputed row count != recorded value | re-acquire at the pinned revision; check split/decoder drift |
| 37 | `CALYX_DATASET_MANIFEST_INVALID` | MANIFEST.md or manifest.json missing/malformed/drifted | re-register via scripts/verify_dataset.sh register |
| 38 | `CALYX_DATASET_SCHEMA_MISMATCH` | dataset columns/fields missing or malformed vs pinned upstream contract | re-acquire at the pinned revision; check upstream schema drift |

Wire shape (test `error_serializes_to_wire_shape`, `error.rs:386`):

```json
{"code":"CALYX_LENS_DIM_MISMATCH","message":"got 384, expected 768","remediation":"fix lens or slot shape"}
```

### 2.3 Module-local `CALYX_*` codes (intentionally NOT in the closed catalog)

These live as `pub const &str` beside their owning module and build `CalyxError` directly. The test
`module_local_codes_are_not_prd_18_catalog_entries` (`error.rs:302`) asserts they are absent from
`CALYX_ERROR_CODES`. PRD 18 must be amended in the same change to promote any of them.

| Code | Owning module / source | Meaning / remediation |
|------|------------------------|-----------------------|
| `CALYX_RECORD_SCHEMA_VIOLATION` | `model/validation.rs:6` | record/schema boundary failure; "submit a constellation matching the record schema with finite values" |
| `CALYX_TEMPORAL_AP60_VIOLATION` | `temporal.rs:6` | AP-60: temporal signals must stay post-retrieval and never dominant |
| `CALYX_TEMPORAL_INVALID_BOOST_CONFIG` | `temporal.rs:7` | boost alpha/causal multipliers out of range |
| `CALYX_TEMPORAL_INVALID_PERIOD` | `temporal.rs:8` | target_hour/day_of_week out of range |
| `CALYX_TEMPORAL_INVALID_WINDOW` | `temporal.rs:9` | empty/invalid temporal window (remediation defined; no code-path validator in this file) |
| `CALYX_TEMPORAL_NEGATIVE_WEIGHT` | `temporal.rs:10` | a fusion weight is negative |
| `CALYX_TEMPORAL_WEIGHT_SUM` | `temporal.rs:11` | fusion weights non-finite or not summing to 1.0 |
| `CALYX_AUTHN_REQUIRED` | `security.rs:22` | mutation without authenticated principal |
| `CALYX_TLS_CONFIG_INVALID` | `security.rs:25` | TLS cert/key/CA path missing or unreadable |
| `CALYX_CONSENT_VIOLATION` | `consent.rs:10` | purpose not permitted or consent expired |
| `CALYX_PROVISIONAL_VAULT` | `cold_start.rs:11` | high-stakes use of an unanchored vault |
| `CALYX_ALLOC_CAP_EXCEEDED` | `alloc/mod.rs:31` | allocation would exceed its owner's hard cap (A26) |
| `CALYX_CACHE_EVICTED` | `cache/lru_ttl.rs:23` | structured log event on LRU eviction (not a panic) |

The `error.rs` test module also references additional module-local codes owned by *other crates*
(e.g. `CALYX_MCP_JSONRPC_INVALID`, `CALYX_PII_REDACTION_REQUIRED`, `CALYX_SBOM_PARSE_ERROR`,
`CALYX_SUPPLY_CHAIN_VULN`, `CALYX_LENS_WEIGHT_TAMPERED`, `CALYX_EXTERNAL_CMD_NOT_ALLOWED`) — these are
named in `MODULE_LOCAL_CODES` (`error.rs:275`) only to assert they are not catalog entries; they are
not defined in `calyx-core`.

---

## 3. Shared Enum Vocabulary (`enums.rs`)

All enums derive serde with `#[serde(rename_all = "snake_case")]` (except `AnchorKind`/`AbsentReason`
which carry data). Byte-stable serialization is pinned by tests (`enums.rs:148`).

### 3.1 `Modality` (`enums.rs:10`)

Closed set (test `modality_variant_set_is_locked`, `enums.rs:184`):
`Text`, `Code`, `Image`, `Audio`, `Video`, `Protein`, `Dna`, `Molecule`, `Structured`, `Mixed`
→ wire strings `"text"`, `"code"`, `"image"`, `"audio"`, `"video"`, `"protein"`, `"dna"`,
`"molecule"`, `"structured"`, `"mixed"`.

### 3.2 `SlotShape` (`enums.rs:36`) — physical vector shape

| Variant | Wire JSON | Meaning |
|---------|-----------|---------|
| `Dense(u32)` | `{"dense":768}` | dense vector, fixed dim |
| `Sparse(u32)` | `{"sparse":N}` | sparse vector, fixed ambient dim |
| `Multi { token_dim: u32 }` | `{"multi":{"token_dim":128}}` | multi-vector token rep |

### 3.3 `Asymmetry` (`enums.rs:48`)

`None` (symmetric) | `Dual { a: SlotId, b: SlotId }` (directed dual-slot relation;
`{"dual":{"a":1,"b":2}}`).

### 3.4 `QuantPolicy` (`enums.rs:58`)

| Variant | Wire JSON | Notes |
|---------|-----------|-------|
| `None` | `"none"` | unquantized |
| `TurboQuant { bits_per_channel_x2: u8 }` | `{"turbo_quant":{"bits_per_channel_x2":7}}` | `x2` field: 7 means 3.5 bpc |
| `MxFp4` | `"mx_fp4"` | Blackwell microscaling FP4; MXFP8 safe fallback |
| `Pq { m: u8, nbits: u8 }` | `{"pq":{"m":8,"nbits":4}}` | product quantization |
| `Float8` | `"float8"` | |
| `Binary` | `"binary"` | |

`QuantPolicy::turboquant_default()` returns `TurboQuant { bits_per_channel_x2: 7 }` (the
quality-neutral default; `enums.rs:75`).

### 3.5 `AnchorKind` (`enums.rs:85`) — grounded outcome axis

Derives `PartialOrd, Ord` in addition to the usual set. Closed set (test at `enums.rs:216`):
`TestPass`, `TieFormed`, `Thumbs`, `Label(String)`, `Reward`, `SpeakerMatch`, `StyleHold`,
`Recurrence`. Wire keys: `test_pass`, `tie_formed`, `thumbs`, `label`, `reward`, `speaker_match`,
`style_hold`, `recurrence`. When used as a `bits_about` map key (see §5.4) `Label("gold")` encodes as
the flat string `"label:gold"`.

### 3.6 `SlotState` (`enums.rs:107`) — panel slot lifecycle

| Variant | Wire | Meaning |
|---------|------|---------|
| `Active` | `"active"` | participates in new ingests and reads |
| `Parked` | `"parked"` | parked but still interpretable for old constellations |
| `Retired` | `"retired"` | tombstoned for future use; historical data still readable |

### 3.7 `AbsentReason` (`enums.rs:119`) — why a slot vector is absent

`NotApplicable`, `Redacted`, `LensUnavailable`, `Deferred`, `LensInactive`, `Error(String)`.
Wire: snake_case; `Error("CALYX_LENS_DIM_MISMATCH")` → `{"error":"CALYX_LENS_DIM_MISMATCH"}`.

---

## 4. Engine Trait Boundaries (`traits.rs`)

All four engine traits require `Send + Sync` and are object-safe (test
`engine_traits_are_object_safe`, `traits.rs:105`).

### 4.1 `Input` (`traits.rs:11`)

Raw input presented to a frozen lens: `{ modality: Modality, bytes: Vec<u8>, pointer: Option<String> }`.
Builders: `Input::new(modality, bytes)`, `.with_pointer(p)`.

### 4.2 Trait method signatures

| Trait | Method | Signature | Source |
|-------|--------|-----------|--------|
| `Lens` | `id` | `fn id(&self) -> LensId` | `traits.rs:39` |
| `Lens` | `shape` | `fn shape(&self) -> SlotShape` | |
| `Lens` | `modality` | `fn modality(&self) -> Modality` | |
| `Lens` | `measure` | `fn measure(&self, input: &Input) -> Result<SlotVector>` | |
| `Lens` | `measure_batch` | `fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>>` (default: maps `measure`) | |
| `Index` | `insert` | `fn insert(&mut self, cx: CxId, vector: &SlotVector) -> Result<()>` | `traits.rs:58` |
| `Index` | `search` | `fn search(&self, query: &SlotVector, k: usize, ef: Option<usize>) -> Result<Vec<(CxId, f32)>>` | |
| `Index` | `rebuild` | `fn rebuild(&mut self) -> Result<()>` | |
| `VaultStore` | `put` | `fn put(&self, constellation: Constellation) -> Result<CxId>` | `traits.rs:70` |
| `VaultStore` | `get` | `fn get(&self, id: CxId, snapshot: Seq) -> Result<Constellation>` | |
| `VaultStore` | `anchor` | `fn anchor(&self, id: CxId, anchor: Anchor) -> Result<()>` | |
| `VaultStore` | `snapshot` | `fn snapshot(&self) -> Seq` | |
| `Estimator` | `mi` | `fn mi(&self, x: &[SlotVector], y: &[Anchor]) -> Result<Signal>` | `traits.rs:85` |
| `Estimator` | `redundancy` | `fn redundancy(&self, a: &[SlotVector], b: &[SlotVector]) -> Result<f32>` | |

`Lens` is the frozen-measurement-instrument boundary (implemented by Registry lens runtimes); `Index`
the per-slot ANN/inverted index; `VaultStore` the Aster storage (group-commit `put`, MVCC `get` at a
`snapshot` `Seq`); `Estimator` the Assay information-signal estimator. These are implemented in sibling
crates — see [05_aster_storage.md](05_aster_storage.md) for `VaultStore`, and the Registry/Assay specs
for `Lens`/`Estimator`.

---

## 5. Constellation Data Model (`model/`)

The constellation is the atomic Calyx record: one input measured by one panel of frozen lenses.

### 5.1 `Constellation` (`model/constellation.rs:20`)

`#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]`. JSON round-trips byte-exactly
(proptest at `model/mod.rs:31`).

| Field | Type | Meaning |
|-------|------|---------|
| `cx_id` | `CxId` | content-addressed constellation id |
| `vault_id` | `VaultId` | owning vault |
| `panel_version` | `u32` | panel version used (must be > 0 at schema validation) |
| `created_at` | `Ts` | server-stamped creation timestamp |
| `input_ref` | `InputRef` | input hash + optional pointer + redacted flag |
| `modality` | `Modality` | input modality |
| `slots` | `BTreeMap<SlotId, SlotVector>` | per-slot vectors; absent slots are explicit |
| `scalars` | `BTreeMap<String, f64>` | scalar measurements derived at ingest |
| `metadata` | `BTreeMap<String, String>` (`#[serde(default)]`) | verbatim string ids / source-system metadata |
| `anchors` | `Vec<Anchor>` | grounded outcomes for this input |
| `provenance` | `LedgerRef` | ledger entry proving input → lens → constellation lineage |
| `flags` | `CxFlags` | trust/degradation flags |

Helpers: `metadata_value(key) -> Option<&str>`, `chunk_id()` (reads `METADATA_CHUNK_ID = "chunk_id"`),
`database_name()` (reads `METADATA_DATABASE_NAME = "database_name"`). These two metadata keys are the
Leapable Vault contract keys (`model/constellation.rs:14`).

`validate_schema()` (`model/constellation.rs:65`) enforces, returning `CALYX_RECORD_SCHEMA_VIOLATION`:
`panel_version != 0`; every slot vector validates; scalar keys non-empty and values finite; metadata
keys non-empty; every anchor validates.

### 5.2 `SlotVector` (`model/vector.rs:23`) and `SparseEntry`

`#[serde(rename_all = "snake_case")]`. Absence is an explicit value — never a zero vector.

| Variant | Fields | Wire JSON |
|---------|--------|-----------|
| `Dense` | `dim: u32, data: Vec<f32>` | `{"dense":{...}}` |
| `Sparse` | `dim: u32, entries: Vec<SparseEntry>` | `{"sparse":{...}}` |
| `Multi` | `token_dim: u32, tokens: Vec<Vec<f32>>` | `{"multi":{...}}` |
| `Absent` | `reason: AbsentReason` | `{"absent":{"reason":"deferred"}}` |

`SparseEntry { idx: u32, val: f32 }`. Helpers: `is_absent()`, `as_dense() -> Option<&[f32]>` (only for
`Dense`). `validate_schema()` (`model/vector.rs:52`) enforces, per variant: dense `dim > 0` and
`data.len() == dim` and all finite; sparse `dim > 0`, every `idx < dim`, no duplicate idx, every val
finite; multi `token_dim > 0`, at least one token, every token length `== token_dim` and finite;
`Absent` always OK.

### 5.3 `Anchor` and `AnchorValue` (`model/anchor.rs`)

`Anchor { kind: AnchorKind, value: AnchorValue, source: String, observed_at: Ts, confidence: f32 }`.
`confidence` is in `[0, 1]`; deterministic oracles use `1.0`. `validate_schema()` requires confidence
finite and in `[0,1]`, then validates the value (`model/anchor.rs:44`).

`AnchorValue` (`#[serde(rename_all = "snake_case")]`, `model/anchor.rs:27`):

| Variant | Validation in `validate_schema` |
|---------|---------------------------------|
| `Bool(bool)` | always ok |
| `Enum(String)` | always ok |
| `Number(f64)` | rejects NaN/Inf |
| `OneHot(Vec<String>)` | always ok |
| `Text(String)` | always ok |
| `Vector(Vec<f32>)` | rejects empty and any NaN/Inf |

### 5.4 `Slot`, `Panel`, and supporting types (`model/slot.rs`)

`Slot` (`model/slot.rs:87`) — a frozen lens slot in a panel:

| Field | Type | Notes |
|-------|------|-------|
| `slot_id` | `SlotId` | compact panel slot id |
| `slot_key` | `SlotKey` | stable human-readable key + id |
| `lens_id` | `LensId` | frozen lens content id |
| `shape` | `SlotShape` | physical vector shape |
| `modality` | `Modality` | measured modality |
| `asymmetry` | `Asymmetry` | directional relation |
| `quant` | `QuantPolicy` | quantization policy |
| `resource` | `SlotResource` | `#[serde(default, skip_serializing_if = "SlotResource::is_default")]` |
| `axis` | `Option<String>` | semantic axis/grouping tag |
| `retrieval_only` | `bool` (`#[serde(default)]`) | post-retrieval signal only, not primary recall |
| `excluded_from_dedup` | `bool` (`#[serde(default)]`) | must not drive dedup |
| `bits_about` | `BTreeMap<AnchorKind, Signal>` | Assay signal by outcome axis (custom serde, see below) |
| `state` | `SlotState` | lifecycle state |
| `added_at_panel_version` | `u32` | panel version that introduced this slot |

`bits_about` uses the `anchor_signal_map` custom serde module (`model/slot.rs:137`): keys are encoded
as flat strings (`AnchorKind::Label(v)` → `"label:v"`, others → their snake_case name). Deserialization
rejects unknown keys and duplicate anchor kinds.

`Panel` (`model/slot.rs:124`): `{ version: u32, slots: Vec<Slot>, created_at: Ts, kernel_ref:
Option<LedgerRef>, guard_ref: Option<LedgerRef> }`.

`LensCost` (`model/slot.rs:16`): `total_ms: f32`, `ms_per_input: f32`, `vram_bytes: u64`,
`ram_bytes: u64`, `batch_ceiling: u32` (all `#[serde(default)]`). `LensCost::zero()` sets
`batch_ceiling = u32::MAX` and all else 0; `is_zero_cost()` checks ms + memory are all zero.
`Default for LensCost` = `zero()`.

`Placement` (`model/slot.rs:62`): `Cpu` (default) | `Gpu`; snake_case.

`SlotResource` (`model/slot.rs:70`): `{ cost: LensCost, placement: Placement }`, both
`#[serde(default)]`; `is_default()` used by the slot's skip-serialize.

### 5.5 Signal and reference structs (`model/signal.rs`)

| Struct | Fields | Source |
|--------|--------|--------|
| `ConfidenceInterval` | `low: f32, high: f32` | `model/signal.rs:9` |
| `Signal` | `bits: f32, ci: ConfidenceInterval, n: usize, estimator: String, ts: Ts` | `model/signal.rs:18` |
| `InputRef` | `hash: [u8; 32], pointer: Option<String>, redacted: bool` | `model/signal.rs:33` |
| `LedgerRef` | `seq: u64, hash: [u8; 32]` | `model/signal.rs:44` |
| `CxFlags` | `ungrounded: bool, degraded: bool, novel_region: bool, redacted_input: bool` (all default false) | `model/signal.rs:53` |

`CxFlags` derives `Default`; `Signal.bits` is "bits above baseline". `LedgerRef` is an append-only
ledger reference — see the ledger spec for the hash-chain. `Signal` byte-determinism under a
`FixedClock` is proven at `time.rs:56`.

---

## 6. Clock Abstraction (`time.rs`)

| Item | Definition | Source |
|------|------------|--------|
| `Seq` | `pub type Seq = u64` — monotonic store sequence number | `time.rs:6` |
| `Ts` | `pub type Ts = u64` — server timestamp in **Unix milliseconds** | `time.rs:9` |
| `Clock` | trait `Send + Sync` with `fn now(&self) -> Ts` | `time.rs:12` |
| `SystemClock` | real wall-clock; `now()` reads `SystemTime` since `UNIX_EPOCH` as millis, saturating to `Ts::MAX` on overflow; panics if system clock predates the epoch | `time.rs:19` |
| `FixedClock` | `FixedClock::new(ts)`; `now()` returns the fixed `ts` — deterministic for tests/FSV | `time.rs:33` |

The injected `Clock` is the mechanism for byte-deterministic timestamped logic: subsystems take a
`Clock` (often `Arc<dyn Clock>`) rather than calling `SystemTime::now()` directly. The cache (§8) is a
concrete consumer.

---

## 7. Cosine Helper (`cosine.rs`)

`dense_cosine(left: &[f32], right: &[f32]) -> Option<f32>` (`cosine.rs:19`) fails closed (returns
`None`) when: lengths differ, slice is empty, any element is non-finite, or the denominator (product of
norms) is non-finite or `<= 0`. Otherwise returns the cosine, again `None` if the result is non-finite.

`GuardTauProfile` (`cosine.rs:8`): trait `fn tau_for(&self, slot: &SlotId) -> Option<f32>`, with a
blanket impl for `BTreeMap<SlotId, f32>`. Lets guard-like policies look up a per-slot tau threshold
without coupling to the guard crate.

---

## 8. Bounded Allocation Primitives (`alloc/`)

Every transient/hot allocation flows through these so that every allocation has an owner and a hard
bound (axiom A26); cap violations surface `CALYX_ALLOC_CAP_EXCEEDED` (`alloc/mod.rs:31`) — never a
panic, silent realloc, or grow. `alloc_cap_exceeded(message)` builds that error with remediation
"raise the cap or shrink the working set; allocations fail closed (A26)".

`AllocStats` (`alloc/mod.rs:45`): `{ arena_high_water_bytes: usize, arena_resets: u64 }` — the
Source-of-Truth read for FSV and the Prometheus metrics surface.

### 8.1 `Arena` / `ArenaVec` (`alloc/arena.rs`)

A bump allocator over one pre-allocated, over-aligned block. `ARENA_BASE_ALIGN = 4096`
(`alloc/arena.rs:24`); the backing block is allocated zeroed and aligned to it, so returned-pointer
alignment is a deterministic function of the cursor.

| Method | Behavior |
|--------|----------|
| `Arena::new(cap)` | rejects `cap == 0` with `CALYX_ALLOC_CAP_EXCEEDED` |
| `alloc(size, align)` | `align` must be a power of two; zero-size is a no-op returning an aligned dangling pointer; fails closed if the padded request crosses the cap, **without advancing the cursor** |
| `alloc_vec::<T>(capacity)` | returns an `ArenaVec` borrowing the arena; ZST/zero-capacity needs no bytes |
| `reset()` | rewinds cursor to 0 in O(1), no `free`, increments `resets`; invalidates prior pointers |
| `used()` / `capacity()` / `high_water()` | live cursor / cap / peak ever consumed (survives reset) |
| `stats()` | `AllocStats` snapshot |

`ArenaVec<'arena, T>` (`alloc/arena.rs:209`): typed fixed-capacity vector over arena bytes. `push`
fails closed at capacity (never grows); `len`, `is_empty`, `capacity`, `as_slice`, `as_mut_slice`. Its
`Drop` drops the initialized elements in place so owned resources don't leak.

### 8.2 `SlabPool` / `PageAlignedSlabPool` (`alloc/slab.rs`)

Fixed-size object pools handing out slots via a free list (RAII guards), failing closed on exhaustion.

Constants/types:

| Item | Value / definition | Source |
|------|--------------------|--------|
| `PAGE_SIZE` | `4096` | `alloc/slab.rs:30` |
| `DEFAULT_EMBED_DIM` | `768` | `alloc/slab.rs:34` |
| `VEC_BLOCK_SIZE` | `DEFAULT_EMBED_DIM * 4` (= 3072) | `alloc/slab.rs:37` |
| `VecBlockPool` | `SlabPool<VEC_BLOCK_SIZE>` | `alloc/slab.rs:40` |
| `AnnNode` | `#[repr(C)] { id: u64, neighbors: [u32; 32], level: u16, pad: [u8; 6] }` | `alloc/slab.rs:45` |
| `ANN_NODE_SIZE` | `size_of::<AnnNode>()` | `alloc/slab.rs:57` |
| `AnnNodePool` | `SlabPool<ANN_NODE_SIZE>` | `alloc/slab.rs:60` |

`SlabPool<const SLOT_SIZE>` (`alloc/slab.rs:113`): `new(cap_slots)` rejects 0; `acquire() ->
Result<SlabGuard>` fails closed on exhaustion; `cap_slots()`, `held()`, `utilization() -> f64`.
`SlabGuard` derefs to `&mut [u8; SLOT_SIZE]` and returns the slot on drop. Internally slots live in
`UnsafeCell`s; the free list guarantees at most one live guard per slot index. A release of a
non-`Held` slot triggers a debug assertion (double-release detection, `alloc/slab.rs:91`).

`PageAlignedSlabPool` (`alloc/slab.rs:214`): 4 KiB-aligned variant for pinned-host CUDA staging.
`new(slot_size, cap_slots)` **panics** if `slot_size` is not a non-zero multiple of `PAGE_SIZE`, and
errors if `cap_slots == 0`. `acquire() -> PageSlabGuard` (page-aligned pointer via `as_mut_ptr` /
`as_mut_slice`). Marked `unsafe impl Send` (owns its allocation; not `Sync` because of `RefCell`).

---

## 9. Bounded Cache (`cache/`)

`LruTtlCache<K, V>` (`cache/lru_ttl.rs:45`) is the single bounded-cache type for all cached artifacts
(A26): a hard byte cap, LRU eviction, and a per-entry TTL driven by an injected `Clock`. Recency is an
intrusive doubly-linked list over a node arena (O(1) get/insert/evict); no external map crate and no
`SystemTime::now()` in logic.

| Method | Behavior | Source |
|--------|----------|--------|
| `new(byte_cap, ttl, clock)` | rejects `byte_cap == 0` with `CALYX_ALLOC_CAP_EXCEEDED`; zero jitter | `cache/lru_ttl.rs:72` |
| `with_jitter(byte_cap, ttl, jitter, clock)` | per-entry TTL randomized by ±jitter/2 to avoid cache-stampede; seeded RNG | `cache/lru_ttl.rs:80` |
| `get(&mut, key)` | TTL-expired entry is removed and reported as a miss; live hit promoted to MRU | `cache/lru_ttl.rs:110` |
| `insert(key, value, size_bytes)` | errors if a single entry exceeds the cap; evicts expired then LRU until it fits; returns `InsertResult { evicted }` | `cache/lru_ttl.rs:136` |
| `evict_expired()` | sweeps all TTL-expired entries; returns count | `cache/lru_ttl.rs:173` |
| `len`/`is_empty`/`used_bytes`/`byte_cap` | accounting (`used_bytes` never exceeds cap — the FSV SoT) | |
| `hit_rate()` | `hits/(hits+misses)`, 0.0 before any access | `cache/lru_ttl.rs:207` |
| `evictions()` / `expired_total()` | monotonic counters (`cache_evictions_total` SoT) | |

`InsertResult { evicted: usize }` (`cache/lru_ttl.rs:30`). `CALYX_CACHE_EVICTED`
(`cache/lru_ttl.rs:23`) is emitted as a `tracing::debug!` structured event on each eviction (never a
panic). The jitter RNG is a `ChaCha8Rng` seeded with the fixed `JITTER_SEED = 0xCA17_8C0F_FEE5_1D0F`
so jittered TTLs are reproducible in FSV. Behavior is exercised in `cache/lru_ttl/tests.rs` (byte-cap
LRU eviction, TTL expiry via an advancing clock).

---

## 10. Temporal Policy Contracts (`temporal.rs`)

Contracts for post-retrieval temporal boosting. AP-60 invariant: temporal signals must stay
post-retrieval and never dominant. All validators return module-local `CALYX_TEMPORAL_*` codes (§2.3)
via `temporal_error(code, message)` (`temporal.rs:469`), which also maps each code to its remediation.

Internal constants (`temporal.rs:13`): `WEIGHT_SUM_EPSILON = 1.0e-6`, `DEFAULT_HALF_LIFE_SECS = 3600`,
`DEFAULT_POST_RETRIEVAL_ALPHA = 0.10`, `MAX_POST_RETRIEVAL_ALPHA = 0.10`, `MAX_CAUSAL_MULTIPLIER =
10.0`, `DEFAULT_RECURRENCE_WEIGHT = 0.05`, `DEFAULT_MAX_RECURRENCE_BOOST = 0.10`.

### 10.1 Enums

| Enum | Variants | Default | Source |
|------|----------|---------|--------|
| `DecayFunction` | `Linear { max_age_secs: u64 }`, `Exponential { half_life_secs: u64 }`, `Step` | `Exponential { half_life_secs: 3600 }` | `temporal.rs:23` |
| `SequenceDirection` | `Forward`, `Backward` | (used in `SequenceOptions`) | `temporal.rs:115` |
| `MultiAnchorMode` | `First`, `Last`, `All` | (used in `SequenceOptions`) | `temporal.rs:122` |

### 10.2 Validated config structs

| Struct | Fields | Validation rule | Default |
|--------|--------|-----------------|---------|
| `PeriodicOptions` | `target_hour: Option<u8>`, `target_day_of_week: Option<u8>`, `use_now: bool` | hour `0..=23`, dow `0..=6` → else `CALYX_TEMPORAL_INVALID_PERIOD` | `{ None, None, use_now: true }` |
| `SequenceOptions` | `direction`, `multi_anchor_mode` | none | `{ Forward, First }` |
| `FusionWeights` | `recency, sequence, periodic: f32` | all finite & non-negative & sum to 1.0 ±1e-6 → else `CALYX_TEMPORAL_WEIGHT_SUM` / `CALYX_TEMPORAL_NEGATIVE_WEIGHT` | `{ 0.50, 0.35, 0.15 }` |
| `BoostConfig` | `post_retrieval_alpha, causal_high_mult, causal_low_mult: f32` | alpha finite in `0.0..=0.10`; high finite in `1.0..=10.0` (exclusive of 1.0); low finite in `0.0..1.0`; low < high | `{ 0.10, 1.10, 0.85 }` |
| `RecurrenceBoostConfig` | `frequency_weight, recency_weight, max_recurrence_boost: f32` | all finite, non-negative, max ≤ 0.10 → else `CALYX_TEMPORAL_INVALID_BOOST_CONFIG` | `{ 0.05, 0.05, 0.10 }` |
| `TemporalPolicy` | `enabled, decay, periodic, sequence, fusion_weights, boost, recurrence_boost: Option<...>, never_dominant` | `never_dominant` must be true (AP-60) → else `CALYX_TEMPORAL_AP60_VIOLATION`; then validates each sub-config | see `temporal.rs:420` |

Every config validates on construction (`new`) and on deserialization (each has a hand-written
`Deserialize` that calls `validate()`), so an invalid temporal config cannot be deserialized.
`FusionWeights` validation is independently exercised by the FSV harness
`tests/fusion_weights_validation_fsv.rs`. The `default_post_retrieval_alpha`/`default_recurrence_boost`
serde defaults supply `0.10` / `Some(RecurrenceBoostConfig::default())` for absent fields.

---

## 11. Transport Security & AuthN (`security.rs`)

Canonical transport-security/identity types (PRD 30 §2), placed in `calyx-core` so `calyx-aster` and
`calyxd` can share them without a cycle. Module-local codes: `CALYX_AUTHN_REQUIRED`,
`CALYX_TLS_CONFIG_INVALID` (§2.3).

`AuthN` (`security.rs:86`) — the three permitted principal identity modes:

| Variant | Fields | `is_server_mode()` |
|---------|--------|--------------------|
| `InProcess` | `host_app_id: String` | false |
| `MtlsToken` | `fingerprint: [u8; 32]` (SHA-256 of verified client cert) | true |
| `CloudflareAccess` | `service_token_id: String` | true |

`no_anonymous_write(authn: Option<&AuthN>) -> Result<()>` (`security.rs:122`): the no-anonymous-write
gate every mutation entry point must satisfy. Returns `Ok(())` when **any** identity is present
(presence, not content — even an empty `host_app_id` passes) and `CALYX_AUTHN_REQUIRED` for `None`.
Fail-closed (A16); it must never return `Ok(())` for `None`.

`TlsConfig` (`security.rs:30`): `{ cert_pem_path, key_pem_path, ca_pem_path: Option<PathBuf> }`.
`validate()` is metadata-only — it checks each path exists and is a regular readable file (does not
parse PEM contents), returning `CALYX_TLS_CONFIG_INVALID` naming the first offending path. `Some`
`ca_pem_path` enables mutual TLS.

`MtlsConfig` (`security.rs:73`): `{ tls: TlsConfig, require_client_cert: bool }`.

---

## 12. Consent & Purpose Gating (`consent.rs`)

Privacy gating (PH61). `Timestamp = Ts` (Unix milliseconds). Module-local code
`CALYX_CONSENT_VIOLATION` (§2.3).

`LawfulBasis` (`consent.rs:18`): `Consent`, `LegitimateInterest`, `ContractPerformance`,
`LegalObligation`, `VitalInterests`, `PublicTask` (snake_case serde + matching `Display`).

`Purpose` (`consent.rs:43`): `Search`, `Intelligence`, `Reranking`, `Analytics`, `Export`,
`AuditOnly`.

`ConsentTag` (`consent.rs:54`): `{ lawful_basis: LawfulBasis, permitted_purposes: Vec<Purpose>,
expires_at: Option<Timestamp> }`.

`consent_expired(tag, now) -> bool` (`consent.rs:61`): true when `expires_at` is `Some` and
`now >= expires_at`.

`check_consent(tag, requested_purpose, now) -> Result<()>` (`consent.rs:66`): fail-closed.
`Purpose::AuditOnly` is always permitted; otherwise an expired tag fails, and a purpose not in
`permitted_purposes` fails — both with `CALYX_CONSENT_VIOLATION`.

---

## 13. Cold-Start Trust Guard (`cold_start.rs`)

Provisional-vault guard (PRD 30 §5). A new vault can search immediately but must not label high-stakes
answers grounded until at least one real anchor exists. Module-local code `CALYX_PROVISIONAL_VAULT`
(§2.3).

`VaultTrustState` (`cold_start.rs:16`): `Provisional` | `Grounded { anchor_count: usize }`.

`ColdStartGuard` (`cold_start.rs:25`) — fail-closed:

| Method | Behavior |
|--------|----------|
| `new()` / `Default` | starts `Provisional` |
| `state()` | current `&VaultTrustState` |
| `anchor_count()` | 0 when provisional, else the grounded count |
| `record_anchor()` | saturating-increments the count and transitions to `Grounded` (count 1 on first call) |
| `assert_grounded(operation)` | `Ok` only when `Grounded` with `anchor_count >= 1`; else `CALYX_PROVISIONAL_VAULT` naming the operation |
| `search_always_ok()` | always `true` — search permitted from day zero |

---

## 14. Crate Root Re-Exports (`lib.rs`)

`lib.rs` declares the modules (`alloc`, `cache`, `cold_start`, `consent`, `cosine`, `enums`, `error`,
`ids`, `model`, `security`, `temporal`, `time`, `traits`) and flattens the public surface via `pub use`
so consumers import everything from the crate root (e.g. `calyx_core::{CxId, CalyxError, Constellation,
Clock, ...}`). The full re-export list is at `lib.rs:17-47`. The crate's own doc line describes it as
"Core Calyx identifiers, model contracts, and shared types."

---

## 15. Cross-References

- Storage / `VaultStore` / ledger / MVCC snapshots: see [05_aster_storage.md](05_aster_storage.md).
- `Lens`, `LensId`, frozen lens specs, and `CALYX_LENS_*` / `CALYX_REGISTRY_*` handling: see the
  Registry spec.
- `Estimator`, `Signal`, `bits_about`, and `CALYX_ASSAY_*` / `CALYX_ORACLE_INSUFFICIENT`: see the Assay
  spec.
- `CALYX_GUARD_*`, OOD/tau calibration, and `GuardTauProfile` consumers: see the Guard/Ward spec.
- Temporal boosting consumers of `TemporalPolicy`: see the temporal/query spec.

> Anything in this document marked **Not determined from source** was not establishable from the
> `calyx-core` source alone and must be confirmed against the consuming crate.
