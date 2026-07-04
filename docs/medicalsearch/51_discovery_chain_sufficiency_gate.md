# #1205 Discovery-Chain Sufficiency Gate Hardening

## Scope

#1205 fixes the discovery-chain trust boundary. Before this slice, the default
`run_grounded_discovery_chain` path accepted hops from topological
anchor-reachability alone. A node within the configured graph radius could pass
without any calibrated assay evidence that the panel carried enough bits about
the outcome.

This is a safety/trust fix for biomedical discovery. It does not make any
clinical claim or cure claim. It prevents chain-walk hypotheses from being
labeled grounded unless a power-calibrated sufficiency instrument is present.

## Code Change

Changed behavior:

- `run_grounded_discovery_chain` and `run_grounded_chain_walks` now fail closed
  with `CALYX_DISCOVERY_NO_SUFFICIENCY_ASSAY` after input validation.
- The topology-only predicate was renamed/exported as `reachability_prior_gate`.
  It is diagnostic/ranking evidence, not a sole gate.
- Injected-gate APIs remain available:
  - `run_discovery_chain_with_gate`
  - `run_chain_walks_with_gate`
- Accepted hops now persist `gate_code` and `gate_evidence`, including the
  sufficiency lower bound evidence.
- `calyx discovery-chain` and `calyx chain-walks` now load:
  - current manifest-backed panel,
  - persisted `AssayStore` rows from the vault,
  - `Panel`, `OutcomeEntropy`, and per-lens rows scoped by vault id, panel
    version, assay domain, and anchor kind.
- CLI gates pass only when:
  - `ci_low >= anchor_entropy_bits`,
  - the panel estimate carries passing power calibration,
  - the reachability prior also passes.

New CLI flags:

```text
--assay-domain <domain>
--assay-anchor <reward|label:name|test_pass|tie_formed|thumbs|speaker_match|style_hold|recurrence>
```

Defaults remain `discovery-chain` and `reward`.

## FSV

Final FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1205-discovery-chain-sufficiency-gate-20260703T224122Z
```

Readback artifacts:

| Artifact | Purpose |
|---|---|
| `issue878_discovery_chain_readback.json` | Lodestar discovery-chain readback; accepted hops carry `ci_low=1.100000` and `anchor_entropy_bits=1.000000` |
| `issue880_chain_walks_readback.json` | Chain-walk report readback; hypothesis provenance carries sufficiency evidence |
| `cli_discovery_chain.log` | Physical CLI vault test: manifest panel + assay CF rows, persisted `chain.json` read back |
| `cli_chain_walks.log` | Chain-walk parser/token gate for assay keying flags |
| `check_lodestar.log` | `cargo check -p calyx-lodestar` |
| `check_cli.log` | `cargo check -p calyx-cli` |
| `linecount.log` | `bash scripts/linecount.sh` |
| `diffcheck.log` | `git diff --check` |

Commands passed:

```text
cargo test -p calyx-lodestar --test issue878_discovery_chain_tests -- --nocapture
cargo test -p calyx-lodestar --test issue880_chain_walks_tests -- --nocapture
cargo test -p calyx-cli discovery_chain -- --nocapture
cargo test -p calyx-cli chain_walks -- --nocapture
cargo check -p calyx-lodestar
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Edge cases covered:

- no injected library sufficiency gate -> `CALYX_DISCOVERY_NO_SUFFICIENCY_ASSAY`;
- missing persisted assay rows -> `CALYX_DISCOVERY_NO_SUFFICIENCY_ASSAY`, no
  chain artifact;
- persisted panel assay with `ci_low < H` -> refused, no chain artifact;
- strict reachability-prior threshold -> no accepted chain artifact;
- unknown start node still fails as `CALYX_GRAPH_UNKNOWN_NODE`;
- accepted physical CLI chain persists and reads back every accepted hop with
  `CALYX_DISCOVERY_SUFFICIENCY_PASS`, `ci_low`, `anchor_entropy_bits`, and
  `power_calibration=passed`.

## Conclusion

#1205 is complete for the discovery-chain and chain-walk trust boundary:
topology can rank and explain proximity, but it can no longer assert grounded
acceptance by itself. A chain now needs a calibrated sufficiency gate, and the
accepted-hop evidence is persisted for readback.
