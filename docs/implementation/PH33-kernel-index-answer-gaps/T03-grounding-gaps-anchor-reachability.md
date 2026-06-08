# PH33 · T03 — `grounding_gaps`: anchor-reachability BFS + gap list

| Field | Value |
|---|---|
| **Phase** | PH33 — Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/grounding_gaps.rs` (≤500) |
| **Depends on** | T01 (kernel members available), PH31-T01 (BFS traversal), PH09 (Anchor types) |
| **Axioms** | A10, A11 |
| **PRD** | `dbprdplans/08 §3` (Stage 4: Anchor check), `08 §6` (`unanchored_members`), `08 §7` (honesty) |

## Goal

Implement `grounding_gaps`: for each kernel member, BFS over the association graph
to find whether it can reach any `Anchor` node within `max_anchor_dist` hops. Members
that cannot reach an anchor are "grounding gaps" — the cheapest grounding plan
(per `08 §6`: "names exactly which constellations need a real outcome label to fully
ground the domain"). The function also computes `grounded_fraction` and emits
`CALYX_KERNEL_UNGROUNDED` when the kernel is fully ungrounded.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn grounding_gaps(kernel: &Kernel, graph: &AssocGraph, anchors: &[CxId], max_anchor_dist: usize) -> Result<GroundingGapReport>`.
- [x] `pub struct GroundingGapReport { gaps: Vec<CxId>, grounded_fraction: f32, grounded_count: usize, member_count: usize, max_anchor_dist: usize, warning: Option<String> }`.
- [x] For each `cx_id` in `kernel.members`: BFS from `cx_id` in `graph`; if any
  node within `max_anchor_dist` hops is in `anchors` → grounded; else → gap.
- [x] `grounded_fraction = (members.len() - gaps.len()) / members.len()`;
  empty kernel → `grounded_fraction = 1.0` (vacuously grounded).
- [x] `grounded_fraction == 0.0` AND `members.len() > 0` → emit
  `CALYX_KERNEL_UNGROUNDED` (structured error code in the return value
  `GroundingGapReport.warning`, not a panic or silent zero).
- [x] `gaps` list is sorted by `CxId` (deterministic output).
- [x] The `Kernel.groundedness.unanchored_members` field is populated from
  `GroundingGapReport.gaps` during `build_kernel_pipeline`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 4 kernel members; anchors reachable from 3 of them within 2 hops;
  1 unreachable → `gaps = [unreachable_cx_id]`; `grounded_fraction = 0.75`.
- [x] unit: all kernel members reachable → `gaps = []`; `grounded_fraction = 1.0`;
  no `CALYX_KERNEL_UNGROUNDED` in `warning`.
- [x] unit: no anchors provided → all members are gaps; `grounded_fraction = 0.0`;
  `warning = CALYX_KERNEL_UNGROUNDED`.
- [x] unit: anchor at distance `max_anchor_dist` exactly → grounded (inclusive).
  anchor at distance `max_anchor_dist + 1` → gap.
- [x] proptest: `gaps.len() + grounded_count == kernel.members.len()` for all inputs.
- [x] edge: empty `kernel.members` → `gaps = []`; `grounded_fraction = 1.0`; no warning.
- [x] fail-closed: `max_anchor_dist = 0` → only direct anchor nodes are grounded
  (a kernel member is grounded iff it IS an anchor); no panic.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar grounding_gaps -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar grounding_gaps 2>&1 | tee /tmp/ph33_t03_fsv.txt && cat /tmp/ph33_t03_fsv.txt`.
- **Prove:** 4-member test prints `gaps = [<unreachable_cx>]` and `grounded_fraction = 0.75`;
  no-anchor test prints `CALYX_KERNEL_UNGROUNDED`; proptest passes; output attached
  to PH33 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH33 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
