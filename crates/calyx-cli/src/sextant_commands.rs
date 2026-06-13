pub(crate) fn run(topic: &str, args: &[String]) -> Result<(), String> {
    match topic {
        "recall-validate" => super::sextant_recall_validation::run(args),
        "diskann-validate" => super::sextant_diskann_validation::run(args),
        other => Err(format!("unknown sextant topic: {other}")),
    }
}
