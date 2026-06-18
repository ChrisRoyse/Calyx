# PH69 — Dataset acquisition + MANIFEST + checksum FSV

**Stage:** S18 — Datasets & Intelligence FSV  ·  **Crate:** `—` (infra/scripts)  ·
**PRD roadmap:** `28 §3`  ·  **Axioms:** A2, A34

## Objective

Gather the full real-dataset catalog so every lens family and every intelligence
metric has a grounded, verified test corpus on aiwonder. Each dataset is acquired to
`/zfs/archive/calyx/datasets/<name>/`, checksum-verified on arrival
(rows/bytes/sha256), and recorded as a row in `datasets/MANIFEST.md`. Acquisition
is itself FSV'd: record expected → download → read back → assert == expected.
`BUILD_DONE` clause DATA requires ≥1 verified dataset per (modality × outcome-type).

## Dependencies

- **Phases:** PH00 (CALYX_HOME exists on aiwonder; `hf_hub_token` in env; ZFS
  archive pool mounted at `/zfs/archive/calyx/datasets`; Infisical wired)
- **Note:** PH69 can run early — start once PH00 exists, independent of all
  engine phases.
- **Provides for:** PH70 (all intelligence FSV runs consume this catalog); PH24
  (Sextant qrels), PH30 (Assay labeled classes), PH33 (Lodestar corpora), PH38
  (Ward injection set), PH48 (Anneal J corpus), PH49 (Oracle SWE-bench).

## Current state (build off what exists)

No datasets directory exists yet; PH00 ZFS provisioning is expected to have
created `/zfs/archive/calyx/datasets/`. PH69 is greenfield. The `hf_hub_token`
secret is already in Infisical as `HF_HUB_TOKEN`/`HF_TOKEN`. Kaggle creds are
added to Infisical only if a Kaggle dataset is actually used (A34 — free sources).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `datasets/MANIFEST.md` | Catalog row per dataset: name, source, version, sha256, rows, license, what-it-tests |
| `scripts/acquire_datasets.sh` | Orchestration script: set `HF_HUB_TOKEN`, call per-dataset fetch + verify |
| `scripts/verify_dataset.sh` | Reusable checksum/row-count readback; exits non-zero on mismatch |
| `scripts/acquire_retrieval.sh` | Fetch BEIR/MS MARCO/Natural Questions/TREC-COVID (≤500 lines) |
| `scripts/acquire_classification.sh` | Fetch AG News/IMDB/SST-2/GLUE/banking77/DBpedia-14 (≤500 lines) |
| `scripts/acquire_code_oracle.sh` | Fetch SWE-bench Lite/HumanEval/MBPP (≤500 lines) |
| `scripts/acquire_graph_kernel.sh` | Fetch WordNet/ConceptNet/Wiktionary/Cora/ogbn (≤500 lines) |
| `scripts/acquire_dedup.sh` | Fetch Quora Question Pairs/PAWS (≤500 lines) |
| `scripts/acquire_audio.sh` | Fetch VoxCeleb1/2/LibriSpeech/RAVDESS/IEMOCAP (≤500 lines) |
| `scripts/acquire_image.sh` | Fetch ImageNet-subset/CIFAR-100/COCO (≤500 lines) |
| `scripts/acquire_media_fsv.py` | Fetch the small real audio/video FSV corpus under `$CALYX_HOME/data/datasets` |
| `scripts/acquire_temporal_adversarial.sh` | Fetch event logs/financial tick/prompt-injection/jailbreak/OOD/personas/drift (≤500 lines) |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | MANIFEST schema + verify_dataset tooling | — |
| T02 | Retrieval corpora acquisition (BEIR/MS MARCO/NQ/TREC-COVID) | T01 |
| T03 | Classification corpora acquisition (AG News/IMDB/SST-2/banking77/DBpedia-14) | T01 |
| T04 | Code oracle acquisition (SWE-bench Lite/HumanEval/MBPP) | T01 |
| T05 | Graph/kernel corpus acquisition (WordNet/ConceptNet/Wiktionary/Cora/ogbn) | T01 |
| T06 | Dedup corpora acquisition (QQP/PAWS) | T01 |
| T07 | Audio/image corpora acquisition (VoxCeleb/LibriSpeech/RAVDESS/IEMOCAP/ImageNet/CIFAR-100/COCO) | T01 |
| T08 | Temporal/adversarial/persona/drift acquisition + coverage gate | T02, T03, T04, T05, T06, T07 |
| T09 | Real media FSV mini-corpus (RAVDESS audio + Commons NASA video) | T01 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Per PRD `28 §3.2`: for each dataset in the catalog —
1. `sha256sum /zfs/archive/calyx/datasets/<name>/**` matches the value recorded in `datasets/MANIFEST.md`;
2. row count from `wc -l` / `python -c "import datasets; print(len(ds))"` matches MANIFEST `rows` field;
3. `datasets/MANIFEST.md` contains ≥1 row per (modality × outcome-type) covering: text-semantic/qrels, text/class-label, code/test-pass-fail, graph/community, text/duplicate-label, audio-speaker/identity, audio/emotion-label, image/class-caption, temporal/recurrence, adversarial-text/injection-benign, civic/tie-formation, text/distribution-shift;
4. `scripts/verify_dataset.sh <name>` exits 0 on aiwonder for every entry.

Readback command (run on aiwonder):
```
bash scripts/verify_dataset.sh ALL   # calls verify for each MANIFEST row
cat datasets/MANIFEST.md             # human-review rows
```
Evidence (screenshot of terminal showing all-green verify output) attached to the PH69 GitHub issue.

## Risks / landmines

- **ZFS EXDEV:** do not rename across mount boundaries; acquire directly to
  `/zfs/archive/calyx/datasets/<name>/` — never to `/tmp` then rename.
- **HF gated datasets:** some (IEMOCAP, VoxCeleb) require license acceptance on
  the HuggingFace website before `hf_hub_token` grants download; gate on that
  manual step before the script runs.
- **Disk quota:** full VoxCeleb2 + LibriSpeech + COCO are large (100s GB total);
  prefer subsets/splits until the full run is needed; record which split in MANIFEST.
- **sha256 drift:** datasets on HF occasionally update silently — pin the exact
  `revision`/`commit` hash in the HF `load_dataset` call and record it in MANIFEST.
- **Kaggle auth:** only add Kaggle creds to Infisical if a Kaggle source is
  actually used — keep the secret surface minimal (A34).
- **Free sources only (A34):** every dataset must be freely accessible or
  freely licensed; no paid API calls.
