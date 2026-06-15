#!/usr/bin/env python3
"""PH69 T08 / issue #558 - deterministic synthetic Polis personas.

Generates the civic 21-slot persona corpus (PRD 28 section 3 row 11) used by
the PH70 Polis constellation/guard FSV (#611): every persona carries exactly
21 signed nonzero scalar axes (polis_axis_01..21, the civic-default panel),
and tie-formation ground truth is SIMULATED by construction - personas belong
to one of 8 communities (distinct sign patterns over the 21 axes); a pair is
a tie iff the two sign patterns agree on every axis (per-axis cosine 1.0
under Ward's Gtau guard), and non-tie pairs record exactly which slots
disagree.

Byte-determinism contract: same (seed, personas, pairs) -> identical bytes.
Only random.Random(seed) (stable Mersenne Twister), sorted-key json, and
fixed 6-decimal magnitudes are used - no clocks, no platform floats in
string formatting beyond round().

Usage: gen_personas.py OUT_DIR [--seed 42] [--personas 1000] [--pairs 600]
"""
import argparse
import json
import pathlib
import random
import sys

AXES = 21
COMMUNITIES = 8


def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def sign_patterns(rng):
    """8 distinct sign patterns over 21 axes (no axis may be zero)."""
    patterns = []
    while len(patterns) < COMMUNITIES:
        pattern = tuple(rng.choice((-1, 1)) for _ in range(AXES))
        if pattern not in patterns:
            patterns.append(pattern)
    return patterns


def generate(out_dir, seed, n_personas, n_pairs):
    if n_personas < COMMUNITIES or n_pairs < 2:
        fail("CALYX_DATASET_MANIFEST_INVALID",
             f"need >= {COMMUNITIES} personas and >= 2 pairs "
             f"(got {n_personas}/{n_pairs})")
    rng = random.Random(seed)
    patterns = sign_patterns(rng)

    personas = []
    for idx in range(n_personas):
        community = idx % COMMUNITIES
        axes = [patterns[community][j] * round(rng.uniform(0.2, 1.0), 6)
                for j in range(AXES)]
        personas.append({
            "axes": axes,
            "community": community,
            "persona_id": f"persona-{seed}-{idx:04d}",
        })

    # Tie ground truth by construction: same community -> all 21 signs agree
    # -> tie; different community -> the disagreeing slot list is exactly the
    # axes where the two sign patterns differ (1-based slot ids, civic panel).
    pairs = []
    for idx in range(n_pairs):
        a = rng.randrange(n_personas)
        if idx % 2 == 0:
            b = (a + COMMUNITIES * (1 + rng.randrange(max(1, n_personas // COMMUNITIES - 1)))) % n_personas
            if personas[a]["community"] != personas[b]["community"] or a == b:
                b = (a + COMMUNITIES) % n_personas
        else:
            b = rng.randrange(n_personas)
            if personas[b]["community"] == personas[a]["community"]:
                b = (b + 1) % n_personas
        ca, cb = personas[a]["community"], personas[b]["community"]
        disagree = [j + 1 for j in range(AXES)
                    if patterns[ca][j] != patterns[cb][j]]
        pairs.append({
            "disagree_slots": disagree,
            "pair_id": f"pair-{seed}-{idx:04d}",
            "persona_a": personas[a]["persona_id"],
            "persona_b": personas[b]["persona_id"],
            "tie": not disagree,
        })

    ties = sum(1 for p in pairs if p["tie"])
    if ties == 0 or ties == len(pairs):
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING",
             f"tie label partition degenerate: {ties}/{len(pairs)} ties")

    target = pathlib.Path(out_dir)
    target.mkdir(parents=True, exist_ok=True)
    with (target / "personas.jsonl").open("w", encoding="utf-8", newline="\n") as out:
        for row in personas:
            out.write(json.dumps(row, sort_keys=True) + "\n")
    with (target / "tie_pairs.jsonl").open("w", encoding="utf-8", newline="\n") as out:
        for row in pairs:
            out.write(json.dumps(row, sort_keys=True) + "\n")
    meta = {
        "axes": AXES,
        "communities": COMMUNITIES,
        "generator": "scripts/gen_personas.py",
        "pairs": len(pairs),
        "personas": len(personas),
        "schema": "civic-default polis_axis_01..21 (signed nonzero scalars)",
        "seed": seed,
        "sign_patterns": [list(p) for p in patterns],
        "synthetic": True,
        "ties": ties,
    }
    (target / "gen_meta.json").write_text(
        json.dumps(meta, sort_keys=True, indent=1) + "\n", encoding="utf-8", newline="\n")
    print(json.dumps({"pairs": len(pairs), "personas": len(personas),
                      "seed": seed, "ties": ties}, sort_keys=True))


def fixture(target_dir, case, seed):
    # Deterministic micro persona/drift fixture exercising the same
    # primitives as production validators.
    target = pathlib.Path(target_dir)
    target.mkdir(parents=True, exist_ok=True)
    axes_n = 21
    plus = [round(0.5 + 0.01 * j, 6) for j in range(axes_n)]
    minus = [-v for v in plus]
    people = [
        {"axes": list(plus), "community": 0, "persona_id": f"fx-{seed}-p0"},
        {"axes": list(plus), "community": 0, "persona_id": f"fx-{seed}-p1"},
        {"axes": list(minus), "community": 1, "persona_id": f"fx-{seed}-p2"},
    ]
    if case == "short-axes":
        people[1]["axes"] = people[1]["axes"][:20]
    elif case == "zero-axis":
        people[1]["axes"][4] = 0.0
    pairs = [
        {"disagree_slots": [], "pair_id": f"fx-{seed}-q0",
         "persona_a": f"fx-{seed}-p0", "persona_b": f"fx-{seed}-p1", "tie": True},
        {"disagree_slots": list(range(1, axes_n + 1)), "pair_id": f"fx-{seed}-q1",
         "persona_a": f"fx-{seed}-p0", "persona_b": f"fx-{seed}-p2", "tie": False},
    ]
    if case == "ghost-persona":
        pairs[1]["persona_b"] = f"fx-{seed}-p9"
    elif case == "mislabeled-tie":
        pairs[1]["tie"] = True
        pairs[1]["disagree_slots"] = []
    if case == "zero-byte":
        (target / "personas.jsonl").write_bytes(b"")
    else:
        (target / "personas.jsonl").write_text(
            "".join(json.dumps(p, sort_keys=True) + "\n" for p in people))
    (target / "tie_pairs.jsonl").write_text(
        "".join(json.dumps(p, sort_keys=True) + "\n" for p in pairs))
    (target / "gen_meta.json").write_text(json.dumps(
        {"seed": seed, "synthetic": True}, sort_keys=True) + "\n")
    meta = {"split_criteria": {"month_a": f"period-A-{seed}", "month_b": f"period-B-{seed}"},
            "row_counts": {"month_a": 2, "month_b": 2}}
    if case == "same-period":
        meta["split_criteria"]["month_b"] = meta["split_criteria"]["month_a"]
    (target / "acquisition_meta.json").write_text(json.dumps(meta, sort_keys=True) + "\n")
    print(json.dumps({"case": case, "pairs": len(pairs), "personas": len(people)}))


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "fixture":
        # self-test support: deterministic micro fixtures (see acquire
        # script's --self-test battery)
        fixture(sys.argv[2], sys.argv[3], sys.argv[4])
        return
    parser = argparse.ArgumentParser()
    parser.add_argument("out_dir")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--personas", type=int, default=1000)
    parser.add_argument("--pairs", type=int, default=600)
    args = parser.parse_args()
    generate(args.out_dir, args.seed, args.personas, args.pairs)


if __name__ == "__main__":
    main()
