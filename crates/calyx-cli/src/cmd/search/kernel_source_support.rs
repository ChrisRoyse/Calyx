//! Deterministic proposition support over DB-native retained constellation bytes.

use std::collections::BTreeSet;

use calyx_aster::ledger_view::read_ledger_seq;
use calyx_aster::retained_input::input_from_ref;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, CxId};
use calyx_ledger::decode;
use calyx_lodestar::{AnswerDerivation, KernelAnswerSourceEvidence, KernelAnswerSourceSupport};

use super::super::kernel_generation::sha256_bytes;
use super::super::vault::{ResolvedVault, vault_salt};
use super::{base_read_cfs, latest_read_vault_options_for_cfs};
use crate::error::{CliError, CliResult};

pub(super) const CALYX_KERNEL_SOURCE_LOW_SUPPORT: &str = "CALYX_KERNEL_SOURCE_LOW_SUPPORT";
const SUPPORT_METHOD_V1: &str = "retained_constellation_lexical_v1";
const SUPPORT_METHOD_V2: &str = "retained_constellation_lexical_v2";
const MINIMUM_WEIGHTED_COVERAGE_BPS: u16 = 6_000;

pub(super) fn evaluate_path_source_support(
    resolved: &ResolvedVault,
    query: &str,
    derivation: &AnswerDerivation,
) -> CliResult<KernelAnswerSourceSupport> {
    evaluate_path_source_support_with_method(resolved, query, derivation, SUPPORT_METHOD_V2)
}

pub(super) fn evaluate_path_source_support_with_method(
    resolved: &ResolvedVault,
    query: &str,
    derivation: &AnswerDerivation,
    method: &str,
) -> CliResult<KernelAnswerSourceSupport> {
    if !matches!(method, SUPPORT_METHOD_V1 | SUPPORT_METHOD_V2) {
        return Err(CliError::runtime(format!(
            "unsupported retained source-support method {method:?}"
        )));
    }
    let query_terms = meaningful_terms(query, method);
    if query_terms.is_empty() {
        return Ok(KernelAnswerSourceSupport {
            schema_version: 1,
            method: method.to_string(),
            verdict: "low_support".to_string(),
            query_terms,
            matched_terms: Vec::new(),
            missing_terms: Vec::new(),
            matched_term_pairs: Vec::new(),
            matched_weight: 0,
            total_weight: 0,
            weighted_coverage_bps: 0,
            minimum_weighted_coverage_bps: MINIMUM_WEIGHTED_COVERAGE_BPS,
            sources: Vec::new(),
        });
    }

    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        latest_read_vault_options_for_cfs(Some(base_read_cfs())),
    )?;
    let snapshot = vault.latest_seq();
    let query_pairs = adjacent_pairs(&query_terms);
    let mut sources = Vec::new();
    let mut union_terms = BTreeSet::new();
    let mut union_pairs = BTreeSet::new();

    for cx_id in path_cx_ids(derivation) {
        let base = vault.get_base(cx_id, snapshot)?;
        verify_base_ledger_ref(&resolved.path, cx_id, &base.provenance)?;
        let input = input_from_ref(&resolved.path, base.modality, &base.input_ref)?;
        let text = std::str::from_utf8(&input.bytes).map_err(|error| {
            CliError::Calyx(CalyxError {
                code: "CALYX_KERNEL_SOURCE_INVALID_UTF8",
                message: format!(
                    "retained source bytes for constellation {cx_id} are not UTF-8 text: {error}"
                ),
                remediation: "restore the exact UTF-8 source bytes or use a modality-native deterministic support gate",
            })
        })?;
        let source_terms = meaningful_terms_with_duplicates(text, method);
        let source_term_set = source_terms.iter().cloned().collect::<BTreeSet<_>>();
        let matched_terms = query_terms
            .iter()
            .filter(|term| source_term_set.contains(*term))
            .cloned()
            .collect::<Vec<_>>();
        let matched_pairs = query_pairs
            .iter()
            .filter(|(left, right)| pair_occurs(&source_terms, left, right))
            .map(|(left, right)| format!("{left} {right}"))
            .collect::<Vec<_>>();
        union_terms.extend(matched_terms.iter().cloned());
        union_pairs.extend(matched_pairs.iter().cloned());
        sources.push(KernelAnswerSourceEvidence {
            cx_id,
            input_blake3: hex(&base.input_ref.hash),
            retained_bytes_sha256: sha256_bytes(&input.bytes),
            input_bytes: input.bytes.len() as u64,
            base_ledger_seq: base.provenance.seq,
            base_ledger_hash: hex(&base.provenance.hash),
            matched_terms,
            matched_term_pairs: matched_pairs,
        });
    }

    let matched_terms = query_terms
        .iter()
        .filter(|term| union_terms.contains(*term))
        .cloned()
        .collect::<Vec<_>>();
    let missing_terms = query_terms
        .iter()
        .filter(|term| !union_terms.contains(*term))
        .cloned()
        .collect::<Vec<_>>();
    let matched_term_pairs = query_pairs
        .iter()
        .map(|(left, right)| format!("{left} {right}"))
        .filter(|pair| union_pairs.contains(pair))
        .collect::<Vec<_>>();
    let total_weight = query_terms
        .iter()
        .map(|term| term_weight(term))
        .sum::<u64>();
    let matched_weight = matched_terms
        .iter()
        .map(|term| term_weight(term))
        .sum::<u64>();
    let weighted_coverage_bps = ((matched_weight * 10_000) / total_weight)
        .try_into()
        .expect("basis-point coverage is bounded by 10000");
    let has_proposition_pair = query_terms.len() == 1 || !matched_term_pairs.is_empty();
    let verdict = if weighted_coverage_bps >= MINIMUM_WEIGHTED_COVERAGE_BPS && has_proposition_pair
    {
        "supported"
    } else {
        "low_support"
    };

    Ok(KernelAnswerSourceSupport {
        schema_version: 1,
        method: method.to_string(),
        verdict: verdict.to_string(),
        query_terms,
        matched_terms,
        missing_terms,
        matched_term_pairs,
        matched_weight,
        total_weight,
        weighted_coverage_bps,
        minimum_weighted_coverage_bps: MINIMUM_WEIGHTED_COVERAGE_BPS,
        sources,
    })
}

fn verify_base_ledger_ref(
    vault: &std::path::Path,
    cx_id: CxId,
    reference: &calyx_core::LedgerRef,
) -> CliResult {
    let row = read_ledger_seq(vault, reference.seq)?.ok_or_else(|| {
        CalyxError::ledger_corrupt(format!(
            "Base provenance ledger seq {} for constellation {cx_id} is absent",
            reference.seq
        ))
    })?;
    let entry = decode(&row.bytes)?;
    if row.seq != reference.seq || entry.seq != reference.seq || entry.entry_hash != reference.hash
    {
        return Err(CalyxError::ledger_corrupt(format!(
            "Base provenance ledger ref for constellation {cx_id} differs from physical seq {}",
            reference.seq
        ))
        .into());
    }
    Ok(())
}

fn path_cx_ids(derivation: &AnswerDerivation) -> Vec<CxId> {
    let mut ids = vec![derivation.anchor_kernel_node];
    for hop in &derivation.hops {
        if ids.last() != Some(&hop.from) {
            ids.push(hop.from);
        }
        if ids.last() != Some(&hop.to) {
            ids.push(hop.to);
        }
    }
    ids
}

fn meaningful_terms(text: &str, method: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    meaningful_terms_with_duplicates(text, method)
        .into_iter()
        .filter(|term| seen.insert(term.clone()))
        .collect()
}

fn meaningful_terms_with_duplicates(text: &str, method: &str) -> Vec<String> {
    calyx_sextant::index::tokenizer::tokenize(text)
        .into_iter()
        .map(|term| normalize_term(term, method))
        .filter(|term| term.len() >= 3 && !is_stop_word(term))
        .collect()
}

fn normalize_term(mut term: String, method: &str) -> String {
    if method == SUPPORT_METHOD_V2 {
        if term.len() > 6 && term.ends_with("ing") {
            term.truncate(term.len() - 3);
            trim_doubled_inflection_consonant(&mut term);
        } else if term.len() > 5 && term.ends_with("ed") {
            term.truncate(term.len() - 2);
            trim_doubled_inflection_consonant(&mut term);
        } else if term.len() > 7 && term.ends_with("ssion") {
            term.truncate(term.len() - 3);
        }
    }
    if term.len() > 4 && term.ends_with("ies") {
        term.truncate(term.len() - 3);
        term.push('y');
    } else if term.len() > 4
        && term.ends_with('s')
        && !term.ends_with("ss")
        && !term.ends_with("us")
        && !term.ends_with("is")
    {
        term.pop();
    }
    term
}

fn trim_doubled_inflection_consonant(term: &mut String) {
    let bytes = term.as_bytes();
    if bytes.len() >= 2
        && bytes[bytes.len() - 1] == bytes[bytes.len() - 2]
        && bytes[bytes.len() - 1] != b's'
    {
        term.pop();
    }
}

fn adjacent_pairs(terms: &[String]) -> Vec<(String, String)> {
    terms
        .windows(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect()
}

fn pair_occurs(source: &[String], left: &str, right: &str) -> bool {
    source.iter().enumerate().any(|(index, term)| {
        term == left
            && source
                .iter()
                .skip(index + 1)
                .take(8)
                .any(|candidate| candidate == right)
    })
}

fn term_weight(term: &str) -> u64 {
    term.chars().count().max(3) as u64
}

fn is_stop_word(term: &str) -> bool {
    matches!(
        term,
        "about"
            | "above"
            | "after"
            | "again"
            | "against"
            | "all"
            | "also"
            | "among"
            | "and"
            | "any"
            | "are"
            | "because"
            | "been"
            | "before"
            | "being"
            | "below"
            | "between"
            | "both"
            | "but"
            | "can"
            | "case"
            | "could"
            | "county"
            | "court"
            | "did"
            | "does"
            | "doing"
            | "down"
            | "during"
            | "each"
            | "few"
            | "for"
            | "from"
            | "further"
            | "had"
            | "has"
            | "have"
            | "having"
            | "her"
            | "here"
            | "hers"
            | "him"
            | "his"
            | "how"
            | "into"
            | "its"
            | "itself"
            | "law"
            | "may"
            | "might"
            | "more"
            | "most"
            | "must"
            | "not"
            | "off"
            | "once"
            | "only"
            | "onto"
            | "opinion"
            | "other"
            | "our"
            | "ours"
            | "out"
            | "over"
            | "own"
            | "same"
            | "shall"
            | "she"
            | "should"
            | "some"
            | "state"
            | "such"
            | "than"
            | "that"
            | "the"
            | "their"
            | "theirs"
            | "them"
            | "themselves"
            | "then"
            | "there"
            | "these"
            | "they"
            | "this"
            | "those"
            | "through"
            | "too"
            | "under"
            | "until"
            | "upon"
            | "very"
            | "was"
            | "were"
            | "what"
            | "when"
            | "where"
            | "which"
            | "while"
            | "who"
            | "whom"
            | "whose"
            | "why"
            | "will"
            | "with"
            | "within"
            | "without"
            | "would"
            | "you"
            | "your"
            | "yours"
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
