use rand::{Rng, seq::SliceRandom};

use calyx_core::{CalyxError, Result};

pub(crate) const M_OUT_OF_N_NUMERATOR: usize = 4;
pub(crate) const M_OUT_OF_N_DENOMINATOR: usize = 5;

pub(crate) fn m_out_of_n_size(
    n: usize,
    k: usize,
    minimum: usize,
    estimator: &str,
) -> Result<usize> {
    let m = n.saturating_mul(M_OUT_OF_N_NUMERATOR) / M_OUT_OF_N_DENOMINATOR;
    if m < minimum || k == 0 || k >= m {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "{estimator} no-replacement CI requires a distinct subsample with at least {minimum} rows and 0 < k < m; got n={n}, m={m}, k={k}, fraction={M_OUT_OF_N_NUMERATOR}/{M_OUT_OF_N_DENOMINATOR}"
        )));
    }
    Ok(m)
}

pub(crate) fn sample_without_replacement_indices<R: Rng + ?Sized>(
    n: usize,
    m: usize,
    rng: &mut R,
) -> Result<Vec<usize>> {
    if m == 0 || m > n {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "no-replacement subsample requires 0 < m <= n; got n={n}, m={m}"
        )));
    }

    let mut indices = (0..n).collect::<Vec<_>>();
    indices.shuffle(rng);
    indices.truncate(m);
    if !indices_are_distinct(&indices, n) {
        return Err(CalyxError::assay_insufficient_samples(
            "no-replacement subsample duplicate index invariant violated",
        ));
    }
    Ok(indices)
}

pub(crate) fn sample_paired_values_without_replacement<R: Rng + ?Sized>(
    columns: &[&[f32]],
    m: usize,
    rng: &mut R,
) -> Result<Vec<Vec<f32>>> {
    let n = columns.first().map_or(0, |column| column.len());
    if columns.iter().any(|column| column.len() != n) {
        return Err(CalyxError::assay_insufficient_samples(
            "paired no-replacement subsample requires equal column lengths",
        ));
    }
    let indices = sample_without_replacement_indices(n, m, rng)?;
    Ok(columns
        .iter()
        .map(|column| indices.iter().map(|&index| column[index]).collect())
        .collect())
}

fn indices_are_distinct(indices: &[usize], n: usize) -> bool {
    let mut seen = vec![false; n];
    for &index in indices {
        if index >= n || seen[index] {
            return false;
        }
        seen[index] = true;
    }
    true
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, fs};

    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;
    use serde_json::json;

    use super::{M_OUT_OF_N_DENOMINATOR, M_OUT_OF_N_NUMERATOR, sample_without_replacement_indices};

    const N: usize = 240;
    const M: usize = N * M_OUT_OF_N_NUMERATOR / M_OUT_OF_N_DENOMINATOR;
    const K: usize = 3;

    #[test]
    fn every_subsample_has_exactly_m_distinct_in_range_indices() {
        for seed in 0..256 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let indices = sample_without_replacement_indices(N, M, &mut rng).unwrap();
            let unique = indices.iter().copied().collect::<HashSet<_>>();
            assert_eq!(indices.len(), M);
            assert_eq!(unique.len(), M);
            assert!(indices.iter().all(|&index| index < N));
        }
    }

    #[test]
    fn invalid_subsample_sizes_fail_closed() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        for m in [0, N + 1] {
            let error = sample_without_replacement_indices(N, m, &mut rng).unwrap_err();
            assert_eq!(error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
        }
    }

    #[test]
    fn old_with_replacement_path_reproduces_zero_radius_pathology() {
        let old = with_replacement_reference(0x1312);
        let old_unique = old.iter().copied().collect::<HashSet<_>>().len();
        let max_multiplicity = (0..N)
            .map(|index| old.iter().filter(|&&value| value == index).count())
            .max()
            .unwrap();
        assert!(
            old_unique < M,
            "old path unexpectedly had no duplicate rows"
        );
        assert!(
            max_multiplicity > K,
            "old path did not reproduce a zero kth-neighbor radius"
        );

        let mut rng = ChaCha8Rng::seed_from_u64(0x1312);
        let corrected = sample_without_replacement_indices(N, M, &mut rng).unwrap();
        assert_eq!(corrected.iter().copied().collect::<HashSet<_>>().len(), M);
    }

    #[test]
    #[ignore = "manual FSV writes deterministic subset uniqueness evidence"]
    fn no_replacement_manual_fsv() {
        let root = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
            std::env::temp_dir().join("calyx-issue1312-subsample-fsv")
        });
        fs::create_dir_all(&root).unwrap();

        let subsets = (0..8)
            .map(|seed| {
                let mut rng = ChaCha8Rng::seed_from_u64(seed);
                let indices = sample_without_replacement_indices(N, M, &mut rng).unwrap();
                let unique = indices.iter().copied().collect::<HashSet<_>>().len();
                json!({"seed": seed, "rows": indices.len(), "unique_rows": unique})
            })
            .collect::<Vec<_>>();
        let old = with_replacement_reference(0x1312);
        let old_unique = old.iter().copied().collect::<HashSet<_>>().len();
        let old_max_multiplicity = (0..N)
            .map(|index| old.iter().filter(|&&value| value == index).count())
            .max()
            .unwrap();
        let report = json!({
            "source_of_truth": "canonical calyx-assay subsample helper output",
            "n": N,
            "m": M,
            "k": K,
            "old_with_replacement": {
                "rows": old.len(),
                "unique_rows": old_unique,
                "duplicate_rows": old.len() - old_unique,
                "max_multiplicity": old_max_multiplicity,
                "zero_kth_radius_present": old_max_multiplicity > K,
            },
            "corrected_without_replacement": subsets,
        });
        let path = root.join("issue1312-subsample-readback.json");
        fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("ISSUE1312_SUBSAMPLE_READBACK={}", path.display());
    }

    fn with_replacement_reference(seed: u64) -> Vec<usize> {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        (0..M).map(|_| rng.random_range(0..N)).collect()
    }
}
