# 18 — API, Core Types & Error Catalog

Concrete Rust surface. Illustrative (not final), enough to start implementation. Names are normative; signatures are a starting contract.

> **File-size rule (DOCTRINE §8):** every `.rs` source/test file MUST be ≤ 500 lines; documentation unlimited. Within each crate, split modules by single responsibility into subdirectories with a `mod.rs` facade (explicit `pub use` re-exports, no wildcard, no circular deps). A file approaching the limit is modularized per `docs2/modulateprompt.md`; an over-limit file gets a GitHub issue and is fixed. (Hence many small modules, not monoliths.)

## 1. Crate layout

```
calyx-core        // types, traits, IDs, errors (no I/O)
calyx-aster       // storage format, LSM, WAL, CFs, MVCC, crash recovery
calyx-forge       // math runtime: CUDA(sm_120) + CPU SIMD, autotune
calyx-registry    // lens lifecycle, frozen contract, capability cards
calyx-loom        // DDA cross-terms, agreement graph
calyx-assay       // MI/bits, differentiation contract, n_eff
calyx-lodestar    // kernel discovery, kernel index/answer
calyx-mincut      // SCC, betweenness, directed-FVS approx (from contextgraph)
calyx-paths       // graph traversal, hop-attenuation
calyx-ward        // Gτ guard, τ calibration
calyx-sextant     // query planner, fusion, indexes (HNSW/DiskANN/SPANN/ColBERT)
calyx-ledger      // provenance, hash-chain, merkle, reproduce
calyx-anneal      // self-heal/learn/optimize, autotune, lens proposal
calyx-mcp         // MCP tool server (embedded stdio + served)
calyx-cli         // `calyx` binary, migration tools
calyxd            // server daemon
```

## 2. Core types (`calyx-core`)

```rust
pub struct VaultId(pub Ulid);
pub struct LensId(pub [u8; 16]);      // content-addressed
pub struct CxId(pub [u8; 16]);        // blake3(input ‖ panel_ver ‖ salt)
pub struct SlotId(pub u16);

pub enum Modality { Text, Code, Image, Audio, Video, Structured, Mixed }
pub enum SlotShape { Dense(u32), Sparse(u32), Multi { token_dim: u32 } }
pub enum Asymmetry { None, Dual { a: SlotId, b: SlotId } }
pub enum QuantPolicy { None, Pq { m: u8, nbits: u8 }, Float8, Binary }

pub enum SlotVector {
    Dense(Vec<f32>),                                   // or quantized bytes + codebook ref
    Sparse { dim: u32, entries: Vec<(u32, f32)> },
    Multi  { token_dim: u32, tokens: Vec<Vec<f32>> },
    Absent { reason: AbsentReason },                   // explicit, never zero-fill (A16)
}

pub enum AnchorKind {
    TestPass, TieFormed, Thumbs, Label(String), Reward,
    SpeakerMatch,   // speaker verification (e.g. WavLM similarity) — identity-locked voice
    StyleHold,      // persona/style consistency under injection — identity-locked style
}
pub struct Anchor { kind: AnchorKind, value: AnchorValue, source: String, observed_at: Ts, confidence: f32 }

pub struct Slot {
    id: SlotId, key: String, lens: LensId, shape: SlotShape, modality: Modality,
    asymmetry: Asymmetry, quant: QuantPolicy, axis: Option<String>,
    bits_about: BTreeMap<AnchorKind, Signal>, state: SlotState,
}
pub enum SlotState { Active, Parked, Retired }
pub struct Signal { bits: f32, ci: (f32, f32), n: usize, estimator: String, ts: Ts }

pub struct Constellation {
    id: CxId, vault: VaultId, panel_version: u32, created_at: Ts,
    input_ref: InputRef, modality: Modality,
    slots: BTreeMap<SlotId, SlotVector>,
    scalars: BTreeMap<String, f64>,
    metadata: BTreeMap<String, String>, // verbatim source identifiers, e.g. chunk_id/database_name
    anchors: Vec<Anchor>,
    provenance: LedgerRef,
    flags: CxFlags,   // ungrounded, degraded, novel_region, redacted_input
}
```

## 3. Key traits

```rust
pub trait Lens: Send + Sync {                          // a frozen instrument (A4)
    fn id(&self) -> LensId;
    fn shape(&self) -> SlotShape;
    fn modality(&self) -> Modality;
    fn measure(&self, input: &Input) -> Result<SlotVector, CalyxError>;   // deterministic, frozen
    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>, CalyxError>;
}

pub trait Estimator {                                  // Assay
    fn mi(&self, x: &[SlotVector], y: &[Anchor]) -> Result<Signal, CalyxError>;
    fn redundancy(&self, a: &[SlotVector], b: &[SlotVector]) -> Result<f32, CalyxError>;
}

pub trait Index {                                      // per-slot ANN/inverted
    fn insert(&mut self, cx: CxId, v: &SlotVector) -> Result<(), CalyxError>;
    fn search(&self, q: &SlotVector, k: usize, ef: Option<usize>) -> Result<Vec<(CxId, f32)>, CalyxError>;
    fn rebuild(&mut self) -> Result<(), CalyxError>;   // self-heal
}

pub trait VaultStore {                                 // Aster
    fn put(&self, cx: Constellation) -> Result<CxId, CalyxError>;     // group-commit, idempotent
    fn get(&self, id: CxId, snapshot: Seq) -> Result<Constellation, CalyxError>;
    fn anchor(&self, id: CxId, a: Anchor) -> Result<(), CalyxError>;
    fn snapshot(&self) -> Seq;
}
```

## 4. Top-level engine API (what `calyx-mcp`/`calyx-cli` call)

```rust
impl Calyx {
  fn create_vault(&self, name, panel_template) -> Result<VaultId>;
  fn add_lens(&self, v: VaultId, spec: LensSpec) -> Result<LensId>;
  fn retire_lens(&self, v, slot) -> Result<()>;
  fn park_lens(&self, v, slot) -> Result<()>;
  fn profile_lens(&self, spec, probe: Option<ProbeSet>) -> Result<CapabilityCard>;

  fn ingest(&self, v, input: Input) -> Result<CxId>;          // idempotent, multi-lens
  fn ingest_batch(&self, v, inputs: &[Input]) -> Result<Vec<CxId>>;
  fn anchor(&self, v, cx, outcome: Anchor) -> Result<()>;
  fn measure(&self, v, input) -> Result<Constellation>;       // no store (for guarding)

  fn search(&self, v, q: Query) -> Result<Vec<Hit>>;
  fn kernel_answer(&self, v, q, anchor) -> Result<Answer>;
  fn neighbors(&self, v, cx, slot, k) -> Result<Vec<Hit>>;
  fn traverse(&self, v, cx, dir: Direction, hops) -> Result<Path>;
  fn skills(&self, v) -> Result<SkillTree>;
  fn define(&self, v, index: LensIndex) -> Result<Constellation>;  // term = constellation other lenses form at index (Gärdenfors)

  fn abundance(&self, v) -> Result<AbundanceReport>;          // N, C(N,2), materialized, n_eff, DPI ceiling
  fn bits(&self, v, anchor) -> Result<BitsReport>;            // per-lens + sufficiency + attribution
  fn build_kernel(&self, v, anchor) -> Result<Kernel>;
  fn grounding_gaps(&self, v, anchor) -> Result<Vec<CxId>>;
  fn calibrate_guard(&self, v, domain, set, target_far) -> Result<GuardProfile>;
  fn guard(&self, v, produced: &Constellation, matched: Option<CxId>) -> Result<GuardVerdict>;
  fn propose_lens(&self, v, anchor) -> Result<CandidateLens>;

  // Universal data layer (20) — collections as any model
  fn create_collection(&self, v, name, mode, schema?) -> Result<CollectionId>;
  fn put_record/get_record/range/query(&self, ...) -> ...;   // relational/doc/KV/TS/blob ops
  fn build_kernel(&self, v, scope: KernelScope, anchor?) -> Result<Kernel>;   // any scope (08 §4b)

  // Oracle / AGI (21)
  fn oracle_predict(&self, v, action, domain) -> Result<Prediction>;          // consequences + sufficiency gate
  fn expand(&self, c: Consequence) -> Result<Vec<Consequence>>;               // butterfly tree
  fn super_intelligence(&self, v, domain) -> Result<PredicateStatus>;         // 6-tier + cheapest fix
  fn reverse_query(&self, v, answer) -> Result<Vec<Question>>;                 // epistemic symmetry (A23)

  // Temporal & dedup (25)
  fn ingest_at(&self, v, input, at: Ts) -> Result<IngestResult>;               // IngestResult = New(CxId) | DedupMerge{into, occurrence}
  fn recurrence_series(&self, v, cx) -> Result<RecurrenceSeries>;             // occurrences + cadence + periodic fit
  fn periodic_recall(&self, v, hour?, day?) -> Result<Vec<Hit>>;             // events that recur then
  fn predict_next_occurrence(&self, v, cx) -> Result<TimePrediction>;         // Oracle over time
  fn temporal_search(&self, v, q, window?, boost) -> Result<Vec<Hit>>;        // E2/E3/E4 post-retrieval boost (AP-60)
  fn as_of(&self, v, t: Ts) -> Result<Snapshot>;                              // time-travel within retention horizon
  fn dedup_audit(&self, v, cx) -> Result<Vec<MergeRecord>>;                   // reversible, Ledger-backed

  // Compression / resource (23, 24)
  fn compression_report(&self, v) -> Result<CompressionReport>;               // bits/channel, distortion vs floor, intelligence delta
  fn resource_status(&self, v) -> Result<ResourceStatus>;                     // heap/VRAM/compaction-debt/pinned-seq/backpressure

  fn provenance(&self, v, cx) -> Result<Lineage>;
  fn answer_trace(&self, id: AnswerId) -> Result<AnswerTrace>;
  fn verify_chain(&self, v, range) -> Result<ChainStatus>;
  fn reproduce(&self, id: AnswerId) -> Result<ReproResult>;
  fn anneal_status(&self, v) -> Result<AnnealStatus>;
}
```

## 5. Query & result

```rust
pub struct Query {
  input: QueryInput,                       // Text | Vector(SlotVector) | Cx(CxId)
  lenses: LensSel,                         // Auto | Explicit(Vec<SlotId>)
  fusion: Fusion,                          // SingleLens | Rrf | WeightedRrf(Profile) | KernelFirst | Pipeline
  filters: Vec<Predicate>,
  guard: GuardMode,                        // Off | InRegionOnly(GuardId)
  guard_vectors: Map<SlotId, SlotVector>,   // multi-slot InRegionOnly produced vectors
  freshness: Freshness,                    // FreshDerived | StaleOk(SeqLag)
  k: usize, rerank: Option<RerankSpec>, explain: bool,
}
pub struct Hit {
  cx: CxId, score: f32,
  per_lens: Vec<LensContribution>,         // (slot, rank, raw, weight, contribution)
  guard: Option<GuardVerdict>, provenance: LedgerRef, freshness: FreshnessInfo,
}
```

## 6. Error catalog (verbatim codes — No-Compress List)

| Code | Meaning | Remediation |
|---|---|---|
| `CALYX_LENS_FROZEN_VIOLATION` | weights hash ≠ registered | re-register as new LensId |
| `CALYX_LENS_DIM_MISMATCH` | output dim ≠ Slot.shape | fix lens or slot shape |
| `CALYX_LENS_NUMERICAL_INVARIANT` | NaN/Inf/non-unit output | check lens runtime/normalize |
| `CALYX_LENS_UNREACHABLE` | runtime endpoint down | restore lens service |
| `CALYX_REGISTRY_DUPLICATE` | lens id already registered | reuse existing LensId or register a distinct frozen spec |
| `CALYX_REGISTRY_UNAVAILABLE` | lens registry unavailable | restore registry before guarded anneal update |
| `CALYX_ASSAY_INSUFFICIENT_SAMPLES` | < quorum (50) anchors | anchor more outcomes |
| `CALYX_ASSAY_LOW_SIGNAL` | lens < 0.05 bits | park/retire lens |
| `CALYX_ASSAY_REDUNDANT` | pair corr > 0.6 | drop duplicate lens |
| `CALYX_KERNEL_UNGROUNDED` | kernel over ungrounded graph | add anchors (grounding_gaps) |
| `CALYX_GUARD_PROVISIONAL` | τ not calibrated | calibrate before high-stakes use |
| `CALYX_GUARD_OOD` | query/output outside trusted region | new-region or reject per policy |
| `CALYX_FORGE_NUMERICAL_INVARIANT` | kernel NaN/Inf | numerical fail-closed |
| `CALYX_FORGE_DEVICE_UNAVAILABLE` | CUDA init failed (server mode) | fix driver (reboot per gotcha) |
| `CALYX_ASTER_CORRUPT_SHARD` | base shard hash mismatch | restore from restic/snapshot |
| `CALYX_ASTER_TORN_WAL` | torn tail on recovery | auto-discarded; logged |
| `CALYX_LEDGER_CHAIN_BROKEN` | hash-chain verify failed | quarantine range, investigate |
| `CALYX_LEDGER_CORRUPT` | ledger CF integrity violation | ledger CF integrity violation — run verify_chain to identify range |
| `CALYX_LEDGER_APPEND_ONLY_VIOLATION` | ledger CF append-only invariant violated | ledger CF is append-only; deletes and tombstones are forbidden |
| `CALYX_LEDGER_SECRET_IN_PAYLOAD` | ledger payload contains secret-like material | ledger payload must store hashes/ids only — redact before writing |
| `CALYX_LEDGER_ACTOR_TOO_LONG` | ledger actor id exceeds 64 UTF-8 bytes | actor id must be <= 64 bytes UTF-8 |
| `CALYX_LEDGER_GROUP_COMMIT_FAILED` | ledger hook failed during group commit | ledger hook failed — group-commit rolled back; retry the write |
| `CALYX_REPRODUCE_NONDETERMINISTIC` | reproduce ledger entry lacks determinism seed | no determinism seed in ledger entry - cannot guarantee reproduce fidelity |
| `CALYX_REPRODUCE_DRIFT_EXCEEDED` | reproduce max_drift exceeded tolerance | reproduce max_drift exceeded 1e-3 - possible lens drift or fusion parameter change |
| `CALYX_VAULT_ACCESS_DENIED` | cross-vault read without grant | request grant |
| `CALYX_STALE_DERIVED` | fresh required, rebuild pending | retry or accept StaleOk |
| `CALYX_ORACLE_INSUFFICIENT` | `I(panel;oracle) < H(Y)` — panel can't predict | add outcome/execution lens (propose_lens) |
| `CALYX_FORGE_VRAM_BUDGET` | dispatch exceeds VRAM budget | split batch / raise budget / wait |
| `CALYX_BACKPRESSURE` | write/query queue at high-water | retry with backoff |
| `CALYX_DISK_PRESSURE` | hotpool near full | free/spill to archive; writes fail-closed |
| `CALYX_QUANT_INTELLIGENCE_LOSS` | quant level would drop bits/cosine/FAR beyond bound | use a gentler level (A25) |
| `CALYX_READER_LEASE_EXPIRED` | long reader aborted to release MVCC version | re-issue with bounded-staleness snapshot |
| `CALYX_DATASET_NOT_FOUND` | dataset dir or MANIFEST row missing | acquire + register via scripts/acquire_datasets.sh |
| `CALYX_DATASET_CHECKSUM_MISMATCH` | recomputed sha256 != recorded value | re-acquire at the pinned revision; never edit dataset bytes in place |
| `CALYX_DATASET_ROWCOUNT_MISMATCH` | recomputed row count != recorded value | re-acquire at the pinned revision; check split/decoder drift |
| `CALYX_DATASET_MANIFEST_INVALID` | MANIFEST.md or manifest.json missing/malformed/drifted | re-register via scripts/verify_dataset.sh register |
| `CALYX_DATASET_SCHEMA_MISMATCH` | dataset columns/fields missing or malformed vs the pinned upstream contract | re-acquire at the pinned revision; check upstream schema drift |

### Catalog boundary

`CALYX_ERROR_CODES` in `crates/calyx-core/src/error.rs` is the closed PRD-18
cross-surface catalog and must match the table above in the same order. A new
code enters that catalog only when this PRD-18 table and the `PRD_18_CODES`
test fixture are amended in the same change.

Subsystem- or phase-local `CALYX_*` errors stay beside the owning guard/type as
module-local `pub const` strings and construct `{ code, message, remediation }`
directly. Examples include temporal policy codes, record-schema validation,
MCP JSON-RPC parse errors, PH60 authn/TLS codes, and PH61 consent/redaction
codes. Task cards for those codes must not instruct agents to edit the closed
catalog unless they also amend this PRD.

Every error is `{ code, message, remediation }` (A16/A17): structured, actionable, never a silent fallback.

## 7. Wire format
- Internal: zero-copy Aster (Arrow layout) + bincode for control rows.
- MCP: JSON-RPC; payloads JSON (tool calls), prose markdown (descriptions).
- Export: signed Merkle bundles (`11`) for provenance attestation.
