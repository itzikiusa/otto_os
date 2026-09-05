//! Nested-agent detection for plain `shell` sessions.
//!
//! A `shell` session is a bare login shell with no `resume_args` (see
//! [`crate::providers`]), so historically it was non-resumable by
//! construction: once its PTY died (daemon restart, app quit, `exit`), opening
//! it again hit `ensure_live`'s no-op branch and the user was left staring at a
//! dead terminal — "this session doesn't exist any more".
//!
//! But a terminal in which the user typed `claude` (or `codex` / `agy`) is no
//! longer a bare shell: its real state is that agent's conversation, and *that*
//! IS persistent — the transcript sits on disk exactly like a first-class Otto
//! agent session's. This module finds it:
//!
//! 1. walk the PTY's descendant processes for one of the known agent CLIs
//!    ([`agent_from_args`]);
//! 2. ask the OS for that process's real cwd ([`process_cwd`]) — the user may
//!    have `cd`-ed before launching, and every provider files its transcript
//!    under the cwd it was launched from;
//! 3. match the transcript that was BORN inside the launch window
//!    ([`claude_transcript_in_window`] for claude; the manager reuses the
//!    existing codex/agy scanners for those).
//!
//! The manager persists the result as the session's `provider_session_id` +
//! `meta.nested_*`, so reopening respawns the shell and types the provider's
//! own resume command ([`resume_command`]) back into it.
//!
//! Same "never guess" rule as the codex rollout capture: a window with more
//! than one unclaimed candidate is refused outright rather than risk adopting
//! (and forking) somebody else's conversation.

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Agent CLIs we recognize inside a shell session. Ordered by how specific the
/// name is; all three mint their own resumable conversation on disk.
pub const NESTED_PROVIDERS: [&str; 3] = ["claude", "codex", "agy"];

/// How long before a nested agent's process start a transcript may have been
/// created and still count as "this launch" (clock skew + the provider writing
/// its file a moment before we sample `etime`).
const BORN_BEFORE_SLACK: Duration = Duration::from_secs(15);

/// Slack on the upper bound of the birth window (the scan runs while the agent
/// process is alive, so "now" is the real ceiling; this only absorbs skew).
const BORN_AFTER_SLACK: Duration = Duration::from_secs(5);

/// One row of the process table used by the nested-agent scan: pid, parent pid,
/// when the process started, and its full command line.
#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    /// Wall-clock start, derived from `ps` elapsed time.
    pub started: SystemTime,
    /// Full argv, space-joined as `ps` prints it.
    pub args: String,
}

/// Snapshot the process table with elapsed time + full argv in one `ps` pass.
/// A failed/absent `ps` yields an empty table (the scan then finds nothing,
/// exactly as before this existed).
///
/// `args` is last on purpose — it is the only field that can contain spaces.
pub fn process_table() -> Vec<ProcInfo> {
    let out = match std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,etime=,args="])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    parse_process_table(&String::from_utf8_lossy(&out), SystemTime::now())
}

/// Parse `ps -axo pid=,ppid=,etime=,args=` output. Split out from
/// [`process_table`] so the parsing is testable without spawning `ps`.
pub fn parse_process_table(out: &str, now: SystemTime) -> Vec<ProcInfo> {
    out.lines()
        .filter_map(|line| {
            // `ps` space-pads the numeric columns, so split on whitespace runs:
            // pid, ppid, etime, then everything left is argv (its own runs of
            // spaces are not worth preserving — we only ever match tokens).
            let mut fields = line.split_whitespace();
            let pid: u32 = fields.next()?.parse().ok()?;
            let ppid: u32 = fields.next()?.parse().ok()?;
            let elapsed = parse_ps_etime(fields.next()?)?;
            let args = fields.collect::<Vec<_>>().join(" ");
            Some(ProcInfo {
                pid,
                ppid,
                started: now.checked_sub(elapsed).unwrap_or(SystemTime::UNIX_EPOCH),
                args,
            })
        })
        .collect()
}

/// Parse `ps` elapsed time (`[[DD-]HH:]MM:SS`) into a duration.
pub fn parse_ps_etime(s: &str) -> Option<Duration> {
    let (days, rest) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<u64>().ok()?, r),
        None => (0, s),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (
            h.parse::<u64>().ok()?,
            m.parse::<u64>().ok()?,
            s.parse::<u64>().ok()?,
        ),
        [m, s] => (0, m.parse::<u64>().ok()?, s.parse::<u64>().ok()?),
        _ => return None,
    };
    Some(Duration::from_secs(
        (days * 86_400) + (h * 3_600) + (m * 60) + sec,
    ))
}

/// Which agent CLI (if any) a command line is running.
///
/// Two shapes are recognized, because the CLIs ship both ways:
///   - a native launcher — argv[0]'s file name IS the provider (`claude`,
///     `/opt/homebrew/bin/codex`, …);
///   - a JS entrypoint under a runtime — `node …/@anthropic-ai/claude-code/cli.js`.
///     Here argv[0] is the runtime, so the remaining tokens are matched against
///     each provider's package/binary marker.
///
/// Returns the canonical provider id (matching [`crate::providers`]' built-in
/// names), never a path.
pub fn agent_from_args(args: &str) -> Option<&'static str> {
    let mut tokens = args.split_whitespace();
    let argv0 = tokens.next()?;
    let head = file_name(argv0);
    if let Some(p) = NESTED_PROVIDERS.iter().find(|p| **p == head) {
        return Some(p);
    }
    // Only chase JS entrypoints when argv[0] is a known runtime — otherwise any
    // command that merely MENTIONS "claude" (`grep claude`, `vim codex.md`)
    // would masquerade as an agent.
    if !matches!(head.as_str(), "node" | "bun" | "deno" | "npx") {
        return None;
    }
    for tok in tokens {
        // Flags are never the entrypoint.
        if tok.starts_with('-') {
            continue;
        }
        let name = file_name(tok);
        if let Some(p) = NESTED_PROVIDERS.iter().find(|p| **p == name) {
            return Some(p);
        }
        if tok.contains("claude-code") {
            return Some("claude");
        }
        if tok.contains("codex") {
            return Some("codex");
        }
        if tok.contains("antigravity") {
            return Some("agy");
        }
    }
    None
}

/// The file-name component of a path-ish token, minus a `.js`/`.mjs`/`.cjs`
/// extension (`/usr/local/bin/claude` → `claude`, `cli.js` → `cli`).
fn file_name(tok: &str) -> String {
    let base = tok.rsplit('/').next().unwrap_or(tok);
    base.strip_suffix(".js")
        .or_else(|| base.strip_suffix(".mjs"))
        .or_else(|| base.strip_suffix(".cjs"))
        .unwrap_or(base)
        .to_string()
}

/// Find the agent CLI running underneath `root` (the session's PTY process).
///
/// Walks the whole descendant tree, not just direct children: the user's shell
/// may sit under `tmux`, a `sudo`, or a nested shell. When several match (an
/// agent that shelled out to another agent) the OLDEST wins — that is the one
/// the user launched and the one whose conversation the terminal is really in.
pub fn find_nested_agent(root: u32, table: &[ProcInfo]) -> Option<(ProcInfo, &'static str)> {
    use std::collections::HashMap;
    let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, p) in table.iter().enumerate() {
        children.entry(p.ppid).or_default().push(i);
    }
    let mut best: Option<(ProcInfo, &'static str)> = None;
    let mut seen: HashSet<u32> = HashSet::new();
    let mut stack: Vec<usize> = children.get(&root).cloned().unwrap_or_default();
    while let Some(i) = stack.pop() {
        let p = &table[i];
        if !seen.insert(p.pid) {
            continue; // defensive: a malformed table must not loop forever
        }
        if let Some(provider) = agent_from_args(&p.args) {
            let replace = match &best {
                Some((cur, _)) => p.started < cur.started,
                None => true,
            };
            if replace {
                best = Some((p.clone(), provider));
            }
        }
        if let Some(kids) = children.get(&p.pid) {
            stack.extend(kids.iter().copied());
        }
    }
    best
}

/// The working directory a live process is running in, via
/// `lsof -a -p <pid> -d cwd -Fn` (macOS has no `/proc`).
///
/// This is the directory the agent CLI files its transcript under, which is NOT
/// necessarily the session's cwd — `cd somewhere && claude` is normal.
pub fn process_cwd(pid: u32) -> Option<String> {
    let out = std::process::Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // `-F` output is one field per line, prefixed by its letter: `p<pid>`,
    // `f<fd>`, `n<name>`.
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix('n').map(str::to_string))
        .filter(|p| !p.is_empty())
}

/// The claude transcript created by a launch at `started`, for a session
/// running in `cwd`.
///
/// claude names its transcript `<session-id>.jsonl` under
/// `~/.claude/projects/<encoded cwd>/` (see [`crate::lifecycle`]). We match on
/// the file's BIRTH time rather than its mtime: a resumed older conversation in
/// the same directory keeps being written to (fresh mtime) but was born long
/// before this launch, so birth time is what actually identifies "the file this
/// process created".
///
/// `claimed` holds the provider session ids other Otto sessions already own.
/// Returns `None` when nothing matches — and also when SEVERAL unclaimed
/// candidates fall in the window (two agents launched in one directory within
/// the same window): we never guess which conversation belongs to whom.
pub fn claude_transcript_in_window(
    home: &Path,
    cwd: &str,
    started: SystemTime,
    claimed: &HashSet<&str>,
) -> Option<String> {
    let dir = home
        .join(".claude")
        .join("projects")
        .join(crate::lifecycle::claude_project_dir_name(cwd));
    let from = started
        .checked_sub(BORN_BEFORE_SLACK)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    // Upper bound is NOW, not a fixed span after launch: claude files its
    // transcript only when the first prompt is sent, which may be a long think
    // after the CLI booted. The scan only ever runs while the process is alive,
    // so "born before now" is exactly "born by this launch".
    let to = SystemTime::now() + BORN_AFTER_SLACK;

    let mut hits: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(sid) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if claimed.contains(sid) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        // `created()` is the APFS birth time on macOS. Where the platform
        // can't report one, fall back to mtime — still bounded by the window.
        let born = match meta.created().or_else(|_| meta.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if born >= from && born <= to {
            hits.push(sid.to_string());
        }
    }
    match hits.len() {
        1 => hits.pop(),
        _ => None,
    }
}

/// The command that puts a terminal back into a nested agent's conversation.
///
/// Mirrors each provider's `resume_args` in [`crate::providers`], but WITHOUT
/// Otto's unattended-mode flags: the user launched this CLI by hand in a plain
/// shell, so we hand back exactly the conversation and nothing else — their own
/// permission mode, their own defaults.
pub fn resume_command(provider: &str, sid: &str, from_cwd: Option<&str>) -> Option<String> {
    let resume = match provider {
        "claude" => format!("claude --resume {}", shell_quote(sid)),
        "codex" => format!("codex resume {}", shell_quote(sid)),
        "agy" => format!("agy --conversation {}", shell_quote(sid)),
        _ => return None,
    };
    // The transcript is resolved from the cwd the agent was launched in, so a
    // session whose shell starts elsewhere has to `cd` back first.
    Some(match from_cwd {
        Some(dir) => format!("cd {} && {resume}", shell_quote(dir)),
        None => resume,
    })
}

/// Single-quote a string for POSIX shells (`it's` → `'it'\''s'`). Used on every
/// value interpolated into the resume command line above.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etime_parses_every_ps_shape() {
        assert_eq!(parse_ps_etime("05:07"), Some(Duration::from_secs(307)));
        assert_eq!(parse_ps_etime("01:05:07"), Some(Duration::from_secs(3907)));
        assert_eq!(
            parse_ps_etime("2-01:05:07"),
            Some(Duration::from_secs(2 * 86_400 + 3907))
        );
        assert_eq!(parse_ps_etime("nonsense"), None);
    }

    #[test]
    fn process_table_keeps_argv_spaces_and_computes_start() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let rows = parse_process_table(
            "  501   500       05:00 claude --resume abc def\n\
             \x20 502   501    00:10 /bin/zsh -l\n\
             garbage line\n",
            now,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pid, 501);
        assert_eq!(rows[0].ppid, 500);
        assert_eq!(rows[0].args, "claude --resume abc def");
        assert_eq!(rows[0].started, now - Duration::from_secs(300));
        assert_eq!(rows[1].args, "/bin/zsh -l");
    }

    #[test]
    fn recognizes_native_and_js_entrypoints() {
        assert_eq!(agent_from_args("claude"), Some("claude"));
        assert_eq!(
            agent_from_args("/opt/homebrew/bin/claude --resume x"),
            Some("claude")
        );
        assert_eq!(agent_from_args("codex resume abc"), Some("codex"));
        assert_eq!(agent_from_args("agy --conversation z"), Some("agy"));
        assert_eq!(
            agent_from_args("node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
            Some("claude")
        );
        assert_eq!(
            agent_from_args("node --enable-source-maps /usr/lib/node_modules/codex/bin.js"),
            Some("codex")
        );
    }

    /// A command that merely MENTIONS an agent is not that agent — otherwise
    /// `tail -f claude.log` in a terminal would make the session "resumable"
    /// into somebody else's conversation.
    #[test]
    fn does_not_mistake_mentions_for_the_cli() {
        assert_eq!(agent_from_args("grep -r claude ."), None);
        assert_eq!(agent_from_args("vim codex-notes.md"), None);
        assert_eq!(agent_from_args("/bin/zsh -l"), None);
        assert_eq!(agent_from_args(""), None);
    }

    #[test]
    fn finds_the_oldest_agent_anywhere_under_the_pty() {
        let t = |pid, ppid, secs, args: &str| ProcInfo {
            pid,
            ppid,
            started: SystemTime::UNIX_EPOCH + Duration::from_secs(secs),
            args: args.into(),
        };
        let table = vec![
            t(100, 1, 0, "/bin/zsh -l"), // the PTY shell
            t(200, 100, 10, "tmux"),     // …under which the user ran tmux
            t(300, 200, 20, "/bin/zsh"), // …a shell inside it
            t(400, 300, 30, "claude"),   // …and finally claude
            t(500, 400, 40, "codex"),    // an agent the agent shelled out to
        ];
        let (proc, provider) = find_nested_agent(100, &table).expect("nested agent");
        assert_eq!(provider, "claude");
        assert_eq!(proc.pid, 400);
        // Nothing under a leaf.
        assert!(find_nested_agent(500, &table).is_none());
    }

    #[test]
    fn cyclic_table_terminates() {
        let t = |pid, ppid| ProcInfo {
            pid,
            ppid,
            started: SystemTime::UNIX_EPOCH,
            args: "x".into(),
        };
        assert!(find_nested_agent(1, &[t(1, 2), t(2, 1)]).is_none());
    }

    /// Birth time inside the launch window identifies the transcript; an older
    /// conversation in the same directory (even one being actively written) and
    /// an id another session already claimed are both ignored.
    #[test]
    fn claude_transcript_matched_by_birth_window() {
        let home = tempfile::tempdir().unwrap();
        let cwd = "/Users/dev/project";
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join(crate::lifecycle::claude_project_dir_name(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fresh.jsonl"), b"{}").unwrap();
        std::fs::write(dir.join("not-a-transcript.txt"), b"x").unwrap();

        let claimed: HashSet<&str> = HashSet::new();
        // Files created just now match a launch that started just now.
        assert_eq!(
            claude_transcript_in_window(home.path(), cwd, SystemTime::now(), &claimed),
            Some("fresh.jsonl".trim_end_matches(".jsonl").to_string())
        );
        // A launch far in the future (i.e. the file predates it) matches nothing.
        let later = SystemTime::now() + Duration::from_secs(3600);
        assert_eq!(
            claude_transcript_in_window(home.path(), cwd, later, &claimed),
            None
        );
        // Already owned by another session → never re-claimed.
        let owned: HashSet<&str> = ["fresh"].into_iter().collect();
        assert_eq!(
            claude_transcript_in_window(home.path(), cwd, SystemTime::now(), &owned),
            None
        );
    }

    /// Two agents launched in one directory inside the same window: refuse
    /// rather than adopt (and later fork) the wrong conversation.
    #[test]
    fn ambiguous_window_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let cwd = "/Users/dev/project";
        let dir = home
            .path()
            .join(".claude")
            .join("projects")
            .join(crate::lifecycle::claude_project_dir_name(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), b"{}").unwrap();
        std::fs::write(dir.join("b.jsonl"), b"{}").unwrap();
        assert_eq!(
            claude_transcript_in_window(home.path(), cwd, SystemTime::now(), &HashSet::new()),
            None
        );
    }

    #[test]
    fn resume_commands_match_each_provider_and_quote_their_inputs() {
        assert_eq!(
            resume_command("claude", "abc-123", None).as_deref(),
            Some("claude --resume 'abc-123'")
        );
        assert_eq!(
            resume_command("codex", "abc", Some("/tmp/x")).as_deref(),
            Some("cd '/tmp/x' && codex resume 'abc'")
        );
        assert_eq!(
            resume_command("agy", "abc", None).as_deref(),
            Some("agy --conversation 'abc'")
        );
        assert_eq!(resume_command("shell", "abc", None), None);
        // A directory with a quote in it can't break out of the command.
        assert_eq!(
            resume_command("claude", "id", Some("/tmp/it's")).as_deref(),
            Some(r"cd '/tmp/it'\''s' && claude --resume 'id'")
        );
    }
}
