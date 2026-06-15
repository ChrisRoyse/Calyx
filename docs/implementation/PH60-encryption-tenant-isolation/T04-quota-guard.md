# PH60 · T04 — `QuotaGuard`: per-tenant counters + backpressure + `CALYX_QUOTA_EXCEEDED`

| Field | Value |
|---|---|
| **Phase** | PH60 — Encryption at rest/in transit + tenant isolation |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault/quota.rs` (≤500) |
| **Depends on** | T02 (`KeyspaceGuard`) · PH09 (VaultId) |
| **Axioms** | A33, A16, A26 |
| **PRD** | `dbprdplans/30 §3` (per-tenant quotas — noisy neighbor); `dbprdplans/30 §1` (DoS axis) |

## Goal

Implement per-vault resource quota tracking so that a heavy tenant cannot starve
others (noisy-neighbor defense, `30 §3`). Quotas cover ingest rate (CXs/s), query
rate (queries/s), and IO budget (bytes/s). When a quota is exceeded the operation
is denied with `CALYX_QUOTA_EXCEEDED` and backpressure is applied, consistent with
the bounded queues + backpressure principle in the DoS row of the STRIDE model
(`30 §1`). Quotas are configured per vault via `QuotaConfig` and can be updated at
runtime without restart.

## Build (checklist of concrete, code-level steps)

- [x] `struct QuotaConfig { max_ingest_cx_per_sec: u32, max_query_per_sec: u32, max_io_bytes_per_sec: u64 }` —
  `Default` impl sets generous but finite values (1000 CX/s, 500 q/s, 256 MiB/s).
- [x] `struct Window { start_ns: u64, ingest_cx: u64, query: u64, io_bytes: u64, active: QuotaConfig, pending: QuotaConfig }` —
  the fixed 1-second window state is guarded as one indivisible unit.
- [x] `struct QuotaGuard { vault_id: VaultId, window: Mutex<Window> }` — one
  per-vault lock makes check-reset-charge atomic. Do not split the counters into
  independent atomics; reset plus charge must be one critical section or
  concurrent resets can clobber increments and silently over-admit.
- [x] `impl QuotaGuard { pub fn new(vault_id: VaultId, config: QuotaConfig) -> Self }`.
- [x] `pub fn charge_ingest(&self, cx_count: u32, now_ns: u64) -> Result<()>` —
  locks the window; advances when `now_ns.saturating_sub(start_ns) >= 1_000_000_000`;
  stages `pending` config into `active`; then add-then-checks `ingest_cx`. If the
  new total exceeds `max_ingest_cx_per_sec`, return `CALYX_QUOTA_EXCEEDED`
  (backpressure: caller must retry after the window expires, not silently drop,
  A16). The tripping charge remains counted so the same window stays fail-closed.
- [x] `pub fn charge_query(&self, count: u32, now_ns: u64) -> Result<()>` — same
  locked add-then-check path for `query`.
- [x] `pub fn charge_io(&self, bytes: u64, now_ns: u64) -> Result<()>` — same
  locked add-then-check path for `io_bytes`.
- [x] `pub fn update_config(&self, config: QuotaConfig)` — stages `pending`
  config under the same window lock; the current window keeps a stable `active`
  budget and the new config applies on the next rollover.
- [x] Add module-local `CALYX_QUOTA_EXCEEDED` in
  `crates/calyx-aster/src/vault/quota.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `charge_ingest(500, T)` under limit → `Ok`; `charge_ingest(600, T)`
  (cumulative 1100) → `CALYX_QUOTA_EXCEEDED`.
- [x] unit: new window (now_ns = T + 1_000_000_001) → counters reset; previously
  over-limit `charge_ingest(500, T+1e9+1)` → `Ok`.
- [x] unit: `charge_io(256 * 1024 * 1024 + 1, T)` on default config → `CALYX_QUOTA_EXCEEDED`.
- [x] proptest: `∀ sequences of (cx_count, now_ns)` with injected clock: total accepted
  ≤ `max_ingest_cx_per_sec` per 1-second window (property: quota is never exceeded
  silently).
- [x] concurrency: 8 threads × 50 unit charges against a 100-CX window admits
  exactly 100; all 400 attempted charges remain visible in the counter so rejected
  load cannot disappear.
- [x] edge (≥3): `cx_count = 0` → always `Ok` (zero charge never exceeds quota);
  window boundary at exactly `1_000_000_000 ns` → resets (not off-by-one); config
  update mid-window → new limit applies on next window not current.
- [x] fail-closed: after `CALYX_QUOTA_EXCEEDED`, subsequent same-window calls also
  fail, not silently pass.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the live `QuotaGuard` window in `calyx-aster`, read independently via
  `counters()` after each charge plus the on-disk source bytes under
  `/home/croyse/calyx/repo/crates/calyx-aster/src/vault/quota.rs`.
- **Readback:** `cargo test -p calyx-aster quota -- --nocapture 2>&1` on aiwonder
  must print the counter state after the synthetic triggers:
  `charge_ingest(600) = Err(CALYX_QUOTA_EXCEEDED); counters = (1100, 0, 0)`,
  rollover counter `(500, 0, 0)`, `charge_io(256MiB+1) = Err(CALYX_QUOTA_EXCEEDED)`,
  and concurrency admission `admitted under concurrency = 100 (limit 100);
  counter = (400, 0, 0)`.
- **Prove:** before/after counter readbacks show the happy path, exact
  `1_000_000_000 ns` rollover boundary, zero-charge edge, mid-window config
  staging, fail-closed same-window rejection, and concurrent non-over-admission.
  The doc-source readback must show `Mutex<Window>` and `>= 1_000_000_000`, with no
  stale lock-free quota design remaining in this card.
- **Reference evidence:** #498 implemented and FSV-proved the corrected design;
  #703 exists to keep the PH60 T04 card from reintroducing the stale atomic race.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder for the
  quota implementation (#498); rerun the quota tests when closing #703.
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
