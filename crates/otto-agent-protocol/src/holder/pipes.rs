//! Child stdio pipe pumping and process-group teardown inside `runtime-holder`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::holder::protocol::LineStream;
use crate::process::{signal_group, MAX_LINE_BYTES};

const STDERR_TAIL_BYTES: usize = 4096;
const PIPE_CHANNEL_DEPTH: usize = 512;

pub(super) enum ChildPipeEvent {
    Line { stream: LineStream, text: String },
    Overlong { stream: LineStream, bytes: usize },
    Exited { code: Option<i32> },
}

pub(super) enum StdinCmd {
    Line(String),
    Close,
}

pub(super) type PipeHandles = (
    mpsc::UnboundedSender<StdinCmd>,
    mpsc::Receiver<ChildPipeEvent>,
    Arc<Mutex<Vec<u8>>>,
);

pub(super) fn spawn_child_pipes(
    mut child: Child,
    mut child_stdin: ChildStdin,
    child_stdout: ChildStdout,
    child_stderr: ChildStderr,
) -> PipeHandles {
    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<StdinCmd>();
    tokio::spawn(async move {
        while let Some(cmd) = stdin_rx.recv().await {
            match cmd {
                StdinCmd::Line(line) => {
                    let mut bytes = line.into_bytes();
                    bytes.push(b'\n');
                    if child_stdin.write_all(&bytes).await.is_err()
                        || child_stdin.flush().await.is_err()
                    {
                        break;
                    }
                }
                StdinCmd::Close => break,
            }
        }
    });

    let stderr_tail = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (pipe_tx, pipe_rx) = mpsc::channel::<ChildPipeEvent>(PIPE_CHANNEL_DEPTH);
    let out_task = tokio::spawn(pump_stream(
        child_stdout,
        LineStream::Out,
        None,
        pipe_tx.clone(),
    ));
    let err_task = tokio::spawn(pump_stream(
        child_stderr,
        LineStream::Err,
        Some(Arc::clone(&stderr_tail)),
        pipe_tx.clone(),
    ));
    tokio::spawn(async move {
        let status = child.wait().await;
        let _ = out_task.await;
        let _ = err_task.await;
        let code = status.ok().and_then(|s| s.code());
        let _ = pipe_tx.send(ChildPipeEvent::Exited { code }).await;
    });

    (stdin_tx, pipe_rx, stderr_tail)
}

pub(super) async fn terminate_child_group(
    child_pid: u32,
    pipe_rx: &mut mpsc::Receiver<ChildPipeEvent>,
    grace: Duration,
) {
    signal_group(child_pid, libc::SIGTERM);
    if !grace.is_zero() {
        let deadline = Instant::now() + grace;
        while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, pipe_rx.recv()).await {
            if matches!(ev, ChildPipeEvent::Exited { .. }) {
                return;
            }
        }
    }
    signal_group(child_pid, libc::SIGKILL);
}

async fn pump_stream<R: AsyncRead + Unpin>(
    stream_io: R,
    stream: LineStream,
    tail: Option<Arc<Mutex<Vec<u8>>>>,
    tx: mpsc::Sender<ChildPipeEvent>,
) {
    let mut reader = BufReader::new(stream_io);
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
        if let Some(tail) = &tail {
            let mut guard = tail.lock().unwrap_or_else(|e| e.into_inner());
            guard.extend_from_slice(&buf[..used]);
            if guard.len() > STDERR_TAIL_BYTES {
                let cut = guard.len() - STDERR_TAIL_BYTES;
                guard.drain(..cut);
            }
        }
        if overlong > 0 || line.len() + chunk.len() > MAX_LINE_BYTES {
            overlong += line.len() + chunk.len();
            line.clear();
        } else {
            line.extend_from_slice(chunk);
        }
        reader.consume(used);
        if done {
            let ev = if overlong > 0 {
                ChildPipeEvent::Overlong {
                    stream,
                    bytes: std::mem::take(&mut overlong),
                }
            } else {
                let mut text = String::from_utf8_lossy(&line).into_owned();
                if text.ends_with('\r') {
                    text.pop();
                }
                line.clear();
                if stream == LineStream::Out && text.trim().is_empty() {
                    continue;
                }
                ChildPipeEvent::Line { stream, text }
            };
            if tx.send(ev).await.is_err() {
                break;
            }
        }
    }
    if overlong == 0 && !line.is_empty() {
        let mut text = String::from_utf8_lossy(&line).into_owned();
        if text.ends_with('\r') {
            text.pop();
        }
        if !(stream == LineStream::Out && text.trim().is_empty()) {
            let _ = tx.send(ChildPipeEvent::Line { stream, text }).await;
        }
    }
}

pub(super) fn redirect_stdio_to_devnull() {
    // SAFETY: dup2 onto standard fds after READY_PREFIX is flushed releases the
    // launcher's stdout pipe so spawn_held does not wait for the holder to exit.
    unsafe {
        let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if fd >= 0 {
            let _ = libc::dup2(fd, libc::STDIN_FILENO);
            let _ = libc::dup2(fd, libc::STDOUT_FILENO);
            let _ = libc::dup2(fd, libc::STDERR_FILENO);
            if fd > libc::STDERR_FILENO {
                let _ = libc::close(fd);
            }
        }
    }
}
