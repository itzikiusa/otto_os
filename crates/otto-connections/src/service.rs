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

    /// Connections visible to a workspace (its own + global).
    pub async fn list(&self, ws: &Id) -> Result<Vec<Connection>> {
        self.repo.list_visible(ws).await
    }

    /// Like `list` but filtered to connections created by `user_id`.
    /// Used when `connections.owner_private = true`.
    pub async fn list_for(&self, ws: &Id, user_id: &Id) -> Result<Vec<Connection>> {
        self.repo.list_visible_for(ws, user_id).await
    }

    /// Create a profile; `workspace_id = None` makes it global (root-managed).
    pub async fn create(
        &self,
        workspace_id: Option<Id>,
        user_id: &Id,
        req: UpsertConnectionReq,
    ) -> Result<Connection> {
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
    pub async fn update(&self, id: &Id, req: UpsertConnectionReq) -> Result<Connection> {
        let existing = self.repo.get(id).await?;
        if req.kind != existing.kind {
            return Err(Error::Invalid(
                "connection kind cannot be changed — create a new connection".into(),
            ));
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
    pub async fn delete(&self, id: &Id) -> Result<()> {
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
    pub async fn set_pinned(&self, id: &Id, pinned: bool) -> Result<Connection> {
        self.repo.set_pinned(id, pinned).await
    }

    /// Headless test-connect: run the command with a kind-specific probe,
    /// 10s timeout, report ok/latency/first stderr line.
    pub async fn test(&self, conn: &Connection) -> Result<TestConnectionResp> {
        let secret = self.fetch_secret(conn)?;
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
            }),
            Ok(Err(e)) => Ok(TestConnectionResp {
                ok: false,
                latency_ms: None,
                message: format!("process error: {e}"),
                warn_argv,
            }),
            Ok(Ok(output)) => {
                let latency_ms = started.elapsed().as_millis() as u64;
                if output.status.success() {
                    Ok(TestConnectionResp {
                        ok: true,
                        latency_ms: Some(latency_ms),
                        message: "ok".into(),
                        warn_argv,
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
                        first_line
                    };
                    Ok(TestConnectionResp {
                        ok: false,
                        latency_ms: Some(latency_ms),
                        message,
                        warn_argv,
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
            // ssh [opts] target  ->  ssh -o BatchMode=yes -o ConnectTimeout=5 [opts] target exit
            let target = spec.args.pop();
            let mut args = vec![
                "-o".to_string(),
                "BatchMode=yes".to_string(),
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
        ConnectionKind::Mysql | ConnectionKind::Clickhouse => (spec, Some(b"SELECT 1;\n")),
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
