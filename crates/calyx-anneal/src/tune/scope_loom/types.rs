use std::collections::{BTreeMap, HashMap, HashSet};

use calyx_core::{LensId, Result};
use calyx_forge::{AutotuneKey, BackendKind, BestConfig};
use serde::{Deserialize, Serialize};

use super::invalid_config;
use crate::{AssayMetrics, shape_key_hash};

pub const MAX_LOOM_CANDIDATES: usize = 8;
pub const MAX_LOOM_EAGER_PAIRS: usize = 64;
pub const MIN_LOOM_PAIR_BITS: f64 = 0.05;
pub const DEFAULT_LOOM_RECALL_TARGET: f32 = 1.0;

const LOOM_PLAN_SHAPE_KEY: &str = "loom:materialization";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ConcatKey {
    pub a: LensId,
    pub b: LensId,
}

impl ConcatKey {
    pub fn new(a: LensId, b: LensId) -> Self {
        let (a, b) = canonical_pair(a, b);
        Self { a, b }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MatPlanConfig {
    pub eager_pairs: Vec<(LensId, LensId)>,
    pub indexed_concat_keys: Vec<ConcatKey>,
}

impl MatPlanConfig {
    pub fn canonicalized(mut self) -> Self {
        self.eager_pairs = canonical_pairs(self.eager_pairs);
        self.indexed_concat_keys = self
            .indexed_concat_keys
            .into_iter()
            .map(|key| ConcatKey::new(key.a, key.b))
            .collect();
        self.indexed_concat_keys.sort();
        self.indexed_concat_keys.dedup();
        self
    }

    pub fn to_best_config(&self, score: PlanScore) -> Result<BestConfig> {
        validate_mat_plan_config(self)?;
        Ok(BestConfig {
            backend: BackendKind::Cpu,
            tile_m: self.eager_pairs.len(),
            tile_n: self.indexed_concat_keys.len(),
            tile_k: score.avg_latency_ns as usize,
            extra: HashMap::from([
                ("scope".to_string(), "loom".to_string()),
                ("source".to_string(), "anneal-loom-scope".to_string()),
                (
                    "eager_pairs_count".to_string(),
                    self.eager_pairs.len().to_string(),
                ),
                (
                    "indexed_concat_keys_count".to_string(),
                    self.indexed_concat_keys.len().to_string(),
                ),
                ("bits_sum".to_string(), format!("{:.12}", score.bits_sum)),
                (
                    "avg_latency_ns".to_string(),
                    score.avg_latency_ns.to_string(),
                ),
                ("eager_pairs".to_string(), join_pairs(&self.eager_pairs)),
                (
                    "indexed_concat_keys".to_string(),
                    join_concat_keys(&self.indexed_concat_keys),
                ),
                ("plan_hash".to_string(), hex(&plan_hash(self)?)),
            ]),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanScore {
    pub avg_latency_ns: u64,
    pub bits_sum: f64,
    pub query_count: usize,
    pub eager_pair_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryLog {
    pub observations: Vec<QueryObservation>,
    pub max_eager_pairs: usize,
    pub max_indexed_concat_keys: usize,
}

impl QueryLog {
    pub fn with_budgets(max_eager_pairs: usize, max_indexed_concat_keys: usize) -> Self {
        Self {
            observations: Vec::new(),
            max_eager_pairs,
            max_indexed_concat_keys,
        }
    }

    pub fn push(&mut self, observation: QueryObservation) {
        self.observations.push(observation.canonicalized());
    }

    fn stats(&self) -> BTreeMap<(LensId, LensId), PairStats> {
        let mut stats = BTreeMap::<(LensId, LensId), PairStats>::new();
        for observation in &self.observations {
            let pair = canonical_pair(observation.a, observation.b);
            let entry = stats.entry(pair).or_insert_with(|| PairStats::new(pair));
            entry.queries += 1;
            entry.bits = entry.bits.max(observation.bits_per_anchor);
        }
        stats
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryObservation {
    pub a: LensId,
    pub b: LensId,
    pub lazy_latency_ns: u64,
    pub eager_latency_ns: u64,
    pub indexed_latency_ns: Option<u64>,
    pub bits_per_anchor: f64,
}

impl QueryObservation {
    pub fn new(
        a: LensId,
        b: LensId,
        lazy_latency_ns: u64,
        eager_latency_ns: u64,
        indexed_latency_ns: Option<u64>,
        bits_per_anchor: f64,
    ) -> Self {
        Self {
            a,
            b,
            lazy_latency_ns,
            eager_latency_ns,
            indexed_latency_ns,
            bits_per_anchor,
        }
        .canonicalized()
    }

    fn canonicalized(mut self) -> Self {
        let (a, b) = canonical_pair(self.a, self.b);
        self.a = a;
        self.b = b;
        self
    }
}

#[derive(Clone, Copy, Debug)]
struct PairStats {
    pair: (LensId, LensId),
    queries: usize,
    bits: f64,
}

impl PairStats {
    fn new(pair: (LensId, LensId)) -> Self {
        Self {
            pair,
            queries: 0,
            bits: 0.0,
        }
    }
}

pub fn evaluate_plan(
    plan: &MatPlanConfig,
    query_log: &QueryLog,
    _assay: &dyn AssayMetrics,
) -> PlanScore {
    let plan = plan.clone().canonicalized();
    let eager: HashSet<_> = plan.eager_pairs.iter().copied().collect();
    let indexed: HashSet<_> = plan
        .indexed_concat_keys
        .iter()
        .map(|key| (key.a, key.b))
        .collect();
    let mut latency_sum = 0_u128;
    for observation in &query_log.observations {
        let pair = canonical_pair(observation.a, observation.b);
        let latency = if indexed.contains(&pair) {
            observation
                .indexed_latency_ns
                .unwrap_or(observation.eager_latency_ns)
        } else if eager.contains(&pair) {
            observation.eager_latency_ns
        } else {
            observation.lazy_latency_ns
        };
        latency_sum += u128::from(latency);
    }
    let stats = query_log.stats();
    let bits_sum = eager
        .iter()
        .map(|pair| stats.get(pair).map(|stat| stat.bits).unwrap_or(0.0))
        .sum();
    PlanScore {
        avg_latency_ns: average_latency(latency_sum, query_log.observations.len()),
        bits_sum,
        query_count: query_log.observations.len(),
        eager_pair_count: eager.len(),
    }
}

pub fn generate_candidate_plan(
    current: &MatPlanConfig,
    _assay: &dyn AssayMetrics,
    query_log: &QueryLog,
) -> MatPlanConfig {
    let mut candidate = current.clone().canonicalized();
    let stats = query_log.stats();
    if stats.is_empty() {
        return candidate;
    }
    let mut ranked: Vec<_> = stats.values().copied().collect();
    ranked.sort_by(compare_candidate_pairs);
    if let Some(pair) = ranked.iter().find_map(|stat| {
        (!candidate.eager_pairs.contains(&stat.pair)
            && stat.bits.is_finite()
            && stat.bits >= MIN_LOOM_PAIR_BITS)
            .then_some(stat.pair)
    }) {
        candidate.eager_pairs.push(pair);
        candidate.eager_pairs = canonical_pairs(candidate.eager_pairs);
    }
    drop_lowest_bits_until_budget(&mut candidate, &stats, query_log.max_eager_pairs);
    candidate.indexed_concat_keys = ranked
        .iter()
        .take(query_log.max_indexed_concat_keys)
        .map(|stat| ConcatKey::new(stat.pair.0, stat.pair.1))
        .collect();
    candidate
}

pub fn validate_mat_plan_config(config: &MatPlanConfig) -> Result<()> {
    if config.eager_pairs.len() > MAX_LOOM_EAGER_PAIRS {
        return Err(invalid_config(format!(
            "Loom eager pair count {} exceeds {MAX_LOOM_EAGER_PAIRS}",
            config.eager_pairs.len()
        )));
    }
    let canonical = canonical_pairs(config.eager_pairs.clone());
    if canonical != config.eager_pairs {
        return Err(invalid_config(
            "Loom eager pairs must be canonical, sorted, and unique",
        ));
    }
    if config.indexed_concat_keys.iter().any(|key| key.a > key.b) {
        return Err(invalid_config("Loom indexed concat keys must be canonical"));
    }
    let mut concat = config.indexed_concat_keys.clone();
    concat.sort();
    concat.dedup();
    if concat != config.indexed_concat_keys {
        return Err(invalid_config(
            "Loom indexed concat keys must be canonical, sorted, and unique",
        ));
    }
    Ok(())
}

pub fn encode_mat_plan_config(config: &MatPlanConfig) -> Result<Vec<u8>> {
    validate_mat_plan_config(config)?;
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(config, &mut bytes)
        .map_err(|error| invalid_config(error.to_string()))?;
    Ok(bytes)
}

pub fn decode_mat_plan_config(bytes: &[u8]) -> Result<MatPlanConfig> {
    let config: MatPlanConfig =
        ciborium::de::from_reader(bytes).map_err(|error| invalid_config(error.to_string()))?;
    validate_mat_plan_config(&config)?;
    Ok(config)
}

pub fn plan_hash(plan: &MatPlanConfig) -> Result<[u8; 32]> {
    Ok(*blake3::hash(&encode_mat_plan_config(plan)?).as_bytes())
}

pub fn loom_plan_shape_key() -> &'static str {
    LOOM_PLAN_SHAPE_KEY
}

pub fn loom_plan_label() -> String {
    LOOM_PLAN_SHAPE_KEY.to_string()
}

pub fn loom_plan_tune_key() -> AutotuneKey {
    AutotuneKey {
        op: "loom".to_string(),
        shape: vec![0],
        dtype: "materialization".to_string(),
        device: "association".to_string(),
        recall_tgt: DEFAULT_LOOM_RECALL_TARGET,
    }
}

pub(super) fn seed_for_loom() -> u64 {
    let hash = shape_key_hash(LOOM_PLAN_SHAPE_KEY);
    u64::from_le_bytes(hash[0..8].try_into().expect("hash slice has 8 bytes"))
}

fn compare_candidate_pairs(left: &PairStats, right: &PairStats) -> std::cmp::Ordering {
    right
        .bits
        .total_cmp(&left.bits)
        .then_with(|| right.queries.cmp(&left.queries))
        .then_with(|| left.pair.cmp(&right.pair))
}

fn drop_lowest_bits_until_budget(
    candidate: &mut MatPlanConfig,
    stats: &BTreeMap<(LensId, LensId), PairStats>,
    budget: usize,
) {
    while candidate.eager_pairs.len() > budget {
        let Some((idx, _)) =
            candidate
                .eager_pairs
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| {
                    pair_bits(left, stats)
                        .total_cmp(&pair_bits(right, stats))
                        .then_with(|| left.cmp(right))
                })
        else {
            return;
        };
        candidate.eager_pairs.remove(idx);
    }
}

fn pair_bits(pair: &(LensId, LensId), stats: &BTreeMap<(LensId, LensId), PairStats>) -> f64 {
    stats.get(pair).map(|stat| stat.bits).unwrap_or(0.0)
}

fn canonical_pair(a: LensId, b: LensId) -> (LensId, LensId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn canonical_pairs(pairs: Vec<(LensId, LensId)>) -> Vec<(LensId, LensId)> {
    let mut pairs: Vec<_> = pairs
        .into_iter()
        .map(|(a, b)| canonical_pair(a, b))
        .filter(|(a, b)| a != b)
        .collect();
    pairs.sort();
    pairs.dedup();
    pairs
}

fn average_latency(sum: u128, count: usize) -> u64 {
    if count == 0 {
        0
    } else {
        (sum / count as u128).min(u128::from(u64::MAX)) as u64
    }
}

fn join_pairs(pairs: &[(LensId, LensId)]) -> String {
    pairs
        .iter()
        .map(|(a, b)| format!("{a}:{b}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn join_concat_keys(keys: &[ConcatKey]) -> String {
    keys.iter()
        .map(|key| format!("{}:{}", key.a, key.b))
        .collect::<Vec<_>>()
        .join(",")
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
