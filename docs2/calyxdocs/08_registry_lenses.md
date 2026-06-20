# 08 — Registry & Lenses (calyx-registry)

`calyx-registry` is the lens (frozen embedder) registry. Each lens is a frozen
measurement instrument that turns a raw `Input` into a `SlotVector`. A lens is
content-addressed by a `FrozenLensContract`, registered fail-closed, validated on
every measurement, and hot-swapped into a versioned `Panel` with priority-ordered
lazy backfill. Seven concrete runtimes implement the trait (algorithmic, TEI HTTP,
candle-local BERT, ONNX, static-lookup, multimodal-adapter, external-cmd) plus three
specialised fastembed lenses (sparse/SPLADE, BGE-M3 multi-vector, cross-encoder
reranker), three closed-form temporal lenses (E2/E3/E4), and a commissioned
algorithmic lens.

**Source files covered:**
- `crates/calyx-registry/src/lib.rs`
- `crates/calyx-registry/src/lens.rs`
- `crates/calyx-registry/src/spec.rs`
- `crates/calyx-registry/src/frozen.rs`
- `crates/calyx-registry/src/swap.rs`
- `crates/calyx-registry/src/backfill.rs`
- `crates/calyx-registry/src/commission.rs`
- `crates/calyx-registry/src/runtime/mod.rs`
- `crates/calyx-registry/src/runtime/algorithmic.rs`
- `crates/calyx-registry/src/runtime/tei_http.rs`
- `crates/calyx-registry/src/runtime/candle.rs`
- `crates/calyx-registry/src/runtime/candle/options.rs`
- `crates/calyx-registry/src/runtime/onnx.rs`
- `crates/calyx-registry/src/runtime/onnx/custom.rs`
- `crates/calyx-registry/src/runtime/onnx/special/{mod,models,vectors}.rs`
- `crates/calyx-registry/src/runtime/static_lookup.rs`
- `crates/calyx-registry/src/runtime/external_cmd.rs`
- `crates/calyx-registry/src/runtime/adapters/{mod,axis,lens,pack}.rs`
- `crates/calyx-registry/src/temporal/{mod,e2_recency,e3_periodic,e4_positional}.rs`
- `crates/calyx-registry/src/panels/{mod,defaults}.rs`
- `crates/calyx-registry/src/panel_ops.rs`
- `crates/calyx-registry/src/profile.rs` + `profile/gating.rs`
- `crates/calyx-registry/src/placement.rs`
- `crates/calyx-registry/src/compression/mod.rs`
- `crates/calyx-registry/src/ingest_microbatch.rs`
- `crates/calyx-registry/src/drift.rs`, `explain.rs`, `persistence.rs`
- `crates/calyx-core/src/traits.rs` (the `Lens` trait, `Input`)
- `crates/calyx-core/src/ids.rs` (`LensId`, `content_address`)
- `crates/calyx-core/src/enums.rs`, `crates/calyx-core/src/model/{vector,slot}.rs`
- `crates/calyx-core/src/error.rs` (error codes)

See [05_core.md](05_core.md) for `Lens`/`Input`/ids, [07_forge_math_runtime.md](07_forge_math_runtime.md)
for quantization, [09_sextant_search.md](09_sextant_search.md) for slot indexes,
[11_assay_signal_bits.md](11_assay_signal_bits.md) for signal bits used in the capability gate.

---

## 1. The Lens / embedder abstraction

### 1.1 The `Lens` trait

The core trait is **`Lens`**, defined in `crates/calyx-core/src/traits.rs` (re-exported
as `calyx_registry::Lens`). It is object-safe (`Send + Sync`). Every runtime in this
crate implements it.

| Method | Signature | Behavior |
|---|---|---|
| `id` | `fn id(&self) -> LensId` | Stable frozen content id. |
| `shape` | `fn shape(&self) -> SlotShape` | Vector shape the lens emits. |
| `modality` | `fn modality(&self) -> Modality` | Accepted input modality. |
| `measure` | `fn measure(&self, input: &Input) -> Result<SlotVector>` | Deterministically measures one input. |
| `measure_batch` | `fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>>` | Default impl maps `measure` over inputs; overridden by HTTP/external/ONNX runtimes for true batching. |

`Input` (in `traits.rs`) is `{ modality: Modality, bytes: Vec<u8>, pointer: Option<String> }`.
The lens "measures" raw `bytes`; the byte encoding is runtime-specific (UTF-8 text,
PNG/JPEG, RIFF/WAVE, little-endian timestamps, etc.).

A **lens** in code is therefore: a Rust value implementing `Lens` whose `id()` equals
`FrozenLensContract::lens_id()`, paired in the `Registry` with that contract and an
optional structured `LensSpec`.

### 1.2 `SlotShape` / `SlotVector` (vector forms)

From `calyx-core/src/enums.rs` and `model/vector.rs`:

| `SlotShape` | `SlotVector` produced |
|---|---|
| `Dense(u32)` | `Dense { dim: u32, data: Vec<f32> }` |
| `Sparse(u32)` | `Sparse { dim: u32, entries: Vec<SparseEntry{idx:u32,val:f32}> }` |
| `Multi { token_dim: u32 }` | `Multi { token_dim: u32, tokens: Vec<Vec<f32>> }` |

`SlotVector::Absent { reason: AbsentReason }` is an explicit absence (never a zero
vector); all shape validators reject `Absent` with `CALYX_LENS_DIM_MISMATCH`.
`Modality` variants: `Text, Code, Image, Audio, Video, Protein, Dna, Molecule,
Structured, Mixed`.

### 1.3 Shape / numeric validation helpers (`lens.rs`)

- `ensure_input_modality(lens, input)` — `input.modality` must equal `lens.modality()`,
  else `CALYX_LENS_DIM_MISMATCH`.
- `ensure_vector_shape(lens_id, shape, vector)` — dense dim AND `data.len()` must equal
  the expected dim; sparse indices must be `< dim`; multi token lengths must equal
  `token_dim`. Non-finite values → `CALYX_LENS_NUMERICAL_INVARIANT`.

---

## 2. Embedder runtimes

`runtime/mod.rs` declares the runtime modules. Every concrete runtime builds its own
`FrozenLensContract` (see §3) and sets `id = contract.lens_id()`.

### 2.1 `AlgorithmicLens` — `runtime/algorithmic.rs`

Deterministic feature encoders with **no model weights**. Encoder enum
`AlgorithmicEncoder`:

| Variant | `dim()` | Output computation |
|---|---|---|
| `ByteFeatures` | 16 | Byte/char-class ratios (ascii, whitespace, alpha, digit, punct, upper, lower, control, NUL, path, brackets, newline), `log2(len)/32`, mean-byte ratio, two FNV-1a hash parts mapped to `[-1,1]`. Empty input → `[1,0,…]`. |
| `Scalar` | 1 | Mean byte / 255 (`[0.0]` if empty). |
| `OneHot { buckets }` | `buckets` (min 1) | FNV-1a hash of bytes mod buckets → one-hot. |
| `AstStyle` | 8 | Per-length counts of `fn`/`let`/`struct`/`impl`, brace, `;`, `(`, newline. |

Inputs: any modality (checked). Output: `Dense`. NormPolicy `Finite` (ByteFeatures) or
`None` (others). FNV constants: offset `0xcbf29ce484222325`, prime `0x100000001b3`.

### 2.2 `TeiHttpLens` — `runtime/tei_http.rs`

Blocking HTTP client to a resident HuggingFace **TEI** endpoint. Fields: `endpoint`,
`modality`, `dim`, `timeout` (default 30s), `max_batch` (default 32, min 1).
`measure_batch` POSTs `{"inputs":[...]}` over a hand-rolled HTTP/1.1 `Connection: close`
request (supports chunked transfer-encoding), parses bare arrays, `{"embedding":[…]}`,
or OpenAI-style `{"data":[{"embedding":[…]}]}`. Dim must equal `self.dim`; non-finite →
`CALYX_LENS_NUMERICAL_INVARIANT`. Constructors: `new`, `resident_8088`,
`with_timeout`, `with_max_batch`. `DEFAULT_TEI_ENDPOINT = "http://127.0.0.1:8088/embed"`.
Inputs must be UTF-8 text. Connection errors → `CALYX_LENS_UNREACHABLE`.

### 2.3 `CandleLens` — `runtime/candle.rs` (+ `candle/{load,options,pooling}.rs`)

Local BERT inference via `candle-transformers::models::bert::BertModel`, tokenized with
`tokenizers`. Fields include `Mutex<BertModel>`, optional `finite_replay_model`
(F32 model re-run when a half-precision run trips `CALYX_LENS_NUMERICAL_INVARIANT`),
`device_policy`, `precision`, `pooling`, `max_tokens`. Modality is fixed **Text**;
output `Dense(hidden_size)` (read from model config). `measure` tokenizes, runs forward
with token_type/attention tensors, casts hidden states to F32, pools, applies norm, and
re-verifies against the contract.

Option enums (`candle/options.rs`):
- `CandleDevicePolicy`: `CpuExplicit` | `CudaFailLoud { ordinal }`.
- `CandlePrecision`: `F32 | F16 | BF16` (parse accepts `float32/fp16/bfloat16` etc.).
- `CandlePoolingPolicy`: `Mean | Cls` (parse accepts `first_token`).
- `CandleModelFiles`/`CandleFileSpec` carry `config`, `tokenizer`, `weights`,
  `contract_paths`.

`DEFAULT_CANDLE_MODEL = "sentence-transformers/all-MiniLM-L6-v2"`. Constructors:
`all_minilm_l6_v2`, `all_minilm_l6_v2_cuda_fail_loud`, `from_hf_cache[_with_device_policy]`,
`from_model[_with_options]`, `from_files`, `from_lens_spec`. The contract `weights_sha256`
is the SHA-256 over all artifact files; mismatch vs an expected hash →
`CALYX_LENS_FROZEN_VIOLATION`. Feature flag `candle-cuda` enables CUDA in candle.

### 2.4 `OnnxLens` — `runtime/onnx.rs` (+ `onnx/{custom,fastembed_runtime}.rs`)

Two backends behind one lens (`OnnxBackend`): `FastEmbed(Mutex<TextEmbedding>)` from the
`fastembed` crate, or `Custom(Mutex<CustomOnnxRuntime>)` using `ort` sessions directly.
`runtime_name()` returns `"onnx-fastembed"` or `"onnx-custom"`. Output `Dense(dim)`;
fastembed path L2-normalizes and checks dim. Enums:
- `OnnxProviderPolicy`: `CudaFailLoud` (`"cuda:0,error_on_failure,no_cpu_fallback"`) |
  `CpuExplicit` (`"cpu_explicit,no_cuda"`).
- `PoolingPolicy`: `Mean | Cls | LastToken` (custom path reads pooling from config JSON).

Structs `OnnxModelFiles` (cache_dir, model_code, model_file, tokenizer, config,
special_tokens_map, tokenizer_config, contract_paths) and `OnnxFileSpec`. Constructors:
`all_minilm_l6_v2[_cpu_explicit]`, `from_hf_cache[_with_policy]`, `from_model[_with_policy]`
(takes a `fastembed::EmbeddingModel`), `from_files`, `from_lens_spec`. Config errors use
`CALYX_LENS_CONFIG_INVALID`; inference failure → `CALYX_LENS_UNREACHABLE`. Dependencies:
`ort = =2.0.0-rc.12` (cuda), `fastembed 5.16`, `tokenizers 0.22`.

#### 2.4a Special fastembed lenses — `runtime/onnx/special/{mod,models,vectors}.rs`

Three additional fastembed-backed lenses cover retrieval shapes the plain dense
`OnnxLens` cannot express. Each holds a `Mutex<…>` model, an `OnnxModelFiles` set, and an
`OnnxProviderPolicy`; `mod.rs` defines the lenses, `models.rs` resolves model names →
fastembed enums plus dims/shapes/norms, and `vectors.rs` holds the batching, CUDA-leak,
and contract helpers.

- **`FastembedSparseLens`** — SPLADE-style sparse term vectors (`SparseTextEmbedding`).
  `runtime_name()` `"fastembed-sparse"`; output sparse (`finite` norm).
- **`FastembedBgem3Lens`** — BGE-M3 multi-vector (`Bgem3Embedding`). One lens type, three
  output modes via `FastembedBgem3Output` (`Dense | Sparse | Colbert`); the dense path is
  unit-normalised, sparse/colbert use `finite` norm. `runtime_name()`
  `"fastembed-bgem3-dense|sparse|colbert"`.
- **`FastembedRerankerLens`** — cross-encoder reranker (`TextRerank`); scores query↔doc
  pairs (`finite` norm).

Exported from `lib.rs` as `FastembedSparseLens`, `FastembedBgem3Lens`,
`FastembedRerankerLens`, and `FastembedBgem3Output`. Commission these with
`--runtime fastembed-sparse | fastembed-bgem3-{dense,sparse,colbert} | fastembed-reranker`
(`lens commission`, see [20_cli_and_daemon_reference.md](20_cli_and_daemon_reference.md));
`--norm` defaults per-runtime (`unit` for dense, `finite` for sparse/colbert/reranker)
unless given explicitly.

### 2.5 `StaticLookupLens` — `runtime/static_lookup.rs`

Token-embedding lookup-and-mean-pool (model2vec / static word vectors). Reads a
memory-mapped (`memmap2`) matrix file with a 24-byte header:

```
[0..8)   magic  "CXLKUP1\0"
[8..12)  rows   u32 LE
[12..16) dim    u32 LE
[16]     dtype  1=int8, 2=f16, 3=f32
[20..24) scale  f32 LE (must be finite, > 0)
```

`measure` tokenizes (max 512 tokens), skips `[UNK]`/`<unk>`, sums the per-token rows
(`raw * scale`, with a hand-rolled f16→f32), divides by count, applies norm. Empty or
all-unknown text → a zero-safe unit vector `[1,0,…]`. Modality fixed **Text**; output
`Dense(dim)`. `StaticLookupDType`: `Int8 | F16 | F32` (widths 1/2/4). Hash drift vs an
expected `weights_sha256` → `CALYX_LENS_FROZEN_VIOLATION`; bad header →
`CALYX_LENS_CONFIG_INVALID`.

### 2.6 `MultimodalAdapterLens` — `runtime/adapters/lens.rs` (+ `axis.rs`, `pack.rs`)

PH74 adapter lenses for non-text modalities. **It does not run a neural net**: it
validates input bytes per axis then emits a **deterministic SHA-256-derived projection**
(`deterministic_projection`: repeatedly hashes `["multimodal-adapter-vector-v1", axis,
model_id, bytes, counter]`, maps 4-byte chunks to `[-1,1]`), then L2-normalizes. Output
`Dense(dim)`, NormPolicy unit.

`MultimodalAxis`: `Image | Audio | Protein | Dna | Molecule` (maps to the matching
`Modality`). Input validation: Image = PNG/JPEG magic; Audio = `RIFF…WAVE`;
Protein/Dna = allowed amino-acid / `ACGTN` alphabet; Molecule = SMILES charset.
`MultimodalAdapterSpec` carries `license` and `allow_non_commercial`.
License gate: `ensure_license_allowed` rejects non-commercial licenses with
`CALYX_LICENSE_DENIED` unless `allow_non_commercial` (env
`CALYX_ALLOW_NONCOMMERCIAL_LENSES` truthy via `allow_noncommercial_from_env`).
`pack.rs` provides `default_multimodal_lens_specs`, `MultimodalLensPackEntry`,
`register_multimodal_lens_pack`.

### 2.7 `ExternalCmdLens` — `runtime/external_cmd.rs`

Spawns an external process and exchanges length-prefixed JSON frames over stdin/stdout
(`[u32 BE length][payload]`). Request `{modality, inputs:[bytes…]}`, response
`{vectors:[[f32]…]}`. Output `Dense(dim)`, NormPolicy `None`. Enforces a timeout
(default 30s) using two worker threads + `try_wait`, killing the child on timeout/exit
failure (`CALYX_LENS_UNREACHABLE`). `with_timeout` configurable.

### 2.8 Temporal lenses — `temporal/{e2_recency,e3_periodic,e4_positional}.rs`

Closed-form algorithmic lenses (no weights), all modality **Structured**, all carrying
`TEMPORAL_FLAGS = { retrieval_only: true, excluded_from_dedup: true }`.

| Lens | Shape | Input bytes | Output |
|---|---|---|---|
| `E2RecencyLens` | `Dense(1)` | 8-byte LE i64 timestamp | recency score from `DecayFunction::{Linear{max_age_secs}, Exponential{half_life_secs}, Step}`. Step: `<1h→0.8`, `<24h→0.5`, else `0.1`. |
| `E3PeriodicLens` | `Dense(2)` | 8-byte LE i64 timestamp | `[hour_score, dow_score]` — circular distance to `target_hour`(0..23)/`target_day_of_week`(Mon=0, 0..6); `None` target ⇒ 1.0; `use_now` derives target from `reference_time`. |
| `E4PositionalLens` | `Dense(4)` | 16 bytes: u64 position, u64 total | sin/cos of forward & backward position ratio·π; `SequenceDirection::{Forward,Backward,Both}` masks pairs; `MultiAnchorMode::{First,Last,All}`. |

Temporal lens ids: `LensId::from_bytes(content_address([name_parts…]))` over a debug
spec string. Bad-length input → `CALYX_LENS_DIM_MISMATCH`.

### 2.9 `CommissionedLens` — `commission.rs`

A lens produced offline from a corpus + base model. `commission_lens(request, dir)`
hashes the corpus (`corpus_hash = sha256(corpus parts)`), derives
`weights_sha256 = sha256(["commissioned-lens-v1", base_model, corpus_hash])`, writes a
`<lens_id>.commissioned.json` artifact, and returns a `CommissionedLensArtifact`
(lens_id, contract, `LensSpec` with `runtime = Algorithmic{kind:"commissioned:<base>"}`).
`measure` emits a deterministic SHA-256-seeded `Dense(dim)` vector (this is a frozen
deterministic stand-in, not real model inference). `register_commissioned` registers it
via `register_frozen_with_spec`. `commission/manifest.rs` adds `LensForgeManifest` /
`LensForgeFile` and `lens_spec_from_manifest[_path|_with_license_override]`.

### Gaps (runtimes)
- `MultimodalAdapterLens` and `CommissionedLens` emit **hash-derived deterministic
  projections**, not learned embeddings — the code is explicit about this.
- The plan’s `tei-http`/`onnx`/`candle-local` "real model" promise holds; the multimodal
  axes are placeholder projections pending real adapter models.

---

## 3. Content-addressing & freezing

### 3.1 `LensId` and `content_address`

`LensId` (`calyx-core/src/ids.rs`) is a 16-byte hex id. The hash primitive
`content_address(parts)` is **BLAKE3** over length-delimited parts (each part prefixed by
its `u64 BE` length), truncated to 16 bytes. `LensId::from_parts(name, weights_sha256,
corpus_hash, output_shape)` = `content_address([name, weights, corpus, output_shape])`.

### 3.2 `FrozenLensContract` preimage (`frozen.rs`)

The contract holds `name, weights_sha256:[u8;32], corpus_hash:[u8;32], shape:SlotShape,
modality, dtype:LensDType(F32), norm:NormPolicy`. The canonical lens id is:

```
lens_id = LensId::from_parts(
    name,
    weights_sha256,
    corpus_hash,
    output_shape_fingerprint().as_bytes()
)
output_shape_fingerprint = "dtype=f32;shape=<dense:N|sparse:N|multi:N>;norm=<finite|unit|declared-by-model>"
```

So the id depends on name + weight hash + corpus hash + shape + dtype + norm-class. Two
lenses with identical specs get the same id across vaults.

`sha256_digest(parts)` (also length-delimited, **SHA-256**) builds the per-runtime
`weights_sha256`/`corpus_hash`. Examples of the preimage strings each runtime feeds:
- algorithmic byte-features: weights `sha256("algorithmic-byte-features-v1")`, corpus
  `sha256("algorithmic-data-oblivious")`.
- candle: corpus `sha256("candle-local-bert-v2", model_id, max_tokens, precision,
  pooling, norm, finite_replay)`, weights = SHA-256 of artifact files.
- static-lookup: corpus `sha256("static-lookup-model2vec-v1", dim, dtype)`.
- multimodal: weights `sha256("multimodal-adapter-v1", axis, model_id, license)`.
- external-cmd: weights `sha256(cmd, args)`, corpus `sha256("external-cmd-runtime-v1")`.
- TEI: weights `sha256(endpoint)`, corpus `sha256("tei-http-runtime")`.

> Note: `LensSpec::lens_id()` (`spec.rs`) uses a **different** preimage —
> `content_address([name, weights, corpus, "shape=…;norm=…;runtime=…"])` (a Debug string
> of shape/norm/runtime). This is the metadata-only id; the authoritative on-the-wire id
> is `FrozenLensContract::lens_id()` and `verify_registration` enforces
> `lens.id() == contract.lens_id()`.

### 3.3 Immutability / freeze guarantees

`FrozenLensContract` enforces, fail-closed:
- `verify_registration(lens)`: `lens.id()` must equal the contract id
  (`CALYX_LENS_FROZEN_VIOLATION`); `shape`/`modality` must match (`CALYX_LENS_DIM_MISMATCH`).
- `verify_vector(lens_id, vector)`: shape exact (`ensure_vector_shape`), finite, and per
  `NormPolicy` unit-norm or declared-norm within tolerance (else
  `CALYX_LENS_NUMERICAL_INVARIANT`). Default unit tolerance `1.0e-3`.
- `verify_determinism_probe(lens, probe)`: measures twice and requires byte-identical
  JSON output; otherwise `CALYX_LENS_FROZEN_VIOLATION`.

`NormPolicy`: `None | L2{tolerance} | DeclaredByModel{declared_norm,tolerance} |
Finite | Unit{tolerance}` (`None`/`Finite` = finite-only; `L2`/`Unit` = unit norm).
`LensDType` has only `F32`.

### 3.4 Runtime drift (`drift.rs`)

`RuntimeGolden { lens_id, runtime_version, golden_output, tolerance }`. `evaluate(observed)`
→ `DriftDecision::Reuse{max_abs_delta}` if within tolerance, else `Drifted{old, new_lens_id,
…, signal:"CALYX_LENS_RUNTIME_DRIFT"}` where the new id is derived from the observed
output — drift becomes a *new* LensId, never silent reuse.

---

## 4. The Registry

`Registry` (`lens.rs`) wraps a `BTreeMap<LensId, RegistryEntry>` where
`RegistryEntry = { lens: Arc<dyn Lens>, frozen: Option<FrozenLensContract>, spec:
Option<LensSpec>, determinism: DeterminismProof }`. `DeterminismProof` is
`ProbeVerified` or `ContractOnlyExemption`.

### 4.1 Registration (fail-closed)

| Method | Behavior |
|---|---|
| `register` / `register_with_spec` | **Always error** `CALYX_LENS_FROZEN_VIOLATION` — a frozen contract is mandatory. |
| `register_frozen(lens, contract)` | Verifies registration; inserts; exemption proof. Duplicate id → `CALYX_REGISTRY_DUPLICATE`. |
| `register_frozen_with_spec(lens, contract, spec)` | As above + stores `LensSpec`. |
| `register_frozen_with_probe(lens, contract, probe)` | Runs the determinism probe → `ProbeVerified`. |
| `register_persisted_arc(...)` | crate-internal, re-hydrates a persisted `Arc<dyn Lens>`. |

### 4.2 Measurement

`measure(id, input)` looks up the entry (`CALYX_LENS_UNREACHABLE` if absent), checks
modality, calls `lens.measure`, then `validate_entry` (re-verify contract + vector, or
shape-only if no contract). `measure_batch` validates count and every vector.
`measure_dual(id, input)` requires `Asymmetry::Dual`; it measures the input and its
byte-reversed form and errors if both directions are identical. `measure_ingest_microbatch`
drives admission across lenses (see §7). Introspection: `contains`, `find_lens_by_name`,
`frozen_contract`, `frozen_lens_snapshots`, `lens_spec`, `lens_snapshots`,
`determinism_proof`, `health`.

### 4.3 `LensSpec` and `LensRuntime` (spec.rs)

`LensSpec` fields: `name, runtime:LensRuntime, output:SlotShape, modality, weights_sha256,
corpus_hash, norm_policy, axis:Option<String>, asymmetry, quant_default(QuantPolicy,
default TurboQuant 3.5bpc), truncate_dim:Option<u32>, recall_delta(f32, default 0.02),
retrieval_only, excluded_from_dedup`.

`LensRuntime` variants (serde snake_case): `Algorithmic{kind}`, `TeiHttp{endpoint}`,
`CandleLocal{model_id,files,dtype,pooling}`, `Onnx{model_id,files}`,
`FastembedSparse{model_id,files}`, `FastembedBgem3{model_id,files,output}` (output =
`FastembedBgem3Output::{Dense,Sparse,Colbert}`), `FastembedReranker{model_id,files}`,
`StaticLookup{embeddings_file,tokenizer,dim}`, `MultimodalAdapter{axis,model_id}`,
`ExternalCmd{cmd,args}`. The three fastembed-special variants share `files`-existence
health probing with `Onnx`. `LensHealth`: `Loaded | Cold | Failing{code,reason}`;
`LensSpec::health()` probes (TCP probe for TEI with 250ms timeout, file existence for
candle/onnx/static, `PATH`/file check for external-cmd).

### 4.4 Hot-swap lifecycle (`swap.rs`)

`SwapController { panel: Panel, queue: BackfillQueue }`. `SlotSpec` declares a slot
(`key, lens_id, shape, modality, asymmetry, quant, axis, retrieval_only,
excluded_from_dedup`).

**`add_lens(registry, spec, candidates, now)`** steps:
1. If an identical non-retired slot exists, no-op (returns it, `ready:true`) after
   confirming the lens is registered.
2. `ensure_unique_slot`: reject duplicate slot key, or a lens already active/parked
   (`CALYX_LENS_FROZEN_VIOLATION`).
3. `ensure_registered_lens`: lens must have a frozen contract whose shape+modality match
   the slot.
4. Allocate `next_slot_id` (max+1, u16), `bump_panel` (version+1, stamp `created_at`).
5. Push a new `Slot{state:Active, added_at_panel_version}`.
6. Enqueue every `BackfillCandidate{cx_id, priority}` into the `BackfillQueue`.
7. Return `AddLensOutcome{slot, panel_version, index:IndexPlaceholder{ready:false,
   queued}, queued}` — searchable immediately for new constellations; old ones fill
   lazily.

`add_lens_durable` additionally enqueues a `BackfillRequest` into a persistent
`BackfillScheduler` and rolls back panel/queue/scheduler on failure.

Lifecycle transitions (`park_lens`/`unpark_lens`/`retire_lens` → `set_slot_state`):
bump panel version, set `SlotState::{Parked,Active,Retired}`; any non-Active transition
cancels that slot’s pending backfill tasks. **Retired is terminal** — any transition
out of Retired is `CALYX_LENS_FROZEN_VIOLATION`. No constellation is rewritten; retired
slots stay readable for history.

`BackfillQueue` is the in-controller queue: `enqueue[_many]`, `claim_batch(limit)`
(priority desc, then id, marks `InFlight`), `complete`, `retry` (re-`Pending`,
`attempts+1`), `cancel_slot`, `pending_len`/`completed_len`. `BackfillTask` states:
`Pending | InFlight | Complete | Failed`.

---

## 5. Lazy backfill (durable scheduler, `backfill.rs`)

`BackfillScheduler` persists `PersistedScheduler{config, next_allowed_ms,
requests: BTreeMap<"slot:lens", RequestState>}` to a JSON file via atomic temp-write +
rename + fsync. `BackfillPriority`: `Normal(0) | Hot(1) | Kernel(2)`.
`BackfillConfig` defaults: `max_concurrent=4, batch_size=16, throttle_ms=50`.

Algorithm:
1. `enqueue(request)` inserts a `RequestState{request, next_index:0, in_flight, last_processed,
   complete}` keyed by `slot:lens` (idempotent — existing key kept).
2. `claim_next_batch(now_ms)`:
   - if `now_ms < next_allowed_ms` → return a `throttled` empty batch;
   - if active (in-flight) count ≥ `max_concurrent.max(1)` → `None`;
   - pick the highest-priority incomplete request with no in-flight batch, ties broken by
     **least progress** (`Reverse(next_index)`);
   - slice `candidates[next_index .. next_index+batch_size]` into `in_flight`, return a
     `BackfillBatch{slot_id, lens_id, candidates, throttled:false}`.
3. `complete_batch(slot, lens, now_ms)`: advance `next_index` by the in-flight len, record
   `last_processed`, clear in-flight, set `complete` when all candidates done, and set
   `next_allowed_ms = now_ms + throttle_ms`.
4. On reopen, in-flight batches are cleared so a claimed-but-uncompleted batch is retried;
   corrupt state fails closed with `CALYX_STALE_DERIVED`.

`watermarks()` reports `{processed, pending, in_flight, complete, last_processed}` per
request. Each `add_lens` schedules only the affected slot — no global stop-the-world
re-embed.

---

## 6. Panels

A **panel** is a versioned set of slots — `calyx_core::Panel { version, slots:Vec<Slot>,
created_at, kernel_ref, guard_ref }`. `Slot` carries `slot_id, slot_key, lens_id, shape,
modality, asymmetry, quant, resource, axis, retrieval_only, excluded_from_dedup,
bits_about: BTreeMap<AnchorKind,Signal>, state, added_at_panel_version`.

### 6.1 Panel templates (`panels/mod.rs`)

`PanelTemplate { name, slots: Vec<PanelSlotSpec> }`. `PanelSlotSpec { name, runtime:
PanelLensRuntime, output, modality, retrieval_only, excluded_from_dedup, required,
asymmetry }`. `PanelLensRuntime`: `Registry{name} | TeiHttp{endpoint} | Algorithmic{lens}
| ExternalCmd{name} | Placeholder{name}`. `AlgorithmicPanelLens`: `ByteFeatures |
TemporalRecent | TemporalPeriodic | TemporalPositional | Scalar`.
`instantiate_panel(template, created_at)` → `InstantiatedPanel{template_name, panel,
slot_specs}`, assigning `slot_id = index`, `added_at_panel_version = index+1`,
`panel.version = slots.len()`. `slot_lens_id` for a template slot is a BLAKE3
content-address of a `"template:name:runtime:output:modality:retrieval_only:
excluded_from_dedup"` debug string.

### 6.2 Default panels (`panels/defaults.rs`)

All eight end with the three temporal slots (`E2_recency` Dense(1), `E3_periodic`
Dense(2), `E4_positional` Dense(4)), all `retrieval_only + excluded_from_dedup`.

| Template | Non-temporal slots (selected) | Notable shapes |
|---|---|---|
| `text-default` | E1_semantic (TEI 768), keyword_splade (Sparse 30522), paraphrase, entity, causal_dual (Dual) | 5 + 3 temporal |
| `code-default` | semantic, ast, cfg, dataflow, type_graph, trace, diff, oracle_anchor, static_analysis, runtime, reasoning, scalars (all algorithmic Dense(16), Code) | 12 + 3 |
| `civic-default` | polis_axis_01..21 (Scalar Dense(1)) | 21 + 3 |
| `legal-default` | legal_bert_small, general_semantic, keyword_splade, entity, causal_dual | registry-runtime |
| `medical-default` | biomedbert_small_embeddings, general_semantic, medical_entity | registry-runtime |
| `bio-default` | protein_esm2 (Protein), dna_dnabert2 (Dna), molecule_chemberta (Molecule), general_semantic | adapter Dense(16) |
| `media-default` | media_semantic, image_siglip2, audio_clap, audio_wave (256), audio_emotion (128), **speaker_wavlm (Audio Dense 512)**, transcript, **style_register (Text Dense 768)** | identity-lock slots |

TEI default endpoint for `text-default` slots: `http://127.0.0.1:8088`.

### 6.3 Panel operations (`panel_ops.rs`)

`list_panel(panel, registry)` → `Vec<PanelSlotListing{slot, lens_name, has_contract,
determinism, health}>`. `list_panel_with_assay` adds Assay metrics. `apply_panel_template`
→ `AppliedPanelTemplate{resolved: Vec<ResolvedPanelLens>, missing}` (`CALYX_PANEL_LENS_MISSING`
when a referenced lens isn’t registered). `apply_capability_gate` →
`PanelCapabilityGateOutcome`. `swap_panel(panel, template, now)` / `swap_panel_to_target`
return a `PanelDiff{added, retired, …}` — bulk add/retire to match a template.

---

## 7. Capability profiling, gate, placement, compression, microbatch

### 7.1 Profiling & capability card (`profile.rs`)

`Profiler::profile_lens(registry, lens_id, probes)` measures each `ProfileProbe`
(`{input, label}`), projects to dense, and computes a `CapabilityCard`:
`{lens_id, probe_count, signal:Option<f32> (Assay, pending here), proxy_signal,
differentiation, proxy_differentiation, spread:SpreadMetrics, separation:SeparationMetrics,
cost:CostMetrics, coverage:CoverageMetrics, health, low_spread}`. `SpreadMetrics` uses the
participation ratio / stable rank of probe variances; `SeparationMetrics` is a silhouette
over labeled probes; `proxy_signal = clamp01(coverage.rate · normalized_participation_ratio ·
proxy_differentiation)`. VRAM is sampled via `nvidia-smi` (0 if unavailable). Empty/degenerate
probe sets → `CALYX_ASSAY_INSUFFICIENT_SAMPLES`. `MetricSource`: `ProfileProxy | AssayPending
| AssayStore`.

### 7.2 Capability gate (`profile/gating.rs`)

`evaluate_capability_gate(card, max_pairwise_corr, thresholds)` →
`CapabilityGateEvaluation{decision, signal_bits, signal_grounded, max_pairwise_corr,
thresholds, reason, card}`. `CapabilityGateDecision`: `Admit | Park | Retire`. Logic:
- `corr > max_pairwise_corr` → **Retire** (redundant);
- no grounded Assay signal → **Park**;
- `low_spread` (collapsed) → **Park**;
- `signal_bits < min_signal_bits` → **Park**;
- else **Admit**.

`CapabilityGateThresholds` defaults come from `calyx_assay::contract::{MIN_SIGNAL_BITS,
MAX_PAIRWISE_CORR}`, overridable via env `CALYX_CAPABILITY_MIN_SIGNAL_BITS` /
`CALYX_CAPABILITY_MAX_PAIRWISE_CORR`. `append_capability_gate_ledger` writes the decision
to the provenance ledger.

### 7.3 Placement (`placement.rs`)

`choose_placement(...)` returns a `PlacementPlan` against a `PlacementBudget`
(RAM/VRAM). `CpuLensPool` / `CpuPoolAdmission` admit CPU lenses. Budget violations →
`CALYX_RAM_BUDGET_EXCEEDED` / `CALYX_VRAM_BUDGET_EXCEEDED` with remediation strings
`LENS_RAM_REMEDIATION` / `LENS_VRAM_REMEDIATION`.

### 7.4 Vector compression (`compression/mod.rs`)

`compress_slot_batch` / `write_compressed_slot_batch` quantize a slot’s vectors and
verify recall@k against raw; if `recall_drop > lens.recall_delta` it falls back
(`fallback_policy`) and finally to raw f32. `StoredSlotCodec`: `RawF32 |
TurboQuantBits3p5 | TurboQuantBits2p5 | MxFp4 | MxFp8 | Binary`. `COMPRESSED_SLOT_TAG=16`,
`COMPRESSED_SLOT_VERSION=1`. `matryoshka_truncate_renormalize` truncates+renorms dense
vectors. Errors: `CALYX_VECTOR_COMPRESSION_EMPTY`, `CALYX_VECTOR_COMPRESSION_INVALID`.

### 7.5 Ingest microbatch admission (`ingest_microbatch.rs`)

`IngestMicrobatchController` bounds per-microbatch bytes (`DEFAULT_INGEST_MICROBATCH_CAP_BYTES
= 16 MiB`, `INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES = 64`) and degrades gracefully.
`Registry::measure_ingest_microbatch` runs each lens’ batch under admission and returns an
`IngestPanelReadout` of `IngestLensOutcome{status: Measured|Degraded|…}`.

### 7.6 Persistence & explain

`persistence.rs`: `persist_vault_panel_state` / `load_vault_panel_state` (round-trips
`VaultPanelState`, `VaultPanelWrite`, `VaultRegistrySnapshot`).
`explain.rs`: `explain_lens[_from_card]` → `LensExplanation{lens_id, corpus_hash(hex),
axis, runtime, bits, redundancy, cost_ms_per_input, vram_bytes}` (bits/redundancy are
`"provisional"` until an Assay report is attached).

---

## 8. Error taxonomy & feature flags

Error codes raised by this crate (constructors in `calyx-core/src/error.rs`):

| Code | Raised when |
|---|---|
| `CALYX_LENS_FROZEN_VIOLATION` | id≠contract, register without contract, retire→other, weight-hash drift, non-deterministic probe. |
| `CALYX_LENS_DIM_MISMATCH` | shape/modality/dim/count mismatch, bad input length, absent vector. |
| `CALYX_LENS_NUMERICAL_INVARIANT` | NaN/Inf, norm out of tolerance. |
| `CALYX_LENS_UNREACHABLE` | lens not registered; TEI/external/ONNX I/O or timeout. |
| `CALYX_REGISTRY_DUPLICATE` | duplicate `LensId` registration. |
| `CALYX_LENS_CONFIG_INVALID` | bad ONNX/candle/static config or unsupported axis/dtype/pooling. |
| `CALYX_LICENSE_DENIED` | non-commercial multimodal license without allow flag. |
| `CALYX_PANEL_LENS_MISSING` | template references an unregistered lens. |
| `CALYX_RAM_BUDGET_EXCEEDED` / `CALYX_VRAM_BUDGET_EXCEEDED` | placement budget exceeded. |
| `CALYX_VECTOR_COMPRESSION_EMPTY` / `_INVALID` | compression preconditions. |
| `CALYX_STALE_DERIVED` | backfill scheduler corruption / persist failure. |
| `CALYX_ASSAY_INSUFFICIENT_SAMPLES` | profiling with no measurable probes. |
| `CALYX_LENS_RUNTIME_DRIFT` (signal string) | golden drift beyond tolerance (drift.rs). |

**Cargo features** (`Cargo.toml`): `default = []`; `candle-cuda` enables CUDA in
candle-core/-nn/-transformers. Dependencies: `blake3`, `candle-* 0.10.2`, `fastembed 5.16`,
`hf-hub 0.5`, `memmap2`, `ort =2.0.0-rc.12` (cuda), `tokenizers 0.22`, `sha2`, `serde`.
Dev-deps pull in `calyx-loom`, `calyx-sextant`, `proptest`.

---

## 9. Divergences from the plan (`docs/dbprdplans/05_EMBEDDER_REGISTRY.md`)

- The plan lists 5 runtimes; the code ships **7** (`tei-http`, `candle-local`, `onnx`,
  `external-cmd`, `algorithmic`, plus `static-lookup` and `multimodal-adapter`), the
  temporal trio, and a `commissioned` lens.
- The plan’s `Registry.measure(lens_id, input)` and `add_lens`/`retire_lens`/`park_lens`
  ergonomics match the code (`Registry::measure`, `SwapController`).
- The plan’s `add_lens` "create empty slot CF + ANN index + codebook placeholder" is
  represented by `IndexPlaceholder` (the index/CF allocation itself lives in the storage/
  search crates, not here). See [09_sextant_search.md](09_sextant_search.md).
- `media-default` matches the plan’s identity-lock intent: `speaker_wavlm` Dense(512) and
  `style_register` Dense(768).

## 10. Gaps / not covered

- `MultimodalAdapterLens` and `CommissionedLens` produce deterministic **hash-derived**
  vectors, not learned model outputs; the registry-runtime panel slots (legal/medical/bio/
  media) name models that require corresponding registered lenses (else
  `CALYX_PANEL_LENS_MISSING`).
- Real model inference paths (candle BERT, ONNX/fastembed, TEI HTTP) require local
  artifacts / a running endpoint; their FSV tests are `#[ignore]` or env-gated.
- `bits`/`redundancy` in `LensExplanation` are provisional placeholders pending Assay.
- Internal helpers in `runtime/common.rs`, `candle/{load,pooling}`, `onnx/{custom,
  fastembed_runtime}`, `compression/{codec,recall}`, and `profile/{assay,cost}` are
  summarized by behavior, not field-by-field.
