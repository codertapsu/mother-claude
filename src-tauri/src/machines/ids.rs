//! Shapes of the ids Machines accepts from a request or another machine.

/// A machine id: the per-install id, 32 lowercase hex characters.
pub fn is_machine_id(s: &str) -> bool {
    crate::claude::backup::is_machine_id(s)
}

/// A machine key as the transport names it: 64 lowercase hex characters.
pub fn is_key(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A short, readable fingerprint of a key, as both screens show it:
/// `3F9A C1D2 77E0 9B41`.
pub fn fingerprint(key: &str) -> String {
    key.chars()
        .take(16)
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|c| c.iter().collect::<String>().to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_64_lowercase_hex() {
        assert!(is_key(&"a".repeat(64)));
        assert!(!is_key(&"A".repeat(64)));
        assert!(!is_key(&"a".repeat(63)));
        assert!(!is_key(&format!("{}g", "a".repeat(63))));
    }

    #[test]
    fn a_fingerprint_is_four_groups_of_four() {
        assert_eq!(
            fingerprint("3f9ac1d277e09b41ffffffffffffffffffffffffffffffffffffffffffffffff"),
            "3F9A C1D2 77E0 9B41"
        );
    }
}
