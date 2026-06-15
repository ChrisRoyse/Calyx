# PH62 · T02 — Vault and lens subcommands

| Field | Value |
|---|---|
| **Phase** | PH62 — calyx-cli (vault/lens/ingest/search/readback) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/cmd/vault.rs` (≤500), `crates/calyx-cli/src/cmd/mod.rs` (≤500) |
| **Depends on** | T01 (error output layer) |
| **Axioms** | A4, A5, A17 |
| **PRD** | `dbprdplans/14 §2` (vault/panel group), `dbprdplans/18 §4` |

## Goal

Implement the six vault-and-lens subcommands that wire the `Calyx` engine API
(`create_vault`, `add_lens`, `retire_lens`, `park_lens`, `list_panel`,
`profile_lens`) to the CLI. The agent calls these once to configure a vault; all
subsequent intelligence rides the same panel. `add-lens` is "the one call that
replaces a whole pipeline" (PRD 14 §2).

## Build (checklist of concrete, code-level steps)

- [ ] `cmd/mod.rs`: `enum Subcommand { CreateVault, AddLens, RetireLens, ParkLens,
  ListPanel, ProfileLens, Ingest, Anchor, Measure, Search, KernelAnswer, Bits,
  Kernel, Guard, Abundance, ProposeLens, Provenance, VerifyChain, AnnealStatus,
  Readback, Healthcheck, Migrate }` with `fn parse(args: &[String]) -> Result<
  Subcommand, CliError>`; dispatch to per-module `fn run(…) -> Result<(), CliError>`
- [ ] `cmd/vault.rs` — `create-vault <name> [--panel-template <tmpl>]`:
  calls `Calyx::create_vault(name, panel_template)` → prints
  `{"vault_id":"<ulid>","name":"<name>","panel_template":"<tmpl>"}` to stdout
- [ ] `cmd/vault.rs` — `add-lens <vault> --name <n> --runtime <r> [--endpoint <url>]
  [--weights <path>] [--shape Dense(<dim>)|Sparse(<dim>)]`:
  calls `Calyx::add_lens(vault_id, LensSpec{…})` → prints
  `{"lens_id":"<hex16>","slot_id":<u16>,"name":"<n>"}` to stdout
- [ ] `cmd/vault.rs` — `retire-lens <vault> --slot <u16>` and
  `park-lens <vault> --slot <u16>`: call `Calyx::retire_lens` / `park_lens` →
  print `{"status":"retired"|"parked","slot":<u16>}`
- [ ] `cmd/vault.rs` — `list-panel <vault>`: calls `Calyx::list_panel` (uses
  `abundance` report skeleton) → `print_table(["slot","name","state","bits",
  "ci_lo","ci_hi"], rows)` to stdout
- [ ] `cmd/vault.rs` — `profile-lens [--runtime <r>] [--endpoint <url>]
  [--weights <path>] [--probe <path>]`: calls `Calyx::profile_lens` →
  prints capability card JSON
- [ ] All errors return `CliError::from(CalyxError)` — e.g. `add-lens` with wrong
  dim → `CALYX_LENS_DIM_MISMATCH` on stderr, exit 2

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: parse `["create-vault", "mydb"]` → `Subcommand::CreateVault { name:
  "mydb".into(), panel_template: None }`
- [ ] unit: parse `["add-lens", "mydb", "--name", "gte", "--runtime", "tei-http",
  "--endpoint", "http://localhost:8088"]` → correct `AddLens` variant with all
  fields populated
- [ ] unit: `retire-lens` with missing `--slot` → `CliError::UsageError` (not panic)
- [ ] proptest: round-trip `Subcommand::parse(args.to_cli_tokens())` is stable for
  all vault subcommand variants
- [ ] edge: unknown `--panel-template` value → `CALYX_CLI_USAGE_ERROR` with
  remediation listing valid values; vault name with spaces → `CALYX_CLI_USAGE_ERROR`
- [ ] fail-closed: `add-lens` against a non-existent vault → `CALYX_VAULT_ACCESS_DENIED`
  on stderr, exit 2; `retire-lens` on an already-retired slot → structured error,
  never panic

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the vault directory `<CALYX_HOME>/vaults/<vault_id>/` created on disk
  after `calyx create-vault`; the manifest file at `manifest/CURRENT`
- **Readback:** `calyx readback --vault-tree <vault_dir>` prints `DIR` / `FILE`
  lines; cross-check with `ls -laR <vault_dir>` on aiwonder
- **Prove:** after `create-vault`, the vault directory exists with the expected
  layout (manifest, cf/, wal/ subdirs present); after `add-lens`, the manifest
  contains the new LensId (read with `calyx readback --hex manifest/CURRENT`);
  after `retire-lens`, re-reading the panel shows `state=retired`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH62 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
