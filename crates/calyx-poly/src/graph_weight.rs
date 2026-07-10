pub(crate) fn canonical_positive_weight(value: f32) -> Option<f32> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let canonical = (value * 1_000_000.0).round() / 1_000_000.0;
    (canonical.is_finite() && canonical > 0.0).then_some(canonical)
}

#[cfg(test)]
mod tests {
    use super::canonical_positive_weight;

    #[test]
    fn issue1394_invalid_graph_weights_are_not_fabricated() {
        assert_eq!(canonical_positive_weight(0.75), Some(0.75));
        assert_eq!(canonical_positive_weight(0.0), None);
        assert_eq!(canonical_positive_weight(-0.1), None);
        assert_eq!(canonical_positive_weight(f32::NAN), None);
        assert_eq!(canonical_positive_weight(f32::MAX), None);
        assert_eq!(canonical_positive_weight(0.000_000_1), None);
    }
}
