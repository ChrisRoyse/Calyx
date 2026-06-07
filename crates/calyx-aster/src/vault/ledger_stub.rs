pub fn encode(_seq: u64) -> Vec<u8> {
    vec![0_u8; 32]
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn ph35_placeholder_is_fixed_width_zero_hash() {
        assert_eq!(encode(42), vec![0_u8; 32]);
    }
}
