# Release Guide — socialmedia2.com

Operational checklist for shipping the `socialmedia2.com` static site (Cloudflare
Pages project `polis-socialmedia2`). Follow this every time you deploy the site so
SEO, indexing, and the soft-404 fix never regress.

> Scope: this covers the **front-end site release** (`site/landing/` → Cloudflare
> Pages) and its SEO/search-engine obligations. Backend (Rust crates), D1 schema,
> and FSV gates are governed separately — see `docs/deployment/socialmedia2-com.md`
> and `docs/POLIS.md`.

---

## 0. Concurrency caution (read first)

This repo is worked by **multiple agents sharing one `main` and one working tree**
(see memory `polis-swarm-and-env-quirks`). A `wrangler pages deploy` uploads your
**entire local bundle** and replaces production. To avoid clobbering concurrent
commits:

1. `git fetch origin && git merge --ff-only origin/main` (or rebase) so your tree
   includes everyone's latest committed work **before** you build.
2. Build from that integrated tree. Then a deploy preserves all committed work and
   only adds your changes.
3. Never build/deploy from a tree that still shows foreign uncommitted edits
   (`git status`) — you'd ship someone's half-finished work.

---

## 1. Regenerate SEO artifacts (idempotent — always run)

All three scripts are safe to re-run; they no-op when already correct and
automatically cover any newly-added pages.

```bash
./scripts/seo/rasterize_og.sh          # assets/og/*.svg -> *.png (1200x630) + default.png
python3 scripts/seo/apply_seo.py       # canonical, OG(PNG)+Twitter, BreadcrumbList, FAQPage, noindex
python3 scripts/seo/generate_sitemap.py # sitemap.xml (excludes noindex routes)
```

What they guarantee per page: canonical; full Open Graph + Twitter card with a
**PNG** `og:image` (1200×630) + `og:image:width/height/alt` + `og:locale`;
`BreadcrumbList` JSON-LD sitewide; `FAQPage` on `/faq/`; `noindex` on dev/gated
pages (`design-system/*`, `review/`, `observability/`, `pilot/*`, `account/*`).

Why PNG, not SVG: social scrapers (Facebook, LinkedIn, X, Slack, iMessage,
Discord, WhatsApp) do **not** render SVG `og:image` — they'd show no preview.

Review the diff: `git status` should only show the pages you actually changed
plus `sitemap.xml`.

---

## 2. Build the Cloudflare Pages bundle

```bash
bash scripts/deploy/build_cloudflare_pages_bundle.sh
# -> target/cloudflare-pages/socialmedia2.com/  (asset version = git short SHA)
```

---

## 3. Verify the built bundle (gate before deploy)

```bash
out=target/cloudflare-pages/socialmedia2.com
[ -f "$out/404.html" ] && echo "404.html OK"                       # soft-404 fix
[ -f "$out"/google*.html ] && echo "GSC verify file OK"            # keeps Search Console verified
ls "$out"/assets/og/*.png | wc -l                                  # OG PNGs present
grep -c "<loc>" "$out/sitemap.xml"                                 # sitemap URL count
grep -rl 'assets/og/[a-z-]*\.svg' "$out" --include=*.html | wc -l  # MUST be 0 (no SVG OG)
python3 - <<'PY'
import re,json,sys; from pathlib import Path
bad=0
for f in Path("target/cloudflare-pages/socialmedia2.com").rglob("*.html"):
    for m in re.finditer(r'<script type="application/ld\+json">(.*?)</script>', f.read_text(), re.S):
        try: json.loads(m.group(1))
        except Exception as e: bad+=1; print("BAD JSON-LD", f, e)
print("invalid JSON-LD blocks:", bad)   # MUST be 0
PY
```

**Critical invariant:** `site/landing/404.html` must exist. Without a `404.html`,
Cloudflare Pages reverts to SPA-fallback mode and serves `index.html` with **HTTP
200** for every unknown URL (a soft-404 that wastes crawl budget and harms
indexing). The file's presence is the documented signal that flips Pages to real
404 serving.

---

## 4. Deploy to production

```bash
npx wrangler pages deploy target/cloudflare-pages/socialmedia2.com \
  --project-name=polis-socialmedia2 --branch=main --commit-dirty=true
```

`--branch=main` is the production branch — this promotes to `socialmedia2.com`.
Any other branch creates a preview URL only.

Commit + push the source so the repo matches production (this repo commits
directly to `main`; fast-forward/rebase first if behind):

```bash
git add site/landing scripts/seo assets/og
git commit -m "…" && git pull --rebase --autostash origin main && git push origin main
```

---

## 5. Post-deploy live verification (source-of-truth)

Run from a tab **on** `https://socialmedia2.com/` (CSP `connect-src 'self'` allows
same-origin fetch), or curl from a network with real internet:

- `GET /this-page-does-not-exist/` → **404** (not 200) and serves the branded 404 page
- `GET /assets/og/home.png` → `content-type: image/png`
- `GET /sitemap.xml` → expected URL count, valid XML
- `GET /google<token>.html` → 200, body `google-site-verification: …`
- A content page's `og:image` resolves to a `…/assets/og/*.png` URL

---

## 6. Search-engine obligations on release

Account for all of these: **chrisroyseai@gmail.com**.

- **Google Search Console** (`https://socialmedia2.com/` URL-prefix property):
  on a significant content/URL change, resubmit `sitemap.xml` (Sitemaps →
  Submit) and confirm Status = **Success**. Use URL Inspection → Request Indexing
  for important new/changed pages.
- **Bing Webmaster Tools**: socialmedia2.com is imported from GSC; the sitemap
  syncs, but resubmit under Sitemaps if you add many pages.
- **Rich Results Test** (`search.google.com/test/rich-results`): after schema
  changes, confirm BreadcrumbList / FAQPage still validate.
- **Never delete** `site/landing/google<token>.html` — removing it un-verifies the
  Search Console property.

---

## 7. Do NOT add on release (governance / invariants)

These are deliberate omissions — don't "fix" them without clearing the gate:

- **Social `sameAs`** links in structured data or a public footer — blocked until
  each URL is independently verified per GitHub issue **#538**.
- **Google Analytics** or any engagement/tracking script — violates the privacy /
  anti-engagement invariants. Cloudflare Web Analytics (cookieless) only.
- **Google Business Profile** — Polis is ineligible (online-only org, no owned
  storefront); attempting it risks suspension.
- **`Person` schema on `/founder/`**, **`JobPosting`** without real listings,
  **`SearchAction`** without a public results URL — all would be inaccurate
  structured data.

---

# Part B — aiwonder ML processing plane

> Scope: this half covers the **backend ML compute** for `socialmedia2.com` — the
> 21-embedder TCT Constellation, reranking, vector generation, and weekly matching.
> All of this runs on **aiwonder** (the RTX 5090 / 32 GB box); **Cloudflare
> (D1 / R2 / Vectorize) stays storage-only**. Tracked by epic **#661** (issues
> #662–#820). Until those issues land this section is the **target-state contract**
> the implementation must satisfy — treat any deviation as a bug.

## 8. First principles (read before touching aiwonder)

1. **aiwonder is the processing plane, Cloudflare is the storage plane.** Embed /
   rerank / cross-terms / matching happen on aiwonder; results are shipped to
   Cloudflare and **nothing about a citizen is retained on aiwonder after a
   request** (see §13).
2. **Total isolation from everything else on that box.** aiwonder also runs
   Synapse / ContextGraph / ClipCannon work. socialmedia2.com gets its **own
   dedicated user, directory subtree, ports, tunnel, GPU MPS context, and
   systemd slice** — zero shared mutable state. Never write socialmedia2 data
   under `/zfs/archive/contextgraph/...` or any other project's tree.
3. **No LLM in the pipeline** (Invariant I15). Only frozen embedder/classifier
   instruments. The only AI involved is the coding AI, offline.
4. **FSV always.** After any action, read the source of truth (Vectorize / D1 /
   the box itself) to prove it worked. No mocks, no silent fallbacks.

## 9. The isolated on-box directory (everything for socialmedia2.com)

aiwonder creates **one self-contained subtree**, owned by a dedicated service
account `aiwonder-sm2`, separate from all other workloads:

```
/opt/aiwonder/socialmedia2/                 # <-- AIWONDER_ROOT; the ONLY sm2 footprint
├── releases/<short-sha>/                   # immutable release dirs (FastAPI+PyTorch service)
├── current -> releases/<short-sha>         # atomic symlink; rollback = repoint + restart
├── venv/                                    # dedicated Python venv (torch pinned for sm_120/Blackwell)
├── models/                                  # offline checkpoint store, read-only, model_sha-pinned
│   ├── s01-sem-self-e5-large-v2/ … s21-care-mode/
│   ├── search-embed/  rerank-bge/
│   └── MODELS.sha256                        # checksum ledger (verified at boot)
├── config/
│   ├── service.env                          # server runtime config (NO secret values; key-name refs)
│   ├── registry.yaml                        # per-slot: device/precision/batch/concurrency/warm/dim
│   └── tunnel/config.yml                    # cloudflared ingress -> 127.0.0.1:<port> (creds NOT in git)
├── run/                                     # tmpfs mount — ALL transient working data, wiped per request
├── logs/                                    # rotating, PII-scrubbed
├── telemetry/                               # prometheus + dcgm-exporter + otel-collector configs
└── state/                                   # operational state ONLY (warm receipts, model_sha ledger).
                                             #   NEVER any citizen data / constellation / raw intake.
```

**Isolation guarantees (all must hold):**

- Dedicated POSIX user/group `aiwonder-sm2`; `AIWONDER_ROOT` is `chown -R aiwonder-sm2` and `chmod 750`.
- Dedicated systemd slice `aiwonder-sm2.slice` with its own CPU/memory/GPU limits.
- Dedicated loopback port range **8730–8739** (service, metrics, collector).
- Dedicated **named Cloudflare Tunnel** (its own credentials), outbound-only.
- Dedicated NVIDIA MPS control/pipe dirs (no sharing with other GPU jobs).
- `run/` is **tmpfs** (`findmnt` must report `tmpfs`) — citizen inputs never hit disk.

## 10. Config files

### 10.1 Operator/control side — `~/.config/aiwonder.env`

Lives on the WSL dev/ops box (and is the contract the Cloudflare Worker mirrors via
bindings). **No secret values in git** — the real file is `chmod 600`; secret
*values* are pulled from Infisical at use time, referenced here by **key name only**.

```bash
# ~/.config/aiwonder.env  — control config for the socialmedia2 aiwonder plane
AIWONDER_ROOT=/opt/aiwonder/socialmedia2
AIWONDER_SSH=aiwonder-sm2@<AIWONDER_HOST>                 # ops shell into the isolated account
AIWONDER_INFER_BASE_URL=https://<TUNNEL_HOSTNAME>         # Access-protected tunnel hostname (#677/#678)
AIWONDER_HEALTH_URL=${AIWONDER_INFER_BASE_URL}/healthz
AIWONDER_READY_URL=${AIWONDER_INFER_BASE_URL}/readyz
AIWONDER_METRICS_URL=${AIWONDER_INFER_BASE_URL}/metrics
AIWONDER_TIMEOUT_MS=8000

# Secrets — KEY NAMES ONLY (values in Infisical /polis/socialmedia2_com):
CF_ACCESS_CLIENT_ID_KEY=aiwonder_cf_access_client_id
CF_ACCESS_CLIENT_SECRET_KEY=aiwonder_cf_access_client_secret
AIWONDER_INFER_API_KEY_KEY=aiwonder_inference_api_key
```

### 10.2 Server side — `/opt/aiwonder/socialmedia2/config/service.env`

```bash
AIWONDER_BIND=127.0.0.1:8730            # tunnel-only; never bind a public interface
AIWONDER_METRICS_BIND=127.0.0.1:8731
AIWONDER_MODELS=/opt/aiwonder/socialmedia2/models
AIWONDER_REGISTRY=/opt/aiwonder/socialmedia2/config/registry.yaml
AIWONDER_TMPDIR=/opt/aiwonder/socialmedia2/run     # tmpfs
AIWONDER_LOGDIR=/opt/aiwonder/socialmedia2/logs
AIWONDER_GPU=0
AIWONDER_DTYPE=fp8                      # fp8/int8 to fit the warm 21-slot slate in 32 GB
AIWONDER_WARM_REQUIRED=true            # /readyz fails until ALL configured models are warm
# secret VALUES injected at runtime from Infisical, never written here
```

### 10.3 Secrets — Infisical `/polis/socialmedia2_com` (key names)

`aiwonder_tunnel_token`, `aiwonder_cf_access_client_id`,
`aiwonder_cf_access_client_secret`, `aiwonder_inference_api_key`,
`aiwonder_hsm_key_ref`. **Never** commit values; **never** paste them into issues,
FSV docs, or logs.

## 11. Networking (Cloudflare Tunnel + Access)

- aiwonder runs `cloudflared` (outbound-only QUIC, **no inbound ports**), ingress
  maps `<TUNNEL_HOSTNAME>` → `127.0.0.1:8730` (#677).
- **Cloudflare Access** sits in front; only the Worker's **service token** and the
  operator may reach it (#678). The origin never sees unauthenticated traffic.
- The Worker authenticates with `CF-Access-Client-Id` / `CF-Access-Client-Secret`
  headers plus the `aiwonder_inference_api_key` bearer.

## 12. How to interact (commands)

```bash
set -a; . ~/.config/aiwonder.env; set +a
CID=$(infisical secrets get "$CF_ACCESS_CLIENT_ID_KEY" --plain --path=/polis/socialmedia2_com)
CSEC=$(infisical secrets get "$CF_ACCESS_CLIENT_SECRET_KEY" --plain --path=/polis/socialmedia2_com)
KEY=$(infisical secrets get "$AIWONDER_INFER_API_KEY_KEY" --plain --path=/polis/socialmedia2_com)
H=(-H "CF-Access-Client-Id: $CID" -H "CF-Access-Client-Secret: $CSEC" -H "Authorization: Bearer $KEY")

curl -s "${H[@]}" "$AIWONDER_HEALTH_URL"            # liveness
curl -s "${H[@]}" "$AIWONDER_READY_URL"             # warm/readiness — all 21 slots loaded?
curl -s "${H[@]}" "$AIWONDER_METRICS_URL" | grep -E 'slot_latency|gpu_|queue_depth'

# single-slot embed (FSV: verify dim + that the vector lands in Vectorize)
curl -s "${H[@]}" -X POST "$AIWONDER_INFER_BASE_URL/embed/s01" \
  -H 'content-type: application/json' -d '{"text":"hello"}' | jq '.dim, (.vector|length)'

# rerank
curl -s "${H[@]}" -X POST "$AIWONDER_INFER_BASE_URL/rerank" \
  -H 'content-type: application/json' -d '{"query":"...","docs":["..."]}'
```

Heavy onboarding constellation jobs are **not** called synchronously — the Worker
writes a row to the D1 jobs table and aiwonder polls/claims it (#681/#682), then
writes the 21 slots + provenance back to Cloudflare.

## 13. Lifecycle & ops

```bash
ssh "$AIWONDER_SSH"
sudo systemctl status  aiwonder-sm2-infer.service     # service health
sudo systemctl restart aiwonder-sm2-infer.service     # restart (re-warms; readyz gates traffic)
sudo systemctl stop    aiwonder-sm2-infer.service     # graceful drain then stop
journalctl -u aiwonder-sm2-infer -n 200 --no-pager    # PII-scrubbed logs
nvidia-smi                                            # GPU/VRAM/warm footprint
# rollback: repoint the atomic symlink + restart
sudo ln -sfn /opt/aiwonder/socialmedia2/releases/<prev-sha> /opt/aiwonder/socialmedia2/current
sudo systemctl restart aiwonder-sm2-infer.service
```

## 14. Zero-retention verification (FSV — run after any processing)

```bash
ssh "$AIWONDER_SSH"
findmnt /opt/aiwonder/socialmedia2/run                # MUST be tmpfs
ls -A /opt/aiwonder/socialmedia2/run                  # MUST be empty between requests
# state/ holds NO citizen data — only operational ledgers:
grep -rIl -E 'intake|constellation|email|@'/opt/aiwonder/socialmedia2/state 2>/dev/null   # MUST be empty
```

Then prove the result landed in the **storage plane** (the real source of truth):
`wrangler vectorize ...` (vector counts up by the expected delta) and the D1
provenance row exists for the forward-pass (#715). A green API response is **not**
proof — the stored vector is.

## 15. Issue cross-reference

Host/runtime #662–#664 · model registry/warm/store #665–#667, #673 · systemd #674 ·
tunnel/access/secrets #677, #678, #683 · D1 job queue #681, #682 · per-slot models
#686–#709 · provenance #715 · telemetry #719–#728 · zero-retention #734–#739 ·
per-citizen encryption #810–#812 · constellation-in-search #817.
