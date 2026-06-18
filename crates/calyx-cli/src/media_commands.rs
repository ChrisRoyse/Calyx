pub(crate) fn run(topic: &str, args: &[String]) -> crate::error::CliResult {
    match topic {
        "image-validate" => super::media_image_validation::run(args),
        "emotion-validate" => super::media_emotion_validation::run(args),
        "video-validate" | "video-readback" => super::media_video_validation::run(topic, args),
        other => Err(format!("unknown media topic: {other}").into()),
    }
}
