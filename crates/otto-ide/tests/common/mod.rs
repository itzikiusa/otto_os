//! Shared test context & HTTP helper for `otto-ide` Round 2 integration suites.
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
pub struct TestIdeCtx {
    pub ide: Arc<IdeEngine>,
    pub pool: DbPool,
    pub secrets: Arc<dyn SecretStore>,
    pub data_dir: PathBuf,
    pub roles: Arc<dyn RoleChecker>,
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
        _user: &'a User,
        connection_id: &'a Id,
    ) -> BoxFuture<'a, Result<IdeSshSpec>> {
        let cid = connection_id.clone();
        Box::pin(async move { Err(Error::NotFound(format!("ssh connection {cid}"))) })
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

pub async fn build_test_app(data_dir: PathBuf, db_path: &Path) -> (TestIdeCtx, Router) {
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
    };
    let router = otto_ide::api_router::<TestIdeCtx>()
        .layer(Extension(AuthUser(root_user())))
        .with_state(ctx.clone());
    (ctx, router)
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
