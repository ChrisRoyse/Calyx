# T-017 — Secrets wiring on aiwonder (Infisical HF token)

**Phase:** PH00 · **Dep:** T-002 · **Sudo:** no

## Objective
Make the HF token (and any future secret) available to Calyx processes on
aiwonder **without ever writing a value into the repo/issue/chat** — sourced from
a `0600` file / Infisical, loaded by `env.sh`.

## Preconditions
- T-002 (home), T-003 (`env.sh`). Infisical present (`~/.infisical`); HF cache
  present. Token value is in the Windows-side `.env` (gitignored).

## Steps
1. Create the secrets file on aiwonder (`0600`, outside the repo):
   ```bash
   mkdir -p ~/.config/calyx && chmod 700 ~/.config/calyx
   cat > ~/.config/calyx/secrets.env <<'EOF'
   export HF_TOKEN=__set_me__
   export HF_HUB_TOKEN=$HF_TOKEN
   EOF
   chmod 600 ~/.config/calyx/secrets.env
   # then set the real value with an editor or: read -s then write — never echo into history
   ```
   `env.sh` already sources this (T-003). **Prefer Infisical** where possible:
   `infisical run --env=prod -- <cmd>` so values stay in memory.
2. Confirm the token works without printing it:
   ```bash
   source /home/croyse/calyx/repo/env.sh
   python3 - <<'PY'
   import os, urllib.request
   tok=os.environ["HF_HUB_TOKEN"]; assert tok and tok!="__set_me__"
   req=urllib.request.Request("https://huggingface.co/api/whoami-v2",
        headers={"Authorization":f"Bearer {tok}"})
   print("hf auth:", urllib.request.urlopen(req).status)   # 200, no token printed
   PY
   ```
3. Verify the value is **not** in the repo: `git grep -i hf_ || echo clean`;
   `.gitignore` excludes `*.env`/secrets (it does).

## Deliverables
- `~/.config/calyx/secrets.env` (`0600`), sourced by `env.sh`; HF auth proven;
  no secret value in the repo.

## FSV gate
The HF whoami call returns **200** on aiwonder (token works) **without printing
the token**; `git grep` for the token prefix in the repo returns nothing (read
the result); `ls -l ~/.config/calyx/secrets.env` shows `0600`.

## Done
Secrets load via `env.sh`/Infisical; HF works; nothing leaked to VCS.

## Refs
PRD `16 §5b`, `28 §4`, DOCTRINE §8c.
