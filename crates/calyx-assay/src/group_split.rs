//! Group-aware held-out splits for Assay estimators.

use std::collections::BTreeMap;

use calyx_core::{CalyxError, Result};
use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSplit {
    pub train: Vec<usize>,
    pub test: Vec<usize>,
}

pub fn group_holdout_split(
    labels: &[bool],
    groups: &[String],
    test_fraction: f32,
    seed: u64,
) -> Result<GroupSplit> {
    if labels.len() != groups.len() || labels.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "group split requires one group per labeled sample",
        ));
    }
    if !test_fraction.is_finite() || !(0.0..1.0).contains(&test_fraction) {
        return Err(unresolved(
            "group split test_fraction must be finite and in (0, 1)",
        ));
    }

    let buckets = group_buckets(labels, groups)?;
    let mut positives = Vec::new();
    let mut negatives = Vec::new();
    for bucket in buckets.into_values() {
        if bucket.label {
            positives.push(bucket.indices);
        } else {
            negatives.push(bucket.indices);
        }
    }
    if positives.len() < 2 || negatives.len() < 2 {
        return Err(CalyxError::assay_insufficient_samples(
            "group split requires at least two anchor groups per class",
        ));
    }

    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    positives.shuffle(&mut rng);
    negatives.shuffle(&mut rng);

    let mut train = Vec::new();
    let mut test = Vec::new();
    split_class(&positives, test_fraction, &mut train, &mut test);
    split_class(&negatives, test_fraction, &mut train, &mut test);
    train.sort_unstable();
    test.sort_unstable();
    validate_split(labels, &train, &test)?;
    Ok(GroupSplit { train, test })
}

pub fn row_groups(len: usize) -> Vec<String> {
    (0..len).map(|idx| format!("row_{idx}")).collect()
}

fn group_buckets(labels: &[bool], groups: &[String]) -> Result<BTreeMap<String, GroupBucket>> {
    let mut buckets: BTreeMap<String, GroupBucket> = BTreeMap::new();
    for (idx, (label, group)) in labels.iter().zip(groups).enumerate() {
        let group = group.trim();
        if group.is_empty() {
            return Err(unresolved("group split received an empty anchor group id"));
        }
        match buckets.get_mut(group) {
            Some(bucket) if bucket.label != *label => {
                return Err(unresolved(format!(
                    "anchor group {group} mixes positive and negative labels"
                )));
            }
            Some(bucket) => bucket.indices.push(idx),
            None => {
                buckets.insert(
                    group.to_string(),
                    GroupBucket {
                        label: *label,
                        indices: vec![idx],
                    },
                );
            }
        }
    }
    Ok(buckets)
}

fn split_class(
    groups: &[Vec<usize>],
    test_fraction: f32,
    train: &mut Vec<usize>,
    test: &mut Vec<usize>,
) {
    let test_groups = ((groups.len() as f32 * test_fraction).round() as usize)
        .clamp(1, groups.len().saturating_sub(1));
    for (idx, group) in groups.iter().enumerate() {
        if idx < test_groups {
            test.extend(group);
        } else {
            train.extend(group);
        }
    }
}

fn validate_split(labels: &[bool], train: &[usize], test: &[usize]) -> Result<()> {
    if train.is_empty() || test.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "group split produced an empty train or test set",
        ));
    }
    let train_classes = class_count(labels, train);
    let test_classes = class_count(labels, test);
    if train_classes != 2 || test_classes != 2 {
        return Err(unresolved(
            "group split must preserve both anchor classes in train and test",
        ));
    }
    Ok(())
}

fn class_count(labels: &[bool], indices: &[usize]) -> usize {
    let mut seen_false = false;
    let mut seen_true = false;
    for &idx in indices {
        if labels[idx] {
            seen_true = true;
        } else {
            seen_false = true;
        }
    }
    usize::from(seen_false) + usize::from(seen_true)
}

fn unresolved(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: crate::contract::CALYX_ASSAY_UNRESOLVED,
        message: message.into(),
        remediation: "re-measure with more grouped anchors or wider held-out evidence",
    }
}

struct GroupBucket {
    label: bool,
    indices: Vec<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_holdout_keeps_anchor_groups_disjoint() {
        let labels = vec![true, true, true, true, false, false, false, false];
        let groups = ["a", "a", "b", "b", "c", "c", "d", "d"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let split = group_holdout_split(&labels, &groups, 0.5, 7).unwrap();
        for left in &split.train {
            for right in &split.test {
                assert_ne!(groups[*left], groups[*right]);
            }
        }
        assert_eq!(class_count(&labels, &split.train), 2);
        assert_eq!(class_count(&labels, &split.test), 2);
    }

    #[test]
    fn mixed_label_group_fails_closed() {
        let labels = vec![true, false, true, false];
        let groups = ["same", "same", "pos", "neg"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let error = group_holdout_split(&labels, &groups, 0.5, 7).unwrap_err();
        assert_eq!(error.code, crate::contract::CALYX_ASSAY_UNRESOLVED);
    }
}
