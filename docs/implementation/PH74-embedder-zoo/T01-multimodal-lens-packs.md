# PH74 T01 - Multimodal Lens Packs

Issue: #788
Stage: S21 Embedder Zoo and Lens Conversion Factory
Crate: `calyx-registry`

## Goal

Bring first-class non-text lens surfaces online for image, audio, protein, DNA,
and molecule inputs so panels can carry cross-modal slots and capability cards
for those slots.

## Implemented Surface

- Core `Modality` includes `protein`, `dna`, and `molecule`; existing tags for
  text/code/image/audio/video/structured/mixed remain stable.
- `LensRuntime::MultimodalAdapter` persists adapter runtime metadata:
  `axis` plus `model_id`.
- `MultimodalAdapterLens` is a frozen registry lens. It validates the input
  modality and byte syntax, projects accepted bytes deterministically, emits a
  finite dense unit vector, and lets the existing frozen contract verify shape
  and norm.
- `tools/lensforge/convert.py` supports `adapter` and emits
  `runtime: multimodal-adapter` manifests without network access.
- LensForge registry entries cover:
  - `image-siglip2-b16-adapter`
  - `audio-clap-htsat-adapter`
  - `protein-esm2-t6-8m-adapter`
  - `dna-dnabert2-117m-adapter`
  - `molecule-chemberta-zinc-adapter`
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

1. Generate PH74 adapter manifests with LensForge into an isolated
   `CALYX_HOME`.
2. Add the five manifests with `target/release/calyx lens add --manifest`.
3. Read `$CALYX_HOME/lenses/registry.json` and confirm five catalog rows with
   modalities image, audio, protein, dna, and molecule.
4. Register the five lenses through `Registry::register_frozen_with_spec`.
5. Measure one accepted input per modality through `Registry::measure`.
6. Independently read the persisted JSON evidence and confirm each vector has
   `dim = 16`, finite values, and norm approximately 1.0.
7. Produce one `CapabilityCard` per modality with `profile_lens` and read back
   the card JSON files from disk.
8. Manually exercise edge cases:
   corrupt image bytes, corrupt WAV bytes, invalid protein symbol, invalid DNA
   symbol, invalid SMILES token, and a CC-BY-NC-SA manifest denied by default.

## Done Evidence

Fill in the issue comment with:

- aiwonder commit hash and branch.
- test and gate commands.
- LensForge manifest paths.
- `calyx lens list` readback.
- registry snapshot/readback path.
- five measurement norms.
- five capability-card paths.
- license-deny readback.
- malformed-input readbacks.
