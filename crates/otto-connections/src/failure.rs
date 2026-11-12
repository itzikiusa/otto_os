//! What a failed connection test tells the user beyond its one-line message:
//! a "what to check" hint for the common non-SSH failure classes, and the
//! full (capped, redacted) client stderr as `detail`.
//!
//! SSH failures are diagnosed by `otto_ssh::diagnose`; this covers the DB
//! drivers and clients (MySQL, Postgres, Redis, MongoDB, ClickHouse), whose
//! errors are worded differently per engine.

/// Cap on `detail`. The explanation of a CLI failure is at the END of its
/// stderr (ssh prints its fatal error last), so the tail is kept.
pub const DETAIL_CAP: usize = 8 * 1024;

const AUTH: &str = "the server rejected the user name or password. Check the credentials \
     (for MongoDB also the auth database / authSource; for MySQL the user's allowed host)";
const UNKNOWN_DB: &str = "the database name doesn't exist on this server, or this user can't \
     see it. Check the database field";
const TLS_REQUIRED: &str =
    "the server only accepts TLS connections. Set TLS to Required (or Preferred)";
const TLS_UNSUPPORTED: &str = "the server doesn't speak TLS on this port. Set TLS to Disabled \
     or Preferred, or use the server's TLS port";
const TLS_UNTRUSTED: &str = "the server's certificate isn't trusted. Add its CA certificate, \
     or turn off verification for a private / self-signed server";
const REFUSED: &str = "nothing is listening on that host and port. Check the port, that the \
     server is running, and the SSH tunnel / jump host";
const TIMEOUT: &str = "the host didn't answer in time. Check the VPN, firewall / security \
     group, the port, or the jump host";
const DNS: &str =
    "the host name doesn't resolve. Check the spelling, or connect the VPN / DNS that knows it";

/// (lower-case needle, hint). The first needle found wins, so the order runs
/// from most to least specific: a TLS handshake error often also says
/// "connection reset", and an auth error can mention the database.
const PATTERNS: &[(&str, &str)] = &[
    // TLS required by the server.
    ("require_secure_transport", TLS_REQUIRED),
    ("requires secure transport", TLS_REQUIRED),
    ("ssl connection is required", TLS_REQUIRED),
    ("ssl is required", TLS_REQUIRED),
    ("no pg_hba.conf entry", TLS_REQUIRED),
    // TLS offered but not spoken by the server.
    ("server does not support tls", TLS_UNSUPPORTED),
    ("server does not support ssl", TLS_UNSUPPORTED),
    ("ssl is not enabled on the server", TLS_UNSUPPORTED),
    ("wrong version number", TLS_UNSUPPORTED),
    ("invalidcontenttype", TLS_UNSUPPORTED),
    ("received corrupt message", TLS_UNSUPPORTED),
    // Certificate trust.
    ("certificate verify failed", TLS_UNTRUSTED),
    ("unknownissuer", TLS_UNTRUSTED),
    ("unknown issuer", TLS_UNTRUSTED),
    ("invalid peer certificate", TLS_UNTRUSTED),
    ("self signed certificate", TLS_UNTRUSTED),
    ("self-signed certificate", TLS_UNTRUSTED),
    ("certificate has expired", TLS_UNTRUSTED),
    // Credentials.
    ("access denied for user", AUTH),
    ("password authentication failed", AUTH),
    ("authentication failed", AUTH),
    ("wrongpass", AUTH),
    ("invalid username-password", AUTH),
    ("noauth", AUTH),
    ("authentication_failed", AUTH),
    ("bad credentials", AUTH),
    // Database name.
    ("unknown database", UNKNOWN_DB),
    ("unknown_database", UNKNOWN_DB),
    ("doesn't exist", UNKNOWN_DB),
    ("does not exist", UNKNOWN_DB),
    // Network.
    ("connection refused", REFUSED),
    ("could not resolve", DNS),
    ("nodename nor servname", DNS),
    ("name or service not known", DNS),
    ("failed to lookup address", DNS),
    ("no such host", DNS),
    ("unknown host", DNS),
    ("timed out", TIMEOUT),
    ("timeout", TIMEOUT),
    ("deadline has elapsed", TIMEOUT),
];

/// A "what to check" hint for a failed (non-SSH) connection test, from the
/// driver's / client's error text. `None` when nothing is recognised.
pub fn classify_failure(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    // "does not exist" names a database only when it says so (Postgres:
    // `database "x" does not exist`; ClickHouse: `Database x doesn't exist`);
    // a missing table or role is not a connection problem.
    PATTERNS
        .iter()
        .filter(|(needle, hint)| {
            *hint != UNKNOWN_DB || needle.starts_with("unknown") || lower.contains("database")
        })
        .find(|(needle, _)| lower.contains(needle))
        .map(|(_, hint)| *hint)
}

/// The full client stderr for `detail`: PEM blocks (private keys, certs)
/// replaced, the connection's own `secret` removed wherever it appears,
/// URI userinfo and `--password` argv scrubbed per line (`redact_line`), and
/// the result capped to its last [`DETAIL_CAP`] bytes. `None` when empty.
pub fn redact_detail(
    stderr: &str,
    secret: Option<&str>,
    redact_line: impl Fn(&str) -> String,
) -> Option<String> {
    let mut out = String::new();
    let mut in_pem = false;
    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("-----BEGIN") {
            in_pem = !trimmed.contains("-----END");
            out.push_str("<key material redacted>\n");
            continue;
        }
        if in_pem {
            if trimmed.starts_with("-----END") {
                in_pem = false;
            }
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let mut line = redact_line(trimmed);
        if let Some(secret) = secret.filter(|s| s.len() >= 3) {
            line = line.replace(secret, "<redacted>");
        }
        out.push_str(&line);
        out.push('\n');
    }
    let out = out.trim_end();
    if out.is_empty() {
        return None;
    }
    Some(tail(out, DETAIL_CAP))
}

/// The last `cap` bytes of `s`, cut on a char boundary, marked when cut.
fn tail(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut start = s.len() - cap;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_common_engine_errors() {
        let cases = [
            (
                "error returned from database: 1045 (28000): Access denied for user 'app'@'10.0.0.1' (using password: YES)",
                AUTH,
            ),
            (
                "Command failed: Authentication failed., code: 18",
                AUTH,
            ),
            ("WRONGPASS invalid username-password pair", AUTH),
            ("1049 (42000): Unknown database 'shopp'", UNKNOWN_DB),
            (
                "error returned from database: database \"shop\" does not exist",
                UNKNOWN_DB,
            ),
            (
                "Code: 81. DB::Exception: Database analytics doesn't exist. (UNKNOWN_DATABASE)",
                UNKNOWN_DB,
            ),
            (
                "3159 (HY000): Connections using insecure transport are prohibited while --require_secure_transport=ON.",
                TLS_REQUIRED,
            ),
            ("error communicating with database: server does not support TLS", TLS_UNSUPPORTED),
            (
                "invalid peer certificate: UnknownIssuer",
                TLS_UNTRUSTED,
            ),
            ("Connection refused (os error 61)", REFUSED),
            (
                "failed to lookup address information: nodename nor servname provided, or not known",
                DNS,
            ),
            ("Server selection timeout: No available servers", TIMEOUT),
            ("pool timed out while waiting for an open connection", TIMEOUT),
        ];
        for (text, want) in cases {
            assert_eq!(classify_failure(text), Some(want), "{text}");
        }
    }

    #[test]
    fn a_missing_table_is_not_a_database_problem() {
        assert_eq!(classify_failure("relation \"users\" does not exist"), None);
        assert_eq!(classify_failure("something new went wrong"), None);
    }

    #[test]
    fn tls_errors_win_over_the_reset_they_cause() {
        assert_eq!(
            classify_failure("tls handshake: received corrupt message; connection reset"),
            Some(TLS_UNSUPPORTED)
        );
    }

    fn scrub(l: &str) -> String {
        l.replace("u:pw@", "")
    }

    #[test]
    fn detail_drops_key_material_and_the_secret() {
        let stderr = "connecting\n-----BEGIN OPENSSH PRIVATE KEY-----\nAAAAsecretkeybytes\n\
                      -----END OPENSSH PRIVATE KEY-----\nauth failed for hunter22 at mongodb://u:pw@h\n";
        let d = redact_detail(stderr, Some("hunter22"), scrub).unwrap();
        assert!(!d.contains("AAAAsecretkeybytes"), "{d}");
        assert!(d.contains("<key material redacted>"), "{d}");
        assert!(!d.contains("hunter22"), "{d}");
        assert!(!d.contains("u:pw@"), "{d}");
        assert!(d.starts_with("connecting\n"), "{d}");
        assert!(d.ends_with("mongodb://h"), "{d}");
    }

    #[test]
    fn detail_keeps_the_tail_within_the_cap() {
        let long = format!("{}\nthe real error", "noise line\n".repeat(2000));
        let d = redact_detail(&long, None, str::to_string).unwrap();
        assert!(d.len() <= DETAIL_CAP + '…'.len_utf8(), "{}", d.len());
        assert!(d.starts_with('…'));
        assert!(d.ends_with("the real error"));
        assert_eq!(redact_detail(" \n\n", None, str::to_string), None);
    }
}
