# PH57 · T07 — Bounded ingest microbatch admission

**GitHub issue:** #590

## Scope

PRD 24 §6 requires the ingest microbatch itself to be bounded by construction:
slow lens endpoints must not let transient input buffers grow until OOM. This
card adds a registry-level `IngestMicrobatchController` that accounts the
bytes required for each admitted microbatch, fails closed with
`CALYX_BACKPRESSURE` when the cap would be exceeded, and records Prometheus
text metrics for buffer bytes, high-water, backpressure, lens timeouts, breaker
trips, degraded lenses, and open breakers.

The ordinary `Registry::measure_batch` path remains strict: absent vectors are
still rejected by frozen-lens validation. The ingest-specific path is
`Registry::measure_ingest_microbatch`, which returns explicit per-lens outcomes
so a timed-out lens can degrade to `SlotVector::Absent { LensUnavailable }`
while remaining lenses continue to measure and the ingest ack count advances.

## Implementation

- `crates/calyx-registry/src/ingest_microbatch.rs`
  - `IngestMicrobatchConfig`
  - `IngestMicrobatchController`
  - RAII `IngestMicrobatchPermit`
  - deterministic byte estimator using input bytes, pointer bytes, and a fixed
    per-input overhead
  - timeout-like `CALYX_LENS_UNREACHABLE` handling that trips a per-lens
    circuit breaker and routes the lens as degraded while preserving other
    lens work
  - Prometheus-format `metrics_text`
- `crates/calyx-registry/src/lens.rs`
  - `Registry::measure_ingest_microbatch`
- `crates/calyx-registry/tests/ph57_ingest_microbatch_fsv.rs`
  - aiwonder evidence generator for deterministic JSON and Prometheus readback

## FSV Source Of Truth

Evidence root:

```text
/home/croyse/calyx/data/fsv-issue590-ingest-microbatch-<timestamp>
```

Physical artifacts:

- `ph57-ingest-microbatch-readback.json`
- `ph57-ingest-microbatch.prom`

The JSON records before/during/after controller stats for:

- happy path admission: two inputs with hand-computed bytes
- empty batch at zero cap
- exact-cap admission
- over-cap rejection with exact `CALYX_BACKPRESSURE`
- sustained panel ingest with one stalled lens and one healthy lens

The Prometheus artifact exposes the same counters/gauges as text bytes so the
readback can prove:

- current buffer bytes return to zero after every admitted microbatch
- high-water never exceeds cap
- backpressure increments on over-cap admission
- lens timeout and breaker trip counters increment
- good-lens outputs and acknowledged input count continue after the stalled
  lens opens its breaker
