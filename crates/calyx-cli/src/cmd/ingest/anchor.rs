use calyx_core::{AnchorKind, AnchorValue};

use super::parse::parse_bool;
use crate::error::{CliError, CliResult};

pub(crate) fn parse_anchor_kind(value: &str) -> CliResult<AnchorKind> {
    Ok(match value {
        "test-pass" => AnchorKind::TestPass,
        "thumbs-up" | "thumbs-down" => AnchorKind::Thumbs,
        "speaker-match" => AnchorKind::SpeakerMatch,
        "style-hold" => AnchorKind::StyleHold,
        label if label.starts_with("label:") && label.len() > "label:".len() => {
            AnchorKind::Label(label["label:".len()..].to_string())
        }
        other => return Err(CliError::usage(format!("unknown anchor kind {other}"))),
    })
}

pub(super) fn anchor_kind_key(kind: &AnchorKind) -> String {
    match kind {
        AnchorKind::TestPass => "test_pass".to_string(),
        AnchorKind::TieFormed => "tie_formed".to_string(),
        AnchorKind::Thumbs => "thumbs".to_string(),
        AnchorKind::Label(value) => format!("label:{value}"),
        AnchorKind::Reward => "reward".to_string(),
        AnchorKind::SpeakerMatch => "speaker_match".to_string(),
        AnchorKind::StyleHold => "style_hold".to_string(),
        AnchorKind::Recurrence => "recurrence".to_string(),
    }
}

pub(super) fn parse_anchor_value(
    kind: &AnchorKind,
    raw_kind: &str,
    value: &str,
) -> CliResult<AnchorValue> {
    match raw_kind {
        "thumbs-up" => return Ok(AnchorValue::Bool(true)),
        "thumbs-down" => return Ok(AnchorValue::Bool(false)),
        _ => {}
    }
    match kind {
        AnchorKind::TestPass => parse_bool(value, "--value").map(AnchorValue::Bool),
        AnchorKind::Label(_) => Ok(AnchorValue::Enum(value.to_string())),
        AnchorKind::SpeakerMatch | AnchorKind::StyleHold => Ok(parse_general_value(value)),
        AnchorKind::Thumbs
        | AnchorKind::TieFormed
        | AnchorKind::Reward
        | AnchorKind::Recurrence => Ok(parse_general_value(value)),
    }
}

fn parse_general_value(value: &str) -> AnchorValue {
    if let Ok(parsed) = value.parse::<bool>() {
        return AnchorValue::Bool(parsed);
    }
    if let Ok(parsed) = value.parse::<f64>()
        && parsed.is_finite()
    {
        return AnchorValue::Number(parsed);
    }
    AnchorValue::Text(value.to_string())
}
