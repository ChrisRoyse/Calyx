pub(crate) fn print_usage() {
    println!("{}", usage());
    println!("prints source-of-truth bytes or listings for manual FSV inspection");
    println!("merkle-root --vault reads Aster cf/ledger plus wal; no side ledger dir is created");
}

pub(crate) fn usage() -> &'static str {
    "usage: calyx readback (--hex <file> | --vault-tree <dir> | vault-manifest --field <name> --vault <dir> | temporal_search --explain --clock-fixed <secs> --tz-offset <secs> | dedup-check --vault <dir> --cx-id <cx> --slot <n> --tau <f> --near-cos <f> --distinct-cos <f> --vault-id <id> --salt <s> | recurrence-series --vault <dir> --cx-id <cx> | periodic-recall --vault <dir> (--hour <0-23> | --day <0-6>) [--hour <0-23>] [--day <0-6>] | time-prediction --vault <dir> --cx-id <cx> --confidence-ceiling <f> | assay-report|temporal-cross-term|kernel-weights|kernel-window|ward-novelty|compression-ratio|anneal-schedule --artifact <json> [--field <path>] | config <tripwire|budget> --vault <dir> | anneal mistakes --vault <dir> --last <n> | dedup-audit --vault <dir> --cx-id <cx> | dedup-undo --vault <dir> --token <json> | cx-list --vault <dir> | --cf <name> --vault <dir> [--seq <n>] | --cf <name> --level <dir> | --wal --vault <dir>)
       calyx anneal status --health --vault <dir>
       calyx anneal replay-status --vault <dir>
       calyx anneal head-status --kind <Predictor|Calibrator|FusionWeights> --vault <dir>
       calyx anneal bandit-status --key <shape_key> --vault <dir>
       calyx anneal ab-log --last <n> --vault <dir>
       calyx anneal soak-report --last <n> --vault <dir>
       calyx anneal autotune-report --scope forge --cache <json> --vault <dir> --last <n>
       calyx anneal autotune-report --scope index --slot <n> --cache <json> --vault <dir> --last <n>
       calyx anneal deficit-map --anchor <anchor_id> --fixture <json> [--threshold <bits>]
       calyx anneal propose-preview --anchor <anchor_id> --deficit <json> --corpus <json>
       calyx anneal frozen-guard-report --artifact <json>
       calyx anneal regression-report --artifact <json>
       calyx anneal status --faults --last <n> --vault <dir>
       calyx ward tau --slot <n> --vault <dir>
       calyx merkle-root (--ledger <dir> | --vault <dir>) --range <a..b>
       calyx verify-chain (--ledger <dir> | --vault <dir>) --range <a..b>
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
       calyx wal-batch-demo --vault <dir> --requests <n>"
}
