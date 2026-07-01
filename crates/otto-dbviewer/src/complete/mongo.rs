//! Pure MongoDB context analysis + completion assembly.
//!
//! Mongo "queries" in the editor look like `db.<coll>.find({ … })` or
//! `db.<coll>.aggregate([{ $match: { … } }])`. We decide whether the cursor is:
//! - naming a **collection** (`db.|`),
//! - naming a **method** (`db.coll.|`),
//! - or at a **field key** position inside the query/pipeline object
//!   (`db.coll.find({ |`), where we surface that collection's fields
//!   **indexes-first**, including embedded paths (`x`, then `x.a`/`x.b`).
//!
//! Key-vs-value detection is a cheap bracket/`:`/`,` scan — at a key position no
//! `:` has appeared since the enclosing `{` or the last `,` at that brace level.
//!
//! The Mongo runner also accepts a **SQL dialect** (`SELECT … FROM <coll> WHERE
//! …`, translated to `find`/`aggregate` by `drivers::mongo_sql`). For that case
//! the driver delegates context analysis to the shared SQL analyzer and calls
//! [`assemble_sql`] here, which maps tables→collections and columns→fields
//! (index-first) so a SQL user gets the same smart completion as MySQL/ClickHouse.

use super::{score, FieldSnap, Rank};
use crate::types::{CompletionItem, CompletionKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MongoExpect {
    /// `db.<cursor>` — a collection name.
    Collection,
    /// `db.coll.<cursor>` — a collection method (find/aggregate/…).
    Method,
    /// Inside `find({ <cursor> })` / a pipeline stage object at a key position.
    Field { path_prefix: String },
    /// A value position or anything unrecognised — operators + the safe fallback.
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MongoCtx {
    pub expect: MongoExpect,
    pub collection: Option<String>,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Analyse the cursor context from the text up to the cursor.
pub fn analyze(prefix: &str) -> MongoCtx {
    let collection = find_collection(prefix);
    let (stack, _) = bracket_scan(prefix);

    // Inside a `{ … }` object that itself sits inside the method-call `( … )`?
    let inside_object = stack.last().map(|f| f.ch) == Some('{');
    let in_value = stack.last().map(|f| f.seen_colon).unwrap_or(false);
    let inside_args = stack.iter().any(|f| f.ch == '(');

    if inside_object && inside_args && !in_value && collection.is_some() {
        return MongoCtx {
            expect: MongoExpect::Field {
                path_prefix: trailing_path(prefix),
            },
            collection,
        };
    }

    // Outside any bracket: collection vs method by the trailing `db.…` shape.
    if stack.is_empty() {
        if let Some(exp) = trailing_db_shape(prefix) {
            return MongoCtx {
                collection: collection.clone(),
                expect: exp,
            };
        }
    }

    MongoCtx {
        expect: MongoExpect::Any,
        collection,
    }
}

/// `db.` / `db.<partial>` → Collection; `db.<coll>.` / `db.<coll>.<partial>` → Method.
fn trailing_db_shape(prefix: &str) -> Option<MongoExpect> {
    let tail = trailing_path(prefix);
    let rest = tail.strip_prefix("db.")?;
    let dots = rest.matches('.').count();
    match dots {
        0 => Some(MongoExpect::Collection), // db.  or  db.us
        1 => Some(MongoExpect::Method),     // db.coll.  or  db.coll.fi
        _ => None,
    }
}

/// The trailing `[A-Za-z0-9_$.]*` run at the cursor (the partial dotted token).
fn trailing_path(prefix: &str) -> String {
    let b = prefix.as_bytes();
    let mut i = b.len();
    while i > 0 && (is_ident_byte(b[i - 1]) || b[i - 1] == b'.') {
        i -= 1;
    }
    prefix[i..].to_string()
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    ch: char,
    /// For `{` frames: have we seen a `:` since the `{` or the last `,`?
    seen_colon: bool,
}

/// Scan brackets (skipping strings) and return the open-frame stack at the end
/// plus the collection (unused here). Strings: `'…'`, `"…"`, backticks.
fn bracket_scan(s: &str) -> (Vec<Frame>, ()) {
    let b = s.as_bytes();
    let mut stack: Vec<Frame> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        match c {
            '\'' | '"' | '`' => i = skip_string(b, i),
            '/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            '(' | '[' | '{' => {
                stack.push(Frame {
                    ch: c,
                    seen_colon: false,
                });
                i += 1;
            }
            ')' | ']' | '}' => {
                stack.pop();
                i += 1;
            }
            ':' => {
                if let Some(f) = stack.last_mut() {
                    if f.ch == '{' {
                        f.seen_colon = true;
                    }
                }
                i += 1;
            }
            ',' => {
                if let Some(f) = stack.last_mut() {
                    if f.ch == '{' {
                        f.seen_colon = false;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    (stack, ())
}

fn skip_string(b: &[u8], mut i: usize) -> usize {
    let q = b[i];
    i += 1;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == q {
            return i + 1;
        }
        i += 1;
    }
    i
}

/// The collection in the last `db.<ident>.` before the cursor (the `.` after the
/// collection makes it a method/field call, not a collection-naming position).
fn find_collection(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut found = None;
    let mut i = 0;
    while i < b.len() {
        // word-boundary "db."
        let boundary = i == 0 || !is_ident_byte(b[i - 1]);
        if boundary && b[i..].starts_with(b"db.") {
            let mut j = i + 3;
            let start = j;
            while j < b.len() && is_ident_byte(b[j]) {
                j += 1;
            }
            if j > start && b.get(j) == Some(&b'.') {
                found = Some(s[start..j].to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

// --- assembly ---------------------------------------------------------------

/// Build the ranked completion list for a Mongo cursor context. `fields` is the
/// in-context collection's sampled+indexed field paths (already index-first);
/// `None` when not in a field position.
pub fn assemble(
    ctx: &MongoCtx,
    collections: &[String],
    fields: Option<&[FieldSnap]>,
    operators: &[(&str, &str)],
    methods: &[(&str, &str)],
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    match &ctx.expect {
        MongoExpect::Collection => push_collections(&mut items, collections),
        MongoExpect::Method => push_methods(&mut items, methods),
        MongoExpect::Field { .. } => {
            if let Some(fields) = fields {
                push_fields(&mut items, fields, /* quote_dotted */ true);
            }
            // Top-level logical operators (`$and`, `$or`, `$expr`, …) are also
            // valid keys — offer them, ranked below real fields.
            push_operators(&mut items, operators);
        }
        MongoExpect::Any => {
            push_operators(&mut items, operators);
            push_methods(&mut items, methods);
            push_collections(&mut items, collections);
        }
    }
    items
}

/// Build completions for a **SQL-dialect** Mongo query — the `SELECT … FROM
/// <collection> WHERE …` form the driver translates to `find`/`aggregate` (see
/// `drivers::mongo_sql`). Reuses the shared, pure SQL context analyzer
/// ([`crate::complete::sql::analyze`]) but speaks Mongo: a table slot offers
/// **collections**, a column slot offers the in-scope collection's **fields**
/// (index-first, identical ranking to native Mongo field completion), and a SQL
/// keyword/function set rounds it out. This is what makes `WHERE ` complete to
/// the collection's fields instead of the collection list, matching MySQL /
/// ClickHouse smart completion. `fields` is the resolved collection's
/// sampled+indexed paths (the caller resolves which collection); `None` when the
/// scope can't be resolved.
pub fn assemble_sql(
    sctx: &crate::complete::sql::SqlCtx,
    collections: &[String],
    fields: Option<&[FieldSnap]>,
    keywords: &[&str],
    functions: &[(&str, &str)],
) -> Vec<CompletionItem> {
    use crate::complete::sql::SqlExpect;
    let mut items = Vec::new();
    match &sctx.expect {
        // After FROM/JOIN/UPDATE/INTO → a collection (Mongo's "table").
        SqlExpect::Table => {
            push_collections(&mut items, collections);
            push_keywords(&mut items, keywords);
        }
        // After WHERE/SELECT/AND/ON/ORDER BY/GROUP BY… → fields of the in-scope
        // collection, index-first, then functions + keywords below.
        SqlExpect::Column { .. } => {
            if let Some(fields) = fields {
                // SQL columns are valid unquoted even when dotted (`a.b`).
                push_fields(&mut items, fields, /* quote_dotted */ false);
            }
            push_functions(&mut items, functions);
            push_keywords(&mut items, keywords);
        }
        // Unknown slot → keywords + functions + collections (mirrors the SQL
        // analyzer's own `Any` fallback so it's never worse than before). Mongo
        // has no SQL routines, so a `Routine` slot degrades to the same fallback.
        SqlExpect::Any | SqlExpect::Routine { .. } => {
            push_keywords(&mut items, keywords);
            push_functions(&mut items, functions);
            push_collections(&mut items, collections);
        }
    }
    items
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

fn push_collections(items: &mut Vec<CompletionItem>, collections: &[String]) {
    for c in collections {
        items.push(
            CompletionItem::new(c.clone(), CompletionKind::Collection)
                .scored(score::MONGO_COLLECTION),
        );
    }
}

fn push_methods(items: &mut Vec<CompletionItem>, methods: &[(&str, &str)]) {
    for (label, detail) in methods {
        items.push(
            CompletionItem::detailed(*label, CompletionKind::Command, *detail)
                .scored(score::MONGO_METHOD),
        );
    }
}

fn push_operators(items: &mut Vec<CompletionItem>, operators: &[(&str, &str)]) {
    for (label, detail) in operators {
        items.push(
            CompletionItem::detailed(*label, CompletionKind::Operator, *detail)
                .scored(score::MONGO_OPERATOR),
        );
    }
}

/// Push field paths index-first. Indexed paths keep their definition order via a
/// small decreasing offset; sampled (non-indexed) paths sit below.
///
/// `quote_dotted`: in a **native** Mongo filter object a dotted key MUST be
/// quoted — `{ "address.city": … }` parses but `{ address.city: … }` does not
/// (the parser hits `.` where it expects `:`). So for the native path we set the
/// inserted text to the quoted form. In the **SQL** dialect a dotted column
/// (`address.city`) is valid bare, so it's left unquoted there.
fn push_fields(items: &mut Vec<CompletionItem>, fields: &[FieldSnap], quote_dotted: bool) {
    let mut indexed_rank = 0i32;
    for f in fields {
        let sc = if f.rank == Rank::Plain {
            score::MONGO_FIELD
        } else {
            let s = score::MONGO_INDEX_FIELD - indexed_rank;
            indexed_rank += 1;
            s
        };
        let detail = match (&f.ty, f.rank.label()) {
            (Some(ty), Some(lbl)) => Some(format!("{ty} · {lbl}")),
            (Some(ty), None) => Some(ty.clone()),
            (None, Some(lbl)) => Some(lbl.to_string()),
            (None, None) => None,
        };
        let mut item = CompletionItem::new(f.name.clone(), CompletionKind::Field).scored(sc);
        item.detail = detail;
        if quote_dotted && f.name.contains('.') {
            item.insert_text = Some(format!("\"{}\"", f.name));
        }
        items.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_context() {
        let c = analyze("db.");
        assert_eq!(c.expect, MongoExpect::Collection);
        let c = analyze("db.cust");
        assert_eq!(c.expect, MongoExpect::Collection);
    }

    #[test]
    fn method_context() {
        let c = analyze("db.customers.");
        assert_eq!(c.expect, MongoExpect::Method);
        assert_eq!(c.collection.as_deref(), Some("customers"));
        let c = analyze("db.customers.fin");
        assert_eq!(c.expect, MongoExpect::Method);
    }

    #[test]
    fn field_context_in_find() {
        let c = analyze("db.customers.find({ ");
        assert!(matches!(c.expect, MongoExpect::Field { .. }));
        assert_eq!(c.collection.as_deref(), Some("customers"));
    }

    #[test]
    fn value_position_is_not_field() {
        // After `name:` we're typing a value, not a key.
        let c = analyze("db.customers.find({ name: ");
        assert_eq!(c.expect, MongoExpect::Any);
    }

    #[test]
    fn second_key_after_comma_is_field_again() {
        let c = analyze("db.customers.find({ name: 'x', ");
        assert!(matches!(c.expect, MongoExpect::Field { .. }));
    }

    #[test]
    fn nested_object_field() {
        // `$and: [ { <cursor> } ]` — innermost {} at a key position.
        let c = analyze("db.orders.find({ $and: [ { ");
        assert!(matches!(c.expect, MongoExpect::Field { .. }));
    }

    #[test]
    fn aggregate_pipeline_match_stage() {
        let c = analyze("db.orders.aggregate([{ $match: { ");
        assert!(matches!(c.expect, MongoExpect::Field { .. }));
        assert_eq!(c.collection.as_deref(), Some("orders"));
    }

    #[test]
    fn embedded_path_prefix_captured() {
        let c = analyze("db.orders.find({ addr.");
        match c.expect {
            MongoExpect::Field { path_prefix } => assert_eq!(path_prefix, "addr."),
            other => panic!("expected Field, got {other:?}"),
        }
    }

    #[test]
    fn string_braces_dont_confuse_scan() {
        // A `{` inside a string must not open a frame.
        let c = analyze("db.orders.find({ note: '{ not a brace', ");
        assert!(matches!(c.expect, MongoExpect::Field { .. }));
    }

    // --- assembly ---

    fn fields() -> Vec<FieldSnap> {
        vec![
            FieldSnap::new("_id", Some("objectId".into()), Rank::Pk),
            FieldSnap::new("customer_id", Some("int".into()), Rank::Index),
            FieldSnap::new("addr", Some("object".into()), Rank::Index), // parent of an indexed embedded path
            FieldSnap::new("addr.city", Some("string".into()), Rank::Index),
            FieldSnap::new("note", Some("string".into()), Rank::Plain),
        ]
    }

    #[test]
    fn fields_indexes_first() {
        let c = analyze("db.orders.find({ ");
        let items = assemble(&c, &[], Some(&fields()), &[("$or", "logical or")], &[]);
        let score_of = |label: &str| {
            items
                .iter()
                .find(|i| i.label == label && i.kind == CompletionKind::Field)
                .unwrap()
                .score
                .unwrap()
        };
        // Indexed fields out-rank the plain `note` and the `$or` operator.
        assert!(score_of("_id") > score_of("note"));
        assert!(score_of("customer_id") > score_of("note"));
        let or = items
            .iter()
            .find(|i| i.label == "$or")
            .unwrap()
            .score
            .unwrap();
        assert!(score_of("note") > or, "real fields rank above operators");
    }

    #[test]
    fn embedded_parent_and_child_both_offered() {
        let c = analyze("db.orders.find({ ");
        let items = assemble(&c, &[], Some(&fields()), &[], &[]);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"addr"), "parent path offered: {labels:?}");
        assert!(
            labels.contains(&"addr.city"),
            "child path offered: {labels:?}"
        );
    }

    #[test]
    fn native_dotted_field_inserts_quoted() {
        // `db.orders.find({ addr.city … })` is invalid; the key must be quoted.
        // So the dotted field's INSERTED text is `"addr.city"`, while a simple
        // field inserts bare. Labels stay bare (what the user reads).
        let c = analyze("db.orders.find({ ");
        let items = assemble(&c, &[], Some(&fields()), &[], &[]);
        let dotted = items.iter().find(|i| i.label == "addr.city").unwrap();
        assert_eq!(dotted.insert_text.as_deref(), Some("\"addr.city\""));
        let simple = items.iter().find(|i| i.label == "note").unwrap();
        assert_eq!(simple.insert_text, None, "simple key inserts bare");
    }

    #[test]
    fn sql_dotted_field_inserts_bare() {
        // In the SQL dialect a dotted column is valid bare (`WHERE addr.city = …`),
        // so it is NOT quoted (that would break the SQL→Mongo translation).
        let sctx = crate::complete::sql::analyze("SELECT * FROM orders WHERE ", "");
        let items = assemble_sql(&sctx, &[], Some(&fields()), &[], &[]);
        let dotted = items.iter().find(|i| i.label == "addr.city").unwrap();
        assert_eq!(dotted.insert_text, None, "SQL dotted column stays bare");
    }

    #[test]
    fn collection_assembly() {
        let c = analyze("db.");
        let items = assemble(
            &c,
            &["orders".to_string(), "customers".to_string()],
            None,
            &[],
            &[("find", "find docs")],
        );
        let colls: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == CompletionKind::Collection)
            .map(|i| i.label.as_str())
            .collect();
        assert!(colls.contains(&"orders") && colls.contains(&"customers"));
        // Not a method context → no methods.
        assert!(!items.iter().any(|i| i.kind == CompletionKind::Command));
    }

    #[test]
    fn method_assembly() {
        let c = analyze("db.orders.");
        let items = assemble(
            &c,
            &[],
            None,
            &[],
            &[("find", "find docs"), ("aggregate", "pipeline")],
        );
        assert!(items
            .iter()
            .any(|i| i.label == "find" && i.kind == CompletionKind::Command));
    }

    // --- SQL-dialect assembly (the `SELECT … FROM <coll> WHERE …` form) --------

    const KW: &[&str] = &["SELECT", "FROM", "WHERE", "AND"];
    const FN: &[(&str, &str)] = &[("COUNT", "COUNT(*)")];

    /// `SELECT * FROM customers WHERE ` must offer the collection's FIELDS
    /// (index-first), never the collection list — the reported Mongo bug.
    #[test]
    fn sql_where_offers_fields_not_collections() {
        let sctx = crate::complete::sql::analyze("SELECT * FROM customers WHERE ", "");
        let items = assemble_sql(
            &sctx,
            &["customers".into(), "orders".into(), "products".into()],
            Some(&fields()),
            KW,
            FN,
        );
        // Fields are present…
        assert!(items
            .iter()
            .any(|i| i.label == "_id" && i.kind == CompletionKind::Field));
        assert!(items
            .iter()
            .any(|i| i.label == "customer_id" && i.kind == CompletionKind::Field));
        // …and NO collection is offered in a WHERE (column) slot.
        assert!(
            !items.iter().any(|i| i.kind == CompletionKind::Collection),
            "collections must not appear after WHERE"
        );
        // Index fields out-rank the plain field and the keywords/functions.
        let score_of = |label: &str| items.iter().find(|i| i.label == label).unwrap().score.unwrap();
        assert!(score_of("_id") > score_of("note"));
        let kw = items
            .iter()
            .find(|i| i.kind == CompletionKind::Keyword)
            .unwrap()
            .score
            .unwrap();
        assert!(score_of("note") > kw, "real fields rank above keywords");
    }

    /// `SELECT * FROM ` must offer COLLECTIONS (Mongo's "tables").
    #[test]
    fn sql_from_offers_collections() {
        let sctx = crate::complete::sql::analyze("SELECT * FROM ", "");
        let items = assemble_sql(&sctx, &["customers".into(), "orders".into()], None, KW, FN);
        let colls: Vec<&str> = items
            .iter()
            .filter(|i| i.kind == CompletionKind::Collection)
            .map(|i| i.label.as_str())
            .collect();
        assert!(colls.contains(&"customers") && colls.contains(&"orders"));
        // No fields in a table slot.
        assert!(!items.iter().any(|i| i.kind == CompletionKind::Field));
    }

    /// A bare prefix (no clear slot) falls back to keywords + collections.
    #[test]
    fn sql_any_offers_keywords_and_collections() {
        let sctx = crate::complete::sql::analyze("SELECT * FROM orders LIMIT ", "");
        let items = assemble_sql(&sctx, &["orders".into()], None, KW, FN);
        assert!(items.iter().any(|i| i.kind == CompletionKind::Keyword));
        assert!(items.iter().any(|i| i.kind == CompletionKind::Collection));
    }
}
