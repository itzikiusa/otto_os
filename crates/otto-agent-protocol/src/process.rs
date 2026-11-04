//! The child process over stdio: newline-delimited JSON in both directions.
//!
//! [`spawn_child`] starts the (already safety-prepared) command in its own
//! process group with piped stdin/stdout/stderr and returns a [`ChildHandle`]:
//! a stdin line sender and an ordered message stream — every stdout line,
//! then exactly one [`ChildMsg::Exited`] (after stdout drained, so no line
//! ever arrives after the exit). Stderr is never parsed; its tail rides the
//! exit message for diagnostics.

use std::process::Stdio;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// One stdout line larger than this is discarded (reported as
/// [`ChildMsg::Overlong`]) instead of buffering without bound.
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Bytes of stderr kept for the exit diagnostic.
const STDERR_TAIL: usize = 4096;
/// Backpressure: a slow consumer pauses the stdout reader, which pauses the
/// child once its pipe fills.
const CHANNEL_DEPTH: usize = 256;

/// The command, as the safety pipeline left it.
#[derive(Debug, Clone)]
pub struct ChildCommand {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildMsg {
    Line(String),
    /// A line over [`MAX_LINE_BYTES`] was dropped (its length).
    Overlong(usize),
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
}

/// The live child. Dropping the stdin sender closes the child's stdin (EOF),
/// which every supported CLI treats as "exit".
pub struct ChildHandle {
    pub pid: u32,
    pub stdin: mpsc::UnboundedSender<String>,
    pub messages: mpsc::Receiver<ChildMsg>,
}

/// Signal the child's whole process group (its tools included).
pub fn signal_group(pid: u32, sig: i32) {
    let Ok(pgid) = libc::pid_t::try_from(pid) else {
        return;
    };
    if pgid <= 1 {
        return;
    }
    // SAFETY: plain syscall on a pid we spawned as a group leader.
    unsafe {
        libc::killpg(pgid, sig);
    }
}

pub fn spawn_child(cmd: &ChildCommand) -> std::io::Result<ChildHandle> {
    let mut command = Command::new(&cmd.program);
    command
        .args(&cmd.args)
        .current_dir(&cmd.cwd)
        .envs(cmd.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let pid = child.id().unwrap_or(0);
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Some(line) = stdin_rx.recv().await {
            let mut buf = line.into_bytes();
            buf.push(b'\n');
            if stdin.write_all(&buf).await.is_err() || stdin.flush().await.is_err() {
                break;
            }
        }
        // Sender dropped (close) or the pipe broke: EOF for the child.
    });

    let tail = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr_task = tokio::spawn(drain_stderr(stderr, Arc::clone(&tail)));

    let (tx, rx) = mpsc::channel(CHANNEL_DEPTH);
    let reader_tx = tx.clone();
    let reader = tokio::spawn(read_lines(stdout, reader_tx));
    tokio::spawn(async move {
        let status = child.wait().await;
        let _ = reader.await;
        let _ = stderr_task.await;
        let code = status.ok().and_then(|s| s.code());
        let stderr_tail = {
            let t = tail.lock().unwrap_or_else(|e| e.into_inner());
            String::from_utf8_lossy(&t).into_owned()
        };
        let _ = tx.send(ChildMsg::Exited { code, stderr_tail }).await;
    });

    Ok(ChildHandle {
        pid,
        stdin: stdin_tx,
        messages: rx,
    })
}

async fn read_lines<R: AsyncRead + Unpin>(stdout: R, tx: mpsc::Sender<ChildMsg>) {
    let mut reader = BufReader::new(stdout);
    let mut line: Vec<u8> = Vec::new();
    let mut overlong = 0usize;
    while let Ok(buf) = reader.fill_buf().await {
        if buf.is_empty() {
            break;
        }
        let (chunk, done, used) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (&buf[..i], true, i + 1),
            None => (buf, false, buf.len()),
        };
        if overlong > 0 || line.len() + chunk.len() > MAX_LINE_BYTES {
            overlong += line.len() + chunk.len();
            line.clear();
        } else {
            line.extend_from_slice(chunk);
        }
        reader.consume(used);
        if done {
            let msg = if overlong > 0 {
                ChildMsg::Overlong(std::mem::take(&mut overlong))
            } else {
                let mut text = String::from_utf8_lossy(&line).into_owned();
                if text.ends_with('\r') {
                    text.pop();
                }
                line.clear();
                if text.trim().is_empty() {
                    continue;
                }
                ChildMsg::Line(text)
            };
            if tx.send(msg).await.is_err() {
                break;
            }
        }
    }
    // A final unterminated line still counts.
    if overlong == 0 && !line.is_empty() {
        let text = String::from_utf8_lossy(&line).into_owned();
        if !text.trim().is_empty() {
            let _ = tx.send(ChildMsg::Line(text)).await;
        }
    }
}

async fn drain_stderr<R: AsyncRead + Unpin>(mut stderr: R, tail: Arc<Mutex<Vec<u8>>>) {
    let mut buf = [0u8; 4096];
    loop {
        match stderr.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                t.extend_from_slice(&buf[..n]);
                if t.len() > STDERR_TAIL {
                    let cut = t.len() - STDERR_TAIL;
                    t.drain(..cut);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lines_then_exit_in_order_with_overlong_dropped() {
        let (mut w, r) = tokio::io::duplex(1 << 16);
        let (tx, mut rx) = mpsc::channel(16);
        let reader = tokio::spawn(read_lines(r, tx));
        w.write_all(b"one\r\n\n  \ntwo\n").await.unwrap();
        w.write_all(b"tail-no-newline").await.unwrap();
        drop(w);
        reader.await.unwrap();
        let mut got = Vec::new();
        while let Some(m) = rx.recv().await {
            got.push(m);
        }
        assert_eq!(
            got,
            vec![
                ChildMsg::Line("one".into()),
                ChildMsg::Line("two".into()),
                ChildMsg::Line("tail-no-newline".into()),
            ]
        );
    }

    #[tokio::test]
    async fn overlong_line_is_reported_not_buffered() {
        let (mut w, r) = tokio::io::duplex(1 << 16);
        let (tx, mut rx) = mpsc::channel(16);
        let reader = tokio::spawn(read_lines(r, tx));
        let writer = tokio::spawn(async move {
            let chunk = vec![b'x'; 1 << 20];
            for _ in 0..17 {
                w.write_all(&chunk).await.unwrap();
            }
            w.write_all(b"\nafter\n").await.unwrap();
        });
        let first = rx.recv().await.unwrap();
        assert!(
            matches!(first, ChildMsg::Overlong(n) if n >= MAX_LINE_BYTES),
            "{first:?}"
        );
        assert_eq!(rx.recv().await.unwrap(), ChildMsg::Line("after".into()));
        writer.await.unwrap();
        drop(reader);
    }
}
