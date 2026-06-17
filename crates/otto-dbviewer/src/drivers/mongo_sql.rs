//! Best-effort SQL → MongoDB translation. Lets a SQL-comfortable user write a
//! `SELECT` against a Mongo collection; we translate to the runner's
//! `db.<coll>.find(...)` / `.aggregate([...])` / `.countDocuments(...)` shorthand
//! and surface the generated command back to the user.
//!
//! Supported: single-base-collection SELECT with column list or `*`, `WHERE`
//! (`= != <> < <= > >=`, `AND`/`OR`, `IN`/`NOT IN`, `BETWEEN`, `LIKE`,
//! `IS [NOT] NULL`, `NOT (...)`, parentheses), `ORDER BY`, `LIMIT`, `COUNT(*)`,
//! aggregates (`COUNT/SUM/AVG/MIN/MAX`, with or without `GROUP BY`), and
//! `INNER`/`LEFT` equi-`JOIN` (→ `$lookup` + `$unwind`). Unsupported constructs
//! (RIGHT/FULL/CROSS joins, non-equi join conditions, subqueries, UNION,
//! HAVING, DISTINCT) return a clear error so the user can fall back to native
//! Mongo.

use std::collections::HashSet;

use serde_json::{json, Map, Value as J};
use sqlparser::ast::{
    BinaryOperator, Expr, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr, Ident,
    Join, JoinConstraint, JoinOperator, LimitClause, OrderByKind, Query, Select, SelectItem,
    SetExpr, Statement, TableFactor, UnaryOperator, Value as SqlValue,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;

use crate::types;
use otto_core::Result;

/// Qualifiers to strip during field resolution: the base collection's name and
/// alias map to top-level fields, so `a.x` (a = base) becomes `x`. Joined
/// collections keep their qualifier (`b.x` stays `b.x`, the `$lookup` subdoc).
struct Ctx {
    base_names: HashSet<String>,
}

/// True when a statement should be treated as SQL to translate (vs native Mongo
/// shorthand / a JSON command).
pub fn looks_like_sql(statement: &str) -> bool {
    let t = statement.trim_start();
    let head: String = t.chars().take(6).collect::<String>().to_ascii_lowercase();
    head == "select"
}

/// Translate a SQL `SELECT` into the Mongo runner shorthand. Returns the
/// generated command string (also shown to the user).
pub fn translate(sql: &str) -> Result<String> {
    let dialect = GenericDialect {};
    let statements =
        Parser::parse_sql(&dialect, sql).map_err(|e| types::invalid(format!("SQL parse error: {e}")))?;
    if statements.len() != 1 {
        return Err(types::invalid("expected a single SELECT statement"));
    }
    let query = match &statements[0] {
        Statement::Query(q) => q,
        _ => return Err(types::invalid("only SELECT statements translate to Mongo")),
    };
    let select = match query.body.as_ref() {
        SetExpr::Select(s) => s.as_ref(),
        _ => return Err(types::invalid("UNION/set queries are not supported")),
    };
    if select.from.len() != 1 {
        return Err(types::invalid("SQL→Mongo supports a single base collection"));
    }
    if select.having.is_some() {
        return Err(types::invalid("HAVING is not supported"));
    }

    let twj = &select.from[0];
    let base = table_name(&twj.relation)?;
    let mut base_names = HashSet::new();
    base_names.insert(base.clone());
    if let Some(a) = table_alias(&twj.relation) {
        base_names.insert(a);
    }
    let ctx = Ctx { base_names };

    // JOINs force an aggregation pipeline (find cannot join).
    if !twj.joins.is_empty() {
        return build_join_query(&base, &ctx, &twj.joins, select, query);
    }

    let filter = match &select.selection {
        Some(expr) => expr_to_filter(expr, &ctx)?,
        None => J::Object(Map::new()),
    };
    let sort = order_by_doc(query, &ctx)?;
    let limit = limit_value(query)?;
    let group_keys = group_by_keys(&select.group_by, &ctx)?;

    if !group_keys.is_empty() {
        let stages = aggregate_stages(&filter, &select.projection, &group_keys, true, &sort, limit, &ctx)?;
        return Ok(format!("db.{base}.aggregate({})", compact(&J::Array(stages))));
    }
    if is_count_star_only(&select.projection) {
        return Ok(format!("db.{base}.countDocuments({})", compact(&filter)));
    }
    if projection_has_aggregate(&select.projection) {
        let stages = aggregate_stages(&filter, &select.projection, &[], true, &sort, limit, &ctx)?;
        return Ok(format!("db.{base}.aggregate({})", compact(&J::Array(stages))));
    }

    // Plain find with projection / sort / limit.
    let projection = projection_doc(&select.projection, &ctx)?;
    let mut out = format!("db.{base}.find({}", compact(&filter));
    if let Some(p) = &projection {
        out.push_str(&format!(", {}", compact(p)));
    }
    out.push(')');
    if let Some(s) = &sort {
        out.push_str(&format!(".sort({})", compact(s)));
    }
    if let Some(n) = limit {
        out.push_str(&format!(".limit({n})"));
    }
    Ok(out)
}

fn compact(v: &J) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".into())
}

// --- table / alias ----------------------------------------------------------

fn table_name(tf: &TableFactor) -> Result<String> {
    match tf {
        TableFactor::Table { name, .. } => {
            let last = name
                .0
                .last()
                .ok_or_else(|| types::invalid("missing collection name"))?;
            Ok(last
                .to_string()
                .trim_matches('"')
                .trim_matches('`')
                .to_string())
        }
        _ => Err(types::invalid("FROM must reference a single collection")),
    }
}

fn table_alias(tf: &TableFactor) -> Option<String> {
    match tf {
        TableFactor::Table { alias, .. } => alias.as_ref().map(|a| a.name.value.clone()),
        _ => None,
    }
}

// --- JOIN → $lookup + $unwind -----------------------------------------------

fn build_join_query(
    base: &str,
    ctx: &Ctx,
    joins: &[Join],
    select: &Select,
    query: &Query,
) -> Result<String> {
    let mut pipeline: Vec<J> = Vec::new();
    for join in joins {
        let coll = table_name(&join.relation)?;
        let alias = table_alias(&join.relation).unwrap_or_else(|| coll.clone());
        let (left_outer, constraint) = match &join.join_operator {
            JoinOperator::Inner(c) | JoinOperator::Join(c) => (false, c),
            JoinOperator::Left(c) | JoinOperator::LeftOuter(c) => (true, c),
            _ => {
                return Err(types::invalid(
                    "only INNER and LEFT JOIN translate to Mongo ($lookup)",
                ))
            }
        };
        let on = match constraint {
            JoinConstraint::On(expr) => expr,
            _ => return Err(types::invalid("JOIN must use an ON <field> = <field> condition")),
        };
        let (local, foreign) = equi_on(on, ctx, &alias)?;
        pipeline.push(json!({
            "$lookup": { "from": coll, "localField": local, "foreignField": foreign, "as": alias }
        }));
        pipeline.push(if left_outer {
            json!({ "$unwind": { "path": format!("${alias}"), "preserveNullAndEmptyArrays": true } })
        } else {
            json!({ "$unwind": format!("${alias}") })
        });
    }

    let filter = match &select.selection {
        Some(expr) => expr_to_filter(expr, ctx)?,
        None => J::Object(Map::new()),
    };
    let sort = order_by_doc(query, ctx)?;
    let limit = limit_value(query)?;
    let group_keys = group_by_keys(&select.group_by, ctx)?;
    let has_agg = !group_keys.is_empty() || projection_has_aggregate(&select.projection);

    let stages = aggregate_stages(&filter, &select.projection, &group_keys, has_agg, &sort, limit, ctx)?;
    pipeline.extend(stages);
    Ok(format!("db.{base}.aggregate({})", compact(&J::Array(pipeline))))
}

/// Resolve a join's `ON a.x = b.y` into `(localField, foreignField)` where
/// `b`/`alias` is the joined collection.
fn equi_on(on: &Expr, ctx: &Ctx, join_alias: &str) -> Result<(String, String)> {
    let (left, right) = match on {
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => (left.as_ref(), right.as_ref()),
        _ => return Err(types::invalid("JOIN ON must be a single <field> = <field> equality")),
    };
    let (lq, lf) = qualified(left)?;
    let (rq, rf) = qualified(right)?;
    if rq.as_deref() == Some(join_alias) {
        Ok((resolve_local(lq, lf, ctx), rf))
    } else if lq.as_deref() == Some(join_alias) {
        Ok((resolve_local(rq, rf, ctx), lf))
    } else {
        Err(types::invalid(
            "JOIN ON must reference the joined collection (e.g. base.x = joined.y)",
        ))
    }
}

/// `(qualifier, field)` for an identifier used in a join condition.
fn qualified(expr: &Expr) -> Result<(Option<String>, String)> {
    match expr {
        Expr::Identifier(id) => Ok((None, id.value.clone())),
        Expr::CompoundIdentifier(parts) if parts.len() >= 2 => {
            let q = parts[0].value.clone();
            let f = parts[1..].iter().map(|p| p.value.clone()).collect::<Vec<_>>().join(".");
            Ok((Some(q), f))
        }
        Expr::Nested(inner) => qualified(inner),
        _ => Err(types::invalid("JOIN ON operands must be column references")),
    }
}

/// Local-side field path: strip a base-table qualifier; keep a prior join alias.
fn resolve_local(q: Option<String>, f: String, ctx: &Ctx) -> String {
    match q {
        None => f,
        Some(q) if ctx.base_names.contains(&q) => f,
        Some(q) => format!("{q}.{f}"),
    }
}

// --- WHERE → filter ---------------------------------------------------------

fn expr_to_filter(expr: &Expr, ctx: &Ctx) -> Result<J> {
    match expr {
        Expr::Nested(inner) => expr_to_filter(inner, ctx),
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr,
        } => Ok(json!({ "$nor": [expr_to_filter(expr, ctx)?] })),
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::And => {
                let l = expr_to_filter(left, ctx)?;
                let r = expr_to_filter(right, ctx)?;
                Ok(merge_and(l, r))
            }
            BinaryOperator::Or => {
                let l = expr_to_filter(left, ctx)?;
                let r = expr_to_filter(right, ctx)?;
                Ok(json!({ "$or": [l, r] }))
            }
            BinaryOperator::Eq => {
                let f = field_path(left, ctx)?;
                Ok(json!({ f: expr_to_json(right)? }))
            }
            BinaryOperator::NotEq => cmp(left, right, "$ne", ctx),
            BinaryOperator::Gt => cmp(left, right, "$gt", ctx),
            BinaryOperator::GtEq => cmp(left, right, "$gte", ctx),
            BinaryOperator::Lt => cmp(left, right, "$lt", ctx),
            BinaryOperator::LtEq => cmp(left, right, "$lte", ctx),
            other => Err(types::invalid(format!("unsupported operator: {other:?}"))),
        },
        Expr::IsNull(inner) => Ok(json!({ field_path(inner, ctx)?: J::Null })),
        Expr::IsNotNull(inner) => Ok(json!({ field_path(inner, ctx)?: { "$ne": J::Null } })),
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            let vals: Result<Vec<J>> = list.iter().map(expr_to_json).collect();
            let key = if *negated { "$nin" } else { "$in" };
            Ok(json!({ field_path(expr, ctx)?: { key: J::Array(vals?) } }))
        }
        Expr::Between {
            expr,
            negated,
            low,
            high,
        } => {
            let f = field_path(expr, ctx)?;
            if *negated {
                Ok(json!({ "$or": [
                    { f.clone(): { "$lt": expr_to_json(low)? } },
                    { f: { "$gt": expr_to_json(high)? } },
                ] }))
            } else {
                Ok(json!({ f: { "$gte": expr_to_json(low)?, "$lte": expr_to_json(high)? } }))
            }
        }
        Expr::Like {
            negated,
            expr,
            pattern,
            ..
        } => {
            let f = field_path(expr, ctx)?;
            let pat = match expr_to_json(pattern)? {
                J::String(s) => like_to_regex(&s),
                _ => return Err(types::invalid("LIKE pattern must be a string")),
            };
            let cond = json!({ "$regex": pat, "$options": "i" });
            if *negated {
                Ok(json!({ f: { "$not": cond } }))
            } else {
                Ok(json!({ f: cond }))
            }
        }
        other => Err(types::invalid(format!(
            "unsupported WHERE expression: {other:?}"
        ))),
    }
}

fn cmp(left: &Expr, right: &Expr, op: &str, ctx: &Ctx) -> Result<J> {
    let f = field_path(left, ctx)?;
    Ok(json!({ f: { op: expr_to_json(right)? } }))
}

/// Merge two AND operands; on key collision fall back to `$and`.
fn merge_and(l: J, r: J) -> J {
    if let (J::Object(mut a), J::Object(b)) = (l.clone(), r.clone()) {
        if !b.keys().any(|k| a.contains_key(k)) {
            for (k, v) in b {
                a.insert(k, v);
            }
            return J::Object(a);
        }
    }
    json!({ "$and": [l, r] })
}

/// A field reference (identifier or dotted path) as a Mongo field path; the
/// base-table qualifier is stripped, joined-collection qualifiers are kept.
fn field_path(expr: &Expr, ctx: &Ctx) -> Result<String> {
    match expr {
        Expr::Identifier(id) => Ok(id.value.clone()),
        Expr::CompoundIdentifier(parts) => {
            let names: Vec<String> = parts.iter().map(|p: &Ident| p.value.clone()).collect();
            if names.len() >= 2 && ctx.base_names.contains(&names[0]) {
                Ok(names[1..].join("."))
            } else {
                Ok(names.join("."))
            }
        }
        Expr::Nested(inner) => field_path(inner, ctx),
        _ => Err(types::invalid("expected a column reference")),
    }
}

fn expr_to_json(expr: &Expr) -> Result<J> {
    match expr {
        Expr::Value(v) => value_to_json(&v.value),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr,
        } => match expr_to_json(expr)? {
            J::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(json!(-i))
                } else if let Some(f) = n.as_f64() {
                    Ok(json!(-f))
                } else {
                    Err(types::invalid("bad numeric literal"))
                }
            }
            _ => Err(types::invalid("unary minus on non-number")),
        },
        Expr::Identifier(id) => Ok(J::String(id.value.clone())),
        _ => Err(types::invalid(format!("unsupported value expression: {expr:?}"))),
    }
}

fn value_to_json(v: &SqlValue) -> Result<J> {
    match v {
        SqlValue::Number(n, _) => serde_json::from_str::<J>(n)
            .map_err(|_| types::invalid(format!("bad number literal: {n}"))),
        SqlValue::SingleQuotedString(s) | SqlValue::DoubleQuotedString(s) => Ok(J::String(s.clone())),
        SqlValue::Boolean(b) => Ok(J::Bool(*b)),
        SqlValue::Null => Ok(J::Null),
        other => Err(types::invalid(format!("unsupported literal: {other:?}"))),
    }
}

/// Convert a SQL LIKE pattern into an anchored regex.
fn like_to_regex(pat: &str) -> String {
    let mut out = String::from("^");
    for ch in pat.chars() {
        match ch {
            '%' => out.push_str(".*"),
            '_' => out.push('.'),
            '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('$');
    out
}

// --- ORDER BY / LIMIT -------------------------------------------------------

fn order_by_doc(query: &Query, ctx: &Ctx) -> Result<Option<J>> {
    let Some(ob) = &query.order_by else {
        return Ok(None);
    };
    let exprs = match &ob.kind {
        OrderByKind::Expressions(e) => e,
        OrderByKind::All(_) => return Err(types::invalid("ORDER BY ALL is not supported")),
    };
    let mut m = Map::new();
    for o in exprs {
        let dir = if o.options.asc == Some(false) { -1 } else { 1 };
        m.insert(field_path(&o.expr, ctx)?, json!(dir));
    }
    Ok(if m.is_empty() { None } else { Some(J::Object(m)) })
}

fn limit_value(query: &Query) -> Result<Option<i64>> {
    let Some(lc) = &query.limit_clause else {
        return Ok(None);
    };
    match lc {
        LimitClause::LimitOffset {
            limit: Some(e), ..
        } => match expr_to_json(e)? {
            J::Number(n) => Ok(n.as_i64()),
            _ => Err(types::invalid("LIMIT must be an integer")),
        },
        _ => Ok(None),
    }
}

// --- projection / COUNT -----------------------------------------------------

fn is_count_star_only(items: &[SelectItem]) -> bool {
    items.len() == 1 && matches!(&items[0], SelectItem::UnnamedExpr(e) if is_count_star(e))
}

/// True when any SELECT item is an aggregate function call.
fn projection_has_aggregate(items: &[SelectItem]) -> bool {
    items.iter().any(|it| {
        let expr = match it {
            SelectItem::UnnamedExpr(e) => e,
            SelectItem::ExprWithAlias { expr, .. } => expr,
            _ => return false,
        };
        if let Expr::Function(f) = expr {
            matches!(
                f.name.to_string().to_ascii_lowercase().as_str(),
                "count" | "sum" | "avg" | "min" | "max"
            )
        } else {
            false
        }
    })
}

fn is_count_star(expr: &Expr) -> bool {
    if let Expr::Function(f) = expr {
        if f.name.to_string().to_ascii_lowercase() == "count" {
            if let FunctionArguments::List(list) = &f.args {
                return list.args.len() == 1
                    && matches!(
                        &list.args[0],
                        FunctionArg::Unnamed(FunctionArgExpr::Wildcard)
                    );
            }
        }
    }
    false
}

/// A Mongo projection from the SELECT list (None = all fields / `*`).
fn projection_doc(items: &[SelectItem], ctx: &Ctx) -> Result<Option<J>> {
    if items
        .iter()
        .any(|i| matches!(i, SelectItem::Wildcard(_) | SelectItem::QualifiedWildcard(_, _)))
    {
        return Ok(None);
    }
    let mut m = Map::new();
    for it in items {
        let expr = match it {
            SelectItem::UnnamedExpr(e) => e,
            SelectItem::ExprWithAlias { expr, .. } => expr,
            _ => return Err(types::invalid("unsupported SELECT item")),
        };
        m.insert(field_path(expr, ctx)?, json!(1));
    }
    Ok(if m.is_empty() { None } else { Some(J::Object(m)) })
}

// --- aggregate stages -------------------------------------------------------

fn group_by_keys(gb: &GroupByExpr, ctx: &Ctx) -> Result<Vec<String>> {
    match gb {
        GroupByExpr::Expressions(exprs, _) => exprs.iter().map(|e| field_path(e, ctx)).collect(),
        GroupByExpr::All(_) => Err(types::invalid("GROUP BY ALL is not supported")),
    }
}

/// The `$match → ($group | $project) → $sort → $limit` tail shared by the plain
/// aggregate path and the JOIN pipeline.
fn aggregate_stages(
    filter: &J,
    items: &[SelectItem],
    group_keys: &[String],
    has_agg: bool,
    sort: &Option<J>,
    limit: Option<i64>,
    ctx: &Ctx,
) -> Result<Vec<J>> {
    let mut stages: Vec<J> = Vec::new();
    if filter.as_object().map(|o| !o.is_empty()).unwrap_or(false) {
        stages.push(json!({ "$match": filter }));
    }
    if has_agg {
        stages.push(group_stage(items, group_keys, ctx)?);
    } else if let Some(proj) = projection_doc(items, ctx)? {
        stages.push(json!({ "$project": proj }));
    }
    if let Some(s) = sort {
        stages.push(json!({ "$sort": s }));
    }
    if let Some(n) = limit {
        stages.push(json!({ "$limit": n }));
    }
    Ok(stages)
}

fn group_stage(items: &[SelectItem], group_keys: &[String], ctx: &Ctx) -> Result<J> {
    let id_val: J = if group_keys.is_empty() {
        J::Null
    } else if group_keys.len() == 1 {
        J::String(format!("${}", group_keys[0]))
    } else {
        let mut m = Map::new();
        for k in group_keys {
            m.insert(k.clone(), J::String(format!("${k}")));
        }
        J::Object(m)
    };
    let mut group = Map::new();
    group.insert("_id".into(), id_val);

    for it in items {
        let (expr, alias) = match it {
            SelectItem::UnnamedExpr(e) => (e, None),
            SelectItem::ExprWithAlias { expr, alias } => (expr, Some(alias.value.clone())),
            _ => return Err(types::invalid("unsupported SELECT item in aggregate")),
        };
        if let Ok(fp) = field_path(expr, ctx) {
            if group_keys.contains(&fp) {
                continue; // group-key column is represented by _id
            }
        }
        let (name, acc) = accumulator(expr, alias, ctx)?;
        group.insert(name, acc);
    }
    Ok(json!({ "$group": group }))
}

/// An aggregate function in the SELECT list → `(output_name, { $op: arg })`.
fn accumulator(expr: &Expr, alias: Option<String>, ctx: &Ctx) -> Result<(String, J)> {
    let Expr::Function(f) = expr else {
        return Err(types::invalid("non-aggregated column must appear in GROUP BY"));
    };
    let fname = f.name.to_string().to_ascii_lowercase();
    let arg_field = first_func_field(f, ctx);
    let field_ref = || json!(format!("${}", arg_field.clone().unwrap_or_default()));

    let (op, value): (&str, J) = match fname.as_str() {
        "count" => ("$sum", json!(1)),
        "sum" => ("$sum", field_ref()),
        "avg" => ("$avg", field_ref()),
        "min" => ("$min", field_ref()),
        "max" => ("$max", field_ref()),
        other => return Err(types::invalid(format!("unsupported aggregate: {other}"))),
    };
    let name = alias.unwrap_or_else(|| {
        if fname == "count" {
            "count".into()
        } else {
            format!("{fname}_{}", arg_field.unwrap_or_else(|| "value".into()))
        }
    });
    Ok((name, json!({ op: value })))
}

fn first_func_field(f: &sqlparser::ast::Function, ctx: &Ctx) -> Option<String> {
    if let FunctionArguments::List(list) = &f.args {
        for a in &list.args {
            if let FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) = a {
                if let Ok(fp) = field_path(e, ctx) {
                    return Some(fp);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(sql: &str) -> String {
        translate(sql).unwrap()
    }

    #[test]
    fn looks_like_sql_detects_select() {
        assert!(looks_like_sql("SELECT * FROM c"));
        assert!(looks_like_sql("  select a from c"));
        assert!(!looks_like_sql("db.c.find({})"));
        assert!(!looks_like_sql("{ \"collection\": \"c\" }"));
    }

    #[test]
    fn simple_find_with_where_sort_limit() {
        assert_eq!(
            t("SELECT a, b FROM users WHERE age > 21 AND status = 'active' ORDER BY age DESC LIMIT 10"),
            r#"db.users.find({"age":{"$gt":21},"status":"active"}, {"a":1,"b":1}).sort({"age":-1}).limit(10)"#
        );
    }

    #[test]
    fn star_with_in_and_like() {
        assert_eq!(
            t("SELECT * FROM players WHERE country IN ('US','CA') AND name LIKE 'jo%'"),
            r#"db.players.find({"country":{"$in":["US","CA"]},"name":{"$regex":"^jo.*$","$options":"i"}})"#
        );
    }

    #[test]
    fn count_star() {
        assert_eq!(
            t("SELECT COUNT(*) FROM events WHERE kind = 'bet'"),
            r#"db.events.countDocuments({"kind":"bet"})"#
        );
    }

    #[test]
    fn group_by_aggregate() {
        assert_eq!(
            t("SELECT country, COUNT(*) AS n FROM players GROUP BY country ORDER BY n DESC LIMIT 5"),
            r#"db.players.aggregate([{"$group":{"_id":"$country","n":{"$sum":1}}},{"$sort":{"n":-1}},{"$limit":5}])"#
        );
    }

    #[test]
    fn global_aggregate_no_group_by() {
        assert_eq!(
            t("SELECT COUNT(*) AS total, AVG(amount) AS avg_amt FROM bets WHERE status = 'won'"),
            r#"db.bets.aggregate([{"$match":{"status":"won"}},{"$group":{"_id":null,"total":{"$sum":1},"avg_amt":{"$avg":"$amount"}}}])"#
        );
    }

    #[test]
    fn not_negation() {
        assert_eq!(
            t("SELECT * FROM c WHERE NOT (status = 'x')"),
            r#"db.c.find({"$nor":[{"status":"x"}]})"#
        );
    }

    #[test]
    fn inner_join_lookup() {
        assert_eq!(
            t("SELECT p.name, a.amount FROM players p JOIN accounts a ON p.id = a.player_id WHERE a.amount > 100"),
            r#"db.players.aggregate([{"$lookup":{"from":"accounts","localField":"id","foreignField":"player_id","as":"a"}},{"$unwind":"$a"},{"$match":{"a.amount":{"$gt":100}}},{"$project":{"name":1,"a.amount":1}}])"#
        );
    }

    #[test]
    fn left_join_preserves_nulls() {
        assert_eq!(
            t("SELECT * FROM orders o LEFT JOIN customers c ON o.cust_id = c.id"),
            r#"db.orders.aggregate([{"$lookup":{"from":"customers","localField":"cust_id","foreignField":"id","as":"c"}},{"$unwind":{"path":"$c","preserveNullAndEmptyArrays":true}}])"#
        );
    }

    #[test]
    fn rejects_right_join() {
        assert!(translate("SELECT * FROM a RIGHT JOIN b ON a.id = b.id").is_err());
    }
}
