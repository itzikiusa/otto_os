//! ClickHouse driver — two transports behind one driver.
//!
//! - **HTTP interface** (8123/8443) via `reqwest` + `FORMAT JSONCompact`: the
//!   default, used for the managed/cloud `https://…:8443` case. Raw HTTP fits
//!   arbitrary SQL better than the `clickhouse` crate's compile-time Row API.
//! - **Native TCP protocol** (9000 plain / 9440 TLS) via `klickhouse`: the
//!   wire protocol the official `clickhouse-client` and most managed setups use.
//!   The HTTP interface's TLS handshake times out for some Yandex Managed CH +
//!   SSH-tunnel setups; the native protocol is what their working tools speak,
//!   so we support it directly.
//!
//! Transport is chosen by port (9000/9440 → native; 9440 → native TLS) or an
//! explicit `transport: "native"` param. Both transports normalize to the same
//! [`RawRows`] (column name+CH-type + JSON rows), so the introspection SQL
//! (`system.databases`/`tables`/`columns`, `SHOW CREATE`), row shaping, cell
//! truncation, and completion logic are shared between them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use otto_core::Result;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::driver::Driver;
use crate::tls::TlsFiles;
use crate::types::{
    self, Capabilities, Column, ColumnDef, CompletionContext, CompletionItem, CompletionKind,
    CompletionResponse, Engine, NodeKind, NodePath, ObjectDetail, QueryRequest, QueryResult,
    QueryStats, ResolvedConfig, SchemaNode, TestResult,
};

/// ClickHouse driver. Caches one transport handle per [`ResolvedConfig::cache_key`]:
///
/// - HTTP: a `reqwest::Client` (cheap to clone; carries its own keep-alive
///   connection pool, so reuse avoids re-establishing TCP/TLS each call).
/// - Native: an `Arc<klickhouse::Client>` (a long-lived multiplexed connection;
///   reusing it across calls avoids re-handshaking the native protocol + TLS).
///
/// Both caches are `Mutex<HashMap>` and so `Default`-constructible, keeping the
/// `#[derive(Default)]` the registry relies on.
#[derive(Default)]
pub struct ClickhouseDriver {
    clients: Mutex<HashMap<String, reqwest::Client>>,
    native: Mutex<HashMap<String, Arc<klickhouse::Client>>>,
}

// --- Transport selection ----------------------------------------------------

/// The two wire transports. Selected from the resolved config: native when the
/// port is the native protocol port (9000 plain / 9440 TLS) or the profile
/// asks for it via `params.transport == "native"`; HTTP otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Http,
    Native,
}

fn transport_for(cfg: &ResolvedConfig) -> Transport {
    match cfg.param_str("transport") {
        Some(t) if t.eq_ignore_ascii_case("native") => return Transport::Native,
        Some(t) if t.eq_ignore_ascii_case("http") => return Transport::Http,
        _ => {}
    }
    // Decide by the ORIGINAL port: when tunneled, the service rewrites cfg.port
    // to the ephemeral local forward port and stashes the real port in
    // `__tunnel_port` (a JSON number), so a 9440 connection over a tunnel still
    // selects native.
    let port = cfg
        .params
        .get("__tunnel_port")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u16)
        .unwrap_or(cfg.port);
    if is_native_port(port) {
        Transport::Native
    } else {
        Transport::Http
    }
}

/// Is this a ClickHouse native-protocol port? The canonical ports are 9000
/// (plain) and 9440 (TLS). We also recognize a forwarded native port — an SSH
/// tunnel's local forward is often the upstream port with a leading digit
/// (e.g. `19000` → `9000`, `19440` → `9440`) — by matching the trailing
/// `9000`/`9440` while excluding the HTTP ports. (An explicit
/// `transport: "native"` param always wins, for any other forward port.)
fn is_native_port(port: u16) -> bool {
    matches!(port, 9000 | 9440) || matches!(port % 10000, 9000 | 9440)
}

/// A transport-agnostic rowset: column (name, CH type) pairs and JSON rows. Both
/// the HTTP `JSONCompact` reply and a decoded native `Block` normalize to this,
/// so all higher-level logic (introspection, run, completion) is shared.
#[derive(Debug, Default)]
struct RawRows {
    meta: Vec<(String, String)>,
    data: Vec<Vec<Value>>,
    bytes_read: u64,
}

impl RawRows {
    /// Column names only — for code paths that read positional cells.
    fn first_col_strs(&self) -> impl Iterator<Item = &str> {
        self.data
            .iter()
            .filter_map(|row| row.first().and_then(Value::as_str))
    }
}

// --- HTTP plumbing ----------------------------------------------------------

/// Everything needed to issue a request: the built client, the base URL
/// (`http(s)://host:port/`), and the auth headers.
struct Conn {
    client: reqwest::Client,
    base: String,
    headers: HeaderMap,
    /// Session time zone (default UTC), sent as the `session_timezone` setting
    /// so DateTime values render in the user's configured zone.
    timezone: Option<String>,
    /// Active database (if the user selected one), sent as the `database`
    /// request param so unqualified table names resolve against it.
    database: Option<String>,
}

/// The shape of a `FORMAT JSONCompact` reply.
#[derive(Debug, Deserialize)]
struct JsonResponse {
    #[serde(default)]
    meta: Vec<MetaCol>,
    #[serde(default)]
    data: Vec<Vec<Value>>,
    #[serde(default)]
    statistics: Statistics,
}

#[derive(Debug, Deserialize)]
struct MetaCol {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

#[derive(Debug, Default, Deserialize)]
struct Statistics {
    #[serde(default)]
    bytes_read: u64,
}

impl JsonResponse {
    /// Normalize an HTTP `JSONCompact` reply into a transport-agnostic [`RawRows`].
    fn into_raw(self) -> RawRows {
        RawRows {
            meta: self.meta.into_iter().map(|m| (m.name, m.ty)).collect(),
            data: self.data,
            bytes_read: self.statistics.bytes_read,
        }
    }
}

impl ClickhouseDriver {
    /// Get (or lazily build + cache) the `reqwest::Client` for `cfg`, keyed by
    /// [`ResolvedConfig::cache_key`]. Holding the tokio mutex across the
    /// (synchronous) build is fine — it only briefly serializes concurrent
    /// *first* builds for the same key; cache hits return immediately. A
    /// `reqwest::Client` clone is cheap (shares the inner connection pool).
    async fn client(&self, cfg: &ResolvedConfig) -> Result<reqwest::Client> {
        let cache_key = cfg.cache_key();
        let mut cache = self.clients.lock().await;
        if let Some(client) = cache.get(&cache_key) {
            return Ok(client.clone());
        }
        let client = build_client(cfg)?;
        cache.insert(cache_key, client.clone());
        Ok(client)
    }

    /// Build a [`Conn`] for one operation: the cached `reqwest::Client` plus the
    /// base URL, auth headers, session timezone, and optional active database
    /// (all derived cheaply from `cfg`).
    async fn connect(&self, cfg: &ResolvedConfig, active_db: Option<&str>) -> Result<Conn> {
        let client = self.client(cfg).await?;

        let scheme = if cfg.tls.enabled() { "https" } else { "http" };
        // Through an SSH tunnel the service rewrites host→127.0.0.1; use the
        // ORIGINAL hostname in the URL so the TLS SNI + Host header are the real
        // host (managed ClickHouse routes by SNI). `build_client` maps that host
        // back to the local tunnel port via reqwest `.resolve`.
        let url_host = cfg.param_str("__tunnel_host").unwrap_or_else(|| cfg.host.clone());
        let base = format!("{scheme}://{url_host}:{}/", cfg.port);

        let user = cfg.user.clone().unwrap_or_else(|| "default".to_string());
        let key = cfg.password.clone().unwrap_or_default();
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-ClickHouse-User",
            HeaderValue::from_str(&user).map_err(types::upstream)?,
        );
        headers.insert(
            "X-ClickHouse-Key",
            HeaderValue::from_str(&key).map_err(types::upstream)?,
        );

        // Default to UTC when unset (user request: "by default, UTC").
        let timezone = Some(
            cfg.param_str("timezone")
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "UTC".to_string()),
        );

        Ok(Conn {
            client,
            base,
            headers,
            timezone,
            database: active_db.map(str::to_string),
        })
    }

    // --- Native (klickhouse) plumbing ---------------------------------------

    /// Get (or lazily open + cache) the native `klickhouse::Client` for `cfg`,
    /// keyed by [`ResolvedConfig::cache_key`]. A `klickhouse::Client` is a
    /// long-lived multiplexed connection, so we keep it behind an `Arc` and
    /// reuse it. A closed (dropped/broken) connection is transparently
    /// reopened: if the cached client reports closed, we discard and rebuild.
    async fn native_client(&self, cfg: &ResolvedConfig) -> Result<Arc<klickhouse::Client>> {
        let cache_key = cfg.cache_key();
        let mut cache = self.native.lock().await;
        if let Some(client) = cache.get(&cache_key) {
            if !client.is_closed() {
                return Ok(client.clone());
            }
            // Stale handle — drop it and open a fresh connection below.
            cache.remove(&cache_key);
        }
        let client = Arc::new(native_connect(cfg).await?);
        cache.insert(cache_key, client.clone());
        Ok(client)
    }

    /// Run one SQL statement over the native protocol and normalize the decoded
    /// blocks into a transport-agnostic [`RawRows`]. This is the native twin of
    /// [`Conn::query_json`]: connect (cached, TLS per cfg), stream result
    /// `Block`s, and decode each column's `Value`s to JSON dynamically.
    async fn native_query(&self, cfg: &ResolvedConfig, sql: &str) -> Result<RawRows> {
        use futures_util::StreamExt;

        let client = self.native_client(cfg).await?;
        let mut stream = client.query_raw(sql).await.map_err(native_err)?;

        let mut meta: Vec<(String, String)> = Vec::new();
        let mut data: Vec<Vec<Value>> = Vec::new();
        while let Some(block) = stream.next().await {
            let block = block.map_err(native_err)?;
            // The first non-empty block establishes the column order/types; the
            // header block (rows == 0) still carries column_types, so we capture
            // metadata from whichever block first exposes columns.
            if meta.is_empty() && !block.column_types.is_empty() {
                meta = block
                    .column_types
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.to_string()))
                    .collect();
            }
            if block.rows == 0 {
                continue;
            }
            // Decode column-major `column_data` into row-major JSON. Columns are
            // ordered by `column_types` (an IndexMap preserves SELECT order).
            let order: Vec<&String> = block.column_types.keys().collect();
            for r in 0..block.rows as usize {
                let mut row = Vec::with_capacity(order.len());
                for name in &order {
                    let cell = block
                        .column_data
                        .get(*name)
                        .and_then(|col| col.get(r))
                        .map(value_to_json)
                        .unwrap_or(Value::Null);
                    row.push(cell);
                }
                data.push(row);
            }
        }

        Ok(RawRows {
            meta,
            data,
            bytes_read: 0,
        })
    }

    /// Run a row-returning statement over whichever transport `cfg` selects.
    /// Introspection/completion callers use this no-scope wrapper (they qualify
    /// their own table names); `run` uses [`Self::query_rows_db`] to scope to the
    /// active database.
    async fn query_rows(&self, cfg: &ResolvedConfig, sql: &str) -> Result<RawRows> {
        self.query_rows_db(cfg, sql, None).await
    }

    /// Like [`Self::query_rows`] but scopes unqualified table names to
    /// `active_db` (when `Some`). On the HTTP transport this sets the `database`
    /// request param. NATIVE TRANSPORT TODO: the native client's default
    /// database is fixed at connect time (cached per config), so active-db
    /// scoping is not applied there yet — unqualified names resolve against the
    /// profile's database. Most connections use HTTP.
    async fn query_rows_db(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
    ) -> Result<RawRows> {
        match transport_for(cfg) {
            Transport::Http => {
                let conn = self.connect(cfg, active_db).await?;
                Ok(conn.query_json(sql).await?.into_raw())
            }
            Transport::Native => self.native_query(cfg, sql).await,
        }
    }

    /// Run a statement whose reply we want as raw text (DDL / `SHOW CREATE`).
    /// Over native there is no "raw text" format — we run it as a normal query
    /// and join the single string column the server returns. No-scope wrapper;
    /// `run` uses [`Self::query_text_db`].
    async fn query_text(&self, cfg: &ResolvedConfig, sql: &str) -> Result<String> {
        self.query_text_db(cfg, sql, None).await
    }

    /// Like [`Self::query_text`] but scopes to `active_db` on the HTTP transport
    /// (see [`Self::query_rows_db`] for the native-transport TODO).
    async fn query_text_db(
        &self,
        cfg: &ResolvedConfig,
        sql: &str,
        active_db: Option<&str>,
    ) -> Result<String> {
        match transport_for(cfg) {
            Transport::Http => {
                let conn = self.connect(cfg, active_db).await?;
                conn.query_raw(sql).await
            }
            Transport::Native => {
                let raw = self.native_query(cfg, sql).await?;
                // SHOW CREATE / single-value replies come back as one row, one
                // string cell; stringify whatever the first cell is.
                let text = raw
                    .data
                    .first()
                    .and_then(|row| row.first())
                    .map(json_cell_to_text)
                    .unwrap_or_default();
                Ok(text)
            }
        }
    }
}

/// Build a fresh `reqwest::Client` from the resolved config, materializing any
/// inline CA for custom-CA TLS. Never called directly by the driver methods —
/// they go through [`ClickhouseDriver::client`] for caching.
fn build_client(cfg: &ResolvedConfig) -> Result<reqwest::Client> {
    // Bound both connection establishment (TCP+TLS) and the overall request so a
    // misconfigured endpoint (e.g. HTTPS against a plain-HTTP port, or the native
    // 9000/9440 port) fails fast with a clear error instead of hanging forever.
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60));
    // Tunnel case: the URL host is the real hostname (for SNI/cert), but the TCP
    // must go to the local forward — map it there. (reqwest uses the URL port
    // with the overridden IP, so the forward's local port is honoured.)
    if let Some(tunnel_host) = cfg.param_str("__tunnel_host") {
        let local = std::net::SocketAddr::from(([127, 0, 0, 1], cfg.port));
        builder = builder.resolve(&tunnel_host, local);
    }
    if cfg.tls.enabled() {
        let files = TlsFiles::materialize(&cfg.tls)?;
        if let Some(ca_path) = files.ca {
            let ca_bytes = std::fs::read(&ca_path).map_err(types::upstream)?;
            let cert = reqwest::Certificate::from_pem(&ca_bytes).map_err(types::upstream)?;
            builder = builder.add_root_certificate(cert);
        }
        if !cfg.tls.verify {
            builder = builder.danger_accept_invalid_certs(true);
        }
    }
    builder.build().map_err(types::upstream)
}

impl Conn {
    /// Run a statement that returns rows, asking for `FORMAT JSONCompact`, and
    /// parse the reply.
    async fn query_json(&self, sql: &str) -> Result<JsonResponse> {
        let body = format!("{sql}\nFORMAT JSONCompact");
        let text = self.post(body).await?;
        serde_json::from_str(&text).map_err(types::upstream)
    }

    /// Run a statement and return the raw response text (for DDL / SHOW CREATE
    /// / write statements with no JSON envelope).
    async fn query_raw(&self, sql: &str) -> Result<String> {
        self.post(sql.to_string()).await
    }

    /// POST a body to the HTTP interface; map non-2xx replies to a 502 carrying
    /// the server's error text.
    async fn post(&self, body: String) -> Result<String> {
        let mut req = self.client.post(&self.base).headers(self.headers.clone());
        if let Some(tz) = &self.timezone {
            // ClickHouse 23.4+ honours session_timezone as a request setting.
            req = req.query(&[("session_timezone", tz.as_str())]);
        }
        if let Some(db) = &self.database {
            // The `database` request param sets the default DB for unqualified
            // table names (the HTTP analogue of `USE <db>`).
            req = req.query(&[("database", db.as_str())]);
        }
        let resp = req.body(body).send().await.map_err(req_err)?;
        let status = resp.status();
        let text = resp.text().await.map_err(types::upstream)?;
        if !status.is_success() {
            return Err(types::upstream(text.trim().to_string()));
        }
        Ok(text)
    }
}

// --- Native connection / TLS ------------------------------------------------

/// Open a native-protocol connection to the resolved endpoint. Native TLS (port
/// 9440 or `cfg.tls` enabled) builds a rustls connector honouring `cfg.tls`
/// (custom inline CA, and `verify=false` → accept any cert); the SNI is set to
/// the real hostname (`__tunnel_host` when tunnelled, else `cfg.host`).
async fn native_connect(cfg: &ResolvedConfig) -> Result<klickhouse::Client> {
    let options = klickhouse::ClientOptions {
        username: cfg.user.clone().unwrap_or_else(|| "default".to_string()),
        password: cfg.password.clone().unwrap_or_default(),
        default_database: cfg.database.clone().unwrap_or_default(),
        tcp_nodelay: true,
    };
    // 9440 (or a forwarded *9440) is the conventional native-TLS port; also
    // honour an explicit TLS mode in the profile.
    let use_tls = cfg.tls.enabled() || cfg.port % 10000 == 9440;
    // The TCP endpoint is always the resolved (post-tunnel) host:port — for a
    // tunnel that's 127.0.0.1:<local-forward>; otherwise the real host.
    let endpoint = format!("{}:{}", cfg.host, cfg.port);

    if !use_tls {
        return klickhouse::Client::connect(&endpoint, options)
            .await
            .map_err(native_err);
    }

    // Native TLS: SNI = the real hostname. Through an SSH tunnel the TCP target
    // is 127.0.0.1:<local>, but the certificate is issued for the real host, so
    // we set the server name from `__tunnel_host` when present (managed CH also
    // routes by SNI). LIMITATION: if the tunnel host is unknown we fall back to
    // `cfg.host` (127.0.0.1 for a tunnel), which won't match a real cert —
    // such setups must either provide `__tunnel_host`, set `tls.server_name`,
    // or disable verification (`tls.verify = false`).
    let connector = build_native_tls(cfg)?;
    let sni = cfg
        .tls
        .server_name
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.param_str("__tunnel_host"))
        .unwrap_or_else(|| cfg.host.clone());
    let server_name = rustls_pki_types::ServerName::try_from(sni)
        .map_err(|e| types::invalid(format!("clickhouse: invalid TLS server name: {e}")))?;

    klickhouse::Client::connect_tls(&endpoint, options, server_name, &connector)
        .await
        .map_err(native_err)
}

/// Build a `tokio_rustls::TlsConnector` for the native transport from `cfg.tls`:
/// honour an inline CA (added to the root store), and when `verify=false`
/// install a no-op certificate verifier (signatures are still checked, the
/// chain/hostname are not) so self-signed / mismatched certs are accepted.
fn build_native_tls(cfg: &ResolvedConfig) -> Result<tokio_rustls::TlsConnector> {
    use rustls::pki_types::pem::PemObject;

    let mut roots = rustls::RootCertStore::empty();
    // Start from the bundled webpki trust anchors (system-equivalent roots).
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // Add any inline CA PEM (private CA / managed-CH bundle).
    if let Some(ca) = cfg.tls.ca_cert.as_deref().filter(|s| !s.is_empty()) {
        for cert in rustls::pki_types::CertificateDer::pem_slice_iter(ca.as_bytes()) {
            let cert = cert.map_err(|e| types::invalid(format!("clickhouse: bad CA PEM: {e}")))?;
            roots
                .add(cert)
                .map_err(|e| types::invalid(format!("clickhouse: bad CA cert: {e}")))?;
        }
    }

    let config = if cfg.tls.verify {
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()
    } else {
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier::new()))
            .with_no_client_auth()
    };

    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

/// A `ServerCertVerifier` that accepts any server certificate (used only when
/// `cfg.tls.verify == false`, the native equivalent of reqwest's
/// `danger_accept_invalid_certs`). Signature verification is still delegated to
/// the crypto provider so the handshake stays well-formed; only chain validity
/// and hostname matching are skipped.
#[derive(Debug)]
struct NoVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl NoVerifier {
    fn new() -> Self {
        Self {
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Map a `klickhouse` error to an Upstream error (a 502 — the database's fault).
fn native_err(e: klickhouse::KlickhouseError) -> otto_core::Error {
    otto_core::Error::Upstream(e.to_string())
}

// --- Native value decoding --------------------------------------------------

/// Convert a decoded native `klickhouse::Value` into a clean JSON value for the
/// grid. The mapping mirrors the HTTP `JSONCompact` shapes: ints/floats →
/// number, String/FixedString → string, Date/DateTime → ISO string, Nullable →
/// null/inner, Array/Tuple/Map → nested JSON. Exotic types fall back to the
/// value's own string form (`Display`), so nothing ever breaks the grid.
fn value_to_json(v: &klickhouse::Value) -> Value {
    use klickhouse::Value as V;
    match v {
        V::Null => Value::Null,

        // Signed ints: keep small ones as JSON numbers; 128/256-bit can exceed
        // f64/i64 precision, so render them as strings to avoid silent loss.
        V::Int8(x) => json_num_i(*x as i64),
        V::Int16(x) => json_num_i(*x as i64),
        V::Int32(x) => json_num_i(*x as i64),
        V::Int64(x) => json_num_i(*x),
        V::Int128(x) => Value::String(x.to_string()),
        V::Int256(_) => Value::String(v.to_string()),

        // Unsigned ints.
        V::UInt8(x) => json_num_u(*x as u64),
        V::UInt16(x) => json_num_u(*x as u64),
        V::UInt32(x) => json_num_u(*x as u64),
        V::UInt64(x) => json_num_u(*x),
        V::UInt128(x) => Value::String(x.to_string()),
        V::UInt256(_) => Value::String(v.to_string()),

        // Floats.
        V::Float32(x) => json_num_f(*x as f64),
        V::Float64(x) => json_num_f(*x),
        V::BFloat16(x) => json_num_f(f32::from(*x) as f64),

        // Decimals render with their fractional point via Display (no f64 loss).
        V::Decimal32(..) | V::Decimal64(..) | V::Decimal128(..) | V::Decimal256(..) => {
            Value::String(v.to_string())
        }

        // String/FixedString arrive as raw bytes; decode lossily to text.
        V::String(bytes) => Value::String(String::from_utf8_lossy(bytes).into_owned()),

        V::Uuid(u) => Value::String(u.to_string()),

        // Dates/times → ISO-8601 strings.
        V::Date(d) => {
            let date: chrono::NaiveDate = (*d).into();
            Value::String(date.to_string())
        }
        V::DateTime(dt) => match chrono::DateTime::<klickhouse::Tz>::try_from(*dt) {
            Ok(t) => Value::String(t.to_rfc3339()),
            Err(_) => Value::String(v.to_string()),
        },
        V::DateTime64(dt) => match chrono::DateTime::<chrono::Utc>::try_from(*dt) {
            Ok(t) => Value::String(t.to_rfc3339()),
            Err(_) => Value::String(v.to_string()),
        },

        // Enums serialize as their backing integer (matches CH JSON for raw enums).
        V::Enum8(x) => json_num_i(*x as i64),
        V::Enum16(x) => json_num_i(*x as i64),

        // Nested containers → nested JSON.
        V::Array(items) | V::Tuple(items) => {
            Value::Array(items.iter().map(value_to_json).collect())
        }
        V::Map(keys, vals) => {
            // Best-effort: a JSON object when keys stringify, else an array of
            // [key, value] pairs (covers non-string keys).
            let mut obj = serde_json::Map::new();
            let mut all_str = true;
            for (k, val) in keys.iter().zip(vals.iter()) {
                match map_key_to_string(k) {
                    Some(ks) => {
                        obj.insert(ks, value_to_json(val));
                    }
                    None => {
                        all_str = false;
                        break;
                    }
                }
            }
            if all_str {
                Value::Object(obj)
            } else {
                Value::Array(
                    keys.iter()
                        .zip(vals.iter())
                        .map(|(k, val)| {
                            Value::Array(vec![value_to_json(k), value_to_json(val)])
                        })
                        .collect(),
                )
            }
        }

        // IPs and geo types: their string form is the natural representation.
        V::Ipv4(_) | V::Ipv6(_) => Value::String(strip_quotes(v.to_string())),
        V::Point(_) | V::Ring(_) | V::Polygon(_) | V::MultiPolygon(_) => {
            Value::String(v.to_string())
        }
    }
}

/// A JSON number from an i64.
fn json_num_i(x: i64) -> Value {
    Value::Number(x.into())
}

/// A JSON number from a u64.
fn json_num_u(x: u64) -> Value {
    Value::Number(x.into())
}

/// A JSON number from an f64; non-finite floats degrade to their string form
/// (JSON has no NaN/Infinity).
fn json_num_f(x: f64) -> Value {
    serde_json::Number::from_f64(x)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(x.to_string()))
}

/// Stringify a map key value for use as a JSON object key (strings & numbers).
fn map_key_to_string(v: &klickhouse::Value) -> Option<String> {
    use klickhouse::Value as V;
    match v {
        V::String(b) => Some(String::from_utf8_lossy(b).into_owned()),
        V::Int8(_) | V::Int16(_) | V::Int32(_) | V::Int64(_) | V::UInt8(_) | V::UInt16(_)
        | V::UInt32(_) | V::UInt64(_) => Some(v.to_string()),
        _ => None,
    }
}

/// The native `Value::Display` wraps IPs in single quotes (`'1.2.3.4'`); strip a
/// single layer of surrounding quotes for a clean cell.
fn strip_quotes(s: String) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        t[1..t.len() - 1].to_string()
    } else {
        s
    }
}

/// Render a JSON cell as plain text for raw-text replies (SHOW CREATE etc.):
/// unwrap a JSON string, leave other scalars as their JSON text.
fn json_cell_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// --- Shared helpers ---------------------------------------------------------

/// Map a reqwest transport error into an Upstream error, unwinding its source
/// chain so the *real* cause (TLS handshake failure, connection refused, …) is
/// visible instead of the terse "error sending request for url (…)". Critical
/// for diagnosing TLS-through-an-SSH-tunnel issues (cert host mismatch, or a
/// plain-HTTP port reached over HTTPS).
fn req_err(e: reqwest::Error) -> otto_core::Error {
    use std::error::Error as _;
    let mut msg = e.to_string();
    let mut src: Option<&dyn std::error::Error> = e.source();
    while let Some(s) = src {
        let part = s.to_string();
        if !msg.contains(&part) {
            msg.push_str(": ");
            msg.push_str(&part);
        }
        src = s.source();
    }
    otto_core::Error::Upstream(msg)
}

/// Escape single quotes for embedding an identifier inside a SQL string literal.
fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// Escape backticks for a backtick-quoted identifier (`db`.`tbl`).
fn esc_ident(s: &str) -> String {
    s.replace('`', "``")
}

/// Does this statement return a rowset? (first keyword decides).
fn returns_rows(sql: &str) -> bool {
    let first = sql
        .trim_start()
        .split(|c: char| c.is_whitespace() || c == '(')
        .find(|w| !w.is_empty())
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        first.as_str(),
        "SELECT" | "SHOW" | "DESC" | "DESCRIBE" | "EXPLAIN" | "WITH"
    )
}

/// System databases that exist on every server; keep them but sort last.
fn is_system_db(name: &str) -> bool {
    matches!(name, "system" | "INFORMATION_SCHEMA" | "information_schema")
}

/// Max characters kept for a single result cell. Set high (~1 MiB) so normal
/// text/DDL/JSON cells pass through untouched — full values reach the grid and
/// Copy/CSV/JSON exports. The cap only exists as a safety valve against a single
/// pathological multi-MB binary blob (e.g. a ClickHouse `AggregateFunction` /
/// `*State` column from AggregatingMergeTree) that could choke the grid or OOM
/// us; such a cell is truncated with a marker while the query still succeeds.
const MAX_CELL_CHARS: usize = 1_048_576;

/// Recursively cap oversized string values in a result cell (covers nested
/// Array/Tuple/Map columns too). Non-strings pass through unchanged.
fn cap_cell(v: Value) -> Value {
    match v {
        Value::String(s) => {
            let len = s.chars().count();
            if len > MAX_CELL_CHARS {
                let kept: String = s.chars().take(MAX_CELL_CHARS).collect();
                Value::String(format!("{kept}…[truncated {} chars]", len - MAX_CELL_CHARS))
            } else {
                Value::String(s)
            }
        }
        Value::Array(a) => Value::Array(a.into_iter().map(cap_cell).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, val)| (k, cap_cell(val))).collect()),
        other => other,
    }
}

#[async_trait]
impl Driver for ClickhouseDriver {
    fn engine(&self) -> Engine {
        Engine::Clickhouse
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            engine: Engine::Clickhouse,
            sql: true,
            joins: true,
            transactions: false,
            multi_statement: true,
            default_port: 8123,
            schema_levels: vec!["Database".into(), "Table".into(), "Column".into()],
            query_language: "sql".into(),
        }
    }

    async fn test(&self, cfg: &ResolvedConfig) -> Result<TestResult> {
        let started = Instant::now();
        match self.query_rows(cfg, "SELECT version()").await {
            Ok(resp) => {
                let latency = started.elapsed().as_millis() as u64;
                let version = resp
                    .data
                    .first()
                    .and_then(|row| row.first())
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Ok(TestResult {
                    ok: true,
                    latency_ms: Some(latency),
                    message: "connected".to_string(),
                    server_version: version,
                })
            }
            Err(e) => Ok(TestResult {
                ok: false,
                latency_ms: None,
                message: e.to_string(),
                server_version: None,
            }),
        }
    }

    async fn schema_root(&self, cfg: &ResolvedConfig) -> Result<Vec<SchemaNode>> {
        let resp = self
            .query_rows(cfg, "SELECT name FROM system.databases ORDER BY name")
            .await?;
        let mut nodes: Vec<(bool, SchemaNode)> = resp
            .first_col_strs()
            .map(|name| {
                let node = SchemaNode::new(format!("db:{name}"), name, NodeKind::Database).expandable();
                (is_system_db(name), node)
            })
            .collect();
        // User databases first, system databases last; preserve name order within.
        nodes.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(nodes.into_iter().map(|(_, n)| n).collect())
    }

    async fn schema_children(
        &self,
        cfg: &ResolvedConfig,
        parent: &NodePath,
        _filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let db = parent
            .get("db")
            .ok_or_else(|| types::invalid("clickhouse: missing database in node path"))?;

        if let Some(table) = parent.get("table") {
            // Columns of a table/view.
            let sql = format!(
                "SELECT name, type FROM system.columns \
                 WHERE database = '{}' AND table = '{}' ORDER BY position",
                esc(db),
                esc(table)
            );
            let resp = self.query_rows(cfg, &sql).await?;
            let base = parent.to_id();
            Ok(resp
                .data
                .iter()
                .filter_map(|row| {
                    let name = row.first().and_then(Value::as_str)?;
                    let ty = row.get(1).and_then(Value::as_str).unwrap_or("");
                    Some(
                        SchemaNode::new(format!("{base}/column:{name}"), name, NodeKind::Column)
                            .with_detail(ty),
                    )
                })
                .collect())
        } else {
            // Tables + views of a database.
            let sql = format!(
                "SELECT name, engine FROM system.tables WHERE database = '{}' ORDER BY name",
                esc(db)
            );
            let resp = self.query_rows(cfg, &sql).await?;
            Ok(resp
                .data
                .iter()
                .filter_map(|row| {
                    let name = row.first().and_then(Value::as_str)?;
                    let engine = row.get(1).and_then(Value::as_str).unwrap_or("");
                    let kind = if engine.ends_with("View") {
                        NodeKind::View
                    } else {
                        NodeKind::Table
                    };
                    // Engine is shown as a secondary, space-permitting detail
                    // (the tree layout gives the table name priority and lets
                    // the engine truncate first; full engine is on hover).
                    Some(
                        SchemaNode::new(format!("db:{db}/table:{name}"), name, kind)
                            .with_detail(engine)
                            .expandable(),
                    )
                })
                .collect())
        }
    }

    async fn object_detail(&self, cfg: &ResolvedConfig, path: &NodePath) -> Result<ObjectDetail> {
        let db = path
            .get("db")
            .ok_or_else(|| types::invalid("clickhouse: missing database in node path"))?;
        let table = path
            .get("table")
            .ok_or_else(|| types::invalid("clickhouse: missing table in node path"))?;

        // Columns.
        let col_sql = format!(
            "SELECT name, type, default_kind, default_expression, comment, is_in_primary_key \
             FROM system.columns WHERE database = '{}' AND table = '{}' ORDER BY position",
            esc(db),
            esc(table)
        );
        let cols = self.query_rows(cfg, &col_sql).await?;
        let mut columns = Vec::new();
        let mut primary_key = Vec::new();
        for row in &cols.data {
            let name = row.first().and_then(Value::as_str).unwrap_or("").to_string();
            let data_type = row.get(1).and_then(Value::as_str).unwrap_or("").to_string();
            let default_kind = row.get(2).and_then(Value::as_str).unwrap_or("");
            let default_expr = row.get(3).and_then(Value::as_str).unwrap_or("");
            let comment = row.get(4).and_then(Value::as_str).unwrap_or("");
            // is_in_primary_key arrives as a UInt8 number (0/1).
            let in_pk = row
                .get(5)
                .map(|v| v.as_u64() == Some(1) || v.as_str() == Some("1"))
                .unwrap_or(false);

            let default = if !default_expr.is_empty() {
                Some(if default_kind.is_empty() {
                    default_expr.to_string()
                } else {
                    format!("{default_kind} {default_expr}")
                })
            } else {
                None
            };

            if in_pk {
                primary_key.push(name.clone());
            }
            // ClickHouse has no SQL NULL semantics unless the type is Nullable(..).
            let nullable = data_type.starts_with("Nullable(");
            columns.push(ColumnDef {
                name,
                data_type,
                nullable,
                default,
                key: in_pk.then(|| "PRI".to_string()),
                extra: None,
                comment: (!comment.is_empty()).then(|| comment.to_string()),
            });
        }

        // Table-level metadata.
        let tbl_sql = format!(
            "SELECT engine, partition_key, sorting_key, primary_key, total_rows \
             FROM system.tables WHERE database = '{}' AND name = '{}'",
            esc(db),
            esc(table)
        );
        let tbl = self.query_rows(cfg, &tbl_sql).await?;
        let row = tbl.data.first();
        let engine = row
            .and_then(|r| r.first())
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let partition_key = row
            .and_then(|r| r.get(1))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let sorting_key = row
            .and_then(|r| r.get(2))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let pk_expr = row
            .and_then(|r| r.get(3))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let row_count = row
            .and_then(|r| r.get(4))
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())));

        let kind = if engine.ends_with("View") {
            NodeKind::View
        } else {
            NodeKind::Table
        };

        // DDL via SHOW CREATE (raw text, tab-separated raw so we get it verbatim
        // over HTTP; over native it comes back as a single string cell).
        let ddl_sql = format!(
            "SHOW CREATE TABLE `{}`.`{}` FORMAT TabSeparatedRaw",
            esc_ident(db),
            esc_ident(table)
        );
        let ddl = self
            .query_text(cfg, &ddl_sql)
            .await
            .ok()
            .map(|s| s.trim_end().to_string())
            .filter(|s| !s.is_empty());

        let mut detail = ObjectDetail::new(table, kind);
        detail.columns = columns;
        detail.primary_key = primary_key;
        detail.row_count = row_count;
        detail.ddl = ddl;
        detail.extra = serde_json::json!({
            "engine": engine,
            "partition_key": partition_key,
            "sorting_key": sorting_key,
            "primary_key": pk_expr,
        });
        Ok(detail)
    }

    async fn run(&self, cfg: &ResolvedConfig, req: &QueryRequest) -> Result<QueryResult> {
        let sql = req.statement.trim();
        if sql.is_empty() {
            return Err(types::invalid("clickhouse: empty statement"));
        }
        let max_rows = req.max_rows.unwrap_or(1000);
        let started = Instant::now();

        // The active database (if the user selected one) scopes unqualified
        // table names — see query_rows_db for how it's applied per transport.
        let active_db = req.node.as_deref().map(str::trim).filter(|s| !s.is_empty());

        if returns_rows(sql) {
            let limited = types::inject_row_limit(sql, max_rows.saturating_add(1));
            let resp = self.query_rows_db(cfg, &limited, active_db).await?;
            let duration_ms = started.elapsed().as_millis() as u64;

            let columns: Vec<Column> = resp
                .meta
                .iter()
                .map(|(name, ty)| Column::typed(name, ty))
                .collect();

            let total = resp.data.len();
            let truncated = total > max_rows;
            // Cap oversized cells (e.g. AggregateFunction/*State blobs) so a
            // giant value can't break the grid.
            let rows: Vec<Vec<Value>> = resp
                .data
                .into_iter()
                .take(max_rows)
                .map(|row| row.into_iter().map(cap_cell).collect())
                .collect();
            let row_count = rows.len();

            Ok(QueryResult {
                columns,
                rows,
                rows_affected: None,
                stats: QueryStats {
                    duration_ms,
                    row_count,
                    bytes_read: Some(resp.bytes_read),
                },
                message: None,
                truncated,
            })
        } else {
            // Write / DDL statement: no rowset, just acknowledge.
            self.query_text_db(cfg, sql, active_db).await?;
            let duration_ms = started.elapsed().as_millis() as u64;
            let mut result = QueryResult::message("OK");
            result.rows_affected = None;
            result.stats = QueryStats {
                duration_ms,
                row_count: 0,
                bytes_read: None,
            };
            Ok(result)
        }
    }

    async fn completion(
        &self,
        cfg: &ResolvedConfig,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        let mut items: Vec<CompletionItem> = Vec::new();

        for kw in KEYWORDS {
            items.push(CompletionItem::new(*kw, CompletionKind::Keyword));
        }
        for (name, sig) in FUNCTIONS {
            items.push(CompletionItem::detailed(*name, CompletionKind::Function, *sig));
        }

        // Live identifiers — best-effort; never fail completion on a DB hiccup.
        if let Ok(resp) = self
            .query_rows(cfg, "SELECT name FROM system.databases ORDER BY name")
            .await
        {
            for name in resp.first_col_strs() {
                items.push(CompletionItem::new(name, CompletionKind::Database));
            }
        }

        let database = ctx
            .database
            .clone()
            .or_else(|| cfg.database.clone())
            .filter(|s| !s.is_empty());
        if let Some(db) = database {
            let tbl_sql = format!(
                "SELECT name FROM system.tables WHERE database = '{}' ORDER BY name LIMIT 500",
                esc(&db)
            );
            if let Ok(resp) = self.query_rows(cfg, &tbl_sql).await {
                for name in resp.first_col_strs() {
                    items.push(CompletionItem::new(name, CompletionKind::Table));
                }
            }

            let col_sql = format!(
                "SELECT name, type FROM system.columns WHERE database = '{}' \
                 ORDER BY table, position LIMIT 2000",
                esc(&db)
            );
            if let Ok(resp) = self.query_rows(cfg, &col_sql).await {
                for row in &resp.data {
                    if let Some(name) = row.first().and_then(Value::as_str) {
                        let ty = row.get(1).and_then(Value::as_str).unwrap_or("");
                        items.push(CompletionItem::detailed(name, CompletionKind::Column, ty));
                    }
                }
            }
        }

        Ok(CompletionResponse { items })
    }
}

// --- Static completion sources ----------------------------------------------

/// ClickHouse SQL keywords / clause heads.
const KEYWORDS: &[&str] = &[
    "SELECT", "DISTINCT", "FROM", "PREWHERE", "WHERE", "GROUP BY", "HAVING", "ORDER BY", "LIMIT",
    "OFFSET", "LIMIT BY", "WITH", "WITH TOTALS", "WITH ROLLUP", "WITH CUBE", "UNION ALL",
    "UNION DISTINCT", "INTERSECT", "EXCEPT", "JOIN", "INNER JOIN", "LEFT JOIN", "RIGHT JOIN",
    "FULL JOIN", "CROSS JOIN", "ANY JOIN", "ALL JOIN", "ASOF JOIN", "ARRAY JOIN",
    "LEFT ARRAY JOIN", "ON", "USING", "AS", "AND", "OR", "NOT", "IN", "GLOBAL IN", "BETWEEN",
    "LIKE", "ILIKE", "IS NULL", "IS NOT NULL", "CASE", "WHEN", "THEN", "ELSE", "END", "ASC",
    "DESC", "NULLS FIRST", "NULLS LAST", "SETTINGS", "FORMAT", "SAMPLE", "FINAL", "INSERT INTO",
    "VALUES", "SELECT *", "CREATE TABLE", "CREATE DATABASE", "CREATE VIEW",
    "CREATE MATERIALIZED VIEW", "ATTACH", "DETACH", "ALTER TABLE", "DROP TABLE", "DROP DATABASE",
    "RENAME TABLE", "TRUNCATE TABLE", "OPTIMIZE TABLE", "ENGINE", "PARTITION BY", "PRIMARY KEY",
    "ORDER BY", "TTL", "SHOW DATABASES", "SHOW TABLES", "SHOW CREATE TABLE", "DESCRIBE TABLE",
    "EXPLAIN", "SYSTEM", "USE",
];

/// ClickHouse builtin functions with a short signature/summary.
const FUNCTIONS: &[(&str, &str)] = &[
    ("count", "count() / count(expr) — number of rows"),
    ("countIf", "countIf(cond) — count where cond"),
    ("countDistinct", "countDistinct(expr) — alias for uniqExact"),
    ("sum", "sum(expr) — total"),
    ("sumIf", "sumIf(expr, cond) — conditional sum"),
    ("avg", "avg(expr) — mean"),
    ("avgIf", "avgIf(expr, cond) — conditional mean"),
    ("min", "min(expr)"),
    ("max", "max(expr)"),
    ("any", "any(expr) — first encountered value"),
    ("anyLast", "anyLast(expr) — last encountered value"),
    ("argMin", "argMin(arg, val) — arg of the minimum val"),
    ("argMax", "argMax(arg, val) — arg of the maximum val"),
    ("uniq", "uniq(expr) — approximate distinct count"),
    ("uniqExact", "uniqExact(expr) — exact distinct count"),
    ("uniqCombined", "uniqCombined(expr) — distinct count"),
    ("quantile", "quantile(level)(expr) — approximate quantile"),
    ("quantiles", "quantiles(l1, l2, ...)(expr)"),
    ("quantileExact", "quantileExact(level)(expr)"),
    ("median", "median(expr) — quantile(0.5)"),
    ("stddevPop", "stddevPop(expr)"),
    ("stddevSamp", "stddevSamp(expr)"),
    ("varPop", "varPop(expr)"),
    ("varSamp", "varSamp(expr)"),
    ("groupArray", "groupArray(expr) — values into an array"),
    ("groupUniqArray", "groupUniqArray(expr) — distinct values array"),
    ("groupArrayInsertAt", "groupArrayInsertAt(x, pos)"),
    ("arrayJoin", "arrayJoin(arr) — unfold an array into rows"),
    ("arrayMap", "arrayMap(func, arr)"),
    ("arrayFilter", "arrayFilter(func, arr)"),
    ("arraySum", "arraySum(arr)"),
    ("arrayCount", "arrayCount(func, arr)"),
    ("arrayElement", "arrayElement(arr, n) — arr[n]"),
    ("has", "has(arr, elem) — array contains element"),
    ("hasAll", "hasAll(arr, subset)"),
    ("hasAny", "hasAny(arr, set)"),
    ("indexOf", "indexOf(arr, x)"),
    ("length", "length(x) — array/string length"),
    ("empty", "empty(x)"),
    ("notEmpty", "notEmpty(x)"),
    ("toDate", "toDate(x) — cast to Date"),
    ("toDateTime", "toDateTime(x) — cast to DateTime"),
    ("toDateTime64", "toDateTime64(x, precision)"),
    ("toStartOfDay", "toStartOfDay(dt)"),
    ("toStartOfHour", "toStartOfHour(dt)"),
    ("toStartOfMinute", "toStartOfMinute(dt)"),
    ("toStartOfMonth", "toStartOfMonth(d)"),
    ("toStartOfWeek", "toStartOfWeek(d)"),
    ("toStartOfYear", "toStartOfYear(d)"),
    ("toYYYYMM", "toYYYYMM(d)"),
    ("toYear", "toYear(d)"),
    ("toMonth", "toMonth(d)"),
    ("toDayOfMonth", "toDayOfMonth(d)"),
    ("toHour", "toHour(dt)"),
    ("toMinute", "toMinute(dt)"),
    ("dateDiff", "dateDiff(unit, start, end)"),
    ("dateAdd", "dateAdd(unit, n, date)"),
    ("dateSub", "dateSub(unit, n, date)"),
    ("now", "now() — current DateTime"),
    ("today", "today() — current Date"),
    ("yesterday", "yesterday() — previous Date"),
    ("formatDateTime", "formatDateTime(dt, format)"),
    ("toString", "toString(x) — cast to String"),
    ("toInt32", "toInt32(x)"),
    ("toInt64", "toInt64(x)"),
    ("toUInt64", "toUInt64(x)"),
    ("toFloat64", "toFloat64(x)"),
    ("toDecimal64", "toDecimal64(x, scale)"),
    ("cast", "cast(x AS Type) / CAST(x, 'Type')"),
    ("ifNull", "ifNull(x, alt) — alt when x is NULL"),
    ("nullIf", "nullIf(a, b) — NULL when a = b"),
    ("coalesce", "coalesce(x, ...) — first non-NULL"),
    ("if", "if(cond, then, else)"),
    ("multiIf", "multiIf(c1, v1, ..., else)"),
    ("lower", "lower(s)"),
    ("upper", "upper(s)"),
    ("concat", "concat(s1, s2, ...)"),
    ("substring", "substring(s, offset, length)"),
    ("splitByChar", "splitByChar(sep, s)"),
    ("replaceAll", "replaceAll(haystack, pattern, replacement)"),
    ("trim", "trim(s)"),
    ("position", "position(haystack, needle)"),
    ("match", "match(s, pattern) — regex test"),
    ("extractAll", "extractAll(s, pattern)"),
    ("round", "round(x, n)"),
    ("floor", "floor(x)"),
    ("ceil", "ceil(x)"),
    ("abs", "abs(x)"),
    ("greatest", "greatest(a, b, ...)"),
    ("least", "least(a, b, ...)"),
    ("rand", "rand() — random UInt32"),
    ("dictGet", "dictGet(dict, attr, key)"),
    ("bitmapCardinality", "bitmapCardinality(bitmap)"),
    ("runningDifference", "runningDifference(x)"),
    ("rowNumberInAllBlocks", "rowNumberInAllBlocks()"),
];

// --- Unit tests (no network) ------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_cfg(port: u16) -> ResolvedConfig {
        ResolvedConfig {
            engine: Engine::Clickhouse,
            host: "127.0.0.1".into(),
            port,
            user: None,
            password: None,
            database: None,
            tls: types::TlsConfig::default(),
            params: json!({}),
        }
    }

    #[test]
    fn detects_statements_that_return_rows() {
        assert!(returns_rows("SELECT 1"));
        assert!(returns_rows("  select count() from t"));
        assert!(returns_rows("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(returns_rows("SHOW TABLES"));
        assert!(returns_rows("DESCRIBE TABLE t"));
        assert!(returns_rows("desc t"));
        assert!(returns_rows("EXPLAIN SELECT 1"));
        assert!(returns_rows("(SELECT 1)"));

        assert!(!returns_rows("INSERT INTO t VALUES (1)"));
        assert!(!returns_rows("CREATE TABLE t (a Int32) ENGINE = Memory"));
        assert!(!returns_rows("ALTER TABLE t ADD COLUMN b Int32"));
        assert!(!returns_rows("DROP TABLE t"));
        assert!(!returns_rows("  optimize table t final"));
        assert!(!returns_rows(""));
    }

    #[test]
    fn escapes_identifier_quotes() {
        assert_eq!(esc("plain"), "plain");
        assert_eq!(esc("o'brien"), "o''brien");
        assert_eq!(esc("a'b'c"), "a''b''c");
        assert_eq!(esc_ident("plain"), "plain");
        assert_eq!(esc_ident("we`ird"), "we``ird");
    }

    #[test]
    fn transport_selected_by_port() {
        // HTTP ports → HTTP.
        assert_eq!(transport_for(&base_cfg(8123)), Transport::Http);
        assert_eq!(transport_for(&base_cfg(8443)), Transport::Http);
        assert_eq!(transport_for(&base_cfg(18123)), Transport::Http);
        // Native protocol ports → native.
        assert_eq!(transport_for(&base_cfg(9000)), Transport::Native);
        assert_eq!(transport_for(&base_cfg(9440)), Transport::Native);
        // Forwarded native ports (tunnel local forward keeps the trailing
        // 9000/9440) → still native.
        assert_eq!(transport_for(&base_cfg(19000)), Transport::Native);
        assert_eq!(transport_for(&base_cfg(19440)), Transport::Native);
        // An arbitrary local-forward port falls back to HTTP unless overridden.
        assert_eq!(transport_for(&base_cfg(34567)), Transport::Http);
    }

    #[test]
    fn native_port_recognition() {
        assert!(is_native_port(9000));
        assert!(is_native_port(9440));
        assert!(is_native_port(19000));
        assert!(is_native_port(19440));
        assert!(!is_native_port(8123));
        assert!(!is_native_port(8443));
        assert!(!is_native_port(18123));
        assert!(!is_native_port(3306));
    }

    #[test]
    fn transport_param_forces_native() {
        // An explicit transport=native param overrides the port heuristic — for
        // any forward port that doesn't carry a recognizable native suffix.
        let mut cfg = base_cfg(34567);
        cfg.params = json!({ "transport": "native" });
        assert_eq!(transport_for(&cfg), Transport::Native);

        // Case-insensitive.
        cfg.params = json!({ "transport": "Native" });
        assert_eq!(transport_for(&cfg), Transport::Native);
    }

    #[test]
    fn value_scalars_to_json() {
        use klickhouse::Value as V;
        assert_eq!(value_to_json(&V::Null), Value::Null);
        assert_eq!(value_to_json(&V::Int32(-7)), json!(-7));
        assert_eq!(value_to_json(&V::UInt64(5)), json!(5));
        assert_eq!(value_to_json(&V::Float64(1.5)), json!(1.5));
        assert_eq!(
            value_to_json(&V::String(b"hello".to_vec())),
            json!("hello")
        );
        // 128-bit ints render as strings (out of f64/i64 range safely).
        assert_eq!(
            value_to_json(&V::Int128(170141183460469231731687303715884105727i128)),
            json!("170141183460469231731687303715884105727")
        );
    }

    #[test]
    fn value_array_and_tuple_to_json() {
        use klickhouse::Value as V;
        let arr = V::Array(vec![V::Int32(1), V::Int32(2), V::Int32(3)]);
        assert_eq!(value_to_json(&arr), json!([1, 2, 3]));

        let tup = V::Tuple(vec![V::String(b"a".to_vec()), V::Int32(9)]);
        assert_eq!(value_to_json(&tup), json!(["a", 9]));
    }

    #[test]
    fn value_map_to_json_object() {
        use klickhouse::Value as V;
        let map = V::Map(
            vec![V::String(b"k1".to_vec()), V::String(b"k2".to_vec())],
            vec![V::Int32(1), V::Int32(2)],
        );
        assert_eq!(value_to_json(&map), json!({ "k1": 1, "k2": 2 }));
    }

    #[test]
    fn strips_surrounding_quotes() {
        assert_eq!(strip_quotes("'1.2.3.4'".to_string()), "1.2.3.4");
        assert_eq!(strip_quotes("plain".to_string()), "plain");
    }

    #[test]
    fn json_cell_to_text_unwraps_strings() {
        assert_eq!(json_cell_to_text(&json!("CREATE TABLE x")), "CREATE TABLE x");
        assert_eq!(json_cell_to_text(&json!(42)), "42");
        assert_eq!(json_cell_to_text(&Value::Null), "");
    }
}
