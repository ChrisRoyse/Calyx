# 17 - Discovery vault molecular extension

- **Issue:** #884   **Phase:** P0 discovery / Phase 5 prep   **Date (UTC):** 2026-06-25   **Vault/panel:** not commissioned yet; data preflight only
- **Goal:** Build toward a molecular discovery vault with protein, molecule, DNA, and text views anchored on ChEMBL/BindingDB/Open Targets evidence.

## What was run (exact commands)

aiwonder source tree: `/home/croyse/calyx/repo`
Discovery data root: `/zfs/archive/calyx/biomed-rx/discovery`
FSV root: `/home/croyse/calyx/fsv/issue884-molecular-preflight-final-20260625T104428Z`

```bash
# Read current local discovery files, hashes, Open Targets parquet counts, and row metadata.
/zfs/archive/calyx/datasets/.dataset_tools_venv/bin/python3 - <<'PY'
# bounded Python readback over /zfs/archive/calyx/biomed-rx/discovery;
# wrote summary.json and printed only scalar counts, bytes, and hashes.
PY

# Repair missed single-file Open Targets 26.03 datasets.
wget -c -q --tries=3 --timeout=120 \
  -P /zfs/archive/calyx/biomed-rx/discovery/opentargets/disease \
  https://ftp.ebi.ac.uk/pub/databases/opentargets/platform/26.03/output/disease/disease.parquet
wget -c -q --tries=3 --timeout=120 \
  -P /zfs/archive/calyx/biomed-rx/discovery/opentargets/clinical_indication \
  https://ftp.ebi.ac.uk/pub/databases/opentargets/platform/26.03/output/clinical_indication/clinical_indication.parquet

# Patch the aiwonder operational downloader regex:
# /zfs/archive/calyx/biomed-rx/scripts/fix_ot2.sh now extracts any href="*.parquet",
# not only part-*-c000.snappy.parquet names.
```

Web/current-source check:

- Exa result: Open Targets Platform docs, "Download datasets", says post-25.03 paths use parquet directories and snake_case singular dataset names.
- Upstream EBI listing confirmed `disease/` and `clinical_indication/` exist in 26.03 and are single-file parquet datasets (`disease.parquet`, `clinical_indication.parquet`), not `part-*` shards.

## Raw evidence / FSV

Primary readback artifact:

- Path: `/home/croyse/calyx/fsv/issue884-molecular-preflight-final-20260625T104428Z/summary.json`
- Bytes: `7772`
- SHA256: `c16348982a462692ab0eb135ab9614ca7c5be27ee71ea5ac1530105110edf960`
- Parquet reader: `/zfs/archive/calyx/datasets/.dataset_tools_venv/bin/python3 + pyarrow`

Key file readback:

- Fresh ChEMBL SQLite tarball: bytes `5764252857`, SHA256 `33c203740555f96067710cdfc1c3c55d890660e5908ec5cbf5817492c290d281`
- Stale ChEMBL SQLite tarball kept separate: bytes `5764989236`, SHA256 `d5dd02de559abf99d1c14d67a1cb7fcb4ba9e5476ab40034ed4d4a199226c55f`
- ChEMBL SDF: bytes `935716795`, SHA256 `f9735be33875fa15999bf9c30f068b3d9545b4e0db737e1387dd1a4e99ca155e`
- BindingDB TSV zip: bytes `590990498`, SHA256 `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7`
- BindingDB target sequences FASTA: bytes `7599053`, SHA256 `e33decfbec34872ac376a4e08312f5cd94f3baa546d24f77dd83185a571b2c81`
- UniProt SwissProt FASTA gzip: bytes `94330734`, SHA256 `f3bff2df3e3883b737791daf9eec4591e9ef3c3dba31f9750067354b92363463`
- UniProt human proteome FASTA gzip: bytes `7752225`, SHA256 `cf49a88c4812dabbd934cb3e2e00b449e70375816e4d47cda7cc5b77b0754024`

Open Targets 26.03 readback after repair:

- `target`: `10` parquet files, `78691` rows, `85031914` bytes
- `disease`: `1` parquet file, `47030` rows, `7312633` bytes
- `drug_molecule`: `5` parquet files, `22230` rows, `2679269` bytes
- `drug_mechanism_of_action`: `2` parquet files, `6505` rows, `579870` bytes
- `drug_warning`: `1` parquet file, `2302` rows, `250932` bytes
- `clinical_indication`: `1` parquet file, `53950` rows, `3453490` bytes
- `association_overall_direct`: `43` parquet files, `4508002` rows, `633763096` bytes

Downloader script readback:

- Path: `/home/croyse/calyx/fsv/issue884-molecular-preflight-final-20260625T104428Z/fix_ot2_script_readback.json`
- Bytes: `815`
- SHA256: `f7513ca343ce2bf5db3b85aa8fee6ebc66d788247909a9b488d46cec7958a888`
- Script `/zfs/archive/calyx/biomed-rx/scripts/fix_ot2.sh`: bytes `930`, SHA256 `7978985792a65e53cf71fd59ccd9f15fe5ff8adb5f1c1c5aeaa7431fc54052ad`
- Filename extraction check: `disease` matched `1`, `clinical_indication` matched `1`, `target` matched `10`

## Findings (honest)

- The molecular source corpus is now materially present for #884 preflight: ChEMBL, BindingDB, UniProt, and the required Open Targets entity/evidence datasets have source-of-truth bytes and row-count metadata.
- The fresh ChEMBL tarball matches the canonical SHA recorded in issue state; the older tarball is still present but hash-distinct and should not be used as canonical input.
- Real operational bug found and fixed on aiwonder: `fix_ot2.sh` missed Open Targets single-file parquet datasets because it matched only `part-*-c000.snappy.parquet`.
- This is not the #884 acceptance yet. No molecular vault has been commissioned, no ESM2/ChemBERTa/ModernGENA embeddings have been written to a Calyx vault, and no clinical-to-molecular bridge has been demonstrated.
- #869 currently owns the GPU. Per #860, do not run model/vault commands that load panels while that ingest is active.

## Conclusion & next step

The #884 data substrate is ready enough for converter design and later GPU-free-to-GPU transition. Once #869 completes and the GPU is free, build a small molecular vault slice first: ChEMBL/BindingDB molecule-target rows plus Open Targets target/disease/drug links, with anchors for binding/activity/clinical indication, then FSV a clinical-to-molecular bridge before scaling.
