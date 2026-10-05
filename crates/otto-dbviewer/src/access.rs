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
struct SafeExpressions;
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
            if !matches!(
                name.as_str(),
                "count"
                    | "sum"
                    | "avg"
                    | "min"
                    | "max"
                    | "abs"
                    | "round"
                    | "floor"
                    | "ceil"
                    | "ceiling"
                    | "lower"
                    | "upper"
                    | "length"
                    | "char_length"
                    | "concat"
                    | "coalesce"
                    | "nullif"
                    | "now"
                    | "date_trunc"
                    | "date_part"
                    | "substring"
                    | "trim"
                    | "replace"
            ) {
                return ControlFlow::Break(Error::Forbidden(
                    "restricted SQL permits only verified built-in pure functions".into(),
                ));
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
            return ControlFlow::Break(Error::Forbidden(
                "native system catalogs are available only through filtered metadata APIs".into(),
            ));
        }
        ControlFlow::Continue(())
    }
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
        || name.contains("advisory")
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
        if let ControlFlow::Break(error) = statement.visit(&mut SafeExpressions) {
            return Err(error);
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
    fn governed_sql_accounts_for_nested_writes_and_rejects_session_commands() {
        assert!(
            operations(
                Engine::Postgres,
                "WITH gone AS (DELETE FROM orders RETURNING *) SELECT * FROM gone"
            )
            .is_err()
        );
        assert!(operations(Engine::Postgres, "SELECT 1; SET ROLE owner").is_err());
        assert!(operations(Engine::Postgres, "SELECT lo_create(0)").is_err());
        assert!(operations(Engine::Postgres, "SELECT * FROM pg_catalog.pg_class").is_err());
        assert!(operations(Engine::Postgres, "SELECT set_config('role','owner',false)").is_err());
        assert!(
            operations(
                Engine::Mysql,
                "SELECT * FROM shop.orders; UPDATE shop.orders SET total=2"
            )
            .unwrap()
            .contains(&"db_data")
        );
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
    }
}
