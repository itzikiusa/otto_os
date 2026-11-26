//! Where a fake agent's stdio, arguments, environment, working directory and
//! pid come from: the real process for the `otto-fake-*` binaries, or — with
//! `test-util` — an in-memory child of [`crate::scripted`], which runs the same
//! program on a thread of the test process behind the runtime's
//! [`crate::process::ChildTransport`] seam. Every fake reads and writes through
//! here instead of `std::io` / `std::env` / `std::process`.

#![allow(dead_code)] // each binary uses the part its program needs

use std::cell::RefCell;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};

/// An in-memory child's world.
pub struct Io {
    pub args: Vec<String>,
    /// Looked up last-first, then the test process's own environment (what a
    /// spawned child would inherit).
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
    pub pid: u32,
    pub stdin: Mutex<Option<mpsc::Receiver<String>>>,
    pub stdout: Box<dyn Fn(String) + Send + Sync>,
}

thread_local! {
    static IO: RefCell<Option<Arc<Io>>> = const { RefCell::new(None) };
}

fn current() -> Option<Arc<Io>> {
    IO.with(|c| c.borrow().clone())
}

/// What an in-memory program unwinds with when it exits.
struct Exit(i32);

/// Run `main` on this thread as the in-memory child `io`; its exit code (101
/// for a panic, as a Rust process would).
pub fn run_as(io: Arc<Io>, main: fn()) -> i32 {
    IO.with(|c| *c.borrow_mut() = Some(io));
    let code = match std::panic::catch_unwind(main) {
        Ok(()) => 0,
        Err(p) => p.downcast_ref::<Exit>().map_or(101, |e| e.0),
    };
    IO.with(|c| *c.borrow_mut() = None);
    code
}

/// `std::thread::spawn`, carrying this child's world to the new thread.
pub fn spawn(f: impl FnOnce() + Send + 'static) -> std::thread::JoinHandle<()> {
    let io = current();
    std::thread::spawn(move || {
        IO.with(|c| *c.borrow_mut() = io);
        f()
    })
}

/// `std::process::exit`; an in-memory child ends its program instead.
pub fn exit(code: i32) -> ! {
    if current().is_some() {
        std::panic::resume_unwind(Box::new(Exit(code)));
    }
    std::process::exit(code)
}

/// One stdout line.
pub fn out(line: &str) {
    match current() {
        Some(io) => (io.stdout)(line.to_string()),
        None => {
            let mut o = std::io::stdout().lock();
            let _ = writeln!(o, "{line}");
            let _ = o.flush();
        }
    }
}

/// stdin's lines until EOF (take it once, then move it to a reader thread).
pub fn stdin_lines() -> Box<dyn Iterator<Item = String> + Send> {
    if let Some(io) = current() {
        let rx = io.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
        return Box::new(rx.into_iter().flatten());
    }
    Box::new(std::iter::from_fn(|| {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
        }
    }))
}

pub fn args() -> Vec<String> {
    current().map_or_else(|| std::env::args().collect(), |io| io.args.clone())
}

pub fn var(key: &str) -> Option<String> {
    current()
        .and_then(|io| {
            io.env
                .iter()
                .rev()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        })
        .or_else(|| std::env::var(key).ok())
}

pub fn cwd() -> PathBuf {
    current().map_or_else(
        || std::env::current_dir().unwrap_or_default(),
        |io| io.cwd.clone(),
    )
}

pub fn pid() -> u32 {
    current().map_or_else(std::process::id, |io| io.pid)
}
