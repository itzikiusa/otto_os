//! File history, blame and log search (git batch WP5) — routes are merged into `crate::http::router`.
//!
//! `log_with` is the search-capable successor of [`LocalGit::log`]: the same
//! `--pretty` record, plus a pathspec (with `--follow` across renames) and the
//! `--grep`/`--author` limiting patterns the graph search bar issues. `blame`
//! runs `--porcelain` and groups its output into per-run rows.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Extension, Json, Router};
use otto_core::api::CommitInfo;
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};

use crate::http::{repo_ctx, ApiResult, GitCtx};
use crate::local::{LocalGit, SpawnClass};

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
}

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

        let mut args: Vec<&str> = vec!["log"];
        if o.all {
            args.push("--all");
        }
        if o.limit > 0 {
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
        if let Some(p) = &o.path {
            args.push("--");
            args.push(p);
        }
        let out = self.run_read(&args).await?;
        crate::parse::parse_log(&out)
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
        let (ok, stdout, stderr, code) = self
            .run_raw_class(
                &["blame", "--porcelain", rev, "--", path],
                &[],
                SpawnClass::LocalRead,
            )
            .await?;
        if !ok {
            return Err(crate::local::upstream_err(&stderr, &stdout, code));
        }
        Ok(BlameResp {
            path: path.to_string(),
            rev: rev.to_string(),
            lines: parse_blame(&stdout),
        })
    }
}

/// A porcelain group header: `<40-hex sha> <orig-line> <final-line> [<count>]`.
struct BlameHeader {
    sha: String,
    orig: u32,
    start: u32,
    count: u32,
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
    let count = match it.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    if it.next().is_some() {
        return None;
    }
    Some(BlameHeader {
        sha: sha.to_string(),
        orig,
        start,
        count,
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
    let mut seen: HashMap<String, (String, String, String)> = HashMap::new();
    let mut out: Vec<BlameLine> = Vec::new();
    let mut cur: Option<BlameHeader> = None;
    let (mut author, mut at, mut summary) = (None, None, None);

    for line in porcelain.lines() {
        if let Some(h) = parse_header(line) {
            cur = Some(h);
            (author, at, summary) = (None, None, None);
            continue;
        }
        if line.starts_with('\t') {
            // The file's own text: content, never metadata — and the group's end.
            let Some(h) = cur.take() else { continue };
            let meta = seen.entry(h.sha.clone()).or_insert_with(|| {
                (
                    author.take().unwrap_or_else(|| "unknown".to_string()),
                    at.take().unwrap_or_default(),
                    summary.take().unwrap_or_default(),
                )
            });
            out.push(BlameLine {
                short_sha: h.sha.chars().take(8).collect(),
                sha: h.sha,
                author: meta.0.clone(),
                at: meta.1.clone(),
                orig_line: h.orig,
                line_start: h.start,
                count: h.count,
                summary: meta.2.clone(),
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
) -> ApiResult<Json<BlameResp>> {
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Viewer).await?;
    let rev = q.rev.as_deref().map(str::trim).filter(|r| !r.is_empty());
    Ok(Json(git.blame(&q.path, rev.unwrap_or("HEAD")).await?))
}

/// Routes owned by this module (merged into `crate::http::router`).
pub fn router<S: GitCtx>() -> Router<S> {
    Router::new().route("/repos/{id}/blame", get(repo_blame::<S>))
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
        assert_eq!(lines.len(), 4, "one row per header/content pair");
        assert_eq!(lines[0].author, "Ada Lovelace");
        assert_eq!(lines[0].short_sha, "11111111");
        assert_eq!(lines[0].count, 2, "the run size comes from the header");
        assert_eq!(lines[0].at, "2023-11-14T22:13:20Z");
        assert_eq!(lines[0].summary, "first commit");
        assert_eq!(lines[1].line_start, 2);
        assert_eq!(lines[1].count, 1, "a continuation header has no count");
        assert_eq!(lines[2].author, "Grace Hopper");
        // The LAST run repeats sha_a with no fields at all — they must come back
        // from the cache, not be blanked.
        assert_eq!(lines[3].author, "Ada Lovelace");
        assert_eq!(lines[3].summary, "first commit");
        assert_eq!(lines[3].orig_line, 3);
        assert_eq!(lines[3].line_start, 4);
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
        assert_eq!(blame.lines.len(), 2);
        assert_eq!(blame.lines[0].author, "Otto Test");
        assert_eq!(blame.lines[0].summary, "add notes");
        assert_eq!(blame.lines[1].line_start, 2);

        let err = git
            .blame("-notes.md", "--output=/tmp/pwn")
            .await
            .expect_err("an option-like rev is refused");
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    }
}
