use std::path::Path;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use otto_state::DbPool;
use reqwest::header::{HeaderMap, CONTENT_TYPE, SET_COOKIE};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::store::{db_error, now};
use crate::{Check, EngineRun, EngineVersion, Finding, ScanMetrics, ScanMode, Severity};

pub const CONFIG_KEY: &str = "OTTO_SECURITY_URL";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Target {
    pub run_config_id: String,
    pub url: String,
    pub allow_public: bool,
    pub scheduled: bool,
    pub confirmed_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub run_config_id: String,
    pub run_config_name: String,
    pub url: String,
    pub public: bool,
}

#[derive(Serialize)]
pub struct ProjectPassive {
    pub confirmed: Option<Target>,
    pub candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmTarget {
    pub run_config_id: String,
    pub url: String,
    pub allow_public: bool,
    pub scheduled: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunDastRequest {
    pub active: bool,
    pub run_config_id: Option<String>,
    pub url: Option<String>,
    pub confirmed_at: Option<String>,
}

impl RunDastRequest {
    /// Binds active DAST opt-in to the exact confirmation identity displayed to
    /// the human in the prompt. Passive runs (`active == false`) may omit these
    /// fields, preserving passive-by-default behavior; if supplied on a passive
    /// run, they are still checked against `target`.
    pub fn verify_target(&self, target: &Target) -> Result<()> {
        if self.active {
            let (Some(run_config_id), Some(url), Some(confirmed_at)) = (
                self.run_config_id.as_deref().filter(|s| !s.is_empty()),
                self.url.as_deref().filter(|s| !s.is_empty()),
                self.confirmed_at.as_deref().filter(|s| !s.is_empty()),
            ) else {
                return Err(Error::Invalid(
                    "Active DAST requires the confirmed target identity (run_config_id, url, confirmed_at).".into(),
                ));
            };
            if run_config_id != target.run_config_id
                || url != target.url
                || confirmed_at != target.confirmed_at
            {
                return Err(Error::Conflict(
                    "The confirmed target changed. Review the current target and confirm again."
                        .into(),
                ));
            }
            return Ok(());
        }
        if let Some(run_config_id) = self.run_config_id.as_deref() {
            if run_config_id != target.run_config_id {
                return Err(Error::Conflict(
                    "The confirmed target or its running configuration changed.".into(),
                ));
            }
        }
        if let Some(url) = self.url.as_deref() {
            if url != target.url {
                return Err(Error::Conflict(
                    "The confirmed target or its running configuration changed.".into(),
                ));
            }
        }
        if let Some(confirmed_at) = self.confirmed_at.as_deref() {
            if confirmed_at != target.confirmed_at {
                return Err(Error::Conflict(
                    "The confirmed target or its running configuration changed.".into(),
                ));
            }
        }
        Ok(())
    }
}

pub fn parse_url(raw: &str) -> Result<(Url, bool)> {
    let url =
        Url::parse(raw.trim()).map_err(|_| Error::Invalid("Enter an HTTP or HTTPS URL.".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.as_str().len() > 512
    {
        return Err(Error::Invalid(
            "Use an HTTP or HTTPS URL without credentials, query parameters, or a fragment.".into(),
        ));
    }
    let port = url.port_or_known_default().unwrap_or_default();
    if port == 0
        || otto_netguard::is_daemon_port(port)
        || otto_netguard::metadata_refusal(&url).is_some()
    {
        return Err(Error::Forbidden(
            "This destination is outside the passive audit scope.".into(),
        ));
    }
    let public = !otto_netguard::is_explicit_local_target(&url);
    if public && url.scheme() != "https" {
        return Err(Error::Invalid("Public audit targets require HTTPS.".into()));
    }
    let permitted = if public {
        otto_netguard::check_url_literal(&url)
    } else {
        otto_netguard::check_url_literal_allow_local(&url)
    };
    if !permitted {
        return Err(Error::Forbidden(
            "This destination is outside the passive audit scope.".into(),
        ));
    }
    Ok((url, public))
}

pub async fn get(pool: &DbPool, project: &str) -> Result<Option<Target>> {
    sqlx::query_as(
        "SELECT run_config_id, url, allow_public, scheduled, confirmed_at
                    FROM ide_security_passive_targets WHERE project_id = ?",
    )
    .bind(project)
    .fetch_optional(pool)
    .await
    .map_err(db_error)
}

pub async fn confirm(
    pool: &DbPool,
    project: &str,
    candidate: Candidate,
    request: ConfirmTarget,
) -> Result<Target> {
    if request.url != candidate.url {
        return Err(Error::Conflict(
            "The run configuration URL changed. Review the new destination.".into(),
        ));
    }
    if candidate.public && !request.allow_public {
        return Err(Error::Forbidden(
            "Confirm this public destination explicitly.".into(),
        ));
    }
    let target = Target {
        url: candidate.url,
        run_config_id: candidate.run_config_id,
        allow_public: candidate.public,
        scheduled: request.scheduled,
        confirmed_at: now(),
    };
    // Scheduled active DAST is disabled; explicitly write 0 to the additive
    // scheduled_active column so any prior value is cleared on reconfirmation.
    sqlx::query(
        "INSERT INTO ide_security_passive_targets (project_id, run_config_id, url, allow_public, scheduled, scheduled_active, confirmed_at)
         VALUES (?, ?, ?, ?, ?, 0, ?) ON CONFLICT(project_id) DO UPDATE SET run_config_id = excluded.run_config_id,
         url = excluded.url, allow_public = excluded.allow_public, scheduled = excluded.scheduled,
         scheduled_active = 0, confirmed_at = excluded.confirmed_at",
    )
    .bind(project)
    .bind(&target.run_config_id)
    .bind(&target.url)
    .bind(target.allow_public)
    .bind(target.scheduled)
    .bind(&target.confirmed_at)
    .execute(pool)
    .await
    .map_err(db_error)?;
    Ok(target)
}

pub async fn revoke(pool: &DbPool, project: &str) -> Result<()> {
    sqlx::query("DELETE FROM ide_security_passive_targets WHERE project_id = ?")
        .bind(project)
        .execute(pool)
        .await
        .map_err(db_error)?;
    Ok(())
}

pub async fn run(
    target: &Target,
    root: &Path,
    cancel: &CancellationToken,
) -> Result<(EngineRun, Vec<Finding>)> {
    let (url, public) = parse_url(&target.url)?;
    if public != target.allow_public {
        return Err(Error::Forbidden("Confirm the audit target again.".into()));
    }
    let start = Instant::now();
    let request = async {
        let client = otto_netguard::pinned_client_builder_for(url.as_str(), !public)
            .await
            .map_err(Error::Forbidden)?
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| Error::Internal("could not construct passive audit client".into()))?;
        client.head(url.clone()).send().await.map_err(|_| {
            Error::Upstream("The confirmed service did not answer the passive audit.".into())
        })
    };
    let response = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err(crate::governor::cancelled()),
        result = tokio::time::timeout(Duration::from_secs(10), request) => {
            result.map_err(|_| Error::Upstream("The passive audit reached its time limit.".into()))??
        }
    };
    let status = response.status();
    if !status.is_success() {
        let reason = if status == StatusCode::METHOD_NOT_ALLOWED {
            "The confirmed URL does not support HEAD. Confirm a URL that accepts HEAD requests."
                .into()
        } else if status.is_redirection() {
            format!("The confirmed URL responded with HTTP {status}. Otto does not follow redirects. Confirm the final URL.")
        } else {
            format!("The confirmed URL responded with HTTP {status}. Use a URL that returns a successful unauthenticated HEAD response.")
        };
        return Err(Error::Upstream(reason));
    }
    let findings = inspect_headers(root, &url, response.headers());
    let sampled = now();
    Ok((
        EngineRun {
            name: "otto-passive-http".into(),
            version: EngineVersion {
                version: env!("CARGO_PKG_VERSION").into(),
                checked_at: sampled,
                updated_at: target.confirmed_at.clone(),
            },
            metrics: ScanMetrics {
                wall_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            },
            mode: ScanMode::Full,
        },
        findings,
    ))
}

fn inspect_headers(root: &Path, url: &Url, headers: &HeaderMap) -> Vec<Finding> {
    let html = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"));
    let csp = headers
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let frame_ancestors = csp.split(';').any(|directive| {
        directive
            .split_whitespace()
            .next()
            .is_some_and(|name| name.eq_ignore_ascii_case("frame-ancestors"))
    });
    let hsts = headers
        .get("strict-transport-security")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .filter_map(|part| part.split_once('='))
                .any(|(name, age)| {
                    name.trim().eq_ignore_ascii_case("max-age")
                        && age
                            .trim()
                            .trim_matches('"')
                            .parse::<u64>()
                            .is_ok_and(|seconds| seconds > 0)
                })
        });
    let cookies_insecure = headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|cookie| cookie.to_str().ok())
        .any(|cookie| {
            let keys: Vec<_> = cookie
                .split(';')
                .skip(1)
                .map(str::trim)
                .map(|part| {
                    part.split('=')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                })
                .collect();
            !keys.iter().any(|key| key == "samesite")
                || (url.scheme() == "https" && !keys.iter().any(|key| key == "secure"))
        });
    let checks = [
        (
            html && csp.is_empty(),
            "http.content-security-policy",
            Severity::Medium,
            "The HTML response has no Content-Security-Policy header.",
            "Set a Content-Security-Policy appropriate to the application.",
        ),
        (
            html && !headers.contains_key("x-frame-options") && !frame_ancestors,
            "http.frame-ancestors",
            Severity::Low,
            "The HTML response does not restrict browser embedding.",
            "Set a frame-ancestors policy for the origins you allow.",
        ),
        (
            html && !headers
                .get("x-content-type-options")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("nosniff")),
            "http.content-type-options",
            Severity::Low,
            "The response does not tell browsers to avoid MIME type guessing.",
            "Send X-Content-Type-Options with the value nosniff.",
        ),
        (
            url.scheme() == "https" && !hsts,
            "http.strict-transport-security",
            Severity::Low,
            "The HTTPS response has no enabled Strict-Transport-Security policy.",
            "Enable HSTS if this service will always be available over HTTPS.",
        ),
        (
            cookies_insecure,
            "http.cookie-attributes",
            Severity::Low,
            "A response cookie is missing an explicit SameSite policy or HTTPS Secure flag.",
            "Review cookie attributes and apply SameSite and Secure where appropriate.",
        ),
    ];
    let root = root.to_string_lossy().into_owned();
    checks
        .into_iter()
        .filter(|check| check.0)
        .map(|(_, rule, severity, description, fix)| {
            let identity = format!("http\\0{root}\\0{url}\\0{rule}");
            Finding {
                fingerprint: hex::encode(Sha256::digest(identity.as_bytes())),
                engine: "otto-passive-http".into(),
                check: Check::Dast,
                severity,
                root: root.clone(),
                path: url.to_string(),
                line: None,
                rule: rule.into(),
                title: description.into(),
                description: description.into(),
                fix: fix.into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn consent_parser_limits_input_to_an_explicit_safe_destination() {
        assert!(parse_url("http://127.0.0.1:3000/").is_ok_and(|(_, public)| !public));
        assert!(parse_url("http://10.0.0.2:3000/").is_ok_and(|(_, public)| !public));
        assert!(parse_url("https://example.org/").is_ok_and(|(_, public)| public));
        for url in [
            "http://169.254.169.254/",
            "http://127.0.0.1:7700/",
            "http://example.org/",
            "http://127.0.0.1:3000/?token=sample",
            "http://user@127.0.0.1:3000/",
            "file:///private",
        ] {
            assert!(parse_url(url).is_err(), "accepted {url}");
        }
    }

    #[tokio::test]
    async fn a_public_url_needs_separate_consent_and_exact_url_match() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO ide_named_projects (id) VALUES ('audit-project')")
            .execute(&pool)
            .await
            .unwrap();
        let candidate = Candidate {
            run_config_id: "cfg".into(),
            run_config_name: "Test".into(),
            url: "https://example.org/service".into(),
            public: true,
        };
        let mut request = ConfirmTarget {
            run_config_id: "cfg".into(),
            url: candidate.url.clone(),
            allow_public: false,
            scheduled: false,
        };
        assert!(matches!(
            confirm(&pool, "audit-project", candidate.clone(), request).await,
            Err(Error::Forbidden(_))
        ));
        request = ConfirmTarget {
            run_config_id: "cfg".into(),
            url: "https://example.org/changed".into(),
            allow_public: true,
            scheduled: false,
        };
        assert!(matches!(
            confirm(&pool, "audit-project", candidate, request).await,
            Err(Error::Conflict(_))
        ));
        assert!(get(&pool, "audit-project").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn confirm_target_rejects_scheduled_active_and_clears_legacy_flag_across_rollback() {
        // SEC-1: scheduled_active cannot be enabled via ConfirmTarget.
        let payload = serde_json::json!({
            "run_config_id": "cfg-a",
            "url": "http://127.0.0.1:3000/a",
            "allow_public": false,
            "scheduled": true,
            "scheduled_active": true,
        });
        assert!(
            serde_json::from_value::<ConfirmTarget>(payload).is_err(),
            "scheduled_active must be rejected when scheduled active DAST is disabled"
        );

        // DATA-1: simulate an older rc5 row with scheduled_active = 1 on target A,
        // followed by a rollback legacy upsert for target B (which leaves the
        // additive scheduled_active column at 1 in SQLite).
        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO ide_named_projects (id) VALUES ('rollback-project')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO ide_security_passive_targets
             (project_id, run_config_id, url, allow_public, scheduled, scheduled_active, confirmed_at)
             VALUES ('rollback-project', 'cfg-a', 'http://127.0.0.1:3000/a', 0, 1, 1, '2026-11-27T10:00:00.000Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // Legacy pre-0241 upsert replaces url and run_config_id without touching scheduled_active.
        sqlx::query(
            "INSERT INTO ide_security_passive_targets
             (project_id, run_config_id, url, allow_public, scheduled, confirmed_at)
             VALUES ('rollback-project', 'cfg-b', 'http://127.0.0.1:4000/b', 0, 1, '2026-11-27T11:00:00.000Z')
             ON CONFLICT(project_id) DO UPDATE SET run_config_id = excluded.run_config_id,
             url = excluded.url, allow_public = excluded.allow_public, scheduled = excluded.scheduled,
             confirmed_at = excluded.confirmed_at",
        )
        .execute(&pool)
        .await
        .unwrap();

        let loaded = get(&pool, "rollback-project").await.unwrap().unwrap();
        assert_eq!(loaded.run_config_id, "cfg-b");
        assert_eq!(loaded.url, "http://127.0.0.1:4000/b");
        // Serialized Target exposes no scheduled_active consent to callers.
        let json = serde_json::to_value(&loaded).unwrap();
        assert!(json.get("scheduled_active").is_none());

        // Reconfirming target B explicitly resets the additive column back to 0.
        confirm(
            &pool,
            "rollback-project",
            Candidate {
                run_config_id: "cfg-b".into(),
                run_config_name: "B".into(),
                url: "http://127.0.0.1:4000/b".into(),
                public: false,
            },
            ConfirmTarget {
                run_config_id: "cfg-b".into(),
                url: "http://127.0.0.1:4000/b".into(),
                allow_public: false,
                scheduled: true,
            },
        )
        .await
        .unwrap();
        let stored_flag: i64 = sqlx::query_scalar(
            "SELECT scheduled_active FROM ide_security_passive_targets WHERE project_id = 'rollback-project'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(stored_flag, 0);
    }

    #[test]
    fn active_run_request_requires_exact_displayed_confirmation_identity() {
        let target_a = Target {
            run_config_id: "cfg-a".into(),
            url: "http://127.0.0.1:3000/a".into(),
            allow_public: false,
            scheduled: false,
            confirmed_at: "2026-11-27T12:00:00.000Z".into(),
        };
        let target_b = Target {
            run_config_id: "cfg-b".into(),
            url: "http://127.0.0.1:4000/b".into(),
            allow_public: false,
            scheduled: false,
            confirmed_at: "2026-11-27T12:00:05.000Z".into(),
        };

        // Passive default (empty request) succeeds without target identity.
        assert!(RunDastRequest::default().verify_target(&target_a).is_ok());

        // Active request missing identity fails with Invalid.
        let bare_active: RunDastRequest = serde_json::from_str(r#"{"active":true}"#).unwrap();
        assert!(matches!(
            bare_active.verify_target(&target_a),
            Err(Error::Invalid(_))
        ));

        // Active request carrying target A's displayed confirmation succeeds on A...
        let bound_to_a = RunDastRequest {
            active: true,
            run_config_id: Some(target_a.run_config_id.clone()),
            url: Some(target_a.url.clone()),
            confirmed_at: Some(target_a.confirmed_at.clone()),
        };
        assert!(bound_to_a.verify_target(&target_a).is_ok());

        // ...but is rejected with Conflict if another window confirmed target B,
        // or if A was reconfirmed with a new timestamp.
        assert!(matches!(
            bound_to_a.verify_target(&target_b),
            Err(Error::Conflict(_))
        ));
        let reconfirmed_a = Target {
            confirmed_at: "2026-11-27T12:00:09.000Z".into(),
            ..target_a
        };
        assert!(matches!(
            bound_to_a.verify_target(&reconfirmed_a),
            Err(Error::Conflict(_))
        ));
    }

    async fn fixture(response: String) -> (Target, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("HEAD /app HTTP/1.1\r\n"));
            assert!(!request.to_ascii_lowercase().contains("authorization:"));
            assert!(!request.to_ascii_lowercase().contains("cookie:"));
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let target = Target {
            url: format!("http://{addr}/app"),
            run_config_id: "test".into(),
            allow_public: false,
            scheduled: false,
            confirmed_at: now(),
        };
        (target, server)
    }

    #[tokio::test]
    async fn live_passive_audit_sends_one_head_and_masks_cookie_values() {
        let (target, server) = fixture(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nSet-Cookie: test=masked-test-value\r\nContent-Length: 0\r\n\r\n".into()
        ).await;
        let (run, findings) = run(&target, Path::new("/test"), &CancellationToken::new())
            .await
            .unwrap();
        server.await.unwrap();
        assert!(findings
            .iter()
            .any(|item| item.rule == "http.cookie-attributes"));
        assert!(!crate::store::encode(&findings)
            .unwrap()
            .contains("masked-test-value"));
        println!(
            "passive HTTP loopback fixture: wall={} ms, {} findings, one HEAD",
            run.metrics.wall_ms,
            findings.len()
        );
    }

    #[tokio::test]
    async fn reports_unsuccessful_head_status_instead_of_clean_findings() {
        let redirect = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_addr = redirect.local_addr().unwrap();
        for (status, expected) in [
            ("405 Method Not Allowed", "does not support HEAD"),
            ("401 Unauthorized", "401"),
            ("404 Not Found", "404"),
            ("500 Internal Server Error", "500"),
            ("302 Found", "redirect"),
        ] {
            let (target, server) = fixture(format!(
                "HTTP/1.1 {status}\r\nLocation: http://{redirect_addr}/\r\nContent-Length: 0\r\n\r\n"
            )).await;
            let audit = run(&target, Path::new("/test"), &CancellationToken::new()).await;
            server.await.unwrap();
            assert!(
                matches!(audit, Err(Error::Upstream(message)) if message.contains(expected)),
                "audit accepted {status} or failed without an actionable message"
            );
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), redirect.accept())
                .await
                .is_err()
        );
    }
}
