# T10 - Ward Speaker Identity-Lock + Speaker-MI FSV

**Issue:** #608
**Status:** Implemented and FSV-passed on aiwonder
**Evidence root:** `/home/croyse/calyx/data/fsv-issue608-voxceleb-identity-20260612T145550Z`

## Source Of Truth

- Dataset: `/zfs/archive/calyx/datasets/voxceleb1_mini_issue608`
- Dataset upstream: Hugging Face `s3prl/mini_voxceleb1`
- Selection: 50 real WAV files, five speakers, ten clips per speaker.
- Repaired dataset manifest: `SHA256SUMS.txt`
  - SHA256: `9139e3e0eab91edc02b3a45dc1293d4cd0542d73f661a69ee1c8947fd320988c`
- Selected-files SHA256: `47554fe0f4ccb208c4ded14339abfc9164c12984b1241d0a691de8827f59ffc2`
- WavLM model: `/home/croyse/calyx/models/wavlm/wavlm-base-plus-sv.onnx`
  - SHA256: `22a38bdd854a11db171357cb997156511697d2f2c621d1262c82ba91b873d08b`

During FSV, one truncated acquisition was found at
`test/id10977-iUUpvrP-gzQ-00025.wav` and reacquired from the upstream dataset.
The repaired file is 248,366 bytes with SHA256
`809ee02dafd25cf37a452c467e0d9e2701952a6c9deab7139e8f2938963fc1d6`.
The independent Python WAV readback after repair found 50 WAV files, zero bad
metadata rows, zero truncated chunks, frame range 65,281 to 288,641, and speaker
counts `{id10977: 10, id11083: 10, id11115: 10, id11132: 10, id11160: 10}`.

## Implementation

The ignored aiwonder FSV test lives under
`crates/calyx-ward/tests/identity_fsv/voxceleb_identity/` and is surfaced by
`crates/calyx-ward/tests/identity_fsv.rs`.

The FSV flow:

1. Load real VoxCeleb PCM16 mono WAV bytes from the selected manifest.
2. Embed each clip through the WavLM speaker lens using CPU-explicit provider policy.
3. Calibrate the Ward speaker-slot tau between genuine and impostor pair scores.
4. Run Ward `guard()` for genuine and impostor pairs with `NoveltyAction::RejectClosed`.
5. Compute KSG speaker-MI for `speaker_id` against the speaker slot.
6. Write fixture, embedding, guard verdict, MI, edge-case, and checksum readbacks.

## Readback Results

- Artifact kind: `ph70.ward-voxceleb-speaker-identity.v1`
- Evidence artifact SHA256: `f1e3d653aa37a82dafeb40390b2b55331b79c8bd6b94a0bac00bb379f8193dd0`
- Calibration tau: `0.8688719868659973`
- Min genuine cosine: `0.9010542035102844`
- Max impostor cosine: `0.8366897702217102`
- Guard pairs: 90 total
- Genuine pass count: 45 / 45
- Impostor fail count: 45 / 45
- Speaker-MI: `2.3807899951934814` bits
- Speaker-MI threshold: `0.05000000074505806` bits

## Edge Cases

The same FSV artifact records fail-closed before/after readbacks for:

- Empty dataset: `CALYX_WARD_VOXCELEB_EMPTY_DATASET`
- Single speaker: `CALYX_WARD_VOXCELEB_NEEDS_IMPOSTOR`
- Bad WAV bytes: `CALYX_WARD_VOXCELEB_BAD_WAV`
- Tau-overlap synthetic embeddings: `CALYX_WARD_VOXCELEB_TAU_OVERLAP`

All four edge paths left their synthetic artifact targets absent before and after,
which proves fail-closed behavior rather than partial writes.

## Aiwonder Gates

- `cargo test -p calyx-ward --test identity_fsv voxceleb_identity::issue608_voxceleb_edges_fail_closed_with_codes -- --exact`
- `CALYX_ISSUE608_FSV_ROOT=/home/croyse/calyx/data/fsv-issue608-voxceleb-identity-20260612T145550Z CALYX_ISSUE608_VOXCELEB_ROOT=/zfs/archive/calyx/datasets/voxceleb1_mini_issue608 CALYX_WARD_SPEAKER_PROVIDER=cpu cargo test -p calyx-ward --test identity_fsv voxceleb_identity::issue608_voxceleb_speaker_identity_fsv_writes_readbacks -- --ignored --nocapture`

Final whole-crate gates are recorded in the GitHub issue closure comment.
