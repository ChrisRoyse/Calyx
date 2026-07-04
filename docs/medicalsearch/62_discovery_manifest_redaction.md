# 62 - Discovery manifest ledger redaction

- **Issue:** #1221   **Phase:** 4   **Date (UTC):** 2026-07-04   **Vault/panel:** discovery-run manifest / ledger
- **Goal:** Let ledger-sealed discovery-run manifests carry benign long provenance tokens while preserving fail-closed rejection of secret-like payloads.

## What was run (exact commands)

```text
cargo test -p calyx-ledger redaction -- --nocapture
cargo test -p calyx-cli cmd::discovery_run -- --nocapture
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Physical FSV:

```text
target/fsv/issue1221-discovery-manifest-redaction-20260704T070413Z/issue1221_fsv_readback_summary.json
```

## Raw evidence / FSV

The failing condition from #1219 was `CALYX_LEDGER_SECRET_IN_PAYLOAD` when a discovery-run manifest ledger payload contained full git SHA and descriptive run/corpus/stage provenance tokens.

The fix admits only bounded discovery manifest provenance token shapes:

- `git_sha`: 7 to 40 hex characters.
- `run_id`, `corpus_vault_id`, `stage_id`, `upstream_stage_id`, `command`: bounded slug-like manifest provenance with separators.

The secret scanner still rejects secret-like fields and opaque long tokens. Focused tests added:

- `check_payload_handles_discovery_manifest_tokens`
- `seal_accepts_long_benign_manifest_provenance_tokens`
- `seal_rejects_secret_like_manifest_token_without_artifact`

Physical readback observed:

- benign manifest `seal_verify_chain=Intact { count: 1 }`
- benign manifest `verify_chain=Intact { count: 1 }`
- ledger payload read back the full `run_id`, `corpus_vault_id`, and full 40-character `git_sha`
- secret-like manifest failed with `CALYX_LEDGER_SECRET_IN_PAYLOAD`
- secret-like seal output was not written

## Findings (honest)

- Grounded engineering finding: the ledger scanner was not distinguishing structured discovery manifest provenance from opaque no-space secret material.
- Grounded behavior after patch: long benign discovery manifest provenance seals and reads back through the ledger payload.
- Guard preserved: a secret-like manifest token returns `CALYX_LEDGER_SECRET_IN_PAYLOAD` and no seal artifact is written.

## Conclusion & next step

#1221 removes a reproducibility blocker for production biomedical discovery manifests. This does not change the clinical boundary: discovery-run manifests prove provenance and reproducibility, not clinical actionability, efficacy, safety, dosing, or cures.
