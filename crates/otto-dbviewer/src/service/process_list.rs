//! The Database Explorer's Processes panel (`ui/src/modules/database/
//! process-list.ts`) lists what is running with one fixed statement per engine,
//! sent through the normal query route with `read_only: true` (classified as a
//! read, run in the engine's native read-only mode). Its refreshes are polling,
//! not the user's queries: they are not recorded in query history. Only these
//! exact statements, and only on a read-only request, are exempt — anything
//! else the user runs is recorded as before. Keep the text in step with
//! `PROCESS_SQL` in the UI module.

/// MySQL: every connection and the statement it is running.
pub const MYSQL_PROCESSLIST: &str = "SHOW FULL PROCESSLIST";
/// ClickHouse: the queries running on this server.
pub const CLICKHOUSE_PROCESSES: &str = "SELECT query_id, user, elapsed, read_rows, memory_usage, \
     query FROM system.processes ORDER BY elapsed DESC";

/// Whether a run should be recorded in history: false only for a read-only
/// run of one of the panel's listing statements.
pub(super) fn records_history(read_only: bool, statement: &str) -> bool {
    if !read_only {
        return true;
    }
    let s = statement.trim().trim_end_matches(';').trim_end();
    !(s == MYSQL_PROCESSLIST || s == CLICKHOUSE_PROCESSES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_read_only_panel_listing_skips_history() {
        assert!(!records_history(true, MYSQL_PROCESSLIST));
        assert!(!records_history(
            true,
            &format!("  {CLICKHOUSE_PROCESSES};\n")
        ));
        // The same text typed into the editor (not read-only) is recorded.
        assert!(records_history(false, MYSQL_PROCESSLIST));
        // Anything else is recorded, read-only or not.
        assert!(records_history(true, "SHOW PROCESSLIST"));
        assert!(records_history(true, "SELECT 1"));
        assert!(records_history(true, "show full processlist"));
    }

    #[test]
    fn the_clickhouse_listing_is_one_line_of_sql() {
        // The UI sends it on one line; a stray newline here would never match.
        assert!(!CLICKHOUSE_PROCESSES.contains('\n'));
        assert!(CLICKHOUSE_PROCESSES.contains("system.processes"));
    }
}
