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
            return format!(
                "{}{}{}",
                &tok[..after],
                &authority[at + 1..],
                &rest[auth_end..]
            );
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

    /// A connection's credential was replaced or the connection deleted: drop
    /// any copy of `secret_ref` the DB Explorer cached (completion reuses a
    /// Keychain read briefly) so it can't keep connecting with the old secret.
    fn forget_secret(&self, _secret_ref: &str) {}
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
        Box::pin(async {
            Err(Error::Invalid(
                "ad-hoc command sessions are not supported here".into(),
            ))
        })
    }
}

fn normalize_credentials(req: &mut UpsertConnectionReq) -> Result<()> {
    if req.kind == ConnectionKind::Mongodb {
        if let Some(uri) = req.params.get("conn_string").and_then(|v| v.as_str()) {
            if let Some((template, password)) =
                otto_core::connection_credentials::extract_password(uri)?
            {
                req.params["conn_string"] = template.into();
                if req.secret.is_none() {
                    req.secret = Some(password);
                }
            }
        }
    }
    Ok(())
}

fn secret_ref_for(id: &Id) -> String {
    format!("conn-{id}")
}

/// CRUD + open/test for connection profiles.
pub struct ConnectionsService {
    repo: ConnectionsRepo,
    sections: ConnectionSectionsRepo,
    secrets: Arc<dyn SecretStore>,
    pub(crate) transfers: crate::transfers::Transfers,
    pub(crate) sftp_pool: Arc<crate::sftp_pool::SftpPool>,
    credentials_lock: tokio::sync::Mutex<()>,
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
            transfers: crate::transfers::Transfers::default(),
            sftp_pool: Arc::new(crate::sftp_pool::SftpPool::default()),
            credentials_lock: tokio::sync::Mutex::new(()),
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
        let _guard = self.credentials_lock.lock().await;
        let mut conn = self.repo.get(id).await?;
        if conn.kind == ConnectionKind::Mongodb {
            if let Some(uri) = conn.params.get("conn_string").and_then(|v| v.as_str()) {
                if let Some((template, password)) =
                    otto_core::connection_credentials::extract_password(uri)?
                {
                    // Write a fresh secret first: a failed Keychain write leaves the row
                    // untouched, and a failed DB write cannot replace its old secret.
                    let key = format!("conn-{}-{}", id, otto_core::new_id());
                    self.secrets.put(&key, &password)?;
                    let superseded = conn.secret_ref.clone();
                    conn.params["conn_string"] = template.into();
                    conn = self
                        .repo
                        .update(
                            id,
                            None,
                            Some(&conn.params),
                            Some(Some(&key)),
                            None,
                            None,
                            None,
                            None,
                        )
                        .await?;
                    // The row now points at the fresh key; the credential it
                    // used to reference would otherwise linger in the Keychain
                    // forever (nothing else ever reads or deletes it).
                    if let Some(old) = superseded.filter(|old| *old != key) {
                        if let Err(e) = self.secrets.delete(&old) {
                            tracing::warn!(connection = %id, "failed to delete superseded secret: {e}");
                        }
                    }
                }
            }
        }
        Ok(conn)
    }

    pub async fn authorize(&self, id: &Id, user_id: &Id, operation: &str) -> Result<()> {
        let conn = self.repo.get(id).await?;
        crate::access::check(&self.repo.pool(), &conn, user_id, operation).await
    }

    /// Explicit root-only portable export. Read the profile and its credential
    /// under the same lock as updates, so an export cannot combine old identity
    /// settings with a newly rotated password. The default never reads Keychain.
    pub async fn export_profile(
        &self,
        id: &Id,
        actor_id: &Id,
        include_passwords: bool,
    ) -> Result<crate::conn_export::ExportProfile> {
        let _guard = self.credentials_lock.lock().await;
        let actor = otto_state::UsersRepo::new(self.repo.pool())
            .get(actor_id)
            .await?;
        if actor.disabled || !actor.is_root {
            return Err(Error::Forbidden(
                "connection export requires an active root user".into(),
            ));
        }
        let connection = self.repo.get(id).await?;
        let password = if include_passwords {
            match connection.secret_ref.as_deref() {
                Some(reference) => Some(self.secrets.get(reference)
                    .map_err(|_| Error::Internal("Could not read a saved connection credential; no export was produced".into()))?
                    .ok_or_else(|| Error::Conflict("A saved connection credential is missing; reconnect it or export without passwords".into()))?),
                None => None,
            }
        } else {
            None
        };
        let record = serde_json::to_value(connection)
            .map_err(|_| Error::Internal("Could not serialize connection profile".into()))?;
        Ok(crate::conn_export::ExportProfile { record, password })
    }

    pub async fn is_enforced(&self, id: &Id) -> Result<bool> {
        crate::access::enforced(&self.repo.pool(), id).await
    }

    pub async fn visible_connection(&self, conn: Connection, user_id: &Id) -> Result<Connection> {
        crate::access::check(&self.repo.pool(), &conn, user_id, "discover").await?;
        if crate::access::check(&self.repo.pool(), &conn, user_id, "configure")
            .await
            .is_err()
        {
            return Ok(crate::access::redact(conn));
        }
        self.get(&conn.id).await
    }

    /// Ids of the connections under an `Enforced` access policy — ONE query
    /// for a whole list instead of `is_enforced` per row (SI-05).
    async fn enforced_ids(&self) -> Result<std::collections::HashSet<Id>> {
        otto_state::resource_access::ResourceAccessRepo::new(self.repo.pool())
            .enforced_ids(otto_core::access::ResourceKind::Connection)
            .await
    }

    /// A row from `list_visible`, finished the way [`Self::get`] would finish
    /// it — without re-reading it or taking the global credentials lock unless
    /// the legacy Mongo inline-password migration actually has work to do.
    async fn finish_listed(&self, conn: Connection) -> Result<Connection> {
        if needs_credential_migration(&conn) {
            self.get(&conn.id).await
        } else {
            Ok(conn)
        }
    }

    /// Connections visible to a workspace (its own + global).
    pub async fn list(&self, ws: &Id) -> Result<Vec<Connection>> {
        // An identity-free legacy adapter must never reveal governed profiles.
        let enforced = self.enforced_ids().await?;
        let mut visible = Vec::new();
        for conn in self.repo.list_visible(ws).await? {
            if !enforced.contains(&conn.id) {
                visible.push(self.finish_listed(conn).await?);
            }
        }
        Ok(visible)
    }

    /// Like `list` but filtered to connections created by `user_id`.
    /// Used when `connections.owner_private = true`.
    ///
    /// Non-enforced rows pass every resource check by definition, so they are
    /// returned straight from the list query; only enforced rows pay for the
    /// per-row discover/configure evaluation (it used to cost 4 sequential
    /// queries plus the global credentials lock per row, SI-05).
    pub async fn list_for(&self, ws: &Id, user_id: &Id) -> Result<Vec<Connection>> {
        let private = otto_state::SettingsRepo::new(self.repo.pool())
            .get("connections.owner_private")
            .await?
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let user = otto_state::UsersRepo::new(self.repo.pool())
            .get(user_id)
            .await?;
        let enforced = self.enforced_ids().await?;
        let mut visible = Vec::new();
        for conn in self.repo.list_visible(ws).await? {
            if !enforced.contains(&conn.id) {
                if private && !user.is_root && conn.created_by != *user_id {
                    continue;
                }
                match self.finish_listed(conn).await {
                    Ok(conn) => visible.push(conn),
                    Err(Error::NotFound(_)) => {} // deleted mid-list
                    Err(error) => return Err(error),
                }
                continue;
            }
            match self.visible_connection(conn, user_id).await {
                Ok(conn) => visible.push(conn),
                Err(Error::Forbidden(_) | Error::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(visible)
    }

    /// Create a profile; `workspace_id = None` makes it global (root-managed).
    pub async fn create(
        &self,
        workspace_id: Option<Id>,
        user_id: &Id,
        mut req: UpsertConnectionReq,
    ) -> Result<Connection> {
        let actor = otto_state::UsersRepo::new(self.repo.pool())
            .get(user_id)
            .await?;
        if actor.disabled || !actor.is_root {
            return Err(Error::Forbidden("root must provision native connection identities and credentials before they can be delegated".into()));
        }
        normalize_credentials(&mut req)?;
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
        let user = otto_state::UsersRepo::new(self.repo.pool())
            .get(user_id)
            .await?;
        let feature_grants = otto_state::GrantsRepo::new(self.repo.pool());
        let connections_cap = feature_grants
            .capability_of(&user, otto_core::domain::Feature::Connections)
            .await?;
        let database_cap = feature_grants
            .capability_of(&user, otto_core::domain::Feature::Database)
            .await?;
        use otto_core::domain::Capability;
        let mut operations: Vec<String> = ["discover", "configure", "manage_access"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        if connections_cap >= Capability::Edit {
            operations.extend(
                ["shell", "sftp_read", "sftp_write"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        if database_cap >= Capability::View {
            operations.extend(
                ["db_browse", "db_query", "db_export"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        if database_cap >= Capability::Edit {
            operations.extend(
                ["db_data", "db_schema", "change_submit"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        if database_cap >= Capability::Admin {
            operations.extend(
                ["change_approve", "change_execute"]
                    .into_iter()
                    .map(str::to_owned),
            );
        }
        otto_state::resource_access::ResourceAccessRepo::new(self.repo.pool())
            .initialize_owner_policy(
                otto_core::access::ResourceKind::Connection,
                &conn.id,
                user_id,
                &operations,
                &operations,
                &otto_core::access::AccessActor {
                    real_user_id: user_id.clone(),
                    effective_user_id: None,
                },
            )
            .await?;
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
    pub async fn update(
        &self,
        id: &Id,
        user_id: &Id,
        mut req: UpsertConnectionReq,
    ) -> Result<Connection> {
        self.authorize(id, user_id, "configure").await?;
        let existing = self.get(id).await?;
        let _guard = self.credentials_lock.lock().await;
        if req.kind != existing.kind {
            return Err(Error::Invalid(
                "connection kind cannot be changed — create a new connection".into(),
            ));
        }
        let actor = otto_state::UsersRepo::new(self.repo.pool())
            .get(user_id)
            .await?;
        if self.is_enforced(id).await? && !actor.is_root {
            // Full-form clients preserve a stored secret by omitting it.
            // Never turn a limited configuration update into a secret oracle.
            let replacing_secret = req.secret.is_some();
            if req.params != existing.params
                || replacing_secret
                || req.first_command != existing.first_command
                || req.environment.is_some_and(|e| e != existing.environment)
                || req.read_only.is_some_and(|v| v != existing.read_only)
            {
                return Err(Error::Forbidden("root must change connection identity, credentials, commands or environment protections".into()));
            }
        }
        normalize_credentials(&mut req)?;
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

    /// Copy configuration, never credentials or access grants. The new profile
    /// starts with its own owner policy and requires a new password.
    pub async fn duplicate(&self, id: &Id, user_id: &Id) -> Result<Connection> {
        self.authorize(id, user_id, "configure").await?;
        let source = self.get(id).await?;
        self.create(
            source.workspace_id,
            user_id,
            UpsertConnectionReq {
                name: format!("{} (copy)", source.name),
                kind: source.kind,
                params: source.params,
                secret: None,
                first_command: source.first_command,
                section_id: source.section_id,
                environment: Some(source.environment),
                read_only: Some(source.read_only),
            },
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
            return Err(Error::Forbidden(
                "governed database/custom profiles cannot open unrestricted terminal sessions"
                    .into(),
            ));
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
        self.visible_connection(self.repo.set_pinned(id, pinned).await?, user_id)
            .await
    }

    /// Headless test-connect: run the command with a kind-specific probe,
    /// 10s timeout, report ok/latency and the line that explains a failure.
    pub async fn test(&self, conn: &Connection, user_id: &Id) -> Result<TestConnectionResp> {
        self.authorize(&conn.id, user_id, "configure").await?;
        let secret = self.fetch_secret(conn)?;
        probe(conn, secret.as_deref()).await
    }

    /// Test-before-save for an SSH profile: probe the form's CURRENT (unsaved)
    /// values. Nothing is persisted. Root-only, like creating a profile — it
    /// dials an arbitrary host with this Mac's keys / agent. SSH profiles carry
    /// no secret, so no Keychain read happens either. Database kinds have their
    /// own driver-backed `/connections/unsaved/db/test`.
    pub async fn test_unsaved(
        &self,
        user_id: &Id,
        kind: ConnectionKind,
        params: serde_json::Value,
    ) -> Result<TestConnectionResp> {
        let actor = otto_state::UsersRepo::new(self.repo.pool())
            .get(user_id)
            .await?;
        if actor.disabled || !actor.is_root {
            return Err(Error::Forbidden(
                "only root can test connection settings before they are saved".into(),
            ));
        }
        if kind != ConnectionKind::Ssh {
            return Err(Error::Invalid(
                "only SSH settings are tested here — database kinds use /connections/unsaved/db/test"
                    .into(),
            ));
        }
        validate_params(kind, &params, false)?;
        let conn = transient_connection(kind, params);
        probe(&conn, None).await
    }

    fn fetch_secret(&self, conn: &Connection) -> Result<Option<String>> {
        match &conn.secret_ref {
            Some(secret_ref) => self.secrets.get(secret_ref),
            None => Ok(None),
        }
    }
}

/// A throwaway, never-persisted profile for probing unsaved settings.
fn transient_connection(kind: ConnectionKind, params: serde_json::Value) -> Connection {
    Connection {
        id: "unsaved".into(),
        workspace_id: None,
        name: "unsaved".into(),
        kind,
        params,
        secret_ref: None,
        first_command: None,
        section_id: None,
        environment: otto_core::domain::Environment::Dev,
        read_only: false,
        created_by: String::new(),
        created_at: chrono::Utc::now(),
        last_opened_at: None,
        pinned: false,
    }
}

/// The param a kind needs before there is anything to test. Without it
/// `build_command` falls back to a local login shell (so a `first_command` can
/// carry the whole invocation) — and probing THAT would report a green "ok"
/// for a profile that never touches the network.
fn missing_target(conn: &Connection) -> Option<&'static str> {
    let key = match conn.kind {
        ConnectionKind::Ssh
        | ConnectionKind::Mysql
        | ConnectionKind::Redis
        | ConnectionKind::Clickhouse
        | ConnectionKind::Postgres => "host",
        ConnectionKind::Mongodb => "conn_string",
        ConnectionKind::Custom => "command_template",
    };
    let present = conn
        .params
        .get(key)
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.trim().is_empty());
    (!present).then_some(match key {
        "host" => "No host is set, so there is nothing to test — add a host (it only opens a local shell as-is).",
        "conn_string" => "No connection string is set, so there is nothing to test.",
        _ => "No command template is set, so there is nothing to test.",
    })
}

/// Hint for a probe that hit [`TEST_TIMEOUT`].
const TIMEOUT_HINT: &str = "the host didn't answer in time. Check the VPN, firewall / security group, the port, or the jump host";

/// Run a connection's kind-specific headless probe (no authorization — the
/// callers check it). `warn_key_perms` stays `None`: the HTTP handlers overlay
/// it uniformly for every probe path.
pub(crate) async fn probe(conn: &Connection, secret: Option<&str>) -> Result<TestConnectionResp> {
    let warn_key_perms = None;
    if let Some(message) = missing_target(conn) {
        return Ok(TestConnectionResp {
            ok: false,
            latency_ms: None,
            message: message.into(),
            warn_argv: false,
            warn_key_perms,
            hint: None,
        });
    }
    let (spec, warn_argv) = build_command(conn, secret)?;
    let (spec, probe) = probe_spec(conn.kind, spec);
    // ssh (the SSH kind, or a DB client run on a jump host) prints notices
    // before its real error; pick the explanatory line + a fix hint.
    let via_ssh = spec.program == "ssh";

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
                hint: (e.kind() == std::io::ErrorKind::NotFound).then(|| {
                    format!(
                        "`{}` isn't installed or isn't on the daemon's PATH",
                        spec.program
                    )
                }),
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
            message: format!("timed out after {}s", TEST_TIMEOUT.as_secs()),
            warn_argv,
            warn_key_perms,
            hint: Some(TIMEOUT_HINT.into()),
        }),
        Ok(Err(e)) => Ok(TestConnectionResp {
            ok: false,
            latency_ms: None,
            message: format!("process error: {e}"),
            warn_argv,
            warn_key_perms,
            hint: None,
        }),
        Ok(Ok(output)) => {
            let latency_ms = started.elapsed().as_millis() as u64;
            if output.status.success() {
                return Ok(TestConnectionResp {
                    ok: true,
                    latency_ms: Some(latency_ms),
                    message: "ok".into(),
                    warn_argv,
                    warn_key_perms,
                    hint: None,
                });
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            let (line, hint) = if via_ssh {
                let d = otto_ssh::diagnose_ssh_stderr(&stderr);
                (d.line, d.hint.map(str::to_string))
            } else {
                let first = stderr
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                (first, None)
            };
            let message = if line.is_empty() {
                format!("exited with {}", output.status)
            } else {
                // Keep the real driver error, but scrub any password a client
                // might echo (mongo `user:pass@`, `--password x`).
                redact_secrets(&line)
            };
            Ok(TestConnectionResp {
                ok: false,
                latency_ms: Some(latency_ms),
                message,
                warn_argv,
                warn_key_perms,
                hint,
            })
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

/// True when [`ConnectionsService::get`] would rewrite this row: a Mongo profile
/// whose `conn_string` still carries an inline password (or one that cannot be
/// parsed — `get` then surfaces the same error it always did).
fn needs_credential_migration(conn: &Connection) -> bool {
    conn.kind == ConnectionKind::Mongodb
        && conn
            .params
            .get("conn_string")
            .and_then(|v| v.as_str())
            .is_some_and(|uri| {
                !matches!(
                    otto_core::connection_credentials::extract_password(uri),
                    Ok(None)
                )
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory Keychain stand-in that records every key it holds.
    #[derive(Default)]
    struct MemSecrets(Mutex<HashMap<String, String>>);
    impl SecretStore for MemSecrets {
        fn put(&self, key: &str, value: &str) -> Result<()> {
            self.0.lock().unwrap().insert(key.into(), value.into());
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

    struct Fixture {
        root_dir: std::path::PathBuf,
        svc: ConnectionsService,
        repo: ConnectionsRepo,
        secrets: Arc<MemSecrets>,
        root: Id,
        member: Id,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root_dir);
        }
    }

    async fn fixture() -> Fixture {
        let root_dir = std::env::temp_dir().join(format!("otto-conn-svc-{}", otto_core::new_id()));
        std::fs::create_dir(&root_dir).unwrap();
        let pool = otto_state::DbPool::from(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    sqlx::sqlite::SqliteConnectOptions::new()
                        .filename(root_dir.join("svc.db"))
                        .create_if_missing(true)
                        .foreign_keys(true),
                )
                .await
                .unwrap(),
        );
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let mut ids = Vec::new();
        for is_root in [1, 0] {
            let id = otto_core::new_id();
            sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,disabled,created_at) VALUES(?,?, 'hash','Fixture',?,0,strftime('%Y-%m-%dT%H:%M:%fZ','now'))")
                .bind(&id).bind(&id).bind(is_root).execute(&pool).await.unwrap();
            ids.push(id);
        }
        let secrets = Arc::new(MemSecrets::default());
        let repo = ConnectionsRepo::new(pool.clone());
        let svc = ConnectionsService::new(
            ConnectionsRepo::new(pool.clone()),
            ConnectionSectionsRepo::new(pool),
            secrets.clone(),
        );
        Fixture {
            root_dir,
            svc,
            repo,
            secrets,
            member: ids.pop().unwrap(),
            root: ids.pop().unwrap(),
        }
    }

    /// A profile with no host only opens a local shell — testing it must not
    /// report a green "ok" for something that never touched the network.
    #[tokio::test]
    async fn probe_without_a_target_says_what_is_missing() {
        for (kind, params) in [
            (ConnectionKind::Ssh, serde_json::json!({"user":"me"})),
            (ConnectionKind::Mysql, serde_json::json!({"host":"  "})),
            (ConnectionKind::Mongodb, serde_json::json!({})),
        ] {
            let resp = probe(&transient_connection(kind, params), None)
                .await
                .unwrap();
            assert!(!resp.ok, "{kind:?}");
            assert!(resp.message.contains("nothing to test"), "{}", resp.message);
            assert_eq!(resp.latency_ms, None);
        }
    }

    #[tokio::test]
    async fn unsaved_test_is_root_only_ssh_only_and_validated() {
        let f = fixture().await;
        let ssh = serde_json::json!({"host":"build.example.com"});
        assert!(matches!(
            f.svc
                .test_unsaved(&f.member, ConnectionKind::Ssh, ssh.clone())
                .await,
            Err(Error::Forbidden(_))
        ));
        assert!(matches!(
            f.svc
                .test_unsaved(&f.root, ConnectionKind::Mysql, ssh)
                .await,
            Err(Error::Invalid(_))
        ));
        // An option-like host is refused before any ssh process starts.
        assert!(matches!(
            f.svc
                .test_unsaved(
                    &f.root,
                    ConnectionKind::Ssh,
                    serde_json::json!({"host":"-oProxyCommand=touch /tmp/x"})
                )
                .await,
            Err(Error::Invalid(_))
        ));
        let resp = f
            .svc
            .test_unsaved(&f.root, ConnectionKind::Ssh, serde_json::json!({}))
            .await
            .unwrap();
        assert!(!resp.ok && resp.message.contains("No host"));
    }

    /// The legacy Mongo inline-password migration moves the credential to a
    /// fresh Keychain key — and must not orphan the one it replaces.
    #[tokio::test]
    async fn mongo_password_migration_deletes_the_superseded_secret() {
        let f = fixture().await;
        let conn = f
            .repo
            .create(NewConnection {
                workspace_id: None,
                name: "legacy".into(),
                kind: ConnectionKind::Mongodb,
                params: serde_json::json!({"conn_string":"mongodb://app:inline-pw@m1:27017/db"}),
                secret_ref: Some("conn-legacy-old".into()),
                first_command: None,
                section_id: None,
                environment: Default::default(),
                read_only: false,
                created_by: f.root.clone(),
            })
            .await
            .unwrap();
        f.secrets.put("conn-legacy-old", "stale-pw").unwrap();

        let migrated = f.svc.get(&conn.id).await.unwrap();
        let new_ref = migrated.secret_ref.clone().unwrap();
        assert_ne!(new_ref, "conn-legacy-old");
        assert!(!migrated.params["conn_string"]
            .as_str()
            .unwrap()
            .contains("inline-pw"));
        let held = f.secrets.0.lock().unwrap().clone();
        assert_eq!(held.get(&new_ref).map(String::as_str), Some("inline-pw"));
        assert!(
            !held.contains_key("conn-legacy-old"),
            "superseded secret must be removed: {held:?}"
        );
    }
}
