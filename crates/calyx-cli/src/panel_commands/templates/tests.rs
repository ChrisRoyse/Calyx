use super::*;

#[test]
fn parses_repeated_lenses() {
    let flags = Flags::parse(&[
        "--name".to_string(),
        "text-deep".to_string(),
        "--lens".to_string(),
        "a".to_string(),
        "--lens".to_string(),
        "b".to_string(),
    ])
    .unwrap();

    assert_eq!(flags.name.as_deref(), Some("text-deep"));
    assert_eq!(flags.lenses, ["a", "b"]);
}

#[test]
fn parses_a37_template_flags() {
    let flags = Flags::parse(&[
        "--template".to_string(),
        "text-deep".to_string(),
        "--assay-card".to_string(),
        "ensemble_card.json".to_string(),
        "--require-a37-gate".to_string(),
        "--a37-admission-card".to_string(),
        "multi_anchor_card.json".to_string(),
    ])
    .unwrap();

    assert_eq!(flags.template.as_deref(), Some("text-deep"));
    assert_eq!(
        flags.assay_card.as_deref(),
        Some(Path::new("ensemble_card.json"))
    );
    assert_eq!(
        flags.a37_admission_card.as_deref(),
        Some(Path::new("multi_anchor_card.json"))
    );
    assert!(flags.require_a37_gate);
}

#[test]
fn modality_parser_matches_catalog_strings() {
    assert_eq!(modality_name(parse_modality("text").unwrap()), "text");
    assert!(parse_modality("temporal").is_err());
}
