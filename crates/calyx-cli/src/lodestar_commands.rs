pub(crate) fn run(topic: &str, args: &[String]) -> Result<(), String> {
    match topic {
        "kernel-validate" => super::lodestar_kernel_validation::run(args),
        other => Err(format!("unknown lodestar topic: {other}")),
    }
}
