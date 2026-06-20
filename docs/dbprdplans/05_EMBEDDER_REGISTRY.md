# 05 — Registry: Lenses as Designable Instruments

> **Living-system role:** perception (the senses) + growth by differentiation — the lenses are how Calyx perceives; adding/pruning them is how it grows (A31 — DOCTRINE §1b)

Implements A4/A5/A6. Solves the user's stated pain: never hand-wire a multi-embedder pipeline again.

> **This is the backbone (Doctrine §5).** Calyx's single most important ergonomic: make it as easy as possible to **plug embedders in/out, analyze their value/bits, and use their associations**. A new lens is one call; its worth is one number; the kernel over its associations is one call at any scope. Every decision in this doc is judged against "does this make plugging in a lens or reading its bits easier?" If not, it's wrong.

## 1. What a lens is

A **lens** is a frozen embedder treated as a measurement instrument (paper §1.3): trained on a corpus, weights frozen, reporting where an input sits in that corpus's association web. Calyx owns the lens *lifecycle*, not the training.

```
Lens {
  lens_id: LensId,                 // content-addressed (03 §2)
  name: String,                    // "gte-multilingual-base", "want-cause-v2", "wavlm-speaker"
  weights_sha256: Hash,            // frozen-weight fingerprint (A4)
  corpus_hash: Hash,               // what it remembers = the axis it measures
  runtime: LensRuntime,            // TEI-http | onnx | candle-local | external-cmd | algorithmic
  output: SlotShape,               // Dense(d) | Sparse(d) | Multi(token_d)
  modality: Modality,
  asymmetry: Option<Asymmetry>,    // dual cause/effect, paraphrase/context
  normalize: NormPolicy,           // L2 | none | declared-by-model
  quant_default: QuantPolicy,
  cost: LensCost,                  // ms/input, VRAM MB, batch ceiling
  health: LensHealth,              // loaded | cold | failing
}
```

## 2. Lens runtimes (how a measurement is taken)

| Runtime | Mechanism | aiwonder fit |
|---|---|---|
| `tei-http` | resident HF TEI endpoint | reuse `:8088` general, `:8090` legal, `:8089` reranker — **never start throwaway TEI** (gotcha) |
| `candle-local` | weights loaded into Forge (candle/cudarc), run on sm_120 | new bespoke lenses, low-latency, no HTTP hop |
| `onnx` | ORT CUDA EP | portability, embedded vaults |
| `external-cmd` | spawn a process, typed stdin/stdout protocol | exotic modalities (audio WavLM, image CLIP) |
| `algorithmic` | deterministic feature encoder, no NN (AST, CFG, scalars, one-hot oracle) | absorbed from ContextGraph `e_*` instruments + `algorithmic_embedder_synthesis` |

A lens is **registered once** with its runtime; thereafter `Registry.measure(lens_id, input)` is uniform. Embedded vaults prefer `candle-local`/`onnx` (no server); server vaults prefer `tei-http` to resident services.


## 3. Hot-swap (A5) — the core ergonomic win

```
add_lens(spec) -> LensId:
  1. validate frozen contract (weights hash present, output shape declared, runtime reachable)
  2. content-address -> LensId; if already registered in vault, no-op
  3. allocate next SlotId in Panel; bump panel_version
  4. create empty slot CF + ANN index + codebook placeholder
  5. schedule lazy backfill: existing constellations get the new slot measured in the background,
     priority-ordered (kernel cx first, then by query frequency) — NOT a global stop-the-world re-embed
  6. Assay schedules a bits-about-outcome measurement once backfill reaches sample quorum (07)
  -> lens is searchable immediately for new cx; backfilled cx become searchable as they fill

retire_lens(slot_id):
  1. mark Slot.state = Retired (tombstone); stop measuring it on new cx; stop searching it
  2. keep its columns/index for historical constellations (interpretability) until GC policy prunes
  3. bump panel_version
```

**No existing constellation is rewritten and no global re-embed runs.** The single property that turns "a nightmare every time" into one call.

## 4. The frozen contract (A4, fail-closed)

Enforced at register and every measure:
- weights hash MUST match the registered fingerprint; mismatch → `CALYX_LENS_FROZEN_VIOLATION`.
- output dim/dtype MUST equal `Slot.shape`; mismatch → `CALYX_LENS_DIM_MISMATCH`.
- output MUST be finite (no NaN/Inf) and, if `normalize=L2`, unit-norm within tolerance; else `CALYX_LENS_NUMERICAL_INVARIANT`.
- a lens MUST NOT be observed to change between two measurements of the same input (determinism probe run on aiwonder).
- a frozen lens MUST NOT receive gradients (no training path touches it). (Inherits ContextGraph `frozen_target`/`grad_hook` guards.)

## 5. Capability assay — "what is this lens good for?" (fast)

To **swap lenses in/out and quickly analyze their capabilities**, `Registry.profile(lens_id, probe_set)` runs a cheap, standardized capability card without full ingestion:

| Capability metric | How | Meaning |
|---|---|---|
| **Signal** (per anchor) | Assay MI on a labeled probe set (`07`) | bits about each real outcome — the headline number |
| **Differentiation** | max pairwise corr vs current panel | does it duplicate an existing lens? (≤0.6 to admit) |
| **Spread / effective dim** | participation ratio / stable rank of probe vectors | is it collapsed (low signal) or rich? |
| **Separation** | silhouette on labeled probes | does it cluster the outcome cleanly? |
| **Cost** | ms/input, VRAM, batch ceiling | budget fit |
| **Coverage** | fraction of probe inputs it can encode (non-degenerate) | modality fit |

Output = a **Lens Capability Card** (one screen / one JSON), so an agent decides "keep / park / retire" in seconds (A17). Generalizes ContextGraph `embedder_foundationality` + Polis `embedder_semantic_probe_suite`.

## 6. Designable & dynamic lenses (A6)

Supports the paper's "commission a lens for any axis":
- **Commissioned (frozen-on-corpus):** point Registry at a corpus + a base model → produce a frozen lens (offline), register it. Calyx tracks `corpus_hash` as the axis identity.
- **Algorithmic synthesis:** when no NN lens carries the needed bits, synthesize a deterministic feature lens (e.g. a typed graph/scalar encoder) — absorbed from ContextGraph `algorithmic_embedder_synthesis` / `learned_head_synthesis`. Anneal can *propose* a new lens when Assay shows an outcome the panel can't predict (the `I(panel;outcome)` deficit, `07`/`12`).
- **Dynamic learned heads:** small projection heads on top of frozen lenses (e.g. LoRA-style causal direction) are allowed and versioned, but they are *online state* (`03 §6`), never mutations of the frozen lens.

## 7. Default panels (batteries included)

Ready-made panels make a new vault multi-lens on day one (no plumbing):

| Panel | Slots | Source heritage |
|---|---|---|
| `text-default` | E1 semantic (GTE) · keyword/SPLADE · paraphrase · entity · causal(dual) · **E2/E3/E4 temporal** | ContextGraph 13-lens subset |
| `code-default` | semantic · AST · CFG · dataflow · type-graph · trace · diff · oracle(anchor) · static-analysis · runtime · reasoning · scalars | ContextGraph ME-JEPA 15-slot panel |
| `civic-default` | the 21-slot Polis Constellation (11 axes) | socialmedia2.com slate |
| `media-default` | semantic · image(CLIP) · audio-wave · audio-emotion · speaker(WavLM) · transcript · style/register | ClipCannon N=7 |

The `speaker(WavLM)` and `style` slots in `media-default` power **identity-locked generation** (`09 §5b`): the paper's measured anchors — a voice reproduced at **0.961 mean WavLM speaker-similarity** (DNSMOS 3.93/3.93) and a style model that holds character under prompt injection (with zero-shot Golden-Age Spanish transfer) — are Ward verdicts over these slots. Commission a `speaker`/`style` lens once; identity-lock comes from Registry + Ward, not bespoke code.

A vault picks a panel and immediately gets DDA + Assay + kernel + guard. Custom panels are `add_lens` calls.

**Temporal family in every panel (A27, `25`).** All default panels include the three temporal lenses **E2 Temporal-Recent, E3 Temporal-Periodic, E4 Temporal-Positional** (from ContextGraph): **algorithmic** (closed-form, no trained weights), **search/retrieval-only** under AP-60 (never dominant; post-retrieval boost), excluded from dedup agreement (`25 §5`). They make every Calyx database time-aware by default and feed the separate time-capture sidecar used for as-of reads and walking state forward/backward through time.

## 8. Registry API (agent-facing summary; full in `18`)

```
add_lens(spec) -> LensId
retire_lens(slot_id)
park_lens(slot_id) / unpark_lens(slot_id)         # keep, don't search (low-signal)
profile(lens_id, probe_set?) -> CapabilityCard
list_panel(vault) -> [Slot + bits_about + state]
swap_panel(vault, panel_template) -> diff          # bulk add/retire to match a template
explain_lens(lens_id) -> {corpus_hash, axis, bits, redundancy, cost}
```

## 7b. Templates are first-class, situation-specific, swappable (A36)

A **panel template** is a named, versioned, content-addressed set of **≥ 10 lens specs** plus its measured ensemble signal — a real database object, not a hardcoded default. The right 10+ lenses **differ by what is being measured**, so the operator develops a library of templates and **swaps them in/out per vault or per query** without re-embedding existing constellations (A5).

| Template (example) | ≥ 10 lenses | When |
|---|---|---|
| `video-capture` | semantic · image(SigLIP2) · image(DINOv2) · audio(CLAP) · speech-emotion · speaker(WavLM) · transcript(Whisper) · OCR-text · motion/scene · E2/E3/E4 temporal | video / multimodal streams |
| `literary-essence` | semantic · style/register · syntax-meter/prosody · rhetorical-device · affect/sentiment · persona/voice · lexical-archaism · entity · paraphrase · E4 positional | replicating an author's voice/spirit (e.g. Shakespeare) |
| `code-oracle` | semantic · AST · CFG · dataflow · type-graph · trace · diff · static-analysis · reasoning · oracle(anchor) · E2/E3/E4 | code understanding + pass/fail oracle |
| `text-deep` | GTE/Qwen3 semantic · BGE-M3 dense · E5-Mistral · Nomic · Jina · SPLADE sparse · paraphrase · entity · causal(dual) · E2/E3/E4 | high-recall text retrieval at scale |

**Same situation, same template; different situation, different template.** Video capture and "replicate Shakespeare's essence" share almost no lenses — the system must let us template each set and load it on demand. Template API (extends `§8`):

```
save_template(name, [lens_spec...], notes) -> TemplateId      # content-addressed; versioned
list_templates() -> [TemplateId + ensemble_signal + n_lenses]
swap_panel(vault, template) -> diff                           # bulk add/retire to match (≥10 enforced)
profile_template(template, probe_set) -> EnsembleCard         # the template's measured fitness (§9)
fork_template(template) -> editable copy                      # develop a new one from an existing
```

A template is admitted for testing only if it has **≥ 10 lenses** (A35) and a recorded `EnsembleCard` (§9); its **fitness for a situation is itself a number**, so picking a template is a measurement, not a guess.
The CLI stores this as immutable template-object bytes: `panel template profile`
may attach `--assay-card <ensemble_card.json>`, copies
`EnsembleCard.a37_diversity` into `ensemble_card.a37_admission`, records the
assay-card path + BLAKE3, and marks `a37_gate_eligible=true` only when the
persisted Assay status is `gate_passed`. `panel template list` prints the A37
status for operator readback. `panel template swap --require-a37-gate` is
fail-closed: it refuses with `CALYX_PANEL_TEMPLATE_A37_GATE_REFUSED` before
touching the vault unless the immutable template object carries a gate-passed
Assay EnsembleCard for the same content-lens roster. Temporal time-control lanes
remain serialized as sidecars and never count toward the content-lens floor.

## 9. The 10+ roster: which lenses to try first, where to get them, how to value them

**Where to get them (all free / open-weight, A34; HuggingFace via `hf_hub_token`).** Run dense lenses on the resident TEI services (`§2`, never throwaway TEI); image/audio via `external-cmd`/`onnx`; temporal/graph/scalar via `algorithmic`.

**Text (dense + sparse) — try first, pick 5–7 *diverse* ones (diversity = low redundancy, A7):**
- `gte-multilingual-base` (already live, TEI `:8088`) · `Qwen3-Embedding` (top MTEB multilingual/code) · `BGE-M3` (dense+sparse+multi-vector, 100+ langs) · `intfloat/e5-mistral-7b-instruct` · `nomic-embed-text-v1.5` · `jinaai/jina-embeddings-v3` · `NV-Embed-v2` · `Alibaba-NLP/gte-Qwen2` · `dunzhang/stella_en_1.5B_v5`.
- **Sparse / lexical (a different signal, not just another dense model):** `naver/splade-v3` (the full-text lens, `DOCTRINE §3`).
- **Reranker (cross-encoder, asymmetric):** the resident `gte` reranker (TEI `:8089`).

**Multimodal / image:** `google/siglip2` (vision-language, multilingual) · `facebook/dinov2` (self-supervised — **diverse from CLIP/SigLIP**, different signal) · CLIP (baseline).

**Audio / speech:** `laion/CLAP` (audio↔text) · `microsoft/wavlm-base-plus-sv` (speaker, the `media-default` identity-lock lens) · Whisper encoder (transcript) · a wav2vec2-based **speech-emotion** model.

**Algorithmic (no weights, always available):** E2/E3/E4 temporal · AST/CFG/dataflow (code) · entity/graph/scalar encoders.

**Choose for diversity, not just leaderboard rank.** Two top-MTEB dense models are often **redundant** (corr > 0.6 → A7 rejects the second); the panel's *intelligence* comes from **complementary** signal — a dense semantic model + a sparse lexical model + a reranker + an image model + an audio model carry far more joint bits than ten near-identical dense models. The capability card (`§5`) and the Ensemble Card (below) make this measurable.

**How to value them in Calyx (value is associational — `07`, A35).** A lens's worth is **not** its solo benchmark; it is the signal it adds *given the others*. So we never profile one lens in isolation for a keep/cut decision — we measure it **inside the panel** of ≥ 10:

```
# the protocol (all numbers from Assay, 07; read back from the persisted assay store)
1. assemble the ≥10-lens candidate panel over a real labeled probe corpus (BEIR/MS MARCO + domain anchors)
2. per-lens solo signal     I(slot_k; anchor)                          # cheap first pass, NOT the decision
3. per-lens MARGINAL value  I(panel; anchor) − I(panel∖k; anchor)      # the load-bearing number — needs the panel
4. cross-term / SYNERGY     I(a,b; anchor) − max(I(a;anchor),I(b;anchor))   # the bits a pair adds together
5. PID per lens             unique / redundant / synergistic bits      # partial-information decomposition
6. pairwise redundancy      corr / NMI ≤ 0.6 to keep (A7)
7. n_eff + panel sufficiency I(panel; anchor) vs H(anchor)             # is the 10+ panel even sufficient?
→ EnsembleCard: keep / park / retire each lens by its *marginal* contribution; propose a new lens for any deficit (12)
```

> **You cannot do steps 3–5 with one or two lenses.** Marginal value, synergy, and PID are *defined only relative to the rest of the panel*; a triple (≥ 3) is the smallest system with non-trivial structure, and ≥ 10 is needed for stable estimates. This is the information-theoretic reason the testing floor is 10 (A35), not a preference.

## 9b. The diversity gate (A37) — count is necessary, not sufficient

**≥ 10 (A35) is a *count* gate; it does not guarantee a *diverse* panel.** Ten general-purpose dense semantic embedders (bge / e5 / gte / mpnet / MiniLM / nomic / jina …) are **one association family**: on the same text they are near-synonyms, so by PID their bits are **redundant**, not unique/synergistic — the panel passes "10 lenses" while adding almost nothing past the first few. The pairwise differentiation contract (A7, corr ≤ 0.6) is **necessary but not sufficient**: it is per-pair and per-admission, so a panel can be **collectively rank-deficient** (low `n_eff`) while *every* pair sits under 0.6.

A panel admitted for any **gate** (test/bench/FSV/SLO/`J`/scale) MUST therefore also clear a **panel-level diversity gate**:

```
# diversity gate (numbers from Assay 07 + the panel matrix; read back from the persisted artifact)
D1 family span      ≥2 distinct association families present, fit to the job (A36):
                    dense-semantic(general)·dense-semantic(domain: legal/clinical/scientific/financial)·
                    lexical/sparse(SPLADE/keyword)·entity/graph·character/byte·structural(AST/CFG)·
                    reranker/asymmetric · temporal-sidecar(A27 time-control/as-of traversal, NOT counted toward the ≥10 floor)
D2 redundancy bound n_eff(panel) ≥ threshold AND mean pairwise corr/NMI ≤ threshold  # panel-WIDE, not per-pair
D3 no collapse      every admitted lens contributes measurable UNIQUE PID bits given the rest (§9 step 5);
                    the marginal-bits curve does not flatten to ~0 after the first few lenses
D4 diversity number EnsembleCard records {n_eff, mean_redundancy, sum_unique_pid_bits} so "diverse enough?"
                    is answered by the bytes, not asserted; a template (A36) carries this with its signal
→ a panel that hits 10 by COUNT but fails D1–D3 is DIAGNOSTIC-ONLY and FAILS CLOSED for gates (A16),
  exactly like a <10 panel.
```

`calyx assay stream-fbin` and `calyx assay i8bin-ensemble-card` enforce this
split in their command contracts. Default `--mode gate` refuses full encodes or
Assay CF writes unless the gate evidence is eligible. Homogeneous or control
runs must say `--diagnostic` / `--baseline`; those runs still persist streamed
bytes, the matrix, marginal/PID readout, and temporal sidecar evidence, but the
evidence is marked diagnostic-only and cannot be used as an A37 gate verdict.
Temporal lanes remain the time-control surface for as-of, forward, and backward
traversal; they are serialized in the A37 readout but never count toward the
content-family floor.

Countable content signals are explicitly typed. Learned neural encoders persist
`signal_kind=learned_encoder`; deterministic lexical/byte/entity/structural
feature lenses persist `signal_kind=deterministic_content_feature`. Legacy
`signal_kind=algorithmic`, placeholders, unknown runtimes, and temporal/as-of
sidecars fail closed for gate-bearing content counts.

**A genuinely diverse text panel** (vs the 10-clone anti-pattern) mixes families: 2–3 dense-semantic anchors (one general + one domain) · a **lexical/sparse** lens (exact terms/names/acronyms dense models blur) · an **entity** lens (for GDELT: actors/orgs/CAMEO codes/country pairs — often the highest-unique-info axis) · a **character/byte** lens (name transliteration robustness) · the **temporal** sidecar for as-of/forward/backward time traversal. That mix maximizes unique + synergistic PID per slot; ten dense clones maximize redundancy.

**PH68 GDELT scale template (#801; cross-ref #796/#787):** the current proven
GDELT/civic scale roster is documented in
`docs/implementation/PH68-diskann-spann/README.md` under "PH68 GDELT Scale
Roster Template". Its aiwonder report reads back 11 content lenses, 5 GPU
content lenses, and 1 temporal sidecar with
`temporal_counts_toward_content_floor=false`; families span dense
general/domain, static semantic, byte/character, lexical sparse,
late-interaction token, and entity/CAMEO graph.

**Homogeneous panels are valid as labeled controls.** Deliberately running a single-family panel to **measure** the redundancy collapse — its correlation matrix, marginal-bits decay, `n_eff`, and fused-RRF gain vs the best single lens — is the empirical evidence for this gate and is encouraged; it is diagnostic, never a production gate (`DOCTRINE §0`).

## 9c. The resource-bounded roster (A38, #1 priority) — see `05a`

The **canonical candidate catalogue** (every embedder worth considering, by family,
with params / dim / quantised VRAM) and the **binding default panel "Constellation-24"**
(the optimal diverse set that fits one 24 GB GPU under ≤ 20 GB resident lens weights,
covering text general + legal/medical/clinical/biomedical/scientific/financial/code/
multilingual · multiple image (CLIP-style + DINO-style) · audio speech/speaker/music/
environmental · doc-image · protein/DNA/molecule) live in
**`05a_EMBEDDER_ROSTER_VRAM_BUDGET.md`** (A38, `DOCTRINE §10.29`). That document is the
roster source-of-truth; this §9/§9b define *how* a roster is measured and gated, `05a`
defines *which* lenses and *under what hardware budget*. Maximise count × measured
`bits / VRAM-MB` (#729); quantise (INT8/FP16/MRL/static) to fit more lenses; admit only
on A35 + A37 `gate_passed` + measured `Σ VRAM ≤ 20 GB` + fused-RRF beats the 1–2-lens
control.

**One sentence:** the Registry turns "frozen embedder" into a database object with a lifecycle, a capability card, and a one-call hot-swap; templates turn "the right 10+ lenses for *this* job" into a swappable, measured object — so the multi-lens system builds itself.
