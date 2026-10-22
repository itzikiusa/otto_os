//! SQL statement classification — the ONE comment-aware first-keyword scanner
//! and the per-engine "is this a read?" keyword table.
//!
//! Before this module the scanner existed three times (mysql, postgres and the
//! engine-agnostic write gate in `types.rs`) and the read rule four times, each
//! with its own keyword list, so a fix (a new read keyword, a comment form) had
//! to be applied up to four times and the guardrail and the execution path could
//! drift apart silently (architecture-2-02). The lists below are the SAME lists
//! those copies carried — this module changes where they live, not what they
//! decide; `tests::matches_the_four_legacy_copies` pins that.
//!
//! Two questions are asked of a statement, and they are deliberately different:
//!
//! - **Run path** ([`ReadClass::of`]): does the DRIVER take its row-returning
//!   read path (fetch rows, auto-LIMIT) or its execute path? A miss here costs a
//!   wrong result shape, never a safety bypass — the guardrail runs first.
//! - **Write gate** ([`gate_keyword_is_read`], [`GATE_PROVEN_READS`],
//!   [`gate_engine_extra_read`]): may a guarded (prod / read-only) connection run
//!   it without `confirm_write`? Conservative: `SET` is never a read, and the
//!   forms that CAN hide a write (`WITH … DELETE`, `SELECT … INTO`, `EXPLAIN
//!   ANALYZE …`) must also pass `access::read_is_provable` on MySQL / Postgres.
//!
//! Known, deliberate divergence kept from before: Postgres' run path treats
//! `WITH` as a read unconditionally (a data-modifying CTE still returns rows, so
//! the read path is the right shape for it), while the gate treats it as a read
//! only when provable.

use crate::types::Engine;

/// Strip leading whitespace and SQL line (`--`) / block (`/* */`) comments,
/// returning the remainder.
pub(crate) fn strip_leading_comments(sql: &str) -> &str {
    let mut s = sql.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            // Line comment: skip to end of line.
            s = rest
                .split_once('\n')
                .map(|x| x.1)
                .unwrap_or("")
                .trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            // Block comment: skip to its close.
            s = rest
                .split_once("*/")
                .map(|x| x.1)
                .unwrap_or("")
                .trim_start();
        } else {
            break;
        }
    }
    s
}

/// The first SQL keyword (uppercased; ASCII letters only), skipping leading
/// whitespace and `--` / `/* */` comments. Empty when the statement does not
/// start with a letter (a `(SELECT …)`, a blank / comment-only string).
pub(crate) fn first_keyword(sql: &str) -> String {
    strip_leading_comments(sql)
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase()
}

/// One engine's run-path keyword lists.
struct RunPathKeywords {
    engine: Engine,
    /// First keywords that always take the row-returning read path.
    reads: &'static [&'static str],
    /// First keywords that take it only when `access::read_is_provable`
    /// proves the whole statement a read.
    provable_reads: &'static [&'static str],
}

/// The per-engine run-path table. ClickHouse matches its list against the
/// first sqlparser TOKEN (skipping whitespace and `(`), not the scanner above —
/// see [`ReadClass::of`]. Redis / MongoDB have no SQL run path.
const RUN_PATH: &[RunPathKeywords] = &[
    RunPathKeywords {
        engine: Engine::Mysql,
        reads: &["SELECT", "SHOW", "DESC", "DESCRIBE", "EXPLAIN"],
        provable_reads: &["WITH"],
    },
    RunPathKeywords {
        engine: Engine::Postgres,
        reads: &["SELECT", "WITH", "SHOW", "EXPLAIN", "TABLE", "VALUES"],
        provable_reads: &[],
    },
    RunPathKeywords {
        engine: Engine::Clickhouse,
        reads: &[
            "SELECT", "SHOW", "DESC", "DESCRIBE", "EXPLAIN", "WITH", "EXISTS", "CHECK", "KILL",
            "BACKUP", "RESTORE",
        ],
        provable_reads: &[],
    },
];

/// Write gate: first keywords read as a read on every engine (the
/// engine-agnostic list the guardrail always used). `SET` is deliberately NOT
/// here: `SET GLOBAL …` / `SET PASSWORD …` mutate server/session state.
pub(crate) const GATE_READS: &[&str] = &[
    "SELECT", "SHOW", "DESC", "DESCRIBE", "EXPLAIN", "WITH", "USE",
];

/// Write gate, MySQL / Postgres: gate reads that can still hide a write
/// (`WITH x AS (DELETE …)`, `SELECT … INTO`, `EXPLAIN ANALYZE <write>`; DESC /
/// DESCRIBE are MySQL synonyms of EXPLAIN) and so must also pass
/// `access::read_is_provable`.
pub(crate) const GATE_PROVEN_READS: &[&str] = &["SELECT", "WITH", "EXPLAIN", "DESC", "DESCRIBE"];

/// Write gate: per-engine extra reads on top of [`GATE_READS`] — ClickHouse's
/// native `EXISTS` / `CHECK TABLE` forms (sqlparser coverage there is partial).
const GATE_ENGINE_EXTRA_READS: &[(Engine, &[&str])] = &[(Engine::Clickhouse, &["EXISTS", "CHECK"])];

/// True when `sql`'s first keyword is a write-gate read on every engine.
pub(crate) fn gate_keyword_is_read(sql: &str) -> bool {
    GATE_READS.contains(&first_keyword(sql).as_str())
}

/// True when `sql`'s first keyword is an engine-specific write-gate read.
pub(crate) fn gate_engine_extra_read(engine: Engine, sql: &str) -> bool {
    let kw = first_keyword(sql);
    GATE_ENGINE_EXTRA_READS
        .iter()
        .any(|(e, kws)| *e == engine && kws.contains(&kw.as_str()))
}

/// How a statement's leading keyword classifies it on a driver's run path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadClass {
    /// A row-returning read by its first keyword alone.
    Read,
    /// A read because `access::read_is_provable` proved it (MySQL `WITH`).
    ProvableRead,
    /// Anything else: a write / DDL / session statement — the execute path.
    Write,
    /// Nothing to classify: no leading keyword, or ClickHouse's tokenizer
    /// rejected the text. Takes the execute path, like a write.
    Unknown,
}

impl ReadClass {
    /// Classify `sql` for `engine`'s run path.
    pub(crate) fn of(engine: Engine, sql: &str) -> ReadClass {
        let Some(row) = RUN_PATH.iter().find(|r| r.engine == engine) else {
            return ReadClass::Unknown;
        };
        let kw = if engine == Engine::Clickhouse {
            match clickhouse_first_word(sql) {
                Some(w) => w,
                None => return ReadClass::Unknown,
            }
        } else {
            first_keyword(sql)
        };
        if kw.is_empty() {
            ReadClass::Unknown
        } else if row.reads.contains(&kw.as_str()) {
            ReadClass::Read
        } else if row.provable_reads.contains(&kw.as_str())
            && crate::access::read_is_provable(engine, sql)
        {
            ReadClass::ProvableRead
        } else {
            ReadClass::Write
        }
    }

    /// Does the driver take its row-returning read path?
    pub(crate) fn returns_rows(self) -> bool {
        matches!(self, ReadClass::Read | ReadClass::ProvableRead)
    }
}

/// ClickHouse: the first unquoted word token (uppercased), skipping whitespace
/// and `(` — comments are whitespace tokens to sqlparser. `None` when the text
/// doesn't tokenize; an empty string when the first token isn't such a word.
fn clickhouse_first_word(sql: &str) -> Option<String> {
    use sqlparser::{
        dialect::ClickHouseDialect,
        tokenizer::{Token, Tokenizer},
    };
    let tokens = Tokenizer::new(&ClickHouseDialect {}, sql).tokenize().ok()?;
    let first = tokens
        .iter()
        .find(|t| !matches!(t, Token::Whitespace(_) | Token::LParen));
    Some(match first {
        Some(Token::Word(w)) if w.quote_style.is_none() => w.value.to_ascii_uppercase(),
        _ => String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── The four pre-consolidation copies, verbatim, as parity oracles ────────

    fn legacy_first_keyword(statement: &str) -> String {
        let mut s = statement.trim_start();
        loop {
            if let Some(rest) = s.strip_prefix("--") {
                s = rest
                    .split_once('\n')
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim_start();
            } else if let Some(rest) = s.strip_prefix("/*") {
                s = rest
                    .split_once("*/")
                    .map(|x| x.1)
                    .unwrap_or("")
                    .trim_start();
            } else {
                break;
            }
        }
        s.chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect::<String>()
            .to_ascii_uppercase()
    }

    /// `drivers/mysql.rs` `is_read_statement`.
    fn legacy_mysql_is_read(statement: &str) -> bool {
        let kw = legacy_first_keyword(statement);
        matches!(
            kw.as_str(),
            "SELECT" | "SHOW" | "DESC" | "DESCRIBE" | "EXPLAIN"
        ) || (kw == "WITH" && crate::access::read_is_provable(Engine::Mysql, statement))
    }

    /// `drivers/postgres.rs` `is_read_statement`.
    fn legacy_postgres_is_read(statement: &str) -> bool {
        matches!(
            legacy_first_keyword(statement).as_str(),
            "SELECT" | "WITH" | "SHOW" | "EXPLAIN" | "TABLE" | "VALUES"
        )
    }

    /// `drivers/clickhouse.rs` `returns_rows`.
    fn legacy_clickhouse_returns_rows(sql: &str) -> bool {
        use sqlparser::{
            dialect::ClickHouseDialect,
            tokenizer::{Token, Tokenizer},
        };
        let Ok(tokens) = Tokenizer::new(&ClickHouseDialect {}, sql).tokenize() else {
            return false;
        };
        let first = tokens
            .iter()
            .find(|t| !matches!(t, Token::Whitespace(_) | Token::LParen));
        matches!(first, Some(Token::Word(w)) if w.quote_style.is_none() && matches!(
            w.value.to_ascii_uppercase().as_str(),
            "SELECT" | "SHOW" | "DESC" | "DESCRIBE" | "EXPLAIN" | "WITH" |
            "EXISTS" | "CHECK" | "KILL" | "BACKUP" | "RESTORE"
        ))
    }

    /// `types.rs` `sql_first_keyword_is_read`.
    fn legacy_gate_is_read(statement: &str) -> bool {
        matches!(
            legacy_first_keyword(statement).as_str(),
            "SELECT" | "SHOW" | "DESC" | "DESCRIBE" | "EXPLAIN" | "WITH" | "USE"
        )
    }

    const CORPUS: &[&str] = &[
        "SELECT 1",
        "  select * from t",
        "-- c\nSELECT 1",
        "/* block */ UPDATE t SET a = 1",
        "/* a */ /* b */ -- c\n  show tables",
        "SHOW TABLES",
        "DESC users",
        "DESCRIBE users",
        "EXPLAIN SELECT 1",
        "EXPLAIN ANALYZE DELETE FROM t",
        "WITH c AS (SELECT 1) SELECT * FROM c",
        "WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x",
        "WITH c AS (SELECT 1) UPDATE t SET a = 1",
        "TABLE customers",
        "VALUES (1),(2)",
        "USE shop",
        "SET GLOBAL max_connections = 10",
        "SET search_path = x",
        "INSERT INTO t VALUES (1)",
        "UPDATE t SET a = 1",
        "DELETE FROM t",
        "CREATE TABLE t (id INT)",
        "DROP TABLE t",
        "TRUNCATE t",
        "EXISTS TABLE t",
        "CHECK TABLE t",
        "KILL QUERY WHERE query_id = 'x'",
        "BACKUP TABLE t TO Disk('b', 'x')",
        "RESTORE TABLE t FROM Disk('b', 'x')",
        "OPTIMIZE TABLE t FINAL",
        "ALTER TABLE t DELETE WHERE 1",
        "(SELECT 1)",
        "((SELECT 1) UNION ALL (SELECT 2))",
        "`select` 1",
        "\"SELECT\" 1",
        "SELECT_x FROM t",
        "",
        "   ",
        "-- only a comment",
        "/* unterminated",
        "'unterminated string",
        "select1",
        "123",
    ];

    #[test]
    fn first_keyword_matches_the_three_legacy_scanners() {
        for sql in CORPUS {
            assert_eq!(first_keyword(sql), legacy_first_keyword(sql), "{sql:?}");
        }
        assert_eq!(first_keyword("-- a comment\nSELECT 1"), "SELECT");
        assert_eq!(first_keyword("/* block */ UPDATE t"), "UPDATE");
        assert_eq!(first_keyword("   \n  select"), "SELECT");
        assert_eq!(
            strip_leading_comments("-- x\n /* y */ (select 1)"),
            "(select 1)"
        );
    }

    /// The pin: every engine's run path and the write gate decide exactly what
    /// the four pre-consolidation copies decided, over the whole corpus.
    #[test]
    fn matches_the_four_legacy_copies() {
        for sql in CORPUS {
            assert_eq!(
                ReadClass::of(Engine::Mysql, sql).returns_rows(),
                legacy_mysql_is_read(sql),
                "mysql: {sql:?}"
            );
            assert_eq!(
                ReadClass::of(Engine::Postgres, sql).returns_rows(),
                legacy_postgres_is_read(sql),
                "postgres: {sql:?}"
            );
            assert_eq!(
                ReadClass::of(Engine::Clickhouse, sql).returns_rows(),
                legacy_clickhouse_returns_rows(sql),
                "clickhouse: {sql:?}"
            );
            assert_eq!(
                gate_keyword_is_read(sql),
                legacy_gate_is_read(sql),
                "gate: {sql:?}"
            );
        }
    }

    #[test]
    fn classes_are_explicit() {
        assert_eq!(ReadClass::of(Engine::Mysql, "SELECT 1"), ReadClass::Read);
        assert_eq!(
            ReadClass::of(Engine::Mysql, "WITH c AS (SELECT 1) SELECT * FROM c"),
            ReadClass::ProvableRead
        );
        assert_eq!(
            ReadClass::of(Engine::Mysql, "INSERT INTO t VALUES (1)"),
            ReadClass::Write
        );
        assert_eq!(ReadClass::of(Engine::Postgres, ""), ReadClass::Unknown);
        assert_eq!(
            ReadClass::of(Engine::Postgres, "(SELECT 1)"),
            ReadClass::Unknown
        );
        assert_eq!(
            ReadClass::of(Engine::Clickhouse, "(SELECT 1)"),
            ReadClass::Read
        );
        assert_eq!(ReadClass::of(Engine::Redis, "GET k"), ReadClass::Unknown);
        assert!(gate_engine_extra_read(Engine::Clickhouse, "EXISTS TABLE t"));
        assert!(!gate_engine_extra_read(Engine::Mysql, "EXISTS TABLE t"));
        assert!(!gate_keyword_is_read("SET GLOBAL x = 1"));
    }
}
