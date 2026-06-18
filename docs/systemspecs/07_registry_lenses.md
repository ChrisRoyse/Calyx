# 07. Lens Registry (calyx-registry)

The `calyx-registry` crate is the lens/embedder registry: it defines the `Lens` measurement trait, the seven runtime implementations behind it (algorithmic, TEI HTTP, Candle local, ONNX, static-lookup, external-command, multimodal-adapter), the frozen content-addressed lens contract and `LensId`, hot-swap + backfill machinery, capability cards + gating, default domain panels, temporal lenses, vault persistence, and vector compression. All claims below are traced to source.

Cross-references:
- The `Lens` trait, `LensId`, `Input`, `SlotShape`, `SlotVector`, `Modality`, `content_address` live in `calyx-core` (see [01_core_types.md](01_core_types.md) — if absent, `crates/calyx-core/src/traits.rs`, `ids.rs`, `enums.rs`).
- Assay signal/redundancy thresholds consumed by the capability gate come from `calyx-assay` (see [09_assay_signal.md](09_assay_signal.md)).
- Sextant indexes consume the slot vectors this crate produces (see [08_sextant_search.md](08_sextant_search.md)).
- `Panel`/`Slot`/`SlotState` types are from `calyx-core`; vault manifest/`ImmutableRef` from the vault layer (see [05_vault_aster.md](05_vault_aster.md)).

## Source files covered

```
src/lib.rs                       crate root, re-exports
src/spec.rs                      LensSpec, LensRuntime, LensHealth, health probes
src/frozen.rs                    FrozenLensContract, LensDType, NormPolicy, contract validation
src/lens.rs                      Registry, shape/modality validation, dual measurement
src/runtime/mod.rs               runtime module tree
src/runtime/common.rs            shared helpers (hashing, UTF-8 extract, normalize_unit)
src/runtime/algorithmic.rs       AlgorithmicLens, AlgorithmicEncoder
src/runtime/tei_http.rs          TeiHttpLens (HTTP embedding server)
src/runtime/candle.rs (+load/options/pooling)  CandleLens (local BERT)
src/runtime/onnx.rs (+fastembed_runtime/custom) OnnxLens
src/runtime/static_lookup.rs     StaticLookupLens (mmap vocab matrix)
src/runtime/external_cmd.rs      ExternalCmdLens (subprocess)
src/runtime/adapters/*           MultimodalAdapterLens, axis, pack, license gating
src/swap.rs                      SwapController, SlotSpec, BackfillQueue
src/backfill.rs                  BackfillScheduler, priorities, watermarks
src/drift.rs                     RuntimeGolden, DriftDecision
src/placement.rs                 choose_placement, CpuLensPool, budgets
src/ingest_microbatch.rs         IngestMicrobatchController, circuit breaker
src/panels/mod.rs                PanelTemplate, instantiate_panel
src/panels/defaults.rs           text/code/legal/medical/bio/civic/media defaults
src/panel_ops.rs                 apply_panel_template, swap_panel, capability gate apply
src/temporal/*                   E2 recency, E3 periodic, E4 positional lenses
src/profile.rs (+assay/cost/gating) Profiler, CapabilityCard, capability gate
src/commission.rs (+manifest.rs) commission_lens, LensForge manifest
src/compression/*                StoredSlotCodec, matryoshka truncation, envelope
src/explain.rs                   LensExplanation
src/persistence.rs               VaultPanelState, persist/load
```

---

## 1. The `Lens` trait and runtime implementations

### 1.1 The `Lens` trait

Defined in `calyx-core` at `crates/calyx-core/src/traits.rs:38`. Every registry runtime is a frozen, deterministic measurement instrument.

| Method | Signature | Purpose |
|--------|-----------|---------|
| `id` | `fn id(&self) -> LensId` | Stable frozen content id |
| `shape` | `fn shape(&self) -> SlotShape` | Vector shape emitted (`Dense`/`Sparse`/`Multi`) |
| `modality` | `fn modality(&self) -> Modality` | Accepted input modality |
| `measure` | `fn measure(&self, input: &Input) -> Result<SlotVector>` | Deterministically measure one input |
| `measure_batch` | `fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>>` | Batch measure; default maps `measure` over inputs |

`Input` (`traits.rs:11`) carries `modality: Modality`, `bytes: Vec<u8>`, `pointer: Option<String>`. The trait bound is `Send + Sync`.

### 1.2 Runtime catalogue

`LensRuntime` (`src/spec.rs:13`, `serde rename_all = "snake_case"`) enumerates the serialized runtime variants stored in a `LensSpec`. The concrete instruments implementing `Lens`:

| Runtime struct | File | Shape | Modality | `measure()` does | Determinism / I/O |
|----------------|------|-------|----------|------------------|--------------------|
| `AlgorithmicLens` | `runtime/algorithmic.rs:42` | `Dense(encoder.dim())` | constructor-set | local feature extraction (byte/char-class histogram + FNV-1a hashing, scalar mean, one-hot bucket, AST-style keyword counts) | fully deterministic, in-process, no I/O |
| `TeiHttpLens` | `runtime/tei_http.rs:17` | `Dense(dim)` | constructor-set | POSTs `{"inputs":[texts]}` over raw TCP HTTP/1.1 to an embedding server, parses JSON float arrays | network; deterministic per server state |
| `CandleLens` | `runtime/candle.rs:36` | `Dense(dim)` | `Text` | tokenizes, runs a `BertModel` forward (mutex-guarded), pools, normalizes | local CPU/GPU; deterministic within precision; F32 finite-replay fallback on numerical error |
| `OnnxLens` | `runtime/onnx.rs:18` | `Dense(dim)` | contract modality | FastEmbed `TextEmbedding::embed()` or a custom `ort` session run, then unit-normalizes | local CPU/GPU; deterministic |
| `StaticLookupLens` | `runtime/static_lookup.rs:20` | `Dense(matrix.dim)` | `Text` | tokenizes, averages mmap'd per-token embedding rows, normalizes | local mmap read; deterministic |
| `ExternalCmdLens` | `runtime/external_cmd.rs:14` | `Dense(dim)` | constructor-set | spawns subprocess, frames a length-prefixed JSON request/response over stdin/stdout | subprocess; deterministic per subprocess output |
| `MultimodalAdapterLens` | `runtime/adapters/lens.rs:29` | `Dense(dim)` | `axis.modality()` | validates input magic bytes per axis, then frames the bytes to the configured ONNX helper and unit-normalizes the returned embedding | local subprocess + ONNXRuntime CPU; fails closed when config/model/helper files are missing |

### 1.3 Runtime details

**AlgorithmicLens** — `AlgorithmicEncoder` (`algorithmic.rs:12`): `ByteFeatures` (dim 16), `Scalar` (dim 1), `OneHot { buckets }` (dim = buckets), `AstStyle` (dim 8). Constructors: `byte_features`, `scalar`, `one_hot`, `ast_style`, `new`. FNV constants: `FNV_OFFSET = 0xcbf29ce484222325`, `FNV_PRIME = 0x100000001b3`. Contract uses `NormPolicy::None`; weight hash seeds with `b"algorithmic-runtime-v2"` (or `FrozenLensContract::algorithmic_byte_features` for byte features).

**TeiHttpLens** — fields `endpoint`, `dim`, `timeout` (default 30 s), `max_batch` (default 32). `DEFAULT_TEI_ENDPOINT = "http://127.0.0.1:8088/embed"` (`tei_http.rs:13`). Constructors `new`, `resident_8088`; builders `with_timeout`, `with_max_batch`. `measure_batch` chunks by `max_batch`, POSTs JSON, parses raw `[[...]]`, OpenAI `{"data":[...]}`, requires HTTP `" 200 "`, handles chunked transfer encoding. Contract from `FrozenLensContract::tei_http` (`NormPolicy::unit()`).

**CandleLens** — `DEFAULT_CANDLE_MODEL = "sentence-transformers/all-MiniLM-L6-v2"` (`candle.rs:15`). Enums in `candle/options.rs`: `CandleDevicePolicy { CpuExplicit, CudaFailLoud { ordinal } }`; `CandlePrecision { F32, F16, BF16 }`; `CandlePoolingPolicy { Mean, Cls }`. Constructors include `all_minilm_l6_v2`, `all_minilm_l6_v2_cuda_fail_loud`, `from_hf_cache`, `from_model[_with_options]`, `from_files`, `from_lens_spec`. `candle/load.rs` raises layer-norm eps to `1.0e-5` for half-precision CUDA. Pooling/normalization in `candle/pooling.rs` (`pool_tokens`, `apply_norm`). Corpus hash seeds with `b"candle-local-bert-v2"` over model id/tokens/precision/pooling/norm.

**OnnxLens** — backend is `FastEmbed(Mutex<TextEmbedding>)` or `Custom(Mutex<CustomOnnxRuntime>)`. `OnnxProviderPolicy { CudaFailLoud, CpuExplicit }`; persisted `LensSpec`/LensForge manifest reload defaults to `CudaFailLoud` so GPU-capable production manifests use CUDA and fail loudly if the CUDA/ORT stack is unavailable. `CpuExplicit` remains an explicit constructor/test policy only. CUDA fail-loud ONNX backends are retained for process lifetime to avoid unsafe ORT CUDA provider teardown after successful inference; CPU-explicit backends drop normally. `PoolingPolicy { Mean, Cls, LastToken }`. `OnnxModelFiles` lists cache dir, model code/file, tokenizer, config, and `contract_paths` override. FastEmbed corpus hash seeds `b"onnx-fastembed-mean-pool-v1"`. Custom path (`onnx/custom.rs`) runs an `ort::session::Session`, pools, normalizes.

**StaticLookupLens** — memory-maps a binary vocabulary matrix. File format (`static_lookup.rs`): magic `b"CXLKUP1\0"` (8 B), `rows: u32`, `dim: u32`, dtype byte (`I8=1, F16=2, F32=3`), 3 reserved, `scale: f32`, then `rows × dim` quantized values. `StaticLookupDType { Int8, F16, F32 }`, widths 1/2/4. Empty text returns a zero-safe unit vector; unknown tokens (`[UNK]`, `<unk>`, `<UNK>`) are skipped; remaining rows are averaged, scaled, normalized. `DEFAULT_MAX_TOKENS = 512`.

**ExternalCmdLens** — fields `cmd`, `args`, `dim`, `timeout` (default 30 s, `with_timeout`). Wire frame: 4-byte big-endian length prefix + JSON. Request `{modality, inputs: [bytes]}`, response `{vectors: [[f32]]}`. Spawns the process, uses a write thread + read thread + 5 ms poll, kills on timeout, requires exit status 0. `NormPolicy::None`; weight hash from `cmd` + null-joined `args`, corpus seed `b"external-cmd-runtime-v1"`.

**MultimodalAdapterLens** — `MultimodalAxis { Image, Audio, Protein, Dna, Molecule }` (`adapters/axis.rs`) maps to a `Modality`. `measure` validates input shape per axis (PNG/JPEG magic, RIFF/WAVE, protein/DNA alphabet, SMILES charset), then runs a strict `calyx-multimodal-adapter-v2` descriptor through `tools/lensforge/multimodal_onnx_embed.py`. The descriptor names the local ONNX tower model, processor files, helper script, explicit CPU provider, and expected dimension. The lens hashes the real model/config/helper artifacts, rejects missing configs, rejects unsupported providers, rejects NaN/Inf or wrong dimensions, and has no byte-hash vector fallback. Built from `MultimodalAdapterSpec { name, axis, model_id, dim, license, allow_non_commercial, adapter_config, files }`.

### 1.4 License gating (multimodal adapters)

`runtime/adapters/`:
- `CALYX_ALLOW_NONCOMMERCIAL_LENSES_ENV = "CALYX_ALLOW_NONCOMMERCIAL_LENSES"`, error code `CALYX_LICENSE_DENIED`.
- `allow_noncommercial_from_env()` accepts `1/true/yes/allow/allowed` (case-insensitive).
- `is_non_commercial_license(raw)` detects `non-commercial`, `noncommercial`, `cc-by-nc`, or bare token `nc` after normalizing separators.
- `ensure_license_allowed(license, non_commercial, allow_non_commercial)` errors `CALYX_LICENSE_DENIED` when the lens is non-commercial and not explicitly allowed.
- `default_multimodal_lens_specs()` returns the commissioned priority media specs when `CALYX_HOME` points at the LensForge bundle paths: `image-siglip2-b16-adapter` (SigLIP2 vision, 768D, apache-2.0) and `audio-clap-htsat-adapter` (LAION-CLAP audio, 512D, apache-2.0). Protein/DNA/molecule adapters are not advertised until real runtimes are commissioned.
- `register_multimodal_lens_pack(registry, specs)` builds and registers each, returning `MultimodalLensPackEntry` records.

### 1.5 The `Registry`

`src/lens.rs:15` — `Registry` is a `BTreeMap<LensId, RegistryEntry>` (entry = `Arc<dyn Lens>` + optional `FrozenLensContract` + optional `LensSpec` + `DeterminismProof`).

Key behavior:
- **Fails closed.** `register` and `register_with_spec` always return a `lens_frozen_violation` error — a `FrozenLensContract` is mandatory (`lens.rs:57`, `:67`).
- Registration paths: `register_frozen`, `register_frozen_with_spec`, `register_frozen_with_probe`. All call `register_frozen_inner` which (1) `contract.verify_registration(&lens)`, (2) optionally `verify_determinism_probe`, (3) records `DeterminismProof::ProbeVerified` (probe) or `ContractOnlyExemption` (no probe), (4) rejects duplicate ids with `registry_duplicate`.
- `measure` / `measure_batch`: enforce input modality, run the lens, then `validate_entry` (contract `verify_registration` + `verify_vector`, or bare `ensure_vector_shape`). `measure_batch` also asserts vector count equals input count.
- `measure_dual`: requires `Asymmetry::Dual`, measures input and its byte-reversed form, and rejects identical outputs.
- `measure_ingest_microbatch`: drives bounded admission (§4.4) per lens.
- Accessors: `contains`, `find_lens_by_name`, `frozen_contract`, `frozen_lens_snapshots`, `lens_spec`, `lens_snapshots`, `determinism_proof`, `health`.
- `register_persisted_arc` (crate-internal): re-registers a persisted `Arc<dyn Lens>` after `verify_registration` (used by §8 vault load).

Shape validation (`ensure_vector_shape`, `lens.rs:365`) requires the emitted `SlotVector` variant and dimensions to exactly match the declared `SlotShape`, rejects `Absent`, and asserts all values finite (`lens_numerical_invariant` on NaN/Inf).

---

## 2. `LensId` and the frozen contract

### 2.1 `LensId` derivation

`LensId` is a 16-byte content address (`calyx-core/src/ids.rs:163`). `content_address` (`ids.rs:275`) is a **length-delimited BLAKE3** hash: each part is prefixed by its `u64` big-endian length before hashing, then truncated to 16 bytes.

`LensId::from_parts(name, weights_sha256, corpus_hash, output_shape)` hashes those four parts in order (`ids.rs:173`). Identical lens specs yield identical ids across vaults.

Two id-derivation surfaces exist (note: **they differ** in how they build the shape part):

| Source | Hash inputs (in order) | Shape encoding |
|--------|------------------------|----------------|
| `LensSpec::lens_id` (`spec.rs:79`) | `name`, `weights_sha256`, `corpus_hash`, `output` string | `format!("shape={:?};norm={:?};runtime={:?}", output, norm_policy, runtime)` |
| `FrozenLensContract::lens_id` (`frozen.rs:139`) | `name`, `weights_sha256`, `corpus_hash`, shape fingerprint | `dtype={};shape={};norm={}` (see §2.3) |

A lens registered with a contract takes its id from the lens' own `id()` (which the runtimes derive from the contract), and `verify_registration` requires `lens.id() == contract.lens_id()`.

### 2.2 `FrozenLensContract`

`src/frozen.rs:69`. Private fields: `name`, `weights_sha256: [u8;32]`, `corpus_hash: [u8;32]`, `shape: SlotShape`, `modality: Modality`, `dtype: LensDType` (only `F32`), `norm: NormPolicy`. Serialized for persistence (§8).

`NormPolicy` (`frozen.rs:26`): `None`, `L2 { tolerance }`, `DeclaredByModel { declared_norm, tolerance }`, `Finite`, `Unit { tolerance }`. Helpers: `unit()` = `L2 { tolerance: 1.0e-3 }`, `finite_only()` = `None`, `declared_by_model`. Fingerprint collapses to `"finite"`, `"unit"`, or `"declared-by-model"`.

Builders: `new`, `algorithmic_byte_features` (`Dense(16)`, `Finite`), `tei_http` / `tei_http_8088` (`unit()`).

### 2.3 LensId hash preimage (exact format)

`FrozenLensContract::lens_id` hashes `name`, `weights_sha256`, `corpus_hash`, and the shape fingerprint string:

```
output_shape_fingerprint = "dtype={dtype};shape={shape};norm={norm}"
  dtype  = "f32"
  shape  = "dense:{dim}" | "sparse:{dim}" | "multi:{token_dim}"
  norm   = "finite" | "unit" | "declared-by-model"
```

Contract field hashes (`sha256_digest`, `frozen.rs:268`) are length-delimited SHA-256: each part prefixed by its `u64` big-endian length. (Note: contract field digests use SHA-256, while the final `LensId` envelope uses BLAKE3 via `content_address`.)

### 2.4 Contract validation (freezing & registration)

`verify_registration(&dyn Lens)` (`frozen.rs:192`):
1. `lens.id()` must equal `self.lens_id()` → else `lens_frozen_violation`.
2. `lens.shape()` must equal `self.shape` → else `lens_dim_mismatch`.
3. `lens.modality()` must equal `self.modality` → else `lens_dim_mismatch`.

`verify_determinism_probe(&dyn Lens, &Input)` (`frozen.rs:221`): measure the probe twice, validate each via `verify_vector`, JSON-serialize both, require byte-identical output → else `lens_frozen_violation`.

`verify_vector(lens_id, &SlotVector)` (`frozen.rs:243`): `ensure_vector_shape` then apply the norm policy — `None`/`Finite` accept any finite vector; `L2`/`Unit` require unit norm within tolerance; `DeclaredByModel` requires the declared norm. Sparse norms use entry values; Multi checks each token; `Absent` is unreachable after shape validation.

`with_mutated_weight_hash()` flips one weight-hash byte (test utility for proving id sensitivity).

### 2.5 `LensSpec` and `LensHealth`

`LensSpec` (`spec.rs:49`) is the serializable structured metadata: `name`, `runtime: LensRuntime`, `output: SlotShape`, `modality`, `weights_sha256`, `corpus_hash`, `norm_policy`, `axis`, `asymmetry`, `quant_default` (default `QuantPolicy::turboquant_default()`), `truncate_dim`, `recall_delta` (default `0.02`), `retrieval_only`, `excluded_from_dedup`.

`LensHealth` (`spec.rs:70`): `Loaded`, `Cold`, `Failing { code, reason }`. `LensSpec::health()` (`spec.rs:92`) decides health by runtime: algorithmic is always `Loaded`; multimodal requires `adapter_config` and all declared files to exist (missing → `Cold`); TEI does a 250 ms TCP `connect_timeout` probe (`probe_http`); Candle/ONNX check file existence (empty → `Cold`); static-lookup checks both files; external-cmd resolves `cmd` on `PATH` (`command_exists`), else `Failing { code: "CALYX_LENS_UNREACHABLE" }`. `health_result()` converts `Failing` to `lens_unreachable` error.

---

## 3. Capability cards and the capability gate

### 3.1 `CapabilityCard`

`src/profile.rs:43`, produced by `Profiler` from a probe set (`ProfileProbe { input, label }`).

| Field | Type | Meaning |
|-------|------|---------|
| `lens_id` | `LensId` | profiled lens |
| `probe_count` | `usize` | number of probes |
| `signal` | `Option<f32>` | grounded assay signal bits (None if not measured) |
| `signal_source` | `MetricSource` | `ProfileProxy` / `AssayPending` / `AssayStore` |
| `proxy_signal` | `f32` | proxy signal from coverage × participation × differentiation |
| `differentiation` | `Option<f32>` | max pairwise gain bits from assay |
| `differentiation_source` | `MetricSource` | origin of differentiation |
| `proxy_differentiation` | `f32` | proxy = separation score |
| `spread` | `SpreadMetrics` | participation ratio, normalized PR, stable rank, total variance, mean pairwise distance |
| `separation` | `SeparationMetrics` | score, silhouette, mean pairwise distance, labeled groups, used_labels |
| `cost` | `CostMetrics` | total_ms, ms_per_input, vram_bytes, ram_bytes, batch_ceiling |
| `coverage` | `CoverageMetrics` | requested, measured, failed, rate |
| `health` | `LensHealth` | runtime health |
| `low_spread` | `bool` | true when normalized PR < `low_spread_threshold` or mean pairwise distance < `low_distance_threshold` |

`ProfileOptions` defaults (`profile.rs:100`): `low_spread_threshold = 0.02`, `low_distance_threshold = 0.001`. `MetricSource` = `ProfileProxy | AssayPending | AssayStore`. Assay overlay helpers `apply_assay_metrics`, `profile_slot_with_assay` (`profile/assay.rs`). `profile/cost.rs` defines `CostMetrics` with `BATCH_TARGET_MS = 1000.0`.

### 3.2 Capability gate algorithm

`evaluate_capability_gate(card, max_pairwise_corr, thresholds)` (`profile/gating.rs:74`):

1. Validate thresholds: `min_signal_bits` finite ≥ 0; `max_pairwise_corr` in `[0,1]`.
2. Validate input `max_pairwise_corr` finite ≥ 0.
3. `signal_bits = card.signal.unwrap_or(0.0)`; `signal_grounded = card.signal.is_some()`.
4. Decision tree (first match wins):
   - **Retire** if `max_pairwise_corr > thresholds.max_pairwise_corr` (redundant with an existing panel lens).
   - **Park** if `!signal_grounded` (no assay signal yet).
   - **Park** if `card.low_spread` (collapsed lens).
   - **Park** if `signal_bits < thresholds.min_signal_bits`.
   - **Admit** otherwise.

`CapabilityGateDecision { Admit, Park, Retire }`. Output `CapabilityGateEvaluation { lens_id, decision, signal_bits, signal_grounded, max_pairwise_corr, thresholds, reason, card }`.

`CapabilityGateThresholds` default to `MIN_SIGNAL_BITS` / `MAX_PAIRWISE_CORR` from `calyx_assay::contract`, overridable via env:
- `CAPABILITY_MIN_SIGNAL_BITS_ENV = "CALYX_CAPABILITY_MIN_SIGNAL_BITS"`
- `CAPABILITY_MAX_PAIRWISE_CORR_ENV = "CALYX_CAPABILITY_MAX_PAIRWISE_CORR"`

`max_panel_pairwise_correlation(registry, panel, candidate, exclude_slot, probes)` (`gating.rs:139`) computes the max absolute Pearson correlation between the candidate's probe-distance signature and each active panel lens' signature (needs ≥3 probes). `capability_gate_json` and `append_capability_gate_ledger` serialize/append the decision.

### 3.3 Explanation

`src/explain.rs` — `explain_lens(registry, lens_id, probes)` profiles then builds `LensExplanation { lens_id, corpus_hash (hex), axis, runtime, bits, redundancy, cost_ms_per_input, vram_bytes }`. `bits`/`redundancy` are `"provisional (Assay report not attached)"`.

---

## 4. Hot-swap, backfill, placement, and ingest admission

### 4.1 Hot-swap (`src/swap.rs`)

`SwapController` holds a `Panel` plus a `BackfillQueue`. `SlotSpec` = `{ key, lens_id, shape, modality, asymmetry, quant, axis, retrieval_only, excluded_from_dedup }`.

`add_lens(spec, constellations, now)` steps:
1. **Idempotent reuse** — if an identical *live* slot already exists (`identical_live_slot`), return immediately with `queued: 0`.
2. **Validate** — `ensure_unique_slot` rejects duplicate slot key or duplicate active/parked lens id; `ensure_registered_lens` requires the lens to be registered with a frozen contract whose shape and modality match the spec.
3. **Create slot** — `next_slot_id` = max existing + 1 (overflow → error); `bump_panel` increments `version` and sets `created_at = now`; new slot state `Active`.
4. **Queue backfill** — `queue.enqueue_many` enqueues each constellation as `Pending`.
5. Return `AddLensOutcome { slot, panel_version, index, queued }`.

`add_lens_durable` wraps step (1)–(5) transactionally: snapshots panel/queue/scheduler, and rolls all three back if `scheduler.enqueue` fails.

Lifecycle: `park_lens` (→ `Parked`), `unpark_lens` (→ `Active`), `retire_lens` (→ `Retired`, terminal). `set_slot_state` validates the state machine, cancels the backfill queue on any non-Active transition, and bumps the panel version.

### 4.2 Backfill (`src/backfill.rs`)

`BackfillPriority { Normal (0), Hot (1), Kernel (2) }`. `BackfillRequest { slot_id, lens_id, priority, candidates: Vec<CxId> }`. `BackfillConfig` defaults: `max_concurrent = 4`, `batch_size = 16`, `throttle_ms = 50`.

`BackfillScheduler` persists JSON state (`requests: BTreeMap`, keyed `"{slot_id}:{lens_id}"`, plus a `next_allowed_ms` throttle gate). Algorithm:
- **open** — load/create state file; clears in-flight batches (resumable), normalizes config.
- **enqueue** — insert request (`next_index: 0`, empty in-flight, not complete); persist atomically.
- **claim next batch** — return throttled if `now_ms < next_allowed_ms`; return `None` if `active_count() >= max_concurrent`; else pick the incomplete, non-in-flight request with max `(priority.rank, next_index)`, slice `[next_index, min(next_index+batch_size, len)]`, mark in-flight, persist.
- **complete batch** — advance `next_index` by in-flight length, record `last_processed`, clear in-flight, set `complete` when exhausted, set `next_allowed_ms = now_ms + throttle_ms`.
- **watermarks** — per request: `processed = next_index`, `pending = total - next_index`, `in_flight = len`.

Persistence is atomic: write `.{file}.tmp-{pid}`, fsync, rename, fsync parent dir (Unix; no-op on Windows).

### 4.3 Placement & drift

`choose_placement(runtime, cost, budget)` (`src/placement.rs:46`):
1. Zero-cost algorithmic → CPU.
2. CPU-native runtimes (Algorithmic, MultimodalAdapter, StaticLookup, ExternalCmd) → `ensure_cpu_budget` → CPU.
3. GPU fit: `cost.vram_bytes <= available_vram_bytes()` → GPU.
4. GPU overflow for a GPU-capable runtime hard-fails → `CALYX_VRAM_BUDGET_EXCEEDED` (remediation `LENS_VRAM_REMEDIATION`); no hidden CPU fallback is selected.

`PlacementBudget.available_vram_bytes()` = `vram_soft_cap − tei_reserved − vram_allocated` (saturating); `available_ram_bytes()` = `ram_soft_cap − ram_used`. `ensure_cpu_budget` fails with `CALYX_RAM_BUDGET_EXCEEDED` (remediation `LENS_RAM_REMEDIATION`) when the resident count is at limit or RAM is insufficient.

`CpuLensPool` is a FIFO=LRU `VecDeque`; `admit` evicts the front while over the resident-limit or RAM cap, returning `CpuPoolAdmission { evicted_lenses, resident_lenses, resident_ram_bytes }`.

`RuntimeGolden { lens_id, runtime_version, golden_output, tolerance }` (`src/drift.rs`): `evaluate(observed)` computes max-abs-delta vs golden (dimension mismatch → `f32::INFINITY`). Within tolerance → `DriftDecision::Reuse`; otherwise → `DriftDecision::Drifted` with a **new** `LensId` derived from `(lens_id:runtime_version, lens bytes, runtime_version, observed bytes)` and signal `"CALYX_LENS_RUNTIME_DRIFT"`.

### 4.4 Ingest microbatch admission (`src/ingest_microbatch.rs`)

Constants: `DEFAULT_INGEST_MICROBATCH_CAP_BYTES = 16 MiB`, `INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES = 64`; default high-water = 75% of cap; default breaker threshold = 3 timeouts; default open window = 30 000 ms.

`estimate_microbatch_bytes` = Σ (64 + bytes.len + pointer.len). `admit(inputs)` reserves bytes against `cap_bytes`, returns `CalyxError::backpressure` on overflow, otherwise an RAII `IngestMicrobatchPermit` (releases bytes on drop).

`measure_lens_batch(lens_id, inputs, now_ms, measure_fn)`: if the lens' circuit breaker is open → degraded outcome with `Absent` vectors; else admit, measure, and on success reset the breaker; on a `CALYX_LENS_UNREACHABLE` error record a timeout. After `breaker_failure_threshold` consecutive timeouts the breaker opens for `breaker_open_ms`. `IngestPanelReadout` aggregates per-lens `IngestLensOutcome` (`Measured`/`Degraded`) and `IngestMicrobatchStats`.

---

## 5. Default panels and temporal lenses

### 5.1 Panel templates (`src/panels/mod.rs`)

`PanelTemplate { name, slots: Vec<PanelSlotSpec> }`. `PanelSlotSpec { name, runtime: PanelLensRuntime, output: SlotShape, modality, retrieval_only, excluded_from_dedup, required, asymmetry }`. `PanelLensRuntime { Registry { name }, TeiHttp { endpoint }, Algorithmic { lens: AlgorithmicPanelLens }, ExternalCmd { name }, Placeholder { name } }`. `AlgorithmicPanelLens { ByteFeatures, TemporalRecent, TemporalPeriodic, TemporalPositional, Scalar }`.

`instantiate_panel(template, created_at)` (`mod.rs:61`): assigns each slot `SlotId(idx)`, key `SlotKey(slot_id, name)`, `lens_id = slot_lens_id(template_name, spec)`, state `Active`, `added_at_panel_version = idx+1`, `quant = QuantPolicy::None`, `axis = Some(name)`; the panel `version = slots.len()`. `slot_lens_id` (`mod.rs:165`) content-addresses `"{template}:{name}:{runtime:?}:{output:?}:{modality:?}:{retrieval_only}:{excluded_from_dedup}"`.

### 5.2 Default domain panels (`src/panels/defaults.rs`)

Every default panel ends with three temporal slots `[E2_recency, E3_periodic, E4_positional]` (modality `Structured`, `retrieval_only = true`, `excluded_from_dedup = true`, `required = false`).

| Panel (factory) | Content slots (slot key → runtime / dim / modality) |
|-----------------|------------------------------------------------------|
| `text_default` | E1_semantic (TEI-GTE 768D Text), keyword_splade (ByteFeatures, Sparse ~30.5K, Text), paraphrase (768D Text), entity (768D Text), causal_dual (768D Text, `Asymmetry::Dual`) |
| `code_default` | semantic, ast, cfg, dataflow, type_graph, trace, diff, oracle_anchor, static_analysis, runtime, reasoning, scalars — all ByteFeatures 16D Code (12 slots) |
| `civic_default` | polis_axis_01 … polis_axis_21 — Algorithmic `Scalar` 1D Text (21 slots) |
| `legal_default` | legal_bert_small (768D), general_semantic (768D), keyword_splade (Sparse ~30.5K), entity (768D), causal_dual (768D, Dual) — Registry, Text |
| `medical_default` | biomedbert_small_embeddings (768D), general_semantic (768D), medical_entity (768D) — Registry, Text |
| `bio_default` | protein_esm2 (16D Protein), dna_dnabert2 (16D Dna), molecule_chemberta (16D Molecule), general_semantic (768D Text) — Registry |
| `media_default` | media_semantic (768D Mixed), image_siglip2 (768D Image), audio_clap (512D Audio), audio_wave (256D Audio), audio_emotion (128D Audio), speaker_wavlm (512D Audio), transcript (768D Text), style_register (768D Text) — Registry |

Default TEI endpoint used by templates: `http://127.0.0.1:8088`.

### 5.3 Temporal lenses (`src/temporal/`)

All three are algorithmic and deterministic, carry `TEMPORAL_FLAGS { retrieval_only: true, excluded_from_dedup: true }`, and derive their id via `temporal_lens_id`.

| Lens | File | Output | Input | Computes |
|------|------|--------|-------|----------|
| `E2RecencyLens` | `e2_recency.rs` | `Dense(1)` | 8-byte LE i64 timestamp | recency score in `[0,1]` from `DecayFunction` |
| `E3PeriodicLens` | `e3_periodic.rs` | `Dense(2)` | 8-byte LE i64 timestamp | `[hour_score, dow_score]` circular proximity |
| `E4PositionalLens` | `e4_positional.rs` | `Dense(4)` | 16 bytes (u64 position, u64 total) | sin/cos of forward & backward position ratios |

`E2RecencyConfig { decay, reference_time }`; `DecayFunction { Linear { max_age_secs }, Exponential { half_life_secs }, Step }`. Age = `max(reference_time − event, 0)`. Linear `1 − age/max_age`; Exponential `exp(−age·ln2/half_life)`; Step = `0.8` if age < 3600 s, `0.5` if < 86400 s, else `0.1`.

`E3PeriodicConfig { options: PeriodicOptions, reference_time }`; `PeriodicOptions { target_hour, target_day_of_week, use_now }`. Scores via `circular_score`: hour uses max-distance `12.0` over span 24, day-of-week max-distance `3.5` over span 7 (Monday = 0); unset target → score `1.0`.

`E4PositionalConfig { options: SequenceOptions }`; `SequenceOptions { direction: SequenceDirection, multi_anchor: MultiAnchorMode }`. `SequenceDirection { Forward (masks [2:4]), Backward (masks [0:2]), Both }`; `MultiAnchorMode { First, Last, All }`. Output `[sin(pos·π), cos(pos·π), sin(bwd·π), cos(bwd·π)]` where `pos = clamp(position/max(total,1), 0, 1)`, `bwd = 1 − pos`.

### 5.4 Panel operations (`src/panel_ops.rs`)

`apply_panel_template(panel, registry, template, now)`: instantiate target → resolve each `Registry { name }` slot to a registered lens id (validating frozen shape/modality, error `CALYX_PANEL_LENS_MISSING` / `CALYX_LENS_DIM_MISMATCH`), set quant from the lens default → `swap_panel_to_target`. Returns `AppliedPanelTemplate { template_name, diff, resolved_lenses }`.

`swap_panel_to_target(panel, target, now)`: mark existing slots whose lens id is in the target as unchanged; retire other non-retired slots (never deleted, ids never reused); append target slots not already present; bump `version` + set `created_at` only when topology changes. Returns `PanelDiff { added, retired, unchanged, panel_version }`.

`apply_capability_gate(controller, slot_id, evaluation, now)`: validates the slot's lens matches the evaluation, then maps `Admit`→active/unpark, `Park`→park, `Retire`→retire; returns `PanelCapabilityGateOutcome`.

`list_panel` / `list_panel_with_assay` produce `PanelSlotListing { slot_id, key, lens_id, state, quant, resource, bits_about, health }`. `resource` is the persisted `SlotResource { cost, placement }`, so status/readback surfaces use the same cost bytes chosen at admission. Built-in temporal slots are detected by `Structured` modality + retrieval-only + dedup-excluded + key in `{E2_recency, E3_periodic, E4_positional}`.

### 5.5 Resource-aware panel admission

Assay exposes `PanelResourceBudget { max_vram_mb, max_ram_mb, max_ms_per_input }`, `ResourceUsage`, `ResourceDensity`, and `pack_panel_by_density`. The admission path still enforces the differentiation contract first (`bits >= 0.05`, `corr <= 0.6`), then ranks feasible candidates by marginal signal density under the fixed budget. Rejections keep explicit reasons such as `CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED`; no default cost is invented.

`calyx assay corpus-build --cost-override-json <costs>` accepts measured resident-resource overrides for runtimes that cannot infer local resident bytes, but an override cannot downgrade a GPU runtime (ONNX, Candle-local, TEI HTTP) to CPU; that fails with `CALYX_FSV_ASSAY_CORPUS_BUILD_GPU_OVERRIDE_PLACEMENT`.

`calyx assay bits-validate --cost-json <costs> --panel-budget-json <budget>` is the FSV-facing path. `--cost-json` is keyed by corpus lens name and requires `placement`, `vram_mb`, `ms_per_input`, and optional `ram_mb`; missing or non-finite entries fail closed. When a budget is supplied the command writes `assay_packed_panel.json` with selected lenses, rejected lenses, used resources, remaining budget, and aggregate density.

`calyx panel status --panel-budget-json <budget>` includes per-lens/slot cost, placement, remaining budget, and, when slot bits are present, density (`bits / VRAM-MB`, `bits / ms`, and budget-fraction density). Catalog status can report cost and remaining budget; vault status can additionally report bits and density from the persisted panel slots.

---

## 6. Commission and compression

### 6.1 Commission (`src/commission.rs`, `commission/manifest.rs`)

`CommissionRequest { name, base_model, corpus, output_dim, modality, axis }`. `commission_lens(request, artifact_dir)`: compute `corpus_hash = sha256(corpus)`, `weights_sha256 = sha256("commissioned-lens-v1", base_model, corpus_hash)`, build a `FrozenLensContract`, derive `lens_id`, write `{artifact_dir}/{lens_id}.commissioned.json`, return `CommissionedLensArtifact`.

`LensForgeManifest` describes a forged embedder: `name, modality, runtime (string), dim (>0), dtype, weights_sha256, artifact_set_sha256?, files: Vec<LensForgeFile>, pooling, norm, source_hf_id, license?, non_commercial, quant_default, truncate_dim?, recall_delta`. `LensForgeFile { role, path, sha256, bytes }`.

`lens_spec_from_manifest` (and `_path`, `_with_license_override`): validate fields, check license allowance, verify every file against its SHA-256, derive the artifact weight hash and a corpus hash seeded `b"lensforge-manifest-v1"` over name/source/runtime/modality/pooling/norm, and return a `LensSpec` with runtime parsed from `manifest.runtime`. `register_commissioned` registers the result.

### 6.2 Compression (`src/compression/`)

`StoredSlotCodec { RawF32, TurboQuantBits3p5, TurboQuantBits2p5, MxFp4, MxFp8, Binary }`. `StoredSlotEnvelope { codec, level, raw_dim, stored_dim, fallback, truncated, payload_bytes }`.

Envelope wire format (`compression/codec.rs`):

```
byte 0      : COMPRESSED_SLOT_TAG (16 / 0x10)
byte 1      : COMPRESSED_SLOT_VERSION (1)
byte 2      : codec code (0–5)
byte 3      : quant level code (0–6)
bytes 4..8  : raw_dim    (u32 big-endian)
bytes 8..12 : stored_dim (u32 big-endian)
byte 12     : flags (bit0 fallback, bit1 truncated)
bytes 13..49: reserved (36 bytes)
bytes 49..53: payload_len (u32 big-endian)
bytes 53..  : quantized payload
```

`matryoshka_truncate_renormalize(raw, truncate_dim)`: require `0 < truncate_dim <= len`, slice `raw[0..truncate_dim]`, renormalize to unit L2.

`compress_slot_batch`: encode with `lens.quant_default`; if `recall_drop <= lens.recall_delta` keep it; else try `fallback_policy` (TurboQuant 2.5/Binary/PQ→3.5, MxFp4→Float8, others→raw); if still over delta, fall back to raw f32. `decode_stored_slot_envelope` returns a `RawF32` envelope when the tag is absent, else parses the header. Errors: `CALYX_VECTOR_COMPRESSION_EMPTY`, `CALYX_VECTOR_COMPRESSION_INVALID`.

---

## 7. Public API summary

Re-exported from `src/lib.rs`. Highlights:

- **Trait & ids**: `Lens`, `Input` (from `calyx_core`); `FrozenLensContract`, `LensDType`, `NormPolicy`.
- **Registry**: `Registry`, `RegistryLensSnapshot`, `FrozenLensSnapshot`, `DeterminismProof`, `DualMeasurement`, `ensure_input_modality`, `ensure_vector_shape`.
- **Spec**: `LensSpec`, `LensRuntime`, `LensHealth`.
- **Runtimes**: `AlgorithmicLens`/`AlgorithmicEncoder`, `TeiHttpLens`/`DEFAULT_TEI_ENDPOINT`, `CandleLens` (+policies, `DEFAULT_CANDLE_MODEL`), `OnnxLens` (+policies), `StaticLookupLens`, `ExternalCmdLens`, `MultimodalAdapterLens` (+spec/axis/pack/license helpers).
- **Panels**: `PanelTemplate`, `PanelSlotSpec`, `PanelLensRuntime`, `AlgorithmicPanelLens`, `InstantiatedPanel`, `instantiate_panel`, `{text,code,legal,medical,bio,civic,media}_default`; panel ops `apply_panel_template`, `swap_panel[_to_target]`, `list_panel[_with_assay]`, `apply_capability_gate`, `PanelDiff`.
- **Swap/backfill/placement**: `SwapController`, `SlotSpec`, `BackfillQueue`, `BackfillScheduler`, `BackfillConfig/Request/Batch/Priority/Watermark`, `choose_placement`, `CpuLensPool`, `PlacementBudget/Plan`.
- **Profile/gate/explain**: `Profiler`, `CapabilityCard`, `ProfileProbe`, `evaluate_capability_gate`, `CapabilityGateDecision/Evaluation/Thresholds`, `max_panel_pairwise_correlation`, `explain_lens`.
- **Commission/compression/drift/ingest/persistence**: as documented in §4, §6, §8.

---

## 8. Persistence and configuration

### 8.1 Vault persistence (`src/persistence.rs`)

`SNAPSHOT_VERSION = 1`. `persist_vault_panel_state(vault_dir, panel, registry)`:
1. Serialize `panel` to pretty JSON, BLAKE3-hash, write `panel/panel-v{version}-{hash[:16]}.json`, build an `ImmutableRef`.
2. Build `VaultRegistrySnapshot { version: 1, panel_ref, lenses: registry.lens_snapshots() }`, serialize, hash, write `registry/registry-{hash[:16]}.json`.
3. Open the vault `ManifestStore`, increment `manifest_seq`, set `panel_ref`/`registry_ref`, validate, write.

Returns `VaultPanelWrite { manifest_seq, durable_seq, panel_ref, registry_ref }`. Format is JSON throughout.

`load_vault_panel_state(vault_dir)` loads the manifest, deserializes the panel, optionally loads + validates the `VaultRegistrySnapshot` (version + `panel_ref` must match the manifest), and rebuilds the registry: for each `RegistryLensSnapshot` it checks `lens_id == contract.lens_id()`, reconstructs the runtime lens (or substitutes a `PersistedUnavailableLens` stub that errors `lens_unreachable` on measure), and calls `register_persisted_arc` preserving the `DeterminismProof`. Returns `VaultPanelState { panel, registry, registry_snapshot }`.

### 8.2 Configuration knobs

| Knob | Default | Where |
|------|---------|-------|
| `CALYX_ALLOW_NONCOMMERCIAL_LENSES` (env) | unset (deny) | `runtime/adapters` license gate |
| `CALYX_CAPABILITY_MIN_SIGNAL_BITS` (env) | `calyx_assay` `MIN_SIGNAL_BITS` | capability gate threshold override |
| `CALYX_CAPABILITY_MAX_PAIRWISE_CORR` (env) | `calyx_assay` `MAX_PAIRWISE_CORR` | capability gate threshold override |
| `HF_HOME` / `CALYX_HOME` (env) | `.hf-cache` fallback | `runtime/common.rs::default_hf_cache_root` |
| TEI timeout / max_batch | 30 s / 32 | `TeiHttpLens` |
| `DEFAULT_TEI_ENDPOINT` | `http://127.0.0.1:8088/embed` | `tei_http.rs` |
| `DEFAULT_CANDLE_MODEL` | `sentence-transformers/all-MiniLM-L6-v2` | `candle.rs` |
| `DEFAULT_MAX_TOKENS` | 512 | `runtime/common.rs`, static-lookup |
| `BackfillConfig` | max_concurrent 4, batch_size 16, throttle 50 ms | `backfill.rs` |
| ingest cap / overhead / breaker | 16 MiB / 64 B / 3 timeouts / 30 000 ms | `ingest_microbatch.rs` |
| `ProfileOptions` | low_spread 0.02, low_distance 0.001 | `profile.rs` |
| `LensSpec.recall_delta` | 0.02 | `spec.rs` |
| `LensSpec.quant_default` | `QuantPolicy::turboquant_default()` | `spec.rs` |
| `NormPolicy::unit` tolerance | `1.0e-3` | `frozen.rs` |
| compression tag / version | 16 / 1 | `compression/mod.rs` |
| `SNAPSHOT_VERSION` | 1 | `persistence.rs` |

Not determined from source: a single TOML/`calyx.toml` config schema for this crate — configuration is via the constructor parameters, env vars, and per-spec fields listed above rather than a dedicated config file.
