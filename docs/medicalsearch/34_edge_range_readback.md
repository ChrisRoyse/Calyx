# #1198 PlainGraph Edge Range Readback

Status: complete for the collection-local edge readback slice. This is graph
storage/readback correctness work for biomedical association evidence. It is
not a treatment, cure, clinical recommendation, or actionability claim.

## What Changed

`PhysicalPlainGraph` now exposes `edge_out_props()`, which reads every outgoing
edge row for one graph collection by that collection's encoded key range. It
does not point-read every expected edge and it does not scan unrelated graph
collections.

Materializer readback now uses collection-local range scans for all expected
values:

- LINCS/CMap reversal: all node values by `node_props()`, all edge values by
  `edge_out_props()`.
- Evidence substrate: all node values by `node_props()`, all edge values by
  `edge_out_props()`.

`PlainGraph::rebuild_csr` is documented as a collection-local projection: it
uses this `PlainGraph` instance's node and outgoing-edge key ranges.

## Unit Proof

Test:

```text
cargo test -p calyx-aster physical_edge_out_props -- --nocapture
```

The test creates `target` and `unrelated` graph collections in one vault, writes
one outgoing edge to each, flushes, and proves `PhysicalPlainGraph::edge_out_props()`
for `target` returns only the `target` edge value.

## Real Vault FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1198-edge-range-readback-20260704T002330Z
```

Command:

```bash
./target/debug/calyx materialize-lincs-reversal \
  corpus-anchored-869-20260625T080546Z \
  --root /home/croyse/calyx/fsv/issue1179-lincs-cmap-reversal-20260703T213000Z \
  --metadata-root /home/croyse/calyx/fsv/issue1199-lincs-perturbation-metadata-20260703T2240Z \
  --collection biomed_lincs_cmap_reversal_v8 \
  --report /home/croyse/calyx/fsv/issue1198-edge-range-readback-20260704T002330Z/calyx_db_readback_v8.json \
  --home /home/croyse/calyx
```

Exit:

```text
0
```

Report:

- `calyx_db_readback_v8.json`: sha256
  `8281d40dc70b5e4beea6dd7b37b4ef8f6aa281bf17108adadd65252ae68822cf`
- `materialize_v8.stdout`: sha256
  `3bc3c5b5d12fb07b2f18efe3d95203a0d15e13f4d026ac4bf4460c15ece4b0df`
- `materialize_v8.stderr`: sha256
  `ef687b619538b54d642647fca9a49bd8287fb8aaddda9ac9ea4543899c74f09d`
- `sha256sums.txt`: sha256
  `04981eecf94ad3c7e596aaed788c9087438e3a3b2f9214590ae1654356560c00`

Readback summary:

```json
{
  "collection": "biomed_lincs_cmap_reversal_v8",
  "graph_generation": "materialize-lincs-reversal-01KWN8RH7MRVJDY9TK9ENHF7BR",
  "nodes": 8993,
  "edges": 19150,
  "physical_edge_out_keys": 19150,
  "edge_value_readback_mode": "all physical edge values read back by collection range",
  "sampled_edge_values_read_back": 19150,
  "all_edge_values_read_back": true,
  "csr_bytes": 1821310,
  "csr_sha256": "57084512f9cbb83829db2b4f8d1d323a0f4d61750b324fb6cebbe689b987390f"
}
```

Lifecycle readback for `biomed_lincs_cmap_reversal_v8`:

- status: `accepted`
- generation: `materialize-lincs-reversal-01KWN8RH7MRVJDY9TK9ENHF7BR`
- value sha256:
  `1f1385e60502cee3f3f11fa2ffff31a15325ffffb59966428b19c9aeccd24ae0`
- lifecycle detail includes report path, node rows `8993`, edge rows `19150`,
  and CSR sha256 `57084512f9cbb83829db2b4f8d1d323a0f4d61750b324fb6cebbe689b987390f`.

## Conclusion

The old LINCS readback mode counted all edge keys and sampled 512 edge values.
The accepted v8 materialization read back all 19,150 edge values from the
collection-local physical Graph CF range. This closes the remaining #1198 edge
readback gap for the current biomedical materializers.
