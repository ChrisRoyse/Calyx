# 59 - Hypothesis evidence bridge

- **Issue:** #1200
- **Date (UTC):** 2026-07-04
- **Status:** Implemented and FSV-backed for the chain-walk -> evaluator-input bridge.
- **FSV root:** `target/fsv/issue1200-hypothesis-evidence-20260704T053721Z`

## What changed

`calyx assemble-hypothesis-evidence <vault> --chain <chain.json> --out <input.json>`
now materializes the JSON input consumed by `calyx hypothesis-evaluate`.

The bridge:

- reads chain-walk hypotheses from the persisted chain artifact;
- reads each required A/B/C/path `CxId` from the physical Calyx vault;
- builds deterministic `RetrievedEvidence` rows with `source_cx_id`, title, persisted
  abstract/text, grounding confidence, and provenance including `source_sha256`;
- dedupes repeated CxIds deterministically;
- fails closed on missing Base rows, missing `source_sha256`, or empty abstract/text.

## Raw evidence

Focused checks:

```text
cargo test -p calyx-lodestar hypothesis_evidence -- --nocapture
cargo test -p calyx-cli cmd::hypothesis_evidence -- --nocapture
cargo test -p calyx-cli vault_subcommands_round_trip -- --nocapture
cargo test -p calyx-cli known_subcommand_help_bypasses_required_arg_validation -- --nocapture
cargo check -p calyx-lodestar
cargo check -p calyx-cli
git diff --check
bash scripts/linecount.sh
```

End-to-end FSV:

```text
CALYX_HOME=target/fsv/issue1200-hypothesis-evidence-20260704T053721Z/home
calyx create-vault issue1200-evidence --panel-template text-default
calyx ingest issue1200-evidence --batch batch.jsonl --idempotent --output rows
calyx assemble-hypothesis-evidence issue1200-evidence \
  --chain chain_walks.synthetic.json \
  --out hypothesis-evaluate.input.json
```

Readback summary:

- `input_count=1`
- `evidence_count=3`
- duplicate terminal `B` deduped to one evidence row
- `all_expected_matches=true`
- `chain_sha256=48973c1983538175275df467b694bc4c31546330d2acc00c24ab2f59299c47a3`
- `output_sha256=e80c22cfe4f29e447f267104dac4b2883d54618146d98ec55f84fd72392a110d`
- `readback_summary_sha256=76ba3ef12d00769104328d600359e2ab2b617213849a06af5d642b01e633fdcb`

## Boundary

This bridge removes the manual JSON-authoring bottleneck for evaluator evidence.
It does not make biomedical hypotheses clinically actionable. Hypotheses remain
evidence-backed research leads until outcome validation, falsification, safety,
and human review gates pass.
