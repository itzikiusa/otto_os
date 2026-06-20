//! Large-batch export to a local file — the *streaming* path behind
//! `POST /connections/{id}/db/export-to-path`.
//!
//! The interactive `export` endpoint in [`crate::http`] buffers the whole
//! result set in RAM (fine for the UI's modest caps). This module exists for the
//! opposite case: a *huge* uncapped result written to a user-chosen local file,
//! where buffering would blow up daemon memory. Everything here is built around a
//! **bounded-memory** contract: the per-engine exporters (in the drivers) pull
//! rows/chunks one at a time from the driver's native cursor/stream and hand them
//! to an [`ExportSink`] that writes straight through to a `BufWriter` on disk.
//!
//! This module owns the engine-agnostic pieces:
//! - [`ExportFormat`] — the selectable output formats (+ their file extension and
//!   ClickHouse `FORMAT` name).
//! - [`ExportSink`] — a row-oriented, byte-counting writer over any `io::Write`.
//!   It knows how to emit a header (for the `*_with_names` / `json` shapes) and
//!   one row at a time in the chosen format; it never holds more than the current
//!   row in memory.
//! - The cell formatters (CSV/TSV escaping, JSON scalar rendering).
//!
//! The drivers call [`ExportSink::write_header`] once, then
//! [`ExportSink::write_row`] per row, then [`ExportSink::finish`]; the ClickHouse
//! HTTP path bypasses the row formatter entirely and streams the server's own
//! `FORMAT` bytes via [`ExportSink::write_raw`].

use std::io::Write;

use serde_json::Value;

use crate::types::Column;

/// A selectable export output format. Mirrors `ExportFormat` in `types.ts` and
/// the `format` field of `ExportToPathReq` (snake_case on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Comma-separated, no header row (RFC-4180 quoting).
    #[default]
    Csv,
    /// Comma-separated with a header row of column names.
    CsvWithNames,
    /// Tab-separated, no header row.
    Tsv,
    /// Tab-separated with a header row of column names.
    TsvWithNames,
    /// A single JSON array of row objects (`[{col: val, …}, …]`).
    Json,
    /// Newline-delimited JSON — one JSON object per line.
    Ndjson,
}

impl ExportFormat {
    /// The file extension (no dot) for this format — used to name the default
    /// `export.<ext>` file when the caller points at a directory.
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv | ExportFormat::CsvWithNames => "csv",
            ExportFormat::Tsv | ExportFormat::TsvWithNames => "tsv",
            ExportFormat::Json => "json",
            ExportFormat::Ndjson => "ndjson",
        }
    }

    /// The ClickHouse `FORMAT <name>` to request so the server renders the bytes
    /// natively and we just stream them through. `json` has no single-pass CH
    /// equivalent (it would need `JSON` which wraps in an envelope), so it returns
    /// `None` and the ClickHouse path falls back to the row formatter.
    pub fn clickhouse_format(self) -> Option<&'static str> {
        match self {
            ExportFormat::Csv => Some("CSV"),
            ExportFormat::CsvWithNames => Some("CSVWithNames"),
            ExportFormat::Tsv => Some("TabSeparated"),
            ExportFormat::TsvWithNames => Some("TabSeparatedWithNames"),
            ExportFormat::Ndjson => Some("JSONEachRow"),
            // A JSON *array* has no streaming single-FORMAT match in CH.
            ExportFormat::Json => None,
        }
    }

    /// True when this format emits a leading header row of column names.
    fn has_header(self) -> bool {
        matches!(self, ExportFormat::CsvWithNames | ExportFormat::TsvWithNames)
    }

    /// The delimited-text field separator, when this is a delimited format.
    fn delimiter(self) -> Option<char> {
        match self {
            ExportFormat::Csv | ExportFormat::CsvWithNames => Some(','),
            ExportFormat::Tsv | ExportFormat::TsvWithNames => Some('\t'),
            ExportFormat::Json | ExportFormat::Ndjson => None,
        }
    }
}

/// Counts of what an export produced — returned to the HTTP handler for the
/// `{rows, bytes}` response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExportCounts {
    pub rows: u64,
    pub bytes: u64,
}

/// A row-oriented, byte-counting writer over an arbitrary `io::Write` sink (in
/// practice a `BufWriter<File>`). Holds at most the current row in memory, so a
/// driver can stream an unbounded result through it with bounded RAM.
///
/// Lifecycle: [`write_header`](Self::write_header) once, then
/// [`write_row`](Self::write_row) per row, then [`finish`](Self::finish). The
/// ClickHouse HTTP fast path skips the row formatter and uses
/// [`write_raw`](Self::write_raw) to splice the server's native `FORMAT` bytes
/// straight to the file.
pub struct ExportSink<W: Write> {
    out: W,
    format: ExportFormat,
    /// Column names, captured at header time, so JSON rows can be keyed even when
    /// the format emits no textual header.
    columns: Vec<String>,
    counts: ExportCounts,
    /// JSON-array state: whether the opening `[` has been written and whether a
    /// row has already been written (so the next needs a leading comma).
    json_started: bool,
    json_any: bool,
}

impl<W: Write> ExportSink<W> {
    pub fn new(out: W, format: ExportFormat) -> Self {
        Self {
            out,
            format,
            columns: Vec::new(),
            counts: ExportCounts::default(),
            json_started: false,
            json_any: false,
        }
    }

    /// Record the column names and emit the textual header for the `*_with_names`
    /// formats (CSV/TSV with names). JSON formats keep the names for keying but
    /// emit no visible header; the `json` array's opening bracket is written
    /// lazily on the first row (or at `finish` for an empty result).
    pub fn write_header(&mut self, columns: &[Column]) -> std::io::Result<()> {
        self.columns = columns.iter().map(|c| c.name.clone()).collect();
        if self.format.has_header() {
            if let Some(delim) = self.format.delimiter() {
                let line = join_delimited(self.columns.iter().map(String::as_str), delim, self.format);
                self.write_line(&line)?;
            }
        }
        Ok(())
    }

    /// Write one result row. `row` cells align positionally with the columns
    /// passed to [`write_header`].
    pub fn write_row(&mut self, row: &[Value]) -> std::io::Result<()> {
        match self.format {
            ExportFormat::Csv
            | ExportFormat::CsvWithNames
            | ExportFormat::Tsv
            | ExportFormat::TsvWithNames => {
                let delim = self.format.delimiter().expect("delimited format");
                let rendered: Vec<String> = row
                    .iter()
                    .map(|v| escape_field(&cell_text(v), self.format))
                    .collect();
                let sep = delim.to_string();
                self.write_line(&rendered.join(&sep))?;
            }
            ExportFormat::Ndjson => {
                let obj = row_object(&self.columns, row);
                let mut bytes = serde_json::to_vec(&Value::Object(obj)).unwrap_or_default();
                bytes.push(b'\n');
                self.write_bytes(&bytes)?;
            }
            ExportFormat::Json => {
                if !self.json_started {
                    self.write_bytes(b"[")?;
                    self.json_started = true;
                }
                let mut bytes = Vec::new();
                if self.json_any {
                    bytes.push(b',');
                }
                let obj = row_object(&self.columns, row);
                bytes.extend_from_slice(&serde_json::to_vec(&Value::Object(obj)).unwrap_or_default());
                self.write_bytes(&bytes)?;
                self.json_any = true;
            }
        }
        self.counts.rows += 1;
        Ok(())
    }

    /// Splice already-formatted bytes straight to the file (the ClickHouse HTTP
    /// `FORMAT` streaming path). Does NOT count rows — the caller tracks the row
    /// count separately (or leaves it 0 when the server formatted the rows).
    pub fn write_raw(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.write_bytes(bytes)
    }

    /// Add to the externally-tracked row count (used by the ClickHouse raw path,
    /// which can't see individual rows but knows the count from the server reply
    /// summary — or leaves it at the bytes-only default).
    pub fn add_rows(&mut self, n: u64) {
        self.counts.rows += n;
    }

    /// Close any open container (the JSON array bracket) and flush. Returns the
    /// accumulated `{rows, bytes}`.
    pub fn finish(mut self) -> std::io::Result<ExportCounts> {
        if self.format == ExportFormat::Json {
            // An empty result still produces a valid `[]`.
            if !self.json_started {
                self.write_bytes(b"[")?;
                self.json_started = true;
            }
            self.write_bytes(b"]")?;
        }
        self.out.flush()?;
        Ok(self.counts)
    }

    /// Write a text line plus a trailing `\n`, counting the bytes.
    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        self.write_bytes(line.as_bytes())?;
        self.write_bytes(b"\n")
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.out.write_all(bytes)?;
        self.counts.bytes += bytes.len() as u64;
        Ok(())
    }
}

/// Render a JSON value as a plain-text cell for delimited formats: strings
/// verbatim, null → empty, scalars as their JSON text, containers as compact
/// JSON. (Quoting/escaping is applied separately by [`escape_field`].)
fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Escape a field for the given delimited format. CSV: RFC-4180 — quote when the
/// value contains the delimiter, a quote, or a newline, doubling embedded quotes.
/// TSV: tabs/newlines/carriage-returns are replaced with spaces (TSV has no
/// quoting), so a cell never breaks the column/row structure.
fn escape_field(s: &str, format: ExportFormat) -> String {
    match format {
        ExportFormat::Csv | ExportFormat::CsvWithNames => {
            if s.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        }
        ExportFormat::Tsv | ExportFormat::TsvWithNames => {
            s.replace(['\t', '\n', '\r'], " ")
        }
        // Not delimited — never called for these.
        ExportFormat::Json | ExportFormat::Ndjson => s.to_string(),
    }
}

/// Join an iterator of *raw* cell strings into one delimited line, escaping each.
fn join_delimited<'a, I>(cells: I, delim: char, format: ExportFormat) -> String
where
    I: Iterator<Item = &'a str>,
{
    let sep = delim.to_string();
    cells
        .map(|c| escape_field(c, format))
        .collect::<Vec<_>>()
        .join(&sep)
}

/// Build a JSON object from positional cells keyed by column name. Extra cells
/// beyond the column list are dropped; missing cells are `null`.
fn row_object(columns: &[String], row: &[Value]) -> serde_json::Map<String, Value> {
    columns
        .iter()
        .enumerate()
        .map(|(i, name)| (name.clone(), row.get(i).cloned().unwrap_or(Value::Null)))
        .collect()
}

/// Open a buffered, byte-counting [`ExportSink`] on the file at `dest` (creating
/// it / truncating an existing one). The 64 KiB `BufWriter` is what keeps the
/// streaming exporters' per-row writes from hitting the disk one row at a time.
pub fn open_sink(
    dest: &std::path::Path,
    format: ExportFormat,
) -> std::io::Result<ExportSink<std::io::BufWriter<std::fs::File>>> {
    let file = std::fs::File::create(dest)?;
    let writer = std::io::BufWriter::with_capacity(64 * 1024, file);
    Ok(ExportSink::new(writer, format))
}

/// Last-resort fallback writer for the trait-default `export_to_path`: take an
/// already-materialised [`QueryResult`] and write it out through the sink. This
/// is NOT streaming — the whole result is in memory by the time we're here — and
/// only used by engines without a native row stream (or as a safety net).
pub fn write_buffered_result(
    dest: &std::path::Path,
    format: ExportFormat,
    result: &crate::types::QueryResult,
) -> std::io::Result<ExportCounts> {
    let mut sink = open_sink(dest, format)?;
    sink.write_header(&result.columns)?;
    for row in &result.rows {
        sink.write_row(row)?;
    }
    sink.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Column;
    use serde_json::json;

    fn cols(names: &[&str]) -> Vec<Column> {
        names.iter().map(|n| Column::new(*n)).collect()
    }

    /// Run a small fixed result through the sink and return the file text.
    fn render(format: ExportFormat, columns: &[Column], rows: &[Vec<Value>]) -> (String, ExportCounts) {
        let mut buf: Vec<u8> = Vec::new();
        let counts = {
            let mut sink = ExportSink::new(&mut buf, format);
            sink.write_header(columns).unwrap();
            for r in rows {
                sink.write_row(r).unwrap();
            }
            sink.finish().unwrap()
        };
        (String::from_utf8(buf).unwrap(), counts)
    }

    // --- format → extension / ClickHouse FORMAT mapping ----------------------

    #[test]
    fn extension_mapping() {
        assert_eq!(ExportFormat::Csv.extension(), "csv");
        assert_eq!(ExportFormat::CsvWithNames.extension(), "csv");
        assert_eq!(ExportFormat::Tsv.extension(), "tsv");
        assert_eq!(ExportFormat::TsvWithNames.extension(), "tsv");
        assert_eq!(ExportFormat::Json.extension(), "json");
        assert_eq!(ExportFormat::Ndjson.extension(), "ndjson");
    }

    #[test]
    fn clickhouse_format_mapping() {
        assert_eq!(ExportFormat::Csv.clickhouse_format(), Some("CSV"));
        assert_eq!(
            ExportFormat::CsvWithNames.clickhouse_format(),
            Some("CSVWithNames")
        );
        assert_eq!(ExportFormat::Tsv.clickhouse_format(), Some("TabSeparated"));
        assert_eq!(
            ExportFormat::TsvWithNames.clickhouse_format(),
            Some("TabSeparatedWithNames")
        );
        assert_eq!(ExportFormat::Ndjson.clickhouse_format(), Some("JSONEachRow"));
        // The JSON *array* shape has no single-pass CH FORMAT.
        assert_eq!(ExportFormat::Json.clickhouse_format(), None);
    }

    #[test]
    fn format_deserializes_snake_case() {
        let f: ExportFormat = serde_json::from_str("\"csv_with_names\"").unwrap();
        assert_eq!(f, ExportFormat::CsvWithNames);
        let f: ExportFormat = serde_json::from_str("\"tsv_with_names\"").unwrap();
        assert_eq!(f, ExportFormat::TsvWithNames);
        let f: ExportFormat = serde_json::from_str("\"ndjson\"").unwrap();
        assert_eq!(f, ExportFormat::Ndjson);
    }

    // --- CSV ------------------------------------------------------------------

    #[test]
    fn csv_no_header_quotes_specials() {
        let (out, counts) = render(
            ExportFormat::Csv,
            &cols(&["a", "b", "c"]),
            &[vec![
                json!("plain"),
                json!("has,comma"),
                json!("has\"quote and\nnewline"),
            ]],
        );
        // No header row for bare csv; one data row.
        assert_eq!(out, "plain,\"has,comma\",\"has\"\"quote and\nnewline\"\n");
        assert_eq!(counts.rows, 1);
        assert_eq!(counts.bytes as usize, out.len());
    }

    #[test]
    fn csv_with_names_emits_header() {
        let (out, counts) = render(
            ExportFormat::CsvWithNames,
            &cols(&["id", "name"]),
            &[vec![json!(1), json!("Ada")], vec![json!(2), json!(null)]],
        );
        // Header + two rows; null renders as an empty cell.
        assert_eq!(out, "id,name\n1,Ada\n2,\n");
        assert_eq!(counts.rows, 2);
    }

    #[test]
    fn csv_header_name_with_comma_is_quoted() {
        let (out, _) = render(ExportFormat::CsvWithNames, &cols(&["a,b", "c"]), &[]);
        assert_eq!(out, "\"a,b\",c\n");
    }

    // --- TSV ------------------------------------------------------------------

    #[test]
    fn tsv_no_header_replaces_control_chars() {
        let (out, counts) = render(
            ExportFormat::Tsv,
            &cols(&["a", "b"]),
            &[vec![json!("with\ttab"), json!("with\nnewline")]],
        );
        // TSV has no quoting: tabs/newlines collapse to spaces so structure holds.
        assert_eq!(out, "with tab\twith newline\n");
        assert_eq!(counts.rows, 1);
    }

    #[test]
    fn tsv_with_names_emits_header() {
        let (out, _) = render(
            ExportFormat::TsvWithNames,
            &cols(&["x", "y"]),
            &[vec![json!(true), json!(3.5)]],
        );
        assert_eq!(out, "x\ty\ntrue\t3.5\n");
    }

    // --- JSON / NDJSON --------------------------------------------------------

    #[test]
    fn json_is_a_single_array_of_objects() {
        let (out, counts) = render(
            ExportFormat::Json,
            &cols(&["id", "tags"]),
            &[
                vec![json!(1), json!(["a", "b"])],
                vec![json!(2), json!(null)],
            ],
        );
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed, json!([{ "id": 1, "tags": ["a", "b"] }, { "id": 2, "tags": null }]));
        assert_eq!(counts.rows, 2);
    }

    #[test]
    fn json_empty_result_is_empty_array() {
        let (out, counts) = render(ExportFormat::Json, &cols(&["a"]), &[]);
        assert_eq!(out, "[]");
        assert_eq!(counts.rows, 0);
    }

    #[test]
    fn ndjson_is_one_object_per_line() {
        let (out, counts) = render(
            ExportFormat::Ndjson,
            &cols(&["a", "b"]),
            &[vec![json!("x"), json!(1)], vec![json!("y"), json!(2)]],
        );
        let lines: Vec<&str> = out.trim_end().split('\n').collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            serde_json::from_str::<Value>(lines[0]).unwrap(),
            json!({ "a": "x", "b": 1 })
        );
        assert_eq!(
            serde_json::from_str::<Value>(lines[1]).unwrap(),
            json!({ "a": "y", "b": 2 })
        );
        assert_eq!(counts.rows, 2);
        // Quotes/commas inside JSON strings are JSON-escaped, not CSV-escaped.
        let (out, _) = render(
            ExportFormat::Ndjson,
            &cols(&["c"]),
            &[vec![json!("a,\"b\"\n")]],
        );
        assert_eq!(
            serde_json::from_str::<Value>(out.trim_end()).unwrap(),
            json!({ "c": "a,\"b\"\n" })
        );
    }

    #[test]
    fn write_raw_counts_bytes_not_rows() {
        let mut buf: Vec<u8> = Vec::new();
        let counts = {
            let mut sink = ExportSink::new(&mut buf, ExportFormat::Csv);
            sink.write_raw(b"a,b\n1,2\n").unwrap();
            sink.add_rows(1);
            sink.finish().unwrap()
        };
        assert_eq!(String::from_utf8(buf).unwrap(), "a,b\n1,2\n");
        assert_eq!(counts.bytes, 8);
        assert_eq!(counts.rows, 1);
    }
}
