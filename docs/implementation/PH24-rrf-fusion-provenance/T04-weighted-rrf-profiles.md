# PH24 · T04 — `WeightedRRF` profiles (14 ContextGraph defaults)

| Field | Value |
|---|---|
| **Phase** | PH24 — RRF/WeightedRRF/SingleLens fusion + provenance hits |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/fusion/rrf.rs` (≤500), `crates/calyx-sextant/src/fusion/profiles.rs` (≤500) |
| **Depends on** | T03 (this phase) |
| **Axioms** | A17, A16 |
| **PRD** | `dbprdplans/10 §2`, `dbprdplans/10 §7` |

## Goal

Extend RRF with named weight profiles: `WeightedRRF(profile_name)` applies
intent-specific slot weights. The 14 ContextGraph default profiles ship as
built-in constants (causal, code, entity, temporal, speaker, style, civic,
media, bridge, kernel, semantic, lexical, multimodal, general). Each profile
maps slot kinds to relative weights; the planner (PH26) will auto-select; here
we wire the data and the lookup.

## Build (checklist of concrete, code-level steps)

- [x] `crates/calyx-sextant/src/fusion/profiles.rs`:
  ```rust
  pub struct FusionProfile {
      pub name: &'static str,
      pub slot_weights: &'static [(SlotKind, f32)],  // SlotKind::Dense / Sparse / Temporal etc.
      pub description: &'static str,
  }
  pub const PROFILES: &[FusionProfile] = &[
      FusionProfile { name: "causal",     slot_weights: &[(SlotKind::Dense, 0.7), (SlotKind::Temporal, 0.3)], description: "causal/directional queries" },
      FusionProfile { name: "code",       slot_weights: &[(SlotKind::Code, 1.0)],  description: "code-only" },
      FusionProfile { name: "entity",     slot_weights: &[(SlotKind::Dense, 0.6), (SlotKind::Sparse, 0.4)], description: "named-entity retrieval" },
      FusionProfile { name: "temporal",   slot_weights: &[(SlotKind::Temporal, 0.6), (SlotKind::Dense, 0.4)], description: "time-sensitive" },
      FusionProfile { name: "speaker",    slot_weights: &[(SlotKind::Speaker, 1.0)], description: "speaker-identity" },
      FusionProfile { name: "style",      slot_weights: &[(SlotKind::Style, 1.0)],  description: "style-locked" },
      FusionProfile { name: "civic",      slot_weights: &[(SlotKind::Dense, 0.5), (SlotKind::Sparse, 0.5)], description: "civic/public-record" },
      FusionProfile { name: "media",      slot_weights: &[(SlotKind::Media, 1.0)],  description: "image/audio" },
      FusionProfile { name: "bridge",     slot_weights: &[(SlotKind::Dense, 0.4), (SlotKind::Code, 0.3), (SlotKind::Sparse, 0.3)], description: "cross-domain bridge" },
      FusionProfile { name: "kernel",     slot_weights: &[(SlotKind::Kernel, 1.0)], description: "kernel-first precision" },
      FusionProfile { name: "semantic",   slot_weights: &[(SlotKind::Dense, 1.0)],  description: "pure semantic similarity" },
      FusionProfile { name: "lexical",    slot_weights: &[(SlotKind::Sparse, 1.0)], description: "pure lexical/BM25" },
      FusionProfile { name: "multimodal", slot_weights: &[(SlotKind::Dense, 0.5), (SlotKind::Media, 0.5)], description: "text+media" },
      FusionProfile { name: "general",    slot_weights: &[(SlotKind::Dense, 0.6), (SlotKind::Sparse, 0.4)], description: "general purpose (default)" },
  ];
  pub fn lookup_profile(name: &str) -> Option<&'static FusionProfile>;
  ```
- [x] `SlotKind` enum in `query.rs` or `fusion/mod.rs` (Dense | Sparse | Temporal |
      Code | Speaker | Style | Media | Kernel — add more as lenses are added)
- [x] `WeightedRrfStrategy` in `rrf.rs`: same as `RrfStrategy` but resolves
      per-slot weights from the profile by matching each slot's `SlotKind`;
      unmatched slots get weight 0.0 (excluded from fusion for that profile)
- [x] Post-sweep #286 enforces this behavior in the current slot-id profile
      implementation: missing weights are excluded for `WeightedRRF`, while
      plain `RRF` keeps unit-weight participation for all result slots.
- [x] `FusionStrategy::WeightedRrf(String)` → look up profile → build weight map;
      `CALYX_SEXTANT_UNKNOWN_PROFILE` if name not found
- [x] `fn list_profiles() -> &'static [&'static str]` — used by `explain` and planner

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `lookup_profile("general")` returns `Some`; `lookup_profile("unknown")` returns `None`
- [x] unit: `PROFILES.len() == 14` — assert exact count
- [x] unit: WeightedRRF with "lexical" profile excludes dense slot (weight 0.0) —
      confirm dense-slot cx do not appear in top results when lexical score is 0
- [x] unit: WeightedRRF with "semantic" profile → results identical to RRF with
      single dense slot at weight 1.0
- [x] proptest: all 14 profiles are non-empty (at least one slot_weight entry)
- [x] edge: `FusionStrategy::WeightedRrf("unknown")` → `CALYX_SEXTANT_UNKNOWN_PROFILE`
- [x] edge: profile with a slot kind not present in the vault → graceful skip,
      not an error (the fusion simply has fewer participants)
- [x] fail-closed: weight 0.0 slot never contributes to fused_score (assert
      `contribution == 0.0` in `per_lens` for the excluded slot)
- [x] regression: AP-60 temporal slots 20/21/22 are absent from primary
      profiles until PH40 and an unlisted temporal result is skipped by
      `WeightedRRF`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant weighted_rrf -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant weighted_rrf -- --nocapture 2>&1`
- **Prove:** test prints `profiles=14 lookup_general=ok unknown=None lexical_excludes_dense=true`

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH24 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
