# T-007 — Create the five pinned `type:context` issues

**Phase:** PH02 · **Dep:** T-006 · **Sudo:** no

## Objective
Create the small, curated, **pinned** `type:context` issues every agent reads at
the start of every turn — the shared working memory. Snapshots (pointers to
docs), not journals.

## Preconditions
- T-006 (repo + labels).

## Steps
Create + pin five issues (bodies = short pointers + a last-verified stamp; never
paste invariant text — link it):

1. **`[CONTEXT] Mission & invariants`** — thesis pointer (`docs/dbprdplans/00`,
   `DOCTRINE`); scope (universal DB + AGI; Leapable = Vault-only, PostgreSQL
   untouched); link axioms A1–A34 (don't duplicate).
2. **`[CONTEXT] You are here`** — current phase = **Stage 0 / PH00–PH04**;
   what's done / in-flight / next; the one or two things that matter now.
3. **`[CONTEXT] Environment & ops`** — everything on aiwonder
   (`docs/implementation/01`); reach via the VPN + `.env` askpass; `CALYX_HOME`;
   reuse TEI :8088/:8089/:8090; secrets via Infisical/`secrets.env`.
4. **`[CONTEXT] Landmines`** — the gotchas that bite every agent:
   - **no passwordless sudo** for croyse → ZFS/systemd/apt are operator-gated
   - **Rust IS installed (rustup)** — PRD's "no rustc on box" is superseded;
     build natively; `source ~/.cargo/env`
   - current user is **croyse**, not `leapable`; no `/opt/leapable/calyx`
   - ≤500-line rule; FSV reads bytes; never secret values in issues; dedup never
     merges conflicting anchors; do not touch leapable/contextgraph/postgres
   - `cmake`/`protoc` were missing → installed in userspace (T-004)
5. **`[CONTEXT] Datasets`** — pointer to `datasets/MANIFEST.md` (Stage 18); which
   real datasets are acquired/verified on aiwonder; what's still needed.

```bash
gh issue create --repo chrisroyse/calyx --label type:context \
  --title "[CONTEXT] Landmines" --body "$(cat body.md)"
# then pin each via the GitHub UI/API
```

## Deliverables
- Five pinned `type:context` issues, each short, pointer-based, last-verified
  stamped.

## FSV gate
`gh issue list --repo chrisroyse/calyx --label type:context` returns **exactly
five**, all pinned; each body links docs (no duplicated invariant text); the
read-state query (PRD `29 §3`) surfaces them.

## Done
The five context issues exist, are pinned, and are accurate to the current
(verified) state.

## Refs
PRD `29 §2/§3`, DOCTRINE §8d.
