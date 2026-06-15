# PH45 · T04 — FrozenLensGuard (hash-stable invariant)

| Field | Value |
|---|---|
| **Phase** | PH45 — Mistake-Closure + Online Heads + Replay Buffer |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/learn/frozen_guard.rs` (≤500) |
| **Depends on** | — (independent; used by T03 and T06 to enforce A4) |
| **Axioms** | A4, A15 |
| **PRD** | `dbprdplans/12 §3` |

## Goal

Implement `FrozenLensGuard`: a pre/post-check that computes the SHA-256 of every
registered frozen lens's weight bytes before and after any Anneal action, and
raises `CALYX_LENS_FROZEN_VIOLATION` if any hash changes. This is the mechanical
enforcement of A4 ("frozen lenses never change") within the Anneal path. The
guard is called by `OnlineHeadState::update` (T03) and the phase integration
test (T06), and can be invoked standalone for auditing.

## Build (checklist of concrete, code-level steps)

- [ ] `struct FrozenLensGuard { registry: Arc<LensRegistry>, known_hashes: HashMap<LensId, [u8;32]> }` — `LensRegistry` from `calyx-registry` (PH18); `known_hashes` populated on `initialize()`.
- [ ] `fn initialize(&mut self) -> Result<(), CalyxError>` — iterates all lenses in `LensRegistry` where `lens.contract.frozen = true`; computes SHA-256 of `lens.weights_bytes()`; stores in `known_hashes`.
- [ ] `fn check(&self) -> Result<FrozenCheckReport, CalyxError>` — re-computes hashes for all frozen lenses; compares to `known_hashes`; returns `FrozenCheckReport { violations: Vec<LensId>, ok: Vec<LensId> }`.
- [ ] `fn assert_no_violation(&self) -> Result<(), CalyxError>` — calls `check()`; if `violations` is non-empty, returns `CALYX_LENS_FROZEN_VIOLATION` with the list of violated lens IDs.
- [ ] `fn report(&self) -> Vec<(LensId, [u8;32], bool)>` — returns `(lens_id, hash, is_stable)` for each frozen lens; used by `calyx anneal frozen-guard-report`.
- [ ] `fn weights_bytes(lens: &Lens) -> Vec<u8>` — serializes the full weight tensor (not just metadata); uses the lens's canonical serialization (same as PH18's content-addressed `LensId` computation).
- [ ] Guard must be called at the start AND end of every Anneal update path; a violation mid-path is a code bug (not user data error) and should panic with a diagnostic message in debug builds, return error in release.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: create two frozen lenses `L1`, `L2`; `initialize()`; call `check()` → `violations=[], ok=[L1,L2]`.
- [ ] unit: after `initialize()`, mutate one byte of `L1.weights_bytes()` in a test double; call `check()` → `violations=[L1]`, `ok=[L2]`.
- [ ] proptest: any set of frozen lenses with unchanged weights → `check()` always returns empty violations.
- [ ] edge: zero frozen lenses → `check()` returns empty report (no panic); lens not in `known_hashes` (added after `initialize()`) → treated as a new lens, NOT a violation; SHA-256 collision probability negligible but handled by: if computed hash != known, always report violation.
- [ ] fail-closed: `LensRegistry` unavailable → `CALYX_REGISTRY_UNAVAILABLE`; `weights_bytes()` returns empty vec for a lens with no weights → stable hash of empty bytes (not a violation unless it changes).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `known_hashes` map + re-computed hashes from live lens weight bytes.
- **Readback:** `calyx anneal frozen-guard-report` — prints `(lens_id, hash_hex, stable: true/false)` for all frozen lenses.
- **Prove:** run `frozen-guard-report` before the PH45 T06 integration test; run the full mistake-closure loop; run `frozen-guard-report` again; all hashes identical (all `stable=true`). The before/after hash hexes must appear in the PH45 GitHub issue evidence.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH45 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
