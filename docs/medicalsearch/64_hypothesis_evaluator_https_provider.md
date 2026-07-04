# Hypothesis Evaluator HTTPS Provider

Issue: #1216

Status: complete FSV.

`calyx hypothesis-evaluator-driver` now supports OpenAI-compatible `https://`
evaluator endpoints in addition to local deterministic `http://` stubs. HTTPS
transport requires `--auth-env <VAR>`; the named environment variable is read at
runtime and used only as an in-memory bearer token.

## Contract

- `http://` endpoints remain supported for local deterministic evaluator FSV.
- `https://` endpoints require `--auth-env <VAR>`.
- Missing or empty HTTPS auth fails closed with
  `CALYX_HYPOTHESIS_EVALUATOR_AUTH_MISSING`.
- HTTP 401/403 from HTTPS fails closed with
  `CALYX_HYPOTHESIS_EVALUATOR_AUTH_FAILED`.
- Other HTTPS non-2xx responses fail closed with
  `CALYX_HYPOTHESIS_EVALUATOR_ENDPOINT_STATUS`.
- Tokens are not persisted or printed.
- Persisted endpoint values are redacted to remove userinfo, query strings, and
  fragments.
- The evaluator artifact still records model id, endpoint, prompt set id/hash,
  temperatures, source input path/hash, input hypotheses, generated evaluator
  runs, and aggregate report.

## Verification

Focused gates:

```text
cargo fmt --check
bash scripts/linecount.sh
cargo test -p calyx-cli cmd::hypothesis_evaluator -- --nocapture
cargo check -p calyx-cli
```

The focused evaluator suite observed 6 passing tests, including HTTPS
mock-transport auth/redaction and missing-auth fail-closed coverage.

Physical FSV root:

```text
target/fsv/issue1216-https-evaluator-provider-20260704T074656Z
```

Readback summary:

```text
target/fsv/issue1216-https-evaluator-provider-20260704T074656Z/issue1216_fsv_readback_summary.json
```

Persisted artifact readback:

- input SHA-256:
  `fc065b8fd0ff4a66f7fd3a26f781d7344b06d6b168b79c05535fe9439be86aab`
- evaluator artifact SHA-256:
  `8be2a9ee3d860ef50be971f192419847b605bb5a5913474f0304b348435e6394`
- CLI stdout SHA-256:
  `aeaabdd63c1fc20fa00dbf232150ef77c535838aee03c07aa5c368a3fb4c5bcd`
- prompt set:
  `biomed_hypothesis_evaluator_v1`
- prompt set SHA-256:
  `a5ee6f1d827b6991bdbe880c36a92f9965beef7ed2d34072674a43d2f6d34a36`
- input count: 1
- evaluator run count: 4
- aggregate evaluation count: 1
- cited evidence readback: `evidence-1`

HTTPS negative readback:

- output artifact existed: `false`
- error stream contained `CALYX_HYPOTHESIS_EVALUATOR_AUTH_MISSING`
- redaction checks found no `api_key=secret` query leak
- redaction checks found no `token@example.invalid` userinfo leak

## Safety Boundary

This is a transport and provenance hardening slice only. Evaluator output remains
a ranked research lead over cited evidence. It is not a cure claim, clinical
recommendation, or clinical actionability proof.
