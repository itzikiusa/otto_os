//! Router-level tests for `/aws/*` against a **fake `aws` binary** (a shell
//! script on disk selected via `OTTO_AWS_BIN`). Covers account CRUD, the
//! Keychain split (secret never in the DTO, injected as env), `/test`
//! (ok / login-required), the permission probe (+ cache), S3 listing and the
//! streamed download, and the `kubernetes:Admin` gate on `import-kubeconfig`.
//!
//! The fake logs every invocation (env + argv) to `calls.log` next to itself
//! so tests can assert what the CLI was asked to do.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Extension;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_aws::AwsCtx;
use otto_connections::Spawner;
use otto_core::auth::{AuthUser, BoxFuture};
use otto_core::domain::{Connection, Session, User};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_state::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Fake `aws`
// ---------------------------------------------------------------------------

const FAKE_AWS: &str = r#"#!/bin/bash
# Fake aws CLI for otto-aws tests. Logs env+argv, answers by subcommand.
dir="$(cd "$(dirname "$0")" && pwd)"
echo "PROFILE=${AWS_PROFILE-<unset>} REGION=${AWS_REGION-<unset>} AKID=${AWS_ACCESS_KEY_ID-<unset>} SECRET=${AWS_SECRET_ACCESS_KEY-<unset>} TOKEN=${AWS_SESSION_TOKEN-<unset>} ARGS=$*" >> "$dir/calls.log"
case "$1 $2" in
  "--version ")
    echo "aws-cli/2.17.0 Python/3.11.9 Darwin/24.0.0 exe/arm64"; exit 0;;
  "sts get-caller-identity")
    if [ "$AWS_PROFILE" = "expired" ]; then
      echo "Error loading SSO Token: Token for https://corp.awsapps.com/start does not exist" >&2; exit 255
    fi
    echo '{"UserId": "AROAEXAMPLE:otto", "Account": "123456789012", "Arn": "arn:aws:sts::123456789012:assumed-role/Dev/otto"}'; exit 0;;
  "s3api list-buckets")
    echo '{"Buckets": [{"Name": "logs-prod", "CreationDate": "2021-03-01T10:00:00+00:00"}, {"Name": "unrelated-private", "CreationDate": "2021-03-01T10:00:00+00:00"}], "Owner": {"ID": "x"}}'; exit 0;;
  "s3api list-objects-v2")
    echo '{"Contents": [{"Key": "logs/app.log", "LastModified": "2024-01-02T00:00:00+00:00", "ETag": "\"abc\"", "Size": 11, "StorageClass": "STANDARD"}], "CommonPrefixes": [{"Prefix": "logs/2024/"}], "IsTruncated": false}'; exit 0;;
  "s3api head-object")
    echo '{"ContentLength": 11, "ContentType": "text/plain", "LastModified": "2024-01-02T00:00:00+00:00", "ETag": "\"abc\"", "Metadata": {}}'; exit 0;;
  "s3 cp")
    printf 'hello world'; exit 0;;
  "sqs list-queues")
    echo '{"QueueUrls": ["https://sqs.eu-west-1.amazonaws.com/123456789012/orders"]}'; exit 0;;
  "ec2 describe-instances")
    echo "An error occurred (UnauthorizedOperation) when calling the DescribeInstances operation: You are not authorized to perform this operation." >&2; exit 254;;
  "athena list-work-groups")
    echo '{"WorkGroups": [{"Name": "primary", "State": "ENABLED"}]}'; exit 0;;
  "eks list-clusters")
    echo '{"clusters": ["prod-eu"]}'; exit 0;;
  "rds describe-db-instances")
    echo '{"DBInstances": [{"DBInstanceIdentifier": "orders-db", "DBInstanceClass": "db.r6g.large", "Engine": "postgres", "EngineVersion": "15.4", "DBInstanceStatus": "available", "AvailabilityZone": "eu-west-1a", "MultiAZ": true, "AllocatedStorage": 100, "Endpoint": {"Address": "orders-db.abc.eu-west-1.rds.amazonaws.com", "Port": 5432}, "InstanceCreateTime": "2024-01-10T08:00:00+00:00", "TagList": [{"Key": "env", "Value": "prod"}]}]}'; exit 0;;
  "cloudwatch get-metric-data")
    # keep a copy of the --metric-data-queries file:// document for assertions
    prev=""; for a in "$@"; do if [ "$prev" = "--metric-data-queries" ]; then cp "${a#file://}" "$dir/queries.json"; fi; prev="$a"; done
    echo '{"MetricDataResults": [{"Id": "messages_sent", "Label": "Sent", "Timestamps": ["2024-06-01T10:00:00+00:00"], "Values": [3.0], "StatusCode": "Complete"}], "Messages": []}'; exit 0;;
  "eks update-kubeconfig")
    # find --kubeconfig <path> and write a stub file there
    prev=""; for a in "$@"; do if [ "$prev" = "--kubeconfig" ]; then echo "apiVersion: v1" > "$a"; fi; prev="$a"; done
    echo "Added new context to $a"; exit 0;;
  *)
    echo "fake aws: unhandled: $*" >&2; exit 252;;
esac
"#;

/// One fake binary per test process (the `OTTO_AWS_BIN` override is
/// process-wide env, so all tests share it).
fn fake_aws_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("otto-aws-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("aws");
        std::fs::write(&bin, FAKE_AWS).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::env::set_var(otto_aws::install::BIN_ENV, &bin);
        dir
    })
}

fn calls_log() -> String {
    std::fs::read_to_string(fake_aws_dir().join("calls.log")).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

async fn mem_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await
        .expect("in-memory pool");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn seed_user(pool: &SqlitePool, name: &str, is_root: bool) -> User {
    let id = otto_core::new_id();
    let now_ts = Utc::now().format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at)
         VALUES (?, ?, 'hash', ?, ?, 0, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(name)
    .bind(is_root as i64)
    .bind(&now_ts)
    .execute(pool)
    .await
    .expect("seed user");
    User {
        id,
        username: name.to_string(),
        display_name: name.to_string(),
        is_root,
        disabled: false,
        created_at: Utc::now(),
    }
}

async fn grant(pool: &SqlitePool, user: &User, feature: &str, cap: &str) {
    sqlx::query("INSERT INTO user_feature_grants (user_id, feature, capability) VALUES (?, ?, ?)")
        .bind(&user.id)
        .bind(feature)
        .bind(cap)
        .execute(pool)
        .await
        .expect("seed grant");
}

async fn allow_account(ctx: &TestCtx, root: &User, user: &User, id: &Id, operation: &str) {
    use otto_core::access::{AccessActor, AccessRule, ResourceKind, RuleEffect, SubjectKind};
    let repo = otto_state::resource_access::ResourceAccessRepo::new(ctx.pool.clone());
    let mut policy = repo.get_policy(ResourceKind::AwsAccount, id).await.unwrap();
    policy.rules.push(AccessRule {
        id: otto_core::new_id(),
        subject_kind: SubjectKind::User,
        subject_id: user.id.clone(),
        effect: RuleEffect::Allow,
        operations: vec![operation.to_string()],
        children: None,
        credential_connection_id: None,
        grantable_operations: vec![],
    });
    repo.put_policy(
        &policy,
        policy.revision,
        &AccessActor {
            real_user_id: root.id.clone(),
            effective_user_id: None,
        },
    )
    .await
    .unwrap();
}

#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);
impl SecretStore for MemSecrets {
    fn put(&self, k: &str, v: &str) -> Result<()> {
        self.0.lock().unwrap().insert(k.into(), v.into());
        Ok(())
    }
    fn get(&self, k: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(k).cloned())
    }
    fn delete(&self, k: &str) -> Result<()> {
        self.0.lock().unwrap().remove(k);
        Ok(())
    }
}

struct NullSpawner;
impl Spawner for NullSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws: &'a Id,
        _user: &'a Id,
        _conn: &'a Connection,
        _spec: otto_pty::CommandSpec,
        _first: Option<String>,
        _title: Option<String>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async { Err(Error::Internal("not used in tests".into())) })
    }
}

#[derive(Clone)]
struct TestCtx {
    pool: SqlitePool,
    secrets: Arc<dyn SecretStore>,
    mem: Arc<MemSecrets>,
    events: tokio::sync::broadcast::Sender<Event>,
    data_dir: PathBuf,
    spawner: Arc<dyn Spawner>,
}

impl TestCtx {
    async fn new() -> Self {
        fake_aws_dir();
        let mem = Arc::new(MemSecrets::default());
        let (events, _rx) = tokio::sync::broadcast::channel(64);
        let data_dir = std::env::temp_dir().join(format!("otto-aws-data-{}", otto_core::new_id()));
        std::fs::create_dir_all(&data_dir).unwrap();
        Self {
            pool: mem_pool().await,
            secrets: mem.clone(),
            mem,
            events,
            data_dir,
            spawner: Arc::new(NullSpawner),
        }
    }
}

impl AwsCtx for TestCtx {
    fn pool(&self) -> SqlitePool {
        self.pool.clone()
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    fn events(&self) -> &tokio::sync::broadcast::Sender<Event> {
        &self.events
    }
    fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }
    fn spawner(&self) -> &Arc<dyn Spawner> {
        &self.spawner
    }
}

async fn call(
    ctx: &TestCtx,
    user: &User,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value, axum::http::HeaderMap) {
    let app = otto_aws::api_router::<TestCtx>()
        .layer(Extension(AuthUser(user.clone())))
        .with_state(ctx.clone());
    let mut req = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = app
        .oneshot(req.body(body).unwrap())
        .await
        .expect("router response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
        })
    };
    (status, json, headers)
}

fn profile_req(name: &str, profile: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "auth_mode": "profile", "profile": profile, "region": "eu-west-1", "environment": "prod", "color": "#f00" })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_sees_the_fake_binary() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let (st, v, _) = call(&ctx, &root, "GET", "/aws/status", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(v["installed"], true);
    assert_eq!(v["version"], "2.17.0");
    assert_eq!(v["install"]["state"], "idle");
    assert_eq!(v["install"]["tool"], "aws");

    let (st, v, _) = call(&ctx, &root, "GET", "/aws/regions", None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["regions"].as_array().unwrap().len() >= 30);
}

#[tokio::test]
async fn profile_account_crud_and_test() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;

    let (st, a, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("prod", "prod-sso")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{a}");
    let id = a["id"].as_str().unwrap().to_string();
    assert_eq!(a["auth_mode"], "profile");
    assert_eq!(a["profile"], "prod-sso");
    assert_eq!(a["environment"], "prod");
    assert_eq!(a["color"], "#f00");
    // Best-effort identity probe ran on create.
    assert_eq!(a["identity"]["account"], "123456789012");
    assert_eq!(a["created_by"], root.id);

    let (st, list, _) = call(&ctx, &root, "GET", "/aws/accounts", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (st, upd, _) = call(
        &ctx,
        &root,
        "PATCH",
        &format!("/aws/accounts/{id}"),
        Some(serde_json::json!({ "name": "prod2", "region": "us-west-2" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{upd}");
    assert_eq!(upd["name"], "prod2");
    assert_eq!(upd["region"], "us-west-2");
    assert_eq!(upd["profile"], "prod-sso", "profile kept on partial patch");

    let (st, t, _) = call(
        &ctx,
        &root,
        "POST",
        &format!("/aws/accounts/{id}/test"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(t["ok"], true);
    assert_eq!(t["login_required"], false);
    assert_eq!(
        t["identity"]["arn"],
        "arn:aws:sts::123456789012:assumed-role/Dev/otto"
    );
    // The CLI saw the profile + (patched) region, and `--output json`.
    let log = calls_log();
    assert!(
        log.lines()
            .any(|l| l.contains("PROFILE=prod-sso REGION=us-west-2")
                && l.contains("ARGS=sts get-caller-identity --output json")),
        "{log}"
    );

    let (st, _, _) = call(&ctx, &root, "DELETE", &format!("/aws/accounts/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _, _) = call(&ctx, &root, "GET", &format!("/aws/accounts/{id}"), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn access_keys_account_keeps_secret_in_keychain_and_injects_env() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let body = serde_json::json!({
        "name": "keys", "auth_mode": "access_keys", "region": "us-east-1",
        "access_key_id": "AKIAIOSFODNN7EXAMPLE",
        "secret_access_key": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "session_token": "FQoGZXIvYXdzTOKEN"
    });
    let (st, a, _) = call(&ctx, &root, "POST", "/aws/accounts", Some(body)).await;
    assert_eq!(st, StatusCode::CREATED, "{a}");
    let id = a["id"].as_str().unwrap().to_string();
    assert_eq!(a["access_key_id"], "AKIAIOSFODNN7EXAMPLE");
    let text = a.to_string();
    assert!(
        !text.contains("wJalrXUtnFEMI"),
        "secret leaked into DTO: {text}"
    );
    assert!(!text.contains("FQoGZXIvYXdzTOKEN"));
    assert!(a.get("secret_ref").is_none());

    // Keychain holds the JSON payload under aws-<id>; the DB row holds only the ref.
    let stored = ctx
        .mem
        .get(&format!("aws-{id}"))
        .unwrap()
        .expect("secret stored");
    let sv: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(
        sv["secret_access_key"],
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
    );
    assert_eq!(sv["session_token"], "FQoGZXIvYXdzTOKEN");
    let row: (Option<String>, String) =
        sqlx::query_as("SELECT secret_ref, params_json FROM aws_accounts WHERE id = ?")
            .bind(&id)
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(row.0.as_deref(), Some(format!("aws-{id}").as_str()));
    assert!(!row.1.contains("wJalrXUtnFEMI"));

    // Env injection: keys, token, region — and AWS_PROFILE UNSET (never `""`:
    // CLI v2 reads an empty AWS_PROFILE as a profile named "" and fails every
    // call with `The config profile () could not be found`).
    let log = calls_log();
    assert!(
        log.lines().any(|l| l.contains("PROFILE=<unset> REGION=us-east-1 AKID=AKIAIOSFODNN7EXAMPLE SECRET=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY TOKEN=FQoGZXIvYXdzTOKEN")),
        "{log}"
    );

    // PATCH without secret fields keeps the stored secret; clearing the token works.
    let (st, _, _) = call(
        &ctx,
        &root,
        "PATCH",
        &format!("/aws/accounts/{id}"),
        Some(serde_json::json!({ "session_token": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let stored = ctx.mem.get(&format!("aws-{id}")).unwrap().unwrap();
    let sv: serde_json::Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(
        sv["secret_access_key"],
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
    );
    assert!(sv.get("session_token").is_none());

    // Delete removes the Keychain entry.
    let (st, _, _) = call(&ctx, &root, "DELETE", &format!("/aws/accounts/{id}"), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(ctx.mem.get(&format!("aws-{id}")).unwrap().is_none());
}

#[tokio::test]
async fn test_reports_login_required_on_expired_sso_and_login_needs_profile_mode() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let (st, a, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("exp", "expired")),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CREATED,
        "account saves even when the probe fails: {a}"
    );
    assert!(a.get("identity").is_none());
    let id = a["id"].as_str().unwrap();

    let (st, t, _) = call(
        &ctx,
        &root,
        "POST",
        &format!("/aws/accounts/{id}/test"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(t["ok"], false);
    assert_eq!(t["login_required"], true);
    assert!(
        t["message"]
            .as_str()
            .unwrap()
            .starts_with("login required:"),
        "{t}"
    );

    // Access-keys accounts have nothing to `sso login` into → 400.
    let (st, k, _) = call(&ctx, &root, "POST", "/aws/accounts", Some(serde_json::json!({
        "name": "k", "auth_mode": "access_keys", "access_key_id": "AKIAX", "secret_access_key": "s"
    }))).await;
    assert_eq!(st, StatusCode::CREATED);
    let kid = k["id"].as_str().unwrap();
    let (st, e, _) = call(
        &ctx,
        &root,
        "POST",
        &format!("/aws/accounts/{kid}/login"),
        Some(serde_json::json!({ "workspace_id": "ws" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "{e}");
}

#[tokio::test]
async fn validation_errors_are_400() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let cases = [
        serde_json::json!({ "name": "x", "auth_mode": "profile" }),
        serde_json::json!({ "name": "x", "auth_mode": "access_keys", "access_key_id": "AKIAX" }),
        serde_json::json!({ "auth_mode": "profile", "profile": "p" }),
        serde_json::json!({ "name": "x", "profile": "p" }),
    ];
    for c in cases {
        let (st, e, _) = call(&ctx, &root, "POST", "/aws/accounts", Some(c.clone())).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{c} → {e}");
        assert_eq!(e["code"], "invalid");
    }
    let (st, _, _) = call(&ctx, &root, "GET", "/aws/accounts/nope", None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn permissions_probe_classifies_and_caches() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let (_, a, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("perm", "perm-profile")),
    )
    .await;
    let id = a["id"].as_str().unwrap();

    let (st, p, _) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/permissions"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{p}");
    assert_eq!(p["services"]["s3"], "allowed");
    assert_eq!(p["services"]["sqs"], "allowed");
    assert_eq!(p["services"]["ec2"], "denied");
    assert_eq!(p["services"]["athena"], "allowed");
    assert_eq!(p["services"]["eks"], "allowed");
    assert_eq!(p["services"]["rds"], "allowed");
    assert_eq!(p["login_required"], false);
    assert_eq!(p["identity"]["account"], "123456789012");
    let probes_before = calls_log()
        .lines()
        .filter(|l| l.contains("PROFILE=perm-profile") && l.contains("ARGS=s3api list-buckets"))
        .count();
    assert_eq!(probes_before, 1);

    // Second call within the TTL is served from permissions_json.
    let (st, p2, _) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/permissions"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(p2["checked_at"], p["checked_at"]);
    let probes_after = calls_log()
        .lines()
        .filter(|l| l.contains("PROFILE=perm-profile") && l.contains("ARGS=s3api list-buckets"))
        .count();
    assert_eq!(probes_after, 1, "cached");

    // ?refresh=true re-probes.
    let (st, _, _) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/permissions?refresh=true"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let probes_refresh = calls_log()
        .lines()
        .filter(|l| l.contains("PROFILE=perm-profile") && l.contains("ARGS=s3api list-buckets"))
        .count();
    assert_eq!(probes_refresh, 2);

    // The account DTO now carries the cached permissions.
    let (_, a2, _) = call(&ctx, &root, "GET", &format!("/aws/accounts/{id}"), None).await;
    assert_eq!(a2["permissions"]["services"]["ec2"], "denied");
}

#[tokio::test]
async fn s3_list_and_streamed_download() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let (_, a, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("s3", "s3-profile")),
    )
    .await;
    let id = a["id"].as_str().unwrap();

    let (st, b, _) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/s3/buckets"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["buckets"][0]["name"], "logs-prod");

    let (st, o, _) = call(
        &ctx,
        &root,
        "GET",
        &format!(
            "/aws/accounts/{id}/s3/buckets/logs-prod/objects?prefix=logs/&region=ap-southeast-2"
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{o}");
    assert_eq!(o["prefixes"][0], "logs/2024/");
    assert_eq!(o["objects"][0]["key"], "logs/app.log");
    assert_eq!(o["is_truncated"], false);
    // Region override reached the CLI env.
    assert!(calls_log()
        .lines()
        .any(|l| l.contains("PROFILE=s3-profile REGION=ap-southeast-2")
            && l.contains("list-objects-v2")));

    let (st, body, h) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/s3/buckets/logs-prod/download?key=logs/app.log"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body, serde_json::Value::String("hello world".into()));
    assert_eq!(
        h.get("content-disposition").unwrap(),
        "attachment; filename=\"app.log\""
    );
    assert_eq!(h.get("content-type").unwrap(), "text/plain");
    assert_eq!(h.get("content-length").unwrap(), "11");

    // Bad bucket names are rejected before any CLI call.
    let (st, _, _) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/s3/buckets/Bad_Bucket/objects"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn import_kubeconfig_requires_root_and_creates_cluster_row() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let (_, a, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("eks", "eks-profile")),
    )
    .await;
    let id = a["id"].as_str().unwrap().to_string();

    // A user with aws_eks:Edit but no kubernetes grant is refused in-handler.
    let editor = seed_user(&ctx.pool, "editor", false).await;
    grant(&ctx.pool, &editor, "aws", "view").await;
    grant(&ctx.pool, &editor, "aws_eks", "edit").await;
    allow_account(&ctx, &root, &editor, &id, "discover").await;
    allow_account(&ctx, &root, &editor, &id, "eks_import").await;
    let uri = format!("/aws/accounts/{id}/eks/clusters/prod-eu/import-kubeconfig");
    let (st, e, _) = call(&ctx, &editor, "POST", &uri, Some(serde_json::json!({}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{e}");

    // Even delegated page Admin cannot attach a native alias. Root can import
    // an EKS cluster, with a private kubeconfig linked to this account.
    grant(&ctx.pool, &editor, "kubernetes", "admin").await;
    let (st, _, _) = call(&ctx, &editor, "POST", &uri, Some(serde_json::json!({}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, c, _) = call(&ctx, &root, "POST", &uri, Some(serde_json::json!({ "cluster_name_override": "prod-eu-otto", "default_namespace": "apps" }))).await;
    assert_eq!(st, StatusCode::CREATED, "{c}");
    assert_eq!(c["source"], "eks");
    assert_eq!(c["name"], "prod-eu-otto");
    assert_eq!(c["context_name"], "prod-eu-otto");
    assert_eq!(c["default_namespace"], "apps");
    assert_eq!(c["aws_account_id"], id);
    assert_eq!(c["environment"], "prod");
    let path = PathBuf::from(c["kubeconfig_path"].as_str().unwrap());
    assert!(path.starts_with(ctx.data_dir.join("kube")));
    assert!(path.is_file());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let row: (String, String, Option<String>, String) = sqlx::query_as(
        "SELECT source, context_name, aws_account_id, params_json FROM k8s_clusters WHERE id = ?",
    )
    .bind(c["id"].as_str().unwrap())
    .fetch_one(&ctx.pool)
    .await
    .unwrap();
    assert_eq!(row.0, "eks");
    assert_eq!(row.1, "prod-eu-otto");
    assert_eq!(row.2.as_deref(), Some(id.as_str()));
    assert!(row.3.contains("\"eks_cluster\":\"prod-eu\""));
    assert!(calls_log().lines().any(|l| l
        .contains("ARGS=eks update-kubeconfig --name prod-eu --kubeconfig")
        && l.contains("--alias prod-eu-otto")));

    // Audit row written.
    let n: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM audit_log WHERE action = 'aws.eks.import_kubeconfig'")
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
    assert_eq!(n.0, 1);
}

#[tokio::test]
async fn rds_list_and_describe_are_read_only() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let (_, a, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("rds", "rds-profile")),
    )
    .await;
    let id = a["id"].as_str().unwrap();

    let (st, b, _) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/rds/instances?region=us-east-1"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{b}");
    assert_eq!(b["instances"][0]["identifier"], "orders-db");
    assert_eq!(b["instances"][0]["engine"], "postgres");
    assert_eq!(b["instances"][0]["port"], 5432);
    assert_eq!(b["instances"][0]["multi_az"], true);
    assert_eq!(b["instances"][0]["tags"]["env"], "prod");
    assert!(calls_log()
        .lines()
        .any(|l| l.contains("PROFILE=rds-profile REGION=us-east-1")
            && l.contains("ARGS=rds describe-db-instances")));

    let (st, d, _) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/rds/instances/orders-db"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{d}");
    assert_eq!(d["identifier"], "orders-db");
    assert_eq!(d["raw"]["DBInstanceClass"], "db.r6g.large");

    // Bad identifiers never reach the CLI.
    let (st, _, _) = call(
        &ctx,
        &root,
        "GET",
        &format!("/aws/accounts/{id}/rds/instances/bad_name"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cloudwatch_metrics_single_call_cached_and_service_gated() {
    otto_aws::metrics::cache_clear();
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let (_, a, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("cw", "cw-profile")),
    )
    .await;
    let id = a["id"].as_str().unwrap();

    let uri = format!(
        "/aws/accounts/{id}/metrics?namespace=AWS/SQS&dim_name=QueueName&dim_value=orders.fifo&range=6h"
    );
    let (st, m, _) = call(&ctx, &root, "GET", &uri, None).await;
    assert_eq!(st, StatusCode::OK, "{m}");
    assert_eq!(m["namespace"], "AWS/SQS");
    assert_eq!(m["dim_name"], "QueueName");
    assert_eq!(m["dim_value"], "orders.fifo");
    assert_eq!(m["range"], "6h");
    assert_eq!(m["period_seconds"], 300);
    let series = m["series"].as_array().unwrap();
    assert_eq!(series.len(), otto_aws::metrics::SQS_CATALOG.len());
    let sent = series.iter().find(|s| s["id"] == "messages_sent").unwrap();
    assert_eq!(sent["metric"], "NumberOfMessagesSent");
    assert_eq!(sent["unit"], "count");
    // The fake returns one (off-window) point of 3.0: kept, so current = 3.
    assert_eq!(sent["current"], 3.0);
    // A metric the fake never returned still comes back, empty.
    let delayed = series
        .iter()
        .find(|s| s["id"] == "messages_delayed")
        .unwrap();
    assert!(delayed["current"].is_null());
    assert!(delayed["points"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["v"].is_null()));

    // Exactly one get-metric-data call, with the whole catalog in ONE
    // file:// document keyed on the queue name.
    let cw_calls = || {
        calls_log()
            .lines()
            .filter(|l| {
                l.contains("PROFILE=cw-profile") && l.contains("ARGS=cloudwatch get-metric-data")
            })
            .count()
    };
    assert_eq!(cw_calls(), 1);
    let line = calls_log()
        .lines()
        .find(|l| l.contains("ARGS=cloudwatch get-metric-data"))
        .unwrap()
        .to_string();
    assert!(line.contains("--metric-data-queries file://"), "{line}");
    assert!(
        line.contains("--start-time ") && line.contains("--end-time "),
        "{line}"
    );
    assert!(line.contains("--scan-by TimestampAscending"), "{line}");
    let queries: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fake_aws_dir().join("queries.json")).unwrap(),
    )
    .unwrap();
    let arr = queries.as_array().unwrap();
    assert_eq!(arr.len(), otto_aws::metrics::SQS_CATALOG.len());
    assert_eq!(arr[0]["MetricStat"]["Metric"]["Namespace"], "AWS/SQS");
    assert_eq!(
        arr[0]["MetricStat"]["Metric"]["Dimensions"][0]["Name"],
        "QueueName"
    );
    assert_eq!(
        arr[0]["MetricStat"]["Metric"]["Dimensions"][0]["Value"],
        "orders.fifo"
    );
    assert_eq!(arr[0]["MetricStat"]["Period"], 300);
    // The scratch document was removed after the call.
    let tmp = ctx.data_dir.join("tmp");
    let leftovers = std::fs::read_dir(&tmp)
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "tmp queries file must be cleaned up");

    // Second identical request within 30 s is served from the cache.
    let (st, m2, _) = call(&ctx, &root, "GET", &uri, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(m2["start"], m["start"]);
    assert_eq!(cw_calls(), 1, "cached");

    // Validation happens before any CLI call.
    for bad in [
        format!("/aws/accounts/{id}/metrics?namespace=AWS/Lambda&dim_value=fn"),
        format!("/aws/accounts/{id}/metrics?namespace=AWS/SQS&dim_value=orders&range=2h"),
        format!(
            "/aws/accounts/{id}/metrics?namespace=AWS/SQS&dim_name=InstanceId&dim_value=orders"
        ),
        format!("/aws/accounts/{id}/metrics?namespace=AWS/EC2&dim_value=i-1%20--debug"),
    ] {
        let (st, e, _) = call(&ctx, &root, "GET", &bad, None).await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{bad}: {e}");
    }
    assert_eq!(cw_calls(), 1);

    // A viewer with `aws:view` but no `aws_ec2` grant is refused for EC2
    // metrics in-handler; the same user with `aws_sqs:view` gets SQS metrics.
    let viewer = seed_user(&ctx.pool, "viewer", false).await;
    grant(&ctx.pool, &viewer, "aws", "view").await;
    grant(&ctx.pool, &viewer, "aws_sqs", "view").await;
    allow_account(&ctx, &root, &viewer, &id.to_string(), "discover").await;
    allow_account(&ctx, &root, &viewer, &id.to_string(), "metrics").await;
    let (st, e, _) = call(
        &ctx,
        &viewer,
        "GET",
        &format!("/aws/accounts/{id}/metrics?namespace=AWS/EC2&dim_value=i-0abc123456789def0"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "{e}");
    assert!(
        e["message"].as_str().unwrap().contains("aws_ec2:View"),
        "{e}"
    );
    let (st, _, _) = call(&ctx, &viewer, "GET", &uri, None).await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn new_accounts_are_private_and_guessed_bucket_reads_are_denied() {
    let ctx = TestCtx::new().await;
    let owner = seed_user(&ctx.pool, "owner", true).await;
    let outsider = seed_user(&ctx.pool, "outsider", false).await;
    let (st, account, _) = call(
        &ctx,
        &owner,
        "POST",
        "/aws/accounts",
        Some(profile_req("private", "dev")),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let id = account["id"].as_str().unwrap();
    let (st, list, _) = call(&ctx, &outsider, "GET", "/aws/accounts", None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list, serde_json::json!([]), "ungranted account leaked");
    for suffix in [
        "",
        "/s3/buckets/logs-prod/preview?key=logs/app.log",
        "/s3/buckets/logs-prod/objects",
    ] {
        let (st, _, _) = call(
            &ctx,
            &outsider,
            "GET",
            &format!("/aws/accounts/{id}{suffix}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{suffix}");
    }
}

#[tokio::test]
async fn bucket_allow_and_read_deny_override_account_administrator() {
    use otto_core::access::{AccessActor, AccessRule, ResourceKind, RuleEffect, SubjectKind};
    let ctx = TestCtx::new().await;
    let owner = seed_user(&ctx.pool, "root", true).await;
    let user = seed_user(&ctx.pool, "limited-admin", false).await;
    grant(&ctx.pool, &user, "aws", "admin").await;
    let (_, account, _) = call(
        &ctx,
        &owner,
        "POST",
        "/aws/accounts",
        Some(profile_req("private", "dev")),
    )
    .await;
    let id = account["id"].as_str().unwrap().to_string();
    let repo = otto_state::resource_access::ResourceAccessRepo::new(ctx.pool.clone());
    let mut policy = repo
        .get_policy(ResourceKind::AwsAccount, &id)
        .await
        .unwrap();
    for (effect, operations, children) in [
        (RuleEffect::Allow, vec!["discover"], None),
        (
            RuleEffect::Allow,
            vec!["s3_list", "s3_read"],
            Some(vec!["bucket:logs-prod"]),
        ),
        (
            RuleEffect::Deny,
            vec!["s3_read"],
            Some(vec!["bucket:logs-prod"]),
        ),
    ] {
        policy.rules.push(AccessRule {
            id: otto_core::new_id(),
            subject_kind: SubjectKind::User,
            subject_id: user.id.clone(),
            effect,
            operations: operations.into_iter().map(str::to_string).collect(),
            children: children.map(|c| c.into_iter().map(str::to_string).collect()),
            credential_connection_id: None,
            grantable_operations: vec![],
        });
    }
    let actor = AccessActor {
        real_user_id: owner.id,
        effective_user_id: None,
    };
    repo.put_policy(&policy, policy.revision, &actor)
        .await
        .unwrap();
    let (st, buckets, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/aws/accounts/{id}/s3/buckets"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(buckets["buckets"][0]["name"], "logs-prod");
    assert_eq!(buckets["buckets"].as_array().unwrap().len(), 1);
    let (st, _, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/aws/accounts/{id}/s3/buckets/logs-prod/objects"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    for suffix in [
        "/s3/buckets/other/objects",
        "/s3/buckets/logs-prod/download?key=logs/app.log",
        "/s3/buckets/logs-prod/preview?key=logs/app.log",
        "/metrics?namespace=AWS/EC2&dim_name=InstanceId&dim_value=i-123",
    ] {
        let (st, _, _) = call(
            &ctx,
            &user,
            "GET",
            &format!("/aws/accounts/{id}{suffix}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{suffix}");
    }
}

#[tokio::test]
async fn delegated_aws_admin_cannot_attach_or_repoint_ambient_credentials() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let user = seed_user(&ctx.pool, "delegated", false).await;
    grant(&ctx.pool, &user, "aws", "admin").await;
    let (_, account, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("visible", "dev")),
    )
    .await;
    let id = account["id"].as_str().unwrap().to_string();
    allow_account(&ctx, &root, &user, &id, "discover").await;
    allow_account(&ctx, &root, &user, &id, "configure").await;
    for patch in [
        serde_json::json!({"profile":"hidden-admin"}),
        serde_json::json!({"role_arn":"arn:aws:iam::123456789012:role/Hidden"}),
        serde_json::json!({"endpoint_url":"https://hidden.example"}),
        serde_json::json!({"auth_mode":"access_keys", "access_key_id":"x", "secret_access_key":"y"}),
    ] {
        let (st, _, _) = call(
            &ctx,
            &user,
            "PATCH",
            &format!("/aws/accounts/{id}"),
            Some(patch),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }
    let (st, _, _) = call(
        &ctx,
        &user,
        "POST",
        "/aws/accounts",
        Some(profile_req("alias", "hidden-admin")),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, updated, _) = call(
        &ctx,
        &user,
        "PATCH",
        &format!("/aws/accounts/{id}"),
        Some(profile_req("renamed", "dev")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{updated}");
    assert_eq!(updated["name"], "renamed");
}

#[tokio::test]
async fn aws_parent_page_is_required_and_discovery_redacts_configuration() {
    let ctx = TestCtx::new().await;
    let root = seed_user(&ctx.pool, "root", true).await;
    let user = seed_user(&ctx.pool, "reader", false).await;
    grant(&ctx.pool, &user, "aws_s3", "view").await;
    let (_, account, _) = call(
        &ctx,
        &root,
        "POST",
        "/aws/accounts",
        Some(profile_req("visible", "dev")),
    )
    .await;
    let id = account["id"].as_str().unwrap().to_string();
    for operation in ["discover", "s3_list", "s3_read"] {
        allow_account(&ctx, &root, &user, &id, operation).await;
    }
    let (st, _, _) = call(
        &ctx,
        &user,
        "GET",
        &format!("/aws/accounts/{id}/s3/buckets/logs-prod/objects"),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "no parent page must hide the resource"
    );
    grant(&ctx.pool, &user, "aws", "view").await;
    let (st, account, _) = call(&ctx, &user, "GET", &format!("/aws/accounts/{id}"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(account["profile"].is_null());
    assert!(account["identity"].is_null());
    assert!(account["permissions"].is_null());
    let (_, list, _) = call(&ctx, &user, "GET", "/aws/accounts", None).await;
    assert!(list[0]["profile"].is_null());
    let (st, probe, _) = call(&ctx, &user, "POST", &format!("/aws/accounts/{id}/test"), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(probe["identity"].is_null());
    assert_eq!(probe["message"], "Connection succeeded");
}
