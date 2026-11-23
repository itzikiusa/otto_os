//! Extensible 3D asset generation drivers (zero ComfyUI):
//! - Tencent Hunyuan3D-Omni (`hunyuan3d_omni`)
//! - Microsoft TRELLIS.2 (`trellis_2`)
//! - Custom HTTP (`custom_http`)
//! - Custom Local CLI (`custom_cli`)

pub mod custom;
pub mod hunyuan3d_omni;
pub mod trellis2;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine;
use otto_core::{Error, Result};
use serde_json::Value;

use crate::net::{apply_auth_header, build_http_client, read_bounded_bytes, validate_endpoint_url};
use crate::types::{Media3dProviderRecord, ProviderTestResult};

const MAX_STDERR_TAIL_BYTES: usize = 8 * 1024;
const MAX_JSON_STATUS_BYTES: usize = 64 * 1024;
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long a stopped CLI gets between SIGTERM and SIGKILL. Otto Forge's own
/// clean-up (SIGTERM its child's group, 2 s, SIGKILL, 2 s) fits inside it.
const CLI_STOP_GRACE: Duration = Duration::from_secs(5);

/// The latest `(pct, label, stage)` a local CLI reported on stderr (`stage`:
/// the engine's own step key, Forge's `"concept"`, `"gameready"`…, or
/// empty); the job runner writes it to the job row (throttled).
pub type CliProgress = tokio::sync::watch::Sender<Option<(i64, String, String)>>;

#[derive(Debug, Clone)]
pub struct Gen3dRequest {
    pub job_id: String,
    pub pipeline_stage: String,
    pub prompt: Option<String>,
    pub image_bytes: Option<Vec<u8>>,
    pub image_mime: Option<String>,
    pub source_mesh_bytes: Option<Vec<u8>>,
    pub source_mesh_format: Option<String>,
    pub output_format: String,
    pub poly_budget: Option<u32>,
    pub seed: Option<i64>,
    pub params: Value,
    pub max_output_bytes: usize,
    pub timeout: Duration,
    pub progress: Option<CliProgress>,
}

#[derive(Debug, Clone)]
pub struct Gen3dOutput {
    pub bytes: Vec<u8>,
    pub format: String,
    pub manifest: Option<Value>,
}

#[async_trait]
pub trait Gen3dDriver: Send + Sync {
    async fn generate(
        &self,
        req: &Gen3dRequest,
        provider: &Media3dProviderRecord,
        api_key: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Gen3dOutput>;

    async fn test_connection(
        &self,
        provider: &Media3dProviderRecord,
        api_key: Option<&str>,
    ) -> Result<ProviderTestResult>;
}

pub fn driver_for_kind(kind: &str) -> Arc<dyn Gen3dDriver> {
    match kind {
        "hunyuan3d_omni" => Arc::new(hunyuan3d_omni::Hunyuan3dOmniDriver),
        "trellis_2" => Arc::new(trellis2::Trellis2Driver),
        "custom_cli" => Arc::new(custom::CustomCliDriver),
        _ => Arc::new(custom::CustomHttpDriver),
    }
}

/// Validate generated 3D bytes before persisting as a Design Hall artifact.
pub fn validate_3d_output(format: &str, bytes: &[u8], max_bytes: usize) -> Result<()> {
    if bytes.is_empty() {
        return Err(Error::Upstream("3D provider returned empty output".into()));
    }
    if bytes.len() > max_bytes {
        return Err(Error::PayloadTooLarge(format!(
            "generated 3D asset ({} bytes) exceeds the {} MB limit",
            bytes.len(),
            max_bytes / (1024 * 1024)
        )));
    }
    match format {
        "glb" => {
            if bytes.len() < 12 || &bytes[0..4] != b"glTF" {
                return Err(Error::Upstream(
                    "3D provider output is not a valid binary GLB (missing glTF header)".into(),
                ));
            }
        }
        "gltf" => {
            let v: Value = serde_json::from_slice(bytes).map_err(|e| {
                Error::Upstream(format!("3D provider output is not valid glTF JSON: {e}"))
            })?;
            if !v.is_object() {
                return Err(Error::Upstream(
                    "3D provider glTF output must be a JSON object".into(),
                ));
            }
        }
        "obj" => {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| Error::Upstream("3D provider OBJ output is not valid UTF-8".into()))?;
            if text.trim().is_empty() {
                return Err(Error::Upstream("3D provider OBJ output is blank".into()));
            }
        }
        other => {
            return Err(Error::Invalid(format!("unsupported 3D format '{other}'")));
        }
    }
    Ok(())
}

/// Verify a configured CLI executable exists as a regular file (never a shell snippet).
pub fn validate_cli_binary(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err(Error::Invalid("cli_binary path cannot be empty".into()));
    }
    let p = PathBuf::from(trimmed);
    if !p.is_file() {
        return Err(Error::Invalid(format!(
            "cli_binary '{}' does not exist or is not a file",
            p.display()
        )));
    }
    Ok(p)
}

/// Safe single-token placeholder expansion for CLI argv templates (no shell).
pub fn expand_cli_args(template: &[String], vars: &HashMap<&str, String>) -> Vec<String> {
    template
        .iter()
        .map(|arg| {
            let mut out = arg.clone();
            for (k, v) in vars {
                let needle = format!("{{{k}}}");
                if out.contains(&needle) {
                    out = out.replace(&needle, v);
                }
            }
            out
        })
        .collect()
}

/// Execute a local 3D generator strictly via argv (never `sh -c`), with
/// `stdin(Stdio::null())`, timeout, and live cancel checks. The CLI runs in its
/// own process group: a cancel or timeout SIGTERMs the group (Otto Forge stops
/// its model runner / Blender children on SIGTERM only), waits
/// [`CLI_STOP_GRACE`], then SIGKILLs it, and dropping the call SIGKILLs it.
///
/// Stderr is read line by line: an Otto Forge progress line
/// (`{"stage","pct","message"}`) goes to `progress` (never backwards, below
/// 100), a `{"error": "..."}` line becomes the failure message, and every
/// line is kept in the tail reported when no error line came.
pub async fn run_cli_argv(
    bin: &Path,
    args: &[String],
    workdir: Option<&Path>,
    timeout: Duration,
    cancel: &AtomicBool,
    progress: Option<&CliProgress>,
) -> Result<()> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    if let Some(dir) = workdir {
        if !dir.is_dir() {
            return Err(Error::Invalid(format!(
                "cli_workdir '{}' is not a directory",
                dir.display()
            )));
        }
        cmd.current_dir(dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("failed to spawn '{}': {e}", bin.display())))?;
    let _group = KillGroupOnDrop(child.id());

    let stderr_handle = child.stderr.take().map(|s| {
        let progress = progress.cloned();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncReadExt};
            let mut reader = tokio::io::BufReader::new(s);
            let (mut tail, mut line) = (Vec::new(), Vec::new());
            let (mut best, mut error) = (0i64, None::<String>);
            loop {
                line.clear();
                // A line longer than the tail cap arrives in pieces (none parse).
                let limit = MAX_STDERR_TAIL_BYTES as u64;
                match (&mut reader).take(limit).read_until(b'\n', &mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                tail.extend_from_slice(&line);
                if tail.len() > MAX_STDERR_TAIL_BYTES {
                    let excess = tail.len() - MAX_STDERR_TAIL_BYTES;
                    tail.drain(0..excess);
                }
                let Ok(v) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if let Some(e) = v.get("error").and_then(Value::as_str) {
                    error = Some(e.to_string());
                } else if let (Some(pct), Some(tx)) =
                    (v.get("pct").and_then(Value::as_f64), &progress)
                {
                    best = best.max((pct as i64).clamp(0, 99));
                    let label = v
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|m| !m.trim().is_empty())
                        .or_else(|| v.get("stage").and_then(Value::as_str))
                        .unwrap_or_default();
                    let stage = v.get("stage").and_then(Value::as_str).unwrap_or_default();
                    tx.send_replace(Some((best, label.trim().to_string(), stage.to_string())));
                }
            }
            (String::from_utf8_lossy(&tail).trim().to_string(), error)
        })
    });
    // Drain stdout concurrently so a verbose script never blocks on a full pipe.
    let stdout_handle = child.stdout.take().map(|mut s| {
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut sink = [0u8; 4096];
            while let Ok(n) = s.read(&mut sink).await {
                if n == 0 {
                    break;
                }
            }
        })
    });

    let started = Instant::now();
    loop {
        // A stopped CLI's reader is aborted: it holds the progress sender, and
        // a child outside the group could keep stderr open.
        if cancel.load(Ordering::Relaxed) {
            stop_cli(&mut child).await;
            stderr_handle.inspect(|h| h.abort());
            return Err(Error::Conflict("the job was cancelled".into()));
        }
        if started.elapsed() >= timeout {
            stop_cli(&mut child).await;
            stderr_handle.inspect(|h| h.abort());
            return Err(Error::Upstream(format!(
                "the local generator CLI timed out after {} s",
                timeout.as_secs()
            )));
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(h) = stdout_handle {
                    let _ = h.await;
                }
                let (tail, error) = match stderr_handle {
                    Some(h) => h.await.unwrap_or_default(),
                    None => Default::default(),
                };
                if status.success() {
                    return Ok(());
                }
                // The engine's own `{"error"}` line is the message, as it wrote it.
                if let Some(e) = error.filter(|e| !e.trim().is_empty()) {
                    return Err(Error::Upstream(e));
                }
                let detail = if tail.is_empty() {
                    format!("{status}")
                } else {
                    format!("{status}: {tail}")
                };
                return Err(Error::Upstream(format!(
                    "the local generator CLI failed ({detail})"
                )));
            }
            Ok(None) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => {
                let _ = child.kill().await;
                return Err(Error::Upstream(format!("waiting on 3D CLI: {e}")));
            }
        }
    }
}

/// SIGTERM the CLI's process group, give the CLI [`CLI_STOP_GRACE`] to exit,
/// then SIGKILL and reap it (the rest of the group goes with [`KillGroupOnDrop`]).
async fn stop_cli(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        signal_group(pid, libc::SIGTERM);
        let _ = tokio::time::timeout(CLI_STOP_GRACE, child.wait()).await;
    }
    let _ = child.kill().await;
}

/// SIGKILLs the CLI's whole process group when `run_cli_argv` returns or is
/// dropped: `kill_on_drop` reaches only the CLI itself, not its children.
pub(crate) struct KillGroupOnDrop(pub(crate) Option<u32>);

impl Drop for KillGroupOnDrop {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            signal_group(pid, libc::SIGKILL);
        }
    }
}

pub(crate) fn signal_group(pid: u32, sig: libc::c_int) {
    // SAFETY: plain syscall on the group spawned with `process_group(0)`
    // (pgid == the CLI's pid); ESRCH (already gone) is ignored.
    unsafe {
        libc::killpg(pid as libc::pid_t, sig);
    }
}

/// Read optional `<output>.manifest.json` written beside a CLI output mesh.
/// Bounded to 256 KiB and ignored cleanly if missing or malformed.
pub async fn read_optional_manifest(output_file: &Path) -> Option<Value> {
    let mut name = output_file.file_name()?.to_os_string();
    name.push(".manifest.json");
    let manifest_path = output_file.with_file_name(name);
    let meta = tokio::fs::metadata(&manifest_path).await.ok()?;
    if !meta.is_file() || meta.len() as usize > MAX_MANIFEST_BYTES {
        return None;
    }
    let bytes = tokio::fs::read(&manifest_path).await.ok()?;
    let val: Value = serde_json::from_slice(&bytes).ok()?;
    val.is_object().then_some(val)
}

/// Shared HTTP submission helper supporting:
/// 1. Direct binary mesh response (`model/gltf-binary`, `application/octet-stream`, `model/obj`)
/// 2. JSON response carrying base64 mesh (`glb_b64` / `mesh_b64` / `model_b64` / `data`)
/// 3. Async job JSON (`{ job_id | task_id, status_url?, download_url? }`) with bounded SSRF-safe polling.
pub async fn post_3d_http_and_collect(
    provider: &Media3dProviderRecord,
    api_key: Option<&str>,
    payload: &Value,
    expected_format: &str,
    max_output_bytes: usize,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<Gen3dOutput> {
    let raw_url = provider
        .endpoint_url
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Error::Invalid("provider has no endpoint_url configured".into()))?;
    let has_creds = api_key.is_some_and(|k| !k.trim().is_empty());
    let base = validate_endpoint_url(raw_url, has_creds, provider.allow_local_network)?;
    let client = build_http_client(provider.allow_local_network, timeout)?;

    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Conflict("3D generation job was cancelled".into()));
    }

    let req = apply_auth_header(
        client.post(base.clone()).json(payload),
        api_key,
        provider.auth_header.as_deref(),
    );
    let resp = req
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("3D HTTP request failed: {e}")))?;
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    if !status.is_success() {
        let body = read_bounded_bytes(resp, MAX_JSON_STATUS_BYTES)
            .await
            .unwrap_or_default();
        let msg = String::from_utf8_lossy(&body);
        return Err(Error::Upstream(format!(
            "3D provider returned HTTP {}: {}",
            status.as_u16(),
            msg.chars().take(400).collect::<String>()
        )));
    }

    let json_cap = max_output_bytes
        .saturating_mul(4)
        .saturating_div(3)
        .saturating_add(64 * 1024);

    // Case 1: JSON response (inline base64, direct download URL, or async polling).
    if content_type.contains("application/json") {
        let bytes = read_bounded_bytes(resp, json_cap).await?;
        let val: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Upstream(format!("invalid JSON from 3D provider: {e}")))?;

        if let Some(out) = try_extract_inline_mesh(&val, expected_format, max_output_bytes)? {
            return Ok(out);
        }

        if let Some(dl_url) = val
            .get("download_url")
            .or_else(|| val.get("glb_url"))
            .or_else(|| val.get("mesh_url"))
            .and_then(|v| v.as_str())
        {
            let manifest = val.get("manifest").filter(|m| m.is_object()).cloned();
            return download_guarded_mesh(
                &client,
                &base,
                dl_url,
                provider,
                api_key,
                expected_format,
                max_output_bytes,
                manifest,
            )
            .await;
        }

        // Async task polling when server returns job_id / task_id / status_url.
        let status_url = val
            .get("status_url")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| {
                val.get("job_id")
                    .or_else(|| val.get("task_id"))
                    .or_else(|| val.get("id"))
                    .and_then(|v| v.as_str())
                    .map(|id| format!("{}/status/{id}", base.as_str().trim_end_matches('/')))
            });

        if let Some(poll_raw) = status_url {
            let started = Instant::now();
            let poll_url = resolve_and_validate_url(&base, &poll_raw, has_creds, provider)?;
            // Credentials go only to the configured origin (as in download_guarded_mesh).
            let same_origin = poll_url.origin() == base.origin();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Conflict("3D generation job was cancelled".into()));
                }
                if started.elapsed() >= timeout {
                    return Err(Error::Upstream("3D provider job polling timed out".into()));
                }
                tokio::time::sleep(POLL_INTERVAL).await;
                let poll_req = client.get(poll_url.clone());
                let poll_req = if same_origin {
                    apply_auth_header(poll_req, api_key, provider.auth_header.as_deref())
                } else {
                    poll_req
                };
                let poll_resp = poll_req
                    .send()
                    .await
                    .map_err(|e| Error::Upstream(format!("polling 3D job failed: {e}")))?;
                if !poll_resp.status().is_success() {
                    return Err(Error::Upstream(format!(
                        "3D job status endpoint returned HTTP {}",
                        poll_resp.status().as_u16()
                    )));
                }
                let poll_bytes = read_bounded_bytes(poll_resp, json_cap).await?;
                let pv: Value = serde_json::from_slice(&poll_bytes)
                    .map_err(|e| Error::Upstream(format!("invalid poll JSON: {e}")))?;
                if let Some(out) = try_extract_inline_mesh(&pv, expected_format, max_output_bytes)?
                {
                    return Ok(out);
                }
                let st = pv
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(st.as_str(), "failed" | "error" | "cancelled") {
                    let msg = pv
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("upstream 3D job failed");
                    return Err(Error::Upstream(msg.to_string()));
                }
                if let Some(dl) = pv
                    .get("download_url")
                    .or_else(|| pv.get("glb_url"))
                    .or_else(|| pv.get("mesh_url"))
                    .and_then(|v| v.as_str())
                {
                    if st.is_empty()
                        || matches!(st.as_str(), "succeeded" | "done" | "completed" | "ready")
                    {
                        let manifest = pv.get("manifest").filter(|m| m.is_object()).cloned();
                        return download_guarded_mesh(
                            &client,
                            &base,
                            dl,
                            provider,
                            api_key,
                            expected_format,
                            max_output_bytes,
                            manifest,
                        )
                        .await;
                    }
                }
            }
        }

        return Err(Error::Upstream(
            "3D provider JSON response contained neither inline mesh base64 nor download_url/status_url"
                .into(),
        ));
    }

    // Case 2: Raw binary mesh response.
    let bytes = read_bounded_bytes(resp, max_output_bytes).await?;
    let fmt = if content_type.contains("model/obj") || expected_format == "obj" {
        "obj"
    } else if content_type.contains("gltf+json") || expected_format == "gltf" {
        "gltf"
    } else {
        "glb"
    };
    validate_3d_output(fmt, &bytes, max_output_bytes)?;
    Ok(Gen3dOutput {
        bytes,
        format: fmt.to_string(),
        manifest: None,
    })
}

fn try_extract_inline_mesh(
    val: &Value,
    expected_format: &str,
    max_output_bytes: usize,
) -> Result<Option<Gen3dOutput>> {
    let b64 = val
        .get("glb_b64")
        .or_else(|| val.get("mesh_b64"))
        .or_else(|| val.get("model_b64"))
        .or_else(|| val.get("data"))
        .and_then(|v| v.as_str());
    let Some(b64) = b64 else {
        return Ok(None);
    };
    let clean = b64.split_once(',').map(|(_, rest)| rest).unwrap_or(b64);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(clean.trim())
        .map_err(|e| Error::Upstream(format!("invalid base64 mesh from 3D provider: {e}")))?;
    let fmt = val
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or(expected_format)
        .to_ascii_lowercase();
    validate_3d_output(&fmt, &bytes, max_output_bytes)?;
    let manifest = val.get("manifest").filter(|m| m.is_object()).cloned();
    Ok(Some(Gen3dOutput {
        bytes,
        format: fmt,
        manifest,
    }))
}

fn resolve_and_validate_url(
    base: &reqwest::Url,
    raw: &str,
    has_creds: bool,
    provider: &Media3dProviderRecord,
) -> Result<reqwest::Url> {
    let joined = base
        .join(raw)
        .map_err(|e| Error::Upstream(format!("invalid upstream URL '{raw}': {e}")))?;
    validate_endpoint_url(joined.as_str(), has_creds, provider.allow_local_network)
}

#[allow(clippy::too_many_arguments)]
async fn download_guarded_mesh(
    client: &reqwest::Client,
    base: &reqwest::Url,
    raw_dl: &str,
    provider: &Media3dProviderRecord,
    api_key: Option<&str>,
    expected_format: &str,
    max_output_bytes: usize,
    manifest: Option<Value>,
) -> Result<Gen3dOutput> {
    let has_creds = api_key.is_some_and(|k| !k.trim().is_empty());
    let url = resolve_and_validate_url(base, raw_dl, has_creds, provider)?;
    let same_origin = url.origin() == base.origin();
    let req = if same_origin {
        apply_auth_header(
            client.get(url.clone()),
            api_key,
            provider.auth_header.as_deref(),
        )
    } else {
        client.get(url.clone())
    };
    let resp = req
        .send()
        .await
        .map_err(|e| Error::Upstream(format!("downloading 3D mesh failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Upstream(format!(
            "3D mesh download returned HTTP {}",
            resp.status().as_u16()
        )));
    }
    let bytes = read_bounded_bytes(resp, max_output_bytes).await?;
    validate_3d_output(expected_format, &bytes, max_output_bytes)?;
    Ok(Gen3dOutput {
        bytes,
        format: expected_format.to_string(),
        manifest,
    })
}

/// Shared connection test for HTTP or local CLI 3D providers.
pub async fn test_3d_provider_connection(
    provider: &Media3dProviderRecord,
    api_key: Option<&str>,
) -> Result<ProviderTestResult> {
    let started = Instant::now();
    match provider.transport.as_str() {
        "local_cli" => {
            let Some(bin) = provider.effective_cli_binary() else {
                return Ok(ProviderTestResult {
                    ok: false,
                    latency_ms: Some(0),
                    message: "No local CLI executable found or configured".into(),
                    detected_version: None,
                    details: None,
                });
            };
            let path = Path::new(&bin);
            if !path.is_file() {
                return Ok(ProviderTestResult {
                    ok: false,
                    latency_ms: Some(started.elapsed().as_millis() as u64),
                    message: format!("Executable not found at {bin}"),
                    detected_version: None,
                    details: None,
                });
            }
            if let Some(wd) = provider
                .cli_workdir
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                if !Path::new(wd).is_dir() {
                    return Ok(ProviderTestResult {
                        ok: false,
                        latency_ms: Some(started.elapsed().as_millis() as u64),
                        message: format!("Working directory not found at {wd}"),
                        detected_version: None,
                        details: None,
                    });
                }
            }
            Ok(ProviderTestResult {
                ok: true,
                latency_ms: Some(started.elapsed().as_millis() as u64),
                message: format!("CLI executable verified at {bin}"),
                detected_version: None,
                details: None,
            })
        }
        "http" => {
            let Some(raw) = provider
                .endpoint_url
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            else {
                return Ok(ProviderTestResult {
                    ok: false,
                    latency_ms: Some(0),
                    message: "No HTTP endpoint_url configured".into(),
                    detected_version: None,
                    details: None,
                });
            };
            let has_creds = api_key.is_some_and(|k| !k.trim().is_empty());
            let url = validate_endpoint_url(raw, has_creds, provider.allow_local_network)?;
            let client = build_http_client(provider.allow_local_network, Duration::from_secs(10))?;
            let health_url = url.join("/health").unwrap_or_else(|_| url.clone());
            let resp = apply_auth_header(
                client.get(health_url.clone()),
                api_key,
                provider.auth_header.as_deref(),
            )
            .send()
            .await;
            let latency_ms = started.elapsed().as_millis() as u64;
            match resp {
                Ok(r) if r.status().is_success() || r.status().as_u16() == 405 => {
                    Ok(ProviderTestResult {
                        ok: true,
                        latency_ms: Some(latency_ms),
                        message: format!("Reachable (HTTP {})", r.status().as_u16()),
                        detected_version: None,
                        details: None,
                    })
                }
                Ok(r) => {
                    // Fallback: probe the exact configured URL in case `/health` is 404.
                    let probe = apply_auth_header(
                        client.get(url.clone()),
                        api_key,
                        provider.auth_header.as_deref(),
                    )
                    .send()
                    .await;
                    match probe {
                        Ok(r2) if r2.status().is_success() || r2.status().as_u16() == 405 => {
                            Ok(ProviderTestResult {
                                ok: true,
                                latency_ms: Some(started.elapsed().as_millis() as u64),
                                message: format!(
                                    "Endpoint reachable (HTTP {})",
                                    r2.status().as_u16()
                                ),
                                detected_version: None,
                                details: None,
                            })
                        }
                        _ => Ok(ProviderTestResult {
                            ok: false,
                            latency_ms: Some(latency_ms),
                            message: format!(
                                "Endpoint responded with HTTP {}",
                                r.status().as_u16()
                            ),
                            detected_version: None,
                            details: None,
                        }),
                    }
                }
                Err(e) => Ok(ProviderTestResult {
                    ok: false,
                    latency_ms: Some(latency_ms),
                    message: format!("Connection failed: {e}"),
                    detected_version: None,
                    details: None,
                }),
            }
        }
        other => Err(Error::Invalid(format!("unsupported transport '{other}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a fake CLI that starts a long `sleep` grandchild (Otto Forge's model
    /// runner / Blender stand-in), stop it by cancel or by timeout, and check
    /// the CLI got a SIGTERM and the grandchild is gone.
    async fn stop_leaves_no_grandchild(by_timeout: bool) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake-cli.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\ntrap 'echo term > \"$1/termed\"; exit 143' TERM\n\
             sleep 300 &\necho $! > \"$1/child.pid\"\nwait\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pid_file = dir.path().join("child.pid");
        let read_pid = || std::fs::read_to_string(&pid_file).unwrap_or_default();

        let cancel = AtomicBool::new(false);
        let timeout = Duration::from_secs(if by_timeout { 2 } else { 60 });
        let args = [dir.path().display().to_string()];
        let run = run_cli_argv(&script, &args, None, timeout, &cancel, None);
        let trigger = async {
            while read_pid().trim().is_empty() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if !by_timeout {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let (res, ()) = tokio::join!(run, trigger);
        match res {
            Err(Error::Upstream(m)) if by_timeout => assert!(m.contains("timed out"), "{m}"),
            Err(Error::Conflict(_)) if !by_timeout => {}
            other => panic!("unexpected result: {other:?}"),
        }

        let pid: libc::pid_t = read_pid().trim().parse().unwrap();
        // SAFETY: signal 0 only probes whether the pid exists.
        let alive = || unsafe { libc::kill(pid, 0) } == 0;
        for _ in 0..50 {
            if !alive() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let survived = alive();
        if survived {
            // SAFETY: the grandchild this test started; leave nothing running.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        assert!(!survived, "grandchild {pid} outlived the stopped CLI");
        assert!(
            dir.path().join("termed").exists(),
            "the CLI never got a SIGTERM to clean up its children"
        );
    }

    /// A CLI that prints plain text (no JSON Lines) still runs: it reports no
    /// progress, and a failure carries its stderr tail.
    #[tokio::test]
    async fn plain_stderr_cli_reports_no_progress_and_keeps_its_tail() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("plain-cli.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'loading weights' >&2\necho '{\"pct\": oops' >&2\n\
             [ \"$1\" = ok ] || { echo 'out of memory' >&2; exit 2; }\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cancel = AtomicBool::new(false);
        let (tx, rx) = tokio::sync::watch::channel(None);
        let t = Duration::from_secs(30);
        run_cli_argv(&script, &["ok".into()], None, t, &cancel, Some(&tx))
            .await
            .unwrap();
        let err = run_cli_argv(&script, &["fail".into()], None, t, &cancel, Some(&tx))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("loading weights"), "{err}");
        assert!(err.contains("out of memory"), "{err}");
        assert!(rx.borrow().is_none(), "plain text was read as progress");
    }

    #[tokio::test]
    async fn cancel_stops_the_cli_and_its_children() {
        stop_leaves_no_grandchild(false).await;
    }

    #[tokio::test]
    async fn timeout_stops_the_cli_and_its_children() {
        stop_leaves_no_grandchild(true).await;
    }
}
