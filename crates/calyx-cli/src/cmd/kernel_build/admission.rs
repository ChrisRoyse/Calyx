use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;

use calyx_core::SlotId;
use calyx_lodestar::{
    TypedPanelFusionLane, TypedPanelVectors, rank_typed_panel_top_k_candidate_refs,
};
use calyx_registry::VaultPanelState;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::super::kernel_generation::{
    KernelAdmissionContract, KernelArtifactRef, artifact_ref, hex32, sha256_bytes,
};
use super::super::vault::ResolvedVault;
use super::KernelBuildNodeProps;
use crate::durable_write::write_bytes_atomic;
use crate::error::{CliError, CliResult};

const CORPUS_SAMPLE_LIMIT: usize = 128;
const MIN_REAL_QUERY_COUNT: usize = 20;
const MAX_REAL_QUERY_COUNT: usize = 512;
const MAX_QUERY_FILE_BYTES: usize = 1_048_576;
const LOWER_TAIL_QUANTILE: f32 = 0.05;
const SAMPLE_SEED: u64 = 1_460;
const SAMPLE_DOMAIN: &[u8] = b"calyx-kernel-admission-sample-v1";
const CORPUS_OBSERVATION_DOMAIN: &[u8] =
    b"calyx-kernel-admission-corpus-observations-v3-typed-block";
const QUERY_ID_DOMAIN: &[u8] = b"calyx-kernel-admission-query-ids-v1";
const QUERY_OBSERVATION_DOMAIN: &[u8] = b"calyx-kernel-admission-query-observations-v3-typed-block";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionQueryRecord {
    query_id: String,
    query: String,
}

struct MeasuredAdmissionQuery {
    query_id: String,
    vectors: TypedPanelVectors,
}

pub(super) fn calibrate_corpus_neighbors(
    slots: &[SlotId],
    rows: &std::collections::BTreeMap<calyx_core::CxId, TypedPanelVectors>,
) -> CliResult<KernelAdmissionContract> {
    validate_corpus(slots, rows)?;
    let mut ranked = rows
        .iter()
        .map(|(cx_id, vectors)| {
            let mut digest = Sha256::new();
            digest.update(SAMPLE_DOMAIN);
            digest.update(SAMPLE_SEED.to_le_bytes());
            digest.update(cx_id.as_bytes());
            (digest.finalize().to_vec(), *cx_id, vectors)
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    ranked.truncate(CORPUS_SAMPLE_LIMIT.min(ranked.len()));

    let mut sample_ids = Sha256::new();
    sample_ids.update(SAMPLE_DOMAIN);
    let mut observations = Sha256::new();
    observations.update(CORPUS_OBSERVATION_DOMAIN);
    let refs = rows
        .iter()
        .map(|(cx_id, vectors)| (*cx_id, vectors))
        .collect::<BTreeMap<_, _>>();
    let mut similarities = Vec::with_capacity(ranked.len());
    for (_, query_id, query) in ranked {
        sample_ids.update(query_id.as_bytes());
        let nearest = rank_typed_panel_top_k_candidate_refs(query, &refs, slots, 2)?
            .into_iter()
            .find(|hit| hit.cx_id != query_id)
            .ok_or_else(|| {
                CliError::runtime(format!(
                    "kernel admission calibration found no non-self neighbor for {}",
                    query_id
                ))
            })?;
        record_similarity(
            &mut observations,
            query_id.as_bytes(),
            nearest.cx_id.as_bytes(),
            nearest.score,
            &nearest.lanes,
            &mut similarities,
        )?;
    }
    contract(
        "loo_typed_block_rrf_p05_v3",
        rows.len(),
        similarities,
        CORPUS_SAMPLE_LIMIT,
        SAMPLE_SEED,
        hex32(&sample_ids.finalize().into()),
        hex32(&observations.finalize().into()),
        None,
    )
}

pub(super) fn calibrate_real_queries(
    resolved: &ResolvedVault,
    state: &VaultPanelState,
    embedding_slots: &[SlotId],
    rows: &std::collections::BTreeMap<calyx_core::CxId, TypedPanelVectors>,
    source_path: &Path,
    resident_addr: SocketAddr,
) -> CliResult<KernelAdmissionContract> {
    validate_corpus(embedding_slots, rows)?;
    let (source_bytes, records) = read_real_queries(source_path)?;
    let mut measured = Vec::with_capacity(records.len());
    for record in records {
        let vectors = match super::super::search::measure_kernel_calibration_query(
            state,
            resolved,
            &record.query,
            resident_addr,
            embedding_slots,
        ) {
            Ok(vector) => vector,
            Err(error) => {
                eprintln!(
                    "kernel-build: admission query measurement failed query_id={} resident_addr={} embedding_slots={:?} code={} message={} remediation={}",
                    record.query_id,
                    resident_addr,
                    embedding_slots
                        .iter()
                        .map(|slot| slot.get())
                        .collect::<Vec<_>>(),
                    error.code(),
                    error.message(),
                    error.remediation()
                );
                return Err(error);
            }
        };
        measured.push(MeasuredAdmissionQuery {
            query_id: record.query_id,
            vectors,
        });
    }

    let query_count = measured.len();
    let mut sample_ids = Sha256::new();
    sample_ids.update(QUERY_ID_DOMAIN);
    let mut observations = Sha256::new();
    observations.update(QUERY_OBSERVATION_DOMAIN);
    let refs = rows
        .iter()
        .map(|(cx_id, vectors)| (*cx_id, vectors))
        .collect::<BTreeMap<_, _>>();
    let mut similarities = Vec::with_capacity(measured.len());
    for query in measured {
        hash_sized(&mut sample_ids, query.query_id.as_bytes());
        let nearest =
            rank_typed_panel_top_k_candidate_refs(&query.vectors, &refs, embedding_slots, 1)?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    CliError::runtime(format!(
                        "real admission query {} found no corpus neighbor",
                        query.query_id
                    ))
                })?;
        let mut query_key = Sha256::new();
        query_key.update(QUERY_ID_DOMAIN);
        query_key.update(query.query_id.as_bytes());
        let query_key = query_key.finalize();
        record_similarity(
            &mut observations,
            &query_key,
            nearest.cx_id.as_bytes(),
            nearest.score,
            &nearest.lanes,
            &mut similarities,
        )?;
    }
    let source = retain_real_queries(&resolved.path, &source_bytes)?;
    contract(
        "real_query_typed_block_rrf_p05_v3",
        rows.len(),
        similarities,
        query_count,
        0,
        hex32(&sample_ids.finalize().into()),
        hex32(&observations.finalize().into()),
        Some(source),
    )
}

fn validate_corpus(
    slots: &[SlotId],
    rows: &std::collections::BTreeMap<calyx_core::CxId, TypedPanelVectors>,
) -> CliResult {
    if rows.len() < 2 {
        return Err(CliError::usage(
            "kernel admission calibration requires at least two corpus vectors",
        ));
    }
    let first = rows.values().next().expect("validated nonempty corpus");
    let refs = rows
        .iter()
        .map(|(cx_id, vectors)| (*cx_id, vectors))
        .collect::<BTreeMap<_, _>>();
    let _ = rank_typed_panel_top_k_candidate_refs(first, &refs, slots, 1)?;
    Ok(())
}

fn read_real_queries(path: &Path) -> CliResult<(Vec<u8>, Vec<AdmissionQueryRecord>)> {
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::io(format!(
            "stat admission query file {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_QUERY_FILE_BYTES as u64 {
        return Err(CliError::usage(format!(
            "admission query source {} must be a nonempty regular file no larger than {MAX_QUERY_FILE_BYTES} bytes",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|error| {
        CliError::io(format!(
            "read admission query file {}: {error}",
            path.display()
        ))
    })?;
    if bytes.is_empty() || bytes.len() > MAX_QUERY_FILE_BYTES {
        return Err(CliError::usage(format!(
            "admission query source {} changed size while it was read; observed {} bytes, expected 1..={MAX_QUERY_FILE_BYTES}",
            path.display(),
            bytes.len()
        )));
    }
    if !bytes.ends_with(b"\n") {
        return Err(CliError::usage(format!(
            "admission query source {} must be newline terminated",
            path.display()
        )));
    }
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        CliError::usage(format!(
            "admission query source {} is not UTF-8: {error}",
            path.display()
        ))
    })?;
    let mut records = Vec::new();
    let mut ids = BTreeSet::new();
    let mut queries = BTreeSet::new();
    for (offset, line) in text.lines().enumerate() {
        let line_number = offset + 1;
        if line.is_empty() || line.trim() != line {
            return Err(CliError::usage(format!(
                "admission query source {} line {line_number} is blank or has surrounding whitespace",
                path.display()
            )));
        }
        let record: AdmissionQueryRecord = serde_json::from_str(line).map_err(|error| {
            CliError::usage(format!(
                "parse admission query source {} line {line_number}: {error}",
                path.display()
            ))
        })?;
        if record.query_id.is_empty()
            || record.query_id.len() > 96
            || !record.query_id.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            })
        {
            return Err(CliError::usage(format!(
                "admission query source {} line {line_number} has an invalid query_id",
                path.display()
            )));
        }
        if record.query.trim() != record.query
            || record.query.len() < 10
            || record.query.len() > 2_000
            || record.query.chars().any(char::is_control)
        {
            return Err(CliError::usage(format!(
                "admission query source {} line {line_number} has an invalid query",
                path.display()
            )));
        }
        if !ids.insert(record.query_id.clone()) || !queries.insert(record.query.clone()) {
            return Err(CliError::usage(format!(
                "admission query source {} line {line_number} duplicates a query_id or query",
                path.display()
            )));
        }
        records.push(record);
    }
    if !(MIN_REAL_QUERY_COUNT..=MAX_REAL_QUERY_COUNT).contains(&records.len()) {
        return Err(CliError::usage(format!(
            "admission query source {} contains {} queries; expected {MIN_REAL_QUERY_COUNT}..={MAX_REAL_QUERY_COUNT}",
            path.display(),
            records.len()
        )));
    }
    Ok((bytes, records))
}

fn retain_real_queries(vault: &Path, bytes: &[u8]) -> CliResult<KernelArtifactRef> {
    let sha256 = sha256_bytes(bytes);
    let path = vault
        .join("inputs")
        .join("kernel-admission")
        .join(format!("{sha256}.jsonl"));
    if path.exists() {
        let existing = fs::read(&path).map_err(|error| {
            CliError::io(format!(
                "read retained admission queries {}: {error}",
                path.display()
            ))
        })?;
        if existing != bytes {
            return Err(CliError::runtime(format!(
                "retained admission query path {} contains bytes that do not match its digest",
                path.display()
            )));
        }
    } else {
        write_bytes_atomic(&path, bytes, "kernel admission queries")?;
    }
    let readback = fs::read(&path).map_err(|error| {
        CliError::io(format!(
            "read back retained admission queries {}: {error}",
            path.display()
        ))
    })?;
    if readback != bytes || sha256_bytes(&readback) != sha256 {
        return Err(CliError::runtime(format!(
            "retained admission query physical readback mismatch at {}",
            path.display()
        )));
    }
    artifact_ref(vault, &path)
}

fn record_similarity(
    observations: &mut Sha256,
    query_key: &[u8],
    nearest_key: &[u8],
    similarity: f32,
    lanes: &[TypedPanelFusionLane],
    similarities: &mut Vec<f32>,
) -> CliResult {
    if !similarity.is_finite() || !(0.0..=1.0).contains(&similarity) {
        return Err(CliError::runtime(format!(
            "kernel admission calibration produced invalid typed fused similarity {similarity}"
        )));
    }
    hash_sized(observations, query_key);
    hash_sized(observations, nearest_key);
    observations.update(similarity.to_bits().to_le_bytes());
    observations.update((lanes.len() as u64).to_le_bytes());
    for lane in lanes {
        lane.similarity.validate().map_err(CliError::from)?;
        observations.update(lane.slot.get().to_le_bytes());
        let evidence = serde_json::to_vec(&lane.similarity).map_err(|error| {
            CliError::runtime(format!(
                "encode typed kernel admission evidence for slot {}: {error}",
                lane.slot
            ))
        })?;
        hash_sized(observations, &evidence);
        observations.update((lane.rank as u64).to_le_bytes());
        observations.update(lane.rrf_contribution.to_bits().to_le_bytes());
    }
    similarities.push(similarity);
    Ok(())
}

fn hash_sized(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

#[allow(clippy::too_many_arguments)]
fn contract(
    method: &str,
    corpus_count: usize,
    mut similarities: Vec<f32>,
    sample_limit: usize,
    sample_seed: u64,
    sample_ids_sha256: String,
    observations_sha256: String,
    calibration_queries: Option<KernelArtifactRef>,
) -> CliResult<KernelAdmissionContract> {
    if similarities.len() < 2 {
        return Err(CliError::runtime(
            "kernel admission calibration produced fewer than two observations",
        ));
    }
    similarities.sort_by(f32::total_cmp);
    let lower_tail_index = (LOWER_TAIL_QUANTILE * (similarities.len() - 1) as f32).floor() as usize;
    Ok(KernelAdmissionContract {
        schema_version: 3,
        method: method.to_string(),
        corpus_count,
        sample_count: similarities.len(),
        sample_limit,
        sample_seed,
        lower_tail_quantile: LOWER_TAIL_QUANTILE,
        threshold: similarities[lower_tail_index],
        min_score: similarities[0],
        median_score: similarities[similarities.len() / 2],
        max_score: similarities[similarities.len() - 1],
        sample_ids_sha256,
        observations_sha256,
        calibration_queries,
    })
}

pub(super) fn emit(contract: &KernelAdmissionContract) {
    eprintln!(
        "kernel-build: calibrated query admission method={} sample={} corpus={} lower_tail_quantile={} threshold={:.6} range=[{:.6},{:.6}] source={}",
        contract.method,
        contract.sample_count,
        contract.corpus_count,
        contract.lower_tail_quantile,
        contract.threshold,
        contract.min_score,
        contract.max_score,
        contract
            .calibration_queries
            .as_ref()
            .map_or("physical-corpus", |source| source.relative_path.as_str())
    );
}

pub(super) fn jurisdiction(
    rows: &[KernelBuildNodeProps],
) -> CliResult<Option<super::super::kernel_generation::KernelJurisdictionContract>> {
    let metadata = rows
        .iter()
        .map(|row| row.metadata.clone())
        .collect::<Vec<_>>();
    super::super::kernel_scope::derive_jurisdiction(&metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_real_query_source_is_strict_and_sufficient() {
        let bytes =
            include_bytes!("../../../../../tools/lawdemo/cuyahoga_admission_queries.v1.jsonl");
        let path = std::env::temp_dir().join(format!(
            "calyx-real-admission-queries-{}.jsonl",
            std::process::id()
        ));
        fs::write(&path, bytes).unwrap();
        let (readback, records) = read_real_queries(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(readback, bytes);
        assert!(records.len() >= MIN_REAL_QUERY_COUNT);
    }
}
