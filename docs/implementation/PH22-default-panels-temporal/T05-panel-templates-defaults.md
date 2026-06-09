# PH22 · T05 — Panel templates + default panel constructors

| Field | Value |
|---|---|
| **Phase** | PH22 — Default panels + temporal lenses E2/E3/E4 |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/panels/mod.rs` (≤500), `crates/calyx-registry/src/panels/defaults.rs` (≤500) |
| **Depends on** | T04 (this phase) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/05 §7` |

## Goal

Define `PanelTemplate` and `instantiate_panel`, and implement the four default
panel constructors (`text_default`, `code_default`, `civic_default`,
`media_default`) so a new vault is multi-lens on day one. Each panel includes
the E2/E3/E4 temporal slots. Panels reference slot names and runtimes; they do
not instantiate real HF models — they build `LensSpec` descriptor lists that
`add_lens` will register.

## Build (checklist of concrete, code-level steps)

- [x] `PanelSlotSpec` struct: `name: String`, `runtime: LensRuntime`,
  `output: SlotShape`, `modality: Modality`, `retrieval_only: bool`,
  `excluded_from_dedup: bool`, `required: bool`.
- [x] `PanelTemplate` struct: `name: String`, `slots: Vec<PanelSlotSpec>`.
- [x] `pub fn instantiate_panel(template: &PanelTemplate, registry: &mut Registry, store: &dyn VaultStore) -> Result<Vec<LensId>>`:
  - for each `PanelSlotSpec`: build a `LensSpec` (with stub `weights_sha256`
    and `corpus_hash` = all zeros for TEI runtimes — operator fills these in
    before production deployment), then call `add_lens`.
  - return vec of allocated `LensId`s.
- [x] `pub fn text_default() -> PanelTemplate`: slots per `05 §7`:
  1. `E1 semantic (GTE)` — `TeiHttp { endpoint: "127.0.0.1:8088" }`, Dense(768)
  2. `keyword/SPLADE` — `Algorithmic(OneHot { vocab_size: 30522 })`, Sparse(30522)
  3. `paraphrase` — `TeiHttp { endpoint: "127.0.0.1:8088" }`, Dense(768)
  4. `entity` — `TeiHttp { endpoint: "127.0.0.1:8088" }`, Dense(768)
  5. `causal(dual)` — `TeiHttp { endpoint: "127.0.0.1:8088" }`, Dense(768),
     `asymmetry: Dual`
  6. `E2_recency` — `Algorithmic(E2)`, Dense(1), `retrieval_only=true,
     excluded_from_dedup=true`
  7. `E3_periodic` — `Algorithmic(E3)`, Dense(2), same flags
  8. `E4_positional` — `Algorithmic(E4)`, Dense(4), same flags
- [x] `pub fn code_default() -> PanelTemplate`: slots from `05 §7`
  `code-default` list (semantic, AST, CFG, dataflow, type-graph, trace, diff,
  oracle-anchor, static-analysis, runtime, reasoning, scalars + E2/E3/E4).
  Use `Algorithmic` placeholders for code-specific slots that have no real
  model yet.
- [x] `pub fn civic_default() -> PanelTemplate`: `05 §7` civic slots (the
  21-slot Polis Constellation — stub with 21 `Algorithmic(Scalar)` placeholders
  + E2/E3/E4).
- [x] `pub fn media_default() -> PanelTemplate`: `05 §7` media slots (semantic,
  image-CLIP, audio-wave, audio-emotion, speaker-WavLM, transcript,
  style-register + E2/E3/E4). Use `ExternalCmd` stubs for modalities that need
  separate processes.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `text_default()` has exactly 8 slots; E2/E3/E4 are slots 6,7,8;
  all three have `retrieval_only=true`.
- [x] unit: `code_default()` slot count ≥ 12+3=15 (code slots + temporal).
- [x] unit: `civic_default()` slot count ≥ 21+3=24.
- [x] unit: `media_default()` slot count ≥ 7+3=10.
- [x] unit: `instantiate_panel` on `text_default()` with a mock `Registry` and
  mock `store` → returns 8 `LensId`s; `registry.panel_version == 8`.
- [x] edge (≥3): (1) calling `instantiate_panel` twice on same template →
  idempotent (same LensIds returned, version not double-bumped); (2) a slot
  with a broken spec fails closed (frozen violation) and does not corrupt the
  rest of the panel; (3) temporal slots always appear last.
- [x] fail-closed: a slot with frozen violation → that slot's `add_lens`
  returns the violation error; panel instantiation continues for remaining
  slots but reports which slots failed.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `registry.slot_map`, `registry.panel_version`, and slot names after
  `instantiate_panel` on aiwonder
- **Readback:** `cargo test -p calyx-registry panels_defaults -- --nocapture 2>&1`
- **Prove:** output shows `text_default: 8 slots [E1-semantic, keyword, paraphrase,
  entity, causal, E2_recency, E3_periodic, E4_positional] panel_version=8`;
  same slot count checks for other three panels; screenshot attached to PH22
  GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH22 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
