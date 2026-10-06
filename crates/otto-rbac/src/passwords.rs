//! argon2id password hashing + the shared minimum-password policy.

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
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

/// Hash a password with argon2id (default params) and a fresh random salt
/// (drawn from the OS RNG by `password-hash`).
pub fn hash_password(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes())
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

/// Concurrent argon2 verifies allowed off the async workers (S8-303). Each one
/// holds ~19 MiB and a blocking thread for tens of ms; pre-auth endpoints
/// (login, share OTP) must not let a flood pin every thread or spike memory.
pub const MAX_CONCURRENT_VERIFIES: usize = 4;
/// How long a verify waits for a permit before the caller is told to back off.
const VERIFY_PERMIT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Every permit is busy (and stayed busy for [`VERIFY_PERMIT_WAIT`]): the
/// caller should answer 503/429 instead of queueing unboundedly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifySaturated;

fn verify_permits() -> &'static tokio::sync::Semaphore {
    static PERMITS: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    PERMITS.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_VERIFIES))
}

/// [`verify_password`] on a blocking thread, behind a process-wide bound of
/// [`MAX_CONCURRENT_VERIFIES`] (S8-303): argon2 never runs on an async worker,
/// and a flood of guesses queues briefly then gets [`VerifySaturated`] rather
/// than stalling terminals, events and the desktop UI.
pub async fn verify_password_bounded(
    password: &str,
    hash: &str,
) -> std::result::Result<Result<bool>, VerifySaturated> {
    let permit = tokio::time::timeout(VERIFY_PERMIT_WAIT, verify_permits().acquire())
        .await
        .map_err(|_| VerifySaturated)?
        .map_err(|_| VerifySaturated)?;
    let (password, hash) = (password.to_string(), hash.to_string());
    let out = tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .unwrap_or_else(|e| Err(Error::Internal(format!("verify password task: {e}"))));
    drop(permit);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Random test password (never a hard-coded literal, so nothing in this
    /// file can be mistaken for a real credential by scanners or readers).
    fn random_pw() -> String {
        hex::encode(rand::random::<[u8; 16]>())
    }

    #[test]
    fn roundtrip() {
        let pw = random_pw();
        let h = hash_password(&pw).unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password(&pw, &h).unwrap());
        assert!(!verify_password(&random_pw(), &h).unwrap());
    }

    #[test]
    fn password_policy() {
        // Exactly MIN_PASSWORD_LEN chars passes; one fewer fails.
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LEN)).is_ok());
        assert!(validate_password(&"a".repeat(MIN_PASSWORD_LEN - 1)).is_err());
    }

    #[tokio::test]
    async fn bounded_verify_runs_off_thread_and_matches() {
        let pw = random_pw();
        let h = hash_password(&pw).unwrap();
        assert!(matches!(
            verify_password_bounded(&pw, &h).await,
            Ok(Ok(true))
        ));
        assert!(matches!(
            verify_password_bounded(&random_pw(), &h).await,
            Ok(Ok(false))
        ));
    }
}
