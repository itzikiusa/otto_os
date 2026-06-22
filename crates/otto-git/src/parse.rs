//! Parsers for `git` plumbing output: porcelain v2 status, branch listings,
//! `git log` records and unified diffs. Pure functions — unit-tested with
//! fixture text, exercised end-to-end from `local.rs`.

use otto_core::api::{
    BranchInfo, CommitInfo, ConflictSegment, DiffLine, DiffResp, FileChange, FileDiff, Hunk,
    LineOrigin, RepoStatusResp, StashInfo,
};
use otto_core::{Error, Result};

// ---------------------------------------------------------------------------
// Porcelain v2 status
// ---------------------------------------------------------------------------

/// Parse `git status --porcelain=v2 --branch` output.
pub fn parse_status(out: &str) -> RepoStatusResp {
    let mut branch = String::new();
    let mut upstream = None;
    let mut ahead = 0u32;
    let mut behind = 0u32;
    let mut changes = Vec::new();

    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("# ") {
            if let Some(v) = rest.strip_prefix("branch.head ") {
                branch = v.to_string();
            } else if let Some(v) = rest.strip_prefix("branch.upstream ") {
                upstream = Some(v.to_string());
            } else if let Some(v) = rest.strip_prefix("branch.ab ") {
                for tok in v.split_whitespace() {
                    if let Some(a) = tok.strip_prefix('+') {
                        ahead = a.parse().unwrap_or(0);
                    } else if let Some(b) = tok.strip_prefix('-') {
                        behind = b.parse().unwrap_or(0);
                    }
                }
            }
            continue;
        }
        if let Some(fc) = parse_status_entry(line) {
            changes.push(fc);
        }
    }

    RepoStatusResp {
        branch,
        upstream,
        ahead,
        behind,
        changes,
    }
}

fn parse_status_entry(line: &str) -> Option<FileChange> {
    let tag = line.chars().next()?;
    match tag {
        '1' => {
            // 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
            let parts: Vec<&str> = line.splitn(9, ' ').collect();
            if parts.len() < 9 {
                return None;
            }
            let xy = parts[1];
            Some(change_from_xy(xy, parts[8].to_string(), None))
        }
        '2' => {
            // 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\t<origPath>
            let parts: Vec<&str> = line.splitn(10, ' ').collect();
            if parts.len() < 10 {
                return None;
            }
            let xy = parts[1];
            let (path, orig) = match parts[9].split_once('\t') {
                Some((p, o)) => (p.to_string(), Some(o.to_string())),
                None => (parts[9].to_string(), None),
            };
            Some(change_from_xy(xy, path, orig))
        }
        'u' => {
            // u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
            let parts: Vec<&str> = line.splitn(11, ' ').collect();
            if parts.len() < 11 {
                return None;
            }
            Some(FileChange {
                path: parts[10].to_string(),
                orig_path: None,
                kind: "conflicted".into(),
                staged: false,
                unstaged: true,
            })
        }
        '?' => {
            let path = line.strip_prefix("? ")?;
            Some(FileChange {
                path: path.to_string(),
                orig_path: None,
                kind: "untracked".into(),
                staged: false,
                unstaged: true,
            })
        }
        _ => None, // '!' ignored entries, headers
    }
}

fn change_from_xy(xy: &str, path: String, orig_path: Option<String>) -> FileChange {
    let mut it = xy.chars();
    let x = it.next().unwrap_or('.');
    let y = it.next().unwrap_or('.');
    let kind = if x == 'R' || y == 'R' || x == 'C' || y == 'C' {
        "renamed"
    } else if x == 'A' || y == 'A' {
        "added"
    } else if x == 'D' || y == 'D' {
        "deleted"
    } else {
        "modified"
    };
    FileChange {
        path,
        orig_path,
        kind: kind.into(),
        staged: x != '.',
        unstaged: y != '.',
    }
}

// ---------------------------------------------------------------------------
// Conflict markers
// ---------------------------------------------------------------------------

/// Split the text of a conflicted file into ordered segments. Runs of normal
/// lines become `Context`; each `<<<<<<< … =======  … >>>>>>>` region becomes a
/// `Conflict` (with `base` populated when diff3 `|||||||` markers are present).
/// Order is preserved so the client can deterministically rebuild the file.
///
/// Marker grammar (git default + diff3):
///   `<<<<<<< ours`        start of conflict, "ours" lines follow
///   `||||||| base`        (diff3 only) start of merge-base lines
///   `=======`             switch to "theirs" lines
///   `>>>>>>> theirs`      end of conflict
pub fn parse_conflict_segments(text: &str) -> Vec<ConflictSegment> {
    let mut segments: Vec<ConflictSegment> = Vec::new();
    let mut context: Vec<String> = Vec::new();

    // Which side of the current conflict we're collecting into.
    enum Side {
        Ours,
        Base,
        Theirs,
    }
    let mut in_conflict = false;
    let mut side = Side::Ours;
    let mut ours: Vec<String> = Vec::new();
    let mut base: Vec<String> = Vec::new();
    let mut theirs: Vec<String> = Vec::new();

    let flush_context = |context: &mut Vec<String>, segments: &mut Vec<ConflictSegment>| {
        if !context.is_empty() {
            segments.push(ConflictSegment::Context {
                lines: std::mem::take(context),
            });
        }
    };

    for line in split_keep_lines(text) {
        if !in_conflict {
            if line.starts_with("<<<<<<<") {
                flush_context(&mut context, &mut segments);
                in_conflict = true;
                side = Side::Ours;
                ours.clear();
                base.clear();
                theirs.clear();
            } else {
                context.push(line.to_string());
            }
            continue;
        }

        // Inside a conflict region.
        if line.starts_with("|||||||") {
            side = Side::Base;
        } else if line.starts_with("=======") {
            side = Side::Theirs;
        } else if line.starts_with(">>>>>>>") {
            segments.push(ConflictSegment::Conflict {
                ours: std::mem::take(&mut ours),
                theirs: std::mem::take(&mut theirs),
                base: std::mem::take(&mut base),
            });
            in_conflict = false;
        } else {
            match side {
                Side::Ours => ours.push(line.to_string()),
                Side::Base => base.push(line.to_string()),
                Side::Theirs => theirs.push(line.to_string()),
            }
        }
    }

    // A never-closed conflict (malformed file): keep what we collected so the
    // client still sees the data instead of silently dropping it.
    if in_conflict {
        segments.push(ConflictSegment::Conflict { ours, theirs, base });
    }
    flush_context(&mut context, &mut segments);

    segments
}

/// Split `text` into logical lines WITHOUT their trailing `\n`, dropping a
/// single trailing empty line produced by a final newline (so a file ending in
/// "\n" doesn't yield a spurious empty context line).
fn split_keep_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if matches!(lines.last(), Some(&"")) {
        lines.pop();
    }
    lines
}

// ---------------------------------------------------------------------------
// Branch list
// ---------------------------------------------------------------------------

/// Parse `git branch --format=%(refname:short)%09%(upstream:short)%09%(HEAD)`.
pub fn parse_branches(out: &str) -> Vec<BranchInfo> {
    out.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('(')) // skip "(HEAD detached …)"
        .map(|line| {
            let mut cols = line.split('\t');
            let name = cols.next().unwrap_or("").to_string();
            let upstream = cols.next().unwrap_or("").trim();
            let head = cols.next().unwrap_or("").trim();
            BranchInfo {
                name,
                is_current: head == "*",
                upstream: if upstream.is_empty() {
                    None
                } else {
                    Some(upstream.to_string())
                },
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Log
// ---------------------------------------------------------------------------

/// Parse `git log --pretty=format:%H%x1f%h%x1f%an%x1f%aI%x1f%s%x1f%P%x1f%D%x1e` output.
/// Fields: sha, short_sha, author, dateISO, subject, parents (space-sep), refs (comma-sep).
pub fn parse_log(out: &str) -> Result<Vec<CommitInfo>> {
    let mut commits = Vec::new();
    for rec in out.split('\u{1e}') {
        let rec = rec.trim_matches(['\n', '\r']);
        if rec.is_empty() {
            continue;
        }
        let fields: Vec<&str> = rec.split('\u{1f}').collect();
        if fields.len() < 5 {
            return Err(Error::Internal(format!("bad log record: {rec:?}")));
        }
        let date = chrono::DateTime::parse_from_rfc3339(fields[3])
            .map_err(|e| Error::Internal(format!("bad commit date {}: {e}", fields[3])))?
            .with_timezone(&chrono::Utc);

        // parents field (index 5): space-separated full SHAs; may be absent for old format
        let parents: Vec<String> = if fields.len() > 5 {
            fields[5]
                .split_whitespace()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        };

        // refs field (index 6): comma-separated decoration names from %D, e.g.
        //   "HEAD -> main, origin/main, origin/HEAD, tag: v1.0".
        // PRESERVE the HEAD marker so the client can tell which commit is checked
        // out: a "HEAD -> <branch>" token is emitted verbatim (the frontend reads
        // the branch after the arrow and the HEAD-ness from the prefix); a bare
        // "HEAD" (detached) is kept too. Everything else (branches/tags) passes
        // through unchanged. Empties are dropped.
        let refs: Vec<String> = if fields.len() > 6 {
            fields[6]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            Vec::new()
        };

        commits.push(CommitInfo {
            sha: fields[0].to_string(),
            short_sha: fields[1].to_string(),
            author: fields[2].to_string(),
            date,
            subject: fields[4].to_string(),
            parents,
            refs,
        });
    }
    Ok(commits)
}

// ---------------------------------------------------------------------------
// Stash list
// ---------------------------------------------------------------------------

/// Parse `git stash list --pretty=format:%gd%x1f%H%x1f%P%x1f%aI%x1f%gs`.
/// Fields per line: selector (`stash@{N}`), sha, parents (space-sep), dateISO,
/// reflog subject (`%gs`). Malformed lines are skipped rather than failing the
/// whole listing (an empty list is the common, healthy case).
pub fn parse_stash_list(out: &str) -> Vec<StashInfo> {
    let mut stashes = Vec::new();
    for line in out.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\u{1f}').collect();
        if fields.len() < 5 {
            continue;
        }
        let selector = fields[0].trim();
        // index = N in "stash@{N}"; default 0 if the selector is unexpected.
        let index = selector
            .strip_prefix("stash@{")
            .and_then(|r| r.strip_suffix('}'))
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or(0);
        let parents: Vec<String> = fields[2]
            .split_whitespace()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let message = fields[4].trim().to_string();
        let branch = parse_stash_branch(&message);
        stashes.push(StashInfo {
            index,
            r#ref: selector.to_string(),
            sha: fields[1].trim().to_string(),
            parents,
            date: fields[3].trim().to_string(),
            message,
            branch,
        });
    }
    stashes
}

/// Extract the branch from a stash reflog subject: "WIP on main: …" or
/// "On main: …" → `Some("main")`; anything else → `None`.
fn parse_stash_branch(msg: &str) -> Option<String> {
    let rest = msg
        .strip_prefix("WIP on ")
        .or_else(|| msg.strip_prefix("On "))?;
    let branch = rest.split(':').next()?.trim();
    // A stash taken on a detached HEAD reads "On (no branch): …" — not a branch.
    if branch.is_empty() || branch == "(no branch)" {
        None
    } else {
        Some(branch.to_string())
    }
}

// ---------------------------------------------------------------------------
// Unified diff
// ---------------------------------------------------------------------------

/// Parse the output of `git diff --no-color -U3 -M` (or any unified diff with
/// `diff --git` file headers) into structured per-file hunks.
pub fn parse_diff(text: &str) -> DiffResp {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileState> = None;

    for line in text.lines() {
        if line.starts_with("diff --git ") {
            if let Some(f) = cur.take() {
                files.push(f.finish());
            }
            cur = Some(FileState::new(line));
            continue;
        }
        let Some(state) = cur.as_mut() else { continue };
        state.feed(line);
    }
    if let Some(f) = cur.take() {
        files.push(f.finish());
    }
    DiffResp { files }
}

/// Parse a bare hunk body (lines starting at `@@`) without `diff --git`
/// headers — used for GitLab `changes[].diff` payloads.
pub fn parse_hunks(text: &str) -> Vec<Hunk> {
    let mut st = FileState::new("diff --git a/x b/x");
    for line in text.lines() {
        st.feed(line);
    }
    st.finish().hunks
}

struct FileState {
    git_old: Option<String>,
    git_new: Option<String>,
    minus_path: Option<String>, // from "--- a/…"
    plus_path: Option<String>,  // from "+++ b/…"
    rename_from: Option<String>,
    rename_to: Option<String>,
    is_binary: bool,
    hunks: Vec<Hunk>,
    old_line: u32,
    new_line: u32,
    in_hunk: bool,
    added_lines: u32,
    deleted_lines: u32,
}

impl FileState {
    fn new(diff_git_line: &str) -> Self {
        let (git_old, git_new) = parse_diff_git_paths(diff_git_line);
        Self {
            git_old,
            git_new,
            minus_path: None,
            plus_path: None,
            rename_from: None,
            rename_to: None,
            is_binary: false,
            hunks: Vec::new(),
            old_line: 0,
            new_line: 0,
            in_hunk: false,
            added_lines: 0,
            deleted_lines: 0,
        }
    }

    fn feed(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix("@@") {
            if let Some((old_start, new_start)) = parse_hunk_header(rest) {
                self.hunks.push(Hunk {
                    header: line.to_string(),
                    lines: Vec::new(),
                });
                self.old_line = old_start;
                self.new_line = new_start;
                self.in_hunk = true;
                return;
            }
        }
        if self.in_hunk {
            let Some(origin_char) = line.chars().next() else {
                // A fully empty line inside a hunk is a context line whose
                // content is empty (git prints " " but be lenient).
                self.push_line(LineOrigin::Context, "");
                return;
            };
            match origin_char {
                ' ' => self.push_line(LineOrigin::Context, &line[1..]),
                '+' => self.push_line(LineOrigin::Add, &line[1..]),
                '-' => self.push_line(LineOrigin::Del, &line[1..]),
                '\\' => {} // "\ No newline at end of file"
                _ => self.in_hunk = false,
            }
            if self.in_hunk {
                return;
            }
        }
        // header territory
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            self.is_binary = true;
        } else if let Some(v) = line.strip_prefix("rename from ") {
            self.rename_from = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("rename to ") {
            self.rename_to = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("--- ") {
            self.minus_path = strip_ab_prefix(v);
        } else if let Some(v) = line.strip_prefix("+++ ") {
            self.plus_path = strip_ab_prefix(v);
        }
    }

    fn push_line(&mut self, origin: LineOrigin, content: &str) {
        let (old_line, new_line) = match origin {
            LineOrigin::Context => {
                let p = (Some(self.old_line), Some(self.new_line));
                self.old_line += 1;
                self.new_line += 1;
                p
            }
            LineOrigin::Add => {
                let p = (None, Some(self.new_line));
                self.new_line += 1;
                self.added_lines += 1;
                p
            }
            LineOrigin::Del => {
                let p = (Some(self.old_line), None);
                self.old_line += 1;
                self.deleted_lines += 1;
                p
            }
        };
        if let Some(h) = self.hunks.last_mut() {
            h.lines.push(DiffLine {
                origin,
                content: content.to_string(),
                old_line,
                new_line,
            });
        }
    }

    fn finish(self) -> FileDiff {
        // Current path: prefer "+++ b/…", then rename-to, then diff --git's b side.
        let path = self
            .plus_path
            .clone()
            .or_else(|| self.rename_to.clone())
            .or_else(|| {
                // deleted file: +++ is /dev/null → use the old side
                self.minus_path.clone()
            })
            .or_else(|| self.git_new.clone())
            .or(self.git_old.clone())
            .unwrap_or_default();
        // Old path only when it differs (rename/copy).
        let old_path = self
            .rename_from
            .clone()
            .or_else(|| match (&self.minus_path, &self.plus_path) {
                (Some(o), Some(n)) if o != n => Some(o.clone()),
                _ => None,
            })
            .or_else(|| match (&self.git_old, &self.git_new) {
                (Some(o), Some(n)) if o != n => Some(o.clone()),
                _ => None,
            });
        let language = lang_from_ext(&path);
        FileDiff {
            path,
            old_path,
            is_binary: self.is_binary,
            hunks: self.hunks,
            too_large: None,
            added: Some(self.added_lines),
            deleted: Some(self.deleted_lines),
            language,
        }
    }
}

fn lang_from_ext(path: &str) -> Option<String> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    let lang = match ext {
        "rs" => "rust", "go" => "go", "py" => "python", "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript", "tsx" => "tsx", "jsx" => "jsx",
        "svelte" => "svelte", "vue" => "vue",
        "java" => "java", "kt" => "kotlin", "scala" => "scala",
        "c" | "h" => "c", "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp", "rb" => "ruby", "php" => "php", "swift" => "swift",
        "sh" | "bash" | "zsh" => "shell", "yaml" | "yml" => "yaml", "toml" => "toml",
        "json" => "json", "md" => "markdown", "sql" => "sql", "html" => "html",
        "css" => "css", "scss" | "sass" => "scss", "xml" => "xml",
        _ => return None,
    };
    Some(lang.to_string())
}

/// "--- a/path" → Some("path"); "--- /dev/null" → None. Quoted paths get the
/// surrounding quotes stripped (escapes left as-is, best effort).
fn strip_ab_prefix(v: &str) -> Option<String> {
    let v = v.trim_end();
    let v = v.trim_matches('"');
    if v == "/dev/null" {
        return None;
    }
    let v = v
        .strip_prefix("a/")
        .or_else(|| v.strip_prefix("b/"))
        .unwrap_or(v);
    Some(v.to_string())
}

/// "diff --git a/old b/new" → (Some(old), Some(new)). Best effort: paths with
/// the literal substring " b/" are ambiguous; the ---/+++ lines win anyway.
fn parse_diff_git_paths(line: &str) -> (Option<String>, Option<String>) {
    let rest = match line.strip_prefix("diff --git ") {
        Some(r) => r,
        None => return (None, None),
    };
    if let Some(idx) = rest.rfind(" b/") {
        let old = rest[..idx].trim().trim_matches('"');
        let new = rest[idx + 3..].trim().trim_matches('"');
        let old = old.strip_prefix("a/").unwrap_or(old);
        return (Some(old.to_string()), Some(new.to_string()));
    }
    (None, None)
}

/// Parse the "@@ -a,b +c,d @@ …" header tail (after the leading "@@") into
/// (old_start, new_start).
fn parse_hunk_header(rest: &str) -> Option<(u32, u32)> {
    let body = rest.split("@@").next()?.trim();
    let mut old_start = None;
    let mut new_start = None;
    for tok in body.split_whitespace() {
        if let Some(v) = tok.strip_prefix('-') {
            old_start = v.split(',').next()?.parse::<u32>().ok();
        } else if let Some(v) = tok.strip_prefix('+') {
            new_start = v.split(',').next()?.parse::<u32>().ok();
        }
    }
    Some((old_start?, new_start?))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stash_list_parses_selector_parents_and_branch() {
        // %gd \x1f %H \x1f %P \x1f %aI \x1f %gs
        let us = '\u{1f}';
        let out = format!(
            "stash@{{0}}{us}dd92c29aaa{us}ebe28ba 931dcdd{us}2026-06-22T12:30:45+03:00{us}WIP on main: my work\n\
             stash@{{1}}{us}aaaa111{us}bbbb222{us}2026-06-20T09:00:00+00:00{us}On feature/x: spike",
        );
        let stashes = parse_stash_list(&out);
        assert_eq!(stashes.len(), 2);
        assert_eq!(stashes[0].index, 0);
        assert_eq!(stashes[0].r#ref, "stash@{0}");
        assert_eq!(stashes[0].sha, "dd92c29aaa");
        assert_eq!(stashes[0].parents, vec!["ebe28ba", "931dcdd"]);
        assert_eq!(stashes[0].branch.as_deref(), Some("main"));
        assert_eq!(stashes[0].message, "WIP on main: my work");
        assert_eq!(stashes[1].index, 1);
        assert_eq!(stashes[1].branch.as_deref(), Some("feature/x"));
        // detached-HEAD stash → "(no branch)" is NOT a real branch label
        let detached = format!(
            "stash@{{0}}{us}c0ffee{us}d00d{us}2026-06-22T00:00:00+00:00{us}On (no branch): poke",
        );
        assert_eq!(parse_stash_list(&detached)[0].branch, None);
        // empty input → empty list (the common, healthy case)
        assert!(parse_stash_list("").is_empty());
        assert!(parse_stash_list("\n  \n").is_empty());
    }

    #[test]
    fn status_branch_and_entries() {
        let out = "\
# branch.oid 1234567890abcdef
# branch.head main
# branch.upstream origin/main
# branch.ab +2 -1
1 .M N... 100644 100644 100644 aaa bbb src/lib.rs
1 A. N... 000000 100644 100644 000 ccc new_file.rs
1 .D N... 100644 100644 000000 ddd eee gone.rs
2 R. N... 100644 100644 100644 fff ggg R100 renamed.rs\told name.rs
u UU N... 100644 100644 100644 100644 h1 h2 h3 conflict.rs
? untracked.txt
";
        let st = parse_status(out);
        assert_eq!(st.branch, "main");
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!(st.ahead, 2);
        assert_eq!(st.behind, 1);
        assert_eq!(st.changes.len(), 6);

        let m = &st.changes[0];
        assert_eq!(
            (m.path.as_str(), m.kind.as_str(), m.staged, m.unstaged),
            ("src/lib.rs", "modified", false, true)
        );
        let a = &st.changes[1];
        assert_eq!(
            (a.path.as_str(), a.kind.as_str(), a.staged, a.unstaged),
            ("new_file.rs", "added", true, false)
        );
        let d = &st.changes[2];
        assert_eq!(
            (d.kind.as_str(), d.staged, d.unstaged),
            ("deleted", false, true)
        );
        let r = &st.changes[3];
        assert_eq!(r.path, "renamed.rs");
        assert_eq!(r.orig_path.as_deref(), Some("old name.rs"));
        assert_eq!(
            (r.kind.as_str(), r.staged, r.unstaged),
            ("renamed", true, false)
        );
        let c = &st.changes[4];
        assert_eq!(
            (c.path.as_str(), c.kind.as_str()),
            ("conflict.rs", "conflicted")
        );
        let u = &st.changes[5];
        assert_eq!(
            (u.path.as_str(), u.kind.as_str()),
            ("untracked.txt", "untracked")
        );
    }

    #[test]
    fn status_detached_no_upstream() {
        let out = "# branch.oid abc\n# branch.head (detached)\n";
        let st = parse_status(out);
        assert_eq!(st.branch, "(detached)");
        assert!(st.upstream.is_none());
        assert_eq!((st.ahead, st.behind), (0, 0));
        assert!(st.changes.is_empty());
    }

    #[test]
    fn branches_parse() {
        let out = "main\torigin/main\t*\nfeature/x\t\t \n(HEAD detached at abc123)\t\t\n";
        let b = parse_branches(out);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].name, "main");
        assert!(b[0].is_current);
        assert_eq!(b[0].upstream.as_deref(), Some("origin/main"));
        assert_eq!(b[1].name, "feature/x");
        assert!(!b[1].is_current);
        assert!(b[1].upstream.is_none());
    }

    #[test]
    fn log_parse() {
        let out = "abc123\u{1f}abc\u{1f}Alice\u{1f}2026-06-01T10:00:00+02:00\u{1f}feat: one\u{1e}\ndef456\u{1f}def\u{1f}Bob\u{1f}2026-05-31T09:00:00Z\u{1f}fix: two\u{1e}";
        let log = parse_log(out).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].sha, "abc123");
        assert_eq!(log[0].short_sha, "abc");
        assert_eq!(log[0].author, "Alice");
        assert_eq!(log[0].subject, "feat: one");
        assert_eq!(log[0].date.to_rfc3339(), "2026-06-01T08:00:00+00:00");
        assert_eq!(log[1].subject, "fix: two");
    }

    #[test]
    fn log_parse_preserves_head_decoration() {
        // The HEAD commit carries a "%D" decoration with the checked-out branch,
        // remote refs and a tag. The parser must PRESERVE "HEAD -> <branch>" (so the
        // client can render the checked-out branch + a "you are here" marker) and
        // keep a bare "HEAD" for the detached case. Parents are space-separated.
        let out = concat!(
            "abc123\u{1f}abc\u{1f}Alice\u{1f}2026-06-01T10:00:00+02:00\u{1f}feat: one",
            "\u{1f}p1 p2\u{1f}HEAD -> main, origin/main, origin/HEAD, tag: v1.0\u{1e}\n",
            // detached HEAD on the next commit: %D = "HEAD, origin/release"
            "def456\u{1f}def\u{1f}Bob\u{1f}2026-05-31T09:00:00Z\u{1f}fix: two",
            "\u{1f}p3\u{1f}HEAD, origin/release\u{1e}"
        );
        let log = parse_log(out).unwrap();
        assert_eq!(log.len(), 2);

        // HEAD -> main is kept verbatim (NOT stripped) alongside the other refs.
        assert_eq!(
            log[0].refs,
            vec![
                "HEAD -> main".to_string(),
                "origin/main".to_string(),
                "origin/HEAD".to_string(),
                "tag: v1.0".to_string(),
            ]
        );
        assert_eq!(log[0].parents, vec!["p1".to_string(), "p2".to_string()]);

        // Detached HEAD: a bare "HEAD" token survives.
        assert_eq!(
            log[1].refs,
            vec!["HEAD".to_string(), "origin/release".to_string()]
        );
    }

    #[test]
    fn diff_modified_line_numbers() {
        let text = "\
diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,7 +10,8 @@ fn main() {
 context one
-removed line
+added line
+second added
 context two
@@ -30,3 +31,3 @@
 ctx
-old
+new
";
        let d = parse_diff(text);
        assert_eq!(d.files.len(), 1);
        let f = &d.files[0];
        assert_eq!(f.path, "src/main.rs");
        assert!(f.old_path.is_none());
        assert!(!f.is_binary);
        assert_eq!(f.hunks.len(), 2);

        let h = &f.hunks[0];
        assert_eq!(h.header, "@@ -10,7 +10,8 @@ fn main() {");
        let l = &h.lines;
        assert_eq!(l.len(), 5);
        // context one: old 10 / new 10
        assert_eq!(
            (l[0].origin, l[0].old_line, l[0].new_line),
            (LineOrigin::Context, Some(10), Some(10))
        );
        // removed: old 11
        assert_eq!(
            (l[1].origin, l[1].old_line, l[1].new_line),
            (LineOrigin::Del, Some(11), None)
        );
        // added: new 11, 12
        assert_eq!(
            (l[2].origin, l[2].old_line, l[2].new_line),
            (LineOrigin::Add, None, Some(11))
        );
        assert_eq!((l[3].origin, l[3].new_line), (LineOrigin::Add, Some(12)));
        // context two: old 12 / new 13
        assert_eq!((l[4].old_line, l[4].new_line), (Some(12), Some(13)));
        assert_eq!(l[1].content, "removed line");

        let h2 = &f.hunks[1];
        assert_eq!(h2.lines[0].old_line, Some(30));
        assert_eq!(h2.lines[0].new_line, Some(31));
    }

    #[test]
    fn diff_rename_and_binary_and_new_file() {
        let text = "\
diff --git a/old/name.txt b/new/name.txt
similarity index 95%
rename from old/name.txt
rename to new/name.txt
index 111..222 100644
--- a/old/name.txt
+++ b/new/name.txt
@@ -1,2 +1,2 @@
 keep
-foo
+bar
diff --git a/img.png b/img.png
index 333..444 100644
Binary files a/img.png and b/img.png differ
diff --git a/pure-rename.txt b/moved.txt
similarity index 100%
rename from pure-rename.txt
rename to moved.txt
diff --git a/brand_new.rs b/brand_new.rs
new file mode 100644
index 0000000..555
--- /dev/null
+++ b/brand_new.rs
@@ -0,0 +1,2 @@
+line one
+line two
diff --git a/dead.rs b/dead.rs
deleted file mode 100644
index 666..0000000
--- a/dead.rs
+++ /dev/null
@@ -1,1 +0,0 @@
-bye
";
        let d = parse_diff(text);
        assert_eq!(d.files.len(), 5);

        let ren = &d.files[0];
        assert_eq!(ren.path, "new/name.txt");
        assert_eq!(ren.old_path.as_deref(), Some("old/name.txt"));
        assert_eq!(ren.hunks.len(), 1);

        let bin = &d.files[1];
        assert_eq!(bin.path, "img.png");
        assert!(bin.is_binary);
        assert!(bin.hunks.is_empty());

        let pure = &d.files[2];
        assert_eq!(pure.path, "moved.txt");
        assert_eq!(pure.old_path.as_deref(), Some("pure-rename.txt"));
        assert!(pure.hunks.is_empty());

        let new = &d.files[3];
        assert_eq!(new.path, "brand_new.rs");
        assert!(new.old_path.is_none());
        assert_eq!(new.hunks[0].lines.len(), 2);
        assert_eq!(new.hunks[0].lines[0].new_line, Some(1));
        assert_eq!(new.hunks[0].lines[1].new_line, Some(2));

        let dead = &d.files[4];
        assert_eq!(dead.path, "dead.rs");
        assert_eq!(dead.hunks[0].lines[0].old_line, Some(1));
        assert_eq!(dead.hunks[0].lines[0].origin, LineOrigin::Del);
    }

    #[test]
    fn diff_no_newline_marker_skipped() {
        let text = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1 @@
-x
\\ No newline at end of file
+y
\\ No newline at end of file
";
        let d = parse_diff(text);
        let lines = &d.files[0].hunks[0].lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].origin, LineOrigin::Del);
        assert_eq!(lines[1].origin, LineOrigin::Add);
    }

    #[test]
    fn parse_hunks_bare() {
        let text = "@@ -1,2 +1,3 @@\n a\n+b\n c\n";
        let hunks = parse_hunks(text);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines.len(), 3);
        assert_eq!(hunks[0].lines[1].origin, LineOrigin::Add);
        assert_eq!(hunks[0].lines[2].old_line, Some(2));
        assert_eq!(hunks[0].lines[2].new_line, Some(3));
    }

    #[test]
    fn conflict_segments_two_conflicts() {
        // A file with leading context, a first conflict, middle context, a
        // second conflict, and trailing context. Standard (non-diff3) markers.
        let text = "\
line 1
line 2
<<<<<<< HEAD
ours a
ours b
=======
theirs a
>>>>>>> feature
middle 1
middle 2
<<<<<<< HEAD
ours c
=======
theirs c
theirs d
>>>>>>> feature
last line
";
        let segs = parse_conflict_segments(text);
        // ctx, conflict, ctx, conflict, ctx
        assert_eq!(segs.len(), 5);

        match &segs[0] {
            ConflictSegment::Context { lines } => {
                assert_eq!(lines, &["line 1".to_string(), "line 2".to_string()]);
            }
            other => panic!("expected context, got {other:?}"),
        }
        match &segs[1] {
            ConflictSegment::Conflict { ours, theirs, base } => {
                assert_eq!(ours, &["ours a".to_string(), "ours b".to_string()]);
                assert_eq!(theirs, &["theirs a".to_string()]);
                assert!(base.is_empty());
            }
            other => panic!("expected conflict, got {other:?}"),
        }
        match &segs[2] {
            ConflictSegment::Context { lines } => {
                assert_eq!(lines, &["middle 1".to_string(), "middle 2".to_string()]);
            }
            other => panic!("expected context, got {other:?}"),
        }
        match &segs[3] {
            ConflictSegment::Conflict { ours, theirs, base } => {
                assert_eq!(ours, &["ours c".to_string()]);
                assert_eq!(theirs, &["theirs c".to_string(), "theirs d".to_string()]);
                assert!(base.is_empty());
            }
            other => panic!("expected conflict, got {other:?}"),
        }
        match &segs[4] {
            ConflictSegment::Context { lines } => {
                assert_eq!(lines, &["last line".to_string()]);
            }
            other => panic!("expected context, got {other:?}"),
        }

        // Round-trip sanity: reassembling "ours" reproduces the our-side file.
        let mut rebuilt = Vec::new();
        for s in &segs {
            match s {
                ConflictSegment::Context { lines } => rebuilt.extend(lines.clone()),
                ConflictSegment::Conflict { ours, .. } => rebuilt.extend(ours.clone()),
            }
        }
        assert_eq!(
            rebuilt,
            vec![
                "line 1",
                "line 2",
                "ours a",
                "ours b",
                "middle 1",
                "middle 2",
                "ours c",
                "last line"
            ]
        );
    }

    #[test]
    fn conflict_segments_diff3_base() {
        // diff3 output adds a ||||||| base section between ours and theirs.
        let text = "\
prefix
<<<<<<< HEAD
ours line
||||||| merged common ancestors
base line 1
base line 2
=======
theirs line
>>>>>>> other
suffix
";
        let segs = parse_conflict_segments(text);
        assert_eq!(segs.len(), 3);
        match &segs[1] {
            ConflictSegment::Conflict { ours, theirs, base } => {
                assert_eq!(ours, &["ours line".to_string()]);
                assert_eq!(theirs, &["theirs line".to_string()]);
                assert_eq!(
                    base,
                    &["base line 1".to_string(), "base line 2".to_string()]
                );
            }
            other => panic!("expected conflict, got {other:?}"),
        }
    }
}
