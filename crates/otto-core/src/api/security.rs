//! Trust & Safety: audit log + security posture (root only).
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/auth.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Trust & Safety: audit log + security posture (root only)
// ---------------------------------------------------------------------------

/// Query string for `GET /api/v1/audit-log` (root only). All filters optional;
/// `from`/`to` bound `ts` (inclusive). `limit`/`offset` page the result newest
/// first; `limit` is clamped server-side.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuditLogQuery {
    /// Lower bound on `ts` (RFC3339), inclusive.
    pub from: Option<DateTime<Utc>>,
    /// Upper bound on `ts` (RFC3339), inclusive.
    pub to: Option<DateTime<Utc>>,
    /// Exact `action` match, e.g. `"login.success"`.
    pub action: Option<String>,
    /// Exact acting-user match.
    pub user_id: Option<Id>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// `GET /api/v1/audit-log` — a page of audit entries plus the total matching the
/// filters (so the UI can paginate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogResp {
    pub entries: Vec<crate::domain::AuditEntry>,
    /// Total rows matching the filters (ignores `limit`/`offset`).
    pub total: i64,
}

/// One self-audit check returned by `GET /api/v1/security-posture`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityPostureCheck {
    pub id: String,
    pub category: String,
    pub title: String,
    /// `"pass"` | `"warn"` | `"fail"`
    pub status: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

/// Raw live daemon measurements fed into [`evaluate_security_posture`].
#[derive(Debug, Clone, Default)]
pub struct SecurityPostureFacts {
    pub has_encrypted_secrets: bool,
    pub has_plaintext_secrets: bool,
    pub orphaned_secret_files: usize,
    pub db_mode: Option<u32>,
    pub tls_dir_mode: Option<u32>,
    pub tls_key_mode: Option<u32>,
    pub network_listener: bool,
    pub network_listener_port: Option<u16>,
    pub allowed_hosts_env: Option<String>,
    pub active_shares: i64,
    pub active_phone_devices: i64,
    pub active_api_tokens: i64,
    pub enabled_plugins: usize,
    pub unrestricted_plugins: Vec<String>,
    pub jira_cred_plugins: Vec<String>,
    pub wal_bytes: u64,
    pub page_count: i64,
    pub freelist_count: i64,
    pub journal_mode: Option<String>,
}

/// Evaluate all live security-posture invariants deterministically.
pub fn evaluate_security_posture(
    facts: &SecurityPostureFacts,
) -> (String, Vec<SecurityPostureCheck>) {
    let mut checks = Vec::with_capacity(6);

    // 1. Secrets encryption & orphaned files
    let (s_status, s_detail, s_rem) = if facts.has_plaintext_secrets && !facts.has_encrypted_secrets
    {
        (
            "warn",
            "Plaintext secrets.json is active (encrypted store not enabled)".to_string(),
            Some("Run \"Secure secrets\" in Settings to migrate to AES-256-GCM Keychain-backed encryption.".into()),
        )
    } else if facts.has_plaintext_secrets && facts.has_encrypted_secrets {
        (
            "fail",
            "Both secrets.enc and plaintext secrets.json exist on disk".to_string(),
            Some(
                "Complete secrets migration and remove the residual plaintext secrets.json.".into(),
            ),
        )
    } else if facts.orphaned_secret_files > 0 {
        (
            "warn",
            format!(
                "Encrypted secrets active, but {} orphaned secret file(s) remain in data_dir",
                facts.orphaned_secret_files
            ),
            Some("Inspect and remove stale secrets.enc.orphaned-* files once verified.".into()),
        )
    } else if facts.has_encrypted_secrets {
        (
            "pass",
            "AES-256-GCM encrypted secrets store active with no plaintext residue".to_string(),
            None,
        )
    } else {
        (
            "pass",
            "Keychain / clean secrets store (no plaintext secrets file on disk)".to_string(),
            None,
        )
    };
    checks.push(SecurityPostureCheck {
        id: "secrets_backend".into(),
        category: "secrets".into(),
        title: "Secrets storage".into(),
        status: s_status.into(),
        detail: s_detail,
        remediation: s_rem,
    });

    // 2. SQLite DB & TLS key permissions
    let mut perm_issues: Vec<String> = Vec::new();
    let mut perm_critical = false;
    if let Some(m) = facts.tls_key_mode {
        if m & 0o077 != 0 {
            perm_critical = true;
            perm_issues.push(format!("tls/key.pem is {:04o} (expected 0600)", m));
        }
    }
    if let Some(m) = facts.tls_dir_mode {
        if m & 0o077 != 0 {
            perm_issues.push(format!("tls/ dir is {:04o} (expected 0700)", m));
        }
    }
    if let Some(m) = facts.db_mode {
        if m & 0o007 != 0 {
            perm_critical = true;
            perm_issues.push(format!("otto.db is world-accessible ({:04o})", m));
        } else if m & 0o070 != 0 {
            perm_issues.push(format!("otto.db is group-accessible ({:04o})", m));
        }
    }
    let (p_status, p_detail, p_rem) = if perm_critical {
        (
            "fail",
            perm_issues.join("; "),
            Some("Tighten permissions to chmod 0600 on otto.db and tls/key.pem and chmod 0700 on tls/.".into()),
        )
    } else if !perm_issues.is_empty() {
        (
            "warn",
            perm_issues.join("; "),
            Some("Tighten group permissions with chmod 0600 / 0700.".into()),
        )
    } else {
        (
            "pass",
            "Database and TLS key permissions are owner-restricted".to_string(),
            None,
        )
    };
    checks.push(SecurityPostureCheck {
        id: "db_and_tls_perms".into(),
        category: "filesystem".into(),
        title: "Database & TLS permissions".into(),
        status: p_status.into(),
        detail: p_detail,
        remediation: p_rem,
    });

    // 3. Network exposure & OTTO_ALLOWED_HOSTS
    let hosts_raw = facts.allowed_hosts_env.as_deref().unwrap_or("").trim();
    let has_wildcard = hosts_raw
        .split(',')
        .map(str::trim)
        .any(|h| h == "*" || h.starts_with("*."));
    let (n_status, n_detail, n_rem) = if has_wildcard {
        (
            "fail",
            format!("OTTO_ALLOWED_HOSTS contains a wildcard entry ({hosts_raw})"),
            Some("Replace wildcard entries in OTTO_ALLOWED_HOSTS with explicit hostnames.".into()),
        )
    } else if facts.network_listener {
        let port = facts.network_listener_port.unwrap_or(0);
        let suffix = if hosts_raw.is_empty() {
            String::new()
        } else {
            format!("; OTTO_ALLOWED_HOSTS={hosts_raw}")
        };
        (
            "warn",
            format!("Network TLS listener active on 0.0.0.0:{port}{suffix}"),
            Some("Disable the network listener in Settings → Daemon when remote access is not needed.".into()),
        )
    } else {
        let suffix = if hosts_raw.is_empty() {
            String::new()
        } else {
            format!(" (OTTO_ALLOWED_HOSTS={hosts_raw})")
        };
        ("pass", format!("Loopback-only (127.0.0.1){suffix}"), None)
    };
    checks.push(SecurityPostureCheck {
        id: "network_exposure".into(),
        category: "network".into(),
        title: "Listener & host allowlist".into(),
        status: n_status.into(),
        detail: n_detail,
        remediation: n_rem,
    });

    // 4. Remote shares, paired phones, and API tokens
    let (a_status, a_detail, a_rem) = if facts.active_shares > 0 || facts.active_phone_devices > 0 {
        (
            "warn",
            format!(
                "{} active share link(s), {} paired phone device(s), {} API token(s)",
                facts.active_shares, facts.active_phone_devices, facts.active_api_tokens
            ),
            Some(
                "Review and revoke unused share links, phone devices, or API tokens periodically."
                    .into(),
            ),
        )
    } else {
        (
            "pass",
            format!(
                "0 active share links, 0 paired phone devices, {} API token(s)",
                facts.active_api_tokens
            ),
            None,
        )
    };
    checks.push(SecurityPostureCheck {
        id: "remote_shares_and_devices".into(),
        category: "access".into(),
        title: "Remote shares & devices".into(),
        status: a_status.into(),
        detail: a_detail,
        remediation: a_rem,
    });

    // 5. Enabled plugins & declared capabilities
    let (pl_status, pl_detail, pl_rem) = if facts.enabled_plugins == 0 {
        ("pass", "No runtime plugins enabled".to_string(), None)
    } else if !facts.unrestricted_plugins.is_empty() || !facts.jira_cred_plugins.is_empty() {
        let mut parts = vec![format!("{} plugin(s) enabled", facts.enabled_plugins)];
        if !facts.unrestricted_plugins.is_empty() {
            parts.push(format!(
                "without explicit capabilities: {}",
                facts.unrestricted_plugins.join(", ")
            ));
        }
        if !facts.jira_cred_plugins.is_empty() {
            parts.push(format!(
                "with jira:credentials access: {}",
                facts.jira_cred_plugins.join(", ")
            ));
        }
        (
            "warn",
            parts.join("; "),
            Some(
                "Declare minimal `capabilities` in otto-plugin.json and disable unused plugins."
                    .into(),
            ),
        )
    } else {
        (
            "pass",
            format!(
                "{} enabled plugin(s), all with explicit scoped capabilities",
                facts.enabled_plugins
            ),
            None,
        )
    };
    checks.push(SecurityPostureCheck {
        id: "plugins_capabilities".into(),
        category: "plugins".into(),
        title: "Plugin capabilities".into(),
        status: pl_status.into(),
        detail: pl_detail,
        remediation: pl_rem,
    });

    // 6. SQLite WAL size & freelist ratio
    let wal_mb = facts.wal_bytes as f64 / (1024.0 * 1024.0);
    let free_pct = if facts.page_count > 0 {
        (facts.freelist_count.max(0) as f64 / facts.page_count as f64) * 100.0
    } else {
        0.0
    };
    let jm = facts.journal_mode.as_deref().unwrap_or("unknown");
    let (db_status, db_detail, db_rem) = if facts.wal_bytes > 128 * 1024 * 1024 || free_pct >= 35.0
    {
        (
            "warn",
            format!("journal={jm}, WAL={wal_mb:.1} MiB, freelist={free_pct:.1}%"),
            Some("Run Database Compact in Settings → Daemon to reclaim freelist pages and checkpoint WAL.".into()),
        )
    } else {
        (
            "pass",
            format!("journal={jm}, WAL={wal_mb:.1} MiB, freelist={free_pct:.1}%"),
            None,
        )
    };
    checks.push(SecurityPostureCheck {
        id: "sqlite_health".into(),
        category: "database".into(),
        title: "SQLite WAL & compaction".into(),
        status: db_status.into(),
        detail: db_detail,
        remediation: db_rem,
    });

    let overall = if checks.iter().any(|c| c.status == "fail") {
        "critical"
    } else if checks.iter().any(|c| c.status == "warn") {
        "warn"
    } else {
        "healthy"
    };
    (overall.into(), checks)
}

/// `GET /api/v1/security-posture` (root only) — a snapshot the Trust & Safety
/// Center renders, derived from the live listener, settings and auth store,
/// plus live self-audit checks (`overall`, `checked_at`, `checks`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPostureResp {
    /// Whether the daemon's network (0.0.0.0) listener is actually bound.
    pub network_listener: bool,
    /// The actual bound network port (None = no network listener).
    pub network_listener_port: Option<u16>,
    /// True when no network listener is bound (daemon is loopback-only).
    pub loopback_only: bool,
    /// Saved configuration differs from the running listener; restart to apply.
    #[serde(default)]
    pub network_listener_restart_required: bool,
    /// Count of currently-active (unexpired) API (personal access) tokens.
    pub active_api_tokens: i64,
    /// Overall self-audit verdict: `"healthy"` | `"warn"` | `"critical"`.
    #[serde(default)]
    pub overall: String,
    /// RFC3339 timestamp when the self-audit ran.
    #[serde(default)]
    pub checked_at: String,
    /// Categorized live self-audit invariant checks.
    #[serde(default)]
    pub checks: Vec<SecurityPostureCheck>,
}

#[cfg(test)]
mod posture_tests {
    use super::*;

    #[test]
    fn healthy_baseline_returns_healthy_with_six_passing_checks() {
        let facts = SecurityPostureFacts {
            has_encrypted_secrets: true,
            db_mode: Some(0o600),
            tls_dir_mode: Some(0o700),
            tls_key_mode: Some(0o600),
            journal_mode: Some("wal".into()),
            page_count: 100,
            freelist_count: 2,
            ..Default::default()
        };
        let (overall, checks) = evaluate_security_posture(&facts);
        assert_eq!(overall, "healthy");
        assert_eq!(checks.len(), 6);
        assert!(checks.iter().all(|c| c.status == "pass"));
    }

    #[test]
    fn world_readable_tls_key_and_wildcard_hosts_trigger_critical() {
        let facts = SecurityPostureFacts {
            has_encrypted_secrets: true,
            tls_key_mode: Some(0o644),
            allowed_hosts_env: Some("*.example.com".into()),
            ..Default::default()
        };
        let (overall, checks) = evaluate_security_posture(&facts);
        assert_eq!(overall, "critical");
        assert_eq!(
            checks
                .iter()
                .find(|c| c.id == "db_and_tls_perms")
                .unwrap()
                .status,
            "fail"
        );
        assert_eq!(
            checks
                .iter()
                .find(|c| c.id == "network_exposure")
                .unwrap()
                .status,
            "fail"
        );
    }

    #[test]
    fn unrestricted_plugins_and_high_freelist_trigger_warn() {
        let facts = SecurityPostureFacts {
            has_encrypted_secrets: true,
            enabled_plugins: 1,
            unrestricted_plugins: vec!["legacy".into()],
            page_count: 100,
            freelist_count: 50,
            ..Default::default()
        };
        let (overall, checks) = evaluate_security_posture(&facts);
        assert_eq!(overall, "warn");
        assert_eq!(
            checks
                .iter()
                .find(|c| c.id == "plugins_capabilities")
                .unwrap()
                .status,
            "warn"
        );
        assert_eq!(
            checks
                .iter()
                .find(|c| c.id == "sqlite_health")
                .unwrap()
                .status,
            "warn"
        );
    }
}
