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

use otto_core::{Error, Result};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex as AsyncMutex;
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
                        let shared = shared.clone();
                        let host = host.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_conn(shared, host, port, client).await {
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
/// Returns a boxed future: this is part of a recursive cycle (the pump rewrites
/// `Metadata`, which calls `ensure_listener`, which spawns `handle_conn` again),
/// and boxing gives the cycle a concrete type so `Send` inference terminates.
fn handle_conn(
    shared: Arc<ProxyShared>,
    host: String,
    port: u16,
    client: TcpStream,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move {
        let socks = Socks5Stream::connect(shared.socks_addr, (host.as_str(), port))
            .await
            .map_err(|e| Error::Upstream(format!("socks dial {host}:{port}: {e}")))?;

        if shared.uses_tls {
            let connector = shared
                .tls
                .clone()
                .ok_or_else(|| Error::Internal("tls connector missing".into()))?;
            let server_name = rustls::pki_types::ServerName::try_from(host.clone())
                .map_err(|_| Error::Invalid(format!("invalid broker hostname: {host}")))?;
            let tls = connector
                .connect(server_name, socks)
                .await
                .map_err(|e| Error::Upstream(format!("broker tls handshake {host}: {e}")))?;
            pump(client, tls, shared).await
        } else {
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

    let corr_up = corr.clone();
    let up = async move {
        while let Ok(Some(frame)) = read_frame(&mut cr).await {
            if let Some(h) = parse_request_header(&frame) {
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
            if write_frame(&mut bw, &frame).await.is_err() {
                break;
            }
        }
    };

    let down = async move {
        while let Ok(Some(mut frame)) = read_frame(&mut br).await {
            if frame.len() >= 4 {
                let cid = i32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
                let entry = corr.lock().await.remove(&cid);
                if let Some((api_key, api_version)) = entry {
                    frame = transform_response(&shared, frame, api_key, api_version).await;
                }
            }
            if write_frame(&mut cw, &frame).await.is_err() {
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

/// Read one length-prefixed Kafka frame (the bytes after the 4-byte length).
/// `Ok(None)` on a clean EOF.
async fn read_frame<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Option<Vec<u8>>> {
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
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    Ok(Some(body))
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
}
