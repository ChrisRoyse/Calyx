# PH74 T02 - Default Domain Panels

Issue: #789

## Implementation

PH74 adds four batteries-included panel templates in `calyx-registry`:

- `legal-default`: legal domain, general semantic, keyword, entity, causal-dual, E2/E3/E4.
- `medical-default`: biomedical, general semantic, medical entity, E2/E3/E4.
- `bio-default`: protein, DNA, molecule, general semantic, E2/E3/E4.
- `media-default`: media semantic, SigLIP image, CLAP audio, wave, emotion, speaker, transcript, style, E2/E3/E4.

Content slots use `PanelLensRuntime::Registry { name }`. Applying a default panel resolves each registry slot against the live `Registry` by frozen/spec name before mutating panel state. Missing or unconverted content lenses fail closed with `CALYX_PANEL_LENS_MISSING` and leave the panel unchanged. The temporal trio remains the built-in E2/E3/E4 template tail and is present on every domain template.

`calyx panel status --vault <dir>` now reads the persisted vault panel source of truth through `load_vault_panel_state`, then prints slot state, bits, health, panel version, and registry snapshot size. Existing `calyx panel status --home <dir>` catalog behavior is unchanged.

## FSV Recipe

Run on aiwonder from `/home/croyse/calyx/repo`:

```bash
export CALYX_FSV_ROOT=/home/croyse/calyx/tmp/issue789-fsv-$(date -u +%Y%m%d-%H%M%S)
cargo test -p calyx-registry --test issue789_default_domain_panels_fsv -- --nocapture
cargo build -p calyx-cli
target/debug/calyx panel status --vault "$CALYX_FSV_ROOT/vault-legal-default" > "$CALYX_FSV_ROOT/cli-panel-status-legal.json"
target/debug/calyx panel status --vault "$CALYX_FSV_ROOT/vault-medical-default" > "$CALYX_FSV_ROOT/cli-panel-status-medical.json"
target/debug/calyx panel status --vault "$CALYX_FSV_ROOT/vault-bio-default" > "$CALYX_FSV_ROOT/cli-panel-status-bio.json"
target/debug/calyx panel status --vault "$CALYX_FSV_ROOT/vault-media-default" > "$CALYX_FSV_ROOT/cli-panel-status-media.json"
```

Read back:

- `summary.json`
- `panel-status-legal-default.json`
- `panel-status-medical-default.json`
- `panel-status-bio-default.json`
- `panel-status-media-default.json`
- `edge-missing-lens.json`
- `cli-panel-status-*.json`

Expected evidence:

- All four panel status files contain the expected slot set and end with `E2_recency`, `E3_periodic`, `E4_positional`.
- Registry-resolved content slots have concrete `lens_id` values from the registered frozen lens names.
- `edge-missing-lens.json` reports `CALYX_PANEL_LENS_MISSING` and identical before/after panel version and slot count.
- CLI `--vault` output reads the same persisted panel version and slot count from the vault assets.
