# Phase 1 (Stage 0 — Foundation) — Task Cards

Every actionable thing that must be done in the **first phase of implementation**
(Stage 0: `../10_STAGE0_FOUNDATION.md`, PH00–PH04). One `.md` per task. These
are the immediate, executable backlog — pick them up in order; the dependency
column shows what can run in parallel.

**All work runs on aiwonder** (`croyse@aiwonder.mst.com` over the Cisco VPN;
connection in `../../../.env`, procedure in `../01_AIWONDER_ENVIRONMENT.md`).
Nothing is "done" until its **FSV gate** is proven by reading bytes on aiwonder
(`../02_WORKING_AGREEMENT.md §2`).

---

## Task list

| ID | Title | Phase | Dep | Sudo? |
|---|---|---|---|---|
| [T-001](T-001-verify-access-baseline.md) | Verify aiwonder access + record system baseline | PH00 | — | no |
| [T-002](T-002-provision-calyx-home.md) | Provision the self-contained Calyx home (+ ZFS datasets) | PH00 | T-001 | ZFS step yes (operator) |
| [T-003](T-003-rust-toolchain-env.md) | Configure Rust toolchain + `env.sh` (reuse rustup, isolate target) | PH00 | T-002 | no |
| [T-004](T-004-userspace-build-deps.md) | Install userspace build deps (cmake, protoc) | PH00 | T-002 | no |
| [T-005](T-005-cuda-smoke-test.md) | CUDA 13.2 / sm_120 GPU build smoke test | PH00 | T-003,T-004 | no |
| [T-006](T-006-git-github-repo.md) | Create git repo + push + GitHub repo + labels | PH02 | T-002 | no |
| [T-007](T-007-context-issues.md) | Create the five pinned `type:context` issues | PH02 | T-006 | no |
| [T-008](T-008-cargo-workspace.md) | Scaffold the cargo workspace + crate skeletons | PH01 | T-003 | no |
| [T-009](T-009-linecount-gate.md) | Line-count gate + `check.sh` wrapper | PH01 | T-008 | no |
| [T-010](T-010-core-ids.md) | calyx-core: IDs + content-addressing | PH03 | T-008 | no |
| [T-011](T-011-core-enums.md) | calyx-core: enums | PH03 | T-008 | no |
| [T-012](T-012-core-errors.md) | calyx-core: error catalog (`CALYX_*`) | PH03 | T-008 | no |
| [T-013](T-013-core-structs.md) | calyx-core: core structs (Constellation/Slot/…) | PH04 | T-010,T-011,T-012 | no |
| [T-014](T-014-core-traits.md) | calyx-core: traits + Clock | PH04 | T-013 | no |
| [T-015](T-015-test-scaffolding.md) | Test scaffolding (seeded RNG, injected Clock, proptest) | PH03 | T-008 | no |
| [T-016](T-016-fsv-readback-tool.md) | FSV readback tool skeleton + Synapse note | PH04 | T-008 | no |
| [T-017](T-017-secrets-wiring.md) | Secrets wiring on aiwonder (Infisical HF token) | PH00 | T-002 | no |
| [T-018](T-018-phase0-signoff.md) | Phase-0 exit-gate FSV checklist + sign-off issue | PH04 | all above | no |

## Critical path
`T-001 → T-002 → T-003 → T-008 → {T-009, T-010, T-011, T-012, T-015, T-016} →
T-013 → T-014 → T-018`. T-004/T-005 (build deps + GPU smoke) and T-006/T-007
(repo + issues) and T-017 (secrets) run in parallel once T-002 is done.

## Conventions
- Each card: **Objective · Preconditions · Steps (runnable on aiwonder) ·
  Deliverables · FSV gate · Done · Refs.**
- Commands assume `source /home/croyse/calyx/repo/env.sh` first (T-003 creates
  it). Until then, `source ~/.cargo/env`.
- Open one GitHub `type:task` issue per card once T-006/T-007 exist; link the
  card; close with FSV evidence.
- ≤500-line rule applies to all `.rs` from the first file (T-009 enforces it).
