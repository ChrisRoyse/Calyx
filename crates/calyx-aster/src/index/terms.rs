//! Shared term normalization for Aster inverted secondary indexes.

use std::collections::BTreeMap;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

pub fn term_frequencies(text: &str) -> (BTreeMap<String, u32>, u32) {
    let terms = tokenize(text);
    let doc_len = terms.len() as u32;
    let mut counts = BTreeMap::new();
    for term in terms {
        *counts.entry(term).or_default() += 1;
    }
    (counts, doc_len)
}

pub fn term_hash(term: &str) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in term.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenization_matches_ph25_shape() {
        assert_eq!(tokenize("Cat, hat! CAT"), ["cat", "hat", "cat"]);
        assert_eq!(tokenize("quick-fox_42"), ["quick", "fox", "42"]);
    }

    #[test]
    fn fnv_hash_is_byte_stable() {
        assert_eq!(term_hash("quick"), 0x4b6e_bd14_3365_6d9c);
    }
}
