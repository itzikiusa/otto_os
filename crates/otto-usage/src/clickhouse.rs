//! Managed **persistent** ClickHouse server + a thin HTTP query client.
//!
//! Earlier this shelled out to a fresh `clickhouse local --path` process per
//! query. Each spawn had to re-attach the whole on-disk dataset, and — fatally —
//! an ephemeral process runs no durable background-merge scheduler, so the usage
//! table accumulated tens of thousands of tiny never-merged parts (1.4 GB of
//! metadata for ~12 MB of data) and a ~30 s cold load.
//!
//! Now we run ONE long-lived `clickhouse server` per daemon, bound to a loopback
//! HTTP port, and query it over HTTP. The data dir is attached ONCE at start,
//! caches stay warm, background merges + TTL run continuously (parts stay low,
//! expired rows get dropped), so queries are ~tens of ms and disk stays small.
//!
//! Tested against ClickHouse 26.3/26.6. A `clickhouse local` data dir is adopted
//! in place by synthesizing the `metadata/default.sql` that `local` mode omits.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::process::{Child, Command};

/// HTTP settings appended to every query — match the old CLI flags exactly:
/// emit 64-bit ints as numbers (not strings), and resolve bare identifiers to
/// columns rather than same-named SELECT aliases (avoids ILLEGAL_AGGREGATION on
/// `sum(a+b) AS total` next to `sum(a) AS a`). Verified equivalent over HTTP.
const QUERY_SETTINGS: &str = "output_format_json_quote_64bit_integers=0&prefer_column_name_to_alias=1";

/// Handle to a persistent ClickHouse server backing an on-disk `--path` dataset.
pub struct ClickHouse {
    bin: PathBuf,
    data_dir: PathBuf,
    http: reqwest::Client,
    base_url: String,
    #[allow(dead_code)]
    port: u16,
    /// The server child process. Killed on `shutdown()`/`Drop`.
    child: Mutex<Option<Child>>,
}

impl ClickHouse {
    /// Resolve the `clickhouse` binary in priority order: an explicit configured
    /// path, then `PATH`, then well-known install locations. Returns an absolute
    /// path so the daemon can run it regardless of the working directory.
    pub fn locate(configured: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = configured.map(str::trim).filter(|s| !s.is_empty()) {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
        }
        if let Ok(out) = std::process::Command::new("which").arg("clickhouse").output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() && Path::new(&s).is_file() {
                    return Some(PathBuf::from(s));
                }
            }
        }
        let mut candidates = vec![
            PathBuf::from("/usr/local/bin/clickhouse"),
            PathBuf::from("/opt/homebrew/bin/clickhouse"),
        ];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("clickhouse"));
            candidates.push(home.join(".local/bin/clickhouse"));
            candidates.push(home.join("Library/Application Support/Otto/bin/clickhouse"));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    pub fn binary(&self) -> &Path {
        &self.bin
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Start (or re-claim then start) a persistent server over `data_dir` and
    /// return a client once it answers `/ping`. Adopts an existing
    /// `clickhouse local` dir in place. Retries on a transient port race.
    pub async fn start(bin: PathBuf, data_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| Error::Internal(format!("create clickhouse dir: {e}")))?;
        // Reclaim the dir from any orphaned server (crash / unclean shutdown).
        reclaim_dir(&data_dir).await;
        // `clickhouse local` omits metadata/default.sql; `server` requires it.
        ensure_default_metadata(&data_dir)?;

        // No global timeout — each request sets its own (queries are fast; DDL /
        // OPTIMIZE FINAL can take many minutes), so a slow merge isn't killed by a
        // short client cap.
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| Error::Internal(format!("http client: {e}")))?;

        let mut last_err = String::new();
        for attempt in 0..3 {
            let port = free_loopback_port()
                .map_err(|e| Error::Internal(format!("pick port: {e}")))?;
            let cfg = write_server_config(&data_dir, port)?;
            let base_url = format!("http://127.0.0.1:{port}");
            tracing::info!(
                "usage: starting clickhouse server (binary {}, data {}, port {})",
                bin.display(),
                data_dir.display(),
                port
            );
            let child = Command::new(&bin)
                .arg("server")
                .arg(format!("--config-file={}", cfg.display()))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| Error::Internal(format!("spawn clickhouse server: {e}")))?;

            let ch = Self {
                bin: bin.clone(),
                data_dir: data_dir.clone(),
                http: http.clone(),
                base_url,
                port,
                child: Mutex::new(Some(child)),
            };
            if ch.wait_ping(Duration::from_secs(45)).await {
                return Ok(ch);
            }
            last_err = format!("server did not answer /ping on port {port} (attempt {})", attempt + 1);
            tracing::warn!("usage: {last_err} — retrying");
            ch.shutdown().await; // kill the failed child before retrying
        }
        Err(Error::Internal(format!("clickhouse server failed to start: {last_err}")))
    }

    async fn wait_ping(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let url = format!("{}/ping", self.base_url);
        while std::time::Instant::now() < deadline {
            if let Ok(resp) = self.http.get(&url).send().await {
                if resp.status().is_success() {
                    if let Ok(body) = resp.text().await {
                        if body.trim() == "Ok." {
                            return true;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        false
    }

    /// POST `sql` (with `settings`) and return the raw response body, erroring on
    /// a non-2xx (ClickHouse returns a readable message in the body). `timeout`
    /// bounds the request — short for queries, long for DDL/OPTIMIZE.
    async fn post(&self, sql: String, settings: &str, timeout: Duration) -> Result<String> {
        let url = if settings.is_empty() {
            format!("{}/", self.base_url)
        } else {
            format!("{}/?{settings}", self.base_url)
        };
        let resp = self
            .http
            .post(&url)
            .timeout(timeout)
            .body(sql)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse http: {e}")))?;
        let ok = resp.status().is_success();
        let body = resp.text().await.unwrap_or_default();
        if ok {
            Ok(body)
        } else {
            Err(Error::Internal(format!("clickhouse query failed: {}", body.trim())))
        }
    }

    /// `clickhouse server`'s version via `SELECT version()`, falling back to the
    /// binary's `--version` if the server isn't reachable.
    pub async fn version(&self) -> Result<String> {
        if let Ok(rows) = self.query_rows("SELECT version() AS v").await {
            if let Some(v) = rows.first().and_then(|r| r.get("v")).and_then(|v| v.as_str()) {
                return Ok(format!("ClickHouse server version {v}"));
            }
        }
        let out = Command::new(&self.bin)
            .arg("local")
            .arg("--version")
            .output()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse --version: {e}")))?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string())
    }

    /// Run one or more `;`-separated statements that produce no rows (DDL,
    /// `ALTER`, …). Each statement is sent as its own HTTP request (our DDL never
    /// embeds `;` inside string literals, so a top-level split is safe).
    pub async fn exec(&self, sql: &str) -> Result<()> {
        // DDL/ALTER are fast, but OPTIMIZE … FINAL can take many minutes — allow up
        // to 30m per statement (the engine also wraps OPTIMIZE in its own timeout).
        for stmt in split_statements(sql) {
            self.post(stmt, "", Duration::from_secs(1800)).await?;
        }
        Ok(())
    }

    /// Run a `SELECT` and return each row as a JSON object.
    pub async fn query_rows(&self, sql: &str) -> Result<Vec<serde_json::Value>> {
        let body = self
            .post(
                format!("{sql}\nFORMAT JSONEachRow"),
                QUERY_SETTINGS,
                Duration::from_secs(120),
            )
            .await?;
        let mut rows = Vec::new();
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| Error::Internal(format!("parse clickhouse row: {e}")))?;
            rows.push(v);
        }
        Ok(rows)
    }

    /// Run several queries and return their row sets in order. With the warm
    /// persistent server, per-query cost is gone, so this is just sequential
    /// `query_rows` — ALL-OR-NOTHING: any failure errors the whole batch (same
    /// contract as the old single-process path, so `summary`'s empty-fallback
    /// still applies).
    pub async fn query_batch(&self, queries: &[String]) -> Result<Vec<Vec<serde_json::Value>>> {
        let mut out = Vec::with_capacity(queries.len());
        for q in queries {
            out.push(self.query_rows(q).await?);
        }
        Ok(out)
    }

    /// Bulk insert into `table` from newline-delimited JSON (one object per line).
    /// A blank payload is a no-op.
    pub async fn insert_ndjson(&self, table: &str, ndjson: &str) -> Result<()> {
        if ndjson.trim().is_empty() {
            return Ok(());
        }
        let q = urlencode(&format!("INSERT INTO {table} FORMAT JSONEachRow"));
        let url = format!("{}/?query={q}", self.base_url);
        let resp = self
            .http
            .post(&url)
            .timeout(Duration::from_secs(120))
            .body(ndjson.to_string())
            .send()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse insert http: {e}")))?;
        let ok = resp.status().is_success();
        let body = resp.text().await.unwrap_or_default();
        if ok {
            Ok(())
        } else {
            Err(Error::Internal(format!("clickhouse insert failed: {}", body.trim())))
        }
    }

    /// Stop the server: SIGTERM (clean flush), bounded wait, SIGKILL fallback.
    pub async fn shutdown(&self) {
        let child = self.child.lock().ok().and_then(|mut g| g.take());
        let Some(mut child) = child else { return };
        if let Some(pid) = child.id() {
            // SIGTERM for a clean shutdown (flush in-flight inserts + merges).
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .output()
                .await;
        }
        match tokio::time::timeout(Duration::from_secs(6), child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
    }
}

impl Drop for ClickHouse {
    fn drop(&mut self) {
        // Safety net if `shutdown()` wasn't called (e.g. a panic). `kill_on_drop`
        // already arms SIGKILL; this makes it explicit while the runtime is alive.
        if let Ok(mut g) = self.child.lock() {
            if let Some(mut child) = g.take() {
                let _ = child.start_kill();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Free helpers (unit-tested where pure)
// ---------------------------------------------------------------------------

/// Split a `;`-separated SQL script into trimmed, non-empty statements. Strips
/// `--` line comments FIRST so a `;` inside a comment (our schema has one) doesn't
/// split a statement mid-way. Safe for our controlled DDL (no `--` or `;` inside
/// string literals).
fn split_statements(sql: &str) -> Vec<String> {
    let no_comments: String = sql
        .lines()
        .map(|l| match l.find("--") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    no_comments
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Minimal percent-encoding for the few chars that appear in our INSERT query
/// strings (space + the reserved ones). Avoids a urlencoding dep.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Escape XML special chars for safe inclusion in the generated config.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Grab a free loopback TCP port by binding to `:0` and reading it back.
fn free_loopback_port() -> std::io::Result<u16> {
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = l.local_addr()?.port();
    drop(l);
    Ok(port)
}

/// Generate the minimal server config and return its path. Loopback-only, empty
/// default user (machine-local trust boundary, same as the old `local` mode),
/// memory-capped to stay desktop-light.
fn write_server_config(data_dir: &Path, port: u16) -> Result<PathBuf> {
    let server_dir = data_dir.join("server");
    std::fs::create_dir_all(&server_dir)
        .map_err(|e| Error::Internal(format!("create server dir: {e}")))?;
    std::fs::create_dir_all(data_dir.join("tmp"))
        .map_err(|e| Error::Internal(format!("create tmp dir: {e}")))?;
    let path = xml_escape(&format!("{}/", data_dir.to_string_lossy()));
    let tmp = xml_escape(&format!("{}/tmp/", data_dir.to_string_lossy()));
    let log = xml_escape(&server_dir.join("ch.log").to_string_lossy());
    let errlog = xml_escape(&server_dir.join("ch.err.log").to_string_lossy());
    let xml = format!(
        "<clickhouse>\n\
         <logger><level>warning</level><log>{log}</log><errorlog>{errlog}</errorlog>\
         <size>10M</size><count>1</count></logger>\n\
         <http_port>{port}</http_port>\n\
         <listen_host>127.0.0.1</listen_host>\n\
         <path>{path}</path>\n\
         <tmp_path>{tmp}</tmp_path>\n\
         <mark_cache_size>268435456</mark_cache_size>\n\
         <max_server_memory_usage_to_ram_ratio>0.3</max_server_memory_usage_to_ram_ratio>\n\
         <users><default><password/><networks><ip>127.0.0.1</ip></networks>\
         <profile>default</profile><quota>default</quota></default></users>\n\
         <profiles><default/></profiles><quotas><default/></quotas>\n\
         </clickhouse>\n"
    );
    let cfg = server_dir.join("config.xml");
    std::fs::write(&cfg, xml).map_err(|e| Error::Internal(format!("write config: {e}")))?;
    Ok(cfg)
}

/// `clickhouse local` creates the default database as a symlink under
/// `metadata/default` but writes no `metadata/default.sql`; `clickhouse server`
/// requires that ATTACH file (else "Data directory for default database exists,
/// but metadata file does not", Code 48). Synthesize it from the symlink's UUID.
/// No-op for a fresh dir (server creates the default DB itself) or an
/// already-adopted dir.
fn ensure_default_metadata(data_dir: &Path) -> Result<()> {
    let meta = data_dir.join("metadata");
    let link = meta.join("default");
    let sql = meta.join("default.sql");
    if sql.exists() {
        return Ok(());
    }
    // Only act when there's a symlinked default DB to adopt.
    let Ok(target) = std::fs::read_link(&link) else {
        return Ok(());
    };
    let uuid = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    if uuid.len() != 36 {
        // Not a UUID-named store dir — leave it to ClickHouse.
        tracing::warn!("usage: default db link target not a uuid ({uuid:?}); skipping metadata synth");
        return Ok(());
    }
    let body = format!("ATTACH DATABASE _ UUID '{uuid}'\nENGINE = Atomic\n");
    std::fs::write(&sql, body)
        .map_err(|e| Error::Internal(format!("write default.sql: {e}")))?;
    tracing::info!("usage: synthesized metadata/default.sql for in-place server adoption (uuid {uuid})");
    Ok(())
}

/// If a stale ClickHouse server still holds `data_dir` (unclean shutdown), stop
/// it so we can start ours. Reads the `status` file's `PID:` line and verifies
/// the process is actually a clickhouse pointed at this dir (PID-reuse-safe)
/// before signalling.
async fn reclaim_dir(data_dir: &Path) {
    let status = data_dir.join("status");
    let Ok(text) = std::fs::read_to_string(&status) else {
        return;
    };
    let Some(pid) = parse_status_pid(&text) else {
        return;
    };
    // Verify the PID is a clickhouse process referencing OUR data dir.
    let out = match Command::new("ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("command=")
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return,
    };
    let cmd = String::from_utf8_lossy(&out.stdout);
    let dir_s = data_dir.to_string_lossy();
    if !(cmd.contains("clickhouse") && cmd.contains(dir_s.as_ref())) {
        return; // stale status file / PID reused by an unrelated process
    }
    tracing::warn!("usage: reclaiming clickhouse data dir from stale server pid {pid}");
    let _ = Command::new("kill").arg("-TERM").arg(pid.to_string()).output().await;
    // Wait up to 5s for it to exit, then SIGKILL.
    for _ in 0..50 {
        let alive = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !alive {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = Command::new("kill").arg("-9").arg(pid.to_string()).output().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// Parse the PID from a ClickHouse `status` file (first `PID: <n>` line).
fn parse_status_pid(text: &str) -> Option<u32> {
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("PID:") {
            if let Ok(pid) = rest.trim().parse::<u32>() {
                return Some(pid);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_statements_basic() {
        let s = split_statements("CREATE TABLE a (x Int);\n  ALTER TABLE a ADD COLUMN y Int; \n");
        assert_eq!(s.len(), 2);
        assert!(s[0].starts_with("CREATE TABLE a"));
        assert!(s[1].starts_with("ALTER TABLE a"));
        assert!(split_statements("  ;; \n ;").is_empty());
    }

    #[test]
    fn split_statements_ignores_semicolon_in_line_comment() {
        // The real schema has `-- … (B1); nullable …` — the `;` is INSIDE a comment
        // and must NOT split the CREATE TABLE.
        let sql = "CREATE TABLE t (\n  a Int,\n  -- note (B1); still one statement\n  b Int\n) ENGINE=MergeTree ORDER BY a;\nALTER TABLE t ADD COLUMN c Int;";
        let s = split_statements(sql);
        assert_eq!(s.len(), 2, "comment semicolon must not split: {s:?}");
        assert!(s[0].contains("CREATE TABLE t") && s[0].contains("b Int"));
        assert!(s[1].starts_with("ALTER TABLE t"));
    }

    #[test]
    fn urlencode_reserved() {
        assert_eq!(urlencode("INSERT INTO t FORMAT JSONEachRow"),
                   "INSERT%20INTO%20t%20FORMAT%20JSONEachRow");
        assert_eq!(urlencode("abc-_.~"), "abc-_.~");
    }

    #[test]
    fn xml_escape_specials() {
        assert_eq!(xml_escape("/a&b/<c>/\"d\"/'e'"), "/a&amp;b/&lt;c&gt;/&quot;d&quot;/&apos;e&apos;");
        assert_eq!(xml_escape("/Users/me/Application Support/Otto"),
                   "/Users/me/Application Support/Otto");
    }

    #[test]
    fn parse_status_pid_works() {
        let s = "PID: 96987\nStarted at: 2026-06-26 23:42:34\nRevision: 54508\n";
        assert_eq!(parse_status_pid(s), Some(96987));
        assert_eq!(parse_status_pid("Started at: x\nRevision: 1"), None);
        assert_eq!(parse_status_pid("PID: notanumber"), None);
    }

    #[test]
    fn config_xml_well_formed_and_escaped() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_server_config(dir.path(), 18999).unwrap();
        let xml = std::fs::read_to_string(&cfg).unwrap();
        assert!(xml.contains("<http_port>18999</http_port>"));
        assert!(xml.contains("<listen_host>127.0.0.1</listen_host>"));
        assert!(xml.contains("max_server_memory_usage_to_ram_ratio"));
        // path is present + the server/tmp dirs were created
        assert!(dir.path().join("server").is_dir());
        assert!(dir.path().join("tmp").is_dir());
    }

    #[test]
    fn ensure_default_metadata_synthesizes_from_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("metadata");
        let store = dir.path().join("store/125/12529a29-611a-4836-91b1-4d7b8c8863fd");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(&meta).unwrap();
        std::os::unix::fs::symlink(&store, meta.join("default")).unwrap();
        ensure_default_metadata(dir.path()).unwrap();
        let sql = std::fs::read_to_string(meta.join("default.sql")).unwrap();
        assert!(sql.contains("ATTACH DATABASE _ UUID '12529a29-611a-4836-91b1-4d7b8c8863fd'"));
        assert!(sql.contains("ENGINE = Atomic"));
    }

    #[test]
    fn ensure_default_metadata_noop_for_fresh_dir() {
        let dir = tempfile::tempdir().unwrap();
        // no metadata/default symlink → no-op, no file written
        ensure_default_metadata(dir.path()).unwrap();
        assert!(!dir.path().join("metadata/default.sql").exists());
    }

    #[test]
    fn free_port_is_loopback() {
        let p = free_loopback_port().unwrap();
        assert!(p > 0);
    }
}
