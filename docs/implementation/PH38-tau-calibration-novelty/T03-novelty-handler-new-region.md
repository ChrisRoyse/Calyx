# PH38 · T03 — `NoveltyHandler` — `NewRegion` / `Quarantine` / `RejectClosed` routing

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/novelty.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH37 T03 · PH09 (vault CF write — stub until live) |
| **Axioms** | A12, A2 |
| **PRD** | `dbprdplans/09 §1`, `09 §5` |

## Goal

When `guard()` returns `overall_pass = false`, the `NoveltyHandler` routes the
failing produced slots to the correct outcome per `GuardProfile.novelty_action`:
`NewRegion` stores the candidate as a novel constellation awaiting grounding;
`Quarantine` holds it for human/agent review without serving it as trusted;
`RejectClosed` refuses immediately. A FAIL is not an error — it is continual
learning (the video's "new dot → whole new constellation"). Novelty is never a
silent accept.

## Build (checklist of concrete, code-level steps)

- [ ] Define `NoveltyRecord` struct:
      `novel_id: NovelId` (new UUID), `guard_id: GuardId`,
      `produced_slots: ProducedSlots`, `failing_verdicts: Vec<SlotVerdict>`,
      `action_taken: NoveltyAction`, `ts: i64`, `status: NoveltyStatus`
- [ ] Define `NoveltyStatus` enum: `AwaitingGrounding | Quarantined | Rejected`
- [ ] Define `NovelId` newtype wrapping `uuid::Uuid`
- [ ] Define `VaultSink` trait (sync, object-safe):
      `fn write_novel(&self, record: &NoveltyRecord) -> Result<(), WardError>`
      — the real impl writes to the vault's `novel_regions` CF (PH09); the
      test impl writes to an in-memory `Vec<NoveltyRecord>`
- [ ] Implement `NoveltyHandler`:
      ```
      struct NoveltyHandler { vault: Arc<dyn VaultSink>, clock: Arc<dyn Clock> }
      fn handle(&self, profile: &GuardProfile, verdict: &GuardVerdict,
                produced: &ProducedSlots) -> Result<NoveltyRecord, WardError>
      ```
      - Check `verdict.overall_pass == true` → `Err(WardError::NotAFailure)` (misuse)
      - Match `profile.novelty_action`:
        - `NewRegion` → build `NoveltyRecord` with `status: AwaitingGrounding`;
          call `vault.write_novel(&record)`; return `Ok(record)`
        - `Quarantine` → build with `status: Quarantined`; write; return `Ok`
        - `RejectClosed` → build with `status: Rejected`; write tombstone
          (write then return `Err(WardError::Ood { .. })`); fail closed
- [ ] `novel_regions(vault, since_ts) -> Vec<NoveltyRecord>` — query the CF for
      records with `ts ≥ since_ts` and `status: AwaitingGrounding`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `NewRegion` action → `NoveltyRecord` written to in-memory sink;
      `status == AwaitingGrounding`; `novel_id` is a valid UUID; `action_taken
      == NewRegion`
- [ ] unit: `Quarantine` action → `status == Quarantined`; record written; not
      served as trusted (no `Ok(GuardVerdict { overall_pass: true })`)
- [ ] unit: `RejectClosed` action → `status == Rejected`; `Err(WardError::Ood)`
      returned after tombstone write; tombstone in sink
- [ ] proptest: for any `NoveltyAction`, `handle()` always writes exactly one
      `NoveltyRecord` to the sink (no duplicate writes)
- [ ] edge: `vault.write_novel()` returns an error → `RejectClosed` still
      returns the error; `NewRegion` propagates the vault error (not swallowed)
- [ ] edge: `novel_regions(since=i64::MAX)` → empty vec; no panic
- [ ] fail-closed: calling `handle()` on a passing verdict (`overall_pass=true`)
      → `Err(WardError::NotAFailure)` — misuse guard

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root containing `NoveltyRecord` JSON for
  `AwaitingGrounding`, `Quarantined`, and `Rejected`, plus tombstone/error JSON
  and a SHA-256 manifest.
- **Readback:** run the manual FSV fixture with `CALYX_WARD_NOVELTY_FSV_DIR=$root`,
  then separately inspect the JSON files with `xxd`, `sha256sum`, and parsed
  JSON.
- **Prove:** durable readback contains all three status variants; `novel_id` is
  a UUID string; `RejectClosed` records `WardError::Ood` alongside the
  tombstone; `NewRegion` records `status: AwaitingGrounding`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH38 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
