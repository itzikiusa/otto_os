//! A connection marked **Read-only** refuses every write/DDL outright.
//!
//! Production keeps its typed confirmation (a person may deliberately write to
//! production); Read-only is the user's own statement that nothing may change
//! through this profile, so no confirmation, from a person or an agent, lifts
//! it. An access-enforced connection already refused here (its access gate);
//! a legacy one used to take the same typed confirm as production, which made
//! "Read-only" a speed bump rather than a guard. The one way to write is to
//! turn the setting off in the connection's settings.
//!
//! Redis `KEYS` is a read: on a read-only connection it keeps the blocking-read
//! confirmation of [`super::DbViewerService::guard_write_conn`].

use otto_core::domain::Connection;
use otto_core::{Error, Result};

use super::READ_ONLY_CONNECTION_PREFIX;
use crate::multirun::MultiRunPlan;
use crate::types::{statement_is_write, Engine};

/// Refuse `statement` when `conn` is read-only and the statement classifies
/// as a write/DDL (the same conservative classifier as the write-guard; an
/// unmappable engine counts as a write). Checked BEFORE any confirmation.
pub(super) fn refuse_write(conn: &Connection, statement: &str) -> Result<()> {
    if !conn.read_only {
        return Ok(());
    }
    let is_write = match Engine::from_kind(conn.kind) {
        Some(engine) => statement_is_write(engine, statement),
        None => true,
    };
    if !is_write {
        return Ok(());
    }
    Err(refusal(&conn.name, super::guard_setting_hint(conn)))
}

/// The refusal text: names the connection and the setting that lifts it, and
/// never invites a confirmation (there is none).
fn refusal(name: &str, hint: &str) -> Error {
    Error::Forbidden(format!(
        "{READ_ONLY_CONNECTION_PREFIX}read-only connection '{name}' refuses writes and \
         schema changes; {hint} in the connection's settings to allow them"
    ))
}

/// The multi-run mirror: a plan with any write on a read-only target is
/// refused as a whole, before anything runs and whatever `confirm_write` says
/// (each run would be refused by [`refuse_write`] anyway; failing up front
/// keeps the batch all-or-nothing).
pub(super) fn multirun_refusal(plan: &MultiRunPlan) -> Option<Error> {
    let writes: Vec<&str> = plan
        .runs
        .iter()
        .filter_map(|r| {
            plan.targets
                .get(r.target)
                .filter(|t| r.is_write && t.read_only)
        })
        .map(|t| t.label.as_str())
        .collect();
    if writes.is_empty() {
        return None;
    }
    let mut names = writes.clone();
    names.dedup();
    Some(Error::Forbidden(format!(
        "{READ_ONLY_CONNECTION_PREFIX}{} run(s) write to read-only targets ({}), which refuse \
         writes and schema changes; turn off Read-only in the connection's settings to allow them",
        writes.len(),
        names.join(", ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::{ConnectionKind, Environment};

    fn conn(kind: ConnectionKind, environment: Environment, read_only: bool) -> Connection {
        Connection {
            id: "c1".into(),
            workspace_id: None,
            name: "ro-db".into(),
            kind,
            params: serde_json::json!({}),
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment,
            read_only,
            created_by: "u1".into(),
            created_at: chrono::Utc::now(),
            last_opened_at: None,
            pinned: false,
        }
    }

    #[test]
    fn a_read_only_connection_refuses_every_write_with_no_confirm_path() {
        let c = conn(ConnectionKind::Mysql, Environment::Dev, true);
        for stmt in [
            "DELETE FROM t WHERE id = 1",
            "UPDATE t SET a = 1 WHERE id = 1",
            "DROP TABLE t",
            "SET @x = 1",
        ] {
            let err = refuse_write(&c, stmt).expect_err(stmt);
            let msg = err.to_string();
            assert!(matches!(err, Error::Forbidden(_)), "{stmt}: {err:?}");
            assert!(msg.contains(READ_ONLY_CONNECTION_PREFIX), "{msg}");
            assert!(msg.contains("read-only connection 'ro-db'"), "{msg}");
            assert!(
                msg.contains("turn off Read-only in the connection's settings"),
                "{msg}"
            );
            assert!(
                !msg.contains("confirm"),
                "no confirmation is offered: {msg}"
            );
        }
    }

    #[test]
    fn reads_and_unguarded_connections_pass() {
        let ro = conn(ConnectionKind::Mysql, Environment::Dev, true);
        refuse_write(&ro, "SELECT * FROM t").expect("a read runs");
        // Redis KEYS is a blocking READ: it keeps its own confirm, not this refusal.
        let redis = conn(ConnectionKind::Redis, Environment::Dev, true);
        refuse_write(&redis, "KEYS *").expect("KEYS is not a write");
        refuse_write(&redis, "SET k v").expect_err("a Redis write is refused");
        // Production alone keeps the typed confirmation (decided elsewhere).
        let prod = conn(ConnectionKind::Mysql, Environment::Prod, false);
        refuse_write(&prod, "DELETE FROM t").expect("prod is not refused here");
    }

    #[test]
    fn read_only_production_names_both_settings() {
        let both = conn(ConnectionKind::Mysql, Environment::Prod, true);
        let msg = refuse_write(&both, "DROP TABLE t")
            .expect_err("refused")
            .to_string();
        assert!(
            msg.contains("turn off Read-only and set an environment other than Production"),
            "{msg}"
        );
    }
}
