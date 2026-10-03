//! File history, blame and log search (git batch WP5) — routes are merged into `crate::http::router`.
//!
//! `log_with` is the search-capable successor of [`LocalGit::log`]: the same
//! `--pretty` record, plus a pathspec (with `--follow` across renames) and the
//! `--grep`/`--author` limiting patterns the graph search bar issues. `blame`
//! runs `--porcelain` and groups its output into per-run rows.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Extension, Router};
use otto_core::api::CommitInfo;
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};

use crate::http::{repo_ctx, ApiResult, GitCtx};
use crate::local::{GitCmd, LocalGit};

/// Everything `GET /repos/{id}/log` can ask for. `limit == 0` means NO `-n` at
/// all (the whole reachable history) — same contract as [`LocalGit::log`].
#[derive(Debug, Clone, Default)]
pub struct LogOpts {
    pub limit: u32,
    pub skip: u32,
    pub all: bool,
    /// Scope history to one path (emitted after `--`).
    pub path: Option<String>,
    /// Follow the path across renames. Git requires EXACTLY one pathspec for
    /// this, so it is only valid together with `path`.
    pub follow: bool,
    /// `--grep=<g>`: subject/body substring (case-insensitive, literal).
    pub grep: Option<String>,
    /// `--author=<a>`: author substring (case-insensitive, literal).
    pub author: Option<String>,
    /// A full or abbreviated commit sha to page TO: the walk (from `skip`)
    /// keeps going past `limit` until that commit has been emitted, then
    /// stops — ONE spawn that is killed at the target, where the graph's
    /// jump-to-an-old-ref used to issue a `--skip` page per 10k commits (each
    /// re-walking every commit before it: O(pages²)). Never more than
    /// `max(limit, position of the target + 1)` records; a target that is not
    /// in the walk reads to the end (bounded by the stdout ceiling).
    pub until: Option<String>,
}

/// Blame of a file bigger than this is refused rather than buffered: the
/// porcelain output of a 20k-line file is ~2–4 MB, so 32 MB is a generated
/// or vendored blob no panel could render anyway.
pub(crate) const BLAME_STDOUT_CAP: usize = 32 * 1024 * 1024;

/// `Some(bytes kept)` once `buf` holds at least `min_records` complete log
/// records AND the record for `target` (a sha prefix) — scanning only the
/// bytes after `from` (each call sees the new chunk; `state` carries the
/// record count and whether the target was seen across calls).
fn until_cut(
    buf: &[u8],
    from: usize,
    target: &[u8],
    min_records: usize,
    state: &mut (usize, bool, usize),
) -> Option<usize> {
    // state = (records completed, target seen, byte offset of the next record)
    let (done, seen, rec_start) = state;
    let mut i = from.max(*rec_start);
    while let Some(off) = buf[i..].iter().position(|&b| b == 0x1e) {
        let end = i + off;
        let rec = &buf[*rec_start..end];
        let rec = match rec.iter().position(|&b| b != b'\n' && b != b'\r') {
            Some(p) => &rec[p..],
            None => rec,
        };
        *done += 1;
        if !*seen && rec.len() >= target.len() && &rec[..target.len()] == target {
            *seen = true;
        }
        *rec_start = end + 1;
        i = end + 1;
        if *seen && *done >= min_records {
            return Some(end + 1);
        }
    }
    None
}

/// One run of consecutive lines attributed to the same commit.
#[derive(Debug, Clone, Serialize)]
pub struct BlameLine {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    /// RFC3339 UTC (git reports the author time as an epoch + tz).
    pub at: String,
    /// Line number in the commit the run came FROM.
    pub orig_line: u32,
    /// First line of the run in the blamed file.
    pub line_start: u32,
    /// Lines in the run (≥ 1).
    pub count: u32,
    pub summary: String,
    /// The run's source lines (one per blamed line, `count` of them), each
    /// capped at [`BLAME_LINE_CAP`] chars — the code column of the panel.
    pub text: Vec<String>,
    /// Porcelain `previous <sha> <path>`: the commit's parent and the file's
    /// path there (follows renames). Blaming `previous.sha:previous.path`
    /// is "blame before this change". None for a root commit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<BlamePrevious>,
}

/// Where a blamed line lived just before the commit that last touched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BlamePrevious {
    pub sha: String,
    pub path: String,
}

/// Longest source line the blame keeps (chars) — a minified bundle's
/// 300 KB line would otherwise ride every response.
pub(crate) const BLAME_LINE_CAP: usize = 1000;

#[derive(Debug, Clone, Serialize)]
pub struct BlameResp {
    pub path: String,
    pub rev: String,
    pub lines: Vec<BlameLine>,
}

impl LocalGit {
    /// Read history with an optional pathspec and `--grep`/`--author` filters.
    ///
    /// The limiting patterns ride INSIDE the option (`--grep=<v>`), so a value
    /// that looks like an option is never re-parsed as one; `--fixed-strings`
    /// keeps a user's `(` or `*` from becoming a regex (and rules out regex
    /// DoS), and `--regexp-ignore-case` makes the search bar case-insensitive.
    pub async fn log_with(&self, o: &LogOpts) -> Result<Vec<CommitInfo>> {
        if o.follow && o.path.is_none() {
            return Err(Error::Invalid("follow requires exactly one path".into()));
        }
        if let Some(p) = &o.path {
            Self::guard_path(p)?;
        }
        let limit_s = o.limit.to_string();
        let skip_s = o.skip.to_string();
        let grep_arg = o.grep.as_deref().map(|g| format!("--grep={g}"));
        let author_arg = o.author.as_deref().map(|a| format!("--author={a}"));

        // `--no-show-signature`: `log.showSignature=true` would put gpg output
        // ahead of each record and corrupt the parsed SHAs (see `LocalGit::log`).
        let mut args: Vec<&str> = vec!["log", "--no-show-signature"];
        if o.all {
            // The graph path: `--date-order` guarantees no parent is printed
            // before all of its children (clock skew would otherwise open a
            // phantom tip lane). Filtered/path logs keep git's order.
            args.push("--all");
            args.push("--date-order");
        }
        if o.limit > 0 && o.until.is_none() {
            args.push("-n");
            args.push(&limit_s);
        }
        args.push("--skip");
        args.push(&skip_s);
        args.push("--pretty=format:%H%x1f%h%x1f%an%x1f%aI%x1f%s%x1f%P%x1f%D%x1e");
        if let Some(g) = &grep_arg {
            args.push(g);
        }
        if let Some(a) = &author_arg {
            args.push(a);
        }
        if grep_arg.is_some() || author_arg.is_some() {
            args.push("--regexp-ignore-case");
            args.push("--fixed-strings");
        }
        if o.follow {
            args.push("--follow");
        }
        // A LITERAL path (`app/[id]/page.tsx` is a file, not a glob that also
        // pulls `app/d/page.tsx`'s history into the list).
        let cmd = GitCmd::read(&args).maybe_path(o.path.as_deref());
        let out = match until_target(o)? {
            // `-n` was left off above: the stop hook ends the walk instead.
            Some(target) => {
                let min = o.limit as usize;
                let mut state = (0usize, false, 0usize);
                let stop: crate::local::StopFn = Box::new(move |buf, from| {
                    until_cut(buf, from, target.as_bytes(), min, &mut state)
                });
                let (bytes, _) = self.exec_truncated(&cmd, Some(stop)).await?;
                String::from_utf8_lossy(&bytes).into_owned()
            }
            None => match self.exec_text(&cmd).await {
                Ok(out) => out,
                // Unborn branch (fresh repo, no commit yet): an empty history,
                // not git's "does not have any commits yet" as an error. Only
                // checked on failure — no extra spawn per page.
                Err(_) if !o.all && !self.head_exists().await => return Ok(Vec::new()),
                Err(e) => return Err(e),
            },
        };
        if o.all {
            // The graph's `--all --date-order` walk is where a missing
            // commit-graph hurts most (no generation numbers: the whole
            // history is walked before the first record). Seed it once.
            self.seed_commit_graph();
        }
        // A 10k-commit page is ~2.5 MB of records — parsed off the workers.
        crate::local::off_runtime(out.len(), move || crate::parse::parse_log(&out)).await?
    }

    /// `git blame --porcelain <rev> -- <path>` grouped into runs.
    ///
    /// NOTE on `--end-of-options`: blame keeps its own `--` handling (it accepts
    /// both `blame <rev> -- <file>` and the legacy `blame <file> <rev>`), and
    /// git 2.54 fails the combination — `blame --end-of-options <rev> -- <path>`
    /// dies with "bad revision '<path>'" because the separator is no longer read
    /// as one. The guards below already make the marker redundant here: `rev`
    /// cannot begin with `-` after [`LocalGit::guard_ref`], and `path` only ever
    /// appears AFTER `--`.
    pub async fn blame(&self, path: &str, rev: &str) -> Result<BlameResp> {
        Self::guard_ref(rev)?;
        Self::guard_path(path)?;
        // Bounded: a read past BLAME_STDOUT_CAP is killed and reported.
        let cmd =
            GitCmd::read(&["blame", "--porcelain", rev, "--", path]).max_stdout(BLAME_STDOUT_CAP);
        let (ok, stdout, stderr, code) = self.exec(&cmd, None).await?;
        if !ok {
            let stdout = String::from_utf8_lossy(&stdout);
            return Err(crate::local::upstream_err(&stderr, &stdout, code));
        }
        // Blame walks history per line; the changed-path Bloom filters of a
        // commit-graph make it several times faster on big repos.
        self.seed_commit_graph();
        // Up to 32 MB of porcelain — parsed off the async workers.
        let lines = crate::local::off_runtime(stdout.len(), move || {
            parse_blame(&String::from_utf8_lossy(&stdout))
        })
        .await?;
        Ok(BlameResp {
            path: path.to_string(),
            rev: rev.to_string(),
            lines,
        })
    }
}

/// A porcelain group header: `<40-hex sha> <orig-line> <final-line> [<count>]`.
struct BlameHeader {
    sha: String,
    orig: u32,
    start: u32,
    count: u32,
    /// The header printed a group size — the FIRST line of a run. A
    /// continuation header (no size) adds a line to the open run.
    opens_run: bool,
}

fn parse_header(line: &str) -> Option<BlameHeader> {
    let mut it = line.split(' ');
    let sha = it.next()?;
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let orig = it.next()?.parse().ok()?;
    let start = it.next()?.parse().ok()?;
    // The group size is only printed on the FIRST header of a run.
    let (count, opens_run) = match it.next() {
        Some(c) => (c.parse().ok()?, true),
        None => (1, false),
    };
    if it.next().is_some() {
        return None;
    }
    Some(BlameHeader {
        sha: sha.to_string(),
        orig,
        start,
        count,
        opens_run,
    })
}

/// Parse `git blame --porcelain` into one [`BlameLine`] per run.
///
/// The porcelain format prints a commit's `author` / `author-time` / `summary`
/// exactly ONCE — every later run of the same sha carries only the header line
/// — so the first sighting is remembered per sha. The `\t`-prefixed content
/// line closes a group (it is the only line the file's own text can appear on,
/// and it is never parsed as anything else).
pub fn parse_blame(porcelain: &str) -> Vec<BlameLine> {
    type Meta = (String, String, String, Option<BlamePrevious>);
    let mut seen: HashMap<String, Meta> = HashMap::new();
    let mut out: Vec<BlameLine> = Vec::new();
    let mut cur: Option<BlameHeader> = None;
    let (mut author, mut at, mut summary, mut previous) = (None, None, None, None);

    for line in porcelain.lines() {
        if let Some(h) = parse_header(line) {
            cur = Some(h);
            (author, at, summary, previous) = (None, None, None, None);
            continue;
        }
        if let Some(content) = line.strip_prefix('\t') {
            // The file's own text: content, never metadata — and the group's end.
            let Some(h) = cur.take() else { continue };
            let text: String = content.chars().take(BLAME_LINE_CAP).collect();
            let meta = seen.entry(h.sha.clone()).or_insert_with(|| {
                (
                    author.take().unwrap_or_else(|| "unknown".to_string()),
                    at.take().unwrap_or_default(),
                    summary.take().unwrap_or_default(),
                    previous.take(),
                )
            });
            // A continuation header (no group size) is the next line of the
            // open run: fold it in so one row = one run, with its code.
            if !h.opens_run {
                if let Some(last) = out
                    .last_mut()
                    .filter(|l| l.sha == h.sha && l.line_start + l.text.len() as u32 == h.start)
                {
                    last.text.push(text);
                    last.count = last.count.max(last.text.len() as u32);
                    continue;
                }
            }
            out.push(BlameLine {
                short_sha: h.sha.chars().take(8).collect(),
                sha: h.sha,
                author: meta.0.clone(),
                at: meta.1.clone(),
                orig_line: h.orig,
                line_start: h.start,
                count: h.count,
                summary: meta.2.clone(),
                text: vec![text],
                previous: meta.3.clone(),
            });
            continue;
        }
        // `author-mail` / `author-tz` don't match (the prefix carries the space).
        if let Some(v) = line.strip_prefix("author ") {
            author = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("author-time ") {
            at = v
                .trim()
                .parse::<i64>()
                .ok()
                .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
                .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        } else if let Some(v) = line.strip_prefix("summary ") {
            summary = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("previous ") {
            // `previous <sha> <path>` — the path may contain spaces.
            if let Some((sha, path)) = v.split_once(' ') {
                previous = Some(BlamePrevious {
                    sha: sha.to_string(),
                    path: path.to_string(),
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct BlameQuery {
    path: String,
    rev: Option<String>,
}

async fn repo_blame<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<BlameQuery>,
) -> ApiResult<axum::response::Response> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let rev = q.rev.as_deref().map(str::trim).filter(|r| !r.is_empty());
    let resp = git.blame(&q.path, rev.unwrap_or("HEAD")).await?;
    let est: usize = resp
        .lines
        .iter()
        .map(|l| 160 + l.text.iter().map(|t| t.len() + 4).sum::<usize>())
        .sum();
    Ok(crate::http::json_off_runtime(resp, est).await?)
}

/// Routes owned by this module (merged into `crate::http::router`).
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new().route("/repos/{id}/blame", get(repo_blame::<S>))
}

/// The validated `until` target: 4–64 hex chars (a sha or a prefix), lower-
/// cased the way `%H` prints it. Anything else is a 400, never argv.
fn until_target(o: &LogOpts) -> Result<Option<String>> {
    let Some(t) = o.until.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if !(4..=64).contains(&t.len()) || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Invalid(format!("until must be a commit sha: {t}")));
    }
    Ok(Some(t.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path as FsPath, PathBuf};

    /// Run `git` synchronously for fixture setup.
    fn sh_git(dir: &FsPath, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write(dir: &FsPath, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    /// Empty repo with identity + signing off (a user's global `commit.gpgsign`
    /// must not reach into the fixtures).
    fn repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", "main"]);
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);
        (tmp, dir)
    }

    /// `--follow` must walk THROUGH a rename: the file's pre-rename commits are
    /// part of its history, and a history panel that stops at the rename is the
    /// whole reason the flag exists.
    #[tokio::test]
    async fn log_follow_across_rename() {
        let (_tmp, dir) = repo();
        write(&dir, "old.txt", "one\ntwo\nthree\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "add old.txt"]);
        write(&dir, "old.txt", "one\nTWO\nthree\n");
        sh_git(&dir, &["commit", "-am", "edit old.txt"]);
        sh_git(&dir, &["mv", "old.txt", "new.txt"]);
        sh_git(&dir, &["commit", "-m", "rename to new.txt"]);

        let git = LocalGit::new(&dir);
        let opts = LogOpts {
            limit: 50,
            path: Some("new.txt".into()),
            follow: true,
            ..Default::default()
        };
        let subjects: Vec<String> = git
            .log_with(&opts)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.subject)
            .collect();
        assert_eq!(
            subjects,
            vec!["rename to new.txt", "edit old.txt", "add old.txt"]
        );

        // Without --follow history stops at the rename.
        let no_follow = LogOpts {
            follow: false,
            ..opts.clone()
        };
        assert_eq!(git.log_with(&no_follow).await.unwrap().len(), 1);
    }

    /// `--grep` / `--author` are literal + case-insensitive, and a value that
    /// looks like an option (or a regex) is data, never argv.
    #[tokio::test]
    async fn log_grep_and_author_filter() {
        let (_tmp, dir) = repo();
        write(&dir, "a.txt", "1\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "feat: needle-xyz lands"]);
        write(&dir, "a.txt", "2\n");
        sh_git(&dir, &["commit", "-am", "chore: unrelated"]);
        write(&dir, "a.txt", "3\n");
        sh_git(
            &dir,
            &[
                "-c",
                "user.name=Grace Hopper",
                "-c",
                "user.email=other@test.local",
                "commit",
                "-am",
                "docs: from another author",
            ],
        );

        let git = LocalGit::new(&dir);
        let grep = |q: &str| LogOpts {
            limit: 50,
            grep: Some(q.to_string()),
            ..Default::default()
        };
        assert_eq!(git.log_with(&grep("NEEDLE-XYZ")).await.unwrap().len(), 1);
        // Literal: the regex metacharacters match nothing instead of everything.
        assert!(git
            .log_with(&grep("needle.*lands"))
            .await
            .unwrap()
            .is_empty());
        // An option-like pattern stays a pattern (it rides inside `--grep=`).
        assert!(git.log_with(&grep("--all")).await.unwrap().is_empty());

        // Case-insensitive, but ASCII-only: every spawn runs under `LC_ALL=C`
        // (`LocalGit::base_cmd`), so git folds ASCII and nothing else.
        let by_author = LogOpts {
            limit: 50,
            author: Some("grace hopper".into()),
            ..Default::default()
        };
        let found = git.log_with(&by_author).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].subject, "docs: from another author");
    }

    /// A linear history of `n` commits built in one `git fast-import` (a
    /// 30k-commit fixture in well under a second), with a branch `old` at
    /// the root so `--all` walks every ref.
    fn big_history(n: usize) -> (tempfile::TempDir, PathBuf) {
        use std::io::Write;
        let (tmp, dir) = repo();
        let mut stream = String::with_capacity(n * 120);
        for i in 1..=n {
            stream.push_str(&format!(
                "commit refs/heads/main\nmark :{i}\ncommitter T <t@t> {} +0000\ndata {}\nc{i}\n",
                1_600_000_000 + i,
                format!("c{i}\n").len()
            ));
            if i > 1 {
                stream.push_str(&format!("from :{}\n", i - 1));
            }
            stream.push('\n');
        }
        stream.push_str("reset refs/heads/old\nfrom :1\n\n");
        let mut child = std::process::Command::new("git")
            .current_dir(&dir)
            .args(["fast-import", "--quiet"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("fast-import");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stream.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        (tmp, dir)
    }

    /// `until` (one spawn, stopped at the target) returns exactly what the
    /// graph used to assemble from `--skip` pages: the same commits in the
    /// same order, ending at the first page boundary that reaches the target
    /// — here `max(limit, position + 1)` records.
    #[tokio::test]
    async fn until_matches_skip_paging_on_30k_commits() {
        let (_tmp, dir) = big_history(30_000);
        let git = LocalGit::new(&dir);
        let page = |skip: u32| LogOpts {
            limit: 10_000,
            skip,
            all: true,
            ..Default::default()
        };
        let mut paged = Vec::new();
        for skip in [0, 10_000, 20_000] {
            paged.extend(git.log_with(&page(skip)).await.unwrap());
        }
        assert_eq!(paged.len(), 30_000);

        // A target deep in the third page.
        let target = paged[25_000].sha.clone();
        let jumped = git
            .log_with(&LogOpts {
                until: Some(target[..12].to_string()),
                ..page(0)
            })
            .await
            .unwrap();
        assert_eq!(jumped.len(), 25_001, "stops right after the target");
        let shas = |v: &[CommitInfo]| v.iter().map(|c| c.sha.clone()).collect::<Vec<_>>();
        assert_eq!(shas(&jumped), shas(&paged[..25_001]));
        assert_eq!(jumped.last().unwrap().sha, target);

        // From a cursor: the rest of the walk up to the target.
        let from_cursor = git
            .log_with(&LogOpts {
                until: Some(target.clone()),
                ..page(10_000)
            })
            .await
            .unwrap();
        assert_eq!(shas(&from_cursor), shas(&paged[10_000..25_001]));

        // A target inside the first page still returns a whole page.
        let near = git
            .log_with(&LogOpts {
                until: Some(paged[5].sha.clone()),
                ..page(0)
            })
            .await
            .unwrap();
        assert_eq!(shas(&near), shas(&paged[..10_000]));

        // Not in the walk: reads to the end (the caller sees history run out).
        let missing = git
            .log_with(&LogOpts {
                until: Some("deadbeefdeadbeef".into()),
                ..page(0)
            })
            .await
            .unwrap();
        assert_eq!(missing.len(), 30_000);

        // Only hex reaches git.
        let err = git
            .log_with(&LogOpts {
                until: Some("--all".into()),
                ..page(0)
            })
            .await
            .expect_err("not a sha");
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    }

    /// The stop hook works on chunk boundaries: a record split across two
    /// reads is only counted once complete, and the kept prefix ends right
    /// after the target's separator.
    #[test]
    fn until_cut_counts_records_across_chunks() {
        let buf = b"aaaa\x1fx\x1e\nbbbb\x1fy\x1e\ncccc\x1fz\x1e\ndddd\x1fw\x1e";
        let mut st = (0usize, false, 0usize);
        // Feed in 5-byte chunks, as a pipe might.
        let mut got = None;
        let mut len = 0;
        while len < buf.len() && got.is_none() {
            let from = len;
            len = (len + 5).min(buf.len());
            got = until_cut(&buf[..len], from, b"bbbb", 3, &mut st);
        }
        let keep = got.expect("stops");
        assert_eq!(&buf[..keep], b"aaaa\x1fx\x1e\nbbbb\x1fy\x1e\ncccc\x1fz\x1e");
        assert_eq!(st.0, 3);
    }

    /// The byte cap: a truncating read keeps exactly N bytes and KILLS the
    /// child (an endless producer — `yes` stands in for git — would
    /// otherwise run into the 5 s budget), and a `max_stdout` read fails.
    #[tokio::test]
    async fn capped_exec_truncates_and_kills_the_child() {
        let (_tmp, dir) = repo();
        let Some(yes) = ["/usr/bin/yes", "/bin/yes"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
        else {
            return; // no `yes` on this host
        };
        let git = LocalGit::new(&dir)
            .with_git_bin(yes)
            .with_budget(std::time::Duration::from_secs(5));
        let started = std::time::Instant::now();
        let (out, cut) = git
            .exec_truncated(&GitCmd::read(&["y"]).truncate_stdout(100_000), None)
            .await
            .expect("a truncated read succeeds");
        assert!(cut);
        assert_eq!(out.len(), 100_000);
        assert!(out.starts_with(b"y\ny\n"));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(4),
            "the producer was killed, not waited out"
        );

        let err = git
            .exec_bytes(&GitCmd::read(&["y"]).max_stdout(100_000))
            .await
            .expect_err("over the cap");
        assert!(
            matches!(&err, Error::Upstream(m) if m.contains("output")),
            "got {err:?}"
        );
    }

    /// `--follow` without a path is git's own 400 ("requires exactly one
    /// pathspec") — caught before the spawn so it reads as a bad request.
    #[tokio::test]
    async fn follow_without_path_is_invalid() {
        let (_tmp, dir) = repo();
        write(&dir, "a.txt", "1\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        let git = LocalGit::new(&dir);
        let err = git
            .log_with(&LogOpts {
                limit: 10,
                follow: true,
                ..Default::default()
            })
            .await
            .expect_err("follow needs a path");
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    }

    /// The porcelain grouping: one row per RUN (not per line), the `count` from
    /// the header, and the per-sha fields remembered for the second run of the
    /// same commit (git prints them only once).
    #[test]
    fn parse_blame_groups_runs_and_two_authors() {
        let sha_a = "1111111111111111111111111111111111111111";
        let sha_b = "2222222222222222222222222222222222222222";
        let porcelain = format!(
            "{sha_a} 1 1 2\n\
             author Ada Lovelace\n\
             author-mail <ada@test.local>\n\
             author-time 1700000000\n\
             author-tz +0000\n\
             summary first commit\n\
             filename a.txt\n\
             \tline one\n\
             {sha_a} 2 2\n\
             \tline two\n\
             {sha_b} 1 3 1\n\
             author Grace Hopper\n\
             author-mail <grace@test.local>\n\
             author-time 1700003600\n\
             author-tz +0000\n\
             summary second commit\n\
             filename a.txt\n\
             \tline three\n\
             {sha_a} 3 4 1\n\
             filename a.txt\n\
             \tline four\n"
        );
        let lines = parse_blame(&porcelain);
        assert_eq!(lines.len(), 3, "one row per run; continuations fold in");
        assert_eq!(lines[0].author, "Ada Lovelace");
        assert_eq!(lines[0].short_sha, "11111111");
        assert_eq!(lines[0].count, 2, "the run size comes from the header");
        assert_eq!(lines[0].at, "2023-11-14T22:13:20Z");
        assert_eq!(lines[0].summary, "first commit");
        assert_eq!(
            lines[0].text,
            vec!["line one", "line two"],
            "the code rides along"
        );
        assert_eq!(lines[1].author, "Grace Hopper");
        assert_eq!(lines[1].text, vec!["line three"]);
        assert_eq!(lines[1].previous, None, "no previous header ⇒ root commit");
        // The LAST run repeats sha_a with no fields at all — they must come back
        // from the cache, not be blanked.
        assert_eq!(lines[2].author, "Ada Lovelace");
        assert_eq!(lines[2].summary, "first commit");
        assert_eq!(lines[2].orig_line, 3);
        assert_eq!(lines[2].line_start, 4);
        assert_eq!(lines[2].text, vec!["line four"]);
    }

    /// A filename that begins with `-` is legal on disk; it must blame (it rides
    /// after `--`), while an option-like REV is refused before any spawn.
    #[tokio::test]
    async fn blame_path_starting_with_dash_ok() {
        let (_tmp, dir) = repo();
        write(&dir, "-notes.md", "alpha\nbeta\n");
        sh_git(&dir, &["add", "--", "-notes.md"]);
        sh_git(&dir, &["commit", "-m", "add notes"]);

        let git = LocalGit::new(&dir);
        let blame = git.blame("-notes.md", "HEAD").await.unwrap();
        assert_eq!(blame.path, "-notes.md");
        assert_eq!(blame.lines.len(), 1, "one commit ⇒ one run");
        assert_eq!(blame.lines[0].author, "Otto Test");
        assert_eq!(blame.lines[0].summary, "add notes");
        assert_eq!(blame.lines[0].count, 2);
        assert_eq!(blame.lines[0].text, vec!["alpha", "beta"]);

        // A second commit: its run points at the parent as `previous`
        // ("blame before this change"), with the path there.
        write(&dir, "-notes.md", "alpha\ngamma\n");
        sh_git(&dir, &["commit", "-am", "edit notes"]);
        let parent = git.rev_parse("HEAD~1").await.unwrap();
        let blame = git.blame("-notes.md", "HEAD").await.unwrap();
        let edited = blame
            .lines
            .iter()
            .find(|l| l.summary == "edit notes")
            .unwrap();
        assert_eq!(edited.text, vec!["gamma"]);
        let prev = edited.previous.as_ref().expect("previous header parsed");
        assert_eq!(
            (prev.sha.as_str(), prev.path.as_str()),
            (parent.as_str(), "-notes.md")
        );

        let err = git
            .blame("-notes.md", "--output=/tmp/pwn")
            .await
            .expect_err("an option-like rev is refused");
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    }
}
