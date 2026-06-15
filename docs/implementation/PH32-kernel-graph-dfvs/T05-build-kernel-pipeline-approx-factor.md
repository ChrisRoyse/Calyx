# PH32 · T05 — `build_kernel_pipeline` wiring + `Kernel` struct + approx-factor reporting

> **STATUS: ✅ DONE / FSV-signed-off.** Implemented in
> `crates/calyx-lodestar/src/kernel.rs` with serializable `Kernel`,
> deterministic `kernel_id`, groundedness/provisional reporting,
> approximation-factor/tau propagation, and serde roundtrip coverage. aiwonder
> FSV readback: `ph32-kernel-pipeline-readback.json`; #645 adds honest
> exact-vs-approximate tau readback under
> `/home/croyse/calyx/data/fsv-issue645-dfvs-honest-20260611T072428Z`
> (`ph32-dfvs-honest-bounds-readback.json` SHA-256
> `82617d924c8e8c47355cbc3dda83b75f27a47fb4a15f690bc983f8e4760322f7`).

> Historical checklist note: the unchecked implementation prompts below were
> satisfied by the closed Stage 6 evidence; current state is the status/evidence
> block above.

| Field | Value |
|---|---|
| **Phase** | PH32 — Kernel-graph (~10% target) + directed MFVS (~1% target) |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/kernel.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04 (full kernel-graph + DFVS pipeline available) |
| **Axioms** | A10, A11 |
| **PRD** | `dbprdplans/08 §3`, `08 §6`, `08 §7` |

## Goal

Wire the pipeline stages (condense -> kernel-graph -> LP scaffold/fallback ->
DFVS) into `build_kernel_pipeline` and produce the complete `Kernel` struct
(per PRD section 6).
The approx-factor and tau certificate are emitted in the struct so they are
always auditable (`08 §7`: "MFVS approximation factor and the recall test are
reported so the kernel's quality is auditable, not asserted"). The `Kernel` is
serializable to the `idx/kernel/` store (PH33 uses it as an index).

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct Kernel { kernel_id: KernelId, panel_version: u64, anchor_kind: Option<AnchorKind>, corpus_shard_hash: [u8;32], members: Vec<CxId>, kernel_graph: Vec<CxId>, groundedness: GroundednessReport, recall: RecallReport, built_at: Timestamp, estimator_provenance: String }` — exact PRD §6 fields.
- [ ] `pub struct GroundednessReport { reached_anchor: f32, unanchored_members: Vec<CxId> }`.
- [ ] `pub struct RecallReport { kernel_only: f32, full: f32, ratio: f32, approx_factor: f64, tau_star_estimate: usize, tau_star_exact: bool }` — approximation fields from `DfvsResult`.
- [ ] `pub fn build_kernel_pipeline(graph: &AssocGraph, anchors: &[CxId], params: &KernelParams) -> Result<Kernel, CalyxError>`:
  1. `tarjan_scc` → `condensate` (PH31).
  2. `select_kernel_graph` + `lp_round_kernel_graph` scaffold/fallback (T01/T02).
  3. `dfvs_approx` on the kernel-graph (T03/T04).
  4. Anchor-reachability check: BFS from each `dfvs_approx` member to any anchor;
     unreachable → `unanchored_members`.
  5. Populate `Kernel` struct; `recall.ratio` is stub `0.0` until PH33 adds the
     recall test; `approx_factor`, `tau_star_estimate`, and `tau_star_exact` are
     always populated from step 3.
- [ ] `KernelId` = `CxId`-space hash of `(panel_version, anchor_kind, corpus_shard_hash)`.
- [ ] If `members.is_empty()` AND input graph is non-empty → `CALYX_KERNEL_EMPTY_RESULT`.
- [ ] Provisional flag: if `unanchored_members.len() == members.len()` (fully
  ungrounded) → add `"provisional"` tag to `estimator_provenance`; emit
  `CALYX_KERNEL_UNGROUNDED` warning.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 6-node planted graph with known 1-node MFVS and 2 anchor nodes;
  `build_kernel_pipeline` → `members.len() == 1`; the planted FVS node is
  in `members`; `unanchored_members` is empty (anchor reachable).
- [ ] unit: same graph with anchors removed → `unanchored_members = [planted_fvs_node]`;
  `estimator_provenance` contains `"provisional"`; `CALYX_KERNEL_UNGROUNDED` logged.
- [ ] unit: `RecallReport.approx_factor`, `tau_star_estimate`, and
  `tau_star_exact` match the underlying `DfvsResult` from `dfvs_approx`.
- [ ] unit: `Kernel` round-trips via serde (JSON); `kernel_id` re-derives to same
  value from same inputs.
- [ ] edge: DAG input (no cycles) → `members = []`; `kernel_graph` contains top-10%
  nodes; no error.
- [ ] fail-closed: graph with non-empty members but all unreachable from anchors →
  `CALYX_KERNEL_UNGROUNDED` (not a silent struct with `reached_anchor = 0.0`
  and no warning).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar kernel -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar kernel 2>&1 | tee /tmp/ph32_t05_fsv.txt && cat /tmp/ph32_t05_fsv.txt`.
- **Prove:** planted-FVS test prints `members` containing the planted node ID;
  `approx_factor` and tau certificate values printed; `provisional` tag visible
  for unanchored case; serde round-trip JSON matches; output table attached to
  PH32 GitHub issue confirming computed vs known planted FVS.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH32 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
