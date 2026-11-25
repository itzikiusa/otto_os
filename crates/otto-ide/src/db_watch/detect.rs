//! Static scanner finding the database tables, MongoDB collections and Redis
//! key prefixes a project's code touches, ranked for DB Watch.
//!
//! One literal + comment scanner serves Go, Java, TS/JS, Python and Rust. SQL
//! is read only from string literals that start with a statement keyword
//! (pieces joined by `+` or whitespace count as one literal), so comments,
//! log lines and error strings ("failed to read from cache") never yield a
//! table. ORM idioms are matched on the code with its comments blanked;
//! `.sql` files are read whole, comments stripped. The names are candidates:
//! the watch engine verifies them against the target's real objects.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ignore::WalkBuilder;
use regex::{Captures, Regex};

use super::{IdeDbEngine, IdeDetectedDbAccess, IdeDetectedDbKind};

const MAX_SCAN_FILES: usize = 300;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_DETECTED_ITEMS: usize = 60;

/// Score of the file a name is found in; an item keeps its best. Files are
/// scanned in this order too, so the file cap never hides the code path.
const SCORE_PAUSED_FILE: u32 = 100;
const SCORE_STACK_FILE: u32 = 60;
const SCORE_PAUSED_DIR: u32 = 40;
const SCORE_ACTIVE_FILE: u32 = 30;
const SCORE_REST: u32 = 10;

type DetectAcc = BTreeMap<(IdeDbEngine, String), IdeDetectedDbAccess>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    Go,
    Java,
    Js,
    Python,
    Rust,
    Sql,
}

fn lang_of(path: &Path) -> Option<Lang> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "go" => Lang::Go,
        "java" => Lang::Java,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Lang::Js,
        "py" => Lang::Python,
        "rs" => Lang::Rust,
        "sql" => Lang::Sql,
        _ => return None,
    })
}

struct DbPatterns {
    sql_string: Regex,
    sql_cte: Regex,
    sql_from: Regex,
    sql_into: Regex,
    sql_update: Regex,
    clickhouse_sql: Regex,
    postgres_sql: Regex,
    builder_table: Regex,
    jpa_table: Regex,
    gorm_table_name: Regex,
    mongo_coll: Regex,
    mongo_doc_ann: Regex,
    redis_call: Regex,
    redis_fmt_key: Regex,
}

/// `[db.]table`, each part optionally quoted: groups `n` and `n + 1`.
const SQL_NAME: &str = r#"[\x60"\[]?([A-Za-z_][A-Za-z0-9_]{1,96})[\x60"\]]?(?:\.[\x60"\[]?([A-Za-z_][A-Za-z0-9_]{1,96})[\x60"\]]?)?"#;

fn patterns() -> &'static DbPatterns {
    static PATTERNS: OnceLock<DbPatterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let re = |s: &str| Regex::new(s).expect("valid regex");
        DbPatterns {
            sql_string: re(r"'(?:[^'\\]|\\.|'')*'"),
            sql_cte: re(r"(?i)(?:\bWITH(?:\s+RECURSIVE)?|,)\s+([A-Za-z_][A-Za-z0-9_]*)\s+AS\s*\("),
            sql_from: re(&format!(r"(?i)\b(DELETE\s+)?(?:FROM|JOIN)\s+{SQL_NAME}")),
            sql_into: re(&format!(
                r"(?i)\b(?:INSERT|REPLACE|MERGE)\s+(?:IGNORE\s+)?INTO\s+{SQL_NAME}"
            )),
            sql_update: re(&format!(
                r"(?i)\bUPDATE\s+{SQL_NAME}(?:\s+(?:AS\s+)?[A-Za-z_][A-Za-z0-9_]*)?\s+SET\b"
            )),
            clickhouse_sql: re(
                r"(?i)\b(?:FINAL|PREWHERE|ENGINE\s*=|GLOBAL\s+(?:IN|JOIN))\b|\b(?:toDate|toDateTime|toStartOf[A-Za-z]+|toYYYYMM|arrayJoin|uniqExact|countIf|sumIf|groupArray)\s*\(",
            ),
            postgres_sql: re(r"\$\d|::[a-z]|(?i:\bRETURNING\b|\bILIKE\b|\bON\s+CONFLICT\b)"),
            builder_table: re(
                r#"\.(From|Into|Insert|Update|Delete|Table)\(\s*["'\x60]([A-Za-z_][A-Za-z0-9_.]{1,96})["'\x60]\s*\)"#,
            ),
            jpa_table: re(r#"@Table\s*\([^)]*\bname\s*=\s*"([A-Za-z_][A-Za-z0-9_.]{1,96})""#),
            gorm_table_name: re(
                r#"func\s*\([^)]*\)\s*TableName\(\)\s*string\s*\{\s*return\s+"([A-Za-z_][A-Za-z0-9_.]{1,96})""#,
            ),
            mongo_coll: re(
                r#"(?:\.Collection|\.collection|\.getCollection)\(\s*["'\x60]([A-Za-z_][A-Za-z0-9_.-]{1,96})["'\x60]"#,
            ),
            mongo_doc_ann: re(
                r#"@Document\s*\([^)]*\bcollection\s*=\s*"([A-Za-z_][A-Za-z0-9_.-]{1,96})""#,
            ),
            redis_call: re(
                r#"(?i)\.(Get|Set|SetNX|SetEx|MGet|HGet|HGetAll|HSet|LPush|RPush|LRange|SAdd|SMembers|ZAdd|ZRange|Del|Incr|Expire|Scan|Keys|HScan|SScan|ZScan|Exists|TTL|PTTL)\s*\(\s*(?:[A-Za-z0-9_.]+\s*,\s*){0,2}(?:match\s*=\s*)?["'\x60]([A-Za-z0-9_.-]{2,48}:[^"'\x60\s]{0,64})["'\x60]"#,
            ),
            redis_fmt_key: re(
                r#"(?:Sprintf|format!)\(\s*["'\x60]([A-Za-z0-9_.-]{2,32}(?::[A-Za-z0-9_.-]{1,32})*:)[%{}]"#,
            ),
        }
    })
}

/// Words a FROM/JOIN/INTO can be followed by that are never a table: SQL
/// keywords and table functions, and English left over from prose that
/// happens to start with a statement keyword ("Select the rows from the …").
const NOISE_WORDS: &str = "select where values dual unnest lateral only stdin stdout json_each \
    sqlite_master table index set null and or not as on join inner left right outer cross using \
    function the an this that these those it its their your our my each all any some which";

/// Catalog schemas: `information_schema.tables` is not the project's table.
const SYSTEM_SCHEMAS: &str = "information_schema pg_catalog performance_schema system sys mysql";

const STATEMENT_KEYWORDS: &[&str] = &[
    "select", "insert", "update", "delete", "replace", "with", "merge",
];

/// Whether `op` (as reported in `operations`) changes data.
pub fn is_write_op(op: &str) -> bool {
    matches!(
        op,
        "INSERT"
            | "UPDATE"
            | "DELETE"
            | "MERGE"
            | "SET"
            | "SETNX"
            | "SETEX"
            | "HSET"
            | "LPUSH"
            | "RPUSH"
            | "SADD"
            | "ZADD"
            | "DEL"
            | "INCR"
            | "EXPIRE"
    )
}

fn writes(item: &IdeDetectedDbAccess) -> bool {
    item.operations.iter().any(|op| is_write_op(op))
}

/// A string literal's body (adjacent pieces joined) and the line it starts on.
struct Literal {
    body: String,
    line: u32,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn find_seq(b: &[u8], from: usize, seq: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(seq.len())
        .position(|w| w == seq)
        .map(|p| p + from)
}

/// The literal opening at `i`: `(body_start, body_end, end, has_escapes)`, or
/// `None` when `i` opens none (a Rust lifetime, an unterminated quote).
fn literal_at(text: &str, i: usize, lang: Lang) -> Option<(usize, usize, usize, bool)> {
    let b = text.as_bytes();
    let q = b[i];
    if lang == Lang::Rust && q == b'r' && (i == 0 || !is_ident_byte(b[i - 1])) {
        let hashes = b[i + 1..].iter().take_while(|&&x| x == b'#').count();
        if b.get(i + 1 + hashes) != Some(&b'"') {
            return None;
        }
        let start = i + 2 + hashes;
        let close: Vec<u8> = std::iter::once(b'"')
            .chain(std::iter::repeat_n(b'#', hashes))
            .collect();
        let end = find_seq(b, start, &close)?;
        return Some((start, end, end + close.len(), false));
    }
    if !matches!(q, b'"' | b'\'' | b'\x60') {
        return None;
    }
    if b[i..].starts_with(&[q, q, q]) && (q == b'"' || (q == b'\'' && lang == Lang::Python)) {
        let end = find_seq(b, i + 3, &[q, q, q])?;
        return Some((i + 3, end, end + 3, true));
    }
    if q == b'\'' && !matches!(lang, Lang::Js | Lang::Python) {
        // A char / rune literal ('x', '\n', '\u{1F600}'); a Rust lifetime is none.
        let len = if b.get(i + 1) == Some(&b'\\') {
            b.get(i + 3..)?.iter().take(10).position(|&x| x == b'\'')? + 2
        } else {
            text[i + 1..].chars().next()?.len_utf8()
        };
        return (b.get(i + 1 + len) == Some(&b'\'')).then_some((
            i + 1,
            i + 1 + len,
            i + 2 + len,
            false,
        ));
    }
    let multiline = q == b'\x60' || (lang == Lang::Rust && q == b'"');
    let escapes = !(q == b'\x60' && lang == Lang::Go);
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            x if x == q => return Some((i + 1, j, j + 1, escapes)),
            b'\\' if escapes => j += 2,
            b'\n' if !multiline => return None,
            _ => j += 1,
        }
    }
    None
}

/// Splits `text` into its string literals and a copy with every comment
/// blanked (same byte offsets and newlines, so code regexes keep their lines).
fn scan_literals(text: &str, lang: Lang) -> (Vec<Literal>, String) {
    let b = text.as_bytes();
    let mut code = b.to_vec();
    let mut literals: Vec<Literal> = Vec::new();
    let mut prev_end = None;
    let mut line = 1u32;
    let mut i = 0;
    while i < b.len() {
        let slash = lang != Lang::Python;
        let comment_end = match (b[i], b.get(i + 1)) {
            (b'/', Some(b'/')) if slash && lang != Lang::Sql => {
                Some(find_seq(b, i, b"\n").unwrap_or(b.len()))
            }
            (b'/', Some(b'*')) if slash => {
                Some(find_seq(b, i + 2, b"*/").map_or(b.len(), |e| e + 2))
            }
            (b'-', Some(b'-')) if lang == Lang::Sql => {
                Some(find_seq(b, i, b"\n").unwrap_or(b.len()))
            }
            (b'#', _) if lang == Lang::Python => Some(find_seq(b, i, b"\n").unwrap_or(b.len())),
            _ => None,
        };
        if let Some(end) = comment_end {
            for byte in &mut code[i..end] {
                if *byte == b'\n' {
                    line += 1;
                } else {
                    *byte = b' ';
                }
            }
            i = end;
            continue;
        }
        let literal = if lang == Lang::Sql {
            None
        } else {
            literal_at(text, i, lang)
        };
        let Some((start, body_end, end, escapes)) = literal else {
            line += u32::from(b[i] == b'\n');
            i += 1;
            continue;
        };
        let raw = &text[start..body_end];
        let body = if escapes {
            unescape(raw)
        } else {
            raw.to_string()
        };
        let gap = prev_end.map(|p| &code[p..i]);
        match literals.last_mut() {
            Some(prev)
                if gap.is_some_and(|g| g.iter().all(|&x| x.is_ascii_whitespace() || x == b'+')) =>
            {
                let newlines = gap
                    .unwrap_or_default()
                    .iter()
                    .filter(|&&x| x == b'\n')
                    .count();
                prev.body.push_str(&if newlines == 0 {
                    " ".to_string()
                } else {
                    "\n".repeat(newlines)
                });
                prev.body.push_str(&body);
            }
            _ => literals.push(Literal { body, line }),
        }
        line += b[i..end].iter().filter(|&&x| x == b'\n').count() as u32;
        prev_end = Some(end);
        i = end;
    }
    let code = String::from_utf8(code).expect("comments start and end on ASCII bytes");
    (literals, code)
}

/// `\"` → `"`, and `\n` / `\t` / `\r` → a space so `*\nFROM t` still reads `FROM t`.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        out.push(match c {
            '\\' => match chars.next() {
                Some('n' | 't' | 'r') | None => ' ',
                Some(next) => next,
            },
            c => c,
        });
    }
    out
}

/// Each statement of a `.sql` file (comments already blanked) as a literal.
fn sql_file_statements(code: &str) -> Vec<Literal> {
    let mut out = Vec::new();
    let mut line = 1u32;
    for stmt in code.split(';') {
        let lead = stmt.len() - stmt.trim_start().len();
        out.push(Literal {
            body: stmt.to_string(),
            line: line + stmt[..lead].matches('\n').count() as u32,
        });
        line += stmt.matches('\n').count() as u32;
    }
    out
}

fn starts_with_statement(body: &str) -> bool {
    let rest = body.trim_start_matches(|c: char| c.is_whitespace() || c == '(');
    let word = rest
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default();
    STATEMENT_KEYWORDS
        .iter()
        .any(|k| word.eq_ignore_ascii_case(k))
}

fn file_sql_engine(lower: &str, default_sql: IdeDbEngine) -> IdeDbEngine {
    let has = |hints: &[&str]| hints.iter().any(|h| lower.contains(h));
    let postgres = has(&["pgx", "postgres", "lib/pq", "psycopg", "\"pg\"", "'pg'"]);
    let clickhouse = has(&["clickhouse", "chnativeconn", "mergetree"]);
    let other_sql = has(&["mysql", "sqlx", "database/sql", "multy_sql", "gorm", "jdbc"]);
    if clickhouse && !postgres && !other_sql {
        IdeDbEngine::Clickhouse
    } else if postgres {
        IdeDbEngine::Postgres
    } else {
        default_sql
    }
}

/// The file being scanned and the accumulator its hits go into.
struct HitSite<'a> {
    rel_file: &'a str,
    score: u32,
    acc: &'a mut DetectAcc,
}

impl HitSite<'_> {
    fn record(
        &mut self,
        engine: IdeDbEngine,
        kind: IdeDetectedDbKind,
        db: Option<&str>,
        name: &str,
        op: &str,
        line: u32,
    ) {
        let name = name.trim().trim_matches('\x60').trim_matches('"');
        let noise = |list: &str, word: &str| {
            list.split_whitespace()
                .any(|w| word.eq_ignore_ascii_case(w))
        };
        if name.len() < 2
            || (kind == IdeDetectedDbKind::Table
                && (noise(NOISE_WORDS, name) || db.is_some_and(|db| noise(SYSTEM_SCHEMAS, db))))
        {
            return;
        }
        let entry = self
            .acc
            .entry((engine, name.to_string()))
            .or_insert_with(|| IdeDetectedDbAccess {
                engine,
                object_name: name.to_string(),
                kind,
                operations: Vec::new(),
                database_hint: None,
                source_file: self.rel_file.to_string(),
                line,
                score: self.score,
            });
        if self.score > entry.score {
            entry.score = self.score;
            entry.source_file = self.rel_file.to_string();
            entry.line = line;
        }
        if entry.database_hint.is_none() {
            entry.database_hint = db.map(str::to_string);
        }
        if !entry.operations.iter().any(|o| o == op) {
            entry.operations.push(op.to_string());
        }
    }

    /// A SQL literal's tables: FROM/JOIN reads, INSERT/UPDATE/DELETE/MERGE writes.
    fn record_sql(&mut self, sql: &str, line: u32, file_engine: IdeDbEngine) {
        let p = patterns();
        let sql = p.sql_string.replace_all(sql, "''");
        let engine = if p.clickhouse_sql.is_match(&sql) {
            IdeDbEngine::Clickhouse
        } else if file_engine != IdeDbEngine::Clickhouse && p.postgres_sql.is_match(&sql) {
            // clickhouse-go takes $1 too, so placeholders only count outside ClickHouse code.
            IdeDbEngine::Postgres
        } else {
            file_engine
        };
        let ctes: HashSet<String> = p
            .sql_cte
            .captures_iter(&sql)
            .map(|c| c[1].to_ascii_lowercase())
            .collect();
        let mut hit = |cap: &Captures, first: usize, op: &str| {
            let (db, table) = match cap.get(first + 1) {
                Some(table) => (Some(&cap[first]), table.as_str()),
                None => (None, &cap[first]),
            };
            let at = line
                + sql[..cap.get(0).map_or(0, |m| m.start())]
                    .matches('\n')
                    .count() as u32;
            if !ctes.contains(&table.to_ascii_lowercase()) {
                self.record(engine, IdeDetectedDbKind::Table, db, table, op, at);
            }
        };
        for cap in p.sql_from.captures_iter(&sql) {
            let after = &sql[cap.get(0).map_or(0, |m| m.end())..];
            if !after.trim_start().starts_with('(') {
                hit(
                    &cap,
                    2,
                    if cap.get(1).is_some() {
                        "DELETE"
                    } else {
                        "SELECT"
                    },
                );
            }
        }
        for cap in p.sql_into.captures_iter(&sql) {
            let merge = cap[0]
                .get(..5)
                .is_some_and(|w| w.eq_ignore_ascii_case("merge"));
            hit(&cap, 1, if merge { "MERGE" } else { "INSERT" });
        }
        for cap in p.sql_update.captures_iter(&sql) {
            hit(&cap, 1, "UPDATE");
        }
    }
}

/// Scan one file's text (its language from `rel_file`'s extension) into `acc`.
fn scan_source_text(
    text: &str,
    rel_file: &str,
    score: u32,
    default_sql: IdeDbEngine,
    acc: &mut DetectAcc,
) {
    let Some(lang) = lang_of(Path::new(rel_file)) else {
        return;
    };
    let mut site = HitSite {
        rel_file,
        score,
        acc,
    };
    let lower = text.to_ascii_lowercase();
    let file_engine = file_sql_engine(&lower, default_sql);
    let (literals, code) = scan_literals(text, lang);
    let statements = if lang == Lang::Sql {
        sql_file_statements(&code)
    } else {
        literals
    };
    for lit in statements.iter().filter(|l| starts_with_statement(&l.body)) {
        site.record_sql(&lit.body, lit.line, file_engine);
    }
    if lang == Lang::Sql {
        return;
    }
    let p = patterns();
    let line_at = |offset: usize| 1 + code[..offset].matches('\n').count() as u32;
    let mut each = |re: &Regex,
                    group: usize,
                    engine: IdeDbEngine,
                    kind: IdeDetectedDbKind,
                    op: fn(&Captures) -> &'static str| {
        for cap in re.captures_iter(&code) {
            let m = cap.get(group).expect("group in pattern");
            let (db, name) = match m.as_str().split_once('.') {
                Some((db, name))
                    if kind == IdeDetectedDbKind::Table && !db.is_empty() && !name.is_empty() =>
                {
                    (Some(db), name)
                }
                _ => (None, m.as_str()),
            };
            site.record(engine, kind, db, name, op(&cap), line_at(m.start()));
        }
    };
    let (table, coll, mongo) = (
        IdeDetectedDbKind::Table,
        IdeDetectedDbKind::Collection,
        IdeDbEngine::Mongodb,
    );
    each(&p.builder_table, 2, file_engine, table, |c| match &c[1] {
        "Into" | "Insert" => "INSERT",
        "Update" => "UPDATE",
        "Delete" => "DELETE",
        _ => "SELECT",
    });
    each(&p.jpa_table, 1, file_engine, table, |_| "ENTITY");
    each(&p.gorm_table_name, 1, file_engine, table, |_| "ENTITY");
    each(&p.mongo_coll, 1, mongo, coll, |_| "COLLECTION");
    each(&p.mongo_doc_ann, 1, mongo, coll, |_| "DOCUMENT");
    if !lower.contains("redis") {
        return;
    }
    for (re, group) in [(&p.redis_call, 2), (&p.redis_fmt_key, 1)] {
        for cap in re.captures_iter(&code) {
            let m = cap.get(group).expect("group in pattern");
            if m.as_str().contains("//") {
                continue; // a URL, not a key
            }
            let op = if group == 2 {
                cap[1].to_ascii_uppercase()
            } else {
                "KEY".to_string()
            };
            let key = normalize_redis_prefix(m.as_str());
            site.record(
                IdeDbEngine::Redis,
                IdeDetectedDbKind::KeyPrefix,
                None,
                &key,
                &op,
                line_at(m.start()),
            );
        }
    }
}

fn normalize_redis_prefix(raw: &str) -> String {
    if let Some((prefix, _)) = raw.split_once('%') {
        return format!("{}*", prefix.trim_end_matches('*'));
    }
    if let Some((prefix, _)) = raw.split_once('{') {
        return format!("{}*", prefix.trim_end_matches('*'));
    }
    if raw.ends_with(':') {
        return format!("{raw}*");
    }
    raw.to_string()
}

fn is_likely_db_file(rel_lower: &str) -> bool {
    [
        "dao",
        "repo",
        "store",
        "db",
        "sql",
        "mongo",
        "redis",
        "clickhouse",
        "test",
    ]
    .iter()
    .any(|h| rel_lower.contains(h))
}

/// Scan `roots` for the database objects the code touches, ranked: the
/// paused frame's file (`stack_files[0]`; DAP order, absolute or root-relative),
/// the other stack files, the paused file's folder, the active file, the rest.
/// Ties go to writes, then to the name.
pub fn detect_project_db_access(
    primary_root: &Path,
    extra_roots: &[PathBuf],
    active_file: Option<&str>,
    stack_files: &[String],
    default_sql: IdeDbEngine,
) -> Vec<IdeDetectedDbAccess> {
    let primary = primary_root
        .canonicalize()
        .unwrap_or_else(|_| primary_root.to_path_buf());
    let mut roots = vec![primary.clone()];
    for extra in extra_roots {
        if let Ok(c) = extra.canonicalize() {
            if !roots.contains(&c) {
                roots.push(c);
            }
        }
    }
    let readable = |path: &Path| -> Option<PathBuf> {
        let canon = path.canonicalize().ok()?;
        let ok = lang_of(&canon).is_some()
            && roots.iter().any(|r| canon.starts_with(r))
            && crate::target::enforce_non_root_path(&canon).is_ok()
            && canon
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() <= MAX_FILE_BYTES);
        ok.then_some(canon)
    };
    let resolve = |file: &str| {
        let file = file.trim();
        let path = Path::new(file);
        (!file.is_empty())
            .then(|| {
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    primary.join(path)
                }
            })
            .and_then(|p| readable(&p))
    };

    let mut queue: Vec<(PathBuf, u32)> = Vec::new();
    let paused = stack_files.first().and_then(|f| resolve(f));
    queue.extend(paused.clone().map(|p| (p, SCORE_PAUSED_FILE)));
    queue.extend(
        stack_files
            .iter()
            .skip(1)
            .filter_map(|f| resolve(f))
            .map(|p| (p, SCORE_STACK_FILE)),
    );
    if let Some(dir) = paused.as_deref().and_then(Path::parent) {
        let siblings = WalkBuilder::new(dir)
            .max_depth(Some(1))
            .sort_by_file_name(|a, b| a.cmp(b))
            .build()
            .flatten()
            .filter_map(|e| readable(e.path()));
        queue.extend(siblings.map(|p| (p, SCORE_PAUSED_DIR)));
    }
    queue.extend(
        active_file
            .and_then(resolve)
            .map(|p| (p, SCORE_ACTIVE_FILE)),
    );

    // The rest: DB-looking sources, then tests, then any other source, so the
    // cap never hides the DAO.
    let (mut sources, mut tests, mut others) = (Vec::new(), Vec::new(), Vec::new());
    for root in &roots {
        let walker = WalkBuilder::new(root)
            .max_depth(Some(6))
            .hidden(true)
            .sort_by_file_name(|a, b| a.cmp(b))
            .filter_entry(|e| {
                e.depth() == 0
                    || !matches!(
                        e.file_name().to_string_lossy().as_ref(),
                        "vendor" | "node_modules" | "target" | "dist" | "build" | ".git"
                    )
            })
            .build();
        for entry in walker.flatten() {
            let path = entry.path();
            if !entry.file_type().is_some_and(|t| t.is_file())
                || lang_of(path).is_none()
                || crate::target::enforce_non_root_path(path).is_err()
                || !entry.metadata().is_ok_and(|m| m.len() <= MAX_FILE_BYTES)
            {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_ascii_lowercase();
            if !is_likely_db_file(&rel) {
                others.push(path.to_path_buf());
            } else if rel.contains("test") {
                tests.push(path.to_path_buf());
            } else {
                sources.push(path.to_path_buf());
            }
        }
    }
    queue.extend(
        sources
            .into_iter()
            .chain(tests)
            .chain(others)
            .map(|p| (p, SCORE_REST)),
    );

    let mut acc = DetectAcc::new();
    let mut seen = HashSet::new();
    let mut scanned = 0usize;
    for (path, score) in queue {
        if scanned >= MAX_SCAN_FILES {
            break;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            let rel = path
                .strip_prefix(&primary)
                .unwrap_or(&path)
                .to_string_lossy();
            scan_source_text(&text, &rel, score, default_sql, &mut acc);
            scanned += 1;
        }
    }

    let mut list: Vec<IdeDetectedDbAccess> = acc.into_values().collect();
    list.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| writes(b).cmp(&writes(a)))
            .then_with(|| {
                a.object_name
                    .to_ascii_lowercase()
                    .cmp(&b.object_name.to_ascii_lowercase())
            })
    });
    list.truncate(MAX_DETECTED_ITEMS);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/db_detect")
            .join(name)
    }

    fn detect(root: &Path, active: Option<&str>, stack: &[&str]) -> Vec<IdeDetectedDbAccess> {
        let stack: Vec<String> = stack.iter().map(|s| s.to_string()).collect();
        detect_project_db_access(root, &[], active, &stack, IdeDbEngine::Mysql)
    }

    fn names(items: &[IdeDetectedDbAccess]) -> Vec<&str> {
        let mut names: Vec<&str> = items.iter().map(|i| i.object_name.as_str()).collect();
        names.sort_unstable();
        names
    }

    fn item<'a>(items: &'a [IdeDetectedDbAccess], name: &str) -> &'a IdeDetectedDbAccess {
        items
            .iter()
            .find(|i| i.object_name == name)
            .unwrap_or_else(|| panic!("{name} not detected"))
    }

    /// 1-based line of the first line of `root/rel` containing `needle`.
    fn line_of(root: &Path, rel: &str, needle: &str) -> u32 {
        let text = std::fs::read_to_string(root.join(rel)).unwrap();
        1 + text.lines().position(|l| l.contains(needle)).unwrap() as u32
    }

    fn scan(rel_file: &str, text: &str) -> Vec<IdeDetectedDbAccess> {
        let mut acc = DetectAcc::new();
        scan_source_text(text, rel_file, SCORE_REST, IdeDbEngine::Mysql, &mut acc);
        acc.into_values().collect()
    }

    #[test]
    fn koala_service_yields_exactly_its_four_tables() {
        // The owner's screenshot: comments and log strings, mostly in
        // *_test.go files, showed as tables (GetAllConfigs, cache, client,
        // failed, users, the …) next to the four real ones.
        let root = fixture("koala_go");
        let items = detect(&root, None, &[]);
        assert_eq!(
            names(&items),
            [
                "failed_transaction",
                "player_session_info",
                "session_info",
                "session_route"
            ]
        );
        assert!(items
            .iter()
            .all(|i| i.engine == IdeDbEngine::Mysql && i.score == SCORE_REST));
        let route = item(&items, "session_route");
        assert_eq!(route.operations, ["SELECT", "UPDATE", "INSERT"]);
        // A hit inside a multi-line raw string points at its own line.
        assert_eq!(route.source_file, "db/dao/session_dao.go");
        assert_eq!(
            route.line,
            line_of(&root, "db/dao/session_dao.go", "INNER JOIN session_route")
        );
        assert_eq!(item(&items, "failed_transaction").operations, ["INSERT"]);
    }

    #[test]
    fn every_language_reads_sql_from_literals_and_orm_idioms_only() {
        let root = fixture("polyglot");
        let items = detect(&root, None, &[]);
        assert_eq!(
            names(&items),
            [
                "bonus_expired",
                "bonus_grants",
                "bonus_wallets",
                "currencies",
                "ledger_accounts",
                "ledger_entries",
                "payout_audit",
                "payouts",
                "player:bal:*",
                "player_audit",
                "player_balance",
                "player_sessions",
                "players",
                "session:token:abc",
                "session_info",
                "wallet_events",
                "wallet_limits",
                "wallets",
            ]
        );
        // Java: "…" + "…" concatenation, a text block, @Table.
        assert_eq!(item(&items, "players").operations, ["SELECT", "ENTITY"]);
        let audit = item(&items, "player_audit");
        assert_eq!(
            audit.line,
            line_of(
                &root,
                "java/PlayerRepository.java",
                "INSERT INTO player_audit"
            )
        );
        // Python: implicit concatenation, triple quotes, `//` is division.
        assert_eq!(
            item(&items, "bonus_grants").operations,
            ["INSERT", "SELECT"]
        );
        assert_eq!(item(&items, "bonus_expired").operations, ["DELETE"]);
        // Rust and TS: $1 placeholders guess Postgres; raw and multi-line strings.
        for name in [
            "ledger_entries",
            "ledger_accounts",
            "wallets",
            "wallet_events",
        ] {
            assert_eq!(item(&items, name).engine, IdeDbEngine::Postgres, "{name}");
        }
        let accounts = item(&items, "ledger_accounts");
        assert_eq!(accounts.operations, ["SELECT", "UPDATE"]);
        assert_eq!(
            accounts.line,
            line_of(&root, "rs/ledger_store.rs", "FROM ledger_accounts")
        );
        // .sql read whole: values and comments are not tables.
        assert_eq!(item(&items, "currencies").operations, ["INSERT", "UPDATE"]);
        // GORM: TableName() and a single-argument .Table(); .Update("col", v) is a column.
        assert_eq!(item(&items, "payouts").operations, ["ENTITY"]);
        assert_eq!(item(&items, "payout_audit").kind, IdeDetectedDbKind::Table);
        // Redis keys only in a file that uses Redis.
        assert_eq!(
            item(&items, "player:bal:*").kind,
            IdeDetectedDbKind::KeyPrefix
        );
        assert_eq!(item(&items, "session:token:abc").operations, ["HGETALL"]);
    }

    #[test]
    fn lifetimes_char_quotes_and_unterminated_quotes_do_not_swallow_strings() {
        let rust = "fn f<'a>(s: &'a str) -> &'a str { let c = '\"'; let e = '\\''; q(\"SELECT * FROM t_one\") }";
        assert_eq!(names(&scan("a.rs", rust)), ["t_one"]);
        let go = "r := '\\'' ; s := \"don't\" ; q := \"DELETE FROM t_two\"";
        assert_eq!(names(&scan("a.go", go)), ["t_two"]);
        let ts = "const re = /'/g;\nconst q = \"UPDATE t_three SET a = 1\";";
        assert_eq!(names(&scan("a.ts", ts)), ["t_three"]);
    }

    #[test]
    fn ctes_table_functions_and_prose_are_not_tables() {
        let go = r#"
            a := "WITH recent AS (SELECT id FROM orders) SELECT * FROM recent JOIN users u ON u.id = recent.id"
            b := "SELECT * FROM generate_series(1, 3)"
            c := "Select the rows from the warehouse"
            d := "UPDATE accounts a SET a.balance = 0"
            e := "INSERT INTO notes (body) VALUES ('copied from archive')"
            f := "SELECT *\nFROM \"quoted_t\""
        "#;
        assert_eq!(
            names(&scan("x.go", go)),
            ["accounts", "notes", "orders", "quoted_t", "users"]
        );
    }

    #[test]
    fn clickhouse_is_guessed_from_the_statement_not_the_word_final_in_code() {
        let go = r#"
            import "bitbucket.org/x/go_casino_kit/clickhouse"
            finalResult := conn.Select(ctx, "SELECT id FROM MdlGm_tblPlayers WHERE id = ?")
            rows := ch.Query(ctx, "SELECT player_id FROM events_ledger FINAL WHERE brand_id = 1")
            db := multy_sql.New()
        "#;
        let items = scan("dao/mixed_dao.go", go);
        assert_eq!(item(&items, "MdlGm_tblPlayers").engine, IdeDbEngine::Mysql);
        assert_eq!(
            item(&items, "events_ledger").engine,
            IdeDbEngine::Clickhouse
        );
        // A ClickHouse DAO with $1 placeholders, a table function, catalog tables.
        let go = r#"
            svc ClickhouseConnectionService
            a := "SELECT SUM(eur) FROM monthly WHERE brand_id = $1"
            b := "INSERT INTO FUNCTION s3(@url, 'CSVWithNames') SELECT * FROM monthly"
            c := "SELECT name FROM system.parts WHERE table = 'monthly'"
            d := "SELECT * FROM information_schema.tables"
        "#;
        let items = scan("dao/compoints_dao.go", go);
        assert_eq!(names(&items), ["monthly"]);
        assert_eq!(items[0].engine, IdeDbEngine::Clickhouse);
    }

    #[test]
    fn ranks_by_the_code_path_then_writes_then_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let files = [
            ("svc/handler.go", "q := \"SELECT * FROM orders\""),
            ("svc/helper.go", "q := \"INSERT INTO order_items (id) VALUES (?)\""),
            ("dao/payment_dao.go", "q := \"UPDATE payments SET paid = 1\""),
            ("web/page.ts", "const q = 'SELECT * FROM ui_prefs';"),
            ("other/report_dao.go", "a := \"SELECT * FROM reports\"\nb := \"INSERT INTO audit_log (x) VALUES (1)\"\nc := \"SELECT * FROM orders\""),
        ];
        for (rel, text) in files {
            std::fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
            std::fs::write(root.join(rel), text).unwrap();
        }
        let stack = [
            root.join("svc/handler.go").to_string_lossy().to_string(),
            "dao/payment_dao.go".into(),
        ];
        let stack: Vec<&str> = stack.iter().map(String::as_str).collect();
        let items = detect(root, Some("web/page.ts"), &stack);
        let ranked: Vec<(&str, u32)> = items
            .iter()
            .map(|i| (i.object_name.as_str(), i.score))
            .collect();
        assert_eq!(
            ranked,
            [
                ("orders", 100),
                ("payments", 60),
                ("order_items", 40),
                ("ui_prefs", 30),
                ("audit_log", 10),
                ("reports", 10)
            ]
        );
        // The best hit's location wins, though the walk meets orders again.
        assert_eq!(item(&items, "orders").source_file, "svc/handler.go");
        // Without a debug session the active file still leads.
        let items = detect(root, Some("web/page.ts"), &[]);
        assert_eq!(items[0].object_name, "ui_prefs");
    }

    #[test]
    fn the_file_cap_cannot_hide_the_dao_behind_tests() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("test")).unwrap();
        for n in 0..MAX_SCAN_FILES + 10 {
            let text = "q := \"INSERT INTO seed_table (id) VALUES (1)\"";
            std::fs::write(root.join(format!("test/case_{n:03}_test.go")), text).unwrap();
        }
        std::fs::create_dir_all(root.join("zz/dao")).unwrap();
        std::fs::write(
            root.join("zz/dao/real_dao.go"),
            "q := \"UPDATE real_table SET x = 1\"",
        )
        .unwrap();
        assert_eq!(
            names(&detect(root, None, &[])),
            ["real_table", "seed_table"]
        );
    }
}
