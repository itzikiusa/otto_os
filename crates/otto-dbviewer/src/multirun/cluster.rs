//! ClickHouse cluster awareness for multi-target runs: decide whether a
//! target is a cluster (and which one), and inject `ON CLUSTER <name>` into
//! the DDL statements of a script — never into DML.
//!
//! **Detection** ([`decide`]) weighs three signals, each read from the target
//! server itself:
//!
//! 1. the database's engine (`system.databases.engine`): a `Replicated` /
//!    `Shared` database (ClickHouse Cloud, the Replicated database engine)
//!    replicates DDL on its own and REJECTS an explicit `ON CLUSTER`, so
//!    nothing is injected there;
//! 2. the `{cluster}` macro (`system.macros`) — the conventional name a
//!    cluster's nodes carry for exactly this purpose — when `system.clusters`
//!    actually lists it;
//! 3. `system.clusters`: a cluster counts as REAL only when it has more than
//!    one host and at least one that is not loopback. Stock configs ship
//!    `default` / `test_*` / `parallel_replicas` entries that are all
//!    localhost / 127.x, so "system.clusters is non-empty" alone would call
//!    every single-node server a cluster. One real cluster (preferring those
//!    that contain this node, `is_local`) is used; several are ambiguous and
//!    the user picks.
//!
//! **Rewriting** ([`analyze`], [`rewrite_script`]) uses a small comment- and
//! quote-aware tokenizer and only recognised DDL shapes: CREATE (TABLE /
//! DATABASE / [MATERIALIZED|LIVE|WINDOW] VIEW / DICTIONARY / FUNCTION), ALTER
//! (TABLE / DATABASE), DROP, TRUNCATE, RENAME, EXCHANGE, ATTACH / DETACH and
//! OPTIMIZE. `ON CLUSTER` goes right after the object name (after the last
//! `TO` pair for RENAME / EXCHANGE). A statement that already says
//! `ON CLUSTER`, a TEMPORARY object (session-local) and an unrecognised DDL
//! form are left untouched and reported, so the preview never hides a skip.

use serde::{Deserialize, Serialize};

use crate::split::{split_statements, SqlDialect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Quoted,
    Str,
    Punct(u8),
    Other,
}

#[derive(Debug, Clone, Copy)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

/// Tokenize one ClickHouse statement: words, quoted identifiers (`` ` `` /
/// `"`), string literals, single-char punctuation; whitespace and comments
/// (`--`, `#`, `/* */`) are skipped. Byte offsets; every delimiter is ASCII.
fn tokenize(s: &str) -> Vec<Tok> {
    let b = s.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if (c == b'-' && b.get(i + 1) == Some(&b'-')) || c == b'#' {
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < n && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            let start = i;
            let mut j = i + 1;
            while j < n {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if b[j] == c {
                    if b.get(j + 1) == Some(&c) {
                        j += 2;
                        continue;
                    }
                    j += 1;
                    break;
                }
                j += 1;
            }
            let end = j.min(n);
            out.push(Tok {
                kind: if c == b'\'' { Kind::Str } else { Kind::Quoted },
                start,
                end,
            });
            i = end;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$') {
                i += 1;
            }
            out.push(Tok {
                kind: Kind::Word,
                start,
                end: i,
            });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
            out.push(Tok {
                kind: Kind::Other,
                start,
                end: i,
            });
            continue;
        }
        if c.is_ascii() {
            out.push(Tok {
                kind: Kind::Punct(c),
                start: i,
                end: i + 1,
            });
            i += 1;
        } else {
            // A non-ASCII run (an unquoted unicode identifier is not valid
            // ClickHouse anyway) — one opaque token.
            let start = i;
            while i < n && !b[i].is_ascii() {
                i += 1;
            }
            out.push(Tok {
                kind: Kind::Other,
                start,
                end: i,
            });
        }
    }
    out
}

/// What [`analyze`] decided for one statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdlRewrite {
    /// Insert ` ON CLUSTER <c>` at byte offset `at`; `object` describes the
    /// statement for the preview (`CREATE TABLE db.events`).
    Inject { at: usize, object: String },
    /// DDL that already names a cluster — left as written.
    AlreadyOnCluster { object: String },
    /// DDL Otto does not rewrite (TEMPORARY objects, users/roles/…).
    Unsupported { reason: String },
    /// Not DDL (SELECT / INSERT / …) — never rewritten.
    NotDdl,
}

struct Cursor<'a> {
    s: &'a str,
    t: Vec<Tok>,
}

impl Cursor<'_> {
    fn word(&self, i: usize) -> Option<String> {
        let t = self.t.get(i)?;
        (t.kind == Kind::Word).then(|| self.s[t.start..t.end].to_ascii_uppercase())
    }
    fn is(&self, i: usize, w: &str) -> bool {
        self.word(i).as_deref() == Some(w)
    }
    fn punct(&self, i: usize, p: u8) -> bool {
        self.t.get(i).map(|t| t.kind) == Some(Kind::Punct(p))
    }
    /// A (possibly `db.`-qualified) object name at `i`: returns the index past
    /// it and its byte end.
    fn name(&self, i: usize) -> Option<(usize, usize)> {
        let first = self.t.get(i)?;
        if !matches!(first.kind, Kind::Word | Kind::Quoted) {
            return None;
        }
        // `ON` here would mean the name is missing (`DROP TABLE ON CLUSTER`).
        if self.is(i, "ON") {
            return None;
        }
        let mut j = i + 1;
        let mut end = first.end;
        if self.punct(j, b'.') {
            if let Some(second) = self.t.get(j + 1) {
                if matches!(second.kind, Kind::Word | Kind::Quoted) {
                    end = second.end;
                    j += 2;
                }
            }
        }
        Some((j, end))
    }
    fn skip_if(&self, mut i: usize) -> usize {
        // `IF [NOT] EXISTS` / `IF EMPTY`, possibly repeated (`DROP TABLE IF
        // EXISTS … IF EMPTY` is spelled either way round).
        while self.is(i, "IF") {
            if self.is(i + 1, "NOT") && self.is(i + 2, "EXISTS") {
                i += 3;
            } else if self.is(i + 1, "EXISTS") || self.is(i + 1, "EMPTY") {
                i += 2;
            } else {
                break;
            }
        }
        i
    }
    fn text(&self, from: usize, to_byte: usize) -> String {
        let start = self.t.get(from).map(|t| t.start).unwrap_or(0);
        self.s[start..to_byte]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn has_on_cluster(&self) -> bool {
        (0..self.t.len()).any(|i| self.is(i, "ON") && self.is(i + 1, "CLUSTER"))
    }
}

/// Decide whether and where `ON CLUSTER` goes in ONE statement.
pub fn analyze(stmt: &str) -> DdlRewrite {
    let c = Cursor {
        s: stmt,
        t: tokenize(stmt),
    };
    let Some(first) = c.word(0) else {
        return DdlRewrite::NotDdl;
    };
    if !matches!(
        first.as_str(),
        "CREATE"
            | "ALTER"
            | "DROP"
            | "TRUNCATE"
            | "RENAME"
            | "EXCHANGE"
            | "ATTACH"
            | "DETACH"
            | "OPTIMIZE"
    ) {
        return DdlRewrite::NotDdl;
    }
    let unsupported = |what: &str| DdlRewrite::Unsupported {
        reason: format!("{what} is not rewritten"),
    };
    // Where the object name starts, and which object kind it is.
    let mut i = 1;
    let kind: String;
    match first.as_str() {
        "CREATE" => {
            if c.is(i, "OR") && c.is(i + 1, "REPLACE") {
                i += 2;
            }
            if c.is(i, "TEMPORARY") {
                return DdlRewrite::Unsupported {
                    reason: "a TEMPORARY table is session-local — not rewritten".into(),
                };
            }
            match c.word(i).as_deref() {
                Some(k @ ("TABLE" | "DATABASE" | "DICTIONARY" | "VIEW" | "FUNCTION")) => {
                    kind = k.to_string();
                    i += 1;
                }
                Some(k @ ("MATERIALIZED" | "LIVE" | "WINDOW")) if c.is(i + 1, "VIEW") => {
                    kind = format!("{k} VIEW");
                    i += 2;
                }
                other => {
                    return unsupported(&format!("CREATE {}", other.unwrap_or_default()));
                }
            }
        }
        "ALTER" => match c.word(i).as_deref() {
            Some(k @ ("TABLE" | "DATABASE")) => {
                kind = k.to_string();
                i += 1;
            }
            Some("TEMPORARY") => {
                return DdlRewrite::Unsupported {
                    reason: "a TEMPORARY table is session-local — not rewritten".into(),
                }
            }
            other => return unsupported(&format!("ALTER {}", other.unwrap_or_default())),
        },
        "DROP" | "DETACH" | "ATTACH" => {
            if c.is(i, "TEMPORARY") {
                return DdlRewrite::Unsupported {
                    reason: "a TEMPORARY table is session-local — not rewritten".into(),
                };
            }
            match c.word(i).as_deref() {
                Some(k @ ("TABLE" | "DATABASE" | "DICTIONARY" | "VIEW" | "FUNCTION")) => {
                    kind = k.to_string();
                    i += 1;
                }
                Some("MATERIALIZED") if c.is(i + 1, "VIEW") => {
                    kind = "MATERIALIZED VIEW".into();
                    i += 2;
                }
                other => return unsupported(&format!("{first} {}", other.unwrap_or_default())),
            }
        }
        "TRUNCATE" => {
            if c.is(i, "TEMPORARY") {
                return DdlRewrite::Unsupported {
                    reason: "a TEMPORARY table is session-local — not rewritten".into(),
                };
            }
            if c.is(i, "ALL") {
                return unsupported("TRUNCATE ALL TABLES");
            }
            kind = if c.is(i, "DATABASE") {
                i += 1;
                "DATABASE".into()
            } else {
                if c.is(i, "TABLE") {
                    i += 1;
                }
                "TABLE".into()
            };
        }
        "RENAME" | "EXCHANGE" => {
            match c.word(i).as_deref() {
                Some(k @ ("TABLE" | "TABLES" | "DATABASE" | "DICTIONARY" | "DICTIONARIES")) => {
                    kind = k.to_string()
                }
                other => return unsupported(&format!("{first} {}", other.unwrap_or_default())),
            }
            let object = c.text(0, c.t.last().map(|t| t.end).unwrap_or(0));
            if c.has_on_cluster() {
                return DdlRewrite::AlreadyOnCluster { object };
            }
            // `ON CLUSTER` trails the whole `a TO b[, c TO d]` / `a AND b` list.
            let Some(last) = c.t.last() else {
                return DdlRewrite::NotDdl;
            };
            if c.t.len() < 3 {
                return unsupported(&format!("{first} {kind} without a name"));
            }
            return DdlRewrite::Inject {
                at: last.end,
                object: clip_object(object),
            };
        }
        "OPTIMIZE" => {
            if !c.is(i, "TABLE") {
                return unsupported("OPTIMIZE without TABLE");
            }
            kind = "TABLE".into();
            i += 1;
        }
        _ => return DdlRewrite::NotDdl,
    }
    i = c.skip_if(i);
    let Some((mut next, mut at)) = c.name(i) else {
        return unsupported(&format!("{first} {kind} without a recognisable name"));
    };
    // `DROP TABLE a, b` drops a list; `ON CLUSTER` follows the last name.
    if first == "DROP" {
        while c.punct(next, b',') {
            match c.name(next + 1) {
                Some((n2, e2)) => {
                    next = n2;
                    at = e2;
                }
                None => break,
            }
        }
    }
    // `CREATE TABLE t UUID '…' ON CLUSTER …`.
    if first == "CREATE"
        && kind == "TABLE"
        && c.is(next, "UUID")
        && c.t.get(next + 1).map(|t| t.kind) == Some(Kind::Str)
    {
        at = c.t[next + 1].end;
    }
    let object = clip_object(c.text(0, at));
    if c.has_on_cluster() {
        return DdlRewrite::AlreadyOnCluster { object };
    }
    DdlRewrite::Inject { at, object }
}

fn clip_object(s: String) -> String {
    super::params::clip(&s, 120)
}

/// `ON CLUSTER` operand: a bare identifier when it is one, else a quoted
/// string literal (ClickHouse accepts both — and `'{cluster}'` is the macro
/// form).
pub fn cluster_literal(name: &str) -> String {
    let b = name.as_bytes();
    let bare = !b.is_empty()
        && (b[0].is_ascii_alphabetic() || b[0] == b'_')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_');
    if bare {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

/// A cluster name a user may supply: printable, no quotes/semicolons, bounded.
pub fn valid_cluster_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'{' | b'}'))
}

/// The outcome of rewriting a whole (possibly multi-statement) script.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptRewrite {
    pub statement: String,
    /// One entry per statement that got `ON CLUSTER` (its object description).
    pub injected: Vec<String>,
    /// DDL statements deliberately left alone, with the reason.
    pub skipped: Vec<String>,
}

/// Inject `ON CLUSTER <cluster>` into every recognised DDL statement of
/// `script` (statement boundaries from the shared ClickHouse splitter), byte-
/// exact everywhere else — comments, spacing and DML are untouched.
pub fn rewrite_script(script: &str, cluster: &str) -> ScriptRewrite {
    let lit = cluster_literal(cluster);
    let mut inserts: Vec<usize> = Vec::new();
    let mut out = ScriptRewrite::default();
    for span in split_statements(script, SqlDialect::Clickhouse) {
        match analyze(&span.text) {
            DdlRewrite::Inject { at, object } => {
                inserts.push(span.start + at);
                out.injected.push(object);
            }
            DdlRewrite::AlreadyOnCluster { object } => {
                out.skipped.push(format!("{object}: already ON CLUSTER"));
            }
            DdlRewrite::Unsupported { reason } => out.skipped.push(reason),
            DdlRewrite::NotDdl => {}
        }
    }
    let mut s = script.to_string();
    for at in inserts.into_iter().rev() {
        s.insert_str(at, &format!(" ON CLUSTER {lit}"));
    }
    out.statement = s;
    out
}

/// Whether any statement of `script` is DDL that WOULD get `ON CLUSTER` —
/// the planner probes a target's cluster topology only then.
pub fn has_injectable_ddl(script: &str) -> bool {
    split_statements(script, SqlDialect::Clickhouse)
        .iter()
        .any(|s| matches!(analyze(&s.text), DdlRewrite::Inject { .. }))
}

/// One `system.clusters` cluster, aggregated over its hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterRow {
    pub name: String,
    pub hosts: u64,
    /// Hosts `is_local` (this server).
    pub local_hosts: u64,
    /// Hosts that are not loopback (`127.*`, `::1`, `localhost`).
    pub remote_hosts: u64,
}

/// Where a target's cluster decision came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterSource {
    /// The `{cluster}` macro, confirmed by `system.clusters`.
    Macro,
    /// The single multi-host cluster in `system.clusters`.
    SystemClusters,
    /// The database is `Replicated` / `Shared`: DDL replicates by itself.
    ReplicatedDatabase,
    /// Several real clusters — the user must pick one.
    Ambiguous,
    /// A single-node server (no real cluster).
    NotCluster,
    /// The topology probe failed (permissions, …); see the note.
    ProbeFailed,
    /// The script has no DDL to rewrite, so nothing was probed.
    NotNeeded,
}

/// Detection result for one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterDetection {
    pub cluster: Option<String>,
    pub source: ClusterSource,
    pub candidates: Vec<String>,
    pub note: Option<String>,
}

/// Pure decision over the probe results (see the module docs).
pub fn decide(
    macro_cluster: Option<&str>,
    rows: &[ClusterRow],
    database_engine: Option<&str>,
) -> ClusterDetection {
    if let Some(engine) = database_engine {
        if engine.starts_with("Replicated") || engine.starts_with("Shared") {
            return ClusterDetection {
                cluster: None,
                source: ClusterSource::ReplicatedDatabase,
                candidates: Vec::new(),
                note: Some(format!(
                    "database engine {engine} replicates DDL itself — ON CLUSTER is not used \
                     (ClickHouse rejects it there)"
                )),
            };
        }
    }
    let real: Vec<&ClusterRow> = rows
        .iter()
        .filter(|r| r.hosts > 1 && r.remote_hosts > 0)
        .collect();
    let mut note = None;
    if let Some(m) = macro_cluster.map(str::trim).filter(|m| !m.is_empty()) {
        if rows.iter().any(|r| r.name == m) {
            return ClusterDetection {
                cluster: Some(m.to_string()),
                source: ClusterSource::Macro,
                candidates: real.iter().map(|r| r.name.clone()).collect(),
                note: None,
            };
        }
        note = Some(format!(
            "the {{cluster}} macro is `{m}` but system.clusters does not list it"
        ));
    }
    let local: Vec<&&ClusterRow> = real.iter().filter(|r| r.local_hosts > 0).collect();
    let pool: Vec<String> = if local.is_empty() {
        real.iter().map(|r| r.name.clone()).collect()
    } else {
        local.iter().map(|r| r.name.clone()).collect()
    };
    match pool.len() {
        0 => ClusterDetection {
            cluster: None,
            source: ClusterSource::NotCluster,
            candidates: Vec::new(),
            note: note.or_else(|| Some("single-node server — no multi-host cluster".into())),
        },
        1 => ClusterDetection {
            cluster: Some(pool[0].clone()),
            source: ClusterSource::SystemClusters,
            candidates: pool,
            note,
        },
        _ => ClusterDetection {
            cluster: None,
            source: ClusterSource::Ambiguous,
            note: Some(format!(
                "{} clusters found ({}) — choose one",
                pool.len(),
                pool.join(", ")
            )),
            candidates: pool,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inj(stmt: &str) -> String {
        rewrite_script(stmt, "main").statement
    }

    #[test]
    fn create_variants() {
        assert_eq!(
            inj("CREATE TABLE db.t (a Int8) ENGINE = MergeTree ORDER BY a"),
            "CREATE TABLE db.t ON CLUSTER main (a Int8) ENGINE = MergeTree ORDER BY a"
        );
        assert_eq!(
            inj("create table if not exists `my db`.`t` (a Int8) engine=Memory"),
            "create table if not exists `my db`.`t` ON CLUSTER main (a Int8) engine=Memory"
        );
        assert_eq!(
            inj("CREATE OR REPLACE VIEW v AS SELECT 1"),
            "CREATE OR REPLACE VIEW v ON CLUSTER main AS SELECT 1"
        );
        assert_eq!(
            inj("CREATE MATERIALIZED VIEW IF NOT EXISTS db.mv TO db.t AS SELECT 1"),
            "CREATE MATERIALIZED VIEW IF NOT EXISTS db.mv ON CLUSTER main TO db.t AS SELECT 1"
        );
        assert_eq!(
            inj("CREATE DATABASE analytics ENGINE = Atomic"),
            "CREATE DATABASE analytics ON CLUSTER main ENGINE = Atomic"
        );
        assert_eq!(
            inj("CREATE FUNCTION f AS (x) -> x + 1"),
            "CREATE FUNCTION f ON CLUSTER main AS (x) -> x + 1"
        );
        assert_eq!(
            inj("CREATE TABLE t UUID '123e4567-e89b-12d3-a456-426614174000' (a Int8) ENGINE=Memory"),
            "CREATE TABLE t UUID '123e4567-e89b-12d3-a456-426614174000' ON CLUSTER main (a Int8) ENGINE=Memory"
        );
    }

    #[test]
    fn alter_drop_truncate_rename() {
        assert_eq!(
            inj("ALTER TABLE db.t ADD COLUMN b String"),
            "ALTER TABLE db.t ON CLUSTER main ADD COLUMN b String"
        );
        assert_eq!(
            inj("ALTER TABLE t DELETE WHERE a = 1"),
            "ALTER TABLE t ON CLUSTER main DELETE WHERE a = 1"
        );
        assert_eq!(
            inj("DROP TABLE IF EXISTS db.t SYNC"),
            "DROP TABLE IF EXISTS db.t ON CLUSTER main SYNC"
        );
        assert_eq!(
            inj("DROP TABLE a, db.b"),
            "DROP TABLE a, db.b ON CLUSTER main"
        );
        assert_eq!(
            inj("TRUNCATE TABLE db.t"),
            "TRUNCATE TABLE db.t ON CLUSTER main"
        );
        assert_eq!(inj("truncate t"), "truncate t ON CLUSTER main");
        assert_eq!(
            inj("RENAME TABLE a TO b, c TO d"),
            "RENAME TABLE a TO b, c TO d ON CLUSTER main"
        );
        assert_eq!(
            inj("EXCHANGE TABLES a AND b"),
            "EXCHANGE TABLES a AND b ON CLUSTER main"
        );
        assert_eq!(
            inj("OPTIMIZE TABLE db.t FINAL"),
            "OPTIMIZE TABLE db.t ON CLUSTER main FINAL"
        );
        assert_eq!(inj("DETACH TABLE t"), "DETACH TABLE t ON CLUSTER main");
    }

    #[test]
    fn dml_and_reads_are_never_rewritten() {
        for s in [
            "SELECT * FROM t",
            "INSERT INTO t VALUES (1)",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "DELETE FROM t WHERE a = 1",
            "SHOW CREATE TABLE t",
            "UPDATE t SET a = 1",
        ] {
            assert_eq!(inj(s), s, "{s}");
            assert_eq!(analyze(s), DdlRewrite::NotDdl, "{s}");
        }
    }

    #[test]
    fn existing_on_cluster_temporary_and_unknown_forms_are_skipped() {
        let s = "CREATE TABLE t ON CLUSTER other (a Int8) ENGINE=Memory";
        let r = rewrite_script(s, "main");
        assert_eq!(r.statement, s);
        assert!(r.injected.is_empty());
        assert_eq!(r.skipped.len(), 1);
        assert!(r.skipped[0].contains("already ON CLUSTER"));

        let s = "CREATE TEMPORARY TABLE t (a Int8)";
        assert_eq!(inj(s), s);
        assert!(matches!(analyze(s), DdlRewrite::Unsupported { .. }));

        let s = "CREATE USER bob IDENTIFIED BY 'x'";
        assert_eq!(inj(s), s);
        assert!(matches!(analyze(s), DdlRewrite::Unsupported { .. }));
    }

    #[test]
    fn keywords_in_strings_and_comments_do_not_confuse_it() {
        // `ON CLUSTER` only inside a comment / string literal: still injected.
        let s = "-- ON CLUSTER x\nALTER TABLE t COMMENT COLUMN a 'on cluster y'";
        assert_eq!(
            inj(s),
            "-- ON CLUSTER x\nALTER TABLE t ON CLUSTER main COMMENT COLUMN a 'on cluster y'"
        );
    }

    #[test]
    fn multi_statement_scripts_rewrite_only_ddl_and_keep_the_rest_byte_exact() {
        let s = "CREATE TABLE a (x Int8) ENGINE=Memory;\n-- seed\nINSERT INTO a VALUES (1);\nALTER TABLE a ADD COLUMN y Int8;\nSELECT * FROM a;";
        let r = rewrite_script(s, "main");
        assert_eq!(
            r.statement,
            "CREATE TABLE a ON CLUSTER main (x Int8) ENGINE=Memory;\n-- seed\nINSERT INTO a VALUES (1);\nALTER TABLE a ON CLUSTER main ADD COLUMN y Int8;\nSELECT * FROM a;"
        );
        assert_eq!(r.injected, vec!["CREATE TABLE a", "ALTER TABLE a"]);
        assert!(has_injectable_ddl(s));
        assert!(!has_injectable_ddl("SELECT 1; INSERT INTO a VALUES (1)"));
    }

    #[test]
    fn cluster_literals() {
        assert_eq!(cluster_literal("main_cluster"), "main_cluster");
        assert_eq!(cluster_literal("prod-ch"), "'prod-ch'");
        assert_eq!(cluster_literal("{cluster}"), "'{cluster}'");
        assert!(valid_cluster_name("prod-ch.1"));
        assert!(valid_cluster_name("{cluster}"));
        assert!(!valid_cluster_name("x; DROP"));
        assert!(!valid_cluster_name("a'b"));
        assert!(!valid_cluster_name(""));
        assert_eq!(
            inj("ALTER TABLE t DROP COLUMN a").replace("main", "x"),
            "ALTER TABLE t ON CLUSTER x DROP COLUMN a"
        );
        assert_eq!(
            rewrite_script("DROP TABLE t", "prod-ch").statement,
            "DROP TABLE t ON CLUSTER 'prod-ch'"
        );
    }

    fn row(name: &str, hosts: u64, local: u64, remote: u64) -> ClusterRow {
        ClusterRow {
            name: name.into(),
            hosts,
            local_hosts: local,
            remote_hosts: remote,
        }
    }

    #[test]
    fn stock_single_node_configs_are_not_clusters() {
        // A fresh server's default config: `default` (one localhost host) and
        // loopback-only test clusters.
        let rows = [
            row("default", 1, 1, 0),
            row("test_cluster_two_shards", 2, 2, 0),
            row("test_shard_localhost", 1, 1, 0),
            row("parallel_replicas", 1, 1, 0),
        ];
        let d = decide(None, &rows, Some("Atomic"));
        assert_eq!(d.source, ClusterSource::NotCluster);
        assert_eq!(d.cluster, None);
    }

    #[test]
    fn a_single_real_cluster_is_detected() {
        let rows = [row("default", 1, 1, 0), row("analytics", 6, 1, 6)];
        let d = decide(None, &rows, Some("Atomic"));
        assert_eq!(d.source, ClusterSource::SystemClusters);
        assert_eq!(d.cluster.as_deref(), Some("analytics"));
    }

    #[test]
    fn the_cluster_macro_wins_when_listed() {
        let rows = [row("a", 3, 1, 3), row("b", 3, 1, 3)];
        let d = decide(Some("b"), &rows, None);
        assert_eq!(d.source, ClusterSource::Macro);
        assert_eq!(d.cluster.as_deref(), Some("b"));
        // An unlisted macro is ignored (with a note) — ON CLUSTER with an
        // unknown name fails on the server.
        let d = decide(Some("ghost"), &rows, None);
        assert_eq!(d.source, ClusterSource::Ambiguous);
        assert_eq!(d.cluster, None);
        assert_eq!(d.candidates, vec!["a", "b"]);
    }

    #[test]
    fn clusters_containing_this_node_are_preferred() {
        let rows = [row("mine", 4, 1, 4), row("remote_only", 4, 0, 4)];
        let d = decide(None, &rows, None);
        assert_eq!(d.cluster.as_deref(), Some("mine"));
    }

    #[test]
    fn replicated_databases_never_get_on_cluster() {
        let rows = [row("analytics", 6, 1, 6)];
        for engine in ["Replicated", "Shared"] {
            let d = decide(Some("analytics"), &rows, Some(engine));
            assert_eq!(d.source, ClusterSource::ReplicatedDatabase, "{engine}");
            assert_eq!(d.cluster, None);
        }
    }
}
