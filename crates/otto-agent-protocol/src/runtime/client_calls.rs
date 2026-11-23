//! Runs the ACP client calls a driver hands over ([`DriverEvent::ClientCall`]:
//! `fs/*`, `terminal/*`) off the session lock and feeds the result back.
//! Files stay inside the session folder (canonical paths, so no `..` or
//! symlink escapes); a terminal's argv already carries the agent's own
//! sandbox wrapper. Terminal output is kept up to its byte limit and also
//! streamed into the tool row as it arrives.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use dashmap::DashMap;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

use super::{apply_step, Inner, Live, State};
use crate::driver::ClientOp;
use otto_core::Id;

/// Open client terminals per session; at most [`MAX_TERMINALS`] each.
static TERMINALS: LazyLock<DashMap<(Id, String), Arc<Terminal>>> = LazyLock::new(DashMap::new);
const MAX_TERMINALS: usize = 16;
const DEFAULT_BYTE_LIMIT: usize = 1 << 20;
const MAX_FILE_BYTES: u64 = 8 << 20;

struct Terminal {
    output: Mutex<(String, bool)>,
    exit: watch::Receiver<Option<(Option<i32>, Option<String>)>>,
    child: Mutex<Option<tokio::process::Child>>,
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

/// The session's terminals end with it.
pub(super) fn drop_session(sid: &Id) {
    TERMINALS.retain(|(s, _), _| s != sid);
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
/// write outside it. `O_NOFOLLOW` also closes the check-then-write race.
async fn write_no_follow(path: &Path, content: &str) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    if tokio::fs::symlink_metadata(path)
        .await
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        return Err(std::io::Error::other("is a symbolic link; not followed"));
    }
    let mut f = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .await?;
    f.write_all(content.as_bytes()).await
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
            .get(&(sid.clone(), tid.to_string()))
            .map(|t| Arc::clone(&t))
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
            let len = tokio::fs::metadata(&real)
                .await
                .map_err(|e| e.to_string())?
                .len();
            if len > MAX_FILE_BYTES {
                return Err(format!("{path} is larger than {} MB", MAX_FILE_BYTES >> 20));
            }
            let text = tokio::fs::read_to_string(&real)
                .await
                .map_err(|e| format!("{path}: {e}"))?;
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
            let old = tokio::fs::read_to_string(&real).await.ok();
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
            if TERMINALS.iter().filter(|e| &e.key().0 == sid).count() >= MAX_TERMINALS {
                return Err(format!("at most {MAX_TERMINALS} terminals at once"));
            }
            let mut cmd = tokio::process::Command::new(&program);
            cmd.args(&args)
                .envs(env)
                .current_dir(&cwd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            let mut child = cmd
                .spawn()
                .map_err(|e| format!("could not start {program}: {e}"))?;
            let (tx, rx) = watch::channel(None);
            let t = Arc::new(Terminal {
                output: Mutex::new((String::new(), false)),
                exit: rx,
                child: Mutex::new(None),
            });
            let limit = byte_limit.map_or(DEFAULT_BYTE_LIMIT, |b| b as usize);
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
            *t.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
            TERMINALS.insert((sid.clone(), terminal_id.clone()), Arc::clone(&t));
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
            let waiter = Arc::clone(&t);
            tokio::spawn(async move {
                for r in readers {
                    let _ = r.await;
                }
                let child = waiter
                    .child
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                let status = match child {
                    Some(mut c) => c.wait().await.ok(),
                    None => None,
                };
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
            if let Some((_, t)) = TERMINALS.remove(&(sid.clone(), terminal_id)) {
                kill(&t);
            }
            Ok(json!({}))
        }
    }
}

fn kill(t: &Terminal) {
    if let Some(c) = t.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = c.start_kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
