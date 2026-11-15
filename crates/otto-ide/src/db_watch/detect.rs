//! Static source & configuration scanner detecting database tables, MongoDB
//! collections, and Redis key prefixes across Go, Java, Rust, TypeScript,
//! Python, and SQL files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ignore::WalkBuilder;
use regex::Regex;

use super::{IdeDbEngine, IdeDetectedDbAccess, IdeDetectedDbKind};

const MAX_SCAN_FILES: usize = 72;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_DETECTED_ITEMS: usize = 60;

struct DbPatterns {
    sql_from: Regex,
    sql_into: Regex,
    sql_update: Regex,
    sql_delete: Regex,
    builder_table: Regex,
    jpa_table: Regex,
    mongo_coll: Regex,
    mongo_doc_ann: Regex,
    redis_call: Regex,
    redis_fmt_key: Regex,
}

fn patterns() -> &'static DbPatterns {
    static PATTERNS: OnceLock<DbPatterns> = OnceLock::new();
    PATTERNS.get_or_init(|| DbPatterns {
        sql_from: Regex::new(
            r#"(?i)\b(?:FROM|JOIN)\s+[`"']?([A-Za-z_][A-Za-z0-9_]{1,96}(?:\.[A-Za-z_][A-Za-z0-9_]{1,96})?)[`"']?"#,
        )
        .expect("valid regex"),
        sql_into: Regex::new(
            r#"(?i)\b(?:INSERT\s+INTO|REPLACE\s+INTO)\s+[`"']?([A-Za-z_][A-Za-z0-9_]{1,96}(?:\.[A-Za-z_][A-Za-z0-9_]{1,96})?)[`"']?"#,
        )
        .expect("valid regex"),
        sql_update: Regex::new(
            r#"(?i)\bUPDATE\s+[`"']?([A-Za-z_][A-Za-z0-9_]{1,96}(?:\.[A-Za-z_][A-Za-z0-9_]{1,96})?)[`"']?\s+SET\b"#,
        )
        .expect("valid regex"),
        sql_delete: Regex::new(
            r#"(?i)\bDELETE\s+FROM\s+[`"']?([A-Za-z_][A-Za-z0-9_]{1,96}(?:\.[A-Za-z_][A-Za-z0-9_]{1,96})?)[`"']?"#,
        )
        .expect("valid regex"),
        builder_table: Regex::new(
            r#"\.(From|Into|Update|Delete|Table)\(\s*["'`]([A-Za-z_][A-Za-z0-9_.]{1,96})["'`]"#,
        )
        .expect("valid regex"),
        jpa_table: Regex::new(
            r#"@Table\s*\([^)]*\bname\s*=\s*"([A-Za-z_][A-Za-z0-9_.]{1,96})""#,
        )
        .expect("valid regex"),
        mongo_coll: Regex::new(
            r#"(?:\.Collection|\.collection|\.getCollection)\(\s*["'`]([A-Za-z_][A-Za-z0-9_.-]{1,96})["'`]"#,
        )
        .expect("valid regex"),
        mongo_doc_ann: Regex::new(
            r#"@Document\s*\([^)]*\bcollection\s*=\s*"([A-Za-z_][A-Za-z0-9_.-]{1,96})""#,
        )
        .expect("valid regex"),
        redis_call: Regex::new(
            r#"(?i)\.(Get|Set|SetNX|SetEx|MGet|HGet|HGetAll|HSet|LPush|RPush|LRange|SAdd|SMembers|ZAdd|ZRange|Del|Incr|Expire|Scan|Keys|HScan|SScan|ZScan|Exists|TTL|PTTL)\s*\(\s*(?:[A-Za-z0-9_.]+\s*,\s*){0,2}(?:match\s*=\s*)?["'`]([A-Za-z0-9_.-]{2,48}:[^"'`\s]{0,64})["'`]"#,
        )
        .expect("valid regex"),
        redis_fmt_key: Regex::new(
            r#"(?:Sprintf|format!)\(\s*["'`]([A-Za-z0-9_.-]{2,32}(?::[A-Za-z0-9_.-]{1,32})*:)[%{}]"#,
        )
        .expect("valid regex"),
    })
}

const SQL_NOISE_WORDS: &[&str] = &[
    "select",
    "where",
    "values",
    "dual",
    "unnest",
    "lateral",
    "stdin",
    "stdout",
    "json_each",
    "information_schema",
    "pg_catalog",
    "sqlite_master",
    "table",
    "index",
    "set",
    "null",
];

fn is_supported_source_ext(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "go" | "java" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "sql"
    )
}

fn infer_sql_engine(text: &str, default_sql: IdeDbEngine) -> IdeDbEngine {
    let lower = text.to_ascii_lowercase();
    if lower.contains("clickhouse")
        || lower.contains("mergetree")
        || lower.contains("chnativeconn")
        || lower.contains(" final")
    {
        IdeDbEngine::Clickhouse
    } else if lower.contains("pgx")
        || lower.contains("postgresql")
        || lower.contains("lib/pq")
        || lower.contains("tokio_postgres")
        || (lower.contains("$1") && lower.contains("returning"))
    {
        IdeDbEngine::Postgres
    } else {
        default_sql
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

fn push_op(ops: &mut Vec<String>, op: &str) {
    if !ops.iter().any(|existing| existing == op) {
        ops.push(op.to_string());
    }
}

#[allow(clippy::too_many_arguments)]
fn record_hit(
    acc: &mut BTreeMap<(IdeDbEngine, String), (usize, IdeDetectedDbAccess)>,
    priority: usize,
    engine: IdeDbEngine,
    object_name: String,
    kind: IdeDetectedDbKind,
    op: &str,
    rel_file: &str,
    line: u32,
) {
    let trimmed = object_name.trim().trim_matches('`').trim_matches('"');
    if trimmed.len() < 2 {
        return;
    }
    if kind == IdeDetectedDbKind::Table
        && SQL_NOISE_WORDS
            .iter()
            .any(|w| trimmed.eq_ignore_ascii_case(w))
    {
        return;
    }
    let (db_hint, clean_name) = if kind == IdeDetectedDbKind::Table {
        match trimmed.split_once('.') {
            Some((db, tbl)) if !db.is_empty() && !tbl.is_empty() => {
                (Some(db.to_string()), tbl.to_string())
            }
            _ => (None, trimmed.to_string()),
        }
    } else {
        (None, trimmed.to_string())
    };
    let entry = acc.entry((engine, clean_name.clone())).or_insert_with(|| {
        (
            priority,
            IdeDetectedDbAccess {
                engine,
                object_name: clean_name,
                kind,
                operations: Vec::new(),
                database_hint: db_hint.clone(),
                source_file: rel_file.to_string(),
                line,
            },
        )
    });
    if priority < entry.0 {
        entry.0 = priority;
        entry.1.source_file = rel_file.to_string();
        entry.1.line = line;
    }
    if entry.1.database_hint.is_none() && db_hint.is_some() {
        entry.1.database_hint = db_hint;
    }
    push_op(&mut entry.1.operations, op);
}

/// Scan a source string and append detected DB accesses into `acc`.
pub fn scan_source_text(
    text: &str,
    rel_file: &str,
    priority: usize,
    default_sql: IdeDbEngine,
    acc: &mut BTreeMap<(IdeDbEngine, String), (usize, IdeDetectedDbAccess)>,
) {
    let sql_engine = infer_sql_engine(text, default_sql);
    let p = patterns();
    for (idx, line_str) in text.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        if line_str.len() > 2_048 {
            continue;
        }
        for cap in p.sql_from.captures_iter(line_str) {
            record_hit(
                acc,
                priority,
                sql_engine,
                cap[1].to_string(),
                IdeDetectedDbKind::Table,
                "SELECT",
                rel_file,
                line_no,
            );
        }
        for cap in p.sql_into.captures_iter(line_str) {
            record_hit(
                acc,
                priority,
                sql_engine,
                cap[1].to_string(),
                IdeDetectedDbKind::Table,
                "INSERT",
                rel_file,
                line_no,
            );
        }
        for cap in p.sql_update.captures_iter(line_str) {
            record_hit(
                acc,
                priority,
                sql_engine,
                cap[1].to_string(),
                IdeDetectedDbKind::Table,
                "UPDATE",
                rel_file,
                line_no,
            );
        }
        for cap in p.sql_delete.captures_iter(line_str) {
            record_hit(
                acc,
                priority,
                sql_engine,
                cap[1].to_string(),
                IdeDetectedDbKind::Table,
                "DELETE",
                rel_file,
                line_no,
            );
        }
        for cap in p.builder_table.captures_iter(line_str) {
            let op = match &cap[1] {
                "Into" => "INSERT",
                "Update" => "UPDATE",
                "Delete" => "DELETE",
                _ => "SELECT",
            };
            record_hit(
                acc,
                priority,
                sql_engine,
                cap[2].to_string(),
                IdeDetectedDbKind::Table,
                op,
                rel_file,
                line_no,
            );
        }
        for cap in p.jpa_table.captures_iter(line_str) {
            record_hit(
                acc,
                priority,
                sql_engine,
                cap[1].to_string(),
                IdeDetectedDbKind::Table,
                "ENTITY",
                rel_file,
                line_no,
            );
        }
        for cap in p.mongo_coll.captures_iter(line_str) {
            record_hit(
                acc,
                priority,
                IdeDbEngine::Mongodb,
                cap[1].to_string(),
                IdeDetectedDbKind::Collection,
                "COLLECTION",
                rel_file,
                line_no,
            );
        }
        for cap in p.mongo_doc_ann.captures_iter(line_str) {
            record_hit(
                acc,
                priority,
                IdeDbEngine::Mongodb,
                cap[1].to_string(),
                IdeDetectedDbKind::Collection,
                "DOCUMENT",
                rel_file,
                line_no,
            );
        }
        for cap in p.redis_call.captures_iter(line_str) {
            let op = cap[1].to_ascii_uppercase();
            let key = normalize_redis_prefix(&cap[2]);
            record_hit(
                acc,
                priority,
                IdeDbEngine::Redis,
                key,
                IdeDetectedDbKind::KeyPrefix,
                &op,
                rel_file,
                line_no,
            );
        }
        for cap in p.redis_fmt_key.captures_iter(line_str) {
            let key = normalize_redis_prefix(&cap[1]);
            record_hit(
                acc,
                priority,
                IdeDbEngine::Redis,
                key,
                IdeDetectedDbKind::KeyPrefix,
                "KEY",
                rel_file,
                line_no,
            );
        }
    }
}

fn is_likely_db_file(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    lower.contains("dao")
        || lower.contains("repo")
        || lower.contains("store")
        || lower.contains("db")
        || lower.contains("sql")
        || lower.contains("mongo")
        || lower.contains("redis")
        || lower.contains("clickhouse")
        || lower.contains("test")
        || lower.ends_with(".sql")
}

/// Scan `roots` (prioritizing `active_file`) for database objects accessed in code.
pub fn detect_project_db_access(
    primary_root: &Path,
    extra_roots: &[PathBuf],
    active_file: Option<&str>,
    default_sql: IdeDbEngine,
) -> Vec<IdeDetectedDbAccess> {
    let mut acc = BTreeMap::new();
    let canonical_primary = primary_root
        .canonicalize()
        .unwrap_or_else(|_| primary_root.to_path_buf());
    let mut canonical_roots = vec![canonical_primary.clone()];
    for extra in extra_roots {
        if let Ok(c) = extra.canonicalize() {
            if !canonical_roots.contains(&c) {
                canonical_roots.push(c);
            }
        }
    }

    if let Some(active) = active_file.map(str::trim).filter(|s| !s.is_empty()) {
        let candidate = if Path::new(active).is_absolute() {
            PathBuf::from(active)
        } else {
            canonical_primary.join(active)
        };
        if let Ok(canon) = candidate.canonicalize() {
            if canonical_roots.iter().any(|r| canon.starts_with(r))
                && crate::target::enforce_non_root_path(&canon).is_ok()
                && canon
                    .metadata()
                    .map(|m| m.len() <= MAX_FILE_BYTES)
                    .unwrap_or(false)
            {
                if let Ok(text) = std::fs::read_to_string(&canon) {
                    let rel = canon
                        .strip_prefix(&canonical_primary)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| active.to_string());
                    scan_source_text(&text, &rel, 0, default_sql, &mut acc);
                }
            }
        }
    }

    let mut scanned = 0usize;
    let mut fallback_files = Vec::new();
    for root in &canonical_roots {
        if scanned >= MAX_SCAN_FILES {
            break;
        }
        let walker = WalkBuilder::new(root)
            .max_depth(Some(6))
            .hidden(true)
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let name = e.file_name().to_string_lossy();
                !matches!(
                    name.as_ref(),
                    "vendor" | "node_modules" | "target" | "dist" | "build" | ".git"
                )
            })
            .build();
        for entry in walker.flatten() {
            if scanned >= MAX_SCAN_FILES {
                break;
            }
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let path = entry.path();
            if !is_supported_source_ext(path) || crate::target::enforce_non_root_path(path).is_err()
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            if is_likely_db_file(path) {
                if let Ok(text) = std::fs::read_to_string(path) {
                    let rel = path
                        .strip_prefix(&canonical_primary)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| path.to_string_lossy().to_string());
                    scan_source_text(&text, &rel, 1, default_sql, &mut acc);
                    scanned += 1;
                }
            } else if fallback_files.len() < 16 {
                fallback_files.push(path.to_path_buf());
            }
        }
    }

    if acc.is_empty() {
        for path in fallback_files {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let rel = path
                    .strip_prefix(&canonical_primary)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.to_string_lossy().to_string());
                scan_source_text(&text, &rel, 2, default_sql, &mut acc);
            }
        }
    }

    let mut list: Vec<(usize, IdeDetectedDbAccess)> = acc.into_values().collect();
    list.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.object_name.cmp(&b.1.object_name))
    });
    list.into_iter()
        .take(MAX_DETECTED_ITEMS)
        .map(|(_, item)| item)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sql_mongo_redis_and_clickhouse_across_polyglot_code() {
        let mut acc = BTreeMap::new();
        let go_dao = r#"
            q := sq.Select("id", "balance").From("MdlGm_tblPlayers")
            _, _ = db.ExecContext(ctx, "UPDATE pr_bo_1.MdlGm_tblPlayers_L01 SET balance = ? WHERE RefID = ?")
            coll := mongoDb.Collection("player_journeys")
            _ = redisClient.Set(ctx, fmt.Sprintf("player:bal:%d", id), "100", 0)
            _ = redisClient.HGetAll(ctx, "session:token:abc")
            rdb.Scan(0, "session:*", 100)
            py_redis.scan(0, match="cache:*")
        "#;
        scan_source_text(
            go_dao,
            "internal/dao/player_dao.go",
            0,
            IdeDbEngine::Mysql,
            &mut acc,
        );

        let ch_dao = r#"
            // clickhouse analytics query
            rows, err := chNativeConn.Query(ctx, "SELECT player_id FROM events_ledger FINAL WHERE brand_id = 1")
        "#;
        scan_source_text(
            ch_dao,
            "internal/dao/ch_dao.go",
            1,
            IdeDbEngine::Mysql,
            &mut acc,
        );

        let names: Vec<(IdeDbEngine, String)> = acc.keys().cloned().collect();
        assert!(names.contains(&(IdeDbEngine::Mysql, "MdlGm_tblPlayers".into())));
        assert!(names.contains(&(IdeDbEngine::Mysql, "MdlGm_tblPlayers_L01".into())));
        assert!(names.contains(&(IdeDbEngine::Mongodb, "player_journeys".into())));
        assert!(names.contains(&(IdeDbEngine::Redis, "player:bal:*".into())));
        assert!(names.contains(&(IdeDbEngine::Redis, "session:token:abc".into())));
        assert!(names.contains(&(IdeDbEngine::Redis, "session:*".into())));
        assert!(names.contains(&(IdeDbEngine::Redis, "cache:*".into())));
        assert!(names.contains(&(IdeDbEngine::Clickhouse, "events_ledger".into())));
    }
}
