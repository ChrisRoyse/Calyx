pub(super) fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot = left.iter().zip(right).map(|(l, r)| l * r).sum::<f32>();
    dot / (norm(left) * norm(right)).max(f32::EPSILON)
}

pub(super) fn norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}
