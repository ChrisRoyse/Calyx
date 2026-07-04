# #1191 Graph CSR and Traversal Cache Readback

## Scope

#1191 verifies that the large #869 association graph has a persisted
collection-local CSR/traversal substrate and that the large readers load it
instead of broad row-scanning the Graph CF.

This is infrastructure for association discovery. It is not a biomedical
hypothesis, treatment, cure, or clinical-actionability claim.

## Code-Level Fail-Closed Coverage

New Aster tests prove persisted CSR corruption fails closed:

- `physical_csr_reader_rejects_tampered_segment_hash`
- `physical_csr_reader_rejects_manifest_count_mismatch`

Both return `CALYX_GRAPH_CORRUPT_ROW` rather than silently rebuilding from graph
rows or accepting a torn CSR stream.

Focused local gates:

```bash
cargo test -p calyx-aster physical_csr_reader_rejects -- --nocapture
cargo test -p calyx-cli graph_csr -- --nocapture
```

Result: both passed.

## Real Vault FSV

Host: aiwonder.

Repo: `/home/croyse/calyx/repo`.

Vault: `corpus-anchored-869-20260625T080546Z`
(`01KVYX0KYVBQSGVC6N2S00FX6J`).

FSV root:

```text
/home/croyse/calyx/fsv/issue1191-graph-csr-traversal-cache-release-20260704T011254Z
```

The real default graph already had a persisted CSR before this issue's refresh
run. Therefore this FSV proves persisted-CSR presence, refresh/readback, and
reader preference. It is not an empty-to-present benchmark.

## CSR Materialization Readback

Command:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx materialize-graph-csr \
  corpus-anchored-869-20260625T080546Z \
  --collection default
```

Result:

```json
{
  "status": "ok",
  "collection": "default",
  "commit_seq": 684029,
  "csr_present_before": true,
  "csr_bytes_before": 157072071,
  "csr_bytes": 157072071,
  "csr_sha256": "72821df5df7bf02bc1c90afc2db4891847c66ea9d3f82b8832cb43424ca667f5",
  "nodes": 198993,
  "csr_edges": 2435817,
  "association_edge_count": 2435817,
  "readback": {
    "assoc_graph_nodes": 198993,
    "assoc_graph_edges": 2435817,
    "physical_node_keys": 198993,
    "physical_edge_out_keys": 2435817
  }
}
```

Stderr source-of-truth lines:

```text
materialize-graph-csr: before csr_present=true csr_bytes=Some(157072071)
materialize-graph-csr: committed seq=684029 nodes=198993 edges=2435817 association_edges=2435817 elapsed_ms=107940
plain-graph: loading persisted CSR collection=default nodes=198993 edges=2435817
materialize-graph-csr: physical readback ok csr_bytes=157072071 nodes=198993 graph_edges=2435817 elapsed_ms=115022
```

## Reader Preference and Timings

All three large readers logged persisted-CSR loading:

```text
plain-graph: loading persisted CSR collection=default nodes=198993 edges=2435817
```

Reader outputs were identical before and after the CSR refresh where expected.

| Reader | Before elapsed | After elapsed | Output readback |
|---|---:|---:|---|
| `spectral-communities` | 1:00.55 | 0:09.71 | 198,993 members, 2 communities, 8 bridge candidates, 8 centrality candidates |
| `domain-bridges` | 1:01.21 | 0:10.51 | 1 pair report, 7 candidates |
| `discovery-chain` | 0:57.11 | 0:06.66 | 198,993 graph nodes, 2,435,817 graph edges, 16 accepted hops, 52 candidates |

The after-run improvement includes OS/cache effects because the CSR already
existed at the start. The acceptance claim is reader behavior and physical CSR
readback, not a pure cold-cache performance ratio.

## Artifact Hashes

Selected hashes from `sha256sums.txt`:

| Artifact | SHA256 |
|---|---|
| `materialize_graph_csr.stdout` | `42da2318ce22731012911c7682aa2e8288718906c543f39efcc5dc3534c6269c` |
| `materialize_graph_csr.stderr` | `aafd4995c3fca0f7802e6d6268944022a9015f409be4254d6dfef00a535c0d4d` |
| `before_spectral_report.json` | `e6d215b547e40657183c9ee0957ec544ee4480089b1aa74d49107a4140065891` |
| `after_spectral_report.json` | `e6d215b547e40657183c9ee0957ec544ee4480089b1aa74d49107a4140065891` |
| `before_domain_report.json` | `8ab4c94b001714e5275e7a27d23d8071ba1b7a6dda097dfab5ee219443a980ef` |
| `after_domain_report.json` | `8ab4c94b001714e5275e7a27d23d8071ba1b7a6dda097dfab5ee219443a980ef` |
| `before_discovery_chain.json` | `da87f776bf2ed75e31eb575dc0e7b8518101f574c4543b749c61ccb545e12077` |
| `after_discovery_chain.json` | `da87f776bf2ed75e31eb575dc0e7b8518101f574c4543b749c61ccb545e12077` |
| `run.log` | `7d44f142bae4f6f575b7bd26662129f88af8108af0e0f8dbde3f8183abd86ca3` |
| `sha256sums.txt` | `0d921d7b9991bc3912f17727440da84a3641ba87aa274d0b783fa65c45686eee` |

`sha256sums.txt` contains 28 rows.

## Conclusion

#1191's graph-cache requirement is satisfied for the large #869 graph:

- persisted CSR exists for `default`;
- `materialize-graph-csr` can refresh it and read it back physically;
- physical readback checks CSR bytes/hash, CSR counts, association graph counts,
  and independent node/edge key counts;
- spectral, domain-bridge, and discovery-chain readers load the persisted CSR;
- corrupt CSR streams fail closed in focused tests.

The next broad-mining dependency is #1192/#1183: scalable probe/all-pair typed
association mining over the now-readable graph substrate.
