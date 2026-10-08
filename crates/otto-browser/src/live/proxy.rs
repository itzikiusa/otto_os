//! A loopback SOCKS5 proxy every daemon Chromium is pointed at
//! (`--proxy-server=socks5://127.0.0.1:<port>` + `--proxy-bypass-list=<-loopback>`),
//! so each TCP connection the browser makes — including the ones CDP `Fetch`
//! interception can't pause (WebSocket, WebTransport-over-TCP, preconnects) —
//! is dialled by the daemon through `otto-netguard`: the host is resolved ONCE,
//! every address vetted (loopback / private / link-local / metadata refused),
//! and the connection pinned to a vetted address. That also closes the DNS
//! rebinding window of the `Fetch` check (Chromium sends the hostname —
//! SOCKS5 remote resolution — and never resolves it itself).
//!
//! CONNECT only (no BIND / UDP ASSOCIATE — QUIC falls back to TCP through a
//! SOCKS proxy), no auth methods (loopback listener; a local process gains
//! nothing it couldn't dial directly — every target is still netguarded).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

const VER: u8 = 5;
const CMD_CONNECT: u8 = 1;
const REP_OK: u8 = 0;
const REP_NOT_ALLOWED: u8 = 2;
const REP_HOST_UNREACHABLE: u8 = 4;
const REP_CMD_UNSUPPORTED: u8 = 7;
const REP_ATYP_UNSUPPORTED: u8 = 8;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 128;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// The target of one CONNECT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Ip(IpAddr, u16),
    Domain(String, u16),
}

/// Running proxy; dropping it stops acceptance and all accepted socket tasks.
pub struct GuardProxy {
    port: u16,
    task: JoinHandle<()>,
}

impl GuardProxy {
    /// Bind `127.0.0.1:0` and start accepting.
    pub async fn start() -> std::io::Result<Self> {
        Self::start_inner(
            #[cfg(test)]
            None,
        )
        .await
    }

    /// Test-only socket substitution after the unchanged production vetting.
    /// Unmapped targets are refused, so a fixture never dials an external host.
    #[cfg(test)]
    pub(super) async fn start_fixture(
        public: SocketAddr,
        local: SocketAddr,
    ) -> std::io::Result<Self> {
        assert!(local.ip().is_loopback());
        Self::start_inner(Some((public, local))).await
    }

    async fn start_inner(
        #[cfg(test)] fixture: Option<(SocketAddr, SocketAddr)>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let task = tokio::spawn(async move {
            // Dropping the accept task also aborts every accepted socket task.
            let mut connections = JoinSet::new();
            loop {
                let accepted = tokio::select! {
                    biased;
                    Some(_) = connections.join_next(), if !connections.is_empty() => continue,
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, peer)) = accepted else {
                    // EMFILE & co: back off instead of spinning.
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                };
                // Loopback listener; refuse anything else defensively.
                if !peer.ip().is_loopback() || connections.len() >= MAX_CONNECTIONS {
                    continue;
                }
                connections.spawn(async move {
                    let _ = serve(
                        stream,
                        #[cfg(test)]
                        fixture,
                    )
                    .await;
                });
            }
        });
        Ok(Self { port, task })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The Chromium flags that route every connection through this proxy
    /// (including loopback, which Chromium would otherwise bypass).
    pub fn chrome_args(&self) -> Vec<String> {
        vec![
            format!("--proxy-server=socks5://127.0.0.1:{}", self.port),
            "--proxy-bypass-list=<-loopback>".to_string(),
        ]
    }
}

impl Drop for GuardProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(
    mut client: TcpStream,
    #[cfg(test)] fixture: Option<(SocketAddr, SocketAddr)>,
) -> std::io::Result<()> {
    let _ = client.set_nodelay(true);
    let target = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut client)).await {
        Ok(Ok(t)) => t,
        _ => return Ok(()),
    };
    let addrs = match tokio::time::timeout(CONNECT_TIMEOUT, vet(&target)).await {
        Ok(Ok(a)) => a,
        _ => {
            reply(&mut client, REP_NOT_ALLOWED).await?;
            return Ok(());
        }
    };
    let mut upstream = None;
    for a in addrs {
        #[cfg(test)]
        let a = match fixture {
            Some((public, local)) if a == public => local,
            Some(_) => continue,
            None => a,
        };
        if let Ok(Ok(s)) = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(a)).await {
            upstream = Some(s);
            break;
        }
    }
    let Some(mut upstream) = upstream else {
        reply(&mut client, REP_HOST_UNREACHABLE).await?;
        return Ok(());
    };
    let _ = upstream.set_nodelay(true);
    reply(&mut client, REP_OK).await?;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

/// Resolve + vet (pinned addresses) — the netguard rule, applied at dial time.
pub async fn vet(target: &Target) -> Result<Vec<SocketAddr>, String> {
    match target {
        Target::Ip(ip, port) => {
            if otto_netguard::is_blocked_ip(*ip) {
                Err(format!("blocked address {ip}"))
            } else {
                Ok(vec![SocketAddr::new(*ip, *port)])
            }
        }
        Target::Domain(host, port) => otto_netguard::resolve_host_checked(host, *port).await,
    }
}

/// Method negotiation + the CONNECT request. Unsupported commands / address
/// types are answered here and surface as an error.
pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> std::io::Result<Target> {
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await?;
    if head[0] != VER {
        return Err(std::io::Error::other("not socks5"));
    }
    let mut methods = vec![0u8; head[1] as usize];
    s.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        s.write_all(&[VER, 0xFF]).await?;
        return Err(std::io::Error::other("no acceptable auth method"));
    }
    s.write_all(&[VER, 0]).await?;

    let mut req = [0u8; 4];
    s.read_exact(&mut req).await?;
    if req[0] != VER {
        return Err(std::io::Error::other("bad request version"));
    }
    if req[1] != CMD_CONNECT {
        reply(s, REP_CMD_UNSUPPORTED).await?;
        return Err(std::io::Error::other("only CONNECT is supported"));
    }
    let target = match req[3] {
        1 => {
            let mut b = [0u8; 4];
            s.read_exact(&mut b).await?;
            Target::Ip(IpAddr::V4(Ipv4Addr::from(b)), read_port(s).await?)
        }
        4 => {
            let mut b = [0u8; 16];
            s.read_exact(&mut b).await?;
            Target::Ip(IpAddr::V6(Ipv6Addr::from(b)), read_port(s).await?)
        }
        3 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize];
            s.read_exact(&mut name).await?;
            let port = read_port(s).await?;
            let host =
                String::from_utf8(name).map_err(|_| std::io::Error::other("non-utf8 host"))?;
            // A literal inside the domain field is vetted as an IP.
            match host
                .trim_matches(|c| c == '[' || c == ']')
                .parse::<IpAddr>()
            {
                Ok(ip) => Target::Ip(ip, port),
                Err(_) => Target::Domain(host, port),
            }
        }
        _ => {
            reply(s, REP_ATYP_UNSUPPORTED).await?;
            return Err(std::io::Error::other("unsupported address type"));
        }
    };
    Ok(target)
}

async fn read_port<S: AsyncRead + Unpin>(s: &mut S) -> std::io::Result<u16> {
    let mut p = [0u8; 2];
    s.read_exact(&mut p).await?;
    Ok(u16::from_be_bytes(p))
}

async fn reply<S: AsyncWrite + Unpin>(s: &mut S, code: u8) -> std::io::Result<()> {
    s.write_all(&[VER, code, 0, 1, 0, 0, 0, 0, 0, 0]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn run_handshake(request: Vec<u8>) -> (std::io::Result<Target>, Vec<u8>) {
        let (mut client, mut server) = tokio::io::duplex(1024);
        client.write_all(&request).await.unwrap();
        let r = handshake(&mut server).await;
        drop(server);
        let mut answered = Vec::new();
        let _ = client.read_to_end(&mut answered).await;
        (r, answered)
    }

    #[tokio::test]
    async fn parses_domain_and_ip_connects() {
        let mut req = vec![5, 1, 0, 5, 1, 0, 3, 11];
        req.extend_from_slice(b"example.com");
        req.extend_from_slice(&443u16.to_be_bytes());
        let (t, answered) = run_handshake(req).await;
        assert_eq!(t.unwrap(), Target::Domain("example.com".into(), 443));
        assert_eq!(answered, vec![5, 0]);

        let mut req = vec![5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1];
        req.extend_from_slice(&80u16.to_be_bytes());
        let (t, _) = run_handshake(req).await;
        assert_eq!(t.unwrap(), Target::Ip(IpAddr::V4(Ipv4Addr::LOCALHOST), 80));

        // An IP literal smuggled in the domain field is still an IP.
        let mut req = vec![5, 1, 0, 5, 1, 0, 3, 15];
        req.extend_from_slice(b"169.254.169.254");
        req.extend_from_slice(&80u16.to_be_bytes());
        let (t, _) = run_handshake(req).await;
        assert!(matches!(t.unwrap(), Target::Ip(_, 80)));
    }

    #[tokio::test]
    async fn refuses_bind_udp_and_auth_only_clients() {
        let (t, answered) = run_handshake(vec![5, 1, 0, 5, 2, 0, 1, 1, 1, 1, 1, 0, 80]).await;
        assert!(t.is_err());
        assert_eq!(answered[2..4], [5, REP_CMD_UNSUPPORTED]);
        let (t, answered) = run_handshake(vec![5, 1, 2]).await;
        assert!(t.is_err());
        assert_eq!(answered, vec![5, 0xFF]);
    }

    #[tokio::test]
    async fn vetting_blocks_internal_targets() {
        assert!(vet(&Target::Ip(IpAddr::V4(Ipv4Addr::LOCALHOST), 7700))
            .await
            .is_err());
        assert!(vet(&Target::Ip("169.254.169.254".parse().unwrap(), 80))
            .await
            .is_err());
        assert!(vet(&Target::Ip("10.1.2.3".parse().unwrap(), 443))
            .await
            .is_err());
        assert!(vet(&Target::Ip("::1".parse().unwrap(), 443)).await.is_err());
        assert!(vet(&Target::Domain("localhost".into(), 7700))
            .await
            .is_err());
        assert!(vet(&Target::Ip("8.8.8.8".parse().unwrap(), 443))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn the_proxy_refuses_a_loopback_connect_end_to_end() {
        let proxy = GuardProxy::start().await.unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        let mut req = vec![5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1];
        req.extend_from_slice(&7700u16.to_be_bytes());
        s.write_all(&req).await.unwrap();
        let mut resp = [0u8; 12];
        s.read_exact(&mut resp).await.unwrap();
        assert_eq!(&resp[..2], &[5, 0]);
        assert_eq!(resp[3], REP_NOT_ALLOWED);
        assert!(proxy.chrome_args()[0].contains(&proxy.port().to_string()));
    }
    #[tokio::test]
    async fn fixture_mapping_never_bypasses_vetting_or_dials_other_targets() {
        let local = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        // Even an exact fixture mapping cannot make a blocked address pass.
        for mapped in ["127.0.0.1:80", "8.8.8.8:80"] {
            let proxy =
                GuardProxy::start_fixture(mapped.parse().unwrap(), local.local_addr().unwrap())
                    .await
                    .unwrap();
            let mut client = TcpStream::connect(("127.0.0.1", proxy.port()))
                .await
                .unwrap();
            let mut request = vec![5, 1, 0, 5, 1, 0, 1];
            let (ip, expected) = if mapped.starts_with("127") {
                ([127, 0, 0, 1], REP_NOT_ALLOWED)
            } else {
                ([1, 1, 1, 1], REP_HOST_UNREACHABLE)
            };
            request.extend_from_slice(&ip);
            request.extend_from_slice(&80u16.to_be_bytes());
            client.write_all(&request).await.unwrap();
            let mut response = [0; 12];
            tokio::time::timeout(Duration::from_secs(1), client.read_exact(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(response[3], expected);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), local.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn dropping_proxy_cancels_accepted_handshakes() {
        let proxy = GuardProxy::start().await.unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut accepted = [0; 2];
        client.read_exact(&mut accepted).await.unwrap();
        assert_eq!(accepted, [5, 0]);
        drop(proxy);
        let mut byte = [0];
        let result = tokio::time::timeout(Duration::from_secs(1), client.read(&mut byte)).await;
        assert!(
            matches!(result, Ok(Ok(0)) | Ok(Err(_))),
            "accepted sockets must close with their proxy owner: {result:?}"
        );
    }
    #[tokio::test]
    async fn repeated_connection_saturation_rejects_excess_and_reclaims_slots() {
        let started = std::time::Instant::now();
        let proxy = GuardProxy::start().await.unwrap();
        for _ in 0..8 {
            let mut held = Vec::new();
            for _ in 0..MAX_CONNECTIONS {
                // A previous round's task cleanup may still be scheduled.
                let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
                loop {
                    let mut client = TcpStream::connect(("127.0.0.1", proxy.port()))
                        .await
                        .unwrap();
                    let mut response = [0; 2];
                    if client.write_all(&[5, 1, 0]).await.is_ok()
                        && client.read_exact(&mut response).await.is_ok()
                    {
                        assert_eq!(response, [5, 0]);
                        held.push(client);
                        break;
                    }
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "released sockets did not reclaim capacity"
                    );
                    tokio::task::yield_now().await;
                }
            }
            let mut excess = TcpStream::connect(("127.0.0.1", proxy.port()))
                .await
                .unwrap();
            let mut byte = [0];
            let result = tokio::time::timeout(Duration::from_secs(1), excess.read(&mut byte)).await;
            assert!(
                matches!(result, Ok(Ok(0)) | Ok(Err(_))),
                "excess socket must be rejected: {result:?}"
            );
            drop(held);
        }
        println!(
            "8 proxy saturation/recovery rounds (1,024 admitted sockets) in {:?}",
            started.elapsed()
        );
    }
}
