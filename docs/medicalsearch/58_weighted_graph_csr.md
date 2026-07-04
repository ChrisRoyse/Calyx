# 58 - #1213 Weighted graph CSR evidence edges

## Scope

#1213 fixes the plain-graph CSR projection used by the biomedical evidence
overlay. Before this change, the projection discarded the stored edge evidence
payload and every `AssocGraph` edge was rebuilt as weight `1.0`.

This is a graph scoring/ranking fix. It does not establish biomedical efficacy,
safety, clinical actionability, or a cure.

## Code Change

Changed files:

| File | Change |
|---|---|
| `crates/calyx-aster/src/plain_graph/types.rs` | Added CSR edge `weight` plus strict edge-value weight parsing |
| `crates/calyx-aster/src/plain_graph/mod.rs` | CSR projection and in-process scan fallback now normalize positive support weights |
| `crates/calyx-aster/src/plain_graph/physical.rs` | Physical no-CSR fallback now reads Graph CF edge values before building `AssocGraph` |
| `crates/calyx-aster/src/plain_graph/assoc_graph.rs` | Persisted CSR decode now validates and applies per-edge weights |
| `crates/calyx-aster/src/plain_graph/csr_store.rs` | CSR manifest version bumped to 3 for the weighted edge schema |
| `crates/calyx-cli/src/cmd/evidence_substrate/write.rs` | Direct evidence-substrate CSR materializer now writes normalized weights |
| `crates/calyx-cli/src/cmd/lincs_reversal/write.rs` | Direct LINCS CSR materializer now writes normalized weights |
| `crates/calyx-aster/tests/issue1213_weighted_csr_fsv.rs` | Persisted weighted CSR/readback and downstream scoring FSV |

Weight derivation:

- Edge values must be a JSON number or a JSON object with numeric `weight`.
- The raw positive finite support values are normalized by the maximum support
  in the projection so `AssocGraph` receives weights in `(0,1]`.
- Empty, malformed, zero, negative, or non-finite values fail closed as
  `CALYX_GRAPH_CORRUPT_ROW`; there is no silent fallback to `1.0`.

## Verification

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1213-weighted-csr-20260704T052000Z
```

Commands:

```powershell
cargo test -p calyx-aster --test issue1213_weighted_csr_fsv -- --nocapture

$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1213-weighted-csr-20260704T052000Z'
cargo test -p calyx-aster --test issue1213_weighted_csr_fsv -- --nocapture

cargo test -p calyx-aster plain_graph -- --nocapture
cargo test -p calyx-aster -- --nocapture
cargo check -p calyx-cli
cargo test -p calyx-cli evidence_substrate -- --nocapture
cargo test -p calyx-cli lincs_reversal -- --nocapture
cargo check -p calyx-aster
bash scripts/linecount.sh
git diff --check
```

Persisted readback artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1213-weighted-csr-20260704T052000Z\issue1213_weighted_csr_readback.json
SHA256 B35BA5F7C81F702DF27C117AD5D214696C2E8C7F4F77FFC70F36C888EA6FA1E8
```

Readback summary:

```json
{
  "csr_edge_weights": [1.0, 0.1, 1.0, 1.0],
  "assoc_graph_edge_weights": [1.0, 0.1, 1.0, 1.0],
  "reach_scored": {
    "high_mid": 0.8999999761581421,
    "low_mid": 0.08999999612569809,
    "high_gt_low": true
  },
  "betweenness": {
    "high_mid": 0.16666666666666666,
    "low_mid": 0.0,
    "high_gt_low": true
  },
  "spectral": {
    "high_mid": 1.0,
    "low_mid": 0.6465868949890137,
    "high_gt_low": true
  },
  "edge_cases": {
    "empty_value": "CALYX_GRAPH_CORRUPT_ROW",
    "zero_weight": "CALYX_GRAPH_CORRUPT_ROW",
    "malformed_weight": "CALYX_GRAPH_CORRUPT_ROW"
  }
}
```

## Conclusion

#1213 is complete for the plain-graph CSR/`AssocGraph` scoring path. Persisted
biomedical graph overlays now carry per-edge evidence mass into traversal,
betweenness, and spectral scoring instead of flattening all edges to `1.0`.
