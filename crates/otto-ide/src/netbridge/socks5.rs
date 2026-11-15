//! Minimal RFC 1928 SOCKS5 loopback server (for JVM `-DsocksProxyHost`) and
//! SOCKS5 client dialer (for `otto_ssh::SshTunnel::open_socks` upstream hops).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const SOCKS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Dial `target_host:target_port` (using SOCKS5 domain-name ATYP `0x03` so DNS
/// resolves on the SSH bastion's network) through `127.0.0.1:socks_port`.
pub async fn connect_via_socks5(
    socks_port: u16,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream> {
    let host_bytes = target_host.as_bytes();
    if host_bytes.is_empty() || host_bytes.len() > 255 {
        return Err(Error::Invalid(format!(
            "invalid SOCKS5 target host '{target_host}'"
        )));
    }
    let mut stream = TcpStream::connect(("127.0.0.1", socks_port))
        .await
        .map_err(|e| Error::Upstream(format!("connect SSH SOCKS5 127.0.0.1:{socks_port}: {e}")))?;
    let _ = stream.set_nodelay(true);

    tokio::time::timeout(SOCKS_HANDSHAKE_TIMEOUT, async {
        stream.write_all(&[0x05, 0x01, 0x00]).await?;
        let mut method_reply = [0u8; 2];
        stream.read_exact(&mut method_reply).await?;
        if method_reply != [0x05, 0x00] {
            return Err(std::io::Error::other("SOCKS5 auth negotiation rejected"));
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

/// Callback implemented by [`super::tunnel::NetBridgeInstance`] to connect a
/// SOCKS5 client request `(host, port)` to either a bridged K8s/SSH endpoint or
/// direct outbound TCP.
pub trait SocksUpstreamResolver: Send + Sync + 'static {
    fn open_upstream(
        &self,
        host: String,
        port: u16,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<TcpStream>> + Send>>;
}

/// Bind a loopback-only `127.0.0.1:0` SOCKS5 server and return `(port, abort_handle)`.
pub async fn spawn_local_socks5_server(
    resolver: Arc<dyn SocksUpstreamResolver>,
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
            tokio::spawn(async move {
                let _ = handle_socks5_client(&mut client, res_clone).await;
            });
        }
    });
    Ok((port, join.abort_handle()))
}

async fn handle_socks5_client(
    client: &mut TcpStream,
    resolver: Arc<dyn SocksUpstreamResolver>,
) -> std::io::Result<()> {
    let mut ver_nmethods = [0u8; 2];
    client.read_exact(&mut ver_nmethods).await?;
    if ver_nmethods[0] != 0x05 {
        return Ok(());
    }
    let mut methods = vec![0u8; usize::from(ver_nmethods[1])];
    client.read_exact(&mut methods).await?;
    if !methods.contains(&0x00) {
        client.write_all(&[0x05, 0xFF]).await?;
        return Ok(());
    }
    client.write_all(&[0x05, 0x00]).await?;

    let mut head = [0u8; 4];
    client.read_exact(&mut head).await?;
    if head[0] != 0x05 || head[1] != 0x01 {
        client
            .write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
            .await?;
        return Ok(());
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
            return Ok(());
        }
    };
    let mut port_buf = [0u8; 2];
    client.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    match resolver.open_upstream(host, port).await {
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
        Err(_) => {
            client
                .write_all(&[0x05, 0x05, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .await?;
        }
    }
    Ok(())
}
