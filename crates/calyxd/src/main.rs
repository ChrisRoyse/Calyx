//! Calyx daemon entry point skeleton.

fn main() {
    println!("calyxd skeleton");
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyxd");
    }
}
