mod data;
mod engine;
mod request;

use std::fs;

use data::VideoMetadata;
use engine::{readback_video_vault, validate_video_corpus};
use request::{VideoCommand, VideoRequest};

pub(crate) fn run(topic: &str, args: &[String]) -> crate::error::CliResult {
    match VideoRequest::parse(topic, args)? {
        VideoCommand::Validate(request) => {
            fs::create_dir_all(&request.metrics_dir)?;
            let rows = VideoMetadata::load(&request.metadata)?;
            let evidence = validate_video_corpus(&request, &rows)?;
            println!("{}", serde_json::to_string_pretty(&evidence)?);
            Ok(())
        }
        VideoCommand::Readback(request) => {
            let evidence = readback_video_vault(&request)?;
            println!("{}", serde_json::to_string_pretty(&evidence)?);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
