//! Authenticated RFC 1928 + RFC 1929 SOCKS5 loopback server and SOCKS5 client
//! dialer (for `otto_ssh::SshTunnel::open_socks` upstream hops and tests).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const SOCKS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKS_AUTH_USER: &[u8] = b"otto";

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

/// Dial `target_host:target_port` (using SOCKS5 domain-name ATYP `0x03` so DNS
/// resolves on the SSH bastion's network) through unauthenticated `127.0.0.1:socks_port`.
pub async fn connect_via_socks5(
    socks_port: u16,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream> {
    connect_via_socks5_with_auth(socks_port, target_host, target_port, None).await
}

/// Dial `target_host:target_port` through `127.0.0.1:socks_port`, optionally
/// authenticating via RFC 1929 Username/Password (`0x02`).
pub async fn connect_via_socks5_with_auth(
    socks_port: u16,
    target_host: &str,
    target_port: u16,
    auth: Option<(&str, &str)>,
) -> Result<TcpStream> {
    let host_bytes = target_host.as_bytes();
    if host_bytes.is_empty() || host_bytes.len() > 255 {
        return Err(Error::Invalid(format!(
            "invalid SOCKS5 target host '{target_host}'"
        )));
    }
    if let Some((u, p)) = auth {
        if u.is_empty() || u.len() > 255 || p.is_empty() || p.len() > 255 {
            return Err(Error::Invalid("invalid SOCKS5 credentials length".into()));
        }
    }
    let mut stream = TcpStream::connect(("127.0.0.1", socks_port))
        .await
        .map_err(|e| Error::Upstream(format!("connect SOCKS5 127.0.0.1:{socks_port}: {e}")))?;
    let _ = stream.set_nodelay(true);

    tokio::time::timeout(SOCKS_HANDSHAKE_TIMEOUT, async {
        match auth {
            Some((user, pass)) => {
                stream.write_all(&[0x05, 0x01, 0x02]).await?;
                let mut method_reply = [0u8; 2];
                stream.read_exact(&mut method_reply).await?;
                if method_reply != [0x05, 0x02] {
                    return Err(std::io::Error::other("SOCKS5 RFC 1929 method rejected"));
                }
                let ub = user.as_bytes();
                let pb = pass.as_bytes();
                let mut sub = Vec::with_capacity(3 + ub.len() + pb.len());
                sub.push(0x01);
                sub.push(ub.len() as u8);
                sub.extend_from_slice(ub);
                sub.push(pb.len() as u8);
                sub.extend_from_slice(pb);
                stream.write_all(&sub).await?;
                let mut auth_reply = [0u8; 2];
                stream.read_exact(&mut auth_reply).await?;
                if auth_reply != [0x01, 0x00] {
                    return Err(std::io::Error::other("SOCKS5 RFC 1929 auth failed"));
                }
            }
            None => {
                stream.write_all(&[0x05, 0x01, 0x00]).await?;
                let mut method_reply = [0u8; 2];
                stream.read_exact(&mut method_reply).await?;
                if method_reply != [0x05, 0x00] {
                    return Err(std::io::Error::other("SOCKS5 auth negotiation rejected"));
                }
            }
        }

        let mut req = Vec::with_capacity(7 + host_bytes.len());
        req.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host_bytes.len() as u8]);
        req.extend_from_slice(host_bytes);
        req.extend_from_slice(&target_port.to_be_bytes());
        stream.write_all(&req).await?;

        let mut head = [0u8; 4];
        stream.read_exact(&mut head).await?;
        if head[0] != 0x05 || head[1] != 0x00 {
            return Err(std::io::Error::other(format!(
                "SOCKS5 connect to {target_host}:{target_port} failed (rep=0x{:02x})",
                head[1]
            )));
        }
        match head[3] {
            0x01 => {
                let mut skip = [0u8; 6];
                stream.read_exact(&mut skip).await?;
            }
            0x03 => {
                let mut len = [0u8; 1];
                stream.read_exact(&mut len).await?;
                let mut skip = vec![0u8; usize::from(len[0]) + 2];
                stream.read_exact(&mut skip).await?;
            }
            0x04 => {
                let mut skip = [0u8; 18];
                stream.read_exact(&mut skip).await?;
            }
            other => {
                return Err(std::io::Error::other(format!(
                    "unsupported SOCKS5 reply ATYP 0x{other:02x}"
                )));
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| {
        Error::Upstream(format!(
            "SOCKS5 handshake timed out for {target_host}:{target_port}"
        ))
    })?
    .map_err(|e| Error::Upstream(format!("SOCKS5 {target_host}:{target_port}: {e}")))?;

    Ok(stream)
}

/// Callback implemented by [`super::tunnel::NetBridgeInstance`] to connect an
/// authenticated SOCKS5 client request `(host, port)` to a bridged endpoint.
pub trait SocksUpstreamResolver: Send + Sync + 'static {
    fn open_upstream(
        &self,
        host: String,
        port: u16,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<TcpStream>> + Send>>;
}

/// Bind a loopback-only `127.0.0.1:0` SOCKS5 server requiring RFC 1929
/// Username/Password auth (`otto:{auth_token}`) and return `(port, abort_handle)`.
pub async fn spawn_local_socks5_server(
    resolver: Arc<dyn SocksUpstreamResolver>,
    auth_token: Arc<str>,
) -> Result<(u16, tokio::task::AbortHandle)> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| Error::Internal(format!("bind local SOCKS5 listener: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Internal(format!("SOCKS5 local_addr: {e}")))?
        .port();

    let join = tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                break;
            };
            let _ = client.set_nodelay(true);
            let res_clone = Arc::clone(&resolver);
            let tok_clone = Arc::clone(&auth_token);
            tokio::spawn(async move {
                let _ = handle_socks5_client(&mut client, res_clone, &tok_clone).await;
            });
        }
    });
    Ok((port, join.abort_handle()))
}

async fn read_authenticated_target(
    client: &mut TcpStream,
    expected_token: &str,
) -> std::io::Result<Option<(String, u16)>> {
    let mut ver_nmethods = [0u8; 2];
    client.read_exact(&mut ver_nmethods).await?;
    if ver_nmethods[0] != 0x05 {
        return Ok(None);
    }
    let mut methods = vec![0u8; usize::from(ver_nmethods[1])];
    client.read_exact(&mut methods).await?;
    if !methods.contains(&0x02) {
        client.write_all(&[0x05, 0xFF]).await?;
        client.flush().await?;
        return Ok(None);
    }
    client.write_all(&[0x05, 0x02]).await?;

    // RFC 1929 Username/Password subnegotiation (version = 0x01).
    let mut sub_hdr = [0u8; 2];
    client.read_exact(&mut sub_hdr).await?;
    if sub_hdr[0] != 0x01 {
        client.write_all(&[0x01, 0x01]).await?;
        client.flush().await?;
        return Ok(None);
    }
    let mut uname = vec![0u8; usize::from(sub_hdr[1])];
    client.read_exact(&mut uname).await?;
    let mut plen = [0u8; 1];
    client.read_exact(&mut plen).await?;
    let mut passwd = vec![0u8; usize::from(plen[0])];
    client.read_exact(&mut passwd).await?;

    let user_ok = constant_time_eq(&uname, SOCKS_AUTH_USER);
    let pass_ok = constant_time_eq(&passwd, expected_token.as_bytes());
    if !(user_ok && pass_ok) {
        client.write_all(&[0x01, 0x01]).await?;
        client.flush().await?;
        return Ok(None);
    }
    client.write_all(&[0x01, 0x00]).await?;

    let mut head = [0u8; 4];
    client.read_exact(&mut head).await?;
    if head[0] != 0x05 || head[1] != 0x01 {
        client
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Ok(None);
    }

    let host = match head[3] {
        0x01 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip).await?;
            Ipv4Addr::from(ip).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut buf = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut buf).await?;
            String::from_utf8_lossy(&buf).into_owned()
        }
        0x04 => {
            let mut ip = [0u8; 16];
            client.read_exact(&mut ip).await?;
            Ipv6Addr::from(ip).to_string()
        }
        _ => {
            client
                .write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
            return Ok(None);
        }
    };
    let mut port_buf = [0u8; 2];
    client.read_exact(&mut port_buf).await?;
    Ok(Some((host, u16::from_be_bytes(port_buf))))
}

async fn handle_socks5_client(
    client: &mut TcpStream,
    resolver: Arc<dyn SocksUpstreamResolver>,
    expected_token: &str,
) -> std::io::Result<()> {
    let target = match tokio::time::timeout(
        SOCKS_HANDSHAKE_TIMEOUT,
        read_authenticated_target(client, expected_token),
    )
    .await
    {
        Ok(Ok(Some(t))) => t,
        Ok(Ok(None)) => return Ok(()),
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(()),
    };

    match resolver.open_upstream(target.0, target.1).await {
        Ok(mut upstream) => {
            let _ = upstream.set_nodelay(true);
            let bind_addr = upstream
                .local_addr()
                .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], 0)));
            let mut ok_reply = vec![0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1];
            ok_reply.extend_from_slice(&bind_addr.port().to_be_bytes());
            client.write_all(&ok_reply).await?;
            let _ = tokio::io::copy_bidirectional(client, &mut upstream).await;
        }
        Err(Error::Forbidden(_)) => {
            client
                .write_all(&[0x05, 0x02, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
        }
        Err(_) => {
            client
                .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
        }
    }
    Ok(())
}
