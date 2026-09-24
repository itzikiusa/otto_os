//! Site Studio routes (merged into `crate::http::router`; `/design/*` RBAC —
//! GET = design View, POST = design Edit — comes from the server's policy
//! table, the workspace role from the site's row):
//!
//!   POST /design/artifacts/{id}/export            (ws editor) {target:"zip"|"local", version?}
//!        zip   → the static site (application/zip download) + a `zip` publish row
//!        local → a `local` publish row + the loopback preview URL (JSON)
//!   GET  /design/artifacts/{id}/preview[/{page}]  (ws viewer) ?version=|?publish=
//!        → one page as a self-contained HTML document (sandbox CSP)
//!   GET  /design/artifacts/{id}/publishes         (ws viewer) → DesignPublish[]
//!
//! Every publish pins: the site version, the brand kit version and the exact
//! version of every 3D embed / library image it rendered go into the row's
//! `pinned_set_json` (and `otto-publish.json` in the zip), and `?publish=`
//! re-renders exactly that set — a later change to a linked artifact never
//! alters what was shipped. Nothing here uploads anywhere; outward publish
//! targets (claude.ai artifact, git hosting) are not built.

use std::collections::HashMap;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::export::{self, pinned_role, pinned_version, PinnedRef, Resolved};
use super::render::{page_file_name, EmbedInfo};
use super::schema::SiteDoc;
use super::theme::Theme;
use crate::http::DesignCtx;
use crate::service::DesignService;
use crate::store::NewPublish;
use crate::types::{DesignArtifact, DesignPublish, DesignVersion};
use crate::uri::{DesignUri, VersionSel, PREFIX};

/// One asset in an export (the format's own content cap).
const MAX_ASSET_BYTES: usize = 25 * 1024 * 1024;
/// The whole archive.
const MAX_EXPORT_BYTES: usize = 150 * 1024 * 1024;
/// Data-URI budget of one preview page.
const MAX_PREVIEW_INLINE: usize = 12 * 1024 * 1024;
/// References resolved per render (embeds + images).
const MAX_REFS: usize = 60;
/// The loopback preview never runs script or submits anywhere.
const PREVIEW_CSP: &str = "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data: https:; media-src https:; font-src https: data:; base-uri 'none'; form-action 'none'";

struct SiteErr(Error);

impl From<Error> for SiteErr {
    fn from(e: Error) -> Self {
        SiteErr(e)
    }
}

impl IntoResponse for SiteErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

type SiteResult<T> = std::result::Result<T, SiteErr>;

#[derive(Deserialize)]
struct IdPath {
    id: Id,
}

#[derive(Deserialize)]
struct PagePath {
    id: Id,
    page: String,
}

/// `POST /design/artifacts/{id}/export`.
#[derive(Debug, Deserialize)]
pub struct SiteExportReq {
    /// `zip` | `local`.
    pub target: String,
    /// Version id / `v12` / `12`; default the head.
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PreviewQuery {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    publish: Option<Id>,
}

#[derive(Debug, Serialize)]
pub struct SitePage {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub file: String,
    pub url: String,
}

/// `POST …/export {target:"local"}`.
#[derive(Debug, Serialize)]
pub struct SiteLocalResp {
    pub publish: DesignPublish,
    pub url: String,
    pub pages: Vec<SitePage>,
    pub pinned: Vec<PinnedRef>,
    pub warnings: Vec<String>,
}

/// The Site Studio routes, relative to the `/api/v1` mount point.
pub fn routes<S: DesignCtx>() -> Router<S> {
    Router::new()
        .route("/design/artifacts/{id}/export", post(export_route::<S>))
        .route("/design/artifacts/{id}/preview", get(preview_home::<S>))
        .route(
            "/design/artifacts/{id}/preview/{page}",
            get(preview_page::<S>),
        )
        .route(
            "/design/artifacts/{id}/publishes",
            get(publishes_route::<S>),
        )
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

async fn load_site<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    id: &str,
    role: WorkspaceRole,
) -> SiteResult<DesignArtifact> {
    let a = svc.store().require_artifact(id).await?;
    ctx.roles().check(user, &a.workspace_id, role).await?;
    if a.format != "otto-site" {
        return Err(SiteErr(Error::Invalid(format!(
            "design artifact {} is a {} document, not a site (otto-site)",
            a.id, a.format
        ))));
    }
    Ok(a)
}

/// `sel` (a version id / `v12` / `12`), else the head.
async fn site_version(
    svc: &DesignService,
    a: &DesignArtifact,
    sel: Option<&str>,
) -> SiteResult<DesignVersion> {
    if let Some(s) = sel.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(svc.resolve_version(a, s).await?);
    }
    let hid = a
        .head_version_id
        .as_deref()
        .ok_or_else(|| Error::NotFound(format!("site {} has no saved version yet", a.id)))?;
    let v = svc
        .store()
        .get_version(hid)
        .await?
        .ok_or_else(|| Error::NotFound(format!("design version {hid}")))?;
    Ok(v)
}

/// A referenced artifact the caller may view (else `None`: rendered as missing).
async fn viewable<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    id: &str,
) -> Option<DesignArtifact> {
    let a = svc.store().get_artifact(id).await.ok().flatten()?;
    ctx.roles()
        .check(user, &a.workspace_id, WorkspaceRole::Viewer)
        .await
        .ok()?;
    Some(a)
}

/// The version a reference renders: the pinned one (a publish re-render),
/// else what its selector / policy asks for (approved, else head).
async fn version_for(
    svc: &DesignService,
    a: &DesignArtifact,
    sel: &VersionSel,
    pinned: Option<&str>,
) -> Option<DesignVersion> {
    if let Some(vid) = pinned {
        return svc
            .store()
            .get_version(vid)
            .await
            .ok()
            .flatten()
            .filter(|v| v.artifact_id == a.id);
    }
    let vid = match sel {
        VersionSel::Seq(n) => {
            return svc
                .store()
                .get_version_by_seq(&a.id, *n)
                .await
                .ok()
                .flatten()
        }
        VersionSel::Latest => a.head_version_id.clone()?,
        VersionSel::Approved | VersionSel::Default => a
            .approved_version_id
            .clone()
            .or_else(|| a.head_version_id.clone())?,
    };
    svc.store().get_version(&vid).await.ok().flatten()
}

/// `(extension, mime)` of an image by its magic bytes.
fn sniff_image(b: &[u8]) -> Option<(&'static str, &'static str)> {
    if b.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some(("png", "image/png"))
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("jpg", "image/jpeg"))
    } else if b.starts_with(b"GIF8") {
        Some(("gif", "image/gif"))
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(("webp", "image/webp"))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Resolution (brand kit + every rendered reference → one version each)
// ---------------------------------------------------------------------------

/// Where resolved images go.
enum AssetMode {
    /// Pins only (a local publish records; the preview GET renders later).
    Pins,
    /// Files under `assets/` (the zip).
    Files,
    /// Data URIs, within a byte budget (the preview page).
    Inline(usize),
}

/// Put one asset where the mode wants it; the URL the page uses, if any.
fn place(
    mode: &mut AssetMode,
    files: &mut Vec<(String, Vec<u8>)>,
    name: String,
    mime: &str,
    bytes: Vec<u8>,
    warnings: &mut Vec<String>,
) -> Option<String> {
    match mode {
        AssetMode::Pins => None,
        AssetMode::Files => {
            if bytes.len() > MAX_ASSET_BYTES {
                warnings.push(format!("{name} is too large to export"));
                return None;
            }
            if !files.iter().any(|(n, _)| *n == name) {
                files.push((name.clone(), bytes));
            }
            Some(name)
        }
        AssetMode::Inline(budget) => {
            if bytes.len() > *budget {
                warnings.push(format!("{name} is too large to inline in the preview"));
                return None;
            }
            *budget -= bytes.len();
            Some(format!("data:{mime};base64,{}", STANDARD.encode(&bytes)))
        }
    }
}

/// The brand kit that themes the site: the project's kit, else the
/// document's `brand` reference, else a `uses_tokens` link — at its approved
/// version (else head); a publish re-render uses the pinned version.
async fn resolve_brand<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    site: &DesignArtifact,
    doc: &SiteDoc,
    pinned: Option<&[PinnedRef]>,
    warnings: &mut Vec<String>,
) -> SiteResult<(Option<Value>, Option<PinnedRef>)> {
    let (uri, pinned_vid): (Option<String>, Option<String>) = match pinned {
        Some(p) => match pinned_role(p, "brand") {
            Some(b) => (Some(b.uri.clone()), b.version_id.clone()),
            None => (None, None),
        },
        None => {
            let mut uri: Option<String> = None;
            if let Some(pid) = &site.project_id {
                if let Some(p) = svc.store().get_project(pid).await? {
                    uri = p.brand_kit_id.map(|k| format!("{PREFIX}{k}"));
                }
            }
            if uri.is_none() {
                uri = doc.brand.clone().filter(|b| !b.trim().is_empty());
            }
            if uri.is_none() {
                uri = svc
                    .store()
                    .links_out(&site.id)
                    .await?
                    .into_iter()
                    .find(|l| l.rel == "uses_tokens" && l.dst_kind == "artifact" && !l.broken)
                    .map(|l| format!("{PREFIX}{}", l.dst_id));
            }
            (uri, None)
        }
    };
    let Some(uri) = uri else {
        return Ok((None, None));
    };
    let Some(u) = DesignUri::parse(&uri) else {
        warnings.push(format!(
            "the brand reference {uri} is malformed — using the default palette"
        ));
        return Ok((None, None));
    };
    let art = viewable(ctx, svc, user, &u.artifact_id)
        .await
        .filter(|a| a.format == "otto-brand");
    let version = match &art {
        Some(a) => version_for(svc, a, &u.version, pinned_vid.as_deref()).await,
        None => None,
    };
    let value = match &version {
        Some(v) => svc
            .version_bytes(v)
            .await
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok()),
        None => None,
    };
    let missing = value.is_none();
    if missing {
        warnings.push(format!(
            "the brand kit {} couldn't be read — using the default palette",
            u.artifact_id
        ));
    }
    let r = PinnedRef {
        role: "brand".into(),
        uri: uri.clone(),
        artifact_id: u.artifact_id.clone(),
        version_id: version.as_ref().map(|v| v.id.clone()),
        seq: version.as_ref().map(|v| v.seq),
        title: art.as_ref().map(|a| a.title.clone()).unwrap_or_default(),
        policy: u.policy_for("uses_tokens").into(),
        missing,
    };
    Ok((value, Some(r)))
}

/// Resolve the brand kit + every rendered reference of `doc` (a site
/// version) into a [`Resolved`], placing images per `mode`.
#[allow(clippy::too_many_arguments)]
async fn resolve<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    site: &DesignArtifact,
    version: &DesignVersion,
    doc: &SiteDoc,
    pinned: Option<&[PinnedRef]>,
    mut mode: AssetMode,
) -> SiteResult<Resolved> {
    let mut warnings: Vec<String> = Vec::new();
    let (kit, brand_ref) = resolve_brand(ctx, svc, user, site, doc, pinned, &mut warnings).await?;
    let theme = Theme::from_kit(kit.as_ref());
    let mut embeds: HashMap<String, EmbedInfo> = HashMap::new();
    let mut assets: HashMap<String, String> = HashMap::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    // The pinned set: the site version first, then its kit, then each reference.
    let mut refs: Vec<PinnedRef> = vec![PinnedRef {
        role: "site".into(),
        uri: format!("{PREFIX}{}@v{}", site.id, version.seq),
        artifact_id: site.id.clone(),
        version_id: Some(version.id.clone()),
        seq: Some(version.seq),
        title: site.title.clone(),
        policy: "pinned".into(),
        missing: false,
    }];
    refs.extend(brand_ref);
    let all = doc.rendered_refs();
    if all.len() > MAX_REFS {
        warnings.push(format!(
            "only the first {MAX_REFS} of {} linked artifacts are resolved",
            all.len()
        ));
    }
    for (uri, role) in all.into_iter().take(MAX_REFS) {
        let Some(u) = DesignUri::parse(&uri) else {
            warnings.push(format!("{uri} is not a valid design reference"));
            continue;
        };
        let pinned_vid = pinned.and_then(|p| pinned_version(p, &uri));
        let art = viewable(ctx, svc, user, &u.artifact_id).await;
        let version = match &art {
            Some(a) => version_for(svc, a, &u.version, pinned_vid.as_deref()).await,
            None => None,
        };
        let title = art.as_ref().map(|a| a.title.clone()).unwrap_or_default();
        let seq = version.as_ref().map(|v| v.seq);
        let policy = u.policy_for("embeds").to_string();
        let mut missing = art.is_none() || version.is_none();
        match role {
            "embed" => {
                // The poster is the 3D artifact's rendered thumbnail.
                let mut poster: Option<String> = None;
                if let Some(sha) = art.as_ref().and_then(|a| a.thumb_blob.clone()) {
                    if let Ok(bytes) = svc.blobs().get(&sha).await {
                        if let Some((ext, mime)) = sniff_image(&bytes) {
                            let name = format!(
                                "assets/{}-v{}-poster.{ext}",
                                u.artifact_id,
                                seq.unwrap_or(0)
                            );
                            poster = place(&mut mode, &mut files, name, mime, bytes, &mut warnings);
                        }
                    }
                }
                if missing {
                    warnings.push(format!(
                        "the 3D embed {uri} is missing — a stand-in card is shown"
                    ));
                }
                embeds.insert(
                    uri.clone(),
                    EmbedInfo {
                        title: title.clone(),
                        seq,
                        policy: policy.clone(),
                        poster,
                        broken: missing,
                    },
                );
            }
            _ => {
                if let Some(v) = &version {
                    match svc.version_bytes(v).await {
                        Ok(bytes) => match sniff_image(&bytes) {
                            Some((ext, mime)) => {
                                let name = format!("assets/{}-v{}.{ext}", u.artifact_id, v.seq);
                                if let Some(url) =
                                    place(&mut mode, &mut files, name, mime, bytes, &mut warnings)
                                {
                                    assets.insert(uri.clone(), url);
                                }
                            }
                            None => {
                                missing = true;
                                warnings
                                    .push(format!("{uri} is not a PNG, JPEG, GIF or WebP image"));
                            }
                        },
                        Err(_) => missing = true,
                    }
                }
                if missing && !matches!(mode, AssetMode::Pins) {
                    warnings.push(format!(
                        "the image {uri} is missing — a placeholder is shown"
                    ));
                }
            }
        }
        refs.push(PinnedRef {
            role: role.to_string(),
            uri,
            artifact_id: u.artifact_id,
            version_id: version.map(|v| v.id),
            seq,
            title,
            policy,
            missing,
        });
    }
    Ok(Resolved {
        theme,
        embeds,
        assets,
        files,
        pinned: refs,
        warnings,
    })
}

/// Query values echoed into URLs are ids / `v12` — keep only those characters.
fn q_safe(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(80)
        .collect()
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn export_route<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<SiteExportReq>,
) -> SiteResult<Response> {
    let target = req.target.trim().to_ascii_lowercase();
    if target != "zip" && target != "local" {
        return Err(SiteErr(Error::Invalid(format!(
            "target must be zip or local, not {:?} (publishing outside this Mac is not built)",
            req.target
        ))));
    }
    let svc = ctx.design();
    let a = load_site(&ctx, &svc, &user, &id, WorkspaceRole::Editor).await?;
    let v = site_version(&svc, &a, req.version.as_deref()).await?;
    let doc = SiteDoc::parse(&svc.version_bytes(&v).await?)?;
    if doc.pages.is_empty() {
        return Err(SiteErr(Error::Invalid(
            "this site has no pages yet — pick a template or add a page first".into(),
        )));
    }
    let mode = if target == "zip" {
        AssetMode::Files
    } else {
        AssetMode::Pins
    };
    let r = resolve(&ctx, &svc, &user, &a, &v, &doc, None, mode).await?;
    let pinned_value =
        serde_json::to_value(&r.pinned).map_err(|e| Error::Internal(format!("pinned set: {e}")))?;

    if target == "zip" {
        let files = export::static_files(&doc, &r);
        let total: usize = files.iter().map(|(_, b)| b.len()).sum();
        if total > MAX_EXPORT_BYTES {
            return Err(SiteErr(Error::PayloadTooLarge(format!(
                "the export would be {} MB (cap {} MB)",
                total / (1024 * 1024),
                MAX_EXPORT_BYTES / (1024 * 1024)
            ))));
        }
        let zip = super::zip::write(&files, super::zip::dos_datetime(v.created_at));
        let publish = svc
            .store()
            .insert_publish(NewPublish {
                artifact_id: a.id.clone(),
                version_id: v.id.clone(),
                target: "zip".into(),
                url: None,
                pinned_set: pinned_value,
                created_by: user.id.clone(),
            })
            .await?;
        let name = export::zip_name(&a.title, v.seq);
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/zip")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            )
            .header(header::CACHE_CONTROL, "no-store")
            .header("x-content-type-options", "nosniff")
            .header("x-design-version", v.id.as_str())
            .header("x-design-seq", v.seq.to_string())
            .header("x-design-publish", publish.id.as_str())
            .body(Body::from(zip))
            .map_err(|e| SiteErr(Error::Internal(format!("build response: {e}"))));
    }

    let base = format!("/api/v1/design/artifacts/{}/preview", a.id);
    let publish = svc
        .store()
        .insert_publish(NewPublish {
            artifact_id: a.id.clone(),
            version_id: v.id.clone(),
            target: "local".into(),
            url: Some(format!("{base}?version={}", q_safe(&v.id))),
            pinned_set: pinned_value,
            created_by: user.id.clone(),
        })
        .await?;
    let query = format!("?publish={}", q_safe(&publish.id));
    let pages = doc
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| SitePage {
            id: p.id.clone(),
            title: p.title.clone(),
            slug: p.slug.clone(),
            file: page_file_name(&doc, i),
            url: match export::preview_segment(&doc, i) {
                Some(seg) => format!("{base}/{seg}{query}"),
                None => format!("{base}{query}"),
            },
        })
        .collect();
    Ok(Json(SiteLocalResp {
        publish,
        url: format!("{base}{query}"),
        pages,
        pinned: r.pinned,
        warnings: r.warnings,
    })
    .into_response())
}

async fn preview_home<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Query(q): Query<PreviewQuery>,
) -> SiteResult<Response> {
    preview(&ctx, &user, &id, None, q).await
}

async fn preview_page<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(PagePath { id, page }): Path<PagePath>,
    Query(q): Query<PreviewQuery>,
) -> SiteResult<Response> {
    preview(&ctx, &user, &id, Some(page), q).await
}

async fn preview<S: DesignCtx>(
    ctx: &S,
    user: &User,
    id: &str,
    page: Option<String>,
    q: PreviewQuery,
) -> SiteResult<Response> {
    let svc = ctx.design();
    let a = load_site(ctx, &svc, user, id, WorkspaceRole::Viewer).await?;
    let publish_id = q
        .publish
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let (v, pinned): (DesignVersion, Option<Vec<PinnedRef>>) = match &publish_id {
        Some(pid) => {
            let p = svc
                .store()
                .list_publishes(&a.id)
                .await?
                .into_iter()
                .find(|p| p.id == *pid)
                .ok_or_else(|| Error::NotFound(format!("publish {pid} of site {}", a.id)))?;
            let v = svc
                .store()
                .get_version(&p.version_id)
                .await?
                .ok_or_else(|| Error::NotFound(format!("design version {}", p.version_id)))?;
            (v, serde_json::from_value(p.pinned_set).ok())
        }
        None => (site_version(&svc, &a, q.version.as_deref()).await?, None),
    };
    let doc = SiteDoc::parse(&svc.version_bytes(&v).await?)?;
    let idx = export::page_index(&doc, page.as_deref()).ok_or_else(|| {
        Error::NotFound(format!(
            "page {:?} of site {}",
            page.clone().unwrap_or_default(),
            a.id
        ))
    })?;
    let r = resolve(
        ctx,
        &svc,
        user,
        &a,
        &v,
        &doc,
        pinned.as_deref(),
        AssetMode::Inline(MAX_PREVIEW_INLINE),
    )
    .await?;
    let base = format!("/api/v1/design/artifacts/{}/preview", a.id);
    let query = match (
        &publish_id,
        q.version
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        (Some(p), _) => format!("?publish={}", q_safe(p)),
        (None, Some(sel)) => format!("?version={}", q_safe(sel)),
        (None, None) => String::new(),
    };
    let html = export::preview_html(&doc, idx, &r, &base, &query);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CONTENT_SECURITY_POLICY, PREVIEW_CSP)
        .header("x-content-type-options", "nosniff")
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-design-version", v.id.as_str())
        .header("x-design-seq", v.seq.to_string())
        .body(Body::from(html))
        .map_err(|e| SiteErr(Error::Internal(format!("build response: {e}"))))
}

async fn publishes_route<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
) -> SiteResult<Response> {
    let svc = ctx.design();
    let a = svc.store().require_artifact(&id).await?;
    ctx.roles()
        .check(&user, &a.workspace_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(svc.store().list_publishes(&a.id).await?).into_response())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::{Method, Request};
    use http_body_util::BodyExt;
    use otto_core::auth::{BoxFuture, RoleChecker};
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;

    /// Allows every workspace except `w-denied`.
    struct Roles;

    impl RoleChecker for Roles {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            workspace_id: &'a Id,
            _min: WorkspaceRole,
        ) -> BoxFuture<'a, otto_core::Result<()>> {
            let denied = workspace_id == "w-denied";
            Box::pin(async move {
                if denied {
                    Err(Error::Forbidden("not a member".into()))
                } else {
                    Ok(())
                }
            })
        }
    }

    #[derive(Clone)]
    struct TestCtx {
        pool: sqlx::SqlitePool,
        dir: Arc<tempfile::TempDir>,
        roles: Arc<dyn RoleChecker>,
    }

    impl DesignCtx for TestCtx {
        fn design(&self) -> DesignService {
            DesignService::new(self.pool.clone(), self.dir.path(), None)
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
    }

    async fn app() -> Router {
        let ctx = TestCtx {
            pool: otto_state::db::test_pool().await,
            dir: Arc::new(tempfile::tempdir().unwrap()),
            roles: Arc::new(Roles),
        };
        let _ = ctx.design().store().ensure_fts().await;
        let user = User {
            id: "u1".into(),
            username: "u1".into(),
            display_name: "U1".into(),
            is_root: false,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        crate::http::router::<TestCtx>()
            .with_state(ctx)
            .layer(Extension(AuthUser(user)))
    }

    async fn call(
        app: &Router,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Vec<u8>, axum::http::HeaderMap) {
        let mut b = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(v) => {
                b = b.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, bytes, headers)
    }

    fn j(b: &[u8]) -> Value {
        serde_json::from_slice(b).unwrap_or(Value::Null)
    }

    async fn create(
        app: &Router,
        format: &str,
        studio: &str,
        title: &str,
        content: String,
    ) -> (String, String) {
        let (st, b, _) = call(
            app,
            Method::POST,
            "/design/artifacts",
            Some(json!({ "workspace_id": "w1", "format": format, "studio": studio, "title": title, "content": content })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));
        let v = j(&b);
        (
            v["artifact"]["id"].as_str().unwrap().to_string(),
            v["version"]["id"].as_str().unwrap().to_string(),
        )
    }

    fn site_doc(kit: &str, card: &str) -> Value {
        json!({
            "type": "otto-site", "version": 1, "title": "Rewards+",
            "brand": format!("otto://design/{kit}"),
            "pages": [
                { "id": "home", "title": "Home", "slug": "", "sections": [
                    { "id": "nav", "block": "nav/bar", "props": { "logo": "acme Rewards+" },
                      "blocks": [{ "id": "l1", "block": "item/link", "props": { "label": "Tiers", "href": "page:tiers" } }] },
                    { "id": "hero", "block": "hero/split",
                      "props": { "headline": "Every purchase moves you up.", "primary_label": "Join free", "primary_href": "#join" },
                      "style": { "background": "token:color.surface-alt", "motion": "fade-up" },
                      "blocks": [{ "id": "card", "block": "embed/3d", "props": { "src": format!("otto://design/{card}@approved"), "alt": "The card" } }] }
                ]},
                { "id": "tiers", "title": "Tiers", "slug": "tiers", "sections": [
                    { "id": "t", "block": "pricing/tiers", "props": { "heading": "Climb faster." },
                      "blocks": [{ "id": "gold", "block": "item/tier", "props": { "name": "Gold", "price": "2,500", "features": ["Free delivery"] } }] }
                ]}
            ]
        })
    }

    async fn fixture(app: &Router) -> (String, String, String) {
        let kit = json!({ "$schema": "otto-brand/1", "name": "Acme",
            "color": { "primary": { "$value": "#5B3DF5" }, "surface-alt": { "$value": "#F6F4FF" } } });
        let (kit_id, _) = create(
            app,
            "otto-brand",
            "brand",
            "Acme Brand Kit",
            kit.to_string(),
        )
        .await;
        let (card_id, card_v1) = create(
            app,
            "scene3d",
            "3d",
            "Rewards Card 3D",
            json!({ "type": "otto-scene3d", "version": 1, "objects": [] }).to_string(),
        )
        .await;
        let (st, _, _) = call(
            app,
            Method::POST,
            &format!("/design/artifacts/{card_id}/approve"),
            Some(json!({})),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (site_id, _) = create(
            app,
            "otto-site",
            "site",
            "Rewards+ landing page",
            site_doc(&kit_id, &card_id).to_string(),
        )
        .await;
        (site_id, card_id, card_v1)
    }

    #[tokio::test]
    async fn invalid_site_documents_are_refused_on_save() {
        let app = app().await;
        let (st, b, _) = call(
            &app,
            Method::POST,
            "/design/artifacts",
            Some(json!({ "workspace_id": "w1", "format": "otto-site", "title": "Bad",
                "content": json!({ "type": "otto-site", "version": 1,
                    "pages": [{ "id": "home", "sections": [{ "id": "x", "block": "hero/fancy" }] }] }).to_string() })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(String::from_utf8_lossy(&b).contains("unknown section block"));
    }

    #[tokio::test]
    async fn zip_export_ships_pages_one_stylesheet_with_tokens_and_records_a_pinned_publish() {
        let app = app().await;
        let (site, card, card_v1) = fixture(&app).await;
        let (st, body, h) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{site}/export"),
            Some(json!({ "target": "zip" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        assert_eq!(h.get("content-type").unwrap(), "application/zip");
        assert!(h
            .get("content-disposition")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("rewards-landing-page-v1.zip"));
        let files = super::super::zip::read(&body).expect("a readable zip");
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["index.html", "tiers.html", "site.css", "otto-publish.json"]
        );
        let text = |n: &str| {
            String::from_utf8(files.iter().find(|(x, _)| x == n).unwrap().1.clone()).unwrap()
        };
        let index = text("index.html");
        assert!(index.contains("Every purchase moves you up."));
        assert!(index.contains("href=\"tiers.html\""));
        assert!(
            index.contains("os-card3d__name\">Rewards Card 3D<"),
            "the embed names its artifact"
        );
        assert!(!index.to_ascii_lowercase().contains("<script"));
        let css = text("site.css");
        assert!(css.contains("--brand-color-primary: #5B3DF5;"));
        assert!(css.contains("--brand-color-surface-alt: #F6F4FF;"));
        assert!(css.contains("--os-surface-alt: var(--brand-color-surface-alt);"));

        // The publish row pins the site, the kit and the embed's approved version.
        let publish_id = h
            .get("x-design-publish")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let (st, b, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{site}/publishes"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let rows = j(&b);
        assert_eq!(rows[0]["id"], publish_id.as_str());
        assert_eq!(rows[0]["target"], "zip");
        // An array of {role, artifact_id, version_id, …} — the shape the
        // retention prune reads, so no pinned version is ever pruned.
        let pinned = &rows[0]["pinned_set"];
        assert_eq!(pinned[0]["role"], "site");
        assert_eq!(pinned[0]["artifact_id"], site.as_str());
        assert_eq!(pinned[0]["seq"], 1);
        assert_eq!(pinned[1]["role"], "brand");
        assert_eq!(pinned[1]["missing"], false);
        assert_eq!(pinned[2]["role"], "embed");
        assert_eq!(pinned[2]["artifact_id"], card.as_str());
        assert_eq!(pinned[2]["version_id"], card_v1.as_str());
        assert_eq!(pinned[2]["policy"], "follow_approved");
        let manifest: Value = serde_json::from_str(&text("otto-publish.json")).unwrap();
        assert_eq!(&manifest, pinned);
    }

    #[tokio::test]
    async fn local_publish_and_preview_render_pages_inside_the_route() {
        let app = app().await;
        let (site, _, _) = fixture(&app).await;
        let (st, b, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{site}/export"),
            Some(json!({ "target": "local" })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
        let resp = j(&b);
        let url = resp["url"].as_str().unwrap().to_string();
        assert!(url.starts_with(&format!("/api/v1/design/artifacts/{site}/preview?publish=")));
        assert_eq!(resp["pages"][1]["file"], "tiers.html");
        let publish = resp["publish"]["id"].as_str().unwrap().to_string();

        let (st, html, h) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{site}/preview?publish={publish}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(h
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("sandbox"));
        let html = String::from_utf8(html).unwrap();
        assert!(html.contains("<style>"), "the stylesheet is inlined");
        assert!(html.contains(&format!(
            "href=\"/api/v1/design/artifacts/{site}/preview/tiers?publish={publish}\""
        )));
        let (st, tiers, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{site}/preview/tiers"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(String::from_utf8(tiers).unwrap().contains("Climb faster."));
        let (st, _, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{site}/preview/nope"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND);

        // Only zip / local; never an outward target.
        let (st, _, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{site}/export"),
            Some(json!({ "target": "claude_artifact" })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn images_are_sniffed_and_query_values_sanitized() {
        assert_eq!(
            sniff_image(&[0x89, 0x50, 0x4E, 0x47, 0x0D]),
            Some(("png", "image/png"))
        );
        assert_eq!(
            sniff_image(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(("webp", "image/webp"))
        );
        assert_eq!(sniff_image(b"<svg"), None);
        assert_eq!(q_safe("v12\"><script>"), "v12script");
    }
}
