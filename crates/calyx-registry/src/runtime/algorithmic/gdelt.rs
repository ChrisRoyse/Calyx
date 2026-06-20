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

fn add_labeled_token(counts: &mut BTreeMap<u32, f32>, dim: u32, label: &str, token: Option<&str>) {
    if let Some(token) = token {
        add_term(counts, dim, &format!("{label}:{token}"), 2.0);
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
}
