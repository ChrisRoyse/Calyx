# calyx-poly

`calyx-poly` is the local Polymarket intelligence crate inside the Poly-owned Calyx workspace. It
ingests public/read-only market evidence, stores it in local Calyx data structures, derives forecast
signals, grounds those signals against resolved outcomes, writes local forecast artifacts, and gates
forecast admission. It does not place orders, sign orders, manage bankroll, use Polymarket trading
surfaces, or emit execution instructions.

## Local Boundary

Allowed work:

- Read public market data and public on-chain data.
- Persist local source snapshots, Calyx constellations, anchors, diagnostics, forecast artifacts,
  policy decisions, admission rows, and score reports.
- Launch forecast agents only through Infisical-backed runtime metadata and local artifact writeback.
- Score forecasts against resolved outcomes with Brier, calibration, sufficiency, recall, and drift
  evidence.

Forbidden work:

- Any order signing, order submission, order cancellation, redemption, website trading, user-order
  monitoring, bankroll management, stake sizing, Kelly sizing, or PnL optimization.
- Any fallback that silently fabricates absent data, unresolved outcomes, uncalibrated guards, or
  source provenance.

## Core Flow

1. Ingest: public/read-only snapshots enter `model`, `raw_sources`, or raw large-corpus helpers.
2. Represent: `features`, `lenses`, `encode`, and `constellation` preserve typed numeric truth and
   write Calyx slots/scalars without flattening.
3. Associate: diagnostics and gates such as `panel_diagnostics`, `pair_gain_gate`, `kernel_recall`,
   `kernel_recall_admission`, and `knn_base_rate` measure local association evidence.
4. Ground: `grounding`, `outcome_backfill`, `anchor_floor`, `no_lookahead`, and
   `resolved_market_corpus` bind evidence to resolved outcomes and refuse lookahead.
5. Predict: `forecast`, `forecast_blend`, `forecast_calibration`, `forecast_ceiling`,
   `calyx_native`, `oracle_forecast`, `ward_calibration`, and agent artifact modules create local
   probability records.
6. Gate: `admission`, `superiority`, `risk`, `oracle`, `wash`, `policy`, and `policy_audit` refuse
   unsafe, stale, ungrounded, uncalibrated, or trading-related actions.
7. Score: `score`, `backtest`, and `reproducibility` write durable local evidence for forecast
   quality and bit-for-bit replay.

## Module Map

- `admission`: local forecast-quality gate; no execution economics.
- `agent_artifacts`, `agent_launcher`, `agent_secrets`: optional DeepSeek forecast-agent artifacts
  and Infisical-backed runtime metadata.
- `book_liquidity`: read-only CLOB book-depth and liquidity feature rows.
- `calyx_native`: local Calyx-native forecast record assembly.
- `constellation`, `pipeline`: snapshot ingestion into Calyx slots, scalars, anchors, and vault rows.
- `panel_sufficiency`, `panel_diagnostics`, `pair_gain_gate`: Assay/Loom-backed sufficiency and
  interaction evidence.
- `kernel_forecast`, `kernel_recall`, `kernel_recall_admission`, `knn_base_rate`: kernel and kNN
  forecast evidence.
- `oracle`, `oracle_forecast`, `risk`, `wash`: read-only data-quality and manipulation-risk screens.
- `policy`, `policy_audit`: local-only runtime boundary and durable policy evidence.
- `raw_sources`, `raw_large_corpus`, `schema_derivation`: public-source inventory, sampling, and
  schema evidence.
- `score`, `backtest`, `reproducibility`: outcome scoring, held-out validation, and artifact replay.
- `ward_calibration`: Ward conformal calibration provenance for admission.

## Build And Verification

From `C:\code\poly\Calyx`:

```powershell
cargo check -p calyx-poly
cargo test -p calyx-poly
cargo clippy -p calyx-poly -- -D warnings
```

Project doctrine treats those commands as supporting gates only. Full State Verification is the
proof standard: execute the code, read back the source of truth separately, exercise at least three
edges, and record the smallest corpus that fully proves the claim.

Operational runbooks that are part of the crate's public verification surface live in
`runbooks/`, with repository-level copies kept under `docs/` in the private development checkout.

## FSV Pattern

Every non-trivial change should include:

- Proof claim: the exact behavior being proven.
- Minimum sufficient corpus: the smallest data set that proves the behavior, with why smaller is
  insufficient and larger is wasteful.
- Happy path: known-truth input with exact expected output.
- At least three edges: stale, malformed, missing, insufficient, circular, ungrounded, uncalibrated,
  crossed/invalid, or forbidden-action inputs as appropriate.
- Source-of-truth readback: vault CF rows, files, graph artifacts, ledger entries, or issue state
  read independently after execution.

## Failure Policy

`calyx-poly` fails closed. Unknown lens, missing source, non-finite numeric field, ungrounded
evidence, stale calibration, insufficient anchors, malformed LLM output, forbidden action, or
readback mismatch must return a structured error and write no successful forecast/admission row.
