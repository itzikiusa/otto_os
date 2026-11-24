//! Runs the ACP client calls a driver hands over ([`DriverEvent::ClientCall`]:
//! `fs/*`, `terminal/*`) off the session lock and feeds the result back.
//! Files stay inside the session folder (canonical paths, so no `..` or
//! symlink escapes); a terminal's argv already carries the agent's own
//! sandbox wrapper. Terminal output is kept up to its byte limit and also
//! streamed into the tool row as it arrives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use dashmap::DashMap;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

use super::{apply_step, Inner, Live, State};
use crate::driver::ClientOp;
use otto_core::Id;

/// Open client terminals, per session (at most [`MAX_TERMINALS`] each): a
/// session's lookups and its limit never walk another session's terminals.
static TERMINALS: LazyLock<DashMap<Id, HashMap<String, Arc<Terminal>>>> =
    LazyLock::new(DashMap::new);
const MAX_TERMINALS: usize = 16;
const DEFAULT_BYTE_LIMIT: usize = 1 << 20;
const MAX_FILE_BYTES: u64 = 8 << 20;

struct Terminal {
    output: Mutex<(String, bool)>,
    exit: watch::Receiver<Option<(Option<i32>, Option<String>)>>,
    /// The command's process group (it leads its own): a kill reaches
    /// everything it started, also after the shell itself exited.
    pgid: Option<i32>,
    /// The group was empty when the command exited: its id may be reused
    /// by an unrelated group, so it is never signalled again.
    gone: std::sync::atomic::AtomicBool,
}

/// Start `op` for the session `st` belongs to.
pub(super) fn start(inner: &Inner, st: &State, id: String, op: ClientOp) {
    let (Some(inner), Some(live), Some(epoch)) = (
        inner.me.upgrade(),
        inner.live.get(&st.session.id).map(|l| Arc::clone(&l)),
        st.proc.as_ref().map(|p| p.epoch),
    ) else {
        return;
    };
    let sid = st.session.id.clone();
    tokio::spawn(async move {
        let result = run(&inner, &live, epoch, &sid, op).await;
        deliver(&inner, &live, epoch, |d| d.on_client_result(&id, result));
    });
}

/// The session's terminals end with it, or with its agent process (close,
/// exit, a replaced process, idle release): their commands are killed.
pub(super) fn drop_session(sid: &Id) {
    if let Some((_, terms)) = TERMINALS.remove(sid) {
        terms.values().for_each(|t| kill(t));
    }
}

/// Every terminal (the daemon is stopping: nothing could reach them after).
pub(super) fn drop_all() {
    TERMINALS.retain(|_, terms| {
        terms.values().for_each(|t| kill(t));
        false
    });
}

fn deliver(
    inner: &Inner,
    live: &Live,
    epoch: u64,
    f: impl FnOnce(&mut dyn crate::driver::StructuredDriver) -> crate::driver::DriverStep,
) {
    let mut st = live.lock();
    let Some(proc) = st.proc.as_mut().filter(|p| p.epoch == epoch) else {
        return;
    };
    let step = f(proc.driver.as_mut());
    apply_step(inner, live, &mut st, step);
}

/// `path` (absolute) inside `root`, both canonical. A file that does not
/// exist yet is checked through its folder.
async fn confined(root: &str, path: &str, must_exist: bool) -> Result<PathBuf, String> {
    let outside = || format!("{path} is outside this thread's folder");
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(format!("{path} is not an absolute path"));
    }
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(|e| e.to_string())?;
    let real = match tokio::fs::canonicalize(p).await {
        Ok(r) => r,
        Err(_) if !must_exist => {
            let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else {
                return Err(outside());
            };
            tokio::fs::canonicalize(dir)
                .await
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .join(name)
        }
        Err(e) => return Err(format!("{path}: {e}")),
    };
    real.starts_with(&root).then_some(real).ok_or_else(outside)
}

/// Write `path` without following a symbolic link at it: a dangling link
/// inside the folder (only its parent is canonical) must not carry the
/// write outside it. Only a regular file (or a new one) is written: opening
/// a FIFO blocks until a reader comes. `O_NOFOLLOW` / `O_NONBLOCK` also
/// close the check-then-open race.
async fn write_no_follow(path: &Path, content: &str) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    if let Ok(m) = tokio::fs::symlink_metadata(path).await {
        if m.file_type().is_symlink() {
            return Err(std::io::Error::other("is a symbolic link; not followed"));
        }
        if !m.is_file() {
            return Err(std::io::Error::other("is not a regular file"));
        }
    }
    let mut f = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .await?;
    f.write_all(content.as_bytes()).await?;
    // tokio's File hands the write to a blocking task: without a flush the
    // answer (and the file's next reader) can come before the bytes land.
    f.flush().await
}

/// The text a write replaces, for its diff row: a regular file within the
/// read cap only (opening a FIFO blocks for good; a huge file fills memory).
async fn old_text(real: &Path) -> Option<String> {
    let m = tokio::fs::symlink_metadata(real).await.ok()?;
    if !m.file_type().is_file() || m.len() > MAX_FILE_BYTES {
        return None;
    }
    tokio::fs::read_to_string(real).await.ok()
}

/// A regular file within the read cap (reading a FIFO blocks for good).
async fn read_capped(real: &Path, path: &str) -> Result<String, String> {
    let m = tokio::fs::metadata(real).await.map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err(format!("{path} is not a regular file"));
    }
    if m.len() > MAX_FILE_BYTES {
        return Err(format!("{path} is larger than {} MB", MAX_FILE_BYTES >> 20));
    }
    tokio::fs::read_to_string(real)
        .await
        .map_err(|e| format!("{path}: {e}"))
}

async fn run(
    inner: &Arc<Inner>,
    live: &Arc<Live>,
    epoch: u64,
    sid: &Id,
    op: ClientOp,
) -> Result<Value, String> {
    let term = |tid: &str| {
        TERMINALS
            .get(sid)
            .and_then(|terms| terms.get(tid).cloned())
            .ok_or_else(|| format!("no terminal {tid}"))
    };
    match op {
        ClientOp::ReadFile {
            root,
            path,
            line,
            limit,
        } => {
            let real = confined(&root, &path, true).await?;
            let text = read_capped(&real, &path).await?;
            let content = match (line, limit) {
                (None, None) => text,
                (l, n) => {
                    let skip = l.unwrap_or(1).saturating_sub(1) as usize;
                    let take = n.map_or(usize::MAX, |n| n as usize);
                    text.split_inclusive('\n').skip(skip).take(take).collect()
                }
            };
            Ok(json!({"content": content}))
        }
        ClientOp::WriteFile {
            root,
            path,
            content,
        } => {
            let real = confined(&root, &path, false).await?;
            let old = old_text(&real).await;
            write_no_follow(&real, &content)
                .await
                .map_err(|e| format!("{path}: {e}"))?;
            Ok(json!({"old": old}))
        }
        ClientOp::TerminalCreate {
            terminal_id,
            program,
            args,
            env,
            cwd,
            byte_limit,
        } => {
            if TERMINALS
                .get(sid)
                .is_some_and(|terms| terms.len() >= MAX_TERMINALS)
            {
                return Err(format!("at most {MAX_TERMINALS} terminals at once"));
            }
            let mut cmd = tokio::process::Command::new(&program);
            cmd.args(&args)
                .envs(env)
                .current_dir(&cwd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .process_group(0)
                .kill_on_drop(true);
            let mut child = cmd
                .spawn()
                .map_err(|e| format!("could not start {program}: {e}"))?;
            let (tx, rx) = watch::channel(None);
            let t = Arc::new(Terminal {
                output: Mutex::new((String::new(), false)),
                exit: rx,
                pgid: child.id().map(|p| p as i32),
                gone: Default::default(),
            });
            let limit = output_limit(byte_limit);
            let pipes = [
                child
                    .stdout
                    .take()
                    .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
                child
                    .stderr
                    .take()
                    .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            ];
            TERMINALS
                .entry(sid.clone())
                .or_default()
                .insert(terminal_id.clone(), Arc::clone(&t));
            // The session closed (or its agent went) while this started: its
            // drop_session ran before the insert, so nothing else kills it.
            // Checked after the insert: a drop under the lock after it sees it.
            if live.lock().proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                if let Some(mut terms) = TERMINALS.get_mut(sid) {
                    terms.remove(&terminal_id);
                }
                kill(&t);
                return Err("the session ended".into());
            }
            let mut readers = Vec::new();
            for mut pipe in pipes.into_iter().flatten() {
                let (t, inner, live, tid) = (
                    Arc::clone(&t),
                    Arc::clone(inner),
                    Arc::clone(live),
                    terminal_id.clone(),
                );
                readers.push(tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    while let Ok(n) = pipe.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                        {
                            let mut o = t.output.lock().unwrap_or_else(|e| e.into_inner());
                            o.0.push_str(&chunk);
                            if o.0.len() > limit {
                                let mut cut = o.0.len() - limit;
                                while !o.0.is_char_boundary(cut) {
                                    cut += 1;
                                }
                                o.0.drain(..cut);
                                o.1 = true;
                            }
                        }
                        deliver(&inner, &live, epoch, |d| d.on_client_output(&tid, &chunk));
                    }
                }));
            }
            tokio::spawn(async move {
                // The command's own exit decides, not its pipes: a background
                // process it left (`npm run dev &`) keeps them open.
                let status = child.wait().await.ok();
                if group_gone(t.pgid) {
                    t.gone.store(true, std::sync::atomic::Ordering::Release);
                }
                let _ = tokio::time::timeout(std::time::Duration::from_millis(200), async {
                    for r in readers {
                        let _ = r.await;
                    }
                })
                .await;
                use std::os::unix::process::ExitStatusExt;
                let _ = tx.send(Some((
                    status.and_then(|s| s.code()),
                    status.and_then(|s| s.signal()).map(|s| s.to_string()),
                )));
            });
            Ok(json!({}))
        }
        ClientOp::TerminalOutput { terminal_id } => {
            let t = term(&terminal_id)?;
            let (output, truncated) = t.output.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let exit = t
                .exit
                .borrow()
                .clone()
                .map(|(code, signal)| json!({"exitCode": code, "signal": signal}));
            Ok(json!({"output": output, "truncated": truncated, "exitStatus": exit}))
        }
        ClientOp::TerminalWait { terminal_id } => {
            let mut rx = term(&terminal_id)?.exit.clone();
            let (code, signal) = rx
                .wait_for(Option::is_some)
                .await
                .map_err(|_| "the terminal went away".to_string())?
                .clone()
                .unwrap_or_default();
            Ok(json!({"exitCode": code, "signal": signal}))
        }
        ClientOp::TerminalKill { terminal_id } => {
            kill(&*term(&terminal_id)?);
            Ok(json!({}))
        }
        ClientOp::TerminalRelease { terminal_id } => {
            let released = TERMINALS
                .get_mut(sid)
                .and_then(|mut terms| terms.remove(&terminal_id));
            if let Some(t) = released {
                kill(&t);
            }
            Ok(json!({}))
        }
    }
}

/// The agent's `outputByteLimit`, never above Otto's own cap.
fn output_limit(asked: Option<u64>) -> usize {
    asked.map_or(DEFAULT_BYTE_LIMIT, |b| {
        usize::try_from(b)
            .unwrap_or(usize::MAX)
            .min(DEFAULT_BYTE_LIMIT)
    })
}

/// No process is left in the group `pgid` (`ESRCH` to a null signal).
fn group_gone(pgid: Option<i32>) -> bool {
    // SAFETY: signal 0 only checks that the group exists.
    pgid.is_some_and(|g| {
        (unsafe { libc::killpg(g, 0) }) == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    })
}

fn kill(t: &Terminal) {
    if t.gone.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    if let Some(pgid) = t.pgid {
        // SAFETY: a plain signal to the process group this terminal created.
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agents_output_limit_is_clamped_to_otto_s() {
        assert_eq!(output_limit(Some(u64::MAX)), DEFAULT_BYTE_LIMIT);
        assert_eq!(output_limit(Some(4096)), 4096);
        assert_eq!(output_limit(None), DEFAULT_BYTE_LIMIT);
    }

    /// Review R1-024: a write over a FIFO or an over-cap file answers at once
    /// without its old text (the open blocked on a FIFO for good).
    #[tokio::test]
    async fn a_write_reads_the_old_text_of_a_regular_capped_file_only() {
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("pipe");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a plain mkfifo on a path in a temp dir.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let old = tokio::time::timeout(std::time::Duration::from_secs(2), old_text(&fifo)).await;
        assert_eq!(old, Ok(None), "no blocking read of a FIFO");
        let big = root.path().join("big.log");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(old_text(&big).await, None);
        let small = root.path().join("a.txt");
        std::fs::write(&small, "hi").unwrap();
        assert_eq!(old_text(&small).await.as_deref(), Some("hi"));
        // Round 2: the write and the read themselves refuse it, at once.
        let quick = std::time::Duration::from_secs(2);
        let w = tokio::time::timeout(quick, write_no_follow(&fifo, "x")).await;
        assert!(w.expect("the write blocked on a FIFO").is_err());
        let r = tokio::time::timeout(quick, read_capped(&fifo, "pipe")).await;
        assert!(r.expect("the read blocked on a FIFO").is_err());
        assert!(read_capped(&root.path().join("."), ".").await.is_err());
        assert_eq!(read_capped(&small, "a.txt").await.as_deref(), Ok("hi"));
    }

    /// Review R1-049: a command's group is "gone" only once nothing is left
    /// in it, so a kill still reaches a background child it left.
    #[test]
    fn a_terminals_group_is_gone_only_when_empty() {
        use std::os::unix::process::CommandExt;
        let group = |script: &str| {
            let mut c = std::process::Command::new("/bin/sh");
            c.args(["-c", script]).process_group(0);
            let mut child = c.spawn().unwrap();
            let pgid = child.id() as i32;
            child.wait().unwrap();
            pgid
        };
        assert!(group_gone(Some(group("exit 0"))));
        let left = group("sleep 5 >/dev/null 2>&1 & exit 0");
        assert!(
            !group_gone(Some(left)),
            "its background child is still in it"
        );
        // SAFETY: the group this test just made.
        unsafe { libc::killpg(left, libc::SIGKILL) };
    }

    /// A dangling link in the folder points outside it: the write is
    /// refused and nothing lands at the link's target.
    #[tokio::test]
    async fn a_write_never_follows_a_dangling_symlink_out_of_the_folder() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("planted.plist");
        let link = root.path().join("a.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let root_s = root.path().canonicalize().unwrap().display().to_string();
        let path = format!("{root_s}/a.txt");
        let real = confined(&root_s, &path, false).await.unwrap();
        let err = write_no_follow(&real, "x").await.unwrap_err();
        assert!(err.to_string().contains("symbolic link"), "{err}");
        assert!(!target.exists(), "nothing written outside the folder");
        // A plain new file still works.
        let ok = confined(&root_s, &format!("{root_s}/b.txt"), false)
            .await
            .unwrap();
        write_no_follow(&ok, "hi").await.unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("b.txt")).unwrap(),
            "hi"
        );
        // The bytes are on disk when the write answers (tokio's File writes in
        // a blocking task; without a flush this read raced it: 1 run in ~25).
        let big = "y".repeat(4 << 20);
        write_no_follow(&ok, &big).await.unwrap();
        assert_eq!(
            std::fs::metadata(root.path().join("b.txt")).unwrap().len(),
            big.len() as u64
        );
    }
}
