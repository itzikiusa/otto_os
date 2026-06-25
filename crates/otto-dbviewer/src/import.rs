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
    let rows = iter
        .map(|r| r.into_iter().map(Value::String).collect())
        .collect();
    Ok(ParsedTable { columns, rows })
}

/// Render a JSON value as a SQL literal. Strings are single-quoted with embedded
/// quotes doubled (`'` → `''`); null → `NULL`; bools → `TRUE`/`FALSE`; numbers
/// verbatim; objects/arrays → a quoted compact-JSON string.
pub fn sql_string_literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        other => format!("'{}'", other.to_string().replace('\'', "''")),
    }
}

/// Quote a SQL identifier by backtick-wrapping and doubling embedded backticks
/// (MySQL/ClickHouse identifier rules).
fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// Build batched multi-row `INSERT` statements. Each statement inserts at most
/// `batch_size` rows. Returns an empty Vec for no rows. Cells align positionally
/// with `columns`; a short row is padded with `NULL`.
pub fn build_insert_statements(
    table: &str,
    columns: &[String],
    rows: &[Vec<Value>],
    batch_size: usize,
) -> Vec<String> {
    if rows.is_empty() || columns.is_empty() {
        return Vec::new();
    }
    let batch_size = batch_size.max(1);
    let col_list = columns
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = Vec::new();
    for chunk in rows.chunks(batch_size) {
        let values: Vec<String> = chunk
            .iter()
            .map(|row| {
                let cells: Vec<String> = (0..columns.len())
                    .map(|i| sql_string_literal(row.get(i).unwrap_or(&Value::Null)))
                    .collect();
                format!("({})", cells.join(", "))
            })
            .collect();
        out.push(format!(
            "INSERT INTO {} ({}) VALUES {}",
            quote_ident(table),
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
        assert_eq!(sql_string_literal(&json!(null)), "NULL");
        assert_eq!(sql_string_literal(&json!(true)), "TRUE");
        assert_eq!(sql_string_literal(&json!(42)), "42");
        assert_eq!(sql_string_literal(&json!("Ada")), "'Ada'");
        // Single quotes are doubled (no injection).
        assert_eq!(sql_string_literal(&json!("O'Brien")), "'O''Brien'");
        // Objects/arrays serialize to a quoted JSON string.
        assert_eq!(sql_string_literal(&json!({"a":1})), "'{\"a\":1}'");
    }

    #[test]
    fn insert_builder_batches_and_quotes_identifiers() {
        let cols = vec!["id".to_string(), "name".to_string()];
        let rows = vec![
            vec![json!(1), json!("Ada")],
            vec![json!(2), json!("O'Brien")],
            vec![json!(3), json!("Grace")],
        ];
        let stmts = build_insert_statements("users", &cols, &rows, 2);
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
    fn insert_builder_empty_rows_is_no_statements() {
        assert!(build_insert_statements("t", &["a".into()], &[], 100).is_empty());
    }
}
