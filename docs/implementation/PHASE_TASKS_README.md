# Phase Task Cards — Convention & Template

This file defines **how the per-phase task subdirectories are organized** and the
**exact template** every atomic task card uses. It is the single source of truth
for the structure under `docs/implementation/PHnn-*/`.

> Stage 0 (PH00–PH04: `calyx-core` + Aster scaffolding) is **already built** and
> is intentionally **not** covered here. These cards cover **PH05 → PH72**.
>
> **Current (2026-06-10):** Stages 1–5 (PH05–PH30) are **DONE** and
> FSV-signed-off on aiwonder, including post-sweep hardening through #340 and
> Registry/Sextant integration #339. Post-sweep PH06 slot-column SoA hardening
> is FSV-backed at
> `/home/croyse/calyx/data/fsv-issue341-slot-column-soa-20260609-b960c58`, and
> Stage 1-5 future seams are mapped to concrete phase/card owners in
> `STAGE1_5_EVIDENCE_MANIFEST.md`.
> Active implementation is tracked in GitHub issues: Stage 6 Lodestar is closed
> through #240 plus readiness follow-ups #331-#332. Stage 7 Ledger is closed
> through #256 after PH35 #242-#248, PH35 failure-atomicity hardening #345,
> PH36 #249-#255, and the Stage 7 exit rollup. Stage 8 Ward is closed through
> #280 after #258-#274, #275/#276/#277/#278/#279, #349, #350, #351, #352, #353,
> #354, #355, #356, #357, #358, and #359. The #280 exit root is
> `/home/croyse/calyx/data/fsv-issue280-stage8-exit-20260609-477d4a4`.
> Stage 9 / PH40 is complete and FSV-backed: #373 TemporalPolicy manifests,
> #374 TimeWindow filtering, #375 `apply_temporal_boost`, #376 causal
> confidence gate, #377 `temporal_search`, #378 temporal-never-dominant proof,
> and #615 AP-60 final-surface hardening. PH41 T01 #379 is complete and
> FSV-backed at
> `/home/croyse/calyx/data/fsv-issue379-dedup-policy-20260610-0083015`;
> PH41 T02 #380 is complete and FSV-backed at
> `/home/croyse/calyx/data/fsv-issue380-dedup-validation-20260610-5af9a20`.
> PH41 T03 #381 is complete and FSV-backed at
> `/home/croyse/calyx/data/fsv-issue381-anchor-conflict-20260610-00c0540`;
> PH41 T04 #382 is complete and FSV-backed; #382 evidence root is
> `/home/croyse/calyx/data/fsv-issue382-ingest-at-20260610-1a0c560`.
> PH41 T05 #383 is complete and FSV-backed at
> `/home/croyse/calyx/data/fsv-issue383-recurrence-series-20260610-bacf9d2`;
> PH41 T06 #384 is complete and FSV-backed at
> `/home/croyse/calyx/data/fsv-issue384-recurrence-signature-20260610-8b0d0bb`;
> post-T06 recurrence fallback hardening #623 is FSV-backed at
> `/home/croyse/calyx/data/fsv-issue623-recurrence-fallback-20260610-1dc61cf`;
> PH41 T07 #385 is FSV-backed at
> `/home/croyse/calyx/data/fsv-issue385-dedup-audit-20260610-cc9f57b`;
> PH41 T08 #386 is FSV-backed at
> `/home/croyse/calyx/data/fsv-issue386-dedup-invariants-20260610-5fdab01`.
> PH41 public recurrence read APIs #578 are FSV-backed at
> `/home/croyse/calyx/data/fsv-issue578-periodic-recall-20260610-240de5a`;
> PH41 recurrence concurrency hardening #621 is FSV-backed at
> `/home/croyse/calyx/data/fsv-issue621-recurrence-concurrency-20260610-b1fdf5d`.
> PH41 WAL recovery/open serialization #624 is FSV-backed at
> `/home/croyse/calyx/data/fsv-issue624-wal-recovery-lock-20260610-1e4b34c`.
> PH41 durable dedup policy validation parity #617 is FSV-backed at
> `/home/croyse/calyx/data/fsv-issue617-dedup-panel-validation-20260610-07884d9`.
> PH41 recurrence WAL-failure error-code contract #622 is FSV-backed at
> `/home/croyse/calyx/data/fsv-issue622-recurrence-wal-failure-20260610-bf0d380`.
> PH41 follow-ups #620/#626/#627/#628 and PH42 readback-surface
> gate #625 are closed and FSV-backed.
> Stage 6 card-level unchecked checklist rows are historical prompts unless
> contradicted by code/readback; the authoritative completion state is #240 and
> the per-card `STATUS: DONE / FSV-signed-off` notes.
> #336 archives PH05-PH30 task cards: their checked boxes are historical
> prompts resolved or superseded by `STAGE1_5_EVIDENCE_MANIFEST.md`; any new
> unchecked PH05-PH30 box must point to an open GitHub issue.

---

## 1. Directory layout

One subdirectory per phase, named `PHnn-<slug>/`, directly in
`docs/implementation/`. They sort after the numbered stage docs (`00`–`30`) and
group naturally by `PH` number. Each phase subdir contains:

```
PHnn-<slug>/
  README.md          # phase overview (objective · deps · deliverables · file plan ·
                     #   FSV exit gate · axioms · risks · current-state notes)
  T01-<slug>.md      # atomic task card 1
  T02-<slug>.md      # atomic task card 2
  …                  # one card per atomic, independently-completable unit of work
```

**Atomic** means: one coherent, testable unit — typically one `≤500-line` module
(or one cohesive feature spanning a couple of small modules) **plus** its tests
**plus** its slice of the phase FSV gate. If a task can't be finished and proven
on its own, split it. When every card in every phase subdir is checked off,
Calyx is built (the PRD `BUILD_DONE` predicate holds, `dbprdplans/19 §5`).

Task counts are typically **4–8 per phase**; complex phases may have more. Order
cards by dependency (T01 first). Cross-phase deps go in the header.

---

## 2. The binding rules every card inherits (do not restate in full per card)

From `DOCTRINE.md` + `02_WORKING_AGREEMENT.md`:

- **FSV is the gate** (DOCTRINE §0): a return value is a claim; the bytes are the
  verdict. Every card's "Done when" requires byte-level readback on **aiwonder**,
  evidence in the phase's GitHub issue. **No green-checkmark harness counts.**
- **≤500 lines** per `.rs` source/test file (hard). Over-limit → modularize first.
- **Fail closed** (A16): every error path returns a structured `CALYX_*` code with
  remediation; never a silent fallback/zero-fill.
- **Build/run/test on aiwonder** (`/home/croyse/calyx`, CUDA 13.2, RTX 5090
  sm_120, resident TEI on :8088/:8089/:8090). This box authors only.
- **Tests support FSV, don't replace it:** FIRST + property/fuzz/mutation; seed
  all RNG; inject the clock (`Clock` trait, never `SystemTime::now()` in logic).
- **No CI** — `cargo check`/`clippy -D warnings`/`test` + line-count gate +
  bit-parity run on aiwonder, agent-invoked, before merge.
- **Provenance always** (A15): every mutation a phase adds writes a Ledger entry
  (a stub until PH35, real after).
- **Strict Royse theory** (A24): intelligence theory only from the corpus;
  external tech (TurboQuant, grouped GEMM, CUDA, ZFS, FoundationDB pattern) is
  engineering scaffolding only.
- **Reuse seeds** (`19 §6`): lift ContextGraph `mincut`/`paths`/`witness`/`mejepa`
  source into the Calyx crates by **copying** into `CALYX_HOME`, never linking
  the live project.

---

## 3. Atomic task card template (copy verbatim, fill every field)

```markdown
# PHnn · Tnn — <atomic task title>

| Field | Value |
|---|---|
| **Phase** | PHnn — <phase name> |
| **Stage** | Sn — <stage name> |
| **Crate** | `calyx-<crate>` |
| **Files** | `crates/calyx-<crate>/src/<path>.rs` (≤500) [, more] |
| **Depends on** | Tnn (this phase) · PHmm (prior phase) |
| **Axioms** | A#, A# |
| **PRD** | `dbprdplans/NN §X` [, …] |

## Goal
<1–3 sentences: the single atomic outcome this card delivers and why.>

## Build (checklist of concrete, code-level steps)
- [ ] <signature / type / behavior 1, e.g. `fn encode(seq,payload)->Vec<u8>`>
- [ ] <step 2 — be specific: data layout, invariant, error code returned>
- [ ] <…>

## Tests (synthetic, deterministic — known input → known bytes/number)
- [ ] unit: <what, with the exact assertion>
- [ ] proptest: <invariant, e.g. `decode(encode(x))==x`>
- [ ] edge (≥3): <empty / max / torn / boundary / conflicting case>
- [ ] fail-closed: <bad input → exact `CALYX_*` code>

## FSV (read the bytes on aiwonder — the truth gate)
- **SoT:** <the exact file / CF row / WAL record / metric / Ledger entry>
- **Readback:** <`calyx readback …` / `xxd` / `zfs list` / `cat metric` cmd>
- **Prove:** <the before→after delta that proves the goal; what must be present
  and what must be absent>

## Done when
- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] [if Forge-touching] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PHnn GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
```

---

## 4. README.md template for each phase subdir

```markdown
# PHnn — <phase title>

**Stage:** Sn — <stage name>  ·  **Crate:** `calyx-<crate>`  ·
**PRD roadmap:** P#  ·  **Axioms:** A#…

## Objective
<the phase's one-paragraph objective, from the stage file>

## Dependencies
- **Phases:** PHmm (<why>), …
- **Provides for:** PHxx, … (what downstream needs this)

## Current state (build off what exists)
<what already exists in the repo for this crate/phase and what remains — for
greenfield stub crates, say "crate is a 9-line stub; greenfield">

## Deliverables (file plan, each ≤500 lines)
| File | Responsibility |
|---|---|
| `src/…rs` | … |

## Tasks (atomic — all must pass for the phase to be DONE)
| Card | Title | Depends |
|---|---|---|
| T01 | … | — |
| T02 | … | T01 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)
<the stage file's FSV gate, restated, with the exact readback that proves it>

## Risks / landmines
- <phase-specific hazard + mitigation, e.g. EXDEV on ZFS rename, VRAM contention>
```

---

## 5. Coverage rule (binding)

The set of task cards for a phase MUST cover **every** item in that phase's stage
file: each **Deliverable**, each **Key task**, and the **FSV gate** must map to at
least one card's Build/Tests/FSV. If the stage file names it, a card owns it.
Nothing in the deliverables is left unaddressed — that is the contract that makes
"if all cards are done, everything is built" true.

When in doubt about a number/threshold/path/error-code, copy it **verbatim** from
the PRD (No-Compress List, DOCTRINE §8b): `0.05 bits`, `0.6 corr`, `≥0.95`,
`≥99%`, `≤1e-3`, `sm_120`, `τ`, `CALYX_*`, file paths, axiom ids.
