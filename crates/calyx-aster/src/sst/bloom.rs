//! Small deterministic Bloom filter for SST point-lookups.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BloomFilter {
    bit_count: u64,
    hash_count: u32,
    bits: Vec<u8>,
}

impl BloomFilter {
    pub fn from_keys<'a>(keys: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let keys: Vec<_> = keys.into_iter().collect();
        let bit_count = ((keys.len().max(1) * 16).next_power_of_two() as u64).max(64);
        let hash_count = 3;
        let mut filter = Self {
            bit_count,
            hash_count,
            bits: vec![0; bit_count.div_ceil(8) as usize],
        };
        for key in keys {
            filter.insert(key);
        }
        filter
    }

    pub fn may_contain(&self, key: &[u8]) -> bool {
        (0..self.hash_count).all(|round| self.bit_is_set(self.bit_index(key, round)))
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.bit_count.to_le_bytes());
        out.extend_from_slice(&self.hash_count.to_le_bytes());
        out.extend_from_slice(&(self.bits.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.bits);
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 16 {
            return None;
        }
        let bit_count = u64::from_le_bytes(bytes[0..8].try_into().ok()?);
        let hash_count = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
        let byte_len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
        let bits = bytes.get(16..16 + byte_len)?.to_vec();
        if bit_count == 0 || hash_count == 0 {
            return None;
        }
        if bits.len() != bit_count.div_ceil(8) as usize {
            return None;
        }
        Some(Self {
            bit_count,
            hash_count,
            bits,
        })
    }

    fn insert(&mut self, key: &[u8]) {
        for round in 0..self.hash_count {
            let index = self.bit_index(key, round);
            self.set_bit(index);
        }
    }

    fn bit_index(&self, key: &[u8], round: u32) -> u64 {
        let mut input = Vec::with_capacity(key.len() + 4);
        input.extend_from_slice(key);
        input.extend_from_slice(&round.to_le_bytes());
        let hash = blake3::hash(&input);
        u64::from_le_bytes(hash.as_bytes()[0..8].try_into().expect("hash width")) % self.bit_count
    }

    fn set_bit(&mut self, index: u64) {
        let byte = (index / 8) as usize;
        let bit = (index % 8) as u8;
        self.bits[byte] |= 1 << bit;
    }

    fn bit_is_set(&self, index: u64) -> bool {
        let byte = (index / 8) as usize;
        let bit = (index % 8) as u8;
        self.bits[byte] & (1 << bit) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rand::rngs::StdRng;
    use rand::{RngCore, SeedableRng};
    use std::collections::HashSet;

    #[test]
    fn inserted_keys_have_no_false_negatives() {
        let keys = [b"alpha".as_slice(), b"beta".as_slice(), b"gamma".as_slice()];
        let filter = BloomFilter::from_keys(keys);

        for key in keys {
            assert!(filter.may_contain(key));
        }
        assert!(BloomFilter::from_keys(std::iter::empty()).bit_count >= 64);
        assert!(BloomFilter::decode(b"").is_none());
    }

    #[test]
    fn seeded_false_positive_rate_stays_below_one_percent() {
        let mut rng = StdRng::seed_from_u64(0xDEAD_BEEF);
        let mut inserted = HashSet::new();
        while inserted.len() < 10_000 {
            inserted.insert(next_key(&mut rng));
        }
        let inserted_vec: Vec<_> = inserted.iter().map(Vec::as_slice).collect();
        let filter = BloomFilter::from_keys(inserted_vec);
        let mut false_positives = 0_u64;
        let mut probes = 0_u64;
        while probes < 100_000 {
            let key = next_key(&mut rng);
            if inserted.contains(&key) {
                continue;
            }
            probes += 1;
            if filter.may_contain(&key) {
                false_positives += 1;
            }
        }
        let rate = false_positives as f64 / probes as f64;
        println!("bloom fpr = {false_positives} / {probes} = {rate:.6}");
        assert!(rate < 0.01);
    }

    proptest! {
        #[test]
        fn no_false_negatives_for_distinct_key_sets(mut keys in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..32), 0..256)) {
            keys.sort();
            keys.dedup();
            let refs: Vec<_> = keys.iter().map(Vec::as_slice).collect();
            let filter = BloomFilter::from_keys(refs.clone());
            for key in refs {
                prop_assert!(filter.may_contain(key));
            }
        }

        #[test]
        fn encode_decode_preserves_inserted_keys(mut keys in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 1..32), 0..128)) {
            keys.sort();
            keys.dedup();
            let refs: Vec<_> = keys.iter().map(Vec::as_slice).collect();
            let filter = BloomFilter::from_keys(refs.clone());
            let mut bytes = Vec::new();
            filter.encode(&mut bytes);
            let decoded = BloomFilter::decode(&bytes).expect("decode bloom");
            for key in refs {
                prop_assert!(decoded.may_contain(key));
            }
        }
    }

    fn next_key(rng: &mut StdRng) -> Vec<u8> {
        let mut key = vec![0_u8; 16];
        rng.fill_bytes(&mut key);
        key
    }
}
