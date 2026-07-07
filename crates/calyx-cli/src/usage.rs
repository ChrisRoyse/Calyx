use crate::error::{CliError, CliResult};
use crate::output::print_lines;

pub(crate) fn print_usage() -> CliResult {
    print_lines(&[
        usage().to_string(),
        "prints source-of-truth bytes or listings for manual FSV inspection".to_string(),
        "merkle-root --vault reads Aster cf/ledger plus wal; no side ledger dir is created"
            .to_string(),
    ])
    .map(|_| ())
}

pub(crate) fn print_command_usage(command: &str) -> CliResult {
    let line = command_usage(command)
        .ok_or_else(|| CliError::usage(format!("missing usage text for command {command}")))?;
    print_lines(&[format!("usage: {}", line.trim_start())]).map(|_| ())
}

fn command_usage(command: &str) -> Option<&'static str> {
    let prefix = format!("calyx {command}");
    usage().lines().find(|line| {
        let trimmed = line.trim_start();
        trimmed == prefix || trimmed.starts_with(&format!("{prefix} "))
    })
}

pub(crate) fn usage() -> &'static str {
    "usage: calyx readback (--hex <file> | --vault-tree <dir> | --cf-row <vault> --cf <cf-name> --key <hex-key> | --wal <segment-path> | --ledger <vault> --seq <n> | --vault <dir> --verify-against <sqlite.db> | --vault <dir> --show-manifest | vault-manifest --field <name> --vault <dir> | partitioned-manifest --vault <dir> | temporal_search --explain --clock-fixed <secs> --tz-offset <secs> | dedup-check --vault <dir> --cx-id <cx> --slot <n> --tau <f> --near-cos <f> --distinct-cos <f> --vault-id <id> --salt <s> | kernel-health --root <dir> --kernel-id <cx> | recurrence-series --vault <dir> --cx-id <cx> | periodic-recall --vault <dir> (--hour <0-23> | --day <0-6>) [--hour <0-23>] [--day <0-6>] | oracle_self_consistency --vault <dir> --domain <domain> --vault-id <id> --salt <s> | oracle_sufficiency --vault <dir> --fixture <json> --vault-id <id> --salt <s> | oracle_predict --vault <dir> --fixture <json> --vault-id <id> --salt <s> | oracle_expand --vault <dir> --fixture <json> --vault-id <id> --salt <s> [--depth <0-4>] | reverse_query --vault <dir> --domain <domain> --answer <text> --fixture <json> --vault-id <id> --salt <s> | super_intelligence --vault <dir> --domain <domain> --fixture <json> --vault-id <id> --salt <s> | temporal-log-recurrence --log <csv> --vault <dir> --out <json> --rows <n> --expected-cadence-secs <secs> --confidence-ceiling <f> | time-prediction --vault <dir> --cx-id <cx> --confidence-ceiling <f> | assay-report|temporal-cross-term|kernel-weights|kernel-window|ward-novelty|compression-ratio|compression-report|anneal-schedule --artifact <json> [--field <path>] | config <tripwire|budget> --vault <dir> | ledger --kind Anneal --action <GoodhartPassed|GoodhartFailed> --last <n> --vault <dir> | anneal mistakes --vault <dir> --last <n> | dedup-audit --vault <dir> --cx-id <cx> | dedup-undo --vault <dir> --token <json> | cx-list --vault <dir> [--cx-id <cx>|--limit <n>|--include-slots|--allow-unbounded|--rebuild-base-page-index|--base-page-index-page-size <n>|--progress-jsonl <stderr|path>|--time-budget-ms <ms>] | time-index --vault <dir> | as-of --vault <dir> --t-millis <ms> | --cf <name> --vault <dir> [--seq <n>] | --cf <name> --level <dir> | --wal --vault <dir>)
       calyx resource-status --vault <dir> [--metrics]
       calyx create-vault <name> [--panel-template <text-default|code-default|civic-default|legal-default|medical-default|bio-default|media-default>]
       calyx add-lens <vault> --name <n> --runtime <algorithmic|tei-http|external-cmd|candle-local|onnx|multimodal-adapter> [--endpoint <url-or-runtime-id>] [--weights <path>] [--shape Dense(<dim>)|Sparse(<dim>)|Multi(<token_dim>)] [--modality <text|code|image|audio|video|structured|mixed>]
       calyx retire-lens <vault> --slot <u16>
       calyx park-lens <vault> --slot <u16>
       calyx retire-vault <vault> --reason <text> [--superseded-by <replacement-vault-or-corpus> --source-issue <issue> --fsv-readback <json> --fsv-sha256 <sha256>]
       calyx list-panel <vault>
       calyx profile-lens [--name <n>] [--runtime <r>] [--endpoint <url-or-runtime-id>] [--weights <path>] [--shape Dense(<dim>)|Sparse(<dim>)] [--modality <m>] [--probe <path>]
       calyx ingest <vault> (--text <s> | --batch <jsonl-path> | --file <path> --modality <image|audio|video>) [--idempotent] [--output <summary|rows>] [--resident-addr <127.0.0.1:port>] [--session-id <id>]
       calyx ingest-status <vault> --session <id>
       calyx anchor <vault> <cx_id> --kind <test-pass|thumbs-up|thumbs-down|label:<s>|speaker-match|style-hold> --value <v> [--confidence <0..1>] [--source <s>]
       calyx measure <vault> --text <s>
       calyx erase <vault> --cx-id <cx_id> [--fsv-out <json>]
       calyx search <vault> <query> [--k <n>] [--fusion <rrf|weighted-rrf|single-lens|kernel-first|pipeline>] [--guard <off|in-region>] [--explain] [--provenance|--no-provenance] [--fresh|--stale-ok] [--filter <json-predicate>] [--resident-addr <127.0.0.1:port>]
       calyx probe-matrix <vault> --frontier <text> [--slot <u16>] [--weighted-profile <name>] [--phrasing <terse|clinical|mechanistic|analogical|contrast>] [--length <entity|phrase|paragraph>] [--top-k <n>] [--guard <off|in-region>] [--guard-tau <cosine in (0,1]>] [--stale-ok] [--out <json>] [--resident-addr <127.0.0.1:port>] [--max-variants <n>] [--time-budget-ms <ms>] [--search-miss-budget-ms <ms>] [--search-hit-budget-ms <ms>]
       calyx kernel-answer <vault> <query> [--anchor <kind>] [--explain]
       calyx bits <vault> <anchor-kind> [--explain]
       calyx kernel <vault> [--anchor <kind>] [--rebuild]
       calyx guard <vault> <calibrate|check|generate> [args]
       calyx abundance <vault>
       calyx propose-lens <vault> --anchor <kind>
       calyx provenance <vault> <cx_id>
       calyx verify-chain [--vault] <vault> [--from <seq>] [--to <seq>] [--batch-size <n>] [--progress-jsonl <stderr|path>] [--time-budget-ms <ms>]
       calyx rebuild-search-index <vault>
       calyx kernel-build <vault> [--held-out-fraction <0..1>] [--top-k <n>] [--min-recall <0..1>]
       calyx weave-loom <vault> [--content-slot <u16>] [--candidate-selection covered|base-prefix] [--coverage-only] [--knn <n>] [--edge-cos-threshold <0..1>] [--max-groundedness-distance <n>] [--batch <n>] [--limit <n>] [--time-budget-ms <ms>]
       calyx discovery-chain <vault> --start <cx> (--anchor <cx>|--anchor-file <path>) [--assay-domain <domain>] [--assay-anchor <reward|label:name|test_pass|tie_formed|thumbs|speaker_match|style_hold|recurrence>] [--max-hops <n>] [--branch-width <n>] [--probe-width <n>] [--max-groundedness-distance <n>] [--min-gate-confidence <f>] [--novelty-weight <f>] [--out <json>]
       calyx chain-walks <vault> --seed-file <json> (--anchor <cx>|--anchor-file <path>) [--assay-domain <domain>] [--assay-anchor <reward|label:name|test_pass|tie_formed|thumbs|speaker_match|style_hold|recurrence>] [--max-hops <n>] [--branch-width <n>] [--probe-width <n>] [--max-groundedness-distance <n>] [--min-gate-confidence <f>] [--novelty-weight <f>] [--max-hypotheses-per-seed <n>] [--min-terminal-confidence <f>] [--out <json>]
       calyx assemble-hypothesis-evidence <vault> --chain <chain.json> --out <input.json>
       calyx hypothesis-evaluator-driver --input <input.json> --out <artifact.json> --endpoint <http(s)://host[:port]/path> --model <id> [--auth-env <VAR>] [--temperature <f>] [--timeout-ms <ms>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx hypothesis-evaluate --input <input.json> --out <artifact.json> [--min-runs-per-hypothesis <n>] [--min-prompt-variants <n>] [--min-temperature-variants <n>] [--min-retrieved-evidence <n>] [--retain-score-floor <0..1>] [--max-ranked <n>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx hypothesis-rank --input <input.json> --out <artifact.json> [--max-ranked <n>] [--review-top-n <n>] [--min-review-score <0..1>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx bridge-falsification-evaluate --miner-report <json> --falsification-report <json> --out <json> [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx bridge-evaluate-rank --evaluation-report <json> --out <json> [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx novelty-calibration-split --atlas <issue>|<domain>|<jsonl> [--atlas <issue>|<domain>|<jsonl> ...] --out-dir <dir> [--top-k <n>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx discovery-run (seal --manifest <manifest.json> --ledger <ledger-dir> --out <seal.json> | reproduce --manifest <manifest.json> --observed <observed.json> --out <report.json> | verify --manifest <manifest.json> --ledger <ledger-dir> --seq <n> --out <verify.json>)
       calyx materialize-bridge-corpus <name> --rows <jsonl> [--home <dir>]
       calyx materialize-molecular-vault <vault> --rows <jsonl> [--home <dir>]
       calyx materialize-evidence-substrate <vault> --pubtator-root <dir> --clinicaltrials-root <dir> --dgidb-root <dir> [--collection <name>] [--report <json>] [--home <dir>]
       calyx materialize-lincs-reversal <vault> --root <dir> [--metadata-root <dir>] [--collection <name>] [--report <json>] [--home <dir>]
       calyx association-validation-gates --typed-root <dir> --open-targets-root <dir> --pubtator-root <dir> --clinicaltrials-root <dir> --dgidb-root <dir> --out-dir <dir> [--cutoff-year <yyyy>] [--score-threshold <0..1>] [--min-auroc <0..1>] [--min-positive-recall <0..1>] [--min-negative-suppression <0..1>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx typed-association-miner --typed-root <dir> --validation-report <json> --out-dir <dir> [--source-type <concept-type>] [--target-type <concept-type>] [--name-contains <text>] [--source-issue <n>] [--min-support <n>] [--max-pairs <n>] [--max-input-edges <n>] [--max-paths-per-pair <n>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx hypothesis-falsification-sweep --hypotheses-report <json> [--hypotheses-report <json> ...] --pubtator-root <dir> --clinicaltrials-root <dir> --dgidb-root <dir> --open-targets-root <dir> --out-dir <dir> [--max-hypotheses <n>] [--run-manifest <manifest.json> --run-stage-id <stage-id>]
       calyx graph-collection-generations <vault> [--collection <name>] [--home <dir>]
       calyx graph-collection-state <vault> --collection <name> --generation <id> --state <writing|accepted|failed|tombstoned> --command <name> [--reason <text>] [--detail <k=v>] [--home <dir>]
       calyx reproduce <vault> <answer_id>
       calyx anneal-status <vault>
       calyx healthcheck [--vault <vault>] [--json|--no-json] [--tei <http://host:port[/path]>]
       calyx build-info
       calyx resource-drill --vault <dir> --ops <n> --value-bytes <n> --memtable-cap <bytes> --pin-max-age-ms <ms>
       calyx migrate vault <sqlite.db> <vault.calyx> [--verify] [--backfill-default-panel] [--offline-backfill] [--batch-size <n>]
       calyx migrate backfill <sqlite.db> <vault.calyx> [--offline-backfill] [--batch-size <n>]
       calyx migrate verify <sqlite.db> <vault.calyx> [--require-backfill]
       calyx migrate status <vault.calyx>
       calyx migrate readback <sqlite.db> <vault.calyx> <chunk_id>
       calyx healthcheck [--wait <secs>] [--out <json>] [--secret-env <env>] [--calyx-home <dir>] [--vault <dir>] [--metrics-url <url>] [--require-env <name>]
       calyx healthcheck --config <calyx.toml> [--wait <secs>] [--out <json>]   (daemon-readiness: CUDA + VRAM + vault read)
       calyx lens add --manifest <manifest.json> [--home <dir>]
       calyx lens card --manifest <manifest.json> [--input <text>|--input-file <path>]
       calyx lens list [--home <dir>]
       calyx lens migrate-catalog [--home <dir>] [--from <registry.json>]
       calyx lens remove (--name <name>|--lens-id <id>) [--home <dir>]
       calyx lens commission --hf <id|fastembed-model> --runtime <onnx-int8|onnx-fp32|onnx-colbert|fastembed-onnx|fastembed-sparse|fastembed-bgem3-*|fastembed-reranker|fastembed-qwen3|candle-fp16|tei> [--home <dir>] [--out <dir>] [--name <n>] [--endpoint <url>] [--dim <n>] [--max-batch <n>]
       calyx lens explain --manifest <manifest.json> [--input <text>|--input-file <path>] [--repeat <n>] [--full-vector]
       calyx lens scale-audit --manifest <manifest.json> [--manifest <manifest.json> ...] --out <report.json> [--batch-size <n>] [--min-content-lenses <n>] [--min-gpu-content-lenses <n>] [--min-effective-batch <n>] [--lens-timeout-secs <n>] [--probe <text>] [--probe-file <path>]
       calyx panel template seed [--home <dir>]
       calyx panel template save --name <name> (--all-current [--modality <m>] | --lens <name-or-id> ...) [--home <dir>] [--notes <text>]
       calyx panel template list [--home <dir>]
       calyx panel template fork --from <name-or-id> --name <name> [--home <dir>] [--notes <text>]
       calyx panel template profile --template <name-or-id> (--card <json> ... | --card-dir <dir>) [--assay-card <ensemble_card.json>] [--home <dir>]
       calyx panel template swap --template <name-or-id> --vault <vault> [--require-a37-gate] [--home <dir>]
       calyx panel batch-limit --vault <vault> --set <name-or-id>=<max_batch> [--set <name-or-id>=<max_batch> ...] [--preflight-text <text>] [--preflight-repeat <n>]
       calyx panel registry-audit --vault <vault>
       calyx panel registry-repair --vault <vault> (--slot <u16>|--all)
       calyx panel manifest-restore --vault <vault> --panel-asset <panel/panel-vNNNN.json> --registry-asset <registry/registry-xxxx.json>
       calyx panel warm --template <name-or-id> [--home <dir>] [--hold-secs <n>] [--out <json>] [--progress-out <jsonl>] [--max-resident-vram-mib <n>] [--resident-overhead-multiplier <n>] [--max-load-secs <n>] [--load-parallelism <n>]
       calyx panel resident serve (--template <name-or-id>|--vault <vault>) [--home <dir>] [--modality <name>] [--slot <u16>]... [--bind <addr>] [--ready-out <json>] [--progress-out <jsonl>] [--max-resident-vram-mib <n>] [--resident-overhead-multiplier <n>] [--max-load-secs <n>] [--load-parallelism <n>]
       calyx panel resident ready [--addr <127.0.0.1:port>] [--out <json>]
       calyx panel resident measure [--addr <127.0.0.1:port>] --modality <name> (--input <text>|--input-file <path>|--input-hex <hex>) [--out <json>]
       calyx panel resident stop [--addr <127.0.0.1:port>] [--out <json>]
       calyx panel a38-bundle save --name <name> --base-template <name-or-id> --required-modality <m> --include-lens <name-or-id> --evidence <json> [--home <dir>] [--budget-vram-mib <n>]
       calyx panel a38-bundle list [--home <dir>]
       calyx assay corpus-build --rows-jsonl <rows.jsonl> --out-dir <dir> --dataset <name> --target-class <n> --manifest <manifest.json> [--manifest <manifest.json> ...] [--limit-per-class <n>] [--batch-size <n>] [--cost-override-json <json>]   (rows require exactly one of text or input_path)
       calyx assay ensemble-card --corpus-dir <dir> --metrics-dir <dir> [--cf-root <dir>] [--target-class <n>] [--domain <domain>] [--min-lenses <n>] [--min-marginal-bits <f>] [--max-redundancy <f>]
       calyx assay i8bin-labels --rows-jsonl <rows.jsonl> --cf-root <aster-dir> [--labels-key <key>|--association-key <key>] [--target-class <n>] [--derive-anchor <label|gdelt_root:04|gdelt_quad:4|gdelt_action_geo:US|gdelt_action_geo_fullname_contains:Gaza|gdelt_actor1_country:USA|gdelt_actor2_country:USA|gdelt_actor_country:USA|gdelt_actor_pair:A->B|gdelt_actor_country_pair:USA->CAN|gdelt_event_code:010|gdelt_event_root:01|gdelt_sqldate_prefix:202401|gdelt_source_host:example.org|gdelt_source_host_contains:news|gdelt_source_tld:org|gdelt_goldstein_sign:pos|gdelt_tone_sign:neg|gdelt_goldstein_bucket:5|gdelt_tone_bucket:9|gdelt_actor1_present|gdelt_actor2_present|source_domain_contains:text>] [--anchor-name <name>] [--limit-per-class <n>] [--chunk-rows <n>]
       calyx assay i8bin-ensemble-card (--plan <partitioned_rrf_plan.json>|--plan-cf-root <aster-dir> [--plan-key <key>]) (--labels-cf-root <aster-dir> [--labels-key <key>]|--rows-jsonl <rows.jsonl> diagnostic-only) (--metrics-dir <dir> [--cf-root <dir>] | --cf-root <dir> --db-only) [--stream-report <stream_fbin_report.json>] [--target-class <n>] [--domain <domain>] [--sample-rows <n>] [--signature-rows <n|all>] [--min-lenses <n>] [--min-marginal-bits <f>] [--max-redundancy <f>] [--mode <gate|diagnostic>|--diagnostic|--baseline] [--db-only|--no-artifacts]
       calyx assay multi-anchor-ensemble-card (--report <a37_i8bin_ensemble_report.json>|--assay-cf-report <cf-root> <domain> <target-class>)... (--out-dir <dir> [--cf-root <aster-dir>] | --cf-root <aster-dir> --db-only) [--association-key <key>] [--min-lenses <n>] [--min-marginal-bits <f>] [--max-redundancy <f>] [--mode <gate|diagnostic>|--diagnostic|--baseline] [--db-only|--no-artifacts]
       calyx assay multi-anchor-readback --cf-root <aster-dir> [--association-key <key>] [--limit-lenses <n>] [--limit-targets <n>]
       calyx assay gdelt-rows --source-dir <dir> --out <rows.jsonl> --manifest <manifest.json> [--dataset <name>] [--limit-per-class <n>|--max-rows <n>] [--actor-country <ISO3>] [--action-country <ISO2>] [--action-name-contains <text>]
       calyx assay export-fbin --corpus-dir <dir> --out-dir <dir> --bits-report <assay_abundance.json> --query-count <n> [--min-bits <f>]
       calyx assay stream-fbin-lens-template --tei <name> <endpoint> <dim> [--algorithmic <name> <kind> <dim> ...] --cf-root <aster-dir> [--lens-template-key <key>|--association-key <key>]
       calyx assay stream-fbin-lens-template --manifest <manifest.json> [--manifest <manifest.json> ...] --cf-root <aster-dir> [--lens-template-key <key>|--association-key <key>]
       calyx assay stream-fbin --rows-jsonl <rows.jsonl> --out-dir <dir> --dataset <name> --target-class <n> [--a37-admission-cf-root <aster-dir> [--a37-admission-key <key>]] --query-count <n> (--lens-template-cf-root <aster-dir> [--lens-template-key <key>] gate|--manifest <manifest.json> [--manifest <manifest.json> ...] diagnostic-only) [--limit-per-class <n>] [--batch-size <n>] [--cost-override-json <json>] [--min-bits <f>] [--vector-format <fbin|i8bin>] [--mode <gate|diagnostic>|--diagnostic|--baseline] [--bits-report <assay_abundance.json> diagnostic-only] [--db-only|--no-artifacts]
       calyx fsv corpus-readback --root <dir>
       calyx fsv vault-health --vault <vault> [--out <json>] [--write-quarantine]
       calyx anneal status --health --vault <dir>
       calyx build-bench-vault --vault <dir> --n-cx <n> --dim <n> --slots <n> --seed <n>
       calyx build-partitioned-vault --vault <dir> (--vectors <file.fbin|file.i8bin>|--n-cx <n> --dim <n>) --regions <n> [--distance-metric <unit-l2|raw-l2>] [--sample <n>] [--chunk <n>] [--m-max <n>] [--ef <n>] [--region-build-parallelism <n>] [--final-assignment-probe <n>] [--final-assignment-cap <n>] [--build-backend <cpu-vamana|cuvs-cagra>] [--progress-file <json>]
       calyx bench search --vault <dir> --strategy KernelFirst --n <n> --report p50,p99,p999 --seed <n> [--k <n>] [--beamwidth <n>] [--posting-cutoff <n>] [--tuner-slo-us <us>]
       calyx bench recall --vault <dir> --n <n> --k <n> [--seed <n>]
       calyx bench partitioned-search --vault <dir> --n <n> --k <n> --n-probe <n> --region-beam <n> [--ground-truth <n> --recall-floor <f>] [--anneal-vault <dir> --tuner-slo-us <us>]
       calyx bench partitioned-search --vault <dir> --queries <file.fbin|file.i8bin> [--corpus <file.fbin|file.i8bin>|--ground-truth-file <file.i32bin> [--ground-truth-id-map <file.i32bin>]] --n <n> --k <n> --n-probe <n> --region-beam <n> --ground-truth <n> --recall-floor <f>
       calyx bench partitioned-rrf-plan --plan <json> --cf-root <aster-dir> [--association-key <key>|--plan-key <key>]
       calyx bench partitioned-rrf-plan-remap --from-cf-root <aster-dir> [--from-plan-key <key>] --vault-root <dir> --cf-root <aster-dir> [--association-key <key>|--plan-key <key>] [--base-dir <dir>] [--queries-from-corpus]
       calyx bench partitioned-rrf (--plan <json>|--plan-cf-root <aster-dir> [--plan-key <key>]) [--timeline-cf-root <aster-dir> [--timeline-key <key>]] --n <n> --k <n> --n-probe <n> --region-beam <n> --ground-truth <n> [--truth-depth <n>] [--ensemble-card <ensemble_card.json>] [--a37-admission-cf-root <aster-dir> [--a37-admission-key <key>]|--a37-admission-card <multi_anchor_ensemble_card.json>] [--fused-ground-truth-cf-root <aster-dir> [--fused-ground-truth-key <key>]|--slot-ground-truth-cf-root <aster-dir> [--slot-ground-truth-key <key>]|--fused-ground-truth-file <i32bin> --fused-ground-truth-manifest <json>|--slot-ground-truth-manifest <json>] [--write-fused-ground-truth-cf-root <aster-dir> [--write-fused-ground-truth-key <key>]|--write-fused-ground-truth-file <i32bin> --write-fused-ground-truth-manifest <json>] [--report-cf-root <aster-dir> [--report-key <key>] [--report-db-only|--no-report-stdout]] [--recall-floor <f>] [--out <json>] [--anneal-vault <dir>] [--tuner-slo-us <us>]
       calyx bench partitioned-rrf --recall-floor requires DB plan, timeline, A37 admission, and truth CF roots; JSON/file cards and truth are diagnostic only
       calyx bench partitioned-rrf-slot-truth (--plan <json>|--plan-cf-root <aster-dir> [--plan-key <key>]) (--cf-root <aster-dir> --db-only [--association-key <key>]|--out-dir <dir>) --query-count <n> --truth-depth <n> [--chunk-rows <n>]
       calyx bench partitioned-rrf-timeline --timeline <timeline.jsonl> --cf-root <aster-dir> [--association-key <key>|--timeline-key <key>] [--expected-rows <n>] [--chunk-rows <n>]
       calyx anneal status --vault <dir> --tuner bw_postcutoff
       calyx anneal replay-status --vault <dir>
       calyx anneal head-status --kind <Predictor|Calibrator|FusionWeights> --vault <dir>
       calyx anneal bandit-status --key <shape_key> --vault <dir>
       calyx anneal ab-log --last <n> --vault <dir>
       calyx anneal soak --queries <n> --vault <dir> --corpus-jsonl <jsonl> --metrics-dir <dir> [--sample-interval <n>]
       calyx anneal soak-report --last <n> --vault <dir>
       calyx anneal autotune-report --scope forge --cache <json> --vault <dir> --last <n>
       calyx anneal autotune-report --scope index --slot <n> --cache <json> --vault <dir> --last <n>
       calyx anneal autotune-report --scope storage --cache <json> --vault <dir> --last <n>
       calyx anneal intelligence-report --fixture <json> [--vault <dir>]
       calyx intelligence abundance --vault <dir>
       calyx anneal growth-curve --vault <dir> [--last <n>]
       calyx anneal goodhart-check --fixture <json> --vault <dir> --vault-id <id> --salt <s>
       calyx sextant recall-validate --corpus-jsonl <jsonl> --queries-jsonl <jsonl> --qrels <tsv> --metrics-dir <dir> --vault <dir> [--query-limit <n>] [--min-delta <f>]
       calyx lodestar kernel-validate --corpora-dir <dir> --metrics-dir <dir> [--query-limit <n>] [--top-k <n>] [--min-ratio <f>]
       calyx summarize --vault <dir> --scope <json|@file> --out <json> [--graph <id>] [--as-of <ms>] [--anchor-label <label>] [--max-kernel-size <n>] [--require-grounded]
       calyx media image-validate --samples <jsonl> --metrics-dir <dir> --vault <dir> [--min-image-bits <f>] [--min-cross-modal-bits <f>] [--k <n>]
       calyx media emotion-validate --samples <jsonl> --metrics-dir <dir> --vault <dir> [--min-bits <f>] [--k <n>]
       calyx media video-validate --metadata <jsonl> [--dataset-root <dir>] --metrics-dir <dir> --vault <dir> [--vault-id <id>] [--salt <s>]
       calyx media video-readback --vault <dir> [--vault-id <id>] [--salt <s>]
       calyx anneal deficit-map --anchor <anchor_id> --fixture <json> [--threshold <bits>]
       calyx anneal propose-preview --anchor <anchor_id> --deficit <json> --corpus <json>
       calyx anneal lens-proposal-log --fixture <json> --last <n>
       calyx anneal lens-proposal-log --vault <dir> --last <n>
       calyx anneal propose-lens-run --fixture <json>
       calyx anneal frozen-guard-report --artifact <json>
       calyx anneal regression-report --artifact <json>
       calyx anneal status --faults --last <n> --vault <dir>
       calyx leapable issue612-fsv --baseline-latency <json> --flipped-latency <json> --pg-before <dir> --pg-after <dir> --out <json>
       calyx leapable dual-write --sqlite <db> --calyx <dir>
       calyx leapable read-flip --sqlite <db> --calyx <dir> [--tau <f>] [--skip-backfill]
       calyx leapable remove-shadow --sqlite <db> (--calyx|--vault) <dir> --vault-type <text|code|civic|media>
       calyx leapable production-fsv <snapshot-pg|verify-pg-unchanged|verify-contract|run> [args]
       calyx leapable ask --vault <dir> (--query-vector <json-array> | --query <text>) [--top-k <n>]
       calyx leapable recall-compare --sqlite <db> --calyx <dir> --queries <jsonl> [--top-k <n>]
       calyx leapable verify-round-trip --sqlite <db> --calyx <dir> [--output <json>] [--benchmark --queries <jsonl>] [--top-k <n>]
       calyx leapable shadow-open --sqlite <db> --vault <dir>
       calyx leapable shadow-readback --vault <dir>
       calyx ward tau --slot <n> --vault <dir>
       calyx merkle-root (--ledger <dir> | --vault <dir>) --range <a..b>
       calyx verify-chain --ledger <dir> --range <a..b>
       calyx verify-chain --vault <dir-or-name> --range <a..b>   (fixed-range legacy output/quarantine form)
       calyx verify-restore --vault <dir> [--json]
       calyx scan --cf ledger --vault <dir>
       calyx get-provenance --vault <dir> --cx <cx-id>
       calyx get-answer-trace --vault <dir> --answer <answer-id-or-hex>
       calyx audit --vault <dir> --kind <kind>
       CALYX_LEDGER_DIR=<dir> calyx merkle-root --range <a..b>
       calyx compact --vault <dir> --cf <name>
       calyx compact-watch --vault <dir> --duration <30s|500ms>
       calyx soak --vault <dir> --ops <n> --threads <n>
       calyx tier --vault <dir> --cf <name> --output <hot|cold>
       calyx vault-demo --vault <dir>
       calyx arrow-demo --vault <dir>
       calyx cf-demo --vault <dir>
       calyx mvcc-demo --vault <dir>
       calyx wal-drill --vault <dir> --records <n>
       calyx wal-replay <wal-dir>
       calyx crash-drill --vault <dir> --point <before-wal-fsync|after-wal-before-commit|after-commit-before-manifest> [--pause-ms <n>]
       calyx recover --vault <dir>
       calyx open-check --vault <dir> --index <n>
       calyx corrupt-shard --vault <dir> --cf <name> --byte-offset <n>
       calyx wal-batch-demo --vault <dir> --requests <n>
       calyx navigate neighbors --spec <json> --cx <cx> --slot <n> --k <n> [--out <json>]
       calyx navigate define --spec <json> --cx <cx> --slot <n> --k <n> [--out <json>]
       calyx navigate agree --spec <json> --anchor <cx> --k <n> [--slots <a,b>] [--out <json>]
       calyx navigate disagree --spec <json> --anchor <cx> --k <n> [--slots <a,b>] [--out <json>]
       calyx navigate traverse --spec <json> --anchor <cx> --direction <forward|backward|both> --hops <1-10> [--out <json>]
       calyx navigate skills --spec <json> [--min-cluster-size <n>] [--min-samples <n>] [--max-constellations <n>] [--slots <a,b>] [--allow-single] [--out <json>]
       calyx navigate search-skill --spec <json> --skill <name> --slot <n> --k <n> --vec <a,b> [--text <s>] [--min-cluster-size <n>] [--min-samples <n>] [--out <json>]"
}
