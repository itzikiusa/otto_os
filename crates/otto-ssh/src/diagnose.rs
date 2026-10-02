//! Turn the system `ssh` / `sftp` client's stderr into one line a person can act
//! on.
//!
//! Every caller used to surface the FIRST non-empty stderr line, which is very
//! often noise rather than the failure: `accept-new` prints "Warning:
//! Permanently added '<host>' … to the list of known hosts." before the real
//! "Permission denied (publickey)", OpenSSH ≥ 10 prints a three-line
//! post-quantum key-exchange notice on every connect, and `ssh -t` without a
//! terminal adds "Pseudo-terminal will not be allocated …". The connection test,
//! the tunnel opener and the SFTP browser then all reported the noise.
//!
//! [`diagnose`] picks the line that explains the failure (a known failure
//! pattern first, else the LAST non-noise line — ssh's fatal error is printed
//! last) and pairs it with a short "what to fix" hint for the common cases.

use std::fmt;

/// The explanatory stderr line plus, when the failure is a recognised one, a
/// short instruction for fixing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshDiagnosis {
    /// The stderr line that explains the failure (trimmed). Empty when ssh said
    /// nothing useful.
    pub line: String,
    /// What the user should change, for the recognised failure classes.
    pub hint: Option<&'static str>,
}

impl SshDiagnosis {
    /// True when ssh printed nothing beyond noise.
    pub fn is_empty(&self) -> bool {
        self.line.is_empty()
    }
}

impl fmt::Display for SshDiagnosis {
    /// `line — hint` (or just the line), for error strings that have no
    /// separate hint field.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.hint {
            Some(hint) if !self.line.is_empty() => write!(f, "{} — {hint}", self.line),
            Some(hint) => f.write_str(hint),
            None => f.write_str(&self.line),
        }
    }
}

/// Lines ssh/sftp print on stderr that never explain a failure.
fn is_noise(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.starts_with("warning: permanently added")
        || l.starts_with("pseudo-terminal will not be allocated")
        // OpenSSH 10 post-quantum notice: "** WARNING: connection is not using
        // a post-quantum key exchange algorithm." + two "** …" follow-ups.
        || l.starts_with("**")
        || l.starts_with("connection to ") && l.ends_with(" closed.")
        || l.starts_with("sftp>")
        || l.starts_with("debug")
}

/// Known failure classes: (lower-case needle, hint). The first needle found in
/// any line wins, so order from most to least specific.
const PATTERNS: &[(&str, &str)] = &[
    (
        "remote host identification has changed",
        "the server's host key changed since it was first trusted. If that is expected (rebuilt server), remove the old key with `ssh-keygen -R <host>` and test again; otherwise do not connect",
    ),
    (
        "host key verification failed",
        "the server's host key does not match ~/.ssh/known_hosts. If the server was rebuilt, run `ssh-keygen -R <host>` and test again",
    ),
    (
        "unprotected private key file",
        "the identity file is readable by other users, so ssh ignores it. Fix: chmod 600 <identity file>",
    ),
    (
        "bad permissions",
        "the identity file is readable by other users, so ssh ignores it. Fix: chmod 600 <identity file>",
    ),
    (
        "no such identity",
        "the identity file does not exist — check the path (a leading ~ is expanded)",
    ),
    (
        "too many authentication failures",
        "ssh offered too many keys before the right one. Set the identity file explicitly on this connection",
    ),
    (
        // Auth failures always list the methods tried — "Permission denied
        // (publickey,password)." — unlike an SFTP file-permission error.
        "permission denied (",
        "the server rejected the login. Check the user name and the identity file, or load the key into ssh-agent (`ssh-add`). Password-only logins work only in a terminal session",
    ),
    (
        "could not resolve hostname",
        "the host name doesn't resolve. Check the spelling, or connect the VPN / DNS that knows it",
    ),
    (
        "nodename nor servname",
        "the host name doesn't resolve. Check the spelling, or connect the VPN / DNS that knows it",
    ),
    (
        "connection refused",
        "nothing is listening on that port. Check the port, and that sshd runs on the host",
    ),
    (
        "no route to host",
        "the host is unreachable from this Mac. Check the VPN, firewall or security group",
    ),
    (
        "network is unreachable",
        "this Mac has no route to that network. Check Wi-Fi / VPN",
    ),
    (
        "timed out",
        "the host didn't answer. Check the VPN, firewall / security group, the port, or the jump host",
    ),
    (
        "administratively prohibited",
        "the SSH server refuses port forwarding (AllowTcpForwarding no) — ask its admin, or use a different bastion",
    ),
    (
        "address already in use",
        "a local port Otto picked was taken by another process — try again",
    ),
    (
        "kex_exchange_identification",
        "the server closed the connection before login — often a firewall, fail2ban or MaxStartups limit. Wait a moment and retry",
    ),
    (
        "connection closed by",
        "the server closed the connection — often a firewall, fail2ban, or a jump host that can't reach the target",
    ),
    (
        "connection reset by",
        "the connection was reset — often a firewall or VPN dropping it. Retry, and check the jump host",
    ),
];

/// Pick the explanatory line out of ssh / sftp stderr and attach a fix hint.
pub fn diagnose(stderr: &str) -> SshDiagnosis {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !is_noise(l))
        .collect();
    for (needle, hint) in PATTERNS {
        if let Some(line) = lines
            .iter()
            .find(|l| l.to_ascii_lowercase().contains(needle))
        {
            return SshDiagnosis {
                line: (*line).to_string(),
                hint: Some(hint),
            };
        }
    }
    SshDiagnosis {
        line: lines.last().map(|l| l.to_string()).unwrap_or_default(),
        hint: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_known_hosts_notice_and_finds_the_auth_failure() {
        let d = diagnose(
            "Warning: Permanently added 'db.example.com' (ED25519) to the list of known hosts.\n\
             deploy@db.example.com: Permission denied (publickey).\n",
        );
        assert_eq!(
            d.line,
            "deploy@db.example.com: Permission denied (publickey)."
        );
        assert!(d.hint.unwrap().contains("ssh-add"));
        assert!(d
            .to_string()
            .starts_with("deploy@db.example.com: Permission denied"));
    }

    #[test]
    fn skips_the_post_quantum_notice() {
        let d = diagnose(
            "** WARNING: connection is not using a post-quantum key exchange algorithm.\n\
             ** This session may be vulnerable to \"store now, decrypt later\" attacks.\n\
             ** The server may need to be upgraded. See https://openssh.com/pq.html\n\
             ssh: connect to host 10.0.0.5 port 22: Connection refused\n",
        );
        assert_eq!(
            d.line,
            "ssh: connect to host 10.0.0.5 port 22: Connection refused"
        );
        assert!(d.hint.unwrap().contains("port"));
    }

    #[test]
    fn changed_host_key_wins_over_the_generic_verification_line() {
        let d = diagnose(
            "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
             @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
             Host key verification failed.\n",
        );
        assert!(d.line.contains("REMOTE HOST IDENTIFICATION HAS CHANGED"));
        assert!(d.hint.unwrap().contains("ssh-keygen -R"));
    }

    #[test]
    fn resolves_and_timeouts_get_their_own_hints() {
        let d = diagnose("ssh: Could not resolve hostname nope.internal: nodename nor servname provided, or not known");
        assert!(d.hint.unwrap().contains("VPN / DNS"));
        let d = diagnose("ssh: connect to host 10.1.2.3 port 22: Operation timed out");
        assert!(d.hint.unwrap().contains("firewall"));
        let d = diagnose("Load key \"/Users/me/.ssh/id_rsa\": bad permissions");
        assert!(d.hint.unwrap().contains("chmod 600"));
    }

    #[test]
    fn remote_file_permission_errors_are_not_called_login_failures() {
        let d = diagnose("Couldn't delete file: Permission denied\n");
        assert_eq!(d.line, "Couldn't delete file: Permission denied");
        assert_eq!(d.hint, None);
    }

    #[test]
    fn unknown_failure_reports_the_last_meaningful_line() {
        let d = diagnose("Warning: Permanently added 'h' to the list of known hosts.\nfirst thing\nthe real fatal error\n");
        assert_eq!(d.line, "the real fatal error");
        assert_eq!(d.hint, None);
        assert_eq!(d.to_string(), "the real fatal error");
    }

    #[test]
    fn noise_only_is_empty() {
        let d =
            diagnose("Pseudo-terminal will not be allocated because stdin is not a terminal.\n\n");
        assert!(d.is_empty());
        assert_eq!(d.to_string(), "");
    }
}
