//! The `OTTO_FAKE:` script protocol every fake agent acts out (design:
//! workflows-on-threads runtime §8, test plan §11.1). One parser and one
//! interpreter shared by `otto-fake-claude`, `otto-fake-codex`,
//! `otto-fake-agent` (every ACP flavour), `otto-fake-pi` (each includes this
//! file with `#[path]`) and the in-process [`crate::scripted::ScriptedDriver`].
//!
//! A prompt with `OTTO_FAKE: <verb> <args>` lines anywhere runs those verbs in
//! order; a prompt without one keeps the fake's exact-text behaviour, so no
//! existing test changes. The verbs:
//!
//! * `write <path> <text | @filler:N>` — `<path>` relative to the cwd;
//!   `$HANDOFF` is the step's handoff file and `$CTX/` the run's context
//!   directory, both read from the engine's preamble.
//! * `expect <path>` — fail the turn "missing <path>" (`*` matches in the
//!   file name; `KEY` in a `jira-KEY.md` too).
//! * `sleep <ms>`, `stream <bytes> <ms>`, `reply <text>`, `usage <in> <out>`.
//! * `ask <question>` (answered: "answer: <text>"), `approve <tool>`
//!   ("approved: <tool>" / "denied: <tool>").
//! * `fail provider|auth|usage_limit|crash|hang` (`hang`: ends only when
//!   interrupted); `fail-attempts <n> <class>` fails the first `n` turns of
//!   the same handoff (or session).
//! * `background <ms>` — the turn ends with background work running; `<ms>`
//!   later the CLI starts its own follow-up turn, which runs the verbs after it.
//! * `child <provider> <script;;script>` — `otto_thread_open` through the
//!   Otto MCP server the fake was given; `finish-child` —
//!   `otto_thread_close {remove: true}` on that child.
//! * `remember <token>` / `recall` — kept per provider session (survives a
//!   resume); `rss <MB>` — hold that much resident memory until exit.
//! * `@script` — the step's script from `$OTTO_FAKE_SCRIPTS/<slug>.txt`
//!   (`<slug>` = the handoff file's name without `step<N>-`, `-iter<N>` and
//!   `.md`); a step with no script file acts as the step persona.
//!
//! A verb this fake cannot act out answers "unsupported: <verb>" and ends the
//! turn, so a test that asks for it fails loudly. A `References:` line in a
//! scripted prompt adds an `expect $CTX/<file>` per file it names.
//!
//! Personas for prompts the engine writes itself: `OTTO_FAKE_DEFAULT=step` — a
//! workflow step prompt without a sentinel `expect`s every context file it
//! names and writes `$HANDOFF`; `OTTO_FAKE_REVIEW_DIR` — a scope, reviewer or
//! summarizer prompt writes its result file from
//! `<dir>/<role>[-<lens>-<provider>].json` (or an empty valid one).
//!
//! Environment: `OTTO_FAKE_TIME_SCALE` multiplies every `sleep`, `stream` and
//! `background`; `OTTO_FAKE_LOG` appends a JSON line per turn
//! `{pid, session, turn, t_start, t_end, verbs}`; `OTTO_FAKE_ARGV_LOG` a line
//! per start (its argv) and per model / effort the protocol sets;
//! `OTTO_FAKE_STATE_DIR` holds `remember` and `fail-attempts` state (default
//! `$TMPDIR/otto-fake-state`).

#![allow(dead_code)] // each binary uses the part its protocol needs

use std::collections::VecDeque;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub const SENTINEL: &str = "OTTO_FAKE:";

/// What `fail` makes the turn end with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailClass {
    Provider,
    Auth,
    UsageLimit,
    Crash,
    Hang,
}

impl FailClass {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "provider" => Self::Provider,
            "auth" => Self::Auth,
            "usage_limit" => Self::UsageLimit,
            "crash" => Self::Crash,
            "hang" => Self::Hang,
            _ => return None,
        })
    }

    /// The provider's error text, worded so `otto_core::agent_runtime::error_class`
    /// reads the class from it.
    pub fn message(self) -> &'static str {
        match self {
            Self::Provider => "API Error: 500 Internal server error (fake)",
            Self::Auth => "Not logged in · Please run /login",
            Self::UsageLimit => "You've hit your usage limit · resets 3pm",
            Self::Crash => "crashing on purpose",
            Self::Hang => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    Write {
        path: String,
        text: String,
    },
    Expect(String),
    Sleep(u64),
    Stream {
        bytes: usize,
        ms: u64,
    },
    Reply(String),
    Ask(String),
    Approve(String),
    Fail(FailClass),
    FailAttempts {
        n: u32,
        class: FailClass,
    },
    Background(u64),
    Child {
        provider: String,
        scripts: Vec<String>,
    },
    FinishChild,
    Usage {
        input: u64,
        output: u64,
    },
    Remember(String),
    Recall,
    Rss(u64),
    Script,
    Unsupported(String),
}

impl Act {
    /// The verb as written (the turn log lists them).
    pub fn verb(&self) -> &str {
        match self {
            Self::Write { .. } => "write",
            Self::Expect(_) => "expect",
            Self::Sleep(_) => "sleep",
            Self::Stream { .. } => "stream",
            Self::Reply(_) => "reply",
            Self::Ask(_) => "ask",
            Self::Approve(_) => "approve",
            Self::Fail(_) => "fail",
            Self::FailAttempts { .. } => "fail-attempts",
            Self::Background(_) => "background",
            Self::Child { .. } => "child",
            Self::FinishChild => "finish-child",
            Self::Usage { .. } => "usage",
            Self::Remember(_) => "remember",
            Self::Recall => "recall",
            Self::Rss(_) => "rss",
            Self::Script => "@script",
            Self::Unsupported(v) => v,
        }
    }
}

/// One verb line (what follows `OTTO_FAKE:`).
pub fn parse_act(line: &str) -> Act {
    let line = line.trim();
    let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    let mut words = rest.split_whitespace();
    let mut num = || words.next().and_then(|w| w.parse::<u64>().ok());
    let bad = || Act::Unsupported(line.to_string());
    match verb {
        "write" => match rest.split_once(char::is_whitespace) {
            Some((path, text)) => Act::Write {
                path: path.into(),
                text: text.trim().into(),
            },
            None if !rest.is_empty() => Act::Write {
                path: rest.into(),
                text: String::new(),
            },
            None => bad(),
        },
        "expect" if !rest.is_empty() => Act::Expect(rest.into()),
        "sleep" => num().map_or_else(bad, Act::Sleep),
        "stream" => match (num(), num()) {
            (Some(bytes), Some(ms)) => Act::Stream {
                bytes: bytes as usize,
                ms,
            },
            _ => bad(),
        },
        "reply" => Act::Reply(rest.into()),
        "ask" => Act::Ask(rest.into()),
        "approve" => Act::Approve(if rest.is_empty() { "Bash" } else { rest }.into()),
        "fail" => FailClass::parse(rest).map_or_else(bad, Act::Fail),
        "fail-attempts" => match rest.split_once(char::is_whitespace) {
            Some((n, class)) => match (n.parse(), FailClass::parse(class.trim())) {
                (Ok(n), Some(class)) => Act::FailAttempts { n, class },
                _ => bad(),
            },
            None => bad(),
        },
        "background" => num().map_or_else(bad, Act::Background),
        "child" => match rest.split_once(char::is_whitespace) {
            Some((provider, scripts)) => Act::Child {
                provider: provider.into(),
                scripts: scripts
                    .split(";;")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
            },
            None => bad(),
        },
        "finish-child" => Act::FinishChild,
        "usage" => match (num(), num()) {
            (Some(input), Some(output)) => Act::Usage { input, output },
            _ => bad(),
        },
        "remember" if !rest.is_empty() => Act::Remember(rest.into()),
        "recall" => Act::Recall,
        "rss" => num().map_or_else(bad, Act::Rss),
        "@script" => Act::Script,
        _ => bad(),
    }
}

/// Every `OTTO_FAKE:` line of `prompt`, in order; `None` when it has none.
pub fn parse(prompt: &str) -> Option<Vec<Act>> {
    let acts: Vec<Act> = prompt
        .lines()
        .filter_map(|l| l.trim().strip_prefix(SENTINEL))
        .map(parse_act)
        .collect();
    (!acts.is_empty()).then_some(acts)
}

/// A script file: one verb per line, with or without the sentinel.
pub fn parse_script(text: &str) -> Vec<Act> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| parse_act(l.strip_prefix(SENTINEL).unwrap_or(l)))
        .collect()
}

/// The run's context directory, from the engine's preamble ("Context directory: <dir>").
pub fn context_dir(prompt: &str) -> Option<String> {
    prompt
        .lines()
        .find_map(|l| l.strip_prefix("Context directory: "))
        .map(|d| d.trim().to_string())
}

/// The step's handoff file: the path ending the line after "[your handoff — required]".
pub fn handoff_path(prompt: &str) -> Option<String> {
    let mut lines = prompt
        .lines()
        .skip_while(|l| !l.contains("[your handoff — required]"));
    lines.next()?;
    let line = lines.next()?;
    line.rsplit_once(" to: ").map(|(_, p)| p.trim().to_string())
}

/// The handoff's script key: `step3-gather-info-iter2.md` → `gather-info`.
pub fn handoff_slug(handoff: &str) -> String {
    let name = Path::new(handoff)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let stem = match stem.strip_prefix("step").and_then(|s| s.split_once('-')) {
        Some((n, rest)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => rest,
        _ => stem,
    };
    match stem.rsplit_once("-iter") {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => stem,
    }
    .to_string()
}

/// The context files a prompt names (the step persona `expect`s them).
pub fn contract_files(text: &str) -> Vec<String> {
    const NAMED: [&str; 7] = [
        "run-brief.md",
        "repos.json",
        "prs.json",
        "instructions.md",
        "prompt.md",
        "previous-review.md",
        "final-output.md",
    ];
    let step_md = |t: &str| {
        t.strip_prefix("step")
            .and_then(|r| r.split_once('-'))
            .is_some_and(|(n, rest)| {
                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && rest.ends_with(".md")
            })
    };
    let mut out: Vec<String> = Vec::new();
    for t in text.split(|c: char| c.is_whitespace() || ",;()[]`'\"".contains(c)) {
        let t = t.trim_end_matches(['.', ':']);
        let t = t.strip_prefix("$CTX/").unwrap_or(t);
        let keep = NAMED.contains(&t)
            || step_md(t)
            || (t.ends_with(".output.json") && !t.contains('/'))
            || (t.starts_with("jira-") && t.ends_with(".md"));
        if keep && !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// What a fake is configured with (the binaries read it from the environment).
#[derive(Debug, Clone)]
pub struct Config {
    /// The provider as the review persona's file names spell it.
    pub provider: String,
    pub time_scale: f64,
    pub scripts_dir: Option<PathBuf>,
    pub step_persona: bool,
    pub review_dir: Option<PathBuf>,
    pub state_dir: PathBuf,
    /// Verbs this fake cannot act out.
    pub unsupported: &'static [&'static str],
    /// Where turn records go (`OTTO_FAKE_LOG`); `None` = nowhere.
    pub log: Option<PathBuf>,
}

impl Config {
    pub fn from_env(provider: &str, unsupported: &'static [&'static str]) -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self {
            provider: provider.into(),
            time_scale: var("OTTO_FAKE_TIME_SCALE")
                .and_then(|v| v.parse().ok())
                .filter(|s: &f64| s.is_finite() && *s >= 0.0)
                .unwrap_or(1.0),
            scripts_dir: var("OTTO_FAKE_SCRIPTS").map(PathBuf::from),
            step_persona: var("OTTO_FAKE_DEFAULT").as_deref() == Some("step"),
            review_dir: var("OTTO_FAKE_REVIEW_DIR").map(PathBuf::from),
            state_dir: var("OTTO_FAKE_STATE_DIR").map_or_else(
                || std::env::temp_dir().join("otto-fake-state"),
                PathBuf::from,
            ),
            unsupported,
            log: var("OTTO_FAKE_LOG").map(PathBuf::from),
        }
    }

    pub fn scaled(&self, ms: u64) -> Duration {
        Duration::from_secs_f64(ms as f64 / 1000.0 * self.time_scale)
    }
}

/// Append one JSON line to `path` (the logs; several fakes share one file).
pub fn append_line(path: &Path, v: &Value) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{v}");
    }
}

/// `OTTO_FAKE_ARGV_LOG`: a line about this process (its argv at start, a
/// model or effort the protocol set later).
pub fn argv_log(mut v: Value) {
    if let Some(path) = std::env::var_os("OTTO_FAKE_ARGV_LOG") {
        v["pid"] = json!(std::process::id());
        append_line(Path::new(&path), &v);
    }
}

/// `OTTO_FAKE_ARGV_LOG` for a starting binary.
pub fn log_start() {
    argv_log(json!({"argv": std::env::args().collect::<Vec<_>>()}));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// `n` bytes of neutral text.
pub fn filler(n: usize) -> String {
    const WORDS: &str =
        "the quick brown fox jumps over the lazy dog while the step writes its summary ";
    WORDS.chars().cycle().take(n).collect()
}

static HELD: std::sync::Mutex<Vec<Vec<u8>>> = std::sync::Mutex::new(Vec::new());

fn hold_memory(mb: u64) {
    // Touch every page so it is resident, not just reserved.
    let block = vec![1u8; (mb as usize) << 20];
    HELD.lock().unwrap_or_else(|e| e.into_inner()).push(block);
}

fn key_file(dir: &Path, prefix: &str, key: &str) -> PathBuf {
    // FNV-1a: a short, stable file name for any key.
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    dir.join(format!("{prefix}-{h:016x}"))
}

/// What the wire (the fake's protocol) must do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Say(String),
    Wait(Duration),
    /// Answer with [`Turn::answered`].
    Ask(String),
    /// Answer with [`Turn::approved`].
    Approve(String),
    Usage {
        input: u64,
        output: u64,
    },
    /// End the turn with this failure (`Crash`: the process dies; `Hang`: the
    /// turn ends only when interrupted).
    Fail(FailClass, String),
    /// End the turn now with background work running; after the wait start
    /// the CLI's own follow-up turn, which goes on with this same [`Turn`].
    Background(Duration),
    /// Call an Otto MCP tool; answer with [`Turn::mcp_answered`].
    Mcp {
        tool: String,
        args: Value,
    },
    Done,
}

/// One turn of a script, stepped by the fake.
#[derive(Debug)]
pub struct Turn {
    acts: VecDeque<Act>,
    cfg: Config,
    cwd: PathBuf,
    handoff: Option<String>,
    ctx: Option<String>,
    session: String,
    turn: u32,
    t_start: u64,
    verbs: Vec<String>,
    child: Option<String>,
    /// The MCP call in flight was `child` (its answer names the thread).
    opening: bool,
}

impl Turn {
    /// The script `prompt` asks for; `None` = not scripted (exact-text behaviour).
    pub fn start(prompt: &str, cwd: &Path, session: &str, turn: u32, cfg: &Config) -> Option<Self> {
        let handoff = handoff_path(prompt);
        let ctx = context_dir(prompt);
        let step_persona = |h: &str| {
            let mut acts: Vec<Act> = contract_files(prompt)
                .into_iter()
                .filter(|f| !h.ends_with(&format!("/{f}")))
                .map(|f| Act::Expect(format!("$CTX/{f}")))
                .collect();
            acts.push(Act::Write {
                path: "$HANDOFF".into(),
                text: "@filler:400".into(),
            });
            acts.push(Act::Reply("step done".into()));
            acts
        };
        let acts = match parse(prompt) {
            Some(acts) => {
                let mut out: Vec<Act> = references(prompt)
                    .into_iter()
                    .map(|f| Act::Expect(format!("$CTX/{f}")))
                    .collect();
                for act in acts {
                    if act != Act::Script {
                        out.push(act);
                        continue;
                    }
                    let file = handoff
                        .as_deref()
                        .zip(cfg.scripts_dir.as_deref())
                        .map(|(h, d)| d.join(format!("{}.txt", handoff_slug(h))));
                    match file.and_then(|f| std::fs::read_to_string(f).ok()) {
                        Some(text) => out.extend(parse_script(&text)),
                        None => match handoff.as_deref() {
                            Some(h) => out.extend(step_persona(h)),
                            None => out.push(Act::Unsupported("@script without a handoff".into())),
                        },
                    }
                }
                out
            }
            None => match review_acts(prompt, cfg) {
                Some(acts) => acts,
                None => step_persona(handoff.as_deref().filter(|_| cfg.step_persona)?),
            },
        };
        let acts = acts
            .into_iter()
            .map(|a| {
                if cfg.unsupported.contains(&a.verb()) {
                    Act::Unsupported(a.verb().to_string())
                } else {
                    a
                }
            })
            .collect();
        Some(Self {
            acts,
            cfg: cfg.clone(),
            cwd: cwd.to_path_buf(),
            handoff,
            ctx,
            session: session.into(),
            turn,
            t_start: now_ms(),
            verbs: Vec::new(),
            child: None,
            opening: false,
        })
    }

    /// The follow-up turn of a [`Step::Background`] starts.
    pub fn resumed(&mut self, turn: u32) {
        self.turn = turn;
        self.t_start = now_ms();
        self.verbs.clear();
    }

    fn path(&self, p: &str) -> Result<PathBuf, String> {
        if p == "$HANDOFF" {
            return self
                .handoff
                .as_deref()
                .map(PathBuf::from)
                .ok_or_else(|| "no handoff file in the prompt".to_string());
        }
        if let Some(rest) = p.strip_prefix("$CTX/") {
            return self
                .ctx
                .as_deref()
                .map(|d| Path::new(d).join(rest))
                .ok_or_else(|| "no context directory in the prompt".to_string());
        }
        Ok(self.cwd.join(p))
    }

    fn state_key(&self) -> &str {
        self.handoff.as_deref().unwrap_or(&self.session)
    }

    /// The next thing the wire must do. Local verbs (files, memory, counters)
    /// run here.
    pub fn next_step(&mut self) -> Step {
        loop {
            let Some(act) = self.acts.pop_front() else {
                return Step::Done;
            };
            self.verbs.push(act.verb().to_string());
            match act {
                Act::Write { path, text } => {
                    let path = match self.path(&path) {
                        Ok(p) => p,
                        Err(e) => return self.fail(FailClass::Provider, e),
                    };
                    let text = match text.strip_prefix("@filler:").map(str::parse::<usize>) {
                        Some(Ok(n)) => filler(n),
                        _ => text,
                    };
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    // Whole or not at all: the engine may be watching the file.
                    let tmp = path.with_extension("otto-fake.tmp");
                    if let Err(e) =
                        std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path))
                    {
                        return self.fail(
                            FailClass::Provider,
                            format!("write {}: {e}", path.display()),
                        );
                    }
                }
                Act::Expect(p) => {
                    let found = self.path(&p).is_ok_and(|path| exists_like(&path));
                    if !found {
                        return self.fail(FailClass::Provider, format!("missing {p}"));
                    }
                }
                Act::Sleep(ms) => return Step::Wait(self.cfg.scaled(ms)),
                Act::Stream { bytes, ms } => {
                    // Ten chunks spread over `ms` (fewer for a tiny stream).
                    let n = bytes.clamp(1, 10);
                    for i in (0..n).rev() {
                        let len = bytes / n + usize::from(i < bytes % n);
                        self.acts.push_front(Act::Reply(filler(len)));
                        self.acts.push_front(Act::Sleep(ms / n as u64));
                    }
                }
                Act::Reply(text) => return Step::Say(text),
                Act::Ask(q) => return Step::Ask(q),
                Act::Approve(tool) => return Step::Approve(tool),
                Act::Fail(class) => {
                    let msg = class.message().to_string();
                    return self.fail(class, msg);
                }
                Act::FailAttempts { n, class } => {
                    let _ = std::fs::create_dir_all(&self.cfg.state_dir);
                    let file = key_file(&self.cfg.state_dir, "attempts", self.state_key());
                    let seen: u32 = std::fs::read_to_string(&file)
                        .ok()
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(0);
                    if seen < n {
                        let _ = std::fs::write(&file, (seen + 1).to_string());
                        let msg = class.message().to_string();
                        return self.fail(class, msg);
                    }
                }
                Act::Background(ms) => return Step::Background(self.cfg.scaled(ms)),
                Act::Child { provider, scripts } => {
                    let prompt = scripts
                        .iter()
                        .map(|s| format!("{SENTINEL} {s}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.opening = true;
                    return Step::Mcp {
                        tool: "otto_thread_open".into(),
                        args: json!({"provider": provider, "prompt": prompt}),
                    };
                }
                Act::FinishChild => {
                    let Some(id) = self.child.clone() else {
                        return self
                            .fail(FailClass::Provider, "finish-child: no child opened".into());
                    };
                    return Step::Mcp {
                        tool: "otto_thread_close".into(),
                        args: json!({"peer_session_id": id, "remove": true}),
                    };
                }
                Act::Usage { input, output } => return Step::Usage { input, output },
                Act::Remember(token) => {
                    let _ = std::fs::create_dir_all(&self.cfg.state_dir);
                    let _ = std::fs::write(
                        key_file(&self.cfg.state_dir, "memory", &self.session),
                        token,
                    );
                }
                Act::Recall => {
                    let token = std::fs::read_to_string(key_file(
                        &self.cfg.state_dir,
                        "memory",
                        &self.session,
                    ))
                    .unwrap_or_default();
                    return Step::Say(format!("recall: {token}"));
                }
                Act::Rss(mb) => hold_memory(mb),
                Act::Script => {}
                Act::Unsupported(verb) => {
                    self.acts.clear();
                    return Step::Say(format!("unsupported: {verb}"));
                }
            }
        }
    }

    fn fail(&mut self, class: FailClass, message: String) -> Step {
        self.acts.clear();
        Step::Fail(class, message)
    }

    pub fn answered(&mut self, text: &str) {
        self.acts.push_front(Act::Reply(format!("answer: {text}")));
    }

    pub fn approved(&mut self, tool: &str, allowed: bool) {
        let word = if allowed { "approved" } else { "denied" };
        self.acts.push_front(Act::Reply(format!("{word}: {tool}")));
    }

    /// The MCP call's outcome (the tool result's `content`, or why it failed).
    pub fn mcp_answered(&mut self, result: Result<Value, String>) {
        let opening = std::mem::take(&mut self.opening);
        let text = match result {
            Ok(v) => {
                let text: String = v["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["text"].as_str())
                    .collect();
                if v["isError"].as_bool() == Some(true) {
                    format!("mcp error: {text}")
                } else if opening {
                    match thread_id(&text) {
                        Some(id) => {
                            let line = format!("opened {id}");
                            self.child = Some(id);
                            line
                        }
                        None => format!("mcp error: no thread id in {text}"),
                    }
                } else {
                    format!("closed {}", self.child.take().unwrap_or_default())
                }
            }
            Err(e) => format!("mcp error: {e}"),
        };
        self.acts.push_front(Act::Reply(text));
    }

    /// `OTTO_FAKE_LOG`'s record of this turn (written by [`Self::log`]).
    pub fn record(&self) -> Value {
        json!({"pid": std::process::id(), "session": self.session, "turn": self.turn,
               "t_start": self.t_start, "t_end": now_ms(), "verbs": self.verbs})
    }

    /// The turn (or the part before a follow-up) ended: log it.
    pub fn log(&self) {
        if let Some(path) = &self.cfg.log {
            append_line(path, &self.record());
        }
    }
}

/// The `References:` line's context files (scrubbed workflow prompts).
fn references(prompt: &str) -> Vec<String> {
    prompt
        .lines()
        .find_map(|l| l.trim().strip_prefix("References:"))
        .map(contract_files)
        .unwrap_or_default()
}

/// A path that exists, `*` (and a `jira-KEY` key) matching any run of
/// characters in the file name.
#[allow(clippy::disallowed_methods)] // a test double listing one small context dir
fn exists_like(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let pattern = name.replace("KEY", "*");
    let Some((head, tail)) = pattern.split_once('*') else {
        return path.exists();
    };
    let tail = tail.replace('*', "");
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.len() >= head.len() + tail.len() && n.starts_with(head) && n.ends_with(&tail)
        })
}

/// The new thread's id in an `otto_thread_open` answer.
fn thread_id(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    [
        "/peer_session_id",
        "/session_id",
        "/id",
        "/session/id",
        "/thread/id",
    ]
    .iter()
    .find_map(|p| v.pointer(p).and_then(Value::as_str))
    .map(str::to_string)
}

/// The review engine's own prompts: where the result goes and what to write.
fn review_acts(prompt: &str, cfg: &Config) -> Option<Vec<Act>> {
    let dir = cfg.review_dir.as_deref()?;
    // A long prompt rides a file; the typed text names it.
    let typed = prompt;
    let file_text = typed
        .split_once("Your complete instructions for this task are in the file ")
        .and_then(|(_, r)| r.split_once(" ("))
        .and_then(|(p, _)| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let all = format!("{typed}\n{file_text}");
    let after = |marker: &str| {
        all.split_once(marker).and_then(|(_, r)| {
            r.split_whitespace()
                .next()
                .map(|p| p.trim_end_matches(['.', ',']).to_string())
        })
    };
    let (role, path) = if let Some(p) = after(otto_core::finding::FINDINGS_PATH_INTRO)
        .or_else(|| after("write the merged array to:"))
    {
        let scope = all.contains("Assess review scope BEFORE any reviewers run");
        (if scope { "scope" } else { "reviewer" }, p)
    } else {
        ("summarizer", after("first, then atomically rename it to ")?)
    };
    let lens = all
        .split_once("Use the `")
        .and_then(|(_, r)| r.split_once('`'))
        .map(|(l, _)| l.to_string());
    let named = lens
        .map(|l| dir.join(format!("{role}-{l}-{}.json", cfg.provider)))
        .filter(|f| f.exists())
        .unwrap_or_else(|| dir.join(format!("{role}.json")));
    let text = std::fs::read_to_string(named).unwrap_or_else(|_| match role {
        "scope" => json!([{"severity": "info", "body": json!({
            "summary": "fake scope", "related_modules": [], "risks": [], "lenses": []}).to_string()}])
        .to_string(),
        _ => "[]".to_string(),
    });
    Some(vec![
        Act::Write { path, text },
        Act::Reply(format!("{role} done")),
    ])
}

/// The blocking side of a binary fake: how it acts each [`Step`] out on its
/// protocol. A `false` / `None` answer means the turn was interrupted meanwhile
/// (the wire already ended it).
pub trait Wire {
    fn say(&mut self, text: &str);
    fn wait(&mut self, d: Duration) -> bool;
    fn ask(&mut self, question: &str) -> Option<String>;
    fn approve(&mut self, tool: &str) -> Option<bool>;
    fn usage(&mut self, input: u64, output: u64);
    /// End the turn with `Provider` / `Auth` / `UsageLimit`.
    fn fail(&mut self, class: FailClass, message: &str);
    /// End the turn with background work running; owe the follow-up turn
    /// (running `rest` through [`drive`]) after `after`.
    fn background(&mut self, after: Duration, rest: Turn);
    fn mcp(&mut self, tool: &str, args: Value) -> Result<Value, String>;
    fn done(&mut self);
}

/// Act `turn` out on `wire` until it ends, is interrupted or goes to the
/// background. A turn is logged before the wire ends it, so a test that reads
/// the log after the turn's end finds it there.
pub fn drive(mut turn: Turn, wire: &mut impl Wire) {
    loop {
        match turn.next_step() {
            Step::Say(t) => wire.say(&t),
            Step::Wait(d) => {
                if !wire.wait(d) {
                    break;
                }
            }
            Step::Ask(q) => match wire.ask(&q) {
                Some(a) => turn.answered(&a),
                None => break,
            },
            Step::Approve(tool) => match wire.approve(&tool) {
                Some(ok) => turn.approved(&tool, ok),
                None => break,
            },
            Step::Usage { input, output } => wire.usage(input, output),
            Step::Fail(FailClass::Crash, _) => {
                turn.log();
                eprintln!("fake agent: crashing on purpose");
                std::process::exit(3);
            }
            Step::Fail(FailClass::Hang, _) => {
                while wire.wait(Duration::from_secs(3600)) {}
                break;
            }
            Step::Fail(class, message) => {
                turn.log();
                return wire.fail(class, &message);
            }
            Step::Background(after) => {
                turn.log();
                return wire.background(after, turn);
            }
            Step::Mcp { tool, args } => {
                let res = wire.mcp(&tool, args);
                turn.mcp_answered(res);
            }
            Step::Done => {
                turn.log();
                return wire.done();
            }
        }
    }
    turn.log();
}

/// One stdio MCP call (`initialize`, `notifications/initialized`,
/// `tools/call`) against a server started from `command`.
pub fn mcp_call(
    command: &str,
    args: &[String],
    env: &[(String, String)],
    tool: &str,
    arguments: Value,
) -> Result<Value, String> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let mut child = Command::new(command)
        .args(args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {command}: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("no stdout")?).lines();
    let mut ask = |id: u64, method: &str, params: Value| -> Result<Value, String> {
        if method.starts_with("notifications/") {
            let note = json!({"jsonrpc":"2.0","method":method});
            return writeln!(stdin, "{note}")
                .map(|_| Value::Null)
                .map_err(|e| e.to_string());
        }
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        writeln!(stdin, "{req}").map_err(|e| e.to_string())?;
        loop {
            let line = lines
                .next()
                .ok_or("server closed")?
                .map_err(|e| e.to_string())?;
            let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            if v["id"] == json!(id) {
                return match v.get("error") {
                    Some(e) => Err(e.to_string()),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    };
    let hello = json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"otto-fake","version":"0"}});
    ask(1, "initialize", hello)?;
    ask(0, "notifications/initialized", Value::Null)?;
    let res = ask(
        2,
        "tools/call",
        json!({"name": tool, "arguments": arguments}),
    );
    let _ = child.kill();
    let _ = child.wait();
    res
}
