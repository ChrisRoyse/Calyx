use calyx_anneal::{
    BanditPolicy, CALYX_ANNEAL_BANDIT_EMPTY, CALYX_ANNEAL_BANDIT_INVALID_CONFIG, ConfigBandit,
};
use proptest::prelude::*;

#[test]
fn epsilon_greedy_exploits_best_rate_and_explores_all_arms() {
    let mut exploit = ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 7);
    exploit.add_arm(vec![0]);
    exploit.add_arm(vec![1]);
    exploit.record_result(1, true).unwrap();
    exploit.record_result(1, true).unwrap();
    exploit.record_result(0, false).unwrap();
    for _ in 0..8 {
        assert_eq!(exploit.select_arm().unwrap(), 1);
    }

    let mut explore = ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 1.0 }, 42);
    for config in [0, 1, 2] {
        explore.add_arm(vec![config]);
    }
    let mut counts = [0usize; 3];
    for _ in 0..300 {
        counts[explore.select_arm().unwrap()] += 1;
    }
    assert!(counts.iter().all(|count| (70..=130).contains(count)));
}

#[test]
fn hysteresis_promotes_only_after_required_consecutive_wins() {
    let mut bandit =
        ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 11).with_hysteresis(3);
    bandit.add_arm(b"incumbent".to_vec());
    bandit.add_arm(b"candidate".to_vec());
    bandit.record_result(1, true).unwrap();
    bandit.record_result(1, true).unwrap();
    assert_eq!(bandit.incumbent_idx, 0);
    bandit.record_result(1, true).unwrap();
    assert_eq!(bandit.incumbent_idx, 1);
    assert!(bandit.arms.iter().all(|arm| arm.consecutive_wins == 0));
}

#[test]
fn thompson_sampling_is_reproducible_by_seed() {
    let mut left = uniform_thompson(42);
    let mut right = uniform_thompson(42);
    let left_sequence: Vec<_> = (0..10).map(|_| left.select_arm().unwrap()).collect();
    let right_sequence: Vec<_> = (0..10).map(|_| right.select_arm().unwrap()).collect();
    assert_eq!(left_sequence, right_sequence);
}

#[test]
fn edge_cases_fail_closed_or_select_expected_arm() {
    let mut empty = ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 1);
    let err = empty.select_arm().unwrap_err();
    assert_eq!(err.code, CALYX_ANNEAL_BANDIT_EMPTY);

    let mut single = ConfigBandit::new(BanditPolicy::Thompson, 2);
    single.add_arm(vec![7]);
    assert_eq!(single.select_arm().unwrap(), 0);

    let mut no_hysteresis =
        ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 3).with_hysteresis(0);
    no_hysteresis.add_arm(vec![0]);
    no_hysteresis.add_arm(vec![1]);
    no_hysteresis.record_result(1, true).unwrap();
    assert_eq!(no_hysteresis.incumbent_idx, 1);

    let err = no_hysteresis.record_result(99, true).unwrap_err();
    assert_eq!(err.code, CALYX_ANNEAL_BANDIT_INVALID_CONFIG);
}

proptest! {
    #[test]
    fn incumbent_index_stays_valid(
        results in proptest::collection::vec((0usize..4, any::<bool>()), 0..80)
    ) {
        let mut bandit = ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.2 }, 123);
        for arm in 0..4 {
            bandit.add_arm(vec![arm as u8]);
        }
        for (arm_idx, won) in results {
            bandit.record_result(arm_idx, won).unwrap();
            prop_assert!(bandit.incumbent_idx < bandit.arms.len());
        }
    }
}

fn uniform_thompson(seed: u64) -> ConfigBandit {
    let mut bandit = ConfigBandit::new(BanditPolicy::Thompson, seed);
    for config in [0, 1, 2, 3] {
        bandit.add_arm(vec![config]);
    }
    bandit
}
