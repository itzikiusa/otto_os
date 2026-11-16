//! Shared test context and HTTP helpers for `otto-ide` integration suites.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::{Extension, Router};
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::{AuthUser, BoxFuture, RoleChecker};
use otto_core::domain::User;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_ide::netbridge::IdeK8sSpec;
use otto_ide::{IdeCtx, IdeEngine, IdeSshSpec};
use otto_rbac::RbacRoleChecker;
use otto_state::DbPool;
use serde_json::Value;
use tower::ServiceExt;

#[derive(Default)]
pub struct MemSecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemSecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[derive(Clone)]
pub struct TestSshEntry {
    pub owner_user_id: Option<String>,
    pub spec: IdeSshSpec,
}

#[derive(Clone)]
pub struct TestK8sEntry {
    pub owner_user_id: Option<String>,
    pub allowed_namespaces: Vec<String>,
    pub spec: IdeK8sSpec,
}

#[derive(Clone)]
pub struct TestIdeCtx {
    pub ide: Arc<IdeEngine>,
    pub pool: DbPool,
    pub secrets: Arc<dyn SecretStore>,
    pub data_dir: PathBuf,
    pub roles: Arc<dyn RoleChecker>,
    pub allowed_roots: Vec<PathBuf>,
    /// When `Some`, `authorize_local_target(..., write = true)` additionally
    /// requires every target root to fall inside `writable_roots` (modeling
    /// `WorkspaceRole::Viewer` vs `WorkspaceRole::Editor`). `None` defaults to
    /// treating all `allowed_roots` as writable.
    pub writable_roots: Option<Vec<PathBuf>>,
    pub ssh_connections: Arc<Mutex<HashMap<String, TestSshEntry>>>,
    pub k8s_clusters: Arc<Mutex<HashMap<String, TestK8sEntry>>>,
}

impl IdeCtx for TestIdeCtx {
    fn ide(&self) -> &Arc<IdeEngine> {
        &self.ide
    }

    fn pool(&self) -> DbPool {
        self.pool.clone()
    }

    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }

    fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }

    fn resolve_ssh_connection<'a>(
        &'a self,
        user: &'a User,
        connection_id: &'a Id,
    ) -> BoxFuture<'a, Result<IdeSshSpec>> {
        let cid = connection_id.clone();
        Box::pin(async move {
            let entry = self
                .ssh_connections
                .lock()
                .unwrap()
                .get(cid.as_str())
                .cloned()
                .ok_or_else(|| Error::NotFound(format!("ssh connection {cid}")))?;
            if !otto_core::auth::root_authority(user)
                && entry.owner_user_id.as_deref() != Some(user.id.as_str())
            {
                return Err(Error::Forbidden(
                    "SSH connection belongs to another user or workspace".into(),
                ));
            }
            Ok(entry.spec)
        })
    }

    fn resolve_k8s_cluster<'a>(
        &'a self,
        user: &'a User,
        cluster_id: &'a Id,
        namespace: Option<&'a str>,
    ) -> BoxFuture<'a, Result<IdeK8sSpec>> {
        let cid = cluster_id.clone();
        let ns_override = namespace.map(str::to_string);
        Box::pin(async move {
            let entry = self
                .k8s_clusters
                .lock()
                .unwrap()
                .get(cid.as_str())
                .cloned()
                .ok_or_else(|| Error::NotFound(format!("k8s cluster {cid}")))?;
            let effective_ns = ns_override
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&entry.spec.namespace)
                .to_ascii_lowercase();
            let has_root_auth = otto_core::auth::root_authority(user);
            if !has_root_auth {
                if entry.owner_user_id.as_deref() != Some(user.id.as_str()) {
                    return Err(Error::Forbidden(
                        "Kubernetes cluster belongs to another user or workspace".into(),
                    ));
                }
                if !entry.allowed_namespaces.is_empty()
                    && !entry.allowed_namespaces.iter().any(|n| n == &effective_ns)
                {
                    return Err(Error::Forbidden(format!(
                        "namespace '{effective_ns}' is not allowed on cluster {cid}"
                    )));
                }
            }
            let allow_cross_namespace = has_root_auth || entry.allowed_namespaces.is_empty();
            Ok(IdeK8sSpec {
                namespace: effective_ns,
                allow_cross_namespace,
                ..entry.spec
            })
        })
    }

    fn authorize_local_target<'a>(
        &'a self,
        user: &'a User,
        spec: &'a otto_ide::types::IdeTargetSpec,
        write: bool,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let has_root_auth = otto_core::auth::root_authority(user);
            if has_root_auth || spec.kind != otto_ide::types::IdeTargetKind::Local {
                return Ok(());
            }
            let raw_root = spec.root.trim();
            if raw_root.is_empty() {
                return Err(Error::Invalid("target root must not be empty".into()));
            }
            let mut candidates = vec![raw_root.to_string()];
            for extra in spec.merged_extra_roots() {
                let t = extra.trim();
                if !t.is_empty() {
                    candidates.push(t.to_string());
                }
            }
            let allowed_canon: Vec<PathBuf> = self
                .allowed_roots
                .iter()
                .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
                .collect();
            let writable_canon: Option<Vec<PathBuf>> = self.writable_roots.as_ref().map(|wr| {
                wr.iter()
                    .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
                    .collect()
            });
            let data_canon =
                std::fs::canonicalize(&self.data_dir).unwrap_or_else(|_| self.data_dir.clone());
            for raw in candidates {
                let expanded =
                    otto_core::paths::expand_home_strict(&raw).map_err(Error::Invalid)?;
                let canon = std::fs::canonicalize(&expanded).unwrap_or_else(|_| expanded.clone());
                if !has_root_auth {
                    otto_ide::target::enforce_non_root_path(&canon)?;
                    if canon.starts_with(&data_canon) {
                        return Err(Error::Forbidden(
                            "IDE workspace root cannot be inside Otto data_dir".into(),
                        ));
                    }
                }
                if !allowed_canon.iter().any(|ar| canon.starts_with(ar)) {
                    return Err(Error::Forbidden(
                        "target root is not inside a workspace you have permission to access"
                            .into(),
                    ));
                }
                if write {
                    if let Some(ref wr) = writable_canon {
                        if !wr.iter().any(|w| canon.starts_with(w)) {
                            return Err(Error::Forbidden(
                                "workspace editor role required for IDE writes and executions"
                                    .into(),
                            ));
                        }
                    }
                }
            }
            Ok(())
        })
    }
}

pub fn root_user() -> User {
    User {
        id: "u_root_test".into(),
        username: "root".into(),
        display_name: "Root Test".into(),
        is_root: true,
        disabled: false,
        created_at: Utc::now(),
    }
}

pub fn non_root_user(id: &str) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: format!("User {id}"),
        is_root: false,
        disabled: false,
        created_at: Utc::now(),
    }
}

pub fn router_as(ctx: &TestIdeCtx, user: User, allowed_roots: Vec<PathBuf>) -> Router {
    let mut scoped = ctx.clone();
    scoped.allowed_roots = allowed_roots;
    scoped.writable_roots = None;
    otto_ide::api_router::<TestIdeCtx>()
        .layer(Extension(AuthUser(user)))
        .with_state(scoped)
}

pub fn router_as_with_write_roots(
    ctx: &TestIdeCtx,
    user: User,
    allowed_roots: Vec<PathBuf>,
    writable_roots: Vec<PathBuf>,
) -> Router {
    let mut scoped = ctx.clone();
    scoped.allowed_roots = allowed_roots;
    scoped.writable_roots = Some(writable_roots);
    otto_ide::api_router::<TestIdeCtx>()
        .layer(Extension(AuthUser(user)))
        .with_state(scoped)
}

pub fn public_router_for(ctx: &TestIdeCtx) -> Router {
    otto_ide::public_router::<TestIdeCtx>().with_state(ctx.clone())
}

pub async fn build_test_app_as(
    data_dir: PathBuf,
    db_path: &Path,
    user: User,
    allowed_roots: Vec<PathBuf>,
) -> (TestIdeCtx, Router) {
    std::fs::create_dir_all(&data_dir).unwrap();
    let pool = otto_state::open(db_path).await.unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
    let roles: Arc<dyn RoleChecker> = Arc::new(RbacRoleChecker::new(pool.clone()));
    let ide = Arc::new(IdeEngine::new(data_dir.clone()));
    let ctx = TestIdeCtx {
        ide,
        pool,
        secrets,
        data_dir,
        roles,
        allowed_roots: allowed_roots.clone(),
        writable_roots: None,
        ssh_connections: Arc::new(Mutex::new(HashMap::new())),
        k8s_clusters: Arc::new(Mutex::new(HashMap::new())),
    };
    let router = router_as(&ctx, user, allowed_roots);
    (ctx, router)
}

pub async fn send_raw(
    app: &Router,
    method: Method,
    uri: &str,
) -> (StatusCode, Option<String>, bytes::Bytes) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, content_type, bytes)
}

pub async fn build_test_app(data_dir: PathBuf, db_path: &Path) -> (TestIdeCtx, Router) {
    build_test_app_as(data_dir, db_path, root_user(), Vec::new()).await
}

pub async fn send_json(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req_builder = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(val) => req_builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&val).unwrap()))
            .unwrap(),
        None => req_builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(
            |_| serde_json::json!({ "_raw": String::from_utf8_lossy(&bytes).into_owned() }),
        )
    };
    (status, parsed)
}

/// GET `uri` as a collab guest: the invite token travels in the
/// `X-Ide-Collab-Token` header (the query form is refused).
pub async fn guest_get_json(app: &Router, uri: &str, token: &str) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header("x-ide-collab-token", token)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() * 2);
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn extended_path() -> String {
    let cur = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/go/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:{cur}")
}

pub fn find_bin(name: &str) -> Option<String> {
    for dir in extended_path().split(':').filter(|s| !s.is_empty()) {
        let cand = Path::new(dir).join(name);
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

pub fn ensure_bins_or_skip(bins: &[&str]) -> bool {
    let missing: Vec<&str> = bins
        .iter()
        .copied()
        .filter(|b| find_bin(b).is_none())
        .collect();
    if missing.is_empty() {
        return true;
    }
    let required = std::env::var("OTTO_REQUIRE_TOOLCHAINS")
        .map(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);
    assert!(
        !required,
        "OTTO_REQUIRE_TOOLCHAINS=1: missing required debugger toolchain binary/binaries: {}",
        missing.join(", ")
    );
    false
}
