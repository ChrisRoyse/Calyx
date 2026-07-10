pub(crate) fn encode_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::encode_component;

    #[test]
    fn issue1394_encodes_query_and_path_delimiters_as_utf8_bytes() {
        assert_eq!(encode_component("safe-._~09AZaz"), "safe-._~09AZaz");
        assert_eq!(
            encode_component("token&side=SELL /?#"),
            "token%26side%3DSELL%20%2F%3F%23"
        );
        assert_eq!(encode_component("cafe\u{301}"), "cafe%CC%81");
    }
}
