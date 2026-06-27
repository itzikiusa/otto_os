//! Pure SQL context analysis + completion assembly (MySQL + ClickHouse).
//!
//! Given the text before the cursor (`prefix`) and after it within the same
//! statement (`suffix`), decide what the cursor expects — a table after `FROM`,
//! a column after `WHERE`, a member after `alias.` — and which tables are in
//! scope (resolved from the whole statement's `FROM`/`JOIN` list, so a column in
//! the `SELECT` list still knows its table even though `FROM` comes after it).
//!
//! This is intentionally a *heuristic* tokenizer, not a full SQL grammar: it is
//! cheap, never blocks, and degrades to [`SqlExpect::Any`] (the
//! keywords/functions/tables dump that was the previous behaviour) on anything
//! it doesn't recognise — so it is never worse than before.

use super::{rank_score, score, ObjKind, SchemaSnapshot};
use crate::types::{CompletionItem, CompletionKind};

/// What identifier the cursor position expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlExpect {
    /// A table/view name (after `FROM`/`JOIN`/`UPDATE`/`INSERT INTO`).
    Table,
    /// A column name. `qualifier` is the `alias.`/`table.` before the cursor.
    Column { qualifier: Option<String> },
    /// Unknown — offer keywords + functions + tables (never worse than before).
    Any,
}

/// A table reference parsed from a `FROM`/`JOIN`/`UPDATE`/`INTO` clause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRef {
    pub name: String,
    pub alias: Option<String>,
}

/// The analysed cursor context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlCtx {
    pub expect: SqlExpect,
    pub tables: Vec<TableRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(String),
    Punct(char),
}

/// Analyse the cursor context. `prefix` is everything up to the cursor; `suffix`
/// is everything after it (the caller scopes both to the current statement, but
/// we defensively re-scope to the last/first top-level statement anyway).
pub fn analyze(prefix: &str, suffix: &str) -> SqlCtx {
    let stmt_prefix = last_statement(prefix);
    let stmt_suffix = first_statement(suffix);
    let full = format!("{stmt_prefix}{stmt_suffix}");

    let prefix_toks = tokenize(stmt_prefix);
    let full_toks = tokenize(&full);

    let tables = extract_tables(&full_toks);
    let qualifier = trailing_qualifier(&prefix_toks);
    let base = clause_expectation(&prefix_toks);

    let expect = match (base, qualifier) {
        // `db.` in a table slot → still a table (we suggest tables in the active
        // db; cross-db qualification just narrows what the user types).
        (Base::Table, _) => SqlExpect::Table,
        (Base::Column, q) => SqlExpect::Column { qualifier: q },
        (Base::Any, Some(q)) => SqlExpect::Column { qualifier: Some(q) },
        (Base::Any, None) => SqlExpect::Any,
    };

    SqlCtx { expect, tables }
}

// --- assembly ---------------------------------------------------------------

/// Build the ranked completion list for a SQL cursor context.
pub fn assemble(
    ctx: &SqlCtx,
    snap: &SchemaSnapshot,
    keywords: &[&str],
    functions: &[(&str, &str)],
) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = Vec::new();
    match &ctx.expect {
        SqlExpect::Table => {
            push_tables(&mut items, snap);
            push_databases(&mut items, snap, score::DATABASE);
            push_keywords(&mut items, keywords);
        }
        SqlExpect::Column { qualifier } => {
            push_columns(&mut items, ctx, snap, qualifier.as_deref());
            push_functions(&mut items, functions);
            push_keywords(&mut items, keywords);
        }
        SqlExpect::Any => {
            push_keywords(&mut items, keywords);
            push_functions(&mut items, functions);
            push_tables(&mut items, snap);
            push_databases(&mut items, snap, score::DATABASE);
        }
    }
    items
}

fn push_tables(items: &mut Vec<CompletionItem>, snap: &SchemaSnapshot) {
    for o in &snap.objects {
        let kind = match o.kind {
            ObjKind::View => CompletionKind::View,
            _ => CompletionKind::Table,
        };
        items.push(CompletionItem::new(o.name.clone(), kind).scored(score::TABLE));
    }
}

fn push_databases(items: &mut Vec<CompletionItem>, snap: &SchemaSnapshot, sc: i32) {
    for db in &snap.databases {
        items.push(CompletionItem::new(db.clone(), CompletionKind::Database).scored(sc));
    }
}

fn push_keywords(items: &mut Vec<CompletionItem>, keywords: &[&str]) {
    for kw in keywords {
        items.push(CompletionItem::new(*kw, CompletionKind::Keyword).scored(score::KEYWORD));
    }
}

fn push_functions(items: &mut Vec<CompletionItem>, functions: &[(&str, &str)]) {
    for (name, sig) in functions {
        items.push(
            CompletionItem::detailed(*name, CompletionKind::Function, *sig).scored(score::FUNCTION),
        );
    }
}

/// Resolve and push the in-scope columns (index-first). When the scope can't be
/// resolved (no parseable `FROM`, or an unknown qualifier) we fall back to every
/// column at a low score so a bare `WHERE` still completes *something*.
fn push_columns(
    items: &mut Vec<CompletionItem>,
    ctx: &SqlCtx,
    snap: &SchemaSnapshot,
    qualifier: Option<&str>,
) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pushed_in_scope = false;

    let targets: Vec<&str> = if let Some(q) = qualifier {
        // Resolve the qualifier as an alias first, then as a table name.
        match resolve_alias(ctx, q) {
            Some(name) => vec![name],
            None => vec![q],
        }
    } else {
        // All in-scope tables.
        ctx.tables.iter().map(|t| t.name.as_str()).collect()
    };

    for tname in &targets {
        if let Some(obj) = snap.object(tname) {
            for f in &obj.fields {
                let key = f.name.to_ascii_lowercase();
                if !seen.insert(key) {
                    continue; // same column in two joined tables — keep highest rank (first wins; fields are index-first)
                }
                items.push(column_item(f, rank_score(f.rank)));
                pushed_in_scope = true;
            }
        }
    }

    // Fallback: nothing resolved → offer every column weakly (still useful).
    if !pushed_in_scope {
        for obj in &snap.objects {
            for f in &obj.fields {
                let key = format!(
                    "{}\u{0}{}",
                    obj.name.to_ascii_lowercase(),
                    f.name.to_ascii_lowercase()
                );
                if !seen.insert(key) {
                    continue;
                }
                items.push(column_item(f, score::OUT_OF_SCOPE_COL));
            }
        }
    }
}

fn column_item(f: &super::FieldSnap, sc: i32) -> CompletionItem {
    let detail = match (&f.ty, f.rank.label()) {
        (Some(ty), Some(lbl)) => Some(format!("{ty} · {lbl}")),
        (Some(ty), None) => Some(ty.clone()),
        (None, Some(lbl)) => Some(lbl.to_string()),
        (None, None) => None,
    };
    let mut item = CompletionItem::new(f.name.clone(), CompletionKind::Column).scored(sc);
    item.detail = detail;
    item
}

/// Resolve a qualifier (alias or table) to a table name from the FROM list.
fn resolve_alias<'a>(ctx: &'a SqlCtx, q: &str) -> Option<&'a str> {
    let ql = q.to_ascii_lowercase();
    for t in &ctx.tables {
        if let Some(a) = &t.alias {
            if a.eq_ignore_ascii_case(&ql) {
                return Some(t.name.as_str());
            }
        }
        if t.name.eq_ignore_ascii_case(&ql) {
            return Some(t.name.as_str());
        }
    }
    None
}

// --- clause / qualifier detection -------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Base {
    Table,
    Column,
    Any,
}

/// The expectation implied by the last clause keyword before the cursor.
fn clause_expectation(toks: &[Tok]) -> Base {
    let mut current = Base::Any;
    let mut last_kw = String::new();
    for t in toks {
        match t {
            Tok::Word(w) => {
                let up = w.to_ascii_uppercase();
                if let Some(b) = clause_of(&up) {
                    current = b;
                    last_kw = up;
                }
            }
            // `INSERT INTO t (col, …)` — the paren list is columns, not a table.
            Tok::Punct('(') if last_kw == "INTO" && current == Base::Table => {
                current = Base::Column;
            }
            _ => {}
        }
    }
    current
}

/// Map a clause-starting keyword to the expectation it introduces.
fn clause_of(up: &str) -> Option<Base> {
    Some(match up {
        "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE" | "DESCRIBE" | "DESC" => Base::Table,
        "SELECT" | "WHERE" | "ON" | "AND" | "OR" | "HAVING" | "SET" | "BY" | "USING"
        | "RETURNING" => Base::Column,
        // Keywords that END a useful column/table slot. `GROUP`/`ORDER` are
        // followed by `BY` (which flips back to Column), so until then they read
        // as Any rather than suggesting columns mid-keyword.
        "VALUES" | "LIMIT" | "OFFSET" | "INSERT" | "DELETE" | "GROUP" | "ORDER" => Base::Any,
        _ => return None,
    })
}

/// The `alias.`/`table.` immediately before the cursor, if any.
fn trailing_qualifier(toks: &[Tok]) -> Option<String> {
    match toks {
        // `t.` (cursor right after the dot)
        [.., Tok::Word(q), Tok::Punct('.')] => Some(q.clone()),
        // `t.partial` (cursor inside the member word)
        [.., Tok::Word(q), Tok::Punct('.'), Tok::Word(_)] => Some(q.clone()),
        _ => None,
    }
}

// --- table extraction --------------------------------------------------------

/// Collect table refs from every `FROM`/`JOIN`/`UPDATE`/`INSERT INTO`/`DELETE
/// FROM` clause in the statement.
fn extract_tables(toks: &[Tok]) -> Vec<TableRef> {
    let mut out: Vec<TableRef> = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        if let Tok::Word(w) = &toks[i] {
            let up = w.to_ascii_uppercase();
            let is_from_like = up == "FROM" || up == "JOIN" || up == "UPDATE" || up == "INTO";
            if is_from_like {
                i += 1;
                // FROM/UPDATE/INTO may list comma-separated tables; JOIN is one.
                let multi = up == "FROM";
                loop {
                    // Skip a leading subquery `( … )` → derived table (no columns).
                    if matches!(toks.get(i), Some(Tok::Punct('('))) {
                        i = skip_parens(toks, i);
                        // optional alias after the subquery
                        let (alias, ni) = read_alias(toks, i);
                        i = ni;
                        if let Some(a) = alias {
                            out.push(TableRef {
                                name: a.clone(),
                                alias: Some(a),
                            });
                        }
                    } else if let Some(name) = read_table_name(toks, &mut i) {
                        let (alias, ni) = read_alias(toks, i);
                        i = ni;
                        out.push(TableRef { name, alias });
                    } else {
                        break;
                    }
                    // Continue a comma list only for FROM/UPDATE/INTO.
                    if multi && matches!(toks.get(i), Some(Tok::Punct(','))) {
                        i += 1;
                        continue;
                    }
                    break;
                }
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Read a (possibly `db.table`) name at `*i`, advancing past it. Returns the
/// last identifier (the table). `None` if the current token isn't a name.
fn read_table_name(toks: &[Tok], i: &mut usize) -> Option<String> {
    let first = match toks.get(*i) {
        Some(Tok::Word(w)) if !is_reserved(w) => w.clone(),
        _ => return None,
    };
    *i += 1;
    let mut name = first;
    // Consume `.ident` chains, keeping the LAST segment as the table name.
    while matches!(toks.get(*i), Some(Tok::Punct('.'))) {
        if let Some(Tok::Word(w)) = toks.get(*i + 1) {
            name = w.clone();
            *i += 2;
        } else {
            break;
        }
    }
    Some(name)
}

/// Optional `[AS] alias` after a table name. Returns (alias, new_index).
fn read_alias(toks: &[Tok], mut i: usize) -> (Option<String>, usize) {
    if let Some(Tok::Word(w)) = toks.get(i) {
        if w.eq_ignore_ascii_case("AS") {
            i += 1;
        }
    }
    if let Some(Tok::Word(w)) = toks.get(i) {
        if !is_reserved(w) {
            return (Some(w.clone()), i + 1);
        }
    }
    (None, i)
}

fn skip_parens(toks: &[Tok], mut i: usize) -> usize {
    // assumes toks[i] == '('
    let mut depth = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Punct('(') => depth += 1,
            Tok::Punct(')') => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

/// Keywords that can't be a table/alias name (so we stop the FROM list there).
fn is_reserved(w: &str) -> bool {
    matches!(
        w.to_ascii_uppercase().as_str(),
        "FROM"
            | "JOIN"
            | "INNER"
            | "LEFT"
            | "RIGHT"
            | "FULL"
            | "CROSS"
            | "OUTER"
            | "ON"
            | "USING"
            | "WHERE"
            | "GROUP"
            | "ORDER"
            | "BY"
            | "HAVING"
            | "LIMIT"
            | "OFFSET"
            | "UNION"
            | "SET"
            | "VALUES"
            | "SELECT"
            | "AS"
            | "AND"
            | "OR"
            | "INTO"
            | "STRAIGHT_JOIN"
    )
}

// --- tokenizer & statement scoping ------------------------------------------

/// Tokenize into words and significant punctuation, skipping whitespace,
/// comments, and string literals. Backtick / double-quoted identifiers become a
/// single `Word` (unquoted); single-quoted strings are dropped entirely.
fn tokenize(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
        } else if (c == '-' && b.get(i + 1) == Some(&b'-')) || c == '#' {
            // line comment to EOL (`-- …` or `# …`)
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else if c == '\'' {
            i = skip_quoted(b, i, b'\'', true);
        } else if c == '`' {
            let (word, ni) = read_quoted_ident(b, i, b'`');
            out.push(Tok::Word(word));
            i = ni;
        } else if c == '"' {
            // Treat double-quoted as an opaque identifier (one Word) — safe for
            // clause detection even on MySQL where it'd be a string, because the
            // inner text becomes a single token, never parsed for keywords.
            let (word, ni) = read_quoted_ident(b, i, b'"');
            out.push(Tok::Word(word));
            i = ni;
        } else if is_ident_byte(b[i]) {
            let start = i;
            while i < b.len() && is_ident_byte(b[i]) {
                i += 1;
            }
            out.push(Tok::Word(
                String::from_utf8_lossy(&b[start..i]).into_owned(),
            ));
        } else {
            out.push(Tok::Punct(c));
            i += 1;
        }
    }
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Skip a quoted run, returning the index just past the closing quote. `doubled`
/// allows the SQL `''` escape.
fn skip_quoted(b: &[u8], mut i: usize, q: u8, doubled: bool) -> usize {
    i += 1; // opening quote
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q {
            if doubled && b.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    i
}

/// Read a quoted identifier's inner text (without quotes), returning (text, idx).
fn read_quoted_ident(b: &[u8], mut i: usize, q: u8) -> (String, usize) {
    i += 1; // opening quote
    let start = i;
    while i < b.len() && b[i] != q {
        i += 1;
    }
    let text = String::from_utf8_lossy(&b[start..i]).into_owned();
    (text, (i + 1).min(b.len()))
}

/// The statement the cursor sits in: everything after the last top-level `;` in
/// the prefix. Exposed so other dialects (Mongo's SQL mode) can scope SQL
/// detection to the current statement before deciding whether to treat it as SQL.
pub fn current_statement(prefix: &str) -> &str {
    last_statement(prefix)
}

/// The substring after the last top-level `;` (string/comment aware).
fn last_statement(s: &str) -> &str {
    let idx = top_level_semis(s).last().copied();
    match idx {
        Some(j) => &s[j + 1..],
        None => s,
    }
}

/// The substring up to the first top-level `;`.
fn first_statement(s: &str) -> &str {
    match top_level_semis(s).first().copied() {
        Some(j) => &s[..j],
        None => s,
    }
}

/// Byte indices of `;` that are not inside a string/comment.
fn top_level_semis(s: &str) -> Vec<usize> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if (c == '-' && b.get(i + 1) == Some(&b'-')) || c == '#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == '/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                i += 1;
            }
            i += 2;
        } else if c == '\'' {
            i = skip_quoted(b, i, b'\'', true);
        } else if c == '"' {
            i = skip_quoted(b, i, b'"', false);
        } else if c == '`' {
            i = skip_quoted(b, i, b'`', false);
        } else {
            if c == ';' {
                out.push(i);
            }
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::complete::{FieldSnap, ObjectSnap, Rank};

    fn snap() -> SchemaSnapshot {
        SchemaSnapshot {
            databases: vec!["shopdb".into(), "analytics".into()],
            objects: vec![
                ObjectSnap {
                    name: "orders".into(),
                    kind: ObjKind::Table,
                    fields: vec![
                        FieldSnap::new("id", Some("int".into()), Rank::Pk),
                        FieldSnap::new("customer_id", Some("int".into()), Rank::Index),
                        FieldSnap::new("email", Some("varchar".into()), Rank::Unique),
                        FieldSnap::new("note", Some("text".into()), Rank::Plain),
                    ],
                    fields_ready: true,
                },
                ObjectSnap {
                    name: "customers".into(),
                    kind: ObjKind::Table,
                    fields: vec![
                        FieldSnap::new("id", Some("int".into()), Rank::Pk),
                        FieldSnap::new("city", Some("varchar".into()), Rank::Plain),
                    ],
                    fields_ready: true,
                },
            ],
        }
    }

    fn analyze_p(prefix: &str) -> SqlCtx {
        analyze(prefix, "")
    }

    #[test]
    fn from_expects_table() {
        let c = analyze_p("select * from ");
        assert_eq!(c.expect, SqlExpect::Table);
    }

    #[test]
    fn from_partial_table() {
        let c = analyze_p("SELECT * FROM ord");
        assert_eq!(c.expect, SqlExpect::Table);
    }

    #[test]
    fn where_expects_column() {
        let c = analyze_p("select * from orders where ");
        assert_eq!(c.expect, SqlExpect::Column { qualifier: None });
        assert_eq!(
            c.tables,
            vec![TableRef {
                name: "orders".into(),
                alias: None
            }]
        );
    }

    #[test]
    fn and_expects_column() {
        let c = analyze_p("select * from orders where id = 1 and ");
        assert_eq!(c.expect, SqlExpect::Column { qualifier: None });
    }

    #[test]
    fn alias_qualifier() {
        let c = analyze_p("select * from orders o where o.");
        assert_eq!(
            c.expect,
            SqlExpect::Column {
                qualifier: Some("o".into())
            }
        );
        assert_eq!(c.tables[0].alias.as_deref(), Some("o"));
    }

    #[test]
    fn join_collects_both_tables() {
        let c = analyze_p("select * from orders o join customers c on o.id = c.id where ");
        let names: Vec<&str> = c.tables.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"orders") && names.contains(&"customers"),
            "{names:?}"
        );
    }

    #[test]
    fn comma_join() {
        let c = analyze_p("select * from orders o, customers c where ");
        let names: Vec<&str> = c.tables.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"orders") && names.contains(&"customers"),
            "{names:?}"
        );
    }

    #[test]
    fn select_list_knows_table_from_suffix() {
        // FROM is AFTER the cursor — resolved via the suffix.
        let c = analyze("select ", " from orders");
        assert_eq!(c.expect, SqlExpect::Column { qualifier: None });
        assert_eq!(
            c.tables,
            vec![TableRef {
                name: "orders".into(),
                alias: None
            }]
        );
    }

    #[test]
    fn qualified_db_table_in_from() {
        let c = analyze_p("select * from shopdb.orders o where ");
        assert_eq!(c.tables[0].name, "orders");
        assert_eq!(c.tables[0].alias.as_deref(), Some("o"));
    }

    #[test]
    fn subquery_from_is_skipped() {
        let c = analyze_p("select * from (select * from orders) sub where ");
        // The derived table `sub` is recorded but has no columns; no crash.
        assert_eq!(c.expect, SqlExpect::Column { qualifier: None });
    }

    #[test]
    fn last_statement_only() {
        let c = analyze_p("select * from customers; select * from orders where ");
        assert_eq!(
            c.tables,
            vec![TableRef {
                name: "orders".into(),
                alias: None
            }]
        );
    }

    #[test]
    fn current_statement_scopes_to_last_segment() {
        assert_eq!(
            current_statement("db.x.find({}); SELECT * FROM c WHERE "),
            " SELECT * FROM c WHERE "
        );
        assert_eq!(current_statement("SELECT * FROM c"), "SELECT * FROM c");
        // A `;` inside a string literal is not a statement boundary.
        assert_eq!(
            current_statement("SELECT * FROM c WHERE x = 'a;b' AND "),
            "SELECT * FROM c WHERE x = 'a;b' AND "
        );
    }

    #[test]
    fn insert_into_paren_is_columns() {
        let c = analyze_p("insert into orders (");
        assert_eq!(c.expect, SqlExpect::Column { qualifier: None });
    }

    #[test]
    fn string_literal_with_from_inside_is_ignored() {
        let c = analyze_p("select * from orders where note = 'taken from stock' and ");
        assert_eq!(c.expect, SqlExpect::Column { qualifier: None });
        assert_eq!(c.tables.len(), 1);
    }

    #[test]
    fn backtick_identifiers() {
        let c = analyze_p("select * from `orders` o where ");
        assert_eq!(c.tables[0].name, "orders");
    }

    // --- assembly ordering ---

    fn labels_scores(items: &[CompletionItem], kind: CompletionKind) -> Vec<(String, i32)> {
        items
            .iter()
            .filter(|i| i.kind == kind)
            .map(|i| (i.label.clone(), i.score.unwrap_or(0)))
            .collect()
    }

    #[test]
    fn where_columns_are_index_first() {
        let c = analyze_p("select * from orders where ");
        let items = assemble(&c, &snap(), &["AND", "OR"], &[("count", "count(x)")]);
        let cols = labels_scores(&items, CompletionKind::Column);
        // id (PK) > email (UNIQUE) > customer_id (INDEX) > note (PLAIN)
        let id = cols.iter().find(|(l, _)| l == "id").unwrap().1;
        let email = cols.iter().find(|(l, _)| l == "email").unwrap().1;
        let cust = cols.iter().find(|(l, _)| l == "customer_id").unwrap().1;
        let note = cols.iter().find(|(l, _)| l == "note").unwrap().1;
        assert!(id > email && email > cust && cust > note, "{cols:?}");
    }

    #[test]
    fn qualified_columns_only_that_table() {
        let c = analyze_p("select * from orders o join customers c on true where c.");
        let items = assemble(&c, &snap(), &[], &[]);
        let cols: Vec<String> = items
            .iter()
            .filter(|i| i.kind == CompletionKind::Column)
            .map(|i| i.label.clone())
            .collect();
        assert!(cols.contains(&"city".to_string()));
        assert!(
            !cols.contains(&"email".to_string()),
            "should not leak orders cols: {cols:?}"
        );
    }

    #[test]
    fn from_offers_tables_above_keywords() {
        let c = analyze_p("select * from ");
        let items = assemble(&c, &snap(), &["SELECT", "WHERE"], &[]);
        let tbl = items
            .iter()
            .find(|i| i.kind == CompletionKind::Table && i.label == "orders")
            .unwrap()
            .score
            .unwrap();
        let kw = items
            .iter()
            .find(|i| i.kind == CompletionKind::Keyword)
            .unwrap()
            .score
            .unwrap();
        assert!(tbl > kw, "tables must out-rank keywords after FROM");
    }

    #[test]
    fn unresolvable_where_falls_back_to_all_columns() {
        // No FROM at all — still offer columns weakly so completion isn't empty.
        let c = analyze_p("where ");
        let items = assemble(&c, &snap(), &[], &[]);
        let has_cols = items.iter().any(|i| i.kind == CompletionKind::Column);
        assert!(has_cols);
    }

    #[test]
    fn bare_prefix_is_any() {
        let c = analyze_p("sel");
        assert_eq!(c.expect, SqlExpect::Any);
        let items = assemble(&c, &snap(), &["SELECT"], &[("now", "now()")]);
        // Any → keywords + functions + tables all present.
        assert!(items.iter().any(|i| i.kind == CompletionKind::Keyword));
        assert!(items.iter().any(|i| i.kind == CompletionKind::Table));
    }
}
