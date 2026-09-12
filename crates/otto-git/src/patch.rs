//! Hunk / line staging, unstaging and discard (git batch R3) — routes are
//! merged into `crate::http::router`.
//!
//! The patch handed to `git apply` is rebuilt from the RAW unified diff the
//! daemon produces ITSELF at apply time, never from the structured `DiffResp`
//! the UI renders and never from client-supplied patch text. Two reasons:
//!
//! * `parse_diff` splits on `str::lines()` (which eats `\r`) and drops the
//!   `\ No newline at end of file` markers, so a patch rebuilt from a
//!   `DiffResp` stages a CRLF file with LF endings and invents a trailing
//!   newline — silently, because such a patch is syntactically valid and
//!   `git apply` accepts it. Everything here works on `split_inclusive('\n')`
//!   slices of the real diff text, so both survive verbatim.
//! * Because the pre-image comes from a fresh diff, `git apply` succeeds by
//!   construction — which means a client's stale `hunk_index` would silently
//!   act on a DIFFERENT hunk. The rendered hunk's `@@` header travels with the
//!   request and must match, or the call is a 409 with nothing applied.

use std::collections::HashSet;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Extension, Json, Router};
use otto_core::api::{DiffResp, RepoStatusResp};
use otto_core::auth::AuthUser;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};

use crate::http::{repo_ctx, repo_lock, ApiResult, GitCtx};
use crate::local::{upstream_err, DiffTarget, LocalGit};

/// Message of the stash `discard` leaves behind. Listed (not just created), so
/// `GET /repos/{id}/stashes` shows it and the user can pop it back.
const DISCARD_BACKUP_MSG: &str = "otto: backup before hunk discard";

/// The 409 text for a request built against a diff that no longer describes the
/// file. The only way it is ever reached (see the module doc).
const STALE: &str = "the file changed since the diff was shown — refresh and retry";

// ---------------------------------------------------------------------------
// Pure patch builder
// ---------------------------------------------------------------------------

/// Rebuild a one-hunk patch for `path` out of `raw_diff` (the output of
/// `git diff [--cached] --no-color -U3 -M -- <path>`).
///
/// `hunk_header` is the `@@ … @@` line the CLIENT was shown; it must equal the
/// located hunk's own header or the request is stale (409). `lines` selects a
/// SUBSET of the hunk body by index — indices count body lines only, skipping
/// `\ No newline at end of file` markers, which is exactly how `parse_diff`
/// numbers `Hunk.lines` (pinned by `raw_and_parsed_hunk_indices_agree`). `None`
/// means the whole hunk.
///
/// Selection semantics: context is kept; a selected `+`/`-` is kept verbatim;
/// an unselected `+` is dropped (it exists on neither side of the patch); an
/// unselected `-` becomes CONTEXT (it must survive the apply, so it has to be
/// present in both images).
pub fn build_hunk_patch(
    raw_diff: &str,
    path: &str,
    hunk_idx: usize,
    hunk_header: &str,
    lines: Option<&[usize]>,
) -> Result<String> {
    // `split_inclusive` keeps every line's own terminator: CRLF endings and a
    // missing final newline round-trip byte for byte. `lines()` normalises both.
    let all: Vec<&str> = raw_diff.split_inclusive('\n').collect();
    let starts: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("diff --git "))
        .map(|(i, _)| i)
        .collect();
    let (from, to) = starts
        .iter()
        .enumerate()
        .map(|(n, &s)| (s, starts.get(n + 1).copied().unwrap_or(all.len())))
        .find(|&(s, _)| diff_git_targets(all[s], path))
        .ok_or_else(|| Error::NotFound(format!("no diff for {path}")))?;
    let block = &all[from..to];

    // A rename carries no hunk for the bytes that moved, and a binary block has
    // no text hunks at all — neither can be applied a hunk at a time. The UI
    // turns this into "stage the whole file".
    if block.iter().any(|l| {
        l.starts_with("rename from")
            || l.starts_with("Binary files")
            || l.starts_with("GIT binary patch")
    }) {
        return Err(Error::Invalid("stage the whole file".into()));
    }

    // Header = everything before the first `@@` (`diff --git`, `index`,
    // `old/new mode`, `new file mode`, `deleted file mode`, `---`, `+++`) —
    // emitted verbatim. No body line can start with `@`: git prefixes them with
    // ' ', '+', '-' or '\'.
    let head_end = block
        .iter()
        .position(|l| l.starts_with("@@"))
        .unwrap_or(block.len());
    let hstarts: Vec<usize> = block
        .iter()
        .enumerate()
        .skip(head_end)
        .filter(|(_, l)| l.starts_with("@@"))
        .map(|(i, _)| i)
        .collect();
    let hunks: Vec<&[&str]> = hstarts
        .iter()
        .enumerate()
        .map(|(n, &s)| &block[s..hstarts.get(n + 1).copied().unwrap_or(block.len())])
        .collect();

    let hunk = match hunks.get(hunk_idx) {
        Some(h) if h[0].trim() == hunk_header.trim() => *h,
        _ => return Err(Error::Conflict(STALE.into())),
    };

    // Pair each body line with the `\ No newline at end of file` marker that
    // follows it. Markers are not body lines: they neither consume an index nor
    // count towards the hunk's line counts.
    let mut body: Vec<(&str, Option<&str>)> = Vec::new();
    for l in &hunk[1..] {
        if l.starts_with('\\') {
            if let Some(last) = body.last_mut() {
                last.1 = Some(l);
            }
            continue;
        }
        body.push((l, None));
    }

    let selected: Option<HashSet<usize>> = match lines {
        None => None,
        Some(idx) => {
            if idx.len() > body.len() || idx.iter().any(|&i| i >= body.len()) {
                return Err(Error::Invalid("lines out of range".into()));
            }
            Some(idx.iter().copied().collect())
        }
    };
    let is_sel = |i: usize| selected.as_ref().is_none_or(|s| s.contains(&i));

    // A `\ No newline` marker is only meaningful on the LAST line of an image.
    // A partial selection can move a marked line off the end (an unselected
    // marked `-` becomes context and a kept `+` follows it); `git apply` then
    // strips that line's newline and glues the next line onto it — silently.
    if selected.is_some() {
        let (mut old_marked, mut new_marked) = (false, false);
        for (i, (line, marker)) in body.iter().enumerate() {
            let (in_old, in_new) = match line.chars().next().unwrap_or(' ') {
                '+' => (false, is_sel(i)),
                '-' => (true, !is_sel(i)),
                _ => (true, true),
            };
            if (in_old && old_marked) || (in_new && new_marked) {
                return Err(Error::Invalid(
                    "this selection splits the file's last line — stage the whole hunk".into(),
                ));
            }
            if in_old {
                old_marked = marker.is_some();
            }
            if in_new {
                new_marked = marker.is_some();
            }
        }
    }

    let mut out = String::with_capacity(raw_diff.len());
    for l in &block[..head_end] {
        out.push_str(l);
    }

    // Counts are recomputed for the SELECTION: `old` = context + every `-`
    // (kept or converted), `new` = context + kept `+` + converted `-`.
    let (mut old_count, mut new_count) = (0u32, 0u32);
    let mut body_out = String::new();
    for (i, (line, marker)) in body.iter().enumerate() {
        match line.chars().next().unwrap_or(' ') {
            '+' => {
                // An unselected `+` exists on neither side — it goes, and its
                // marker goes with it.
                if is_sel(i) {
                    body_out.push_str(line);
                    push_marker(&mut body_out, *marker);
                    new_count += 1;
                }
            }
            '-' => {
                if is_sel(i) {
                    body_out.push_str(line);
                    push_marker(&mut body_out, *marker);
                    old_count += 1;
                } else {
                    body_out.push(' ');
                    body_out.push_str(&line[1..]);
                    push_marker(&mut body_out, *marker);
                    old_count += 1;
                    new_count += 1;
                }
            }
            // Context (' '), and defensively anything else, is carried verbatim
            // on both sides.
            _ => {
                body_out.push_str(line);
                push_marker(&mut body_out, *marker);
                old_count += 1;
                new_count += 1;
            }
        }
    }

    // Starts come from the located hunk (its pre-image is what is on disk right
    // now); only the counts change. `git apply --recount` is passed anyway, so
    // the numbers are belt-and-braces — but a readable patch matters in a log.
    // The text after the closing `@@` (git's function-context hint) and the
    // line terminator are preserved verbatim.
    let (old_start, new_start, trailer) = split_hunk_header(hunk[0])?;
    out.push_str(&format!(
        "@@ -{old_start},{old_count} +{new_start},{new_count} @@{trailer}"
    ));
    out.push_str(&body_out);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn push_marker(out: &mut String, marker: Option<&str>) {
    if let Some(m) = marker {
        out.push_str(m);
    }
}

/// True when a `diff --git a/<old> b/<new>` line names `path` on its `b/` side.
/// Diffs here are rendered with `core.quotePath=false`, so names are raw UTF-8;
/// a quoted name is also compared verbatim so a caller echoing what it was
/// shown still matches.
fn diff_git_targets(line: &str, path: &str) -> bool {
    let Some(rest) = line
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .strip_prefix("diff --git ")
    else {
        return false;
    };
    // An exact ` b/<path>` SUFFIX first: `rfind(" b/")` alone cuts at the LAST
    // occurrence, so a name that itself contains " b/" (`a/x b/y b/x b/y`)
    // yields `y` and the block is never found (a 404 "no diff for …").
    if rest.trim_end().ends_with(&format!(" b/{path}")) {
        return true;
    }
    // `rfind`, not `find`: a path may itself contain " b/".
    if let Some(idx) = rest.rfind(" b/") {
        if rest[idx + 3..].trim() == path {
            return true;
        }
    }
    // git still quotes a name carrying a `"` or a backslash even with
    // `core.quotePath=false`; compare what is inside the quotes verbatim.
    if let Some(idx) = rest.rfind(" \"b/") {
        if rest[idx + 4..].trim().trim_end_matches('"') == path {
            return true;
        }
    }
    false
}

/// `@@ -12,7 +12,9 @@ fn foo() {\n` → `(12, 12, " fn foo() {\n")`.
fn split_hunk_header(line: &str) -> Result<(u32, u32, &str)> {
    let bad = || Error::Invalid("unparsable hunk header".to_string());
    let rest = line.strip_prefix("@@").ok_or_else(bad)?;
    let close = rest.find("@@").ok_or_else(bad)?;
    let (mut old_start, mut new_start) = (None, None);
    for tok in rest[..close].split_whitespace() {
        if let Some(v) = tok.strip_prefix('-') {
            old_start = v.split(',').next().and_then(|n| n.parse::<u32>().ok());
        } else if let Some(v) = tok.strip_prefix('+') {
            new_start = v.split(',').next().and_then(|n| n.parse::<u32>().ok());
        }
    }
    match (old_start, new_start) {
        (Some(a), Some(c)) => Ok((a, c, &rest[close + 2..])),
        _ => Err(bad()),
    }
}

// ---------------------------------------------------------------------------
// LocalGit: raw diff, apply, backup stash
// ---------------------------------------------------------------------------

impl LocalGit {
    /// The raw unified diff text for ONE path, before `parse_diff` touches it.
    /// Only `Worktree` and `Staged` are meaningful: `Working` synthesises
    /// `--no-index` diffs for untracked files (which `git apply` cannot stage
    /// and `git stash create` cannot back up), and commit/range targets have no
    /// index or worktree to apply to.
    pub async fn diff_raw(&self, target: DiffTarget, path: &str) -> Result<String> {
        Self::guard_path(path)?;
        let cached = match target {
            DiffTarget::Worktree => false,
            DiffTarget::Staged => true,
            _ => {
                return Err(Error::Invalid(
                    "hunk operations work on the worktree or the index only".into(),
                ))
            }
        };
        let mut args: Vec<&str> = vec![
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-color",
            "-U3",
            "-M",
        ];
        if cached {
            args.push("--cached");
        }
        args.extend_from_slice(&["--", path]);
        self.run_read(&args).await
    }

    /// `git apply [--cached] [--reverse] --recount --whitespace=nowarn -`, with
    /// the patch on stdin (no positional arguments, so nothing the caller sent
    /// can reach argv). `-U3` context is retained deliberately — no
    /// `--unidiff-zero` — so `apply` verifies placement instead of trusting the
    /// line numbers.
    pub async fn apply_patch(&self, patch: &str, cached: bool, reverse: bool) -> Result<()> {
        let mut args: Vec<&str> = vec!["apply"];
        if cached {
            args.push("--cached");
        }
        if reverse {
            args.push("--reverse");
        }
        args.extend_from_slice(&["--recount", "--whitespace=nowarn", "-"]);

        for attempt in 1u64..=3 {
            // Bounded `LocalWrite` spawn: detached from the request so a client
            // abort never SIGKILLs a half-written index.
            let (ok, stdout, stderr, code) = self.run_raw_stdin(&args, patch.as_bytes()).await?;
            if ok {
                return Ok(());
            }
            // A concurrent git (an agent session in the same repo) holding the
            // index lock is transient — the same bounded retry every other
            // index-writing command here uses.
            if stderr.contains("index.lock") && attempt < 3 {
                tokio::time::sleep(Duration::from_millis(300 * attempt)).await;
                continue;
            }
            let lc = stderr.to_ascii_lowercase();
            if lc.contains("does not apply") || lc.contains("patch failed") {
                let line = stderr
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("git apply refused the patch");
                return Err(Error::Conflict(line.to_string()));
            }
            return Err(upstream_err(&stderr, &stdout, code));
        }
        unreachable!("loop returns on its final attempt")
    }

    /// Snapshot the working tree into a stash commit and LIST it, so a discard
    /// is recoverable from `GET /repos/{id}/stashes`. `Ok(None)` when there was
    /// nothing to snapshot (clean tree). `stash create` records tracked changes
    /// only — which is exactly the scope `discard` can touch, since its diff
    /// target is `worktree`.
    pub async fn stash_backup(&self, message: &str) -> Result<Option<String>> {
        let sha = self.run(&["stash", "create"]).await?.trim().to_string();
        if sha.is_empty() {
            return Ok(None);
        }
        self.run(&["stash", "store", "-m", message, &sha]).await?;
        Ok(Some(sha))
    }
}

// ---------------------------------------------------------------------------
// Route
// ---------------------------------------------------------------------------

/// What to do with the addressed hunk (or line selection inside it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HunkOp {
    /// Worktree → index.
    Stage,
    /// Index → worktree (reverse-apply the staged diff).
    Unstage,
    /// Drop the change from the worktree, keeping a backup stash.
    Discard,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StageHunkReq {
    pub path: String,
    pub hunk_index: usize,
    /// The `@@ … @@` line the client rendered — the staleness guard.
    pub hunk_header: String,
    /// Optional line selection inside the hunk (indices into `Hunk.lines`).
    #[serde(default)]
    pub lines: Option<Vec<usize>>,
    pub op: HunkOp,
    /// Required for `discard` — it rewrites the working file.
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Serialize)]
pub struct StageHunkResp {
    pub status: RepoStatusResp,
    /// The same target the operation read, re-rendered so the UI can redraw
    /// without a follow-up round trip.
    pub diff: DiffResp,
    /// `discard` only: the stash the backup was stored under.
    pub backup_stash: Option<String>,
}

pub fn router<S: GitCtx>() -> Router<S> {
    Router::new().route("/repos/{id}/stage-hunk", post(repo_stage_hunk::<S>))
}

/// Request-shape validation, split out so it is testable without a `GitCtx`.
pub(crate) fn validate(req: &StageHunkReq) -> Result<()> {
    LocalGit::guard_path(&req.path)?;
    if req.op == HunkOp::Discard && !req.confirm {
        return Err(Error::Invalid("discard requires confirm:true".into()));
    }
    Ok(())
}

/// The whole operation minus auth/locking — the unit the tests drive.
pub(crate) async fn run_hunk_op(git: &LocalGit, req: &StageHunkReq) -> Result<StageHunkResp> {
    validate(req)?;
    // The target is derived from `op`, never taken from the client: `working`
    // (which carries untracked files) must never reach `apply`/`stash create`.
    let target = match req.op {
        HunkOp::Unstage => DiffTarget::Staged,
        HunkOp::Stage | HunkOp::Discard => DiffTarget::Worktree,
    };
    let raw = git.diff_raw(target.clone(), &req.path).await?;
    let patch = build_hunk_patch(
        &raw,
        &req.path,
        req.hunk_index,
        &req.hunk_header,
        req.lines.as_deref(),
    )?;
    let backup_stash = match req.op {
        HunkOp::Stage => {
            git.apply_patch(&patch, true, false).await?;
            None
        }
        HunkOp::Unstage => {
            git.apply_patch(&patch, true, true).await?;
            None
        }
        HunkOp::Discard => {
            // Backed up BEFORE the rewrite, and only once the patch is known to
            // be buildable — a stale request must not leave a stray stash.
            let backup = git.stash_backup(DISCARD_BACKUP_MSG).await?;
            git.apply_patch(&patch, false, true).await?;
            backup
        }
    };
    Ok(StageHunkResp {
        status: git.status().await?,
        diff: git.diff(target, Some(&req.path)).await?,
        backup_stash,
    })
}

async fn repo_stage_hunk<S: GitCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<StageHunkReq>,
) -> ApiResult<Json<StageHunkResp>> {
    let lock = repo_lock(&id);
    let _g = lock.lock().await;
    let (_, git) = repo_ctx(&s, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(run_hunk_op(&git, &req).await?))
}

// ---------------------------------------------------------------------------
// Tests — pure builder on raw diff text, then real throwaway repos
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::api::LineOrigin;
    use std::path::{Path as FsPath, PathBuf};

    /// Two well-separated hunks in one file.
    const TWO_HUNKS: &str = "\
diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,5 +1,6 @@
 l1
-l2
+L2
+L2b
 l3
 l4
 l5
@@ -12,5 +13,5 @@ fn tail()
 l12
 l13
-l14
+L14
 l15
 l16
";

    const HUNK1_HEADER: &str = "@@ -1,5 +1,6 @@";

    const TWO_DELS: &str = "\
diff --git a/d.txt b/d.txt
index aaaaaaa..bbbbbbb 100644
--- a/d.txt
+++ b/d.txt
@@ -1,5 +1,3 @@
 k1
-k2
-k3
 k4
 k5
";

    const NEW_FILE: &str = "\
diff --git a/n.txt b/n.txt
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/n.txt
@@ -0,0 +1,3 @@
+n1
+n2
+n3
";

    const DELETED_FILE: &str = "\
diff --git a/x.txt b/x.txt
deleted file mode 100644
index 1234567..0000000
--- a/x.txt
+++ /dev/null
@@ -1,3 +0,0 @@
-x1
-x2
-x3
";

    const RENAMED: &str = "\
diff --git a/old.txt b/new.txt
similarity index 90%
rename from old.txt
rename to new.txt
index 1111111..2222222 100644
--- a/old.txt
+++ b/new.txt
@@ -1,3 +1,3 @@
 r1
-r2
+R2
 r3
";

    const BINARY: &str = "\
diff --git a/img.png b/img.png
index 1111111..2222222 100644
Binary files a/img.png and b/img.png differ
";

    /// `\\ No newline at end of file` attached to a `-` line.
    const MARKED_DEL: &str = "\
diff --git a/m.txt b/m.txt
index 1111111..2222222 100644
--- a/m.txt
+++ b/m.txt
@@ -1,3 +1,3 @@
 m1
 m2
-m3
\\ No newline at end of file
+M3
";

    /// `\\ No newline at end of file` attached to the LAST `+` line.
    const MARKED_ADD: &str = "\
diff --git a/p.txt b/p.txt
index 1111111..2222222 100644
--- a/p.txt
+++ b/p.txt
@@ -1,2 +1,4 @@
 p1
 p2
+p3
+p4
\\ No newline at end of file
";

    /// Multi-hunk WITH markers — the fixture that pins raw-body indices against
    /// `parse_diff`'s `Hunk.lines` indices.
    const MULTI_MARKED: &str = "\
diff --git a/z.txt b/z.txt
index 1111111..2222222 100644
--- a/z.txt
+++ b/z.txt
@@ -1,5 +1,6 @@
 z1
-z2
+Z2
+Z2b
 z3
 z4
 z5
@@ -12,3 +13,3 @@
 z12
 z13
-z14
\\ No newline at end of file
+Z14
\\ No newline at end of file
";

    // -- pure builder --------------------------------------------------------

    #[test]
    fn whole_hunk_round_trips() {
        let p = build_hunk_patch(TWO_HUNKS, "a.txt", 0, HUNK1_HEADER, None).unwrap();
        assert_eq!(
            p,
            "\
diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,5 +1,6 @@
 l1
-l2
+L2
+L2b
 l3
 l4
 l5
"
        );
        // …and the SECOND hunk keeps its function-context trailer verbatim.
        let p2 =
            build_hunk_patch(TWO_HUNKS, "a.txt", 1, "@@ -12,5 +13,5 @@ fn tail()", None).unwrap();
        assert!(p2.contains("@@ -12,5 +13,5 @@ fn tail()\n"), "{p2}");
        assert!(p2.contains("+L14\n") && !p2.contains("+L2\n"), "{p2}");
    }

    #[test]
    fn subset_of_adds_keeps_context() {
        // Body indices: 0 ' l1', 1 '-l2', 2 '+L2', 3 '+L2b', 4..6 context.
        let p = build_hunk_patch(TWO_HUNKS, "a.txt", 0, HUNK1_HEADER, Some(&[2])).unwrap();
        assert!(
            p.ends_with(
                "\
@@ -1,5 +1,6 @@
 l1
 l2
+L2
 l3
 l4
 l5
"
            ),
            "{p}"
        );
    }

    #[test]
    fn subset_of_dels_converts_rest_to_context() {
        // Keep only '-k2'; '-k3' must survive the apply, so it becomes context.
        let p = build_hunk_patch(TWO_DELS, "d.txt", 0, "@@ -1,5 +1,3 @@", Some(&[1])).unwrap();
        assert!(
            p.ends_with(
                "\
@@ -1,5 +1,4 @@
 k1
-k2
 k3
 k4
 k5
"
            ),
            "{p}"
        );
    }

    #[test]
    fn mixed_selection() {
        // Keep the deletion and the SECOND addition; drop the first addition.
        let p = build_hunk_patch(TWO_HUNKS, "a.txt", 0, HUNK1_HEADER, Some(&[1, 3])).unwrap();
        assert!(
            p.ends_with(
                "\
@@ -1,5 +1,5 @@
 l1
-l2
+L2b
 l3
 l4
 l5
"
            ),
            "{p}"
        );
    }

    #[test]
    fn new_file_block() {
        let p = build_hunk_patch(NEW_FILE, "n.txt", 0, "@@ -0,0 +1,3 @@", Some(&[0, 1])).unwrap();
        assert!(
            p.starts_with("diff --git a/n.txt b/n.txt\nnew file mode 100644\n"),
            "{p}"
        );
        assert!(p.ends_with("@@ -0,0 +1,2 @@\n+n1\n+n2\n"), "{p}");
    }

    #[test]
    fn deleted_file_block() {
        let p = build_hunk_patch(DELETED_FILE, "x.txt", 0, "@@ -1,3 +0,0 @@", None).unwrap();
        assert_eq!(p, DELETED_FILE);
    }

    #[test]
    fn renamed_block_is_invalid() {
        let e = build_hunk_patch(RENAMED, "new.txt", 0, "@@ -1,3 +1,3 @@", None).unwrap_err();
        assert!(
            matches!(&e, Error::Invalid(m) if m == "stage the whole file"),
            "{e:?}"
        );
    }

    #[test]
    fn binary_block_is_invalid() {
        let e = build_hunk_patch(BINARY, "img.png", 0, "@@ -1 +1 @@", None).unwrap_err();
        assert!(
            matches!(&e, Error::Invalid(m) if m == "stage the whole file"),
            "{e:?}"
        );
    }

    #[test]
    fn marked_del_converted_to_context_before_a_kept_add_is_invalid() {
        // Select only '+M3': '-m3' would become context and KEEP its marker,
        // with '+M3' emitted after it. `git apply` reads that marker as "strip
        // the newline of the preceding line" and glues the two together
        // ("m3M3") without a word of complaint — so the selection is refused.
        let e =
            build_hunk_patch(MARKED_DEL, "m.txt", 0, "@@ -1,3 +1,3 @@", Some(&[3])).unwrap_err();
        assert!(
            matches!(&e, Error::Invalid(m) if m.contains("splits the file's last line")),
            "{e:?}"
        );
    }

    #[test]
    fn marker_dropped_with_dropped_add() {
        // Select only '+p3': the unselected '+p4' AND its marker disappear.
        let p = build_hunk_patch(MARKED_ADD, "p.txt", 0, "@@ -1,2 +1,4 @@", Some(&[2])).unwrap();
        assert!(p.ends_with("@@ -1,2 +1,3 @@\n p1\n p2\n+p3\n"), "{p}");
        assert!(!p.contains("No newline"), "{p}");
    }

    #[test]
    fn whole_hunk_with_markers_still_ok() {
        // No selection = no line can move off the end of either image, so the
        // guard never fires and a marked hunk round-trips byte for byte.
        let p = build_hunk_patch(MARKED_DEL, "m.txt", 0, "@@ -1,3 +1,3 @@", None).unwrap();
        assert_eq!(p, MARKED_DEL);
        let p = build_hunk_patch(MARKED_ADD, "p.txt", 0, "@@ -1,2 +1,4 @@", None).unwrap();
        assert_eq!(p, MARKED_ADD);
    }

    #[test]
    fn stale_header_is_conflict() {
        let e = build_hunk_patch(TWO_HUNKS, "a.txt", 0, "@@ -1,9 +1,9 @@", None).unwrap_err();
        assert!(matches!(&e, Error::Conflict(m) if m == STALE), "{e:?}");
    }

    #[test]
    fn index_out_of_range_is_conflict() {
        let e = build_hunk_patch(TWO_HUNKS, "a.txt", 7, HUNK1_HEADER, None).unwrap_err();
        assert!(matches!(&e, Error::Conflict(m) if m == STALE), "{e:?}");
    }

    #[test]
    fn lines_out_of_range_is_invalid() {
        // Past the end…
        let e = build_hunk_patch(TWO_HUNKS, "a.txt", 0, HUNK1_HEADER, Some(&[99])).unwrap_err();
        assert!(
            matches!(&e, Error::Invalid(m) if m == "lines out of range"),
            "{e:?}"
        );
        // …and more indices than the hunk has body lines.
        let many: Vec<usize> = (0..20).collect();
        let e = build_hunk_patch(TWO_HUNKS, "a.txt", 0, HUNK1_HEADER, Some(&many)).unwrap_err();
        assert!(
            matches!(&e, Error::Invalid(m) if m == "lines out of range"),
            "{e:?}"
        );
    }

    #[test]
    fn quoted_path_block_is_located() {
        let raw = "\
diff --git \"a/we\\\"ird.txt\" \"b/we\\\"ird.txt\"
index 1111111..2222222 100644
--- \"a/we\\\"ird.txt\"
+++ \"b/we\\\"ird.txt\"
@@ -1,2 +1,2 @@
 q1
-q2
+Q2
";
        let p = build_hunk_patch(raw, "we\\\"ird.txt", 0, "@@ -1,2 +1,2 @@", None).unwrap();
        assert_eq!(p, raw);
    }

    #[test]
    fn path_containing_b_slash_is_located() {
        // `rfind(" b/")` alone cuts at the LAST occurrence and yields "y" —
        // the block is then never found. The exact ` b/<path>` suffix wins.
        let raw = "\
diff --git a/x b/y b/x b/y
index 1111111..2222222 100644
--- a/x b/y
+++ b/x b/y
@@ -1,2 +1,2 @@
 w1
-w2
+W2
";
        let p = build_hunk_patch(raw, "x b/y", 0, "@@ -1,2 +1,2 @@", None).unwrap();
        assert_eq!(p, raw);
    }

    #[test]
    fn unknown_path_is_not_found() {
        let e = build_hunk_patch(TWO_HUNKS, "nope.txt", 0, HUNK1_HEADER, None).unwrap_err();
        assert!(
            matches!(&e, Error::NotFound(m) if m == "no diff for nope.txt"),
            "{e:?}"
        );
    }

    /// The builder indexes raw body lines with markers skipped; `parse_diff`
    /// drops markers and treats nothing else as a body line. The two indexings
    /// MUST coincide, or a line selection made in the UI would apply to a
    /// different line server-side.
    #[test]
    fn raw_and_parsed_hunk_indices_agree() {
        let parsed = crate::parse::parse_diff(MULTI_MARKED);
        assert_eq!(parsed.files.len(), 1);

        let mut raw_hunks: Vec<Vec<&str>> = Vec::new();
        for l in MULTI_MARKED.split_inclusive('\n') {
            if l.starts_with("@@") {
                raw_hunks.push(Vec::new());
            } else if let Some(h) = raw_hunks.last_mut() {
                if !l.starts_with('\\') {
                    h.push(l);
                }
            }
        }
        assert_eq!(raw_hunks.len(), parsed.files[0].hunks.len());
        for (hi, (rh, ph)) in raw_hunks.iter().zip(&parsed.files[0].hunks).enumerate() {
            assert_eq!(rh.len(), ph.lines.len(), "hunk {hi} body length");
            for (i, (r, p)) in rh.iter().zip(&ph.lines).enumerate() {
                let want = match r.chars().next().unwrap_or(' ') {
                    '+' => LineOrigin::Add,
                    '-' => LineOrigin::Del,
                    _ => LineOrigin::Context,
                };
                assert_eq!(p.origin, want, "hunk {hi} line {i} origin");
                assert_eq!(
                    p.content,
                    r[1..].trim_end_matches('\n').trim_end_matches('\r'),
                    "hunk {hi} line {i} content"
                );
            }
        }
    }

    #[test]
    fn discard_requires_confirm() {
        let mut r = req("a.txt", 0, HUNK1_HEADER, HunkOp::Discard);
        r.confirm = false;
        let e = validate(&r).unwrap_err();
        assert!(
            matches!(&e, Error::Invalid(m) if m == "discard requires confirm:true"),
            "{e:?}"
        );
        r.confirm = true;
        assert!(validate(&r).is_ok());
        // Stage/unstage need no confirmation…
        assert!(validate(&req("a.txt", 0, HUNK1_HEADER, HunkOp::Stage)).is_ok());
        // …but an empty or control-char path never reaches git.
        assert!(matches!(
            validate(&req("", 0, HUNK1_HEADER, HunkOp::Stage)),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            validate(&req("a\nb", 0, HUNK1_HEADER, HunkOp::Stage)),
            Err(Error::Invalid(_))
        ));
    }

    // -- repo-backed ---------------------------------------------------------

    fn req(path: &str, hunk_index: usize, header: &str, op: HunkOp) -> StageHunkReq {
        StageHunkReq {
            path: path.to_string(),
            hunk_index,
            hunk_header: header.to_string(),
            lines: None,
            op,
            confirm: op == HunkOp::Discard,
        }
    }

    /// Run `git` synchronously for fixture setup.
    fn sh_git(dir: &FsPath, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
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

    /// `git` stdout as RAW BYTES — byte-exactness assertions must not go
    /// through a lossy String.
    fn git_bytes(dir: &FsPath, args: &[&str]) -> Vec<u8> {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .args(args)
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {:?} failed", args);
        out.stdout
    }

    fn write(dir: &FsPath, rel: &str, content: &str) {
        std::fs::write(dir.join(rel), content).unwrap();
    }

    /// Empty repo with deterministic identity, no signing and no CRLF fiddling.
    fn repo() -> (tempfile::TempDir, PathBuf, LocalGit) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", "main"]);
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);
        sh_git(&dir, &["config", "core.autocrlf", "false"]);
        let git = LocalGit::new(&dir);
        (tmp, dir, git)
    }

    /// 20 numbered lines committed, then lines 2 and 15 changed — two hunks
    /// that `-U3` keeps well apart.
    fn two_hunk_repo() -> (tempfile::TempDir, PathBuf, LocalGit) {
        let (tmp, dir, git) = repo();
        let base: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        write(&dir, "two.txt", &base);
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        let dirty: String = (1..=20)
            .map(|i| match i {
                2 => "LINE TWO\n".to_string(),
                15 => "LINE FIFTEEN\n".to_string(),
                _ => format!("line {i}\n"),
            })
            .collect();
        write(&dir, "two.txt", &dirty);
        (tmp, dir, git)
    }

    /// The `@@` header of hunk `hi` exactly as the UI would read it.
    async fn header_of(git: &LocalGit, target: DiffTarget, path: &str, hi: usize) -> String {
        let d = git.diff(target, Some(path)).await.unwrap();
        d.files[0].hunks[hi].header.clone()
    }

    #[tokio::test]
    async fn stage_one_of_two_hunks() {
        let (_tmp, dir, git) = two_hunk_repo();
        let h0 = header_of(&git, DiffTarget::Worktree, "two.txt", 0).await;
        let resp = run_hunk_op(&git, &req("two.txt", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();
        assert!(resp.backup_stash.is_none());

        let staged = String::from_utf8(git_bytes(&dir, &["diff", "--cached"])).unwrap();
        assert!(staged.contains("+LINE TWO"), "{staged}");
        assert!(!staged.contains("+LINE FIFTEEN"), "{staged}");
        let worktree = String::from_utf8(git_bytes(&dir, &["diff"])).unwrap();
        assert!(worktree.contains("+LINE FIFTEEN"), "{worktree}");
        assert!(!worktree.contains("+LINE TWO"), "{worktree}");
        // The response's own diff is the (still dirty) worktree for that path.
        assert_eq!(resp.diff.files.len(), 1);
    }

    #[tokio::test]
    async fn unstage_reverses() {
        let (_tmp, dir, git) = two_hunk_repo();
        let h0 = header_of(&git, DiffTarget::Worktree, "two.txt", 0).await;
        run_hunk_op(&git, &req("two.txt", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();

        let sh0 = header_of(&git, DiffTarget::Staged, "two.txt", 0).await;
        run_hunk_op(&git, &req("two.txt", 0, &sh0, HunkOp::Unstage))
            .await
            .unwrap();

        let staged = String::from_utf8(git_bytes(&dir, &["diff", "--cached"])).unwrap();
        assert!(
            staged.trim().is_empty(),
            "index should be clean again: {staged}"
        );
        // The worktree change is untouched — unstaging moves the index only.
        let worktree = String::from_utf8(git_bytes(&dir, &["diff"])).unwrap();
        assert!(worktree.contains("+LINE TWO") && worktree.contains("+LINE FIFTEEN"));
    }

    #[tokio::test]
    async fn discard_restores_hunk_and_keeps_backup_stash() {
        let (_tmp, dir, git) = two_hunk_repo();
        let h0 = header_of(&git, DiffTarget::Worktree, "two.txt", 0).await;
        let resp = run_hunk_op(&git, &req("two.txt", 0, &h0, HunkOp::Discard))
            .await
            .unwrap();
        assert!(resp.backup_stash.is_some());

        let content = std::fs::read_to_string(dir.join("two.txt")).unwrap();
        assert!(content.contains("line 2\n"), "hunk 1 reverted: {content}");
        assert!(content.contains("LINE FIFTEEN\n"), "hunk 2 kept: {content}");

        let stashes = git.stash_list().await.unwrap();
        assert_eq!(stashes.len(), 1);
        assert!(
            stashes[0].message.contains(DISCARD_BACKUP_MSG),
            "{}",
            stashes[0].message
        );
    }

    #[tokio::test]
    async fn stage_hunk_stale_header_is_409() {
        let (_tmp, dir, git) = two_hunk_repo();
        let stale = header_of(&git, DiffTarget::Worktree, "two.txt", 0).await;
        // A concurrent writer prepends a whole new hunk ABOVE the rendered one:
        // index 0 now addresses a DIFFERENT change.
        let shifted: String = std::iter::once("brand new first line\n".to_string())
            .chain((1..=20).map(|i| match i {
                2 => "LINE TWO\n".to_string(),
                15 => "LINE FIFTEEN\n".to_string(),
                _ => format!("line {i}\n"),
            }))
            .collect();
        write(&dir, "two.txt", &shifted);
        let before = std::fs::read(dir.join("two.txt")).unwrap();

        let e = run_hunk_op(&git, &req("two.txt", 0, &stale, HunkOp::Stage))
            .await
            .unwrap_err();
        assert!(matches!(&e, Error::Conflict(m) if m == STALE), "{e:?}");
        let staged = String::from_utf8(git_bytes(&dir, &["diff", "--cached"])).unwrap();
        assert!(staged.trim().is_empty(), "nothing may be staged: {staged}");

        // Same for discard: bytes untouched AND no stray backup stash.
        let e = run_hunk_op(&git, &req("two.txt", 0, &stale, HunkOp::Discard))
            .await
            .unwrap_err();
        assert!(matches!(&e, Error::Conflict(m) if m == STALE), "{e:?}");
        assert_eq!(std::fs::read(dir.join("two.txt")).unwrap(), before);
        assert!(git.stash_list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn stage_hunk_path_starting_with_dash_ok() {
        let (_tmp, dir, git) = repo();
        write(&dir, "-notes.md", "n1\nn2\nn3\n");
        sh_git(&dir, &["add", "--", "-notes.md"]);
        sh_git(&dir, &["commit", "-m", "init"]);
        write(&dir, "-notes.md", "n1\nN2\nn3\n");

        let h0 = header_of(&git, DiffTarget::Worktree, "-notes.md", 0).await;
        run_hunk_op(&git, &req("-notes.md", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();
        let staged = String::from_utf8(git_bytes(&dir, &["diff", "--cached"])).unwrap();
        assert!(staged.contains("+N2"), "{staged}");
    }

    #[tokio::test]
    async fn crlf_hunk_round_trips_bytes() {
        let (_tmp, dir, git) = repo();
        write(&dir, "crlf.txt", "c1\r\nc2\r\nc3\r\nc4\r\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        write(&dir, "crlf.txt", "c1\r\nC2\r\nc3\r\nc4\r\n");

        let h0 = header_of(&git, DiffTarget::Worktree, "crlf.txt", 0).await;
        run_hunk_op(&git, &req("crlf.txt", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();

        // What landed in the index is the file on disk, byte for byte — an
        // LF-normalised stage would leave a phantom follow-up diff.
        let indexed = git_bytes(&dir, &["show", ":crlf.txt"]);
        assert_eq!(indexed, std::fs::read(dir.join("crlf.txt")).unwrap());
        let worktree = git
            .diff(DiffTarget::Worktree, Some("crlf.txt"))
            .await
            .unwrap();
        assert!(worktree.files.is_empty(), "{worktree:?}");
    }

    #[tokio::test]
    async fn no_trailing_newline_stage_keeps_bytes() {
        let (_tmp, dir, git) = repo();
        write(&dir, "nl.txt", "a\nb\nc\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        // Appended line WITHOUT a trailing newline → the diff carries a
        // `\ No newline at end of file` marker.
        write(&dir, "nl.txt", "a\nb\nc\nd");

        let h0 = header_of(&git, DiffTarget::Worktree, "nl.txt", 0).await;
        run_hunk_op(&git, &req("nl.txt", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();

        let indexed = git_bytes(&dir, &["show", ":nl.txt"]);
        assert_eq!(indexed, b"a\nb\nc\nd");
        assert_ne!(indexed.last(), Some(&b'\n'));
        let worktree = git
            .diff(DiffTarget::Worktree, Some("nl.txt"))
            .await
            .unwrap();
        assert!(worktree.files.is_empty(), "{worktree:?}");
    }

    #[tokio::test]
    async fn diff_raw_refuses_non_index_targets() {
        let (_tmp, _dir, git) = two_hunk_repo();
        assert!(matches!(
            git.diff_raw(DiffTarget::Working, "two.txt").await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            git.diff_raw(DiffTarget::Commit("HEAD".into()), "two.txt")
                .await,
            Err(Error::Invalid(_))
        ));
    }
}
