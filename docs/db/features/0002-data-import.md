# Data Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Import a local file (CSV / TSV / NDJSON / JSON-array) into an existing SQL table, the mirror of the export-to-path feature the explorer already has — closing the explorer's one purely-functional gap (it is export-only today).

**Architecture:** A new `import` module holds the **pure, fully-tested** core: parse a file into `{columns, rows}` and build batched `INSERT` statements with safely-escaped literals. `DbViewerService::import_from_path` parses the file, builds batches, and runs each batch **through the existing `self.run` path** — so the already-built write guard (`guard_write`) automatically requires `confirm_write` on a Prod/read-only connection, masking/history apply, and **no new safety code is written**. The HTTP route reuses the export endpoint's NDJSON-progress streaming verbatim. v1 targets SQL engines (MySQL/ClickHouse); Mongo `insertMany` and Redis are explicit follow-ups.

**Tech Stack:** Rust (axum, `serde_json`, `tokio`), the existing `DbViewerService::run` + `guard_write`, the export NDJSON-progress handler pattern (`http.rs:538`), Svelte 5 + TypeScript.

## Global Constraints

- **Import is a write.** Every batch goes through `DbViewerService::run`, so `guard_write` applies: a guarded (Prod/read-only) connection refuses the import unless the request carries `confirm_write`. Do **not** add a parallel guard — reuse `run`.
- **Identifier + literal safety.** Table/column identifiers are backtick-quoted (MySQL/ClickHouse); cell values are rendered through a tested `sql_string_literal` that escapes embedded quotes. This is the user importing their own file into their own DB, but escaping is still mandatory (`'` → `''`).
- **Role gate = `Editor`** (global connections: root) — same as `run_query`/`export`.
- **Bounded statements.** Rows are batched into `INSERT … VALUES (…),(…)` of `batch_size` (default 500, clamp 1..=5000) so no single statement is unbounded.
- **v1 engine scope:** MySQL + ClickHouse (anything whose `run` accepts `INSERT`). Mongo/Redis return a clear "not supported yet" error.
- **Contract lockstep:** new endpoint ⇒ update `docs/contracts/api.md`, `ui/src/lib/api/types.ts`, `docs/features/database-explorer.md` together.
- Match surrounding code: dense + documented, mirror `export.rs`/`http.rs` idioms.

---

### Task 1: `import` module — formats + `parse_rows`

**Files:**
- Create: `crates/otto-dbviewer/src/import.rs`
- Modify: `crates/otto-dbviewer/src/lib.rs` (`pub mod import;`)

**Interfaces:**
- Produces: `import::ImportFormat`, `import::ParsedTable { columns: Vec<String>, rows: Vec<Vec<Value>> }`, `import::parse_rows(format, &[u8]) -> otto_core::Result<ParsedTable>`.

- [ ] **Step 1: Write the failing test for `parse_rows`**

Create `crates/otto-dbviewer/src/import.rs` with:

```rust
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
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p otto-dbviewer import::tests`
Expected: FAIL — `parse_rows` / `ImportFormat` / `ParsedTable` not found.

- [ ] **Step 3: Implement the module header, types, and `parse_rows`**

At the top of `crates/otto-dbviewer/src/import.rs`:

```rust
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
                .map(|l| serde_json::from_str::<Value>(l))
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
    let mut iter = records.into_iter().filter(|r| !(r.len() == 1 && r[0].is_empty()));
    let columns = iter
        .next()
        .ok_or_else(|| Error::Invalid("delimited file has no header row".into()))?;
    let rows = iter
        .map(|r| r.into_iter().map(Value::String).collect())
        .collect();
    Ok(ParsedTable { columns, rows })
}
```

Register it — in `crates/otto-dbviewer/src/lib.rs`: `pub mod import;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p otto-dbviewer import::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otto-dbviewer/src/import.rs crates/otto-dbviewer/src/lib.rs
git commit -m "feat(db): import module — CSV/TSV/NDJSON/JSON parser"
```

---

### Task 2: SQL literal escaping + batched INSERT builder

**Files:**
- Modify: `crates/otto-dbviewer/src/import.rs`

**Interfaces:**
- Produces:
  - `import::sql_string_literal(&Value) -> String`
  - `import::build_insert_statements(table: &str, columns: &[String], rows: &[Vec<Value>], batch_size: usize) -> Vec<String>`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module:

```rust
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
        assert_eq!(stmts[1], "INSERT INTO `users` (`id`, `name`) VALUES (3, 'Grace')");
    }

    #[test]
    fn insert_builder_empty_rows_is_no_statements() {
        assert!(build_insert_statements("t", &["a".into()], &[], 100).is_empty());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p otto-dbviewer import::tests::literals_escape_quotes_and_render_scalars import::tests::insert_builder_batches_and_quotes_identifiers`
Expected: FAIL — functions not found.

- [ ] **Step 3: Implement the two functions**

Add to `crates/otto-dbviewer/src/import.rs`:

```rust
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
    let col_list = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p otto-dbviewer import::tests`
Expected: PASS (all import tests).

- [ ] **Step 5: Commit**

```bash
git add crates/otto-dbviewer/src/import.rs
git commit -m "feat(db): SQL literal escaping + batched INSERT builder for import"
```

---

### Task 3: `DbViewerService::import_from_path`

**Files:**
- Modify: `crates/otto-dbviewer/src/service.rs`

**Interfaces:**
- Consumes: `self.connections.get`, `self.run`, `crate::import::{ImportFormat, parse_rows, build_insert_statements}`, `crate::types::{Engine, QueryRequest}`.
- Produces: `DbViewerService::import_from_path(&self, conn_id: &Id, user_id: &Id, local_path: &str, format: ImportFormat, table: &str, batch_size: usize, confirm_write: bool) -> Result<ImportCounts>` and `pub struct ImportCounts { pub rows: u64, pub batches: u64 }`.

- [ ] **Step 1: Add the counts type and the method**

In `crates/otto-dbviewer/src/service.rs`, add near the top (after the other small structs):

```rust
/// What an import produced — returned to the HTTP handler for its final line.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct ImportCounts {
    pub rows: u64,
    pub batches: u64,
}
```

In `impl DbViewerService`, after `export_to_path`:

```rust
    /// Import a local file into an existing SQL table. Parses the file, builds
    /// batched `INSERT`s, and runs each batch **through the guarded `run` path**
    /// — so a Prod/read-only connection refuses the import unless `confirm_write`
    /// is set (no new guard here), and history records each batch. v1 supports
    /// SQL engines (MySQL/ClickHouse); Mongo/Redis return a clear error.
    #[allow(clippy::too_many_arguments)]
    pub async fn import_from_path(
        &self,
        conn_id: &Id,
        user_id: &Id,
        local_path: &str,
        format: crate::import::ImportFormat,
        table: &str,
        batch_size: usize,
        confirm_write: bool,
    ) -> Result<ImportCounts> {
        let conn = self.connections.get(conn_id).await?;
        match Engine::from_kind(conn.kind) {
            Some(Engine::Mysql) | Some(Engine::Clickhouse) => {}
            Some(other) => {
                return Err(Error::Invalid(format!(
                    "file import is not supported for {} yet (SQL engines only in v1)",
                    other.as_str()
                )))
            }
            None => return Err(Error::Invalid("connection is not a browsable database".into())),
        }

        let bytes = tokio::fs::read(local_path)
            .await
            .map_err(|e| Error::Invalid(format!("read import file: {e}")))?;
        let parsed = crate::import::parse_rows(format, &bytes)?;
        let statements =
            crate::import::build_insert_statements(table, &parsed.columns, &parsed.rows, batch_size);

        let mut counts = ImportCounts::default();
        for stmt in statements {
            let req = QueryRequest {
                statement: stmt,
                confirm_write,
                ..QueryRequest::default()
            };
            // Routes through guard_write + history. A guarded connection without
            // confirm_write fails here with the standard write_blocked: 409.
            let res = self.run(conn_id, user_id, &req).await?;
            counts.rows += res.rows_affected.unwrap_or(0);
            counts.batches += 1;
        }
        Ok(counts)
    }
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p otto-dbviewer`
Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add crates/otto-dbviewer/src/service.rs
git commit -m "feat(db): import_from_path runs batched INSERTs through the guarded path"
```

---

### Task 4: route + NDJSON-progress handler (mirror export-to-path)

**Files:**
- Modify: `crates/otto-dbviewer/src/http.rs`

**Interfaces:**
- Consumes: `crate::import::ImportFormat`, `ctx.db().import_from_path`, `check_conn_role`, `resolve_export_dest`/`expand_home` (existing helpers), the export NDJSON-progress pattern.
- Produces: `struct ImportReq`, route `POST /connections/{id}/db/import` → `import_query::<S>`.

- [ ] **Step 1: Register the route**

In `api_router::<S>()`, after `export-to-path`:

```rust
        .route("/connections/{id}/db/import", post(import_query::<S>))
```

- [ ] **Step 2: Add the request struct + handler**

Add near `ExportToPathReq`:

```rust
#[derive(Debug, Deserialize)]
struct ImportReq {
    /// Source file on the daemon host (leading `~` expands to the daemon home).
    local_path: String,
    /// File format.
    format: crate::import::ImportFormat,
    /// Destination table (must already exist).
    table: String,
    /// Rows per INSERT; clamped 1..=5000 server-side (default 500).
    #[serde(default)]
    batch_size: Option<usize>,
    /// Typed-confirmation acknowledgement for a guarded (Prod/read-only) connection.
    #[serde(default)]
    confirm_write: bool,
}
```

Handler (mirrors `export_to_path`'s spawn + ticker + NDJSON stream, but the
ticker has no on-disk file to size, so it reports batch progress instead via a
shared counter; simplest correct v1 = run to completion then emit one final
line, matching the export "done" line shape):

```rust
/// Import a local file into a SQL table. Gated at `Editor` (global: root). Streams
/// a single NDJSON result line (`{done,rows,batches}` or `{error}`) so the client
/// uses the same reader as export. A write on a guarded connection without
/// `confirm_write` returns the standard `write_blocked:` error in the `{error}`
/// line.
async fn import_query<S: DbViewerCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<ImportReq>,
) -> ApiResult<Response> {
    let conn = ctx.db().get_connection(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    if req.local_path.trim().is_empty() || req.table.trim().is_empty() {
        return Err(ApiErr(Error::Invalid("local_path and table are required".into())));
    }
    let path = expand_home(&req.local_path);
    let batch_size = req.batch_size.unwrap_or(500).clamp(1, 5000);

    let db = ctx.db().clone();
    let uid = user.id.clone();
    let conn_id = id.clone();
    let (table, format, confirm) = (req.table.clone(), req.format, req.confirm_write);

    let (tx, rx) =
        tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::convert::Infallible>>(4);
    tokio::spawn(async move {
        let line = match db
            .import_from_path(&conn_id, &uid, &path, format, &table, batch_size, confirm)
            .await
        {
            Ok(c) => serde_json::json!({ "done": true, "rows": c.rows, "batches": c.batches }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        };
        let _ = tx.send(Ok(ndjson_line(&line))).await;
    });

    let stream =
        futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|i| (i, rx)) });
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        axum::body::Body::from_stream(stream),
    )
        .into_response())
}
```

- [ ] **Step 3: Add a role-gate test**

In the `http.rs` `tests` module:

```rust
    #[tokio::test]
    async fn import_gate_requires_editor() {
        let stub = Arc::new(StubRoles::new(WorkspaceRole::Viewer));
        let ctx = TestCtx { roles: stub.clone() };
        let c = conn(Some(new_id()));
        let err = check_conn_role(&ctx, &user(false), &c, WorkspaceRole::Editor)
            .await
            .expect_err("viewer denied");
        assert!(matches!(err, Error::Forbidden(_)));
        assert_eq!(*stub.last_min.lock().unwrap(), Some(WorkspaceRole::Editor));
    }
```

- [ ] **Step 4: Build + run the gate test**

Run: `cargo build -p otto-dbviewer && cargo test -p otto-dbviewer http::tests::import_gate_requires_editor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/otto-dbviewer/src/http.rs
git commit -m "feat(db): POST /db/import route streaming an NDJSON result line"
```

---

### Task 5: Contract + TypeScript types + client

**Files:**
- Modify: `docs/contracts/api.md`, `ui/src/lib/api/types.ts`, the DB API client module.

- [ ] **Step 1: Document the endpoint**

In `docs/contracts/api.md`, DB engine-access table:

```
| `POST …/db/import` | import a local file into a table (`{local_path, format, table, batch_size?, confirm_write?}` → NDJSON `{done,rows,batches}`/`{error}`); `Editor` |
```

- [ ] **Step 2: Mirror the types**

Add to `ui/src/lib/api/types.ts`:

```ts
export type ImportFormat = "csv" | "tsv" | "ndjson" | "json";

export interface ImportReq {
  local_path: string;
  format: ImportFormat;
  table: string;
  batch_size?: number;
  confirm_write?: boolean;
}

export interface ImportResult {
  done?: boolean;
  rows?: number;
  batches?: number;
  error?: string;
}
```

- [ ] **Step 3: Add the client (NDJSON reader, like the export-to-path client)**

In the DB API client module, mirroring the existing `exportToPath` NDJSON-reading call (read the stream, parse each line, surface the final `done`/`error`):

```ts
export async function dbImport(
  connId: string,
  body: ImportReq,
  onLine?: (line: ImportResult) => void,
): Promise<ImportResult> {
  // Reuse the project's existing NDJSON streaming helper used by exportToPath.
  return streamNdjson(`/connections/${connId}/db/import`, body, onLine);
}
```

(Use whatever helper `exportToPath` already uses to read the NDJSON stream; import `ImportReq`/`ImportResult`.)

- [ ] **Step 4: Type-check**

Run: `cd ui && npm run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add docs/contracts/api.md ui/src/lib/api/types.ts ui/src/lib/api/
git commit -m "feat(db): contract + TS client for /db/import"
```

---

### Task 6: UI — the Import dialog

**Files:**
- Modify: `ui/src/modules/database/` — add an **Import…** entry next to the existing **Download…** export action (likely in the schema-tree table right-click menu and/or the toolbar), and an `ImportDialog.svelte` mirroring the existing export/Download dialog.

> **Component spec — mirror the existing Download (export-to-path) dialog.**

- [ ] **Step 1: Add the Import dialog**

A dialog with: a **file path** input (reuse the existing folder/file picker the Download dialog uses), a **format** select (`csv` / `tsv` / `ndjson` / `json`), a **target table** field (prefilled when launched from a table node's right-click "Import into…"), and a **batch size** (default 500). A live region shows the streamed result.

- [ ] **Step 2: Run the import and handle the guarded-write case**

On **Import**: call `dbImport(connId, { local_path, format, table, batch_size, confirm_write })`. If the final line is `{ error }` whose text starts with `write_blocked:`, surface the **same typed-confirmation flow the query path already uses** (type the connection name), set `confirm_write: true`, and retry. On `{ done }`, show "Imported N rows in B batches" and refresh the table's structure/row count.

- [ ] **Step 3: Type-check + build**

Run: `cd ui && npm run check && npm run build`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add ui/src/modules/database/
git commit -m "feat(db): Import dialog (file -> table) reusing the guarded-write confirmation"
```

---

### Task 7: Component test + docs + full gate

**Files:**
- Modify: `crates/otto-dbviewer` tests (MySQL testcontainer harness, if present), `docs/features/database-explorer.md`.

- [ ] **Step 1: Component test (MySQL harness)**

Create a table, import a 3-row CSV via `import_from_path`, then `SELECT COUNT(*)` to assert 3 rows landed; assert a guarded connection without `confirm_write` returns a `write_blocked:` error. (If no in-crate harness exists, add to the workspace DB component-test crate per its conventions; the pure-core guarantees from Tasks 1–2 hold regardless.)

- [ ] **Step 2: Document import**

Add a **"§10b Import"** section to `docs/features/database-explorer.md`: formats, header rule, batched INSERTs, that it routes through the **same write guard** (guarded connections need typed confirmation), `Editor`-gated, and the v1 SQL-only scope (Mongo `insertMany` / Redis as follow-ups, also note in §12 Limitations).

- [ ] **Step 3: Full gate**

```bash
cargo build --workspace && cargo test -p otto-dbviewer
cd ui && npm run check
```
Expected: green.

- [ ] **Step 4: Commit**

```bash
git add crates/otto-dbviewer docs/features/database-explorer.md
git commit -m "test(db): import component coverage + document data import"
```

---

## Self-review

- **Reuses the safety layer:** import batches run through `self.run` → `guard_write`; no parallel guard; guarded connections require `confirm_write` (Task 3 + Task 6 retry). ✓
- **Injection-safe:** `sql_string_literal` doubles quotes, `quote_ident` doubles backticks; unit-tested. ✓
- **Role gate Editor:** `import_query` calls `check_conn_role(..Editor)`; tested. ✓
- **Bounded statements:** `build_insert_statements` batches (clamp 1..=5000). ✓
- **Type consistency:** `ImportFormat`, `ParsedTable`, `parse_rows`, `sql_string_literal`, `build_insert_statements`, `ImportCounts`, `import_from_path`, `ImportReq` names match across tasks. ✓
- **Contract lockstep:** Task 5 (`api.md` + `types.ts`), Task 7 (feature doc). ✓
- **Honest scope:** v1 SQL-only; Mongo/Redis return a clear error and are documented follow-ups. The only non-literal UI bits (folder picker, `streamNdjson` helper) explicitly reuse the existing export dialog's components.
- **Memory note (follow-up):** v1 buffers the file before parsing (fine for typical imports); a streaming line-by-line parser for very large files is a documented follow-up — the INSERT batching already bounds per-statement size.

## Execution handoff

Plan complete and saved to `docs/db/features/0002-data-import.md`. Execute with **`superpowers:subagent-driven-development`** or **`superpowers:executing-plans`**. Tasks 1–2 (pure core) land first and are fully test-covered.
