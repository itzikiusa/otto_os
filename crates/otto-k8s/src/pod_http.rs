//! Pod HTTP actions (K-3) — `POST /k8s/clusters/{id}/pod-http`: send one HTTP
//! request to a pod's container port (e.g. Spring Boot actuator
//! `POST /actuator/loggers/<logger>`), or fan it out to every pod of a
//! workload with one result per pod.
//!
//! Transport, cheapest first:
//! * **proxy** — the API server's `pods/{pod}:{port}/proxy{path}` through a
//!   pooled long-lived `kubectl proxy` per cluster (the monitor's gateway,
//!   started here with `allow_mutating` so POST/PUT/PATCH/DELETE pass; still
//!   pod-proxy paths only, still a private Unix socket). Needs `pods/proxy`.
//! * **port_forward** — when the proxy cannot start, fails at transport level
//!   or the API server refuses `pods/proxy` (RBAC): `kubectl port-forward
//!   pod/<pod> 0:<port>` + a plain loopback request. Needs `pods/portforward`.
//!
//! Guard rails: the path must be a plain absolute path (no `..`, no scheme, no
//! whitespace, no unfilled `{{template}}`), at most [`MAX_PODS`] pods, bodies
//! capped at [`MAX_BODY`], credentials headers redacted in what we return, and
//! a mutating method on a prod cluster needs `confirm_name == target name`
//! (checked by the route, see [`needs_confirm`]).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use futures_util::StreamExt;
use otto_core::domain::Environment;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::cli::Kubectl;
use crate::monitor::gateway::KubeProxy;
use crate::monitor::scrape::{spawn_forward, ScrapeTarget};
use crate::resources::Kind;

/// Response body cap per pod (the rest is cut, `truncated: true`).
pub const MAX_BODY: usize = 256 * 1024;
/// Request body cap.
pub const MAX_REQUEST_BODY: usize = 1024 * 1024;
/// Pods one workload run may fan out to.
pub const MAX_PODS: usize = 50;
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
pub const MAX_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_CONCURRENCY: usize = 4;
pub const MAX_CONCURRENCY: usize = 8;
const MAX_HEADERS: usize = 32;
/// A pooled mutating proxy idle this long is stopped on the next run.
const GATEWAY_IDLE: Duration = Duration::from_secs(600);

/// `POST …/pod-http` body.
#[derive(Debug, Clone, Deserialize)]
pub struct PodHttpReq {
    pub namespace: String,
    #[serde(default)]
    pub pod: Option<String>,
    #[serde(default)]
    pub workload: Option<WorkloadRef>,
    pub port: u32,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_concurrency: Option<usize>,
    #[serde(default)]
    pub confirm_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkloadRef {
    pub kind: String,
    pub name: String,
}

/// One pod's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct PodHttpResult {
    pub pod: String,
    pub status: Option<u16>,
    pub duration_ms: u64,
    pub headers: BTreeMap<String, String>,
    pub body: String,
    pub body_base64: bool,
    pub truncated: bool,
    pub error: Option<String>,
    pub via: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PodHttpResp {
    pub results: Vec<PodHttpResult>,
    pub target_name: String,
    pub mutating: bool,
}

/// What a validated request targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Pod(String),
    Workload(Kind, String),
}

/// A request that passed [`validate`].
#[derive(Debug, Clone)]
pub struct Validated {
    pub namespace: String,
    pub target: Target,
    pub port: u16,
    pub method: hyper::Method,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub timeout: Duration,
    pub concurrency: usize,
}

impl Validated {
    pub fn mutating(&self) -> bool {
        self.method != hyper::Method::GET
    }

    /// Pod name, or workload name — what `confirm_name` must equal.
    pub fn target_name(&self) -> &str {
        match &self.target {
            Target::Pod(n) | Target::Workload(_, n) => n,
        }
    }
}

/// `GET|POST|PUT|PATCH|DELETE` (case-insensitive) → method.
pub fn parse_method(m: &str) -> Result<hyper::Method> {
    Ok(match m.trim().to_ascii_uppercase().as_str() {
        "GET" => hyper::Method::GET,
        "POST" => hyper::Method::POST,
        "PUT" => hyper::Method::PUT,
        "PATCH" => hyper::Method::PATCH,
        "DELETE" => hyper::Method::DELETE,
        _ => {
            return Err(Error::Invalid(
                "method must be GET, POST, PUT, PATCH or DELETE".into(),
            ))
        }
    })
}

/// A plain absolute request path (+ optional query): starts with `/`, no
/// `..` segment, no scheme, no whitespace/control chars, no `#`, no unfilled
/// `{{template}}` variable.
pub fn validate_path(p: &str) -> Result<()> {
    let bad = |why: &str| Err(Error::Invalid(format!("invalid path: {why}")));
    if !p.starts_with('/') {
        return bad("must start with /");
    }
    if p.len() > 2048 {
        return bad("too long");
    }
    if p.contains("://") || p.starts_with("//") {
        return bad("must not carry a scheme or host");
    }
    if p.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return bad("must not contain whitespace or control characters");
    }
    if p.contains('#') {
        return bad("must not contain a fragment");
    }
    if p.contains("{{") || p.contains("}}") {
        return bad("fill in the template variables first");
    }
    let path_only = p.split('?').next().unwrap_or(p);
    if path_only
        .split('/')
        .any(|seg| seg == ".." || seg.eq_ignore_ascii_case("%2e%2e"))
        || path_only.to_ascii_lowercase().contains("%2e%2e")
        || path_only.to_ascii_lowercase().contains("%2f")
    {
        return bad("must not contain .. or encoded slashes");
    }
    Ok(())
}

/// RFC 7230 token header name, sane value, and no hop-by-hop / host headers.
pub fn validate_headers(h: &BTreeMap<String, String>) -> Result<Vec<(String, String)>> {
    if h.len() > MAX_HEADERS {
        return Err(Error::Invalid(format!("at most {MAX_HEADERS} headers")));
    }
    let mut out = Vec::with_capacity(h.len());
    for (k, v) in h {
        let k = k.trim();
        let token = !k.is_empty()
            && k.len() <= 128
            && k.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
        if !token {
            return Err(Error::Invalid(format!("invalid header name '{k}'")));
        }
        if matches!(
            k.to_ascii_lowercase().as_str(),
            "host" | "content-length" | "transfer-encoding" | "connection" | "upgrade" | "te"
        ) {
            return Err(Error::Invalid(format!("header '{k}' cannot be set")));
        }
        if v.len() > 8192 || v.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
            return Err(Error::Invalid(format!("invalid value for header '{k}'")));
        }
        out.push((k.to_string(), v.clone()));
    }
    Ok(out)
}

/// DNS-1123 subdomain (pod / workload names).
fn valid_object_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 253
        && n.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && !n.starts_with(['-', '.'])
        && !n.ends_with(['-', '.'])
}

/// Workload kinds a run (or a saved action) can target.
pub fn workload_kind(kind: &str) -> Result<Kind> {
    match Kind::parse(kind) {
        Some(
            k @ (Kind::Deployments
            | Kind::Statefulsets
            | Kind::Daemonsets
            | Kind::Replicasets
            | Kind::Jobs),
        ) => Ok(k),
        _ => Err(Error::Invalid(
            "workload kind must be deployment, statefulset, daemonset, replicaset or job".into(),
        )),
    }
}

/// Validate and normalise a request (pure).
pub fn validate(req: &PodHttpReq) -> Result<Validated> {
    let namespace = req.namespace.trim().to_string();
    if crate::access::namespace(Some(namespace.as_str()))?.is_none() {
        return Err(Error::Invalid("namespace is required".into()));
    }
    let target = match (&req.pod, &req.workload) {
        (Some(p), None) => {
            let p = p.trim();
            if !valid_object_name(p) {
                return Err(Error::Invalid("invalid pod name".into()));
            }
            Target::Pod(p.to_string())
        }
        (None, Some(w)) => {
            let kind = workload_kind(&w.kind)?;
            let n = w.name.trim();
            if !valid_object_name(n) {
                return Err(Error::Invalid("invalid workload name".into()));
            }
            Target::Workload(kind, n.to_string())
        }
        _ => return Err(Error::Invalid("set exactly one of pod or workload".into())),
    };
    let port = u16::try_from(req.port)
        .ok()
        .filter(|p| *p > 0)
        .ok_or_else(|| Error::Invalid("port must be 1-65535".into()))?;
    let method = parse_method(&req.method)?;
    let path = req.path.trim().to_string();
    validate_path(&path)?;
    let headers = validate_headers(&req.headers.clone().unwrap_or_default())?;
    let body = match &req.body {
        Some(b) if b.len() > MAX_REQUEST_BODY => {
            return Err(Error::PayloadTooLarge("request body over 1 MiB".into()))
        }
        Some(b) if method == hyper::Method::GET && !b.is_empty() => {
            return Err(Error::Invalid("a GET request has no body".into()))
        }
        Some(b) if !b.is_empty() => Some(b.clone().into_bytes()),
        _ => None,
    };
    Ok(Validated {
        namespace,
        target,
        port,
        method,
        path,
        headers,
        body,
        timeout: Duration::from_millis(
            req.timeout_ms
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .clamp(500, MAX_TIMEOUT_MS),
        ),
        concurrency: req
            .max_concurrency
            .unwrap_or(DEFAULT_CONCURRENCY)
            .clamp(1, MAX_CONCURRENCY),
    })
}

/// Prod guard: a mutating call on a prod cluster needs the typed target name.
pub fn needs_confirm(
    env: Environment,
    mutating: bool,
    confirm_name: Option<&str>,
    target_name: &str,
) -> bool {
    env == Environment::Prod && mutating && confirm_name.map(str::trim) != Some(target_name)
}

/// Headers whose values never leave the daemon (responses + audit).
pub fn is_secret_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "x-api-key"
    )
}

/// Copy of `h` with credential headers replaced by `[redacted]`.
pub fn redact_headers<'a>(
    h: impl IntoIterator<Item = (&'a String, &'a String)>,
) -> BTreeMap<String, String> {
    h.into_iter()
        .map(|(k, v)| {
            let v = if is_secret_header(k) {
                "[redacted]".to_string()
            } else {
                v.clone()
            };
            (k.clone(), v)
        })
        .collect()
}

/// Body bytes → `(text, is_base64, truncated)`, capped at [`MAX_BODY`].
pub fn encode_body(mut bytes: Vec<u8>, already_truncated: bool) -> (String, bool, bool) {
    let mut truncated = already_truncated;
    if bytes.len() > MAX_BODY {
        bytes.truncate(MAX_BODY);
        truncated = true;
    }
    match String::from_utf8(bytes) {
        Ok(s) => (s, false, truncated),
        Err(e) => {
            let bytes = e.into_bytes();
            // A cut in the middle of a UTF-8 scalar is still text.
            if truncated {
                if let Err(err) = std::str::from_utf8(&bytes) {
                    if err.error_len().is_none() {
                        let ok = err.valid_up_to();
                        return (
                            String::from_utf8_lossy(&bytes[..ok]).into_owned(),
                            false,
                            true,
                        );
                    }
                }
            }
            (B64.encode(&bytes), true, truncated)
        }
    }
}

/// Hex sha256 of the request body (audited instead of the body).
pub fn body_sha256(body: Option<&[u8]>) -> Option<String> {
    body.map(|b| hex::encode(Sha256::digest(b)))
}

/// The pods behind a target: the named pod, or the workload's running pods
/// (label selector from `spec.selector.matchLabels`), name-sorted, capped.
pub async fn resolve_pods(k: &Kubectl, ns: &str, target: &Target) -> Result<Vec<String>> {
    let (kind, name) = match target {
        Target::Pod(p) => return Ok(vec![p.clone()]),
        Target::Workload(kind, name) => (*kind, name),
    };
    let object = format!("{}/{}", kind.kubectl_resource(), name);
    let item = k
        .json(["get", object.as_str(), "-n", ns, "-o", "json"])
        .await?;
    let sel = crate::resources::label_selector(&item)
        .ok_or_else(|| Error::Invalid(format!("{} {name} has no label selector", kind.as_str())))?;
    let pods = k
        .json(["get", "pods", "-n", ns, "-l", sel.as_str(), "-o", "json"])
        .await?;
    let mut names = running_pod_names(&pods);
    names.sort();
    if names.is_empty() {
        return Err(Error::NotFound(format!(
            "no running pods for {} {name}",
            kind.as_str()
        )));
    }
    if names.len() > MAX_PODS {
        return Err(Error::Invalid(format!(
            "{} {name} has {} running pods; at most {MAX_PODS} per run",
            kind.as_str(),
            names.len()
        )));
    }
    Ok(names)
}

/// Names of `Running`, not-terminating pods in a `get pods -o json` list.
pub fn running_pod_names(list: &Value) -> Vec<String> {
    list.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.pointer("/status/phase").and_then(Value::as_str) == Some("Running"))
        .filter(|p| p.pointer("/metadata/deletionTimestamp").is_none())
        .filter_map(|p| p.pointer("/metadata/name").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

type Pool = tokio::sync::Mutex<HashMap<String, (Arc<KubeProxy>, Instant)>>;
static GATEWAYS: LazyLock<Pool> = LazyLock::new(Default::default);

/// The cluster's pooled mutating-capable `kubectl proxy` (started on demand,
/// restarted when the credentials changed). `None` ⇒ use port-forward.
async fn gateway(cluster_id: &str, k: &Kubectl) -> Option<Arc<KubeProxy>> {
    let mut pool = GATEWAYS.lock().await;
    pool.retain(|_, (_, used)| used.elapsed() < GATEWAY_IDLE);
    if let Some((gw, used)) = pool.get_mut(cluster_id) {
        if gw.usable_for(k) {
            *used = Instant::now();
            return Some(gw.clone());
        }
    }
    pool.remove(cluster_id);
    match KubeProxy::start_with(k, true).await {
        Ok(gw) => {
            let gw = Arc::new(gw);
            pool.insert(cluster_id.to_string(), (gw.clone(), Instant::now()));
            Some(gw)
        }
        Err(e) => {
            tracing::debug!("k8s pod-http: kubectl proxy unavailable ({e}); port-forward");
            None
        }
    }
}

/// Drop the pooled gateway of a cluster (it misbehaved).
async fn drop_gateway(cluster_id: &str) {
    GATEWAYS.lock().await.remove(cluster_id);
}

/// Did the API server refuse the pod proxy itself (RBAC / not proxyable),
/// rather than the app answering? Then port-forward may still work.
pub fn proxy_refused(status: u16, body: &[u8]) -> bool {
    if !matches!(status, 403 | 405) {
        return false;
    }
    let s = String::from_utf8_lossy(&body[..body.len().min(4096)]);
    s.contains("\"kind\":\"Status\"") && (s.contains("pods/proxy") || s.contains("proxy"))
}

/// Run a validated request against its pods (already authorised + confirmed).
pub async fn run(k: &Kubectl, cluster_id: &str, v: &Validated) -> Result<PodHttpResp> {
    let pods = resolve_pods(k, &v.namespace, &v.target).await?;
    let gw = gateway(cluster_id, k).await;
    let results: Vec<PodHttpResult> = futures_util::stream::iter(pods)
        .map(|pod| {
            let gw = gw.clone();
            async move { one_pod(k, gw.as_deref(), v, pod).await }
        })
        .buffered(v.concurrency)
        .collect()
        .await;
    if gw.is_some()
        && results
            .iter()
            .all(|r| r.via == "port_forward" && r.error.is_none())
    {
        // Every pod needed the fallback: the proxy is no use for this cluster.
        drop_gateway(cluster_id).await;
    }
    Ok(PodHttpResp {
        results,
        target_name: v.target_name().to_string(),
        mutating: v.mutating(),
    })
}

async fn one_pod(k: &Kubectl, gw: Option<&KubeProxy>, v: &Validated, pod: String) -> PodHttpResult {
    let started = Instant::now();
    if let Some(gw) = gw {
        let path = format!(
            "/api/v1/namespaces/{}/pods/{}:{}/proxy{}",
            v.namespace, pod, v.port, v.path
        );
        match gw
            .request(
                v.method.clone(),
                &path,
                &v.headers,
                v.body.clone(),
                v.timeout,
                MAX_BODY,
            )
            .await
        {
            Ok(r) if !proxy_refused(r.status, &r.body) => {
                let headers: BTreeMap<String, String> = r.headers.into_iter().collect();
                let (body, body_base64, truncated) = encode_body(r.body, r.truncated);
                return PodHttpResult {
                    pod,
                    status: Some(r.status),
                    duration_ms: started.elapsed().as_millis() as u64,
                    headers: redact_headers(&headers),
                    body,
                    body_base64,
                    truncated,
                    error: None,
                    via: "proxy",
                };
            }
            Ok(_) => tracing::debug!("k8s pod-http: pod proxy refused; port-forward"),
            // A timeout is the app being slow, not the transport: retrying via
            // port-forward would double a (possibly mutating) call.
            Err(e) if e.to_string().contains("timeout") => {
                return failed(pod, started, "proxy", e.to_string())
            }
            Err(e) => tracing::debug!("k8s pod-http: proxy transport failed ({e}); port-forward"),
        }
    }
    match forward_request(k, v, &pod).await {
        Ok((status, headers, bytes, cut)) => {
            let (body, body_base64, truncated) = encode_body(bytes, cut);
            PodHttpResult {
                pod,
                status: Some(status),
                duration_ms: started.elapsed().as_millis() as u64,
                headers: redact_headers(&headers),
                body,
                body_base64,
                truncated,
                error: None,
                via: "port_forward",
            }
        }
        Err(e) => failed(pod, started, "port_forward", e.to_string()),
    }
}

fn failed(pod: String, started: Instant, via: &'static str, error: String) -> PodHttpResult {
    PodHttpResult {
        pod,
        status: None,
        duration_ms: started.elapsed().as_millis() as u64,
        headers: BTreeMap::new(),
        body: String::new(),
        body_base64: false,
        truncated: false,
        error: Some(crate::cli::redact(&error)),
        via,
    }
}

type ForwardResp = (u16, BTreeMap<String, String>, Vec<u8>, bool);

async fn forward_request(k: &Kubectl, v: &Validated, pod: &str) -> Result<ForwardResp> {
    let target = ScrapeTarget {
        namespace: v.namespace.clone(),
        pod: pod.to_string(),
        port: v.port,
    };
    let (mut child, local) = spawn_forward(k, &target).await?;
    let res = async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::Internal(format!("http client: {e}")))?;
        let method = reqwest::Method::from_bytes(v.method.as_str().as_bytes())
            .map_err(|e| Error::Invalid(format!("method: {e}")))?;
        let mut rb = client
            .request(method, format!("http://127.0.0.1:{local}{}", v.path))
            .timeout(v.timeout);
        for (name, val) in &v.headers {
            rb = rb.header(name.as_str(), val.as_str());
        }
        if let Some(b) = &v.body {
            rb = rb.body(b.clone());
        }
        let mut resp = rb.send().await.map_err(|e| {
            Error::Upstream(if e.is_timeout() {
                "timeout".into()
            } else {
                format!("request: {e}")
            })
        })?;
        let status = resp.status().as_u16();
        let headers: BTreeMap<String, String> = resp
            .headers()
            .iter()
            .map(|(a, b)| {
                (
                    a.as_str().to_string(),
                    String::from_utf8_lossy(b.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut buf = Vec::new();
        let mut cut = false;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| Error::Upstream(format!("read body: {e}")))?
        {
            let room = MAX_BODY.saturating_sub(buf.len());
            if chunk.len() > room {
                buf.extend_from_slice(&chunk[..room]);
                cut = true;
                break;
            }
            buf.extend_from_slice(&chunk);
        }
        Ok((status, headers, buf, cut))
    }
    .await;
    let _ = child.kill().await;
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req(v: Value) -> PodHttpReq {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn paths_are_plain_absolute_paths() {
        for ok in [
            "/actuator/loggers",
            "/actuator/loggers/com.acme.Foo",
            "/q?x=1&y=a%20b",
        ] {
            assert!(validate_path(ok).is_ok(), "{ok}");
        }
        for bad in [
            "actuator",
            "/a/../b",
            "/..",
            "http://evil/x",
            "//evil/x",
            "/a b",
            "/a\r\nX: y",
            "/a#frag",
            "/actuator/loggers/{{logger}}",
            "/a/%2e%2e/b",
            "/a%2Fb",
        ] {
            assert!(validate_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn validate_requires_exactly_one_target_and_sane_fields() {
        let base = json!({"namespace":"shop","port":8081,"method":"get","path":"/actuator/health"});
        let mut r = base.clone();
        r["pod"] = json!("api-1");
        let v = validate(&req(r)).unwrap();
        assert_eq!(v.target, Target::Pod("api-1".into()));
        assert!(!v.mutating());
        assert_eq!(v.timeout, Duration::from_millis(DEFAULT_TIMEOUT_MS));
        assert_eq!(v.concurrency, DEFAULT_CONCURRENCY);

        assert!(validate(&req(base.clone())).is_err(), "no target");
        let mut both = base.clone();
        both["pod"] = json!("a");
        both["workload"] = json!({"kind":"deployment","name":"api"});
        assert!(validate(&req(both)).is_err(), "both targets");

        let mut w = base.clone();
        w["workload"] = json!({"kind":"deploy","name":"api"});
        w["method"] = json!("POST");
        w["body"] = json!("{\"configuredLevel\":\"DEBUG\"}");
        w["timeout_ms"] = json!(999_999);
        w["max_concurrency"] = json!(99);
        let v = validate(&req(w)).unwrap();
        assert_eq!(v.target, Target::Workload(Kind::Deployments, "api".into()));
        assert!(v.mutating());
        assert_eq!(v.target_name(), "api");
        assert_eq!(v.timeout, Duration::from_millis(MAX_TIMEOUT_MS));
        assert_eq!(v.concurrency, MAX_CONCURRENCY);

        let mut cj = base.clone();
        cj["workload"] = json!({"kind":"cronjob","name":"x"});
        assert!(
            validate(&req(cj)).is_err(),
            "cronjob is not a pod owner here"
        );
        let mut port = base.clone();
        port["pod"] = json!("a");
        port["port"] = json!(70000);
        assert!(validate(&req(port)).is_err());
        let mut m = base.clone();
        m["pod"] = json!("a");
        m["method"] = json!("TRACE");
        assert!(validate(&req(m)).is_err());
        let mut gb = base.clone();
        gb["pod"] = json!("a");
        gb["body"] = json!("x");
        assert!(validate(&req(gb)).is_err(), "GET with a body");
        let mut ns = base;
        ns["pod"] = json!("a");
        ns["namespace"] = json!("Bad NS");
        assert!(validate(&req(ns)).is_err());
    }

    #[test]
    fn headers_are_validated() {
        let ok = BTreeMap::from([
            ("Accept".to_string(), "text/plain".to_string()),
            ("X-Trace".to_string(), "1".to_string()),
        ]);
        assert_eq!(validate_headers(&ok).unwrap().len(), 2);
        for (k, v) in [
            ("Bad Name", "x"),
            ("Host", "evil"),
            ("Content-Length", "1"),
            ("X-Ok", "a\r\nInjected: 1"),
        ] {
            let h = BTreeMap::from([(k.to_string(), v.to_string())]);
            assert!(validate_headers(&h).is_err(), "{k}");
        }
    }

    #[test]
    fn credential_headers_are_redacted() {
        let h = BTreeMap::from([
            ("Authorization".to_string(), "Bearer abc".to_string()),
            ("set-cookie".to_string(), "s=1".to_string()),
            ("Cookie".to_string(), "s=1".to_string()),
            ("content-type".to_string(), "application/json".to_string()),
        ]);
        let r = redact_headers(&h);
        assert_eq!(r["Authorization"], "[redacted]");
        assert_eq!(r["set-cookie"], "[redacted]");
        assert_eq!(r["Cookie"], "[redacted]");
        assert_eq!(r["content-type"], "application/json");
    }

    #[test]
    fn bodies_are_capped_and_binary_is_base64() {
        let (s, b64, cut) = encode_body(b"{\"ok\":true}".to_vec(), false);
        assert_eq!((s.as_str(), b64, cut), ("{\"ok\":true}", false, false));

        let (s, b64, cut) = encode_body(vec![b'a'; MAX_BODY + 10], false);
        assert_eq!((s.len(), b64, cut), (MAX_BODY, false, true));

        let (s, b64, cut) = encode_body(vec![0xff, 0xfe, 0x00], false);
        assert!(b64 && !cut);
        assert_eq!(B64.decode(s).unwrap(), vec![0xff, 0xfe, 0x00]);

        // Cut inside a multi-byte scalar: still text, minus the partial char.
        let mut v = vec![b'a'; MAX_BODY - 1];
        v.extend("é".as_bytes());
        let (s, b64, cut) = encode_body(v, false);
        assert!(!b64 && cut);
        assert_eq!(s.len(), MAX_BODY - 1);
    }

    #[test]
    fn prod_mutations_need_the_typed_name() {
        use Environment::*;
        assert!(needs_confirm(Prod, true, None, "api"));
        assert!(needs_confirm(Prod, true, Some("apx"), "api"));
        assert!(!needs_confirm(Prod, true, Some(" api "), "api"));
        assert!(
            !needs_confirm(Prod, false, None, "api"),
            "GET is never gated"
        );
        assert!(!needs_confirm(Staging, true, None, "api"));
        assert!(!needs_confirm(Dev, true, None, "api"));
    }

    #[test]
    fn running_pods_and_proxy_refusal() {
        let list = json!({"items":[
            {"metadata":{"name":"b"},"status":{"phase":"Running"}},
            {"metadata":{"name":"a"},"status":{"phase":"Pending"}},
            {"metadata":{"name":"c","deletionTimestamp":"x"},"status":{"phase":"Running"}},
            {"metadata":{"name":"d"},"status":{"phase":"Running"}}
        ]});
        assert_eq!(running_pod_names(&list), vec!["b", "d"]);
        let refused = br#"{"kind":"Status","apiVersion":"v1","status":"Failure","message":"pods \"x\" is forbidden: User \"u\" cannot create resource \"pods/proxy\""}"#;
        assert!(proxy_refused(403, refused));
        assert!(!proxy_refused(403, b"app says no"));
        assert!(!proxy_refused(200, refused));
        assert_eq!(
            body_sha256(Some(b"abc")).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(body_sha256(None).is_none());
    }
}
