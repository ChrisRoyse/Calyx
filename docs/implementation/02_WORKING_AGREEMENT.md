# 02 — Working Agreement (per-phase discipline)

The rules every phase obeys. Derived from `DOCTRINE.md`, PRD `19`/`28`/`29`.
A phase is not "done" until it passes **all** of this.

---

## 1. Definition of Done (per phase)

A phase `PHnn` is DONE iff:
1. **Code compiles & lints clean on aiwonder:** `cargo check` + `cargo clippy
   -D warnings` + `cargo test` green (run on aiwonder, the fast inner loop).
2. **File-size gate passes:** no `.rs` source/test file > **500 lines** (the
   gate script, §5). Over-limit → a `type:task` issue + modularize first.
3. **CPU↔GPU bit-parity** holds for any Forge-touching code (≤1e-3 rel tol).
4. **FSV exit gate met:** the phase's specific gate (in its stage file) is
   proven by **reading the persisted bytes on aiwonder** — not a return value,
   not a harness. Evidence (readback output / screenshots) attached to the
   phase's GitHub issue.
5. **Provenance + fail-closed** wired where the phase touches data: Ledger entry
   on every mutation it adds; every error path returns a structured `CALYX_*`
   code, never a silent fallback (A16).
6. **Context issues updated:** `[CONTEXT] You are here` reflects the new state;
   landmines/datasets updated; done task closed with FSV evidence (PRD `29`).

## 2. FSV protocol (binding — DOCTRINE §0)

> A return value is a claim. The source of truth is the bytes. Read the bytes.

Five steps, perceived via **Synapse** (PRD `28 §2c`) when driving on the box:
1. **Identify** the bytes that prove the claim (Aster CF rows, WAL, Ledger
   entry, ZFS file, the metric).
2. **Read them before** the action (`reality_baseline` + a readback cmd:
   `calyx readback` / `xxd` / `zfs list` / `cat metric`).
3. **Execute** the action.
4. **Read them after** (`observe_delta`/`reality_audit` + `read_text`/`find`).
5. **Inspect the delta**; record evidence in the GitHub issue.

**FSV harnesses are banned and cannot satisfy FSV.** Calyx may ship *readback
tools* that print bytes for a human/agent to judge — never a green-checkmark
harness. Two kinds of test data (PRD `28 §1`): **synthetic deterministic** for
mechanics (known input→known bytes), **real datasets** for intelligence claims
(recall/bits/kernel/oracle/J). Mechanics FSV'd on synthetic; intelligence FSV'd
on real; both read persisted state.

## 3. Tests support FSV; they don't replace it (PRD `28 §6c`)

Every test must pass the two questions: **fails when the code is wrong, passes
when right.** FIRST + properties: Fast (<100 ms unit), Independent/parallel-
safe, Repeatable (**seed all RNG**, **inject the clock** — never
`SystemTime::now()` in logic), Self-validating (`assert!`), Behavior-not-
implementation (test the public API + persisted bytes). Tooling (all free OSS,
A34): `#[cfg(test)]` unit, **proptest** (round-trips/invariants), **cargo-fuzz**
(parser/wire boundaries), **cargo-mutants** (proves tests assert — on diff),
`tests/` integration against a real Aster vault, **criterion** (perf budgets).
Refuse: assertion roulette, `sleep()`-to-wait, order-dependent/shared-state
tests, over-mocking, lingering `#[ignore]`. **Zero tolerance for flakiness**
(usually a real race). Bug → failing regression test → fix → keep it.

## 4. No CI — FSV is our CI (PRD `28 §6b`, A34)

No hosted pipeline (slow, costs money). The per-merge checks run **on aiwonder,
agent-invoked**: `cargo check`/`test`/`clippy -D warnings`, the ≤500-line gate,
bit-parity. A passing test is a *claim*; FSV (byte readback) is the *truth
gate*. Everything free/OSS; storage POSIX-on-ZFS; no paid services/scanners.

The canonical per-merge command is:
```bash
cd /home/croyse/calyx/repo
source ./env.sh
bash scripts/check.sh
```

## 5. The ≤500-line gate (DOCTRINE §8)

Every `.rs` source/test file ≤ 500 lines (docs unlimited). Modularize per
`docs2/modulateprompt.md`: SRP module dirs, `mod.rs` facade with explicit
`pub use` (no wildcard), no circular deps, identical public API, tests green.
Over-limit file → open a `type:task` issue, split, re-run the gate.
```bash
# run on aiwonder before every merge
bash scripts/linecount.sh
```

## 6. Code reuse (lift the proven seeds — PRD `19 §6`)

Calyx is the *unification and hardening* of code that already works. Lift
ContextGraph `mincut`/`paths`/`solver`/`witness` and the `mejepa`
Assay/kernel/guard logic as seeds of `calyx-mincut`/`-lodestar`/`-assay`/
`-ledger`/`-ward` — by **copying source into the Calyx crates** under
`CALYX_HOME` (never linking the live project). Reuse OSS crates/ideas freely
(they're free); the load-bearing engine is hand-built in Rust (A13/A34).

## 7. Dev-state on GitHub Issues (DOCTRINE §8d, PRD `29`)

Repo **`chrisroyse/calyx-dev`**. Pinned `type:context` issues (Mission &
invariants · You-are-here · Environment & ops · Landmines · Datasets) read at
the start of every turn; kept current by **editing to truth (never appending
contradictions)**; **pruned every phase**. One `type:task` per phase (and per
modularization). Decisions = ADR issues; gotchas = discovery issues. Dedupe
before every create. Read-state queries in PRD `29 §3`.

## 8. Orchestration (DOCTRINE §8e, PRD `31`)

Substantive build work is driven on aiwonder via **Synapse** computer-use —
open real terminals, run `cargo`/`calyx`, **command Claude (`cldy`)/Codex
(`codex --yolo`) worker agents** (preferred over the subagent tool; each worker
gets full Synapse + real FSV). Web/dashboards: a **new tab in the one main
Chrome** (auto-authed). Screenshot+AI-vision is a primary perception mode for
Grafana/`J`-curve/GUI/error state. Humans direct + approve outward/destructive
actions only.

## 9. Per-phase doctrine compliance checklist

Before closing any phase, confirm it does **not** violate an anti-pattern
(DOCTRINE §9): no panel flattening (A3); no `C(N,2)` sold past the DPI ceiling
(A8); nothing labeled "trusted" without grounding (A2 → say `provisional`); no
frozen-lens mutation / synthetic-generator training; no external intelligence
theory (A24); lens plug-in / bits / kernel never made harder (§5); no harness
standing in for FSV (§0); no >500-line file without a tracking issue; no bolt-on
search/graph/vector DB instead of the Association Engine (A19/§3); right-to-
erasure never refused citing A25 (A33).

## 10. Branch/commit/PR hygiene
- One change at a time; plan before architectural/storage/security changes;
  document failure as carefully as success.
- Branch per phase (`ph05-wal`), small commits, FSV evidence in the PR/issue.
- Never `--no-verify`; never bypass the gates.
- Commit trailer: `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
