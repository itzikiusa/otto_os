//! The Database Explorer's Processes panel (`ui/src/modules/database/
//! process-list.ts`) lists what is running with one fixed statement per engine,
//! sent through the normal query route with `read_only: true` (classified as a
//! read, run in the engine's native read-only mode). Its refreshes are polling,
//! not the user's queries: they are not recorded in query history. Only these
//! exact statements, and only on a read-only request, are exempt — anything
//! else the user runs is recorded as before. Keep the text in step with
//! `PROCESS_SQL` in the UI module.
//!
//! Kill is NOT a route of its own: the panel sends the engine's kill statement
//! (`KILL QUERY <id>`, `SELECT pg_cancel_backend(<pid>)` /
//! `pg_terminate_backend`, `{"killOp": 1, "op": <opid>}`) through the same
//! query route as any write, so it gets every write rule unchanged — refused
//! on a Read-only connection, the typed confirm on production (never from an
//! agent credential), and a query-history row as its audit trail. The
//! classifier calls each of them a write (`KILL` is not a read keyword; the
//! PostgreSQL functions are side-effect built-ins in
//! `access::function_has_side_effect`; `killOp` is not a parseable Mongo
//! read) — the tests below pin that.

/// MySQL: every connection and the statement it is running.
pub const MYSQL_PROCESSLIST: &str = "SHOW FULL PROCESSLIST";
/// ClickHouse: the queries running on this server.
pub const CLICKHOUSE_PROCESSES: &str = "SELECT query_id, user, elapsed, read_rows, memory_usage, \
     query FROM system.processes ORDER BY elapsed DESC";
/// PostgreSQL: every client session (and any busy background worker), with
/// the time in its current state and its statement capped at 4,000
/// characters. A session of another role reads `<insufficient privilege>` as
/// its query (and NULL state/client/backend type) unless this login has
/// `pg_read_all_stats` — so a NULL backend type is kept, not filtered away,
/// and the panel says why its text is hidden. Only restricted-SQL built-ins
/// and casts (`access::pure_builtin`), so it also runs on an access-enforced
/// connection: hence `now()` (a fresh read-only transaction), and no
/// `pg_backend_pid()` — the panel drops its own row by its query text.
pub const POSTGRES_ACTIVITY: &str = "SELECT pid, usename, datname, client_addr::text AS \
     client_addr, application_name, state, wait_event_type, wait_event, backend_type, \
     round(EXTRACT(EPOCH FROM now() - CASE WHEN state = 'active' THEN query_start ELSE \
     state_change END)::numeric, 1) AS seconds, substr(query, 1, 4000) AS query FROM \
     pg_stat_activity WHERE state = 'active' OR backend_type IS NULL OR backend_type = \
     'client backend' ORDER BY query_start NULLS LAST";
/// MongoDB: a JSON aggregate the Mongo parser reads as a plain (read)
/// aggregate, which the driver (`drivers/mongodb/processes.rs`) runs as
/// `$currentOp` on `admin` instead of on a collection.
pub const MONGO_CURRENT_OP: &str = r#"{"collection":"$cmd.currentOp","pipeline":[{"$currentOp":{"allUsers":true,"idleConnections":false}}]}"#;

/// Whether a run should be recorded in history: false only for a read-only
/// run of one of the panel's listing statements.
pub(super) fn records_history(read_only: bool, statement: &str) -> bool {
    if !read_only {
        return true;
    }
    let s = statement.trim().trim_end_matches(';').trim_end();
    ![
        MYSQL_PROCESSLIST,
        CLICKHOUSE_PROCESSES,
        POSTGRES_ACTIVITY,
        MONGO_CURRENT_OP,
    ]
    .contains(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{statement_is_write, Engine};

    #[test]
    fn only_a_read_only_panel_listing_skips_history() {
        assert!(!records_history(true, MYSQL_PROCESSLIST));
        assert!(!records_history(
            true,
            &format!("  {CLICKHOUSE_PROCESSES};\n")
        ));
        assert!(!records_history(true, POSTGRES_ACTIVITY));
        assert!(!records_history(true, MONGO_CURRENT_OP));
        // The same text typed into the editor (not read-only) is recorded.
        assert!(records_history(false, MYSQL_PROCESSLIST));
        assert!(records_history(false, POSTGRES_ACTIVITY));
        // Anything else is recorded, read-only or not.
        assert!(records_history(true, "SHOW PROCESSLIST"));
        assert!(records_history(true, "SELECT 1"));
        assert!(records_history(true, "show full processlist"));
        // A kill is a write and always leaves its history row (the audit).
        assert!(records_history(false, "SELECT pg_cancel_backend(42)"));
        assert!(records_history(false, r#"{"killOp":1,"op":42}"#));
    }

    #[test]
    fn the_listings_are_one_line_and_classify_as_reads() {
        // The UI sends each on one line; a stray newline here would never match.
        for s in [CLICKHOUSE_PROCESSES, POSTGRES_ACTIVITY, MONGO_CURRENT_OP] {
            assert!(!s.contains('\n'), "{s}");
        }
        assert!(CLICKHOUSE_PROCESSES.contains("system.processes"));
        // `read_only: true` refuses anything the classifier calls a write, so a
        // listing that failed to parse would be refused on every connection.
        for (engine, s) in [
            (Engine::Mysql, MYSQL_PROCESSLIST),
            (Engine::Clickhouse, CLICKHOUSE_PROCESSES),
            (Engine::Postgres, POSTGRES_ACTIVITY),
            (Engine::Mongodb, MONGO_CURRENT_OP),
        ] {
            assert!(!statement_is_write(engine, s), "{engine:?}: {s}");
            super::super::ensure_read_only_request(engine, s).expect("a read-only listing");
        }
        // An access-enforced connection runs restricted SQL only: built-in
        // pure functions and plain casts. The listing must stay inside it.
        let ops = crate::access::operations(Engine::Postgres, POSTGRES_ACTIVITY)
            .expect("restricted SQL accepts the PostgreSQL listing");
        assert!(crate::access::only_reads(&ops), "{ops:?}");
    }

    /// The kill statements the panel sends are writes, so the query route's
    /// write rules (Read-only refusal, production typed confirm, no confirm
    /// from an agent) apply to them exactly as to `KILL QUERY`.
    #[test]
    fn every_kill_statement_classifies_as_a_write() {
        for (engine, s) in [
            (Engine::Mysql, "KILL QUERY 42"),
            (Engine::Postgres, "SELECT pg_cancel_backend(42)"),
            (Engine::Postgres, "SELECT pg_terminate_backend(42)"),
            (Engine::Mongodb, r#"{"killOp":1,"op":42}"#),
            (Engine::Mongodb, r#"{"killOp":1,"op":"shard01:42"}"#),
        ] {
            assert!(statement_is_write(engine, s), "{engine:?}: {s}");
            super::super::ensure_read_only_request(engine, s)
                .expect_err("a kill is refused on a read-only run");
        }
    }
}
