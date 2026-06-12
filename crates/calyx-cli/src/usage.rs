pub(crate) fn print_usage() {
    println!("{}", usage());
    println!("prints source-of-truth bytes or listings for manual FSV inspection");
    println!("merkle-root --vault reads Aster cf/ledger plus wal; no side ledger dir is created");
}

pub(crate) fn usage() -> &'static str {
    "usage: calyx readback (--hex <file> | --vault-tree <dir> | vault-manifest --field <name> --vault <dir> | temporal_search --explain --clock-fixed <secs> --tz-offset <secs> | dedup-check --vault <dir> --cx-id <cx> --slot <n> --tau <f> --near-cos <f> --distinct-cos <f> --vault-id <id> --salt <s> | kernel-health --root <dir> --kernel-id <cx> | recurrence-series --vault <dir> --cx-id <cx> | periodic-recall --vault <dir> (--hour <0-23> | --day <0-6>) [--hour <0-23>] [--day <0-6>] | oracle_self_consistency --vault <dir> --domain <domain> --vault-id <id> --salt <s> | oracle_sufficiency --vault <dir> --fixture <json> --vault-id <id> --salt <s> | oracle_predict --vault <dir> --fixture <json> --vault-id <id> --salt <s> | oracle_expand --vault <dir> --fixture <json> --vault-id <id> --salt <s> [--depth <0-4>] | reverse_query --vault <dir> --domain <domain> --answer <text> --fixture <json> --vault-id <id> --salt <s> | super_intelligence --vault <dir> --domain <domain> --fixture <json> --vault-id <id> --salt <s> | temporal-log-recurrence --log <csv> --vault <dir> --out <json> --rows <n> --expected-cadence-secs <secs> --confidence-ceiling <f> | time-prediction --vault <dir> --cx-id <cx> --confidence-ceiling <f> | assay-report|temporal-cross-term|kernel-weights|kernel-window|ward-novelty|compression-ratio|compression-report|anneal-schedule --artifact <json> [--field <path>] | config <tripwire|budget> --vault <dir> | ledger --kind Anneal --action <GoodhartPassed|GoodhartFailed> --last <n> --vault <dir> | anneal mistakes --vault <dir> --last <n> | dedup-audit --vault <dir> --cx-id <cx> | dedup-undo --vault <dir> --token <json> | cx-list --vault <dir> | --cf <name> --vault <dir> [--seq <n>] | --cf <name> --level <dir> | --wal --vault <dir>)
       calyx resource-status --vault <dir> [--metrics]
       calyx resource-drill --vault <dir> --ops <n> --value-bytes <n> --memtable-cap <bytes> --pin-max-age-ms <ms>
       calyx anneal status --health --vault <dir>
       calyx anneal replay-status --vault <dir>
       calyx anneal head-status --kind <Predictor|Calibrator|FusionWeights> --vault <dir>
       calyx anneal bandit-status --key <shape_key> --vault <dir>
       calyx anneal ab-log --last <n> --vault <dir>
       calyx anneal soak-report --last <n> --vault <dir>
       calyx anneal autotune-report --scope forge --cache <json> --vault <dir> --last <n>
       calyx anneal autotune-report --scope index --slot <n> --cache <json> --vault <dir> --last <n>
       calyx anneal intelligence-report --fixture <json> [--vault <dir>]
       calyx anneal growth-curve --vault <dir> [--last <n>]
       calyx anneal goodhart-check --fixture <json> --vault <dir> --vault-id <id> --salt <s>
       calyx anneal deficit-map --anchor <anchor_id> --fixture <json> [--threshold <bits>]
       calyx anneal propose-preview --anchor <anchor_id> --deficit <json> --corpus <json>
       calyx anneal lens-proposal-log --fixture <json> --last <n>
       calyx anneal lens-proposal-log --vault <dir> --last <n>
       calyx anneal propose-lens-run --fixture <json>
       calyx anneal frozen-guard-report --artifact <json>
       calyx anneal regression-report --artifact <json>
       calyx anneal status --faults --last <n> --vault <dir>
       calyx leapable issue612-fsv --baseline-latency <json> --flipped-latency <json> --pg-before <dir> --pg-after <dir> --out <json>
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
