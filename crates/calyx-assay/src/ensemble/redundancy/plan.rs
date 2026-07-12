use calyx_core::{CalyxError, Result};

pub const DEFAULT_LINEAR_CKA_SEED: u64 = 0xCA1A_CAFE_4C4B_4131;
pub const MIN_LINEAR_CKA_TUPLES: usize = 4_096;
pub const MAX_LINEAR_CKA_TUPLES: usize = 160_000;
pub const LINEAR_CKA_TUPLES_PER_ROW: usize = 16;
pub const LINEAR_CKA_JACKKNIFE_BLOCKS: usize = 32;

#[derive(Clone, Debug)]
pub struct LinearCkaTuplePlan {
    pub(super) row_count: usize,
    pub(super) seed: u64,
    pub(super) exact: bool,
    pub(super) tuples: Vec<[usize; 4]>,
    pub(super) digest: [u8; 32],
}

impl LinearCkaTuplePlan {
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    pub fn tuple_count(&self) -> usize {
        self.tuples.len()
    }

    pub fn is_exact(&self) -> bool {
        self.exact
    }

    pub(super) fn seed(&self) -> u64 {
        self.seed
    }

    pub(super) fn digest_hex(&self) -> String {
        blake3::Hash::from_bytes(self.digest).to_hex().to_string()
    }

    #[cfg(test)]
    pub(super) fn tuples(&self) -> &[[usize; 4]] {
        &self.tuples
    }
}

pub fn linear_cka_tuple_plan(row_count: usize) -> Result<LinearCkaTuplePlan> {
    if row_count < 4 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "linear CKA requires at least four rows; got {row_count}"
        )));
    }
    let budget = row_count
        .saturating_mul(LINEAR_CKA_TUPLES_PER_ROW)
        .clamp(MIN_LINEAR_CKA_TUPLES, MAX_LINEAR_CKA_TUPLES);
    let exact = choose_four(row_count) <= budget as u128;
    let tuples = if exact {
        exact_tuples(row_count)
    } else {
        sampled_tuples(row_count, budget, DEFAULT_LINEAR_CKA_SEED)
    };
    let digest = tuple_plan_digest(row_count, DEFAULT_LINEAR_CKA_SEED, exact, &tuples);
    Ok(LinearCkaTuplePlan {
        row_count,
        seed: DEFAULT_LINEAR_CKA_SEED,
        exact,
        tuples,
        digest,
    })
}

fn choose_four(row_count: usize) -> u128 {
    let n = row_count as u128;
    n.checked_mul(n.saturating_sub(1))
        .and_then(|value| value.checked_mul(n.saturating_sub(2)))
        .and_then(|value| value.checked_mul(n.saturating_sub(3)))
        .map(|value| value / 24)
        .unwrap_or(u128::MAX)
}

fn exact_tuples(row_count: usize) -> Vec<[usize; 4]> {
    let mut tuples = Vec::with_capacity(choose_four(row_count) as usize);
    for a in 0..row_count - 3 {
        for b in (a + 1)..row_count - 2 {
            for c in (b + 1)..row_count - 1 {
                for d in (c + 1)..row_count {
                    tuples.push([a, b, c, d]);
                }
            }
        }
    }
    tuples
}

fn sampled_tuples(row_count: usize, count: usize, seed: u64) -> Vec<[usize; 4]> {
    let mut sampler = Blake3Counter::new(seed, row_count);
    let mut tuples = Vec::with_capacity(count);
    for _ in 0..count {
        let mut tuple = [usize::MAX; 4];
        for position in 0..4 {
            loop {
                let candidate = sampler.index(row_count);
                if !tuple[..position].contains(&candidate) {
                    tuple[position] = candidate;
                    break;
                }
            }
        }
        tuple.sort_unstable();
        tuples.push(tuple);
    }
    tuples
}

struct Blake3Counter {
    seed: u64,
    row_count: usize,
    counter: u64,
}

impl Blake3Counter {
    fn new(seed: u64, row_count: usize) -> Self {
        Self {
            seed,
            row_count,
            counter: 0,
        }
    }

    fn index(&mut self, bound: usize) -> usize {
        let bound = bound as u64;
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next_u64();
            if value >= threshold {
                return (value % bound) as usize;
            }
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"calyx-linear-cka-u4-counter-v1");
        hasher.update(&self.seed.to_le_bytes());
        hasher.update(&(self.row_count as u128).to_le_bytes());
        hasher.update(&self.counter.to_le_bytes());
        self.counter = self.counter.wrapping_add(1);
        let bytes = hasher.finalize();
        let mut value = [0_u8; 8];
        value.copy_from_slice(&bytes.as_bytes()[..8]);
        u64::from_le_bytes(value)
    }
}

fn tuple_plan_digest(row_count: usize, seed: u64, exact: bool, tuples: &[[usize; 4]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-linear-cka-u4-plan-v1");
    hasher.update(&(row_count as u128).to_le_bytes());
    hasher.update(&seed.to_le_bytes());
    hasher.update(&[u8::from(exact)]);
    for tuple in tuples {
        for index in tuple {
            hasher.update(&(*index as u128).to_le_bytes());
        }
    }
    *hasher.finalize().as_bytes()
}
