# Calyx Embedder (Lens) Roster — Full Report

> **Generated:** 2026-06-22
> **Source of truth:** live `/home/croyse/calyx/lenses/registry.json` + `panels/templates/` on aiwonder (read directly, not reconstructed from issues).
> **Doctrine refs:** `docs/dbprdplans/05_EMBEDDER_REGISTRY.md`, `docs/dbprdplans/05a_EMBEDDER_ROSTER_VRAM_BUDGET.md`, epic #814 (A38), #796/#787/#802 (A35/A36/A37 mandate).

---

## Headline numbers

| Metric | Value |
|---|---|
| **Total commissioned lenses (live registry)** | **42** |
| GPU-placed / CPU-placed | 24 / 18 |
| Sum of resident GPU VRAM | ~13.5 GB (well under the 20 GB Constellation budget) |
| **Admitted default panel** (`constellation-24`, A37 `gate_passed`) | **10 content lenses + 3 temporal sidecars**, 4 association families |
| Registered panel templates | 8 |
| Roster epic **#814** / acquisition backlog **#836** | **CLOSED** (the A38 sprint landed) |

## Verification readback

Verified against aiwonder source-of-truth bytes on 2026-06-22:

| Source | Evidence |
|---|---|
| `/home/croyse/calyx/lenses/registry.json` | 27,810 bytes, sha256 `c7351bbac1bf44bdffc27628507dbf689a9449e7db690e4fc8416e0367e61e3f`; 42 lenses; placement 24 GPU / 18 CPU; GPU `cost.vram_bytes` sum 13,488,986,564 bytes (~12,864.1 MiB / 13,489.0 MB) |
| `/home/croyse/calyx/panels/templates/index.json` | 10,335 bytes, sha256 `81361c10dfbc49c0539199122beb6660b6c7ffb9cfc4f87ee6ea81e6001db252`; 8 registered templates; `constellation-24` active template `15d0fd0a648da6686c72471d103b5fbc20e26a44e87c0c204c45a8df69af6149` |
| `/home/croyse/calyx/fsv/issue832-bgem3-colbert-roster-20260621T141356Z/issue822_final_admission_readback.json` | 13,990 bytes, sha256 `6dd4361e91bb01d5d7ca7fe351ea78ffa5a65e7d000a9fc66d5fa21a054a863c`; A37 gate status `gate_passed` |

**Key distinction:** *42 lenses are commissioned and available in the registry*, but only *10* are in the gate-passed diverse **default** panel (`constellation-24`); the remaining 32 are available lenses (image / audio / bio / extra text) that vaults and other templates draw on.

A lens earns a slot by **measured `bits / VRAM-MB`** and **associational diversity (A37)**, not raw benchmark rank. Ten dense-semantic clones fail the gate; admission requires ≥10 learned-encoder content lenses spanning ≥2 families, `n_eff ≥ 0.6×count`, mean corr/NMI ≤ 0.6, every lens ≥ 0.05 marginal bits, Σ resident VRAM ≤ 20 GB, and fused-RRF beating the best 1–2-lens control.

---

## 1. ALL 42 embedders currently commissioned (by modality)

### Text — general semantic (DS-gen)

| Lens | Runtime | Dim | VRAM MB |
|---|---|---|---|
| a38_embeddinggemma_300m | onnx | 768 | 1256 |
| a38_qwen3_embedding_06b | onnx | 1024 | 616 |
| semantic-nomic-embed-text-v1-5-fastembed | onnx | 768 | 548 |
| a38_bge_base_en_v15 | onnx | 768 | 437 |
| semantic-bge-small-en-v1-5 | onnx | 384 | cpu/0 |
| semantic-multilingual-e5-base-fastembed | onnx | 768 | 1127 |
| semantic-mxbai-embed-large-v1-onnx-cls | onnx | 1024 | 1338 |
| semantic-jina-v2-base-en-fastembed | onnx | 768 | 548 |
| semantic-gte-base-tei | tei_http | 768 | resident TEI |
| semantic-all-minilm-l6-v2-onnx | onnx | 384 | cpu/0 |
| semantic-all-minilm-l6-v2-candle | candle_local | 384 | cpu/0 |
| semantic-clip-vit-b32-text-fastembed | onnx | 512 | 256 |
| semantic-potion-base-8m | static_lookup | 256 | **0 (static)** |

### Text — domain (DS-dom)

| Lens | Domain | Runtime | VRAM MB |
|---|---|---|---|
| domain-scincl-onnx-fp32 | scientific | onnx | 438 |
| domain-scibert-scivocab-uncased | scientific | onnx | cpu/0 |
| a38_biolord_2023_m_int8 | biomedical | onnx | 295 |
| a38_pubmedbert_int8 | biomedical / clinical | onnx | 111 |
| a38_medcpt_query_int8 | clinical retrieval (query) | onnx | 110 |
| a38_medcpt_article_int8 | clinical retrieval (article) | onnx | 110 |
| domain-modernbert-legal-tei | legal | tei_http | resident TEI |
| a38_finbert_int8 | finance | onnx | 112 |
| a38_jina_v2_base_code | code | onnx | 644 |

> BioLORD and MedCPT (query + article) are now **admitted** (the #826 decision path resolved them under local-use policy).

### Text — lexical/sparse + late-interaction (LEX / LI)

| Lens | Family | Runtime | Dim | VRAM MB |
|---|---|---|---|---|
| a38_bgem3_dense | DS-gen (BGE-M3) | fastembed_bgem3 | 1024 | 587 |
| a38_bgem3_sparse | LEX (BGE-M3) | fastembed_bgem3 | 250002 | 587 |
| a38_bgem3_colbert | LI (BGE-M3) | fastembed_bgem3 | multi/1024 | 587 |
| a38_spladepp_v1_sparse | LEX | fastembed_sparse | 30522 | 533 |
| a38_answerai_colbert_small_v1 | LI | onnx_colbert | 384 | 35 |
| text-jina-colbert-v2-onnx | LI (multilingual) | onnx_colbert | 128 | 2255 |

> BGE-M3 supplies three families from one model (dense + sparse + ColBERT). SPLADE++ (`Qdrant/Splade_PP_en_v1`) is the admitted stand-in for exact `naver/splade-v3` (no ONNX sibling + NC license).

### Image (IMG / MM / DOC)

| Lens | Kind | Dim | VRAM MB |
|---|---|---|---|
| image-siglip2-b16-adapter | zero-shot CLIP-style | 768 | cpu/0 |
| image-clip-vit-b32-adapter | classic zero-shot | 512 | cpu/0 |
| image-dinov2-base-adapter | self-supervised (sees differently) | 768 | cpu/0 |
| image-jina-clip-v2-adapter | multilingual unified text+image | 1024 | cpu/0 |
| image-nomic-embed-vision-v1-5-adapter | aligned to nomic-text space | 768 | cpu/0 |
| image-colsmol-256m-doc-adapter | document-image (PDF/table) | 128 | 958 |

### Audio (AUD)

| Lens | Kind | Dim |
|---|---|---|
| audio-clap-htsat-adapter | audio↔text semantic (LAION-CLAP) | 512 |
| audio-wav2vec2-base-960h | speech content | 768 |
| audio-wavlm-base-plus-adapter | speaker / robust speech | 512 |
| audio-mert-v1-95m-adapter | music | 768 |
| audio-panns-cnn14-environmental-adapter | environmental / event audio | 527 |

### Science / BIO

| Lens | Modality | Dim |
|---|---|---|
| protein-esm2-t30-150m-adapter | protein | 640 |
| dna-moderngena-base-adapter | DNA | 768 |
| molecule-chemberta-100m-adapter | molecule (SMILES) | 768 |

### Modality / runtime / placement breakdown

- **By modality:** text 28 · image 6 · audio 5 · protein 1 · dna 1 · molecule 1
- **By runtime:** onnx 18 · multimodal_adapter 14 · fastembed_bgem3 3 · onnx_colbert 2 · tei_http 2 · fastembed_sparse 1 · static_lookup 1 · candle_local 1
- **By placement:** gpu 24 · cpu 18
- **Sum resident GPU VRAM:** ~13,489 MB (~13.5 GB)

---

## 2. The admitted default panel — `constellation-24` (A37 `gate_passed`)

Object `15d0fd0a648da6686c72471d103b5fbc20e26a44e87c0c204c45a8df69af6149`, v2. The proven diverse default: 10 of the 42 commissioned lenses, gated on measured diversity — **4 association families**, family span ✓, redundancy bound ✓, no-collapse ✓.

| # | Slot | Family |
|---|---|---|
| 1 | a38_embeddinggemma_300m | DS-gen |
| 2 | a38_qwen3_embedding_06b | DS-gen |
| 3 | semantic-multilingual-e5-base-fastembed | DS-gen (multilingual) |
| 4 | domain-scincl-onnx-fp32 | DS-dom (scientific) |
| 5 | a38_pubmedbert_int8 | DS-dom (biomedical) |
| 6 | domain-modernbert-legal-tei | DS-dom (legal) |
| 7 | a38_finbert_int8 | DS-dom (finance) |
| 8 | a38_jina_v2_base_code | DS-dom (code) |
| 9 | a38_spladepp_v1_sparse | LEX (learned sparse) |
| 10 | a38_bgem3_colbert | LI (late-interaction) |
| — | E2 recency · E3 periodic · E4 positional | temporal sidecars (NOT counted toward the ≥10 floor) |

**Admission evidence:** EnsembleCard with A37 multi-anchor `status=gate_passed` (`family_span=true; redundancy_bound=true; no_collapse=true`; association_family_count=4); RRF fused recall beats best-single and best-two controls; Σ resident VRAM ≤ 20 GB (#822 final readback `/home/croyse/calyx/fsv/issue832-bgem3-colbert-roster-20260621T141356Z/issue822_final_admission_readback.json`, sha256 `6dd4361e91bb01d5d7ca7fe351ea78ffa5a65e7d000a9fc66d5fa21a054a863c`).

---

## 3. Other registered panel templates (8 total)

| Template | Content lenses | A37 status |
|---|---|---|
| **constellation-24** | 10 | ✅ `gate_passed` |
| code-oracle | 10 | missing ensemble card |
| text-deep | 10 | missing ensemble card |
| literary-essence | 10 | missing ensemble card |
| video-capture | 13 | missing ensemble card |
| issue798-manual-text | 10 | not gated |
| issue798-manual-text-fork | 10 | not gated |
| issue798-manual-text-fork-final | 10 | not gated |

Only `constellation-24` is currently A37-gate-eligible; the others are registered scaffolds awaiting their own EnsembleCards.

---

## 4. Planned / future

The A38 roster epic **#814** and the Tier-2/3 acquisition backlog **#836** are now **CLOSED** — the bulk of planned commissioning landed (image multi-family, audio four-family, protein/DNA/molecule, document-image, MedCPT/BioLORD all admitted since the prior snapshot). Remaining forward work falls into three buckets.

### A. Tier-3 "heavy" candidates
Catalogued in `05a §3`, admitted one-at-a-time under budget-watch, not yet commissioned:
- `Qwen3-Embedding-4B` (~4 GB)
- `nomic-embed-code` / `gte-Qwen2-7B` / `e5-mistral-7b` (~7–8 GB)
- `jina-embeddings-v4` 3.8B (unified multimodal)
- `ColPali-v1.3` / `ColQwen2` (full document-image late-interaction)
- `GME-Qwen2-VL-2B` / `VLM2Vec` (VLM-derived universal embedder)

### B. Catalogue candidates still on the "try" list (not yet in registry)
- **Text general:** `gte-modernbert-base`, `snowflake-arctic-embed-m-v2.0`, `granite-embedding-278m`, `stella_en_400M_v5`, `jina-embeddings-v3`, `LaBSE`
- **Code:** `Qodo-Embed-1-1.5B`, `CodeRankEmbed`
- **Sparse/LI:** `opensearch-neural-sparse-v2`
- **Reranker family** (only bge-reranker catalogued so far): `jina-reranker-v2-base-multilingual`, `mxbai-rerank-base-v2`, `Qwen3-Reranker-0.6B`
- **Image:** `SigLIP2-so400m`, `DINOv3-ViT-H+/16`, domain CLIPs (FashionCLIP, medical-CLIP, RemoteCLIP)
- **Bio:** `ESM-C-300M` / `ProtT5-XL` (protein); `DNABERT-2` (blocked on the current aiwonder runtime stack — ModernGENA is the admitted stand-in); `Nucleotide-Transformer-v2`; `MolFormer-XL` (molecule)
- **Unified MM:** `ImageBind` (6-modality joint space)

### C. Open-ended self-extension (binding A30/A31/A38 doctrine)
The roster is a **living set**, not a fixed list. Any uncovered modality/domain is a coverage gap → `propose_lens` (#725) → measure bits → A35/A37/A38 gate → hot-add (A5), all under the fits-in-24 GB / max `bits/VRAM-MB` budget. Standing-policy epics remain open:
- **#796** — 10+ embedder panels, templates & ensemble signal measurement (A35/A36)
- **#802** — associational diversity gate (A37): count to 10 is necessary, not sufficient
- **#787** — multi-embedder testing mandate: floor ≥10 lenses; value is associational; <10 fails closed

---

## Caveats

1. The 42-count is the live registry on aiwonder as of 2026-06-22; it drifts as lenses are commissioned/retired.
2. "VRAM MB" is per-lens resident weight cost; CPU-placed adapters (most image/audio/bio) report 0 GPU VRAM.
3. Templates other than `constellation-24` are registered but not yet A37-proven.
4. aiwonder is the runtime source-of-truth; the Windows checkout is authoring-only.
