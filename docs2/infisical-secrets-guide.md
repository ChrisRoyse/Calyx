# Infisical secrets guide — AI-agent operational reference

> **Status as of 2026-06-06 (live vault readback):** authoritative
> operational guide for working with the Leapable secrets vault. Infisical
> (`leapable-aiwonder-prod`) is the single source of truth for every
> Leapable production secret. This doc is intended for **AI agents and
> operators** picking up the system fresh — it tells you what is in the
> vault, how to read it safely, who consumes each key, how to access it
> **from this Windows workstation** (§2.5), what **aiwonder** is (§10), and
> how to add/rotate/verify slots.
>
> **2026-06-06 reconciliation against the live `prod` environment (98 keys
> at `/`).** The catalog was re-verified by a key-name readback of the
> actual vault via the Infisical REST API as `chrisroyseai@gmail.com` (org
> **Leapable**). Values are intentionally never recorded here. Drift since
> the 2026-06-03 snapshot:
>
> - **`prod` is now 98 keys at the root path** (was 100). The two
>   Cloudflare **tunnel** slots are gone: `cf_legacy_tunnel_provisioning_token`
>   (the deprecated one the prior snapshot scheduled for removal — **removal
>   now complete**) and `cf_tunnel_id`. Tunnel provisioning runs entirely on
>   `cf_provisioner_api_token` now.
> - **New folder namespaces appeared** (secrets now also live under
>   sub-paths, not just `/`). `prod` has a `/polis` folder containing a
>   `socialmedia2_com` sub-folder; `dev` has a `/social-media-2` folder
>   holding a single `SM2_INFISICAL_README` marker. These look like a
>   separate "social-media-2 / polis" workstream staged in the same project.
>   Their contents are **not catalogued in §3** (which covers `prod` `/`);
>   treat them as out-of-scope until the operator confirms ownership.
> - **Access from this machine is now configured.** The Infisical CLI is
>   installed (winget, v0.43.91) and this Windows user is connected via a
>   persisted session token — see **§2.5** for how, the token expiry, and
>   how to refresh it.
> - Everything else from 2026-06-03 still holds — restated for the
>   fresh reader:
> - **Postgres/pgbouncer backend ("backenddb") slots are live and
>   loader-rendered** — `aiwonder_postgres_*` and `aiwonder_pgbouncer_*`
>   (URLs, per-role passwords, observer DSN). The loader materializes
>   **13** env files after Calyx wiring: `outbox-relay.env`,
>   `postgres-exporter.env`, `pgbouncer-exporter.env`, `pgbouncer.env`, and
>   `calyx.env` are the newer ones.
> - **Tauri updater signing keys** (`tauri_updater_signing_key`,
>   `tauri_updater_signing_key_password`, `tauri_updater_pubkey`) are
>   present and LIVE — pulled at build time by
>   `infra/aiwonder/bin/fetch-updater-signing-key.{sh,ps1}`.
> - **A second tenant shares this prod environment**: the `MF_*` +
>   `SLACK_WORKSPACE_ID` "marketing-fleet" keys (§3.2.2). They are **not**
>   consumed by anything in this repo.
> - **`cf_legacy_api_key` is NOT deprecated** — despite the name it is the
>   active scoped Cloudflare token (R2 + Pages + DNS) used by
>   `publish-sidecar.sh` / `publish-installer.sh` as `CLOUDFLARE_API_TOKEN`.
> - **`pushover_token` is still a `PENDING_FROM_PUSHOVER_APP` placeholder**;
>   only `pushover_user_key` is populated.
> - `dev` now holds 2 keys (`GEMINI_API_KEY`, `GEMINI_DEFAULT_MODEL`);
>   `staging` is still empty.
> - The Azure Trusted Signing `azure_*` slots (§3.2.1) are **still absent**
>   from the vault — confirmed PENDING, not yet provisioned.
>
> 2026-05-11 native tunnel note (still current): `PAGES_FN_SHARED_SECRET`,
> `cf_provisioner_api_token`, `cf_account_id`, and `cf_zone_id` are required
> live secrets for signed-in `/local/*` tunnel lookup and provisioning.
> `DASHBOARD_URL=https://leapable.ai` is intentionally **not** a secret; it
> is static marketplace config and must stay distinct from
> `PLATFORM_URL=https://marketplace.leapable.ai`.
>
> **Source-of-truth docs (read these for deep architecture).** Note: this
> guide lives in the **Calyx** repo; several referenced files live in the
> sibling **`leapablememory`** repo and are prefixed accordingly:
>
> - `docs/dbprdplans/16_AIWONDER_DEPLOYMENT.md` (**this repo**) — hardware
>   map, systemd, ZFS, GPU policy, and how `calyxd` deploys onto aiwonder.
> - `leapablememory/docs2/aiwonder-system.md §"Infisical secrets vault"` —
>   canonical IDs, bootstrap creds layout, REST/CLI/SDK examples, what lives
>   outside Infisical and why.
> - `leapablememory/docs/datacenterrefactor/16_secrets_and_credentials.md` —
>   full per-category breakdown of every live secret with usage notes,
>   rotation procedures, and known-compromised history.
>
> **This guide focuses on:** complete operational secret/config catalog with primary
> consumer mapping, AI-agent workflow, and findings (orphan slots in the
> vault that don't match the current architecture).

---

## 0. Hard rules (do not violate)

1. **Never paste live secret values** into this repo, into `memory/`, into
   `tmp/`, into chat, into commits, or into any docs. Use Infisical key
   names (e.g. `stripe_secret_key`) or the literal placeholder
   `<REDACTED:LABEL>`. The 2026-04-24 incident leaked 8+ live keys this
   way; pre-commit secret scan + history rewrite are now in place. See
   auto-memory `feedback_no_live_tokens_in_memory.md` and the repo
   pre-commit hook at `.githooks/pre-commit`.
2. **Live credentials reach this codebase only through the loader or
   the Infisical CLI/REST/SDK.** No env files committed, no
   `.env.example` with non-placeholder values, no hardcoded fallback
   defaults.
3. **The Universal Auth `client_secret` is chicken-and-egg.** It cannot
   live in Infisical itself. It lives at
   `/etc/leapable/secrets-loader.env` on aiwonder (mode `0400`,
   root-only) and in the operator's Bitwarden entry "Infisical
   aiwonder-prod loader". Nowhere else.
4. **Pre-commit secret scan is the safety net, not the policy.** The
   regex catches AKIA-style, sk*live*_, AIzaSy_, ghp\_\*, etc. — but it
   will not catch an unknown new vendor's format. Don't rely on it;
   use the placeholder convention.
5. **Exposed-values warning (2026-05-11).** Google OAuth, Supabase, Gemini,
   Fathom, beehiiv, Docker Hub, and a personal email password were pasted
   into an agent chat during live setup. The project-operational values were
   moved into Infisical; the providers should still rotate them after the
   login/account-sync path is fully verified. Do not reuse the personal
   email password or Docker PAT for the no-Docker end-user architecture.
6. **Pages HMAC secret must match in two places.** `pages_fn_shared_secret`
   in Infisical must equal the Cloudflare Pages function secret used by
   `leapable.ai/local/*`. A mismatch fails closed as `/local/health`
   `401/503`, depending on which side is missing. Verify by key-name
   readback only; never paste the value into docs or logs.
7. **Do not store non-secrets in Infisical.** `DASHBOARD_URL`,
   `PLATFORM_URL`, `LEAPABLE_BROWSER_TUNNEL_MODE_DEFAULT`, and
   `LEAPABLE_TUNNEL_PROVISIONING` are static runtime config. They belong in
   the aiwonder marketplace static env, not the secrets vault.

---

## 1. Project coordinates

These IDs are safe to record. Bootstrap creds (`client_id`, `client_secret`)
are not.

```text
Infisical site:           https://app.infisical.com
API base URL:             https://app.infisical.com/api   (US Cloud)
Org:                      Leapable (id 42ae02b8-1340-4f91-9268-125b1f540fdf)
Project name:             leapable-aiwonder-prod
Project id:               c2d7e44c-d7d1-4b27-aa23-2ed5a97fa0b5
Project slug:             leapable-aiwonder-prod-j-ejx
Environments:             prod (98 keys at /), dev (2 keys at /), staging (empty)
Secret path:              /   (plus newer folders: prod /polis, dev /social-media-2 — §intro)
Machine identity:         aiwonder-prod
Machine identity id:      <REDACTED:INFISICAL_MACHINE_ID>
Auth method:              Universal Auth (for aiwonder + automation)
Universal Auth client_id: <REDACTED:INFISICAL_UNIVERSAL_AUTH_CLIENT_ID>
Project role:             Admin
Org role:                 No Access  (least-privilege at org scope)
```

The three environment slugs are `dev`, `staging`, `prod` (their display
names are Development / Staging / Production). Pass the **slug** to the CLI
and REST API, not the display name.

**Universal Auth client_secret location:** `/etc/leapable/secrets-loader.env`
on aiwonder, mode `0400`, owner `root:root`, plus Bitwarden item
`Infisical aiwonder-prod loader`. Fetch it by SSH-ing to aiwonder as
root or by reading it from Bitwarden. On the operator's laptops/WSL the
same `INFISICAL_UA_CLIENT_ID` / `INFISICAL_UA_CLIENT_SECRET` /
`INFISICAL_PROJECT_ID` bootstrap lives in `~/.config/aiwonder.env`
(mode `0600`).

---

## 2. How AI agents access secrets

Choose the access mechanism by task:

| Task                                               | Mechanism                                                                                                                    | Why                                                                     |
| -------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| Run a Leapable systemd service that needs a secret | The loader has already rendered `/run/leapable/secrets/<file>.env`. Reference via `EnvironmentFile=` in the unit.            | Service code never calls Infisical; the loader handles auth + rotation. |
| One-off operator script on aiwonder                | `infisical run --projectId=… --env=prod -- <cmd>` (with loader-env sourced)                                                  | Injects every secret into the child process env; nothing touches disk.  |
| Build/release pipeline (CI / dev workstation)      | Infisical CLI or REST with a **separate** identity (machine identity per role), never the loader's identity.                 | Loader identity has Admin; CI should have its own narrower role.        |
| Ad-hoc lookup ("what's stripe_secret_key?")        | REST: `GET /api/v3/secrets/raw/<id>` via Universal Auth. Always over HTTPS.                                                  | Auditable in Infisical's Audit Logs tab.                                |
| Discovery ("what secrets exist?")                  | REST: `GET /api/v3/secrets/raw?workspaceId=…&environment=prod&secretPath=/`                                                  | Returns just the inventory metadata.                                    |
| Adding a new secret slot                           | Infisical web UI → `leapable-aiwonder-prod` → `prod` → New Secret. Then wire into `secrets-map.json` if the loader needs it. | Manual policy review; secrets are not auto-provisioned.                 |

> **Discovery safety note.** `infisical secrets … --plain` prints
> `KEY=VALUE` pairs — it leaks values. For a names-only inventory pipe
> through `cut -d= -f1`, and never write the raw output to `tmp/` or the
> repo. The 2026-06-03 readback that produced this catalog used
> `infisical secrets --env=prod --plain | cut -d= -f1` and shredded the
> value-bearing intermediate immediately.

### 2.1 Bash + curl — minimal recipe

```bash
INFISICAL_SITE=https://app.infisical.com
PROJECT_ID=c2d7e44c-d7d1-4b27-aa23-2ed5a97fa0b5
ENV=prod
SECRET_PATH=/

# Universal Auth → access token (5-min default TTL)
TOKEN=$(curl -fsS "$INFISICAL_SITE/api/v1/auth/universal-auth/login" \
  -H 'Content-Type: application/json' \
  -d "$(jq -cn --arg cid "$INFISICAL_UA_CLIENT_ID" --arg cs "$INFISICAL_UA_CLIENT_SECRET" \
        '{clientId:$cid,clientSecret:$cs}')" \
  | jq -er .accessToken)

# Fetch a single secret value
curl -fsS -G "$INFISICAL_SITE/api/v3/secrets/raw/stripe_secret_key" \
  -H "Authorization: Bearer $TOKEN" \
  --data-urlencode "workspaceId=$PROJECT_ID" \
  --data-urlencode "environment=$ENV" \
  --data-urlencode "secretPath=$SECRET_PATH" \
  | jq -r .secret.secretValue
```

### 2.2 Infisical CLI

```bash
# One-time install (Debian/Ubuntu; verified locally on WSL)
curl -1sLf 'https://artifacts-cli.infisical.com/setup.deb.sh' | sudo -E bash
sudo apt-get update
sudo apt-get install -y infisical
infisical --version  # 0.43.84 on the current WSL + aiwonder hosts

# Authenticate as the aiwonder-prod machine identity
# (on aiwonder: . /etc/leapable/secrets-loader.env ;
#  on a laptop/WSL: . ~/.config/aiwonder.env)
export INFISICAL_TOKEN=$(infisical login \
  --method=universal-auth \
  --client-id="$INFISICAL_UA_CLIENT_ID" \
  --client-secret="$INFISICAL_UA_CLIENT_SECRET" \
  --plain --silent)

# Names-only inventory (SAFE — never prints values)
infisical secrets --projectId="$INFISICAL_PROJECT_ID" --env=prod --plain | cut -d= -f1 | sort

# Inject every prod secret into a child process (DOES NOT WRITE TO DISK)
infisical run --projectId="$INFISICAL_PROJECT_ID" --env=prod -- env | grep -E '^stripe|^jwt|^redis'

# Get one secret value, plain text
infisical secrets get stripe_secret_key \
  --projectId="$INFISICAL_PROJECT_ID" --env=prod --plain
```

### 2.3 SDKs

| Language | Package            | Usage location in repo                                                                       |
| -------- | ------------------ | -------------------------------------------------------------------------------------------- |
| Bun/TS   | `@infisical/sdk`   | Not currently used at runtime — services consume env files. SDK is fine for one-off scripts. |
| Python   | `infisical-python` | Not currently used; CLI is preferred for ops scripts.                                        |
| Rust     | (none official)    | The installer does not call Infisical directly; build scripts shell out to the CLI.          |

Examples in `docs2/aiwonder-system.md §"SDK examples"`.

### 2.4 The aiwonder loader (preferred for services)

`leapable-secrets-load.service` runs at boot. It reads
`infra/aiwonder/secrets-loader/secrets-map.json`, authenticates as the
`aiwonder-prod` machine identity, fetches each named secret, and writes
the mapped env files under `/run/leapable/secrets/` at mode `0400` with the
owner declared in the map. Each service has
`EnvironmentFile=/run/leapable/secrets/<name>.env` in its systemd unit.

Loader source: `infra/aiwonder/secrets-loader/`. Validator:
`infra/aiwonder/bin/validate-infisical-secrets.sh` — map-driven; exits 0
only when every required loader secret is populated and non-placeholder.

**Restart sequence after a secret rotation:**

```bash
sudo systemctl restart leapable-secrets-load.service
sudo systemctl restart leapable-marketplace.service leapable-worker-embed.service leapable-worker-redact.service
# … plus any other dependent unit (see §4)
```

### 2.5 This Windows workstation (PowerShell) — current setup

This dev box (Windows 11, PowerShell) is **already connected** to the vault.
Here is exactly how, so it can be reproduced or refreshed.

**1. CLI install (done).** Installed with winget, not the Debian script:

```powershell
winget install --id infisical.infisical
infisical --version   # 0.43.91
```

winget adds the package dir
(`%LOCALAPPDATA%\Microsoft\WinGet\Packages\infisical.infisical_*\`) to the
**user PATH**, so `infisical` resolves in any *new* terminal. (In an
already-open shell that predates the install, call the full path or open a
fresh terminal.)

**2. Auth (done) — persisted user session token.** The normal
`infisical login` browser flow **cannot complete from a non-interactive /
agent shell**: it starts a localhost callback server that dies with
`Login via browser failed. The handle is invalid` when there is no real
console. So this box is instead connected via a persisted **user session
token** (a Google-auth JWT captured from the browser login callback), stored
as Windows **user environment variables**:

```text
INFISICAL_TOKEN        = <user session JWT>   # value never recorded here
INFISICAL_PROJECT_ID   = c2d7e44c-d7d1-4b27-aa23-2ed5a97fa0b5
INFISICAL_API_URL      = https://app.infisical.com/api
```

When `INFISICAL_TOKEN` is set, every `infisical …` command and REST call
authenticates with it directly — no `infisical login` needed.

> ⚠️ **This token expires 2026-06-16** (10-day user session, issued
> 2026-06-06). After that, commands return `401`. It does **not** auto-refresh.
> See "Refreshing / durable auth" below before relying on it long-term.

**3. Use it (PowerShell).** With the env vars set, in a fresh terminal:

```powershell
# Names-only inventory of prod (SAFE — strips values before printing)
infisical secrets --env=prod --projectId=$env:INFISICAL_PROJECT_ID --plain --silent |
  ForEach-Object { ($_ -split '=')[0] } | Sort-Object

# One secret value (prints the value — do NOT redirect to a repo/tmp file)
infisical secrets get stripe_secret_key --env=prod --projectId=$env:INFISICAL_PROJECT_ID --plain

# Inject every prod secret into a child process, nothing on disk
infisical run --env=prod --projectId=$env:INFISICAL_PROJECT_ID -- node .\some-script.js
```

Pure REST (no CLI) works the same way — `Authorization: Bearer $env:INFISICAL_TOKEN`:

```powershell
$h = @{ Authorization = "Bearer $env:INFISICAL_TOKEN" }
# names-only inventory
$r = Invoke-RestMethod -Headers $h -Method Get `
  "$env:INFISICAL_API_URL/v3/secrets/raw?workspaceId=$env:INFISICAL_PROJECT_ID&environment=prod&secretPath=/"
$r.secrets | ForEach-Object { $_.secretKey } | Sort-Object
```

**Refreshing / durable auth (when the session token expires).** Pick one:

- **Interactive refresh (simplest):** open a *real* terminal window (so the
  browser callback works) and run `infisical login` → choose *Infisical Cloud
  (US Region)* → approve in the browser. This stores a durable, auto-refreshing
  session in the OS keyring and supersedes the env-var token. Then you can
  clear `INFISICAL_TOKEN` from the user env if you prefer the keyring session.
- **Machine-identity (recommended for automation / unattended use):** use the
  `aiwonder-prod` Universal Auth identity (or a narrower new one) exactly like
  the loader does — set `INFISICAL_UA_CLIENT_ID` / `INFISICAL_UA_CLIENT_SECRET`
  and exchange them for a short-lived token:
  ```powershell
  $env:INFISICAL_TOKEN = infisical login --method=universal-auth `
    --client-id=$env:INFISICAL_UA_CLIENT_ID `
    --client-secret=$env:INFISICAL_UA_CLIENT_SECRET --plain --silent
  ```
  The UA `client_secret` is chicken-and-egg (Hard rule §0.3): it is **not** in
  the vault — it lives on aiwonder at `/etc/leapable/secrets-loader.env`, in
  Bitwarden (`Infisical aiwonder-prod loader`), and in the operator's
  `~/.config/aiwonder.env`. It is not currently present on this Windows box.

**Same Hard rules apply here (§0).** Never write a secret *value* into this
repo, `tmp/`, chat, or a committed file; `--plain` prints values, so pipe to
`($_ -split '=')[0]` for inventories. The token in `INFISICAL_TOKEN` is itself
a live credential — do not echo it, commit it, or paste it anywhere.

---

## 3. Complete secret/config catalog (live vault readback 2026-06-06)

Columns:

- **Secret id** — the Infisical key name (snake_case, case-sensitive).
- **Loader env var** — what the systemd loader exposes it as.
- **Primary consumer** — the consuming systemd unit / source file. The full
  loader → env-file → unit chain is in §4.
- **Status** — `LIVE` (currently used), `OPERATOR` (humans only), `BUILD/CI`
  (out-of-band release pipeline), `PENDING` (slot exists but value is a
  placeholder), `EXTERNAL` (a different tenant/app sharing this vault),
  `FUTURE` (provisioned for an integration not yet wired), `RETIRED`
  (orphan in vault, no code refs).

The `prod` environment holds **98 keys** at the root path `/` (down from
100 — the two `cf_*` tunnel slots were removed; see §3.2). The loader
consumes **39 secret ids** across **12 env files** (§3.1); the remainder are
read directly via CLI/REST, used by humans, by the release pipeline, or by
the marketing-fleet tenant (§3.2–§3.2.2). Newer `/polis` (prod) and
`/social-media-2` (dev) folders hold a separate workstream's secrets and are
**not** part of this `/`-path catalog (see the intro reconciliation).

### 3.1 Loader-consumed (39 secret ids → 12 env files)

Wired into `infra/aiwonder/secrets-loader/secrets-map.json` and
materialized at boot. `validate-infisical-secrets.sh` is map-driven: every
id below is exactly what it checks. Env file shown in the consumer column;
see §4 for the file → owner → unit table.

#### Stripe / auth / core marketplace (`marketplace.env`)

| Secret id                               | Loader env var                                                                        | Primary consumer                                                                                | Status |
| --------------------------------------- | ------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ------ |
| `stripe_secret_key`                     | `STRIPE_SECRET_KEY`                                                                   | `packages/marketplace/src/services/stripe/*.ts`                                                 | LIVE   |
| `stripe_webhook_secret`                 | `STRIPE_WEBHOOK_SECRET`                                                               | `routes/stripe-webhooks.ts` (comma-joined dual secret for overlapping endpoint-secret rotation) | LIVE   |
| `stripe_public_key`                     | `STRIPE_PUBLIC_KEY`                                                                   | dashboard JS (live mode)                                                                        | LIVE   |
| `stripe_connect_client_id`              | `STRIPE_CONNECT_CLIENT_ID`                                                            | `packages/marketplace/src/routes/connect.ts`                                                    | LIVE   |
| `jwt_signing_key`                       | `JWT_SIGNING_KEY` + `MARKETPLACE_JWT_SECRET`                                          | `packages/marketplace/src/auth.ts`, `session-mint.ts`                                           | LIVE   |
| `jwt_signing_key_next`                  | `JWT_SIGNING_KEY_NEXT`                                                                | `packages/marketplace/src/auth.ts` (rotation slot)                                              | LIVE   |
| `redis_password`                        | `REDIS_PASSWORD` (also `REQUIREPASS` in redis.env)                                    | `services/redis/*`, BullMQ workers, `redis-server.service`                                      | LIVE   |
| `runpod_api_token`                      | `RUNPOD_API_KEY` + `RUNPOD_API_TOKEN`                                                 | `packages/marketplace/src/runpod/*.ts`, `src/services/cloud-api/serverless.ts`                  | LIVE   |
| `cf_access_aud`                         | `CF_ACCESS_AUD`                                                                       | `packages/marketplace/src/middleware/cf-access.ts`                                              | LIVE   |
| `cf_access_issuer`                      | `CF_ACCESS_ISSUER`                                                                    | same                                                                                            | LIVE   |
| `cf_provisioner_api_token`              | `CF_API_TOKEN`                                                                        | `routes/install-tunnel.ts` creates/updates native Cloudflare tunnels                            | LIVE   |
| `cf_account_id`                         | `CF_ACCOUNT_ID`                                                                       | same                                                                                            | LIVE   |
| `cf_zone_id`                            | `CF_ZONE_ID`                                                                          | same                                                                                            | LIVE   |
| `pages_fn_shared_secret`                | `PAGES_FN_SHARED_SECRET`                                                              | `packages/dashboard/functions/local/[[path]].ts` ↔ marketplace `/v1/install/lookup-tunnel` HMAC | LIVE   |
| `hf_hub_token`                          | `HF_HUB_TOKEN` (also `HF_TOKEN`/`HF_HUB_TOKEN` in embed.env)                          | TEI containers + RunPod OCR image                                                               | LIVE   |
| `resend_api_key`                        | `RESEND_API_KEY`                                                                      | `packages/marketplace/src/services/email/*.ts`                                                  | LIVE   |
| `cloud_redact_worker_token`             | `CLOUD_REDACT_WORKER_TOKEN`                                                           | `src/bin-cloud-redact-worker.ts`, `routes/redact.ts`                                            | LIVE   |
| `google_oauth_client_id` (optional)     | `GOOGLE_OAUTH_CLIENT_ID`                                                              | `routes/auth-google.ts`, `/auth/google`                                                         | LIVE   |
| `google_oauth_client_secret` (optional) | `GOOGLE_OAUTH_CLIENT_SECRET`                                                          | same                                                                                            | LIVE   |
| `google_oauth_redirect_uri` (optional)  | `GOOGLE_OAUTH_REDIRECT_URI`                                                           | same; must exactly match Google console redirect URI                                            | LIVE   |
| `slack_ops_webhook` (optional)          | `REBRAND_WEEKLY_SLACK_WEBHOOK` (marketplace.env) + `SLACK_WEBHOOK` (alertmanager.env) | `leapable-rebrand-{weekly-metrics,daily-standup}.service` + Alertmanager Slack receiver         | LIVE   |

#### Postgres / pgbouncer backend ("backenddb" — new since 2026-05-11)

aiwonder central state is PostgreSQL only (vaults stay SQLite). These feed
the marketplace app, the outbox relay, pgbouncer userlist rendering, and
the Prometheus exporters.

| Secret id                                       | Loader env var                                              | Primary consumer (env file)                                        | Status |
| ----------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------ | ------ |
| `aiwonder_postgres_url`                         | `AIWONDER_POSTGRES_URL`                                     | marketplace + outbox-relay (`marketplace.env`, `outbox-relay.env`) | LIVE   |
| `aiwonder_pgbouncer_url`                        | `AIWONDER_PGBOUNCER_URL`                                    | marketplace + outbox-relay (pooled app DSN)                        | LIVE   |
| `aiwonder_pgbouncer_admin_url`                  | `AIWONDER_PGBOUNCER_ADMIN_URL`                              | marketplace pgbouncer admin console (`marketplace.env`)            | LIVE   |
| `aiwonder_postgres_migrator_url`                | `AIWONDER_DATABASE_MIGRATOR_URL`                            | schema migrations (`marketplace.env`)                              | LIVE   |
| `aiwonder_postgres_readonly_url`                | `AIWONDER_READONLY_DATABASE_URL`                            | read-replica / analytics queries (`marketplace.env`)               | LIVE   |
| `aiwonder_postgres_app_password`                | `PGBOUNCER_AIWONDER_APP_PASSWORD`                           | pgbouncer userlist render (`pgbouncer.env`)                        | LIVE   |
| `aiwonder_postgres_migrator_password`           | `PGBOUNCER_AIWONDER_MIGRATOR_PASSWORD`                      | same                                                               | LIVE   |
| `aiwonder_postgres_readonly_password`           | `PGBOUNCER_AIWONDER_READONLY_PASSWORD`                      | same                                                               | LIVE   |
| `aiwonder_pgbouncer_admin_password`             | `PGBOUNCER_AIWONDER_ADMIN_PASSWORD`                         | same                                                               | LIVE   |
| `aiwonder_postgres_observer_user`               | `DATA_SOURCE_USER`                                          | `leapable-postgres-exporter.service` (`postgres-exporter.env`)     | LIVE   |
| `aiwonder_postgres_observer_password`           | `DATA_SOURCE_PASS` + `PGBOUNCER_AIWONDER_OBSERVER_PASSWORD` | postgres-exporter + pgbouncer userlist                             | LIVE   |
| `aiwonder_postgres_observer_dsn_suffix`         | `DATA_SOURCE_URI`                                           | `leapable-postgres-exporter.service`                               | LIVE   |
| `aiwonder_pgbouncer_observer_connection_string` | `PGBOUNCER_EXPORTER_CONNECTION_STRING`                      | `leapable-pgbouncer-exporter.service` (`pgbouncer-exporter.env`)   | LIVE   |

#### Workers, backup, alerting

| Secret id                               | Loader env var                          | Primary consumer (env file)                                              | Status                                          |
| --------------------------------------- | --------------------------------------- | ------------------------------------------------------------------------ | ----------------------------------------------- |
| `ingest_bundle_ed25519_private_key_b64` | `INGEST_BUNDLE_ED25519_PRIVATE_KEY_B64` | `leapable-worker-ingest.service`, `src/workers/ingest.ts` bundle signing | LIVE                                            |
| `restic_repo_password`                  | `RESTIC_PASSWORD`                       | `leapable-restic.service` (`restic.env`)                                 | LIVE                                            |
| `restic_repo_path`                      | `RESTIC_REPOSITORY`                     | same                                                                     | LIVE                                            |
| `pushover_user_key` (optional)          | `PUSHOVER_USER_KEY`                     | Alertmanager Pushover receiver (`alertmanager.env`)                      | LIVE                                            |
| `pushover_token` (optional)             | `PUSHOVER_TOKEN`                        | same                                                                     | **PENDING** (value `PENDING_FROM_PUSHOVER_APP`) |

### 3.2 Read directly via Infisical CLI/REST (not loader-rendered)

Used at build time, in install/provisioning scripts, by humans, or by
ad-hoc validators. Not materialized into systemd env files.

#### Cloudflare (Access, tunnel, IdP, publishing)

| Secret id                                     | Used by                                                                                                                                                                                       | Status                               |
| --------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| `cf_legacy_api_key`                           | **Active R2 + Pages + DNS token** (name is misleading). Used as `CLOUDFLARE_API_TOKEN` by `publish-sidecar.sh` / `publish-installer.sh`. See memory `feedback_cf_legacy_api_key_is_r2_token`. | LIVE (BUILD/CI)                      |
| `cf_zone_name`                                | DNS-record provisioning in `install-tunnel.ts`                                                                                                                                                | LIVE                                 |
| `cf_team_domain`                              | CF Access policy lookups                                                                                                                                                                      | LIVE                                 |
| `cf_access_aud_embed`                         | `embed.leapable.ai` JWT-aud validator                                                                                                                                                         | LIVE                                 |
| `cf_access_aud_marketplace_finalize`          | Legacy `/v1/ocr/finalize/*` Access app AUD. Route now uses Access bypass + marketplace `X-License-Key` auth.                                                                                  | LIVE                                 |
| `cf_access_aud_ops`                           | `ops.leapable.ai` (Grafana) JWT-aud validator                                                                                                                                                 | LIVE                                 |
| `cf_access_client_id_mcp_cli_embed`           | Service token for MCP CLI → `embed.leapable.ai`; used by `validate-cloudflare-access.sh`                                                                                                      | LIVE                                 |
| `cf_access_client_secret_mcp_cli_embed`       | same — service-token secret                                                                                                                                                                   | LIVE                                 |
| `cf_access_client_id_runpod_ocr_finalize`     | Legacy/service-probe token for `marketplace.leapable.ai/v1/ocr/finalize*`; local MCP finalize does not depend on it.                                                                          | LIVE                                 |
| `cf_access_client_secret_runpod_ocr_finalize` | same — service-token secret                                                                                                                                                                   | LIVE                                 |
| `cf_idp_id_github`                            | CF Access GitHub IdP id, referenced when provisioning Access apps                                                                                                                             | LIVE                                 |
| `cf_idp_github_oauth_app_client_id`           | GitHub OAuth app backing CF Access GitHub IdP                                                                                                                                                 | LIVE                                 |
| `cf_idp_github_oauth_app_client_secret`       | same — OAuth secret (note: `github_leapable_cf_token` is a stale duplicate of this value — see §3.3)                                                                                          | LIVE                                 |
| `cf_policy_id_leapable_engineering`           | CF Access policy id for the `leapable-engineering` group                                                                                                                                      | LIVE                                 |

> **Removed since 2026-06-03:** `cf_tunnel_id` and
> `cf_legacy_tunnel_provisioning_token` (the latter was superseded by
> `cf_provisioner_api_token` and previously flagged for removal). Both are
> gone from the live `prod` vault as of 2026-06-06. Tunnel creation/update
> now runs entirely on `cf_provisioner_api_token` + `cf_account_id` +
> `cf_zone_id` + `cf_zone_name`.

#### Build / release pipeline (BUILD/CI)

| Secret id                            | Used by                                                                                                                                                  | Status          |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------- |
| `tauri_updater_signing_key`          | ed25519 Tauri-updater private key; pulled to `TAURI_SIGNING_PRIVATE_KEY` by `fetch-updater-signing-key.{sh,ps1}` for `tauri build`. Never lands on disk. | BUILD/CI        |
| `tauri_updater_signing_key_password` | passphrase for the above → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`                                                                                          | BUILD/CI        |
| `tauri_updater_pubkey`               | minisign public key baked into the installer `updater.pubkey`; verifies signed update artifacts                                                          | LIVE            |
| `docker_pat`                         | Docker Hub PAT for `docker login -u leapable` (RunPod OCR image builds + dev box). Not for end-user systems.                                             | BUILD/CI        |
| `npm_token`                          | `npm publish --access public` for `leapable-mcp`                                                                                                         | BUILD/CI        |
| `github_pat`                         | GitHub PAT for repo automation (release notes, branch ops, delta-bundle pushes)                                                                          | BUILD/CI        |
| `stripe_test_secret_key`             | Stripe **test** mode — dev/test scripts only; never read by prod runtime code                                                                            | LIVE (dev/test) |

#### Operator-only / external integrations (OPERATOR / FUTURE)

| Secret id                          | Used by                                                                                       | Status            |
| ---------------------------------- | --------------------------------------------------------------------------------------------- | ----------------- |
| `aiwonder_sudo_password`           | Operator sudo/SSH password for `croyse@aiwonder` — human only                                 | OPERATOR          |
| `supabase_project_id`              | Supabase BLTC project metadata; for management/API scripts if that integration is reactivated | OPERATOR / FUTURE |
| `supabase_project_name`            | Human-readable Supabase project label                                                         | OPERATOR / FUTURE |
| `supabase_url`                     | Supabase BLTC REST endpoint                                                                   | OPERATOR / FUTURE |
| `supabase_publishable_api_key`     | Browser/client-safe Supabase publishable key                                                  | OPERATOR / FUTURE |
| `supabase_secret_api_key`          | Supabase service/secret API key; server-side only                                             | OPERATOR / FUTURE |
| `supabase_legacy_jwt_secret`       | Supabase legacy JWT secret; server-side only                                                  | OPERATOR / FUTURE |
| `supabase_database_password`       | Supabase database password; server-side/operator only                                         | OPERATOR / FUTURE |
| `supabase_access_token`            | Supabase management access token; operator/automation only                                    | OPERATOR / FUTURE |
| `gemini_api_key_chrisleapable`     | Gemini API key for the `chrisleapable` Google project/account                                 | OPERATOR / FUTURE |
| `gemini_api_key_chris_leapable_ai` | Gemini API key for the `chris@leapable.ai` Google project/account                             | OPERATOR / FUTURE |
| `fathom_api_key`                   | Fathom analytics API integration                                                              | OPERATOR / FUTURE |
| `fathom_webhook_secret`            | Fathom webhook verification secret                                                            | OPERATOR / FUTURE |
| `beehiiv_sunday_api_key`           | beehiiv Sunday publication API integration                                                    | OPERATOR / FUTURE |
| `beehiiv_sunday_publication_id`    | beehiiv Sunday publication id                                                                 | OPERATOR / FUTURE |

> Note: the `dev` environment holds `GEMINI_API_KEY` and
> `GEMINI_DEFAULT_MODEL` (2 keys) for the local image-generation skill; the
> prod `gemini_api_key_*` slots above are distinct per-account keys.

### 3.2.1 Release signing — updater (live) + Azure Trusted Signing (pending)

The **Tauri updater** signing keys (`tauri_updater_signing_key`,
`tauri_updater_signing_key_password`, `tauri_updater_pubkey`) are present
and LIVE — see the BUILD/CI table in §3.2. They sign the auto-update
artifacts on every platform.

**Windows Authenticode signing via Azure Trusted Signing is still
pending.** Confirmed by the 2026-06-03 readback: **no `azure_*` keys exist
in the vault yet.** The default Windows build stays UNSIGNED and
byte-identical until the service principal + certificate profile are
provisioned (`packages/installer/build-windows.ps1` §1257, gated on
`LEAPABLE_SIGN_WINDOWS=1`). Safe Azure metadata is recorded here so future
operators don't have to rediscover it; the actual service-principal secrets
must live only in Infisical/Bitwarden/release-host env when created.

Known Azure resource state (operator-recorded, not vault keys):

```text
subscription id: f25939b2-d977-410c-8c27-3290928caca9
resource group:  leapable-rg
location:        East US
account URI:     https://eus.codesigning.azure.net/
sku:             Basic
directory label: Default Directory (chrisroyseaigmail.onmicrosoft.com)
status:          Active
```

| Slot id (planned)                | Build-env var passed to `build-windows.ps1`                                | Status                  |
| -------------------------------- | -------------------------------------------------------------------------- | ----------------------- |
| `azure_tenant_id`                | `AZURE_TENANT_ID`                                                          | PENDING — not in vault  |
| `azure_client_id`                | `AZURE_CLIENT_ID` (service-principal app id)                               | PENDING — not in vault  |
| `azure_client_secret`            | `AZURE_CLIENT_SECRET` (never echo/log)                                     | PENDING — not in vault  |
| `azure_trusted_signing_account`  | `LEAPABLE_AZURE_TS_ACCOUNT`                                                | PENDING — not in vault  |
| `azure_trusted_signing_profile`  | `LEAPABLE_AZURE_TS_PROFILE` (e.g. `leapable-release-windows`)              | PENDING — not in vault  |
| `azure_trusted_signing_endpoint` | `LEAPABLE_AZURE_TS_ENDPOINT` (default `https://eus.codesigning.azure.net`) | known: East US endpoint |

Windows public release remains blocked until: identity validation is
complete in Azure Trusted Signing; `azure_trusted_signing_profile` exists;
the service principal has `Trusted Signing Certificate Profile Signer` on
the certificate profile scope; and `Get-AuthenticodeSignature` /
`signtool verify /pa /v` reads back a valid signature from the built
installer artifact.

### 3.2.2 Marketing-fleet tenant (`MF_*` + `SLACK_WORKSPACE_ID`) — EXTERNAL

A separate "marketing-fleet" app shares this `prod` environment. **None of
these keys are consumed by anything in the `leapablememory` repo** (grep
finds only stale SITREP references). They drive an external Slack/Stripe
marketing bot that queries Leapable over MCP. Treat them as another
tenant's secrets: do not wire them into the loader, do not assume they
follow Leapable's rotation discipline, and consider moving them to their
own Infisical project/environment to restore single-tenant least-privilege.

| Secret id (group)                                                                                                                                                             | Purpose                                              | Status   |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- | -------- |
| `MF_BUILD_MODE`, `MF_ROCKSDB_PATH`                                                                                                                                            | marketing-fleet runtime config                       | EXTERNAL |
| `MF_LEAPABLE_MCP_COMMAND`, `MF_LEAPABLE_MCP_TRANSPORT`, `MF_LEAPABLE_SERVICE_URL`                                                                                             | how the bot reaches the Leapable MCP                 | EXTERNAL |
| `MF_GITHUB_TOKEN`                                                                                                                                                             | bot GitHub token                                     | EXTERNAL |
| `MF_SLACK_APP_ID`, `MF_SLACK_APP_LEVEL_TOKEN`, `MF_SLACK_BOT_TOKEN`, `MF_SLACK_CLIENT_ID`, `MF_SLACK_CLIENT_SECRET`, `MF_SLACK_SIGNING_SECRET`, `MF_SLACK_VERIFICATION_TOKEN` | Slack app credentials for the marketing bot          | EXTERNAL |
| `MF_SLACK_OPERATOR_ASK_CHANNEL`, `MF_SLACK_WORKSPACE_ID`, `MF_SLACK_WORKSPACE_URL`, `SLACK_WORKSPACE_ID`                                                                      | Slack workspace/channel ids                          | EXTERNAL |
| `MF_STRIPE_ACCOUNT_ID`, `MF_STRIPE_MODE`, `MF_STRIPE_PUBLISHABLE_KEY`, `MF_STRIPE_RESTRICTED_KEY`, `MF_STRIPE_SECRET_KEY`                                                     | marketing-fleet's **own** (test-mode) Stripe account | EXTERNAL |

### 3.3 Orphan / retired slots

These have **zero references in the live codebase**. Per the architecture
decisions, retired provider secrets must not remain as latent fallback
surfaces.

| Secret id                  | Notes                                                                                                                                                                                                            | Recommended action                          |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `b2_*` (5 slots)           | Backblaze B2 Litestream/off-host backup. **Confirmed deleted** — the 2026-06-03 readback shows no `b2_*`, `fly_*`, or `tigris_*` slots remain.                                                                   | DONE.                                       |
| `github_leapable_cf_token` | Still present. Its value **duplicates `cf_idp_github_oauth_app_client_secret`** (the GitHub OAuth app secret) and 401s on R2 (memory `feedback_cf_legacy_api_key_is_r2_token`). Unreferenced in code/docs/infra. | Verify with operator; delete the duplicate. |

**Why these matter:** retired secrets in a vault are dead weight at best
and an attack surface at worst (rotation discipline slips on slots nobody
is checking). The B2 slots are gone; `github_leapable_cf_token` is a stale
duplicate that should be confirmed and deleted.

### 3.4 Ingest worker signing key (provisioned 2026-05-11)

| Secret id                               | Loader file         | Status                                                                                                                                                   |
| --------------------------------------- | ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ingest_bundle_ed25519_private_key_b64` | `worker-ingest.env` | **LIVE**. Ed25519 private key, stored only in Infisical. The worker signs generated vault bundles and writes the public key into each `.signature.json`. |

This is the Spec 16 ingest worker dependency. If deleted or replaced with a
placeholder, `leapable-secrets-load.service`'s validator must fail with
`MISSING worker-ingest.env/INGEST_BUNDLE_ED25519_PRIVATE_KEY_B64`.

**Rotation procedure:**

```bash
# On an operator workstation
openssl genpkey -algorithm ed25519 -outform DER \
  | base64 -w0 \
  | tee /tmp/ingest-bundle-priv-b64.txt
# Upload the base64 value to Infisical as ingest_bundle_ed25519_private_key_b64.
# Do not commit the private key. Each produced bundle carries its public key
# in the signature artifact for downstream verification.
shred -u /tmp/ingest-bundle-priv-b64.txt
```

---

## 4. Loader → env file mapping (the materialized aiwonder state)

After `leapable-secrets-load.service` runs with the Calyx wiring installed,
`/run/leapable/secrets/` contains these **thirteen** files. Mode `0400` on
every one.

| File                     | Owner                   | Consumed by systemd unit(s)                                                                                              | Loader map source                          |
| ------------------------ | ----------------------- | ------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------ |
| `marketplace.env`        | `leapable:leapable`     | `leapable-marketplace.service`, `leapable-outbox-relay.service`, `leapable-rebrand-{weekly,daily}.service`, worker units | `secrets-map.json::marketplace.env`        |
| `embed.env`              | `leapable:leapable`     | `leapable-tei-general.service`, `leapable-tei-legal.service`, `leapable-tei-reranker.service` (install scripts)          | `secrets-map.json::embed.env`              |
| `worker-embed.env`       | `leapable:leapable`     | `leapable-worker-embed.service`, `leapable-marketplace.service`                                                          | `secrets-map.json::worker-embed.env`       |
| `worker-redact.env`      | `leapable:leapable`     | `leapable-worker-redact.service`, `leapable-worker-redact@.service`                                                      | `secrets-map.json::worker-redact.env`      |
| `worker-ingest.env`      | `leapable:leapable`     | `leapable-worker-ingest.service`                                                                                         | `secrets-map.json::worker-ingest.env`      |
| `outbox-relay.env`       | `leapable:leapable`     | `leapable-outbox-relay.service`                                                                                          | `secrets-map.json::outbox-relay.env`       |
| `postgres-exporter.env`  | `prometheus:prometheus` | `leapable-postgres-exporter.service`                                                                                     | `secrets-map.json::postgres-exporter.env`  |
| `pgbouncer-exporter.env` | `prometheus:prometheus` | `leapable-pgbouncer-exporter.service`                                                                                    | `secrets-map.json::pgbouncer-exporter.env` |
| `restic.env`             | `root:root`             | `leapable-restic.service`                                                                                                | `secrets-map.json::restic.env`             |
| `alertmanager.env`       | `prometheus:prometheus` | `prometheus-alertmanager.service`                                                                                        | `secrets-map.json::alertmanager.env`       |
| `redis.env`              | `redis:redis`           | `redis-server.service`                                                                                                   | `secrets-map.json::redis.env`              |
| `pgbouncer.env`          | `root:root`             | `leapable-render-pgbouncer-userlist.service` (renders the pgbouncer SCRAM userlist)                                      | `secrets-map.json::pgbouncer.env`          |
| `calyx.env`              | `leapable:leapable`     | `calyxd.service`; `calyx-aiwonder-healthcheck.sh`                                                                         | `secrets-map.json::calyx.env`              |

`zfs-archive.key` (per the ZFS-encryption work in
`docs/datacenterrefactor/zfs-archive-encryption-runbook.md`) will be added
here once that migration lands; the loader will need an entry binding
`keymaterial → zfs_archive_keymaterial`.

---

## 5. AI-agent task playbook

### 5.1 "What does secret X do?" — lookup

1. Find it in §3 above to confirm it's live.
2. Grep `grep -rln "<secret_id>" packages/ src/ infra/ scripts/ python/` to find the consumer file.
3. If 0 hits → it's in the orphan/retired list (§3.3), an external-tenant `MF_*` key (§3.2.2), or the catalog is stale; flag to the operator before assuming.

### 5.2 "I need to rotate secret X"

1. Generate a new value at the issuing provider (Stripe dashboard, Cloudflare console, etc.).
2. Update the value in Infisical web UI → `leapable-aiwonder-prod` → `prod` → `X` → Update.
3. On aiwonder, restart the loader + dependent services:
   ```bash
   sudo systemctl restart leapable-secrets-load.service
   sudo systemctl restart leapable-marketplace.service  # plus any other consumers (see §4)
   ```
4. Verify the new value is in place:
   ```bash
   sudo head -1 /run/leapable/secrets/<file>.env  # confirms re-render
   sudo infra/aiwonder/bin/validate-infisical-secrets.sh  # passes
   ```
5. Run the relevant service's health probe (e.g. `curl marketplace.leapable.ai/health`).
6. Old value at the provider: revoke after 24 h of green health.

For dual-key secrets (e.g. `jwt_signing_key` + `jwt_signing_key_next`):
write the new value to `_next`, deploy, wait for cache warm-up, then swap.

### 5.3 "I need to add a new secret X"

1. Decide: does the loader need to materialize it into a systemd env file?
   If yes:
   - Add to `infra/aiwonder/secrets-loader/secrets-map.json` under the
     right file. Use the object form `{"secret":"x","optional":false}`
     unless it really is optional.
   - Update the consuming systemd unit's `EnvironmentFile=` only if you
     created a new env file.
   - Update the relevant validator/install script.
2. Create the secret in Infisical web UI → `prod`.
3. Run `infra/aiwonder/bin/validate-infisical-secrets.sh` from aiwonder.
   It must exit 0.
4. Restart the loader + dependent services.
5. If the secret is **not** loader-rendered (operator/CI/ad-hoc), just
   create it in Infisical; document the consumer in §3.2 of this guide.
6. **Commit the catalog update.** This file (§3) needs to stay in sync.

### 5.4 "I need a secret value right now for a one-off script"

```bash
# On aiwonder (or anywhere with the loader's UA creds in ~/.config/aiwonder.env)
. /etc/leapable/secrets-loader.env  # or: . ~/.config/aiwonder.env
infisical run --projectId=c2d7e44c-d7d1-4b27-aa23-2ed5a97fa0b5 --env=prod -- ./your-script.sh
```

This injects every secret as an env var (`$stripe_secret_key`,
`$jwt_signing_key`, etc.) without writing anything to disk.

For a single value:

```bash
infisical secrets get stripe_secret_key \
  --projectId=c2d7e44c-d7d1-4b27-aa23-2ed5a97fa0b5 --env=prod --plain
```

### 5.5 "I need to verify aiwonder is in sync with Infisical"

```bash
sudo infra/aiwonder/bin/validate-infisical-secrets.sh
# Expected: PASS, every required key OK.
# pushover_token is currently a PENDING placeholder; it is mapped optional,
# so the validator reports it as pending/optional_skipped rather than failing.
# To require Pushover end-to-end once the token is real:
#   LEAPABLE_ALERTMANAGER_REQUIRE_PUSHOVER=1 infra/aiwonder/bin/validate-alertmanager-notifications.sh
```

The Infisical Audit Logs tab (web UI) shows the read events; if you ran the
validator at T, you should see N read events at ~T.

### 5.6 "I need to bootstrap a new operator or new AI-agent identity"

1. Web UI → Access Control → Identities → Create. Use Universal Auth.
   Give it the narrowest project role that works (Read-only for read-only
   agents; Admin only for the loader).
2. Copy the `client_id`. Generate a `client_secret` (single chance — save
   to Bitwarden immediately).
3. For an AI agent: store the creds in the agent's secret store. For a
   human: Bitwarden only.
4. Test:
   ```bash
   curl -fsS https://app.infisical.com/api/v1/auth/universal-auth/login \
     -H 'Content-Type: application/json' \
     -d '{"clientId":"…","clientSecret":"…"}' | jq .
   ```
   Returns `{"accessToken": …}` with 200.
5. Document the new identity (id + intended scope) in the operator notes —
   Bitwarden item or `~/.config/aiwonder.env` comments.

---

## 6. Findings as of 2026-06-03

Audit results from the live-vault readback that produced §3:

1. **Backenddb migration is fully reflected in the vault + loader.** All
   `aiwonder_postgres_*` / `aiwonder_pgbouncer_*` URLs and per-role
   passwords are present and loader-rendered into 4 new env files
   (`outbox-relay.env`, `postgres-exporter.env`, `pgbouncer-exporter.env`,
   `pgbouncer.env`). No action — documented in §3.1 / §4.
2. **`pushover_token` is still `PENDING_FROM_PUSHOVER_APP`.** Alerting via
   Pushover is not actually wired end-to-end. **Action: populate the real
   token from the Pushover app, then drop the `optional: true` flag in
   `secrets-map.json` and run the alertmanager-notifications validator.**
3. **`github_leapable_cf_token` is a stale duplicate** of
   `cf_idp_github_oauth_app_client_secret`, unreferenced, and 401s on R2.
   **Action: confirm with operator and delete.**
4. **`cf_legacy_api_key` is mislabeled but LIVE.** It is the active
   R2 + Pages + DNS publishing token (`CLOUDFLARE_API_TOKEN` in
   `publish-sidecar.sh` / `publish-installer.sh`), not a deprecated global
   key. **Action: rename in a future sweep for clarity; do NOT delete.**
   The genuinely deprecated CF slot `cf_legacy_tunnel_provisioning_token`
   (superseded by `cf_provisioner_api_token`) **has now been deleted**, along
   with `cf_tunnel_id` — the prod root dropped 100 → 98 keys. No action.
9. **New folder namespaces in the project (2026-06-06).** `prod` now has a
   `/polis` folder (with a `socialmedia2_com` sub-folder) and `dev` a
   `/social-media-2` folder (one `SM2_INFISICAL_README` marker). They appear
   to be a separate "social-media-2 / polis" workstream sharing this project.
   **Action: confirm ownership with the operator; if it is a distinct app,
   give it its own Infisical project/environment rather than nesting folders
   in `leapable-aiwonder-prod` (same least-privilege argument as the
   marketing-fleet tenant, finding 5).**
5. **Marketing-fleet (`MF_*`, 21 keys + `SLACK_WORKSPACE_ID`) shares the
   prod environment.** A second tenant's secrets live alongside Leapable's.
   **Action: consider moving them to their own Infisical project to restore
   single-tenant least-privilege; none are consumed by this repo.**
6. **Azure Trusted Signing `azure_*` slots do not exist yet.** Windows
   public release stays blocked on identity validation + service-principal
   provisioning (§3.2.1). The Tauri updater signing keys, by contrast, are
   present and LIVE.
7. **B2/Fly/Tigris retirement is complete in the vault.** No `b2_*`,
   `fly_*`, or `tigris_*` slots remain. No action.
8. **`stripe_test_secret_key` is alongside live `stripe_secret_key` in
   prod.** Intentional for test-mode dev scripts; confirmed no prod runtime
   path reads the test value.

---

## 7. Anti-patterns

| Don't                                                                                             | Why                                                                                                  | Do this instead                                                               |
| ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| Paste a live value into `memory/`, `tmp/`, or any committed file.                                 | Both dirs ship to GitHub history; pre-commit catches known patterns but not novel ones.              | Use `<REDACTED:LABEL>` or the Infisical key name.                             |
| Run `infisical secrets … --plain` and save the output.                                            | `--plain` prints `KEY=VALUE` — it dumps every value.                                                 | Pipe through `cut -d= -f1` for a names-only inventory; shred any temp.        |
| Read a secret with `cat /run/leapable/secrets/<file>.env` and copy it elsewhere.                  | Defeats the entire vault model.                                                                      | Use the loader or Infisical CLI in-process.                                   |
| Add a hardcoded fallback default for an "optional" secret (e.g. `STRIPE_KEY ?? 'sk_test_dummy'`). | Test-mode values leak into production code paths.                                                    | Fail closed: `if (!process.env.STRIPE_KEY) throw new ConfigError(…)`.         |
| Hand-edit `/run/leapable/secrets/*.env` on aiwonder to test something.                            | The loader overwrites on next restart; your "fix" disappears.                                        | Update Infisical, restart loader.                                             |
| Use the loader's `aiwonder-prod` machine identity for CI / dev / experiments.                     | Admin scope on the prod project; one leak compromises everything.                                    | Create a narrower-scope identity per use case.                                |
| Skip the validator after a rotation.                                                              | Silent partial failures (one service has the new key, another doesn't) are the worst case.           | `sudo infra/aiwonder/bin/validate-infisical-secrets.sh` after every rotation. |
| Wire an `MF_*` marketing-fleet key into the Leapable loader.                                      | They belong to a separate tenant sharing the vault; coupling them breaks the runtime split.          | Leave them read-direct; ideally move them to their own project.               |
| Reintroduce a retired provider's slot (Fly, Tigris, B2, SeaweedFS) "just in case".                | The retirement decision in `docs2/aiwonder-system.md §"Retired cloud service contract"` is explicit. | Use POSIX-on-ZFS + Litestream + restic per current architecture.              |

---

## 8. Cross-references

| Topic                                                                                 | Read                                                            |
| ------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| What aiwonder is + this project's deployment onto it                                   | §10 above; `docs/dbprdplans/16_AIWONDER_DEPLOYMENT.md` (this repo) |
| Access from this Windows workstation                                                   | §2.5 above                                                      |
| Deep architecture of the Infisical project                                            | `leapablememory/docs2/aiwonder-system.md §"Infisical secrets vault"` |
| Per-category secret breakdown + rotation procedures                                   | `docs/datacenterrefactor/16_secrets_and_credentials.md`         |
| What lives outside Infisical                                                          | Same, §7                                                        |
| Boot-time loading details                                                             | `16_secrets_and_credentials.md §4`                              |
| Per-service env files                                                                 | `16_secrets_and_credentials.md §5`                              |
| Loader source code                                                                    | `infra/aiwonder/secrets-loader/`                                |
| Loader map                                                                            | `infra/aiwonder/secrets-loader/secrets-map.json`                |
| Loader install script                                                                 | `infra/aiwonder/bin/install-secrets-loader.sh`                  |
| Loader validator                                                                      | `infra/aiwonder/bin/validate-infisical-secrets.sh`              |
| Updater signing-key fetch (build time)                                                | `infra/aiwonder/bin/fetch-updater-signing-key.{sh,ps1}`         |
| R2/Pages publishing (uses `cf_legacy_api_key`)                                        | `infra/aiwonder/bin/publish-sidecar.sh`, `publish-installer.sh` |
| Cloudflare Access validator                                                           | `infra/aiwonder/bin/validate-cloudflare-access.sh`              |
| Alertmanager delivery validation                                                      | `infra/aiwonder/bin/validate-alertmanager-notifications.sh`     |
| Operator onboarding checklist (Pushover + Cloudflare service tokens)                  | `docs/datacenterrefactor/operator-onboarding-checklist.md`      |
| Release-signing secrets (Apple/Azure/GPG/Tauri; tracked here but not loader-rendered) | `docs/datacenterrefactor/release-signing-runbook.md`            |

---

## 9. Glossary

- **Universal Auth** — Infisical's machine-identity auth method.
  `client_id` (safe to record) + `client_secret` (chicken-and-egg, lives on
  aiwonder at `/etc/leapable/secrets-loader.env`, in Bitwarden, and in the
  operator's `~/.config/aiwonder.env`).
- **Loader** — `leapable-secrets-load.service`. Reads Infisical at boot,
  writes the 12 `/run/leapable/secrets/*.env` files. Source:
  `infra/aiwonder/secrets-loader/`.
- **Loader map** — `infra/aiwonder/secrets-loader/secrets-map.json`. The
  contract between Infisical key ids and systemd-loaded env vars.
- **Backenddb** — the aiwonder PostgreSQL backend (central state). Reached
  through pgbouncer; vaults themselves stay SQLite.
- **Service token (CF Access)** — non-identity credential for Cloudflare
  Access. Used by RunPod callbacks (`cf_access_client_*_runpod_*`) and MCP
  CLI (`cf_access_client_*_mcp_*`).
- **Marketing-fleet** — an external tenant (`MF_*` keys) that shares this
  prod environment but is not part of the Leapable runtime.
- **Bootstrap creds** — the credentials needed to _reach_ Infisical or
  aiwonder. They cannot live in Infisical itself. Locations:
  `/etc/leapable/secrets-loader.env` (on aiwonder, root-only) and
  `~/.config/aiwonder.env` (on the operator's laptop, mode `0600`).

---

## 10. aiwonder — what an AI agent needs to know

This vault exists to serve one machine. If you are an agent picking up this
system, read this before touching anything. Authoritative deployment detail:
`docs/dbprdplans/16_AIWONDER_DEPLOYMENT.md` (this repo) and
`leapablememory/docs2/aiwonder-system.md` (deep architecture). The summary
below is grounded in those + the 2026-06-05/06 live readbacks.

### 10.1 What "aiwonder" is

**aiwonder is a single, self-hosted datacenter box** — the one physical
machine that runs all of Leapable's production services. It is **not** a
cloud account, a cluster, or an abstraction; it is one Ubuntu host on a
VPN-only network. The retired-cloud posture is deliberate: no Fly, Tigris,
B2, S3, or SeaweedFS — storage is **POSIX-on-ZFS** on local disks, and
whole-host loss is an accepted risk mitigated by WAL + ZFS snapshots +
restic, not by HA.

| Resource (live) | Role |
|---|---|
| Ryzen 9 9950X, 16c/32t | app + workers, ingest batching, CPU SIMD fallback, graph/background jobs |
| 128 GB DDR5 (~84 GiB free) | memtables, in-RAM indexes, host buffers |
| RTX 5090 32 GB (Blackwell, `sm_120`) | embeddings (resident TEI) + GPU compute; **shared under a soft VRAM budget**, 600 W cap (`leapable-gpu-max-power.service`) |
| `hotpool` NVMe ~1.5 TB → `/zfs/hot` | WAL, hot data, active indexes, online state. **No redundancy.** |
| `archive` HDD mirror ~8.5 TB → `/zfs/archive` | cold data, retired slots, restic backup target |
| PostgreSQL 18.4 + PgBouncer | **the control plane** — customers, billing, creators, queries, outbox |

GPU hygiene gotcha: after unattended-upgrades, the NVIDIA driver/userspace
can skew and `nvidia-smi` mismatches until a **reboot**. CUDA is 13.2 from
the NVIDIA runfile (not the Ubuntu package). Health checks must probe CUDA
init and **fail loud**, never silently CPU-fallback in server mode.

### 10.2 Reaching the box

SSH/VPN bootstrap creds live **only** in `~/.config/aiwonder.env` on the
operator machine (mode `0600`, outside any repo): `AIWONDER_HOST` / `IP` /
`SSH_PORT` / `USER`, `AIWONDER_SSH_PASSWORD`, `AIWONDER_SUDO_PASSWORD`, VPN
creds, and the Infisical UA bootstrap (`INFISICAL_UA_CLIENT_ID` /
`INFISICAL_UA_CLIENT_SECRET` / `INFISICAL_PROJECT_ID`). **This file is not
present on this Windows workstation** — only the operator's primary
machine/aiwonder have it. Everything beyond bootstrap is in Infisical.

- VPN-only; UFW default-deny; SSH only from the current subnet.
- **Never** change UFW/sshd without a second live session open (lockout risk).
- **Never** copy a value from `aiwonder.env` (or any secret) into a repo,
  issue, PR, or chat — reference env-var **names** only (Hard rule §0).

### 10.3 How the box is organized (so secrets make sense)

- **Everything is a `leapable-*` systemd unit.** Services run as the
  `leapable` user from `/opt/leapable/…`, bind **loopback only**, and get
  their config from `EnvironmentFile=/run/leapable/secrets/<file>.env`.
- **`leapable-secrets-load.service`** is the bridge between this vault and
  the running box: at boot it authenticates as the `aiwonder-prod` machine
  identity, reads `secrets-map.json`, and renders the mapped `*.env` files
  under `/run/leapable/secrets/` (mode `0400`). Service code never calls
  Infisical directly — see §2.4 and §4.
- **Ingress is Cloudflare Tunnel + Caddy** in front of the loopback
  services. Public hostnames: `marketplace.leapable.ai` (app),
  `embed.leapable.ai` (embeddings), `ops.leapable.ai` (Grafana),
  `leapable.ai` (dashboard / Cloudflare Pages). `mcp.leapable.ai` is
  intentionally **503** (no per-user public MCP).
- **Resident GPU workloads:** three TEI containers (general / legal /
  reranker) + dcgm-exporter are always up. Do **not** start throwaway
  TEI/Redis/cloudflared — reuse the resident services.
- **Observability:** Prometheus + Grafana + Alertmanager; restic timer +
  ZFS auto-snapshots for backup; single-host, no off-machine replica today.

### 10.4 The PostgreSQL control plane is off-limits

PostgreSQL 18.4 + PgBouncer on the box is the **control plane** and stays
**untouched** by new workloads. The `aiwonder_postgres_*` / `aiwonder_pgbouncer_*`
secrets in this vault (§3.1) belong to the existing marketplace app and its
exporters. A new service must not connect to PostgreSQL unless that is its
explicit, reviewed job.

### 10.5 "This project on aiwonder" — Calyx / `calyxd`

This repo (**Calyx**) is an association database that deploys **onto**
aiwonder as `calyxd`, alongside — not replacing — the existing stack:

- **Shape:** a loopback `calyxd.service` (systemd) running as `leapable`
  from `/opt/leapable/calyx/`. There is **no `rustc` on the box**, so
  `calyxd` ships as a **cross-built static binary + `.deb`** (cross-built on
  aiwonder or another host), synced to `/opt/leapable/calyx/`. Do not assume
  host `cargo`.
- **Storage:** Aster on ZFS — `/zfs/hot/calyx` (WAL, hot slots, ANN graphs)
  and `/zfs/archive/calyx` (cold sidecars, restic source). Stage temp files
  *inside* the destination dataset to avoid `EXDEV` on rename.
- **GPU:** the Forge math runtime shares the RTX 5090 with the resident TEI
  lenses under the **soft VRAM budget** and yields to them; target `sm_120`
  cubin + PTX JIT fallback.
- **Control plane:** `calyxd` **does not connect to PostgreSQL.** It hosts
  published/Discover Vaults; the PostgreSQL control plane is left untouched.
- **Secrets Calyx needs from this vault:** today, effectively **one** —
  `hf_hub_token` (`HF_HUB_TOKEN` / `HF_TOKEN`) to pull/host embedder models
  and gated HF datasets. It is already live in the vault (§3.1). `calyxd`
  gets its own rendered `calyx.env` (the 13th loader file) via
  `infra/aiwonder/secrets-loader/calyx.env.map.json`, installed by
  `infra/aiwonder/bin/install-calyx-deploy-wiring.sh`. Add any future token
  (e.g. Kaggle) via the CLI (`infisical secrets set kaggle_key=…`) — never a
  new value in the repo.

### 10.6 Binding operating rules on the box

1. **Verify the source of truth after every op** — `systemctl`, `zpool
   status`, `nvidia-smi`, actual on-disk bytes. A `200`/return value is a
   claim, not proof.
2. **Fail closed.** No fallback that hides a failure (no `?? 'dummy'`
   defaults, no silent CPU fallback in server mode).
3. **Reuse resident services**; don't spin up throwaway TEI/Redis/cloudflared.
4. **Don't reintroduce retired infra** (Fly/Tigris/B2/S3/SeaweedFS) — Aster
   is POSIX-on-ZFS only.
5. **Production synthetic test data** must be cleanup-tagged and provably
   inert before the turn ends.
