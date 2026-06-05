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
- a lens MUST NOT be observed to change between two measurements of the same input (determinism probe in CI).
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

**Temporal family in every panel (A27, `25`).** All default panels include the three temporal lenses **E2 Temporal-Recent, E3 Temporal-Periodic, E4 Temporal-Positional** (from ContextGraph): **algorithmic** (closed-form, no trained weights), **search/retrieval-only** under AP-60 (never dominant; post-retrieval boost), excluded from dedup agreement (`25 §5`). They make every Calyx database time-aware by default.

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

**One sentence:** the Registry turns "frozen embedder" into a database object with a lifecycle, a capability card, and a one-call hot-swap — so the multi-lens system builds itself.
