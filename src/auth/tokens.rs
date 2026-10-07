use rand::RngCore;
use sha2::{Digest, Sha256};

/// A fresh 32-byte random token, hex-encoded. This is what the client receives.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// A 6-digit numeric code for emails (verification, reset). Short enough to type, and
/// still hashed at rest — see [hash_token] — so a DB leak doesn't hand out live codes.
pub fn random_code() -> String {
    format!("{:06}", rand::thread_rng().next_u32() % 1_000_000)
}

/// sha256 hex of a token. This is what we store — a DB leak doesn't hand out
/// usable sessions.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_and_stable() {
        let t = random_token();
        assert_eq!(t.len(), 64);
        assert_eq!(hash_token(&t), hash_token(&t));
        assert_ne!(hash_token(&t), t);
    }

    #[test]
    fn code_is_six_digits() {
        for _ in 0..100 {
            let c = random_code();
            assert_eq!(c.len(), 6);
            assert!(c.chars().all(|ch| ch.is_ascii_digit()));
        }
    }
}
