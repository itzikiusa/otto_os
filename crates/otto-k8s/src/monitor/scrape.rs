//! How a probe body gets from a pod to the parser. Two transports:
//!
//! * **proxy** — `kubectl get --raw /api/v1/namespaces/{ns}/pods/{pod}:{port}/proxy{path}`.
//!   One process per fetch, but the HTTP hop is done by the API server, so it
//!   is the cheap path. Needs `pods/proxy` RBAC (denied on Rancher-managed
//!   clusters in practice).
//! * **port_forward** — `kubectl port-forward pod/{pod} 0:{port}` (kubectl
//!   picks a free local port, printed on stdout), then a plain `reqwest` GET to
//!   `127.0.0.1:{local}{path}`, then kill the child. One forward serves every
//!   probe on that port. Needs only `pods/portforward`.
//!
//! `pick_transport` probes the proxy once per cycle when the config says
//! `auto`. Nothing here talks to anything but the API server / loopback.
//!
//! The collector passes a [`KubeProxy`] gateway: then the proxy transport is a
//! pooled HTTP request to one long-lived `kubectl proxy` instead of a process
//! per fetch (r3-08-02). Without one (the one-off probe test, or when the
//! proxy cannot start) it falls back to `kubectl get --raw` per fetch.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::gateway::{GatewayResponse, KubeProxy};
use super::probes::{Probe, Transport};
use crate::cli::Kubectl;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportUsed {
    Proxy,
    PortForward,
}

impl TransportUsed {
    pub fn as_str(self) -> &'static str {
        match self {
            TransportUsed::Proxy => "proxy",
            TransportUsed::PortForward => "port_forward",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrapeTarget {
    pub namespace: String,
    pub pod: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub probe: String,
    pub status: u16,
    pub body: String,
    pub ms: u64,
}

/// Time to wait for `port-forward` to print its local port.
const FORWARD_READY: Duration = Duration::from_secs(10);
/// Cap on a proxy body (kubectl buffers stdout; a runaway `/metrics` should
/// not pin memory).
const MAX_BODY: usize = 8 * 1024 * 1024;

/// `/api/v1/namespaces/{ns}/pods/{pod}:{port}/proxy{path}`.
pub fn proxy_path(t: &ScrapeTarget, path: &str) -> String {
    format!(
        "/api/v1/namespaces/{}/pods/{}:{}/proxy{}",
        t.namespace, t.pod, t.port, path
    )
}

/// `Forwarding from 127.0.0.1:54321 -> 9000` → `54321`.
pub fn parse_forward_port(stdout_line: &str) -> Option<u16> {
    let rest = stdout_line.trim().strip_prefix("Forwarding from ")?;
    let addr = rest.split(" -> ").next()?;
    let port = addr.rsplit(':').next()?;
    port.parse().ok()
}

/// Decide the transport for this cycle. `Auto` tries the proxy against
/// `sample`: see [`pick_transport_multi`] for what counts as "unavailable".
pub async fn pick_transport(
    k: &Kubectl,
    want: Transport,
    sample: &ScrapeTarget,
    path: &str,
) -> TransportUsed {
    pick_transport_via(k, None, want, sample, path).await
}

/// [`pick_transport`] through the cluster's gateway when there is one (an HTTP
/// request, not a process).
pub async fn pick_transport_via(
    k: &Kubectl,
    gw: Option<&KubeProxy>,
    want: Transport,
    sample: &ScrapeTarget,
    path: &str,
) -> TransportUsed {
    pick_transport_multi(k, gw, want, std::slice::from_ref(sample), path)
        .await
        .0
}

/// Most pods the `Auto` sniff tries before deciding (K1: one sample pod that
/// answered its app 404 used to send the whole cycle to port-forward).
pub const SNIFF_TARGETS: usize = 3;

/// What one proxy sniff against one pod proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SniffVerdict {
    /// The API server delivered the APP's answer — any status, a 404 or 500
    /// from the app included. The proxy transport works.
    ProxyWorks,
    /// The API server itself refused (RBAC `Forbidden` / `Unauthorized`
    /// `Status`) or the hop to it failed: port-forward is the way.
    ApiDenied,
    /// Says nothing about the transport (the pod vanished, is not listening on
    /// that port, timed out): try another pod.
    Inconclusive,
}

/// Is `body` a Kubernetes API `Status` object (what the API server — not the
/// app — answers an error with)? Returns its `reason`.
fn k8s_status_reason(body: &str) -> Option<String> {
    let t = body.trim_start();
    if !t.starts_with('{') || !t.contains("\"Status\"") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(t).ok()?;
    if v.get("kind").and_then(|k| k.as_str()) != Some("Status") {
        return None;
    }
    Some(
        v.get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string(),
    )
}

/// Classify a gateway (`kubectl proxy`) answer to the sniff.
pub fn verdict_from_gateway(res: &Result<GatewayResponse>) -> SniffVerdict {
    match res {
        Ok(r) if (200..300).contains(&r.status) => SniffVerdict::ProxyWorks,
        Ok(r) => match k8s_status_reason(&r.body).as_deref() {
            // Not a k8s Status: the app answered (its 404 / 500 still proves
            // the API server proxied the request).
            None => SniffVerdict::ProxyWorks,
            Some("Forbidden") | Some("Unauthorized") => SniffVerdict::ApiDenied,
            // NotFound (pod gone), ServiceUnavailable (nothing listening on
            // the port), InternalError, … — not about the transport.
            Some(_) => SniffVerdict::Inconclusive,
        },
        Err(e) => {
            let m = e.to_string().to_ascii_lowercase();
            if m.contains("timeout") || m.contains("timed out") {
                SniffVerdict::Inconclusive
            } else {
                SniffVerdict::ApiDenied
            }
        }
    }
}

/// Classify a `kubectl get --raw` answer to the sniff (kubectl turns every
/// non-2xx into an exit 1 + `Error from server (<Reason>): …`).
pub fn verdict_from_kubectl<T>(res: &Result<T>) -> SniffVerdict {
    match res {
        Ok(_) => SniffVerdict::ProxyWorks,
        Err(Error::Forbidden(_)) => SniffVerdict::ApiDenied,
        Err(e) if crate::cli::is_credentials_rejected(e) => SniffVerdict::ApiDenied,
        // The app's 404 relayed by the API server — the pod itself missing
        // reads `pods "x" not found` instead.
        Err(Error::NotFound(m)) if m.contains("could not find the requested resource") => {
            SniffVerdict::ProxyWorks
        }
        Err(_) => SniffVerdict::Inconclusive,
    }
}

/// Fold per-pod verdicts (in order) into the transport + whether the answer
/// is conclusive (cacheable). First decisive verdict wins; all-inconclusive
/// stays on the cheap proxy path but is re-sniffed next cycle.
pub fn decide(verdicts: &[SniffVerdict]) -> (TransportUsed, bool) {
    for v in verdicts {
        match v {
            SniffVerdict::ProxyWorks => return (TransportUsed::Proxy, true),
            SniffVerdict::ApiDenied => return (TransportUsed::PortForward, true),
            SniffVerdict::Inconclusive => {}
        }
    }
    (TransportUsed::Proxy, false)
}

/// Pick up to [`SNIFF_TARGETS`] sniff pods spread over `targets` (first,
/// middle, last — different namespaces/workloads more often than not).
pub fn sniff_sample<T: Clone>(targets: &[T]) -> Vec<T> {
    let n = targets.len();
    let mut idx: Vec<usize> = match n {
        0 => vec![],
        1..=3 => (0..n).collect(),
        _ => vec![0, n / 2, n - 1],
    };
    idx.dedup();
    idx.into_iter().map(|i| targets[i].clone()).collect()
}

/// The `Auto` sniff over several pods (K1). Only an API-server-level failure
/// (RBAC `Status`, a broken hop) selects port-forward; an app's own non-2xx
/// proves the proxy works. Returns the transport and whether it is
/// conclusive — the collector caches a conclusive answer per cluster.
pub async fn pick_transport_multi(
    k: &Kubectl,
    gw: Option<&KubeProxy>,
    want: Transport,
    samples: &[ScrapeTarget],
    path: &str,
) -> (TransportUsed, bool) {
    match want {
        Transport::Proxy => (TransportUsed::Proxy, true),
        Transport::PortForward => (TransportUsed::PortForward, true),
        Transport::Auto => {
            let mut verdicts = Vec::with_capacity(samples.len());
            for sample in samples.iter().take(SNIFF_TARGETS) {
                let p = proxy_path(sample, path);
                let v = match gw {
                    Some(gw) => {
                        verdict_from_gateway(&gw.get(&p, Duration::from_secs(5), MAX_BODY).await)
                    }
                    None => verdict_from_kubectl(
                        &k.run_timeout(["get", "--raw", p.as_str()], Duration::from_secs(5))
                            .await,
                    ),
                };
                verdicts.push(v);
                if v != SniffVerdict::Inconclusive {
                    break;
                }
            }
            let (t, conclusive) = decide(&verdicts);
            if t == TransportUsed::PortForward {
                tracing::debug!("k8s monitor: proxy denied by the API server; using port-forward");
            }
            (t, conclusive)
        }
    }
}

/// `kubectl get --raw` fails on a non-2xx answer; the gateway path keeps that
/// meaning so a pod is counted scraped/failed exactly as before.
fn require_2xx(status: u16) -> Result<()> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(Error::Upstream(format!("HTTP {status}")))
    }
}

/// Fetch every probe for one pod/port. One result per probe, in order.
pub async fn fetch(
    k: &Kubectl,
    t: TransportUsed,
    target: &ScrapeTarget,
    probes: &[Probe],
) -> Vec<Result<ProbeResult>> {
    fetch_via(k, None, t, target, probes).await
}

/// [`fetch`], with proxy fetches sent through `gw` when given.
pub async fn fetch_via(
    k: &Kubectl,
    gw: Option<&KubeProxy>,
    t: TransportUsed,
    target: &ScrapeTarget,
    probes: &[Probe],
) -> Vec<Result<ProbeResult>> {
    fetch_pooled(k, gw, None, t, target, probes).await
}

/// [`fetch_via`]; port-forward fetches reuse a long-lived forward from
/// `pool` (kept across cycles) instead of spawning + killing one per pod.
pub async fn fetch_pooled(
    k: &Kubectl,
    gw: Option<&KubeProxy>,
    pool: Option<&ForwardPool>,
    t: TransportUsed,
    target: &ScrapeTarget,
    probes: &[Probe],
) -> Vec<Result<ProbeResult>> {
    match t {
        TransportUsed::Proxy => {
            let mut out = Vec::with_capacity(probes.len());
            for p in probes {
                out.push(match gw {
                    Some(gw) => fetch_gateway(gw, target, p).await,
                    None => fetch_proxy(k, target, p).await,
                });
            }
            out
        }
        TransportUsed::PortForward => {
            let res = match pool {
                Some(pool) => pool.fetch(k, target, probes).await,
                None => fetch_forward(k, target, probes).await,
            };
            match res {
                Ok(v) => v,
                Err(e) => probes
                    .iter()
                    .map(|_| Err(Error::Upstream(e.to_string())))
                    .collect(),
            }
        }
    }
}

/// `(namespace, pod, port)` — one forward each.
type ForwardKey = (String, String, u16);

/// Most `kubectl port-forward` children the pool keeps alive at once (perf
/// R2). Each is a resident kubectl (~30 MB RSS) and one process against the
/// per-user limit (`kern.maxprocperuid`, 2666 on a stock Mac): unbounded, a
/// 150-pod cluster held several GB and a ~2.6k-pod one broke process spawning
/// for the whole login session. Pods past the cap get a one-shot forward
/// (spawned, used, killed) — the pre-pool behaviour, bounded by the scrape
/// concurrency.
pub const FORWARD_POOL_MAX: usize = 32;

struct Forward {
    child: tokio::process::Child,
    local: u16,
    /// Last checkout (LRU eviction + idle reaping).
    last_used: Instant,
    /// The [`ForwardPool::begin_cycle`] epoch of the last checkout: an entry
    /// already used THIS cycle is never evicted for another target, so a
    /// cycle over more targets than the cap keeps a stable resident set
    /// instead of thrashing (LRU over a cyclic scan always misses).
    epoch: u64,
    /// Checked out by a scrape right now — never evicted or reaped.
    busy: bool,
}

#[derive(Default)]
struct PoolState {
    map: std::collections::HashMap<ForwardKey, Forward>,
    /// Slots reserved by spawns in flight (counted against the cap).
    pending: usize,
    epoch: u64,
}

/// How a scrape reaches its pod's port.
enum Slot {
    /// A live pooled forward on this local port (checked out).
    Pooled(u16),
    /// A slot was reserved: spawn a forward and add it to the pool.
    Spawn,
    /// The pool is full of forwards in use this cycle: forward once.
    Overflow,
}

/// Long-lived `kubectl port-forward`s keyed by `(ns, pod, port)`, kept across
/// collector cycles (K1: a forward per pod per cycle was 40 s of a 60 s cycle
/// at 150 pods), plus ONE shared HTTP client. Bounded (R2): at most `cap`
/// children, the least-recently-used idle one evicted for a new target, and
/// one-shot forwards past the cap. A forward whose child exited or whose
/// connection broke is dropped and re-spawned on the next use; the collector
/// [`retain`](Self::retain)s only pods still being scraped and
/// [`reap_idle`](Self::reap_idle)s forwards nobody used for a while. Every
/// child is `kill_on_drop`, so dropping the pool ends them all.
pub struct ForwardPool {
    client: reqwest::Client,
    cap: usize,
    state: tokio::sync::Mutex<PoolState>,
    spawned: std::sync::atomic::AtomicU64,
    overflow: std::sync::atomic::AtomicU64,
}

impl Default for ForwardPool {
    fn default() -> Self {
        Self::new()
    }
}

impl ForwardPool {
    pub fn new() -> Self {
        Self::with_cap(FORWARD_POOL_MAX)
    }

    /// A pool holding at most `cap` (≥ 1) forwards.
    pub fn with_cap(cap: usize) -> Self {
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap_or_default(),
            cap: cap.max(1),
            state: Default::default(),
            spawned: Default::default(),
            overflow: Default::default(),
        }
    }

    fn key(t: &ScrapeTarget) -> ForwardKey {
        (t.namespace.clone(), t.pod.clone(), t.port)
    }

    /// The most forwards this pool keeps alive.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Forwards spawned over the pool's life, pooled + one-shot (tests /
    /// diagnostics).
    pub fn spawned(&self) -> u64 {
        self.spawned.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// One-shot forwards used because the pool was full.
    pub fn overflowed(&self) -> u64 {
        self.overflow.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Open forwards right now.
    pub async fn len(&self) -> usize {
        self.state.lock().await.map.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// Start a collector cycle: forwards checked out from now on are this
    /// cycle's resident set (see [`Forward::epoch`]).
    pub async fn begin_cycle(&self) {
        self.state.lock().await.epoch += 1;
    }

    /// Kill every forward whose target `keep` rejects.
    pub async fn retain(&self, keep: impl Fn(&str, &str, u16) -> bool) {
        self.state
            .lock()
            .await
            .map
            .retain(|(ns, pod, port), _| keep(ns, pod, *port));
    }

    /// Kill every idle forward not used for `max_idle`; returns how many.
    pub async fn reap_idle(&self, max_idle: Duration) -> usize {
        let mut s = self.state.lock().await;
        let before = s.map.len();
        s.map
            .retain(|_, f| f.busy || f.last_used.elapsed() < max_idle);
        before - s.map.len()
    }

    pub async fn clear(&self) {
        self.state.lock().await.map.clear();
    }

    /// Check out a forward to `target`: a live pooled one, else a reserved
    /// slot (evicting the LRU idle forward not used this cycle when full),
    /// else overflow. The lock is only held for map work — never across a
    /// spawn.
    async fn checkout(&self, key: &ForwardKey) -> Slot {
        let mut s = self.state.lock().await;
        let epoch = s.epoch;
        if let Some(f) = s.map.get_mut(key) {
            if !f.busy && matches!(f.child.try_wait(), Ok(None)) {
                f.busy = true;
                f.last_used = Instant::now();
                f.epoch = epoch;
                return Slot::Pooled(f.local);
            }
            if !f.busy {
                s.map.remove(key);
            } else {
                // The same target scraped twice at once (not the collector's
                // pattern): don't share, don't grow.
                return Slot::Overflow;
            }
        }
        if s.map.len() + s.pending >= self.cap {
            let victim = s
                .map
                .iter()
                .filter(|(_, f)| !f.busy && f.epoch < epoch)
                .min_by_key(|(_, f)| f.last_used)
                .map(|(k, _)| k.clone());
            match victim {
                Some(v) => {
                    s.map.remove(&v);
                }
                None => return Slot::Overflow,
            }
        }
        s.pending += 1;
        Slot::Spawn
    }

    /// The local port of a live forward to `target` (checked out), or `None`
    /// when the pool is full and the caller should forward once.
    async fn local_port(&self, k: &Kubectl, target: &ScrapeTarget) -> Result<Option<u16>> {
        let key = Self::key(target);
        match self.checkout(&key).await {
            Slot::Pooled(local) => Ok(Some(local)),
            Slot::Overflow => Ok(None),
            Slot::Spawn => {
                let spawned = spawn_forward(k, target).await;
                let mut s = self.state.lock().await;
                s.pending -= 1;
                let (child, local) = spawned?;
                self.spawned
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let epoch = s.epoch;
                s.map.insert(
                    key,
                    Forward {
                        child,
                        local,
                        last_used: Instant::now(),
                        epoch,
                        busy: true,
                    },
                );
                Ok(Some(local))
            }
        }
    }

    /// Return a checked-out forward; a broken one is killed (the next use
    /// starts a fresh one).
    async fn release(&self, key: &ForwardKey, broken: bool) {
        let mut s = self.state.lock().await;
        if broken {
            s.map.remove(key);
        } else if let Some(f) = s.map.get_mut(key) {
            f.busy = false;
            f.last_used = Instant::now();
        }
    }

    async fn fetch(
        &self,
        k: &Kubectl,
        target: &ScrapeTarget,
        probes: &[Probe],
    ) -> Result<Vec<Result<ProbeResult>>> {
        let Some(local) = self.local_port(k, target).await? else {
            self.overflow
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.spawned
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return fetch_forward_with(&self.client, k, target, probes).await;
        };
        let (out, broken) = get_probes(&self.client, local, probes).await;
        // A broken forward died under us (pod restarted, kubectl lost the
        // stream): drop it so the next cycle starts a fresh one.
        self.release(&Self::key(target), broken).await;
        Ok(out)
    }
}

/// GET every probe on `127.0.0.1:{local}`. The flag is true when every probe
/// failed to CONNECT (the forward itself is broken, not the app).
async fn get_probes(
    client: &reqwest::Client,
    local: u16,
    probes: &[Probe],
) -> (Vec<Result<ProbeResult>>, bool) {
    let mut out = Vec::with_capacity(probes.len());
    let mut connect_failures = 0usize;
    for p in probes {
        let started = Instant::now();
        let url = format!("http://127.0.0.1:{local}{}", p.path);
        let res = client
            .get(&url)
            .timeout(Duration::from_millis(p.timeout_ms))
            .send()
            .await;
        let r = match res {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match resp.text().await {
                    Ok(mut body) => {
                        if body.len() > MAX_BODY {
                            body.truncate(MAX_BODY);
                        }
                        Ok(ProbeResult {
                            probe: p.name.clone(),
                            status,
                            body,
                            ms: started.elapsed().as_millis() as u64,
                        })
                    }
                    Err(e) => Err(Error::Upstream(format!("{}: read body: {e}", p.name))),
                }
            }
            Err(e) => {
                if e.is_connect() || e.is_request() {
                    connect_failures += 1;
                }
                Err(Error::Upstream(format!(
                    "{}: {}",
                    p.name,
                    if e.is_timeout() {
                        "timeout".to_string()
                    } else {
                        e.to_string()
                    }
                )))
            }
        };
        out.push(r);
    }
    let broken = !probes.is_empty() && connect_failures == probes.len();
    (out, broken)
}

async fn fetch_proxy(k: &Kubectl, target: &ScrapeTarget, p: &Probe) -> Result<ProbeResult> {
    let started = Instant::now();
    let path = proxy_path(target, &p.path);
    // Capped while reading (S6-09): a probe on `/actuator/heapdump` × the
    // cycle's concurrency must not buffer whole bodies before the cap.
    let out = k
        .run_timeout_capped(
            ["get", "--raw", path.as_str()],
            Duration::from_millis(p.timeout_ms.max(1000) + 2000),
            MAX_BODY,
        )
        .await?;
    let mut body = out.stdout;
    if body.len() > MAX_BODY {
        // Lossy UTF-8 can grow a cut multi-byte tail; keep a char boundary.
        let mut cut = MAX_BODY;
        while !body.is_char_boundary(cut) {
            cut -= 1;
        }
        body.truncate(cut);
    }
    Ok(ProbeResult {
        probe: p.name.clone(),
        status: 200,
        body,
        ms: started.elapsed().as_millis() as u64,
    })
}

async fn fetch_gateway(gw: &KubeProxy, target: &ScrapeTarget, p: &Probe) -> Result<ProbeResult> {
    let started = Instant::now();
    let path = proxy_path(target, &p.path);
    let r = gw
        .get(
            &path,
            Duration::from_millis(p.timeout_ms.max(1000) + 2000),
            MAX_BODY,
        )
        .await?;
    require_2xx(r.status)?;
    Ok(ProbeResult {
        probe: p.name.clone(),
        status: r.status,
        body: r.body,
        ms: started.elapsed().as_millis() as u64,
    })
}

/// Spawn `kubectl port-forward pod/<pod> 0:<port>` and return the child +
/// the local port it bound.
pub(crate) async fn spawn_forward(
    k: &Kubectl,
    target: &ScrapeTarget,
) -> Result<(tokio::process::Child, u16)> {
    let argv = k.argv_stream([
        "port-forward",
        "-n",
        target.namespace.as_str(),
        &format!("pod/{}", target.pod),
        &format!("0:{}", target.port),
        "--address",
        "127.0.0.1",
    ]);
    let mut cmd = Command::new(&k.program);
    cmd.args(&argv)
        .envs(k.env.iter().map(|(a, b)| (a.as_str(), b.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("spawn kubectl port-forward: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("port-forward stdout not piped".into()))?;
    let mut lines = BufReader::new(stdout).lines();
    let port = tokio::time::timeout(FORWARD_READY, async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(p) = parse_forward_port(&line) {
                return Some(p);
            }
        }
        None
    })
    .await;
    match port {
        Ok(Some(p)) => {
            // Keep draining stdout for the life of the forward: kubectl logs
            // "Handling connection for <port>" per request, and a closed read
            // end would SIGPIPE it mid-scrape (the requests then fail with
            // "error sending request"). The task ends at EOF when we kill the
            // child.
            tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
            if let Some(stderr) = child.stderr.take() {
                let mut err_lines = BufReader::new(stderr).lines();
                tokio::spawn(async move { while let Ok(Some(_)) = err_lines.next_line().await {} });
            }
            Ok((child, p))
        }
        Ok(None) => {
            let stderr = drain_stderr(&mut child).await;
            let _ = child.kill().await;
            Err(Error::Upstream(format!(
                "port-forward to {}/{} exited: {}",
                target.namespace,
                target.pod,
                stderr.lines().next().unwrap_or("").trim()
            )))
        }
        Err(_) => {
            let _ = child.kill().await;
            Err(Error::Upstream(format!(
                "port-forward to {}/{} did not become ready within {}s",
                target.namespace,
                target.pod,
                FORWARD_READY.as_secs()
            )))
        }
    }
}

async fn drain_stderr(child: &mut tokio::process::Child) -> String {
    let Some(stderr) = child.stderr.take() else {
        return String::new();
    };
    let mut lines = BufReader::new(stderr).lines();
    let mut out = String::new();
    let read = tokio::time::timeout(Duration::from_millis(500), async {
        while let Ok(Some(l)) = lines.next_line().await {
            out.push_str(&l);
            out.push('\n');
            if out.len() > 4096 {
                break;
            }
        }
    });
    let _ = read.await;
    out
}

async fn fetch_forward(
    k: &Kubectl,
    target: &ScrapeTarget,
    probes: &[Probe],
) -> Result<Vec<Result<ProbeResult>>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| Error::Internal(format!("http client: {e}")))?;
    fetch_forward_with(&client, k, target, probes).await
}

/// One-shot forward: spawn, GET every probe, kill.
async fn fetch_forward_with(
    client: &reqwest::Client,
    k: &Kubectl,
    target: &ScrapeTarget,
    probes: &[Probe],
) -> Result<Vec<Result<ProbeResult>>> {
    let (mut child, local) = spawn_forward(k, target).await?;
    let (out, _) = get_probes(client, local, probes).await;
    let _ = child.kill().await;
    Ok(out)
}

/// Group probes by the port they hit (a probe without a port uses
/// `default_port`, the container's first declared port). Probes with no
/// resolvable port are returned separately so the caller can count them as
/// failed.
pub fn group_by_port(
    probes: &[Probe],
    default_port: Option<u16>,
) -> (BTreeMap<u16, Vec<Probe>>, Vec<String>) {
    let mut by_port: BTreeMap<u16, Vec<Probe>> = BTreeMap::new();
    let mut unresolved = Vec::new();
    for p in probes {
        match p.port.or(default_port) {
            Some(port) => by_port.entry(port).or_default().push(p.clone()),
            None => unresolved.push(p.name.clone()),
        }
    }
    (by_port, unresolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::probes::ProbeFormat;

    /// perf R2: a port-forward cycle over more targets than the cap keeps at
    /// most `cap` children alive, holds a STABLE resident set (later cycles
    /// spawn only `targets - cap` one-shots, not `targets`), and idle
    /// forwards are reaped. The fake `kubectl port-forward` announces a local
    /// port served by an in-test HTTP stub, then sleeps like the real one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn forward_pool_is_bounded_with_a_stable_resident_set() {
        use futures_util::StreamExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = sock.read(&mut buf).await;
                    let _ = sock
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                });
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("kubectl");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"Forwarding from 127.0.0.1:{port} -> 9000\"\nexec sleep 300\n"
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let k = Kubectl {
            program: script.display().to_string(),
            base: vec![],
            base_stream: vec![],
            env: vec![],
            reauth: None,
        };
        let probes: Vec<Probe> = vec![serde_json::from_value(serde_json::json!({
            "name": "health", "port": 9000, "path": "/health", "format": "health"
        }))
        .unwrap()];
        let targets: Vec<ScrapeTarget> = (0..40)
            .map(|i| ScrapeTarget {
                namespace: "shop".into(),
                pod: format!("web-{i:03}"),
                port: 9000,
            })
            .collect();
        const CAP: usize = 8;
        let pool = ForwardPool::with_cap(CAP);
        for cycle in 0..3 {
            pool.begin_cycle().await;
            let before = pool.spawned();
            let results: Vec<_> =
                futures_util::stream::iter(targets.iter().map(|t| pool.fetch(&k, t, &probes)))
                    .buffered(4)
                    .collect()
                    .await;
            for r in &results {
                let r = r.as_ref().expect("forward");
                assert_eq!(r[0].as_ref().expect("probe").status, 200);
            }
            assert!(
                pool.len().await <= CAP,
                "cycle {cycle}: {} > {CAP}",
                pool.len().await
            );
            let spawned = (pool.spawned() - before) as usize;
            let want = if cycle == 0 {
                targets.len()
            } else {
                targets.len() - CAP
            };
            assert_eq!(spawned, want, "cycle {cycle}");
        }
        assert_eq!(pool.overflowed() as usize, 3 * (targets.len() - CAP));
        // Every forward idle past the reap age is killed.
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(pool.reap_idle(Duration::from_millis(10)).await, CAP);
        assert!(pool.is_empty().await);
    }

    #[test]
    fn forward_port_parse() {
        assert_eq!(
            parse_forward_port("Forwarding from 127.0.0.1:54321 -> 9000"),
            Some(54321)
        );
        assert_eq!(
            parse_forward_port("Forwarding from [::1]:54321 -> 9000"),
            Some(54321)
        );
        assert_eq!(parse_forward_port("error: unable to forward"), None);
        assert_eq!(parse_forward_port(""), None);
    }

    fn gw(status: u16, body: &str) -> Result<GatewayResponse> {
        Ok(GatewayResponse {
            status,
            body: body.into(),
        })
    }

    #[test]
    fn an_app_error_proves_the_proxy_works() {
        // The live K1 failure: Spring's 404 JSON on /actuator/info.
        let spring = r#"{"timestamp":"2026-10-03T10:00:00Z","status":404,"error":"Not Found","path":"/actuator/info"}"#;
        assert_eq!(
            verdict_from_gateway(&gw(404, spring)),
            SniffVerdict::ProxyWorks
        );
        assert_eq!(
            verdict_from_gateway(&gw(500, "<html>boom</html>")),
            SniffVerdict::ProxyWorks
        );
        assert_eq!(
            verdict_from_gateway(&gw(200, "{}")),
            SniffVerdict::ProxyWorks
        );
    }

    #[test]
    fn only_an_api_server_refusal_selects_port_forward() {
        let status = |reason: &str, code: u16| {
            format!(
                r#"{{"kind":"Status","apiVersion":"v1","status":"Failure","reason":"{reason}","code":{code}}}"#
            )
        };
        assert_eq!(
            verdict_from_gateway(&gw(403, &status("Forbidden", 403))),
            SniffVerdict::ApiDenied
        );
        assert_eq!(
            verdict_from_gateway(&gw(401, &status("Unauthorized", 401))),
            SniffVerdict::ApiDenied
        );
        assert_eq!(
            verdict_from_gateway(&gw(404, &status("NotFound", 404))),
            SniffVerdict::Inconclusive,
            "pod gone"
        );
        assert_eq!(
            verdict_from_gateway(&gw(503, &status("ServiceUnavailable", 503))),
            SniffVerdict::Inconclusive,
            "nothing listening"
        );
        assert_eq!(
            verdict_from_gateway(&Err(Error::Upstream("connect: refused".into()))),
            SniffVerdict::ApiDenied
        );
        assert_eq!(
            verdict_from_gateway(&Err(Error::Upstream("gateway timeout".into()))),
            SniffVerdict::Inconclusive
        );
    }

    #[test]
    fn kubectl_raw_verdicts() {
        let ok: Result<()> = Ok(());
        assert_eq!(verdict_from_kubectl(&ok), SniffVerdict::ProxyWorks);
        let app404: Result<()> = Err(Error::NotFound(
            "Error from server (NotFound): the server could not find the requested resource".into(),
        ));
        assert_eq!(verdict_from_kubectl(&app404), SniffVerdict::ProxyWorks);
        let gone: Result<()> = Err(Error::NotFound(
            "Error from server (NotFound): pods \"x\" not found".into(),
        ));
        assert_eq!(verdict_from_kubectl(&gone), SniffVerdict::Inconclusive);
        let denied: Result<()> = Err(Error::Forbidden("cluster RBAC: forbidden".into()));
        assert_eq!(verdict_from_kubectl(&denied), SniffVerdict::ApiDenied);
    }

    #[test]
    fn decide_takes_the_first_decisive_verdict() {
        use SniffVerdict::*;
        assert_eq!(
            decide(&[Inconclusive, ProxyWorks]),
            (TransportUsed::Proxy, true)
        );
        assert_eq!(
            decide(&[Inconclusive, ApiDenied]),
            (TransportUsed::PortForward, true)
        );
        assert_eq!(
            decide(&[Inconclusive, Inconclusive, Inconclusive]),
            (TransportUsed::Proxy, false),
            "all inconclusive: stay cheap, don't cache"
        );
        assert_eq!(decide(&[]), (TransportUsed::Proxy, false));
    }

    #[test]
    fn sniff_sample_spreads_over_targets() {
        assert!(sniff_sample::<u32>(&[]).is_empty());
        assert_eq!(sniff_sample(&[1]), vec![1]);
        assert_eq!(sniff_sample(&[1, 2, 3]), vec![1, 2, 3]);
        assert_eq!(
            sniff_sample(&(0..150).collect::<Vec<_>>()),
            vec![0, 75, 149]
        );
    }

    #[test]
    fn proxy_path_shape() {
        let t = ScrapeTarget {
            namespace: "mscasino".into(),
            pod: "auditlog-1".into(),
            port: 9000,
        };
        assert_eq!(
            proxy_path(&t, "/actuator/info"),
            "/api/v1/namespaces/mscasino/pods/auditlog-1:9000/proxy/actuator/info"
        );
    }

    #[test]
    fn grouping_uses_default_port_and_reports_unresolved() {
        let mk = |name: &str, port: Option<u16>| Probe {
            name: name.into(),
            port,
            path: "/m".into(),
            format: ProbeFormat::Health,
            mappings: vec![],
            include: vec![],
            exclude: vec![],
            timeout_ms: 1000,
        };
        let (g, un) = group_by_port(
            &[mk("a", Some(9000)), mk("b", None), mk("c", Some(8080))],
            Some(9000),
        );
        assert_eq!(g[&9000].len(), 2);
        assert_eq!(g[&8080].len(), 1);
        assert!(un.is_empty());
        let (g, un) = group_by_port(&[mk("b", None)], None);
        assert!(g.is_empty());
        assert_eq!(un, vec!["b".to_string()]);
    }
}
