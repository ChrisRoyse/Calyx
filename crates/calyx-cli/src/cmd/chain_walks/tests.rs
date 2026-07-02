use super::*;

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}

#[test]
fn parse_chain_walks_command() {
    let anchor = "00000000000000000000000000000000"
        .parse::<calyx_core::CxId>()
        .unwrap();
    let parsed = parse_chain_walks(&tokens([
        "corpus",
        "--seed-file",
        "seeds.json",
        "--anchor",
        "00000000000000000000000000000000",
        "--max-hops",
        "4",
        "--branch-width",
        "8",
        "--probe-width",
        "16",
        "--max-groundedness-distance",
        "2",
        "--min-gate-confidence",
        "0.5",
        "--novelty-weight",
        "0.25",
        "--max-hypotheses-per-seed",
        "3",
        "--min-terminal-confidence",
        "0.5",
        "--out",
        "target/chain-walks.json",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::ChainWalks(ChainWalksArgs {
            vault: "corpus".to_string(),
            seed_file: "seeds.json".into(),
            anchors: vec![anchor],
            anchor_files: Vec::new(),
            max_hops: 4,
            branch_width: 8,
            probe_width: 16,
            max_groundedness_distance: 2,
            min_gate_confidence: 0.5,
            novelty_weight: 0.25,
            max_hypotheses_per_seed: 3,
            min_terminal_confidence: 0.5,
            out: Some("target/chain-walks.json".into()),
        })
    );
}

#[test]
fn parse_chain_walks_requires_seed_file_and_anchor_source() {
    let err = parse_chain_walks(&tokens(["corpus"])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--seed-file"));

    let err = parse_chain_walks(&tokens(["corpus", "--seed-file", "seeds.json"])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--anchor"));
}
