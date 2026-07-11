use std::collections::BTreeSet;

use calyx_assay::{PcSeries, PcStableReport, pc_stable_gaussian};

pub struct Fixture {
    i: Vec<f32>,
    a: Vec<f32>,
    w: Vec<f32>,
    j: Vec<f32>,
}

pub fn asymmetric_separator_fixture(n: usize) -> Fixture {
    assert_eq!(n % 16, 0);
    let mut i = Vec::with_capacity(n);
    let mut a = Vec::with_capacity(n);
    let mut w = Vec::with_capacity(n);
    let mut j = Vec::with_capacity(n);
    for row in 0..n {
        let i_value = walsh(row, 0b0001);
        let w_value = walsh(row, 0b0010);
        let a_noise = walsh(row, 0b0100);
        let j_noise = walsh(row, 0b1000);
        let a_value = i_value + w_value + 0.3 * a_noise;
        let j_value = a_value + w_value + j_noise;
        i.push(i_value);
        a.push(a_value);
        w.push(w_value);
        j.push(j_value);
    }
    Fixture { i, a, w, j }
}

pub fn discover(data: &Fixture, order: &[&str], max_conditioning: usize) -> PcStableReport {
    let series: Vec<PcSeries<'_>> = order
        .iter()
        .map(|name| PcSeries {
            name,
            values: values(data, name),
        })
        .collect();
    pc_stable_gaussian(&series, 0.01, max_conditioning).unwrap()
}

pub fn has_edge(report: &PcStableReport, left: &str, right: &str) -> bool {
    report
        .retained_edges
        .iter()
        .any(|edge| same_pair(&edge.left, &edge.right, left, right))
}

pub fn canonical_edges(report: &PcStableReport) -> BTreeSet<(String, String)> {
    report
        .retained_edges
        .iter()
        .map(|edge| ordered_pair(&edge.left, &edge.right))
        .collect()
}

pub fn separating_set(
    report: &PcStableReport,
    left: &str,
    right: &str,
) -> Option<BTreeSet<String>> {
    report
        .removed_edges
        .iter()
        .find(|edge| same_pair(&edge.left, &edge.right, left, right))
        .map(|edge| edge.conditioning_set.iter().cloned().collect())
}

pub fn name_set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

pub fn values<'a>(data: &'a Fixture, name: &str) -> &'a [f32] {
    match name {
        "i" => &data.i,
        "a" => &data.a,
        "w" => &data.w,
        "j" => &data.j,
        unexpected => panic!("unexpected variable {unexpected}"),
    }
}

fn walsh(row: usize, mask: usize) -> f32 {
    if (row & mask).count_ones().is_multiple_of(2) {
        -1.0
    } else {
        1.0
    }
}

fn ordered_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    }
}

fn same_pair(a: &str, b: &str, left: &str, right: &str) -> bool {
    ordered_pair(a, b) == ordered_pair(left, right)
}
