//! Daemon-side spawn, attach, and reconnect client for detached `runtime-holder` processes.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedReadHalf;
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::holder::paths::HolderLauncher;
use crate::holder::protocol::{
    read_frame, write_frame, DaemonFrame, HolderCheckpoint, HolderExitStatus, HolderFrame,
    HolderReady, HolderSignal, HolderSpawnSpec, SequencedLine, ERROR_PREFIX, HOLDER_ENV,
    PROTO_VERSION, READY_PREFIX,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const EVENT_CHANNEL_DEPTH: usize = 512;

/// Events streamed from an attached [`HolderClient`] to `AgentRuntime`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HolderEvent {
    Line(SequencedLine),
    Exited {
        code: Option<i32>,
        stderr_tail: String,
    },
    /// Socket closed without a child `Exit` (daemon detaching or superseded).
    Disconnected,
}

enum ClientCmd {
    Stdin(String),
    CloseStdin,
    Signal(HolderSignal),
    Detach(Box<HolderCheckpoint>),
    Shutdown { grace_ms: u64 },
}

/// Handle to an attached `runtime-holder` Unix domain socket.
pub struct HolderClient {
    cmd_tx: mpsc::UnboundedSender<ClientCmd>,
    cp_tx: watch::Sender<Option<HolderCheckpoint>>,
    writer_task: JoinHandle<()>,
    socket_path: PathBuf,
}

impl HolderClient {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn send_stdin(&self, line: String) -> bool {
        self.cmd_tx.send(ClientCmd::Stdin(line)).is_ok()
    }

    pub fn close_stdin(&self) {
        let _ = self.cmd_tx.send(ClientCmd::CloseStdin);
    }

    pub fn signal(&self, sig: HolderSignal) {
        let _ = self.cmd_tx.send(ClientCmd::Signal(sig));
    }

    /// Coalesced O(1) checkpoint update; latest state wins without queuing.
    pub fn update_checkpoint(&self, checkpoint: HolderCheckpoint) {
        self.cp_tx.send_replace(Some(checkpoint));
    }

    #[cfg(test)]
    pub(crate) fn for_test() -> (Self, watch::Receiver<Option<HolderCheckpoint>>) {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ClientCmd>();
        let (cp_tx, cp_rx) = watch::channel::<Option<HolderCheckpoint>>(None);
        let writer_task = tokio::spawn(async move { while cmd_rx.recv().await.is_some() {} });
        (
            Self {
                cmd_tx,
                cp_tx,
                writer_task,
                socket_path: PathBuf::from("/tmp/otto-holder-test.sock"),
            },
            cp_rx,
        )
    }

    /// Flush the final checkpoint + `DETACH` so the holder and child remain
    /// alive across `ottod` restart.
    pub async fn detach_with_checkpoint(self, checkpoint: HolderCheckpoint) {
        let _ = self.cmd_tx.send(ClientCmd::Detach(Box::new(checkpoint)));
        let _ = tokio::time::timeout(Duration::from_millis(1500), self.writer_task).await;
    }

    /// Explicitly terminate the held child + holder and unlink its socket.
    pub fn shutdown_fire_and_forget(&self, grace: Duration) {
        let grace_ms = u64::try_from(grace.as_millis()).unwrap_or(5_000);
        let _ = self.cmd_tx.send(ClientCmd::Shutdown { grace_ms });
    }

    /// Explicitly terminate the held child + holder and wait for command flush.
    pub async fn shutdown(self, grace: Duration) {
        let grace_ms = u64::try_from(grace.as_millis()).unwrap_or(5_000);
        let _ = self.cmd_tx.send(ClientCmd::Shutdown { grace_ms });
        let _ = tokio::time::timeout(Duration::from_millis(1500), self.writer_task).await;
    }
}

pub struct HeldChild {
    pub holder_pid: u32,
    pub child_pid: u32,
    pub client: HolderClient,
    pub events: mpsc::Receiver<HolderEvent>,
}

pub struct AdoptedHolder {
    pub socket_path: PathBuf,
    pub holder_pid: u32,
    pub child_pid: u32,
    pub checkpoint: HolderCheckpoint,
    pub exited: Option<HolderExitStatus>,
    pub replay_lines: Vec<SequencedLine>,
    pub client: HolderClient,
    pub events: mpsc::Receiver<HolderEvent>,
}

/// Spawn a detached `runtime-holder` process, wait for its `READY` line, and
/// attach a live [`HolderClient`].
pub async fn spawn_held(
    launcher: &HolderLauncher,
    socket_path: &Path,
    spec: &HolderSpawnSpec,
) -> io::Result<HeldChild> {
    let mut cmd = Command::new(&launcher.program);
    cmd.args(&launcher.prefix_args)
        .arg("--socket")
        .arg(socket_path)
        .env(HOLDER_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    // SAFETY: `setsid()` places `runtime-holder` in a new session so signals to
    // `ottod`'s process group never propagate to the holder or its child CLI.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut holder = cmd.spawn()?;
    let mut stdin = holder.stdin.take().expect("piped holder stdin");
    let stdout = holder.stdout.take().expect("piped holder stdout");
    let mut stderr = holder.stderr.take().expect("piped holder stderr");

    let payload = serde_json::to_vec(spec)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    stdin.write_all(&payload).await?;
    drop(stdin);

    let ready = tokio::time::timeout(STARTUP_TIMEOUT, read_ready_line(stdout, &mut stderr))
        .await
        .map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "runtime-holder startup timed out")
        })??;

    // Reap the holder process asynchronously if it ever exits during our lifetime;
    // dropping the JoinHandle on daemon exit leaves the holder running (`kill_on_drop(false)`).
    tokio::spawn(async move {
        let _ = holder.wait().await;
    });

    let adopted = adopt_socket_inner(socket_path, true).await?;
    Ok(HeldChild {
        holder_pid: ready.holder_pid,
        child_pid: adopted.child_pid.max(ready.child_pid),
        client: adopted.client,
        events: adopted.events,
    })
}

/// Attach to an existing `runtime-holder` socket during daemon boot recovery.
pub async fn adopt_socket(socket_path: &Path) -> io::Result<AdoptedHolder> {
    adopt_socket_inner(socket_path, false).await
}

async fn adopt_socket_inner(
    socket_path: &Path,
    inline_initial_replay: bool,
) -> io::Result<AdoptedHolder> {
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(socket_path))
        .await
        .map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "connect holder socket timed out")
        })??;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut buf = Vec::new();

    write_frame(
        &mut write_half,
        &DaemonFrame::Hello {
            proto_version: PROTO_VERSION,
        },
    )
    .await?;

    let (holder_pid, child_pid, checkpoint, exited, replay_lines) =
        tokio::time::timeout(CONNECT_TIMEOUT, async {
            let hello = read_frame::<_, HolderFrame>(&mut reader, &mut buf)
                .await?
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "holder closed before HELLO")
                })?;
            let HolderFrame::Hello {
                proto_version,
                holder_pid,
                child_pid,
                checkpoint,
                exited,
            } = hello
            else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected initial frame from runtime holder",
                ));
            };
            if proto_version != PROTO_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported runtime holder protocol version {proto_version}"),
                ));
            }
            let mut replay = Vec::new();
            loop {
                let frame = read_frame::<_, HolderFrame>(&mut reader, &mut buf)
                    .await?
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::UnexpectedEof, "holder closed during replay")
                    })?;
                match frame {
                    HolderFrame::ReplayLine { entry } => replay.push(entry),
                    HolderFrame::ReplayDone => break,
                    HolderFrame::Superseded => {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionAborted,
                            "holder connection superseded during attach",
                        ));
                    }
                    _ => {}
                }
            }
            Ok::<_, io::Error>((holder_pid, child_pid, *checkpoint, exited, replay))
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "holder handshake timed out"))??;

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ClientCmd>();
    let (cp_tx, mut cp_rx) = watch::channel::<Option<HolderCheckpoint>>(None);
    let writer_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(ClientCmd::Stdin(line)) => {
                            if write_frame(&mut write_half, &DaemonFrame::Stdin { line }).await.is_err() {
                                break;
                            }
                        }
                        Some(ClientCmd::CloseStdin) => {
                            if write_frame(&mut write_half, &DaemonFrame::CloseStdin).await.is_err() {
                                break;
                            }
                        }
                        Some(ClientCmd::Signal(signal)) => {
                            if write_frame(&mut write_half, &DaemonFrame::Signal { signal }).await.is_err() {
                                break;
                            }
                        }
                        Some(ClientCmd::Detach(checkpoint)) => {
                            let _ = write_frame(&mut write_half, &DaemonFrame::Checkpoint { checkpoint }).await;
                            let _ = write_frame(&mut write_half, &DaemonFrame::Detach).await;
                            let _ = write_half.shutdown().await;
                            break;
                        }
                        Some(ClientCmd::Shutdown { grace_ms }) => {
                            let _ = write_frame(&mut write_half, &DaemonFrame::Shutdown { grace_ms }).await;
                            let _ = write_half.shutdown().await;
                            break;
                        }
                        None => break,
                    }
                }
                changed = cp_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let next_cp = cp_rx.borrow_and_update().clone();
                    if let Some(cp) = next_cp {
                        if write_frame(
                            &mut write_half,
                            &DaemonFrame::Checkpoint { checkpoint: Box::new(cp) },
                        ).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    let (ev_tx, ev_rx) = mpsc::channel::<HolderEvent>(EVENT_CHANNEL_DEPTH);
    let initial_for_stream = if inline_initial_replay {
        (replay_lines.clone(), exited.clone())
    } else {
        (Vec::new(), None)
    };
    tokio::spawn(stream_holder_events(reader, ev_tx, initial_for_stream));

    let client = HolderClient {
        cmd_tx,
        cp_tx,
        writer_task,
        socket_path: socket_path.to_path_buf(),
    };
    Ok(AdoptedHolder {
        socket_path: socket_path.to_path_buf(),
        holder_pid,
        child_pid,
        checkpoint,
        exited,
        replay_lines: if inline_initial_replay {
            Vec::new()
        } else {
            replay_lines
        },
        client,
        events: ev_rx,
    })
}

async fn stream_holder_events(
    mut reader: BufReader<OwnedReadHalf>,
    ev_tx: mpsc::Sender<HolderEvent>,
    initial: (Vec<SequencedLine>, Option<HolderExitStatus>),
) {
    for entry in initial.0 {
        if ev_tx.send(HolderEvent::Line(entry)).await.is_err() {
            return;
        }
    }
    if let Some(st) = initial.1 {
        let _ = ev_tx
            .send(HolderEvent::Exited {
                code: st.code,
                stderr_tail: st.stderr_tail,
            })
            .await;
        return;
    }
    let mut buf = Vec::new();
    while let Ok(Some(frame)) = read_frame::<_, HolderFrame>(&mut reader, &mut buf).await {
        match frame {
            HolderFrame::Line { entry } | HolderFrame::ReplayLine { entry } => {
                if ev_tx.send(HolderEvent::Line(entry)).await.is_err() {
                    return;
                }
            }
            HolderFrame::Exit { status } => {
                let _ = ev_tx
                    .send(HolderEvent::Exited {
                        code: status.code,
                        stderr_tail: status.stderr_tail,
                    })
                    .await;
                return;
            }
            HolderFrame::Superseded => {
                let _ = ev_tx.send(HolderEvent::Disconnected).await;
                return;
            }
            HolderFrame::Hello { .. } | HolderFrame::ReplayDone => {}
        }
    }
    let _ = ev_tx.send(HolderEvent::Disconnected).await;
}

async fn read_ready_line(
    stdout: tokio::process::ChildStdout,
    stderr: &mut tokio::process::ChildStderr,
) -> io::Result<HolderReady> {
    let mut lines = BufReader::new(stdout).lines();
    while let Some(line) = lines.next_line().await? {
        if let Some(rest) = line.strip_prefix(READY_PREFIX) {
            return serde_json::from_str(rest)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
        }
        if let Some(err) = line.strip_prefix(ERROR_PREFIX) {
            return Err(io::Error::other(format!("runtime-holder failed: {err}")));
        }
    }
    let mut err_buf = String::new();
    let _ = stderr.read_to_string(&mut err_buf).await;
    Err(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        format!("runtime-holder exited before READY: {}", err_buf.trim()),
    ))
}
