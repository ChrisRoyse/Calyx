# PH44 · T02 — Fault detectors (corruption / drift / decay)

| Field | Value |
|---|---|
| **Phase** | PH44 — Self-Heal (Rebuild Derived, Degrade Flags) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/heal/triggers.rs` (≤500) |
| **Depends on** | T01 (DegradeRegistry — detectors call `set_health`) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/12 §2`, `dbprdplans/24 §7` |

## Goal

Implement the five fault detectors that continuously monitor healable components
and transition their `DegradeRegistry` state when a fault is detected: (1) ANN/
kernel/guard checksum corruption; (2) lens endpoint health probe failure; (3)
τ drift (FAR creep past threshold); (4) lens signal decay below `0.05 bits`;
(5) stale derived structure (rebuild lag exceeds bound). Each detector runs in
the background budget (T04 of PH43) and fires into `DegradeRegistry`.

## Build (checklist of concrete, code-level steps)

- [ ] `trait FaultDetector: Send + Sync { fn check(&self, registry: &mut DegradeRegistry) -> Vec<FaultEvent>; }` — each detector implements this; `FaultEvent` carries `component`, `fault_kind`, and `recommendation`.
- [ ] `struct ChecksumDetector { components: Vec<(ComponentKind, ChecksumEntry)> }` — computes SHA-256 of ANN/kernel/guard index files; compares to stored `ChecksumEntry`; fires `FaultKind::Corruption` on mismatch.
- [ ] `struct LensProbeDetector { endpoints: Vec<(LensId, Url)>, http_client: Arc<dyn HttpProbe> }` — probes each TEI endpoint with a timeout; fires `FaultKind::EndpointFailing` on timeout/error; uses exponential backoff before `Failing` transition.
- [ ] `struct TauDriftDetector { ward_metrics: Arc<dyn WardMetrics> }` — reads current FAR from Ward; fires `FaultKind::TauDrifted` when FAR exceeds `τ + drift_tolerance`.
- [ ] `struct SignalDecayDetector { assay: Arc<dyn AssayMetrics> }` — reads per-lens `bits_per_anchor` from Assay; fires `FaultKind::SignalDecayed` when `bits < 0.05`.
- [ ] `struct StaleDetector { rebuild_lag_bound: Duration }` — fires `FaultKind::StaleIndex` when a derived structure's last-rebuild timestamp is older than bound.
- [ ] `struct FaultMonitor` — owns all detectors + a `BudgetHandle`; runs each on a configurable cadence (default `tick_interval_ms=10_000`); feeds results into `DegradeRegistry`; logs `FaultEvent`s to the Anneal Ledger.
- [ ] Clock-injected everywhere; no `SystemTime::now()`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `ChecksumDetector` with a known-good checksum + unmodified file → no fault; flip one byte in the checksum → `FaultKind::Corruption` fired for the right component.
- [ ] unit: `SignalDecayDetector` with `bits=0.04` → `FaultKind::SignalDecayed`; `bits=0.06` → no fault.
- [ ] proptest: for any set of `(bits, threshold)` pairs, `SignalDecayDetector` fires iff `bits < 0.05`.
- [ ] edge: `LensProbeDetector` with all endpoints timing out → all lens endpoints → `Failing`; `TauDriftDetector` with FAR exactly at boundary → no fault (boundary is exclusive); empty component list → no faults, no panic.
- [ ] fail-closed: `WardMetrics` returns `Err` → `FaultKind::MetricsUnavailable` (not a silent no-fault); HTTP probe panics → caught, logged as `FaultKind::ProbeError`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `DegradeRegistry` state after a simulated fault; Ledger `FaultEvent` entries.
- **Readback:** `calyx anneal status --faults --last 5` — prints recent fault events with component, kind, and timestamp.
- **Prove:** modify the SHA-256 checksum entry for the ANN index (don't touch the index itself, just the stored hash); run `FaultMonitor.check`; confirm `DegradeRegistry` shows `AnnIndex: Degraded`; `calyx anneal status --faults` shows `FaultKind::Corruption` entry with correct component.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH44 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
