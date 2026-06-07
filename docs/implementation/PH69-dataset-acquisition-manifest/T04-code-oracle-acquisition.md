# PH69 · T04 — Code oracle acquisition (SWE-bench Lite / HumanEval / MBPP)

| Field | Value |
|---|---|
| **Phase** | PH69 — Dataset acquisition + MANIFEST + checksum FSV |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `—` (scripts/infra) |
| **Files** | `scripts/acquire_code_oracle.sh` (≤500) |
| **Depends on** | T01 (MANIFEST schema + verify tooling) |
| **Axioms** | A2, A34 |
| **PRD** | `28 §3` row 3, `28 §3.2` |

## Goal

Acquire the code-oracle corpora (SWE-bench Lite 300×8, HumanEval, MBPP) to
`/zfs/archive/calyx/datasets/<name>/`, checksum-verify each on arrival, and write
MANIFEST rows. SWE-bench Lite is the primary deterministic test oracle for PH70's
Oracle FSV: it provides a real pass/fail ground truth against which Oracle
sufficiency (≈0.46 deficit on a form-only panel) is measured (PRD `28 §2`, `28 §3`
row 3).

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/acquire_code_oracle.sh`:
      downloads with pinned revision:
      `/zfs/archive/calyx/datasets/swebench_lite/` — SWE-bench Lite (Princeton-NLP,
      HF `princeton-nlp/SWE-bench_Lite`), 300 instances × 8 fields
      (instance_id, repo, problem_statement, hints_text, patch, test_patch,
      FAIL_TO_PASS, PASS_TO_PASS);
      `/zfs/archive/calyx/datasets/humaneval/` — OpenAI HumanEval (164 problems);
      `/zfs/archive/calyx/datasets/mbpp/` — MBPP (374 problems, sanitized split).
- [ ] For each: record expected rows/sha256 pre-download; post-download verify;
      fail-closed on mismatch.
- [ ] MANIFEST rows, e.g.:
      `| swebench_lite | huggingface:princeton-nlp/SWE-bench_Lite | <revision> | <sha256> | 300 | <bytes> | MIT | Oracle sufficiency / ≈0.46 deficit |`
      `| humaneval | huggingface:openai_humaneval | <revision> | <sha256> | 164 | <bytes> | MIT | Oracle deterministic test pass/fail |`
      `| mbpp | huggingface:google-research-datasets/mbpp | <revision> | <sha256> | 374 | <bytes> | CC-BY-4.0 | Oracle deterministic test pass/fail |`
- [ ] Verify the 300-instance count for SWE-bench Lite exactly (the paper's own
      instantiation); assert `len(dataset) == 300` in the verify step.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: parse a synthetic 5-instance SWE-bench-format JSON with known
      instance_ids; assert row count = 5, each instance has `FAIL_TO_PASS` and
      `PASS_TO_PASS` keys, sha256 matches known value.
- [ ] proptest: property that verify round-trips — sha256 of downloaded file equals
      value from MANIFEST.
- [ ] edge (≥3):
      (1) SWE-bench Lite row count ≠ 300 (e.g., filtered split) → verify exits 1,
          `CALYX_DATASET_ROWCOUNT_MISMATCH`;
      (2) `FAIL_TO_PASS` field absent in an instance → script logs
          `CALYX_DATASET_SCHEMA_MISMATCH`, does not write MANIFEST row;
      (3) partial download → sha256 mismatch → `CALYX_DATASET_CHECKSUM_MISMATCH`.
- [ ] fail-closed: `acquire_code_oracle.sh` without `HF_HUB_TOKEN` → exits 1,
      `CALYX_SECRET_MISSING: HF_HUB_TOKEN`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/archive/calyx/datasets/swebench_lite/`,
  `/zfs/archive/calyx/datasets/humaneval/`,
  `/zfs/archive/calyx/datasets/mbpp/` on aiwonder; MANIFEST rows.
- **Readback:**
  ```
  bash scripts/verify_dataset.sh swebench_lite
  bash scripts/verify_dataset.sh humaneval
  bash scripts/verify_dataset.sh mbpp
  python3 -c "import json,pathlib; d=json.loads(pathlib.Path('/zfs/archive/calyx/datasets/swebench_lite/data.jsonl').read_text().splitlines()[0]); print(list(d.keys()))"
  cat $CALYX_HOME/datasets/MANIFEST.md | grep -E 'swebench|humaneval|mbpp'
  ```
- **Prove:** before: directories absent; after: verify exits 0 for all three;
  SWE-bench row count confirmed = 300 by Python one-liner; MANIFEST has 3 rows
  with populated sha256; live sha256 matches stored value.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH69 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
