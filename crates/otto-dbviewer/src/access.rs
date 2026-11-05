//! Resource authorization shared by HTTP, assistant, MCP, and dashboard paths.
use otto_core::access::{AccessMode, AccessPolicy, ResourceKind, ResourceRef, RuleEffect};
use otto_core::domain::{Capability, Feature};
use otto_core::domain::{Connection, User};
use otto_core::{Error, Id, Result};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::{DbPool, GrantsRepo, UsersRepo, WorkspacesRepo};
use sqlparser::ast::{Expr, ObjectName, Statement, Visit, Visitor};
use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;
use std::ops::ControlFlow;

use crate::types::{Engine, Scope};

pub(crate) fn target(conn: &Id, child: Option<&str>) -> ResourceRef {
    ResourceRef {
        kind: ResourceKind::Connection,
        id: conn.clone(),
        child: child.filter(|s| !s.is_empty()).map(str::to_owned),
    }
}

/// The resource-access child a node names: the bare database / schema name or
/// Redis keyspace index. Parsed through [`Scope`] so this, the canonical node
/// the drivers receive ([`canonical_node`]) and the drivers themselves agree.
pub(crate) fn child(node: Option<&str>) -> Option<String> {
    Scope::parse(node).map(|s| s.child())
}

/// The node a driver receives: the request's [`Scope`] in canonical form
/// (`kdb:<n>` for a Redis keyspace, the plain name otherwise). It must NOT be
/// the access [`child`]: a bare `3` is how access rules name a keyspace, but
/// rewriting the node to it made the Redis driver fall back to its default
/// database, so a command run with db3 selected read and wrote db0.
pub(crate) fn canonical_node(node: Option<&str>) -> Option<String> {
    Scope::parse(node).map(|s| s.to_node())
}

/// Per-thread count of state-DB reads made by the access/connection helpers.
/// Test-only: it backs the "≤ N state reads per Run" budget tests that lock in
/// the request-scoped [`crate::service`] access snapshot (perf DB-06/DB-10).
#[cfg(test)]
pub(crate) mod reads {
    use std::cell::Cell;
    thread_local!(static COUNT: Cell<u64> = const { Cell::new(0) });
    pub(crate) fn bump() {
        COUNT.with(|c| c.set(c.get() + 1));
    }
    pub(crate) fn take() -> u64 {
        COUNT.with(|c| c.replace(0))
    }
}
#[cfg(test)]
pub(crate) use reads::bump as count_read;
#[cfg(not(test))]
#[inline(always)]
pub(crate) fn count_read() {}

pub(crate) async fn policy(pool: &DbPool, id: &Id) -> Result<AccessPolicy> {
    count_read();
    otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
        .get_policy(ResourceKind::Connection, id)
        .await
}

/// Reload the effective user and membership for every action. The passed id is
/// supplied by authenticated adapters, never accepted from a request body.
pub(crate) async fn current_user(pool: &DbPool, conn: &Connection, id: &Id) -> Result<User> {
    count_read();
    let user = UsersRepo::new(pool.clone()).get(id).await?;
    if user.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    GrantsRepo::new(pool.clone())
        .check_global(
            &user,
            Feature::Database,
            Capability::View,
            "Database feature access required",
        )
        .await?;
    if let Some(ws) = &conn.workspace_id {
        if WorkspacesRepo::new(pool.clone())
            .role_of(&user, ws)
            .await?
            .is_none()
        {
            return Err(Error::NotFound("connection".into()));
        }
    }
    Ok(user)
}

pub(crate) async fn check(
    pool: &DbPool,
    conn: &Connection,
    user_id: &Id,
    child: Option<&str>,
    operation: &str,
) -> Result<()> {
    let policy = policy(pool, &conn.id).await?;
    check_with(pool, conn, &policy, user_id, child, operation).await
}

/// [`check`] against a policy the caller already loaded for this request (the
/// request-scoped access snapshot) — no second policy read.
pub(crate) async fn check_with(
    pool: &DbPool,
    conn: &Connection,
    policy: &AccessPolicy,
    user_id: &Id,
    child: Option<&str>,
    operation: &str,
) -> Result<()> {
    if policy.mode == AccessMode::Legacy {
        return Ok(());
    }
    let user = current_user(pool, conn, user_id).await?;
    check_loaded_user(pool, conn, &user, child, operation).await
}

/// The enforced half of [`check_with`] for a user the caller JUST reloaded via
/// [`current_user`] in the same request step (no second user/grant reload).
async fn check_loaded_user(
    pool: &DbPool,
    conn: &Connection,
    user: &User,
    child: Option<&str>,
    operation: &str,
) -> Result<()> {
    let access = ResourceAccess::new(pool.clone());
    if !access
        .evaluate(user, &target(&conn.id, None), "discover")
        .await?
        .allowed
    {
        return Err(Error::NotFound("connection".into()));
    }
    access
        .check(user, &target(&conn.id, child), operation)
        .await
}

/// [`check_with`] for many children at once (tree roots, search hits, ERD edge
/// targets): the user/membership reload and the connection-level `discover`
/// evaluation run ONCE instead of once per child. Returns one allow flag per
/// child; a caller-level failure (disabled account, no discover) denies all.
pub(crate) async fn check_many(
    pool: &DbPool,
    conn: &Connection,
    policy: &AccessPolicy,
    user_id: &Id,
    children: &[Option<String>],
    operation: &str,
) -> Result<Vec<bool>> {
    if policy.mode == AccessMode::Legacy {
        return Ok(vec![true; children.len()]);
    }
    let Ok(user) = current_user(pool, conn, user_id).await else {
        return Ok(vec![false; children.len()]);
    };
    let access = ResourceAccess::new(pool.clone());
    if !access
        .evaluate(&user, &target(&conn.id, None), "discover")
        .await?
        .allowed
    {
        return Ok(vec![false; children.len()]);
    }
    let mut out = Vec::with_capacity(children.len());
    for child in children {
        out.push(
            access
                .check(&user, &target(&conn.id, child.as_deref()), operation)
                .await
                .is_ok(),
        );
    }
    Ok(out)
}

/// Choose only credentials attached to matching Allow rules. Ambiguous profiles
/// (including an explicit primary profile plus an alternate) are rejected.
pub(crate) async fn credential_profile(
    pool: &DbPool,
    conn: &Connection,
    user_id: &Id,
    child: Option<&str>,
    operation: &str,
) -> Result<(Id, Option<String>)> {
    let policy = policy(pool, &conn.id).await?;
    credential_profile_with(pool, conn, &policy, user_id, child, operation).await
}

/// [`credential_profile`] against an already-loaded policy.
pub(crate) async fn credential_profile_with(
    pool: &DbPool,
    conn: &Connection,
    policy: &AccessPolicy,
    user_id: &Id,
    child: Option<&str>,
    operation: &str,
) -> Result<(Id, Option<String>)> {
    if policy.mode == AccessMode::Legacy {
        return Ok((conn.id.clone(), None));
    }
    let user = current_user(pool, conn, user_id).await?;
    check_loaded_user(pool, conn, &user, child, operation).await?;
    let decision = ResourceAccess::new(pool.clone())
        .evaluate(&user, &target(&conn.id, child), operation)
        .await?;
    let mut profiles = std::collections::BTreeSet::new();
    for rule in &policy.rules {
        if rule.effect == RuleEffect::Allow && decision.matched_rule_ids.contains(&rule.id) {
            profiles.insert(
                rule.credential_connection_id
                    .as_ref()
                    .unwrap_or(&conn.id)
                    .clone(),
            );
        }
    }
    if profiles.len() > 1 {
        return Err(crate::native_access::setup_error(
            "matching access rules select conflicting credential profiles",
        ));
    }
    let profile = profiles
        .into_iter()
        .next()
        .unwrap_or_else(|| conn.id.clone());
    let scope = format!(
        "{}:{}:{}:{}:{}",
        user.id,
        policy.revision,
        operation,
        child.unwrap_or(""),
        decision.matched_rule_ids.join(",")
    );
    Ok((profile, Some(scope)))
}

/// Restrict executable expressions as an additional boundary around builtin
/// side effects and native PUBLIC catalogs. Native table/role checks remain
/// authoritative for data privileges; this never grants a database operation.
/// Walks a statement for anything restricted SQL refuses. A read of a native
/// system catalog is not refused: it is recorded (`catalog`) and costs the
/// connection-wide `db_catalog` operation (see [`operations`]).
#[derive(Default)]
struct SafeExpressions {
    catalog: bool,
}
impl Visitor for SafeExpressions {
    type Break = Error;
    fn pre_visit_query(&mut self, query: &sqlparser::ast::Query) -> ControlFlow<Self::Break> {
        if !query_body_is_read(&query.body)
            || query.with.as_ref().is_some_and(|w| {
                w.cte_tables
                    .iter()
                    .any(|cte| !query_body_is_read(&cte.query.body))
            })
        {
            return ControlFlow::Break(Error::Forbidden(
                "data-changing CTEs require a reviewed change".into(),
            ));
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_table_factor(
        &mut self,
        table: &sqlparser::ast::TableFactor,
    ) -> ControlFlow<Self::Break> {
        use sqlparser::ast::TableFactor;
        match table {
            TableFactor::Table { args: None, .. }
            | TableFactor::Derived { .. }
            | TableFactor::NestedJoin { .. } => ControlFlow::Continue(()),
            _ => ControlFlow::Break(Error::Forbidden(
                "table functions and specialized table sources are unsupported in restricted SQL"
                    .into(),
            )),
        }
    }
    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        if let Expr::Function(function) = expr {
            let name = function.name.to_string().to_ascii_lowercase();
            if !pure_builtin(&name) {
                return ControlFlow::Break(Error::Forbidden(format!(
                    "function {name}() isn't on restricted SQL's list of built-in pure functions \
                     (a stored / user-defined function can write)"
                )));
            }
        }
        let cast_type = match expr {
            Expr::Cast { data_type, .. } => Some(data_type),
            Expr::TypedString(typed) => Some(&typed.data_type),
            _ => None,
        };
        if let Some(data_type) = cast_type {
            let ty = data_type.to_string().to_ascii_lowercase();
            let base = ty.split(['(', '[', ' ']).next().unwrap_or("");
            if !matches!(
                base,
                "text"
                    | "varchar"
                    | "character"
                    | "char"
                    | "int"
                    | "integer"
                    | "bigint"
                    | "smallint"
                    | "numeric"
                    | "decimal"
                    | "float"
                    | "double"
                    | "real"
                    | "boolean"
                    | "bool"
                    | "date"
                    | "time"
                    | "timestamp"
                    | "datetime"
                    | "json"
                    | "jsonb"
                    | "uuid"
                    | "bytea"
                    | "binary"
                    | "varbinary"
                    | "signed"
                    | "unsigned"
            ) {
                return ControlFlow::Break(Error::Forbidden(
                    "catalog and custom-type casts are unsupported in restricted SQL".into(),
                ));
            }
        }
        ControlFlow::Continue(())
    }
    fn pre_visit_relation(&mut self, relation: &ObjectName) -> ControlFlow<Self::Break> {
        let name = relation
            .to_string()
            .replace(['"', '`'], "")
            .to_ascii_lowercase();
        if name.split('.').any(|p| {
            p.starts_with("pg_")
                || matches!(
                    p,
                    "information_schema" | "mysql" | "sys" | "performance_schema"
                )
        }) {
            self.catalog = true;
        }
        ControlFlow::Continue(())
    }
}

/// A built-in, side-effect-free SQL function (MySQL + PostgreSQL), matched
/// on its UNQUALIFIED lower-cased name — so a schema-qualified call (a
/// user-defined / stored function, which can write) never matches. Nothing
/// here sleeps, locks, reads files, touches sequences or other sessions
/// ([`function_has_side_effect`] is the deny side of the same line).
fn pure_builtin(name: &str) -> bool {
    const PURE: &[&str] = &[
        // aggregates + window
        "count",
        "sum",
        "avg",
        "min",
        "max",
        "group_concat",
        "string_agg",
        "array_agg",
        "json_agg",
        "jsonb_agg",
        "json_arrayagg",
        "json_objectagg",
        "bit_and",
        "bit_or",
        "bit_xor",
        "bool_and",
        "bool_or",
        "every",
        "stddev",
        "stddev_pop",
        "stddev_samp",
        "std",
        "variance",
        "var_pop",
        "var_samp",
        "percentile_cont",
        "percentile_disc",
        "mode",
        "row_number",
        "rank",
        "dense_rank",
        "percent_rank",
        "cume_dist",
        "ntile",
        "lag",
        "lead",
        "first_value",
        "last_value",
        "nth_value",
        "any_value",
        // conditional / null / type
        "coalesce",
        "nullif",
        "ifnull",
        "if",
        "isnull",
        "greatest",
        "least",
        "cast",
        "convert",
        "to_char",
        "to_date",
        "to_timestamp",
        "to_number",
        // numeric
        "abs",
        "round",
        "floor",
        "ceil",
        "ceiling",
        "trunc",
        "truncate",
        "mod",
        "power",
        "pow",
        "sqrt",
        "exp",
        "ln",
        "log",
        "log2",
        "log10",
        "sign",
        "pi",
        "degrees",
        "radians",
        "sin",
        "cos",
        "tan",
        "asin",
        "acos",
        "atan",
        "atan2",
        "cot",
        "div",
        "format",
        "conv",
        "hex",
        "unhex",
        "bin",
        "oct",
        "crc32",
        "md5",
        "sha1",
        "sha2",
        "width_bucket",
        // string
        "lower",
        "upper",
        "lcase",
        "ucase",
        "length",
        "char_length",
        "character_length",
        "octet_length",
        "bit_length",
        "concat",
        "concat_ws",
        "substring",
        "substr",
        "mid",
        "left",
        "right",
        "trim",
        "ltrim",
        "rtrim",
        "btrim",
        "replace",
        "reverse",
        "repeat",
        "lpad",
        "rpad",
        "locate",
        "position",
        "instr",
        "strpos",
        "split_part",
        "substring_index",
        "field",
        "find_in_set",
        "ascii",
        "ord",
        "char",
        "chr",
        "space",
        "initcap",
        "insert",
        "elt",
        "quote",
        "regexp_like",
        "regexp_replace",
        "regexp_substr",
        "regexp_instr",
        "regexp_match",
        "regexp_matches",
        "starts_with",
        "translate",
        "soundex",
        "to_base64",
        "from_base64",
        "encode",
        "decode",
        // date / time
        "now",
        "current_date",
        "current_time",
        "current_timestamp",
        "curdate",
        "curtime",
        "localtime",
        "localtimestamp",
        "utc_date",
        "utc_time",
        "utc_timestamp",
        "date",
        "time",
        "timestamp",
        "year",
        "quarter",
        "month",
        "monthname",
        "week",
        "weekday",
        "weekofyear",
        "yearweek",
        "day",
        "dayofmonth",
        "dayofweek",
        "dayofyear",
        "dayname",
        "hour",
        "minute",
        "second",
        "microsecond",
        "date_format",
        "time_format",
        "date_add",
        "date_sub",
        "adddate",
        "subdate",
        "addtime",
        "subtime",
        "datediff",
        "timediff",
        "timestampdiff",
        "timestampadd",
        "date_trunc",
        "date_part",
        "extract",
        "last_day",
        "makedate",
        "maketime",
        "make_date",
        "make_timestamp",
        "from_unixtime",
        "unix_timestamp",
        "str_to_date",
        "sec_to_time",
        "time_to_sec",
        "to_days",
        "from_days",
        "period_add",
        "period_diff",
        "convert_tz",
        "age",
        "date_bin",
        "justify_days",
        "justify_hours",
        "justify_interval",
        "isfinite",
        // json
        "json_extract",
        "json_unquote",
        "json_value",
        "json_contains",
        "json_contains_path",
        "json_keys",
        "json_length",
        "json_type",
        "json_valid",
        "json_object",
        "json_array",
        "json_search",
        "json_depth",
        "json_pretty",
        "json_quote",
        "json_overlaps",
        "json_build_object",
        "json_build_array",
        "jsonb_build_object",
        "jsonb_build_array",
        "json_extract_path",
        "json_extract_path_text",
        "jsonb_extract_path",
        "jsonb_extract_path_text",
        "json_array_length",
        "jsonb_array_length",
        "json_typeof",
        "jsonb_typeof",
        "jsonb_pretty",
        "to_json",
        "to_jsonb",
        "row_to_json",
        "json_object_keys",
        "jsonb_object_keys",
        // arrays (PostgreSQL)
        "array_length",
        "cardinality",
        "array_position",
        "array_to_string",
        "string_to_array",
        "array_upper",
        "array_lower",
        "unnest",
    ];
    PURE.contains(&name)
}

/// True when `sql` provably only READS: it parses with the engine's sqlparser
/// dialect into queries whose body and CTEs are all reads (no `SELECT … INTO`,
/// no data-changing CTE), `EXPLAIN`s without an executing `ANALYZE` of such a
/// query, or `DESCRIBE <table>`. Used by the legacy write-guard and the MCP
/// read-only gate in addition to their keyword checks: a first keyword cannot
/// reveal a write nested inside a read-looking statement.
///
/// Unlike [`operations`] this does NOT allow-list functions or catalogs — that
/// is the enforced-mode boundary; the legacy paths rely on native privileges
/// and, for MCP, a native read-only transaction behind this check. It DOES
/// refuse the built-ins a read-only transaction cannot stop
/// ([`function_has_side_effect`]: killing backends, advisory locks, `dblink`,
/// `nextval`, …), so such a `SELECT` is never treated as a read. A parse
/// failure is unproven and returns `false` (the callers treat it as a write).
/// Engines other than MySQL / PostgreSQL are not parsed and return `true`.
pub(crate) fn read_is_provable(engine: Engine, sql: &str) -> bool {
    let parsed = match engine {
        Engine::Mysql => Parser::parse_sql(&MySqlDialect {}, sql),
        Engine::Postgres => Parser::parse_sql(&PostgreSqlDialect {}, sql),
        _ => return true,
    };
    let Ok(statements) = parsed else {
        return false;
    };
    !statements.is_empty()
        && statements.iter().all(statement_is_pure_read)
        && statements.visit(&mut SideEffectFunctions).is_continue()
}

/// True for a built-in whose CALL has a side effect even inside a read-only
/// transaction: it signals or kills other backends, reloads / rotates server
/// state, takes session-level locks, writes over a separate session
/// (`dblink*`), touches large objects or server files, publishes
/// notifications, or advances a sequence. A `SELECT` that calls one is not a
/// proven read — `BEGIN READ ONLY` (the MCP second barrier) does not stop any
/// of them. Matched on the unqualified, unquoted, lower-cased name so
/// `pg_catalog."pg_terminate_backend"` cannot slip past.
pub(crate) fn function_has_side_effect(name: &str) -> bool {
    let name = name
        .rsplit('.')
        .next()
        .unwrap_or(name)
        .replace(['"', '`'], "")
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        // PostgreSQL
        "pg_terminate_backend"
            | "pg_cancel_backend"
            | "pg_reload_conf"
            | "pg_rotate_logfile"
            | "pg_switch_wal"
            | "pg_switch_xlog"
            | "pg_promote"
            | "pg_create_restore_point"
            | "pg_wal_replay_pause"
            | "pg_wal_replay_resume"
            | "pg_notify"
            | "pg_logical_emit_message"
            | "pg_sleep"
            | "pg_sleep_for"
            | "pg_sleep_until"
            | "set_config"
            | "nextval"
            | "setval"
            | "txid_current"
            | "pg_current_xact_id"
            | "pg_stat_statements_reset"
            | "pg_start_backup"
            | "pg_stop_backup"
            | "pg_log_backend_memory_contexts"
            // Execute a query given as TEXT: the visitor never sees the inner
            // statement, so `query_to_xml('SELECT pg_terminate_backend(…)', …)`
            // would hide any other entry here behind a pure-looking SELECT.
            | "ts_stat"
            | "ts_rewrite"
            // MySQL
            | "get_lock"
            | "release_lock"
            | "release_all_locks"
            | "sleep"
            | "benchmark"
            | "master_pos_wait"
            | "source_pos_wait"
            | "load_file"
    ) || name.starts_with("dblink")
        || name.starts_with("lo_")
        || name.starts_with("pg_read_")
        || name.starts_with("pg_ls_")
        || name.starts_with("pg_stat_reset")
        || name.starts_with("pg_file_")
        || name.starts_with("pg_create_")
        || name.starts_with("pg_drop_")
        || name.starts_with("pg_replication_")
        || name.starts_with("pg_backup_")
        || name.starts_with("pg_background")
        || name.starts_with("query_to_xml")
        || name.starts_with("cursor_to_xml")
        || name.contains("advisory")
}

/// True when a string literal is itself SQL — it parses as one or more
/// statements in either dialect we classify. A function handed such text may
/// execute it (`query_to_xml`, `ts_stat`, `dblink`, an extension we do not
/// know by name), and the visitor cannot see inside a literal, so a call
/// carrying one is not a proven read. Defence in depth behind the name list:
/// a concatenated or `format()`-built query still needs the name list.
fn literal_is_sql(text: &str) -> bool {
    let t = text.trim_start().trim_start_matches('(').trim_start();
    let head: String = t
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_lowercase();
    if !matches!(
        head.as_str(),
        "select"
            | "with"
            | "insert"
            | "update"
            | "delete"
            | "merge"
            | "values"
            | "table"
            | "call"
            | "do"
            | "copy"
            | "create"
            | "drop"
            | "alter"
            | "truncate"
            | "grant"
            | "revoke"
            | "set"
            | "explain"
            | "lock"
            | "vacuum"
            | "analyze"
            | "refresh"
            | "notify"
    ) {
        return false;
    }
    Parser::parse_sql(&PostgreSqlDialect {}, text).is_ok_and(|s| !s.is_empty())
        || Parser::parse_sql(&MySqlDialect {}, text).is_ok_and(|s| !s.is_empty())
}

/// Finds a SQL-text string literal anywhere inside one function call's
/// arguments (see [`literal_is_sql`]).
struct SqlTextLiteral;
impl Visitor for SqlTextLiteral {
    type Break = ();
    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        use sqlparser::ast::Value;
        let Expr::Value(v) = expr else {
            return ControlFlow::Continue(());
        };
        let text = match &v.value {
            Value::SingleQuotedString(s)
            | Value::DoubleQuotedString(s)
            | Value::EscapedStringLiteral(s)
            | Value::NationalStringLiteral(s)
            | Value::TripleSingleQuotedString(s)
            | Value::TripleDoubleQuotedString(s) => s.as_str(),
            Value::DollarQuotedString(d) => d.value.as_str(),
            _ => return ControlFlow::Continue(()),
        };
        if literal_is_sql(text) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

/// Refuses any call (scalar or table function) of a
/// [`function_has_side_effect`] built-in, anywhere in the statement.
struct SideEffectFunctions;
impl Visitor for SideEffectFunctions {
    type Break = ();
    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        match expr {
            Expr::Function(f) if function_has_side_effect(&f.name.to_string()) => {
                ControlFlow::Break(())
            }
            // A function handed SQL as text may execute it out of the
            // visitor's sight; refuse rather than guess which ones do.
            Expr::Function(f) if f.args.visit(&mut SqlTextLiteral).is_break() => {
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(()),
        }
    }
    fn pre_visit_table_factor(
        &mut self,
        table: &sqlparser::ast::TableFactor,
    ) -> ControlFlow<Self::Break> {
        use sqlparser::ast::TableFactor;
        let name = match table {
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            } => Some(name.to_string()),
            TableFactor::Function { name, .. } => Some(name.to_string()),
            _ => None,
        };
        if name.is_some_and(|n| function_has_side_effect(&n)) {
            return ControlFlow::Break(());
        }
        // `SELECT * FROM ts_stat('SELECT …')`-style table functions: the
        // arguments of a table-valued call are SQL text just the same.
        let sql_text_arg = match table {
            TableFactor::Table {
                args: Some(args), ..
            } => args.visit(&mut SqlTextLiteral).is_break(),
            TableFactor::Function { args, .. } => args.visit(&mut SqlTextLiteral).is_break(),
            _ => false,
        };
        if sql_text_arg {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

fn statement_is_pure_read(statement: &Statement) -> bool {
    match statement {
        Statement::Query(query) => query_is_read(query),
        Statement::ExplainTable { .. } => true,
        Statement::Explain {
            analyze,
            statement,
            options,
            ..
        } => {
            let executes = *analyze
                || options.as_ref().is_some_and(|opts| {
                    opts.iter()
                        .any(|o| o.name.value.eq_ignore_ascii_case("analyze"))
                });
            !executes && matches!(statement.as_ref(), Statement::Query(q) if query_is_read(q))
        }
        _ => false,
    }
}

fn query_is_read(query: &sqlparser::ast::Query) -> bool {
    query_body_is_read(&query.body)
        && query
            .with
            .as_ref()
            .is_none_or(|w| w.cte_tables.iter().all(|cte| query_is_read(&cte.query)))
}

fn query_body_is_read(body: &sqlparser::ast::SetExpr) -> bool {
    use sqlparser::ast::SetExpr;
    match body {
        SetExpr::Select(select) => select.into.is_none(),
        SetExpr::Values(_) => true,
        SetExpr::Query(query) => {
            query_body_is_read(&query.body)
                && query.with.as_ref().is_none_or(|w| {
                    w.cte_tables
                        .iter()
                        .all(|cte| query_body_is_read(&cte.query.body))
                })
        }
        SetExpr::SetOperation { left, right, .. } => {
            query_body_is_read(left) && query_body_is_read(right)
        }
        _ => false,
    }
}

/// AST operation accounting supplements (never replaces) native privileges.
/// Unknown session/admin/routine commands are refused so SQL cannot change roles
/// or defeat the per-operation approval path. Multi-statement scripts union all
/// required operations, including data-changing CTEs conservatively.
pub(crate) fn operations(engine: Engine, sql: &str) -> Result<Vec<&'static str>> {
    let statements = match engine {
        Engine::Mysql => Parser::parse_sql(&MySqlDialect {}, sql),
        Engine::Postgres => Parser::parse_sql(&PostgreSqlDialect {}, sql),
        // Redis / MongoDB / ClickHouse have no AST parser here. They used to be
        // refused outright — which, since a new connection starts Enforced,
        // meant a freshly created one could run NOTHING, not even for its
        // owner or root. Classify with the engine's own lexer instead (the
        // same conservative classifier the MCP read-only gate trusts; unknown
        // counts as a write). A write needs data AND schema rights (the lexer
        // can't tell them apart); a read is `db_query`, and the caller
        // additionally requires `db_data` for any non-parser-proven read
        // (see [`unparsed_read_requires`]) so viewer-level trust is unchanged.
        _ => {
            if sql.trim().is_empty() {
                return Err(Error::Invalid("empty statement".into()));
            }
            return Ok(if crate::types::statement_is_write(engine, sql) {
                vec!["db_query", "db_data", "db_schema"]
            } else {
                vec!["db_query"]
            });
        }
    }
    .map_err(|_| {
        Error::Forbidden("restricted execution requires a fully parsed supported SQL script".into())
    })?;
    if statements.is_empty() {
        return Err(Error::Invalid("empty statement".into()));
    }
    let mut ops = vec!["db_query"];
    for statement in statements {
        let mut safe = SafeExpressions::default();
        if let ControlFlow::Break(error) = statement.visit(&mut safe) {
            return Err(error);
        }
        if safe.catalog && !ops.contains(&"db_catalog") {
            ops.push("db_catalog");
        }
        let op = match statement {
            Statement::Query(ref q) => {
                // PostgreSQL allows modifying statements inside a CTE. Traverse
                // the entire query's rendered SQL via the existing conservative
                // parser classifier before deciding it is a read.
                if q.with.as_ref().is_some_and(|w| {
                    w.cte_tables
                        .iter()
                        .any(|cte| !query_body_is_read(&cte.query.body))
                }) || !query_body_is_read(&q.body)
                {
                    return Err(Error::Forbidden(
                        "data-changing CTEs require a reviewed change".into(),
                    ));
                }
                "db_query"
            }
            Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) => "db_data",
            Statement::Truncate(_)
            | Statement::CreateTable(_)
            | Statement::AlterTable(_)
            | Statement::CreateIndex(_)
            | Statement::Drop { .. } => "db_schema",
            Statement::Explain {
                analyze: false,
                ref statement,
                ref options,
                ..
            } if options.is_none() && matches!(statement.as_ref(), Statement::Query(_)) => {
                "db_query"
            }
            // Plain transaction control grants nothing by itself: the grid's
            // Review & apply wraps a multi-row edit in `BEGIN; … COMMIT;`
            // (ui review-batch.ts) and every statement inside is still
            // classified above. A `BEGIN … END` block body is not — refuse it.
            Statement::StartTransaction {
                ref statements,
                ref exception,
                ..
            } if statements.is_empty() && exception.is_none() => "db_query",
            Statement::Commit { .. }
            | Statement::Rollback {
                savepoint: None, ..
            } => "db_query",
            // Metadata reads (SHOW … / DESCRIBE): connection-wide, like a
            // system-catalog SELECT — they can name any table.
            Statement::ShowColumns { .. }
            | Statement::ShowCreate { .. }
            | Statement::ShowTables { .. }
            | Statement::ShowViews { .. }
            | Statement::ShowDatabases { .. }
            | Statement::ShowSchemas { .. }
            | Statement::ShowFunctions { .. }
            | Statement::ShowCollation { .. }
            | Statement::ShowCharset(_)
            | Statement::ShowVariable { .. }
            | Statement::ShowVariables { .. }
            | Statement::ShowStatus { .. }
            | Statement::ExplainTable { .. } => "db_catalog",
            _ => {
                return Err(Error::Forbidden(
                    "this SQL form is unsupported for governed direct execution".into(),
                ));
            }
        };
        if !ops.contains(&op) {
            ops.push(op);
        }
    }
    Ok(ops)
}

/// Every operation in `ops` only reads (`db_query`, or a metadata /
/// system-catalog read — `db_catalog`).
pub(crate) fn only_reads(ops: &[&str]) -> bool {
    ops.iter()
        .all(|op| matches!(*op, "db_query" | "db_catalog"))
}

/// The (operation, child) an operation is authorized as: `db_catalog` (a
/// system-catalog / SHOW / DESCRIBE read, which can name ANY table) is a
/// `db_query` on the WHOLE connection — so a user confined to some schemas
/// or tables can't list the rest; everything else keeps the request's child.
pub(crate) fn authorized_as<'a>(op: &'a str, child: Option<&'a str>) -> (&'a str, Option<&'a str>) {
    if op == "db_catalog" {
        ("db_query", None)
    } else {
        (op, child)
    }
}

/// Extra operation an authorizer must hold for a statement whose read-ness
/// is lexer-classified rather than parser-proven (every engine but MySQL /
/// PostgreSQL): editor-level trust. Keeps a `db_query`-only viewer exactly as
/// restricted as before on those engines, while owners and editors can query
/// their own connections. `None` for the parsed SQL engines.
pub(crate) fn unparsed_read_requires(engine: Engine) -> Option<&'static str> {
    match engine {
        Engine::Mysql | Engine::Postgres => None,
        _ => Some("db_data"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unparsed_engines_classify_reads_and_writes_instead_of_refusing() {
        assert_eq!(
            operations(Engine::Redis, "GET k").unwrap(),
            vec!["db_query"]
        );
        assert_eq!(
            operations(Engine::Redis, "SET k v").unwrap(),
            vec!["db_query", "db_data", "db_schema"]
        );
        assert_eq!(
            operations(Engine::Mongodb, "db.users.find({})").unwrap(),
            vec!["db_query"]
        );
        assert_eq!(
            operations(Engine::Mongodb, "db.users.deleteOne({})").unwrap(),
            vec!["db_query", "db_data", "db_schema"]
        );
        assert_eq!(
            operations(Engine::Clickhouse, "SELECT 1").unwrap(),
            vec!["db_query"]
        );
        assert_eq!(
            operations(Engine::Clickhouse, "DROP TABLE t").unwrap(),
            vec!["db_query", "db_data", "db_schema"]
        );
        assert!(matches!(
            operations(Engine::Redis, "   ").unwrap_err(),
            Error::Invalid(_)
        ));
        // Reads on those engines still need editor trust; parsed SQL does not.
        assert_eq!(unparsed_read_requires(Engine::Redis), Some("db_data"));
        assert_eq!(unparsed_read_requires(Engine::Mysql), None);
    }

    /// `run` authorizes with the bare child but hands drivers the canonical
    /// node. They differ exactly for Redis: the child `3` names the keyspace
    /// for access rules, the driver needs `kdb:3` (rewriting the node to the
    /// child ran every Redis command on db0).
    #[test]
    fn child_and_canonical_node_split_authorization_from_execution() {
        assert_eq!(child(Some("kdb:3")).as_deref(), Some("3"));
        assert_eq!(canonical_node(Some("kdb:3")).as_deref(), Some("kdb:3"));
        assert_eq!(
            canonical_node(Some("kdb:3/key:session:42")).as_deref(),
            Some("kdb:3")
        );
        assert_eq!(child(Some("db:shop/table:orders")).as_deref(), Some("shop"));
        assert_eq!(canonical_node(Some("db:shop")).as_deref(), Some("shop"));
        assert_eq!(canonical_node(Some("shop")).as_deref(), Some("shop"));
        // Databases named like a path tag keep their scope (was "" before).
        for name in ["db", "kdb", "a:b", "a/b"] {
            assert_eq!(child(Some(name)).as_deref(), Some(name));
            assert_eq!(canonical_node(Some(name)).as_deref(), Some(name));
        }
        assert_eq!(child(None), None);
        assert_eq!(canonical_node(Some("")), None);
    }

    #[test]
    fn governed_sql_allows_the_review_batch_transaction_wrapper() {
        let ops = operations(
            Engine::Mysql,
            "BEGIN;\nUPDATE `shop`.`orders` SET `total` = 2 WHERE `id` = 1;\nUPDATE `shop`.`orders` SET `total` = 3 WHERE `id` = 2;\nCOMMIT;",
        )
        .unwrap();
        assert_eq!(ops, vec!["db_query", "db_data"]);
        assert_eq!(
            operations(
                Engine::Postgres,
                "BEGIN; DELETE FROM shop.orders; ROLLBACK;"
            )
            .unwrap(),
            vec!["db_query", "db_data"]
        );
        // The wrapper never hides what runs inside it.
        assert!(
            operations(Engine::Mysql, "BEGIN; DROP TABLE shop.orders; COMMIT;")
                .unwrap()
                .contains(&"db_schema")
        );
        assert!(operations(Engine::Postgres, "BEGIN; SET ROLE owner; COMMIT;").is_err());
    }
    #[test]
    fn governed_sql_accounts_for_nested_writes_and_rejects_session_commands() {
        assert!(operations(
            Engine::Postgres,
            "WITH gone AS (DELETE FROM orders RETURNING *) SELECT * FROM gone"
        )
        .is_err());
        assert!(operations(Engine::Postgres, "SELECT 1; SET ROLE owner").is_err());
        assert!(operations(Engine::Postgres, "SELECT lo_create(0)").is_err());
        // A system-catalog read is no longer refused: it costs the
        // connection-wide `db_catalog` (authorized as `db_query`, no child).
        assert_eq!(
            operations(Engine::Postgres, "SELECT * FROM pg_catalog.pg_class").unwrap(),
            vec!["db_query", "db_catalog"]
        );
        assert!(operations(Engine::Postgres, "SELECT set_config('role','owner',false)").is_err());
        assert!(operations(
            Engine::Mysql,
            "SELECT * FROM shop.orders; UPDATE shop.orders SET total=2"
        )
        .unwrap()
        .contains(&"db_data"));
        assert_eq!(
            operations(
                Engine::Postgres,
                "SELECT count(*) FROM shop.orders WHERE total > 2"
            )
            .unwrap(),
            vec!["db_query"]
        );
    }
    #[test]
    fn everyday_read_only_reporting_sql_runs_in_restricted_mode() {
        // Real queries an agent was refused: per-day breakdowns and metadata.
        for sql in [
            "SELECT DATE(created_at) d, COUNT(*) FROM MdlGm_tblPlayers \
             WHERE created_at >= NOW() - INTERVAL 7 DAY GROUP BY DATE(created_at) ORDER BY d",
            "SELECT DATE_FORMAT(created_at, '%Y-%m-%d'), YEAR(created_at), MONTH(created_at), \
             IF(status = 1, 'on', 'off'), IFNULL(email, ''), DATEDIFF(NOW(), created_at), \
             CAST(created_at AS DATE) FROM MdlGm_tblPlayers",
        ] {
            assert_eq!(
                operations(Engine::Mysql, sql).unwrap(),
                vec!["db_query"],
                "{sql}"
            );
        }
        for sql in [
            "SELECT COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_NAME = 'MdlGm_tblPlayers'",
            "SHOW COLUMNS FROM MdlGm_tblPlayers",
            "DESCRIBE MdlGm_tblPlayers",
            "SHOW TABLES",
        ] {
            assert_eq!(
                operations(Engine::Mysql, sql).unwrap(),
                vec!["db_query", "db_catalog"],
                "{sql}"
            );
        }
        assert!(only_reads(&["db_query", "db_catalog"]));
        assert!(!only_reads(&["db_query", "db_data"]));
        assert_eq!(
            authorized_as("db_catalog", Some("shop")),
            ("db_query", None)
        );
        assert_eq!(
            authorized_as("db_query", Some("shop")),
            ("db_query", Some("shop"))
        );
        // Still refused: side effects and stored (qualified) functions, which can write.
        for sql in [
            "SELECT SLEEP(5)",
            "SELECT LOAD_FILE('/etc/passwd')",
            "SELECT GET_LOCK('x', 1)",
            "SELECT shop.give_bonus(1)",
            "SELECT my_udf(id) FROM t",
        ] {
            let err = operations(Engine::Mysql, sql).unwrap_err().to_string();
            assert!(
                err.contains("isn't on restricted SQL's list"),
                "{sql}: {err}"
            );
        }
    }
    #[test]
    fn governed_sql_rejects_table_functions_and_custom_casts() {
        for sql in [
            "SELECT * FROM lo_create(0)",
            "SELECT * FROM LATERAL lo_create(0)",
            "SELECT 'x'::dangerous_type",
        ] {
            assert!(operations(Engine::Postgres, sql).is_err(), "accepted {sql}");
        }
    }
}

#[cfg(test)]
mod select_into_regressions {
    use super::*;
    #[test]
    fn select_into_never_receives_read_only_authority() {
        for sql in [
            "SELECT 1 INTO shop.new_table",
            "WITH copied AS (SELECT 1 INTO shop.new_table) SELECT * FROM copied",
        ] {
            assert!(operations(Engine::Postgres, sql).is_err(), "{sql}");
        }
    }

    #[test]
    fn side_effect_functions_are_not_provable_reads() {
        for sql in [
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = current_user",
            "SELECT pg_cancel_backend(42)",
            "SELECT pg_reload_conf()",
            "SELECT pg_rotate_logfile()",
            "SELECT pg_switch_wal()",
            "SELECT pg_promote()",
            "SELECT pg_create_restore_point('x')",
            "SELECT dblink_exec('dbname=x', 'DELETE FROM t')",
            "SELECT * FROM dblink('dbname=x', 'SELECT 1') AS t(a int)",
            "SELECT lo_unlink(1)",
            "SELECT lo_import('/etc/passwd')",
            "SELECT pg_advisory_lock(1)",
            "SELECT pg_try_advisory_lock(1)",
            "SELECT pg_advisory_xact_lock_shared(1)",
            "SELECT set_config('search_path', 'x', false)",
            "SELECT pg_notify('ch', 'payload')",
            "SELECT pg_read_file('/etc/passwd')",
            "SELECT pg_read_binary_file('/etc/passwd')",
            "SELECT * FROM pg_ls_dir('.')",
            "SELECT pg_stat_reset()",
            "SELECT pg_logical_emit_message(true, 'p', 'x')",
            "SELECT nextval('orders_id_seq')",
            "SELECT setval('orders_id_seq', 1)",
            "SELECT pg_catalog.pg_terminate_backend(1)",
            "SELECT \"pg_terminate_backend\"(1)",
            "WITH k AS (SELECT pg_terminate_backend(1)) SELECT * FROM k",
            "SELECT 1 WHERE EXISTS (SELECT pg_cancel_backend(2))",
            "EXPLAIN SELECT pg_sleep(1)",
            // S6-301: built-ins that EXECUTE a query given as text hide any
            // call above behind a pure-looking outer SELECT.
            "SELECT query_to_xml('SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = current_user', true, false, '')",
            "SELECT query_to_xmlschema('SELECT 1', true, false, '')",
            "SELECT query_to_xml_and_xmlschema('SELECT 1', true, false, '')",
            "SELECT pg_catalog.query_to_xml('SELECT nextval(''s'')', true, false, '')",
            "SELECT cursor_to_xml('c', 10, true, false, '')",
            "SELECT * FROM ts_stat('SELECT pg_advisory_lock(1)::text::tsvector')",
            "SELECT ts_rewrite('a & b'::tsquery, 'SELECT t, s FROM aliases')",
            "SELECT pg_stat_statements_reset()",
            "SELECT pg_backup_start('x')",
            "SELECT pg_backup_stop()",
            "SELECT pg_start_backup('x')",
            "SELECT pg_stop_backup()",
            "SELECT pg_log_backend_memory_contexts(1)",
            // Any function handed SQL text is refused — even one we don't
            // know by name (an extension's executor).
            "SELECT my_ext.run_sql('SELECT pg_terminate_backend(1)')",
            "SELECT my_ext.run_sql($$DELETE FROM t$$)",
            "SELECT * FROM my_ext.exec_rows('select 1') AS t(a int)",
            "SELECT length(query_to_xml('SELECT 1', true, false, '')::text)",
        ] {
            assert!(!read_is_provable(Engine::Postgres, sql), "{sql}");
        }
        for sql in [
            "SELECT GET_LOCK('x', 10)",
            "SELECT RELEASE_LOCK('x')",
            "SELECT RELEASE_ALL_LOCKS()",
            "SELECT SLEEP(100)",
            "SELECT BENCHMARK(100000000, MD5('a'))",
            "SELECT LOAD_FILE('/etc/passwd')",
        ] {
            assert!(!read_is_provable(Engine::Mysql, sql), "{sql}");
        }
        // Ordinary function calls stay provable reads.
        assert!(read_is_provable(
            Engine::Postgres,
            "SELECT count(*), lower(name), now() FROM shop.orders"
        ));
        assert!(read_is_provable(
            Engine::Mysql,
            "SELECT CONCAT(a, b), COALESCE(c, 0) FROM t"
        ));
        // A string argument that merely starts with an SQL word but is not a
        // statement stays a read; a non-function literal is never inspected.
        assert!(read_is_provable(
            Engine::Postgres,
            "SELECT lower('Select, then confirm'), replace(name, 'update', 'x') FROM t"
        ));
        assert!(read_is_provable(
            Engine::Postgres,
            "SELECT * FROM logs WHERE msg = 'SELECT 1'"
        ));
        assert!(read_is_provable(
            Engine::Postgres,
            "SELECT table_to_xml('shop.orders', true, false, '')"
        ));
    }
}
