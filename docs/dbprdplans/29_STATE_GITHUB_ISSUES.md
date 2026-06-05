# 29 — Development State via GitHub Issues (Context Issues & Pruning)

> **Living-system role:** shared working memory — the current, curated state every agent reads in before acting (A15/`DOCTRINE §0`)

Development state for Calyx is managed in **the Calyx repository's GitHub Issues** — the single coordination + memory surface (`AICodingAgentSuperPrompt.md` §3; `DOCTRINE §8b`). This doc specifies the **context issues** every agent must read every turn, and the **pruning discipline** that keeps issues from confusing future agents who open them with a fresh context window.

## 1. Why issues, and the cardinal rule

Agents are stateless across sessions; the issue tracker is their shared, durable memory. **Open issues = active state; comments = journal; closed issues = institutional knowledge; labels = organization.** The cardinal rule (`DOCTRINE §0`) applies to issues too: **an issue is a claim; the source of truth is the bytes.** A context issue that drifts from reality is a *lie that misleads every future agent* — so currency and pruning (§4) are not housekeeping, they are correctness.

## 2. Context issues — the must-read state (`type:context`, pinned)

A **small, curated set** of pinned `type:context` issues holds the critical, crucial information **every agent must read at the start of every turn and keep in context at all times.** They are a **current-state snapshot, not a journal** — tight, true, and short enough to always fit in context.

| Pinned context issue | Holds (current truth only) |
|---|---|
| **`[CONTEXT] Mission & invariants`** | the thesis pointer (`00`/`DOCTRINE`); the binding invariants (link `DOCTRINE` axioms A1–A32, do not duplicate); scope (universal DB + AGI; Leapable = Vault-only, PostgreSQL untouched) |
| **`[CONTEXT] You are here`** | current phase (`19`), what's done/in-flight/next; the one or two things that matter *right now* |
| **`[CONTEXT] Environment & ops`** | everything runs on aiwonder (`28 §5`/`16`); reach via `~/.config/aiwonder.env`; secrets = Infisical (`hf_hub_token`); build/test = on aiwonder |
| **`[CONTEXT] Landmines`** | the gotchas that bite *every* agent (e.g. no `rustc` on box; ≤500-line rule; FSV reads bytes; never secret values in issues; dedup never merges conflicting anchors) |
| **`[CONTEXT] Datasets`** | which real datasets are acquired + verified on aiwonder (`28 §3`, `datasets/MANIFEST.md` pointer); what's still needed |

Rules for context issues:
- **Read every turn, before acting** (§3). Treat them as authoritative current state.
- **Pointers, not copies.** Link `DOCTRINE`/PRD sections; never paste duplicate content that can drift. The single source of an invariant is the doc; the context issue points to it + states the *current* status.
- **Snapshot, not history.** They hold what is true *now*. History/rationale lives in closed `type:decision`/`type:discovery` issues, not here.
- **Short.** If a context issue grows long, it's accreting history — prune it (§4). It must stay small enough to always be in-context.
- **Last-verified stamp + owner** on each (date of last reality-check against the SoT).

## 3. Read-state protocol (start of every turn)

Every agent, every session, runs the read-state queries first (`AICodingAgentSuperPrompt.md` §3.3), Calyx-specific:
```bash
REPO=<calyx-repo>
gh issue list --repo $REPO --state open --label type:context --json number,title,body,updatedAt   # 1. pinned context — READ ALL
gh issue list --repo $REPO --state open --label status:in-progress --json number,title,assignees    # 2. don't step on
gh issue list --repo $REPO --state open --label status:blocked                                        # 3. unblockable?
gh issue list --repo $REPO --state open --label type:task --search "no:assignee sort:updated-asc"     # 4. queue
gh issue list --repo $REPO --state closed --label type:decision --search "<topic>"                    # 5. binding decisions
gh issue list --repo $REPO --state closed --label "type:discovery,type:pattern" --search "<task>"     # 6. prior gotchas
```
Then claim atomically before work, comment at milestones, pause/blocked/done per protocol (`§3.4–§3.8`). The ≤500-line modularization rule files `type:task` issues here (`DOCTRINE §8`).

## 4. Maintenance & pruning — keep issues from confusing future agents (the key discipline)

Fresh-context agents trust the tracker literally. Stale or contradictory issues actively mislead them. So:

**Currency (keep it true):**
- Context issues are **living**: update them at every phase transition and the moment an invariant/status changes. **Edit the body to the new truth — never append a contradiction.** A context issue must never contain two conflicting statements; the false one is deleted, not struck through.
- Re-stamp last-verified each update; if a context issue hasn't been reality-checked against the SoT in its review interval, an agent re-verifies each line and updates or prunes it.

**Pruning (remove the stale):**
- **Completed work → close** the `type:task` immediately (open = still active; a done-but-open task is a lie). Closing comment carries the FSV evidence.
- **Superseded decisions →** mark `superseded-by #N`, link both ways, **close** the old one. Only the current decision stays open/referenced.
- **Obsolete discoveries/gotchas →** when a gotcha is fixed or a landmine removed, close the `type:discovery` with a resolution note and **remove it from `[CONTEXT] Landmines`**.
- **Per-phase context-hygiene pass (mandatory):** at each phase boundary, re-read every pinned context issue, verify each line against the bytes (`DOCTRINE §0`), **delete stale lines**, fold in what's newly load-bearing, re-pin only the still-relevant set. Record the hygiene pass as a comment.
- **Dedupe before every create** (`AICodingAgentSuperPrompt.md` §3.14): search existing issues first; never open a second issue for a covered topic.

**Anti-patterns (refuse):**
- A context issue that has become an ever-growing log → it confuses new agents. Context issues are **snapshots**; move history to closed issues.
- Leaving a done task open, or two open decisions that contradict.
- Pasting an invariant's full text into an issue (it will drift from the doc) instead of linking.
- A secret value in any issue/comment (`DOCTRINE §8c`).
- Trusting an unverified issue claim over the bytes (`DOCTRINE §0`).

## 5. Issue taxonomy (labels)

- **Types:** `type:context` (pinned current state) · `type:task` · `type:decision` (ADR) · `type:discovery`/`type:pattern` (gotcha/lesson) · `type:blocker`.
- **Status:** `status:in-progress` · `status:blocked`.
- **Area:** per-engine (`area:aster`, `area:forge`, `area:assay`, `area:lodestar`, `area:ward`, `area:sextant`, `area:anneal`, `area:oracle`, `area:temporal`, `area:deploy`, …).
- **Priority:** `p0`–`p3`.

Decision/discovery bodies use the ADR / discovery templates (`AICodingAgentSuperPrompt.md` §3.10–§3.11): Context · Decision · Rationale · Alternatives · Consequences · Supersedes · References (decision); Signature · Cause · Workaround · Example · Where-it-bit · Frequency · Related (discovery).

## 6. How this composes

GitHub Issues are the *development-time* state surface; **Calyx's runtime state of truth is always the bytes on aiwonder** (`DOCTRINE §0`, `28 §5`) — issues track *what we are doing and have learned*, never substitute for reading the persisted SoT. The two stay consistent because every "done" is proven by FSV against the bytes and recorded in the closing comment.

**One sentence:** Calyx development state lives in GitHub Issues, with a small curated set of pinned `type:context` issues — mission/invariants, "you are here," environment, landmines, datasets — that every agent reads first and keeps in context, kept *current by editing to the new truth (never appending contradictions)* and *pruned every phase* (close done tasks, supersede old decisions, remove fixed gotchas) so a future agent opening them with a fresh context window sees a true, tight snapshot, never a confusing stale log.
