//! SSH private-key file permission check.
//!
//! OpenSSH refuses a private key that is readable by group or other
//! ("Permissions 0644 for '…' are too open … UNPROTECTED PRIVATE KEY FILE …
//! This private key will be ignored."). We surface that *before* the user hits
//! the cryptic ssh error, with the exact `chmod 600 <path>` fix.

/// Pure check: given a key path and its Unix file mode, return a warning when
/// the file is group/other-accessible (any of the `0o077` bits set), else
/// `None`. Split out from the filesystem so it can be unit-tested directly.
pub(crate) fn key_perm_warning(path: &str, mode: u32) -> Option<String> {
    // Only the low 9 permission bits matter; mask off file-type/setuid bits so
    // the octal we print to the user matches what `chmod`/`ls -l` would show.
    let perms = mode & 0o777;
    if perms & 0o077 != 0 {
        Some(format!(
            "SSH key file '{path}' has insecure permissions ({:#o}) — readable by \
             group/other. OpenSSH may refuse it. Fix: chmod 600 {path}",
            perms
        ))
    } else {
        None
    }
}

/// Expand a leading `~` / `~/` to `$HOME`. Anything else is returned as-is.
/// (Only a leading bare `~` is expanded — `~user` is left alone, matching how
/// the keys are typically stored, e.g. `~/.ssh/id_rsa`.)
fn expand_tilde(path: &str) -> String {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return home.to_string_lossy().into_owned();
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let mut p = home.to_string_lossy().into_owned();
            if !p.ends_with('/') {
                p.push('/');
            }
            p.push_str(rest);
            return p;
        }
    }
    path.to_string()
}

/// Stat the key file and return a permission warning if it is group/other
/// accessible. Returns `None` on non-unix, or when the file is missing /
/// unreadable (ssh will surface its own error in that case).
#[cfg(unix)]
pub(crate) fn check_key_permissions(path: &str) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let resolved = expand_tilde(path);
    let meta = std::fs::metadata(&resolved).ok()?;
    // Report against the path the user entered, not the tilde-expanded one.
    key_perm_warning(path, meta.permissions().mode())
}

#[cfg(not(unix))]
pub(crate) fn check_key_permissions(_path: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_modes_return_none() {
        assert!(key_perm_warning("/k", 0o600).is_none());
        assert!(key_perm_warning("/k", 0o400).is_none());
        // High bits (file-type / setuid) don't count as group/other access.
        assert!(key_perm_warning("/k", 0o100600).is_none());
    }

    #[test]
    fn group_or_other_readable_warns() {
        let w = key_perm_warning("/home/u/.ssh/id_rsa", 0o644).expect("644 must warn");
        assert!(w.contains("chmod 600 /home/u/.ssh/id_rsa"), "fix cmd: {w}");
        assert!(w.contains("0o644"), "octal printed: {w}");
        assert!(w.contains("/home/u/.ssh/id_rsa"), "path printed: {w}");

        assert!(key_perm_warning("/k", 0o660).is_some()); // group rw
        assert!(key_perm_warning("/k", 0o640).is_some()); // group r
        assert!(key_perm_warning("/k", 0o604).is_some()); // other r
        assert!(key_perm_warning("/k", 0o601).is_some()); // other x
    }

    #[test]
    fn each_077_bit_trips_independently() {
        for bit in [0o040, 0o020, 0o010, 0o004, 0o002, 0o001] {
            assert!(
                key_perm_warning("/k", 0o600 | bit).is_some(),
                "bit {bit:#o} should warn",
            );
        }
    }

    #[test]
    fn octal_in_message_excludes_file_type_bits() {
        // A real stat() yields 0o100644 for a regular file; the message must
        // show 0o644, the chmod-relevant value.
        let w = key_perm_warning("/k", 0o100644).expect("644 must warn");
        assert!(w.contains("0o644"), "got: {w}");
        assert!(!w.contains("0o100644"), "raw mode leaked: {w}");
    }
}
