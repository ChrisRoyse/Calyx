# #1197 Graph Collection Lifecycle Cleanup

Status: complete for the graph lifecycle cleanup slice. This is database
hygiene for association discovery evidence; it is not a treatment, cure,
clinical recommendation, or actionability claim.

## What Changed

Calyx now has explicit graph collection generation lifecycle rows in the Aster
Graph CF. The lifecycle collection is `__calyx_graph_lifecycle`, with generation
states:

- `writing`
- `accepted`
- `failed`
- `tombstoned`

Materializers now create a `writing` generation state before graph writes and an
`accepted` generation state only after physical graph/CSR/report readback.
Maintenance can also write lifecycle rows with:

```text
calyx graph-collection-state <vault> --collection <name> --generation <id> --state <writing|accepted|failed|tombstoned> --command <name> [--reason <text>] [--detail <k=v>] [--home <dir>]
calyx graph-collection-generations <vault> [--collection <name>] [--home <dir>]
```

Default physical graph readers now fail closed when lifecycle rows exist for a
collection and none are `accepted`. Materializers use the explicit
`PhysicalPlainGraph::open_latest_unchecked` path only for their own
pre-acceptance readback.

## Real Vault FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1197-graph-lifecycle-20260704T001050Z
```

Before readback showed no lifecycle rows:

```json
{"before_counts": {}, "before_generations": 0}
```

After write/readback:

- total lifecycle generations: 10
- accepted generations: 3
- tombstoned generations: 7
- every per-row command exit file read back as `0`

Accepted collections:

- `biomed_evidence_substrate_v3`, generation `issue1196-accepted-v3`
- `biomed_lincs_cmap_reversal_v4`, generation `issue1179-accepted-v4`
- `biomed_lincs_cmap_reversal_v7`, generation `issue1199-accepted-v7`

Tombstoned collections:

- `biomed_evidence_substrate`, generation `issue1196-aborted-v1`
- `biomed_evidence_substrate_v2`, generation `issue1196-aborted-v2`
- `biomed_lincs_cmap_reversal_v1`, generation `issue1179-aborted-v1`
- `biomed_lincs_cmap_reversal_v2`, generation `issue1179-aborted-v2`
- `biomed_lincs_cmap_reversal_v3`, generation `issue1179-aborted-v3`
- `biomed_lincs_cmap_reversal_v5`, generation `issue1199-aborted-v5`
- `biomed_lincs_cmap_reversal_v6`, generation `issue1199-aborted-v6`

Artifact hashes:

- `lifecycle_all.json`: sha256
  `eb57a6b1f85f60ee7b4bdd51ada46be614926d2c059c0a44635e89b9bab85940`
- `lifecycle_summary.json`: sha256
  `8c54cd75808f13656fb05d5d1353d4aa6582a831afd09f0dee76b421337d1d3e`
- `sha256sums.txt`: sha256
  `ad8c7e079abb433de7ae4c9fb1d20979ca8e6c483323b53f52cef351a8f53915`

The 10 persisted lifecycle value hashes were:

```text
0bc1bee03a2975d5ae683e37476d1ebb52ff658cd05d18fd2b114b312f66e3f0
2fb7cf3aa820ddef672fac5fce934dacdb8c8d27ff5b3b4339635e6496530412
3b8f2f02e7fe0f63f3a8e248ed715cb55737e706defda62e6cd8bfb8a2bac28c
709f33281e6268999819070ded9253a7658f8deaac36aad8cb08282385dd7018
8b8339194dacc3a48bccf25bb131e4ccf00eed07d845048849ca101e1c1bcc1f
8e5a6067440c9ec8bc5adba4c0491f089fd6db8854eb8b80907f04a48bc03491
97c54365dac094d75d131d57575810b3ad9f0802a40e27629864f93ce13ae737
a6d5ff0dd21dcdc9801371c169bbea4e106d25264a2e39cd08e56f7f12851748
c099826c14b65be3ff89cd65b933d9ecc89eed7cc9e43bac61fba2078e75a564
e37ae188e5e5e4d05c1a96edd05cca1a7b4545a404b92581843443f289a5bad8
```

## Fail-Closed Reader Proof

Command:

```text
CALYX_HOME=/home/croyse/calyx ./target/debug/calyx materialize-graph-csr corpus-anchored-869-20260625T080546Z --collection biomed_lincs_cmap_reversal_v5
```

This read path failed before CSR materialization because v5 is tombstoned:

```json
{"code":"CALYX_GRAPH_COLLECTION_NOT_ACCEPTED","message":"graph collection biomed_lincs_cmap_reversal_v5 has lifecycle rows but no accepted generation","remediation":"mark an accepted generation after physical readback or use a different graph collection"}
```

Fail-closed artifacts:

- `tombstoned_reader_failclosed.exit`: `2`
- `tombstoned_reader_failclosed.stdout`: 0 bytes
- `tombstoned_reader_failclosed.stderr`: sha256
  `8d2624d0d6e416f8be0114e284963bae0e034a26d462044e16f310661f79008c`

## Gates

Local and aiwonder focused gates passed:

- `cargo fmt --check`
- `bash scripts/linecount.sh`
- `cargo check -p calyx-aster -p calyx-cli`
- `cargo test -p calyx-aster graph_collection_lifecycle -- --nocapture`
- `cargo test -p calyx-cli graph_collection -- --nocapture`
- `cargo test -p calyx-cli lincs_reversal -- --nocapture`
- `cargo test -p calyx-cli evidence_substrate -- --nocapture`
- `cargo test -p calyx-cli token_roundtrip -- --nocapture`

`#1198` remains the performance follow-up for batch/range edge readback APIs.
