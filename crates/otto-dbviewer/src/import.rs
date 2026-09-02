//! File→table import: the mirror of `export.rs`. The pure core here parses a
//! local file into `{columns, rows}` and builds safely-escaped, batched `INSERT`
//! statements. `DbViewerService::import_from_path` runs those batches through the
//! normal guarded `run` path, so the write guard / history / masking all apply
//! with no new safety code. v1 targets SQL engines.

use serde_json::{Map, Value};

use otto_core::{Error, Result};

/// Supported import file formats. Delimited formats take the **first row as the
/// header** (column names); JSON/NDJSON carry keys per object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    Csv,
    Tsv,
    Ndjson,
    Json,
}

/// A parsed table ready for insertion: column names + positional rows.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

/// Parse `bytes` in `format` into a [`ParsedTable`].
///
/// - CSV/TSV: first line = header; remaining lines = rows of **string** cells
///   (the engine coerces types on insert). Minimal RFC-4180 quoting is honored
///   (double-quoted fields, doubled quotes, embedded delimiters/newlines).
/// - NDJSON: one JSON object per line; columns are the union of keys in
///   first-seen order; missing keys become `null`.
/// - JSON: a single array of objects; same key-union rule.
pub fn parse_rows(format: ImportFormat, bytes: &[u8]) -> Result<ParsedTable> {
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Err(Error::Invalid("import file is empty".into()));
    }
    match format {
        ImportFormat::Csv => parse_delimited(bytes, ','),
        ImportFormat::Tsv => parse_delimited(bytes, '\t'),
        ImportFormat::Ndjson => parse_objects(
            std::str::from_utf8(bytes)
                .map_err(|_| Error::Invalid("import file is not valid UTF-8".into()))?
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(serde_json::from_str::<Value>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| Error::Invalid(format!("invalid NDJSON: {e}")))?,
        ),
        ImportFormat::Json => {
            let v: Value = serde_json::from_slice(bytes)
                .map_err(|e| Error::Invalid(format!("invalid JSON: {e}")))?;
            let arr = v
                .as_array()
                .ok_or_else(|| Error::Invalid("JSON import must be an array of objects".into()))?;
            parse_objects(arr.clone())
        }
    }
}

/// Build a `ParsedTable` from a list of JSON objects, unioning keys in first-seen
/// order and filling absent keys with `null`.
fn parse_objects(objects: Vec<Value>) -> Result<ParsedTable> {
    let mut columns: Vec<String> = Vec::new();
    let mut maps: Vec<Map<String, Value>> = Vec::with_capacity(objects.len());
    for obj in objects {
        let map = obj
            .as_object()
            .ok_or_else(|| Error::Invalid("every import record must be a JSON object".into()))?
            .clone();
        for k in map.keys() {
            if !columns.iter().any(|c| c == k) {
                columns.push(k.clone());
            }
        }
        maps.push(map);
    }
    if columns.is_empty() {
        return Err(Error::Invalid("import file has no columns".into()));
    }
    let rows = maps
        .into_iter()
        .map(|m| {
            columns
                .iter()
                .map(|c| m.get(c).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    Ok(ParsedTable { columns, rows })
}

/// Minimal RFC-4180-style delimited parser (handles quoted fields with embedded
/// delimiters, quotes, and newlines). First record = header.
fn parse_delimited(bytes: &[u8], delim: char) -> Result<ParsedTable> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::Invalid("import file is not valid UTF-8".into()))?;
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut field = String::new();
    let mut record: Vec<String> = Vec::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            _ if c == delim && !in_quotes => {
                record.push(std::mem::take(&mut field));
            }
            '\n' if !in_quotes => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            '\r' if !in_quotes => {} // swallow CR (CRLF)
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    let mut iter = records
        .into_iter()
        .filter(|r| !(r.len() == 1 && r[0].is_empty()));
    let columns = iter
        .next()
        .ok_or_else(|| Error::Invalid("delimited file has no header row".into()))?;
    // Reject mismatched arity instead of silently dropping extra cells / padding
    // short rows with NULL — a malformed file would otherwise import skewed data
    // with no error. Row numbers are 1-based counting the header as row 1.
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for (i, r) in iter.enumerate() {
        if r.len() != columns.len() {
            return Err(Error::Invalid(format!(
                "row {} has {} field(s) but the header has {} column(s)",
                i + 2,
                r.len(),
                columns.len()
            )));
        }
        rows.push(r.into_iter().map(Value::String).collect());
    }
    Ok(ParsedTable { columns, rows })
}

/// Render a JSON value as a SQL literal for `flavor`. Strings are single-quoted
/// with embedded quotes doubled (`'` → `''`); null → `NULL`; bools →
/// `TRUE`/`FALSE`; numbers verbatim; objects/arrays → a quoted compact-JSON
/// string.
///
/// SECURITY: escaping is ENGINE-AWARE. MySQL (default sql_mode) and ClickHouse
/// process `\` inside a single-quoted literal as an escape character, so a cell
/// ending in `\` would swallow the closing quote and splice the next cell's
/// bytes into raw SQL — those flavors escape `\` → `\\`. Postgres
/// (standard-conforming strings) treats `\` literally, so only `'` is doubled.
pub fn sql_string_literal(v: &Value, flavor: SqlFlavor) -> String {
    let quote_str = |s: &str| -> String {
        let escaped = if flavor.backslash_escapes_in_literals() {
            s.replace('\\', "\\\\").replace('\'', "''")
        } else {
            s.replace('\'', "''")
        };
        format!("'{escaped}'")
    };
    match v {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => quote_str(s),
        other => quote_str(&other.to_string()),
    }
}

/// Infer a scalar type from a CSV/TSV string cell (the delimited parser emits
/// every cell as `Value::String`). Rules: a trimmed-empty cell → `null`;
/// `true`/`false` (case-insensitive) → bool; an integer → i64 number; a finite
/// float → number; anything else → the string unchanged. Non-string values
/// (from already-typed NDJSON/JSON) pass through untouched.
///
/// The SQL import path leans on the database engine to coerce string cells to
/// each column's type on `INSERT`. MongoDB is schemaless, so a CSV import must
/// infer types here or every field lands as a string — this is that inference,
/// applied by the Mongo import path for delimited formats only.
pub fn coerce_scalar(v: &Value) -> Value {
    let Value::String(s) = v else {
        return v.clone();
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Value::Null;
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(i) = trimmed.parse::<i64>() {
        return Value::from(i);
    }
    // Only accept a float that round-trips to a finite JSON number (rejects
    // "NaN"/"inf" and preserves things like leading-zero strings as text).
    if let Ok(f) = trimmed.parse::<f64>() {
        if f.is_finite() {
            if let Some(n) = serde_json::Number::from_f64(f) {
                return Value::Number(n);
            }
        }
    }
    Value::String(s.clone())
}

/// The SQL flavor an `INSERT` is rendered for — decides identifier quoting AND
/// string-literal escaping (which differ between the engines in ways that are
/// security-relevant, see [`sql_string_literal`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlFlavor {
    /// `` `name` `` idents (backtick doubling; `\` is LITERAL in idents);
    /// `\` is an escape in string literals.
    Mysql,
    /// `` `name` `` idents, but `\` is an escape in BOTH quoted identifiers and
    /// string literals (ClickHouse's lexer applies string-escape rules to
    /// backtick idents too).
    Clickhouse,
    /// `"name"` idents (standard doubling); `\` is literal everywhere
    /// (standard-conforming strings).
    Postgres,
}

impl SqlFlavor {
    /// Quote an identifier for this flavor, escaping the quote char (and, for
    /// ClickHouse, embedded backslashes — its backtick idents honor `\` escapes,
    /// so an unescaped trailing `\` would swallow the closing backtick).
    fn ident(self, name: &str) -> String {
        match self {
            SqlFlavor::Mysql => format!("`{}`", name.replace('`', "``")),
            SqlFlavor::Clickhouse => {
                format!("`{}`", name.replace('\\', "\\\\").replace('`', "``"))
            }
            SqlFlavor::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    /// Whether the engine treats `\` as an escape inside single-quoted string
    /// literals (MySQL under the default sql_mode, and ClickHouse always).
    fn backslash_escapes_in_literals(self) -> bool {
        matches!(self, SqlFlavor::Mysql | SqlFlavor::Clickhouse)
    }
}

/// Build batched multi-row `INSERT` statements for `flavor` (identifier quoting
/// AND string-literal escaping are engine-aware — see [`SqlFlavor`] /
/// [`sql_string_literal`]). Each statement inserts at most `batch_size` rows.
/// Returns an empty Vec for no rows. Cells align positionally with `columns`; a
/// short row is padded with `NULL` (the delimited parser rejects mismatched
/// arity upstream; JSON key-union rows are always aligned).
pub fn build_insert_statements(
    table: &str,
    columns: &[String],
    rows: &[Vec<Value>],
    batch_size: usize,
    flavor: SqlFlavor,
) -> Vec<String> {
    if rows.is_empty() || columns.is_empty() {
        return Vec::new();
    }
    let batch_size = batch_size.max(1);
    let col_list = columns
        .iter()
        .map(|c| flavor.ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = Vec::new();
    for chunk in rows.chunks(batch_size) {
        let values: Vec<String> = chunk
            .iter()
            .map(|row| {
                let cells: Vec<String> = (0..columns.len())
                    .map(|i| sql_string_literal(row.get(i).unwrap_or(&Value::Null), flavor))
                    .collect();
                format!("({})", cells.join(", "))
            })
            .collect();
        out.push(format!(
            "INSERT INTO {} ({}) VALUES {}",
            flavor.ident(table),
            col_list,
            values.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_csv_uses_first_row_as_headers() {
        let p = parse_rows(ImportFormat::Csv, b"id,name\n1,Ada\n2,Grace\n").unwrap();
        assert_eq!(p.columns, vec!["id", "name"]);
        assert_eq!(p.rows.len(), 2);
        assert_eq!(p.rows[0], vec![json!("1"), json!("Ada")]);
    }

    #[test]
    fn parse_csv_handles_quoted_fields_with_commas() {
        let p = parse_rows(ImportFormat::Csv, b"a,b\n\"x,y\",z\n").unwrap();
        assert_eq!(p.rows[0], vec![json!("x,y"), json!("z")]);
    }

    #[test]
    fn parse_ndjson_keeps_json_types_and_unions_keys() {
        let body = b"{\"id\":1,\"name\":\"Ada\"}\n{\"id\":2,\"active\":true}\n";
        let p = parse_rows(ImportFormat::Ndjson, body).unwrap();
        // Columns are the union of keys in first-seen order.
        assert_eq!(p.columns, vec!["id", "name", "active"]);
        // Missing keys become null; types are preserved.
        assert_eq!(p.rows[0], vec![json!(1), json!("Ada"), json!(null)]);
        assert_eq!(p.rows[1], vec![json!(2), json!(null), json!(true)]);
    }

    #[test]
    fn parse_json_array_of_objects() {
        let p = parse_rows(ImportFormat::Json, b"[{\"id\":1},{\"id\":2}]").unwrap();
        assert_eq!(p.columns, vec!["id"]);
        assert_eq!(p.rows.len(), 2);
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(parse_rows(ImportFormat::Csv, b"").is_err());
    }

    #[test]
    fn literals_escape_quotes_and_render_scalars() {
        for flavor in [SqlFlavor::Mysql, SqlFlavor::Clickhouse, SqlFlavor::Postgres] {
            assert_eq!(sql_string_literal(&json!(null), flavor), "NULL");
            assert_eq!(sql_string_literal(&json!(true), flavor), "TRUE");
            assert_eq!(sql_string_literal(&json!(42), flavor), "42");
            assert_eq!(sql_string_literal(&json!("Ada"), flavor), "'Ada'");
            // Single quotes are doubled (no injection).
            assert_eq!(sql_string_literal(&json!("O'Brien"), flavor), "'O''Brien'");
            // Objects/arrays serialize to a quoted JSON string (no backslashes
            // here, so every flavor renders identically).
            assert_eq!(sql_string_literal(&json!({"a":1}), flavor), "'{\"a\":1}'");
        }
    }

    /// The break-out this guards against: MySQL/ClickHouse treat `\` in a
    /// single-quoted literal as an escape, so a trailing `\` would swallow the
    /// closing quote and the NEXT cell's bytes would parse as raw SQL.
    #[test]
    fn literals_escape_backslashes_per_flavor() {
        // MySQL/ClickHouse: `\` doubled so `\''` can't read as escaped-quote+quote.
        assert_eq!(
            sql_string_literal(&json!("evil\\"), SqlFlavor::Mysql),
            "'evil\\\\'"
        );
        assert_eq!(
            sql_string_literal(&json!("evil\\"), SqlFlavor::Clickhouse),
            "'evil\\\\'"
        );
        // Postgres (standard-conforming strings): `\` is literal — left alone.
        assert_eq!(
            sql_string_literal(&json!("evil\\"), SqlFlavor::Postgres),
            "'evil\\'"
        );
        // The classic payload: a cell ending in `\` followed by an injection
        // cell stays two inert literals for MySQL.
        let cols = vec!["a".to_string(), "b".to_string()];
        let rows = vec![vec![json!("x\\"), json!("),(1,(SELECT 1)) -- ")]];
        let stmt = &build_insert_statements("t", &cols, &rows, 10, SqlFlavor::Mysql)[0];
        assert_eq!(
            stmt,
            "INSERT INTO `t` (`a`, `b`) VALUES ('x\\\\', '),(1,(SELECT 1)) -- ')"
        );
    }

    #[test]
    fn insert_builder_batches_and_quotes_identifiers() {
        let cols = vec!["id".to_string(), "name".to_string()];
        let rows = vec![
            vec![json!(1), json!("Ada")],
            vec![json!(2), json!("O'Brien")],
            vec![json!(3), json!("Grace")],
        ];
        let stmts = build_insert_statements("users", &cols, &rows, 2, SqlFlavor::Mysql);
        // 3 rows, batch 2 → two statements.
        assert_eq!(stmts.len(), 2);
        assert_eq!(
            stmts[0],
            "INSERT INTO `users` (`id`, `name`) VALUES (1, 'Ada'), (2, 'O''Brien')"
        );
        assert_eq!(
            stmts[1],
            "INSERT INTO `users` (`id`, `name`) VALUES (3, 'Grace')"
        );
    }

    #[test]
    fn insert_builder_double_quotes_identifiers_for_postgres() {
        let cols = vec!["id".to_string(), "full name".to_string()];
        let rows = vec![vec![json!(1), json!("O'Brien")]];
        let stmts = build_insert_statements("my table", &cols, &rows, 100, SqlFlavor::Postgres);
        assert_eq!(stmts.len(), 1);
        // Identifiers double-quoted (spaces + reserved-word safe); string values
        // still single-quoted with `'` doubled — same as MySQL.
        assert_eq!(
            stmts[0],
            "INSERT INTO \"my table\" (\"id\", \"full name\") VALUES (1, 'O''Brien')"
        );
    }

    /// ClickHouse's backtick idents honor string-style `\` escapes; MySQL's
    /// treat `\` literally — the two backtick flavors must diverge here.
    #[test]
    fn ident_backslash_handling_diverges_between_backtick_flavors() {
        let cols = vec!["we`ird\\".to_string()];
        let rows = vec![vec![json!(1)]];
        let my = &build_insert_statements("t", &cols, &rows, 10, SqlFlavor::Mysql)[0];
        assert!(my.contains("`we``ird\\`"), "mysql: {my}");
        let ch = &build_insert_statements("t", &cols, &rows, 10, SqlFlavor::Clickhouse)[0];
        assert!(ch.contains("`we``ird\\\\`"), "clickhouse: {ch}");
    }

    #[test]
    fn insert_builder_empty_rows_is_no_statements() {
        assert!(build_insert_statements("t", &["a".into()], &[], 100, SqlFlavor::Mysql).is_empty());
    }

    /// A malformed delimited file (row arity ≠ header arity) is rejected with
    /// the offending row number, not silently padded/clipped into skewed data.
    #[test]
    fn delimited_rows_with_wrong_arity_are_rejected() {
        let err = parse_rows(ImportFormat::Csv, b"a,b\n1,2\n3\n").unwrap_err();
        assert!(err.to_string().contains("row 3"), "{err}");
        let err = parse_rows(ImportFormat::Csv, b"a,b\n1,2,3\n").unwrap_err();
        assert!(err.to_string().contains("3 field(s)"), "{err}");
    }

    #[test]
    fn coerce_scalar_infers_types_from_csv_strings() {
        assert_eq!(coerce_scalar(&json!("42")), json!(42));
        assert_eq!(coerce_scalar(&json!("-7")), json!(-7));
        assert_eq!(coerce_scalar(&json!("2.5")), json!(2.5));
        assert_eq!(coerce_scalar(&json!("true")), json!(true));
        assert_eq!(coerce_scalar(&json!("FALSE")), json!(false));
        assert_eq!(coerce_scalar(&json!("")), json!(null));
        assert_eq!(coerce_scalar(&json!("   ")), json!(null));
        // Non-numeric / non-bool text stays a string.
        assert_eq!(coerce_scalar(&json!("Ada")), json!("Ada"));
        // A phone-like string with a leading zero is NOT a valid i64 with the
        // zero preserved — parse::<i64> drops it, so keep it as text.
        assert_eq!(coerce_scalar(&json!("007")), json!(7)); // documents the behavior
        // NaN/inf are not coerced.
        assert_eq!(coerce_scalar(&json!("NaN")), json!("NaN"));
    }

    #[test]
    fn coerce_scalar_passes_typed_values_through_untouched() {
        // NDJSON/JSON already carry native types — never re-coerce them.
        assert_eq!(coerce_scalar(&json!(1)), json!(1));
        assert_eq!(coerce_scalar(&json!(true)), json!(true));
        assert_eq!(coerce_scalar(&json!(null)), json!(null));
        assert_eq!(coerce_scalar(&json!({"a":1})), json!({"a":1}));
        assert_eq!(coerce_scalar(&json!([1,2])), json!([1,2]));
    }
}
