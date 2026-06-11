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

- [x] `trait FaultDetector: Send + Sync { fn check(&self, registry: &mut DegradeRegistry) -> Vec<FaultEvent>; }` — each detector implements this; `FaultEvent` carries `component`, `fault_kind`, and `recommendation`.
- [x] `struct ChecksumDetector { components: Vec<(ComponentKind, ChecksumEntry)> }` — computes SHA-256 of ANN/kernel/guard index files; compares to stored `ChecksumEntry`; fires `FaultKind::Corruption` on mismatch.
- [x] `struct LensProbeDetector { endpoints: Vec<(LensId, Url)>, http_client: Arc<dyn HttpProbe> }` — probes each TEI endpoint with a timeout; fires `FaultKind::EndpointFailing` on timeout/error; uses exponential backoff before `Failing` transition.
- [x] `struct TauDriftDetector { ward_metrics: Arc<dyn WardMetrics> }` — reads current FAR from Ward; fires `FaultKind::TauDrifted` when FAR exceeds `τ + drift_tolerance`.
- [x] `struct SignalDecayDetector { assay: Arc<dyn AssayMetrics> }` — reads per-lens `bits_per_anchor` from Assay; fires `FaultKind::SignalDecayed` when `bits < 0.05`.
- [x] `struct StaleDetector { rebuild_lag_bound: Duration }` — fires `FaultKind::StaleIndex` when a derived structure's last-rebuild timestamp is older than bound.
- [x] `struct FaultMonitor` — owns all detectors + a `BudgetHandle`; runs each on a configurable cadence (default `tick_interval_ms=10_000`); feeds results into `DegradeRegistry`; logs `FaultEvent`s to the Anneal Ledger.
- [x] Clock-injected everywhere; no `SystemTime::now()`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `ChecksumDetector` with a known-good checksum + unmodified file → no fault; flip one byte in the checksum → `FaultKind::Corruption` fired for the right component.
- [x] unit: `SignalDecayDetector` with `bits=0.04` → `FaultKind::SignalDecayed`; `bits=0.06` → no fault.
- [x] proptest: for any set of `(bits, threshold)` pairs, `SignalDecayDetector` fires iff `bits < 0.05`.
- [x] edge: `LensProbeDetector` with all endpoints timing out → all lens endpoints → `Failing`; `TauDriftDetector` with FAR exactly at boundary → no fault (boundary is exclusive); empty component list → no faults, no panic.
- [x] fail-closed: `WardMetrics` returns `Err` → `FaultKind::MetricsUnavailable` (not a silent no-fault); HTTP probe panics → caught, logged as `FaultKind::ProbeError`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `DegradeRegistry` state after a simulated fault; Ledger `FaultEvent` entries.
- **Readback:** `calyx anneal status --faults --last 5` — prints recent fault events with component, kind, and timestamp.
- **Prove:** modify the SHA-256 checksum entry for the ANN index (don't touch the index itself, just the stored hash); run `FaultMonitor.check`; confirm `DegradeRegistry` shows `AnnIndex: Degraded`; `calyx anneal status --faults` shows `FaultKind::Corruption` entry with correct component.

### PH44 T02 FSV evidence

- **Root:** `/home/croyse/calyx/data/fsv-issue401-faults-20260611T084328Z`
- **Trigger:** stored checksum for synthetic ANN index slot 0 was intentionally wrong while the artifact bytes stayed fixed; `FaultMonitor::run_once` consumed budget, ran `ChecksumDetector`, wrote `DegradeRegistry`, and appended an Anneal `FaultEvent`.
- **Before SoT:** `ph44-fault-detectors-readback.json` recorded `before_health: {"state":"ok"}`.
- **After health SoT:** `calyx-anneal-status-health.txt` read `AnnIndex(slot_0): Degraded since=1785601400 reason=corruption: rebuild derived artifact from base slots`; `anneal-health-cf-readback.txt` and `anneal-health-sst-head.txt` contain key `ann_index/slot_0000` and value tag `anneal_health_v1` with `state:"degraded"`.
- **After ledger SoT:** `calyx-anneal-status-faults.txt` read `FaultEvent ts=1785601400 component=AnnIndex(slot_0) kind=corruption recommendation=rebuild derived artifact from base slots`; `anneal-ledger-scan.jsonl`, `wal-readback.txt`, and `anneal-ledger-sst-head.txt` contain `action:"fault_event"` and `fault_kind:"corruption"`.
- **SHA-256 readbacks:** `ph44-fault-detectors-readback.json` `9cf4544ab1ef2fc8348769599ca4a5e4b08b06ff2c7a2727a9a09b87b9f1371e`; `calyx-anneal-status-health.txt` `005b48555b698dce81e0d0447066229dc9b3c08309008d1cfdd718ba5f2107e1`; `calyx-anneal-status-faults.txt` `5ffe05f2ec466b36e260fdfe62c196fbe792adc79a58f961fc63d14582871576`; `anneal-health-cf-readback.txt` `3b3eddc0deba4cc9a754da5ee8008cdc1ddae1d0bcbb10587e5b8987ba33b485`; `anneal-ledger-scan.jsonl` `7ade9fe143b9d30837c553311fa01e30331faea2404305cc192fd64e934d48d6`; `wal-readback.txt` `a23d248568500fdea23b0fdcbc1cf2101c44cdbb2dd956ee54bcbb0ebc3486d7`; `physical-files.txt` `a988a1c0578a383523fc7bee7b5d6147b742f27621491714e7d7b8b0f09aaefb`.
- **BLAKE3 readbacks:** `ph44-fault-detectors-readback.json` `acc72a357505f620b92be39417bd6fbdb10060a0d5ba35cb4947770d76ca313f`; `calyx-anneal-status-health.txt` `5debd953124470cdc94d6c41487bfcc8aa2cac218302cf0f50854127b60e0c80`; `calyx-anneal-status-faults.txt` `9b47bb64264612ef4e2bf1b7b531ec9ad5f038130728f01a9c1d5d7be6f84fdf`; `anneal-health-cf-readback.txt` `82f50bd67d32829b2f87f3beffb9e5a51a498bf8067f46d6e3458d1dd616d9c6`; `anneal-ledger-scan.jsonl` `e3072537b4b483cbd8b43999b627ca0f6d16f557028db016472b8f10918ed26a`; `wal-readback.txt` `b0ede991f01f894e23a75d4ae16c89981674b5ba11aa3dc12e01f5eea76214d5`; `physical-files.txt` `125922235739eb52564218abb0c508458108d3edec8cc8cb15dbadaee6029b8a`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH44 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
