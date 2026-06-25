#!/usr/bin/env bash
# Provision the website Calyx/Aster vault with a diverse multi-lens panel and
# ingest the Calyx documentation corpus, then prove it is searchable.
#
# Doctrine (multi-lens): >=4 diverse, low-correlation lenses; never single.
# This panel uses 5 fundamentally different signal types:
#   - semantic-gte         tei-http :8088  Dense(768)     neural semantic
#   - semantic-modernbert  tei-http :8090  Dense(768)     neural semantic (other family)
#   - keyword-sparse       algorithmic     Sparse(30522)  lexical / keyword
#   - byte-lexical         algorithmic     Dense(16)      byte signal
#   - token-multi          algorithmic     Multi          token multi-vector
#
# Usage: provision-website-vault.sh [vault-name]
set -euo pipefail
cd "$(dirname "$0")/../.." 2>/dev/null || cd /home/croyse/calyx/repo
source ~/calyx_env.sh 2>/dev/null || true
BIN=target/release/calyx
NAME="${1:-website-calyx}"
MAX_CHARS=6000   # keep chunks safely under the embedder's 8192-token window

V=$("$BIN" create-vault "$NAME" | python3 -c "import json,sys;print(json.load(sys.stdin)['vault_id'])")
echo "vault_id=$V"

# Bind the 5-lens panel (slots 8..12 on top of the default text-default template).
"$BIN" add-lens "$V" --name semantic-gte        --runtime tei-http --endpoint http://127.0.0.1:8088 --shape "Dense(768)"   --modality text
"$BIN" add-lens "$V" --name semantic-modernbert --runtime tei-http --endpoint http://127.0.0.1:8090 --shape "Dense(768)"   --modality text
"$BIN" add-lens "$V" --name keyword-sparse      --runtime "algorithmic:sparse-keywords" --shape "Sparse(30522)" --modality text
"$BIN" add-lens "$V" --name byte-lexical        --runtime "algorithmic:byte-features"   --modality text
"$BIN" add-lens "$V" --name token-multi         --runtime "algorithmic:token-hash"      --modality text

# Retire the unbound template slots so the panel is exactly the 5 selected lenses.
for s in 0 1 2 3 4 5 6 7; do "$BIN" retire-lens "$V" --slot "$s" >/dev/null; done

# Build a chunked corpus from the Calyx docs (chunk on blank-line boundaries,
# hard-split oversized paragraphs). Chunking — not truncation — keeps all content
# and stays under the embedder token limit (fail-loud guard rejects oversize).
python3 - "$MAX_CHARS" > /tmp/website_corpus.jsonl <<'PYEOF'
import json, glob, re, sys
MAX = int(sys.argv[1])
def chunk(txt, maxc=MAX):
    txt = txt.strip()
    if len(txt) <= maxc: return [txt] if txt else []
    out, cur = [], ""
    for p in re.split(r"\n\s*\n", txt):
        if len(p) > maxc:
            if cur: out.append(cur); cur = ""
            out += [p[i:i+maxc] for i in range(0, len(p), maxc)]; continue
        if len(cur)+len(p)+2 > maxc:
            if cur: out.append(cur)
            cur = p
        else: cur = (cur+"\n\n"+p) if cur else p
    if cur: out.append(cur)
    return [c for c in out if c.strip()]
for path in sorted(glob.glob("docs/systemspecs/*.md")) + sorted(glob.glob("dbprdplans/*.md")):
    cs = chunk(open(path, encoding="utf-8").read())
    for i, c in enumerate(cs):
        print(json.dumps({"text": c, "source": f"{path}#chunk{i}" if len(cs) > 1 else path}))
PYEOF
echo "corpus rows: $(wc -l < /tmp/website_corpus.jsonl)"

"$BIN" ingest "$V" --batch /tmp/website_corpus.jsonl --idempotent | tail -1
"$BIN" healthcheck --vault "$V"
echo "provisioned vault_id=$V"
