# 63 - Native discovery bridge CLIs

- **Issue:** #1220   **Phase:** 4   **Date (UTC):** 2026-07-04   **Vault/panel:** #1219 real discovery-run artifacts
- **Goal:** Replace run-local hand-authored bridge JSON with native deterministic CLIs between falsification, evaluation, and ranking.

## What changed

Added two native commands:

```text
calyx bridge-falsification-evaluate --miner-report <json> --falsification-report <json> --out <json> [--run-manifest <manifest.json> --run-stage-id <stage-id>]
calyx bridge-evaluate-rank --evaluation-report <json> --out <json> [--run-manifest <manifest.json> --run-stage-id <stage-id>]
```

Both commands:

- read persisted predecessor artifacts from disk;
- run discovery-run preflight against the exact input bytes before output;
- write the downstream input JSON accepted by `hypothesis-evaluate` or `hypothesis-rank`;
- write a sibling `.readback.json` with source paths, source SHA-256 values, output SHA-256, counts, preflight readback, `research_lead_only=true`, and the no-clinical-actionability boundary.

## Verification

Focused checks:

```text
cargo fmt --check
bash scripts/linecount.sh
cargo test -p calyx-cli cmd::discovery_bridge -- --nocapture
cargo test -p calyx-cli known_subcommand_help_bypasses_required_arg_validation -- --nocapture
cargo test -p calyx-cli vault_subcommands_round_trip -- --nocapture
cargo test -p calyx-cli cmd::discovery_run -- --nocapture
cargo check -p calyx-cli
git diff --check
```

Physical FSV:

```text
target/fsv/issue1220-native-discovery-bridges-20260704T073437Z/issue1220_fsv_readback_summary.json
```

The physical run consumed the real #1219 predecessor artifacts:

- miner report SHA-256: `5d170fa287ef0394c38476b714465612dbfdea0ed1d299e136bf0dac649b4b28`
- falsification report SHA-256: `95394107e6f563217017662bd457329f5bc0ecad87676af417122299fe65a780`

Readback leaves:

- `bridge_falsification_evaluate.readback_input_count=1`
- `hypothesis_evaluate.input_count=1`
- `hypothesis_evaluate.retained_count=1`
- `bridge_evaluate_rank.readback_input_count=1`
- `hypothesis_rank.ranked_count=1`
- `hypothesis_rank.top_hypothesis_id=typed-assoc:concept:drug::concept:disease`
- stale bridge preflight failed with `CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN`
- stale bridge output was not written

## Boundary

The bridge creates reproducible downstream inputs from grounded association artifacts. It does not create clinical actionability, efficacy, safety, dosing, or cure evidence. The ranked output remains a research lead requiring outcome, safety, and human-review gates before claim escalation.
