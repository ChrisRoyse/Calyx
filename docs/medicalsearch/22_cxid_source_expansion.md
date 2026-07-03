# 22 - CxId source expansion

- **Issue:** #1171
- **Date (UTC):** 2026-07-03
- **Status:** Complete FSV for current association-result CxId surfaces.
- **Final FSV root:** `/home/croyse/calyx/fsv/issue1171-complete-cxid-source-expansion-20260703T153528Z`
- **Controlled-copy repair root:** `/home/croyse/calyx/fsv/issue1171-controlled-base-compact-20260703T152816Z`

## Bottom line

The current association-result CxIds are now expanded to physical source rows in one persisted pack.

- `2,612` unique current association-result CxIds targeted.
- `2,612` source-row verified.
- `0` unresolved.
- `8` #994/#884 molecular bridge row records included as row evidence.
- The #870 source vault was not compacted or mutated.

The canonical #870 vault still fails closed on direct CxId reads because of legacy base-CF SST ordering ambiguity. To avoid mutating the source vault, the work created a controlled copy containing top-level manifest files plus `cf/base`, copied required small manifest references, compacted only the copied `base` CF, scanned all `198,993` copied base rows, and mapped each target CxId back to archived source JSONL rows by `source_dataset`, `source_id`, and `source_sha256`.

This is not a cure, treatment recommendation, or efficacy proof. It is the evidence expansion layer needed before concept normalization, typed biomedical scoring, and external validation.

## Persisted Artifacts

| Artifact | Purpose | SHA256 |
|---|---|---|
| `complete_cxid_source_expansion.jsonl` | One verified source-backed record per current association-result CxId | `3f6c25f4394d24815dcf01548afd86662c6295a41cd826266801f5ca1b1775b6` |
| `complete_cxid_source_expansion.json` | JSON object form of the same records | `13d186d15e428bc693d9a8d7eb0e8102e24c57fc98b3f153f82d2b4278d8ab07` |
| `target_cxids_by_surface.json` | CxId inventory by association-result surface | `5580f32afc04ba35914335c93a2887e3ccef45b86debe473855133d517396aea` |
| `molecular_rows_expansion.jsonl` | #994/#884 clinical/molecular bridge rows | `1046b927a71bef77a7cd8c74009c06f34af40cffbc53b296e0c41b0a3f5794d8` |
| `unresolved_cxids.jsonl` | Empty unresolved set | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `readback_summary.json` | FSV summary and source/hash inventory | `569db92d5cd7c8c092553c63ed696afcdb78dfc76a82bdd6c1ecc9bbd9ff78bc` |
| `persisted_readback.json` | Separate readback from persisted JSONL files | `5e13a468bae109a7c835f91dade6d0d987f27c372d10d2d6c23e5b6b20c446d8` |

## Controlled-Copy Proof

The source vault point read still fails closed:

```text
CALYX_ASTER_SST_ORDER_AMBIGUOUS
```

The controlled-copy repair was limited to the FSV copy:

```text
COMPACTED CF base INPUT_FILES 99498 INPUT_BYTES 640669344 OUTPUT_BYTES 318073064 LOGICAL_BYTES 306405120 WRITE_AMP_MILLI 1038
```

The copied base scan then read all rows:

```json
{
  "base_scan_exit": "0",
  "base_scan_rows": 198993,
  "base_scan_stdout_bytes": 697479783
}
```

Controlled-copy artifacts:

| Artifact | Path | SHA256 |
|---|---|---|
| Base compact stdout | `/home/croyse/calyx/fsv/issue1171-controlled-base-compact-20260703T152816Z/compact.retry.stdout` | `8956739e18ef82573fcc4856ce77c9ec13574416e8d34d1026dd6e49becb38c8` |
| Base scan JSON | `/home/croyse/calyx/fsv/issue1171-controlled-base-compact-20260703T152816Z/cx-list-all.stdout.json` | `82824e685e3add8f22dcdfa3dc6b887664c4fcc82cff0606f8c86aeda71b6586` |

## Coverage Readback

| Surface | Target CxIds | Source-row verified |
|---|---:|---:|
| #875 blind-spot candidates | `128` | `128` |
| #875 blind-spot neighbors | `1,944` | `1,944` |
| #876 domain bridges | `7` | `7` |
| #877 spectral bridge candidates | `30` | `30` |
| #877 spectral centrality candidates | `32` | `32` |
| #878 discovery accepted hops | `403` | `403` |
| #878 discovery accepted paths | `403` | `403` |
| #880 chain-walk node metadata | `166` | `166` |
| #882 ranked hypotheses | `16` | `16` |
| #882 ranked hypothesis source refs | `16` | `16` |

Separate persisted readback:

```json
{
  "complete_records_jsonl_readback": 2612,
  "molecular_rows_jsonl_readback": 8,
  "status_counts_jsonl_readback": {
    "source_row_verified": 2612
  },
  "summary_source_row_verified": 2612,
  "summary_unresolved_unique_cxids": 0,
  "unresolved_records_jsonl_readback": 0
}
```

## Input Source Hashes

| Input | Path | SHA256 |
|---|---|---|
| #882 ranked hypotheses | `/home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z/ranked_hypotheses_report.json` | `0483d8bc475526f65d76cd2fbb8a2a42c59751fa463c338b6e3fba54ac992257` |
| #880 chain walks | `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json` | `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd` |
| #876 domain bridges | `/home/croyse/calyx/fsv/issue876-domain-bridges-20260629-030832/real_pubmedqa_medxpertqa.json` | `a9649d15c48b60c28e508633e65a66acc89945da21fcf0c9124f519ee4731f04` |
| #877 spectral report | `/home/croyse/calyx/fsv/issue877-real-spectral-20260629-045547/happy2_stdout.json` | `cf37a095b014865863fdd2d41c21d9a7a371dc94e7b9a6e6aa6e1fc70fb3b5b2` |
| #878 discovery chain | `/home/croyse/calyx/fsv/issue878-real-discovery-fullanchors-20260629-061500/happy_stdout.json` | `12faacad51d9108a8eeff28e9d3fa3cb544c6ad8268aaba24a866f3345377990` |
| #875 blind-spot sweep | `/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J/idx/blind_spot/1782619688096-5dd11782/blind_spot_sweep.json` | `270315cfea2ea101987cb6da2fd96fb202cb5374e8649ff39e92f1efa4ee1801` |
| #994 bridge rows | `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/bridge_rows.jsonl` | `c5d02f132a7f286644f1ce3ab2aa4415e2a470a38647257f77de546c21372ad1` |
| #884 molecular rows | `/home/croyse/calyx/fsv/issue884-molecular-vault-20260703T142944Z/molecular_rows.jsonl` | `37b75c42d888aa1304a0b135e370f8c36291c76e4efbc99fe76c5c875855b3d2` |

## Source-Row Files

| Dataset | Path | SHA256 |
|---|---|---|
| `medmcqa` | `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medmcqa.anchored.jsonl` | `ede2cd900fa48756dbba18b891d24b5c95b7f04011e4fe93a49c63c8e788ffc2` |
| `medqa` | `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medqa.anchored.jsonl` | `180330c8deaa086dced6e9d2beec0a39068f842d967e1f06da96b99506ac944c` |
| `medxpertqa` | `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medxpertqa.anchored.jsonl` | `880e5a2208c2de97edd6ca2aab979846b9a5a85f91253dbbe5c19e68e051326c` |
| `pubmedqa` | `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/pubmedqa.anchored.jsonl` | `4e86655c4cf83e7c5c38f81a3bcdc6ed3a538ec17bba43ab61a5665c430ca5ff` |

## Current Signal

The top #882 cluster expands to known asthma pharmacology exam-source rows: salbutamol/terbutaline for acute attack and salmeterol/formoterol as long-acting beta-2 agonists. That is useful as a known-positive validation and calibration signal, not as a new discovery claim.

The #875 blind-spot candidates and neighbors are now inspectable source rows rather than opaque `input_hash` references. The next useful step is concept normalization (#1172), because raw source rows need drug, disease, gene/protein, pathway, phenotype, and evidence-type IDs before typed discovery scoring can produce biomedical intelligence.

The #994/#884 molecular slice remains narrow but source-backed: clinical metformin rows, BindingDB metformin rows, DPP4 protein, and DPP4 DNA. It proves text/molecule/protein/DNA materialization and bridge mechanics, not therapeutic efficacy.

## Closeout

#1171 acceptance is met for current association-result CxId surfaces:

- Persisted expansion artifact includes CxId, source text, dataset, source ID, source hash, source file, source line, parent surfaces, and parent candidate/hypothesis refs.
- Top #882 hypotheses are expanded from opaque A/B/C CxIds into source text and metadata.
- All targeted CxIds are resolved; unresolved file is empty.
- FSV used separate readback from persisted JSONL and archived source files.
- The source vault was not mutated.

Next execution should move to #1172 concept normalization and #1170 result-pack assembly using this expansion as input.
