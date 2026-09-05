use calyx_core::{CalyxError, Result};

const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
const DIM: usize = super::LEGAL_DOCUMENT_STRUCTURE_DIM as usize;

pub(super) fn features(bytes: &[u8]) -> Result<Vec<f32>> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(CalyxError {
            code: "CALYX_LENS_INPUT_TOO_LARGE",
            message: format!(
                "legal-document structure input is {} bytes; frozen complete-document limit is {} bytes",
                bytes.len(),
                MAX_INPUT_BYTES
            ),
            remediation: "commission a bounded complete-document structure lens with a larger frozen limit and new LensId",
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|error| {
        CalyxError::lens_dim_mismatch(format!(
            "legal-document structure input is not UTF-8: {error}"
        ))
    })?;
    if text.trim().is_empty() {
        return Err(CalyxError {
            code: "CALYX_LENS_EMPTY_INPUT",
            message: "legal-document structure cannot measure empty or whitespace-only input"
                .to_string(),
            remediation: "provide the complete non-empty judicial document bytes",
        });
    }

    let lower = text.to_ascii_lowercase();
    let lines = text.lines().collect::<Vec<_>>();
    let words = text.split_whitespace().collect::<Vec<_>>();
    let nonempty_lines = lines.iter().filter(|line| !line.trim().is_empty()).count();
    let blank_lines = lines.len().saturating_sub(nonempty_lines);
    let paragraphs = paragraph_count(&lines);
    let sentences = bytes
        .iter()
        .filter(|byte| matches!(byte, b'.' | b'?' | b'!'))
        .count()
        .max(1);
    let heading_lines = lines.iter().filter(|line| is_heading(line)).count();
    let numbered_paragraphs = lines
        .iter()
        .filter(|line| is_numbered_paragraph(line))
        .count();
    let footnote_lines = lines.iter().filter(|line| is_footnote(line)).count();

    let citation_markers = occurrence_sum(
        &lower,
        &[
            " ohio st.",
            " ohio app.",
            " ohio misc.",
            " n.e.",
            " u.s.",
            " s.ct.",
            " f.2d",
            " f.3d",
            " fed. appx.",
            " l.ed.",
        ],
    );
    let statutory_markers = occurrence_sum(
        &lower,
        &[
            "r.c.",
            "revised code",
            "u.s.c.",
            "administrative code",
            "civ.r.",
            "crim.r.",
            "app.r.",
        ],
    );
    let docket_markers = occurrence_sum(
        &lower,
        &["case no.", "case nos.", "docket no.", "appeal no."],
    );
    let party_markers = occurrence_sum(&lower, &[" v. ", "\nv. ", "\nin re ", "\nstate ex rel."]);
    let judge_markers = occurrence_sum(
        &lower,
        &[
            "judge",
            "justice",
            "magistrate",
            "j., concurs",
            "j., dissents",
            "jj.",
        ],
    );
    let section_markers = lower.matches('§').count()
        + occurrence_sum(&lower, &[" section ", "\nsection ", "\narticle "]);
    let outcome_markers = occurrence_sum(
        &lower,
        &[
            "affirmed",
            "reversed",
            "remanded",
            "vacated",
            "dismissed",
            "overruled",
            "sustained",
        ],
    );
    let procedural_markers = occurrence_sum(
        &lower,
        &[
            "motion",
            "appeal",
            "summary judgment",
            "trial court",
            "plea",
            "complaint",
            "indictment",
        ],
    );
    let reasoning_markers = occurrence_sum(
        &lower,
        &[
            "because",
            "therefore",
            "however",
            "pursuant to",
            "we conclude",
            "we hold",
            "accordingly",
        ],
    );
    let introductory_markers = occurrence_sum(
        &lower,
        &["introduction", "background", "statement of facts", "facts"],
    );
    let decision_markers = occurrence_sum(
        &lower,
        &["decision", "conclusion", "disposition", "judgment"],
    );
    let separate_opinion_markers = occurrence_sum(
        &lower,
        &["dissent", "concurring", "concurrence", "separate opinion"],
    );

    let byte_den = bytes.len() as f32;
    let line_den = lines.len().max(1) as f32;
    let alpha_count = bytes
        .iter()
        .filter(|byte| byte.is_ascii_alphabetic())
        .count();
    let alpha_den = alpha_count.max(1) as f32;
    let punctuation_count = bytes
        .iter()
        .filter(|byte| byte.is_ascii_punctuation())
        .count();
    let quote_count = bytes
        .iter()
        .filter(|byte| matches!(byte, b'\'' | b'"'))
        .count();
    let parenthesis_count = bytes
        .iter()
        .filter(|byte| matches!(byte, b'(' | b')' | b'[' | b']'))
        .count();

    let output = vec![
        log_scaled(bytes.len(), 32.0),
        log_scaled(text.chars().count(), 32.0),
        log_scaled(words.len(), 24.0),
        log_scaled(lines.len(), 16.0),
        log_scaled(paragraphs, 16.0),
        log_scaled(sentences, 16.0),
        blank_lines as f32 / line_den,
        heading_lines as f32 / line_den,
        numbered_paragraphs as f32 / line_den,
        footnote_lines as f32 / line_den,
        log_value(words.len() as f32 / sentences as f32) / 12.0,
        log_value(words.len() as f32 / paragraphs.max(1) as f32) / 16.0,
        log_scaled(citation_markers, 12.0),
        log_scaled(statutory_markers, 12.0),
        log_scaled(docket_markers, 8.0),
        log_scaled(party_markers, 8.0),
        log_scaled(judge_markers, 8.0),
        log_scaled(section_markers, 8.0),
        bytes.iter().filter(|byte| byte.is_ascii_digit()).count() as f32 / byte_den,
        bytes
            .iter()
            .filter(|byte| byte.is_ascii_uppercase())
            .count() as f32
            / alpha_den,
        punctuation_count as f32 / byte_den,
        quote_count as f32 / byte_den,
        parenthesis_count as f32 / byte_den,
        bytes.iter().filter(|byte| **byte == b';').count() as f32 / byte_den,
        bytes.iter().filter(|byte| **byte == b':').count() as f32 / byte_den,
        bytes
            .iter()
            .filter(|byte| matches!(byte, b'\n' | b'\r'))
            .count() as f32
            / byte_den,
        log_scaled(outcome_markers, 12.0),
        log_scaled(procedural_markers, 12.0),
        log_scaled(reasoning_markers, 12.0),
        log_scaled(introductory_markers, 8.0),
        log_scaled(decision_markers, 8.0),
        log_scaled(separate_opinion_markers, 8.0),
    ];
    if output.len() != DIM || output.iter().any(|value| !value.is_finite()) {
        return Err(CalyxError::lens_numerical_invariant(format!(
            "legal-document structure produced {} values with finite={}",
            output.len(),
            output.iter().all(|value| value.is_finite())
        )));
    }
    Ok(output)
}

fn paragraph_count(lines: &[&str]) -> usize {
    let mut count = 0;
    let mut inside = false;
    for line in lines {
        if line.trim().is_empty() {
            inside = false;
        } else if !inside {
            count += 1;
            inside = true;
        }
    }
    count.max(1)
}

fn is_heading(line: &&str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.len() > 120 {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if [
        "introduction",
        "background",
        "statement of facts",
        "facts",
        "analysis",
        "law and analysis",
        "decision",
        "conclusion",
        "disposition",
        "judgment",
    ]
    .contains(&lower.as_str())
    {
        return true;
    }
    let alpha = trimmed
        .bytes()
        .filter(|byte| byte.is_ascii_alphabetic())
        .count();
    let uppercase = trimmed
        .bytes()
        .filter(|byte| byte.is_ascii_uppercase())
        .count();
    alpha >= 3 && uppercase * 4 >= alpha * 3
}

fn is_numbered_paragraph(line: &&str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('¶') || trimmed.starts_with("{¶") {
        return true;
    }
    let prefix = trimmed.split_whitespace().next().unwrap_or_default();
    let marker = prefix.trim_matches(|ch: char| matches!(ch, '(' | ')' | '.' | '[' | ']'));
    !marker.is_empty()
        && marker.len() <= 8
        && (marker.chars().all(|ch| ch.is_ascii_digit())
            || marker
                .chars()
                .all(|ch| matches!(ch.to_ascii_lowercase(), 'i' | 'v' | 'x')))
        && (prefix.ends_with('.') || prefix.ends_with(')') || prefix.starts_with('('))
}

fn is_footnote(line: &&str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('[') else {
        return false;
    };
    rest.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

fn occurrence_sum(text: &str, needles: &[&str]) -> usize {
    needles
        .iter()
        .map(|needle| text.matches(needle).count())
        .sum()
}

fn log_scaled(value: usize, denominator: f32) -> f32 {
    log_value(value as f32) / denominator
}

fn log_value(value: f32) -> f32 {
    value.max(0.0).ln_1p() / std::f32::consts::LN_2
}
