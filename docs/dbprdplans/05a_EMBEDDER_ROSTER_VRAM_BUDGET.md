# 05a — Embedder Roster & the 24 GB VRAM Budget (canonical roster, A38 candidate)

> **Status: PROPOSED #1 PRIORITY / TOP DOCTRINE (2026-06-20).** This document is the
> exhaustive candidate catalogue of frozen embedder lenses for Calyx and the
> binding recommendation for the **default general-purpose panel** that must fit a
> **24 GB GPU (RTX 4090 / 5090 class) with ≤ 20 GB of resident VRAM for lens weights**,
> leaving headroom for activations, the ONNX/TEI runtimes, and the index/search hot set.
>
> It operationalises A35 (≥10 lenses), A36 (templates), A37 (associational
> diversity) into an actual, measured, resource-bounded roster. It is the roster
> referenced by `DOCTRINE` and `05 §9`. **Numbers here are the *target*; the bits
> and the resident VRAM cost are decided by measurement (A32/§0), not by this table.**

---

## §1 — The goal (verbatim, bake into all context)

**Calyx's default panel must be the maximally diverse, maximally grounded set of frozen
embedders that fits on one 24 GB GPU under a 20 GB lens-weight budget, covering every
modality and every domain a general database is asked to serve.**

Concretely:
- **Budget:** one 24 GB card (4090/5090/A5000/L4-24/A10-24 class or better). Keep the
  **sum of resident lens weights ≤ 20 GB**; reserve ≥ 4 GB for activation working set,
  CUDA/ORT/TEI context, and the in-RAM/on-disk index hot set. On a 32 GB 5090 the same
  roster runs with large headroom (the current aiwonder card).
- **Maximise count × signal:** prefer **many small high-signal lenses** over a few
  giants. A lens earns its slot by **bits-per-VRAM-MB** (signal density, #729), not raw
  bits. Quantise aggressively — **ONNX + INT8 / FP8 / Matryoshka-truncation** — to fit
  more lenses (A25 maximal *measured* compression; INT8 ≈ 4× smaller than FP32, ≈ 2×
  smaller than FP16, with near-zero retrieval-quality loss for embedders).
- **Cover all use cases (one general DB):** text (general semantic + **legal, medical,
  clinical, biomedical, scientific, financial, code, multilingual**), **multiple image
  embedders** (zero-shot CLIP-style *and* self-supervised DINO-style — they see
  differently), **audio** (speech, speaker, music, environmental), document-image, and
  the science modalities (protein / DNA / molecule). Text is the priority and must show
  **many different perspectives** of the same input.
- **Diversity is the gate (A37):** the panel must span ≥ 2 (target: most) association
  *families*, clear a panel-wide redundancy bound (`n_eff`, mean corr/NMI), and show a
  non-collapsing unique-PID curve. 10 dense-semantic clones is a *failure*, not a panel.

---

## §1b — This is the AGI substrate: cover *all general situations* (binding framing)

Calyx is the **AGI / Oracle substrate** (`DOCTRINE §1`, A20/A21/A22), not a niche retriever.
The roster is therefore held to an **artificial-general-intelligence standard: it must give
the system grounded perception of *any* situation a general intelligence could be asked
about** — every modality (text, code, image, audio, video, document, protein, DNA,
molecule, structured/tabular, time) and every domain perspective (general semantic, legal,
medical, clinical, biomedical, scientific, financial, civic, multilingual, and any future
specialty). Three consequences are binding:

1. **Open-ended, not a fixed list.** The catalogue in §3 is exhaustive *as of today* but is
   a **living set**. The objective is *coverage of the space of general situations*, so any
   discovered modality/domain with no adequate lens is a **coverage gap = a new acquisition
   issue**, not an accepted blind spot (A30 connection-of-truths: the system proposes the
   highest-grounded lens for an uncovered axis).
2. **Generality = perception breadth × grounded depth.** A35/A37 force *diversity of
   perspective* precisely because general intelligence is the ability to see one thing
   through many independent, grounded lenses and fuse them — the panel is the system's
   **sensorium**. Maximising distinct, non-redundant, grounded bits per situation **is**
   maximising grounded intelligence (A32, `J`).
3. **General default + situational templates (A36).** The resident **"Constellation-24"**
   default panel must be the best *general-purpose* coverage of all situations at once;
   **templates** then specialise (video ≠ literary-essence ≠ code ≠ genomics) and swap in
   per vault/query (A5) **without re-embedding**. The catalogue must be rich enough that any
   situation's template can be assembled from it — and where it can't, that's a gap to fill.
4. **Self-extending (the living-intelligence loop, A31).** `propose_lens` (#725) → measure
   bits → gate (A35/A37/A38) → hot-add (A5): the system **grows its own perception** when a
   corpus reveals signal an existing lens can't capture. The roster is something Calyx
   **maintains and expands itself**, under the same fits-in-24 GB, max-`bits/VRAM-MB` budget.

> **One line:** the embedder roster is Calyx's senses; an AGI substrate must be able to
> perceive *any* general situation, so the roster must be the broadest grounded sensorium
> that fits the hardware — and must keep growing toward total coverage.

---

## §2 — VRAM budgeting methodology (how the table numbers are derived)

Resident VRAM for an embedder ≈ **weights + activation working set + runtime context.**

- **Weights** = `params × bytes_per_param`. FP32 = 4 B, **FP16/BF16 = 2 B**, **INT8 = 1 B**,
  FP8 = 1 B, INT4 = 0.5 B. This dominates and is what the table sizes.
- **Activation working set** ≈ a function of `batch × seq_len × hidden × layers`. For a
  base encoder at batch 32 / seq 512 this is typically **+0.2–0.6 GB**; it is *shared
  transiently*, not per-lens-resident, if lenses run sequentially (see §7 scheduling).
- **Runtime context**: an ONNX Runtime CUDA session and the CUDA context cost a few
  hundred MB **once per process**; TEI containers carry their own. Budget ~1–2 GB total.

**Rule of thumb used below:** "resident" column = INT8 weights for ONNX-quantisable
encoders (the default), FP16 where INT8 hurts or isn't available, **0 MB** for static
(model2vec) lenses, and the model's native size for VLM-class items. Treat every number
as **± a measurement**: the real cost is read back via the `--cost-json` sidecar and the
resource-aware admission packer (#729) gates by `bits / VRAM-MB`.

> A 110 M-param encoder is ≈ **220 MB FP16 / 110 MB INT8**. A 300–335 M base is ≈
> **0.6 GB FP16 / 0.3 GB INT8**. A 0.6 B LLM-embedder is ≈ **1.2 GB FP16 / 0.6 GB INT8**.
> A 7–8 B embedder is ≈ **14–16 GB FP16 / 7–8 GB INT8** — at most *one* of these fits and
> it eats the whole budget, so they are **not** default-roster material.

---

## §3 — Exhaustive candidate catalogue

Legend — **Fam** = A37 association family: `DS-gen` dense-semantic general · `DS-dom`
dense-semantic domain · `LEX` lexical/sparse · `LI` late-interaction/multi-vector ·
`ENT` entity/graph · `RERANK` reranker/asymmetric · `IMG` image · `AUD` audio · `DOC`
document-image · `BIO` protein/DNA/molecule · `MM` unified multimodal · `STATIC` static
token-pool (0 VRAM). Sizes are weights only; **INT8 ≈ ½ the FP16 figure.**

### 3.1 Text — general semantic (the DS-gen pool; pick a *diverse* few, not clones)

| Model | Params | Dim | Ctx | Fam | INT8 ≈ | Notes |
|---|---|---|---|---|---|---|
| **EmbeddingGemma-300M** | 300 M | 768 (MRL 128/256/512) | 2048 | DS-gen | ~0.30 GB | SOTA <500 M, 100+ langs, code-aware; Gemma3 backbone |
| **Qwen3-Embedding-0.6B** | 600 M | 32–1024 (MRL) | 32 K | DS-gen | ~0.60 GB | LLM-based, very high signal, instruction-aware |
| **Qwen3-Embedding-4B** | 4 B | up to 2560 | 32 K | DS-gen | ~4.0 GB | top-tier; *budget-heavy*, optional tier-3 |
| **gte-modernbert-base** | 305 M | 768 | 8192 | DS-gen | ~0.30 GB | modern ModernBERT, long ctx, strong |
| **nomic-embed-text-v1.5** | 137 M | 768 (MRL 64–768) | 8192 | DS-gen | ~0.14 GB | Matryoshka, already commissioned on aiwonder |
| **bge-base-en-v1.5** / **bge-small** | 110 M / 33 M | 768 / 384 | 512 | DS-gen | ~0.11 / 0.03 GB | workhorse; small already commissioned |
| **gte-base-en-v1.5** | 137 M | 768 | 8192 | DS-gen | ~0.14 GB | strong base, long ctx |
| **multilingual-e5-base** / **-large** | 278 M / 560 M | 768 / 1024 | 514 | DS-gen | ~0.28 / 0.56 GB | multilingual; -base already commissioned |
| **mxbai-embed-large-v1** | 335 M | 1024 | 512 | DS-gen | ~0.34 GB | top English retrieval |
| **snowflake-arctic-embed-m-v2.0** | 305 M | 768 | 8192 | DS-gen | ~0.30 GB | strong multilingual, MRL |
| **granite-embedding-278m-multilingual-r2** | 278 M | 768 | 8192 | DS-gen | ~0.28 GB | IBM, Apache-2.0, multilingual |
| **stella_en_400M_v5** | 400 M | 1024 (MRL) | 8192 | DS-gen | ~0.40 GB | high MTEB, MRL |
| **jina-embeddings-v2/v3-base** | 137 M / 570 M | 768 / 1024 | 8192 | DS-gen | ~0.14 / 0.57 GB | v2-base already commissioned; v3 task-LoRA |
| **all-MiniLM-L6-v2** | 22 M | 384 | 256 | DS-gen | ~0.02 GB | cheap baseline; candle build already commissioned |
| **gte-Qwen2-7B / e5-mistral-7b** | 7 B | 3584 / 4096 | 32 K | DS-gen | ~7–8 GB | SOTA but eats the budget; tier-3 only |
| **potion-base-8M (model2vec)** | static | 256 | n/a | STATIC | **0 GB** | CPU, zero VRAM; already commissioned |

### 3.2 Text — domain perspectives (the DS-dom pool; *this* is what makes one input show many faces)

| Model | Params | Domain | Fam | INT8 ≈ | Notes |
|---|---|---|---|---|---|
| **sciNCL** / **SPECTER2** | 110 M | scientific papers | DS-dom | ~0.11 GB | citation-grounded; sciNCL already commissioned |
| **SciBERT** | 110 M | scientific text | DS-dom | ~0.11 GB | classic; replace with sciNCL if batch-unstable (#812) |
| **BioLORD-2023** | 110 M | biomedical concepts | DS-dom | ~0.11 GB | SOTA biomedical sentence embeddings |
| **MedCPT-Query/Article-Encoder** | 110 M | clinical/PubMed retrieval | DS-dom | ~0.11 GB | trained on 255 M PubMed click pairs |
| **S-PubMedBERT-MS-MARCO** / **PubMedBERT** | 110 M | biomedical | DS-dom | ~0.11 GB | strong biomedical embedder |
| **Bio_ClinicalBERT** | 110 M | clinical notes (MIMIC-III) | DS-dom | ~0.11 GB | EHR / clinical |
| **legal-bert-base** / **modernbert-legal** | 110 M / 150 M | legal | DS-dom | ~0.11 / 0.15 GB | legal-bert classic; modernbert-legal long-ctx (TEI on aiwonder) |
| **FinBERT** / finance-embeddings-investopedia | 110 M | finance | DS-dom | ~0.11 GB | financial text |
| **CodeRankEmbed / jina-embeddings-v2-base-code** | 137–160 M | code | DS-dom | ~0.16 GB | code retrieval; jina-code commissionable via ONNX |
| **Qodo-Embed-1-1.5B** | 1.5 B | code (SOTA CoIR) | DS-dom | ~1.5 GB | beats 7 B code models; optional tier-2 |
| **nomic-embed-code** | 7 B | code | DS-dom | ~7 GB | SOTA but tier-3 only |
| **LaBSE** | 471 M | 109-lang sentence | DS-dom | ~0.47 GB | cross-lingual bitext / translation pairs |

### 3.3 Text — lexical / sparse and late-interaction (non-semantic families — the diversity multipliers)

| Model | Params | Fam | INT8 ≈ | Notes |
|---|---|---|---|---|
| **BGE-M3** | 568 M | DS-gen **+ LEX + LI** | ~0.57 GB | **three families in ONE model**: dense + sparse + ColBERT multi-vector; very high diversity/MB |
| **SPLADE-v3 / naver-splade** | 110 M | LEX | ~0.11 GB | learned sparse term-expansion (BM25++) |
| **opensearch-neural-sparse-v2** | 110 M | LEX | ~0.11 GB | doc-side-only sparse, cheap query |
| **answerai-colbert-small-v1** | 33 M | LI | ~0.03 GB | tiny ColBERT, strong late-interaction |
| **jina-colbert-v2** | 560 M | LI | ~0.56 GB | multilingual ColBERT, 89 langs |
| *(algorithmic)* SPLADE-keyword / token-hash | 0 (weights-free) | LEX / LI | 0 GB | already in Calyx; diagnostic family fillers |

### 3.4 Reranker / asymmetric (the RERANK family — cross-encoder, retrieval_only slot)

| Model | Params | Fam | INT8 ≈ | Notes |
|---|---|---|---|---|
| **bge-reranker-v2-m3** | 568 M | RERANK | ~0.57 GB | multilingual cross-encoder, SOTA |
| **jina-reranker-v2-base-multilingual** | 278 M | RERANK | ~0.28 GB | fast multilingual reranker |
| **mxbai-rerank-base-v2** | ~150 M | RERANK | ~0.15 GB | strong, small |
| **Qwen3-Reranker-0.6B** | 600 M | RERANK | ~0.60 GB | LLM reranker, instruction-aware |

### 3.5 Image (the user asked for MULTIPLE — zero-shot *and* self-supervised see differently)

| Model | Params | Dim | Fam | INT8/FP16 ≈ | Notes |
|---|---|---|---|---|---|
| **SigLIP2-base-patch16** | 200 M | 768 | IMG/MM | ~0.4 GB FP16 | best zero-shot image-text, multilingual |
| **SigLIP2-so400m** | 400 M | 1152 | IMG/MM | ~0.8 GB FP16 | higher-signal SigLIP (ColPali's encoder) |
| **DINOv2-base** / **-large** | 86 M / 300 M | 768 / 1024 | IMG | ~0.17 / 0.6 GB | **self-supervised**; SOTA fine-grained visual features (no text) |
| **DINOv3-ViT-H+/16** | 840 M | 1280 | IMG | ~0.84 GB INT8 | latest self-supervised, dense features |
| **CLIP ViT-B/32** | 151 M | 512 | IMG/MM | ~0.15 GB INT8 | classic, fast, broad; ViT-B/32 already commissioned |
| **CLIP ViT-L/14** | 304 M | 768 | IMG/MM | ~0.30 GB INT8 | stronger zero-shot |
| **jina-clip-v2** | 865 M | 1024 (MRL) | MM | ~0.87 GB INT8 | multilingual text+image unified, 8 K text ctx |
| **nomic-embed-vision-v1.5** | 92 M | 768 | MM | ~0.09 GB | **aligned to nomic-embed-text** latent space (shared space!) |
| **Marqo-FashionCLIP / domain CLIPs** | ~150 M | 512 | IMG/MM | ~0.15 GB | domain image (commerce, medical-CLIP, RemoteCLIP) |

### 3.6 Audio (the user asked for these — speech, speaker, music, environmental are different families)

| Model | Params | Fam | FP16 ≈ | Notes |
|---|---|---|---|---|
| **larger_clap_general (LAION-CLAP)** | 193 M | AUD/MM | ~0.39 GB | audio↔text semantic (open-vocab); already adapter-staged |
| **CLAP-htsat-fused** | 153 M | AUD/MM | ~0.31 GB | audio-text, fused variant |
| **wav2vec2-base** / **HuBERT-base** | 95 M | AUD | ~0.19 GB | speech content/phonetic; wav2vec2 already staged |
| **WavLM-base-plus** | 95 M | AUD | ~0.19 GB | **speaker / robust** speech, noisy-condition |
| **MERT-v1-95M** | 95 M | AUD | ~0.19 GB | **music** representation |
| **PANNs CNN14** | 80 M | AUD | ~0.16 GB | environmental/audio-event tagging |
| **Whisper-base encoder** | 74 M | AUD | ~0.15 GB | ASR-grounded speech features |
| **pyannote/wespeaker speaker-embedding** | ~20 M | AUD/ENT | ~0.04 GB | speaker identity (diarization anchor) |

### 3.7 Document-image (visual PDF/table retrieval — the DOC/LI family)

| Model | Params | Fam | INT8 ≈ | Notes |
|---|---|---|---|---|
| **ColPali-v1.3 / ColQwen2** | 2–3 B | DOC/LI | ~2–3 GB | embeds whole page images (tables/figures/layout) via late interaction; very high value for PDFs |
| **ColSmol-256M / ColSmolVLM** | 256 M | DOC/LI | ~0.26 GB | small visual-doc retriever; budget-friendly DOC slot |

### 3.8 Science modalities (BIO — protein / DNA / molecule)

| Model | Params | Modality | Fam | FP16/INT8 ≈ | Notes |
|---|---|---|---|---|---|
| **ESM2-t12-35M** / **t30-150M** / **t33-650M** | 35 / 150 / 650 M | protein | BIO | 0.07 / 0.30 / 0.65 GB INT8 | pick by budget; 35 M is a cheap, strong protein slot |
| **ESM-C-300M** | 300 M | protein | BIO | ~0.30 GB | newer ESM, better/efficient |
| **ProtT5-XL** | 1.2 B | protein | BIO | ~1.2 GB | optional larger protein |
| **DNABERT-2-117M** | 117 M | DNA | BIO | ~0.12 GB | BPE genomic; already staged |
| **Nucleotide-Transformer-v2-50M/100M** | 50–100 M | DNA | BIO | ~0.05–0.1 GB | multi-species genomic |
| **ChemBERTa-77M-MLM** | 77 M | molecule (SMILES) | BIO | ~0.08 GB | chemistry; small |
| **MolFormer-XL** | 44 M | molecule | BIO | ~0.04 GB | strong molecular rep |

### 3.9 Unified multimodal (one model, many modalities — MM family, optional heavy)

| Model | Params | Fam | INT8 ≈ | Notes |
|---|---|---|---|---|
| **jina-embeddings-v4** | 3.8 B | MM (text+image+doc) | ~3.8 GB | unified pathway, no modality gap; tier-3 |
| **GME-Qwen2-VL-2B** / **VLM2Vec** | 2 B | MM | ~2 GB | VLM-derived universal embedder; tier-2/3 |
| **ImageBind** | ~1.2 B | MM (6 modalities) | ~1.2 GB | image/audio/text/depth/thermal/IMU joint space |

### 3.10 Temporal sidecars (A27 — already built, 0 VRAM, do **not** count toward the content floor)

`E2_recency` (1-d) · `E3_periodic` (2-d) · `E4_positional` (4-d) — algorithmic, weights-free.

---

## §4 — THE RECOMMENDATION: the default general panel ("Constellation-24")

Design rule: **one or two best-in-class members per family**, quantised, chosen so no two
are clones. Tiered so the operator scales to the card. Resident sizes are INT8 unless
noted FP16; **temporal sidecars excluded from the content count.**

### Tier 1 — CORE (always on; ~8 GB resident; runs on a 12–16 GB card too)

| # | Lens | Fam | ≈ VRAM | Why it's here (unique perspective) |
|---|---|---|---|---|
| 1 | EmbeddingGemma-300M | DS-gen | 0.30 | best small general semantic, multilingual+code |
| 2 | Qwen3-Embedding-0.6B (INT8) | DS-gen | 0.60 | LLM-grade signal, instruction-aware |
| 3 | nomic-embed-text-v1.5 | DS-gen | 0.14 | long-ctx, Matryoshka, distinct training |
| 4 | bge-base-en-v1.5 (INT8) | DS-gen | 0.11 | workhorse, different objective |
| 5 | **BGE-M3 (INT8)** | DS-gen+**LEX+LI** | 0.57 | **3 families in one** — dense+sparse+ColBERT |
| 6 | SPLADE-v3 | LEX | 0.11 | pure learned-sparse lexical perspective |
| 7 | answerai-colbert-small-v1 | LI | 0.03 | token late-interaction perspective |
| 8 | bge-reranker-v2-m3 (INT8, retrieval_only) | RERANK | 0.57 | asymmetric cross-encoder |
| 9 | sciNCL | DS-dom | 0.11 | scientific perspective |
| 10 | BioLORD-2023 | DS-dom | 0.11 | biomedical perspective |
| 11 | legal-bert / modernbert-legal | DS-dom | 0.11 | legal perspective |
| 12 | FinBERT | DS-dom | 0.11 | financial perspective |
| 13 | jina-embeddings-v2-base-code | DS-dom | 0.16 | code perspective |
| 14 | SigLIP2-base | IMG/MM | 0.40 (FP16) | zero-shot image-text |
| 15 | DINOv2-base | IMG | 0.17 | self-supervised fine-grained vision (sees differently than CLIP) |
| 16 | CLIP ViT-B/32 (INT8) | IMG/MM | 0.15 | fast broad zero-shot vision |
| 17 | LAION-CLAP (larger_general) | AUD/MM | 0.39 (FP16) | audio↔text semantic |
| 18 | wav2vec2-base | AUD | 0.19 (FP16) | speech content |
| 19 | WavLM-base-plus | AUD | 0.19 (FP16) | speaker / robust speech |
| 20 | MERT-v1-95M | AUD | 0.19 (FP16) | music |
| 21 | potion-base-8M (model2vec) | STATIC | **0** | free CPU lexical-semantic baseline |
| — | E2/E3/E4 temporal sidecars | (sidecar) | 0 | time-walk, not content |

**Tier-1 content lenses = 21, families spanned = DS-gen · DS-dom · LEX · LI · RERANK ·
IMG · AUD · STATIC · MM ≈ 9 families. Resident ≈ 5.3 GB FP16-equivalent.** Clears A35
(≥10), A36 (it is a named template), and A37 (≥2 families, low redundancy by design).

### Tier 2 — EXTENDED (add when serving science/multilingual/doc corpora; → ~12–14 GB)

| Lens | Fam | ≈ VRAM | Adds |
|---|---|---|---|
| multilingual-e5-base | DS-gen | 0.28 | non-English coverage |
| mxbai-embed-large-v1 | DS-gen | 0.34 | top English retrieval, 1024-d |
| MedCPT-Article-Encoder | DS-dom | 0.11 | clinical retrieval distinct from BioLORD |
| jina-colbert-v2 | LI | 0.56 | multilingual late-interaction |
| jina-clip-v2 (INT8) | MM | 0.87 | multilingual unified text+image |
| nomic-embed-vision-v1.5 | MM | 0.09 | image aligned to nomic-text shared space |
| ColSmol-256M | DOC/LI | 0.26 | visual PDF/table retrieval |
| ESM2-t30-150M | BIO | 0.30 | protein |
| DNABERT-2-117M | BIO | 0.12 | DNA |
| ChemBERTa-77M | BIO | 0.08 | molecule |
| PANNs CNN14 | AUD | 0.16 | environmental audio events |

**Tier-1+2 ≈ 32 content lenses, ~9 GB resident — still well under 20 GB.**

### Tier 3 — HEAVY (only when the job needs a giant; one at a time, budget-watch)

`Qwen3-Embedding-4B` (~4 GB) · `nomic-embed-code` / `gte-Qwen2-7B` (~7–8 GB) ·
`jina-embeddings-v4` 3.8 B (~3.8 GB) · `ColPali-v1.3` (~2–3 GB) · `GME-Qwen2-VL-2B` (~2 GB).
Adding **one** Tier-3 to Tier-1+2 stays ≤ ~14 GB; adding two approaches the 20 GB line.

> **Net:** ~30+ diverse, high-signal lenses across **every modality and domain** fit in
> **well under 20 GB**, leaving the rest of the 24 GB for activations + indexes. The
> budget is not the constraint — **diversity and measured bits are.** Spend the headroom
> on *more families*, not more dense-semantic clones.

---

## §5 — Quantisation & commissioning policy (how each lens gets small)

- **Default = ONNX + dynamic INT8** for every BERT/encoder-class text & domain lens
  (`onnx-int8` / `onnx-fastembed` runtimes already exist). ≈ 2× smaller than FP16, ≈ 4×
  smaller than FP32, retrieval quality delta typically < 1% — verify per-lens via the
  recall-delta contract (`recall_delta`, default 0.02).
- **FP16** for vision/audio encoders where INT8 conv/attention support is weaker (SigLIP,
  DINO, CLAP, wav2vec2, WavLM, MERT) — still 2× smaller than FP32.
- **Matryoshka truncation** (`truncate_dim`) for MRL models (EmbeddingGemma, Qwen3,
  nomic, stella, jina-clip-v2, arctic) — shrinks the *stored* slot and index, not just VRAM.
- **Static (model2vec)** for the zero-VRAM baseline lens — CPU, first-class preferred case.
- **TEI / FP8** for the few large GPU-resident encoders that justify a resident container
  (gte-base, modernbert-legal, reranker) on the Blackwell card.
- Every commissioned lens is a **SHA-256-verified `lensforge.manifest.json`** (frozen,
  content-addressed) and must carry a **measured `--cost-json`** (resident VRAM MB,
  ms/input) so the **resource-aware panel packer (#729) gates by `bits / VRAM-MB`.**

---

## §6 — Acceptance: how we *prove* this roster (not just assert it)

A roster/template is admitted only when an **EnsembleCard** over a real corpus shows:
1. **≥ 10 learned-encoder content lenses** (A35; `signal_kind = learned_encoder`, not
   placeholder/algorithmic — synthetic lenses are diagnostic-only, #808).
2. **A37 diversity gate = `gate_passed`**: ≥ 2 association families, `n_eff ≥ 0.6 ×
   content_count`, mean pairwise corr ≤ 0.6 **and** mean NMI ≤ 0.6, every content lens
   ≥ 0.05 marginal bits (non-collapsing unique-PID).
3. **Resource fit:** measured `Σ resident VRAM ≤ 20 GB`, panel packs under the fixed
   24 GB budget per the density packer; per-lens `bits / VRAM-MB` recorded.
4. **Fused-RRF readback** beats the best 1–2-lens control on the same corpus.

The current aiwonder **#803 GDELT-1M** run is the *homogeneous baseline/control* (mostly
DS-gen) that this roster is designed to beat — it measures the redundancy collapse A37
predicts and is the evidence that count ≠ diversity.

---

## §7 — Scheduling note (why ≤ 20 GB weights ≠ all-resident-at-once)

Lenses need not be co-resident. The ingest/measure path can **stream lenses one runtime
at a time** (already done for stream-fbin, commit `7c893c9`/#793), so peak VRAM = largest
single lens + its activation set, while *all* lens weights live on disk/host and page in.
The 20 GB budget is therefore a **comfort ceiling for the fully-resident serving panel**,
not a hard cap on roster size — a vault may carry far more commissioned lenses than are
resident at any instant (A5 hot-swap, A36 templates).

---

## §8 — Sources (web research, 2026-06-20)

- MTEB leaderboard / model survey — Modal, BentoML, Baseten, Milvus, Ailog guides.
- Qwen3-Embedding (0.6B/4B/8B) — QwenLM blog & GitHub.
- EmbeddingGemma-300M — Google / arXiv 2509.20354; HF model card.
- Granite-Embedding R2 — arXiv 2508.21085 / 2605.13521.
- Arctic-Embed — arXiv 2405.05374.
- Image: jina-clip-v2 (arXiv 2412.08802), Nomic Embed Vision (2406.18587), DINOv2/v3,
  SigLIP2, Jina vision-encoder survey; Voxel51 image-embedding benchmark.
- Audio: HuBERT/WavLM/wav2vec2/CLAP/MERT/PANNs — Zilliz audio-embeddings guide; MAEB
  (arXiv 2602.16008); LAION-CLAP.
- Domain text: BioBERT/PubMedBERT/ClinicalBERT/SciBERT/SPECTER2/sciNCL/BioLORD/MedCPT/
  legal-bert/FinBERT — arXiv 2507.19407 (medical embedding survey), 2006.08097 (FinBERT),
  2409.18511 (do-we-need-domain-models).
- Late-interaction / sparse / rerank: ColBERTv2, jina-ColBERT-v2 (arXiv 2408.16672),
  SPLADE, bge-reranker-v2-m3, jina-reranker-v3 (2509.25085).
- Code: nomic-embed-code, Qodo-Embed-1 (qodo.ai), jina-code-v2, Gemini-Embedding code.
- Bio: ESM-2 (8M–15B), ProtT5, DNABERT-2, Nucleotide-Transformer-v2, HyenaDNA, ChemBERTa,
  MolFormer.
- Document-image: ColPali (arXiv 2407.01449), ColQwen2, ColSmol.
- Quantisation: SBERT efficiency docs, ONNX-Runtime INT8 (Microsoft), Optimum dynamic
  INT8, TensorRT FP8.

> Param counts/dims are best-published-estimates; **the binding numbers are the ones
> Calyx measures** (resident VRAM via `--cost-json`, bits via Assay) on aiwonder.
