pub(crate) fn print_usage() {
    println!("{}", usage());
    println!("prints source-of-truth bytes or listings for manual FSV inspection");
    println!("merkle-root --vault reads Aster cf/ledger plus wal; no side ledger dir is created");
}

pub(crate) fn usage() -> &'static str {
    "usage: calyx readback (--hex <file> | --vault-tree <dir> | --cf-row <vault> --cf <cf-name> --key <hex-key> | --wal <segment-path> | --ledger <vault> --seq <n> | --vault <dir> --verify-against <sqlite.db> | --vault <dir> --show-manifest | vault-manifest --field <name> --vault <dir> | temporal_search --explain --clock-fixed <secs> --tz-offset <secs> | dedup-check --vault <dir> --cx-id <cx> --slot <n> --tau <f> --near-cos <f> --distinct-cos <f> --vault-id <id> --salt <s> | kernel-health --root <dir> --kernel-id <cx> | recurrence-series --vault <dir> --cx-id <cx> | periodic-recall --vault <dir> (--hour <0-23> | --day <0-6>) [--hour <0-23>] [--day <0-6>] | oracle_self_consistency --vault <dir> --domain <domain> --vault-id <id> --salt <s> | oracle_sufficiency --vault <dir> --fixture <json> --vault-id <id> --salt <s> | oracle_predict --vault <dir> --fixture <json> --vault-id <id> --salt <s> | oracle_expand --vault <dir> --fixture <json> --vault-id <id> --salt <s> [--depth <0-4>] | reverse_query --vault <dir> --domain <domain> --answer <text> --fixture <json> --vault-id <id> --salt <s> | super_intelligence --vault <dir> --domain <domain> --fixture <json> --vault-id <id> --salt <s> | temporal-log-recurrence --log <csv> --vault <dir> --out <json> --rows <n> --expected-cadence-secs <secs> --confidence-ceiling <f> | time-prediction --vault <dir> --cx-id <cx> --confidence-ceiling <f> | assay-report|temporal-cross-term|kernel-weights|kernel-window|ward-novelty|compression-ratio|compression-report|anneal-schedule --artifact <json> [--field <path>] | config <tripwire|budget> --vault <dir> | ledger --kind Anneal --action <GoodhartPassed|GoodhartFailed> --last <n> --vault <dir> | anneal mistakes --vault <dir> --last <n> | dedup-audit --vault <dir> --cx-id <cx> | dedup-undo --vault <dir> --token <json> | cx-list --vault <dir> | time-index --vault <dir> | as-of --vault <dir> --t-millis <ms> | --cf <name> --vault <dir> [--seq <n>] | --cf <name> --level <dir> | --wal --vault <dir>)
       calyx resource-status --vault <dir> [--metrics]
       calyx create-vault <name> [--panel-template <text-default|code-default|civic-default|legal-default|medical-default|bio-default|media-default>]
       calyx add-lens <vault> --name <n> --runtime <algorithmic|tei-http|external-cmd|candle-local|onnx|multimodal-adapter> [--endpoint <url-or-runtime-id>] [--weights <path>] [--shape Dense(<dim>)|Sparse(<dim>)|Multi(<token_dim>)] [--modality <text|code|image|audio|video|structured|mixed>]
       calyx retire-lens <vault> --slot <u16>
       calyx park-lens <vault> --slot <u16>
       calyx list-panel <vault>
       calyx profile-lens [--name <n>] [--runtime <r>] [--endpoint <url-or-runtime-id>] [--weights <path>] [--shape Dense(<dim>)|Sparse(<dim>)] [--modality <m>] [--probe <path>]
       calyx ingest <vault> (--text <s> | --batch <jsonl-path> | --file <path> --modality <audio|video>) [--idempotent]
       calyx anchor <vault> <cx_id> --kind <test-pass|thumbs-up|thumbs-down|label:<s>|speaker-match|style-hold> --value <v> [--confidence <0..1>] [--source <s>]
       calyx measure <vault> --text <s>
       calyx search <vault> <query> [--k <n>] [--fusion <rrf|weighted-rrf|single-lens|kernel-first|pipeline>] [--guard <off|in-region>] [--explain] [--provenance|--no-provenance] [--fresh|--stale-ok] [--filter <json-predicate>]
       calyx kernel-answer <vault> <query> [--anchor <kind>] [--explain]
       calyx bits <vault> <anchor-kind> [--explain]
       calyx kernel <vault> [--anchor <kind>] [--rebuild]
       calyx guard <vault> <calibrate|check|generate> [args]
       calyx abundance <vault>
       calyx propose-lens <vault> --anchor <kind>
       calyx provenance <vault> <cx_id>
       calyx verify-chain <vault> [--from <seq>] [--to <seq>]
       calyx reproduce <vault> <answer_id>
       calyx anneal-status <vault>
       calyx healthcheck [--vault <vault>] [--json|--no-json] [--tei <http://host:port[/path]>]
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
       calyx panel a38-bundle save --name <name> --base-template <name-or-id> --required-modality <m> --include-lens <name-or-id> --evidence <json> [--home <dir>] [--budget-vram-mib <n>]
       calyx panel a38-bundle list [--home <dir>]
       calyx assay corpus-build --rows-jsonl <rows.jsonl> --out-dir <dir> --dataset <name> --target-class <n> --manifest <manifest.json> [--manifest <manifest.json> ...] [--limit-per-class <n>] [--batch-size <n>] [--cost-override-json <json>]   (rows require exactly one of text or input_path)
       calyx assay ensemble-card --corpus-dir <dir> --metrics-dir <dir> [--cf-root <dir>] [--target-class <n>] [--domain <domain>] [--min-lenses <n>] [--min-marginal-bits <f>] [--max-redundancy <f>]
       calyx assay i8bin-ensemble-card --plan <partitioned_rrf_plan.json> --rows-jsonl <rows.jsonl> --metrics-dir <dir> [--stream-report <stream_fbin_report.json>] [--cf-root <dir>] [--target-class <n>] [--domain <domain>] [--sample-rows <n>] [--signature-rows <n|all>] [--min-lenses <n>] [--min-marginal-bits <f>] [--max-redundancy <f>] [--mode <gate|diagnostic>|--diagnostic|--baseline]
       calyx assay multi-anchor-ensemble-card --report <a37_i8bin_ensemble_report.json> --report <a37_i8bin_ensemble_report.json> --out-dir <dir> [--min-lenses <n>] [--min-marginal-bits <f>] [--max-redundancy <f>] [--mode <gate|diagnostic>|--diagnostic|--baseline]
       calyx assay gdelt-rows --source-dir <dir> --out <rows.jsonl> --manifest <manifest.json> [--dataset <name>] [--limit-per-class <n>|--max-rows <n>] [--actor-country <ISO3>] [--action-country <ISO2>] [--action-name-contains <text>]
       calyx assay export-fbin --corpus-dir <dir> --out-dir <dir> --bits-report <assay_abundance.json> --query-count <n> [--min-bits <f>]
       calyx assay stream-fbin --rows-jsonl <rows.jsonl> --out-dir <dir> --dataset <name> --target-class <n> --bits-report <assay_abundance.json> --query-count <n> --manifest <manifest.json> [--manifest <manifest.json> ...] [--limit-per-class <n>] [--batch-size <n>] [--cost-override-json <json>] [--min-bits <f>] [--vector-format <fbin|i8bin>] [--mode <gate|diagnostic>|--diagnostic|--baseline]
       calyx fsv corpus-readback --root <dir>
       calyx anneal status --health --vault <dir>
       calyx build-bench-vault --vault <dir> --n-cx <n> --dim <n> --slots <n> --seed <n>
       calyx build-partitioned-vault --vault <dir> (--vectors <file.fbin|file.i8bin>|--n-cx <n> --dim <n>) --regions <n> [--distance-metric <unit-l2|raw-l2>] [--sample <n>] [--chunk <n>] [--m-max <n>] [--ef <n>] [--region-build-parallelism <n>] [--final-assignment-probe <n>] [--final-assignment-cap <n>] [--build-backend <cpu-vamana|cuvs-cagra>] [--progress-file <json>]
       calyx bench search --vault <dir> --strategy KernelFirst --n <n> --report p50,p99,p999 --seed <n> [--k <n>] [--beamwidth <n>] [--posting-cutoff <n>] [--tuner-slo-us <us>]
       calyx bench recall --vault <dir> --n <n> --k <n> [--seed <n>]
       calyx bench partitioned-search --vault <dir> --n <n> --k <n> --n-probe <n> --region-beam <n> [--ground-truth <n> --recall-floor <f>] [--anneal-vault <dir> --tuner-slo-us <us>]
       calyx bench partitioned-search --vault <dir> --queries <file.fbin|file.i8bin> [--corpus <file.fbin|file.i8bin>|--ground-truth-file <file.i32bin> [--ground-truth-id-map <file.i32bin>]] --n <n> --k <n> --n-probe <n> --region-beam <n> --ground-truth <n> --recall-floor <f>
       calyx bench partitioned-rrf --plan <json> --n <n> --k <n> --n-probe <n> --region-beam <n> --ground-truth <n> [--truth-depth <n>] [--ensemble-card <ensemble_card.json>] [--fused-ground-truth-file <i32bin> --fused-ground-truth-manifest <json>|--slot-ground-truth-manifest <json>] [--write-fused-ground-truth-file <i32bin> --write-fused-ground-truth-manifest <json>] [--recall-floor <f>] [--out <json>] [--anneal-vault <dir>] [--tuner-slo-us <us>]
       calyx bench partitioned-rrf-slot-truth --plan <json> --out-dir <dir> --query-count <n> --truth-depth <n> [--chunk-rows <n>]
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
       calyx verify-chain (--ledger <dir> | --vault <dir>) --range <a..b>
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
