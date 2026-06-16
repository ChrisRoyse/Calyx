#!/usr/bin/env python3
"""Fine-tune a transformer injection guard on the deepset prompt_injection corpus
(GPU), evaluate on the held-out test split, and export per-example guard scores
for Ward conformal tau calibration. Honest metrics: BOTH injection-block-rate and
benign-FRR are reported at the operating point.
"""
import argparse, json, os, sys
import numpy as np
import torch
from torch.utils.data import Dataset
from transformers import (AutoTokenizer, AutoModelForSequenceClassification,
                          TrainingArguments, Trainer)

def load_split(path):
    rows = json.load(open(path))
    return [r["text"] for r in rows], [int(r["label"]) for r in rows]

class DS(Dataset):
    def __init__(self, enc, labels):
        self.enc, self.labels = enc, labels
    def __len__(self): return len(self.labels)
    def __getitem__(self, i):
        item = {k: v[i] for k, v in self.enc.items()}
        item["labels"] = self.labels[i]
        return item

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default="microsoft/deberta-v3-base")
    ap.add_argument("--train", required=True)
    ap.add_argument("--test", required=True)
    ap.add_argument("--out", required=True)            # model dir
    ap.add_argument("--scores-out", required=True)     # scores jsonl (both splits)
    ap.add_argument("--epochs", type=float, default=6.0)
    ap.add_argument("--bs", type=int, default=16)
    ap.add_argument("--lr", type=float, default=2e-5)
    ap.add_argument("--target-frr", type=float, default=0.01)
    args = ap.parse_args()

    assert torch.cuda.is_available(), "CUDA must be available for GPU fine-tuning"
    dev = torch.cuda.get_device_name(0)
    print(f"GPU={dev} cap={torch.cuda.get_device_capability(0)} torch_cuda={torch.version.cuda}", flush=True)

    tr_text, tr_lab = load_split(args.train)
    te_text, te_lab = load_split(args.test)
    tok = AutoTokenizer.from_pretrained(args.model)
    def enc(texts): return dict(tok(texts, truncation=True, max_length=256, padding="max_length", return_tensors="pt"))
    tr = DS(enc(tr_text), tr_lab); te = DS(enc(te_text), te_lab)
    model = AutoModelForSequenceClassification.from_pretrained(args.model, num_labels=2)

    targs = TrainingArguments(
        output_dir=args.out, num_train_epochs=args.epochs,
        per_device_train_batch_size=args.bs, per_device_eval_batch_size=64,
        learning_rate=args.lr, weight_decay=0.01, warmup_ratio=0.1,
        logging_steps=10, save_strategy="no", report_to=[], bf16=True, seed=42)
    Trainer(model=model, args=targs, train_dataset=tr).train()

    # Inference -> injection probability (label 1 = injection)
    model.eval().cuda()
    def inj_prob(texts):
        out = []
        for i in range(0, len(texts), 64):
            e = tok(texts[i:i+64], truncation=True, max_length=256, padding=True, return_tensors="pt").to("cuda")
            with torch.no_grad():
                p = torch.softmax(model(**e).logits, -1)[:, 1]
            out.extend(p.cpu().tolist())
        return out
    tr_p = inj_prob(tr_text); te_p = inj_prob(te_text)

    # Honest metrics on held-out test: choose tau at benign FRR<=target on TEST benign,
    # report injection block-rate AND benign acceptance.
    te_b = [p for p, l in zip(te_p, te_lab) if l == 0]
    te_i = [p for p, l in zip(te_p, te_lab) if l == 1]
    sb = sorted(te_b)
    tau = sb[min(len(sb)-1, int((1-args.target_frr)*len(sb)))]  # ~99th pct of benign inj-prob
    block = sum(1 for x in te_i if x >= tau)/len(te_i)
    benign_acc = sum(1 for x in te_b if x < tau)/len(te_b)
    block50 = sum(1 for x in te_i if x >= 0.5)/len(te_i)
    bacc50 = sum(1 for x in te_b if x < 0.5)/len(te_b)
    print(f"TEST benign={len(te_b)} inj={len(te_i)}", flush=True)
    print(f"@benign-FRR<={args.target_frr}: tau={tau:.4f} BLOCK={block:.4f} benign_acc={benign_acc:.4f}", flush=True)
    print(f"@tau=0.5: BLOCK={block50:.4f} benign_acc={bacc50:.4f}", flush=True)

    os.makedirs(os.path.dirname(args.scores_out), exist_ok=True)
    with open(args.scores_out, "w") as f:
        for split, texts, probs, labs in [("train", tr_text, tr_p, tr_lab), ("test", te_text, te_p, te_lab)]:
            for j,(p,l) in enumerate(zip(probs, labs)):
                # Ward convention: higher score = more benign-like (cos-like). benign_score = 1 - inj_prob
                f.write(json.dumps({"split": split, "row": j, "label": l,
                                    "inj_prob": round(float(p),6),
                                    "benign_score": round(1.0-float(p),6)})+"\n")
    model.save_pretrained(args.out); tok.save_pretrained(args.out)
    print(f"SAVED model={args.out} scores={args.scores_out}", flush=True)

if __name__ == "__main__":
    main()
