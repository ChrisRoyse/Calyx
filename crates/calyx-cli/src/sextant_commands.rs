pub(crate) fn run(topic: &str, args: &[String]) -> crate::error::CliResult {
    match topic {
        "build-bench-vault" => super::sextant_bench::run_build(args),
        "bench-search" => super::sextant_bench::run_bench("search", args),
        "bench-recall" => super::sextant_bench::run_bench("recall", args),
        "recall-validate" => super::sextant_recall_validation::run(args),
        "diskann-validate" => super::sextant_diskann_validation::run(args),
        other => Err(format!("unknown sextant topic: {other}").into()),
    }
}
