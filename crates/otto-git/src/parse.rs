//! Parsers for `git` plumbing output: porcelain v2 status, branch listings,
//! `git log` records and unified diffs. Pure functions — unit-tested with
//! fixture text, exercised end-to-end from `local.rs`.

use otto_core::api::{
    BranchInfo, CommitInfo, ConflictSegment, DiffLine, DiffResp, FileChange, FileChangeStatus,
    FileDiff, Hunk, LineOrigin, RepoStatusResp, StashInfo, SubmoduleInfo, WorktreeInfo,
};
use otto_core::{Error, Result};

// ---------------------------------------------------------------------------
// Porcelain v2 status
// ---------------------------------------------------------------------------

/// Parse `git status --porcelain=v2 --branch [-z]` output.
///
/// With `-z` (what [`crate::local::LocalGit::status`] runs) every record is
/// NUL-terminated, names are NEVER quoted, and a rename/copy (`2`) entry is
/// followed by its origPath as the NEXT record. Newline-separated output (a
/// `\t` between path and origPath, quoted names) is still accepted.
pub fn parse_status(out: &str) -> RepoStatusResp {
    parse_status_capped(out, usize::MAX)
}

/// Untracked rows a status RESPONSE carries at most. A non-ignored
/// `node_modules`/build dir is 200k rows ≈ 30 MB of JSON per watcher event;
/// past the cap the client gets a count and a "add it to .gitignore" hint.
pub const UNTRACKED_ROW_CAP: usize = 5_000;

/// [`parse_status`] keeping only the first `cap` untracked (`?`) rows — the
/// rest are counted, never allocated. Tracked, staged and conflicted rows are
/// never capped (they are what a commit is made of).
pub fn parse_status_capped(out: &str, cap: usize) -> RepoStatusResp {
    let mut untracked = 0usize;
    let mut branch = String::new();
    let mut upstream = None;
    let mut ahead = 0u32;
    let mut behind = 0u32;
    let mut changes = Vec::new();

    let nul = out.contains('\0');
    let mut records: Box<dyn Iterator<Item = &str> + '_> = if nul {
        Box::new(out.split('\0').filter(|r| !r.is_empty()))
    } else {
        Box::new(out.lines())
    };
    while let Some(line) = records.next() {
        if nul && line.starts_with("2 ") {
            // `2 … <path>\0<origPath>\0`: the origPath is its own record.
            let orig = records.next();
            if let Some(fc) = parse_rename_entry(line, orig) {
                changes.push(fc);
            }
            continue;
        }
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
        if line.starts_with("? ") {
            untracked += 1;
            if untracked > cap {
                continue;
            }
        }
        if let Some(fc) = parse_status_entry(line) {
            changes.push(fc);
        }
    }

    let truncated = untracked > cap;
    RepoStatusResp {
        branch,
        upstream,
        ahead,
        behind,
        changes,
        // Filled by LocalGit::status from the git dir's state files — the
        // porcelain output alone can't tell a merge from a rebase.
        op_in_progress: None,
        untracked_total: truncated.then(|| u32::try_from(untracked).unwrap_or(u32::MAX)),
        untracked_truncated: truncated,
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
            parse_rename_entry(line, None)
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

/// A `2` (rename/copy) entry. `orig` is the origPath record that follows it
/// under `-z`; `None` means the newline format, where it rides after a `\t`.
fn parse_rename_entry(line: &str, orig: Option<&str>) -> Option<FileChange> {
    // 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>
    let parts: Vec<&str> = line.splitn(10, ' ').collect();
    if parts.len() < 10 {
        return None;
    }
    let xy = parts[1];
    let (path, orig) = match orig {
        Some(o) => (parts[9].to_string(), Some(o.to_string())),
        None => match parts[9].split_once('\t') {
            Some((p, o)) => (p.to_string(), Some(o.to_string())),
            None => (parts[9].to_string(), None),
        },
    };
    Some(change_from_xy(xy, path, orig))
}

fn change_from_xy(xy: &str, path: String, orig_path: Option<String>) -> FileChange {
    let mut it = xy.chars();
    let x = it.next().unwrap_or('.');
    let y = it.next().unwrap_or('.');
    // Only a RENAME is "renamed": discard/unstage expand a renamed entry to
    // its origPath too. A COPY (`C`, with `status.renames=copies`) leaves its
    // source in place — expanding it reverted the user's uncommitted edits
    // in the source file. A copy is a new file: "added" (origPath kept as a
    // hint), which discard removes and unstage un-adds, source untouched.
    let kind = if x == 'R' || y == 'R' {
        "renamed"
    } else if x == 'A' || y == 'A' || x == 'C' || y == 'C' {
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

/// Parse `%(upstream:track,nobracket)` from a bulk for-each-ref query.
/// LocalGit pins LC_ALL=C; equal, absent and gone upstreams have no counts.
pub fn parse_upstream_tracking(out: &str) -> (u32, u32) {
    let (mut ahead, mut behind) = (0, 0);
    for part in out.split(',').map(str::trim) {
        if let Some(value) = part.strip_prefix("ahead ") {
            ahead = value.parse().unwrap_or(0);
        } else if let Some(value) = part.strip_prefix("behind ") {
            behind = value.parse().unwrap_or(0);
        }
    }
    (ahead, behind)
}

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

/// Parse `git worktree list --porcelain` output. Entries are blank-line
/// separated attribute blocks; the FIRST entry is always the main worktree.
/// `dirty` is left false here — `local.rs` fills it in with a per-worktree
/// status probe (it needs process access this pure parser doesn't have).
pub fn parse_worktree_list(out: &str) -> Vec<WorktreeInfo> {
    let mut wts: Vec<WorktreeInfo> = Vec::new();
    let mut cur: Option<WorktreeInfo> = None;
    for line in out.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if let Some(wt) = cur.take() {
                wts.push(wt);
            }
            continue;
        }
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(wt) = cur.take() {
                wts.push(wt);
            }
            cur = Some(WorktreeInfo {
                path: path.to_string(),
                head: String::new(),
                branch: None,
                is_main: wts.is_empty(),
                locked: false,
                lock_reason: None,
                prunable: false,
                dirty: false,
                dirty_known: false,
            });
            continue;
        }
        let Some(wt) = cur.as_mut() else { continue };
        if let Some(sha) = line.strip_prefix("HEAD ") {
            wt.head = sha.trim().to_string();
        } else if let Some(branch) = line.strip_prefix("branch ") {
            wt.branch = Some(
                branch
                    .trim()
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch.trim())
                    .to_string(),
            );
        } else if line == "detached" {
            wt.branch = None;
        } else if line == "locked" || line.starts_with("locked ") {
            wt.locked = true;
            wt.lock_reason = line
                .strip_prefix("locked ")
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_string);
        } else if line == "prunable" || line.starts_with("prunable ") {
            wt.prunable = true;
        }
        // "bare" and unknown future attributes are ignored.
    }
    if let Some(wt) = cur.take() {
        wts.push(wt);
    }
    wts
}

/// Parse `git submodule status` output. Line shape:
/// `<state-char><sha> <path> (<describe>)` where the leading char is
/// ' ' = ok, '-' = uninitialized, '+' = checked-out ≠ recorded, 'U' = conflict.
/// Malformed lines are skipped (empty output = no submodules, the common case).
pub fn parse_submodule_status(out: &str) -> Vec<SubmoduleInfo> {
    let mut subs = Vec::new();
    for line in out.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        let (state_ch, rest) = line.split_at(1);
        let state = match state_ch {
            "-" => "uninitialized",
            "+" => "modified",
            "U" => "conflict",
            _ => "ok",
        };
        let mut it = rest.trim_start().splitn(2, ' ');
        let (Some(sha), Some(tail)) = (it.next(), it.next()) else {
            continue;
        };
        // Path may itself contain " (" — take the describe from the LAST " ("
        // only when the line ends with ")" (git prints "(<describe>)" or nothing).
        let (path, describe) = match (tail.rfind(" ("), tail.ends_with(')')) {
            (Some(i), true) => (
                tail[..i].trim(),
                Some(tail[i + 2..tail.len() - 1].to_string()),
            ),
            _ => (tail.trim(), None),
        };
        if sha.is_empty() || path.is_empty() {
            continue;
        }
        subs.push(SubmoduleInfo {
            path: path.to_string(),
            sha: sha.to_string(),
            state: state.to_string(),
            describe,
            url: None,
            branch: None,
        });
    }
    subs
}

/// Enrich parsed submodules with `.gitmodules` config (`git config -f
/// .gitmodules --list` output): submodule.<name>.path/url/branch triples are
/// grouped by <name> and matched to entries by path.
pub fn enrich_submodules(subs: &mut [SubmoduleInfo], config_list: &str) {
    use std::collections::HashMap;
    // name → (path, url, branch)
    type ModCfg<'a> = (Option<&'a str>, Option<&'a str>, Option<&'a str>);
    let mut by_name: HashMap<&str, ModCfg> = HashMap::new();
    for line in config_list.lines() {
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let Some(rest) = key.strip_prefix("submodule.") else {
            continue;
        };
        // The submodule NAME may contain dots; the trailing segment is the field.
        let Some((name, field)) = rest.rsplit_once('.') else {
            continue;
        };
        let entry = by_name.entry(name).or_default();
        match field {
            "path" => entry.0 = Some(val),
            "url" => entry.1 = Some(val),
            "branch" => entry.2 = Some(val),
            _ => {}
        }
    }
    for (path, url, branch) in by_name.values() {
        let Some(path) = path else { continue };
        if let Some(sub) = subs.iter_mut().find(|s| s.path == *path) {
            sub.url = url.map(str::to_string);
            sub.branch = branch.map(str::to_string);
        }
    }
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
    parse_diff_bytes(text.as_bytes())
}

/// Size limits applied while a diff is parsed (the HTTP routes pass these;
/// internal consumers parse uncapped). "Bytes" are rendered text: each kept
/// line's content plus its newline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffCaps {
    /// One file above either limit → `too_large`, `hunks: []` (counts kept).
    pub file_lines: usize,
    pub file_bytes: usize,
    /// Whole-response budget: the first file that doesn't fit, and every file
    /// after it, comes back `hunks_omitted` and the response `truncated`.
    pub total_lines: usize,
    pub total_bytes: usize,
}

impl DiffCaps {
    /// The default per-file cap (200 KB / 5,000 lines) and response budget
    /// (20,000 lines / 4 MB) every non-summary `/diff` response is held to.
    pub const DEFAULT: Self = Self {
        file_lines: 5_000,
        file_bytes: 200 * 1024,
        total_lines: 20_000,
        total_bytes: 4 * 1024 * 1024,
    };
    /// `?full=true` on a single-file request: the per-file cap is lifted to a
    /// hard ceiling (50,000 lines / 5 MB) that still ends in `too_large`.
    pub const FULL_FILE: Self = Self {
        file_lines: 50_000,
        file_bytes: 5 * 1024 * 1024,
        total_lines: 50_000,
        total_bytes: 5 * 1024 * 1024,
    };
}

/// [`parse_diff`] over git's RAW output. Each file's `fingerprint` hashes the
/// exact BYTES git printed — the same bytes `patch::run_hunk_op` re-reads,
/// hashes and patches — while the rendered text is decoded lossily line by
/// line (`\n` is never part of a multi-byte sequence, so lines map 1:1). A
/// non-UTF-8 line therefore displays with U+FFFD but is never STAGED that way.
pub fn parse_diff_bytes(bytes: &[u8]) -> DiffResp {
    parse_diff_bytes_capped(bytes, None)
}

/// [`parse_diff_bytes`] held to `caps`: a file over the per-file cap stops
/// storing lines as soon as it crosses it, and once the response budget is
/// spent later files store none at all — so a 100k-line patch never becomes
/// 100k `DiffLine`s. Counts, status and fingerprints are always complete.
pub fn parse_diff_bytes_capped(bytes: &[u8], caps: Option<&DiffCaps>) -> DiffResp {
    use sha2::{Digest, Sha256};
    let mut resp = DiffResp::default();
    let mut budget = Budget::new(caps);
    let mut cur: Option<(FileState, Sha256)> = None;
    // A TYPE CHANGE (symlink ⇄ file) prints two consecutive blocks with the
    // SAME `diff --git` header. `diff_raw(path)` — what a hunk op re-reads and
    // hashes — returns both, so every block of such a group carries the hash
    // of the whole group (the UI merges the group into one file and sends
    // that fingerprint back). A lone block's group hash is its own hash.
    let mut group: Option<(String, Sha256, usize)> = None; // (header, hash, first file index)
    let finish = |resp: &mut DiffResp,
                  budget: &mut Budget,
                  cur: &mut Option<(FileState, Sha256)>,
                  group: &Option<(String, Sha256, usize)>| {
        if let Some((f, h)) = cur.take() {
            budget.push(resp, f, h.finalize().as_slice());
            if let Some((_, g, first)) = group {
                if resp.files.len() - first > 1 {
                    let fp = hex::encode(g.clone().finalize());
                    for f in &mut resp.files[*first..] {
                        f.fingerprint = fp.clone();
                    }
                }
            }
        }
    };
    for raw_line in bytes.split_inclusive(|b| *b == b'\n') {
        let text = String::from_utf8_lossy(raw_line);
        let line = text.trim_end_matches('\n').trim_end_matches('\r');
        // `diff --cc` / `diff --combined` start an UNMERGED path's block
        // (a conflicted file in `git diff` during a merge). Their `@@@` hunks
        // are not unified hunks, so the file is listed without hunks — before,
        // the whole block was skipped and a conflicted tree diffed to 0 files.
        if line.starts_with("diff --git ") || is_combined_diff_start(line) {
            finish(&mut resp, &mut budget, &mut cur, &group);
            if group.as_ref().is_none_or(|(hdr, _, _)| hdr != line) {
                group = Some((line.to_string(), Sha256::new(), resp.files.len()));
            }
            if let Some((_, g, _)) = group.as_mut() {
                g.update(raw_line);
            }
            let mut h = Sha256::new();
            h.update(raw_line);
            cur = Some((FileState::new(line).capped(caps, budget.exhausted), h));
            continue;
        }
        let Some((state, h)) = cur.as_mut() else {
            continue;
        };
        h.update(raw_line);
        if let Some((_, g, _)) = group.as_mut() {
            g.update(raw_line);
        }
        state.feed(line);
    }
    finish(&mut resp, &mut budget, &mut cur, &group);
    fill_totals(&mut resp);
    resp
}

/// The whole-response budget while files are appended in git's order.
struct Budget {
    caps: Option<DiffCaps>,
    lines: usize,
    bytes: usize,
    exhausted: bool,
}

impl Budget {
    fn new(caps: Option<&DiffCaps>) -> Self {
        Self {
            caps: caps.copied(),
            lines: 0,
            bytes: 0,
            exhausted: false,
        }
    }

    fn push(&mut self, resp: &mut DiffResp, f: FileState, digest: &[u8]) {
        let (lines, bytes) = (f.kept_lines, f.kept_bytes);
        let mut file = f.finish();
        file.fingerprint = hex::encode(digest);
        self.admit(&mut file, lines, bytes);
        if self.exhausted {
            resp.truncated = Some(true);
        }
        resp.files.push(file);
    }

    /// Charge one parsed file against the budget.
    fn admit(&mut self, file: &mut FileDiff, lines: usize, bytes: usize) {
        if file.too_large == Some(true) || file.hunks_omitted == Some(true) {
            return;
        }
        match self.judge(lines, bytes, !file.hunks.is_empty()) {
            Verdict::Keep => {}
            Verdict::TooLarge => mark_too_large(file),
            Verdict::Omitted => {
                file.hunks = Vec::new();
                file.hunks_omitted = Some(true);
            }
        }
    }

    /// Per-file cap first (a property of the file), then the response
    /// budget: the first file that doesn't fit spends it for everything after.
    fn judge(&mut self, lines: usize, bytes: usize, has_hunks: bool) -> Verdict {
        let Some(caps) = self.caps else {
            return Verdict::Keep;
        };
        if lines > caps.file_lines || bytes > caps.file_bytes {
            return Verdict::TooLarge;
        }
        if !has_hunks {
            return Verdict::Keep;
        }
        if self.exhausted
            || self.lines + lines > caps.total_lines
            || self.bytes + bytes > caps.total_bytes
        {
            self.exhausted = true;
            return Verdict::Omitted;
        }
        self.lines += lines;
        self.bytes += bytes;
        Verdict::Keep
    }
}

enum Verdict {
    Keep,
    TooLarge,
    Omitted,
}

fn mark_too_large(file: &mut FileDiff) {
    file.hunks = Vec::new();
    file.too_large = Some(true);
    file.hunks_omitted = Some(true);
}

/// Rendered size of a parsed file: (lines, bytes) — the unit [`DiffCaps`]
/// counts in.
fn rendered_size(file: &FileDiff) -> (usize, usize) {
    file.hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .fold((0, 0), |(n, b), l| (n + 1, b + l.content.len() + 1))
}

/// A capped (or summary) COPY of an already-parsed diff, limited to the files
/// `keep` selects — built without cloning the hunks of any file the caps
/// leave out, so serving one file of a cached 30 MB PR diff clones one file.
pub fn capped_view(
    src: &DiffResp,
    keep: impl Fn(&FileDiff) -> bool,
    caps: Option<&DiffCaps>,
    summary: bool,
) -> DiffResp {
    let mut budget = Budget::new(caps);
    let mut out = DiffResp::default();
    for f in src.files.iter().filter(|f| keep(f)) {
        let mut file = FileDiff {
            fingerprint: f.fingerprint.clone(),
            path: f.path.clone(),
            old_path: f.old_path.clone(),
            is_binary: f.is_binary,
            hunks: Vec::new(),
            too_large: f.too_large,
            hunks_omitted: f.hunks_omitted,
            added: f.added,
            deleted: f.deleted,
            status: f.status,
            language: f.language.clone(),
        };
        if file.added.is_none() && !f.is_binary {
            let mut counted = f.clone();
            fill_counts(&mut counted);
            (file.added, file.deleted) = (counted.added, counted.deleted);
        }
        if summary {
            file.hunks_omitted = Some(true);
        } else if f.too_large != Some(true) && f.hunks_omitted != Some(true) {
            let (lines, bytes) = rendered_size(f);
            match budget.judge(lines, bytes, !f.hunks.is_empty()) {
                Verdict::Keep => file.hunks = f.hunks.clone(),
                Verdict::TooLarge => mark_too_large(&mut file),
                Verdict::Omitted => file.hunks_omitted = Some(true),
            }
        }
        out.files.push(file);
    }
    // The source itself may be incomplete (a provider file list cut at its
    // page limit) — that survives any view of it.
    if budget.exhausted || src.truncated == Some(true) {
        out.truncated = Some(true);
    }
    out.renames_incomplete = src.renames_incomplete;
    fill_totals(&mut out);
    out
}

/// `added`/`deleted` from the hunks when a source didn't provide them (GitLab
/// `changes`, GitHub's per-file patches). Binary files stay `None`.
pub fn fill_counts(file: &mut FileDiff) {
    if file.is_binary || (file.added.is_some() && file.deleted.is_some()) {
        return;
    }
    let (mut a, mut d) = (0u32, 0u32);
    for l in file.hunks.iter().flat_map(|h| h.lines.iter()) {
        match l.origin {
            LineOrigin::Add => a += 1,
            LineOrigin::Del => d += 1,
            LineOrigin::Context => {}
        }
    }
    file.added = Some(a);
    file.deleted = Some(d);
}

/// `total_added` / `total_deleted` over the files (binary counts 0).
pub fn fill_totals(resp: &mut DiffResp) {
    let (a, d) = resp.files.iter().fold((0u64, 0u64), |(a, d), f| {
        (
            a + u64::from(f.added.unwrap_or(0)),
            d + u64::from(f.deleted.unwrap_or(0)),
        )
    });
    resp.total_added = Some(a);
    resp.total_deleted = Some(d);
}

/// Parse `git diff|show --raw --numstat -z` — the SUMMARY of a diff, never the
/// patch. git prints every `--raw` record first (`:<modes> <oids> <S>[score]
/// NUL path NUL`, or `… NUL old NUL new NUL` for R/C), then every numstat
/// record (`<a> TAB <d> TAB path NUL`, or `<a> TAB <d> TAB NUL old NUL new NUL`
/// for a rename; `-` counts for binary). Both lists are in the same order;
/// pairing falls back to the path if the lengths ever disagree. With `-z`
/// names are raw bytes — never C-quoted — so spaces, tabs, quotes and
/// non-ASCII survive as-is.
pub fn parse_raw_numstat(out: &[u8]) -> DiffResp {
    struct Raw {
        status: FileChangeStatus,
        old_path: Option<String>,
        path: String,
    }
    let text = String::from_utf8_lossy(out);
    let mut tok = text.split('\0');
    let mut raws: Vec<Raw> = Vec::new();
    let mut stats: Vec<(Option<u32>, Option<u32>, String)> = Vec::new();
    while let Some(t) = tok.next() {
        if t.is_empty() {
            continue;
        }
        if let Some(meta) = t.strip_prefix(':') {
            // `<old mode> <new mode> <old oid> <new oid> <status>[score]`
            let letter = meta
                .rsplit(' ')
                .next()
                .and_then(|s| s.chars().next())
                .unwrap_or('M');
            let status = match letter {
                'A' => FileChangeStatus::Added,
                'D' => FileChangeStatus::Deleted,
                'R' => FileChangeStatus::Renamed,
                'C' => FileChangeStatus::Copied,
                'T' => FileChangeStatus::Typechange,
                _ => FileChangeStatus::Modified,
            };
            let first = tok.next().unwrap_or("").to_string();
            let (old_path, path) = if matches!(letter, 'R' | 'C') {
                (Some(first), tok.next().unwrap_or("").to_string())
            } else {
                (None, first)
            };
            raws.push(Raw {
                status,
                old_path,
                path,
            });
            continue;
        }
        let mut cols = t.splitn(3, '\t');
        let (a, d) = (cols.next().unwrap_or(""), cols.next().unwrap_or(""));
        let rest = cols.next().unwrap_or("");
        let path = if rest.is_empty() {
            // Rename: the names follow as their own records (old, new).
            let _old = tok.next();
            tok.next().unwrap_or("").to_string()
        } else {
            rest.to_string()
        };
        stats.push((a.parse().ok(), d.parse().ok(), path));
    }
    let by_path: Option<std::collections::HashMap<&str, usize>> =
        (raws.len() != stats.len()).then(|| {
            stats
                .iter()
                .enumerate()
                .map(|(i, s)| (s.2.as_str(), i))
                .collect()
        });
    let mut resp = DiffResp::default();
    for (i, r) in raws.iter().enumerate() {
        let idx = match &by_path {
            None => Some(i),
            Some(m) => m.get(r.path.as_str()).copied(),
        };
        let (added, deleted) = idx
            .and_then(|i| stats.get(i))
            .map(|s| (s.0, s.1))
            .unwrap_or((Some(0), Some(0)));
        // numstat prints `-\t-` for a binary file — the only null counts.
        let is_binary = added.is_none() && deleted.is_none();
        resp.files.push(FileDiff {
            fingerprint: String::new(),
            language: lang_from_ext(&r.path),
            path: r.path.clone(),
            old_path: r.old_path.clone(),
            is_binary,
            hunks: Vec::new(),
            too_large: None,
            hunks_omitted: Some(true),
            added,
            deleted,
            status: Some(r.status),
        });
    }
    fill_totals(&mut resp);
    resp
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
    status: FileChangeStatus,
    hunks: Vec<Hunk>,
    old_line: u32,
    new_line: u32,
    in_hunk: bool,
    added_lines: u32,
    deleted_lines: u32,
    /// `@@` headers seen, stored or not.
    hunks_seen: u32,
    /// Lines/bytes actually stored in `hunks` (what [`DiffCaps`] charges).
    kept_lines: usize,
    kept_bytes: usize,
    /// Per-file cap `(lines, bytes)`; `None` = uncapped.
    file_cap: Option<(usize, usize)>,
    /// False once the file went `too_large` or the response budget was
    /// already spent when it started: lines are counted, not stored.
    store: bool,
    too_large: bool,
    budget_omitted: bool,
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
            status: FileChangeStatus::Modified,
            hunks: Vec::new(),
            old_line: 0,
            new_line: 0,
            in_hunk: false,
            added_lines: 0,
            deleted_lines: 0,
            hunks_seen: 0,
            kept_lines: 0,
            kept_bytes: 0,
            file_cap: None,
            store: true,
            too_large: false,
            budget_omitted: false,
        }
    }

    /// Arm the per-file cap; `exhausted` = the response budget is already
    /// spent, so nothing of this file is stored.
    fn capped(mut self, caps: Option<&DiffCaps>, exhausted: bool) -> Self {
        if let Some(c) = caps {
            self.file_cap = Some((c.file_lines, c.file_bytes));
            if exhausted {
                self.store = false;
                self.budget_omitted = true;
            }
        }
        self
    }

    fn feed(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix("@@") {
            if let Some((old_start, new_start)) = parse_hunk_header(rest) {
                self.hunks_seen += 1;
                if self.store {
                    self.hunks.push(Hunk {
                        header: line.to_string(),
                        lines: Vec::new(),
                    });
                }
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
        } else if line.starts_with("new file mode ") {
            self.status = FileChangeStatus::Added;
        } else if line.starts_with("deleted file mode ") {
            self.status = FileChangeStatus::Deleted;
        } else if let Some(v) = line.strip_prefix("rename from ") {
            self.status = FileChangeStatus::Renamed;
            self.rename_from = Some(unquote_c(v));
        } else if let Some(v) = line.strip_prefix("rename to ") {
            self.rename_to = Some(unquote_c(v));
        } else if let Some(v) = line.strip_prefix("copy from ") {
            self.status = FileChangeStatus::Copied;
            self.rename_from = Some(unquote_c(v));
        } else if let Some(v) = line.strip_prefix("copy to ") {
            self.rename_to = Some(unquote_c(v));
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
        if !self.store {
            return;
        }
        if let Some(h) = self.hunks.last_mut() {
            h.lines.push(DiffLine {
                origin,
                content: content.to_string(),
                old_line,
                new_line,
            });
            self.kept_lines += 1;
            self.kept_bytes += content.len() + 1;
        }
        if let Some((max_lines, max_bytes)) = self.file_cap {
            if self.kept_lines > max_lines || self.kept_bytes > max_bytes {
                // Over the per-file cap: drop what was kept and only count
                // from here on — the file comes back `too_large`.
                self.too_large = true;
                self.store = false;
                self.hunks = Vec::new();
            }
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
        let omitted = self.too_large || (self.budget_omitted && self.hunks_seen > 0);
        // A binary file has no line counts (numstat's `-`), not zero.
        let (added, deleted) = if self.is_binary {
            (None, None)
        } else {
            (Some(self.added_lines), Some(self.deleted_lines))
        };
        FileDiff {
            fingerprint: String::new(),
            path,
            old_path,
            is_binary: self.is_binary,
            hunks: if omitted { Vec::new() } else { self.hunks },
            too_large: self.too_large.then_some(true),
            hunks_omitted: omitted.then_some(true),
            added,
            deleted,
            status: Some(self.status),
            language,
        }
    }
}

pub(crate) fn lang_from_ext(path: &str) -> Option<String> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    let lang = match ext {
        "rs" => "rust",
        "go" => "go",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "svelte" => "svelte",
        "vue" => "vue",
        "java" => "java",
        "kt" => "kotlin",
        "scala" => "scala",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "sh" | "bash" | "zsh" => "shell",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "json" => "json",
        "md" => "markdown",
        "sql" => "sql",
        "html" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "xml" => "xml",
        _ => return None,
    };
    Some(lang.to_string())
}

/// "--- a/path" → Some("path"); "--- /dev/null" → None. Quoted paths get the
/// surrounding quotes stripped (escapes left as-is, best effort).
fn strip_ab_prefix(v: &str) -> Option<String> {
    let v = unquote_c(v.trim_end());
    if v == "/dev/null" {
        return None;
    }
    let v = v
        .strip_prefix("a/")
        .or_else(|| v.strip_prefix("b/"))
        .unwrap_or(&v);
    Some(v.to_string())
}

/// Undo git's C-style name quoting. Even with `core.quotePath=false` a name
/// holding a tab, newline, `"` or `\` is printed as `"…"` with escapes in
/// patch headers (`diff --git`, `---`/`+++`, `rename from`) — while `-z`
/// listings (the summary, `status`) print it raw. Unquoted, a per-file diff
/// names the file exactly as the summary did. Unquoted input is returned as-is.
pub(crate) fn unquote_c(v: &str) -> String {
    let Some(inner) = v
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .filter(|_| v.len() >= 2)
    else {
        return v.to_string();
    };
    let b = inner.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 == b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let c = b[i + 1];
        i += 2;
        out.push(match c {
            b't' => b'\t',
            b'n' => b'\n',
            b'r' => b'\r',
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'v' => 0x0b,
            b'0'..=b'3'
                if i + 1 < b.len()
                    && (b'0'..=b'7').contains(&b[i])
                    && (b'0'..=b'7').contains(&b[i + 1]) =>
            {
                let n = (c - b'0') * 64 + (b[i] - b'0') * 8 + (b[i + 1] - b'0');
                i += 2;
                n
            }
            other => other, // `\"`, `\\` and anything unknown
        });
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `diff --cc <path>` / `diff --combined <path>` — a combined (unmerged) block.
fn is_combined_diff_start(line: &str) -> bool {
    line.starts_with("diff --cc ") || line.starts_with("diff --combined ")
}

/// "diff --git a/old b/new" → (Some(old), Some(new)). Best effort: paths with
/// the literal substring " b/" are ambiguous; the ---/+++ lines win anyway.
/// Either side may be C-quoted (`"a/x\ty" "b/x\ty"`).
fn parse_diff_git_paths(line: &str) -> (Option<String>, Option<String>) {
    if let Some(p) = line
        .strip_prefix("diff --cc ")
        .or_else(|| line.strip_prefix("diff --combined "))
    {
        // A combined diff names the one (unmerged) path, unprefixed.
        let p = unquote_c(p.trim());
        return (None, Some(p).filter(|p| !p.is_empty()));
    }
    let rest = match line.strip_prefix("diff --git ") {
        Some(r) => r,
        None => return (None, None),
    };
    let split = rest
        .rfind(" \"b/")
        .map(|i| (i, i + 1))
        .or_else(|| rest.rfind(" b/").map(|i| (i, i + 1)));
    if let Some((end_old, start_new)) = split {
        let old = unquote_c(rest[..end_old].trim());
        let new = unquote_c(rest[start_new..].trim());
        let old = old.strip_prefix("a/").unwrap_or(&old).to_string();
        let new = new.strip_prefix("b/").unwrap_or(&new).to_string();
        return (Some(old), Some(new));
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
    #[test]
    fn status_caps_untracked_rows_only() {
        let mut out = String::from("# branch.head main\0");
        out.push_str("1 .M N... 100644 100644 100644 aaa bbb src/lib.rs\0");
        for i in 0..7 {
            out.push_str(&format!("? new/{i}.txt\0"));
        }
        out.push_str("u UU N... 100644 100644 100644 100644 a b c conflict.rs\0");
        let st = parse_status_capped(&out, 3);
        let untracked = st.changes.iter().filter(|c| c.kind == "untracked").count();
        assert_eq!(untracked, 3);
        assert!(st.changes.iter().any(|c| c.path == "src/lib.rs"));
        assert!(st.changes.iter().any(|c| c.kind == "conflicted"));
        assert!(st.untracked_truncated);
        assert_eq!(st.untracked_total, Some(7));
        // Under the cap: nothing flagged, nothing serialized.
        let st = parse_status_capped(&out, 100);
        assert!(!st.untracked_truncated);
        assert_eq!(st.untracked_total, None);
        let json = serde_json::to_string(&st).unwrap();
        assert!(!json.contains("untracked_t"), "{json}");
    }

    /// A conflicted path during a merge prints a combined `diff --cc`
    /// block; it must be listed (not silently dropped), next to normal files.
    #[test]
    fn combined_diff_blocks_are_listed() {
        let text = "diff --cc conflicted.txt\n\
index 1111111,2222222..0000000\n\
--- a/conflicted.txt\n\
+++ b/conflicted.txt\n\
@@@ -1,1 -1,1 +1,5 @@@\n\
++<<<<<<< HEAD\n\
 +ours\n\
++=======\n\
+ theirs\n\
++>>>>>>> other\n\
diff --git a/ok.txt b/ok.txt\n\
--- a/ok.txt\n\
+++ b/ok.txt\n\
@@ -1 +1 @@\n\
-a\n\
+b\n";
        let d = parse_diff(text);
        let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["conflicted.txt", "ok.txt"], "{d:?}");
        assert!(d.files[0].hunks.is_empty());
        assert_eq!(d.files[1].hunks.len(), 1);
        assert_eq!(
            parse_diff("diff --combined x.rs\n--- a/x.rs\n+++ b/x.rs\n").files[0].path,
            "x.rs"
        );
    }

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

    /// `-z` records: names are raw (never C-quoted) and a rename's origPath is
    /// the NEXT record, not a `\t`-suffix.
    #[test]
    fn status_z_raw_names_and_rename_records() {
        let out = "# branch.oid 1234\0# branch.head main\0\
1 .M N... 100644 100644 100644 aaa bbb app/[id]/page.tsx\0\
2 R. N... 100644 100644 100644 fff ggg R100 app/d/p\u{e2}ge.tsx\0app/d/page.tsx\0\
? caf\u{e9}.txt\0\
? q\"uote.txt\0\
u UU N... 100644 100644 100644 100644 h1 h2 h3 with space.rs\0";
        let st = parse_status(out);
        assert_eq!(st.branch, "main");
        let paths: Vec<&str> = st.changes.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "app/[id]/page.tsx",
                "app/d/p\u{e2}ge.tsx",
                "caf\u{e9}.txt",
                "q\"uote.txt",
                "with space.rs"
            ]
        );
        let r = &st.changes[1];
        assert_eq!(r.kind, "renamed");
        assert_eq!(r.orig_path.as_deref(), Some("app/d/page.tsx"));
        assert_eq!(st.changes[4].kind, "conflicted");
    }

    /// A copy is a NEW file: discard/unstage must not expand to its source.
    #[test]
    fn status_copy_is_added_not_renamed() {
        let out = "2 C. N... 100644 100644 100644 fff ggg C100 copy.txt\0orig.txt\0";
        let st = parse_status(out);
        assert_eq!(st.changes.len(), 1);
        assert_eq!(st.changes[0].path, "copy.txt");
        assert_eq!(st.changes[0].kind, "added");
        assert_eq!(st.changes[0].orig_path.as_deref(), Some("orig.txt"));
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
    fn upstream_tracking_counts() {
        assert_eq!(parse_upstream_tracking("ahead 12, behind 3"), (12, 3));
        assert_eq!(parse_upstream_tracking("ahead 2"), (2, 0));
        assert_eq!(parse_upstream_tracking("behind 7"), (0, 7));
        assert_eq!(parse_upstream_tracking(""), (0, 0));
        assert_eq!(parse_upstream_tracking("gone"), (0, 0));
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

    #[test]
    fn worktree_list_porcelain() {
        let out = "\
worktree /Users/me/repo
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /Users/me/wt/feature-x
HEAD 2222222222222222222222222222222222222222
branch refs/heads/feature/x
locked agent run in progress

worktree /Users/me/wt/detached
HEAD 3333333333333333333333333333333333333333
detached
prunable gitdir file points to non-existent location
";
        let wts = parse_worktree_list(out);
        assert_eq!(wts.len(), 3);
        assert!(wts[0].is_main);
        assert_eq!(wts[0].path, "/Users/me/repo");
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
        assert!(!wts[0].locked && !wts[0].prunable);
        assert!(!wts[1].is_main);
        assert_eq!(wts[1].branch.as_deref(), Some("feature/x"));
        assert!(wts[1].locked);
        assert_eq!(wts[1].lock_reason.as_deref(), Some("agent run in progress"));
        assert!(wts[2].branch.is_none());
        assert!(wts[2].prunable);
        // no trailing blank line required
        assert_eq!(
            parse_worktree_list("worktree /a\nHEAD abc\ndetached").len(),
            1
        );
        assert!(parse_worktree_list("").is_empty());
    }

    #[test]
    fn submodule_status_lines() {
        let out = "\
 4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b vendor/libfoo (v1.2.0-3-g4a5b6c7)
-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa modules/not-inited
+bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb tools/drifted (heads/main)
Ucccccccccccccccccccccccccccccccccccccccc conflicted/mod
";
        let subs = parse_submodule_status(out);
        assert_eq!(subs.len(), 4);
        assert_eq!(subs[0].state, "ok");
        assert_eq!(subs[0].path, "vendor/libfoo");
        assert_eq!(subs[0].describe.as_deref(), Some("v1.2.0-3-g4a5b6c7"));
        assert_eq!(subs[1].state, "uninitialized");
        assert!(subs[1].describe.is_none());
        assert_eq!(subs[2].state, "modified");
        assert_eq!(subs[3].state, "conflict");
        assert!(parse_submodule_status("").is_empty());
    }

    #[test]
    fn submodules_enriched_from_gitmodules() {
        let mut subs = parse_submodule_status(
            " 4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b vendor/libfoo (v1.2.0)\n",
        );
        let cfg = "\
submodule.libfoo.path=vendor/libfoo
submodule.libfoo.url=https://github.com/acme/libfoo.git
submodule.libfoo.branch=main
submodule.other.path=elsewhere
submodule.other.url=https://example.com/other.git
";
        enrich_submodules(&mut subs, cfg);
        assert_eq!(
            subs[0].url.as_deref(),
            Some("https://github.com/acme/libfoo.git")
        );
        assert_eq!(subs[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn raw_numstat_pairs_records_with_nul_safe_names() {
        // Exactly what `git show --raw --numstat -z -M` printed for a commit
        // with a binary edit, a delete, a type change, a rename with a space
        // in both names, an add and a non-ASCII name.
        let out = b":100644 100644 bdc955b 8835708 M\0bin.dat\0\
:100644 000000 2fa992c 0000000 D\0keep.txt\0\
:120000 100644 1de5659 3bcf9fd T\0link\0\
:100644 100644 f9d9a01 5c2dbfa R085\0sp ace.txt\0moved sp.txt\0\
:000000 100644 0000000 3e75765 A\0new.txt\0\
:100644 100644 587be6b b77b4eb M\0\xc3\xbcn\xc3\xaf.txt\0\
-\t-\tbin.dat\0\
0\t1\tkeep.txt\0\
1\t1\tlink\0\
1\t1\t\0sp ace.txt\0moved sp.txt\0\
1\t0\tnew.txt\0\
1\t0\t\xc3\xbcn\xc3\xaf.txt\0";
        let d = parse_raw_numstat(out);
        let got: Vec<_> = d
            .files
            .iter()
            .map(|f| {
                (
                    f.path.as_str(),
                    f.old_path.as_deref(),
                    f.status.unwrap(),
                    f.added,
                    f.deleted,
                )
            })
            .collect();
        use FileChangeStatus::*;
        assert_eq!(
            got,
            vec![
                ("bin.dat", None, Modified, None, None),
                ("keep.txt", None, Deleted, Some(0), Some(1)),
                ("link", None, Typechange, Some(1), Some(1)),
                (
                    "moved sp.txt",
                    Some("sp ace.txt"),
                    Renamed,
                    Some(1),
                    Some(1)
                ),
                ("new.txt", None, Added, Some(1), Some(0)),
                ("\u{fc}n\u{ef}.txt", None, Modified, Some(1), Some(0)),
            ]
        );
        assert!(d.files[0].is_binary);
        assert!(d.files.iter().all(|f| f.hunks_omitted == Some(true)));
        assert_eq!((d.total_added, d.total_deleted), (Some(4), Some(3)));
        assert!(parse_raw_numstat(b"").files.is_empty());
    }

    #[test]
    fn quoted_header_names_are_unquoted() {
        assert_eq!(
            unquote_c(r#""a\tb \"q\" \\ \303\274""#),
            "a\tb \"q\" \\ \u{fc}"
        );
        assert_eq!(unquote_c("plain name"), "plain name");
        let text = "\
diff --git \"a/old\\tname.txt\" \"b/new\\tname.txt\"
similarity index 90%
rename from \"old\\tname.txt\"
rename to \"new\\tname.txt\"
--- \"a/old\\tname.txt\"
+++ \"b/new\\tname.txt\"
@@ -1 +1,2 @@
 x
+y
diff --git \"a/q\\\"uote\" \"b/q\\\"uote\"
new file mode 100644
--- /dev/null
+++ \"b/q\\\"uote\"
@@ -0,0 +1 @@
+z
";
        let d = parse_diff(text);
        assert_eq!(d.files[0].path, "new\tname.txt");
        assert_eq!(d.files[0].old_path.as_deref(), Some("old\tname.txt"));
        assert_eq!(d.files[0].status, Some(FileChangeStatus::Renamed));
        assert_eq!(d.files[1].path, "q\"uote");
        assert_eq!(d.files[1].old_path, None);
        assert_eq!(d.files[1].status, Some(FileChangeStatus::Added));
    }

    #[test]
    fn full_parse_status_counts_and_binary_nulls() {
        let text = "\
diff --git a/img.png b/img.png
index 333..444 100644
Binary files a/img.png and b/img.png differ
diff --git a/dead.rs b/dead.rs
deleted file mode 100644
--- a/dead.rs
+++ /dev/null
@@ -1,1 +0,0 @@
-bye
diff --git a/src.rs b/copy.rs
similarity index 100%
copy from src.rs
copy to copy.rs
";
        let d = parse_diff(text);
        assert_eq!((d.files[0].added, d.files[0].deleted), (None, None));
        assert_eq!(d.files[1].status, Some(FileChangeStatus::Deleted));
        assert_eq!(d.files[2].status, Some(FileChangeStatus::Copied));
        assert_eq!(d.files[2].old_path.as_deref(), Some("src.rs"));
        assert_eq!(d.files[2].path, "copy.rs");
        assert_eq!((d.total_added, d.total_deleted), (Some(0), Some(1)));
    }

    #[test]
    fn per_file_cap_counts_on_and_budget_marks_the_rest() {
        let mut text = String::new();
        let file = |name: &str, n: usize, text: &mut String| {
            text.push_str(&format!(
                "diff --git a/{name} b/{name}\nnew file mode 100644\n--- /dev/null\n+++ b/{name}\n@@ -0,0 +1,{n} @@\n"
            ));
            for i in 0..n {
                text.push_str(&format!("+{i}\n"));
            }
        };
        file("a", 12, &mut text);
        file("b", 30, &mut text);
        file("c", 8, &mut text);
        file("d", 1, &mut text);
        let caps = DiffCaps {
            file_lines: 20,
            file_bytes: 1 << 20,
            total_lines: 25,
            total_bytes: 1 << 20,
        };
        let d = parse_diff_bytes_capped(text.as_bytes(), Some(&caps));
        let full = parse_diff(&text);
        // a fits; b is over the per-file cap (not charged); c (12+8=20) fits;
        // d fits too (21 ≤ 25).
        assert_eq!(d.files[0].hunks[0].lines.len(), 12);
        assert_eq!(d.files[1].too_large, Some(true));
        assert_eq!(d.files[1].added, Some(30));
        assert_eq!(d.files[2].hunks[0].lines.len(), 8);
        assert_eq!(d.files[3].hunks[0].lines.len(), 1);
        assert_eq!(d.truncated, None);
        for (c, f) in d.files.iter().zip(&full.files) {
            assert_eq!(c.fingerprint, f.fingerprint);
        }
        // A tighter budget: c no longer fits, and d after it is omitted too.
        let tight = DiffCaps {
            total_lines: 15,
            ..caps
        };
        let d = parse_diff_bytes_capped(text.as_bytes(), Some(&tight));
        assert_eq!(d.files[0].hunks_omitted, None);
        assert_eq!(d.files[2].hunks_omitted, Some(true));
        assert_eq!(d.files[3].hunks_omitted, Some(true));
        assert!(d.files[3].hunks.is_empty());
        assert_eq!(d.truncated, Some(true));
        assert_eq!(d.total_added, Some(51));
        // Summary of a parsed diff: no hunks anywhere, counts kept.
        let s = capped_view(&full, |_| true, None, true);
        assert!(s
            .files
            .iter()
            .all(|f| f.hunks.is_empty() && f.hunks_omitted == Some(true)));
        assert_eq!(s.total_added, Some(51));
    }
}
