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

- [ ] `struct QuotaConfig { max_ingest_cx_per_sec: u32, max_query_per_sec: u32, max_io_bytes_per_sec: u64 }` —
  `Default` impl sets generous but finite values (1000 CX/s, 500 q/s, 256 MiB/s).
- [ ] `struct QuotaCounters { ingest_cx: AtomicU64, query: AtomicU64, io_bytes: AtomicU64, window_start_ns: AtomicU64 }` —
  sliding 1-second window; all `Relaxed` loads except window_start uses `AcqRel`.
- [ ] `struct QuotaGuard { vault_id: VaultId, config: QuotaConfig, counters: Arc<QuotaCounters> }`.
- [ ] `impl QuotaGuard { pub fn new(vault_id: VaultId, config: QuotaConfig) -> Self }`.
- [ ] `pub fn charge_ingest(&self, cx_count: u32, now_ns: u64) -> Result<()>` —
  advances window if `now_ns - window_start > 1_000_000_000`; adds `cx_count` to
  `ingest_cx`; if total exceeds `max_ingest_cx_per_sec` returns
  `CALYX_QUOTA_EXCEEDED` (backpressure: caller must retry after the window expires,
  not silently drop, A16).
- [ ] `pub fn charge_query(&self, count: u32, now_ns: u64) -> Result<()>` — same
  pattern for `query` counter.
- [ ] `pub fn charge_io(&self, bytes: u64, now_ns: u64) -> Result<()>` — same for
  `io_bytes`.
- [ ] `pub fn update_config(&self, config: QuotaConfig)` — replaces config; takes
  `&self` (interior mutability via `Mutex<QuotaConfig>` for the config field);
  effective on next window.
- [ ] Add `CALYX_QUOTA_EXCEEDED` to `calyx-core/src/error.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `charge_ingest(500, T)` under limit → `Ok`; `charge_ingest(600, T)`
  (cumulative 1100) → `CALYX_QUOTA_EXCEEDED`.
- [ ] unit: new window (now_ns = T + 1_000_000_001) → counters reset; previously
  over-limit `charge_ingest(500, T+1e9+1)` → `Ok`.
- [ ] unit: `charge_io(256 * 1024 * 1024 + 1, T)` on default config → `CALYX_QUOTA_EXCEEDED`.
- [ ] proptest: `∀ sequences of (cx_count, now_ns)` with injected clock: total accepted
  ≤ `max_ingest_cx_per_sec` per 1-second window (property: quota is never exceeded
  silently).
- [ ] edge (≥3): `cx_count = 0` → always `Ok` (zero charge never exceeds quota);
  window boundary at exactly `1_000_000_000 ns` → resets (not off-by-one); config
  update mid-window → new limit applies on next window not current.
- [ ] fail-closed: after `CALYX_QUOTA_EXCEEDED`, subsequent same-window calls also
  fail, not silently pass.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** compiled test binary with a synthetic `QuotaGuard` using a seeded injected
  clock.
- **Readback:** `cargo test -p calyx-aster quota -- --nocapture 2>&1` prints
  `charge_ingest(600) = Err(CALYX_QUOTA_EXCEEDED)` and `charge_ingest(500) = Ok(())`
  after window reset; assert printed values match known thresholds.
- **Prove:** before: no quota type; after: charge over limit → structured error;
  window reset → counter starts fresh; `update_config` to lower limit takes effect
  on next window.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH60 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
