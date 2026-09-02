//! Account service: CRUD over `AwsAccountsRepo` + Keychain secrets, the env
//! builder every service module goes through, `test` (`sts
//! get-caller-identity`), the cached per-service permission probe, and
//! `login` (an `aws sso login` PTY session via `Spawner::spawn_command`).
//!
//! Secrets: access-keys accounts keep `{"secret_access_key", "session_token"}`
//! as JSON in the Keychain under `aws-<id>`; the DB row only holds the
//! (non-secret) access-key id in `params_json`. Credentials reach the CLI as
//! **environment variables** on each subprocess — Otto never writes `~/.aws`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use otto_core::domain::{Environment, Session};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_pty::CommandSpec;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::cli::{self, CliOutput, StderrClass, DEFAULT_TIMEOUT, PROBE_TIMEOUT};
use crate::install;
use crate::AwsCtx;
use otto_connections::Spawner;
use otto_state::{AwsAccountPatch, AwsAccountRow, AwsAccountsRepo, NewAwsAccount};

/// Permission-probe cache TTL (§1).
pub const PERMISSIONS_TTL: Duration = Duration::from_secs(10 * 60);
/// Assume-role temp creds are requested with the default 1 h lifetime; we
/// refresh a few minutes early.
const ASSUME_ROLE_TTL: Duration = Duration::from_secs(55 * 60);

// ---------------------------------------------------------------------------
// DTOs (contract §2.1)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    Profile,
    AccessKeys,
}

impl AuthMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "profile" => Some(Self::Profile),
            "access_keys" => Some(Self::AccessKeys),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::AccessKeys => "access_keys",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsIdentity {
    pub account: String,
    pub arn: String,
    pub user_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermState {
    Allowed,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsServicePerms {
    pub s3: PermState,
    pub sqs: PermState,
    pub ec2: PermState,
    pub athena: PermState,
    pub eks: PermState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsPermissions {
    pub checked_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<AwsIdentity>,
    pub services: AwsServicePerms,
    pub login_required: bool,
}

/// `AwsAccount` — never includes secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AwsAccount {
    pub id: Id,
    pub name: String,
    pub auth_mode: AuthMode,
    pub profile: Option<String>,
    pub region: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_arn: Option<String>,
    /// Custom service endpoint (`params_json.endpoint_url`) — LocalStack, VPC
    /// interface endpoints, S3-compatible stores. Injected as
    /// `AWS_ENDPOINT_URL` into every subprocess when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_url: Option<String>,
    pub environment: Environment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<AwsIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<AwsPermissions>,
    pub created_by: Option<Id>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl AwsAccount {
    pub fn from_row(r: &AwsAccountRow) -> Self {
        let str_param = |k: &str| {
            r.params
                .get(k)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            auth_mode: AuthMode::parse(&r.auth_mode).unwrap_or(AuthMode::Profile),
            profile: r.profile.clone(),
            region: r.region.clone(),
            access_key_id: str_param("access_key_id"),
            role_arn: str_param("role_arn"),
            endpoint_url: str_param("endpoint_url"),
            environment: r.environment,
            color: str_param("color"),
            identity: r
                .identity
                .as_ref()
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            permissions: r
                .permissions
                .as_ref()
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            created_by: r.created_by.clone(),
            created_at: r.created_at,
            updated_at: r.updated_at,
            last_used_at: r.last_used_at,
        }
    }
}

/// Body of `POST /aws/accounts` and (all-optional) `PATCH /aws/accounts/{id}`.
/// Secret fields omitted on PATCH ⇒ keep the stored secret.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpsertAwsAccountReq {
    pub name: Option<String>,
    pub auth_mode: Option<AuthMode>,
    pub profile: Option<String>,
    pub region: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub role_arn: Option<String>,
    /// Optional custom endpoint (`http(s)://…`; plain `http` only for loopback
    /// hosts). On PATCH an empty string clears it, omitted keeps it.
    pub endpoint_url: Option<String>,
    pub environment: Option<Environment>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AwsTestResp {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<AwsIdentity>,
    pub login_required: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginReq {
    pub workspace_id: Id,
}

/// Keychain payload for access-keys accounts.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct KeySecret {
    secret_access_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_token: Option<String>,
}

fn secret_ref_for(id: &Id) -> String {
    format!("aws-{id}")
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// Static credentials as the CLI wants them in the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCreds {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

/// Validate a caller-supplied custom endpoint: `http://` / `https://` only
/// (no `ws`/`grpc`/`file`), plain `http` only for loopback hosts (the same
/// rule as `otto_netguard::require_tls_or_loopback` — credentials would
/// otherwise travel unencrypted), no whitespace / control characters.
/// Returns the normalised URL (trimmed, trailing `/` dropped).
pub fn validate_endpoint_url(raw: &str) -> Result<String> {
    let s = raw.trim().trim_end_matches('/').to_string();
    if s.len() > 512 {
        return Err(Error::Invalid("endpoint_url is too long (max 512)".into()));
    }
    if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Invalid(
            "endpoint_url must not contain whitespace".into(),
        ));
    }
    let lower = s.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(Error::Invalid(
            "endpoint_url must be an http:// or https:// URL (e.g. http://localhost:4566)".into(),
        ));
    }
    otto_netguard::require_tls_or_loopback(&s)
        .map_err(|e| Error::Invalid(format!("endpoint_url: {e}")))?;
    Ok(s)
}

/// The env every `aws` subprocess gets (§1 "Auth injection"). `profile` mode
/// sets `AWS_PROFILE`; keys mode sets the three `AWS_*` credential vars.
/// Both set the region (+ `AWS_DEFAULT_REGION` for older code paths), disable
/// the pager and the v2 auto-prompt. A custom `endpoint_url` (LocalStack, VPC
/// endpoints, S3-compatible stores) becomes `AWS_ENDPOINT_URL` — honoured by
/// every CLI v2 ≥ 2.13 command, `s3 cp` included — plus
/// `AWS_EC2_METADATA_DISABLED=true` so the CLI never stalls on the IMDS
/// credential provider when pointed at a non-AWS host.
pub fn build_env(
    mode: AuthMode,
    profile: Option<&str>,
    region: &str,
    creds: Option<&StaticCreds>,
    endpoint_url: Option<&str>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("AWS_REGION".to_string(), region.to_string()),
        ("AWS_DEFAULT_REGION".to_string(), region.to_string()),
        ("AWS_PAGER".to_string(), String::new()),
        ("AWS_CLI_AUTO_PROMPT".to_string(), "off".to_string()),
    ];
    if let Some(u) = endpoint_url.map(str::trim).filter(|u| !u.is_empty()) {
        env.push(("AWS_ENDPOINT_URL".into(), u.to_string()));
        env.push(("AWS_EC2_METADATA_DISABLED".into(), "true".into()));
    }
    match mode {
        AuthMode::Profile => {
            if let Some(p) = profile {
                env.push(("AWS_PROFILE".into(), p.to_string()));
            }
        }
        AuthMode::AccessKeys => {
            if let Some(c) = creds {
                env.push(("AWS_ACCESS_KEY_ID".into(), c.access_key_id.clone()));
                env.push(("AWS_SECRET_ACCESS_KEY".into(), c.secret_access_key.clone()));
                if let Some(t) = &c.session_token {
                    env.push(("AWS_SESSION_TOKEN".into(), t.clone()));
                }
            }
            // A stray AWS_PROFILE in the daemon env must not override the keys —
            // but it must be REMOVED, not blanked: CLI v2 treats `AWS_PROFILE=""`
            // as a profile literally named "" and fails every call with
            // `The config profile () could not be found`. `cli::run_raw` and the
            // S3 download spawn strip it before applying this env.
        }
    }
    env
}

/// `sts get-caller-identity` JSON → `AwsIdentity`.
pub fn parse_identity(v: &serde_json::Value) -> Option<AwsIdentity> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    Some(AwsIdentity {
        account: s("Account")?,
        arn: s("Arn")?,
        user_id: s("UserId").unwrap_or_default(),
    })
}

/// Classify one probe outcome (§1 "Permission probe"). The second tuple
/// element flags a login-required stderr so the caller can raise
/// `login_required` on the whole result.
pub fn classify_probe(out: &Result<CliOutput>) -> (PermState, bool) {
    match out {
        Ok(o) if o.ok() => (PermState::Allowed, false),
        Ok(o) => match cli::classify_stderr(&o.stderr) {
            StderrClass::AccessDenied => (PermState::Denied, false),
            StderrClass::LoginRequired => (PermState::Unknown, true),
            StderrClass::Other => (PermState::Unknown, false),
        },
        Err(_) => (PermState::Unknown, false),
    }
}

/// Region resolution: `?region=` override (validated to a sane shape) or the
/// account's own.
pub fn resolve_region<'a>(account: &'a AwsAccountRow, over: Option<&'a str>) -> Result<&'a str> {
    match over.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => {
            if r.len() > 32 || !r.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(Error::Invalid(format!("invalid region '{r}'")));
            }
            Ok(r)
        }
        None => Ok(account.region.as_str()),
    }
}

// ---------------------------------------------------------------------------
// Assume-role cache (process-wide; keyed by account id)
// ---------------------------------------------------------------------------

struct AssumedCreds {
    creds: StaticCreds,
    fetched: Instant,
}

fn assume_cache() -> &'static Mutex<HashMap<Id, AssumedCreds>> {
    static CACHE: OnceLock<Mutex<HashMap<Id, AssumedCreds>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn assume_cache_get(id: &Id) -> Option<StaticCreds> {
    let map = assume_cache().lock().unwrap_or_else(|p| p.into_inner());
    map.get(id)
        .filter(|a| a.fetched.elapsed() < ASSUME_ROLE_TTL)
        .map(|a| a.creds.clone())
}

fn assume_cache_put(id: &Id, creds: StaticCreds) {
    let mut map = assume_cache().lock().unwrap_or_else(|p| p.into_inner());
    map.insert(
        id.clone(),
        AssumedCreds {
            creds,
            fetched: Instant::now(),
        },
    );
}

fn assume_cache_evict(id: &Id) {
    let mut map = assume_cache().lock().unwrap_or_else(|p| p.into_inner());
    map.remove(id);
}

/// `sts assume-role` JSON → temp creds.
pub fn parse_assumed(v: &serde_json::Value) -> Option<StaticCreds> {
    let c = v.get("Credentials")?;
    let s = |k: &str| c.get(k).and_then(|x| x.as_str()).map(str::to_string);
    Some(StaticCreds {
        access_key_id: s("AccessKeyId")?,
        secret_access_key: s("SecretAccessKey")?,
        session_token: s("SessionToken"),
    })
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

/// Cheap to construct per request (holds only handles).
#[derive(Clone)]
pub struct AwsService {
    pub repo: AwsAccountsRepo,
    secrets: Arc<dyn SecretStore>,
    events: broadcast::Sender<Event>,
    pub data_dir: PathBuf,
    spawner: Arc<dyn Spawner>,
}

impl AwsService {
    pub fn from_ctx<S: AwsCtx>(ctx: &S) -> Self {
        Self {
            repo: AwsAccountsRepo::new(ctx.pool()),
            secrets: ctx.secrets().clone(),
            events: ctx.events().clone(),
            data_dir: ctx.data_dir().to_path_buf(),
            spawner: ctx.spawner().clone(),
        }
    }

    /// The `aws` binary, or the contract's `not installed` error.
    pub fn bin(&self) -> Result<PathBuf> {
        install::locate(&self.data_dir).ok_or_else(|| Error::Invalid(cli::NOT_INSTALLED_MSG.into()))
    }

    fn emit(&self, account_id: &Id, deleted: bool) {
        let _ = self.events.send(Event::AwsAccountUpdated {
            account_id: account_id.clone(),
            deleted,
        });
    }

    // ----- CRUD -------------------------------------------------------------

    pub async fn list(&self) -> Result<Vec<AwsAccount>> {
        Ok(self
            .repo
            .list()
            .await?
            .iter()
            .map(AwsAccount::from_row)
            .collect())
    }

    pub async fn get_row(&self, id: &Id) -> Result<AwsAccountRow> {
        self.repo.get(id).await
    }

    pub async fn get(&self, id: &Id) -> Result<AwsAccount> {
        Ok(AwsAccount::from_row(&self.repo.get(id).await?))
    }

    pub async fn create(&self, creator: &Id, req: UpsertAwsAccountReq) -> Result<AwsAccount> {
        let name = req
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Invalid("name is required".into()))?
            .to_string();
        let mode = req.auth_mode.ok_or_else(|| {
            Error::Invalid("auth_mode is required ('profile' | 'access_keys')".into())
        })?;
        let region = req
            .region
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("us-east-1")
            .to_string();
        let mut params = serde_json::Map::new();
        let mut profile = None;
        let mut secret: Option<KeySecret> = None;
        match mode {
            AuthMode::Profile => {
                let p = req
                    .profile
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| Error::Invalid("profile mode needs 'profile'".into()))?;
                profile = Some(p.to_string());
            }
            AuthMode::AccessKeys => {
                let akid = req
                    .access_key_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        Error::Invalid("access_keys mode needs 'access_key_id'".into())
                    })?;
                let sk = req
                    .secret_access_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        Error::Invalid("access_keys mode needs 'secret_access_key'".into())
                    })?;
                params.insert("access_key_id".into(), akid.into());
                secret = Some(KeySecret {
                    secret_access_key: sk.to_string(),
                    session_token: req
                        .session_token
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                });
            }
        }
        if let Some(r) = req
            .role_arn
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            params.insert("role_arn".into(), r.into());
        }
        if let Some(u) = req
            .endpoint_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            params.insert("endpoint_url".into(), validate_endpoint_url(u)?.into());
        }
        if let Some(c) = req
            .color
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            params.insert("color".into(), c.into());
        }
        let row = self
            .repo
            .create(NewAwsAccount {
                name,
                auth_mode: mode.as_str().into(),
                profile,
                region,
                params: serde_json::Value::Object(params),
                secret_ref: None,
                environment: req.environment.unwrap_or_default(),
                created_by: Some(creator.clone()),
            })
            .await?;
        let row = if let Some(s) = secret {
            let sref = secret_ref_for(&row.id);
            self.secrets.put(&sref, &serde_json::to_string(&s)?)?;
            self.repo
                .update(
                    &row.id,
                    AwsAccountPatch {
                        secret_ref: Some(Some(sref)),
                        ..Default::default()
                    },
                )
                .await?
        } else {
            row
        };
        // Best-effort identity — a wrong key or an expired SSO login must not
        // block saving the account (the card shows "Sign in" instead).
        let _ = self.probe_identity(&row).await;
        self.emit(&row.id, false);
        self.get(&row.id).await
    }

    pub async fn update(&self, id: &Id, req: UpsertAwsAccountReq) -> Result<AwsAccount> {
        let cur = self.repo.get(id).await?;
        let mode = req
            .auth_mode
            .or_else(|| AuthMode::parse(&cur.auth_mode))
            .unwrap_or(AuthMode::Profile);
        let mut params = cur.params.as_object().cloned().unwrap_or_default();
        let mut patch = AwsAccountPatch {
            name: req
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            region: req
                .region
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            environment: req.environment,
            ..Default::default()
        };
        if req.auth_mode.is_some() {
            patch.auth_mode = Some(mode.as_str().into());
        }
        if let Some(c) = &req.color {
            if c.trim().is_empty() {
                params.remove("color");
            } else {
                params.insert("color".into(), c.trim().into());
            }
        }
        if let Some(r) = &req.role_arn {
            if r.trim().is_empty() {
                params.remove("role_arn");
            } else {
                params.insert("role_arn".into(), r.trim().into());
            }
            assume_cache_evict(id);
        }
        if let Some(u) = &req.endpoint_url {
            if u.trim().is_empty() {
                params.remove("endpoint_url");
            } else {
                params.insert("endpoint_url".into(), validate_endpoint_url(u)?.into());
            }
            // Assumed creds were minted against the old endpoint's STS.
            assume_cache_evict(id);
        }
        match mode {
            AuthMode::Profile => {
                let p = req
                    .profile
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| cur.profile.clone())
                    .ok_or_else(|| Error::Invalid("profile mode needs 'profile'".into()))?;
                patch.profile = Some(Some(p));
                if req.auth_mode.is_some() && cur.auth_mode != "profile" {
                    // Switching away from keys: drop the secret.
                    if let Some(sref) = &cur.secret_ref {
                        let _ = self.secrets.delete(sref);
                    }
                    patch.secret_ref = Some(None);
                    params.remove("access_key_id");
                }
            }
            AuthMode::AccessKeys => {
                if let Some(a) = req
                    .access_key_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    params.insert("access_key_id".into(), a.into());
                }
                if !params.contains_key("access_key_id") {
                    return Err(Error::Invalid(
                        "access_keys mode needs 'access_key_id'".into(),
                    ));
                }
                let sref = cur.secret_ref.clone().unwrap_or_else(|| secret_ref_for(id));
                let existing: Option<KeySecret> = self
                    .secrets
                    .get(&sref)?
                    .and_then(|s| serde_json::from_str(&s).ok());
                let new_sk = req
                    .secret_access_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if new_sk.is_none() && existing.is_none() {
                    return Err(Error::Invalid(
                        "access_keys mode needs 'secret_access_key'".into(),
                    ));
                }
                let merged = KeySecret {
                    secret_access_key: new_sk
                        .map(str::to_string)
                        .or_else(|| existing.as_ref().map(|e| e.secret_access_key.clone()))
                        .unwrap_or_default(),
                    session_token: match &req.session_token {
                        Some(t) if t.trim().is_empty() => None,
                        Some(t) => Some(t.trim().to_string()),
                        None => existing.and_then(|e| e.session_token),
                    },
                };
                self.secrets.put(&sref, &serde_json::to_string(&merged)?)?;
                patch.secret_ref = Some(Some(sref));
                if req.auth_mode.is_some() && cur.auth_mode != "access_keys" {
                    patch.profile = Some(None);
                }
                assume_cache_evict(id);
            }
        }
        patch.params = Some(serde_json::Value::Object(params));
        let row = self.repo.update(id, patch).await?;
        let _ = self.probe_identity(&row).await;
        self.emit(id, false);
        self.get(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        let cur = self.repo.get(id).await?;
        if let Some(sref) = &cur.secret_ref {
            let _ = self.secrets.delete(sref);
        }
        assume_cache_evict(id);
        self.repo.delete(id).await?;
        self.emit(id, true);
        Ok(())
    }

    // ----- Env / run --------------------------------------------------------

    /// Static creds for a keys-mode account (from the Keychain).
    fn static_creds(&self, account: &AwsAccountRow) -> Result<StaticCreds> {
        let akid = account
            .params
            .get("access_key_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Invalid("account has no access_key_id".into()))?
            .to_string();
        let sref = account
            .secret_ref
            .as_deref()
            .ok_or_else(|| Error::Invalid("account has no stored secret".into()))?;
        let raw = self
            .secrets
            .get(sref)?
            .ok_or_else(|| Error::Invalid("login required: the stored secret key is missing from the Keychain — re-enter the access keys".into()))?;
        let ks: KeySecret = serde_json::from_str(&raw)?;
        Ok(StaticCreds {
            access_key_id: akid,
            secret_access_key: ks.secret_access_key,
            session_token: ks.session_token,
        })
    }

    /// Build the subprocess env for `account` (§1), running `sts assume-role`
    /// (cached ~1 h) when `params.role_arn` is set.
    pub async fn env_for(
        &self,
        account: &AwsAccountRow,
        region: Option<&str>,
    ) -> Result<Vec<(String, String)>> {
        let region = resolve_region(account, region)?;
        let mode = AuthMode::parse(&account.auth_mode).unwrap_or(AuthMode::Profile);
        let base_creds = match mode {
            AuthMode::Profile => None,
            AuthMode::AccessKeys => Some(self.static_creds(account)?),
        };
        let endpoint = endpoint_url_of(account);
        let base = build_env(
            mode,
            account.profile.as_deref(),
            region,
            base_creds.as_ref(),
            endpoint,
        );
        let role_arn = account
            .params
            .get("role_arn")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let Some(role_arn) = role_arn else {
            return Ok(base);
        };
        // Profile-mode accounts whose profile already carries `role_arn` let the
        // CLI chain roles itself; an explicit params.role_arn on top is the
        // "assume this from the base creds" case, handled here.
        let creds = match assume_cache_get(&account.id) {
            Some(c) => c,
            None => {
                let bin = self.bin()?;
                let session_name = format!("otto-{}", &account.id[..account.id.len().min(24)]);
                let args: Vec<String> = [
                    "sts",
                    "assume-role",
                    "--role-arn",
                    role_arn,
                    "--role-session-name",
                    &session_name,
                    "--output",
                    "json",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect();
                let v = cli::run_json(&bin, &args, &base, DEFAULT_TIMEOUT).await?;
                let c = parse_assumed(&v).ok_or_else(|| {
                    Error::Upstream("sts assume-role returned no Credentials".into())
                })?;
                assume_cache_put(&account.id, c.clone());
                c
            }
        };
        Ok(build_env(
            AuthMode::AccessKeys,
            None,
            region,
            Some(&creds),
            endpoint,
        ))
    }

    /// Run `aws <args> --output json` for `account` and return the raw output
    /// (errors already mapped: non-zero exit ⇒ classified `Error`).
    pub async fn run(
        &self,
        account: &AwsAccountRow,
        region: Option<&str>,
        args: &[&str],
    ) -> Result<CliOutput> {
        self.run_with(account, region, args, DEFAULT_TIMEOUT, None)
            .await
    }

    pub async fn run_with(
        &self,
        account: &AwsAccountRow,
        region: Option<&str>,
        args: &[&str],
        timeout: Duration,
        stdin: Option<&[u8]>,
    ) -> Result<CliOutput> {
        let bin = self.bin()?;
        let env = self.env_for(account, region).await?;
        let argv = with_json_output(args);
        let out = cli::run(&bin, &argv, &env, timeout, stdin).await;
        if out.is_ok() {
            self.repo.touch_used(&account.id).await;
        }
        out
    }

    /// [`Self::run`] + JSON parse.
    pub async fn run_json(
        &self,
        account: &AwsAccountRow,
        region: Option<&str>,
        args: &[&str],
    ) -> Result<serde_json::Value> {
        let out = self.run(account, region, args).await?;
        cli::parse_stdout(&out.stdout)
    }

    /// Resolved binary + env for callers that spawn their own child (S3
    /// download stream, `sso login` PTY).
    pub async fn bin_and_env(
        &self,
        account: &AwsAccountRow,
        region: Option<&str>,
    ) -> Result<(PathBuf, Vec<(String, String)>)> {
        Ok((self.bin()?, self.env_for(account, region).await?))
    }

    // ----- test / permissions / login --------------------------------------

    /// `sts get-caller-identity`; caches the identity on success.
    async fn probe_identity(&self, account: &AwsAccountRow) -> Result<AwsIdentity> {
        let v = self
            .run_json(account, None, &["sts", "get-caller-identity"])
            .await?;
        let ident = parse_identity(&v).ok_or_else(|| {
            Error::Upstream("sts get-caller-identity returned no Account/Arn".into())
        })?;
        self.repo
            .set_identity(&account.id, &serde_json::to_value(&ident)?)
            .await?;
        Ok(ident)
    }

    pub async fn test(&self, id: &Id) -> Result<AwsTestResp> {
        let account = self.repo.get(id).await?;
        let started = Instant::now();
        match self.probe_identity(&account).await {
            Ok(identity) => Ok(AwsTestResp {
                ok: true,
                latency_ms: started.elapsed().as_millis() as u64,
                message: format!("{} · {}", identity.account, identity.arn),
                identity: Some(identity),
                login_required: false,
            }),
            // "not installed" is a setup problem, not a credential one — let it
            // surface as the 400 the first-run panel keys off.
            Err(Error::Invalid(m)) if m.contains("not installed") => Err(Error::Invalid(m)),
            Err(e) => {
                let message = match &e {
                    Error::Invalid(m)
                    | Error::Forbidden(m)
                    | Error::Upstream(m)
                    | Error::Internal(m) => m.clone(),
                    other => other.to_string(),
                };
                Ok(AwsTestResp {
                    ok: false,
                    latency_ms: started.elapsed().as_millis() as u64,
                    login_required: message.starts_with("login required:"),
                    message,
                    identity: None,
                })
            }
        }
    }

    /// Per-service permission probe, cached 10 min in `permissions_json`.
    pub async fn permissions(&self, id: &Id, refresh: bool) -> Result<AwsPermissions> {
        let account = self.repo.get(id).await?;
        if !refresh {
            if let (Some(v), Some(at)) = (&account.permissions, account.permissions_checked_at) {
                if Utc::now()
                    .signed_duration_since(at)
                    .to_std()
                    .unwrap_or(Duration::MAX)
                    < PERMISSIONS_TTL
                {
                    if let Ok(p) = serde_json::from_value::<AwsPermissions>(v.clone()) {
                        return Ok(p);
                    }
                }
            }
        }
        let bin = self.bin()?;
        let env = self.env_for(&account, None).await?;
        let probe = |args: &'static [&'static str]| {
            let bin = bin.clone();
            let env = env.clone();
            async move {
                let argv = with_json_output(args);
                cli::run_raw(&bin, &argv, &env, PROBE_TIMEOUT, None).await
            }
        };
        let (sts, s3, sqs, ec2, athena, eks) = tokio::join!(
            probe(&["sts", "get-caller-identity"]),
            probe(&["s3api", "list-buckets", "--max-items", "1"]),
            probe(&["sqs", "list-queues", "--max-results", "1"]),
            probe(&["ec2", "describe-instances", "--max-results", "5"]),
            probe(&["athena", "list-work-groups", "--max-results", "1"]),
            probe(&["eks", "list-clusters", "--max-results", "1"]),
        );
        let mut login_required = false;
        let mut cls = |r: &Result<CliOutput>| {
            let (state, lr) = classify_probe(r);
            login_required |= lr;
            state
        };
        let services = AwsServicePerms {
            s3: cls(&s3),
            sqs: cls(&sqs),
            ec2: cls(&ec2),
            athena: cls(&athena),
            eks: cls(&eks),
        };
        let (_, sts_lr) = classify_probe(&sts);
        login_required |= sts_lr;
        let identity = match &sts {
            Ok(o) if o.ok() => cli::parse_stdout(&o.stdout)
                .ok()
                .and_then(|v| parse_identity(&v)),
            _ => None,
        };
        if let Some(i) = &identity {
            let _ = self.repo.set_identity(id, &serde_json::to_value(i)?).await;
        }
        let perms = AwsPermissions {
            checked_at: Utc::now(),
            identity,
            services,
            login_required,
        };
        // Don't cache a login-required snapshot: the next call after
        // `sso login` should re-probe immediately.
        if !login_required {
            self.repo
                .set_permissions(id, &serde_json::to_value(&perms)?)
                .await?;
        }
        self.emit(id, false);
        Ok(perms)
    }

    /// Spawn `aws sso login --profile <p>` as a PTY session (profile mode only).
    pub async fn login(&self, id: &Id, ws_id: &Id, user_id: &Id) -> Result<Session> {
        let account = self.repo.get(id).await?;
        if AuthMode::parse(&account.auth_mode) != Some(AuthMode::Profile) {
            return Err(Error::Invalid(
                "login is only for profile accounts — access-keys accounts have nothing to sign in to; re-enter the keys instead".into(),
            ));
        }
        let profile = account
            .profile
            .clone()
            .ok_or_else(|| Error::Invalid("account has no profile".into()))?;
        let bin = self.bin()?;
        // No assume-role here: the login is for the base profile.
        let env = build_env(
            AuthMode::Profile,
            Some(&profile),
            &account.region,
            None,
            endpoint_url_of(&account),
        );
        let spec = CommandSpec {
            program: bin.to_string_lossy().into_owned(),
            args: vec![
                "sso".into(),
                "login".into(),
                "--profile".into(),
                profile.clone(),
            ],
            cwd: None,
            env,
        };
        let title = format!("aws sso login · {}", account.name);
        let meta = serde_json::json!({ "aws": { "account_id": id, "profile": profile } });
        self.spawner
            .spawn_command(ws_id, user_id, "aws", spec, title, Some(meta))
            .await
    }
}

/// `params_json.endpoint_url` of a row, if set and non-empty.
pub fn endpoint_url_of(account: &AwsAccountRow) -> Option<&str> {
    account
        .params
        .get("endpoint_url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Append `--output json` unless the caller already chose an output mode
/// (`s3 cp … -` streams bytes, `s3 ls` is text).
pub fn with_json_output(args: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let is_s3_hl = args.first() == Some(&"s3");
    if !is_s3_hl && !args.contains(&"--output") {
        v.push("--output".into());
        v.push("json".into());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_map(v: Vec<(String, String)>) -> HashMap<String, String> {
        v.into_iter().collect()
    }

    #[test]
    fn env_for_profile_mode() {
        let e = env_map(build_env(
            AuthMode::Profile,
            Some("dev-sso"),
            "eu-west-1",
            None,
            None,
        ));
        assert_eq!(e["AWS_PROFILE"], "dev-sso");
        assert_eq!(e["AWS_REGION"], "eu-west-1");
        assert_eq!(e["AWS_DEFAULT_REGION"], "eu-west-1");
        assert_eq!(e["AWS_PAGER"], "");
        assert_eq!(e["AWS_CLI_AUTO_PROMPT"], "off");
        assert!(!e.contains_key("AWS_ACCESS_KEY_ID"));
        assert!(!e.contains_key("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn env_for_access_keys_mode() {
        let creds = StaticCreds {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: Some("tok".into()),
        };
        let e = env_map(build_env(
            AuthMode::AccessKeys,
            None,
            "us-west-2",
            Some(&creds),
            None,
        ));
        assert_eq!(e["AWS_ACCESS_KEY_ID"], "AKIAIOSFODNN7EXAMPLE");
        assert_eq!(
            e["AWS_SECRET_ACCESS_KEY"],
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        );
        assert_eq!(e["AWS_SESSION_TOKEN"], "tok");
        assert_eq!(e["AWS_REGION"], "us-west-2");
        // A daemon-level AWS_PROFILE is stripped by the spawner, never blanked
        // (`AWS_PROFILE=""` breaks the CLI: "config profile () could not be found").
        assert!(!e.contains_key("AWS_PROFILE"));

        let no_tok = StaticCreds {
            session_token: None,
            ..creds
        };
        let e = env_map(build_env(
            AuthMode::AccessKeys,
            None,
            "us-west-2",
            Some(&no_tok),
            None,
        ));
        assert!(!e.contains_key("AWS_SESSION_TOKEN"));
        assert!(!e.contains_key("AWS_ENDPOINT_URL"));
        assert!(!e.contains_key("AWS_EC2_METADATA_DISABLED"));
    }

    #[test]
    fn endpoint_url_is_injected_in_both_auth_modes() {
        let e = env_map(build_env(
            AuthMode::Profile,
            Some("p"),
            "us-east-1",
            None,
            Some("http://localhost:4566"),
        ));
        assert_eq!(e["AWS_ENDPOINT_URL"], "http://localhost:4566");
        assert_eq!(e["AWS_EC2_METADATA_DISABLED"], "true");
        assert_eq!(e["AWS_PROFILE"], "p");

        let creds = StaticCreds {
            access_key_id: "test".into(),
            secret_access_key: "test".into(),
            session_token: None,
        };
        let e = env_map(build_env(
            AuthMode::AccessKeys,
            None,
            "us-east-1",
            Some(&creds),
            Some("http://127.0.0.1:4566"),
        ));
        assert_eq!(e["AWS_ENDPOINT_URL"], "http://127.0.0.1:4566");
        assert_eq!(e["AWS_EC2_METADATA_DISABLED"], "true");
        assert_eq!(e["AWS_ACCESS_KEY_ID"], "test");
        // Blank / whitespace endpoint ⇒ nothing injected.
        let e = env_map(build_env(AuthMode::Profile, Some("p"), "us-east-1", None, Some("  ")));
        assert!(!e.contains_key("AWS_ENDPOINT_URL"));
    }

    #[test]
    fn endpoint_url_validation() {
        assert_eq!(
            validate_endpoint_url(" http://localhost:4566/ ").unwrap(),
            "http://localhost:4566"
        );
        assert_eq!(
            validate_endpoint_url("http://127.0.0.1:4566").unwrap(),
            "http://127.0.0.1:4566"
        );
        assert_eq!(
            validate_endpoint_url("http://[::1]:4566").unwrap(),
            "http://[::1]:4566"
        );
        assert_eq!(
            validate_endpoint_url("https://vpce-0abc.s3.eu-west-1.vpce.amazonaws.com").unwrap(),
            "https://vpce-0abc.s3.eu-west-1.vpce.amazonaws.com"
        );
        // Plain http to a non-loopback host leaks credentials.
        assert!(matches!(
            validate_endpoint_url("http://minio.internal:9000"),
            Err(Error::Invalid(m)) if m.contains("https")
        ));
        assert!(validate_endpoint_url("http://10.0.0.5:4566").is_err());
        // Non-http(s) schemes and junk are rejected up front.
        assert!(validate_endpoint_url("ws://localhost:4566").is_err());
        assert!(validate_endpoint_url("file:///etc/passwd").is_err());
        assert!(validate_endpoint_url("localhost:4566").is_err());
        assert!(validate_endpoint_url("http://localhost:4566 --debug").is_err());
        assert!(validate_endpoint_url("").is_err());
    }

    #[test]
    fn endpoint_url_of_reads_params() {
        let mut row = AwsAccountRow {
            id: "01J".into(),
            name: "n".into(),
            auth_mode: "access_keys".into(),
            profile: None,
            region: "us-east-1".into(),
            params: serde_json::json!({"access_key_id": "test", "endpoint_url": "http://localhost:4566"}),
            secret_ref: None,
            identity: None,
            permissions: None,
            permissions_checked_at: None,
            environment: Environment::Dev,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_used_at: None,
        };
        assert_eq!(endpoint_url_of(&row), Some("http://localhost:4566"));
        assert_eq!(
            AwsAccount::from_row(&row).endpoint_url.as_deref(),
            Some("http://localhost:4566")
        );
        row.params = serde_json::json!({"access_key_id": "test", "endpoint_url": ""});
        assert_eq!(endpoint_url_of(&row), None);
        assert!(AwsAccount::from_row(&row).endpoint_url.is_none());
    }

    #[test]
    fn identity_and_assumed_parse_from_real_shapes() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"UserId": "AROAEXAMPLE:otto", "Account": "123456789012", "Arn": "arn:aws:sts::123456789012:assumed-role/Dev/otto"}"#,
        )
        .unwrap();
        let i = parse_identity(&v).unwrap();
        assert_eq!(i.account, "123456789012");
        assert!(i.arn.ends_with("/otto"));

        let v: serde_json::Value = serde_json::from_str(
            r#"{"Credentials": {"AccessKeyId": "ASIAEXAMPLE", "SecretAccessKey": "s", "SessionToken": "t", "Expiration": "2026-09-02T12:00:00+00:00"}, "AssumedRoleUser": {"AssumedRoleId": "x", "Arn": "y"}}"#,
        )
        .unwrap();
        let c = parse_assumed(&v).unwrap();
        assert_eq!(c.access_key_id, "ASIAEXAMPLE");
        assert_eq!(c.session_token.as_deref(), Some("t"));
        assert!(parse_assumed(&serde_json::json!({})).is_none());
    }

    #[test]
    fn probe_classification() {
        let out = |status: i32, stderr: &str| -> Result<CliOutput> {
            Ok(CliOutput {
                status,
                stdout: String::new(),
                stderr: stderr.into(),
                duration_ms: 0,
            })
        };
        assert_eq!(classify_probe(&out(0, "")), (PermState::Allowed, false));
        assert_eq!(
            classify_probe(&out(
                254,
                "An error occurred (AccessDenied) when calling the ListBuckets operation"
            )),
            (PermState::Denied, false)
        );
        assert_eq!(
            classify_probe(&out(
                255,
                "An error occurred (UnauthorizedOperation) when calling DescribeInstances"
            )),
            (PermState::Denied, false)
        );
        assert_eq!(
            classify_probe(&out(255, "Error loading SSO Token: expired")),
            (PermState::Unknown, true)
        );
        assert_eq!(
            classify_probe(&out(255, "Could not connect to the endpoint URL")),
            (PermState::Unknown, false)
        );
        assert_eq!(
            classify_probe(&Err(Error::Upstream("timed out".into()))),
            (PermState::Unknown, false)
        );
    }

    #[test]
    fn json_output_appended_except_for_s3_high_level() {
        assert_eq!(
            with_json_output(&["sqs", "list-queues"]),
            vec!["sqs", "list-queues", "--output", "json"]
        );
        assert_eq!(
            with_json_output(&["s3", "cp", "s3://b/k", "-"]),
            vec!["s3", "cp", "s3://b/k", "-"]
        );
        assert_eq!(
            with_json_output(&["ec2", "describe-instances", "--output", "json"]),
            vec!["ec2", "describe-instances", "--output", "json"]
        );
    }

    #[test]
    fn account_dto_never_carries_secrets() {
        let row = AwsAccountRow {
            id: "01J".into(),
            name: "n".into(),
            auth_mode: "access_keys".into(),
            profile: None,
            region: "us-east-1".into(),
            params: serde_json::json!({"access_key_id": "AKIAX", "color": "#0f0"}),
            secret_ref: Some("aws-01J".into()),
            identity: Some(serde_json::json!({"account": "1", "arn": "a", "user_id": "u"})),
            permissions: None,
            permissions_checked_at: None,
            environment: Environment::Dev,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_used_at: None,
        };
        let dto = AwsAccount::from_row(&row);
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["auth_mode"], "access_keys");
        assert_eq!(v["access_key_id"], "AKIAX");
        assert_eq!(v["color"], "#0f0");
        assert_eq!(v["identity"]["account"], "1");
        assert!(v.get("secret_ref").is_none());
        assert!(v.get("secret_access_key").is_none());
    }
}
