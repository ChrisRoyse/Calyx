#!/usr/bin/env python3
"""Build an anchored ingest JSONL for a clean-court CourtListener slice.

Reference lineage
-----------------
This tool extends the ``tools/lawslice`` extractor family (#1463 ``extract_cuyahoga.py``
and #1464 ``build_ingest_jsonl.py``) with an *additive, court-filtered* selection path
for the 2x corpus expansion (#1965). It changes no existing invocation: the Cuyahoga
extractor and its Eighth-District inference selector are untouched.

Why a court filter — not the Eighth-District selector — for Ohio Supreme Court
-------------------------------------------------------------------------------
The Cuyahoga selector (`cuyahoga_contract.classify`) exists because the generic
``ohioctapp`` court_id mixes all twelve Ohio appellate districts, so Eighth-District
membership has to be *proven* from caption/ROD evidence, and it hard-binds exactly
five source corrections and two authoritative-PDF supplements to specific Cuyahoga
opinion IDs. None of that applies to a court whose CourtListener ``court_id`` is
itself authoritative (e.g. ``ohio`` = Ohio Supreme Court): every opinion under a
docket with that court_id *is* that court's opinion. So this path selects opinions
by exact docket ``court_id`` membership and accepts them directly, while reusing the
*identical* text-resolution policy (`cuyahoga_contract.resolve_text`) and the
*identical* composed-text / six-anchor / metadata BatchLine composition
(`build_ingest_jsonl.build_text` / `.anchors`) so a v5 composed text is byte-for-byte
comparable to a canonical-v4 composed text (that is what makes cross-slice dedup real).

Discovery (#1965 Step 1) confirmed the alias trap the same way ``ohctapp8cuyahog`` was
an empty alias of the real Eighth-District data: the district-specific ``ohctapp9*`` /
``ohctapp11*`` court_ids carry only a handful of dockets each, while the real Ninth /
Eleventh District opinions live in the generic ``ohioctapp`` bucket and would need the
same district inference. Only ``ohio`` is a clean-court slice.
"""

from __future__ import annotations

import argparse
import io
import json
import os
import subprocess
import sys
import time
from pathlib import Path

# Shared, proven primitives — single source of truth (no reimplementation).
from cuyahoga_contract import AuthoritativeTextError, resolve_text, sha256_text
from build_ingest_jsonl import (
    ANCHOR_COUNT,
    LOCATOR_KEYS,
    REQUIRED_METADATA,
    anchors as compose_anchors,
    build_text,
    canonical_preference,
    field_string,
    validate_batch_line,
)

TEXT_FIELDS = (
    "html_with_citations",
    "plain_text",
    "html_columbia",
    "html_lawbox",
    "html_anon_2020",
    "xml_harvard",
    "html",
)
OPINION_META = (
    "id",
    "type",
    "author_str",
    "per_curiam",
    "author_id",
    "cluster_id",
    "sha1",
    "download_url",
    "local_path",
    "date_created",
    "date_modified",
)
LBZIP2 = os.path.expanduser("~/.local/bin/lbzip2")
LBZIP2_THREADS = "8"
PROGRESS_EVERY = 1_000_000

# Per-court jurisdiction facets (metadata only; Phase B does not require them).
COURT_JURISDICTION = {
    "ohio": {
        "jurisdiction_country": "United States",
        "jurisdiction_court_system": "state",
        "jurisdiction_state": "Ohio",
        "jurisdiction_court": "Supreme Court of Ohio",
        "jurisdiction_court_level": "supreme",
    },
}
LICENSE = "public-domain (CourtListener bulk export; US court opinions)"


class BuildError(Exception):
    pass


def fail(message, **ctx):
    raise BuildError("%s %s" % (message, json.dumps(ctx, sort_keys=True) if ctx else ""))


def _import_csv():
    import csv

    csv.field_size_limit(sys.maxsize)
    return csv


def stream(path):
    proc = subprocess.Popen([LBZIP2, "-dc", "-n", LBZIP2_THREADS, path], stdout=subprocess.PIPE)
    return proc, io.TextIOWrapper(proc.stdout, encoding="utf-8", newline="")


def reader(fh):
    csv = _import_csv()
    r = csv.reader(fh, quotechar='"', escapechar="\\", doublequote=False, strict=True)
    header = next(r)
    return r, {c: i for i, c in enumerate(header)}


def log(msg):
    sys.stderr.write("[%s] %s\n" % (time.strftime("%H:%M:%S"), msg))
    sys.stderr.flush()


# ---------------------------------------------------------------------------
# Extraction: dockets -> clusters -> opinions, filtered to the chosen courts.
# ---------------------------------------------------------------------------

def scan_dockets(path, courts):
    proc, fh = stream(path)
    r, idx = reader(fh)
    i_id, i_court, i_dn, i_case, i_appeal, i_date = (
        idx["id"], idx["court_id"], idx["docket_number"], idx["case_name"],
        idx["appeal_from_str"], idx["date_filed"],
    )
    out = {}
    counts = {}
    n = 0
    t0 = time.time()
    for row in r:
        n += 1
        c = row[i_court]
        if c in courts:
            try:
                did = int(row[i_id])
            except ValueError:
                continue
            out[did] = {
                "court_id": c,
                "docket_number": row[i_dn],
                "case_name": row[i_case],
                "appeal_from_str": row[i_appeal],
                "date_filed": row[i_date],
            }
            counts[c] = counts.get(c, 0) + 1
        if n % (2 * PROGRESS_EVERY) == 0:
            log("dockets %d rows, %.1f min, %d candidates" % (n, (time.time() - t0) / 60, len(out)))
    proc.stdout.close()
    proc.wait()
    return out, {"rows_scanned": n, "candidate_dockets": len(out), "by_court": counts}


def scan_clusters(path, dockets):
    proc, fh = stream(path)
    r, idx = reader(fh)
    fields = ("id", "docket_id", "slug", "case_name", "case_name_full", "date_filed",
              "judges", "disposition", "posture", "nature_of_suit", "syllabus",
              "precedential_status", "citation_count", "source")
    ix = {f: idx[f] for f in fields}
    out = {}
    counts = {}
    n = 0
    t0 = time.time()
    for row in r:
        n += 1
        d = row[ix["docket_id"]]
        if not d:
            continue
        try:
            did = int(d)
        except ValueError:
            continue
        docket = dockets.get(did)
        if docket is None:
            continue
        try:
            clid = int(row[ix["id"]])
        except ValueError:
            continue
        out[clid] = {
            "docket_id": did,
            "court_id": docket["court_id"],
            "docket_number": docket["docket_number"],
            "appeal_from_str": docket["appeal_from_str"],
            "cluster_slug": row[ix["slug"]],
            "case_name": row[ix["case_name"]] or docket["case_name"],
            "case_name_full": row[ix["case_name_full"]],
            "date_filed": row[ix["date_filed"]],
            "judges": row[ix["judges"]],
            "disposition": row[ix["disposition"]],
            "posture": row[ix["posture"]],
            "nature_of_suit": row[ix["nature_of_suit"]],
            "syllabus": row[ix["syllabus"]],
            "precedential_status": row[ix["precedential_status"]],
            "citation_count": row[ix["citation_count"]],
            "cluster_source": row[ix["source"]],
        }
        counts[docket["court_id"]] = counts.get(docket["court_id"], 0) + 1
        if n % (2 * PROGRESS_EVERY) == 0:
            log("clusters %d rows, %.1f min, %d candidates" % (n, (time.time() - t0) / 60, len(out)))
    proc.stdout.close()
    proc.wait()
    return out, {"rows_scanned": n, "candidate_clusters": len(out), "by_court": counts}


def scan_opinions(path, clusters, snapshot_date, acquired_at, raw_out, empty_out):
    proc, fh = stream(path)
    r, idx = reader(fh)
    want = set(OPINION_META) | set(TEXT_FIELDS)
    ix = {f: idx[f] for f in want if f in idx}
    missing_meta = [f for f in OPINION_META if f not in ix]
    if missing_meta:
        fail("opinions CSV missing required columns", missing=missing_meta)
    n = 0
    matched = 0
    accepted = 0
    empty = 0
    seen_ids = set()
    by_court_matched = {}
    by_court_accepted = {}
    t0 = time.time()
    for row in r:
        n += 1
        c = row[ix["cluster_id"]]
        if c:
            try:
                clid = int(c)
            except ValueError:
                clid = None
            cluster = clusters.get(clid) if clid is not None else None
            if cluster is not None:
                matched += 1
                court = cluster["court_id"]
                by_court_matched[court] = by_court_matched.get(court, 0) + 1
                try:
                    opinion_id = int(row[ix["id"]])
                except ValueError:
                    fail("opinion id is not decimal", line=n, value=row[ix["id"]])
                if opinion_id in seen_ids:
                    fail("duplicate opinion id", opinion_id=opinion_id)
                seen_ids.add(opinion_id)
                values = {f: row[ix[f]] for f in TEXT_FIELDS if f in ix}
                try:
                    resolved = resolve_text(values, where="opinion %d" % opinion_id)
                except AuthoritativeTextError as err:
                    empty += 1
                    empty_out.write(json.dumps({
                        "opinion_id": opinion_id, "cluster_id": clid, "court_id": court,
                        "reason": err.code if hasattr(err, "code") else "authoritative_text_error",
                        "message": str(err),
                    }, ensure_ascii=False, sort_keys=True) + "\n")
                    if n % PROGRESS_EVERY == 0:
                        log("opinions %d rows, %.1f min, matched=%d accepted=%d empty=%d"
                            % (n, (time.time() - t0) / 60, matched, accepted, empty))
                    continue
                author_id = row[ix["author_id"]]
                per_curiam = row[ix["per_curiam"]]
                if per_curiam not in ("", "t", "f"):
                    fail("per_curiam not boolean", opinion_id=opinion_id, value=per_curiam)
                record = {
                    "opinion_id": opinion_id,
                    "cluster_id": clid,
                    "cluster_slug": cluster["cluster_slug"],
                    "docket_id": cluster["docket_id"],
                    "court_id": court,
                    "opinion_type": row[ix["type"]],
                    "author_id": int(author_id) if author_id else None,
                    "author_str": row[ix["author_str"]],
                    "per_curiam": per_curiam == "t",
                    "date_filed": cluster["date_filed"],
                    "case_name": cluster["case_name"],
                    "case_name_full": cluster["case_name_full"],
                    "docket_number": cluster["docket_number"],
                    "judges": cluster["judges"],
                    "disposition": cluster["disposition"],
                    "posture": cluster["posture"],
                    "nature_of_suit": cluster["nature_of_suit"],
                    "syllabus": cluster["syllabus"],
                    "precedential_status": cluster["precedential_status"],
                    "citation_count": cluster["citation_count"],
                    "cluster_source": cluster["cluster_source"],
                    "text": resolved.text,
                    "text_source": resolved.source_field,
                    "text_chars": resolved.normalized_chars,
                    "normalized_sha256": resolved.normalized_sha256,
                    "source_raw_sha256": resolved.source_raw_sha256,
                    "sha1": row[ix["sha1"]] or None,
                    "download_url": row[ix["download_url"]] or None,
                    "local_path": row[ix["local_path"]] or None,
                    "date_created": row[ix["date_created"]] or None,
                    "date_modified": row[ix["date_modified"]] or None,
                    "source_snapshot_date": snapshot_date,
                    "source_acquired_at": acquired_at,
                    "canonical_source_url": "https://www.courtlistener.com/opinion/%d/%s/"
                    % (clid, cluster["cluster_slug"]),
                }
                raw_out.write(json.dumps(record, ensure_ascii=False, sort_keys=True) + "\n")
                accepted += 1
                by_court_accepted[court] = by_court_accepted.get(court, 0) + 1
        if n % PROGRESS_EVERY == 0:
            log("opinions %d rows, %.1f min, matched=%d accepted=%d empty=%d"
                % (n, (time.time() - t0) / 60, matched, accepted, empty))
    proc.stdout.close()
    proc.wait()
    return {
        "rows_scanned": n,
        "matched_candidate_opinions": matched,
        "accepted_nonempty": accepted,
        "empty_dropped": empty,
        "by_court_matched": by_court_matched,
        "by_court_accepted": by_court_accepted,
    }


# ---------------------------------------------------------------------------
# Build: dedup (within-slice + against canonical-v4), aliases, anchored ingest.
# ---------------------------------------------------------------------------

def load_v4_canonical_shas(path):
    shas = set()
    with open(path, "r", encoding="utf-8") as fh:
        for lineno, line in enumerate(fh, 1):
            row = json.loads(line)
            sha = row["metadata"]["ingest_text_sha256"]
            text = row["text"]
            if sha != sha256_text(text):
                fail("v4 canonical row ingest_text_sha256 mismatch", line=lineno)
            shas.add(sha)
    return shas


def raw_rows(path):
    with open(path, "r", encoding="utf-8") as fh:
        for lineno, line in enumerate(fh, 1):
            if not line.strip():
                fail("raw contains blank line", line=lineno)
            yield lineno, json.loads(line)


def compose(row):
    return build_text(row, line=row["opinion_id"])


def build_metadata(row, dataset_tag, canonical_id, ingest_sha):
    md = {}
    md.update(COURT_JURISDICTION.get(row["court_id"], {}))
    carry = ("opinion_id", "cluster_id", "cluster_slug", "docket_id", "court_id",
             "date_filed", "case_name", "case_name_full", "docket_number",
             "citation_count", "opinion_type", "author_id", "author_str",
             "per_curiam", "judges", "disposition", "posture", "nature_of_suit",
             "syllabus", "precedential_status", "text_source", "cluster_source",
             "source_snapshot_date", "source_acquired_at")
    for key in carry:
        md[key] = field_string(row, key)
    md.update({
        "canonical_opinion_id": str(canonical_id),
        "source_dataset": dataset_tag,
        "source_sha256": row["source_raw_sha256"],
        "normalized_text_sha256": row["normalized_sha256"],
        "ingest_text_sha256": ingest_sha,
        "license": LICENSE,
        "retrieval_ts": row["source_acquired_at"],
        "source_url": row["canonical_source_url"],
        "courtlistener_sha1": field_string(row, "sha1"),
        "courtlistener_download_url": field_string(row, "download_url"),
        "courtlistener_date_created": field_string(row, "date_created"),
        "courtlistener_date_modified": field_string(row, "date_modified"),
    })
    for k, v in md.items():
        if not isinstance(k, str) or not isinstance(v, str):
            fail("metadata key/value not string", opinion_id=row["opinion_id"], key=k)
    return md


def alias_record(row, canonical_id, content_sha):
    return {
        "opinion_id": row["opinion_id"],
        "canonical_opinion_id": canonical_id,
        "is_canonical": row["opinion_id"] == canonical_id,
        "content_sha256": content_sha,
        "source_normalized_sha256": row["normalized_sha256"],
        "source_raw_sha256": row["source_raw_sha256"],
        "source_field": row["text_source"],
        "cluster_id": row["cluster_id"],
        "docket_id": row["docket_id"],
        "court_id": row["court_id"],
        "source_url": row["canonical_source_url"],
    }


def row_year(row):
    d = row.get("date_filed") or ""
    if len(d) >= 4 and d[:4].isdigit():
        return int(d[:4])
    return None


def passes_filter(row, filters):
    """Return (ok, reason). Slice-bound filters are documented, reproducible params."""
    prec = filters.get("precedential")
    if prec is not None and (row.get("precedential_status") or "") not in prec:
        return False, "precedential_status_not_in_%s" % ",".join(sorted(prec))
    y = row_year(row)
    lo, hi = filters.get("min_year"), filters.get("max_year")
    if lo is not None and (y is None or y < lo):
        return False, "before_min_year_%d" % lo
    if hi is not None and (y is None or y > hi):
        return False, "after_max_year_%d" % hi
    return True, None


def build(raw_path, out_dir, dataset_tag, v4_shas, extract_stats, filters):
    filtered_path = os.path.join(out_dir, "filtered_out.jsonl")
    filtered_excluded = 0
    filter_reasons = {}
    # Pass A: index composed-text identities, split off cross-v4 collisions.
    canonical = {}            # sha -> (preference, opinion_id) for within-slice canonical
    content_counts = {}       # sha -> count (within-slice, excluding cross-v4)
    cross_v4 = {}             # opinion_id -> sha (composed text already in canonical-v4)
    text_by_sha = {}
    opinion_ids = set()
    selected_ids = set()
    with open(filtered_path, "w", encoding="utf-8") as flt:
        for lineno, row in raw_rows(raw_path):
            oid = row["opinion_id"]
            if oid in opinion_ids:
                fail("duplicate raw opinion_id", opinion_id=oid)
            opinion_ids.add(oid)
            ok, reason = passes_filter(row, filters)
            if not ok:
                filtered_excluded += 1
                filter_reasons[reason] = filter_reasons.get(reason, 0) + 1
                flt.write(json.dumps({"opinion_id": oid, "reason": reason,
                                      "precedential_status": row.get("precedential_status"),
                                      "date_filed": row.get("date_filed")},
                                     ensure_ascii=False, sort_keys=True) + "\n")
                continue
            selected_ids.add(oid)
            text = compose(row)
            sha = sha256_text(text)
            if sha in v4_shas:
                cross_v4[oid] = sha
                continue
            prev = text_by_sha.setdefault(sha, text)
            if prev != text:
                fail("sha256 collision between unequal composed texts", sha=sha, opinion_id=oid)
            content_counts[sha] = content_counts.get(sha, 0) + 1
            cand = canonical_preference(row, oid)
            cur = canonical.get(sha)
            if cur is None or cand < cur:
                canonical[sha] = cand
    canonical_id_by_sha = {sha: key[1] for sha, key in canonical.items()}

    # Pass B: emit aliases + one anchored canonical row per unique within-slice sha.
    ingest_path = os.path.join(out_dir, "ingest.jsonl")
    alias_path = os.path.join(out_dir, "opinion_aliases.jsonl")
    dropped_path = os.path.join(out_dir, "dedup_dropped.jsonl")
    crossv4_path = os.path.join(out_dir, "cross_v4_dropped.jsonl")
    content_rows = 0
    within_dup = 0
    alias_rows = 0
    longest = []
    with open(ingest_path, "w", encoding="utf-8") as ing, \
         open(alias_path, "w", encoding="utf-8") as ali, \
         open(dropped_path, "w", encoding="utf-8") as drp, \
         open(crossv4_path, "w", encoding="utf-8") as xv4:
        for lineno, row in raw_rows(raw_path):
            oid = row["opinion_id"]
            if oid not in selected_ids:
                continue
            text = compose(row)
            sha = sha256_text(text)
            if oid in cross_v4:
                xv4.write(json.dumps({
                    "opinion_id": oid, "cluster_id": row["cluster_id"],
                    "court_id": row["court_id"], "content_sha256": sha,
                    "note": "composed text equals a canonical-v4 content; not re-ingested",
                }, ensure_ascii=False, sort_keys=True) + "\n")
                continue
            canonical_id = canonical_id_by_sha[sha]
            ali.write(json.dumps(alias_record(row, canonical_id, sha),
                                 ensure_ascii=False, sort_keys=True) + "\n")
            alias_rows += 1
            if oid != canonical_id:
                within_dup += 1
                drp.write(json.dumps({
                    "opinion_id": oid, "canonical_opinion_id": canonical_id,
                    "content_sha256": sha, "court_id": row["court_id"],
                    "case_name": row.get("case_name"),
                }, ensure_ascii=False, sort_keys=True) + "\n")
                continue
            md = build_metadata(row, dataset_tag, canonical_id, sha)
            batch = {"text": text, "metadata": md, "anchors": compose_anchors(row, dataset_tag, line=oid)}
            validate_batch_line(batch, where="opinion %d" % oid)
            ing.write(json.dumps(batch, ensure_ascii=False, sort_keys=True) + "\n")
            content_rows += 1
            longest.append((len(text), sha))
    longest.sort(key=lambda x: (-x[0], x[1]))
    manifest = {
        "format": "calyx-expansion-ingest-v5",
        "dataset_tag": dataset_tag,
        "slice_filters": {
            "precedential": sorted(filters["precedential"]) if filters.get("precedential") else None,
            "min_year": filters.get("min_year"),
            "max_year": filters.get("max_year"),
        },
        "cross_slice_dedup_baseline": {
            "canonical_v4_contents": len(v4_shas),
        },
        "extract": extract_stats,
        "counts": {
            "raw_accepted_nonempty": len(opinion_ids),
            "filter_excluded": filtered_excluded,
            "filter_excluded_reasons": filter_reasons,
            "selected": len(selected_ids),
            "cross_v4_dropped": len(cross_v4),
            "within_slice_alias_rows": alias_rows,
            "within_slice_duplicate_aliases": within_dup,
            "canonical_content_rows": content_rows,
        },
        "longest_content_chars": [x[0] for x in longest[:10]],
    }
    # Arithmetic self-check (no row unaccounted for).
    raw_total = extract_stats["accepted_nonempty"]
    if len(opinion_ids) != raw_total:
        fail("raw opinion count changed between scans", first=raw_total, second=len(opinion_ids))
    if raw_total != filtered_excluded + len(selected_ids):
        fail("accounting: raw != filter_excluded + selected")
    if len(selected_ids) != len(cross_v4) + alias_rows:
        fail("accounting: selected != cross_v4 + within_slice_aliases",
             selected=len(selected_ids), cross_v4=len(cross_v4), aliases=alias_rows)
    if alias_rows != content_rows + within_dup:
        fail("accounting: aliases != canonical + duplicate_aliases")
    with open(os.path.join(out_dir, "expansion_manifest.json"), "w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2, sort_keys=True)
    return manifest, ingest_path


def cmd_extract(args):
    out_dir = args.out
    os.makedirs(out_dir, exist_ok=True)
    courts = set(args.court)
    for c in courts:
        if c not in COURT_JURISDICTION:
            log("WARNING: no jurisdiction facets defined for court_id %r (metadata will omit them)" % c)
    raw = os.path.join(args.raw_dir, "dockets-%s.csv.bz2" % args.snapshot_date)
    clu = os.path.join(args.raw_dir, "opinion-clusters-%s.csv.bz2" % args.snapshot_date)
    opi = os.path.join(args.raw_dir, "opinions-%s.csv.bz2" % args.snapshot_date)

    log("PASS 1 dockets, courts=%s" % sorted(courts))
    dockets, dstat = scan_dockets(raw, courts)
    log("dockets done: %s" % json.dumps(dstat["by_court"]))

    log("PASS 2 clusters")
    clusters, cstat = scan_clusters(clu, dockets)
    del dockets
    log("clusters done: %s" % json.dumps(cstat["by_court"]))

    log("PASS 3 opinions (full text resolution)")
    raw_path = os.path.join(out_dir, "expansion.raw.jsonl")
    empty_path = os.path.join(out_dir, "empty_text.jsonl")
    with open(raw_path, "w", encoding="utf-8") as raw_out, \
         open(empty_path, "w", encoding="utf-8") as empty_out:
        ostat = scan_opinions(opi, clusters, args.snapshot_date, args.acquired_at, raw_out, empty_out)
    del clusters
    ostat["courts"] = sorted(courts)
    ostat["dockets"] = dstat
    ostat["clusters"] = cstat
    with open(os.path.join(out_dir, "extract_stats.json"), "w", encoding="utf-8") as fh:
        json.dump(ostat, fh, indent=2, sort_keys=True)
    print(json.dumps({k: ostat[k] for k in
        ("matched_candidate_opinions", "accepted_nonempty", "empty_dropped", "by_court_accepted")},
        indent=2, sort_keys=True))
    log("EXTRACT DONE raw=%s accepted_nonempty=%d empty=%d"
        % (raw_path, ostat["accepted_nonempty"], ostat["empty_dropped"]))


def cmd_build(args):
    out_dir = args.out
    os.makedirs(out_dir, exist_ok=True)
    with open(args.extract_stats, "r", encoding="utf-8") as fh:
        extract_stats = json.load(fh)
    filters = {
        "precedential": set(x.strip() for x in args.precedential.split(",")) if args.precedential else None,
        "min_year": args.min_year,
        "max_year": args.max_year,
    }
    log("BUILD filters=%s" % json.dumps({k: (sorted(v) if isinstance(v, set) else v) for k, v in filters.items()}))
    v4_shas = load_v4_canonical_shas(args.v4_canonical)
    manifest, ingest_path = build(args.raw, out_dir, args.dataset_tag, v4_shas, extract_stats, filters)
    print(json.dumps(manifest, indent=2, sort_keys=True))
    log("BUILD DONE ingest=%s canonical_rows=%d" % (ingest_path, manifest["counts"]["canonical_content_rows"]))


def cmd_characterize(args):
    """Cheap breakdown of an extracted raw.jsonl to inform slice bounding."""
    prec = {}
    era = {}
    cited = {}
    total = 0
    for lineno, row in raw_rows(args.raw):
        total += 1
        prec[row.get("precedential_status") or ""] = prec.get(row.get("precedential_status") or "", 0) + 1
        y = row_year(row)
        decade = "%ds" % (y // 10 * 10) if y is not None else "unknown"
        era[decade] = era.get(decade, 0) + 1
        try:
            cc = int(row.get("citation_count") or 0)
        except (TypeError, ValueError):
            cc = 0
        bucket = "0" if cc == 0 else ("1-9" if cc < 10 else "10+")
        cited[bucket] = cited.get(bucket, 0) + 1
    print(json.dumps({"total": total, "by_precedential": prec, "by_era": dict(sorted(era.items())),
                      "by_cited_bucket": cited}, indent=2, sort_keys=True))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="command", required=True)

    ext = sub.add_parser("extract", help="court-filtered extract of accepted non-empty opinions -> raw.jsonl")
    ext.add_argument("--court", action="append", required=True,
                     help="exact CourtListener court_id to select (repeatable)")
    ext.add_argument("--raw-dir", required=True, help="dir with <table>-<snapshot>.csv.bz2")
    ext.add_argument("--out", required=True)
    ext.add_argument("--snapshot-date", required=True)
    ext.add_argument("--acquired-at", required=True, help="ISO8601 UTC retrieval timestamp")

    bld = sub.add_parser("build", help="dedup + alias + anchored ingest from an extracted raw.jsonl")
    bld.add_argument("--raw", required=True, help="expansion.raw.jsonl from `extract`")
    bld.add_argument("--extract-stats", required=True, help="extract_stats.json from `extract`")
    bld.add_argument("--out", required=True)
    bld.add_argument("--dataset-tag", required=True)
    bld.add_argument("--v4-canonical", required=True,
                     help="path to canonical-v4 cuyahoga-canonical.jsonl for cross-slice dedup")
    bld.add_argument("--precedential", default=None,
                     help="comma list of allowed precedential_status values (slice bound)")
    bld.add_argument("--min-year", type=int, default=None, help="drop opinions filed before this year")
    bld.add_argument("--max-year", type=int, default=None, help="drop opinions filed after this year")

    ch = sub.add_parser("characterize", help="print precedential/era/citation breakdown of a raw.jsonl")
    ch.add_argument("--raw", required=True)

    args = p.parse_args()
    try:
        if args.command == "extract":
            cmd_extract(args)
        elif args.command == "build":
            cmd_build(args)
        else:
            cmd_characterize(args)
    except BuildError as err:
        sys.stderr.write("BUILD ERROR: %s\n" % err)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
