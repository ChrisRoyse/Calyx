# 31 — Synapse: Computer Use & Agent Orchestration (Dev Runtime)

> **Living-system role:** hands & eyes — the agent's ability to actually operate the machine, perceive real state, and command other agents to build Calyx (A15/`DOCTRINE §0`)

How agents build, run, test, and FSV Calyx **on the real machine**, and how they **orchestrate other agents** to get work done. Synapse is the **preferred mechanism over subagents** for substantive development. The founder's coding doctrine already states *"Synapse MCP runtime is part of FSV"* and *"Synapse should feed reality deltas, then audit drift"* (`AICodingAgentSuperPrompt.md`) — this doc operationalizes that for Calyx.

## 1. What Synapse is

Synapse is a **perception + action MCP runtime for full computer use** (M1 perception · M2 action · M3 reflex/orchestration), resident on the dev/aiwonder environment. It lets an agent *see* the actual screen/terminal/files and *act* on them — including opening terminals, typing into them, and driving other AI coding agents — instead of being confined to a tool-call sandbox.

## 2. Capabilities (the real tool surface)

| Class | Synapse tools | Use |
|---|---|---|
| **Perceive** | `observe`, `observe_delta`, `capture_screenshot`, `read_text` (OCR), `find` (locate UI/text), `audio_tail`, `audio_transcribe` | see the real screen/terminal output, diff what changed, read what an agent or test actually printed |
| **Act** | `act_launch` (open apps/terminals), `act_type`, `act_press`, `act_combo`, `act_keymap`, `act_click`, `act_scroll`, `act_stroke`, `act_pad`, `act_run_shell` | open a terminal, type commands/prompts, run shells, drive any GUI |
| **Reflexes** | `reflex_register`, `reflex_list`, `reflex_cancel`, `reflex_history` | reactive automation — fire an action when an observed condition occurs (e.g. a build finishes, an error appears) |
| **Reality / FSV** | `reality_baseline`, `reality_audit` | snapshot expected state, then audit the *actual* perceived state against it → **reality deltas + drift audit** (this is FSV's perception arm) |
| **Profiles** | `profile_*` | app-specific automation profiles (reusable, audited) |
| **Control / audit** | `set_capture_target`, `set_perception_mode`, `subscribe`, `replay_record`, `health`, `release_all`, `storage_*`, `audit_*` | target a window, stream events, record/replay, clean up, query the audit trail |

## 3. Full computer use

With Act + Perceive, an agent **operates the real machine**: open a terminal (`act_launch`), run `cargo build`/`cargo test`/`calyx ...` (`act_run_shell` or `act_type`), read the *actual* output (`read_text`/`observe`), drive any GUI tool, inspect dashboards (Grafana), and confirm real file/terminal state. This is real-environment execution on aiwonder, not a simulated return value — exactly what FSV requires (`DOCTRINE §0`).

## 4. Agent orchestration — drive Claude/Codex in real terminals (preferred over subagents)

The highest-leverage use: **Synapse opens terminals and commands other AI coding agents** (Claude Code, Codex) to do the work.

```
Orchestrate(task):
  1. act_launch a terminal (one per worker)
  2. start a coding agent in it:
       act_type "cldy"          # opens Claude
       act_type "codex --yolo"  # opens Codex
  3. act_type the task/prompt (point it at the Calyx repo, a context issue, a PRD)
  4. observe / read_text its real output as it works (screenshot+vision for editor/GUI, §6c)
  5. reflex_register to react when it finishes / errors / asks
  6. reality_audit its result against the bytes on aiwonder (FSV, §5)
  7. release_all / clean up the tab/terminal
```

Run **many such agents across parallel terminals**, each on a scoped task (a crate, a phase gate, a dataset acquisition). **The commanded agents themselves have full Synapse capabilities** — they perceive, act, run shells, drive the browser, and orchestrate further agents — so computer-use is recursive. These Claude (`cldy`) / Codex (`codex --yolo`) workers are **much, much more powerful than the built-in subagent tool**: full coding agents in real terminals, full toolset, full Synapse control, real FSV.

**Why this beats subagents** for substantive dev work:
- Real environment, real terminals, real builds/tests on aiwonder → **real FSV against real bytes**, not a sandboxed claim.
- Each worker is a full coding agent (Claude/Codex) with the whole toolset + Synapse, not a constrained tool-call helper.
- Long-running real work (compile, soak tests, dataset downloads) runs in actual terminals and is observable/auditable.
- Coordination via the `chrisroyse/calyx` GitHub context issues (`29`) + Synapse perception of each terminal — fully visible, fully audited.
- Cost-free and self-hosted (A34): our own agents on our own box.

Use the built-in Agent/subagent tool only for quick, read-only fan-out; use **Synapse-driven agents for building, testing, and any task that touches the real machine.**

## 5. Synapse is part of FSV

FSV = read the source-of-truth bytes (`DOCTRINE §0`). Synapse is **how an agent perceives that reality**, and FSV MUST use its **full** ability set — the complete step→ability mapping is in **`28 §2c`** (binding). In brief: `set_capture_target`/`health` → `reality_baseline` + `act_run_shell` readback + `read_text`/`find` (SoT *before*) → `act_*` (do the thing) → `observe_delta`/`reality_audit` + `read_text`/`find` (SoT *after*; **the delta is the evidence**) → `capture_screenshot` + `replay_record` + `audit_export_bundle` (recorded evidence → GitHub issue). Async ops use `reflex_register` to FSV the real end-state when it appears; `subscribe`/`observe_delta` stream live changes. A green test printed in a real terminal that Synapse *reads* is evidence; a harness asserting "passed" is not. **If a step could pass on a return value instead of a Synapse-perceived byte, it is not FSV.**

## 6. Using Synapse to build Calyx

- The orchestrating agent drives build/test agents on aiwonder to execute the roadmap (`19`): open terminals, `cargo build`/`test`/`clippy`, run the ≤500-line gate (`DOCTRINE §8`), `calyx` CLI against real vaults, read real output, FSV against real Aster/Ledger bytes (`28 §2`).
- Dataset acquisition (`28 §3`): drive an agent to download + checksum datasets onto aiwonder; `reality_audit` the files present.
- Parallel phase work: one agent per crate/phase gate, coordinated via context issues (`29`), observed via Synapse.
- Identity-locked / media work, deployment ops, Grafana checks: all via Synapse computer use.

## 6b. Browser use — one main Chrome, new tabs only (binding)

For **anything web** — dashboards, websites, accounts — agents use the operator's **main Chrome browser**, which is already logged into everything.

- **Exactly ONE Chrome open at any time** — the operator's main instance. Agents MUST NOT launch a second Chrome, a new window/profile, incognito, or any other browser. A fresh instance is not logged in; the main one is.
- **Open a NEW TAB** in that Chrome for whatever is needed (Synapse: `find`/`set_capture_target` the Chrome window → `act_combo` `Ctrl+T` → `act_type` the URL → `read_text`/`observe` the page). New tabs **auto-authenticate** via the main browser's existing sessions — GitHub, Cloudflare, Grafana (`ops.leapable.ai`), HuggingFace, Infisical web, any dashboard or site — no login step, no credentials typed.
- **Reuse, don't proliferate.** Open the tab, do the task, perceive the result (FSV via `read_text`/`capture_screenshot`), then close that tab. **Never close the main Chrome**, and never leave a pile of orphan tabs.
- **Why:** zero auth friction (sessions already live), zero secret handling in the browser (A33/§7 — never type credentials), and a single, observable, auditable browser surface. Setting up any dashboard or accessing any web resource = one new tab in the one Chrome.

## 6c. Visual perception — screenshot + AI-vision (a primary mode)

Synapse is **exceptional at capturing screenshots/images and having the AI look at them to see what's actually going on** — and this is a first-class perception mode, not a fallback. `capture_screenshot` → the agent **visually analyzes the image** (it's a vision-capable model) and understands the real state directly.

- **Catches what text/OCR can't:** Grafana charts and the growth curve `J` (`27`) trends, latency graphs, GUI layout/state, error dialogs, progress bars, rendered media (identity-locked output, ClipCannon frames), a coding agent's terminal *and* editor, color-coded status — things `read_text` alone misses.
- **Use it for FSV:** screenshot the dashboard/terminal/file view and *look* — the AI confirms the real state from the pixels (e.g. "the recall curve rose, no error banner, the test pane is green") — un-fakeable evidence, attached to the issue (`28 §2c`, `28 §5`).
- **Use it for dev observation:** watch a build/soak/agent visually; `observe`/`observe_delta` for change, `capture_screenshot` + vision when the meaning is visual.
- **Combine:** `find` to locate, `read_text` for exact numbers, **screenshot+vision for holistic "what's happening."** All three together give complete perception.

## 6d. Agents handle every aspect of development (the purpose)

This whole capability stack exists for one end: **agents, using Synapse, optimally create Calyx and handle every single aspect of development end-to-end** — autonomously, on aiwonder, free. The full lifecycle, all via Synapse computer use:

| Aspect | How agents do it (Synapse) |
|---|---|
| Author + scaffold | drive Claude/Codex in terminals to write crates (`act_launch`/`act_type`, `31 §4`) |
| Build | `act_run_shell` `cargo build` on aiwonder; `read_text`/screenshot the result |
| Test | `cargo test`/`proptest`/`cargo-fuzz`/`cargo-mutants`/`criterion`; perceive real output (`28 §6c`) |
| **FSV** | `reality_baseline`→act→`reality_audit`, `read_text`/`find`/screenshot+vision, evidence bundle (`28 §2c`) |
| Datasets | drive an agent to download + checksum onto aiwonder; `reality_audit` files present (`28 §3`) |
| Deploy + ops | systemd, ZFS, GPU, restic via `act_run_shell`; verify SoT (`16`) |
| Dashboards / web | new tab in the one main Chrome (auto-authed) + screenshot+vision (`§6b`/`§6c`) |
| Debug | hypothesis-driven, perceive real state, 5-Whys to root cause (`AICodingAgentSuperPrompt.md`) |
| Coordinate | GitHub context issues (`29`) + observe each worker terminal |
| Parallelize | many Claude/Codex workers across terminals, each FSV'd |

The intent (founder): agents should be able to **handle every aspect of development** — nothing requires a human in the loop except direction and approval of outward-facing/destructive actions (`§7`). Synapse is what makes that real: full perception, full action, full orchestration, on the real machine, with real FSV.

## 7. Safety & ops bounds (binding)

- **Real actions have real consequences.** Confirm destructive/outward-facing actions first; never `act_run_shell` a `rm -rf`/UFW/sshd change without the safeguards in `16` (e.g. a second live session before firewall changes — lockout risk).
- **Fail closed + audit.** Use `audit_*`/`reality_audit` to verify outcomes; `release_all` to clean up held input/automation; respect aiwonder ops rules (`16 §9`: don't start throwaway services, verify SoT after every op).
- **Secrets:** never `act_type` a secret value into a terminal that logs it; pull from Infisical via `infisical run` (`16 §5b`).
- **Provenance:** Synapse `audit_*` + the Calyx Ledger + the GitHub issue journal together record what each agent did — auditable, reproducible.

**One sentence:** Synapse is Calyx's computer-use and agent-orchestration runtime — full perceive/act control of the real machine (observe/read/find + type/launch/run_shell), reflexes, and reality-audit that *is* FSV's perception arm — and its highest use is opening terminals to command Claude/Codex agents (which themselves get full Synapse capabilities) to build, test, and verify Calyx on aiwonder in real environments, which is strictly better than sandboxed subagents for any task that touches the machine.
