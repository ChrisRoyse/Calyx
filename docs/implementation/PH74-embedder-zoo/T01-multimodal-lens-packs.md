# PH74 T01 - Multimodal Lens Packs

Issue: #788
Stage: S21 Embedder Zoo and Lens Conversion Factory
Crate: `calyx-registry`

## Goal

Bring first-class non-text lens surfaces online so panels can carry cross-modal
slots and capability cards for those slots. Image and audio are live real-model
priority axes; protein, DNA, and molecule remain declared modalities until their
own real runtimes are commissioned.

## Implemented Surface

- Core `Modality` includes `protein`, `dna`, and `molecule`; existing tags for
  text/code/image/audio/video/structured/mixed remain stable.
- `LensRuntime::MultimodalAdapter` persists adapter runtime metadata:
  `axis`, `model_id`, `adapter_config`, and the real artifact file set.
- `MultimodalAdapterLens` is a frozen registry lens. It validates the input
  modality and byte syntax, then runs a strict ONNXRuntime helper against the
  configured local model artifact. There is no byte-hash fallback.
- `tools/lensforge/convert.py` supports `adapter` for real priority media
  models and emits `runtime: multimodal-adapter` manifests containing ONNX
  tower artifacts, processor/config files, the helper script, and `adapter.json`.
- LensForge real adapter registry entries cover:
  - `image-siglip2-b16-adapter` (SigLIP2 vision, 768D)
  - `audio-clap-htsat-adapter` (LAION-CLAP audio, 512D)
- Protein/DNA/molecule adapter entries are skipped with an explicit
  `real_multimodal_adapter_not_configured` reason until real model runtimes are
  wired.
- LensForge manifest registration refuses non-commercial manifests with
  `CALYX_LICENSE_DENIED` unless `CALYX_ALLOW_NONCOMMERCIAL_LENSES=true`.
- Custom ONNX specs preserve declared modality so later protein/DNA/molecule
  ONNX manifests are not forced through `text`.

## Adapter Input Contracts

- Image accepts PNG or JPEG bytes.
- Audio accepts RIFF/WAVE bytes.
- Protein accepts amino-acid sequence letters:
  `ACDEFGHIKLMNPQRSTVWY`.
- DNA accepts `ACGTN`.
- Molecule accepts a strict ASCII SMILES token subset.

Malformed input returns `CALYX_LENS_DIM_MISMATCH`; no adapter path panics.

## Manual FSV Recipe

Run on aiwonder from `/home/croyse/calyx/repo`.

1. Generate adapter manifests with LensForge into an isolated
   `CALYX_HOME`.
2. Add the image/audio manifests with `target/release/calyx lens add --manifest`.
3. Read `$CALYX_HOME/lenses/registry.json` and confirm image/audio catalog rows
   with real artifact paths.
4. Register the lenses through `Registry::register_frozen_with_spec`.
5. Measure known image/audio bytes through `Registry::measure`.
6. Independently read the persisted JSON evidence and confirm image vectors are
   768D, audio vectors are 512D, all values are finite, and norm is
   approximately 1.0.
7. Compare helper ONNX outputs to upstream Transformers reference outputs at a
   cosine threshold recorded in the issue.
8. Manually exercise edge cases: missing model file, corrupt image bytes,
   corrupt WAV bytes, and a CC-BY-NC-SA manifest denied by default.

## Done Evidence

Fill in the issue comment with:

- aiwonder commit hash and branch.
- test and gate commands.
- LensForge manifest paths and ONNX artifact hashes.
- `calyx lens list` readback.
- registry snapshot/readback path.
- image/audio measurement norms and upstream-reference cosines.
- license-deny readback.
- malformed-input readbacks.
