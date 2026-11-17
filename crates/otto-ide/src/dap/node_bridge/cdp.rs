//! RFC 6455 loopback WebSocket client and `node --inspect-brk=127.0.0.1:0` process spawner
//! for the built-in Node.js / V8 Inspector-to-DAP bridge.

use std::io::{self, ErrorKind};
use std::time::Duration;

use otto_core::{Error, Result};
use otto_ssh::SshTunnel;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use super::NodeInspectorBridgeSpec;
use crate::dap::wire;
use crate::dap::{spawn_target_process, ssh_tunnel_config, BridgeGuard};
use crate::types::{IdeSshSpec, IdeTargetKind, IdeTargetSpec};

pub(super) struct InspectorWs {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    /// Bytes read but not yet framed, and the fragments of the message being
    /// assembled: kept here so a cancelled [`Self::next_json`] loses nothing.
    rx: Vec<u8>,
    message: Vec<u8>,
}

impl InspectorWs {
    pub async fn connect(port: u16, ws_path: &str) -> Result<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|e| Error::Upstream(format!("connect V8 inspector 127.0.0.1:{port}: {e}")))?;
        let _ = stream.set_nodelay(true);
        let (read_half, mut writer) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let req = format!(
            "GET {ws_path} HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        );
        writer
            .write_all(req.as_bytes())
            .await
            .map_err(|e| Error::Upstream(format!("inspector handshake write: {e}")))?;
        writer
            .flush()
            .await
            .map_err(|e| Error::Upstream(format!("inspector handshake flush: {e}")))?;

        let mut status_ok = false;
        loop {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| Error::Upstream(format!("inspector handshake read: {e}")))?;
            if n == 0 {
                return Err(Error::Upstream("V8 inspector closed before upgrade".into()));
            }
            if line.starts_with("HTTP/1.1 101") || line.starts_with("HTTP/1.0 101") {
                status_ok = true;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        if !status_ok {
            return Err(Error::Upstream(
                "V8 inspector rejected WebSocket upgrade".into(),
            ));
        }
        Ok(Self {
            reader,
            writer,
            rx: Vec::new(),
            message: Vec::new(),
        })
    }

    pub async fn send_json(&mut self, val: &Value) -> io::Result<()> {
        let payload = serde_json::to_vec(val)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))?;
        Self::write_masked_frame(&mut self.writer, 0x81, &payload).await
    }

    async fn write_masked_frame(
        writer: &mut tokio::net::tcp::OwnedWriteHalf,
        first_byte: u8,
        payload: &[u8],
    ) -> io::Result<()> {
        let mask: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
        let mut header = Vec::with_capacity(14);
        header.push(first_byte);
        let len = payload.len();
        if len < 126 {
            header.push(0x80 | (len as u8));
        } else if len <= 65535 {
            header.push(0x80 | 126);
            header.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            header.push(0x80 | 127);
            header.extend_from_slice(&(len as u64).to_be_bytes());
        }
        header.extend_from_slice(&mask);
        let masked: Vec<u8> = payload
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i & 3])
            .collect();
        writer.write_all(&header).await?;
        writer.write_all(&masked).await?;
        writer.flush().await
    }

    /// Next CDP message, `None` once the inspector closes. Cancel-safe: the bridge
    /// polls it inside `select!`, and a frame split across TCP reads stays buffered
    /// when another branch wins (dropping it mid-frame desynced the stream, so the
    /// bridge never saw `Debugger.paused` and the debuggee hung at its first break).
    pub async fn next_json(&mut self) -> io::Result<Option<Value>> {
        loop {
            while let Some((fin, opcode, payload)) = self.take_frame()? {
                match opcode {
                    0x8 => return Ok(None),
                    0x9 => Self::write_masked_frame(&mut self.writer, 0x8a, &payload).await?,
                    0x0 | 0x1 => {
                        self.message.extend_from_slice(&payload);
                        if fin {
                            let msg = std::mem::take(&mut self.message);
                            return serde_json::from_slice(&msg).map(Some).map_err(|e| {
                                io::Error::new(ErrorKind::InvalidData, format!("cdp json: {e}"))
                            });
                        }
                    }
                    _ => {}
                }
            }
            // `read_buf` is cancel-safe: no bytes are consumed unless it returns.
            if !matches!(self.reader.read_buf(&mut self.rx).await, Ok(n) if n > 0) {
                return Ok(None);
            }
        }
    }

    /// Pops one complete frame `(fin, opcode, unmasked payload)` off `rx`.
    fn take_frame(&mut self) -> io::Result<Option<(bool, u8, Vec<u8>)>> {
        let b = &self.rx;
        if b.len() < 2 {
            return Ok(None);
        }
        let (mut len, mut at) = ((b[1] & 0x7f) as usize, 2);
        if len == 126 {
            if b.len() < 4 {
                return Ok(None);
            }
            (len, at) = (u16::from_be_bytes([b[2], b[3]]) as usize, 4);
        } else if len == 127 {
            if b.len() < 10 {
                return Ok(None);
            }
            let ext = u64::from_be_bytes(b[2..10].try_into().expect("8 bytes"));
            (len, at) = (usize::try_from(ext).unwrap_or(usize::MAX), 10);
        }
        if len > wire::MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "V8 inspector frame exceeds 16 MiB",
            ));
        }
        let mask_at = at;
        if b[1] & 0x80 != 0 {
            at += 4;
        }
        if b.len() < at + len {
            return Ok(None);
        }
        let mut payload = b[at..at + len].to_vec();
        if at > mask_at {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= b[mask_at + (i & 3)];
            }
        }
        let (fin, opcode) = (b[0] & 0x80 != 0, b[0] & 0x0f);
        self.rx.drain(..at + len);
        Ok(Some((fin, opcode, payload)))
    }
}

pub(super) async fn discover_attach_ws_path(port: u16) -> Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| Error::Upstream(format!("connect Node inspector 127.0.0.1:{port}: {e}")))?;
    let req =
        format!("GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| Error::Upstream(format!("query /json/list: {e}")))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .map_err(|e| Error::Upstream(format!("read /json/list: {e}")))?;
    let text = String::from_utf8_lossy(&raw);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or(text.as_ref());
    let arr: Value = serde_json::from_str(body.trim())
        .map_err(|e| Error::Upstream(format!("parse /json/list: {e}")))?;
    let ws_url = arr
        .as_array()
        .and_then(|a| a.first())
        .and_then(|item| item.get("webSocketDebuggerUrl"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Upstream("Node inspector returned no debuggable target".into()))?;
    extract_ws_port_and_path(ws_url)
        .map(|(_, path)| path)
        .ok_or_else(|| Error::Upstream(format!("invalid webSocketDebuggerUrl '{ws_url}'")))
}

pub(super) fn extract_ws_port_and_path(line: &str) -> Option<(u16, String)> {
    let idx = line.find("ws://")?;
    let rest = &line[idx + 5..];
    let token = rest.split_whitespace().next()?;
    let (host_port, path_part) = match token.split_once('/') {
        Some((hp, p)) => (hp, format!("/{p}")),
        None => (token, "/".to_string()),
    };
    let port: u16 = host_port.rsplit_once(':')?.1.parse().ok()?;
    Some((port, path_part))
}

pub(super) async fn maybe_tunnel_port(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    guard: &BridgeGuard,
    remote_port: u16,
) -> Result<u16> {
    if target.kind == IdeTargetKind::Ssh {
        let ssh_spec = ssh.ok_or_else(|| Error::Invalid("missing SSH spec".into()))?;
        let tunnel =
            SshTunnel::open(&ssh_tunnel_config(ssh_spec)?, "127.0.0.1", remote_port).await?;
        let lp = tunnel.local_port();
        guard.push_tunnel(tunnel);
        Ok(lp)
    } else {
        Ok(remote_port)
    }
}

pub(super) async fn spawn_inspected_node(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    spec: &NodeInspectorBridgeSpec,
    guard: &BridgeGuard,
    out_tx: &mpsc::UnboundedSender<(String, String)>,
) -> Result<(u16, String)> {
    // A runner that opens the inspector in its own worker (vitest `--inspect-brk`) gets none
    // here: the bridge attaches to the worker's endpoint instead.
    let runner_inspects = spec.args.iter().any(|a| a.starts_with("--inspect-brk"));
    let mut argv = Vec::new();
    if !runner_inspects {
        argv.push("--inspect-brk=127.0.0.1:0".to_string());
    }
    argv.extend(spec.node_flags.clone());
    argv.push(spec.program.clone());
    argv.extend(spec.args.clone());

    let mut child =
        spawn_target_process(target, ssh, &spec.node_bin, &argv, &spec.cwd, &spec.env).await?;
    let stdout = child.stdout.take();
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Internal("missing node stderr pipe".into()))?;

    if let Some(out) = stdout {
        let tx = out_tx.clone();
        crate::line_reader::spawn_line_drain(
            out,
            crate::line_reader::MAX_OUTPUT_LINE_BYTES,
            move |line| {
                let _ = tx.send(("stdout".into(), format!("{line}\n")));
            },
        );
    }

    let mut err_lines =
        crate::line_reader::LossyLines::new(stderr, crate::line_reader::MAX_OUTPUT_LINE_BYTES);
    let mut found: Option<(u16, String)> = None;
    let mut boot_err = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), err_lines.next_line()).await {
            Ok(Some(line)) => {
                if let Some(parsed) = extract_ws_port_and_path(&line) {
                    found = Some(parsed);
                    break;
                }
                if boot_err.len() < 4096 {
                    boot_err.push_str(&line);
                    boot_err.push('\n');
                }
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    let Some(res) = found else {
        let _ = child.start_kill();
        return Err(Error::Upstream(format!(
            "node --inspect-brk did not expose inspector endpoint: {}",
            boot_err.trim()
        )));
    };

    let tx_err = out_tx.clone();
    tokio::spawn(async move {
        while let Some(line) = err_lines.next_line().await {
            if !line.starts_with("Debugger listening on ")
                && !line.starts_with("For help, see: https://nodejs.org")
                && !line.starts_with("Debugger attached.")
                && !line.starts_with("Waiting for the debugger to disconnect")
            {
                let _ = tx_err.send(("stderr".into(), format!("{line}\n")));
            }
        }
    });

    guard.push_child(child);
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// The bridge polls `next_json` inside `select!`, so a frame that arrives in two
    /// TCP reads must survive the read being cancelled between them.
    #[tokio::test]
    async fn next_json_survives_cancellation_mid_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (go_tx, go_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut req = BufReader::new(&mut sock);
            let mut line = String::new();
            while req.read_line(&mut line).await.unwrap() > 0 && line != "\r\n" {
                line.clear();
            }
            sock.write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n")
                .await
                .unwrap();
            let payload = br#"{"method":"Debugger.paused","params":{}}"#;
            let mut frame = vec![0x81, payload.len() as u8];
            frame.extend_from_slice(payload);
            sock.write_all(&frame[..10]).await.unwrap();
            go_rx.await.unwrap();
            sock.write_all(&frame[10..]).await.unwrap();
            sock
        });

        let mut ws = InspectorWs::connect(port, "/x").await.unwrap();
        tokio::select! {
            got = ws.next_json() => panic!("half a frame must not parse: {got:?}"),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        go_tx.send(()).unwrap();
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next_json())
            .await
            .expect("frame completed after the cancelled read")
            .unwrap()
            .unwrap();
        assert_eq!(msg["method"], "Debugger.paused");
        drop(server);
    }
}
