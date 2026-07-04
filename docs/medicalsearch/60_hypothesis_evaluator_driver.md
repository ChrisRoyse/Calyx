# Hypothesis Evaluator Driver

Issues: #1201, #1216

`calyx hypothesis-evaluator-driver` replaces manual evaluator JSON plumbing for
biomedical A-B-C hypotheses. It consumes the persisted evidence bundle produced
by `calyx assemble-hypothesis-evidence`, calls a configured structured evaluator
endpoint for a versioned prompt set, validates evidence citations, aggregates
the resulting evaluator runs, and writes one replayable artifact.

## Command

```text
calyx hypothesis-evaluator-driver \
  --input <input.json> \
  --out <artifact.json> \
  --endpoint <http(s)://host[:port]/path> \
  --model <id> \
  [--auth-env <VAR>] \
  [--temperature <f>] \
  [--timeout-ms <ms>]
```

Default temperatures are `0.2` and `0.8`, giving two prompt templates and two
temperature variants per hypothesis. `http://` endpoints are supported for
local deterministic stubs. `https://` endpoints are OpenAI-compatible and
require `--auth-env <VAR>`; the named environment variable must contain the
bearer token. The token is never persisted or printed, and persisted endpoint
values are redacted to remove userinfo, query strings, and fragments. The
endpoint contract is OpenAI-like JSON or direct evaluator JSON. The persisted
artifact records:

- schema version
- model id
- endpoint
- prompt set id
- prompt set SHA-256
- temperatures
- source input path and SHA-256
- input hypotheses with generated evaluator runs
- aggregate `HypothesisEvaluationReport`

## Fail-Closed Contract

The driver refuses to persist an artifact when any evaluator variant fails:

- endpoint unreachable:
  `CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_UNREACHABLE`
- missing HTTPS bearer-token environment configuration:
  `CALYX_HYPOTHESIS_EVALUATOR_AUTH_MISSING`
- HTTPS bearer-token rejected:
  `CALYX_HYPOTHESIS_EVALUATOR_AUTH_FAILED`
- HTTPS endpoint returned a non-auth non-2xx status:
  `CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_STATUS`
- malformed evaluator response:
  `CALYX_HYPOTHESIS_EVALUATOR_MALFORMED_RESPONSE`
- cited evidence id missing from the input bundle:
  `CALYX_HYPOTHESIS_EVALUATOR_BAD_CITATION`

Each variant failure includes the hypothesis id, prompt id, and temperature.
This keeps ranking evidence tied to physical Calyx evidence rows and prevents an
LLM response from smuggling in uncited support.

## Safety Boundary

Evaluator output is a grounded research-lead score, not a cure claim or clinical
recommendation. Clinical actionability still requires outcome anchors,
falsification against independent real datasets, mechanism checks, safety
evidence, and human review gates.

## Verification

Focused tests:

```text
cargo test -p calyx-cli cmd::hypothesis_evaluator -- --nocapture
cargo test -p calyx-cli cmd::hypothesis_evidence -- --nocapture
cargo test -p calyx-cli known_subcommand_help_bypasses_required_arg_validation -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
```

Physical FSV artifact:

```text
target/fsv/issue1201-hypothesis-evaluator-20260704T055120Z/readback-summary.json
```

The FSV run used a local stub endpoint, invoked the real CLI, persisted
`driver-artifact.json`, and read it back. Readback observed four endpoint calls,
four evaluator runs, one aggregate evaluation, prompt set
`biomed_hypothesis_evaluator_v1`, prompt hash
`a5ee6f1d827b6991bdbe880c36a92f9965beef7ed2d34072674a43d2f6d34a36`, and
artifact SHA-256
`1946f486c768e146bd11afbc92f0cc2e76813d95d47de6ead3f9c4bcab48444b`.
