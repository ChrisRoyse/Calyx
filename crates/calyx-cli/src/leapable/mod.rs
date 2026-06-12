mod issue612_fsv;

pub(crate) fn run(topic: &str, args: &[String]) -> Result<(), String> {
    match topic {
        "issue612-fsv" => issue612_fsv::run(args),
        _ => Err(format!("unknown leapable command: {topic}")),
    }
}
