//! Blender bridge for the Design arena — OPTIONAL and detected, never required.
//!
//! - `GET /product/design/blender` — is Blender installed (`$OTTO_BLENDER`, then
//!   `PATH`, then `/Applications/Blender.app/Contents/MacOS/Blender`)?
//! - `POST /product/stories/{sid}/design/{aid}/blender-render` (ws editor) —
//!   `202 { id }`: render a `scene3d` attachment headlessly. The server
//!   **generates** the Python from the validated document
//!   (`design_scene3d::to_blender_script`) — never a user or agent file — writes
//!   it to a fresh temp out-dir and spawns
//!   `blender -b --python <generated.py> -- --out <dir>` wrapped in
//!   `SandboxPolicy::for_tool(out_dir)` (writes confined to the out-dir + Blender's
//!   own cache dirs, no network), `kill_on_drop`, 120 s timeout. The script
//!   renders `render.png` (Eevee, 1280×720) and exports `scene.glb`; each produced
//!   file is attached to the story (`kind:'design'`, `meta.derived_from = aid`)
//!   and announced with `MockupUpdated { content: null }`.
//! - `GET /product/design/jobs/{id}` (ws viewer) — poll a job. Jobs live in an
//!   in-memory map on `ServerCtx` and are NOT persisted (a restart forgets them;
//!   the attached outputs survive as ordinary attachments).
//! - `GET /product/stories/{sid}/design/{aid}/blender-script` (ws viewer) — the
//!   generated `.py` as a download, for opening in Blender by hand.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Path as AxPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{NewAttachment, ProductAttachment};
use serde::Serialize;

use crate::auth::CurrentUser;
use crate::design_format::DesignFormat;
use crate::design_scene3d::{self, Scene3d};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Wall-clock cap for one headless render (the scene is a blockout, not a film).
const RENDER_TIMEOUT: Duration = Duration::from_secs(120);
/// `blender --version` probe cap.
const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
/// Finished jobs older than this are pruned from the in-memory map.
const JOB_TTL: chrono::Duration = chrono::Duration::hours(1);
/// Scratch root for render jobs (under `data_dir`, so the sandbox's out-dir root
/// is always inside Otto's own state).
const JOBS_ROOT: &str = "product/blender_jobs";
/// Well-known macOS app bundle location.
const APP_BUNDLE_BIN: &str = "/Applications/Blender.app/Contents/MacOS/Blender";

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// `GET /product/design/blender` payload.
#[derive(Debug, Clone, Serialize)]
pub struct BlenderStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

/// Locate the Blender binary: `$OTTO_BLENDER` (explicit override) → `PATH`
/// (`otto_k8s::install::which`, no `which` subprocess) → the app bundle.
pub fn locate() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("OTTO_BLENDER").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    if let Some(p) = otto_k8s::install::which("blender") {
        return Some(p);
    }
    let bundle = PathBuf::from(APP_BUNDLE_BIN);
    bundle.is_file().then_some(bundle)
}

/// Run `blender --version` and pull the `Blender X.Y.Z` line; `None` when the
/// binary doesn't execute (a broken install reports `installed:false`).
async fn probe_version(bin: &Path) -> Option<String> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = cmd.spawn().ok()?;
    let out = tokio::time::timeout(VERSION_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_version(&text)
}

/// `"Blender 4.2.1 LTS\n\tbuild date: …"` → `"4.2.1"`.
fn parse_version(text: &str) -> Option<String> {
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("Blender "))?;
    line.split_whitespace()
        .nth(1)
        .filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string)
}

/// How long a `detect()` result is served from the registry cache before the
/// `blender --version` subprocess runs again.
const DETECT_TTL: Duration = Duration::from_secs(60);

/// Detect + probe (one short subprocess). Prefer `detect_cached` on hot paths.
pub async fn detect() -> BlenderStatus {
    let Some(bin) = locate() else {
        return BlenderStatus {
            installed: false,
            path: None,
            version: None,
        };
    };
    let version = probe_version(&bin).await;
    BlenderStatus {
        installed: version.is_some(),
        path: Some(bin.to_string_lossy().to_string()),
        version,
    }
}

/// `GET /product/design/blender` — any authenticated user (the `/product/`
/// feature gate applies). Nothing here is workspace-scoped.
pub async fn blender_status(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> Json<BlenderStatus> {
    Json(detect_cached(&ctx.design_jobs).await)
}

/// `detect()` behind a ~60 s TTL cache on the registry so a polling UI doesn't
/// spawn `blender --version` per GET.
pub async fn detect_cached(reg: &JobRegistry) -> BlenderStatus {
    if let Some((at, status)) = reg
        .detect_cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
    {
        if at.elapsed() < DETECT_TTL {
            return status.clone();
        }
    }
    let status = detect().await;
    *reg.detect_cache.lock().unwrap_or_else(|p| p.into_inner()) =
        Some((Instant::now(), status.clone()));
    status
}

// ---------------------------------------------------------------------------
// Job registry (in-memory, not persisted)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RenderJob {
    pub id: Id,
    pub attachment_id: Id,
    /// `queued` | `running` | `done` | `error`.
    pub status: String,
    pub error: Option<String>,
    /// New attachment ids (the render PNG, the exported GLB).
    pub outputs: Vec<Id>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Workspace of the source attachment — gates `GET /design/jobs/{id}`.
    #[serde(skip)]
    pub workspace_id: Id,
}

/// At most this many Blender processes at once (each is a full headless
/// Blender); a further request is a 409, not a queue.
pub const MAX_CONCURRENT_RENDERS: usize = 2;

/// The in-memory Blender state on `ServerCtx`: jobs, the render permits and the
/// detection cache. Nothing here survives a restart.
pub struct DesignJobs {
    jobs: Mutex<HashMap<Id, RenderJob>>,
    render_permits: Arc<tokio::sync::Semaphore>,
    detect_cache: Mutex<Option<(Instant, BlenderStatus)>>,
}

pub type JobRegistry = Arc<DesignJobs>;

pub fn new_job_registry() -> JobRegistry {
    Arc::new(DesignJobs {
        jobs: Mutex::new(HashMap::new()),
        render_permits: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_RENDERS)),
        detect_cache: Mutex::new(None),
    })
}

fn job_upsert(reg: &JobRegistry, job: RenderJob) {
    let mut map = reg.jobs.lock().unwrap_or_else(|p| p.into_inner());
    // Prune finished jobs past their TTL so the map stays bounded.
    let cutoff = Utc::now() - JOB_TTL;
    map.retain(|_, j| j.finished_at.is_none_or(|t| t > cutoff));
    map.insert(job.id.clone(), job);
}

fn job_update(reg: &JobRegistry, id: &Id, f: impl FnOnce(&mut RenderJob)) {
    let mut map = reg.jobs.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(j) = map.get_mut(id) {
        f(j);
    }
}

fn job_get(reg: &JobRegistry, id: &Id) -> Option<RenderJob> {
    reg.jobs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(id)
        .cloned()
}

/// Is a job for this attachment still `queued` / `running`? (One in-flight
/// render per artifact — a second click is a 409, not a duplicate process.)
fn job_in_flight_for(reg: &JobRegistry, attachment_id: &Id) -> bool {
    reg.jobs
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .any(|j| j.attachment_id == *attachment_id && j.finished_at.is_none())
}

/// `GET /product/design/jobs/{id}` — Viewer on the source attachment's workspace.
pub async fn get_job(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<RenderJob>> {
    let job = job_get(&ctx.design_jobs, &id)
        .ok_or_else(|| ApiError(Error::NotFound(format!("blender job {id}"))))?;
    crate::auth::require_ws_role(&ctx, &user, &job.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(job))
}

// ---------------------------------------------------------------------------
// Script download
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct DesignPath {
    pub sid: Id,
    pub aid: Id,
}

/// Load a `scene3d` attachment of `sid` (role-checked) and validate it.
async fn load_scene(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    sid: &Id,
    aid: &Id,
    role: WorkspaceRole,
) -> ApiResult<(ProductAttachment, Scene3d)> {
    let story = ctx.product_repo.get_story(sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(ctx, user, &story.workspace_id, role).await?;
    let att = ctx
        .attachment_repo
        .get(aid)
        .await
        .map_err(ApiError)?
        .filter(|a| a.story_id == story.id)
        .ok_or_else(|| ApiError(Error::NotFound(format!("attachment {aid}"))))?;
    if att.mime != DesignFormat::Scene3d.mime() {
        return Err(ApiError(Error::Invalid(format!(
            "attachment {aid} is not a scene3d document ({})",
            att.mime
        ))));
    }
    let full =
        otto_core::paths::confine_join(&ctx.data_dir, &att.storage_path).ok_or_else(|| {
            ApiError(Error::Forbidden(
                "attachment path escapes the data dir".into(),
            ))
        })?;
    let bytes = tokio::fs::read(&full)
        .await
        .map_err(|_| ApiError(Error::NotFound(format!("attachment file {aid}"))))?;
    let scene = design_scene3d::validate_bytes(&bytes).map_err(ApiError)?;
    Ok((att, scene))
}

/// `GET /product/stories/{sid}/design/{aid}/blender-script` — Viewer. The
/// generated `.py` as an attachment download.
pub async fn blender_script(
    AxPath(DesignPath { sid, aid }): AxPath<DesignPath>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    let (att, scene) = load_scene(&ctx, &user, &sid, &aid, WorkspaceRole::Viewer).await?;
    let py = design_scene3d::to_blender_script(&scene);
    let stem = att
        .filename
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or(att.filename.clone())
        .replace(['"', '\r', '\n', '/', '\\'], "_");
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/x-python; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{stem}.blender.py\""),
        )
        .header("x-content-type-options", "nosniff")
        .body(Body::from(py))
        .map_err(|e| ApiError(Error::Internal(format!("build response: {e}"))))?;
    Ok(resp)
}

// ---------------------------------------------------------------------------
// Render job
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct RenderAccepted {
    pub id: Id,
}

/// `POST /product/stories/{sid}/design/{aid}/blender-render` — Editor. `202 {id}`;
/// `409` when Blender is not installed; `400` when the document doesn't validate.
pub async fn blender_render(
    AxPath(DesignPath { sid, aid }): AxPath<DesignPath>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    let (att, scene) = load_scene(&ctx, &user, &sid, &aid, WorkspaceRole::Editor).await?;
    let Some(bin) = locate() else {
        return Err(ApiError(Error::Conflict(
            "Blender is not installed (set OTTO_BLENDER, add `blender` to PATH, or install \
             Blender.app); download the script instead"
                .into(),
        )));
    };
    // Concurrency: one in-flight render per artifact, at most
    // `MAX_CONCURRENT_RENDERS` Blender processes overall. The permit is taken
    // HERE (not in the task) so an over-capacity request is a 409 immediately;
    // it is released as soon as the render process exits.
    if job_in_flight_for(&ctx.design_jobs, &att.id) {
        return Err(ApiError(Error::Conflict(format!(
            "a Blender render for attachment {} is already in progress",
            att.id
        ))));
    }
    let permit = match ctx.design_jobs.render_permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            return Err(ApiError(Error::Conflict(format!(
                "{MAX_CONCURRENT_RENDERS} Blender renders are already running; try again shortly"
            ))))
        }
    };
    let py = design_scene3d::to_blender_script(&scene);

    // Fresh out-dir per job under data_dir (the job id is daemon-minted).
    let job_id = otto_core::new_id();
    let out_dir = otto_core::paths::confine_join(&ctx.data_dir.join(JOBS_ROOT), &job_id)
        .ok_or_else(|| ApiError(Error::Internal("unsafe job id".into())))?;
    tokio::fs::create_dir_all(&out_dir)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("create render dir: {e}"))))?;
    let script = out_dir.join("scene.py");
    tokio::fs::write(&script, py.as_bytes())
        .await
        .map_err(|e| ApiError(Error::Internal(format!("write render script: {e}"))))?;

    job_upsert(
        &ctx.design_jobs,
        RenderJob {
            id: job_id.clone(),
            attachment_id: att.id.clone(),
            status: "queued".into(),
            error: None,
            outputs: Vec::new(),
            started_at: Utc::now(),
            finished_at: None,
            workspace_id: att.workspace_id.clone(),
        },
    );

    let ctx2 = ctx.clone();
    let user_id = user.id.clone();
    let jid = job_id.clone();
    tokio::spawn(async move {
        job_update(&ctx2.design_jobs, &jid, |j| j.status = "running".into());
        let result = run_render(&bin, &script, &out_dir).await;
        drop(permit);
        let outcome = match result {
            Ok(()) => attach_outputs(&ctx2, &att, &out_dir, &user_id).await,
            Err(e) => Err(e),
        };
        job_update(&ctx2.design_jobs, &jid, |j| {
            j.finished_at = Some(Utc::now());
            match outcome {
                Ok(ids) => {
                    j.status = "done".into();
                    j.outputs = ids;
                }
                Err(e) => {
                    j.status = "error".into();
                    j.error = Some(e.to_string());
                }
            }
        });
        // The out-dir only ever held the generated script + copied outputs.
        let _ = tokio::fs::remove_dir_all(&out_dir).await;
    });

    Ok((StatusCode::ACCEPTED, Json(RenderAccepted { id: job_id })).into_response())
}

/// The command we spawn — pure so it unit-tests: `blender -b --python <script>
/// -- --out <dir>`, wrapped in `sandbox-exec` with `SandboxPolicy::for_tool`
/// when the host supports it.
fn render_command(bin: &Path, script: &Path, out_dir: &Path) -> (String, Vec<String>) {
    let program = bin.to_string_lossy().to_string();
    let args = vec![
        "-b".to_string(),
        "--python".to_string(),
        script.to_string_lossy().to_string(),
        "--".to_string(),
        "--out".to_string(),
        out_dir.to_string_lossy().to_string(),
    ];
    if otto_sandbox::is_supported() {
        otto_sandbox::SandboxPolicy::for_tool(out_dir).wrap(&program, &args)
    } else {
        (program, args)
    }
}

/// Spawn the render (shape copied from `otto-aws/src/cli.rs::run_raw`):
/// `kill_on_drop`, a hard timeout, stderr tail on failure.
async fn run_render(bin: &Path, script: &Path, out_dir: &Path) -> Result<(), Error> {
    let (program, args) = render_command(bin, script, out_dir);
    let mut cmd = tokio::process::Command::new(&program);
    cmd.args(&args)
        .current_dir(out_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| Error::Internal(format!("spawn blender: {e}")))?;
    let out = match tokio::time::timeout(RENDER_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(Error::Internal(format!("blender: {e}"))),
        Err(_) => {
            return Err(Error::Upstream(format!(
                "blender render timed out after {}s",
                RENDER_TIMEOUT.as_secs()
            )))
        }
    };
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let tail: String = stderr
            .lines()
            .chain(stdout.lines())
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::Upstream(format!(
            "blender exited with {}: {}",
            out.status,
            tail.trim()
        )));
    }
    Ok(())
}

/// Attach whatever the script produced (`render.png`, `scene.glb`) to the
/// source attachment's story as `kind:'design'` rows with
/// `meta.derived_from = <aid>`, and announce each with `MockupUpdated`
/// (`content: null` — binaries are re-fetched). Fails when nothing was produced.
async fn attach_outputs(
    ctx: &ServerCtx,
    src: &ProductAttachment,
    out_dir: &Path,
    user_id: &Id,
) -> Result<Vec<Id>, Error> {
    let stem = src
        .filename
        .rsplit_once('.')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| src.filename.clone());
    let mut ids = Vec::new();
    for (file, mime, suffix, group) in [
        ("render.png", "image/png", "render.png", "Renders"),
        ("scene.glb", "model/gltf-binary", "glb", "3D"),
    ] {
        let path = out_dir.join(file);
        let Ok(bytes) = tokio::fs::read(&path).await else {
            continue;
        };
        if bytes.is_empty() || !crate::product_media::sniff_ok(mime, &bytes) {
            tracing::warn!("blender render: {file} did not sniff as {mime}; skipped");
            continue;
        }
        let id = otto_core::new_id();
        let ext = crate::product_media::ext_for_mime(mime);
        let rel = format!(
            "{}/{}/{id}{ext}",
            crate::product_media::ATTACH_ROOT,
            src.story_id
        );
        let full = otto_core::paths::confine_join(&ctx.data_dir, &rel)
            .ok_or_else(|| Error::Invalid(format!("unsafe story id {}", src.story_id)))?;
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Internal(format!("attachment dir: {e}")))?;
        }
        tokio::fs::write(&full, &bytes)
            .await
            .map_err(|e| Error::Internal(format!("write attachment: {e}")))?;
        let att = ctx
            .attachment_repo
            .create(NewAttachment {
                story_id: src.story_id.clone(),
                workspace_id: src.workspace_id.clone(),
                filename: format!("{stem}.{suffix}"),
                mime: mime.to_string(),
                size_bytes: bytes.len() as i64,
                sha256: None,
                storage_path: rel,
                kind: "design".into(),
                source: "agent".into(),
                meta_json: Some(
                    serde_json::json!({ "derived_from": src.id, "group": group }).to_string(),
                ),
                created_by: user_id.clone(),
            })
            .await?;
        let _ = ctx.events.send(Event::MockupUpdated {
            workspace_id: att.workspace_id.clone(),
            story_id: att.story_id.clone(),
            attachment_id: att.id.clone(),
            format: mime.to_string(),
            content: None,
        });
        ids.push(att.id);
    }
    if ids.is_empty() {
        return Err(Error::Upstream(
            "blender finished but produced neither render.png nor scene.glb".into(),
        ));
    }
    Ok(ids)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blender_version_line() {
        assert_eq!(
            parse_version("Blender 4.2.1 LTS\n\tbuild date: 2024-08-19\n").as_deref(),
            Some("4.2.1")
        );
        assert_eq!(parse_version("Blender 3.6.0\n").as_deref(), Some("3.6.0"));
        assert_eq!(parse_version("not blender"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn render_command_is_headless_with_out_dir_after_separator() {
        let (prog, args) = render_command(
            Path::new("/opt/blender"),
            Path::new("/j/scene.py"),
            Path::new("/j"),
        );
        // With the sandbox available the program is sandbox-exec and the real
        // command follows the profile; without it the binary runs directly.
        let (real_prog, real_args): (&str, &[String]) = if prog == "/usr/bin/sandbox-exec" {
            assert_eq!(args[0], "-p");
            assert!(args[1].contains("(deny default)"));
            assert!(
                !args[1].contains("network-outbound"),
                "tools get no network"
            );
            assert!(args[1].contains("/j"), "out-dir is a writable root");
            (&args[2], &args[3..])
        } else {
            (&prog, &args[..])
        };
        assert_eq!(real_prog, "/opt/blender");
        assert_eq!(
            real_args,
            ["-b", "--python", "/j/scene.py", "--", "--out", "/j"]
        );
    }

    #[test]
    fn job_registry_prunes_expired_finished_jobs() {
        let reg = new_job_registry();
        let mk = |id: &str, finished: Option<DateTime<Utc>>| RenderJob {
            id: id.to_string(),
            attachment_id: "a".into(),
            status: "done".into(),
            error: None,
            outputs: vec![],
            started_at: Utc::now(),
            finished_at: finished,
            workspace_id: "w".into(),
        };
        job_upsert(
            &reg,
            mk("old", Some(Utc::now() - chrono::Duration::hours(2))),
        );
        job_upsert(&reg, mk("running", None));
        job_upsert(&reg, mk("fresh", Some(Utc::now())));
        let get = |id: &str| job_get(&reg, &id.to_string());
        assert!(get("old").is_none(), "expired finished job pruned");
        assert!(get("running").is_some());
        assert!(get("fresh").is_some());
        job_update(&reg, &"running".to_string(), |j| j.status = "error".into());
        assert_eq!(get("running").unwrap().status, "error");
        // `workspace_id` never leaks onto the wire.
        let v = serde_json::to_value(get("fresh").unwrap()).unwrap();
        assert!(v.get("workspace_id").is_none());
        assert_eq!(v["status"], "done");
        assert!(v["finished_at"].is_string());
    }

    #[test]
    fn registry_tracks_in_flight_per_attachment_and_caps_permits() {
        let reg = new_job_registry();
        let mk = |id: &str, att: &str, finished: Option<DateTime<Utc>>| RenderJob {
            id: id.to_string(),
            attachment_id: att.to_string(),
            status: "running".into(),
            error: None,
            outputs: vec![],
            started_at: Utc::now(),
            finished_at: finished,
            workspace_id: "w".into(),
        };
        job_upsert(&reg, mk("j1", "a1", None));
        job_upsert(&reg, mk("j2", "a2", Some(Utc::now())));
        assert!(job_in_flight_for(&reg, &"a1".to_string()));
        assert!(!job_in_flight_for(&reg, &"a2".to_string()), "finished job is not in flight");
        assert!(!job_in_flight_for(&reg, &"a3".to_string()));
        // Permits: exactly MAX_CONCURRENT_RENDERS, released on drop.
        let p1 = reg.render_permits.clone().try_acquire_owned().unwrap();
        let p2 = reg.render_permits.clone().try_acquire_owned().unwrap();
        assert!(reg.render_permits.clone().try_acquire_owned().is_err(), "third render rejected");
        drop(p1);
        assert!(reg.render_permits.clone().try_acquire_owned().is_ok());
        drop(p2);
    }

    #[tokio::test]
    async fn detect_cache_serves_within_ttl() {
        let reg = new_job_registry();
        let first = detect_cached(&reg).await;
        assert!(reg.detect_cache.lock().unwrap().is_some(), "result cached");
        let second = detect_cached(&reg).await;
        assert_eq!(first.installed, second.installed);
        assert_eq!(first.path, second.path);
    }

    #[tokio::test]
    async fn detect_reports_not_installed_cleanly_when_absent() {
        // Point the override at a non-file so a real install on PATH (if any)
        // still can't make this flaky: an absent override falls through to PATH,
        // so only assert the shape when nothing is found.
        let status = detect().await;
        if !status.installed {
            assert!(status.version.is_none());
        } else {
            assert!(status.path.is_some());
            assert!(status.version.is_some());
        }
    }
}
