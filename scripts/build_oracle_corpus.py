import json, urllib.request, sys, hashlib
from datasets import load_dataset
def fetch(u): return json.loads(urllib.request.urlopen(u,timeout=30).read())
base="https://raw.githubusercontent.com/swe-bench/experiments/main/evaluation/lite"
ORACLE=sys.argv[1] if len(sys.argv)>1 else "20240402_sweagent_gpt4"
OUT=sys.argv[2] if len(sys.argv)>2 else "/home/croyse/calyx/data/oracle_sufficiency"
import os; os.makedirs(OUT,exist_ok=True)
r=fetch(f"{base}/{ORACLE}/results/results.json")
resolved=set(r["resolved"]); applied=set(r["applied"])
lite=load_dataset("princeton-nlp/SWE-bench_Lite")["test"]
rows=[]
def embed(b):
    req=urllib.request.Request("http://127.0.0.1:8088/embed",data=json.dumps({"inputs":b,"normalize":True}).encode(),headers={"Content-Type":"application/json"}); return json.loads(urllib.request.urlopen(req,timeout=120).read())
items=[x for x in lite if x["instance_id"] in applied]
probs=[(x["problem_statement"] or "")[:2000] for x in items]
hints=[((x["hints_text"] or "").strip() or "(no hints)")[:2000] for x in items]
pv=[]; hv=[]
for s in range(0,len(probs),16): pv.extend(embed(probs[s:s+16]))
for s in range(0,len(hints),16): hv.extend(embed(hints[s:s+16]))
for x,p,h in zip(items,pv,hv):
    rows.append({"id":x["instance_id"],"split":"test","label":1 if x["instance_id"] in resolved else 0,
                 "lenses":{"problem":[round(v,6) for v in p],"hints":[round(v,6) for v in h]}})
with open(f"{OUT}/vectors.jsonl","w") as f:
    for r2 in rows: f.write(json.dumps(r2,separators=(",",":"))+"\n")
manifest={"oracle_model":ORACLE,"dataset":"princeton-nlp/SWE-bench_Lite","anchor":"test_pass_fail(resolved)",
          "n":len(rows),"resolved":sum(x["label"] for x in rows),"embedding_model_id":"tei:gte",
          "lenses":[{"name":"problem"},{"name":"hints"}],"results_url":f"{base}/{ORACLE}/results/results.json"}
json.dump(manifest,open(f"{OUT}/manifest.json","w"),indent=2)
print(f"n={len(rows)} resolved={sum(x['label'] for x in rows)} dim={len(pv[0])} -> {OUT}/vectors.jsonl")
