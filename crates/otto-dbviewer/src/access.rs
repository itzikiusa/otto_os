//! Resource authorization shared by HTTP, assistant, MCP, and dashboard paths.
use otto_core::access::{AccessMode, AccessPolicy, ResourceKind, ResourceRef, RuleEffect};
use otto_core::domain::{Capability, Feature};
use otto_core::domain::{Connection, User};
use otto_core::{Error, Id, Result};
use otto_rbac::resource_access::ResourceAccess;
use otto_state::{GrantsRepo, SqlitePool, UsersRepo, WorkspacesRepo};
use sqlparser::ast::{Expr, ObjectName, Statement, Visit, Visitor};
use sqlparser::dialect::{MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;
use std::ops::ControlFlow;

use crate::types::{Engine, NodePath};

pub(crate) fn target(conn: &Id, child: Option<&str>) -> ResourceRef {
    ResourceRef {
        kind: ResourceKind::Connection,
        id: conn.clone(),
        child: child.filter(|s| !s.is_empty()).map(str::to_owned),
    }
}

pub(crate) fn child(node: Option<&str>) -> Option<String> {
    node.filter(|n| !n.is_empty()).map(|n| {
        let path = NodePath::parse(n);
        path.get("db")
            .or_else(|| path.get("kdb"))
            .unwrap_or(n)
            .to_owned()
    })
}

pub(crate) async fn policy(pool: &SqlitePool, id: &Id) -> Result<AccessPolicy> {
    otto_state::resource_access::ResourceAccessRepo::new(pool.clone())
        .get_policy(ResourceKind::Connection, id)
        .await
}

/// Reload the effective user and membership for every action. The passed id is
/// supplied by authenticated adapters, never accepted from a request body.
pub(crate) async fn current_user(pool: &SqlitePool, conn: &Connection, id: &Id) -> Result<User> {
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
    pool: &SqlitePool,
    conn: &Connection,
    user_id: &Id,
    child: Option<&str>,
    operation: &str,
) -> Result<()> {
    if policy(pool, &conn.id).await?.mode == AccessMode::Legacy {
        return Ok(());
    }
    let user = current_user(pool, conn, user_id).await?;
    let access = ResourceAccess::new(pool.clone());
    if !access
        .evaluate(&user, &target(&conn.id, None), "discover")
        .await?
        .allowed
    {
        return Err(Error::NotFound("connection".into()));
    }
    access
        .check(&user, &target(&conn.id, child), operation)
        .await
}

/// Choose only credentials attached to matching Allow rules. Ambiguous profiles
/// (including an explicit primary profile plus an alternate) are rejected.
pub(crate) async fn credential_profile(
    pool: &SqlitePool,
    conn: &Connection,
    user_id: &Id,
    child: Option<&str>,
    operation: &str,
) -> Result<(Id, Option<String>)> {
    let policy = policy(pool, &conn.id).await?;
    if policy.mode == AccessMode::Legacy {
        return Ok((conn.id.clone(), None));
    }
    let user = current_user(pool, conn, user_id).await?;
    check(pool, conn, user_id, child, operation).await?;
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
        _ => {
            return Err(crate::native_access::setup_error(
                "restricted scripts are unsupported for this engine",
            ))
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
                ))
            }
        };
        if !ops.contains(&op) {
            ops.push(op);
        }
    }
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn governed_sql_accounts_for_nested_writes_and_rejects_session_commands() {
        assert!(operations(
            Engine::Postgres,
            "WITH gone AS (DELETE FROM orders RETURNING *) SELECT * FROM gone"
        )
        .is_err());
        assert!(operations(Engine::Postgres, "SELECT 1; SET ROLE owner").is_err());
        assert!(operations(Engine::Postgres, "SELECT lo_create(0)").is_err());
        assert!(operations(Engine::Postgres, "SELECT * FROM pg_catalog.pg_class").is_err());
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
        for sql in ["SELECT 1 INTO shop.new_table", "WITH copied AS (SELECT 1 INTO shop.new_table) SELECT * FROM copied"] {
            assert!(operations(Engine::Postgres,sql).is_err(),"{sql}");
        }
    }
}
