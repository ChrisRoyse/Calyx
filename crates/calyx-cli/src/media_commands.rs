pub(crate) fn run(topic: &str, args: &[String]) -> Result<(), String> {
    match topic {
        "image-validate" => super::media_image_validation::run(args),
        "emotion-validate" => super::media_emotion_validation::run(args),
        other => Err(format!("unknown media topic: {other}")),
    }
}
