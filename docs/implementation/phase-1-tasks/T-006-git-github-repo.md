# T-006 — Create git repo + push + GitHub repo + labels

**Phase:** PH02 · **Dep:** T-002 · **Sudo:** no

## Objective
Stand up `chrisroyse/calyx` as the code + dev-state surface, with the planning
docs committed and the issue taxonomy created.

## Preconditions
- T-002 (home). `gh` present + authed on aiwonder (verify `gh auth status`;
  authenticate if needed). The Calyx planning docs (this `docs/` tree) and the
  `.gitignore` already exist on the Windows authoring box — bring them over.

## Steps
1. **Repo on aiwonder** under `CALYX_HOME/repo`:
   ```bash
   source /home/croyse/calyx/repo/env.sh
   cd /home/croyse/calyx/repo && git init -b main
   # copy in: docs/ (dbprdplans + implementation), docs2/, .gitignore, .env.example
   git add -A && git commit -m "Calyx: PRD + implementation plan + scaffolding"
   ```
   Confirm `.gitignore` excludes `.env`/secrets/`target/`/data (it does).
2. **GitHub repo** (private):
   ```bash
   gh repo create chrisroyse/calyx --private --source=. --remote=origin --push
   ```
3. **Labels** (taxonomy from PRD `29 §5`):
   ```bash
   for l in type:context type:task type:decision type:discovery type:blocker \
            status:in-progress status:blocked p0 p1 p2 p3 \
            area:aster area:forge area:registry area:sextant area:loom area:assay \
            area:lodestar area:ward area:ledger area:anneal area:oracle area:temporal \
            area:universal area:resource area:security area:deploy area:mcp area:cli; do
     gh label create "$l" --repo chrisroyse/calyx 2>/dev/null || true; done
   ```
4. Verify the push + labels.

## Deliverables
- `chrisroyse/calyx` on GitHub with the planning docs committed and the full
  label taxonomy.

## FSV gate
`gh repo view chrisroyse/calyx` shows the repo; `git ls-files` includes the docs
but **not** `.env`/`target`/`data` (read the list); `gh label list` shows the
taxonomy. Clone fresh on aiwonder → matches.

## Done
Repo live, planning docs in, labels created, secrets excluded.

## Refs
PRD `29`, DOCTRINE §8d, `../02_WORKING_AGREEMENT.md §7`.
