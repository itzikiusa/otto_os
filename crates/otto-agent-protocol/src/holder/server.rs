//! Detached `runtime-holder` process server (`ottod runtime-holder --socket <path>`).
//!
//! Owns one structured-runtime child process in an independent session (`setsid`),
//! keeps its stdin pipe open across `ottod` restarts, buffers stdout/stderr in a
//! bounded 16 MiB [`LineRing`], and replays unacknowledged output on re-attach.

use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::BufReader;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::holder::pipes::{
    redirect_stdio_to_devnull, spawn_child_pipes, terminate_child_group, ChildPipeEvent, StdinCmd,
};
use crate::holder::protocol::{
    checkpoint_acked_seq, read_frame, read_tolerant_frame, write_frame, DaemonFrame,
    HolderExitStatus, HolderFrame, HolderReady, HolderSignal, HolderSpawnSpec, LineRing,
    DEFAULT_EXIT_LINGER, DEFAULT_ORPHAN_TTL, ERROR_PREFIX, HOLDER_ENV, MAX_RING_BYTES,
    PROTO_VERSION, READY_PREFIX, SOCKET_WATCHDOG_INTERVAL,
};
use crate::process::signal_group;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

enum ClientIn {
    Ready {
        conn_id: u64,
        reader: BufReader<OwnedReadHalf>,
        writer: OwnedWriteHalf,
    },
    Frame {
        conn_id: u64,
        frame: DaemonFrame,
    },
    Disconnected {
        conn_id: u64,
    },
}

struct ActiveClient {
    id: u64,
    out_tx: mpsc::UnboundedSender<HolderFrame>,
    reader_task: JoinHandle<()>,
    writer_task: JoinHandle<()>,
}

impl ActiveClient {
    fn send(&self, frame: HolderFrame) -> bool {
        self.out_tx.send(frame).is_ok()
    }

    fn close(self) {
        self.reader_task.abort();
        drop(self.out_tx);
        drop(self.writer_task);
    }
}

/// CLI entry point shared by `ottod runtime-holder --socket <path>` and the
/// standalone `otto-runtime-holder --socket <path>` binary.
#[allow(clippy::disallowed_methods)]
pub fn run_from_args_and_stdin() -> i32 {
    let socket_path = match parse_socket_arg(std::env::args().skip(1)) {
        Ok(p) => p,
        Err(e) => {
            println!("{ERROR_PREFIX}{e}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            return 2;
        }
    };
    match run_holder_cli(&socket_path) {
        Ok(()) => 0,
        Err(e) => {
            println!("{ERROR_PREFIX}{e}");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            1
        }
    }
}

fn parse_socket_arg(mut args: impl Iterator<Item = String>) -> Result<PathBuf, String> {
    while let Some(arg) = args.next() {
        if arg == "runtime-holder" {
            continue;
        }
        if arg == "--socket" {
            let val = args
                .next()
                .ok_or_else(|| "missing value after --socket".to_string())?;
            return Ok(PathBuf::from(val));
        }
        if let Some(val) = arg.strip_prefix("--socket=") {
            if val.is_empty() {
                return Err("empty --socket path".into());
            }
            return Ok(PathBuf::from(val));
        }
    }
    Err("missing required --socket <path> argument".into())
}

/// Read [`HolderSpawnSpec`] from stdin, bind `socket_path`, spawn the child,
/// emit `READY_PREFIX` on stdout, detach stdio, and serve reconnects.
#[allow(clippy::disallowed_methods)]
pub fn run_holder_cli(socket_path: &Path) -> Result<(), String> {
    if std::env::var_os(HOLDER_ENV).is_none() {
        return Err(format!("{HOLDER_ENV} must be set"));
    }
    // SAFETY: ignoring SIGPIPE lets broken UDS / child stdin writes surface as EPIPE.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let mut raw_spec = String::new();
    std::io::stdin()
        .read_to_string(&mut raw_spec)
        .map_err(|e| format!("read holder spec from stdin: {e}"))?;
    let spec: HolderSpawnSpec =
        serde_json::from_str(&raw_spec).map_err(|e| format!("invalid holder spec json: {e}"))?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("build holder tokio runtime: {e}"))?;
    rt.block_on(serve_holder(socket_path, spec))
}

async fn serve_holder(socket_path: &Path, spec: HolderSpawnSpec) -> Result<(), String> {
    if tokio::fs::symlink_metadata(socket_path).await.is_ok() {
        if UnixStream::connect(socket_path).await.is_ok() {
            return Err(format!(
                "runtime holder socket already active: {}",
                socket_path.display()
            ));
        }
        let _ = tokio::fs::remove_file(socket_path).await;
    }
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create holder socket dir {}: {e}", parent.display()))?;
    }
    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("bind holder socket {}: {e}", socket_path.display()))?;
    tokio::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|e| format!("chmod holder socket {}: {e}", socket_path.display()))?;
    let bound_socket_id = tokio::fs::symlink_metadata(socket_path)
        .await
        .ok()
        .map(|m| (m.dev(), m.ino()));

    let mut cmd = Command::new(&spec.command.program);
    cmd.args(&spec.command.args)
        .current_dir(&spec.command.cwd)
        .envs(
            spec.command
                .env
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str())),
        )
        .env_remove(HOLDER_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(false);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tokio::fs::remove_file(socket_path).await;
            return Err(format!("spawn child {}: {e}", spec.command.program));
        }
    };

    let child_pid = child.id().unwrap_or(0);
    let child_stdin = child.stdin.take().expect("piped stdin");
    let child_stdout = child.stdout.take().expect("piped stdout");
    let child_stderr = child.stderr.take().expect("piped stderr");

    let ready = HolderReady {
        holder_pid: std::process::id(),
        child_pid,
    };
    let ready_json = serde_json::to_string(&ready).map_err(|e| e.to_string())?;
    println!("{READY_PREFIX}{ready_json}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    redirect_stdio_to_devnull();

    let (stdin_tx, mut pipe_rx, stderr_tail) =
        spawn_child_pipes(child, child_stdin, child_stdout, child_stderr);
    let mut stdin_tx = Some(stdin_tx);

    let linger_dur = spec
        .exit_linger_ms
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_EXIT_LINGER);
    let orphan_dur = spec
        .orphan_ttl_ms
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_ORPHAN_TTL);
    let watchdog_every = SOCKET_WATCHDOG_INTERVAL.min(orphan_dur.max(Duration::from_millis(50)));
    let mut watchdog_tick = tokio::time::interval(watchdog_every);
    watchdog_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    watchdog_tick.tick().await;

    let mut ring = LineRing::new(MAX_RING_BYTES);
    let mut checkpoint: serde_json::Value = spec.initial_checkpoint;
    let mut exited: Option<HolderExitStatus> = None;
    let mut active_client: Option<ActiveClient> = None;
    let mut next_conn_id: u64 = 1;
    let (client_in_tx, mut client_in_rx) = mpsc::unbounded_channel::<ClientIn>();
    let mut linger_deadline: Option<Instant> = None;
    let mut orphan_deadline: Option<Instant> = Some(Instant::now() + orphan_dur);

    loop {
        let linger_sleep = async {
            match linger_deadline {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending::<()>().await,
            }
        };
        let orphan_sleep = async {
            match orphan_deadline {
                Some(at) if active_client.is_none() => tokio::time::sleep_until(at).await,
                _ => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    let conn_id = next_conn_id;
                    next_conn_id += 1;
                    let tx = client_in_tx.clone();
                    tokio::spawn(async move {
                        let (read_half, write_half) = stream.into_split();
                        let mut reader = BufReader::new(read_half);
                        let mut buf = Vec::new();
                        let hello = tokio::time::timeout(
                            HANDSHAKE_TIMEOUT,
                            read_frame::<_, DaemonFrame>(&mut reader, &mut buf),
                        ).await;
                        if let Ok(Ok(Some(DaemonFrame::Hello { .. }))) = hello {
                            let _ = tx.send(ClientIn::Ready { conn_id, reader, writer: write_half });
                        }
                    });
                }
            }
            Some(pipe_ev) = pipe_rx.recv() => {
                match pipe_ev {
                    ChildPipeEvent::Line { stream, text } => {
                        let entry = ring.push_line(stream, text);
                        if let Some(client) = &active_client {
                            let _ = client.send(HolderFrame::Line { entry });
                        }
                    }
                    ChildPipeEvent::Overlong { stream, bytes } => {
                        let entry = ring.push_overlong(stream, bytes);
                        if let Some(client) = &active_client {
                            let _ = client.send(HolderFrame::Line { entry });
                        }
                    }
                    ChildPipeEvent::Exited { code } => {
                        let tail_str = {
                            let guard = stderr_tail.lock().unwrap_or_else(|e| e.into_inner());
                            String::from_utf8_lossy(&guard).into_owned()
                        };
                        let status = HolderExitStatus { code, stderr_tail: tail_str };
                        exited = Some(status.clone());
                        stdin_tx = None;
                        orphan_deadline = None;
                        if let Some(client) = &active_client {
                            let _ = client.send(HolderFrame::Exit { status });
                        } else {
                            linger_deadline = Some(Instant::now() + linger_dur);
                        }
                    }
                }
            }
            Some(cin) = client_in_rx.recv() => {
                match cin {
                    ClientIn::Ready { conn_id, mut reader, mut writer } => {
                        if let Some(prev) = active_client.take() {
                            let _ = prev.send(HolderFrame::Superseded);
                            prev.close();
                        }
                        linger_deadline = None;
                        orphan_deadline = None;
                        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<HolderFrame>();
                        let _ = out_tx.send(HolderFrame::Hello {
                            proto_version: PROTO_VERSION,
                            holder_pid: std::process::id(),
                            child_pid,
                            checkpoint: checkpoint.clone(),
                            exited: exited.clone(),
                        });
                        for entry in ring.replay_after(checkpoint_acked_seq(&checkpoint)) {
                            let _ = out_tx.send(HolderFrame::ReplayLine { entry });
                        }
                        let _ = out_tx.send(HolderFrame::ReplayDone);

                        let writer_task = tokio::spawn(async move {
                            while let Some(frame) = out_rx.recv().await {
                                if write_frame(&mut writer, &frame).await.is_err() {
                                    break;
                                }
                            }
                        });
                        let in_tx = client_in_tx.clone();
                        let reader_task = tokio::spawn(async move {
                            let mut buf = Vec::new();
                            while let Ok(Some(maybe_frame)) =
                                read_tolerant_frame::<_, DaemonFrame>(&mut reader, &mut buf).await
                            {
                                let Some(frame) = maybe_frame else {
                                    continue;
                                };
                                if in_tx.send(ClientIn::Frame { conn_id, frame }).is_err() {
                                    return;
                                }
                            }
                            let _ = in_tx.send(ClientIn::Disconnected { conn_id });
                        });
                        active_client = Some(ActiveClient {
                            id: conn_id,
                            out_tx,
                            reader_task,
                            writer_task,
                        });
                    }
                    ClientIn::Frame { conn_id, frame } => {
                        if active_client.as_ref().map(|c| c.id) != Some(conn_id) {
                            continue;
                        }
                        match frame {
                            DaemonFrame::Hello { .. } => {}
                            DaemonFrame::Stdin { line } => {
                                if let Some(tx) = &stdin_tx {
                                    let _ = tx.send(StdinCmd::Line(line));
                                }
                            }
                            DaemonFrame::CloseStdin => {
                                if let Some(tx) = stdin_tx.take() {
                                    let _ = tx.send(StdinCmd::Close);
                                }
                            }
                            DaemonFrame::Signal { signal } => {
                                let sig = match signal {
                                    HolderSignal::Interrupt => libc::SIGINT,
                                    HolderSignal::Terminate => libc::SIGTERM,
                                    HolderSignal::Kill => libc::SIGKILL,
                                };
                                signal_group(child_pid, sig);
                            }
                            DaemonFrame::Checkpoint { checkpoint: cp } => {
                                ring.prune_acked(checkpoint_acked_seq(&cp));
                                checkpoint = cp;
                            }
                            DaemonFrame::Detach => {
                                if let Some(prev) = active_client.take() {
                                    prev.close();
                                }
                                if exited.is_some() {
                                    linger_deadline = Some(Instant::now() + linger_dur);
                                    orphan_deadline = None;
                                } else {
                                    orphan_deadline = Some(Instant::now() + orphan_dur);
                                }
                            }
                            DaemonFrame::Shutdown { grace_ms } => {
                                let _ = tokio::fs::remove_file(socket_path).await;
                                if let Some(prev) = active_client.take() {
                                    prev.close();
                                }
                                if let Some(tx) = stdin_tx.take() {
                                    let _ = tx.send(StdinCmd::Close);
                                }
                                if exited.is_none() {
                                    terminate_child_group(
                                        child_pid,
                                        &mut pipe_rx,
                                        Duration::from_millis(grace_ms.min(5_000)),
                                    )
                                    .await;
                                }
                                break;
                            }
                        }
                    }
                    ClientIn::Disconnected { conn_id } => {
                        if active_client.as_ref().map(|c| c.id) == Some(conn_id) {
                            if let Some(prev) = active_client.take() {
                                prev.close();
                            }
                            if exited.is_some() {
                                linger_deadline = Some(Instant::now() + linger_dur);
                                orphan_deadline = None;
                            } else {
                                orphan_deadline = Some(Instant::now() + orphan_dur);
                            }
                        }
                    }
                }
            }
            _ = watchdog_tick.tick(), if active_client.is_none() => {
                let socket_intact = match (bound_socket_id, tokio::fs::symlink_metadata(socket_path).await) {
                    (Some((dev, ino)), Ok(m)) => m.dev() == dev && m.ino() == ino,
                    (None, Ok(_)) => true,
                    (_, Err(_)) => false,
                };
                if !socket_intact {
                    if let Some(tx) = stdin_tx.take() {
                        let _ = tx.send(StdinCmd::Close);
                    }
                    if exited.is_none() {
                        terminate_child_group(child_pid, &mut pipe_rx, Duration::from_millis(500)).await;
                    }
                    break;
                }
            }
            () = orphan_sleep => {
                let _ = tokio::fs::remove_file(socket_path).await;
                if let Some(tx) = stdin_tx.take() {
                    let _ = tx.send(StdinCmd::Close);
                }
                if exited.is_none() {
                    terminate_child_group(child_pid, &mut pipe_rx, Duration::from_millis(500)).await;
                }
                break;
            }
            () = linger_sleep => {
                let _ = tokio::fs::remove_file(socket_path).await;
                break;
            }
        }
    }
    Ok(())
}
