//! Shared types for the DB Explorer: connection config (TLS/SSH), the lazy
//! schema tree, query requests/results, object detail, autocomplete, and
//! per-engine capabilities. These form the stable contract every driver
//! implements — keep them engine-agnostic.

use otto_core::domain::ConnectionKind;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The four supported engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Mysql,
    Redis,
    Mongodb,
    Clickhouse,
}

impl Engine {
    pub fn as_str(&self) -> &'static str {
        match self {
            Engine::Mysql => "mysql",
            Engine::Redis => "redis",
            Engine::Mongodb => "mongodb",
            Engine::Clickhouse => "clickhouse",
        }
    }

    /// Map a stored connection kind to a DB-explorer engine (None for
    /// ssh/custom kinds, which aren't browsable data sources).
    pub fn from_kind(kind: ConnectionKind) -> Option<Engine> {
        match kind {
            ConnectionKind::Mysql => Some(Engine::Mysql),
            ConnectionKind::Redis => Some(Engine::Redis),
            ConnectionKind::Mongodb => Some(Engine::Mongodb),
            ConnectionKind::Clickhouse => Some(Engine::Clickhouse),
            ConnectionKind::Ssh | ConnectionKind::Custom => None,
        }
    }

    pub fn default_port(&self) -> u16 {
        match self {
            Engine::Mysql => 3306,
            Engine::Redis => 6379,
            Engine::Mongodb => 27017,
            Engine::Clickhouse => 8123,
        }
    }
}

// --- TLS --------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// No TLS (default).
    #[default]
    Disabled,
    /// Use TLS if the server offers it; don't fail if it doesn't.
    Preferred,
    /// TLS is mandatory.
    Required,
}

/// TLS material for a connection. PEM strings are inline (the UI may also keep
/// the client key in the keychain and inject it here at resolve time).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    #[serde(default)]
    pub mode: TlsMode,
    /// Verify the server certificate against `ca_cert` / the system roots.
    #[serde(default = "default_true")]
    pub verify: bool,
    /// Inline CA certificate PEM (for private CAs / RDS bundles).
    #[serde(default)]
    pub ca_cert: Option<String>,
    /// Inline client certificate PEM (mutual TLS / X.509 auth).
    #[serde(default)]
    pub client_cert: Option<String>,
    /// Inline client private key PEM.
    #[serde(default)]
    pub client_key: Option<String>,
    /// Override the SNI / hostname used for verification.
    #[serde(default)]
    pub server_name: Option<String>,
}

fn default_true() -> bool {
    true
}

impl TlsConfig {
    pub fn enabled(&self) -> bool {
        !matches!(self.mode, TlsMode::Disabled)
    }
    pub fn required(&self) -> bool {
        matches!(self.mode, TlsMode::Required)
    }
}

// --- SSH tunnel -------------------------------------------------------------

/// SSH tunnel (local port-forward) config. Auth uses the system ssh client, so
/// it honours the ssh-agent, `~/.ssh/config`, and known_hosts. Provide an
/// `identity_file` for key auth, or rely on the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTunnelConfig {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub user: String,
    /// Path to a private key on disk (optional; agent is used otherwise).
    #[serde(default)]
    pub identity_file: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

// --- Resolved config (what a driver receives) -------------------------------

/// A connection profile fully resolved for a driver: the SSH tunnel (if any)
/// is already established by the service, so `host`/`port` are reachable
/// directly. `params` carries the original profile params for engine extras
/// (mongo conn_string/srv/replica_set, redis cluster, clickhouse http scheme…).
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
    pub tls: TlsConfig,
    pub params: Value,
}

impl ResolvedConfig {
    /// Read a string field from the original profile params.
    pub fn param_str(&self, key: &str) -> Option<String> {
        self.params
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// Read a bool field from the original profile params.
    pub fn param_bool(&self, key: &str) -> Option<bool> {
        self.params.get(key).and_then(Value::as_bool)
    }

    /// A stable key identifying every session-affecting facet of this config,
    /// so a driver's connection cache reuses a handle only for a *truly*
    /// equivalent connection. Two configs that differ in anything that changes
    /// the wire session — endpoint, credentials, database, TLS material, or any
    /// param (which includes the session `timezone`) — get different keys, and
    /// thus separate cached handles. (The endpoint is the *resolved* host:port,
    /// so a re-opened SSH tunnel on a new local port naturally rekeys to a fresh
    /// handle.)
    pub fn cache_key(&self) -> String {
        format!(
            "{engine}|{host}|{port}|{user}|{password}|{database}|\
             {tls_mode:?}|{verify}|{ca}|{cert}|{key}|{server_name}|{params}",
            engine = self.engine.as_str(),
            host = self.host,
            port = self.port,
            user = self.user.as_deref().unwrap_or(""),
            password = self.password.as_deref().unwrap_or(""),
            database = self.database.as_deref().unwrap_or(""),
            tls_mode = self.tls.mode,
            verify = self.tls.verify,
            ca = self.tls.ca_cert.as_deref().unwrap_or(""),
            cert = self.tls.client_cert.as_deref().unwrap_or(""),
            key = self.tls.client_key.as_deref().unwrap_or(""),
            server_name = self.tls.server_name.as_deref().unwrap_or(""),
            // The full params blob carries engine extras AND the session
            // timezone — including it keeps two otherwise-identical configs that
            // differ only by timezone on separate cached sessions.
            params = serde_json::to_string(&self.params).unwrap_or_default(),
        )
    }
}

// --- Schema tree ------------------------------------------------------------

/// Kind of a node in the lazy schema tree. Engine-agnostic; not every engine
/// uses every variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Database,
    Schema,
    Table,
    View,
    Column,
    Index,
    Collection,
    Field,
    /// A logical Redis DB (index 0..N).
    Keyspace,
    /// A grouping of Redis keys by prefix (e.g. `session:`).
    KeyNamespace,
    /// A single Redis key.
    Key,
    /// A folder grouping (e.g. "Tables", "Views").
    Folder,
}

/// A node in the lazily-loaded object tree. `id` is an opaque [`NodePath`]
/// string the driver knows how to parse on expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaNode {
    pub id: String,
    pub label: String,
    pub kind: NodeKind,
    /// Optional short detail shown dimmed next to the label (row count, type…).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Whether this node can be expanded (has lazy children).
    pub has_children: bool,
}

impl SchemaNode {
    pub fn new(id: impl Into<String>, label: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind,
            detail: None,
            has_children: false,
        }
    }
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
    pub fn expandable(mut self) -> Self {
        self.has_children = true;
        self
    }
}

/// A parsed node id. Segments are `kind:value` joined by `/`, e.g.
/// `db:shopdb/table:orders`. Drivers build/parse these freely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePath {
    pub segments: Vec<(String, String)>,
}

impl NodePath {
    pub fn parse(raw: &str) -> NodePath {
        let segments = raw
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|seg| match seg.split_once(':') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => (seg.to_string(), String::new()),
            })
            .collect();
        NodePath { segments }
    }

    pub fn to_id(&self) -> String {
        self.segments
            .iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Value of the first segment with the given kind.
    pub fn get(&self, kind: &str) -> Option<&str> {
        self.segments
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, v)| v.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn child(&self, kind: &str, value: &str) -> NodePath {
        let mut segments = self.segments.clone();
        segments.push((kind.to_string(), value.to_string()));
        NodePath { segments }
    }
}

// --- Object detail (Structure tab) ------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// e.g. "PRI", "UNI", "MUL" for SQL engines; free-form otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

/// A foreign-key relationship — the basis for the visual JOIN builder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_schema: Option<String>,
}

/// Full structure of a selected object (table/view/collection/key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectDetail {
    pub name: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub columns: Vec<ColumnDef>,
    #[serde(default)]
    pub primary_key: Vec<String>,
    #[serde(default)]
    pub indexes: Vec<IndexDef>,
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKey>,
    /// DDL / definition (SHOW CREATE TABLE, collection validator, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ddl: Option<String>,
    /// Estimated row/document count when cheaply available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count: Option<i64>,
    /// Engine-specific extras (Redis TTL/type, Mongo sampled types, CH engine…).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

impl ObjectDetail {
    pub fn new(name: impl Into<String>, kind: NodeKind) -> Self {
        Self {
            name: name.into(),
            kind,
            columns: Vec::new(),
            primary_key: Vec::new(),
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            ddl: None,
            row_count: None,
            extra: Value::Null,
        }
    }
}

// --- Query request / result -------------------------------------------------

/// A query/command to run. SQL engines: `statement` is SQL. Redis: one command
/// per line. Mongo: a JSON command or `db.coll.find(...)` shorthand.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryRequest {
    pub statement: String,
    /// Soft cap on returned rows (the driver should apply or enforce it).
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// Optional positional/named params (engine-specific; usually unused here).
    #[serde(default)]
    pub params: Option<Value>,
    /// Optional node context (e.g. the selected database) to scope execution.
    #[serde(default)]
    pub node: Option<String>,
    /// Return the query plan instead of running (Mongo: server `explain`).
    #[serde(default)]
    pub explain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// A human/engine type hint for the column (e.g. "int", "varchar", "json").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_hint: Option<String>,
}

impl Column {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_hint: None,
        }
    }
    pub fn typed(name: impl Into<String>, type_hint: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_hint: Some(type_hint.into()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryStats {
    pub duration_ms: u64,
    pub row_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_read: Option<u64>,
}

/// A tabular result. Non-tabular replies (a Redis OK, a Mongo insert) still use
/// this shape: one column ("result") and one row, with `message` set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
    pub stats: QueryStats,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// True when `max_rows` clipped the result.
    #[serde(default)]
    pub truncated: bool,
}

impl QueryResult {
    pub fn empty() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected: None,
            stats: QueryStats::default(),
            message: None,
            truncated: false,
        }
    }

    /// A single-cell status result (e.g. "OK", "3 rows affected").
    pub fn message(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            columns: vec![Column::new("result")],
            rows: vec![vec![Value::String(text.clone())]],
            rows_affected: None,
            stats: QueryStats::default(),
            message: Some(text),
            truncated: false,
        }
    }
}

// --- Autocomplete -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompletionKind {
    Keyword,
    Function,
    Table,
    View,
    Column,
    Database,
    Collection,
    Field,
    Command,
    Operator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    /// Short one-line description (signature / summary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Text inserted if different from `label` (e.g. `func()` with cursor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
}

impl CompletionItem {
    pub fn new(label: impl Into<String>, kind: CompletionKind) -> Self {
        Self {
            label: label.into(),
            kind,
            detail: None,
            insert_text: None,
        }
    }
    pub fn detailed(label: impl Into<String>, kind: CompletionKind, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            kind,
            detail: Some(detail.into()),
            insert_text: None,
        }
    }
}

/// Context the editor sends to scope completions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionContext {
    /// The text up to the cursor (the driver may parse the last token / scope).
    #[serde(default)]
    pub prefix: String,
    /// Currently-selected database/keyspace, if any.
    #[serde(default)]
    pub database: Option<String>,
    /// The node id currently selected in the tree (e.g. a table), if any.
    #[serde(default)]
    pub node: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub items: Vec<CompletionItem>,
}

// --- Test result & capabilities ---------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    pub message: String,
    /// e.g. server version string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
}

/// What an engine supports — drives UI affordances (JOIN builder, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub engine: Engine,
    pub sql: bool,
    pub joins: bool,
    pub transactions: bool,
    pub multi_statement: bool,
    pub default_port: u16,
    /// Human labels for the tree levels (e.g. ["Database","Table","Column"]).
    pub schema_levels: Vec<String>,
    /// Hint for the editor language mode: "sql" | "redis" | "mongo".
    pub query_language: String,
}

// --- Helpers ----------------------------------------------------------------

/// Standard "not yet implemented" error a stub driver returns. Replaced by the
/// per-engine agent with the real implementation.
pub fn unimplemented(engine: Engine, op: &str) -> Error {
    Error::Internal(format!("{}: {op} not implemented", engine.as_str()))
}

/// Convenience: turn any driver-level error into [`otto_core::Error::Upstream`]
/// (a 502 — the failure is the database's, not the request's).
pub fn upstream<E: std::fmt::Display>(e: E) -> Error {
    Error::Upstream(e.to_string())
}

/// Convenience: an invalid-request error (400).
pub fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

/// Append `LIMIT n` to a single read statement that doesn't already constrain
/// its row count, so a huge table isn't fully scanned/streamed before we clip
/// it. Conservative: returns the statement UNCHANGED when it already has a
/// `LIMIT`, spans multiple statements, or uses a clause where a trailing LIMIT
/// would be invalid/ambiguous (FORMAT/SETTINGS/INTO OUTFILE/INTO DUMPFILE/
/// FOR UPDATE/FOR SHARE/UNION/LIMIT BY). Callers should pass this only for
/// statements they've already classified as row-returning reads.
pub fn inject_row_limit(statement: &str, limit: usize) -> String {
    let trimmed = statement.trim().trim_end_matches(';').trim_end();
    let lower = trimmed.to_ascii_lowercase();
    // Only SELECT (incl. `WITH … SELECT` and a parenthesized `(SELECT …)`)
    // accepts a trailing LIMIT. Statements like SHOW / DESCRIBE / DESC / EXISTS
    // / EXPLAIN are row-returning but REJECT a trailing LIMIT, so never touch
    // them — appending `LIMIT n` there is a syntax error.
    let head = strip_leading_comments(&lower);
    let first_word: String = head.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if first_word != "select" && first_word != "with" && !head.starts_with('(') {
        return statement.to_string();
    }
    // Multiple statements — too risky to rewrite.
    if trimmed.contains(';') {
        return statement.to_string();
    }
    // Already constrained: `limit <digit>` somewhere (honors offset,count and
    // `LIMIT n OFFSET m`). A column literally named `limit` won't match because
    // it isn't followed by a digit.
    if has_word_then_digit(&lower, "limit") {
        return statement.to_string();
    }
    // Clauses after which a trailing LIMIT is invalid or changes meaning.
    const SKIP: &[&str] = &[
        "format ", "settings ", "into outfile", "into dumpfile", "for update",
        "for share", " union ", "limit by",
    ];
    for kw in SKIP {
        if lower.contains(kw) {
            return statement.to_string();
        }
    }
    format!("{trimmed} LIMIT {limit}")
}

/// Strip leading whitespace and SQL line (`--`) / block (`/* */`) comments,
/// returning the remainder — used to find a statement's first keyword.
fn strip_leading_comments(sql: &str) -> &str {
    let mut s = sql.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            s = rest.splitn(2, '\n').nth(1).unwrap_or("").trim_start();
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.splitn(2, "*/").nth(1).unwrap_or("").trim_start();
        } else {
            break;
        }
    }
    s
}

/// True if `word` appears as a whole word (surrounded by non-alphanumeric/non-`_`
/// boundaries) immediately followed (after spaces) by a digit. Used to detect an
/// existing `LIMIT 123` while ignoring identifiers like `rate_limit`.
fn has_word_then_digit(haystack: &str, word: &str) -> bool {
    let bytes = haystack.as_bytes();
    let mut from = 0;
    while let Some(pos) = haystack[from..].find(word) {
        let start = from + pos;
        let end = start + word.len();
        let before_ok = start == 0
            || !matches!(bytes[start - 1], b'a'..=b'z' | b'0'..=b'9' | b'_');
        // after the word, skip spaces, then require a digit
        let mut i = end;
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        let after_ok = i < bytes.len() && bytes[i].is_ascii_digit();
        if before_ok && after_ok {
            return true;
        }
        from = end;
    }
    false
}

/// Re-export for drivers.
pub type DbResult<T> = Result<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injects_limit_on_plain_select() {
        assert_eq!(
            inject_row_limit("SELECT * FROM t", 1000),
            "SELECT * FROM t LIMIT 1000"
        );
    }

    #[test]
    fn strips_trailing_semicolon_before_appending() {
        assert_eq!(
            inject_row_limit("SELECT * FROM t;", 1000),
            "SELECT * FROM t LIMIT 1000"
        );
    }

    #[test]
    fn leaves_existing_limit_untouched() {
        assert_eq!(
            inject_row_limit("SELECT * FROM t LIMIT 5", 1000),
            "SELECT * FROM t LIMIT 5"
        );
    }

    #[test]
    fn appends_after_order_by() {
        assert!(inject_row_limit("select a from t order by a", 50).ends_with(" LIMIT 50"));
    }

    #[test]
    fn rate_limit_identifier_is_not_a_limit_clause() {
        assert_eq!(
            inject_row_limit("SELECT rate_limit FROM t", 10),
            "SELECT rate_limit FROM t LIMIT 10"
        );
    }

    #[test]
    fn union_is_left_untouched() {
        assert_eq!(
            inject_row_limit("SELECT * FROM a UNION SELECT * FROM b", 10),
            "SELECT * FROM a UNION SELECT * FROM b"
        );
    }

    #[test]
    fn format_clause_is_left_untouched() {
        assert_eq!(
            inject_row_limit("SELECT * FROM t FORMAT JSON", 10),
            "SELECT * FROM t FORMAT JSON"
        );
    }

    #[test]
    fn multi_statement_is_left_untouched() {
        assert_eq!(
            inject_row_limit("SELECT 1; SELECT 2", 10),
            "SELECT 1; SELECT 2"
        );
    }

    #[test]
    fn non_select_row_returning_statements_are_left_untouched() {
        // These return rows but REJECT a trailing LIMIT — must never be rewritten.
        for sql in [
            "SHOW CREATE TABLE etl_aws.daily",
            "show tables",
            "DESCRIBE etl_aws.daily",
            "DESC t",
            "EXPLAIN SELECT * FROM t",
            "EXISTS TABLE t",
        ] {
            assert_eq!(inject_row_limit(sql, 1000), sql, "must not touch: {sql}");
        }
    }

    #[test]
    fn injects_after_leading_comment_and_for_cte_and_paren() {
        assert!(inject_row_limit("-- pick\nSELECT * FROM t", 10).ends_with(" LIMIT 10"));
        assert!(inject_row_limit("WITH c AS (SELECT 1) SELECT * FROM c", 10).ends_with(" LIMIT 10"));
        assert!(inject_row_limit("(SELECT * FROM t)", 10).ends_with(" LIMIT 10"));
    }
}
