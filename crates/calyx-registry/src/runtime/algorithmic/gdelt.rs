use std::collections::BTreeMap;

use calyx_core::{Result, SlotVector, SparseEntry, content_address};

use super::hash_part;

pub(super) fn cameo_features(bytes: &[u8]) -> Vec<f32> {
    let text = String::from_utf8_lossy(bytes);
    let event_code = numeric_after(&text, "EventCode");
    let root = numeric_after(&text, "root");
    let quad = numeric_after(&text, "quad");
    let goldstein = numeric_after(&text, "Goldstein");
    let tone = numeric_after(&text, "tone");
    let mut out = vec![0.0_f32; 16];
    out[0] = event_code.is_some() as u8 as f32;
    out[1] = event_code.unwrap_or(0.0).min(999.0) / 999.0;
    out[2] = root.unwrap_or(0.0).min(20.0) / 20.0;
    out[3] = quad.unwrap_or(0.0).min(4.0) / 4.0;
    out[4] = (goldstein.unwrap_or(0.0) / 10.0).clamp(-1.0, 1.0);
    out[5] = (tone.unwrap_or(0.0) / 100.0).clamp(-1.0, 1.0);
    if let Some(quad) = quad.and_then(|value| usize::try_from(value as i64).ok())
        && (1..=4).contains(&quad)
    {
        out[5 + quad] = 1.0;
    }
    let root = root.unwrap_or(0.0);
    out[10] = (root > 0.0 && root < 10.0) as u8 as f32;
    out[11] = (root >= 13.0) as u8 as f32;
    out[12] = text.contains("Actor1 ") as u8 as f32;
    out[13] = text.contains("Actor2 ") as u8 as f32;
    out[14] = text.contains("ActionGeo ") as u8 as f32;
    out[15] = hash_part(u32::from_be_bytes(
        content_address([bytes])[..4]
            .try_into()
            .expect("content hash has bytes"),
    ));
    out
}

pub(super) fn actor_geo(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    add_labeled_token(&mut counts, dim, "actor1", token_after(&text, "Actor1 "));
    add_labeled_token(&mut counts, dim, "actor2", token_after(&text, "Actor2 "));
    for country in text.split(" country ").skip(1).filter_map(first_token) {
        add_term(&mut counts, dim, &format!("country:{country}"), 1.0);
    }
    if let Some((geo, _)) = text
        .split_once("ActionGeo ")
        .and_then(|(_, tail)| tail.split_once(" | SourceURL"))
    {
        for token in geo
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|token| token.len() >= 2)
            .take(16)
        {
            add_term(&mut counts, dim, &format!("geo:{token}"), 1.0);
        }
    }
    sparse_vector(dim, counts)
}

pub(super) fn source_domain(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    if let Some(host) = source_host(&text) {
        add_domain_terms(&mut counts, dim, &host, 2.0);
    }
    if let Some(path) = source_path(&text) {
        for token in split_ascii_terms(path).take(12) {
            add_term(&mut counts, dim, &format!("source_path:{token}"), 1.0);
        }
    }
    sparse_vector(dim, counts)
}

pub(super) fn event_geo(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    let event = event_parts(&text);
    add_event_terms(&mut counts, dim, &event, 2.0);
    for country in country_tokens(&text) {
        add_term(&mut counts, dim, &format!("country:{country}"), 1.0);
        add_event_combo(&mut counts, dim, &event, "country", &country);
    }
    for geo in geo_tokens(&text) {
        add_term(&mut counts, dim, &format!("geo:{geo}"), 0.75);
        add_event_combo(&mut counts, dim, &event, "geo", &geo);
    }
    sparse_vector(dim, counts)
}

pub(super) fn actor_pair(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    let actor1 = clean_token_after(&text, "Actor1 ");
    let actor2 = clean_token_after(&text, "Actor2 ");
    add_labeled_clean(&mut counts, dim, "actor1", actor1.as_deref(), 1.5);
    add_labeled_clean(&mut counts, dim, "actor2", actor2.as_deref(), 1.5);
    if let (Some(left), Some(right)) = (actor1.as_deref(), actor2.as_deref()) {
        add_term(
            &mut counts,
            dim,
            &format!("actor_pair:{left}->{right}"),
            3.0,
        );
        let mut ordered = [left, right];
        ordered.sort_unstable();
        add_term(
            &mut counts,
            dim,
            &format!("actor_pair_unordered:{}:{}", ordered[0], ordered[1]),
            1.0,
        );
    }
    sparse_vector(dim, counts)
}

pub(super) fn event_actor(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    let event = event_parts(&text);
    add_event_terms(&mut counts, dim, &event, 1.0);
    for (label, actor) in [
        ("actor1", clean_token_after(&text, "Actor1 ")),
        ("actor2", clean_token_after(&text, "Actor2 ")),
    ] {
        if let Some(actor) = actor {
            add_term(&mut counts, dim, &format!("{label}:{actor}"), 1.0);
            add_event_combo(&mut counts, dim, &event, label, &actor);
        }
    }
    sparse_vector(dim, counts)
}

pub(super) fn tone_signal(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    let event = event_parts(&text);
    for (label, value, min, max, width) in [
        (
            "goldstein",
            numeric_after(&text, "Goldstein"),
            -10.0,
            10.0,
            2.0,
        ),
        ("tone", numeric_after(&text, "tone"), -100.0, 100.0, 10.0),
    ] {
        if let Some(value) = value {
            let bucket = bucket(value, min, max, width);
            add_term(&mut counts, dim, &format!("{label}_bucket:{bucket}"), 2.0);
            add_term(
                &mut counts,
                dim,
                &format!("{label}_sign:{}", sign(value)),
                1.0,
            );
            add_event_combo(&mut counts, dim, &event, label, &bucket.to_string());
        }
    }
    sparse_vector(dim, counts)
}

pub(super) fn source_event(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    let event = event_parts(&text);
    add_event_terms(&mut counts, dim, &event, 1.0);
    if let Some(host) = source_host(&text) {
        add_domain_terms(&mut counts, dim, &host, 1.0);
        add_event_combo(&mut counts, dim, &event, "source", &host);
    }
    sparse_vector(dim, counts)
}

fn sparse_vector(dim: u32, counts: BTreeMap<u32, f32>) -> Result<SlotVector> {
    let total = counts.values().sum::<f32>().max(1.0);
    Ok(SlotVector::Sparse {
        dim,
        entries: counts
            .into_iter()
            .map(|(idx, val)| SparseEntry {
                idx,
                val: val / total,
            })
            .collect(),
    })
}

#[derive(Default)]
struct EventParts {
    code: Option<String>,
    root: Option<String>,
    quad: Option<String>,
}

fn event_parts(text: &str) -> EventParts {
    EventParts {
        code: clean_token_after(text, "EventCode "),
        root: clean_token_after(text, "root "),
        quad: clean_token_after(text, "quad "),
    }
}

fn numeric_after(text: &str, label: &str) -> Option<f32> {
    text.split_once(label)?
        .1
        .split_whitespace()
        .next()?
        .trim_matches(|ch: char| !ch.is_ascii_digit() && ch != '-' && ch != '.')
        .parse::<f32>()
        .ok()
}

fn token_after<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    text.split_once(label)?.1.split_whitespace().next()
}

fn first_token(value: &str) -> Option<&str> {
    value.split_whitespace().next()
}

fn clean_token_after(text: &str, label: &str) -> Option<String> {
    clean_token(token_after(text, label)?)
}

fn clean_token(value: &str) -> Option<String> {
    let token = value
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .to_ascii_uppercase();
    (!token.is_empty()).then_some(token)
}

fn source_host(text: &str) -> Option<String> {
    let raw = token_after(text, "SourceURL ")?;
    let without_scheme = raw.split_once("://").map_or(raw, |(_, tail)| tail);
    let authority = without_scheme
        .split(['/', '?', '#'])
        .next()?
        .rsplit('@')
        .next()?;
    let host = authority
        .split(':')
        .next()?
        .trim_matches('.')
        .trim_start_matches("www.");
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn source_path(text: &str) -> Option<&str> {
    let raw = token_after(text, "SourceURL ")?;
    let without_scheme = raw.split_once("://").map_or(raw, |(_, tail)| tail);
    without_scheme.split_once('/').map(|(_, path)| path)
}

fn split_ascii_terms(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 2)
        .map(str::to_ascii_lowercase)
}

fn country_tokens(text: &str) -> Vec<String> {
    text.split(" country ")
        .skip(1)
        .filter_map(first_token)
        .filter_map(clean_token)
        .collect()
}

fn geo_tokens(text: &str) -> Vec<String> {
    text.split_once("ActionGeo ")
        .and_then(|(_, tail)| tail.split_once(" | SourceURL"))
        .map(|(geo, _)| split_ascii_terms(geo).take(16).collect())
        .unwrap_or_default()
}

fn add_labeled_token(counts: &mut BTreeMap<u32, f32>, dim: u32, label: &str, token: Option<&str>) {
    if let Some(token) = token {
        add_term(counts, dim, &format!("{label}:{token}"), 2.0);
    }
}

fn add_labeled_clean(
    counts: &mut BTreeMap<u32, f32>,
    dim: u32,
    label: &str,
    token: Option<&str>,
    weight: f32,
) {
    if let Some(token) = token {
        add_term(counts, dim, &format!("{label}:{token}"), weight);
    }
}

fn add_event_terms(counts: &mut BTreeMap<u32, f32>, dim: u32, event: &EventParts, weight: f32) {
    add_labeled_clean(counts, dim, "event_code", event.code.as_deref(), weight);
    add_labeled_clean(counts, dim, "event_root", event.root.as_deref(), weight);
    add_labeled_clean(counts, dim, "event_quad", event.quad.as_deref(), weight);
}

fn add_event_combo(
    counts: &mut BTreeMap<u32, f32>,
    dim: u32,
    event: &EventParts,
    label: &str,
    token: &str,
) {
    for (event_label, value) in [
        ("code", event.code.as_deref()),
        ("root", event.root.as_deref()),
        ("quad", event.quad.as_deref()),
    ] {
        if let Some(value) = value {
            add_term(
                counts,
                dim,
                &format!("event_{event_label}_{label}:{value}:{token}"),
                2.0,
            );
        }
    }
}

fn add_domain_terms(counts: &mut BTreeMap<u32, f32>, dim: u32, host: &str, weight: f32) {
    add_term(counts, dim, &format!("source_host:{host}"), weight);
    let parts = host
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if let Some(tld) = parts.last() {
        add_term(counts, dim, &format!("source_tld:{tld}"), 0.75);
    }
    if parts.len() >= 2 {
        add_term(
            counts,
            dim,
            &format!(
                "source_domain:{}.{}",
                parts[parts.len() - 2],
                parts[parts.len() - 1]
            ),
            1.5,
        );
    }
}

fn bucket(value: f32, min: f32, max: f32, width: f32) -> i32 {
    ((value.clamp(min, max) - min) / width).floor() as i32
}

fn sign(value: f32) -> &'static str {
    if value < 0.0 {
        "neg"
    } else if value > 0.0 {
        "pos"
    } else {
        "zero"
    }
}

fn add_term(counts: &mut BTreeMap<u32, f32>, dim: u32, term: &str, weight: f32) {
    let digest = content_address([term.as_bytes()]);
    let hash = u32::from_be_bytes(digest[..4].try_into().expect("content hash has bytes"));
    *counts.entry(hash % dim).or_default() += weight;
}

#[cfg(test)]
mod tests {
    use super::*;

    const GDELT_ROW: &[u8] = b"EventCode 031 root 03 quad 1 | Goldstein 5.2 tone -1.25 | Actor1 USAGOV Actor2 PAL | ActionGeo Gaza Gaza Strip country IS | SourceURL https://example.test/gdelt";
    type SparseLensFn = fn(&[u8], u32) -> Result<SlotVector>;

    #[test]
    fn cameo_features_extract_event_time_series_fields() {
        let vector = cameo_features(GDELT_ROW);

        println!("GDELT_CAMEO_VECTOR={vector:?}");
        assert_eq!(vector.len(), 16);
        assert_eq!(vector[0], 1.0);
        assert!((vector[1] - 31.0 / 999.0).abs() < 0.0001);
        assert!((vector[2] - 3.0 / 20.0).abs() < 0.0001);
        assert!((vector[3] - 0.25).abs() < 0.0001);
        assert!((vector[4] - 0.52).abs() < 0.0001);
        assert!((vector[5] + 0.0125).abs() < 0.0001);
        assert_eq!(vector[6], 1.0);
        assert_eq!(vector[12], 1.0);
        assert_eq!(vector[13], 1.0);
        assert_eq!(vector[14], 1.0);
    }

    #[test]
    fn actor_geo_emits_normalized_sparse_entity_vector() {
        let SlotVector::Sparse { dim, entries } = actor_geo(GDELT_ROW, 64).unwrap() else {
            panic!("expected sparse GDELT entity vector");
        };
        let sum = entries.iter().map(|entry| entry.val).sum::<f32>();

        println!("GDELT_ACTOR_GEO_ENTRIES={entries:?}");
        assert_eq!(dim, 64);
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|entry| entry.idx < 64));
        assert!((sum - 1.0).abs() < 0.0001);
    }

    #[test]
    fn actor_geo_empty_input_is_empty_sparse_state() {
        let SlotVector::Sparse { dim, entries } = actor_geo(b"", 64).unwrap() else {
            panic!("expected sparse GDELT entity vector");
        };

        println!("GDELT_ACTOR_GEO_EMPTY_ENTRIES={entries:?}");
        assert_eq!(dim, 64);
        assert!(entries.is_empty());
    }

    #[test]
    fn gdelt_sparse_signal_lenses_emit_normalized_vectors() {
        let cases: [(&str, SparseLensFn); 6] = [
            ("source_domain", source_domain),
            ("event_geo", event_geo),
            ("actor_pair", actor_pair),
            ("event_actor", event_actor),
            ("tone_signal", tone_signal),
            ("source_event", source_event),
        ];

        for (name, build) in cases {
            let SlotVector::Sparse { dim, entries } = build(GDELT_ROW, 128).unwrap() else {
                panic!("expected sparse GDELT vector for {name}");
            };
            let sum = entries.iter().map(|entry| entry.val).sum::<f32>();

            println!("GDELT_{name}_ENTRIES={entries:?}");
            assert_eq!(dim, 128);
            assert!(!entries.is_empty(), "{name} should emit features");
            assert!(entries.iter().all(|entry| entry.idx < 128));
            assert!((sum - 1.0).abs() < 0.0001, "{name} sum={sum}");
        }
    }
}
