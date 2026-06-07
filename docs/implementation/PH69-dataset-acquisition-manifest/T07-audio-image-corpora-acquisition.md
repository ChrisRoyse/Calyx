# PH69 · T07 — Audio/image corpora acquisition (VoxCeleb / LibriSpeech / RAVDESS / IEMOCAP / ImageNet-subset / CIFAR-100 / COCO)

| Field | Value |
|---|---|
| **Phase** | PH69 — Dataset acquisition + MANIFEST + checksum FSV |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `—` (scripts/infra) |
| **Files** | `scripts/acquire_audio.sh` (≤500), `scripts/acquire_image.sh` (≤500) |
| **Depends on** | T01 (MANIFEST schema + verify tooling) |
| **Axioms** | A2, A34 |
| **PRD** | `28 §3` rows 6, 7, 8; `28 §3.2` |

## Goal

Acquire the audio-speaker, audio-emotion, and image corpora to
`/zfs/archive/calyx/datasets/<name>/`, checksum-verify each, and write MANIFEST
rows. VoxCeleb1/2 + LibriSpeech provide speaker-identity ground truth for Ward
identity-lock FSV; RAVDESS/IEMOCAP provide emotion labels for media-panel
emotion lens FSV; ImageNet-subset/CIFAR-100/COCO provide image class/caption
labels for cross-modal lens FSV (PRD `28 §3` rows 6–8, `28 §2`).

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/acquire_audio.sh`:
      VoxCeleb1 verification split — HF `ProgramComputer/voxceleb1` or academic
      mirror to `/zfs/archive/calyx/datasets/voxceleb1/`; use the test/verification
      split (37 720 pairs), not the full 1 251-speaker training set, to stay within
      disk budget.
      VoxCeleb2 — verification split only to `/zfs/archive/calyx/datasets/voxceleb2/`
      if license accepted on HF; skip gracefully if not (log warning, no MANIFEST row).
      LibriSpeech — `test-clean` split (2620 utterances) from
      `openslr.org/12/` or HF `openslr/librispeech_asr` to
      `/zfs/archive/calyx/datasets/librispeech/`.
      RAVDESS — HF `narad/RAVDESS` to
      `/zfs/archive/calyx/datasets/ravdess/`; 7356 clips, 8 emotion labels.
      IEMOCAP — gated; if `hf_hub_token` grants access: HF mirror to
      `/zfs/archive/calyx/datasets/iemocap/`; else skip gracefully.
      Call `verify_dataset.sh <name>` after each; fail-closed on mismatch.
- [ ] `scripts/acquire_image.sh`:
      ImageNet-subset — ILSVRC 2012 val subset (first 5000 images, 1000 classes)
      or HF `imagenet-1k` val split to `/zfs/archive/calyx/datasets/imagenet_subset/`.
      CIFAR-100 — HF `uoft-cs/cifar100` to
      `/zfs/archive/calyx/datasets/cifar100/`; 60 000 images, 100 classes.
      COCO — val2017 (5000 images + captions) from COCO API or HF to
      `/zfs/archive/calyx/datasets/coco/`; ~1 GB.
      Call verify after each; fail-closed.
- [ ] MANIFEST rows for each successfully acquired dataset, noting license, row count
      (file/clip/image count as appropriate), and `what_it_tests`.
- [ ] Record in the MANIFEST `version` field which split was used and why (disk budget).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: create a synthetic 3-clip metadata CSV (fixed speaker IDs, emotion labels,
      known sha256); assert row count = 3 and label set contains at least 2 classes.
- [ ] proptest: property that verify round-trips — sha256 of acquired metadata file
      equals value in MANIFEST.
- [ ] edge (≥3):
      (1) VoxCeleb gating not accepted → script skips gracefully (exit 0, no MANIFEST
          row, logs `CALYX_DATASET_GATED_SKIP: voxceleb2`);
      (2) partial audio download → sha256 mismatch → `CALYX_DATASET_CHECKSUM_MISMATCH`;
      (3) CIFAR-100 row count ≠ 60 000 → `CALYX_DATASET_ROWCOUNT_MISMATCH`.
- [ ] fail-closed: missing `HF_HUB_TOKEN` for gated datasets → exits 1,
      `CALYX_SECRET_MISSING: HF_HUB_TOKEN`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/archive/calyx/datasets/voxceleb1/`, `librispeech/`, `ravdess/`,
  `imagenet_subset/`, `cifar100/`, `coco/` (and optionally `voxceleb2/`,
  `iemocap/`) on aiwonder; MANIFEST rows.
- **Readback:**
  ```
  bash scripts/verify_dataset.sh voxceleb1
  bash scripts/verify_dataset.sh librispeech
  bash scripts/verify_dataset.sh ravdess
  bash scripts/verify_dataset.sh cifar100
  bash scripts/verify_dataset.sh coco
  cat $CALYX_HOME/datasets/MANIFEST.md | grep -E 'voxceleb|librispeech|ravdess|iemocap|imagenet|cifar|coco'
  du -sh /zfs/archive/calyx/datasets/cifar100/
  ```
- **Prove:** before: directories absent; after: verify exits 0 for ≥4 of the 7
  (VoxCeleb2 and IEMOCAP may be gated-skip); MANIFEST rows populated; live sha256
  matches stored value for each present dataset.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH69 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
