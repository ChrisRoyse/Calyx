# 26 - Molecular vault scaleout

- **Issue:** #1175
- **Status:** Complete FSV for the first scaled ChEMBL/BindingDB molecular vault slice.
- **FSV root:** `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z`
- **Final vault:** `issue1175-molecular-scaleout-20260703t161602z-v8-fixed-full`
- **Vault id:** `01KWME1BXHQKZ0R1WQ7J4D51P0`
- **Vault dir:** `/home/croyse/calyx/vaults/01KWME1BXHQKZ0R1WQ7J4D51P0`

This is discovery and triage evidence only. It is not a cure, treatment recommendation, efficacy proof, causality proof, or clinical actionability claim.

## What changed

The #884 molecular bridge was a four-row proof slice. #1175 scales that into a measured molecular vault slice with ChEMBL, BindingDB, Open Targets, ChEMBL indication, protein sequence, and DNA evidence.

The scaleout exposed a real runtime blocker: CPU multimodal adapter helpers are one-shot framed commands. Reusing the same child process worked for #884 because each multimodal lens saw one row, but it failed on repeated molecule rows with `multimodal response header read failed`. The fix respawns CPU multimodal helpers per request while leaving GPU mux workers shared.

Runtime patch:

- `crates/calyx-registry/src/runtime/adapters/bridge.rs`
- `crates/calyx-registry/src/runtime/adapters/tests.rs`

Linux regression:

```bash
cargo test -p calyx-registry \
  runtime::adapters::tests::cpu_adapter_respawns_one_shot_helper_for_repeated_measurements \
  -- --nocapture
```

Result: passed on `aiwonder`.

## Source hashes

| Source | Path | SHA256 |
|---|---|---|
| ChEMBL 37 SQLite tarball | `/zfs/archive/calyx/biomed-rx/discovery/chembl-fresh/chembl_37_sqlite.tar.gz` | `33c203740555f96067710cdfc1c3c55d890660e5908ec5cbf5817492c290d281` |
| ChEMBL 37 extracted DB | `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/chembl_sqlite/chembl_37/chembl_37_sqlite/chembl_37.db` | `4be13df3b68e25dcd0bff44bf094033b5aebe98f415acdc8c1cdf380e0c15142` |
| ChEMBL 37 SDF | `/zfs/archive/calyx/biomed-rx/discovery/chembl/chembl_37.sdf.gz` | `f9735be33875fa15999bf9c30f068b3d9545b4e0db737e1387dd1a4e99ca155e` |
| ChEMBL 37 FASTA | `/zfs/archive/calyx/biomed-rx/discovery/chembl/chembl_37.fa.gz` | `8f59596c4ee8f6cc7abcc59a4ea6f785ce428945de322fe9be6fb50808a7a9ae` |
| BindingDB all TSV zip | `/zfs/archive/calyx/biomed-rx/discovery/bindingdb/BindingDB_All_202606_tsv.zip` | `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7` |
| BindingDB target FASTA | `/zfs/archive/calyx/biomed-rx/discovery/bindingdb/BindingDBTargetSequences.fasta` | `e33decfbec34872ac376a4e08312f5cd94f3baa546d24f77dd83185a571b2c81` |
| Open Targets validation edges | `/home/croyse/calyx/fsv/issue1174-open-targets-validation-20260703T160748Z/open_targets_validation_edges.jsonl` | `824ee4f86c0a408593f5f5378f501d9422e1e1c426a649ab80017a4cb1a23869` |
| NCBI DPP4 RefSeq FASTA | `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/ncbi_DPP4_NM_001935.fasta` | `024fa67def136d8572230fe1eb301de3b877c56c76a6de6015cad7fdd63fd4b7` |

## Candidate generation

BindingDB scan:

- Scanned rows: `3,182,518`
- Selected candidate counts before bounding: DPP4 `6,327`, TNF `3,799`, CD4 `84`, PLA2R1 `1`
- Persisted bounded BindingDB rows: `25`
- Artifact: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/bindingdb_activity_candidates.jsonl`
- SHA256: `924c7fb2d20003ce3ba403b3f5e709eeb3360bb2a068642ab883749772c63a6d`

ChEMBL target activity counts:

| Target | Activities | Molecules | nM rows | SMILES rows |
|---|---:|---:|---:|---:|
| DPP4 | 8,397 | 5,752 | 6,469 | 8,392 |
| TNF | 6,447 | 2,582 | 3,023 | 6,442 |
| CD4 | 115 | 95 | 50 | 115 |
| PLA2R1 | 3 | 3 | 1 | 3 |

Generated row files:

- Raw generated rows: `57`, SHA256 `276fc1411ebecf95f7080bfdc32bdb082bb1c9bfc6ab16f5d6f11fb598e1300e`
- Deduped materialized rows: `53`, SHA256 `44d6444aba4e7f3d274d84a97cef416d09b7d864113f2ead705a3730e7d2bbb6`
- Duplicate provenance merged: `4` molecule duplicate rows where ChEMBL and BindingDB had the same measurement input.

## Materialized vault

Final materializer artifact:

- `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/materialize.v8.stdout.json`
- SHA256 `9b54ec106935f0ca2a26342e03bdc167acc02ad0273ac4182cbe4f91d69deb40`

Persisted row counts:

| Count | Value |
|---|---:|
| Rows | 53 |
| Clinical rows | 22 |
| Molecular rows | 31 |
| Text rows | 22 |
| Molecule rows | 26 |
| Protein rows | 4 |
| DNA rows | 1 |
| Affinity/activity rows | 26 |
| Bridge terms | 58 |
| Anchors | 251 |
| Graph nodes | 111 |
| Graph edges | 238 |

Measured slot readback:

| Lens | Dense rows |
|---|---:|
| `semantic_bge_small_en_v1_5` | 22 |
| `semantic_all_minilm_l6_v2_onnx` | 22 |
| `domain_scibert_scivocab_uncased` | 22 |
| `bge_small_fp32_gpu` | 22 |
| `minilm_fp32_gpu` | 22 |
| `medcpt_query_fp32_gpu` | 22 |
| `medcpt_article_fp32_gpu` | 22 |
| `a38_medcpt_query_int8` | 22 |
| `a38_medcpt_article_int8` | 22 |
| `protein_esm2_t30_150m_adapter` | 4 |
| `dna_moderngena_base_adapter` | 1 |
| `molecule_chemberta_100m_adapter` | 26 |

Separate `cx-list` readback:

- Path: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/cx_list_v8_readback.json`
- SHA256: `e364ac842cb49cd2183be6d6ad4794892431308ab73bf4cdfb1ae3cb4427f827`
- Rows: `53`
- Rows with dense slot payloads: `53`

Vault tree readback:

- Path: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/vault_tree_v8_readback.json`
- SHA256: `1db907b52104ea3be9b61ac2f69871a8db5ab257dba726e408ddb9441a8f3d61`
- Lines: `809`
- Files: `753`
- Graph SST files: `352`
- Base SST files: `2`
- Anchor SST files: `2`

## Bridge report

Command output:

- `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/domain_bridges.v8.stdout.json`

Report:

- `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/domain_bridges.v8.report.json`
- SHA256 `d9dc59f2c7db4bbb8f73134049ff99ed2c0dd99cfc83da5fa6f76874e96f08e2`

Bridge candidates:

| Rank | Bridge | Row count | Rank score | Gate |
|---:|---|---:|---:|---|
| 1 | DPP4 | 15 | 0.9000 | pass |
| 2 | TNF | 14 | 0.8667 | pass |
| 3 | CD4 | 12 | 0.8000 | pass |
| 4 | PLA2R1 | 6 | 0.6000 | pass |
| 5 | metformin | 5 | 0.5667 | pass |
| 6 | ChEMBL1431 | 4 | 0.5333 | pass |
| 7 | ChEMBL237500 | 4 | 0.5333 | pass |
| 8 | linagliptin | 4 | 0.5333 | pass |

The gates prove the terms bridge the persisted clinical and molecular rows. They do not prove a treatment effect or novel biology.

## Fail-closed probes

Bad missing `source_sha256`:

- Input: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/bad_missing_source_sha_rows.jsonl`
- RC: `2`
- Error: `molecular vault row 1 metadata requires source_sha256`

Bad unsupported modality:

- Input: `/home/croyse/calyx/fsv/issue1175-molecular-scaleout-20260703T161602Z/bad_unsupported_modality_rows.jsonl`
- RC: `2`
- Error: `molecular vault row 1 modality must be text, protein, dna, or molecule`

Mutation check:

- Pre-bad `cx-list` SHA256: `e364ac842cb49cd2183be6d6ad4794892431308ab73bf4cdfb1ae3cb4427f827`
- Post-bad `cx-list` SHA256: `e364ac842cb49cd2183be6d6ad4794892431308ab73bf4cdfb1ae3cb4427f827`
- Post-bad row count: `53`

## Full-scale plan

The first scaled vault is intentionally bounded. Full-scale execution should now use the fixed adapter lifecycle and split into deterministic batches:

1. Generate per-target ChEMBL activity batches with a row cap per target and a source hash on every row.
2. Generate per-target BindingDB batches with deduplicated molecule inputs and merged duplicate-source provenance.
3. Add target sequence rows for every mapped target with UniProt/ChEMBL accession and sequence hash.
4. Add DNA/transcript rows only from source-backed FASTA/RefSeq/Ensembl artifacts; do not synthesize DNA.
5. Materialize one vault per bounded target family or disease area, then merge graph-level bridge reports instead of overloading one vault transaction.
6. Run external validation layers before ranking: Open Targets, PubMed/PubTator relation evidence, ClinicalTrials.gov, DGIdb, safety/counter-evidence, and oncology-specific resources.
7. Treat all ranked outputs as hypotheses until falsification, safety, trial status, and expert review gates are complete.

The next useful work is not to call any result a cure. It is to keep expanding the validation stack around these measured bridge candidates.
