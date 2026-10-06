//! Migration additivity lint (S10-306).
//!
//! `db::open` sets `set_ignore_missing(true)` so a rolled-back daemon boots on
//! the schema a NEWER build already migrated. That is only safe while every
//! migration leaves the schema readable and writable by the build before it:
//! a dropped/renamed table or column, or a new NOT NULL column without a
//! DEFAULT on an existing table, lets the old binary boot fine and then fail
//! its INSERTs at runtime — worse than the loud boot failure it replaced.
//!
//! The check is semantic, not textual: every migration is applied in order to
//! an in-memory database and the schema before/after each one above
//! [`BASELINE`] is diffed. The table-rebuild pattern (create `t_new` → copy →
//! drop `t` → rename) passes as long as the rebuilt `t` keeps every column and
//! adds none the old build could not satisfy. Narrowing a CHECK constraint is
//! not detectable this way — keep CHECK changes widening-only (AGENTS.md).

use std::collections::BTreeMap;

use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Connection, SqliteConnection, SqlitePool};

/// Every migration up to and including this version predates the lint (some
/// rebuild tables, e.g. 0094/0095/0103/0134) and is grandfathered.
const BASELINE: i64 = 176;

#[derive(Debug, Clone, PartialEq)]
struct Column {
    not_null: bool,
    has_default: bool,
    pk: bool,
}

/// table → column → shape.
type Schema = BTreeMap<String, BTreeMap<String, Column>>;

async fn schema(conn: &mut SqliteConnection) -> Schema {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' \
         AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%'",
    )
    .fetch_all(&mut *conn)
    .await
    .unwrap();
    let mut out = Schema::new();
    for t in tables {
        let rows: Vec<(String, i64, Option<String>, i64)> =
            sqlx::query_as("SELECT name, \"notnull\", dflt_value, pk FROM pragma_table_info(?)")
                .bind(&t)
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        let cols = rows
            .into_iter()
            .map(|(name, nn, dflt, pk)| {
                (
                    name,
                    Column {
                        not_null: nn != 0,
                        has_default: dflt.is_some(),
                        pk: pk != 0,
                    },
                )
            })
            .collect();
        out.insert(t, cols);
    }
    out
}

/// What `after` breaks for a binary built against `before`.
fn compat_violations(before: &Schema, after: &Schema) -> Vec<String> {
    let mut out = Vec::new();
    for (table, old_cols) in before {
        let Some(new_cols) = after.get(table) else {
            out.push(format!("table `{table}` dropped or renamed"));
            continue;
        };
        for (col, old) in old_cols {
            match new_cols.get(col) {
                None => out.push(format!("column `{table}.{col}` dropped or renamed")),
                Some(new) if new.not_null && !old.not_null && !new.has_default && !new.pk => out
                    .push(format!(
                        "column `{table}.{col}` became NOT NULL without a DEFAULT"
                    )),
                Some(_) => {}
            }
        }
        for (col, new) in new_cols {
            if !old_cols.contains_key(col) && new.not_null && !new.has_default && !new.pk {
                out.push(format!(
                    "new column `{table}.{col}` is NOT NULL without a DEFAULT — \
                     the previous build's INSERTs would fail"
                ));
            }
        }
    }
    out
}

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

#[tokio::test]
async fn migrations_after_the_baseline_stay_compatible_with_the_previous_build() {
    let pool = memory().await;
    let mut conn = pool.acquire().await.unwrap();
    let migrator = sqlx::migrate!("./migrations");
    let mut failures = Vec::new();
    for m in migrator
        .iter()
        .filter(|m| !m.migration_type.is_down_migration())
    {
        let before = if m.version > BASELINE {
            Some(schema(&mut conn).await)
        } else {
            None
        };
        apply(&mut conn, m.sql.as_str(), m.no_tx).await;
        if let Some(before) = before {
            let after = schema(&mut conn).await;
            for v in compat_violations(&before, &after) {
                failures.push(format!("{:04}_{}: {v}", m.version, m.description));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "migrations must stay readable + writable by the previous build \
         (a rollback boots it on this schema — see AGENTS.md):\n{}",
        failures.join("\n")
    );
}

/// The lint itself catches what it claims to (and passes the rebuild idiom).
#[tokio::test]
async fn lint_flags_breaking_changes_and_passes_additive_ones() {
    async fn diff(setup: &str, migration: &str) -> Vec<String> {
        let pool = memory().await;
        let mut conn = pool.acquire().await.unwrap();
        apply(&mut conn, setup, false).await;
        let before = schema(&mut conn).await;
        apply(&mut conn, migration, false).await;
        compat_violations(&before, &schema(&mut conn).await)
    }
    let base = "CREATE TABLE t (id TEXT PRIMARY KEY, name TEXT NOT NULL, note TEXT);";
    // Additive: a nullable / defaulted column, a new table, an index.
    assert!(diff(
        base,
        "ALTER TABLE t ADD COLUMN extra TEXT; \
         ALTER TABLE t ADD COLUMN flag INTEGER NOT NULL DEFAULT 0; \
         CREATE TABLE u (id INTEGER PRIMARY KEY, v TEXT NOT NULL); \
         CREATE INDEX idx_t_name ON t(name);"
    )
    .await
    .is_empty());
    // The rebuild idiom that keeps every column passes.
    assert!(diff(
        base,
        "CREATE TABLE t_new (id TEXT PRIMARY KEY, name TEXT NOT NULL, note TEXT, \
         kind TEXT NOT NULL DEFAULT 'a' CHECK (kind IN ('a','b'))); \
         INSERT INTO t_new (id, name, note) SELECT id, name, note FROM t; \
         DROP TABLE t; ALTER TABLE t_new RENAME TO t;"
    )
    .await
    .is_empty());
    // Breaking: drop / rename a column, rename a table, NOT NULL w/o default.
    let v = diff(base, "ALTER TABLE t DROP COLUMN note;").await;
    assert!(v.iter().any(|v| v.contains("t.note")), "{v:?}");
    let v = diff(base, "ALTER TABLE t RENAME COLUMN note TO memo;").await;
    assert!(v.iter().any(|v| v.contains("t.note")), "{v:?}");
    let v = diff(base, "ALTER TABLE t RENAME TO t2;").await;
    assert!(v.iter().any(|v| v.contains("table `t`")), "{v:?}");
    let v = diff(
        base,
        "CREATE TABLE t_new (id TEXT PRIMARY KEY, name TEXT NOT NULL, note TEXT NOT NULL, \
         owner TEXT NOT NULL); \
         INSERT INTO t_new SELECT id, name, coalesce(note,''), '' FROM t; \
         DROP TABLE t; ALTER TABLE t_new RENAME TO t;",
    )
    .await;
    assert!(v.iter().any(|v| v.contains("t.owner")), "{v:?}");
    assert!(v.iter().any(|v| v.contains("t.note")), "{v:?}");
}
