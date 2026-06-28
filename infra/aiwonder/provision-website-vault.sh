#!/usr/bin/env bash
# Provision the website Calyx/Aster vault with a diverse multi-lens panel and
# ingest the Calyx documentation corpus, then prove it is searchable.
#
# Doctrine (multi-lens): >=4 diverse, low-correlation lenses; never single.
# This panel uses 5 fundamentally different signal types:
#   - semantic-bge-m3      tei-http :18188 Dense(1024)    neural semantic + BGE family
#   - semantic-e5          tei-http :18190 Dense(768)     neural semantic + E5 family
#   - keyword-sparse       algorithmic     Sparse(30522)  lexical / keyword
#   - byte-lexical         algorithmic     Dense(16)      byte signal
#   - token-multi          algorithmic     Multi          token multi-vector
#
# The dense TEI ports are Calyx-owned user-systemd services. Do not point this
# vault at Leapable-owned :8088/:8090 containers; those are separate tenants.
#
# Usage: provision-website-vault.sh [vault-name]
set -euo pipefail
cd "$(dirname "$0")/../.." 2>/dev/null || cd /home/croyse/calyx/repo
source ~/calyx_env.sh 2>/dev/null || true
BIN=target/release/calyx
NAME="${1:-website-calyx}"
MAX_CHARS=900    # keep chunks safely under the E5 512-token window
MAX_WORDS=260

V=$("$BIN" create-vault "$NAME" | python3 -c "import json,sys;print(json.load(sys.stdin)['vault_id'])")
echo "vault_id=$V"

# Bind the 5-lens panel (slots 8..12 on top of the default text-default template).
"$BIN" add-lens "$V" --name semantic-bge-m3 --runtime tei-http --endpoint http://127.0.0.1:18188 --shape "Dense(1024)"  --modality text
"$BIN" add-lens "$V" --name semantic-e5     --runtime tei-http --endpoint http://127.0.0.1:18190 --shape "Dense(768)"   --modality text
"$BIN" add-lens "$V" --name keyword-sparse  --runtime "algorithmic:sparse-keywords" --shape "Sparse(30522)" --modality text
"$BIN" add-lens "$V" --name byte-lexical    --runtime "algorithmic:byte-features"   --modality text
"$BIN" add-lens "$V" --name token-multi     --runtime "algorithmic:token-hash"      --modality text

# Retire the unbound template slots so the panel is exactly the 5 selected lenses.
for s in 0 1 2 3 4 5 6 7; do "$BIN" retire-lens "$V" --slot "$s" >/dev/null; done

# Build a chunked corpus from the Calyx docs (chunk on blank-line boundaries,
# hard-split oversized paragraphs). Chunking — not truncation — keeps all content
# and stays under the embedder token limit (fail-loud guard rejects oversize).
python3 - "$MAX_CHARS" "$MAX_WORDS" > /tmp/website_corpus.jsonl <<'PYEOF'
import json, glob, re, sys
MAX_CHARS = int(sys.argv[1])
MAX_WORDS = int(sys.argv[2])

def split_words(txt):
    words = txt.split()
    out, cur, cur_chars = [], [], 0
    for word in words:
        add_chars = len(word) + (1 if cur else 0)
        if cur and (len(cur) >= MAX_WORDS or cur_chars + add_chars > MAX_CHARS):
            out.append(" ".join(cur))
            cur, cur_chars = [word], len(word)
        else:
            cur.append(word)
            cur_chars += add_chars
    if cur:
        out.append(" ".join(cur))
    return out

def chunk(txt):
    txt = txt.strip()
    if not txt:
        return []
    out, cur, cur_words, cur_chars = [], [], 0, 0
    for p in re.split(r"\n\s*\n", txt):
        p = p.strip()
        if not p:
            continue
        p_words = len(p.split())
        if len(p) > MAX_CHARS or p_words > MAX_WORDS:
            if cur:
                out.append("\n\n".join(cur))
                cur, cur_words, cur_chars = [], 0, 0
            out.extend(split_words(p))
            continue
        add_chars = len(p) + (2 if cur else 0)
        if cur and (cur_chars + add_chars > MAX_CHARS or cur_words + p_words > MAX_WORDS):
            out.append("\n\n".join(cur))
            cur, cur_words, cur_chars = [p], p_words, len(p)
        else:
            cur.append(p)
            cur_words += p_words
            cur_chars += add_chars
    if cur:
        out.append("\n\n".join(cur))
    return [c for c in out if c.strip()]

for path in sorted(glob.glob("docs/systemspecs/*.md")) + sorted(glob.glob("docs/dbprdplans/*.md")):
    cs = chunk(open(path, encoding="utf-8").read())
    for i, c in enumerate(cs):
        print(json.dumps({"text": c, "source": f"{path}#chunk{i}" if len(cs) > 1 else path}))
PYEOF
echo "corpus rows: $(wc -l < /tmp/website_corpus.jsonl)"

"$BIN" ingest "$V" --batch /tmp/website_corpus.jsonl --idempotent | tail -1
"$BIN" healthcheck --vault "$V"
echo "provisioned vault_id=$V"
