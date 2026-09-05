//! Connection profile service: CRUD over `ConnectionsRepo` + Keychain
//! secrets, open-as-session (via the injected `Spawner`) and test-connect.

use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::{TestConnectionResp, UpsertConnectionReq};
use otto_core::auth::BoxFuture;
use otto_core::domain::{Connection, ConnectionKind, ConnectionSection, Session};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_pty::CommandSpec;
use otto_state::{ConnectionSectionsRepo, ConnectionsRepo, NewConnection};
use tokio::io::AsyncWriteExt;

use crate::builders::{build_command, validate_params};

/// Test-connect timeout.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Strip `user:password@` userinfo from a single URL-like token.
fn strip_userinfo_token(tok: &str) -> String {
    if let Some(scheme_end) = tok.find("://") {
        let after = scheme_end + 3;
        let rest = &tok[after..];
        let auth_end = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..auth_end];
        if let Some(at) = authority.rfind('@') {
            return format!("{}{}{}", &tok[..after], &authority[at + 1..], &rest[auth_end..]);
        }
    }
    tok.to_string()
}

/// Best-effort redaction of credentials a DB client might echo into its stderr,
/// so `test()` can surface a detailed, useful error WITHOUT leaking the password:
/// scrubs `scheme://user:pass@host` userinfo and `--password <x>` / `-p <x>` /
/// `--password=<x>` argv. Everything else (the actual error text) is preserved.
fn redact_secrets(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut redact_next = false;
    for tok in s.split_whitespace() {
        if redact_next {
            out.push("<redacted>".into());
            redact_next = false;
            continue;
        }
        if tok == "--password" || tok == "-p" {
            out.push(tok.into());
            redact_next = true;
        } else if tok.starts_with("--password=") {
            out.push("--password=<redacted>".into());
        } else {
            out.push(strip_userinfo_token(tok));
        }
    }
    out.join(" ")
}

/// Resolve the SSH private-key path a connection authenticates with, if any.
/// Direct SSH / Custom (and CLI-built `jump` tunnels) store it at the top level
/// (`params["identity_file"]`); DB connections tunneled over SSH nest it under
/// `params["ssh"]["identity_file"]`. We check both and return the first
/// non-empty value so the perms check works regardless of connection shape.
fn ssh_key_path(params: &serde_json::Value) -> Option<String> {
    let direct = params.get("identity_file").and_then(|v| v.as_str());
    let nested = params
        .get("ssh")
        .and_then(|s| s.get("identity_file"))
        .and_then(|v| v.as_str());
    direct
        .or(nested)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The SSH key-permission warning for a connection's params, if its private key
/// file is group/other-readable. Reusable across the two `test` code paths so
/// the warning fires uniformly (the CLI subprocess path here AND the DB-driver
/// path the server routes DB-kind connections through). Independent of the
/// probe outcome.
pub(crate) fn key_perms_warning_for(params: &serde_json::Value) -> Option<String> {
    ssh_key_path(params)
        .as_deref()
        .and_then(crate::keyperms::check_key_permissions)
}

/// Optional hook that lets the server layer route a connection's test probe
/// through the DB Explorer's warm-tunnel path (which reuses a cached `ssh -L`
/// forward) instead of spawning a fresh `ssh -J` child each time.
///
/// Implemented at integration time on top of `otto_dbviewer::DbViewerService`
/// (kept as a trait here so `otto-connections` does not depend on
/// `otto-dbviewer`). `ConnectionsCtx::db_tester` defaults to `None`; the
/// server wires in a concrete implementation so DB-kind probes reuse the warm
/// tunnel pool.
pub trait DbTester: Send + Sync {
    /// Run a connectivity probe on the named connection, returning the same
    /// [`TestConnectionResp`] shape the `connections().test()` path returns
    /// (minus `warn_argv`, which is `false` for driver-backed probes — no
    /// CLI secret exposure).
    fn test_db_connection<'a>(
        &'a self,
        id: &'a Id,
        user_id: &'a Id,
    ) -> otto_core::auth::BoxFuture<'a, Result<TestConnectionResp>>;
}

/// Spawns a connection session. Implemented at integration time on top of
/// `otto_sessions::SessionManager` (kept as a trait so otto-connections does
/// not depend on otto-sessions).
///
/// Implementations should write `first_command + "\n"` to the PTY ~1500ms
/// after spawn, and default the session title to `conn.name` when `title`
/// is `None`.
pub trait Spawner: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn spawn_connection<'a>(
        &'a self,
        ws_id: &'a Id,
        user_id: &'a Id,
        conn: &'a Connection,
        spec: CommandSpec,
        first_command: Option<String>,
        title: Option<String>,
    ) -> BoxFuture<'a, Result<Session>>;

    /// Spawns an ad-hoc command as a `SessionKind::Connection` PTY session
    /// that is NOT backed by a saved connection row (`connection_id = None`).
    /// Used by the AWS / Kubernetes consoles for `aws sso login`,
    /// `kubectl exec`, `k9s`, … `provider` is a short tag shown in the session
    /// list (e.g. "aws", "k8s"). Default: unsupported (test doubles).
    fn spawn_command<'a>(
        &'a self,
        _ws_id: &'a Id,
        _user_id: &'a Id,
        _provider: &'a str,
        _spec: CommandSpec,
        _title: String,
        _meta: Option<serde_json::Value>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async { Err(Error::Invalid("ad-hoc command sessions are not supported here".into())) })
    }
}

fn secret_ref_for(id: &Id) -> String {
    format!("conn-{id}")
}

/// CRUD + open/test for connection profiles.
pub struct ConnectionsService {
    repo: ConnectionsRepo,
    sections: ConnectionSectionsRepo,
    secrets: Arc<dyn SecretStore>,
}

impl ConnectionsService {
    pub fn new(
        repo: ConnectionsRepo,
        sections: ConnectionSectionsRepo,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            repo,
            sections,
            secrets,
        }
    }

    // --- Sections -----------------------------------------------------------

    /// The single global section tree (all workspaces + scopes), ordered by
    /// position. Both the Connections page and the DB Explorer share it.
    pub async fn list_sections(&self) -> Result<Vec<ConnectionSection>> {
        self.sections.list_all().await
    }

    pub async fn get_section(&self, id: &Id) -> Result<ConnectionSection> {
        self.sections.get(id).await
    }

    pub async fn create_section(
        &self,
        ws: &Id,
        user_id: &Id,
        parent_id: Option<&str>,
        name: &str,
        scope: &str,
    ) -> Result<ConnectionSection> {
        // One global tree: a section may nest under any existing section. The
        // FK guarantees the parent exists.
        self.sections
            .create(ws, parent_id, name, scope, user_id)
            .await
    }

    pub async fn rename_section(&self, id: &Id, name: &str) -> Result<ConnectionSection> {
        self.sections.rename(id, name).await
    }

    /// Reparent a section (None = top-level) in the single global tree,
    /// rejecting cycles.
    pub async fn reparent_section(
        &self,
        id: &Id,
        parent_id: Option<&str>,
    ) -> Result<ConnectionSection> {
        // Validate the section exists before moving it.
        self.sections.get(id).await?;
        if let Some(pid) = parent_id {
            if pid == id.as_str() {
                return Err(Error::Invalid("a section cannot be its own parent".into()));
            }
            // Reject moving a section under one of its own descendants (one
            // global tree, so consider every section).
            let all = self.sections.list_all().await?;
            let mut cursor = Some(pid.to_string());
            while let Some(cur) = cursor {
                if cur == id.as_str() {
                    return Err(Error::Invalid(
                        "cannot move a section into its own descendant".into(),
                    ));
                }
                cursor = all
                    .iter()
                    .find(|s| s.id == cur)
                    .and_then(|s| s.parent_id.clone());
            }
        }
        self.sections.reparent(id, parent_id).await
    }

    pub async fn delete_section(&self, id: &Id) -> Result<()> {
        self.sections.delete(id).await
    }

    pub async fn reorder_sections(&self, ws: &Id, ids: &[Id]) -> Result<()> {
        self.sections.reorder(ws, ids).await
    }

    pub async fn get(&self, id: &Id) -> Result<Connection> {
        self.repo.get(id).await
    }

    pub async fn authorize(&self, id: &Id, user_id: &Id, operation: &str) -> Result<()> {
        let conn = self.repo.get(id).await?;
        crate::access::check(&self.repo.pool(), &conn, user_id, operation).await
    }

    pub async fn is_enforced(&self, id: &Id) -> Result<bool> {
        crate::access::enforced(&self.repo.pool(), id).await
    }

    pub async fn visible_connection(&self, conn: Connection, user_id: &Id) -> Result<Connection> {
        crate::access::check(&self.repo.pool(), &conn, user_id, "discover").await?;
        if crate::access::check(&self.repo.pool(), &conn, user_id, "configure").await.is_err() {
            return Ok(crate::access::redact(conn));
        }
        Ok(conn)
    }

    /// Connections visible to a workspace (its own + global).
    pub async fn list(&self, ws: &Id) -> Result<Vec<Connection>> {
        // An identity-free legacy adapter must never reveal governed profiles.
        let mut visible = Vec::new();
        for conn in self.repo.list_visible(ws).await? {
            if !self.is_enforced(&conn.id).await? { visible.push(conn); }
        }
        Ok(visible)
    }

    /// Like `list` but filtered to connections created by `user_id`.
    /// Used when `connections.owner_private = true`.
    pub async fn list_for(&self, ws: &Id, user_id: &Id) -> Result<Vec<Connection>> {
        let private = otto_state::SettingsRepo::new(self.repo.pool()).get("connections.owner_private").await?
            .and_then(|v| v.as_bool()).unwrap_or(false);
        let user = otto_state::UsersRepo::new(self.repo.pool()).get(user_id).await?;
        let mut visible = Vec::new();
        for conn in self.repo.list_visible(ws).await? {
            if !self.is_enforced(&conn.id).await? && private && !user.is_root && conn.created_by != *user_id { continue; }
            if let Ok(conn) = self.visible_connection(conn, user_id).await { visible.push(conn); }
        }
        Ok(visible)
    }

    /// Create a profile; `workspace_id = None` makes it global (root-managed).
    pub async fn create(
        &self,
        workspace_id: Option<Id>,
        user_id: &Id,
        req: UpsertConnectionReq,
    ) -> Result<Connection> {
        let actor=otto_state::UsersRepo::new(self.repo.pool()).get(user_id).await?;
        if actor.disabled || !actor.is_root {return Err(Error::Forbidden("root must provision native connection identities and credentials before they can be delegated".into()));}
        validate_params(req.kind, &req.params, req.secret.is_some())?;
        let conn = self
            .repo
            .create(NewConnection {
                workspace_id,
                name: req.name,
                kind: req.kind,
                params: req.params,
                secret_ref: None,
                first_command: req.first_command,
                section_id: req.section_id.clone(),
                environment: req.environment.unwrap_or_default(),
                read_only: req.read_only.unwrap_or(false),
                created_by: user_id.clone(),
            })
            .await?;
        let user = otto_state::UsersRepo::new(self.repo.pool()).get(user_id).await?;
        let feature_grants = otto_state::GrantsRepo::new(self.repo.pool());
        let connections_cap = feature_grants.capability_of(&user, otto_core::domain::Feature::Connections).await?;
        let database_cap = feature_grants.capability_of(&user, otto_core::domain::Feature::Database).await?;
        use otto_core::domain::Capability;
        let mut operations: Vec<String> = ["discover", "configure", "manage_access"].into_iter().map(str::to_owned).collect();
        if connections_cap >= Capability::Edit {
            operations.extend(["shell", "sftp_read", "sftp_write"].into_iter().map(str::to_owned));
        }
        if database_cap >= Capability::View {
            operations.extend(["db_browse", "db_query", "db_export"].into_iter().map(str::to_owned));
        }
        if database_cap >= Capability::Edit {
            operations.extend(["db_data", "db_schema", "change_submit"].into_iter().map(str::to_owned));
        }
        if database_cap >= Capability::Admin {
            operations.extend(["change_approve", "change_execute"].into_iter().map(str::to_owned));
        }
        otto_state::resource_access::ResourceAccessRepo::new(self.repo.pool()).initialize_owner_policy(
            otto_core::access::ResourceKind::Connection, &conn.id, user_id, &operations, &operations,
            &otto_core::access::AccessActor { real_user_id: user_id.clone(), effective_user_id: None }
        ).await?;
        if let Some(secret) = req.secret {
            let secret_ref = secret_ref_for(&conn.id);
            self.secrets.put(&secret_ref, &secret)?;
            return self
                .repo
                .update(
                    &conn.id,
                    None,
                    None,
                    Some(Some(&secret_ref)),
                    None,
                    None,
                    None,
                    None,
                )
                .await;
        }
        Ok(conn)
    }

    /// Update a profile. Absent secret keeps the stored one; a provided
    /// secret replaces it. `kind` cannot change. `environment` / `read_only`
    /// are true PATCH semantics: absent (None) keeps the stored value, so a
    /// PATCH that omits them can't silently downgrade a `Prod`/read-only
    /// connection and disable the write-guard.
    pub async fn update(&self, id: &Id, user_id: &Id, req: UpsertConnectionReq) -> Result<Connection> {
        self.authorize(id, user_id, "configure").await?;
        let existing = self.repo.get(id).await?;
        if req.kind != existing.kind {
            return Err(Error::Invalid(
                "connection kind cannot be changed — create a new connection".into(),
            ));
        }
        let actor=otto_state::UsersRepo::new(self.repo.pool()).get(user_id).await?;
        if self.is_enforced(id).await? && !actor.is_root {
            // Full-form clients preserve a stored secret by omitting it.
            // Never turn a limited configuration update into a secret oracle.
            let replacing_secret=req.secret.is_some();
            if req.params!=existing.params || replacing_secret || req.first_command!=existing.first_command
                || req.environment.is_some_and(|e|e!=existing.environment) || req.read_only.is_some_and(|v|v!=existing.read_only) {
                return Err(Error::Forbidden("root must change connection identity, credentials, commands or environment protections".into()));
            }
        }
        let will_have_secret = req.secret.is_some() || existing.secret_ref.is_some();
        validate_params(req.kind, &req.params, will_have_secret)?;

        let mut new_secret_ref: Option<Option<String>> = None;
        if let Some(secret) = &req.secret {
            let secret_ref = existing
                .secret_ref
                .clone()
                .unwrap_or_else(|| secret_ref_for(id));
            self.secrets.put(&secret_ref, secret)?;
            new_secret_ref = Some(Some(secret_ref));
        }

        self.repo
            .update(
                id,
                Some(&req.name),
                Some(&req.params),
                new_secret_ref.as_ref().map(|opt| opt.as_deref()),
                Some(req.first_command.as_deref()),
                Some(req.section_id.as_deref()),
                req.environment,
                req.read_only,
            )
            .await
    }

    /// Delete the profile and its Keychain secret.
    pub async fn delete(&self, id: &Id, user_id: &Id) -> Result<()> {
        self.authorize(id, user_id, "configure").await?;
        let conn = self.repo.get(id).await?;
        if let Some(secret_ref) = &conn.secret_ref {
            if let Err(e) = self.secrets.delete(secret_ref) {
                tracing::warn!(connection = %id, "failed to delete secret: {e}");
            }
        }
        self.repo.delete(id).await
    }

    /// Open a connection as a terminal session in `ws_id` via the spawner.
    /// Stamps `last_opened_at` on the profile for recency ordering.
    pub async fn open(
        &self,
        conn: &Connection,
        ws_id: &Id,
        user_id: &Id,
        title: Option<String>,
        spawner: &dyn Spawner,
    ) -> Result<Session> {
        self.authorize(&conn.id, user_id, "shell").await?;
        if self.is_enforced(&conn.id).await? && conn.kind != ConnectionKind::Ssh {
            return Err(Error::Forbidden("governed database/custom profiles cannot open unrestricted terminal sessions".into()));
        }
        let secret = self.fetch_secret(conn)?;
        let (spec, _warn_argv) = build_command(conn, secret.as_deref())?;
        let session = spawner
            .spawn_connection(
                ws_id,
                user_id,
                conn,
                spec,
                conn.first_command.clone(),
                title,
            )
            .await?;
        // Best-effort recency stamp — ignored if the column doesn't exist yet.
        self.repo.stamp_opened(&conn.id).await;
        Ok(session)
    }

    /// Toggle the pinned status for a connection.
    pub async fn set_pinned(&self, id: &Id, user_id: &Id, pinned: bool) -> Result<Connection> {
        self.authorize(id, user_id, "discover").await?;
        self.visible_connection(self.repo.set_pinned(id, pinned).await?, user_id).await
    }

    /// Headless test-connect: run the command with a kind-specific probe,
    /// 10s timeout, report ok/latency/first stderr line.
    pub async fn test(&self, conn: &Connection, user_id: &Id) -> Result<TestConnectionResp> {
        self.authorize(&conn.id, user_id, "configure").await?;
        let secret = self.fetch_secret(conn)?;
        // NOTE: `warn_key_perms` is filled by the `test_connection` HTTP handler
        // (the single spot that covers both this CLI path and the cached-tunnel
        // DB-driver path uniformly), so it stays `None` on every return here.
        let warn_key_perms = None;
        let (spec, warn_argv) = build_command(conn, secret.as_deref())?;
        let (spec, probe) = probe_spec(conn.kind, spec);

        let started = Instant::now();
        let mut cmd = tokio::process::Command::new(&spec.program);
        cmd.args(&spec.args)
            .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ok(TestConnectionResp {
                    ok: false,
                    latency_ms: None,
                    message: format!("failed to start {}: {e}", spec.program),
                    warn_argv,
                    warn_key_perms,
                });
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if let Some(probe) = probe {
                let _ = stdin.write_all(probe).await;
            }
            drop(stdin); // EOF so the client exits after the probe.
        }

        match tokio::time::timeout(TEST_TIMEOUT, child.wait_with_output()).await {
            Err(_) => Ok(TestConnectionResp {
                ok: false,
                latency_ms: Some(TEST_TIMEOUT.as_millis() as u64),
                message: "timed out after 10s".into(),
                warn_argv,
                warn_key_perms,
            }),
            Ok(Err(e)) => Ok(TestConnectionResp {
                ok: false,
                latency_ms: None,
                message: format!("process error: {e}"),
                warn_argv,
                warn_key_perms,
            }),
            Ok(Ok(output)) => {
                let latency_ms = started.elapsed().as_millis() as u64;
                if output.status.success() {
                    Ok(TestConnectionResp {
                        ok: true,
                        latency_ms: Some(latency_ms),
                        message: "ok".into(),
                        warn_argv,
                        warn_key_perms,
                    })
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let first_line = stderr
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let message = if first_line.is_empty() {
                        format!("exited with {}", output.status)
                    } else {
                        // Keep the real driver error, but scrub any password a
                        // client might echo (mongo `user:pass@`, `--password x`).
                        redact_secrets(&first_line)
                    };
                    Ok(TestConnectionResp {
                        ok: false,
                        latency_ms: Some(latency_ms),
                        message,
                        warn_argv,
                        warn_key_perms,
                    })
                }
            }
        }
    }

    fn fetch_secret(&self, conn: &Connection) -> Result<Option<String>> {
        match &conn.secret_ref {
            Some(secret_ref) => self.secrets.get(secret_ref),
            None => Ok(None),
        }
    }
}

/// Adapt the interactive command into a headless probe per kind.
/// Returns the (possibly modified) spec and optional stdin payload.
fn probe_spec(kind: ConnectionKind, mut spec: CommandSpec) -> (CommandSpec, Option<&'static [u8]>) {
    match kind {
        ConnectionKind::Ssh => {
            // ssh [opts] target  ->  ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new
            //                          -o ConnectTimeout=5 [opts] target exit
            // `accept-new` lets a valid first-time host succeed (and records its
            // key) instead of failing the probe with "Host key verification
            // failed" under BatchMode; a *changed* known key is still rejected.
            let target = spec.args.pop();
            let mut args = vec![
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                "-o".to_string(),
                "StrictHostKeyChecking=accept-new".to_string(),
                "-o".to_string(),
                "ConnectTimeout=5".to_string(),
            ];
            args.append(&mut spec.args);
            if let Some(target) = target {
                args.push(target);
            }
            args.push("exit".to_string());
            spec.args = args;
            (spec, None)
        }
        ConnectionKind::Mysql | ConnectionKind::Clickhouse | ConnectionKind::Postgres => {
            // psql / mysql / clickhouse-client all read the probe from stdin and exit.
            (spec, Some(b"SELECT 1;\n"))
        }
        ConnectionKind::Redis => (spec, Some(b"PING\n")),
        ConnectionKind::Mongodb => {
            spec.args.push("--quiet".into());
            spec.args.push("--eval".into());
            spec.args.push("db.runCommand({ping:1})".into());
            (spec, None)
        }
        ConnectionKind::Custom => (spec, None),
    }
}
