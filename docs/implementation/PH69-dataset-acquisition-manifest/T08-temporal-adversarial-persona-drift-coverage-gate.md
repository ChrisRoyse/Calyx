# PH69 · T08 — Temporal / adversarial / persona / drift acquisition + coverage gate

| Field | Value |
|---|---|
| **Phase** | PH69 — Dataset acquisition + MANIFEST + checksum FSV |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `—` (scripts/infra) |
| **Files** | `scripts/acquire_temporal_adversarial.sh` (≤500) |
| **Depends on** | T02, T03, T04, T05, T06, T07 (all prior acquisition cards complete) |
| **Axioms** | A2, A34 |
| **PRD** | `28 §3` rows 9–12, `28 §3.2`, `28 §7` (`DATA` BUILD_DONE clause) |

## Goal

Acquire the remaining four modality groups (event-log/temporal, adversarial/OOD,
synthetic personas, drift pair) to `/zfs/archive/calyx/datasets/<name>/`,
checksum-verify, and write MANIFEST rows. Then run the coverage gate: assert
`datasets/MANIFEST.md` contains ≥1 verified dataset per (modality × outcome-type),
satisfying the `DATA` BUILD_DONE clause (PRD `28 §7`).

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/acquire_temporal_adversarial.sh`:

      **Event logs / temporal (row 9):**
      Download server/application event logs or financial tick data from a free
      source (e.g., HF `datasets` `numenta/NAB` or `nasdaq_100` tick, or Kaggle
      `datasets/server-log-data` if Kaggle creds added to Infisical) to
      `/zfs/archive/calyx/datasets/temporal_logs/`; if private/unavailable, generate
      a synthetic event stream with planted periodicity (fixed seed, deterministic)
      and store it there; record `synthetic=true` in MANIFEST.

      **Prompt-injection / jailbreak / OOD (row 10):**
      HF `deepset/prompt-injections`, `JasperLS/prompt-injections`, or
      `lmsys/toxic-chat` to `/zfs/archive/calyx/datasets/prompt_injection/`;
      OOD split derived from an existing text dataset with distribution shift label.

      **Synthetic personas — Polis (row 11):**
      Generate in-repo via `scripts/gen_personas.py` (fixed seed, 21-slot civic
      constellation schema, ~1000 personas) to
      `/zfs/archive/calyx/datasets/synthetic_personas/`;
      `synthetic=true` in MANIFEST; no external download required.

      **Drift pair (row 12):**
      Derive month-A vs month-B splits from an acquired text dataset (e.g.,
      AG News by date or a news corpus with timestamps) and write to
      `/zfs/archive/calyx/datasets/drift_pair/`; record source dataset and split
      criteria in MANIFEST.

      Call `verify_dataset.sh <name>` after each; fail-closed on mismatch.

- [ ] MANIFEST rows for each, e.g.:
      `| prompt_injection | huggingface:deepset/prompt-injections | <revision> | <sha256> | <rows> | <bytes> | Apache-2.0 | Ward injection-block ≥99% |`
      `| synthetic_personas | in-repo/scripts/gen_personas.py | seed=42 | <sha256> | 1000 | <bytes> | MIT | Polis constellation/guard |`

- [ ] **Coverage gate script** `scripts/check_manifest_coverage.sh`:
      reads `datasets/MANIFEST.md`; asserts at least one row per required
      (modality × outcome-type) cell:
      `text-semantic/qrels` (T02), `text/class-label` (T03),
      `code/test-pass-fail` (T04), `graph/community` (T05),
      `text/duplicate-label` (T06), `audio-speaker/identity` (T07),
      `audio/emotion-label` (T07), `image/class-caption` (T07),
      `temporal/recurrence` (this card), `adversarial-text/injection-benign` (this card),
      `civic/tie-formation` (this card), `text/distribution-shift` (this card);
      exits 1 + prints missing cells if any are absent.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: run `check_manifest_coverage.sh` against a synthetic MANIFEST that has
      all 12 required cells → exits 0; remove one row → exits 1 with the missing
      cell named.
- [ ] proptest: property that a MANIFEST with the full required set always passes
      the coverage gate.
- [ ] edge (≥3):
      (1) MANIFEST missing `audio-speaker/identity` row → gate fails with clear
          message naming the missing cell;
      (2) synthetic persona generation with seed=42 produces deterministic sha256
          (run twice → same file);
      (3) drift pair where both splits are from the same month → script warns
          `CALYX_DATASET_DRIFT_SAME_PERIOD`, does not write MANIFEST row.
- [ ] fail-closed: `check_manifest_coverage.sh` on an empty MANIFEST → exits 1,
      lists all 12 missing cells.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** all 12 modality-group directories under `/zfs/archive/calyx/datasets/`
  on aiwonder; `datasets/MANIFEST.md` with ≥12 rows; exit code of
  `check_manifest_coverage.sh`.
- **Readback:**
  ```
  bash scripts/verify_dataset.sh prompt_injection
  bash scripts/verify_dataset.sh synthetic_personas
  bash scripts/verify_dataset.sh temporal_logs
  bash scripts/verify_dataset.sh drift_pair
  bash scripts/check_manifest_coverage.sh
  wc -l $CALYX_HOME/datasets/MANIFEST.md
  cat $CALYX_HOME/datasets/MANIFEST.md
  ```
- **Prove:** `check_manifest_coverage.sh` exits 0 (all 12 cells covered);
  `wc -l MANIFEST.md` shows ≥13 lines (header + ≥12 data rows);
  every verify exits 0; the DATA BUILD_DONE clause is satisfiable.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot of `check_manifest_coverage.sh` exit 0
      and full MANIFEST) attached to the PH69 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
