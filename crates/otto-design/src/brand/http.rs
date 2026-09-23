//! Brand Kit routes (merged into `crate::http::router`; `/design/*` RBAC —
//! GET = design View, POST = design Edit — comes from the server's policy
//! table, the workspace role from the kit's row):
//!
//!   POST /design/artifacts/{id}/brand/impact  (ws viewer) → BrandImpactResp
//!   GET  /design/artifacts/{id}/brand/export  (ws viewer) → css | tailwind | dtcg text
//!
//! Saving a kit is the ordinary `PUT …/content` (validated by
//! `brand::validate` via `format::validate`); approving a version is the
//! ordinary `POST …/approve`, whose `design_link_updated {reason:
//! "target_approved"}` fan-out tells every `follow_approved` consumer.

use std::collections::{BTreeSet, HashMap};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id};
use serde::Deserialize;
use serde_json::Value;

use super::{contrast, doc, export, impact};
use crate::format::{self, Encoding};
use crate::http::DesignCtx;
use crate::service::DesignService;
use crate::types::DesignArtifact;

/// Consumers whose head is read to find token references (the rest are
/// listed as whole-kit — never under-reported).
const MAX_SCANNED: usize = 300;
/// Larger consumer heads are not scanned.
const MAX_SCAN_BYTES: usize = 4 * 1024 * 1024;

struct BrandErr(Error);

impl From<Error> for BrandErr {
    fn from(e: Error) -> Self {
        BrandErr(e)
    }
}

impl IntoResponse for BrandErr {
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

type BrandResult<T> = std::result::Result<T, BrandErr>;

#[derive(Deserialize)]
struct IdPath {
    id: Id,
}

#[derive(Deserialize, Default)]
struct ExportQuery {
    /// `css` (default) | `tailwind` | `dtcg`.
    #[serde(default)]
    format: Option<String>,
    /// A version id / `v12` / `12` / `approved`; default the head.
    #[serde(default)]
    version: Option<String>,
}

/// The brand routes, relative to the `/api/v1` mount point.
pub fn routes<S: DesignCtx>() -> Router<S> {
    Router::new()
        .route(
            "/design/artifacts/{id}/brand/impact",
            post(impact_route::<S>),
        )
        .route(
            "/design/artifacts/{id}/brand/export",
            get(export_route::<S>),
        )
}

/// The kit (an `otto-brand` artifact) the caller may view.
async fn load_kit<S: DesignCtx>(
    ctx: &S,
    svc: &DesignService,
    user: &User,
    id: &str,
) -> BrandResult<DesignArtifact> {
    let a = svc.store().require_artifact(id).await?;
    ctx.roles()
        .check(user, &a.workspace_id, WorkspaceRole::Viewer)
        .await?;
    if a.format != "otto-brand" {
        return Err(BrandErr(Error::Invalid(format!(
            "design artifact {} is a {} document, not a brand kit (otto-brand)",
            a.id, a.format
        ))));
    }
    Ok(a)
}

/// A consumer's head as text, when it is a text/JSON format of sane size.
async fn consumer_text(svc: &DesignService, a: &DesignArtifact) -> Option<String> {
    let spec = format::spec(&a.format)?;
    if spec.encoding == Encoding::Binary {
        return None;
    }
    let (_, bytes) = svc.head_content(a).await.ok()?;
    if bytes.len() > MAX_SCAN_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

async fn impact_route<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Json(req): Json<impact::BrandImpactReq>,
) -> BrandResult<Response> {
    let svc = ctx.design();
    let kit = load_kit(&ctx, &svc, &user, &id).await?;

    // Baseline: what consumers follow — the approved version, else the head.
    let base_version = match (&kit.approved_version_id, &kit.head_version_id) {
        (Some(v), _) | (None, Some(v)) => svc.store().get_version(v).await?,
        (None, None) => None,
    };
    let base = match (&base_version, &kit.approved_version_id) {
        (None, _) => "none",
        (Some(_), Some(_)) => "approved",
        (Some(_), None) => "head",
    };
    let base_model = match &base_version {
        Some(v) => doc::BrandModel::from_bytes(&svc.version_bytes(v).await?),
        None => doc::BrandModel::default(),
    };

    // The proposal; omitted = the head (a plain "who uses this kit" listing).
    let proposed_value: Value = match req.content {
        Some(Value::String(s)) => {
            let v: Value = serde_json::from_str(&s)
                .map_err(|e| Error::Invalid(format!("content is not valid JSON: {e}")))?;
            doc::validate(&v)?;
            v
        }
        Some(v) => {
            doc::validate(&v)?;
            v
        }
        None => match &kit.head_version_id {
            Some(_) => {
                let (_, bytes) = svc.head_content(&kit).await?;
                serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::Object(Default::default()))
            }
            None => Value::Object(Default::default()),
        },
    };
    let proposed = doc::BrandModel::from_value(&proposed_value);
    let changes = impact::diff(&base_model, &proposed);
    let changed: BTreeSet<String> = changes.iter().map(|c| c.token.clone()).collect();
    let css = impact::css_index(&[&base_model, &proposed]);

    // Consumers: one row per artifact with a `uses_tokens` link into the kit
    // (the first link's policy wins when a document links twice).
    let mut edges: Vec<(Id, String, Option<Id>)> = Vec::new();
    for l in svc.store().links_in(&kit.id).await? {
        if l.rel != "uses_tokens"
            || l.src_artifact_id == kit.id
            || edges.iter().any(|(s, _, _)| *s == l.src_artifact_id)
        {
            continue;
        }
        edges.push((l.src_artifact_id, l.policy, l.pinned_version_id));
    }
    let ids: Vec<String> = edges.iter().map(|(s, _, _)| s.clone()).collect();
    let mut can_view: HashMap<Id, bool> = HashMap::new();
    let mut hidden = 0i64;
    let mut scanned = 0usize;
    let mut consumers = Vec::new();
    for a in svc.store().artifacts_by_ids(&ids).await? {
        if a.status == "archived" {
            continue;
        }
        let ok = match can_view.get(&a.workspace_id) {
            Some(ok) => *ok,
            None => {
                let ok = ctx
                    .roles()
                    .check(&user, &a.workspace_id, WorkspaceRole::Viewer)
                    .await
                    .is_ok();
                can_view.insert(a.workspace_id.clone(), ok);
                ok
            }
        };
        if !ok {
            hidden += 1;
            continue;
        }
        let (policy, pinned) = edges
            .iter()
            .find(|(s, _, _)| *s == a.id)
            .map(|(_, p, v)| (p.clone(), v.clone()))
            .unwrap_or_else(|| ("follow_approved".to_string(), None));
        let text = if scanned < MAX_SCANNED {
            scanned += 1;
            consumer_text(&svc, &a).await
        } else {
            None
        };
        consumers.push(impact::consumer(
            a,
            policy,
            pinned,
            text.as_deref(),
            &css,
            &changed,
        ));
    }

    let report = contrast::report(&proposed);
    let warnings = contrast::warnings(&report);
    let resp = impact::summarize(
        kit.id.clone(),
        base,
        base_version.as_ref().map(|v| (v.id.clone(), v.seq)),
        changes,
        consumers,
        hidden,
        report,
        warnings,
    );
    Ok(Json(resp).into_response())
}

async fn export_route<S: DesignCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(IdPath { id }): Path<IdPath>,
    Query(q): Query<ExportQuery>,
) -> BrandResult<Response> {
    let svc = ctx.design();
    let kit = load_kit(&ctx, &svc, &user, &id).await?;
    let fmt_name = q.format.unwrap_or_else(|| "css".to_string());
    let fmt = export::ExportFormat::parse(&fmt_name).ok_or_else(|| {
        Error::Invalid(format!(
            "format must be css | tailwind | dtcg, not {fmt_name:?}"
        ))
    })?;
    let sel = q
        .version
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let (v, bytes) = match sel.as_deref() {
        Some("approved") => {
            let vid = kit.approved_version_id.clone().ok_or_else(|| {
                Error::NotFound(format!("brand kit {} has no approved version", kit.id))
            })?;
            let v = svc
                .store()
                .get_version(&vid)
                .await?
                .ok_or_else(|| Error::NotFound(format!("design version {vid}")))?;
            let b = svc.version_bytes(&v).await?;
            (v, b)
        }
        Some(s) => {
            let v = svc.resolve_version(&kit, s).await?;
            let b = svc.version_bytes(&v).await?;
            (v, b)
        }
        None => svc.head_content(&kit).await?,
    };
    let model = doc::BrandModel::from_bytes(&bytes);
    let body = export::export(&model, fmt);
    let name = if model.name.trim().is_empty() {
        kit.title.as_str()
    } else {
        model.name.as_str()
    };
    let file = fmt.file_name(&export::slug(name));
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, fmt.mime())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{file}\""),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .header("x-design-version", v.id.as_str())
        .header("x-design-seq", v.seq.to_string())
        .body(Body::from(body))
        .map_err(|e| BrandErr(Error::Internal(format!("build response: {e}"))))
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

    fn kit_doc(primary: &str) -> Value {
        json!({
            "$schema": "otto-brand/1",
            "name": "Acme Brand Kit",
            "color": { "primary": { "$value": primary }, "ink": { "$value": "#14122B" } },
            "radius": { "card": { "$value": 14 } }
        })
    }

    async fn create(
        app: &Router,
        ws: &str,
        format: &str,
        studio: &str,
        title: &str,
        content: String,
    ) -> String {
        let (st, b, _) = call(
            app,
            Method::POST,
            "/design/artifacts",
            Some(json!({ "workspace_id": ws, "format": format, "studio": studio, "title": title, "content": content })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));
        j(&b)["artifact"]["id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn impact_lists_consumers_and_what_a_change_reaches() {
        let app = app().await;
        let kit = create(
            &app,
            "w1",
            "otto-brand",
            "brand",
            "Acme",
            kit_doc("#5B3DF5").to_string(),
        )
        .await;

        // A site that names two tokens (the `brand` key → a `uses_tokens` link).
        let site = json!({
            "type": "otto-site", "brand": format!("otto://design/{kit}"),
            "pages": [{ "id": "home", "sections": [{ "id": "hero", "props": { "bg": "token:color.primary", "r": "token:radius.card" } }] }]
        });
        let site_id = create(&app, "w1", "otto-site", "site", "Landing", site.to_string()).await;
        // An HTML frame linked explicitly, using only the radius var.
        let frame = create(
            &app,
            "w1",
            "html",
            "frames",
            "Card",
            "<div style=\"border-radius:var(--brand-radius-card)\"></div>".into(),
        )
        .await;
        let (st, b, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{frame}/links"),
            Some(json!({ "rel": "uses_tokens", "dst_kind": "artifact", "dst_id": kit })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));

        // No content → the plain listing: no changes, both consumers.
        let (st, b, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{kit}/brand/impact"),
            Some(json!({})),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
        let r = j(&b);
        assert_eq!(r["base"], "head");
        assert_eq!(r["artifact_count"], 2);
        assert_eq!(r["studio_count"], 2);
        assert_eq!(r["affected_count"], 0);
        assert!(r["changes"].as_array().unwrap().is_empty());

        // Change the primary: only the site references it.
        let (st, b, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{kit}/brand/impact"),
            Some(json!({ "content": kit_doc("#0F9D8A") })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{}", String::from_utf8_lossy(&b));
        let r = j(&b);
        assert_eq!(r["changes"][0]["token"], "color.primary");
        assert_eq!(r["changes"][0]["after"], "#0F9D8A");
        assert_eq!(r["affected_count"], 1);
        assert_eq!(r["affected_by_studio"][0]["studio"], "site");
        let first = &r["consumers"][0];
        assert_eq!(first["artifact"]["id"], site_id.as_str());
        assert_eq!(first["affected"], json!(["color.primary"]));
        assert_eq!(first["tokens"], json!(["color.primary", "radius.card"]));
        assert_eq!(first["policy"], "follow_approved");
        assert!(r["contrast"]["colors"].as_array().unwrap().len() == 2);

        // Content as text works too; an invalid kit is a 400.
        let (st, _, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{kit}/brand/impact"),
            Some(json!({ "content": kit_doc("#123456").to_string() })),
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let (st, b, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{kit}/brand/impact"),
            Some(json!({ "content": { "color": { "primary": { "$value": "blue" } } } })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        assert!(String::from_utf8_lossy(&b).contains("color.primary.$value"));

        // Not a kit → 400.
        let (st, _, _) = call(
            &app,
            Method::POST,
            &format!("/design/artifacts/{frame}/brand/impact"),
            Some(json!({})),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn saves_are_validated_and_exports_render() {
        let app = app().await;
        // format::validate runs the brand validator on create and save.
        let (st, b, _) = call(
            &app,
            Method::POST,
            "/design/artifacts",
            Some(json!({ "workspace_id": "w1", "format": "otto-brand", "title": "Bad", "content": "{\"color\":{\"x\":{\"$value\":\"nope\"}}}" })),
        )
        .await;
        assert_eq!(
            st,
            StatusCode::BAD_REQUEST,
            "{}",
            String::from_utf8_lossy(&b)
        );
        // No content → the default kit (valid).
        let (st, b, _) = call(
            &app,
            Method::POST,
            "/design/artifacts",
            Some(json!({ "workspace_id": "w1", "format": "otto-brand", "title": "Blank" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "{}", String::from_utf8_lossy(&b));

        let kit = create(
            &app,
            "w1",
            "otto-brand",
            "brand",
            "Acme",
            kit_doc("#5B3DF5").to_string(),
        )
        .await;
        let (st, b, h) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{kit}/brand/export"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let css = String::from_utf8_lossy(&b);
        assert!(css.contains("--brand-color-primary: #5B3DF5;"), "{css}");
        assert!(h[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/css"));
        assert!(h[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("acme-brand-kit.tokens.css"));
        assert_eq!(h["x-design-seq"], "1");

        let (st, b, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{kit}/brand/export?format=tailwind"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert!(String::from_utf8_lossy(&b).contains("--color-primary: #5B3DF5;"));

        let (st, b, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{kit}/brand/export?format=dtcg&version=v1"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(j(&b)["color"]["primary"]["$value"], "#5B3DF5");

        let (st, _, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{kit}/brand/export?format=scss"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let (st, _, _) = call(
            &app,
            Method::GET,
            &format!("/design/artifacts/{kit}/brand/export?version=approved"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND, "nothing approved yet");
    }
}
