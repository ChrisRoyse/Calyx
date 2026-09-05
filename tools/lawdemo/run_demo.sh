#!/usr/bin/env bash
set -Eeuo pipefail

umask 002

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/../.." && pwd -P)

CALYX_HOME=${CALYX_HOME:-/home/${USER}/calyx}
FSV_HOME=${LAWDEMO_FSV_HOME:-$CALYX_HOME/fsv}
BINARY=${LAWDEMO_BINARY:-$CALYX_HOME/repo/target/release/calyx}
PYTHON=${LAWDEMO_PYTHON:-$FSV_HOME/epic1460-canonical-f58beaba-20260714T211800Z/venv/bin/python}
VAULT=${LAWDEMO_VAULT:-$CALYX_HOME/vaults/01KXZZH7EA4B4BAMPC434X84Q4}
RESIDENT_ADDR=${LAWDEMO_RESIDENT_ADDR:-127.0.0.1:18401}
ENV_FILE=${LAWDEMO_ENV_FILE:-$FSV_HOME/issue1837-weave-weight-bfe31521-20260716T210747Z/source/env.sh}

D1_ROOT=${LAWDEMO_D1_ROOT:-$FSV_HOME/issue1476-doctrine-kernel-27f7c730-20260716T225955Z}
D1_VAULT=${LAWDEMO_D1_VAULT:-$CALYX_HOME/vaults/01KY050EW43QWJM2XSABEHRJJ1}
D1_KERNEL=6b8282b621212651cda5f5addb422092

D2_ROOT=${LAWDEMO_D2_ROOT:-$FSV_HOME/issue1477-judge-intel-27f7c730-20260716T232602Z}
D2_VAULT=${LAWDEMO_D2_VAULT:-$CALYX_HOME/vaults/01KY068KMQ9PJQVPVKY17KK8VR}
D2_KERNEL=fc7b3f92caa373180f1d5cf383d077aa

D3_ROOT=${LAWDEMO_D3_ROOT:-$FSV_HOME/issue1478-sanctions-shield-ebc7e31a-20260717T004700Z}
D4_ROOT=${LAWDEMO_D4_ROOT:-$FSV_HOME/issue1479-dissent-aefbd247-20260717T012900Z}

EXPECTED_HEADLINE=
OUT_DIR=

usage() {
  cat <<'EOF'
usage: tools/lawdemo/run_demo.sh [--out <new-directory>] [--expect-headline <prior-headline.json>]

Runs the sealed Cuyahoga County law demo against live vault state. Heavy
accepted generations are checksum-verified before display. The output
directory must not already exist.
EOF
}

while (($#)); do
  case "$1" in
    --out)
      (($# >= 2)) || { echo "--out requires a value" >&2; exit 2; }
      OUT_DIR=$2
      shift 2
      ;;
    --expect-headline)
      (($# >= 2)) || { echo "--expect-headline requires a value" >&2; exit 2; }
      EXPECTED_HEADLINE=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unexpected argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="$PWD/lawdemo-run-$(date -u +%Y%m%dT%H%M%SZ)"
fi
if [[ -e "$OUT_DIR" ]]; then
  echo "output path already exists: $OUT_DIR" >&2
  exit 2
fi
mkdir -p -- "$OUT_DIR"
OUT_DIR=$(cd -- "$OUT_DIR" && pwd -P)
TIMING_JSONL="$OUT_DIR/timing.jsonl"
: > "$TIMING_JSONL"

CURRENT_SECTION=setup

now_ms() {
  local stamp
  stamp=$(date +%s%N)
  printf '%s\n' "${stamp:0:13}"
}

CURRENT_STARTED_MS=$(now_ms)
RUN_STARTED_MS=$CURRENT_STARTED_MS

fail_line() {
  local line=$1
  local status=$2
  printf '\nFAILED section=%s line=%s exit=%s\n' "$CURRENT_SECTION" "$line" "$status" >&2
}
trap 'fail_line "$LINENO" "$?"' ERR

require_file() {
  [[ -f "$1" ]] || { echo "required file is absent: $1" >&2; exit 1; }
}

require_dir() {
  [[ -d "$1" ]] || { echo "required directory is absent: $1" >&2; exit 1; }
}

print_command() {
  printf '  $'
  printf ' %q' "$@"
  printf '\n'
}

capture_command() {
  local stdout_file=$1
  local stderr_file=$2
  shift 2
  print_command "$@"
  "$@" > "$stdout_file" 2> "$stderr_file"
}

section_begin() {
  CURRENT_SECTION=$1
  CURRENT_STARTED_MS=$(now_ms)
  printf '\n================================================================\n'
  printf '%s\n' "$2"
  printf '================================================================\n'
}

section_end() {
  local ended_ms elapsed_ms seconds millis
  ended_ms=$(now_ms)
  elapsed_ms=$((ended_ms - CURRENT_STARTED_MS))
  seconds=$((elapsed_ms / 1000))
  millis=$((elapsed_ms % 1000))
  printf '{"section":"%s","elapsed_ms":%s}\n' "$CURRENT_SECTION" "$elapsed_ms" >> "$TIMING_JSONL"
  printf 'SECTION_TIME %s.%03ds\n' "$seconds" "$millis"
}

canonical_compare() {
  local left=$1
  local right=$2
  local label=$3
  jq -S . "$left" > "$OUT_DIR/.left.canonical.json"
  jq -S . "$right" > "$OUT_DIR/.right.canonical.json"
  cmp "$OUT_DIR/.left.canonical.json" "$OUT_DIR/.right.canonical.json"
  rm -f -- "$OUT_DIR/.left.canonical.json" "$OUT_DIR/.right.canonical.json"
  printf 'VERIFIED %s\n' "$label"
}

require_file "$BINARY"
require_file "$PYTHON"
require_file "$ENV_FILE"
require_dir "$VAULT"
require_dir "$D1_VAULT"
require_dir "$D2_VAULT"
require_file "$D1_ROOT/kernel-health.json"
require_file "$D1_ROOT/kernel-top40-physical.json"
require_file "$D2_ROOT/judge-intel-core.json"
require_file "$D2_ROOT/judge-8055-kernel-health.json"
require_dir "$D3_ROOT/generation"
require_file "$D4_ROOT/dissent-intelligence.json"
require_file "$SCRIPT_DIR/cite_check.py"
require_file "$SCRIPT_DIR/fixtures/sanctions_shield_output.json"

export CALYX_HOME
export CUDA_VISIBLE_DEVICES=${CUDA_VISIBLE_DEVICES:-0}
# The file supplies model/runtime credentials. Its contents are never printed.
# shellcheck disable=SC1090
source "$ENV_FILE"

section_begin preflight "PREFLIGHT - live production vault and resident"
printf 'Scope: Cuyahoga County opinions only. No trial-court docket documents.\n'
printf 'Runtime: %s\n' "$BINARY"
printf 'Vault:   %s\n' "$VAULT"
printf 'Live Base readback is shared by demos 2-4; it is not cached source text.\n'

capture_command "$OUT_DIR/resident.before.json" "$OUT_DIR/resident.before.stderr" \
  "$BINARY" panel resident ready --addr "$RESIDENT_ADDR"
jq -e '
  .ready == true and
  .gpu_content_lens_count == 9 and
  .cpu_content_lens_count == 0 and
  .warmed_lens_count == 9
' "$OUT_DIR/resident.before.json" >/dev/null

capture_command "$OUT_DIR/live-base.json" "$OUT_DIR/live-base.stderr" \
  "$BINARY" readback cx-list --vault "$VAULT" --allow-unbounded \
  --progress-jsonl "$OUT_DIR/live-base.progress.jsonl"
jq -e 'length == 12596 and all(.[]; .metadata.jurisdiction_county == "Cuyahoga")' \
  "$OUT_DIR/live-base.json" >/dev/null
sha256sum "$OUT_DIR/live-base.json" > "$OUT_DIR/live-base.sha256"
jq '{ready,process_id,gpu_content_lens_count,cpu_content_lens_count,warmed_lens_count}' \
  "$OUT_DIR/resident.before.json"
printf 'LIVE_BASE rows=12596 bytes=%s\n' "$(wc -c < "$OUT_DIR/live-base.json")"
section_end

section_begin demo1 "DEMO 1 - doctrine kernel and top-10 reading list"
printf 'Mode: fresh scoped-vault kernel readback plus live Base join of a verified heavy ranking artifact.\n'
printf 'Why precomputed: ingest and graph construction are build steps, not a 15-minute stage action.\n'

printf '%s  %s\n' \
  'df764a94aa8f9b144b64d36b5cfeddc8253430d8c891f2a501121f02c74f482c' \
  "$D1_ROOT/kernel-health.json" | sha256sum -c -
printf '%s  %s\n' \
  '4a25f8c8e9244d3fc9aa98213be9d3eb01fddda7f993091b7043e1a9c1fc0f83' \
  "$D1_ROOT/kernel-top40-physical.json" | sha256sum -c -

capture_command "$OUT_DIR/demo1-kernel-health.json" "$OUT_DIR/demo1-kernel-health.stderr" \
  "$BINARY" readback kernel-health --root "$D1_VAULT" --kernel-id "$D1_KERNEL"
canonical_compare "$OUT_DIR/demo1-kernel-health.json" "$D1_ROOT/kernel-health.json" \
  "demo-1 kernel health equals immutable accepted bytes"

capture_command "$OUT_DIR/demo1-live-base.json" "$OUT_DIR/demo1-live-base.stderr" \
  "$BINARY" readback cx-list --vault "$D1_VAULT" --allow-unbounded
jq -e 'length == 3282' "$OUT_DIR/demo1-live-base.json" >/dev/null
jq -n \
  --slurpfile base "$OUT_DIR/demo1-live-base.json" \
  --slurpfile ranking "$D1_ROOT/kernel-top40-physical.json" '
    INDEX($base[0][]; .cx_id) as $live
    | [$ranking[0][0:10][] as $rank
       | $live[$rank.cx_id] as $row
       | {
           cx_id: $rank.cx_id,
           case_name: $row.metadata.case_name,
           date_filed: $row.metadata.date_filed,
           opinion_id: $row.metadata.opinion_id,
           source_url: $row.metadata.source_url,
           centrality_score: $rank.centrality_score,
           degree: $rank.degree
         }]
  ' > "$OUT_DIR/demo1-top10-live.json"
jq -e 'length == 10 and all(.[]; .case_name != null and .source_url != null)' \
  "$OUT_DIR/demo1-top10-live.json" >/dev/null
jq -r '
  "KERNEL members=\(.size) graph_members=\(.kernel_graph_size) recall=\(.recall.ratio) n=\(.recall.n_queries_tested) grounded=\(.grounded_fraction)"
' "$OUT_DIR/demo1-kernel-health.json"
jq -r 'to_entries[] | "  \(.key + 1). \(.value.case_name) (\(.value.date_filed))"' \
  "$OUT_DIR/demo1-top10-live.json"
section_end

section_begin demo2 "DEMO 2 - Mary J. Boyle authority diet"
printf 'Mode: fresh judge-vault kernel/Base readback plus verified citation-diet projection joined to live production Base rows.\n'
printf 'Use boundary: descriptive corpus history, never a vote or outcome prediction.\n'

printf '%s  %s\n' \
  '9f2a4a28f3e815703de0940cf1b33336055245bd0e7dbecba08f70c4565f1f27' \
  "$D2_ROOT/judge-intel-core.json" | sha256sum -c -
capture_command "$OUT_DIR/demo2-kernel-health.json" "$OUT_DIR/demo2-kernel-health.stderr" \
  "$BINARY" readback kernel-health --root "$D2_VAULT" --kernel-id "$D2_KERNEL"
canonical_compare "$OUT_DIR/demo2-kernel-health.json" "$D2_ROOT/judge-8055-kernel-health.json" \
  "Mary J. Boyle kernel health equals immutable accepted bytes"
capture_command "$OUT_DIR/demo2-live-base.json" "$OUT_DIR/demo2-live-base.stderr" \
  "$BINARY" readback cx-list --vault "$D2_VAULT" --allow-unbounded
jq -e 'length == 862' "$OUT_DIR/demo2-live-base.json" >/dev/null
jq '.judges[] | select(.person_id == 8055)' "$D2_ROOT/judge-intel-core.json" \
  > "$OUT_DIR/demo2-judge.json"
jq -n \
  --slurpfile base "$OUT_DIR/live-base.json" \
  --slurpfile judge "$OUT_DIR/demo2-judge.json" '
    INDEX($base[0][]; .cx_id) as $live
    | [$judge[0].in_corpus_authorities.top20[0:5][] as $authority
       | $live[$authority.cx_id] as $row
       | $authority + {
           live_case_name: $row.metadata.case_name,
           live_opinion_id: $row.metadata.opinion_id,
           live_source_url: $row.metadata.source_url
         }]
  ' > "$OUT_DIR/demo2-top5-live.json"
jq -e '
  length == 5 and
  all(.[]; .case_name == .live_case_name and .opinion_id == .live_opinion_id)
' "$OUT_DIR/demo2-top5-live.json" >/dev/null
jq -r '
  "JUDGE \(.judge) authored_n=\(.canonical_authored_n) dissent=\(.profile.dissent_n)/\(.canonical_authored_n) in_corpus_edges=\(.in_corpus_authorities.edge_n) frontier_edges=\(.frontier_authorities.edge_n)"
' "$OUT_DIR/demo2-judge.json"
jq -r 'to_entries[] | "  \(.key + 1). \(.value.case_name): edges=\(.value.edge_count) depth=\(.value.total_depth)"' \
  "$OUT_DIR/demo2-top5-live.json"
section_end

section_begin demo3 "DEMO 3 - sanctions-shield cite check"
printf 'Mode: VERIFIED PRECOMPUTED HEAVY ARTIFACT plus live physical C12 target readback.\n'
printf 'The full 32-search build measured 13:26.58, so the stage action verifies every immutable member and displays the catch.\n'

printf '%s  %s\n' \
  '62c40d258aad9705eef5668bd9c046e6dc6f53c8ebaf409d24629bdd2aa0b615' \
  "$D3_ROOT/generation/cite_check_report.json" | sha256sum -c -
print_command "$PYTHON" "$SCRIPT_DIR/cite_check.py" verify --generation "$D3_ROOT/generation"
"$PYTHON" "$SCRIPT_DIR/cite_check.py" verify --generation "$D3_ROOT/generation" \
  > "$OUT_DIR/demo3-verify.json" 2> "$OUT_DIR/demo3-verify.stderr"
cmp "$SCRIPT_DIR/fixtures/sanctions_shield_output.json" \
  "$D3_ROOT/generation/cite_check_report.json"
printf 'VERIFIED committed cite-check output is byte-identical to the accepted generation\n'
jq '
  .[] | select(.cx_id == "94c933e0453ecd05571c9d170dd496ba")
  | {
      cx_id,
      case_name: .metadata.case_name,
      docket_number: .metadata.docket_number,
      opinion_id: .metadata.opinion_id,
      source_url: .metadata.source_url,
      input_pointer: .input_ref.pointer
    }
' "$OUT_DIR/live-base.json" > "$OUT_DIR/demo3-c12-live.json"
jq -e '
  .case_name == "State v. Bonnell" and
  .docket_number == "96368" and
  .opinion_id == "2704114"
' "$OUT_DIR/demo3-c12-live.json" >/dev/null
jq -r '
  "CITATIONS \(.counts.citations): found=\(.counts.found) not_in_corpus=\(.counts.not_in_corpus) low_support=\(.counts.low_support) agreement=\(.counts.reviewed_agreement)"
' "$D3_ROOT/generation/cite_check_report.json"
jq -r '
  .rows[]
  | select(.corpus_verdict == "NOT_IN_CORPUS" or .support_verdict == "LOW_SUPPORT")
  | "  \(.citation_id) \(.case_name): corpus=\(.corpus_verdict) support=\(.support_verdict) gain=\(.support_score)"
' "$D3_ROOT/generation/cite_check_report.json"
section_end

section_begin demo4 "DEMO 4 - dissent intelligence"
printf 'Mode: verified accepted analytics plus a fresh opinion-type census over the shared live Base readback.\n'
printf 'The citation-traction result is a structural null, not a legal conclusion.\n'

printf '%s  %s\n' \
  'c8087cc7d6118bb71e81bf897d79abf8216d4b0b3eb75ef4251ef0571c47f936' \
  "$D4_ROOT/dissent-intelligence.json" | sha256sum -c -
jq '
  group_by(.metadata.opinion_type)
  | map({key: .[0].metadata.opinion_type, value: length})
  | from_entries
' "$OUT_DIR/live-base.json" > "$OUT_DIR/demo4-live-opinion-types.json"
jq -e \
  --slurpfile live "$OUT_DIR/demo4-live-opinion-types.json" '
    .physical_opinions_n == 12596 and
    .opinion_type_counts == $live[0] and
    .dissents_n == 55 and
    .concurrence_family_n == 42
  ' "$D4_ROOT/dissent-intelligence.json" >/dev/null
jq -r '
  "DISSENTS \(.dissents_n)/\(.physical_opinions_n); concurrence_family=\(.concurrence_family_n)/\(.physical_opinions_n); resolved_judge_rows=\(.resolved_physical_n); unresolved=\(.unresolved_physical_n)",
  (.communities | sort_by(-.dissent_density)[0]
    | "  community=\(.community) dissents=\(.dissents_n)/\(.members_n) density=\(.dissent_density)"),
  "  accepted_citation_edges=\(.citation_overlay.accepted_unique_edges_n) dissent_targets=\(.citation_overlay.edges_targeting_dissents_n)"
' "$D4_ROOT/dissent-intelligence.json"
section_end

section_begin demo5 "DEMO 5 - live paths, refusal, and cryptographic replay"
printf 'Mode: live resident measurement and persisted association-graph derivation. No LLM answer generator.\n'
printf 'Authority boundary: admission uses kernel slot 8; fused-panel repair is tracked in #1857.\n'

Q1='When does prolonging a traffic stop require suppression of evidence in an Ohio criminal appeal?'
Q4='What evidence establishes standing to enforce a note and mortgage in an Ohio foreclosure action?'
Q6='Under Delaware law, when does a corporate director breach fiduciary duties owed to stockholders?'

capture_command "$OUT_DIR/demo5-q1.json" "$OUT_DIR/demo5-q1.stderr" \
  "$BINARY" kernel-answer "$VAULT" "$Q1" --resident-addr "$RESIDENT_ADDR"
capture_command "$OUT_DIR/demo5-q4.json" "$OUT_DIR/demo5-q4.stderr" \
  "$BINARY" kernel-answer "$VAULT" "$Q4" --resident-addr "$RESIDENT_ADDR"
jq -e '.status == "grounded" and .physical_readback == "verified"' \
  "$OUT_DIR/demo5-q1.json" "$OUT_DIR/demo5-q4.json" >/dev/null

capture_command "$OUT_DIR/demo5-ledger-before-refusal.json" "$OUT_DIR/demo5-ledger-before-refusal.stderr" \
  "$BINARY" ledger-tail --vault "$VAULT" --last 1
capture_command "$OUT_DIR/demo5-refusal.json" "$OUT_DIR/demo5-refusal.stderr" \
  "$BINARY" kernel-answer "$VAULT" "$Q6" --resident-addr "$RESIDENT_ADDR"
capture_command "$OUT_DIR/demo5-ledger-after-refusal.json" "$OUT_DIR/demo5-ledger-after-refusal.stderr" \
  "$BINARY" ledger-tail --vault "$VAULT" --last 1
cmp "$OUT_DIR/demo5-ledger-before-refusal.json" "$OUT_DIR/demo5-ledger-after-refusal.json"
jq -e '
  .status == "refused" and
  .code == "CALYX_KERNEL_QUERY_OUT_OF_SCOPE" and
  .reason == "explicit_query_jurisdiction_conflicts_with_kernel_scope" and
  .detected_scope == "Delaware"
' "$OUT_DIR/demo5-refusal.json" >/dev/null
printf 'VERIFIED refusal made no ledger mutation\n'

ANSWER_ID=$(jq -er '.answer_id' "$OUT_DIR/demo5-q1.json")
capture_command "$OUT_DIR/demo5-reproduce.json" "$OUT_DIR/demo5-reproduce.stderr" \
  "$BINARY" reproduce --record --resident-addr "$RESIDENT_ADDR" "$VAULT" "$ANSWER_ID"
jq -e '.bit_parity == true and .original_hash == .reproduced_hash' \
  "$OUT_DIR/demo5-reproduce.json" >/dev/null

jq -n \
  --slurpfile base "$OUT_DIR/live-base.json" \
  --slurpfile answer "$OUT_DIR/demo5-q1.json" '
    INDEX($base[0][]; .cx_id) as $live
    | $answer[0] as $a
    | {
        question: "traffic-stop suppression",
        answer_id: $a.answer_id,
        nearest_similarity: $a.nearest_similarity,
        admission_threshold: $a.admission_threshold,
        hops: ($a.hops | length),
        path: (([$a.anchor_kernel_node_id] + [$a.hops[].to_cx_id])
          | map($live[.].metadata.case_name))
      }
  ' > "$OUT_DIR/demo5-q1-path.json"
jq -n \
  --slurpfile base "$OUT_DIR/live-base.json" \
  --slurpfile answer "$OUT_DIR/demo5-q4.json" '
    INDEX($base[0][]; .cx_id) as $live
    | $answer[0] as $a
    | {
        question: "foreclosure standing",
        answer_id: $a.answer_id,
        nearest_similarity: $a.nearest_similarity,
        admission_threshold: $a.admission_threshold,
        hops: ($a.hops | length),
        path: (([$a.anchor_kernel_node_id] + [$a.hops[].to_cx_id])
          | map($live[.].metadata.case_name))
      }
  ' > "$OUT_DIR/demo5-q4-path.json"
jq -r '
  "ANSWER \(.question): similarity=\(.nearest_similarity) threshold=\(.admission_threshold) hops=\(.hops)",
  "  path: \(.path | join(" -> "))"
' "$OUT_DIR/demo5-q1-path.json" "$OUT_DIR/demo5-q4-path.json"
jq -r '"REFUSAL code=\(.code) reason=\(.reason) scope=\(.detected_scope)"' \
  "$OUT_DIR/demo5-refusal.json"
jq -r '"REPRODUCE parity=\(.bit_parity) hash=\(.reproduced_hash)"' \
  "$OUT_DIR/demo5-reproduce.json"
section_end

section_begin final_audit "FINAL AUDIT - full ledger chain and resident"
capture_command "$OUT_DIR/chain.final.json" "$OUT_DIR/chain.final.stderr" \
  "$BINARY" verify-chain "$VAULT" --progress-jsonl "$OUT_DIR/chain.final.progress.jsonl"
jq -e '.status == "ok" and .break_at == null' "$OUT_DIR/chain.final.json" >/dev/null
capture_command "$OUT_DIR/resident.after.json" "$OUT_DIR/resident.after.stderr" \
  "$BINARY" panel resident ready --addr "$RESIDENT_ADDR"
jq -e '
  .ready == true and
  .gpu_content_lens_count == 9 and
  .cpu_content_lens_count == 0 and
  .process_id == $pid
' --argjson pid "$(jq '.process_id' "$OUT_DIR/resident.before.json")" \
  "$OUT_DIR/resident.after.json" >/dev/null
printf 'CHAIN status=ok break_at=null\n'
jq -r '"RESIDENT pid=\(.process_id) gpu_lenses=\(.gpu_content_lens_count) cpu_lenses=\(.cpu_content_lens_count) ready=\(.ready)"' \
  "$OUT_DIR/resident.after.json"
section_end

jq -n \
  --slurpfile d1h "$OUT_DIR/demo1-kernel-health.json" \
  --slurpfile d1top "$OUT_DIR/demo1-top10-live.json" \
  --slurpfile d2h "$OUT_DIR/demo2-kernel-health.json" \
  --slurpfile d2j "$OUT_DIR/demo2-judge.json" \
  --slurpfile d2top "$OUT_DIR/demo2-top5-live.json" \
  --slurpfile d3 "$D3_ROOT/generation/cite_check_report.json" \
  --slurpfile d4 "$D4_ROOT/dissent-intelligence.json" \
  --slurpfile d4live "$OUT_DIR/demo4-live-opinion-types.json" \
  --slurpfile q1 "$OUT_DIR/demo5-q1-path.json" \
  --slurpfile q4 "$OUT_DIR/demo5-q4-path.json" \
  --slurpfile refusal "$OUT_DIR/demo5-refusal.json" \
  --slurpfile reproduce "$OUT_DIR/demo5-reproduce.json" \
  --slurpfile resident "$OUT_DIR/resident.after.json" '
  {
    schema: "calyx.law.demo-harness-headline.v1",
    scope: {county: "Cuyahoga", physical_opinions: 12596},
    resident: {
      gpu_content_lenses: $resident[0].gpu_content_lens_count,
      cpu_content_lenses: $resident[0].cpu_content_lens_count
    },
    demo1: {
      scoped_opinions: 3282,
      kernel_members: $d1h[0].size,
      kernel_graph_members: $d1h[0].kernel_graph_size,
      recall: $d1h[0].recall.ratio,
      recall_n: $d1h[0].recall.n_queries_tested,
      grounded_fraction: $d1h[0].grounded_fraction,
      top10: ($d1top[0] | map({case_name, date_filed, opinion_id}))
    },
    demo2: {
      judge: $d2j[0].judge,
      authored_opinions: $d2j[0].canonical_authored_n,
      kernel_members: $d2h[0].size,
      recall: $d2h[0].recall.ratio,
      dissent_n: $d2j[0].profile.dissent_n,
      in_corpus_edges: $d2j[0].in_corpus_authorities.edge_n,
      frontier_edges: $d2j[0].frontier_authorities.edge_n,
      top5: ($d2top[0] | map({case_name, opinion_id, edge_count, total_depth}))
    },
    demo3: {
      counts: $d3[0].counts,
      flagged: ($d3[0].rows[] | select(.citation_id == "C12")
        | {citation_id, case_name, corpus_verdict, support_verdict, support_score})
    },
    demo4: {
      live_opinion_types: $d4live[0],
      dissents_n: $d4[0].dissents_n,
      concurrence_family_n: $d4[0].concurrence_family_n,
      top_dissent_community: ($d4[0].communities | sort_by(-.dissent_density)[0]
        | {community, members_n, dissents_n, dissent_density}),
      accepted_citation_edges: $d4[0].citation_overlay.accepted_unique_edges_n,
      dissent_target_edges: $d4[0].citation_overlay.edges_targeting_dissents_n
    },
    demo5: {
      answers: [$q1[0], $q4[0]],
      refusal: ($refusal[0] | {code, reason, detected_scope}),
      reproduction: ($reproduce[0] | {bit_parity, original_hash, reproduced_hash})
    }
  }
' > "$OUT_DIR/headline.json"
jq -S . "$OUT_DIR/headline.json" > "$OUT_DIR/headline.canonical.json"

if [[ -n "$EXPECTED_HEADLINE" ]]; then
  require_file "$EXPECTED_HEADLINE"
  jq -S . "$EXPECTED_HEADLINE" > "$OUT_DIR/expected-headline.canonical.json"
  cmp "$OUT_DIR/expected-headline.canonical.json" "$OUT_DIR/headline.canonical.json"
  printf '\nDETERMINISM verified: headline numbers equal %s\n' "$EXPECTED_HEADLINE"
fi

jq -s '{sections: ., total_ms: (map(.elapsed_ms) | add)}' "$TIMING_JSONL" \
  > "$OUT_DIR/timing.json"
RUN_ENDED_MS=$(now_ms)
TOTAL_MS=$((RUN_ENDED_MS - RUN_STARTED_MS))
printf '{"wall_elapsed_ms":%s}\n' "$TOTAL_MS" > "$OUT_DIR/wall-time.json"

find "$OUT_DIR" -maxdepth 1 -type f ! -name SHA256SUMS -print0 \
  | sort -z | xargs -0 sha256sum > "$OUT_DIR/SHA256SUMS"
sha256sum -c "$OUT_DIR/SHA256SUMS" >/dev/null

printf '\n================================================================\n'
printf 'DEMO COMPLETE\n'
printf '================================================================\n'
printf 'wall_time=%d.%03ds\n' "$((TOTAL_MS / 1000))" "$((TOTAL_MS % 1000))"
printf 'headline=%s\n' "$OUT_DIR/headline.json"
printf 'timing=%s\n' "$OUT_DIR/timing.json"
printf 'checksums=%s\n' "$OUT_DIR/SHA256SUMS"
printf 'No LLM answer generator, CPU model fallback, tests, or CI were used.\n'
