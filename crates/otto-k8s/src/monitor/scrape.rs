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

use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

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

/// Decide the transport for this cycle. `Auto` tries the proxy once against
/// `sample`; a 2xx (kubectl exit 0) picks proxy, anything else port-forward.
pub async fn pick_transport(
    k: &Kubectl,
    want: Transport,
    sample: &ScrapeTarget,
    path: &str,
) -> TransportUsed {
    match want {
        Transport::Proxy => TransportUsed::Proxy,
        Transport::PortForward => TransportUsed::PortForward,
        Transport::Auto => {
            let p = proxy_path(sample, path);
            match k
                .run_timeout(["get", "--raw", p.as_str()], Duration::from_secs(5))
                .await
            {
                Ok(_) => TransportUsed::Proxy,
                Err(e) => {
                    tracing::debug!("k8s monitor: proxy unavailable ({e}); using port-forward");
                    TransportUsed::PortForward
                }
            }
        }
    }
}

/// Fetch every probe for one pod/port. One result per probe, in order.
pub async fn fetch(
    k: &Kubectl,
    t: TransportUsed,
    target: &ScrapeTarget,
    probes: &[Probe],
) -> Vec<Result<ProbeResult>> {
    match t {
        TransportUsed::Proxy => {
            let mut out = Vec::with_capacity(probes.len());
            for p in probes {
                out.push(fetch_proxy(k, target, p).await);
            }
            out
        }
        TransportUsed::PortForward => match fetch_forward(k, target, probes).await {
            Ok(v) => v,
            Err(e) => probes
                .iter()
                .map(|_| Err(Error::Upstream(e.to_string())))
                .collect(),
        },
    }
}

async fn fetch_proxy(k: &Kubectl, target: &ScrapeTarget, p: &Probe) -> Result<ProbeResult> {
    let started = Instant::now();
    let path = proxy_path(target, &p.path);
    let out = k
        .run_timeout(
            ["get", "--raw", path.as_str()],
            Duration::from_millis(p.timeout_ms.max(1000) + 2000),
        )
        .await?;
    let mut body = out.stdout;
    if body.len() > MAX_BODY {
        body.truncate(MAX_BODY);
    }
    Ok(ProbeResult {
        probe: p.name.clone(),
        status: 200,
        body,
        ms: started.elapsed().as_millis() as u64,
    })
}

/// Spawn `kubectl port-forward pod/<pod> 0:<port>` and return the child +
/// the local port it bound.
async fn spawn_forward(k: &Kubectl, target: &ScrapeTarget) -> Result<(tokio::process::Child, u16)> {
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
        Ok(Some(p)) => Ok((child, p)),
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
    let (mut child, local) = spawn_forward(k, target).await?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| Error::Internal(format!("http client: {e}")))?;
    let mut out = Vec::with_capacity(probes.len());
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
            Err(e) => Err(Error::Upstream(format!(
                "{}: {}",
                p.name,
                if e.is_timeout() { "timeout".to_string() } else { e.to_string() }
            ))),
        };
        out.push(r);
    }
    let _ = child.kill().await;
    Ok(out)
}

/// Group probes by the port they hit (a probe without a port uses
/// `default_port`, the container's first declared port). Probes with no
/// resolvable port are returned separately so the caller can count them as
/// failed.
pub fn group_by_port(probes: &[Probe], default_port: Option<u16>) -> (BTreeMap<u16, Vec<Probe>>, Vec<String>) {
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

    #[test]
    fn forward_port_parse() {
        assert_eq!(parse_forward_port("Forwarding from 127.0.0.1:54321 -> 9000"), Some(54321));
        assert_eq!(parse_forward_port("Forwarding from [::1]:54321 -> 9000"), Some(54321));
        assert_eq!(parse_forward_port("error: unable to forward"), None);
        assert_eq!(parse_forward_port(""), None);
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
        let (g, un) = group_by_port(&[mk("a", Some(9000)), mk("b", None), mk("c", Some(8080))], Some(9000));
        assert_eq!(g[&9000].len(), 2);
        assert_eq!(g[&8080].len(), 1);
        assert!(un.is_empty());
        let (g, un) = group_by_port(&[mk("b", None)], None);
        assert!(g.is_empty());
        assert_eq!(un, vec!["b".to_string()]);
    }
}
