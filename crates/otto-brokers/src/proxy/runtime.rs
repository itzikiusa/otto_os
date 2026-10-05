//! Async tunnel + Kafka-aware reverse proxy built on [`super::protocol`].
//!
//! One [`BrokerTunnel`] per cluster owns an `ssh -D` SOCKS5 tunnel and a set of
//! local TCP listeners (one per real broker, created on demand). librdkafka
//! connects to the local bootstrap listener in plaintext; each accepted
//! connection is forwarded to the real broker through SOCKS (remote DNS) with
//! optional broker-side TLS, while `Metadata`/`FindCoordinator` responses are
//! rewritten so every advertised address points back at a local listener.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::{Error, Result};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio_rustls::TlsConnector;
use tokio_socks::tcp::Socks5Stream;

use super::protocol::{
    self, is_flexible, parse_request_header, API_FIND_COORDINATOR, API_METADATA, API_VERSIONS,
    FIND_COORDINATOR_MAX, METADATA_MAX,
};

/// Refuse absurd frame lengths (guards against a corrupt/hostile length prefix).
const MAX_FRAME: usize = 256 * 1024 * 1024;
const DEFAULT_KAFKA_PORT: u16 = 9092;

/// Cap on concurrent in-flight upstream DIALS per tunnel. librdkafka
/// reconnects aggressively (admin + producer + consumer, each × N brokers) when
/// a broker is unreachable; without a cap every stuck upstream dial pins two
/// file descriptors and they pile up until the daemon hits its open-files limit
/// ("Too many open files"). Past the cap we shed the accepted connection
/// immediately — librdkafka simply retries once a slot frees. The permit is
/// released as soon as the upstream is established: long-lived broker
/// connections (a big cluster × producer/consumer/admin clients) must not
/// count against it, or a healthy tunnel sheds every new connection.
const MAX_INFLIGHT_CONNS: usize = 64;

/// Upper bound on the upstream dial (SOCKS CONNECT + optional broker TLS
/// handshake). Without it a hung bastion→broker connect blocks a `handle_conn`
/// forever, leaking the two fds it holds; librdkafka meanwhile gives up after
/// its own `socket.timeout.ms` (~10s) and reconnects, so the stuck tasks would
/// otherwise accumulate without bound. Kept at 10s to match that budget.
const PROXY_DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Shared proxy state: how to dial brokers and the real→local listener map.
struct ProxyShared {
    socks_addr: SocketAddr,
    uses_tls: bool,
    tls: Option<TlsConnector>,
    /// real (host, port) → local listener port.
    endpoints: AsyncMutex<HashMap<(String, u16), u16>>,
    /// local listener port → real (host, port), for displaying real broker
    /// addresses (the metadata the client sees is rewritten to 127.0.0.1).
    reverse: Mutex<HashMap<u16, (String, u16)>>,
    /// Accept-loop handles, aborted when the tunnel drops.
    handles: Mutex<Vec<JoinHandle<()>>>,
    /// Caps concurrent in-flight proxy connections (see [`MAX_INFLIGHT_CONNS`]),
    /// so a reconnect storm against an unreachable broker can't exhaust fds.
    sem: Arc<Semaphore>,
}

/// A live SSH-tunnelled Kafka proxy for one cluster. Drop tears down the
/// listeners and (via [`SshTunnel`]) the `ssh` child.
pub struct BrokerTunnel {
    _ssh: SshTunnel,
    socks_port: u16,
    shared: Arc<ProxyShared>,
    bootstrap_local: String,
}

impl BrokerTunnel {
    /// Open the SOCKS tunnel and bind a local listener for each bootstrap
    /// broker. `uses_tls`/`skip_verify` describe the proxy→broker hop.
    pub async fn open(
        ssh: &SshTunnelConfig,
        bootstrap: &str,
        uses_tls: bool,
        skip_verify: bool,
    ) -> Result<BrokerTunnel> {
        let ssh_t = SshTunnel::open_socks(ssh).await?;
        let socks_port = ssh_t.local_port();
        let socks_addr: SocketAddr = ([127, 0, 0, 1], socks_port).into();
        let tls = if uses_tls {
            Some(build_tls(skip_verify)?)
        } else {
            None
        };
        let shared = Arc::new(ProxyShared {
            socks_addr,
            uses_tls,
            tls,
            endpoints: AsyncMutex::new(HashMap::new()),
            reverse: Mutex::new(HashMap::new()),
            handles: Mutex::new(Vec::new()),
            sem: Arc::new(Semaphore::new(MAX_INFLIGHT_CONNS)),
        });

        let mut locals = Vec::new();
        for (host, port) in parse_bootstrap(bootstrap) {
            let lp = shared.ensure_listener(&host, port).await?;
            locals.push(format!("127.0.0.1:{lp}"));
        }
        if locals.is_empty() {
            return Err(Error::Invalid("no bootstrap servers to tunnel".into()));
        }

        Ok(BrokerTunnel {
            _ssh: ssh_t,
            socks_port,
            shared,
            bootstrap_local: locals.join(","),
        })
    }

    /// `bootstrap.servers` to hand librdkafka (the local listeners).
    pub fn local_bootstrap(&self) -> String {
        self.bootstrap_local.clone()
    }

    /// SOCKS5 proxy URL for SOCKS-aware HTTP clients (schema registry, metrics).
    pub fn socks_url(&self) -> String {
        format!("socks5h://127.0.0.1:{}", self.socks_port)
    }

    /// Whether the underlying `ssh` tunnel is still up.
    pub fn is_alive(&self) -> bool {
        self._ssh.is_alive()
    }

    /// Real broker `(host, port)` for a local proxy listener port — used to show
    /// the actual broker address instead of the rewritten `127.0.0.1:<local>`.
    pub fn real_endpoint(&self, local_port: u16) -> Option<(String, u16)> {
        self.shared.reverse.lock().ok()?.get(&local_port).cloned()
    }
}

impl Drop for BrokerTunnel {
    fn drop(&mut self) {
        if let Ok(mut handles) = self.shared.handles.lock() {
            for h in handles.drain(..) {
                h.abort();
            }
        }
    }
}

impl ProxyShared {
    /// Local listener port forwarding to real broker `host:port` (created once).
    async fn ensure_listener(self: &Arc<Self>, host: &str, port: u16) -> Result<u16> {
        let key = (host.to_string(), port);
        if let Some(lp) = self.endpoints.lock().await.get(&key) {
            return Ok(*lp);
        }
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| Error::Internal(format!("bind broker listener: {e}")))?;
        let lp = listener
            .local_addr()
            .map_err(|e| Error::Internal(format!("listener addr: {e}")))?
            .port();

        {
            // Re-check under the lock so a concurrent open doesn't double-bind.
            let mut map = self.endpoints.lock().await;
            if let Some(existing) = map.get(&key) {
                return Ok(*existing);
            }
            map.insert(key, lp);
        }
        if let Ok(mut rev) = self.reverse.lock() {
            rev.insert(lp, (host.to_string(), port));
        }

        let shared = self.clone();
        let host = host.to_string();
        let handle = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((client, _)) => {
                        // Bound concurrent upstream dials. `handle_conn` drops the
                        // permit once the upstream is up; if none is free we shed
                        // this connection (close it) instead of piling up fds —
                        // librdkafka reconnects when a slot opens.
                        let permit = match Arc::clone(&shared.sem).try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::warn!(
                                    broker = %host,
                                    "broker proxy: {MAX_INFLIGHT_CONNS} upstream dials already in flight — shedding connection"
                                );
                                drop(client);
                                continue;
                            }
                        };
                        let shared = shared.clone();
                        let host = host.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_conn(
                                shared,
                                host,
                                port,
                                client,
                                PROXY_DIAL_TIMEOUT,
                                Some(permit),
                            )
                            .await
                            {
                                tracing::debug!("broker proxy connection closed: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::debug!("broker proxy accept error: {e}");
                        break;
                    }
                }
            }
        });
        if let Ok(mut handles) = self.handles.lock() {
            handles.push(handle);
        }
        Ok(lp)
    }
}

/// Forward one accepted client connection to the real broker through SOCKS
/// (with optional TLS), running the framed, rewriting pump.
///
/// `dial_timeout` bounds the upstream SOCKS CONNECT and broker-TLS handshake so
/// a hung bastion→broker path can't pin this connection's file descriptors
/// indefinitely (the pump itself unwinds on either side's EOF).
///
/// `dial_permit` (see [`MAX_INFLIGHT_CONNS`]) is held only until the upstream
/// (SOCKS + TLS) is established, then released before the pump runs.
///
/// Returns a boxed future: this is part of a recursive cycle (the pump rewrites
/// `Metadata`, which calls `ensure_listener`, which spawns `handle_conn` again),
/// and boxing gives the cycle a concrete type so `Send` inference terminates.
fn handle_conn(
    shared: Arc<ProxyShared>,
    host: String,
    port: u16,
    client: TcpStream,
    dial_timeout: Duration,
    dial_permit: Option<OwnedSemaphorePermit>,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move {
        let socks = tokio::time::timeout(
            dial_timeout,
            Socks5Stream::connect(shared.socks_addr, (host.as_str(), port)),
        )
        .await
        .map_err(|_| Error::Upstream(format!("socks dial {host}:{port} timed out")))?
        .map_err(|e| Error::Upstream(format!("socks dial {host}:{port}: {e}")))?;

        if shared.uses_tls {
            let connector = shared
                .tls
                .clone()
                .ok_or_else(|| Error::Internal("tls connector missing".into()))?;
            let server_name = rustls::pki_types::ServerName::try_from(host.clone())
                .map_err(|_| Error::Invalid(format!("invalid broker hostname: {host}")))?;
            let tls = tokio::time::timeout(dial_timeout, connector.connect(server_name, socks))
                .await
                .map_err(|_| Error::Upstream(format!("broker tls handshake {host} timed out")))?
                .map_err(|e| Error::Upstream(format!("broker tls handshake {host}: {e}")))?;
            drop(dial_permit);
            pump(client, tls, shared).await
        } else {
            drop(dial_permit);
            pump(client, socks, shared).await
        }
    })
}

/// Bidirectional framed proxy. Tracks request correlation ids (Metadata /
/// FindCoordinator / ApiVersions) so the matching responses can be rewritten.
/// Generic over both ends so it can be driven by in-memory pipes in tests.
async fn pump<C, B>(client: C, broker: B, shared: Arc<ProxyShared>) -> Result<()>
where
    C: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut cr, mut cw) = tokio::io::split(client);
    let (mut br, mut bw) = tokio::io::split(broker);
    let corr: Arc<AsyncMutex<HashMap<i32, (i16, i16)>>> = Arc::new(AsyncMutex::new(HashMap::new()));

    // Requests are never rewritten: only the 8-byte header is read (to note
    // the correlation ids whose responses get rewritten) and the body streams
    // through, so a Produce batch is never buffered whole.
    let corr_up = corr.clone();
    let up = async move {
        while let Ok(Some(len)) = read_len(&mut cr).await {
            let mut head = [0u8; 8];
            let head = &mut head[..len.min(8)];
            if cr.read_exact(head).await.is_err() {
                break;
            }
            if let Some(h) = parse_request_header(head) {
                if matches!(
                    h.api_key,
                    API_METADATA | API_FIND_COORDINATOR | API_VERSIONS
                ) {
                    corr_up
                        .lock()
                        .await
                        .insert(h.correlation_id, (h.api_key, h.api_version));
                }
            }
            if stream_frame(&mut cr, &mut bw, len, head).await.is_err() {
                break;
            }
        }
    };

    // Only tracked responses (Metadata / FindCoordinator / ApiVersions, all
    // small) are read whole and rewritten; everything else — Fetch responses
    // up to `fetch.max.bytes` — is passed through as it arrives instead of
    // store-and-forward (SC-20).
    let down = async move {
        while let Ok(Some(len)) = read_len(&mut br).await {
            let mut head = [0u8; 4];
            let head = &mut head[..len.min(4)];
            if br.read_exact(head).await.is_err() {
                break;
            }
            let entry = match <[u8; 4]>::try_from(&*head) {
                Ok(cid) => corr.lock().await.remove(&i32::from_be_bytes(cid)),
                Err(_) => None,
            };
            let sent = match entry {
                Some((api_key, api_version)) => {
                    let mut frame = vec![0u8; len];
                    frame[..4].copy_from_slice(head);
                    if br.read_exact(&mut frame[4..]).await.is_err() {
                        break;
                    }
                    let frame = transform_response(&shared, frame, api_key, api_version).await;
                    write_frame(&mut cw, &frame).await
                }
                None => stream_frame(&mut br, &mut cw, len, head).await,
            };
            if sent.is_err() {
                break;
            }
        }
    };

    // When either direction ends, the other future is dropped (cancelled),
    // closing its halves.
    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
    Ok(())
}

/// Apply the address/version rewrites to a tracked response frame; on any parse
/// error fall back to forwarding the original bytes unchanged.
async fn transform_response(
    shared: &Arc<ProxyShared>,
    mut frame: Vec<u8>,
    api_key: i16,
    api_version: i16,
) -> Vec<u8> {
    match api_key {
        API_VERSIONS => {
            protocol::clamp_api_versions(&mut frame, is_flexible(API_VERSIONS, api_version));
            frame
        }
        API_METADATA if api_version <= METADATA_MAX => {
            match rewrite_metadata_frame(shared, &frame, api_version).await {
                Ok(out) => out,
                Err(_) => frame,
            }
        }
        API_FIND_COORDINATOR if api_version <= FIND_COORDINATOR_MAX => {
            match rewrite_fc_frame(shared, &frame, api_version).await {
                Ok(out) => out,
                Err(_) => frame,
            }
        }
        _ => frame,
    }
}

async fn rewrite_metadata_frame(
    shared: &Arc<ProxyShared>,
    frame: &[u8],
    version: i16,
) -> Result<Vec<u8>> {
    let eps = protocol::metadata_broker_endpoints(frame, version)?;
    let mut map: HashMap<(String, u16), u16> = HashMap::new();
    for (host, port) in eps {
        let lp = shared.ensure_listener(&host, port).await?;
        map.insert((host, port), lp);
    }
    protocol::rewrite_metadata(frame, version, |host, port| {
        let lp = map.get(&(host.to_string(), port)).copied().unwrap_or(port);
        ("127.0.0.1".to_string(), lp)
    })
}

async fn rewrite_fc_frame(
    shared: &Arc<ProxyShared>,
    frame: &[u8],
    version: i16,
) -> Result<Vec<u8>> {
    match protocol::find_coordinator_endpoint(frame, version)? {
        Some((host, port)) => {
            let lp = shared.ensure_listener(&host, port).await?;
            protocol::rewrite_find_coordinator(frame, version, move |_h, _p| {
                ("127.0.0.1".to_string(), lp)
            })
        }
        None => Ok(frame.to_vec()),
    }
}

/// Read a frame's 4-byte length prefix. `Ok(None)` on a clean EOF.
async fn read_len<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Option<usize>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = i32::from_be_bytes(len_buf);
    if len < 0 || len as usize > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "kafka frame length out of range",
        ));
    }
    Ok(Some(len as usize))
}

/// Read one length-prefixed Kafka frame (the bytes after the 4-byte length).
/// `Ok(None)` on a clean EOF.
#[cfg(test)]
async fn read_frame<R: tokio::io::AsyncRead + Unpin>(
    r: &mut R,
) -> std::io::Result<Option<Vec<u8>>> {
    let Some(len) = read_len(r).await? else {
        return Ok(None);
    };
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok(Some(body))
}

/// Forward one frame unchanged without holding it: the length prefix and the
/// `head` already read, then the remaining `len - head.len()` bytes copied
/// straight from `r` to `w` through tokio's bounded copy buffer.
async fn stream_frame<R, W>(r: &mut R, w: &mut W, len: usize, head: &[u8]) -> std::io::Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut prefix = [0u8; 12];
    prefix[..4].copy_from_slice(&(len as i32).to_be_bytes());
    prefix[4..4 + head.len()].copy_from_slice(head);
    w.write_all(&prefix[..4 + head.len()]).await?;
    let rest = (len - head.len()) as u64;
    let copied = tokio::io::copy(&mut AsyncReadExt::take(&mut *r, rest), w).await?;
    if copied != rest {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    w.flush().await
}

/// Write a length-prefixed Kafka frame.
async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(
    w: &mut W,
    body: &[u8],
) -> std::io::Result<()> {
    w.write_all(&(body.len() as i32).to_be_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

/// Parse `host:port,host:port` (default port 9092; tolerates whitespace).
fn parse_bootstrap(bootstrap: &str) -> Vec<(String, u16)> {
    bootstrap
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            match entry.rsplit_once(':') {
                Some((host, port)) if !host.is_empty() => Some((
                    host.to_string(),
                    port.trim().parse().unwrap_or(DEFAULT_KAFKA_PORT),
                )),
                _ => Some((entry.to_string(), DEFAULT_KAFKA_PORT)),
            }
        })
        .collect()
}

/// A `tokio_rustls::TlsConnector` for the proxy→broker hop. Uses the bundled
/// webpki roots (Amazon's CA, so MSK certs validate); when `skip_verify` is set,
/// installs the no-op verifier (chain/hostname unchecked, signatures still are).
fn build_tls(skip_verify: bool) -> Result<TlsConnector> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Internal(format!("tls config: {e}")))?;
    let config = if skip_verify {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerifier::new()))
            .with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    Ok(TlsConnector::from(Arc::new(config)))
}

/// A `ServerCertVerifier` that accepts any server certificate (only used when
/// the cluster has "skip TLS verify" set). Signatures are still verified by the
/// crypto provider; only chain validity and hostname matching are skipped.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bootstrap_variants() {
        assert_eq!(
            parse_bootstrap("b-1.msk.amazonaws.com:9094, b-2.msk.amazonaws.com:9094"),
            vec![
                ("b-1.msk.amazonaws.com".to_string(), 9094),
                ("b-2.msk.amazonaws.com".to_string(), 9094),
            ]
        );
        // Missing port → default; empty entries skipped.
        assert_eq!(
            parse_bootstrap("host-only,,h2:9095"),
            vec![("host-only".to_string(), 9092), ("h2".to_string(), 9095)]
        );
    }

    /// Drive `pump` with in-memory pipes (no SSH/SOCKS/TLS): a Metadata request
    /// flows app→broker, and the broker's Metadata response comes back to the
    /// app with the advertised broker rewritten to a local listener.
    #[tokio::test]
    async fn pump_rewrites_metadata_end_to_end() {
        let shared = Arc::new(ProxyShared {
            socks_addr: ([127, 0, 0, 1], 1).into(), // unused: no real broker dial here
            uses_tls: false,
            tls: None,
            endpoints: AsyncMutex::new(HashMap::new()),
            reverse: Mutex::new(HashMap::new()),
            handles: Mutex::new(Vec::new()),
            sem: Arc::new(Semaphore::new(MAX_INFLIGHT_CONNS)),
        });

        let (mut client_app, client_proxy) = tokio::io::duplex(64 * 1024);
        let (broker_proxy, mut broker_app) = tokio::io::duplex(64 * 1024);
        tokio::spawn(pump(client_proxy, broker_proxy, shared.clone()));

        // App → Metadata request (api_key=3, version=4, corr=55) + client_id "x".
        let mut req = Vec::new();
        req.extend_from_slice(&3i16.to_be_bytes());
        req.extend_from_slice(&4i16.to_be_bytes());
        req.extend_from_slice(&55i32.to_be_bytes());
        req.extend_from_slice(&1i16.to_be_bytes());
        req.push(b'x');
        write_frame(&mut client_app, &req).await.unwrap();

        // Broker receives the forwarded request unchanged.
        let got = read_frame(&mut broker_app).await.unwrap().unwrap();
        assert_eq!(&got[0..2], &3i16.to_be_bytes());

        // Broker → Metadata v4 response: corr=55, throttle, one broker.
        let mut resp = Vec::new();
        resp.extend_from_slice(&55i32.to_be_bytes());
        resp.extend_from_slice(&0i32.to_be_bytes());
        resp.extend_from_slice(&1i32.to_be_bytes());
        resp.extend_from_slice(&101i32.to_be_bytes());
        let host = b"b-1.msk.amazonaws.com";
        resp.extend_from_slice(&(host.len() as i16).to_be_bytes());
        resp.extend_from_slice(host);
        resp.extend_from_slice(&9094i32.to_be_bytes());
        resp.extend_from_slice(&(-1i16).to_be_bytes()); // rack null
        write_frame(&mut broker_app, &resp).await.unwrap();

        // App receives a response with the broker rewritten to 127.0.0.1:<local>.
        let out = read_frame(&mut client_app).await.unwrap().unwrap();
        let eps = protocol::metadata_broker_endpoints(&out, 4).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].0, "127.0.0.1");
        assert_ne!(eps[0].1, 9094); // remapped to a local listener port
    }

    /// Untracked responses (a big Fetch) are passed through as they arrive —
    /// the client sees the frame's first bytes before the broker has sent the
    /// rest (store-and-forward would hold everything back until the end) — and
    /// arrive intact; a tracked response right after still gets rewritten.
    #[tokio::test]
    async fn pump_streams_untracked_frames() {
        let shared = Arc::new(ProxyShared {
            socks_addr: ([127, 0, 0, 1], 1).into(),
            uses_tls: false,
            tls: None,
            endpoints: AsyncMutex::new(HashMap::new()),
            reverse: Mutex::new(HashMap::new()),
            handles: Mutex::new(Vec::new()),
            sem: Arc::new(Semaphore::new(MAX_INFLIGHT_CONNS)),
        });
        let (mut client_app, client_proxy) = tokio::io::duplex(16 * 1024);
        let (broker_proxy, mut broker_app) = tokio::io::duplex(16 * 1024);
        tokio::spawn(pump(client_proxy, broker_proxy, shared));

        // A 1 MiB Fetch-style response (corr 7, never tracked). Send the prefix
        // and the first 1000 body bytes only.
        let body: Vec<u8> = (0..1024 * 1024u32).map(|i| (i % 251) as u8).collect();
        let mut frame = 7i32.to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        broker_app
            .write_all(&(frame.len() as i32).to_be_bytes())
            .await
            .unwrap();
        broker_app.write_all(&frame[..1004]).await.unwrap();
        broker_app.flush().await.unwrap();
        let mut first = [0u8; 4 + 1004];
        tokio::time::timeout(Duration::from_secs(2), client_app.read_exact(&mut first))
            .await
            .expect("pass-through: the head arrives before the rest of the frame is sent")
            .unwrap();
        assert_eq!(&first[4..8], &7i32.to_be_bytes());
        // The rest follows (the writer runs concurrently: the duplex is small).
        let rest = frame[1004..].to_vec();
        let writer = tokio::spawn(async move {
            broker_app.write_all(&rest).await.unwrap();
            broker_app
        });
        let mut tail = vec![0u8; frame.len() - 1004];
        client_app.read_exact(&mut tail).await.unwrap();
        assert_eq!(&tail[..], &frame[1004..], "bytes forwarded intact");
        let mut broker_app = writer.await.unwrap();

        // A tracked request/response pair still works after a streamed frame.
        let mut req = Vec::new();
        req.extend_from_slice(&3i16.to_be_bytes());
        req.extend_from_slice(&4i16.to_be_bytes());
        req.extend_from_slice(&56i32.to_be_bytes());
        req.extend_from_slice(&(-1i16).to_be_bytes());
        write_frame(&mut client_app, &req).await.unwrap();
        assert_eq!(read_frame(&mut broker_app).await.unwrap().unwrap(), req);
        let mut resp = Vec::new();
        resp.extend_from_slice(&56i32.to_be_bytes());
        resp.extend_from_slice(&0i32.to_be_bytes());
        resp.extend_from_slice(&1i32.to_be_bytes());
        resp.extend_from_slice(&101i32.to_be_bytes());
        let host = b"b-2.msk.amazonaws.com";
        resp.extend_from_slice(&(host.len() as i16).to_be_bytes());
        resp.extend_from_slice(host);
        resp.extend_from_slice(&9094i32.to_be_bytes());
        resp.extend_from_slice(&(-1i16).to_be_bytes());
        write_frame(&mut broker_app, &resp).await.unwrap();
        let out = read_frame(&mut client_app).await.unwrap().unwrap();
        let eps = protocol::metadata_broker_endpoints(&out, 4).unwrap();
        assert_eq!(eps[0].0, "127.0.0.1");
    }

    /// The dial permit is released once the upstream is established, so a
    /// long-lived broker connection does not count against
    /// [`MAX_INFLIGHT_CONNS`] for its whole life.
    #[tokio::test]
    async fn dial_permit_is_released_once_upstream_is_established() {
        // Minimal SOCKS5 server: no-auth, accept any CONNECT, then hold the
        // tunnel open (a live, idle broker connection).
        let socks = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let socks_addr = socks.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = socks.accept().await {
                tokio::spawn(async move {
                    let mut g = [0u8; 2];
                    s.read_exact(&mut g).await.unwrap();
                    let mut m = vec![0u8; g[1] as usize];
                    s.read_exact(&mut m).await.unwrap();
                    s.write_all(&[5, 0]).await.unwrap();
                    let mut h = [0u8; 4];
                    s.read_exact(&mut h).await.unwrap();
                    let rest = match h[3] {
                        1 => 4 + 2,
                        4 => 16 + 2,
                        _ => {
                            let mut l = [0u8; 1];
                            s.read_exact(&mut l).await.unwrap();
                            l[0] as usize + 2
                        }
                    };
                    let mut r = vec![0u8; rest];
                    s.read_exact(&mut r).await.unwrap();
                    s.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                        .await
                        .unwrap();
                    let mut sink = [0u8; 64];
                    while let Ok(n) = s.read(&mut sink).await {
                        if n == 0 {
                            break;
                        }
                    }
                });
            }
        });
        let sem = Arc::new(Semaphore::new(MAX_INFLIGHT_CONNS));
        let shared = Arc::new(ProxyShared {
            socks_addr,
            uses_tls: false,
            tls: None,
            endpoints: AsyncMutex::new(HashMap::new()),
            reverse: Mutex::new(HashMap::new()),
            handles: Mutex::new(Vec::new()),
            sem: sem.clone(),
        });
        let front = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let client = TcpStream::connect(front.local_addr().unwrap())
            .await
            .unwrap();
        let _server_side = front.accept().await.unwrap();
        let permit = sem.clone().try_acquire_owned().unwrap();
        assert_eq!(sem.available_permits(), MAX_INFLIGHT_CONNS - 1);
        let conn = tokio::spawn(handle_conn(
            shared,
            "broker.internal".into(),
            9094,
            client,
            Duration::from_secs(5),
            Some(permit),
        ));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while sem.available_permits() < MAX_INFLIGHT_CONNS {
            assert!(
                std::time::Instant::now() < deadline,
                "permit still held by an established connection"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!conn.is_finished(), "the connection itself stays up");
        conn.abort();
    }

    /// A hung upstream (a "SOCKS" endpoint that accepts but never speaks — like a
    /// bastion whose broker dial stalls) must make `handle_conn` give up within
    /// `dial_timeout` and release its fds, not block forever. This is the leak
    /// that exhausted the daemon's open-files limit under a reconnect storm.
    #[tokio::test]
    async fn handle_conn_times_out_on_hung_upstream() {
        use std::time::Instant;

        // Fake SOCKS server: accept connections and hold them open, silent.
        let socks = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let socks_addr = socks.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((s, _)) = socks.accept().await {
                held.push(s); // never respond to the SOCKS handshake
            }
        });

        let shared = Arc::new(ProxyShared {
            socks_addr,
            uses_tls: false,
            tls: None,
            endpoints: AsyncMutex::new(HashMap::new()),
            reverse: Mutex::new(HashMap::new()),
            handles: Mutex::new(Vec::new()),
            sem: Arc::new(Semaphore::new(MAX_INFLIGHT_CONNS)),
        });

        // A real client TcpStream to hand the proxy (the "librdkafka" side).
        let front = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let front_addr = front.local_addr().unwrap();
        let client = TcpStream::connect(front_addr).await.unwrap();
        let _server_side = front.accept().await.unwrap();

        let start = Instant::now();
        let res = handle_conn(
            shared,
            "broker.internal".into(),
            9094,
            client,
            Duration::from_millis(200),
            None,
        )
        .await;
        assert!(res.is_err(), "a hung upstream dial must error, not hang");
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "handle_conn must give up promptly (was {:?})",
            start.elapsed()
        );
    }
}
