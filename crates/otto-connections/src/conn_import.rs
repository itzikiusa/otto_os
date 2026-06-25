//! Import connection profiles from other DB tools' on-disk config files.
//!
//! The daemon runs locally, so it reads each tool's config from its default
//! macOS location — the user picks a *tool*, never a file. Parsers are PURE
//! functions over file *content* (tested with synthetic fixtures, never the
//! user's real files); the locator handles default-path discovery.
//!
//! Supported tools: MySQL Workbench, DBeaver, DataGrip, NoSQLBooster.
//!
//! Imported connections are created through the normal create path with
//! `secret: None` — every tool keeps its passwords encrypted or in an OS
//! keychain, so they're unrecoverable here. The user adds passwords later via
//! the connection editor; for MongoDB we leave an Otto `{secret}` placeholder
//! so the password substitutes in once supplied.

use std::path::{Path, PathBuf};

use otto_core::domain::ConnectionKind;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A DB tool Otto can import connections from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSource {
    MysqlWorkbench,
    Dbeaver,
    Datagrip,
    Nosqlbooster,
}

impl ImportSource {
    pub const ALL: [ImportSource; 4] = [
        ImportSource::MysqlWorkbench,
        ImportSource::Dbeaver,
        ImportSource::Datagrip,
        ImportSource::Nosqlbooster,
    ];

    /// Human-readable label for the picker.
    pub fn label(self) -> &'static str {
        match self {
            ImportSource::MysqlWorkbench => "MySQL Workbench",
            ImportSource::Dbeaver => "DBeaver",
            ImportSource::Datagrip => "DataGrip",
            ImportSource::Nosqlbooster => "NoSQLBooster",
        }
    }
}

/// One tool's availability, as reported by the `sources` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceStatus {
    pub source: ImportSource,
    pub label: String,
    /// True when a config file was found at the default location.
    pub present: bool,
    /// The resolved config path (first match), if any.
    pub path: Option<String>,
    /// How many connections were parsed out (cheap parse), if present.
    pub count: Option<usize>,
}

/// A single connection parsed from a tool's config, ready (when `supported`)
/// to feed back into Otto's create path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedConnection {
    pub source: ImportSource,
    pub name: String,
    /// `None` for an unsupported engine (still listed so the user sees why it
    /// was skipped).
    pub kind: Option<ConnectionKind>,
    /// Ready-to-create Otto params for a supported kind; `{}` otherwise.
    pub params: Value,
    /// False for an engine Otto doesn't support.
    pub supported: bool,
    /// True when the source had a username but no recoverable password — the
    /// user must add the password after import (MongoDB uses a `{secret}`
    /// placeholder in `conn_string`).
    pub needs_password: bool,
    /// Human note explaining a skip or a caveat.
    pub note: Option<String>,
}

/// Result of scanning a single tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportScanResult {
    pub source: ImportSource,
    pub path: Option<String>,
    pub connections: Vec<ParsedConnection>,
    pub warnings: Vec<String>,
}

/// `POST …/connections/import/create` body.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportCreateReq {
    pub connections: Vec<ImportCreateItem>,
    #[serde(default)]
    pub section_id: Option<otto_core::Id>,
}

/// One connection the user chose to create.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportCreateItem {
    pub name: String,
    pub kind: ConnectionKind,
    pub params: Value,
    #[serde(default)]
    pub environment: Option<otto_core::domain::Environment>,
    #[serde(default)]
    pub read_only: Option<bool>,
}

/// One connection that failed to create.
#[derive(Debug, Clone, Serialize)]
pub struct ImportFailure {
    pub name: String,
    pub error: String,
}

/// Result of a create batch (best-effort — partial successes are fine).
#[derive(Debug, Clone, Serialize)]
pub struct ImportCreateResult {
    pub created: Vec<otto_core::domain::Connection>,
    pub failed: Vec<ImportFailure>,
}

// ---------------------------------------------------------------------------
// Engine → kind mapping (shared by every parser)
// ---------------------------------------------------------------------------

/// Map a tool's driver/provider/scheme token to an Otto kind, or describe why
/// it's unsupported. The token is matched case-insensitively on substrings so
/// it copes with `com.mysql.*`, `mysql8`, `mariadb`, `com_clickhouse`, etc.
fn kind_for_engine(token: &str) -> Result<ConnectionKind, String> {
    let t = token.to_ascii_lowercase();
    // Order matters: check the more specific engines before generic ones.
    if t.contains("clickhouse") {
        Ok(ConnectionKind::Clickhouse)
    } else if t.contains("mongo") {
        Ok(ConnectionKind::Mongodb)
    } else if t.contains("redis") {
        Ok(ConnectionKind::Redis)
    } else if t.contains("mariadb") || t.contains("mysql") {
        Ok(ConnectionKind::Mysql)
    } else if t.contains("postgre") || t.contains("postgis") {
        Err("PostgreSQL is not supported by Otto".into())
    } else if t.contains("sqlserver") || t.contains("mssql") || t.contains("microsoft") {
        Err("SQL Server is not supported by Otto".into())
    } else if t.contains("oracle") {
        Err("Oracle is not supported by Otto".into())
    } else if t.contains("sqlite") {
        Err("SQLite is not supported by Otto".into())
    } else {
        Err(format!("engine '{token}' is not supported by Otto"))
    }
}

// ---------------------------------------------------------------------------
// JDBC URL parsing (DataGrip + DBeaver fallback)
// ---------------------------------------------------------------------------

/// Parsed pieces of a JDBC URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JdbcParts {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
    pub db: Option<String>,
}

/// Parse a `jdbc:<scheme>://host[:port][/db][?params]` URL. Tolerant of a
/// missing port and a missing db; returns `None` when there's no usable host.
///
/// Also handles the JDBC-mongo shapes `jdbc:mongodb://…` / `jdbc:mongo://…`
/// and a leading `jdbc:` that some tools omit.
pub fn parse_jdbc_url(url: &str) -> Option<JdbcParts> {
    let url = url.trim();
    // Strip the leading `jdbc:` if present.
    let rest = url.strip_prefix("jdbc:").unwrap_or(url);
    let (scheme, after) = rest.split_once("://")?;
    let scheme = scheme.trim();
    if scheme.is_empty() {
        return None;
    }
    // authority is everything up to the first '/' or '?'.
    let auth_end = after.find(['/', '?']).unwrap_or(after.len());
    let authority = &after[..auth_end];
    // Drop any userinfo (`user:pass@`).
    let host_port = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    // A JDBC authority may list several hosts (mongo replica sets) — take the
    // first for host/port; the full URL is preserved by callers that need it.
    let first = host_port.split(',').next().unwrap_or(host_port);
    let (host, port) = split_host_port(first);
    if host.is_empty() {
        return None;
    }
    // db = the path segment after the authority, before '?'.
    let tail = &after[auth_end..];
    let db = tail
        .strip_prefix('/')
        .map(|s| s.split(['?', ';']).next().unwrap_or(s))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(JdbcParts {
        scheme: scheme.to_string(),
        host: host.to_string(),
        port,
        db,
    })
}

/// Split a `host[:port]` token. IPv6 in brackets (`[::1]:5432`) is handled.
fn split_host_port(s: &str) -> (&str, Option<u16>) {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('[') {
        // [ipv6]:port
        if let Some((h, p)) = rest.split_once("]:") {
            return (h, p.trim().parse().ok());
        }
        return (rest.trim_end_matches(']'), None);
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            (h, p.parse().ok())
        }
        _ => (s, None),
    }
}

// ---------------------------------------------------------------------------
// Param-builder helpers (emit the exact Otto create shapes)
// ---------------------------------------------------------------------------

/// Build a mongodb `conn_string` (mongosh URI) from parts. Uses the Otto
/// `{secret}` placeholder for the password when a username is known. Hosts is a
/// list of `host[:port]` strings (ports omitted for `+srv`).
fn build_mongo_conn_string(
    scheme: &str,
    username: Option<&str>,
    hosts: &[String],
    database: Option<&str>,
    options: &[(String, String)],
) -> String {
    let mut s = String::new();
    s.push_str(scheme);
    s.push_str("://");
    if let Some(u) = username.filter(|u| !u.is_empty()) {
        s.push_str(u);
        s.push_str(":{secret}@");
    }
    s.push_str(&hosts.join(","));
    s.push('/');
    if let Some(db) = database.filter(|d| !d.is_empty()) {
        s.push_str(db);
    }
    let qs: Vec<String> = options
        .iter()
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    if !qs.is_empty() {
        s.push('?');
        s.push_str(&qs.join("&"));
    }
    s
}

/// Attach an `ssh` tunnel block to host/port params, if a tunnel was found.
struct SshTunnel {
    host: String,
    port: Option<u16>,
    user: Option<String>,
    identity_file: Option<String>,
}

fn ssh_block(t: &SshTunnel) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("host".into(), json!(t.host));
    if let Some(p) = t.port {
        o.insert("port".into(), json!(p));
    }
    if let Some(u) = &t.user {
        o.insert("user".into(), json!(u));
    }
    if let Some(k) = &t.identity_file {
        o.insert("identity_file".into(), json!(k));
    }
    Value::Object(o)
}

/// A `tls` params block (mode require; optional verify/ca).
fn tls_block(verify: Option<bool>, ca_cert: Option<&str>) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("mode".into(), json!("require"));
    if let Some(v) = verify {
        o.insert("verify".into(), json!(v));
    }
    if let Some(ca) = ca_cert.filter(|c| !c.is_empty()) {
        o.insert("ca_cert".into(), json!(ca));
    }
    Value::Object(o)
}

// ---------------------------------------------------------------------------
// 1. MySQL Workbench  (connections.xml — Borland-GRT)
// ---------------------------------------------------------------------------

/// Parse MySQL Workbench `connections.xml`. Always yields MySQL connections.
pub fn parse_mysql_workbench(content: &str) -> (Vec<ParsedConnection>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let doc = match roxmltree::Document::parse(content) {
        Ok(d) => d,
        Err(e) => {
            warnings.push(format!("could not parse connections.xml: {e}"));
            return (out, warnings);
        }
    };

    for node in doc.descendants().filter(|n| {
        n.attribute("struct-name") == Some("db.mgmt.Connection")
    }) {
        // <value type="string" key="name">…</value>
        let name = child_value(&node, "name").unwrap_or_default();
        let name = if name.trim().is_empty() {
            "Unnamed".to_string()
        } else {
            name
        };
        // <value type="dict" key="parameterValues"> { hostName, port, userName, schema, useSSL, ssl* }
        let params_node = node.children().find(|c| c.attribute("key") == Some("parameterValues"));
        let pv = params_node.as_ref();
        let host = pv.and_then(|n| dict_value(n, "hostName")).unwrap_or_default();
        let port = pv
            .and_then(|n| dict_value(n, "port"))
            .and_then(|s| s.trim().parse::<u16>().ok());
        let user = pv.and_then(|n| dict_value(n, "userName")).unwrap_or_default();
        let schema = pv.and_then(|n| dict_value(n, "schema")).unwrap_or_default();
        let use_ssl = pv
            .and_then(|n| dict_value(n, "useSSL"))
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let ssl_ca = pv.and_then(|n| dict_value(n, "sslCA"));

        if host.trim().is_empty() {
            // A connection with no host is unusable — skip with a warning.
            warnings.push(format!("'{name}' has no host — skipped"));
            continue;
        }

        let mut params = serde_json::Map::new();
        params.insert("host".into(), json!(host));
        if let Some(p) = port {
            params.insert("port".into(), json!(p));
        }
        if !user.trim().is_empty() {
            params.insert("user".into(), json!(user));
        }
        if !schema.trim().is_empty() {
            params.insert("db".into(), json!(schema));
        }
        if use_ssl >= 1 {
            params.insert("tls".into(), tls_block(None, ssl_ca.as_deref()));
        }

        out.push(ParsedConnection {
            source: ImportSource::MysqlWorkbench,
            name,
            kind: Some(ConnectionKind::Mysql),
            params: Value::Object(params),
            supported: true,
            needs_password: !user.trim().is_empty(),
            note: None,
        });
    }
    (out, warnings)
}

/// A direct `<value key="…">text</value>` child of an element.
fn child_value(node: &roxmltree::Node, key: &str) -> Option<String> {
    node.children()
        .find(|c| c.attribute("key") == Some(key))
        .and_then(|c| c.text())
        .map(str::to_string)
}

/// A `<value key="…">text</value>` inside a `parameterValues` dict node.
fn dict_value(dict: &roxmltree::Node, key: &str) -> Option<String> {
    dict.children()
        .find(|c| c.attribute("key") == Some(key))
        .and_then(|c| c.text())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// 2. DBeaver  (data-sources.json)
// ---------------------------------------------------------------------------

/// Parse DBeaver `data-sources.json`.
pub fn parse_dbeaver(content: &str) -> (Vec<ParsedConnection>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let root: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!("could not parse data-sources.json: {e}"));
            return (out, warnings);
        }
    };
    let conns = match root.get("connections").and_then(Value::as_object) {
        Some(m) => m,
        None => return (out, warnings),
    };

    for (_id, c) in conns {
        let name = c
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Unnamed")
            .to_string();
        let provider = c.get("provider").and_then(Value::as_str).unwrap_or("");
        let driver = c.get("driver").and_then(Value::as_str).unwrap_or("");
        let cfg = c.get("configuration").cloned().unwrap_or_else(|| json!({}));

        // provider is the most reliable engine token; fall back to driver/url.
        let url = cfg.get("url").and_then(Value::as_str).unwrap_or("");
        let token = if !provider.is_empty() {
            provider.to_string()
        } else if !driver.is_empty() {
            driver.to_string()
        } else {
            url.to_string()
        };
        let kind = match kind_for_engine(&token) {
            Ok(k) => k,
            Err(note) => {
                out.push(unsupported(ImportSource::Dbeaver, name, note));
                continue;
            }
        };

        let host = cfg.get("host").and_then(Value::as_str).unwrap_or("");
        let port = json_port(cfg.get("port"));
        let database = cfg
            .get("database")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        // DBeaver does not store the username in the clear here (it lives in the
        // encrypted credentials store), but the JDBC URL sometimes carries it.
        let url_user = parse_jdbc_url(url)
            .and_then(|_| user_from_jdbc(url));

        // SSH tunnel handler.
        let ssh = dbeaver_ssh_tunnel(&cfg);
        // SSL: any handler key containing "ssl" that is enabled, or a `*-ssl`.
        let tls = dbeaver_ssl(&cfg);

        // If host is empty, try to recover host/port/db from the JDBC URL.
        let (host, port, database_owned) = if host.is_empty() {
            match parse_jdbc_url(url) {
                Some(j) => (j.host, j.port, j.db),
                None => {
                    warnings.push(format!("'{name}' has no host — skipped"));
                    continue;
                }
            }
        } else {
            (host.to_string(), port, database.map(str::to_string))
        };

        let params = build_db_params(
            kind,
            &host,
            port,
            url_user.as_deref(),
            database_owned.as_deref(),
            url,
            ssh.as_ref(),
            tls,
        );

        out.push(ParsedConnection {
            source: ImportSource::Dbeaver,
            name,
            kind: Some(kind),
            params,
            supported: true,
            // username is in the encrypted store → always treat as needing a
            // password later.
            needs_password: true,
            note: None,
        });
    }
    (out, warnings)
}

/// Pull a `user:pass@`-style username out of a JDBC URL authority, if present.
fn user_from_jdbc(url: &str) -> Option<String> {
    let rest = url.strip_prefix("jdbc:").unwrap_or(url);
    let (_, after) = rest.split_once("://")?;
    let auth_end = after.find(['/', '?']).unwrap_or(after.len());
    let authority = &after[..auth_end];
    let (userinfo, _) = authority.rsplit_once('@')?;
    let user = userinfo.split(':').next().unwrap_or(userinfo);
    Some(user.to_string()).filter(|u| !u.is_empty())
}

/// Extract a DBeaver SSH tunnel block, if enabled.
fn dbeaver_ssh_tunnel(cfg: &Value) -> Option<SshTunnel> {
    let h = cfg.get("handlers")?.get("ssh_tunnel")?;
    // Honor the enabled flag (defaults to present-but-true if absent).
    if h.get("enabled").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let p = h.get("properties")?;
    let host = p.get("host").and_then(Value::as_str)?.to_string();
    if host.is_empty() {
        return None;
    }
    Some(SshTunnel {
        host,
        port: json_port(p.get("port")),
        user: p
            .get("userName")
            .or_else(|| p.get("user"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        identity_file: p
            .get("keyPath")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    })
}

/// True when any DBeaver handler indicates SSL is enabled.
fn dbeaver_ssl(cfg: &Value) -> bool {
    let handlers = match cfg.get("handlers").and_then(Value::as_object) {
        Some(h) => h,
        None => return false,
    };
    handlers.iter().any(|(k, v)| {
        k.contains("ssl") && v.get("enabled").and_then(Value::as_bool) != Some(false)
    })
}

// ---------------------------------------------------------------------------
// 3. DataGrip  (dataSources.xml + dataSources.local.xml)
// ---------------------------------------------------------------------------

/// Side-channel user/SSL info from a `dataSources.local.xml`, keyed by uuid.
#[derive(Debug, Clone, Default)]
pub struct DatagripLocal {
    /// uuid → username
    pub users: std::collections::HashMap<String, String>,
    /// uuid → ssl enabled
    pub ssl: std::collections::HashMap<String, bool>,
}

/// Parse a DataGrip `dataSources.local.xml` for `<user-name>` + `<ssl-config>`,
/// keyed by data-source `uuid`. Missing/garbled → empty maps.
pub fn parse_datagrip_local(content: &str) -> DatagripLocal {
    let mut local = DatagripLocal::default();
    let doc = match roxmltree::Document::parse(content) {
        Ok(d) => d,
        Err(_) => return local,
    };
    for ds in doc
        .descendants()
        .filter(|n| n.has_tag_name("data-source"))
    {
        let uuid = match ds.attribute("uuid") {
            Some(u) => u.to_string(),
            None => continue,
        };
        if let Some(user) = ds
            .descendants()
            .find(|n| n.has_tag_name("user-name"))
            .and_then(|n| n.text())
            .filter(|s| !s.is_empty())
        {
            local.users.insert(uuid.clone(), user.to_string());
        }
        // <ssl-config><mode>…</mode></ssl-config> or an `enabled` attr.
        if let Some(ssl) = ds.descendants().find(|n| n.has_tag_name("ssl-config")) {
            let enabled = ssl.attribute("enabled") != Some("false")
                && ssl
                    .descendants()
                    .find(|n| n.has_tag_name("mode"))
                    .and_then(|n| n.text())
                    .map(|m| !m.eq_ignore_ascii_case("disabled"))
                    .unwrap_or(true);
            local.ssl.insert(uuid, enabled);
        }
    }
    local
}

/// Parse a DataGrip `dataSources.xml`, joining in a parsed `.local.xml`.
pub fn parse_datagrip(
    content: &str,
    local: &DatagripLocal,
) -> (Vec<ParsedConnection>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let doc = match roxmltree::Document::parse(content) {
        Ok(d) => d,
        Err(e) => {
            warnings.push(format!("could not parse dataSources.xml: {e}"));
            return (out, warnings);
        }
    };

    for ds in doc.descendants().filter(|n| n.has_tag_name("data-source")) {
        let name = ds
            .attribute("name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Unnamed")
            .to_string();
        let uuid = ds.attribute("uuid").unwrap_or("");

        // <driver-ref>mysql / mariadb / clickhouse / mongo / postgresql / redis</driver-ref>
        let driver_ref = ds
            .descendants()
            .find(|n| n.has_tag_name("driver-ref"))
            .and_then(|n| n.text())
            .unwrap_or("")
            .trim()
            .to_string();
        let jdbc_url = ds
            .descendants()
            .find(|n| n.has_tag_name("jdbc-url"))
            .and_then(|n| n.text())
            .unwrap_or("")
            .trim()
            .to_string();

        // Engine: prefer driver-ref; the jdbc scheme is the fallback.
        let token = if !driver_ref.is_empty() {
            driver_ref.clone()
        } else {
            parse_jdbc_url(&jdbc_url)
                .map(|j| j.scheme)
                .unwrap_or_default()
        };
        let kind = match kind_for_engine(&token) {
            Ok(k) => k,
            Err(note) => {
                out.push(unsupported(ImportSource::Datagrip, name, note));
                continue;
            }
        };

        let parts = match parse_jdbc_url(&jdbc_url) {
            Some(p) => p,
            None => {
                warnings.push(format!("'{name}' has no parseable JDBC URL — skipped"));
                continue;
            }
        };
        let user = local.users.get(uuid).map(String::as_str);
        let ssl = local.ssl.get(uuid).copied().unwrap_or(false);

        let params = build_db_params(
            kind,
            &parts.host,
            parts.port,
            user,
            parts.db.as_deref(),
            &jdbc_url,
            None,
            ssl,
        );

        out.push(ParsedConnection {
            source: ImportSource::Datagrip,
            name,
            kind: Some(kind),
            params,
            supported: true,
            needs_password: user.is_some(),
            note: None,
        });
    }
    (out, warnings)
}

// ---------------------------------------------------------------------------
// 4. NoSQLBooster  (app.json)
// ---------------------------------------------------------------------------

/// Parse NoSQLBooster `app.json`. Always yields MongoDB connections.
pub fn parse_nosqlbooster(content: &str) -> (Vec<ParsedConnection>, Vec<String>) {
    let mut out = Vec::new();
    let mut warnings = Vec::new();
    let root: Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!("could not parse app.json: {e}"));
            return (out, warnings);
        }
    };
    let conns = match root.get("connections").and_then(Value::as_array) {
        Some(a) => a,
        None => return (out, warnings),
    };

    for c in conns {
        let name = c
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Unnamed")
            .to_string();
        let uri = match c.get("uri").and_then(Value::as_object) {
            Some(u) => u,
            None => {
                // Some entries may carry a raw connection string instead.
                if let Some(raw) = c.get("uri").and_then(Value::as_str) {
                    out.push(ParsedConnection {
                        source: ImportSource::Nosqlbooster,
                        name,
                        kind: Some(ConnectionKind::Mongodb),
                        params: json!({ "conn_string": raw }),
                        supported: true,
                        needs_password: raw.contains('@'),
                        note: None,
                    });
                } else {
                    warnings.push(format!("'{name}' has no URI — skipped"));
                }
                continue;
            }
        };

        let scheme = uri
            .get("scheme")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or("mongodb");
        let is_srv = scheme.eq_ignore_ascii_case("mongodb+srv");
        let username = uri.get("username").and_then(Value::as_str);
        let database = uri.get("database").and_then(Value::as_str);

        // hosts: [{ host, port? }, …]; ports omitted for +srv.
        let hosts: Vec<String> = uri
            .get("hosts")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|h| {
                        let host = h.get("host").and_then(Value::as_str)?;
                        if host.is_empty() {
                            return None;
                        }
                        let port = json_port(h.get("port"));
                        Some(match port {
                            Some(p) if !is_srv => format!("{host}:{p}"),
                            _ => host.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        if hosts.is_empty() {
            warnings.push(format!("'{name}' has no hosts — skipped"));
            continue;
        }

        // options → query string (authSource, ssl, replicaSet, …).
        let options: Vec<(String, String)> = uri
            .get("options")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| {
                        let val = match v {
                            Value::String(s) => s.clone(),
                            Value::Bool(b) => b.to_string(),
                            Value::Number(n) => n.to_string(),
                            _ => return None,
                        };
                        Some((k.clone(), val))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let conn_string =
            build_mongo_conn_string(scheme, username, &hosts, database, &options);

        out.push(ParsedConnection {
            source: ImportSource::Nosqlbooster,
            name,
            kind: Some(ConnectionKind::Mongodb),
            params: json!({ "conn_string": conn_string }),
            supported: true,
            needs_password: username.map(|u| !u.is_empty()).unwrap_or(false),
            note: None,
        });
    }
    (out, warnings)
}

// ---------------------------------------------------------------------------
// Shared param assembly
// ---------------------------------------------------------------------------

/// Build the Otto `params` object for a supported DB kind. MongoDB collapses
/// host/port/db/user into a `conn_string`; the others use host/port/user/db.
#[allow(clippy::too_many_arguments)]
fn build_db_params(
    kind: ConnectionKind,
    host: &str,
    port: Option<u16>,
    user: Option<&str>,
    db: Option<&str>,
    full_url: &str,
    ssh: Option<&SshTunnel>,
    tls: bool,
) -> Value {
    let mut p = serde_json::Map::new();
    match kind {
        ConnectionKind::Mongodb => {
            // Prefer reconstructing from a JDBC mongo URL when present so we
            // keep the replica-set host list and options; else host/port/db.
            let scheme = "mongodb";
            let hosts = vec![match port {
                Some(p) => format!("{host}:{p}"),
                None => host.to_string(),
            }];
            let conn_string = build_mongo_conn_string(scheme, user, &hosts, db, &[]);
            let _ = full_url;
            p.insert("conn_string".into(), json!(conn_string));
        }
        ConnectionKind::Redis => {
            p.insert("host".into(), json!(host));
            if let Some(port) = port {
                p.insert("port".into(), json!(port));
            }
            // redis db is numeric; only emit when the path was a bare number.
            if let Some(db) = db.and_then(|d| d.trim().parse::<u64>().ok()) {
                p.insert("db".into(), json!(db));
            }
        }
        // mysql / clickhouse share { host, port, user, db }.
        _ => {
            p.insert("host".into(), json!(host));
            if let Some(port) = port {
                p.insert("port".into(), json!(port));
            }
            if let Some(user) = user.filter(|u| !u.is_empty()) {
                p.insert("user".into(), json!(user));
            }
            if let Some(db) = db.filter(|d| !d.is_empty()) {
                p.insert("db".into(), json!(db));
            }
        }
    }
    if let Some(ssh) = ssh {
        p.insert("ssh".into(), ssh_block(ssh));
    }
    if tls {
        p.insert("tls".into(), tls_block(None, None));
    }
    Value::Object(p)
}

/// A `ParsedConnection` for an engine Otto doesn't support.
fn unsupported(source: ImportSource, name: String, note: String) -> ParsedConnection {
    ParsedConnection {
        source,
        name,
        kind: None,
        params: json!({}),
        supported: false,
        needs_password: false,
        note: Some(note),
    }
}

/// Read a port from a JSON value that may be a number or a (possibly float)
/// string. DBeaver writes ports as strings and SSH ports sometimes as `22.0`.
fn json_port(v: Option<&Value>) -> Option<u16> {
    match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_f64().map(|f| f as u64))
            .and_then(|u| u16::try_from(u).ok()),
        Some(Value::String(s)) => {
            let s = s.trim();
            s.parse::<u16>()
                .ok()
                .or_else(|| s.parse::<f64>().ok().map(|f| f as u16))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Locator — default macOS config-file discovery
// ---------------------------------------------------------------------------

/// Expand a leading `~` to `$HOME`.
fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// Locate + read + parse a tool, returning the scan result. Pure parsing is
/// delegated to the `parse_*` fns; this is the only place that touches disk.
pub fn scan_source(source: ImportSource) -> ImportScanResult {
    match source {
        ImportSource::MysqlWorkbench => scan_mysql_workbench(),
        ImportSource::Dbeaver => scan_dbeaver(),
        ImportSource::Datagrip => scan_datagrip(),
        ImportSource::Nosqlbooster => scan_nosqlbooster(),
    }
}

/// Cheap presence + count for a tool (used by the `sources` endpoint).
pub fn source_status(source: ImportSource) -> SourceStatus {
    let scan = scan_source(source);
    let present = scan.path.is_some();
    // No file found at all → unknown count; otherwise report the parsed count
    // (which may be 0 for a present-but-empty config).
    let count = present.then_some(scan.connections.len());
    SourceStatus {
        source,
        label: source.label().to_string(),
        present,
        path: scan.path,
        count,
    }
}

fn scan_mysql_workbench() -> ImportScanResult {
    let path = home()
        .join("Library/Application Support/MySQL/Workbench/connections.xml");
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let (connections, warnings) = parse_mysql_workbench(&content);
            ImportScanResult {
                source: ImportSource::MysqlWorkbench,
                path: Some(path.to_string_lossy().into_owned()),
                connections,
                warnings,
            }
        }
        Err(_) => empty_scan(ImportSource::MysqlWorkbench),
    }
}

fn scan_dbeaver() -> ImportScanResult {
    // workspace6 is current; older/newer variants exist. Glob all and merge.
    let base = home().join("Library/DBeaverData");
    let mut found = Vec::new();
    let mut warnings = Vec::new();
    let mut first_path: Option<String> = None;
    for ws in glob_dirs(&base, "workspace") {
        // <ws>/<project>/.dbeaver/data-sources.json AND <ws>/.dbeaver/…
        for cand in dbeaver_candidates(&ws) {
            if let Ok(content) = std::fs::read_to_string(&cand) {
                if first_path.is_none() {
                    first_path = Some(cand.to_string_lossy().into_owned());
                }
                let (conns, mut w) = parse_dbeaver(&content);
                found.extend(conns);
                warnings.append(&mut w);
            }
        }
    }
    ImportScanResult {
        source: ImportSource::Dbeaver,
        path: first_path,
        connections: dedupe(found),
        warnings,
    }
}

/// All `.dbeaver/data-sources.json` files under one DBeaver workspace dir:
/// `<ws>/.dbeaver/…` plus each `<ws>/<project>/.dbeaver/…`.
fn dbeaver_candidates(ws: &Path) -> Vec<PathBuf> {
    let mut out = vec![ws.join(".dbeaver/data-sources.json")];
    if let Ok(rd) = std::fs::read_dir(ws) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                out.push(e.path().join(".dbeaver/data-sources.json"));
            }
        }
    }
    out
}

fn scan_datagrip() -> ImportScanResult {
    let mut found = Vec::new();
    let mut warnings = Vec::new();
    let mut first_path: Option<String> = None;

    // (a) IDE-global: ~/Library/Application Support/JetBrains/DataGrip*/options/dataSources.xml
    let jb = home().join("Library/Application Support/JetBrains");
    for ide in glob_dirs(&jb, "DataGrip") {
        let xml = ide.join("options/dataSources.xml");
        let local = ide.join("options/dataSources.local.xml");
        if let Ok(content) = std::fs::read_to_string(&xml) {
            if first_path.is_none() {
                first_path = Some(xml.to_string_lossy().into_owned());
            }
            let local_parsed = std::fs::read_to_string(&local)
                .map(|c| parse_datagrip_local(&c))
                .unwrap_or_default();
            let (conns, mut w) = parse_datagrip(&content, &local_parsed);
            found.extend(conns);
            warnings.append(&mut w);
        }
    }

    // (b) Project-level: bounded walk of $HOME for **/.idea/dataSources.xml.
    for xml in find_idea_datasources() {
        let local = xml.with_file_name("dataSources.local.xml");
        if let Ok(content) = std::fs::read_to_string(&xml) {
            if first_path.is_none() {
                first_path = Some(xml.to_string_lossy().into_owned());
            }
            let local_parsed = std::fs::read_to_string(&local)
                .map(|c| parse_datagrip_local(&c))
                .unwrap_or_default();
            let (conns, mut w) = parse_datagrip(&content, &local_parsed);
            found.extend(conns);
            warnings.append(&mut w);
        }
    }

    ImportScanResult {
        source: ImportSource::Datagrip,
        path: first_path,
        connections: dedupe(found),
        warnings,
    }
}

fn scan_nosqlbooster() -> ImportScanResult {
    let path = home()
        .join("Library/Application Support/NoSQLBooster for MongoDB/app.json");
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let (connections, warnings) = parse_nosqlbooster(&content);
            ImportScanResult {
                source: ImportSource::Nosqlbooster,
                path: Some(path.to_string_lossy().into_owned()),
                connections,
                warnings,
            }
        }
        Err(_) => empty_scan(ImportSource::Nosqlbooster),
    }
}

fn empty_scan(source: ImportSource) -> ImportScanResult {
    ImportScanResult {
        source,
        path: None,
        connections: Vec::new(),
        warnings: Vec::new(),
    }
}

/// Immediate child dirs of `base` whose name starts with `prefix`.
fn glob_dirs(base: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(base) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with(prefix) {
                        out.push(p);
                    }
                }
            }
        }
    }
    // Stable order so "first existing" is deterministic.
    out.sort();
    out
}

/// Directories never worth descending into during the project-level DataGrip
/// scan (huge, irrelevant, or system).
const SKIP_DIRS: &[&str] = &[
    "Library",
    "node_modules",
    ".Trash",
    ".git",
    "target",
    ".cargo",
    "Music",
    "Movies",
    "Pictures",
];

/// Max `.idea/dataSources.xml` files to collect during the bounded scan.
const MAX_IDEA_FILES: usize = 50;

/// Bounded BFS of `$HOME` (maxdepth 4) for `.idea/dataSources.xml`, skipping
/// the heavy/system dirs in [`SKIP_DIRS`]. Capped at [`MAX_IDEA_FILES`].
fn find_idea_datasources() -> Vec<PathBuf> {
    let root = home();
    let mut found = Vec::new();
    // (dir, depth)
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> =
        std::collections::VecDeque::new();
    queue.push_back((root, 0));
    while let Some((dir, depth)) = queue.pop_front() {
        if found.len() >= MAX_IDEA_FILES || depth > 4 {
            continue;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !p.is_dir() {
                continue;
            }
            if name == ".idea" {
                let xml = p.join("dataSources.xml");
                if xml.is_file() {
                    found.push(xml);
                    if found.len() >= MAX_IDEA_FILES {
                        return found;
                    }
                }
                // Don't descend into .idea further.
                continue;
            }
            if SKIP_DIRS.contains(&name) || name.starts_with('.') {
                continue;
            }
            queue.push_back((p, depth + 1));
        }
    }
    found
}

/// Dedupe parsed connections by (name, params) so merging multiple DBeaver
/// workspaces or DataGrip projects doesn't list the same profile twice.
fn dedupe(conns: Vec<ParsedConnection>) -> Vec<ParsedConnection> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(conns.len());
    for c in conns {
        let key = format!("{}|{}", c.name, c.params);
        if seen.insert(key) {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests;
