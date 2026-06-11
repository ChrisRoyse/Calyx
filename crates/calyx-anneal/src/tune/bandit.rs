use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result};
use rand::{Rng, RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::CALYX_ASTER_CF_UNAVAILABLE;

pub type ConfigVariant = Vec<u8>;

pub const DEFAULT_HYSTERESIS_WINS: u32 = 3;
pub const CALYX_ANNEAL_BANDIT_EMPTY: &str = "CALYX_ANNEAL_BANDIT_EMPTY";
pub const CALYX_ANNEAL_BANDIT_INVALID_CONFIG: &str = "CALYX_ANNEAL_BANDIT_INVALID_CONFIG";
pub const CALYX_ANNEAL_BANDIT_INVALID_ROW: &str = "CALYX_ANNEAL_BANDIT_INVALID_ROW";

const BANDIT_ROW_TAG: &str = "anneal_bandit_v1";
const BANDIT_KEY_PREFIX: &[u8] = b"bandit\0";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BanditPolicy {
    EpsilonGreedy { epsilon: f64 },
    Thompson,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arm {
    pub config: ConfigVariant,
    pub wins: u32,
    pub trials: u32,
    pub consecutive_wins: u32,
}

impl Arm {
    pub fn new(config: ConfigVariant) -> Self {
        Self {
            config,
            wins: 0,
            trials: 0,
            consecutive_wins: 0,
        }
    }

    pub fn win_rate(&self) -> f64 {
        f64::from(self.wins) / f64::from(self.trials.max(1))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigBandit {
    pub policy: BanditPolicy,
    pub arms: Vec<Arm>,
    pub incumbent_idx: usize,
    pub hysteresis_wins: u32,
    pub rng_seed: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArmStatus {
    pub idx: usize,
    pub config_hash: [u8; 32],
    pub config_len: usize,
    pub wins: u32,
    pub trials: u32,
    pub win_rate: f64,
    pub consecutive_wins: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BanditStatus {
    pub shape_key_hash: [u8; 32],
    pub policy: BanditPolicy,
    pub incumbent: Option<usize>,
    pub arm_count: usize,
    pub hysteresis_wins: u32,
    pub rng_seed: u64,
    pub arms: Vec<ArmStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BanditReadback {
    pub shape_key_hash: [u8; 32],
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub bandit: ConfigBandit,
}

#[derive(Serialize, Deserialize)]
struct BanditRow {
    tag: String,
    bandit: ConfigBandit,
}

pub trait BanditStorage: Send + Sync {
    fn load(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
    fn save(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()>;
    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

pub struct AsterBanditStorage<'a, C>
where
    C: Clock,
{
    vault: &'a AsterVault<C>,
}

impl<'a, C> AsterBanditStorage<'a, C>
where
    C: Clock,
{
    pub const fn new(vault: &'a AsterVault<C>) -> Self {
        Self { vault }
    }
}

impl<C> BanditStorage for AsterBanditStorage<'_, C>
where
    C: Clock,
{
    fn load(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.vault
            .read_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealBandit, key)
            .map_err(|error| cf_unavailable("read anneal_bandit CF", error))
    }

    fn save(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.vault
            .write_cf(ColumnFamily::AnnealBandit, key, value)
            .map(|_| ())
            .map_err(|error| cf_unavailable("write anneal_bandit CF", error))
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::AnnealBandit)
            .map_err(|error| cf_unavailable("scan anneal_bandit CF", error))
    }
}

impl ConfigBandit {
    pub fn new(policy: BanditPolicy, rng_seed: u64) -> Self {
        Self {
            policy,
            arms: Vec::new(),
            incumbent_idx: 0,
            hysteresis_wins: DEFAULT_HYSTERESIS_WINS,
            rng_seed,
        }
    }

    pub fn with_hysteresis(mut self, hysteresis_wins: u32) -> Self {
        self.hysteresis_wins = hysteresis_wins;
        self
    }

    pub fn add_arm(&mut self, config: ConfigVariant) {
        self.arms.push(Arm::new(config));
    }

    pub fn select_arm(&mut self) -> Result<usize> {
        self.validate()?;
        if self.arms.is_empty() {
            return Err(empty_bandit("cannot select from zero arms"));
        }
        if self.arms.len() == 1 {
            return Ok(0);
        }
        match self.policy {
            BanditPolicy::EpsilonGreedy { epsilon } => self.select_epsilon_greedy(epsilon),
            BanditPolicy::Thompson => self.select_thompson(),
        }
    }

    pub fn record_result(&mut self, arm_idx: usize, won: bool) -> Result<()> {
        self.validate()?;
        if arm_idx >= self.arms.len() {
            return Err(invalid_config(format!("arm index {arm_idx} out of range")));
        }
        let arm = &mut self.arms[arm_idx];
        arm.trials = arm
            .trials
            .checked_add(1)
            .ok_or_else(|| invalid_config("bandit trial counter exhausted"))?;
        if won {
            arm.wins = arm
                .wins
                .checked_add(1)
                .ok_or_else(|| invalid_config("bandit win counter exhausted"))?;
            arm.consecutive_wins = arm
                .consecutive_wins
                .checked_add(1)
                .ok_or_else(|| invalid_config("bandit consecutive win counter exhausted"))?;
        } else {
            arm.consecutive_wins = 0;
        }
        if won && (self.hysteresis_wins == 0 || arm.consecutive_wins >= self.hysteresis_wins) {
            self.incumbent_idx = arm_idx;
            for arm in &mut self.arms {
                arm.consecutive_wins = 0;
            }
        }
        Ok(())
    }

    pub fn incumbent(&self) -> Result<&Arm> {
        self.validate()?;
        self.arms
            .get(self.incumbent_idx)
            .ok_or_else(|| empty_bandit("zero arms have no incumbent"))
    }

    pub fn status(&self, shape_key_hash: [u8; 32]) -> Result<BanditStatus> {
        self.validate()?;
        Ok(BanditStatus {
            shape_key_hash,
            policy: self.policy,
            incumbent: (!self.arms.is_empty()).then_some(self.incumbent_idx),
            arm_count: self.arms.len(),
            hysteresis_wins: self.hysteresis_wins,
            rng_seed: self.rng_seed,
            arms: self
                .arms
                .iter()
                .enumerate()
                .map(|(idx, arm)| ArmStatus {
                    idx,
                    config_hash: *blake3::hash(&arm.config).as_bytes(),
                    config_len: arm.config.len(),
                    wins: arm.wins,
                    trials: arm.trials,
                    win_rate: arm.win_rate(),
                    consecutive_wins: arm.consecutive_wins,
                })
                .collect(),
        })
    }

    pub fn validate(&self) -> Result<()> {
        match self.policy {
            BanditPolicy::EpsilonGreedy { epsilon }
                if !epsilon.is_finite() || !(0.0..=1.0).contains(&epsilon) =>
            {
                return Err(invalid_config("epsilon must be finite and within [0, 1]"));
            }
            _ => {}
        }
        if self.arms.is_empty() {
            return Ok(());
        }
        if self.incumbent_idx >= self.arms.len() {
            return Err(invalid_config(format!(
                "incumbent_idx {} out of range for {} arms",
                self.incumbent_idx,
                self.arms.len()
            )));
        }
        for (idx, arm) in self.arms.iter().enumerate() {
            if arm.wins > arm.trials {
                return Err(invalid_config(format!(
                    "arm {idx} has wins {} > trials {}",
                    arm.wins, arm.trials
                )));
            }
        }
        Ok(())
    }

    fn select_epsilon_greedy(&mut self, epsilon: f64) -> Result<usize> {
        if epsilon == 0.0 {
            return Ok(self.best_win_rate_index());
        }
        let mut rng = ChaCha8Rng::seed_from_u64(self.rng_seed);
        let idx = if rng.gen_range(0.0..1.0) < epsilon {
            rng.gen_range(0..self.arms.len())
        } else {
            self.best_win_rate_index()
        };
        self.rng_seed = rng.next_u64();
        Ok(idx)
    }

    fn select_thompson(&mut self) -> Result<usize> {
        let mut rng = ChaCha8Rng::seed_from_u64(self.rng_seed);
        let mut best_idx = 0;
        let mut best_score = f64::NEG_INFINITY;
        for (idx, arm) in self.arms.iter().enumerate() {
            let losses = arm.trials - arm.wins;
            let score = sample_beta(f64::from(arm.wins + 1), f64::from(losses + 1), &mut rng);
            if score > best_score {
                best_score = score;
                best_idx = idx;
            }
        }
        self.rng_seed = rng.next_u64();
        Ok(best_idx)
    }

    fn best_win_rate_index(&self) -> usize {
        let mut best_idx = self.incumbent_idx;
        let mut best_rate = self.arms[best_idx].win_rate();
        for (idx, arm) in self.arms.iter().enumerate() {
            let rate = arm.win_rate();
            if rate > best_rate {
                best_rate = rate;
                best_idx = idx;
            }
        }
        best_idx
    }
}

pub struct ConfigBanditStore<S> {
    storage: S,
}

impl<S> ConfigBanditStore<S>
where
    S: BanditStorage,
{
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    pub fn load(&self, shape_key_hash: [u8; 32]) -> Result<Option<ConfigBandit>> {
        self.storage
            .load(&bandit_key(shape_key_hash))?
            .map(|value| decode_config_bandit(&value))
            .transpose()
    }

    pub fn save(&self, shape_key_hash: [u8; 32], bandit: &ConfigBandit) -> Result<()> {
        self.storage
            .save(bandit_key(shape_key_hash), encode_config_bandit(bandit)?)
    }

    pub fn readback(&self, shape_key_hash: [u8; 32]) -> Result<Option<BanditReadback>> {
        let key = bandit_key(shape_key_hash);
        self.storage
            .load(&key)?
            .map(|value| {
                let bandit = decode_config_bandit(&value)?;
                Ok(BanditReadback {
                    shape_key_hash,
                    key,
                    value,
                    bandit,
                })
            })
            .transpose()
    }
}

pub fn shape_key_hash(shape_key: &str) -> [u8; 32] {
    *blake3::hash(shape_key.as_bytes()).as_bytes()
}

pub fn bandit_key(shape_key_hash: [u8; 32]) -> Vec<u8> {
    let mut key = Vec::with_capacity(BANDIT_KEY_PREFIX.len() + shape_key_hash.len());
    key.extend_from_slice(BANDIT_KEY_PREFIX);
    key.extend_from_slice(&shape_key_hash);
    key
}

pub fn encode_config_bandit(bandit: &ConfigBandit) -> Result<Vec<u8>> {
    bandit.validate()?;
    let row = BanditRow {
        tag: BANDIT_ROW_TAG.to_string(),
        bandit: bandit.clone(),
    };
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&row, &mut bytes).map_err(|error| invalid_row(error.to_string()))?;
    Ok(bytes)
}

pub fn decode_config_bandit(bytes: &[u8]) -> Result<ConfigBandit> {
    let row: BanditRow =
        ciborium::de::from_reader(bytes).map_err(|error| invalid_row(error.to_string()))?;
    if row.tag != BANDIT_ROW_TAG {
        return Err(invalid_row(format!(
            "unexpected bandit row tag {}",
            row.tag
        )));
    }
    row.bandit.validate()?;
    Ok(row.bandit)
}

fn sample_beta(alpha: f64, beta: f64, rng: &mut ChaCha8Rng) -> f64 {
    let left = sample_gamma(alpha, rng);
    let right = sample_gamma(beta, rng);
    left / (left + right)
}

fn sample_gamma(shape: f64, rng: &mut ChaCha8Rng) -> f64 {
    if shape == 1.0 {
        return -rng.gen_range(f64::MIN_POSITIVE..1.0).ln();
    }
    let d = shape - (1.0 / 3.0);
    let c = (1.0 / (9.0 * d)).sqrt();
    loop {
        let x = standard_normal(rng);
        let v = 1.0 + c * x;
        if v <= 0.0 {
            continue;
        }
        let v3 = v * v * v;
        let u = rng.gen_range(0.0..1.0);
        if u < 1.0 - 0.0331 * x.powi(4) {
            return d * v3;
        }
        if u.ln() < 0.5 * x * x + d * (1.0 - v3 + v3.ln()) {
            return d * v3;
        }
    }
}

fn standard_normal(rng: &mut ChaCha8Rng) -> f64 {
    let u1 = rng.gen_range(f64::MIN_POSITIVE..1.0);
    let u2 = rng.gen_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn empty_bandit(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_BANDIT_EMPTY,
        message: message.into(),
        remediation: "add at least one config arm before selecting or reading the incumbent",
    }
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_BANDIT_INVALID_CONFIG,
        message: message.into(),
        remediation: "repair the ConfigBandit policy, arms, or incumbent index",
    }
}

fn invalid_row(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_BANDIT_INVALID_ROW,
        message: message.into(),
        remediation: "repair or quarantine anneal_bandit CF rows before autotuning",
    }
}

fn cf_unavailable(context: &'static str, error: CalyxError) -> CalyxError {
    CalyxError {
        code: CALYX_ASTER_CF_UNAVAILABLE,
        message: format!("{context}: {}: {}", error.code, error.message),
        remediation: "restore Aster anneal_bandit CF availability",
    }
}
