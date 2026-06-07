# Calyx — Implementation Plan (master README)

This directory is the **build plan** for Calyx: how the system specified in
`docs/dbprdplans/` (the PRD) and bound by `docs/dbprdplans/DOCTRINE.md` is
actually constructed, phase by phase, **entirely on the aiwonder datacenter
PC**. The PRD says *what* and *why*; this plan says *in what order, on what
machine, proven how*.

> **Read order:** `DOCTRINE.md` → this README → `01_AIWONDER_ENVIRONMENT.md`
> → `02_WORKING_AGREEMENT.md` → `03_PHASE_MAP.md` → the stage files in order.
> Per-phase, do-now atomic task cards live in one subdir per phase
> (`PH05-*/` … `PH72-*/`), governed by `PHASE_TASKS_README.md`. Stage 0
> (PH00–PH04) is built and intentionally has no subdir.

---

## 1. The three non-negotiable framing facts

1. **Everything happens on aiwonder.** This Windows/WSL repo **authors** plan +
   code only. The project is **built, stored, run, and tested on aiwonder**
   (`croyse@aiwonder.mst.com`, over the Cisco VPN). The source-of-truth bytes
   that FSV reads live on aiwonder; a local run never counts (DOCTRINE §0/§8c,
   PRD `28 §5`). Connection + secrets: `../../.env` (gitignored).
2. **Calyx is self-contained on aiwonder.** All Calyx work lives under ONE root
   — `CALYX_HOME=/home/croyse/calyx` (and its dedicated ZFS datasets once
   provisioned). Nothing Calyx touches the existing `leapable`/`contextgraph`
   projects, the PostgreSQL control plane, or any shared dotfile. Build output,
   datasets, vaults, logs, HF cache — all under `CALYX_HOME`.
3. **FSV is the gate.** Every phase below is "done" only when a clause is proven
   by **reading the persisted bytes on aiwonder** (not a return value, not a
   green harness). There is no CI; FSV is CI (PRD `28 §6b`).

## 2. How the plan is organized

| File | Owns |
|---|---|
| `00_README.md` | this — how to use the plan, conventions, the dependency spine |
| `01_AIWONDER_ENVIRONMENT.md` | the **real** box (live readback), the self-contained Calyx layout, toolchain, GPU/CUDA, ZFS, services, secrets, the sudo constraint, the connect procedure |
| `02_WORKING_AGREEMENT.md` | the per-phase discipline: FSV protocol, ≤500-line rule, GitHub issues, test taxonomy, definition-of-done, doctrine compliance checklist |
| `03_PHASE_MAP.md` | the master table of **every** phase (PH00–PH72), its stage, dependencies, PRD/axiom mapping, exit gate, and the critical path |
| `10_STAGE0_FOUNDATION.md` … `30_STAGE20_CRITICAL_CAPS.md` | one file per stage; each details its phases (objective · deps · deliverables · key tasks · FSV exit gate · axioms · risks) |
| `PHASE_TASKS_README.md` | **the per-phase task convention** — directory layout, the atomic task-card template, the README template, the binding rules every card inherits, and the coverage rule. Read before opening any phase subdir. |
| `PH05-*/` … `PH72-*/` | **one subdir per phase** (Stage 1 → Stage 20). Each holds a `README.md` (phase overview) + one `.md` atomic task card per actionable unit (`T01-…`, `T02-…`). When every card in every subdir is done, `BUILD_DONE` holds. Stage 0 (PH00–PH04) is already built and intentionally has no subdir. |

## 3. Numbering

- **Phases** are `PH00`–`PH72`, globally ordered, grouped into **Stages**
  `S0`–`S20`. Phase IDs are stable handles used in GitHub issues and commits.
- Each phase cross-references the PRD's own roadmap phases (`P0`–`P12` and the
  `Pxb` sub-phases in `dbprdplans/19`) and the axioms (`A1`–`A34`) it satisfies.
- Phases are sized so each maps to a small set of ≤500-line crate modules and a
  single FSV exit gate — i.e. a few days of agent work, not months.

## 4. The dependency spine (critical path)

```
S0 Foundation ─▶ S1 Aster ─▶ S2 Forge ─▶ S3 Registry ─▶ S4 Sextant ─▶ S5 Loom/Assay
                                                  │                         │
                                                  ▼                         ▼
                                            (lenses live)            S6 Lodestar ─▶ S8 Ward
                                                                          │
S7 Ledger threads through S1+ (provenance in group-commit) ───────────────┤
                                                                          ▼
S9 Temporal/Dedup ─▶ S10 Anneal+J ─▶ S11 Oracle/AGI                  (kernel + guard)
S12 Universal data layer (parallel to S5–S8, needs S1)
S13 Resource/GC + S14 Security (cross-cutting; harden continuously)
S15 Interfaces (MCP/CLI) usable from S4 onward; S16 Server/Deploy after S8
S17 Scale ▸ S18 Datasets/Intelligence-FSV ▸ S19 Leapable ▸ S20 Critical caps
```

The **recommended first demo** (PRD `19 §2`): `S0 → S1 → S2(CPU) → S3 → S4` +
the migration shadow (`S15`/`S19-V0`) — a Calyx vault answering with multiple
lenses + provenance. That alone justifies the project.

## 5. Engine → crate → stage cheat sheet

| Engine (PRD codename) | Crate | Stage |
|---|---|---|
| Aster (storage) | `calyx-aster` | S1 |
| Forge (GPU/SIMD math) | `calyx-forge` | S2 |
| Registry (lenses) | `calyx-registry` | S3 |
| Sextant (search/nav) | `calyx-sextant` | S4 |
| Loom (DDA) / Assay (bits) | `calyx-loom` / `calyx-assay` | S5 |
| Lodestar (kernel) | `calyx-lodestar` + `calyx-mincut`/`-paths` | S6 |
| Ledger (provenance) | `calyx-ledger` | S7 |
| Ward (Gτ guard) | `calyx-ward` | S8 |
| Temporal/Dedup | (in `aster`/`registry`/`loom`) | S9 |
| Anneal (self-opt) + `J` | `calyx-anneal` | S10 |
| Oracle/AGI | `calyx-oracle` | S11 |
| Universal data layer | `calyx-aster` (layers) + `calyx-sextant` | S12 |
| Resource/GC, Security | cross-cutting | S13, S14 |
| MCP / CLI / Server | `calyx-mcp` / `calyx-cli` / `calyxd` | S15, S16 |

## 6. What "ground truth" already exists on aiwonder (reuse, don't rebuild)

Confirmed live (`01_AIWONDER_ENVIRONMENT.md`): RTX 5090 sm_120 + **CUDA 13.2**
toolkit, **Rust via rustup** (so we build natively — the PRD's "no rustc on
box" note is *superseded*), resident **TEI lenses** on :8088/:8089/:8090,
Prometheus on :9090, Docker, Infisical, HF cache, ZFS hot+cold pools. We lift
the ContextGraph `mincut`/`paths`/`witness`/`mejepa` logic as seeds (PRD
`19 §6`). Missing and to be installed in userspace: `cmake`, `protoc`.

## 7. Status (current: 2026-06-07, commit `8dcddaa`)

**DONE — Stage 0 + Stage 1 (PH00–PH11), FSV-signed-off on aiwonder:**
- **Stage 0** (PH00–PH04): `calyx-core` — IDs, enums, the full `CALYX_*` error
  catalog, the constellation model structs, engine traits, the injected `Clock`.
- **Stage 1** (PH05–PH11): `calyx-aster` storage core — WAL + group-commit,
  memtable + LSM SSTable, column families + key codecs, MVCC snapshots,
  constellation CRUD + idempotent ingest, manifest + crash recovery, compaction
  + hot/cold tiering. Plus `calyx-cli` readback/FSV/crash commands and
  `calyx-testkit`. Proven by byte-level readback on aiwonder (87 `calyx-aster`
  tests + 6 `calyx-cli` tests green; crash-drill recovered to last-acked seq;
  corrupt-shard failed closed). Evidence: GitHub issue #23 (`[CONTEXT] You are
  here`); FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.
  This satisfies PRD `CORE` (`dbprdplans/19 §5`).
- **Stage-1 tracked follow-ups (non-blocking debt):** PH10 recovery diverges
  from its card — `AsterVault::open` replays the full WAL instead of using the
  manifest-anchored `recover_vault` + `set_start_seq`; `manifest/recovery.rs`
  was not split out; `degraded_rebuildable` is never set (deferred to PH44); and
  the durable-write / `CfRouter` / `CompactionScheduler` paths are not yet
  unified. Recorded in `11_STAGE1_ASTER.md` and as GitHub `type:task` issues.

**NEXT — Stage 2: Forge (PH12–PH16)** — the CPU SIMD + CUDA sm_120 math runtime
(`calyx-forge`, currently a 9-line stub). Deps (PH04, PH09) are satisfied;
start at `PH12-cpu-simd-backend/`. See `12_STAGE2_FORGE.md`.

**Remaining:** every engine crate except `calyx-core`/`calyx-aster` is still a
~9-line stub. Phases **PH12–PH72** are laid out as atomic task cards in the
per-phase subdirs (`PH12-*/` … `PH72-*/`), governed by `PHASE_TASKS_README.md`,
each with a byte-level FSV exit gate. Execution proceeds down the dependency
spine (§4); the recommended first demo is `S0 → S1 → S2(CPU) → S3 → S4` + the
migration shadow. Track live state in the `ChrisRoyse/Calyx` GitHub
`type:context` issues (doctrine §8d, PRD `29`).
