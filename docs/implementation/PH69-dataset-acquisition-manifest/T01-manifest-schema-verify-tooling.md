# PH69 · T01 — MANIFEST schema + verify_dataset tooling

| Field | Value |
|---|---|
| **Phase** | PH69 — Dataset acquisition + MANIFEST + checksum FSV |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `—` (scripts/infra) |
| **Files** | `datasets/MANIFEST.md` (schema + header), `scripts/verify_dataset.sh` (≤500), `scripts/acquire_datasets.sh` (≤500) |
| **Depends on** | PH00 (CALYX_HOME, ZFS archive pool, `hf_hub_token` in Infisical) |
| **Axioms** | A2, A15, A34 |
| **PRD** | `28 §3.2`, `28 §3` |

## Goal

Establish the `datasets/MANIFEST.md` schema (name, source, version, sha256, rows,
license, what-it-tests) and the reusable `verify_dataset.sh` tooling that every
subsequent acquisition card uses to checksum-verify a dataset on aiwonder. This
tooling is itself the FSV mechanism for PH69: acquisition is not "done" until
`verify_dataset.sh` exits 0 for that dataset's MANIFEST row.

## Build (checklist of concrete, code-level steps)

- [ ] Create `datasets/MANIFEST.md` with header row:
      `| name | source | version/revision | sha256 | rows | size_bytes | license | what_it_tests |`
      and a `_template` comment row explaining each field.
- [ ] Write `scripts/verify_dataset.sh <name|ALL>`:
      reads the MANIFEST row for `<name>` (or iterates all rows for `ALL`);
      recomputes `sha256sum` of the dataset directory;
      recomputes row count via `wc -l` on the primary split file or a Python one-liner;
      asserts sha256 matches MANIFEST value → exits 1 + prints `CALYX_DATASET_CHECKSUM_MISMATCH` if not;
      asserts row count matches MANIFEST value → exits 1 + prints `CALYX_DATASET_ROWCOUNT_MISMATCH` if not;
      prints `[OK] <name>` on success.
- [ ] Write `scripts/acquire_datasets.sh`:
      sources `$CALYX_HOME/.env` for `HF_HUB_TOKEN`;
      calls each per-modality acquire script in sequence;
      calls `verify_dataset.sh ALL` at the end;
      exits non-zero on any failure (fail-closed, A16).
- [ ] `CALYX_DATASET_CHECKSUM_MISMATCH` and `CALYX_DATASET_ROWCOUNT_MISMATCH` added
      to `calyx-core` error catalog (PH03 extension) so CLI and scripts emit structured codes.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: create a synthetic 3-row CSV with known sha256; run `verify_dataset.sh`
      against a MANIFEST row with the correct sha256/rows → asserts exit 0.
- [ ] proptest: property that `verify_dataset.sh` is idempotent — running it twice
      on the same directory produces the same exit code and stdout.
- [ ] edge (≥3):
      (1) wrong sha256 in MANIFEST → exits 1, prints `CALYX_DATASET_CHECKSUM_MISMATCH`;
      (2) correct sha256 but wrong row count → exits 1, prints `CALYX_DATASET_ROWCOUNT_MISMATCH`;
      (3) dataset directory missing → exits 1 with `CALYX_DATASET_NOT_FOUND`.
- [ ] fail-closed: `verify_dataset.sh NONEXISTENT_NAME` → exits 1,
      prints `CALYX_DATASET_NOT_FOUND`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `datasets/MANIFEST.md` on aiwonder at `$CALYX_HOME/datasets/MANIFEST.md`;
  the script exit codes as printed in the terminal.
- **Readback:**
  ```
  bash scripts/verify_dataset.sh ALL
  cat $CALYX_HOME/datasets/MANIFEST.md
  ```
- **Prove:** before: no `datasets/MANIFEST.md` exists;
  after: `MANIFEST.md` has the schema header; `verify_dataset.sh --self-test`
  passes against the synthetic fixture (exit 0); running it against a missing
  dataset name exits 1 with the structured error code.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH69 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
