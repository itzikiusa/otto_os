//! argon2id password hashing + the shared minimum-password policy.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use otto_core::{Error, Result};

/// Minimum password length enforced everywhere a password is set (onboarding,
/// user create, user password change). Single source of truth so the rules stay
/// in sync.
pub const MIN_PASSWORD_LEN: usize = 10;

/// Validate a password against the shared policy. Returns `Error::Invalid` with
/// a user-facing message when it does not meet the minimum length.
pub fn validate_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(Error::Invalid(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

/// Hash a password with argon2id (default params) and a fresh random salt.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| Error::Internal(format!("hash password: {e}")))
}

/// Verify a password against a stored PHC-format hash.
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed =
        PasswordHash::new(hash).map_err(|e| Error::Internal(format!("bad password hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let h = hash_password("correct horse battery").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse battery", &h).unwrap());
        assert!(!verify_password("wrong", &h).unwrap());
    }

    #[test]
    fn password_policy() {
        assert!(validate_password("short").is_err());
        // Exactly MIN_PASSWORD_LEN chars passes; one fewer fails.
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LEN)).is_ok());
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LEN - 1)).is_err());
    }
}
