use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::{Error, Result};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::download::io_error;
use crate::engine::{fresh, read_json, write_json, Engine};
use crate::governor::{self, cancelled, NetworkScope};
use crate::passive::{self, Target};
use crate::report::{text, MAX_FINDINGS};
use crate::{Check, EngineRun, Finding, ScanMetrics, ScanMode, ScanSettings, Severity};

const TEMPLATES_REFRESH_HOURS: i64 = 24;
const RATE_LIMIT_PER_SEC: u32 = 20;
const REQUEST_TIMEOUT_SECS: u32 = 5;
const MAX_HOST_ERRORS: u32 = 30;
const EXCLUDED_TAGS: &str = "dos,fuzz,fuzzing,intrusive,bruteforce,brute-force";
const SAFE_SEVERITIES: &str = "low,medium,high,critical";
const DEV_TEMPLATE_SUBDIRS: &[&str] = &[
    "http/exposures/configs",
    "http/exposures/files",
    "http/exposures/apis",
    "http/exposures/logs",
];
const SECRET_TEMPLATE_HINTS: &[&str] = &[
    "secret",
    "token",
    "credential",
    "password",
    "api-key",
    "apikey",
    "private-key",
    "access-key",
    "aws-key",
    "env-file",
    "dotenv",
    "key-exposure",
    "leak",
];

#[derive(Serialize, Deserialize)]
struct TemplatesState {
    checked_at: String,
}

pub(crate) async fn run<V, Fut>(
    engine: &Engine,
    target: &Target,
    root: &Path,
    settings: &ScanSettings,
    cancel: &CancellationToken,
    verify_before_spawn: V,
) -> Result<(EngineRun, Vec<Finding>)>
where
    V: Fn() -> Fut,
    Fut: Future<Output = Result<()>>,
{
    if !cfg!(target_os = "macos") {
        return Err(Error::Invalid(
            "Active DAST scans require macOS Seatbelt.".into(),
        ));
    }
    let (url, public) = passive::parse_url(&target.url)?;
    if public != target.allow_public {
        return Err(Error::Forbidden("Confirm the audit target again.".into()));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::Invalid("Enter an HTTP or HTTPS URL.".into()))?;
    // Fail fast on blocked/unreachable targets before initializing or downloading Nuclei.
    let _ = otto_netguard::resolve_checked_for(url.as_str(), !public)
        .await
        .map_err(Error::Forbidden)?;

    let timeout = Duration::from_secs(u64::from(settings.timeout_secs));
    let (version, program) = engine.prepare_binary(timeout, cancel).await?;
    let templates = refresh_templates(engine, &program, settings, cancel).await?;
    let templates = tokio::fs::canonicalize(&templates)
        .await
        .map_err(io_error)?;
    let template_dirs = allowlisted_template_dirs(&templates).await?;

    // Single probe-boundary consent gate: re-verify that human consent for this
    // exact target and its live run configuration still holds AFTER binary and
    // template preparation, immediately before opening the proxy or spawning Nuclei.
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    verify_before_spawn().await?;
    let (host, addrs) = otto_netguard::resolve_checked_for(url.as_str(), !public)
        .await
        .map_err(Error::Forbidden)?;

    let proxy = PinnedTargetProxy::start(host, port, addrs, cancel.child_token()).await?;
    let scratch = tempfile::tempdir_in(engine.dir()).map_err(io_error)?;
    let output = tokio::fs::canonicalize(scratch.path())
        .await
        .map_err(io_error)?;
    let args = nuclei_args(&url, &template_dirs, proxy.port(), settings);

    let findings = Arc::new(Mutex::new(Vec::new()));
    let sink = findings.clone();
    let root_str = root.to_string_lossy().into_owned();
    let fallback_url = url.clone();
    let run_result = governor::run_with_network_lines(
        &program,
        &args,
        root,
        &templates,
        &output,
        settings,
        NetworkScope::LoopbackPort(proxy.port()),
        cancel,
        move |line| {
            let mut guard = sink
                .lock()
                .map_err(|_| Error::Internal("security DAST report lock poisoned".into()))?;
            parse_jsonl_line(line, &fallback_url, &root_str, &mut guard)
        },
    )
    .await;
    drop(proxy);
    let metrics = run_result?;

    let mut findings = Arc::try_unwrap(findings)
        .map_err(|_| Error::Internal("security DAST report sink still held".into()))?
        .into_inner()
        .map_err(|_| Error::Internal("security DAST report lock poisoned".into()))?;
    findings.sort_unstable_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
    Ok((
        EngineRun {
            name: "nuclei".into(),
            version,
            metrics,
            mode: ScanMode::Full,
        },
        findings,
    ))
}

async fn refresh_templates(
    engine: &Engine,
    program: &Path,
    settings: &ScanSettings,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    let templates_dir = engine.dir().join("templates");
    let marker_path = engine.dir().join("templates.json");
    let existing: Option<TemplatesState> = read_json(&marker_path).await;
    let had_templates = has_templates(&templates_dir).await;
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    if had_templates
        && existing
            .as_ref()
            .is_some_and(|state| fresh(&state.checked_at, TEMPLATES_REFRESH_HOURS))
    {
        return Ok(templates_dir);
    }
    tokio::fs::create_dir_all(&templates_dir)
        .await
        .map_err(io_error)?;
    let templates_canonical = tokio::fs::canonicalize(&templates_dir)
        .await
        .map_err(io_error)?;
    let scratch = tempfile::tempdir_in(engine.dir()).map_err(io_error)?;
    let output = tokio::fs::canonicalize(scratch.path())
        .await
        .map_err(io_error)?;
    let args = vec![
        "-ut".into(),
        "-ud".into(),
        templates_canonical.to_string_lossy().into_owned(),
        "-nc".into(),
        "-silent".into(),
    ];
    let update_settings = ScanSettings {
        parallelism: 1,
        memory_mib: settings.memory_mib.max(512),
        timeout_secs: settings.timeout_secs.max(120),
        ..settings.clone()
    };
    let update = governor::run_with_network(
        program,
        &args,
        engine.dir(),
        &templates_canonical,
        &output,
        &update_settings,
        NetworkScope::TemplateUpdate,
        cancel,
    )
    .await;
    if cancel.is_cancelled() {
        return Err(cancelled());
    }
    let populated = has_templates(&templates_canonical).await;
    match update {
        Ok(_metrics) if populated => {
            write_json(
                &marker_path,
                &TemplatesState {
                    checked_at: crate::store::now(),
                },
            )
            .await?;
            Ok(templates_canonical)
        }
        Err(error) if had_templates => {
            tracing::warn!(%error, "Nuclei template refresh failed; keeping cached templates");
            write_json(
                &marker_path,
                &TemplatesState {
                    checked_at: crate::store::now(),
                },
            )
            .await?;
            Ok(templates_canonical)
        }
        Ok(_) => Err(Error::Upstream(
            "Nuclei template update produced no templates.".into(),
        )),
        Err(error) => {
            tracing::warn!(%error, "Nuclei template download failed");
            Err(Error::Upstream(
                "Could not download Nuclei templates into Otto's engine directory.".into(),
            ))
        }
    }
}

async fn has_templates(dir: &Path) -> bool {
    allowlisted_template_dirs(dir).await.is_ok()
}

async fn dir_has_yaml(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&current).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file()
                && matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some(ext) if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml")
                )
            {
                return true;
            }
        }
    }
    false
}

/// Validates that the cached Nuclei template tree contains every approved
/// dev-safe exposure directory with at least one YAML template. Never falls
/// back to the root template directory (prevents partial installs from
/// widening the scan scope).
async fn allowlisted_template_dirs(templates: &Path) -> Result<Vec<PathBuf>> {
    let mut selected_dirs = Vec::with_capacity(DEV_TEMPLATE_SUBDIRS.len());
    for subdir in DEV_TEMPLATE_SUBDIRS {
        let candidate = templates.join(subdir);
        if !tokio::fs::metadata(&candidate)
            .await
            .is_ok_and(|meta| meta.is_dir())
            || !dir_has_yaml(&candidate).await
        {
            return Err(Error::Upstream(
                "Nuclei template cache is missing the approved HTTP exposure templates. Retry the scan to refresh templates.".into(),
            ));
        }
        selected_dirs.push(candidate);
    }
    Ok(selected_dirs)
}

fn nuclei_args(
    url: &Url,
    template_dirs: &[PathBuf],
    proxy_port: u16,
    settings: &ScanSettings,
) -> Vec<String> {
    let concurrency = u32::from(settings.parallelism).clamp(1, 2) * 4;
    let mut args = vec!["-u".into(), url.to_string()];
    for dir in template_dirs {
        args.extend(["-t".into(), dir.to_string_lossy().into_owned()]);
    }
    args.extend([
        "-type".into(),
        "http".into(),
        "-severity".into(),
        SAFE_SEVERITIES.into(),
        "-etags".into(),
        EXCLUDED_TAGS.into(),
        "-no-interactsh".into(),
        "-disable-update-check".into(),
        "-omit-raw".into(),
        "-omit-template".into(),
        "-no-stdin".into(),
        "-no-color".into(),
        "-silent".into(),
        "-jsonl".into(),
        "-rate-limit".into(),
        RATE_LIMIT_PER_SEC.to_string(),
        "-concurrency".into(),
        concurrency.to_string(),
        "-bulk-size".into(),
        "1".into(),
        "-timeout".into(),
        REQUEST_TIMEOUT_SECS.to_string(),
        "-retries".into(),
        "0".into(),
        "-max-host-error".into(),
        MAX_HOST_ERRORS.to_string(),
        "-proxy".into(),
        format!("http://127.0.0.1:{proxy_port}"),
        "-proxy-internal".into(),
    ]);
    args
}

/// Ephemeral loopback HTTP/CONNECT proxy that pins every outbound Nuclei
/// connection to the single confirmed `(host, port)` and its `otto-netguard`
/// vetted `SocketAddr` list. Any request for another host or port is refused
/// with 403 Forbidden without ever dialing.
struct PinnedTargetProxy {
    port: u16,
    stop: CancellationToken,
}

impl PinnedTargetProxy {
    async fn start(
        host: String,
        port: u16,
        addrs: Vec<SocketAddr>,
        stop: CancellationToken,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(io_error)?;
        let listen_port = listener.local_addr().map_err(io_error)?.port();
        let allowed_host = host.to_ascii_lowercase();
        let addrs = std::sync::Arc::new(addrs);
        let loop_stop = stop.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = loop_stop.cancelled() => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else {
                            break;
                        };
                        let allowed_host = allowed_host.clone();
                        let addrs = addrs.clone();
                        let conn_stop = loop_stop.clone();
                        tokio::spawn(async move {
                            let _ = tokio::select! {
                                biased;
                                _ = conn_stop.cancelled() => Ok(()),
                                res = handle_proxy_conn(stream, &allowed_host, port, &addrs) => res,
                            };
                        });
                    }
                }
            }
        });
        Ok(Self {
            port: listen_port,
            stop,
        })
    }

    fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for PinnedTargetProxy {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn handle_proxy_conn(
    mut client: TcpStream,
    allowed_host: &str,
    allowed_port: u16,
    addrs: &[SocketAddr],
) -> std::io::Result<()> {
    const MAX_HEADER_BYTES: usize = 64 * 1024;
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if buf.len() >= MAX_HEADER_BYTES {
            let _ = client
                .write_all(
                    b"HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\n\r\n",
                )
                .await;
            return Ok(());
        }
        let n = tokio::time::timeout(Duration::from_secs(10), client.read(&mut chunk))
            .await
            .map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "proxy read timeout")
            })??;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
    };
    let head = std::str::from_utf8(&buf[..header_end]).unwrap_or_default();
    let tail = &buf[header_end + 4..];
    let mut lines = head.split("\r\n");
    let Some(request_line) = lines.next() else {
        return reject_proxy(&mut client).await;
    };
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return reject_proxy(&mut client).await;
    };
    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = parse_authority(target) else {
            return reject_proxy(&mut client).await;
        };
        if !host.eq_ignore_ascii_case(allowed_host) || port != allowed_port {
            return reject_proxy(&mut client).await;
        }
        let Some(mut upstream) = dial_pinned(addrs).await else {
            let _ = client
                .write_all(
                    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            return Ok(());
        };
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        if !tail.is_empty() {
            upstream.write_all(tail).await?;
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(30),
            tokio::io::copy_bidirectional(&mut client, &mut upstream),
        )
        .await;
        return Ok(());
    }

    let Ok(parsed) = Url::parse(target) else {
        return reject_proxy(&mut client).await;
    };
    if parsed.scheme() != "http" {
        return reject_proxy(&mut client).await;
    }
    let bare = parsed
        .host_str()
        .map(|h| h.trim_start_matches('[').trim_end_matches(']'))
        .unwrap_or_default();
    if !bare.eq_ignore_ascii_case(allowed_host)
        || parsed.port_or_known_default() != Some(allowed_port)
    {
        return reject_proxy(&mut client).await;
    }
    let Some(mut upstream) = dial_pinned(addrs).await else {
        let _ = client
            .write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        return Ok(());
    };
    let origin_target = match parsed.query() {
        Some(q) => format!("{}?{q}", parsed.path()),
        None => parsed.path().to_string(),
    };
    let mut forwarded = format!("{method} {origin_target} {version}\r\n");
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("proxy-connection:") || lower.starts_with("connection:") {
            continue;
        }
        forwarded.push_str(line);
        forwarded.push_str("\r\n");
    }
    forwarded.push_str("Connection: close\r\n\r\n");
    upstream.write_all(forwarded.as_bytes()).await?;
    if !tail.is_empty() {
        upstream.write_all(tail).await?;
    }
    let (mut cr, mut cw) = client.split();
    let (mut ur, mut uw) = upstream.split();
    let _ = tokio::time::timeout(Duration::from_secs(15), async {
        let upload = async {
            let _ = tokio::io::copy(&mut cr, &mut uw).await;
            let _ = uw.shutdown().await;
        };
        let download = async {
            let _ = tokio::io::copy(&mut ur, &mut cw).await;
            let _ = cw.shutdown().await;
        };
        tokio::pin!(upload);
        tokio::pin!(download);
        tokio::select! {
            biased;
            _ = &mut download => {}
            _ = &mut upload => {
                download.await;
            }
        }
    })
    .await;
    Ok(())
}

async fn reject_proxy(client: &mut TcpStream) -> std::io::Result<()> {
    client
        .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await
}

async fn dial_pinned(addrs: &[SocketAddr]) -> Option<TcpStream> {
    for addr in addrs {
        if let Ok(Ok(stream)) =
            tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await
        {
            return Some(stream);
        }
    }
    None
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn parse_authority(raw: &str) -> Option<(&str, u16)> {
    if let Some(rest) = raw.strip_prefix('[') {
        let (host, port_part) = rest.split_once("]:")?;
        let port = port_part.parse::<u16>().ok()?;
        Some((host, port))
    } else {
        let (host, port_part) = raw.rsplit_once(':')?;
        let port = port_part.parse::<u16>().ok()?;
        Some((host, port))
    }
}

#[derive(Deserialize)]
struct NucleiEvent {
    #[serde(rename = "template-id", default)]
    template_id: String,
    #[serde(rename = "matcher-name", default)]
    matcher_name: String,
    #[serde(rename = "matched-at", default)]
    matched_at: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    info: NucleiInfo,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct NucleiInfo {
    name: String,
    severity: String,
    description: String,
    remediation: String,
    #[serde(default)]
    tags: TagsField,
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum TagsField {
    #[default]
    None,
    List(Vec<String>),
    Csv(String),
}

impl TagsField {
    fn as_list(&self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::List(items) => items.clone(),
            Self::Csv(raw) => raw
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn parse_jsonl(data: &[u8], fallback_url: &Url, root: &Path) -> Result<Vec<Finding>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let content = std::str::from_utf8(data)
        .map_err(|_| Error::internal_public("Nuclei returned an unreadable report."))?;
    let root_str = root.to_string_lossy().into_owned();
    let mut findings = Vec::new();
    for line in content.lines() {
        parse_jsonl_line(line, fallback_url, &root_str, &mut findings)?;
    }
    Ok(findings)
}

pub(crate) fn parse_jsonl_line(
    line: &str,
    fallback_url: &Url,
    root_str: &str,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let event: NucleiEvent = serde_json::from_str(trimmed)
        .map_err(|_| Error::internal_public("Nuclei returned an unreadable report."))?;
    if event.template_id.trim().is_empty() {
        return Ok(());
    }
    if findings.len() == MAX_FINDINGS {
        return Err(Error::internal_public(
            "This scan produced too many findings. Narrow its scope and retry.",
        ));
    }
    let path = sanitize_matched_url(&event.matched_at, &event.host, fallback_url);
    let rule_raw = if event.matcher_name.trim().is_empty() {
        event.template_id.trim().to_string()
    } else {
        format!("{}:{}", event.template_id.trim(), event.matcher_name.trim())
    };
    let rule = text(&rule_raw, 180);
    let severity = match event.info.severity.trim().to_ascii_lowercase().as_str() {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Unknown,
    };
    let tags = event.info.tags.as_list();
    let secret_like = is_secret_template(&event.template_id, &tags);
    let title_src = if event.info.name.trim().is_empty() {
        rule.clone()
    } else {
        otto_core::redact::redact_text(event.info.name.trim()).value
    };
    let title = text(&title_src, 240);
    let description = if secret_like {
        "Nuclei matched a sensitive exposure or credential template. Matched response content is omitted."
            .into()
    } else if event.info.description.trim().is_empty() {
        title.clone()
    } else {
        text(
            &otto_core::redact::redact_text(event.info.description.trim()).value,
            900,
        )
    };
    let fix = if event.info.remediation.trim().is_empty() {
        if secret_like {
            "Restrict access to this endpoint, remove exposed credentials, and rotate any leaked secrets."
                .into()
        } else {
            "Review the matched endpoint configuration and remediate the reported issue.".into()
        }
    } else {
        text(
            &otto_core::redact::redact_text(event.info.remediation.trim()).value,
            600,
        )
    };
    let identity = format!("nuclei\0{root_str}\0{path}\0{rule}");
    findings.push(Finding {
        fingerprint: hex::encode(Sha256::digest(identity.as_bytes())),
        engine: "nuclei".into(),
        check: Check::Dast,
        severity,
        root: root_str.to_owned(),
        path,
        line: None,
        rule,
        title,
        description,
        fix,
    });
    Ok(())
}

fn sanitize_matched_url(matched_at: &str, host: &str, fallback: &Url) -> String {
    let candidate = if !matched_at.trim().is_empty() {
        matched_at.trim()
    } else {
        host.trim()
    };
    if let Ok(mut parsed) = Url::parse(candidate) {
        if matches!(parsed.scheme(), "http" | "https") {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.set_query(None);
            parsed.set_fragment(None);
            return text(parsed.as_str(), 512);
        }
    }
    fallback.to_string()
}

fn is_secret_template(template_id: &str, tags: &[String]) -> bool {
    let normalized_id = template_id.to_ascii_lowercase().replace('_', "-");
    if SECRET_TEMPLATE_HINTS
        .iter()
        .any(|hint| normalized_id.contains(hint))
    {
        return true;
    }
    tags.iter().any(|tag| {
        let normalized = tag.to_ascii_lowercase().replace('_', "-");
        SECRET_TEMPLATE_HINTS
            .iter()
            .any(|hint| normalized.contains(hint))
    })
}

/// Ensures `ScanMetrics` stays imported in non-macOS builds if needed.
#[allow(dead_code)]
fn _metrics_type_check(_: ScanMetrics) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineKind;
    use crate::store;

    #[test]
    fn parse_jsonl_normalizes_findings_and_redacts_secrets_and_urls() {
        let fallback = Url::parse("http://127.0.0.1:3000/").unwrap();
        let fixture = concat!(
            r#"{"template-id":"git-config","matcher-name":"git","matched-at":"http://user:pass@127.0.0.1:3000/.git/config?token=SUPER_SECRET_QUERY#frag","host":"http://127.0.0.1:3000","info":{"name":"Git Config Exposure","severity":"medium","description":"Git config exposed with token ghp_1234567890abcdefghijklmnopqrstuvwxyz","remediation":"Block /.git/ paths.","tags":["exposure","config"]},"request":"GET /.git/config HTTP/1.1\r\nAuthorization: Bearer RAW_AUTH_TOKEN\r\n\r\n","response":"[core]\n\tpassword = RAW_RESPONSE_SECRET","extracted-results":["RAW_EXTRACTED_KEY"],"curl-command":"curl http://127.0.0.1:3000/.git/config"}"#,
            "\n",
            r#"{"template-id":"aws-secret-key-exposure","matched-at":"http://127.0.0.1:3000/env","info":{"name":"AWS Key Leak","severity":"critical","description":"Found AKIAIOSFODNN7EXAMPLE in response","tags":"exposure,token,aws"},"extracted-results":["AKIAIOSFODNN7EXAMPLE"]}"#,
            "\n",
            r#"{"template-id":"cors-misconfig","matched-at":"http://127.0.0.1:3000/api","info":{"name":"CORS Misconfiguration","severity":"high","description":"Origin reflected.","remediation":"Restrict Access-Control-Allow-Origin.","tags":["misconfig"]}}"#,
            "\n",
        );

        let findings = parse_jsonl(fixture.as_bytes(), &fallback, Path::new("/project")).unwrap();
        assert_eq!(findings.len(), 3);
        assert_eq!(findings[0].engine, "nuclei");
        assert_eq!(findings[0].check, Check::Dast);
        assert_eq!(findings[0].severity, Severity::Medium);
        assert_eq!(findings[0].rule, "git-config:git");
        assert_eq!(findings[0].path, "http://127.0.0.1:3000/.git/config");
        assert_eq!(findings[0].line, None);
        assert_eq!(findings[1].severity, Severity::Critical);
        assert!(findings[1]
            .description
            .contains("Matched response content is omitted"));
        assert_eq!(findings[2].severity, Severity::High);
        assert_eq!(findings[2].fix, "Restrict Access-Control-Allow-Origin.");

        let encoded = serde_json::to_string(&findings).unwrap();
        for forbidden in [
            "SUPER_SECRET_QUERY",
            "user:pass",
            "RAW_AUTH_TOKEN",
            "RAW_RESPONSE_SECRET",
            "RAW_EXTRACTED_KEY",
            "ghp_1234567890abcdefghijklmnopqrstuvwxyz",
            "AKIAIOSFODNN7EXAMPLE",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "leaked sensitive token {forbidden} in {encoded}"
            );
        }
    }

    fn create_allowlisted_templates(templates: &Path) {
        for subdir in DEV_TEMPLATE_SUBDIRS {
            let dir = templates.join(subdir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("stub.yaml"), "id: stub\n").unwrap();
        }
    }

    #[cfg(target_os = "macos")]
    fn setup_cached_nuclei(
        tools: &Path,
        script: &str,
        populate_allowlist: bool,
    ) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let nuclei_dir = tools.join("security-nuclei");
        let binary = nuclei_dir.join("releases/3.11.1/nuclei");
        let templates = nuclei_dir.join("templates");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&templates).unwrap();
        if populate_allowlist {
            create_allowlisted_templates(&templates);
        }
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        (nuclei_dir, binary)
    }

    #[cfg(target_os = "macos")]
    async fn write_fresh_nuclei_markers(nuclei_dir: &Path) {
        let stamp = store::now();
        write_json(
            &nuclei_dir.join("engine.json"),
            &crate::EngineVersion {
                version: "3.11.1".into(),
                checked_at: stamp.clone(),
                updated_at: stamp.clone(),
            },
        )
        .await
        .unwrap();
        write_json(&nuclei_dir.join("release-attempt.json"), &stamp)
            .await
            .unwrap();
        write_json(
            &nuclei_dir.join("templates.json"),
            &TemplatesState { checked_at: stamp },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn nuclei_args_enforce_safe_dev_defaults_and_rate_limits() {
        let dir = tempfile::tempdir().unwrap();
        create_allowlisted_templates(dir.path());
        let template_dirs = allowlisted_template_dirs(dir.path()).await.unwrap();
        let url = Url::parse("http://127.0.0.1:3000/app").unwrap();
        let settings = ScanSettings {
            parallelism: 1,
            ..Default::default()
        };
        let args = nuclei_args(&url, &template_dirs, 41234, &settings);
        let pair = |flag: &str| {
            args.windows(2)
                .find(|w| w[0] == flag)
                .map(|w| w[1].as_str())
        };
        assert_eq!(pair("-type"), Some("http"));
        assert_eq!(pair("-severity"), Some(SAFE_SEVERITIES));
        assert_eq!(pair("-etags"), Some(EXCLUDED_TAGS));
        assert_eq!(pair("-rate-limit"), Some("20"));
        assert_eq!(pair("-concurrency"), Some("4"));
        assert_eq!(pair("-proxy"), Some("http://127.0.0.1:41234"));
        assert!(args.iter().any(|a| a == "-proxy-internal"));
        assert!(args.iter().any(|a| a == "-no-interactsh"));
        assert!(args.iter().any(|a| a == "-disable-update-check"));
        assert!(args.iter().any(|a| a == "-omit-raw"));
        assert!(args.iter().any(|a| a == "-omit-template"));
        assert!(args.iter().any(|a| a == "-jsonl"));
        assert!(
            !args.iter().any(|a| a == "-jsonl-export"),
            "Nuclei must stream JSONL over stdout, never write -jsonl-export to disk"
        );
        for subdir in DEV_TEMPLATE_SUBDIRS {
            assert!(args.iter().any(|a| a.ends_with(subdir)));
        }
    }

    #[tokio::test]
    async fn pinned_proxy_forwards_confirmed_target_and_blocks_other_hosts_and_ports() {
        let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_addr = target_listener.local_addr().unwrap();
        let decoy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let decoy_addr = decoy_listener.local_addr().unwrap();

        let target_server = tokio::spawn(async move {
            let (mut stream, _) = target_listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut byte = [0u8; 1];
            while !req.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                req.push(byte[0]);
            }
            let text = String::from_utf8(req).unwrap();
            assert!(text.starts_with("GET /hello HTTP/1.1\r\n"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });

        let proxy = PinnedTargetProxy::start(
            "127.0.0.1".into(),
            target_addr.port(),
            vec![target_addr],
            CancellationToken::new(),
        )
        .await
        .unwrap();

        // Allowed request succeeds through the pinned proxy.
        let mut allowed = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        allowed
            .write_all(
                format!(
                    "GET http://127.0.0.1:{}/hello HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
                    target_addr.port(),
                    target_addr.port()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut resp = String::new();
        allowed.read_to_string(&mut resp).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.ends_with("ok"));
        target_server.await.unwrap();

        // Disallowed targets (decoy port, Otto daemon port, cloud metadata, CONNECT) get 403.
        for bad_request in [
            format!(
                "GET http://127.0.0.1:{}/secret HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
                decoy_addr.port(),
                decoy_addr.port()
            ),
            "GET http://127.0.0.1:7700/api/v1/health HTTP/1.1\r\nHost: 127.0.0.1:7700\r\n\r\n"
                .into(),
            "GET http://169.254.169.254/latest/meta-data/ HTTP/1.1\r\nHost: 169.254.169.254\r\n\r\n"
                .into(),
            format!("CONNECT 127.0.0.1:{} HTTP/1.1\r\n\r\n", decoy_addr.port()),
        ] {
            let mut conn = TcpStream::connect(("127.0.0.1", proxy.port()))
                .await
                .unwrap();
            conn.write_all(bad_request.as_bytes()).await.unwrap();
            let mut reply = String::new();
            conn.read_to_string(&mut reply).await.unwrap();
            assert!(
                reply.starts_with("HTTP/1.1 403 Forbidden"),
                "expected 403 for {bad_request:?}, got {reply:?}"
            );
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(80), decoy_listener.accept())
                .await
                .is_err(),
            "decoy listener received a connection through the pinned proxy"
        );
    }

    #[tokio::test]
    async fn active_scan_refuses_unconfirmed_or_blocked_targets_without_spawning_nuclei() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(dir.path(), EngineKind::Nuclei);
        let cancel = CancellationToken::new();
        let settings = ScanSettings::default();
        for (url, allow_public) in [
            ("http://127.0.0.1:7700/", false),
            ("http://169.254.169.254/", false),
            ("http://example.org/", true),
            ("https://example.org/", false),  // public mismatch
            ("http://127.0.0.1:3000/", true), // local/public mismatch
        ] {
            let target = Target {
                run_config_id: "cfg".into(),
                url: url.into(),
                allow_public,
                scheduled: false,
                confirmed_at: store::now(),
            };
            assert!(
                run(
                    &engine,
                    &target,
                    Path::new("/project"),
                    &settings,
                    &cancel,
                    || async { Ok(()) }
                )
                .await
                .is_err(),
                "expected refusal for {url} (allow_public={allow_public})"
            );
        }
        assert!(
            !engine.dir().exists(),
            "refused target must never initialize or spawn nuclei"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn partial_template_cache_without_allowlist_dirs_is_rejected_without_spawning_nuclei() {
        // BUG-1: a cancelled first template extraction leaves non-allowlist files
        // (e.g. http/cves/vuln.yaml) in templates/. Neither cache detection nor
        // argument construction may fall back to the root template directory.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let tools = root.join("tools");
        let probe_marker = root.join("nuclei-probed");
        // Stub fails template refresh (-ut) and would write probe_marker if invoked for probing.
        let script = format!(
            "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = \"-ut\" ]; then exit 1; fi\ndone\ntouch '{}'\n",
            probe_marker.display()
        );
        let (nuclei_dir, _) = setup_cached_nuclei(&tools, &script, false);
        let cves = nuclei_dir.join("templates/http/cves");
        std::fs::create_dir_all(&cves).unwrap();
        std::fs::write(cves.join("vuln.yaml"), "id: cve-test\n").unwrap();
        write_fresh_nuclei_markers(&nuclei_dir).await;

        assert!(
            !has_templates(&nuclei_dir.join("templates")).await,
            "partial cache without approved exposure subdirs must not count as cached templates"
        );
        assert!(
            allowlisted_template_dirs(&nuclei_dir.join("templates"))
                .await
                .is_err(),
            "missing approved exposure dirs must return an error instead of falling back to root"
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let engine = Engine::new(&tools, EngineKind::Nuclei);
        let target = Target {
            run_config_id: "cfg".into(),
            url: format!("http://{addr}/app"),
            allow_public: false,
            scheduled: false,
            confirmed_at: store::now(),
        };
        let err = run(
            &engine,
            &target,
            &root,
            &ScanSettings::default(),
            &CancellationToken::new(),
            || async { Ok(()) },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, Error::Upstream(_)),
            "expected template-cache Upstream error, got: {err:?}"
        );
        assert!(
            !probe_marker.exists(),
            "Nuclei probe must never spawn against a partial template cache"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn revoking_consent_during_preparation_blocks_nuclei_spawn() {
        // CONC-1: consent is valid before preparation starts, then revoked before
        // Nuclei is spawned. The probe-boundary verify callback must prevent
        // Nuclei from spawning or touching the target.
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let tools = root.join("tools");
        let spawn_marker = root.join("nuclei-spawned");
        let script = format!("#!/bin/sh\ntouch '{}'\n", spawn_marker.display());
        let (nuclei_dir, _) = setup_cached_nuclei(&tools, &script, true);
        write_fresh_nuclei_markers(&nuclei_dir).await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let engine = Engine::new(&tools, EngineKind::Nuclei);
        let target = Target {
            run_config_id: "cfg".into(),
            url: format!("http://{addr}/app"),
            allow_public: false,
            scheduled: false,
            confirmed_at: store::now(),
        };

        let revoked = Arc::new(AtomicBool::new(true));
        let checks = Arc::new(AtomicUsize::new(0));
        let revoked_flag = revoked.clone();
        let checks_counter = checks.clone();
        let err = run(
            &engine,
            &target,
            &root,
            &ScanSettings::default(),
            &CancellationToken::new(),
            move || {
                let revoked = revoked_flag.clone();
                let checks = checks_counter.clone();
                async move {
                    checks.fetch_add(1, Ordering::SeqCst);
                    if revoked.load(Ordering::SeqCst) {
                        Err(Error::Conflict(
                            "The confirmed target or its running configuration changed.".into(),
                        ))
                    } else {
                        Ok(())
                    }
                }
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "got {err:?}");
        assert_eq!(checks.load(Ordering::SeqCst), 1);
        assert!(
            !spawn_marker.exists(),
            "Nuclei must not spawn after consent is revoked during preparation"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(80), listener.accept())
                .await
                .is_err(),
            "target must receive zero connections when spawn-boundary consent fails"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn nuclei_streams_jsonl_over_stdout_without_writing_extracted_credentials_to_disk() {
        // SEC-3: Nuclei outputs JSONL containing extracted credentials via stdout
        // (-jsonl). Otto drops extracted-results in memory and never writes a
        // temporary report file to disk.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let tools = root.join("tools");
        let secret = "SUPER_SECRET_GIT_CREDENTIAL_9999";
        let script = format!(
            r#"#!/bin/sh
SAW_JSONL=0
TARGET=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -u) TARGET="$2"; shift 2 ;;
    -jsonl) SAW_JSONL=1; shift ;;
    -jsonl-export) exit 99 ;;
    *) shift ;;
  esac
done
[ "$SAW_JSONL" -eq 1 ] || exit 98
printf '%s\n' '{{"template-id":"git-config","matched-at":"'"$TARGET"'/.git/config?token={secret}","info":{{"name":"Git Config Exposure","severity":"medium","description":"Git config exposed","tags":["exposure","config"]}},"extracted-results":["https://user:{secret}@git.example.invalid/repo.git"]}}'
"#
        );
        let (nuclei_dir, _) = setup_cached_nuclei(&tools, &script, true);
        write_fresh_nuclei_markers(&nuclei_dir).await;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let service = crate::SecurityScans::new(&tools);
        let target = Target {
            run_config_id: "cfg".into(),
            url: format!("http://{addr}/app"),
            allow_public: false,
            scheduled: false,
            confirmed_at: store::now(),
        };
        let (run_info, findings) = service
            .run_dast(
                &target,
                &root,
                &ScanSettings::default(),
                true,
                &CancellationToken::new(),
                || async { Ok(()) },
            )
            .await
            .unwrap();
        assert_eq!(run_info.name, "nuclei");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "git-config");
        assert!(!serde_json::to_string(&findings).unwrap().contains(secret));

        // Walk engine directory and verify no report file or leaked secret exists on disk.
        let mut stack = vec![nuclei_dir];
        while let Some(path) = stack.pop() {
            let mut entries = tokio::fs::read_dir(&path).await.unwrap();
            while let Some(entry) = entries.next_entry().await.unwrap() {
                let p = entry.path();
                if entry.file_type().await.unwrap().is_dir() {
                    stack.push(p);
                } else {
                    assert_ne!(
                        p.file_name().and_then(|n| n.to_str()),
                        Some("report.jsonl"),
                        "no temporary report.jsonl file may be written"
                    );
                    if p.file_name().and_then(|n| n.to_str()) != Some("nuclei") {
                        let bytes = tokio::fs::read(&p).await.unwrap_or_default();
                        assert!(
                            !String::from_utf8_lossy(&bytes).contains(secret),
                            "extracted credential found on disk at {}",
                            p.display()
                        );
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn active_dast_failure_persists_failed_scan_status_and_error() {
        // OBS-1: active Nuclei failure must persist ScanStatus::Failed with an
        // operator-visible error message rather than Completed with error: None.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let tools = root.join("tools");
        let (nuclei_dir, _) = setup_cached_nuclei(&tools, "#!/bin/sh\nexit 1\n", true);
        write_fresh_nuclei_markers(&nuclei_dir).await;

        let pool = otto_state::db::test_pool().await;
        sqlx::query("INSERT INTO ide_named_projects (id) VALUES ('obs-project')")
            .execute(&pool)
            .await
            .unwrap();
        let service = Arc::new(crate::SecurityScans::new(&tools));
        let runner = service.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let target = Target {
            run_config_id: "cfg".into(),
            url: format!("http://{addr}/app"),
            allow_public: false,
            scheduled: false,
            confirmed_at: store::now(),
        };
        let root_clone = root.clone();
        let scan = service
            .enqueue_job(
                pool.clone(),
                "obs-project".into(),
                || async { false },
                move |settings, cancel, _| async move {
                    let (run, findings) = runner
                        .run_dast(&target, &root_clone, &settings, true, &cancel, || async {
                            Ok(())
                        })
                        .await?;
                    Ok((vec![run], findings))
                },
            )
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let detail = store::detail(&pool, "obs-project", &scan.id).await.unwrap();
                if detail.scan.status != crate::ScanStatus::Queued
                    && detail.scan.status != crate::ScanStatus::Running
                {
                    assert_eq!(detail.scan.status, crate::ScanStatus::Failed);
                    assert!(
                        detail.scan.error.as_deref().is_some_and(|e| !e.is_empty()),
                        "failed active DAST scan must persist a non-empty error"
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn opt_in_off_never_spawns_nuclei_and_cancel_kills_nuclei_group() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let tools = root.join("tools");
        let service = crate::SecurityScans::new(&tools);
        let (nuclei_dir, binary) = setup_cached_nuclei(&tools, "#!/bin/sh\nexit 42\n", true);
        write_fresh_nuclei_markers(&nuclei_dir).await;

        // 1. Opt-in OFF -> passive HTTP audit runs, Nuclei is NEVER spawned
        //    (even if the nuclei binary is absent / fails immediately).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let passive_server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req = Vec::new();
            let mut b = [0u8; 1];
            while !req.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut b).await.unwrap();
                req.push(b[0]);
            }
            assert!(String::from_utf8(req)
                .unwrap()
                .starts_with("HEAD /app HTTP/1.1"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let target = Target {
            run_config_id: "cfg".into(),
            url: format!("http://{addr}/app"),
            allow_public: false,
            scheduled: false,
            confirmed_at: store::now(),
        };
        let settings = ScanSettings::default();
        let cancel = CancellationToken::new();
        let (passive_run, _) = service
            .run_dast(&target, &root, &settings, false, &cancel, || async {
                Ok(())
            })
            .await
            .unwrap();
        passive_server.await.unwrap();
        assert_eq!(passive_run.name, "otto-passive-http");

        // 2. Opt-in ON -> Nuclei runs under governor and streams JSONL over stdout.
        std::fs::write(
            &binary,
            r#"#!/bin/sh
TARGET=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -u) TARGET="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "$TARGET" in
  */hang)
    sleep 60 &
    wait
    ;;
  *)
    printf '%s\n' '{"template-id":"dev-exposure","matched-at":"'"$TARGET"'/debug","info":{"name":"Debug Endpoint","severity":"medium","description":"Debug route enabled."}}'
    ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (active_run, active_findings) = service
            .run_dast(&target, &root, &settings, true, &cancel, || async {
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(active_run.name, "nuclei");
        assert_eq!(active_findings.len(), 1);
        assert_eq!(active_findings[0].engine, "nuclei");
        assert_eq!(active_findings[0].check, Check::Dast);
        assert_eq!(active_findings[0].rule, "dev-exposure");

        // 3. Cancel kills the Nuclei process group immediately.
        let hang_target = Target {
            url: format!("http://{addr}/hang"),
            ..target
        };
        let cancel_hang = CancellationToken::new();
        let trigger = cancel_hang.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            trigger.cancel();
        });
        let err = service
            .run_dast(
                &hang_target,
                &root,
                &settings,
                true,
                &cancel_hang,
                || async { Ok(()) },
            )
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), cancelled().to_string());
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    #[ignore = "downloads real Nuclei binary and templates, then runs against a local throwaway HTTP server"]
    async fn live_nuclei_against_throwaway_http_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let server_stop = stop.clone();
        let server = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = server_stop.cancelled() => break,
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        tokio::spawn(async move {
                            let mut buf = [0u8; 4096];
                            let Ok(n) = stream.read(&mut buf).await else { return };
                            let req = String::from_utf8_lossy(&buf[..n]);
                            let first = req.lines().next().unwrap_or_default();
                            let path = first.split_whitespace().nth(1).unwrap_or("/");
                            let response = if path == "/.git/config" {
                                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n[remote \"origin\"]\n\turl = https://example.invalid/repo.git\n"
                            } else {
                                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            };
                            let _ = stream.write_all(response.as_bytes()).await;
                        });
                    }
                }
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let engine = Engine::new(&root.join("tools"), EngineKind::Nuclei);
        let target = Target {
            run_config_id: "live-test".into(),
            url: format!("http://{addr}"),
            allow_public: false,
            scheduled: false,
            confirmed_at: store::now(),
        };
        let settings = ScanSettings {
            parallelism: 1,
            memory_mib: 1024,
            timeout_secs: 180,
            ..Default::default()
        };
        let cancel = CancellationToken::new();
        let (run_info, findings) = run(&engine, &target, &root, &settings, &cancel, || async {
            Ok(())
        })
        .await
        .unwrap();
        stop.cancel();
        let _ = server.await;
        println!(
            "Nuclei {} live DAST on throwaway HTTP server: {} findings, wall={} ms, sampled peak CPU={:.1}%, peak RSS={} MiB",
            run_info.version.version,
            findings.len(),
            run_info.metrics.wall_ms,
            run_info.metrics.peak_cpu_percent,
            run_info.metrics.peak_rss_bytes / (1024 * 1024),
        );
        assert_eq!(run_info.name, "nuclei");
        assert!(
            findings.iter().any(|f| f.rule.contains("git-config")),
            "expected git-config exposure finding, got: {findings:?}"
        );
    }
}
