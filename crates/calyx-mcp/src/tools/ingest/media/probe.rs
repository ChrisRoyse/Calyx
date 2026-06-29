use super::*;

pub(super) fn ffprobe_media(source: &Path, modality: Modality) -> ToolResult<MediaProbe> {
    let codec_type = ffprobe_codec_type(modality);
    let mut command = Command::new("ffprobe");
    command.arg("-v").arg("error");
    if modality == Modality::Video {
        command.arg("-count_frames");
    }
    let output = command
        .arg("-show_streams")
        .arg("-show_format")
        .arg("-of")
        .arg("json")
        .arg(source)
        .output()
        .map_err(|error| {
            media_error(
                "CALYX_MEDIA_PROBE_MISSING",
                format!("spawn ffprobe for {}: {error}", source.display()),
            )
        })?;
    if !output.status.success() {
        return Err(media_error(
            "CALYX_MEDIA_DECODE_FAILED",
            format!(
                "ffprobe failed for {}: {}",
                source.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        media_error(
            "CALYX_MEDIA_DECODE_FAILED",
            format!("parse ffprobe JSON for {}: {error}", source.display()),
        )
    })?;
    probe_from_json(&value, modality, codec_type, source)
}

fn probe_from_json(
    value: &Value,
    modality: Modality,
    codec_type: &str,
    source: &Path,
) -> ToolResult<MediaProbe> {
    let stream = value["streams"].as_array().and_then(|streams| {
        streams
            .iter()
            .find(|stream| stream["codec_type"].as_str() == Some(codec_type))
    });
    let Some(stream) = stream else {
        return Err(media_error(
            "CALYX_MEDIA_DECODE_FAILED",
            format!("{} has no {codec_type} stream", source.display()),
        ));
    };
    let container = value["format"]["format_name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let duration = stream["duration"]
        .as_str()
        .or_else(|| value["format"]["duration"].as_str())
        .and_then(|raw| raw.parse::<f64>().ok());
    let mut probe = MediaProbe {
        codec: stream["codec_name"].as_str().unwrap_or("").to_string(),
        container,
        duration_seconds: duration,
        sample_rate_hz: None,
        channels: None,
        width: None,
        height: None,
        frame_count: None,
        fps: None,
    };
    if modality == Modality::Audio {
        probe.sample_rate_hz = stream["sample_rate"]
            .as_str()
            .and_then(|raw| raw.parse::<u32>().ok());
        probe.channels = stream["channels"].as_u64().map(|value| value as u32);
        if probe.sample_rate_hz.unwrap_or(0) == 0 || probe.channels.unwrap_or(0) == 0 {
            return Err(incomplete_decode(source, "audio"));
        }
    } else {
        probe.width = stream["width"].as_u64().map(|value| value as u32);
        probe.height = stream["height"].as_u64().map(|value| value as u32);
        if probe.width.unwrap_or(0) == 0 || probe.height.unwrap_or(0) == 0 {
            return Err(incomplete_decode(source, media_modality_name(modality)));
        }
        if modality == Modality::Image {
            probe.frame_count = Some(1);
        } else {
            probe.frame_count = stream["nb_read_frames"]
                .as_str()
                .or_else(|| stream["nb_frames"].as_str())
                .and_then(|raw| raw.parse::<u64>().ok());
            probe.fps = stream["avg_frame_rate"]
                .as_str()
                .or_else(|| stream["r_frame_rate"].as_str())
                .and_then(parse_fps);
            if probe.frame_count.unwrap_or(0) == 0 || probe.fps.unwrap_or(0.0) <= 0.0 {
                return Err(incomplete_decode(source, "video"));
            }
        }
    }
    Ok(probe)
}

fn parse_fps(raw: &str) -> Option<f64> {
    let Some((left, right)) = raw.split_once('/') else {
        return raw.parse::<f64>().ok();
    };
    let numerator = left.parse::<f64>().ok()?;
    let denominator = right.parse::<f64>().ok()?;
    (denominator != 0.0).then_some(numerator / denominator)
}
