use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

use crate::{config::PasswordConfig, error::AppError};

/// Hash a password with argon2id. Cost params come from config so a small box
/// can dial the memory cost down without a code change.
pub fn hash(config: &PasswordConfig, password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(hasher(config)?
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

/// Verify a password against a stored hash. Wrong password is `Ok(false)`,
/// a malformed hash is an error.
pub fn verify(config: &PasswordConfig, password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed = PasswordHash::new(hash)?;
    Ok(hasher(config)?
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

fn hasher(config: &PasswordConfig) -> Result<Argon2<'static>, AppError> {
    let params = Params::new(
        config.memory_kib,
        config.time_cost,
        config.parallelism,
        None,
    )
    .map_err(|e| AppError::internal(format!("invalid argon2 params: {e}")))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PasswordConfig {
        PasswordConfig {
            memory_kib: 8 * 1024,
            time_cost: 1,
            parallelism: 1,
        }
    }

    #[test]
    fn roundtrip() {
        let h = hash(&cfg(), "correct horse battery staple").unwrap();
        assert!(verify(&cfg(), "correct horse battery staple", &h).unwrap());
        assert!(!verify(&cfg(), "wrong", &h).unwrap());
    }
}
