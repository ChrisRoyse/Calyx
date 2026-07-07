const KNOWN_COMMANDS: &str = "\
create-vault add-lens retire-lens park-lens retire-vault list-panel profile-lens \
ingest ingest-status anchor measure erase search kernel-answer bits kernel guard abundance \
propose-lens provenance verify-chain reproduce anneal-status rebuild-search-index kernel-build \
weave-loom domain-bridges materialize-bridge-corpus discovery-chain chain-walks probe-matrix spectral-communities \
materialize-graph-csr materialize-molecular-vault materialize-evidence-substrate materialize-lincs-reversal \
assemble-hypothesis-evidence association-validation-gates typed-association-miner hypothesis-falsification-sweep \
biomedical-blindspot-audit bridge-falsification-evaluate bridge-evaluate-rank novelty-calibration-split \
graph-collection-generations graph-collection-state";

pub(super) fn is_cmd(command: &str) -> bool {
    KNOWN_COMMANDS
        .split_whitespace()
        .any(|known| known == command)
}
