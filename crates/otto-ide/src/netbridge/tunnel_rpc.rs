//! Unix control-socket RPC handler (`RESOLVE` / `CONNECT`) for the POSIX
//! `getaddrinfo` + `connect` interposer library.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::k8s_discover::{resolve_destination, should_intercept_dns};
use super::socks5::constant_time_eq;
use super::tunnel::NetBridgeInstance;
use super::tunnel_forward::{ensure_not_metadata_target, is_ssh_routable_private_ip};

impl NetBridgeInstance {
    fn check_token(&self, candidate: &str) -> bool {
        constant_time_eq(self.auth_token.as_bytes(), candidate.as_bytes())
    }

    pub(super) async fn handle_control_client(
        self: &Arc<Self>,
        stream: UnixStream,
    ) -> std::io::Result<()> {
        let (rd, mut wr) = stream.into_split();
        let mut reader = BufReader::new(rd);
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
            .await
            .map_err(std::io::Error::other)??;
        if n == 0 || line.len() > 1024 {
            return Ok(());
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["RESOLVE", token, host] => {
                if !self.check_token(token) {
                    wr.write_all(b"ERR unauthorized\n").await?;
                    return Ok(());
                }
                let snap = self.current_k8s_snapshot().await;
                if should_intercept_dns(host, &self.cfg, snap.as_deref(), self.ssh_cfg.is_some()) {
                    let vip = self.allocate_vip(host);
                    wr.write_all(format!("VIP {vip}\n").as_bytes()).await?;
                } else {
                    wr.write_all(b"MISS\n").await?;
                }
            }
            ["CONNECT", token, ip_str, port_str] => {
                if !self.check_token(token) {
                    wr.write_all(b"ERR unauthorized\n").await?;
                    return Ok(());
                }
                let Ok(port) = port_str.parse::<u16>() else {
                    wr.write_all(b"ERR bad_port\n").await?;
                    return Ok(());
                };
                match self.resolve_connect_port(ip_str, port).await {
                    Ok(Some(lport)) => {
                        wr.write_all(format!("PORT {lport}\n").as_bytes()).await?;
                    }
                    Ok(None) => {
                        wr.write_all(b"PASSTHROUGH\n").await?;
                    }
                    Err(e) => {
                        tracing::debug!(ip = ip_str, port, error = %e, "netbridge connect failed");
                        wr.write_all(b"ERR upstream\n").await?;
                    }
                }
            }
            _ => {
                wr.write_all(b"ERR syntax\n").await?;
            }
        }
        wr.flush().await
    }

    async fn resolve_connect_port(
        self: &Arc<Self>,
        ip_str: &str,
        req_port: u16,
    ) -> Result<Option<u16>> {
        let snap = self.current_k8s_snapshot().await;
        if let Ok(v4) = ip_str.parse::<Ipv4Addr>() {
            if let Some(host_entry) = self.vip_to_host.get(&v4) {
                let host = host_entry.value().clone();
                drop(host_entry);
                ensure_not_metadata_target(&host)?;
                let Some(dest) = resolve_destination(
                    &host,
                    req_port,
                    &self.cfg,
                    snap.as_deref(),
                    self.ssh_cfg.is_some(),
                ) else {
                    return Err(Error::NotFound(format!(
                        "no bridge route for {host}:{req_port}"
                    )));
                };
                return self.ensure_local_forward_port(dest).await.map(Some);
            }
        }

        let parsed_ip = ip_str
            .parse::<IpAddr>()
            .map_err(|_| Error::Invalid(format!("invalid connect IP '{ip_str}'")))?;
        if otto_netguard::is_metadata_ip(parsed_ip) || parsed_ip.is_loopback() {
            return Err(Error::Forbidden(format!(
                "blocked connect IP {parsed_ip} (SSRF guard)"
            )));
        }

        // Direct ClusterIP or RFC1918 IP connection (`OTTO_NETBRIDGE_PRIVATE_IPS=1`).
        if let Some(s) = snap.as_deref() {
            if let Some(svc_name) = s.cluster_ips.get(ip_str) {
                if let Some(entry) = s.services.get(svc_name) {
                    let port = entry.resolve_service_port(req_port);
                    return self
                        .ensure_k8s_port_forward(&s.namespace, &entry.name, port)
                        .await
                        .map(Some);
                }
            }
        }
        if self.ssh_cfg.is_some() && is_ssh_routable_private_ip(parsed_ip) {
            return self
                .ensure_ssh_local_relay(ip_str, req_port)
                .await
                .map(Some);
        }
        Ok(None)
    }
}
