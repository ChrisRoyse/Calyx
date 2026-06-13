//! Conjugate Bayesian posteriors for recurrence rates and oracle consistency.
//!
//! These posteriors make the small-sample uncertainty explicit: recurrence
//! frequency is a Gamma-Poisson rate, and consistency/flakiness is a
//! Beta-Bernoulli success probability.

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, CalyxError, Clock, Result, Seq};
use serde::{Deserialize, Serialize};

use crate::recurrence_anchor::Domain;
use crate::special_fn::{gammp, ln_gamma};

pub const CALYX_BAYES_INVALID_INTERVAL: &str = "CALYX_BAYES_INVALID_INTERVAL";
pub const DEFAULT_BAYES_PRIOR_ALPHA: f64 = 1.0;
pub const DEFAULT_BAYES_PRIOR_BETA: f64 = 1.0;
pub const BAYESIAN_POSTERIOR_KEY_PREFIX: &[u8] = b"bayesian/posterior/v1";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GammaPoisson {
    pub alpha: f64,
    pub beta: f64,
}

impl Default for GammaPoisson {
    fn default() -> Self {
        Self {
            alpha: DEFAULT_BAYES_PRIOR_ALPHA,
            beta: DEFAULT_BAYES_PRIOR_BETA,
        }
    }
}

impl GammaPoisson {
    pub fn new(prior_alpha: f64, prior_beta: f64) -> Result<Self> {
        let posterior = Self {
            alpha: prior_alpha,
            beta: prior_beta,
        };
        posterior.validate()?;
        Ok(posterior)
    }

    pub fn update(&mut self, events: u64, interval: f64) -> Result<()> {
        if !interval.is_finite() || interval <= 0.0 {
            return Err(invalid_bayes(format!(
                "Gamma-Poisson update interval must be finite and positive, got {interval}"
            )));
        }
        self.alpha += events as f64;
        self.beta += interval;
        self.validate()
    }

    pub fn update_signed(&mut self, events: i64, interval: f64) -> Result<()> {
        if events < 0 {
            return Err(invalid_bayes(format!(
                "Gamma-Poisson events must be non-negative, got {events}"
            )));
        }
        self.update(events as u64, interval)
    }

    pub fn mean_rate(&self) -> f64 {
        self.alpha / self.beta
    }

    pub fn credible_interval_95(&self) -> Result<(f64, f64)> {
        self.credible_interval(0.95)
    }

    pub fn credible_interval(&self, mass: f64) -> Result<(f64, f64)> {
        self.validate()?;
        validate_probability("credible mass", mass)?;
        let tail = (1.0 - mass) / 2.0;
        Ok((
            gamma_rate_quantile(self.alpha, self.beta, tail)?,
            gamma_rate_quantile(self.alpha, self.beta, 1.0 - tail)?,
        ))
    }

    pub fn next_occurrence_expected(&self) -> f64 {
        1.0 / self.mean_rate()
    }

    pub fn validate(&self) -> Result<()> {
        validate_positive("Gamma-Poisson alpha", self.alpha)?;
        validate_positive("Gamma-Poisson beta", self.beta)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BetaBernoulli {
    pub alpha: f64,
    pub beta: f64,
}

impl Default for BetaBernoulli {
    fn default() -> Self {
        Self {
            alpha: DEFAULT_BAYES_PRIOR_ALPHA,
            beta: DEFAULT_BAYES_PRIOR_BETA,
        }
    }
}

impl BetaBernoulli {
    pub fn new(prior_alpha: f64, prior_beta: f64) -> Result<Self> {
        let posterior = Self {
            alpha: prior_alpha,
            beta: prior_beta,
        };
        posterior.validate()?;
        Ok(posterior)
    }

    pub fn update(&mut self, successes: u64, failures: u64) -> Result<()> {
        self.alpha += successes as f64;
        self.beta += failures as f64;
        self.validate()
    }

    pub fn update_signed(&mut self, successes: i64, failures: i64) -> Result<()> {
        if successes < 0 || failures < 0 {
            return Err(invalid_bayes(format!(
                "Beta-Bernoulli counts must be non-negative, got successes={successes}, failures={failures}"
            )));
        }
        self.update(successes as u64, failures as u64)
    }

    pub fn mean_consistency(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    pub fn credible_interval_95(&self) -> Result<(f64, f64)> {
        self.credible_interval(0.95)
    }

    pub fn credible_interval(&self, mass: f64) -> Result<(f64, f64)> {
        self.validate()?;
        validate_probability("credible mass", mass)?;
        let tail = (1.0 - mass) / 2.0;
        Ok((
            beta_quantile(self.alpha, self.beta, tail)?,
            beta_quantile(self.alpha, self.beta, 1.0 - tail)?,
        ))
    }

    pub fn reliability_probability(&self, threshold: f64) -> Result<f64> {
        self.validate()?;
        validate_unit("reliability threshold", threshold)?;
        Ok(1.0 - regularized_beta(self.alpha, self.beta, threshold)?)
    }

    pub fn is_reliable(&self, threshold: f64, confidence: f64) -> Result<bool> {
        validate_unit("reliability confidence", confidence)?;
        Ok(self.reliability_probability(threshold)? >= confidence)
    }

    pub fn validate(&self) -> Result<()> {
        validate_positive("Beta-Bernoulli alpha", self.alpha)?;
        validate_positive("Beta-Bernoulli beta", self.beta)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BayesianPosteriorRow {
    pub domain_id: String,
    pub outcome_anchor: AnchorKind,
    pub gamma_poisson: GammaPoisson,
    pub beta_bernoulli: BetaBernoulli,
    pub written_at_seq: Seq,
}

pub fn bayesian_posterior_key(domain: &Domain) -> Result<Vec<u8>> {
    let mut key = Vec::with_capacity(BAYESIAN_POSTERIOR_KEY_PREFIX.len() + domain.id.len() + 32);
    key.extend_from_slice(BAYESIAN_POSTERIOR_KEY_PREFIX);
    push_len_prefixed(&mut key, domain.id.as_bytes());
    let anchor = serde_json::to_vec(&domain.outcome_anchor)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode anchor: {error}")))?;
    push_len_prefixed(&mut key, &anchor);
    Ok(key)
}

pub fn persist_bayesian_posterior<C>(
    vault: &AsterVault<C>,
    domain: &Domain,
    gamma_poisson: GammaPoisson,
    beta_bernoulli: BetaBernoulli,
) -> Result<Seq>
where
    C: Clock,
{
    gamma_poisson.validate()?;
    beta_bernoulli.validate()?;
    let row = BayesianPosteriorRow {
        domain_id: domain.id.clone(),
        outcome_anchor: domain.outcome_anchor.clone(),
        gamma_poisson,
        beta_bernoulli,
        written_at_seq: vault.latest_seq() + 1,
    };
    let value = serde_json::to_vec(&row)
        .map_err(|error| CalyxError::disk_pressure(format!("encode bayesian row: {error}")))?;
    vault.write_cf(ColumnFamily::Assay, bayesian_posterior_key(domain)?, value)
}

pub fn bayesian_posterior_for_domain<C>(
    vault: &AsterVault<C>,
    domain: &Domain,
) -> Result<Option<BayesianPosteriorRow>>
where
    C: Clock,
{
    let Some(bytes) = vault.read_cf_at(
        vault.latest_seq(),
        ColumnFamily::Assay,
        &bayesian_posterior_key(domain)?,
    )?
    else {
        return Ok(None);
    };
    let row: BayesianPosteriorRow = serde_json::from_slice(&bytes).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("decode bayesian row: {error}"))
    })?;
    row.gamma_poisson.validate()?;
    row.beta_bernoulli.validate()?;
    if row.domain_id != domain.id || row.outcome_anchor != domain.outcome_anchor {
        return Err(CalyxError::aster_corrupt_shard(
            "bayesian posterior CF key does not match row domain",
        ));
    }
    Ok(Some(row))
}

pub fn gamma_poisson_for_domain<C>(vault: &AsterVault<C>, domain: &Domain) -> Result<GammaPoisson>
where
    C: Clock,
{
    Ok(bayesian_posterior_for_domain(vault, domain)?
        .map(|row| row.gamma_poisson)
        .unwrap_or_default())
}

pub fn beta_bernoulli_for_domain<C>(vault: &AsterVault<C>, domain: &Domain) -> Result<BetaBernoulli>
where
    C: Clock,
{
    Ok(bayesian_posterior_for_domain(vault, domain)?
        .map(|row| row.beta_bernoulli)
        .unwrap_or_default())
}

fn gamma_rate_quantile(alpha: f64, beta: f64, p: f64) -> Result<f64> {
    if p <= 0.0 {
        return Ok(0.0);
    }
    if p >= 1.0 {
        return Ok(f64::INFINITY);
    }
    let mut hi = (alpha / beta).max(1.0 / beta);
    let mut guard = 0;
    while gammp(alpha, beta * hi)? < p {
        hi *= 2.0;
        guard += 1;
        if guard > 200 || !hi.is_finite() {
            return Err(invalid_bayes("Gamma-Poisson quantile failed to bracket"));
        }
    }
    let mut lo = 0.0;
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if gammp(alpha, beta * mid)? < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(0.5 * (lo + hi))
}

fn beta_quantile(alpha: f64, beta: f64, p: f64) -> Result<f64> {
    if p <= 0.0 {
        return Ok(0.0);
    }
    if p >= 1.0 {
        return Ok(1.0);
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if regularized_beta(alpha, beta, mid)? < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(0.5 * (lo + hi))
}

fn regularized_beta(a: f64, b: f64, x: f64) -> Result<f64> {
    validate_positive("beta a", a)?;
    validate_positive("beta b", b)?;
    validate_unit("beta x", x)?;
    if x == 0.0 || x == 1.0 {
        return Ok(x);
    }
    let bt = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        Ok((bt * beta_continued_fraction(a, b, x)? / a).clamp(0.0, 1.0))
    } else {
        Ok((1.0 - bt * beta_continued_fraction(b, a, 1.0 - x)? / b).clamp(0.0, 1.0))
    }
}

fn beta_continued_fraction(a: f64, b: f64, x: f64) -> Result<f64> {
    const MAX_ITER: usize = 200;
    const EPS: f64 = 3.0e-14;
    const TINY: f64 = 1.0e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=MAX_ITER {
        let m2 = 2.0 * m as f64;
        let mut aa = m as f64 * (b - m as f64) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = d.recip();
        h *= d * c;
        aa = -(a + m as f64) * (qab + m as f64) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = d.recip();
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < EPS {
            return Ok(h);
        }
    }
    Err(invalid_bayes(
        "regularized beta continued fraction did not converge",
    ))
}

fn validate_probability(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..1.0).contains(&value) {
        return Err(invalid_bayes(format!(
            "{name} must be finite in (0, 1), got {value}"
        )));
    }
    Ok(())
}

fn validate_unit(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(invalid_bayes(format!(
            "{name} must be finite in [0, 1], got {value}"
        )));
    }
    Ok(())
}

fn validate_positive(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        return Err(invalid_bayes(format!(
            "{name} must be finite and positive, got {value}"
        )));
    }
    Ok(())
}

fn push_len_prefixed(key: &mut Vec<u8>, value: &[u8]) {
    key.extend_from_slice(&(value.len() as u32).to_be_bytes());
    key.extend_from_slice(value);
}

fn invalid_bayes(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_BAYES_INVALID_INTERVAL,
        message: message.into(),
        remediation: "use finite positive intervals and non-negative Bayesian counts",
    }
}
