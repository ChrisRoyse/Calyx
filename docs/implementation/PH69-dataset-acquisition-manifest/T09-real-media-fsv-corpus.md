# PH69 T09 - Real Media FSV Mini-Corpus

| Field | Value |
|---|---|
| **Phase** | PH69 - Dataset acquisition + MANIFEST + checksum FSV |
| **Issue** | #756 |
| **Script** | `scripts/acquire_media_fsv.py` |
| **Dataset SoT** | `/home/croyse/calyx/data/datasets/media_fsv_mini` |
| **Evidence SoT** | `/home/croyse/calyx/data/fsv-issue756-media-fsv-20260618T0120Z` |

## Corpus

`media_fsv_mini` is the reusable small real-media corpus for multimodal FSV:

- audio: all 60 RAVDESS Actor_01 speech WAV files from Zenodo record `1188976`,
  license `CC-BY-NC-SA-4.0`, stable sorted filename selection.
- video: fixed Wikimedia Commons NASA titles `File:2005 TG45.ogv`,
  `File:2012 DA14.ogv`, and
  `File:2013 Daily Arctic Sea Ice from AMSR2 May - September 2013 01.webm`,
  licenses `Public domain` / `CC0`.

The canonical catalog row lives in
`/home/croyse/calyx/data/datasets/MANIFEST.md`; the per-dataset generated
manifest is `media_fsv_mini/manifest.json`. The acquisition manifest with source
URLs, license text/URLs, the subset rule, byte counts, and per-file SHA256 is
`media_fsv_mini/metadata/acquisition_manifest.json`.

## Commands

Run on aiwonder from `/home/croyse/calyx/repo`:

```bash
export CALYX_DATASET_ROOT=/home/croyse/calyx/data/datasets
python3 scripts/acquire_media_fsv.py acquire
python3 scripts/acquire_media_fsv.py validate

cargo run -p calyx-cli -- media emotion-validate \
  --samples /home/croyse/calyx/data/datasets/media_fsv_mini/metadata/audio_samples.jsonl \
  --metrics-dir /home/croyse/calyx/data/fsv-issue756-media-fsv-<stamp>/metrics \
  --vault /home/croyse/calyx/data/fsv-issue756-media-fsv-<stamp>/vault \
  --min-bits 0.05 \
  --k 3
```

## FSV Readback

The #756 FSV run proved:

- before: `media_fsv_mini` directory and MANIFEST row were absent.
- trigger: `python3 scripts/acquire_media_fsv.py acquire --force` on aiwonder.
- after: 60 WAV files, 3 video files, 63 media metadata rows, 67 cataloged files,
  dataset digest `fd0f9e4a87cd2c9103bf94a09bab3097779fe749a45f86b84a38cef89bebea7c`.
- decoded metadata: audio sample rate/channels/duration read by `ffprobe`;
  video frame count/fps/resolution/container read by `ffprobe -count_frames`.
- Calyx persisted state: `calyx media emotion-validate` wrote 2 Assay rows and
  1 Online metric row into the evidence vault; `calyx readback --cf assay` and
  `calyx readback --cf online` read back the physical SST bytes.

Edges captured under the evidence root:

- corrupt WAV bytes with matching manifest SHA -> `CALYX_MEDIA_FSV_DECODE_FAILED`.
- unsupported `.txt` in the video directory -> `CALYX_MEDIA_FSV_UNSUPPORTED_MEDIA_EXTENSION`.
- duplicate audio sample ID -> `CALYX_FSV_MEDIA_EMOTION_DUPLICATE_SAMPLE_ID`,
  with no emotion summary/bits metric emitted.

## Follow-Up

#758 tracks the missing raw audio/video ingest and video validation path. Until
that is implemented, this corpus can validate physical media acquisition and the
currently supported audio-emotion Calyx persistence path, but raw video rows,
vectors, provenance, and ledger entries are not yet wired.
