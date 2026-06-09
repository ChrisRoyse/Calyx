# PH37 · T04 — `guard()` `KofN` policy + `CALYX_GUARD_OOD` fail-closed

| Field | Value |
|---|---|
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/guard.rs` (≤500) |
| **Depends on** | T03 (this phase) |
| **Axioms** | A12, A16 |
| **PRD** | `dbprdplans/09 §4` |

## Goal

Extend `guard()` with `KofN { k }` policy: the constellation passes only if at
least `k` of the required slots individually pass their `τ`. When `k >
required_slots.len()`, fail closed with `CALYX_GUARD_POLICY_VIOLATION` (not
a panic). All-slot per-slot verdicts are still returned in every case.

## Post-implementation note

Implemented in `crates/calyx-ward/src/guard.rs` and
`crates/calyx-ward/tests/guard_kofn.rs` in commit
`bd35e1e40a02b14ff20d9b287620d6e1aa761207`. `guard()` now supports
`GuardPolicy::KofN { k }`, returns `WardError::PolicyViolation` when `k` exceeds
the unique required-slot count, and keeps full per-slot verdicts for pass/fail
policy outcomes. `guard_result()` is exported from `calyx-ward` and wraps any
non-passing verdict in `WardError::Ood` with the failing slot details.

aiwonder FSV root:
`/home/croyse/calyx/data/fsv-issue261-ph37-t04-20260609-bd35e1e`. Readback
artifacts include `kofn-k2-pass-verdict.json`, `kofn-k3-fail-verdict.json`,
`kofn-k0-pass-verdict.json`, `guard-result-ood-error.json`,
`policy-violation-error.json`, and `kofn-fsv.log`. Separate readback used
`xxd`, `sha256sum`, parsed JSON, and source inspection of
`crates/calyx-ward/src/guard.rs` and `crates/calyx-ward/tests/guard_kofn.rs`.

## Build (checklist of concrete, code-level steps)

- [ ] In `guard()`, after computing per-slot verdicts, branch on
      `profile.policy`:
      - `AllRequired` (already in T03): `pass_count == required_slots.len()`
      - `KofN { k }`:
        - Guard: if `k > profile.required_slots.len()` →
          `return Err(WardError::PolicyViolation { k, n_required:
          profile.required_slots.len() })`
        - `overall_pass = pass_count >= k`
- [ ] `pass_count: usize` computed as `per_slot.iter().filter(|v| v.pass).count()`
      before the policy branch (shared for both policies)
- [ ] `action` set to `Some(profile.novelty_action.clone())` when
      `!overall_pass`; `None` when pass (same as T03)
- [ ] Add integration helper `guard_result(profile, produced, matched) ->
      Result<GuardVerdict, WardError>`: same slot math as `guard()`, but wraps
      a non-passing verdict into `Err(WardError::Ood { guard_id, failing })`
      for ergonomic `?` propagation. Callers that need the complete
      pass-and-fail breakdown call `guard()` directly; `WardError::Ood`
      carries the failing slots only.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: three required slots, τ=0.7 each; cos scores `[0.8, 0.4, 0.9]`;
      `KofN { k: 2 }` → `overall_pass = true` (2 of 3 pass); verify
      `failing_slots().len() == 1`
- [ ] unit: same setup, `KofN { k: 3 }` → `overall_pass = false`
- [ ] unit: `KofN { k: 1 }` with all slots failing → `overall_pass = false`
- [ ] unit: `KofN { k: 0 }` with all slots failing → `overall_pass = true`
      (0-of-N is trivially satisfied)
- [ ] edge: `KofN { k: 4 }` with 3 required slots → `WardError::PolicyViolation`
      returned; no panic
- [ ] edge: `KofN { k: 1 }` with a single required slot at exactly τ (boundary)
      → `overall_pass = true`
- [ ] fail-closed: `PolicyViolation` display contains
      `"CALYX_GUARD_POLICY_VIOLATION"`, `k=4`, `n_required=3`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root containing KofN verdict JSON,
  policy-violation error JSON/log, and a SHA-256 manifest.
- **Readback:** run the manual FSV fixture with `CALYX_WARD_KOFN_FSV_DIR=$root`,
  then separately inspect the files with `xxd`, `sha256sum`, and JSON parsing.
- **Prove:** readback shows `k=2, pass_count=2 -> overall_pass=true`;
  `k=3, pass_count=2 -> overall_pass=false`; `k=4, n=3` produces
  `CALYX_GUARD_POLICY_VIOLATION`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
