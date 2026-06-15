# PH26 · T01 — Intent classifier (keyword rules → profile name)

| Field | Value |
|---|---|
| **Phase** | PH26 — Query planner + intent + explain |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/planner.rs` (≤500) |
| **Depends on** | PH24 T04 (14 profile names) · PH25 T04 (`SlotKind`) |
| **Axioms** | A17, A16 |
| **PRD** | `dbprdplans/10 §2`, `dbprdplans/10 §7` |

## Goal

A deterministic keyword-rule intent classifier: given a `Query`, output a
`IntentLabel` that maps to one of the 14 ContextGraph profiles. The classifier
is the first thing the planner runs and is always overridable by an explicit
`FusionStrategy` in the query. No ML model; pure rule-based matching on query
text keywords and structural cues (slot selection, anchor type).

## Build (checklist of concrete, code-level steps)

- [x] `IntentLabel` enum:
  ```rust
  pub enum IntentLabel {
      Code, Causal, Entity, Temporal, Speaker, Style,
      Civic, Media, Bridge, Kernel, Semantic, Lexical, Multimodal, General,
  }
  ```
  (14 variants matching the 14 profiles from PH24 T04)
- [x] `fn classify_intent(query: &Query) -> IntentLabel`:
      Rule priority (first match wins):
      1. If `query.lenses == Explicit([single_slot])` and that slot's `SlotKind`
         implies a specific intent (Code → `IntentLabel::Code`, Speaker →
         `IntentLabel::Speaker`, etc.) → use that
      2. If `query.input` is `QueryInput::Text(text)`:
         - text contains any of `["def ", "fn ", "class ", "import ", "->", "::", "impl "]`
           → `Code`
         - text starts with `["because", "why ", "cause of", "reason for", "led to"]`
           → `Causal`
         - text matches `\b(who|person|organization|company|named)\b` (regex)
           → `Entity`
         - text matches `\b(since|before|after|during|when|in \d{4})\b`
           → `Temporal`
         - any `"voice of"` or `"speaker"` in text → `Speaker`
         - `"style of"` or `"write like"` → `Style`
      3. If `query.input` is `QueryInput::Anchor(_)` → `Semantic` (anchor-to-anchor)
      4. Default: `General`
- [x] `classify_intent` is pure (no I/O, no side effects); `#[must_use]`
- [x] If `query.fusion` is already an explicit non-Auto strategy, the planner
      skips classification — document this as the A17 override path

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `"def foo(x: int) -> str:"` → `IntentLabel::Code`
- [x] unit: `"why did the Roman Empire fall"` → `IntentLabel::Causal`
- [x] unit: `"who founded Apple"` → `IntentLabel::Entity`
- [x] unit: `"events in 1789"` → `IntentLabel::Temporal`
- [x] unit: `"summarize this document"` → `IntentLabel::General` (no specific cue)
- [x] unit: anchor input → `IntentLabel::Semantic`
- [x] unit: explicit single-slot Code kind → `IntentLabel::Code` regardless of text
- [x] proptest: classifier never panics for any `&str` input
- [x] edge: empty text → `IntentLabel::General`
- [x] edge: text matching multiple rules → first rule wins (Code > Causal in priority)
- [x] fail-closed: `query.fusion = FusionStrategy::Rrf` (explicit) → classifier
      is not called; planner uses `Rrf` directly (test by asserting
      `classify_intent` is not called when fusion is explicit — use a call counter)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant intent_classifier -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant intent_classifier -- --nocapture 2>&1`
- **Prove:** test prints per-case results:
  `code_ok=true causal_ok=true entity_ok=true temporal_ok=true general_ok=true`

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH26 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
