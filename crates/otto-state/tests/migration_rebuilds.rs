//! What `migration_compat.rs` cannot see in a table rebuild (create `t_new` →
//! copy → drop `t` → rename): the column set can survive while the DATA, the
//! table's indexes or its triggers do not. `DROP TABLE` silently takes every
//! index and trigger on the old table with it, so a rebuild that forgets to
//! re-create one passes the schema lint and ships a slow (or unguarded) table.
//!
//! Two checks:
//! * every index and trigger that exists before a post-baseline migration
//!   still exists after it, unless the migration drops it BY NAME (or drops
//!   its table for good);
//! * migration 0185 (`issue_accounts` rebuilt to allow Linear) keeps every
//!   existing Jira row byte-for-byte and only WIDENS the provider CHECK.

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Connection, SqliteConnection, SqlitePool};

/// Migrations up to here predate this check (mirrors migration_compat.rs).
const BASELINE: i64 = 176;

async fn memory() -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap()
}

/// Run one migration the way sqlx does (in a transaction unless `no_tx`).
async fn apply(conn: &mut SqliteConnection, sql: &str, no_tx: bool) {
    if no_tx {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&mut *conn)
            .await
            .unwrap();
    } else {
        let mut tx = conn.begin().await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
}

/// `(type, name, table)` of every named index and trigger (sqlite's implicit
/// `sqlite_autoindex_*` follow their table's PRIMARY KEY / UNIQUE and are
/// covered by the column lint).
async fn objects(conn: &mut SqliteConnection) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT type, name, tbl_name FROM sqlite_master \
         WHERE type IN ('index','trigger') AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap()
}

async fn tables(conn: &mut SqliteConnection) -> Vec<String> {
    sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(&mut *conn)
        .await
        .unwrap()
}

/// Whether `sql` drops `kind` (`INDEX`/`TRIGGER`) `name` explicitly.
fn drops_by_name(sql: &str, kind: &str, name: &str) -> bool {
    let norm = sql
        .to_ascii_uppercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name = name.to_ascii_uppercase();
    [
        format!("DROP {kind} {name}"),
        format!("DROP {kind} IF EXISTS {name}"),
    ]
    .iter()
    .any(|needle| {
        norm.match_indices(needle.as_str()).any(|(i, _)| {
            // Whole identifier only: `idx_a` must not match `idx_ab`.
            norm[i + needle.len()..]
                .chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        })
    })
}

#[tokio::test]
async fn rebuilds_keep_every_index_and_trigger_of_the_table() {
    let pool = memory().await;
    let mut conn = pool.acquire().await.unwrap();
    let migrator = sqlx::migrate!("./migrations");
    let mut failures = Vec::new();
    for m in migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
    {
        let before = if m.version > BASELINE {
            Some(objects(&mut conn).await)
        } else {
            None
        };
        apply(&mut conn, m.sql.as_str(), m.no_tx).await;
        let Some(before) = before else { continue };
        let after = objects(&mut conn).await;
        let live = tables(&mut conn).await;
        for (kind, name, table) in before {
            let still = after.iter().any(|(k, n, _)| *k == kind && *n == name);
            let deliberate = drops_by_name(m.sql.as_str(), &kind.to_ascii_uppercase(), &name)
                || !live.contains(&table);
            if !still && !deliberate {
                failures.push(format!(
                    "{:04}_{}: {kind} `{name}` on `{table}` vanished — a rebuilt table \
                     must re-create it (DROP TABLE takes it along)",
                    m.version, m.description
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn drop_detection_matches_whole_names_only() {
    assert!(drops_by_name("DROP INDEX idx_a;", "INDEX", "idx_a"));
    assert!(drops_by_name(
        "drop  index if exists\nidx_a",
        "INDEX",
        "idx_a"
    ));
    assert!(!drops_by_name("DROP INDEX idx_ab;", "INDEX", "idx_a"));
    assert!(!drops_by_name("DROP TABLE idx_a;", "INDEX", "idx_a"));
    assert!(drops_by_name("DROP TRIGGER trg_x", "TRIGGER", "trg_x"));
}

/// The lint catches the bug it exists for: a rebuild without its index.
#[tokio::test]
async fn a_rebuild_that_forgets_its_index_is_visible() {
    let pool = memory().await;
    let mut conn = pool.acquire().await.unwrap();
    apply(
        &mut conn,
        "CREATE TABLE t (id TEXT PRIMARY KEY, u TEXT); CREATE INDEX idx_t_u ON t(u);",
        false,
    )
    .await;
    let before = objects(&mut conn).await;
    apply(
        &mut conn,
        "CREATE TABLE t_new (id TEXT PRIMARY KEY, u TEXT); INSERT INTO t_new SELECT * FROM t; \
         DROP TABLE t; ALTER TABLE t_new RENAME TO t;",
        false,
    )
    .await;
    let after = objects(&mut conn).await;
    assert_eq!(before.len(), 1);
    assert!(after.is_empty(), "DROP TABLE took the index: {after:?}");
}

#[tokio::test]
async fn issue_accounts_rebuild_keeps_jira_rows_and_only_widens_the_provider_check() {
    const REBUILD: i64 = 185;
    let pool = memory().await;
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .unwrap();
    let migrator = sqlx::migrate!("./migrations");
    let ups: Vec<_> = migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
        .collect();
    for m in ups.iter().filter(|m| m.version < REBUILD) {
        apply(&mut conn, m.sql.as_str(), m.no_tx).await;
    }

    // A user's Jira account as the previous build stored it.
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) \
         VALUES ('u1', 'dana', 'h', 'Dana', 0, '2026-01-01T00:00:00Z')",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO issue_accounts (id, user_id, provider, label, username, token_ref, \
         api_base_url, created_at, token_expires_at) VALUES \
         ('a1', 'u1', 'jira', 'Work', 'dana@acme.io', 'issueacct-1', 'https://acme.atlassian.net', \
          '2026-02-01T00:00:00Z', '2027-02-01T00:00:00Z'), \
         ('a2', 'u1', 'jira', 'Old', 'dana@old.io', 'issueacct-2', NULL, '2026-03-01T00:00:00Z', NULL)",
    )
    .execute(&mut *conn)
    .await
    .unwrap();
    // Before the rebuild a Linear row is refused — the point of 0185.
    assert!(sqlx::query(
        "INSERT INTO issue_accounts (id, user_id, provider, label, username, token_ref, created_at) \
         VALUES ('x', 'u1', 'linear', 'L', '', 'r', '2026-01-01T00:00:00Z')"
    )
    .execute(&mut *conn)
    .await
    .is_err());

    type Row = (
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
    );
    const SELECT: &str = "SELECT id, user_id, provider, label, username, token_ref, api_base_url, \
                          created_at, token_expires_at FROM issue_accounts ORDER BY id";
    let before: Vec<Row> = sqlx::query_as(SELECT).fetch_all(&mut *conn).await.unwrap();

    for m in ups.iter().filter(|m| m.version >= REBUILD) {
        apply(&mut conn, m.sql.as_str(), m.no_tx).await;
    }

    let after: Vec<Row> = sqlx::query_as(SELECT).fetch_all(&mut *conn).await.unwrap();
    assert_eq!(
        after, before,
        "every column of every row survives the rebuild"
    );
    assert_eq!(after.len(), 2);

    // Widened, not replaced: jira and linear go in, anything else is refused.
    for (id, provider, ok) in [
        ("n1", "linear", true),
        ("n2", "jira", true),
        ("n3", "github", false),
    ] {
        let r = sqlx::query(
            "INSERT INTO issue_accounts (id, user_id, provider, label, username, token_ref, created_at) \
             VALUES (?, 'u1', ?, 'L', '', 'r', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(provider)
        .execute(&mut *conn)
        .await;
        assert_eq!(r.is_ok(), ok, "{provider}: {r:?}");
    }
    // The FK to users still cascades (the rebuilt table kept its REFERENCES).
    sqlx::query("DELETE FROM users WHERE id = 'u1'")
        .execute(&mut *conn)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM issue_accounts")
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    assert_eq!(left, 0, "accounts go with their user");
    // And the per-user lookup index is back.
    let idx: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' \
         AND name = 'idx_issue_accounts_user' AND tbl_name = 'issue_accounts'",
    )
    .fetch_one(&mut *conn)
    .await
    .unwrap();
    assert_eq!(idx, 1);
}
