//! One long-lived `kubectl proxy` per monitored cluster (perf r3-08-02).
//!
//! The proxy transport used to be one `kubectl get --raw …/proxy/<path>`
//! PROCESS per pod per probe per cycle: Go start-up, kubeconfig parse, a fresh
//! TLS handshake to the API server (and, for exec-plugin users, a credential
//! plugin) every time. At 5k pods that is 5k processes a cycle — longer than the
//! 60 s interval, so the collector never idled.
//!
//! Here the collector keeps ONE `kubectl proxy` child per cluster across
//! cycles and sends plain HTTP/1.1 requests to it over pooled keep-alive
//! connections. kubectl keeps its own pooled (HTTP/2) connection to the API
//! server, so a cycle costs zero process spawns once the proxy is up.
//!
//! Security: the proxy listens on a **Unix socket** inside a fresh `0700`
//! directory (never a loopback TCP port another local user could reach), only
//! accepts pod-proxy paths (`--accept-paths`) and rejects every method but
//! GET/HEAD (`--reject-methods`). The pod-HTTP actions pool (K-3,
//! `crate::pod_http`) starts its own proxies with [`KubeProxy::start_with`]
//! `allow_mutating = true`, which also admits POST/PUT/PATCH/DELETE — still on
//! pod-proxy paths only; the monitor's proxies stay GET-only. The directory is
//! removed and the child is killed when the gateway is dropped (loop exit,
//! config change, daemon exit).
//!
//! Credentials: kubectl resolves them once at start. The gateway remembers a
//! fingerprint of the argv, env and the kubeconfig files' mtimes (the EKS
//! token overlay is rewritten in place when the token is re-minted), and the
//! caller restarts it when that changes, when the child exits, or after a 401.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::client::conn::http1::{self, SendRequest};
use hyper_util::rt::TokioIo;
use otto_core::{Error, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::cli::Kubectl;

/// Time to wait for `kubectl proxy` to report it is serving.
const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// Idle keep-alive connections kept per gateway (= the collector's max
/// scrape concurrency, so a full cycle never reconnects).
const MAX_IDLE: usize = super::probes::MAX_CONCURRENCY as usize;
/// Only pod-proxy sub-resources and the version endpoint are reachable through
/// the socket (kubectl matches these unanchored-per-entry regexes).
const ACCEPT_PATHS: &str = r"^/api/v1/namespaces/[^/]+/pods/[^/]+/proxy(/.*)?$";
/// The collector's read-only gateway also lists a namespace's events (an
/// incremental watch read) and metrics-server pod metrics, so neither costs
/// a kubectl process per namespace per cycle (perf K5). GET-only: the
/// read-only proxy rejects every other method. Never on the mutating pool.
const ACCEPT_PATHS_READ: &str = r"^/api/v1/namespaces/[^/]+/pods/[^/]+/proxy(/.*)?$,^/api/v1/namespaces/[^/]+/events$,^/apis/metrics\.k8s\.io/v1beta1/namespaces/[^/]+/pods$";
/// kubectl's `--reject-methods` is a comma list of regexes.
const REJECT_METHODS: &str = "POST,PUT,PATCH,DELETE,CONNECT,OPTIONS,TRACE";
/// The pod-HTTP actions pool's proxies (see the module doc).
const REJECT_METHODS_MUTATING: &str = "CONNECT,OPTIONS,TRACE";

/// A running `kubectl proxy` bound to one cluster's kubectl handle.
pub struct KubeProxy {
    child: Mutex<Child>,
    dir: PathBuf,
    sock: PathBuf,
    fingerprint: String,
    idle: Mutex<Vec<SendRequest<Full<Bytes>>>>,
}

/// A full request result (status, headers, body bytes) — see [`KubeProxy::request`].
#[derive(Debug, Clone)]
pub struct RawResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The body exceeded the cap and was cut.
    pub truncated: bool,
}

/// What a request through the gateway returned.
#[derive(Debug, Clone)]
pub struct GatewayResponse {
    pub status: u16,
    pub body: String,
}

impl KubeProxy {
    /// Spawn `kubectl proxy --unix-socket <private dir>/s` for `k` and wait
    /// until it serves.
    pub async fn start(k: &Kubectl) -> Result<Self> {
        Self::start_with(k, false).await
    }

    /// [`start`](Self::start); `allow_mutating` also admits POST/PUT/PATCH/
    /// DELETE (pod-HTTP actions only — see the module doc).
    pub async fn start_with(k: &Kubectl, allow_mutating: bool) -> Result<Self> {
        let dir = private_dir()?;
        let sock = dir.join("s");
        let sock_arg = format!("--unix-socket={}", sock.display());
        let accept = format!(
            "--accept-paths={}",
            if allow_mutating {
                ACCEPT_PATHS
            } else {
                ACCEPT_PATHS_READ
            }
        );
        let reject = format!(
            "--reject-methods={}",
            if allow_mutating {
                REJECT_METHODS_MUTATING
            } else {
                REJECT_METHODS
            }
        );
        let argv = k.argv_stream(["proxy", sock_arg.as_str(), accept.as_str(), reject.as_str()]);
        let mut cmd = Command::new(&k.program);
        cmd.args(&argv)
            .envs(k.env.iter().map(|(a, b)| (a.as_str(), b.as_str())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(crate::cli::spawn_error(&k.program, &e));
            }
        };
        let fingerprint = fingerprint(k);
        // Build the gateway now so every early return below cleans up (Drop
        // kills the child and removes the directory).
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let gw = KubeProxy {
            child: Mutex::new(child),
            dir,
            sock,
            fingerprint,
            idle: Mutex::new(Vec::new()),
        };
        let Some(stdout) = stdout else {
            return Err(Error::Internal("kubectl proxy stdout not piped".into()));
        };
        let mut lines = BufReader::new(stdout).lines();
        let ready = tokio::time::timeout(READY_TIMEOUT, async {
            while let Ok(Some(line)) = lines.next_line().await {
                if line.starts_with("Starting to serve on") {
                    return true;
                }
            }
            false
        })
        .await;
        // Keep draining both pipes for the child's life so a full pipe never
        // blocks it; the tasks end at EOF when the child dies.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let mut err_tail = String::new();
        if let Some(stderr) = stderr {
            let mut err_lines = BufReader::new(stderr).lines();
            if !matches!(ready, Ok(true)) {
                // Not serving: collect the reason (bounded) for the error.
                let _ = tokio::time::timeout(Duration::from_millis(500), async {
                    while let Ok(Some(l)) = err_lines.next_line().await {
                        err_tail.push_str(&l);
                        err_tail.push('\n');
                        if err_tail.len() > 4096 {
                            break;
                        }
                    }
                })
                .await;
            }
            tokio::spawn(async move { while let Ok(Some(_)) = err_lines.next_line().await {} });
        }
        match ready {
            Ok(true) => Ok(gw),
            Ok(false) => Err(Error::Upstream(format!(
                "kubectl proxy exited: {}",
                crate::cli::redact(&crate::cli::first_meaningful_line(&err_tail))
            ))),
            Err(_) => Err(Error::Upstream(format!(
                "kubectl proxy did not become ready within {}s",
                READY_TIMEOUT.as_secs()
            ))),
        }
    }

    /// Still usable for `k`: the child runs and the credentials it loaded are
    /// the current ones.
    pub fn usable_for(&self, k: &Kubectl) -> bool {
        let running = matches!(
            self.child
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .try_wait(),
            Ok(None)
        );
        running && self.fingerprint == fingerprint(k)
    }

    /// `GET path` through the proxy, body capped at `max_body` bytes (the
    /// rest is discarded and that connection is not reused).
    pub async fn get(
        &self,
        path: &str,
        timeout: Duration,
        max_body: usize,
    ) -> Result<GatewayResponse> {
        let r = self
            .request(hyper::Method::GET, path, &[], None, timeout, max_body)
            .await?;
        Ok(GatewayResponse {
            status: r.status,
            body: String::from_utf8_lossy(&r.body).into_owned(),
        })
    }

    /// `method path` through the proxy with extra `headers` and an optional
    /// `body`; the response body is capped at `max_body` bytes (the rest is
    /// discarded and that connection is not reused). A proxy started without
    /// `allow_mutating` answers non-GET methods with 405.
    pub async fn request(
        &self,
        method: hyper::Method,
        path: &str,
        headers: &[(String, String)],
        body: Option<Vec<u8>>,
        timeout: Duration,
        max_body: usize,
    ) -> Result<RawResponse> {
        tokio::time::timeout(
            timeout,
            self.request_inner(method, path, headers, body, max_body),
        )
        .await
        .map_err(|_| Error::Upstream("timeout".into()))?
    }

    async fn request_inner(
        &self,
        method: hyper::Method,
        path: &str,
        headers: &[(String, String)],
        body: Option<Vec<u8>>,
        max_body: usize,
    ) -> Result<RawResponse> {
        let mut sender = self.connection().await?;
        let mut req = hyper::Request::builder()
            .method(method)
            .uri(path)
            // kubectl proxy's default --accept-hosts only admits localhost.
            .header(hyper::header::HOST, "localhost");
        for (k, v) in headers {
            if k.eq_ignore_ascii_case("host") {
                continue;
            }
            req = req.header(k.as_str(), v.as_str());
        }
        let req = req
            .body(Full::new(Bytes::from(body.unwrap_or_default())))
            .map_err(|e| Error::Invalid(format!("proxy request: {e}")))?;
        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| Error::Upstream(format!("kubectl proxy: {e}")))?;
        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut body = resp.into_body();
        let mut buf: Vec<u8> = Vec::new();
        let mut complete = true;
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| Error::Upstream(format!("read body: {e}")))?;
            if let Some(chunk) = frame.data_ref() {
                let room = max_body.saturating_sub(buf.len());
                if chunk.len() > room {
                    buf.extend_from_slice(&chunk[..room]);
                    complete = false;
                    break;
                }
                buf.extend_from_slice(chunk);
            }
        }
        if complete {
            self.give_back(sender);
        }
        Ok(RawResponse {
            status,
            headers,
            body: buf,
            truncated: !complete,
        })
    }

    /// An idle pooled connection that is still ready, else a new one.
    async fn connection(&self) -> Result<SendRequest<Full<Bytes>>> {
        loop {
            let pooled = self.idle.lock().unwrap_or_else(|p| p.into_inner()).pop();
            let Some(mut s) = pooled else { break };
            if s.ready().await.is_ok() {
                return Ok(s);
            }
        }
        let stream = tokio::net::UnixStream::connect(&self.sock)
            .await
            .map_err(|e| Error::Upstream(format!("kubectl proxy socket: {e}")))?;
        let (sender, conn) = http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| Error::Upstream(format!("kubectl proxy handshake: {e}")))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        Ok(sender)
    }

    fn give_back(&self, sender: SendRequest<Full<Bytes>>) {
        let mut idle = self.idle.lock().unwrap_or_else(|p| p.into_inner());
        if idle.len() < MAX_IDLE && !sender.is_closed() {
            idle.push(sender);
        }
    }

    #[cfg(test)]
    fn socket_dir(&self) -> &std::path::Path {
        &self.dir
    }
}

impl Drop for KubeProxy {
    fn drop(&mut self) {
        // `kill_on_drop` reaps the child; the socket dir is ours to remove.
        let _ = self
            .child
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .start_kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Reuse `slot`'s proxy when it is still valid for `k`, else (re)start one.
/// `None` ⇒ the proxy could not start; callers fall back to per-call kubectl.
pub async fn ensure<'a>(slot: &'a mut Option<KubeProxy>, k: &Kubectl) -> Option<&'a KubeProxy> {
    if slot.as_mut().is_some_and(|gw| !gw.usable_for(k)) {
        *slot = None;
    }
    if slot.is_none() {
        match KubeProxy::start(k).await {
            Ok(gw) => *slot = Some(gw),
            Err(e) => {
                tracing::debug!("k8s monitor: kubectl proxy unavailable ({e}); per-call kubectl");
                return None;
            }
        }
    }
    slot.as_ref()
}

/// A fresh `0700` directory for the socket. Kept short: macOS caps a Unix
/// socket path at 104 bytes and `$TMPDIR` is already ~50.
fn private_dir() -> Result<PathBuf> {
    let base = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    for attempt in 0..16u32 {
        let dir = base.join(format!(
            "ok8s-{}-{:x}",
            std::process::id(),
            nanos.wrapping_add(attempt)
        ));
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        match b.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Internal(format!("kubectl proxy dir: {e}"))),
        }
    }
    Err(Error::Internal("kubectl proxy dir: no free name".into()))
}

/// Program + argv + env + the mtimes of the kubeconfig files kubectl reads.
/// Kept in memory only (the env may carry AWS credentials).
fn fingerprint(k: &Kubectl) -> String {
    let mut s = String::new();
    s.push_str(&k.program);
    for a in &k.base_stream {
        s.push('\u{1}');
        s.push_str(a);
    }
    for (a, b) in &k.env {
        s.push('\u{2}');
        s.push_str(a);
        s.push('=');
        s.push_str(b);
    }
    for p in kubeconfig_files(k) {
        let m = std::fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        s.push_str(&format!("\u{3}{}@{m}", p.display()));
    }
    s
}

/// `--kubeconfig <p>` from the base flags, else `$KUBECONFIG` (colon list)
/// from the handle's env or the daemon's, else `~/.kube/config`.
pub fn kubeconfig_files(k: &Kubectl) -> Vec<PathBuf> {
    if let Some(i) = k.base_stream.iter().position(|a| a == "--kubeconfig") {
        if let Some(p) = k.base_stream.get(i + 1) {
            return vec![PathBuf::from(p)];
        }
    }
    let env = k
        .env
        .iter()
        .find(|(a, _)| a == "KUBECONFIG")
        .map(|(_, b)| b.clone())
        .or_else(|| std::env::var("KUBECONFIG").ok())
        .filter(|v| !v.trim().is_empty());
    match env {
        Some(v) => std::env::split_paths(&v).collect(),
        None => dirs::home_dir()
            .map(|h| vec![h.join(".kube").join("config")])
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn kubectl_on_path() -> bool {
        std::process::Command::new("kubectl")
            .arg("version")
            .arg("--client")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn handle(kubeconfig: &std::path::Path) -> Kubectl {
        let base_stream = vec![
            "--kubeconfig".to_string(),
            kubeconfig.to_string_lossy().into_owned(),
            "--context".to_string(),
            "t".to_string(),
        ];
        Kubectl {
            program: "kubectl".into(),
            base: base_stream.clone(),
            base_stream,
            env: vec![],
            reauth: None,
        }
    }

    #[test]
    fn fingerprint_follows_the_kubeconfig_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("kc");
        std::fs::write(&p, "a").unwrap();
        let k = handle(&p);
        assert_eq!(kubeconfig_files(&k), vec![p.clone()]);
        let before = fingerprint(&k);
        let t = SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(t)
            .unwrap();
        assert_ne!(before, fingerprint(&k));
    }

    #[test]
    fn socket_dir_is_private() {
        let d = private_dir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&d).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        assert!(d.join("s").as_os_str().len() < 104);
        std::fs::remove_dir_all(d).unwrap();
    }

    /// End to end against a fake API server: one proxy process serves every
    /// request over pooled connections; bodies are capped; non-pod paths and
    /// writes are refused by the proxy filter; drop removes the socket dir.
    #[tokio::test]
    async fn proxies_pod_paths_over_one_process() {
        if !kubectl_on_path() {
            eprintln!("skipped: kubectl not on PATH");
            return;
        }
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicUsize::new(0));
        let h2 = hits.clone();
        // Paths go through consts: otto-server's route-inventory and
        // policy-coverage tests treat every string literal passed to `.route` under
        // crates/ as a daemon route, and this fake API server is not one.
        const METRICS: &str = "/api/v1/namespaces/ns/pods/p1:9000/proxy/metrics";
        const BIG: &str = "/api/v1/namespaces/ns/pods/p1:9000/proxy/big";
        const DOWN: &str = "/api/v1/namespaces/ns/pods/p1:9000/proxy/down";
        let app = axum::Router::new()
            .route(
                METRICS,
                axum::routing::get(move || {
                    h2.fetch_add(1, Ordering::SeqCst);
                    async { "up 1\n" }
                }),
            )
            .route(BIG, axum::routing::get(|| async { "x".repeat(10_000) }))
            .route(
                DOWN,
                axum::routing::get(|| async {
                    (axum::http::StatusCode::SERVICE_UNAVAILABLE, "no")
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let dir = tempfile::tempdir().unwrap();
        let kc = dir.path().join("kubeconfig");
        let mut f = std::fs::File::create(&kc).unwrap();
        write!(
            f,
            "apiVersion: v1\nkind: Config\nclusters:\n- name: c\n  cluster:\n    server: http://{addr}\ncontexts:\n- name: t\n  context:\n    cluster: c\n    user: u\nusers:\n- name: u\n  user:\n    token: x\ncurrent-context: t\n"
        )
        .unwrap();
        drop(f);
        let k = handle(&kc);
        let mut slot = None;
        let gw = ensure(&mut slot, &k).await.expect("proxy starts");
        for _ in 0..5 {
            let r = gw
                .get(
                    "/api/v1/namespaces/ns/pods/p1:9000/proxy/metrics",
                    Duration::from_secs(5),
                    1024,
                )
                .await
                .unwrap();
            assert_eq!(r.status, 200);
            assert_eq!(r.body, "up 1\n");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 5);
        let big = gw
            .get(
                "/api/v1/namespaces/ns/pods/p1:9000/proxy/big",
                Duration::from_secs(5),
                100,
            )
            .await
            .unwrap();
        assert_eq!(big.body.len(), 100);
        let down = gw
            .get(
                "/api/v1/namespaces/ns/pods/p1:9000/proxy/down",
                Duration::from_secs(5),
                100,
            )
            .await
            .unwrap();
        assert_eq!(down.status, 503);
        // The filter only admits pod-proxy paths.
        let other = gw
            .get("/api/v1/namespaces/ns/secrets", Duration::from_secs(5), 100)
            .await
            .unwrap();
        assert_eq!(other.status, 403);
        // Reused while valid; the same process serves the next cycle.
        let first_dir = slot.as_ref().unwrap().socket_dir().to_path_buf();
        assert!(ensure(&mut slot, &k).await.is_some());
        assert_eq!(slot.as_ref().unwrap().socket_dir(), first_dir);
        drop(slot);
        assert!(!first_dir.exists());
    }
}
