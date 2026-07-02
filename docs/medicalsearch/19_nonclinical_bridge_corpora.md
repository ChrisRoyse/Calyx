# 19 - non-clinical bridge corpora

- **Issue:** #994   **Phase:** P0 discovery   **Date (UTC):** 2026-07-02   **Vault/panel:** `issue994-nonclinical-final-20260702t112430z` / bridge-corpus
- **Goal:** materialize a real non-clinical corpus substrate so `calyx domain-bridges` can prove clinical x molecular bridge behavior against persisted source bytes, not only the #869 clinical-QA vault.

## Implemented
- Added `calyx materialize-bridge-corpus <name> --rows <jsonl> [--home <dir>]`.
- The command requires every input row to carry `source_dataset`, `source_path`, and `source_sha256`, rejects empty/duplicate rows, and rejects any declared `bridge_terms` value that does not appear in the row text.
- It writes a durable Aster vault, creates anchored row nodes plus bridge-term nodes, persists CSR, writes `home/vaults/index.json`, then reopens `PhysicalAsterAssocSnapshot::latest` as readback.
- The materialized row nodes use `label:bridge-corpus`, so the existing `calyx domain-bridges` command can mine clinical x molecular roots with `--anchor-kind label:bridge-corpus`.

## Real source corpus
FSV root: `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z`

Source files:
- `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/pubmedqa.anchored.jsonl`
  - bytes `2191493`, rows `1000`
  - sha256 `4e86655c4cf83e7c5c38f81a3bcdc6ed3a538ec17bba43ab61a5665c430ca5ff`
  - selected row `271`, source id `21801416`
- `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medmcqa.anchored.jsonl`
  - bytes `207914204`, rows `182822`
  - sha256 `ede2cd900fa48756dbba18b891d24b5c95b7f04011e4fe93a49c63c8e788ffc2`
  - selected row `416`, source id `8b1e7f01-b79f-4f24-a759-3f3fed9c1978`
- `/zfs/archive/calyx/biomed-rx/discovery/bindingdb/BindingDB_All_202606_tsv.zip`
  - bytes `590990498`
  - sha256 `87d69d552be6dff78bb8e071d0e6c5b4c8f98312cce354ecb0b1ab8bfa8650c7`
  - entry `BindingDB_All.tsv`, uncompressed bytes `8856851970`, data rows `3182518`
  - selected rows `108468` and `50408024`

Derived proof input:
- `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/bridge_rows.jsonl`
- rows `4`, domain counts `clinical=2`, `molecular=2`, bridge term `metformin`
- bytes `5428`, sha256 `c5d02f132a7f286644f1ce3ab2aa4415e2a470a38647257f77de546c21372ad1`
- source summary bytes `1867`, sha256 `ae4a5d661876edc7cac5578a7be2d6de9b640b93f6134e6d106364428b27566c`

## Full State Verification
Materialize command:
```bash
CALYX_HOME="$FSV/home" "$BIN" materialize-bridge-corpus "$VAULT" \
  --rows "$FSV/bridge_rows.jsonl" \
  --home "$FSV/home" \
  > "$FSV/materialize.stdout.json" 2> "$FSV/materialize.stderr"
```

Materialize readback:
- vault id `01KWH9791WR1E0J6WJBKKV3G7G`
- index entry path `vaults/01KWH9791WR1E0J6WJBKKV3G7G`
- materialize stdout bytes `660`, sha256 `99d3c4a874ae2d529257b52b39a15f8b4d4dcc2f29aefbf00f9c21c070ea7196`
- index artifact bytes `234`, sha256 `f7f6c9992cf365d2742a29e81e9136e78dcd1080329b5a53047efe028eb70c06`
- graph nodes written `5`, edges written `8`, CSR persisted `true`
- snapshot readback: `index_contains_name=true`, `node_count=5`, `edge_count=8`
- stderr confirmed physical CSR readback: `plain-graph: loading persisted CSR collection=default nodes=5 edges=8`

Bridge command:
```bash
CALYX_HOME="$FSV/home" "$BIN" domain-bridges "$VAULT" \
  --pair metadata:domain=clinical metadata:domain=molecular \
  --anchor-kind label:bridge-corpus \
  --scope-radius 1 \
  --max-evidence-hops 2 \
  --kernel-target-fraction 1.0 \
  --max-per-pair 10 \
  --out "$FSV/domain_bridges.report.json" \
  > "$FSV/domain_bridges.stdout.json" 2> "$FSV/domain_bridges.stderr"
```

Bridge report readback:
- report bytes `1591`, sha256 `fecb530aa2d5d1414f86116d57fb1cceaae17c34c4191aadc7fff0c858f9821f`
- stdout bytes `1669`, sha256 `567bfefb2611a66c8acb2076dae48385203696c443974b62f7d28a141560b21f`
- `pair_count=1`, `candidate_count=1`
- candidate text `metformin`
- candidate provenance included `metadata:domain=bridge_term`, `metadata:source_dataset=bridge_terms`, `metadata:source_id=metformin`, `metadata:term=metformin`, `metadata:row_count=4`
- persisted report hash matched the hash printed by the CLI.

Negative cases:
- Missing molecular pair root:
  - command used `--pair metadata:domain=clinical metadata:domain=not_real`
  - rc `2`
  - no `missing_domain.report.json` was created
  - stderr contained `domain scope metadata:domain=not_real has no source-of-truth root nodes`
- Invalid bridge term:
  - command used a row with text containing `metformin` and declared bridge term `absent-term`
  - rc `2`
  - stderr contained `bridge corpus row 1 text does not contain bridge term absent-term`
  - no `issue994-bad-bridge-term` entry was written to the vault index.

Final readback artifact:
- `/home/croyse/calyx/fsv/issue994-nonclinical-final-20260702T112430Z/fsv_readback.json`
- Confirms index entry, artifact hashes, report hash match, failure return codes, absent failure artifacts, and vault disk files under `cf/graph`, `cf/time_index`, `codebooks`, `panel`, and `wal`.

## Local Gates
- `cargo fmt --all -- --check`: pass.
- `git diff --check`: pass.
- `bash scripts/linecount.sh`: pass, all `.rs` files <= 500 lines.
- `cargo test -p calyx-cli cmd::bridge_corpus::tests --target-dir target\issue994-cli-bridge-corpus-final -- --nocapture`: 4 passed.
- `cargo test -p calyx-cli cmd::tests::vault_subcommands_round_trip --target-dir target\issue994-cli-roundtrip-final -- --nocapture`: 1 passed.

## Finding
The non-clinical corpus gap from #876 is now materially closed for a molecular proof slice: real clinical rows and real BindingDB molecular rows were hashed, counted, converted into a persisted physical Aster vault, reopened from disk, and mined by `domain-bridges`.

This is still a narrow bridge-corpus materialization, not a full BindingDB-scale vault ingest. The existing `create-vault` template path remains blocked by stale frozen lens contracts in shared templates; that is tracked separately as #1128 and is a registry/template hygiene issue rather than proof that the source corpus is unavailable.
