use std::collections::BTreeMap;

use calyx_core::{Result, SlotVector};

use super::{
    add_domain_terms, add_event_terms, add_labeled_clean, add_term, clean_token_after,
    country_tokens, event_parts, geo_tokens, source_host, sparse_vector,
};

pub(in crate::runtime::algorithmic) fn action_geo(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    for geo in geo_tokens(&text) {
        add_term(&mut counts, dim, &format!("action_geo:{geo}"), 2.0);
    }
    for country in country_tokens(&text) {
        add_term(&mut counts, dim, &format!("action_country:{country}"), 3.0);
    }
    sparse_vector(dim, counts)
}

pub(in crate::runtime::algorithmic) fn actor_country(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    for (label, actor) in [
        ("actor1", clean_token_after(&text, "Actor1 ")),
        ("actor2", clean_token_after(&text, "Actor2 ")),
    ] {
        add_term(
            &mut counts,
            dim,
            &format!("{label}_present:{}", actor.is_some()),
            1.0,
        );
        add_labeled_clean(&mut counts, dim, label, actor.as_deref(), 2.0);
    }
    sparse_vector(dim, counts)
}

pub(in crate::runtime::algorithmic) fn source_host_lens(
    bytes: &[u8],
    dim: u32,
) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    if let Some(host) = source_host(&text) {
        add_domain_terms(&mut counts, dim, &host, 3.0);
    }
    sparse_vector(dim, counts)
}

pub(in crate::runtime::algorithmic) fn sql_date(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    if let Some(date) = token_after(&text, "SQLDATE ") {
        if date.len() >= 8 {
            add_term(&mut counts, dim, &format!("year:{}", &date[0..4]), 2.0);
            add_term(&mut counts, dim, &format!("month:{}", &date[4..6]), 1.5);
            add_term(&mut counts, dim, &format!("day:{}", &date[6..8]), 1.0);
        }
        add_term(&mut counts, dim, &format!("sqldate:{date}"), 3.0);
    }
    sparse_vector(dim, counts)
}

pub(in crate::runtime::algorithmic) fn event_code(bytes: &[u8], dim: u32) -> Result<SlotVector> {
    let dim = dim.max(1);
    let text = String::from_utf8_lossy(bytes);
    let mut counts = BTreeMap::<u32, f32>::new();
    add_event_terms(&mut counts, dim, &event_parts(&text), 3.0);
    sparse_vector(dim, counts)
}

fn token_after<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    text.split_once(label)?.1.split_whitespace().next()
}
