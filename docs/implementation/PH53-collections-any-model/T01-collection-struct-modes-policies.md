# PH53 · T01 — `Collection` struct, `CollectionMode`, schema, and policies

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model (relational/doc/KV/TS/blob) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/collection/mod.rs` (≤500), `crates/calyx-aster/src/collection/schema.rs` (≤500), `crates/calyx-aster/src/collection/policy.rs` (≤500) |
| **Depends on** | PH03 (error catalog, `CALYX_*` codes), PH04 (core types, VaultId) |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/03 §0`, `dbprdplans/20 §3` |

## Goal

Define the `Collection` struct (verbatim from `03 §0`) as the single container
that expresses any data model. Implement `create_collection` which persists the
collection descriptor into a dedicated `collections` CF row (key = collection
`name`, value = bincode-encoded `Collection` metadata). Schema enforcement
types (`SchemaFull | SchemaLess`) and all policy types (`DedupPolicy`,
`TemporalPolicy`, `RetentionPolicy`, `TxnPolicy`) are defined here and validated
at creation; once written they are immutable.

## Build (checklist of concrete, code-level steps)

- [ ] Define `CollectionMode` enum:
  ```rust
  pub enum CollectionMode { Records, Documents, KV, TimeSeries, Blob, Constellations }
  ```
- [ ] Define `Schema` enum: `SchemaFull(Vec<FieldDef>)` | `SchemaLess`. A
  `FieldDef` holds `name: String`, `ty: FieldType` (Bool|I64|F64|Text|Bytes|
  Timestamp), `nullable: bool`.
- [ ] Define `Collection` struct verbatim from `dbprdplans/03 §0`:
  ```rust
  pub struct Collection {
      pub name: String,
      pub mode: CollectionMode,
      pub schema: Option<Schema>,           // SchemaFull | SchemaLess
      pub panel: Option<PanelRef>,          // None/0 lenses = plain store; ≥1 lens = constellation store
      pub indexes: Vec<SecondaryIndexSpec>, // btree | inverted | ANN | kernel
      pub dedup: DedupPolicy,
      pub temporal: TemporalPolicy,
      pub retention: RetentionPolicy,
      pub txn_policy: TxnPolicy,
      pub tenant: TenantId,
  }
  ```
- [ ] Define `DedupPolicy` enum: `Off` | `Exact` |
  `TctCosine { required_slots: Vec<SlotKey>, tau: f32, action: DedupAction }`.
  `DedupAction`: `Reject` | `RecurrenceSeries` | `Merge`.
- [ ] Define `TemporalPolicy` struct: `boost_weights: [f32; 3]` (AP-60:
  `[0.50, 0.35, 0.15]` for E2/E3/E4 default). Temporal lenses are **not**
  dominant in retrieval weight (enforced: sum of boost_weights ≤ 1.0).
- [ ] Define `RetentionPolicy` enum: `Forever` | `DropAfter(Duration)` |
  `RollupOnly`.
- [ ] Define `TxnPolicy` struct: `isolation: IsolationLevel` (ReadCommitted |
  Serializable), `cost_cap_ms: Option<u32>`.
- [ ] Define `TenantId(u64)` newtype; 0 = default single-tenant.
- [ ] Implement `create_collection(vault: &AsterVault, col: Collection) -> Result<()>`:
  - Validate `col.name` non-empty, ≤128 UTF-8 bytes.
  - If `SchemaFull`, validate ≥1 field defined.
  - If `DedupPolicy::TctCosine`, validate `0.0 < tau <= 1.0`.
  - If `col.panel.is_some()`, mode must be `Constellations`; otherwise mode
    must not be `Constellations` (plain store).
  - Encode `Collection` with `bincode` into `collections` CF; key =
    `b"coll\x00" ++ name.as_bytes()`.
  - Write in the group-commit WAL batch; fail closed with
    `CALYX_COLLECTION_ALREADY_EXISTS` on duplicate name.
- [ ] Implement `get_collection(vault: &AsterVault, name: &str) -> Result<Collection>`;
  fail closed with `CALYX_COLLECTION_NOT_FOUND` on missing.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `create_collection` on `name="orders"`, `mode=Records`,
  `schema=SchemaFull([{name:"pk",ty:I64,nullable:false}])` → encode round-trip
  produces exact bincode bytes; `get_collection("orders")` returns identical struct.
- [ ] proptest: `bincode::decode(bincode::encode(col)) == col` for randomly
  generated `Collection` (all modes, all policy variants).
- [ ] edge (≥3): (1) `name=""` → `CALYX_INVALID_ARGUMENT`; (2) `name` = 129
  UTF-8 bytes → `CALYX_INVALID_ARGUMENT`; (3) `mode=Constellations` with
  `panel=None` → `CALYX_INVALID_ARGUMENT`; (4) duplicate `create_collection`
  same name → `CALYX_COLLECTION_ALREADY_EXISTS`.
- [ ] fail-closed: `DedupPolicy::TctCosine { tau: -0.1, … }` →
  `CALYX_INVALID_ARGUMENT`; `tau: 1.1` → `CALYX_INVALID_ARGUMENT`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `collections` CF row in the vault SST.
- **Readback:**
  ```
  calyx collection create --vault /home/croyse/calyx/test-vault --name orders --mode records --schema full
  xxd /home/croyse/calyx/test-vault/cf/collections/000001.sst | head -8
  calyx readback --cf collections --vault /home/croyse/calyx/test-vault
  ```
- **Prove:** The `xxd` output contains the ASCII bytes of `"orders"` at the
  expected key offset; `get_collection("orders")` after a vault restart returns
  the identical `Collection` struct (schema, mode, policies unchanged).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH53 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
