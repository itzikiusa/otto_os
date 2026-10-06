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
//!   request together with a SHA-256 fingerprint of the byte-exact file diff.
//!   Both must match, or the call is a 409 with nothing applied.

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
use crate::local::{upstream_err, DiffTarget, GitCmd, LocalGit};

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
///
/// Text front-end of [`build_hunk_patch_bytes`], which is what the route uses.
pub fn build_hunk_patch(
    raw_diff: &str,
    path: &str,
    hunk_idx: usize,
    hunk_header: &str,
    lines: Option<&[usize]>,
) -> Result<String> {
    let patch = build_hunk_patch_bytes(raw_diff.as_bytes(), path, hunk_idx, hunk_header, lines)?;
    // Every output byte is an input byte or ASCII, so UTF-8 in → UTF-8 out.
    String::from_utf8(patch).map_err(|_| Error::Internal("hunk patch is not UTF-8".into()))
}

/// [`build_hunk_patch`] over git's RAW diff bytes, returning the patch as
/// bytes. The route never decodes the diff: a lossy UTF-8 round-trip turned a
/// Latin-1 `caf\xE9` into `caf\xEF\xBF\xBD`, and because the context lines
/// still matched, `git apply` accepted the patch and the corruption was
/// staged (or written back by unstage/discard).
pub fn build_hunk_patch_bytes(
    raw_diff: &[u8],
    path: &str,
    hunk_idx: usize,
    hunk_header: &str,
    lines: Option<&[usize]>,
) -> Result<Vec<u8>> {
    build_hunk_patch_dir(raw_diff, path, hunk_idx, hunk_header, lines, false)
}

/// [`build_hunk_patch_bytes`] for a patch that will be applied with
/// `--reverse` (Unstage, Discard). A reverse apply runs against the POST-image
/// (the index / the worktree), so a partial selection inverts: an unselected
/// `+` is on disk and must stay → CONTEXT; an unselected `-` is not on disk
/// and must not come back → dropped. Using the forward rules here made every
/// partial Unstage/Discard fail to apply (or touch the wrong lines).
pub fn build_reverse_hunk_patch_bytes(
    raw_diff: &[u8],
    path: &str,
    hunk_idx: usize,
    hunk_header: &str,
    lines: Option<&[usize]>,
) -> Result<Vec<u8>> {
    build_hunk_patch_dir(raw_diff, path, hunk_idx, hunk_header, lines, true)
}

fn build_hunk_patch_dir(
    raw_diff: &[u8],
    path: &str,
    hunk_idx: usize,
    hunk_header: &str,
    lines: Option<&[usize]>,
    reverse: bool,
) -> Result<Vec<u8>> {
    // `split_inclusive` keeps every line's own terminator: CRLF endings and a
    // missing final newline round-trip byte for byte. `lines()` normalises both.
    let all: Vec<&[u8]> = raw_diff.split_inclusive(|b| *b == b'\n').collect();
    let starts: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with(b"diff --git "))
        .map(|(i, _)| i)
        .collect();
    // Every block for this path, in order. Normally one; a TYPE CHANGE
    // (symlink ⇄ file) is two — git prints the old side's deletion, then the
    // new side's creation — and the UI shows them as ONE file whose hunks run
    // on across both (`diff-load.ts` `fileFor`). `hunk_idx` is that merged
    // index, so it is resolved across the blocks: past the first block's
    // hunks it addresses the second (it used to always hit the first).
    let blocks: Vec<(usize, usize)> = starts
        .iter()
        .enumerate()
        .map(|(n, &s)| (s, starts.get(n + 1).copied().unwrap_or(all.len())))
        .filter(|&(s, _)| diff_git_targets(&String::from_utf8_lossy(all[s]), path))
        .collect();
    if blocks.is_empty() {
        return Err(Error::NotFound(format!("no diff for {path}")));
    }
    let hunks_in = |b: &[&[u8]]| b.iter().filter(|l| l.starts_with(b"@@")).count();
    let mut hunk_idx = hunk_idx;
    let mut pick = blocks[0];
    for &(f, t) in &blocks {
        pick = (f, t);
        let n = hunks_in(&all[f..t]);
        if hunk_idx < n {
            break;
        }
        if (f, t) != *blocks.last().expect("non-empty") {
            hunk_idx -= n;
        }
    }
    let (from, to) = pick;
    let block = &all[from..to];

    // A rename carries no hunk for the bytes that moved, and a binary block has
    // no text hunks at all — neither can be applied a hunk at a time. The UI
    // turns this into "stage the whole file".
    if block.iter().any(|l| {
        l.starts_with(b"rename from")
            || l.starts_with(b"Binary files")
            || l.starts_with(b"GIT binary patch")
    }) {
        return Err(Error::Invalid("stage the whole file".into()));
    }

    // Header = everything before the first `@@` (`diff --git`, `index`,
    // `old/new mode`, `new file mode`, `deleted file mode`, `---`, `+++`) —
    // emitted verbatim. No body line can start with `@`: git prefixes them with
    // ' ', '+', '-' or '\'.
    let head_end = block
        .iter()
        .position(|l| l.starts_with(b"@@"))
        .unwrap_or(block.len());
    let hstarts: Vec<usize> = block
        .iter()
        .enumerate()
        .skip(head_end)
        .filter(|(_, l)| l.starts_with(b"@@"))
        .map(|(i, _)| i)
        .collect();
    let hunks: Vec<&[&[u8]]> = hstarts
        .iter()
        .enumerate()
        .map(|(n, &s)| &block[s..hstarts.get(n + 1).copied().unwrap_or(block.len())])
        .collect();

    // The client saw the header through the same lossy decode `parse_diff`
    // renders with; compare in that space (the header itself is ASCII bar the
    // function-context hint).
    let hunk = match hunks.get(hunk_idx) {
        Some(h) if String::from_utf8_lossy(h[0]).trim() == hunk_header.trim() => *h,
        _ => return Err(Error::Conflict(STALE.into())),
    };

    // Pair each body line with the `\ No newline at end of file` marker that
    // follows it. Markers are not body lines: they neither consume an index nor
    // count towards the hunk's line counts.
    let mut body: Vec<(&[u8], Option<&[u8]>)> = Vec::new();
    for &l in &hunk[1..] {
        if l.starts_with(b"\\") {
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
            let (in_old, in_new) = match (line.first().copied().unwrap_or(b' '), reverse) {
                (b'+', false) => (false, is_sel(i)),
                (b'-', false) => (true, !is_sel(i)),
                // Reverse: an unselected `+` is context, an unselected `-` is gone.
                (b'+', true) => (!is_sel(i), true),
                (b'-', true) => (is_sel(i), false),
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

    // Counts are recomputed for the SELECTION. Forward: `old` = context +
    // every `-` (kept or converted), `new` = context + kept `+` + converted
    // `-`. Reverse: `old` = context + kept `-` + converted `+`, `new` =
    // context + every `+` (kept or converted).
    let (mut old_count, mut new_count) = (0u32, 0u32);
    let mut body_out: Vec<u8> = Vec::new();
    for (i, (line, marker)) in body.iter().enumerate() {
        match line.first().copied().unwrap_or(b' ') {
            b'+' => {
                if is_sel(i) {
                    body_out.extend_from_slice(line);
                    push_marker(&mut body_out, *marker);
                    new_count += 1;
                } else if reverse {
                    // Reverse: the unselected `+` is in the post-image the
                    // patch is applied against — it must survive → context.
                    body_out.push(b' ');
                    body_out.extend_from_slice(&line[1..]);
                    push_marker(&mut body_out, *marker);
                    old_count += 1;
                    new_count += 1;
                }
                // Forward: an unselected `+` exists on neither side — it goes,
                // and its marker goes with it.
            }
            b'-' if reverse && !is_sel(i) => {
                // Reverse: the unselected `-` is NOT in the post-image and must
                // not be restored — it exists on neither side of the patch.
            }
            b'-' => {
                if is_sel(i) {
                    body_out.extend_from_slice(line);
                    push_marker(&mut body_out, *marker);
                    old_count += 1;
                } else {
                    body_out.push(b' ');
                    body_out.extend_from_slice(&line[1..]);
                    push_marker(&mut body_out, *marker);
                    old_count += 1;
                    new_count += 1;
                }
            }
            // Context (' '), and defensively anything else, is carried verbatim
            // on both sides.
            _ => {
                body_out.extend_from_slice(line);
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
    let (mut old_start, mut new_start, trailer) = split_hunk_header(hunk[0])?;

    // A PARTIAL selection on a created/deleted file is no longer a creation or
    // deletion: unselected lines survive as context, so the pre-image of a
    // "new file" (reverse Unstage) or the post-image of a "deleted file"
    // (forward Stage) is non-empty and `git apply` refuses ("removal patch
    // leaves file contents"). Rewrite such a block as a plain modification:
    // drop the creation/deletion mode and the all-zero `index` line, replace
    // the `/dev/null` side with the real path, and move a zero start to 1.
    let is_new = block[..head_end]
        .iter()
        .any(|l| l.starts_with(b"new file mode "));
    let is_deleted = block[..head_end]
        .iter()
        .any(|l| l.starts_with(b"deleted file mode "));
    let as_modification = (is_new && old_count > 0) || (is_deleted && new_count > 0);
    let other_side = |prefix: &[u8]| -> Option<Vec<u8>> {
        block[..head_end]
            .iter()
            .find_map(|l| l.strip_prefix(prefix))
            .map(|rest| rest.to_vec())
    };

    let mut out: Vec<u8> = Vec::with_capacity(raw_diff.len());
    for l in &block[..head_end] {
        // A hunk operation moves CONTENT only. `old mode`/`new mode` would ride
        // along with any single hunk, so discarding one hunk also reverted a
        // `chmod +x` (and staging one staged it). Creation/deletion modes
        // (`new file mode`, `deleted file mode`) are structural and stay —
        // unless the selection turned the block into a modification (above).
        if l.starts_with(b"old mode ") || l.starts_with(b"new mode ") {
            continue;
        }
        if as_modification {
            if l.starts_with(b"new file mode ")
                || l.starts_with(b"deleted file mode ")
                || l.starts_with(b"index ")
            {
                continue;
            }
            if l.starts_with(b"--- /dev/null") {
                let rest = other_side(&b"+++ "[..])
                    .ok_or_else(|| Error::Invalid("stage the whole file".into()))?;
                out.extend_from_slice(b"--- ");
                out.extend_from_slice(&swap_side_prefix(&rest, b'b', b'a')?);
                continue;
            }
            if l.starts_with(b"+++ /dev/null") {
                let rest = other_side(&b"--- "[..])
                    .ok_or_else(|| Error::Invalid("stage the whole file".into()))?;
                out.extend_from_slice(b"+++ ");
                out.extend_from_slice(&swap_side_prefix(&rest, b'a', b'b')?);
                continue;
            }
        }
        out.extend_from_slice(l);
    }
    if as_modification {
        if old_start == 0 && old_count > 0 {
            old_start = 1;
        }
        if new_start == 0 && new_count > 0 {
            new_start = 1;
        }
    }

    out.extend_from_slice(
        format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@").as_bytes(),
    );
    out.extend_from_slice(trailer);
    out.extend_from_slice(&body_out);
    if out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    Ok(out)
}

/// `b/path\n` → `a/path\n` (and `"b/p q"\n` → `"a/p q"\n`): the path of
/// one side of a `---`/`+++` pair, re-labelled for the other side. Anything
/// else (a `diff.noprefix` diff) can't be rewritten safely.
fn swap_side_prefix(rest: &[u8], from: u8, to: u8) -> Result<Vec<u8>> {
    let mut v = rest.to_vec();
    let at = usize::from(v.first() == Some(&b'"'));
    if v.get(at) == Some(&from) && v.get(at + 1) == Some(&b'/') {
        v[at] = to;
        Ok(v)
    } else {
        Err(Error::Invalid("stage the whole file".into()))
    }
}

fn push_marker(out: &mut Vec<u8>, marker: Option<&[u8]>) {
    if let Some(m) = marker {
        out.extend_from_slice(m);
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
    // git still quotes a name carrying a `"`, a backslash, a tab or a control
    // character even with `core.quotePath=false`. The client sends the RAW
    // name (status/summary are `-z`, and the parser unquotes headers), so
    // compare the UNQUOTED form; the escaped text is still accepted verbatim
    // for a caller echoing what it was shown.
    if let Some(idx) = rest.rfind(" \"b/") {
        let quoted = rest[idx + 1..].trim();
        if crate::parse::unquote_c(quoted).strip_prefix("b/") == Some(path) {
            return true;
        }
        if rest[idx + 4..].trim().trim_end_matches('"') == path {
            return true;
        }
    }
    false
}

/// `@@ -12,7 +12,9 @@ fn foo() {\n` → `(12, 12, b" fn foo() {\n")`. The
/// trailer stays bytes: git copies the function-context hint straight out of
/// the file, in whatever encoding the file uses.
fn split_hunk_header(line: &[u8]) -> Result<(u32, u32, &[u8])> {
    let bad = || Error::Invalid("unparsable hunk header".to_string());
    let rest = line.strip_prefix(b"@@").ok_or_else(bad)?;
    let close = rest.windows(2).position(|w| w == b"@@").ok_or_else(bad)?;
    let ranges = std::str::from_utf8(&rest[..close]).map_err(|_| bad())?;
    let (mut old_start, mut new_start) = (None, None);
    for tok in ranges.split_whitespace() {
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
    ///
    /// BYTES, built exactly like [`LocalGit::diff`]'s worktree/staged calls
    /// (same [`GitCmd::diff`] format flags, literal `-- <path>`), so the
    /// SHA-256 here equals the `FileDiff.fingerprint` the client was shown.
    pub async fn diff_raw(&self, target: DiffTarget, path: &str) -> Result<Vec<u8>> {
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
        let mut cmd = GitCmd::diff("diff")
            .args(["-U3"])
            .args(crate::local::RENAMES);
        if cached {
            cmd = cmd.args(["--cached"]);
        }
        self.exec_bytes(&cmd.paths([path])).await
    }

    /// `git apply [--cached] [--reverse] --recount --whitespace=nowarn -`, with
    /// the patch on stdin (no positional arguments, so nothing the caller sent
    /// can reach argv). `-U3` context is retained deliberately — no
    /// `--unidiff-zero` — so `apply` verifies placement instead of trusting the
    /// line numbers.
    pub async fn apply_patch(&self, patch: &[u8], cached: bool, reverse: bool) -> Result<()> {
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
            let (ok, stdout, stderr, code) = self.run_raw_stdin(&args, patch).await?;
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
    /// The `@@ … @@` line the client rendered; coordinates supplement the content fingerprint.
    pub hunk_header: String,
    #[serde(default)]
    pub fingerprint: String,
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
    use sha2::{Digest, Sha256};
    if req.fingerprint != hex::encode(Sha256::digest(raw.as_slice())) {
        return Err(Error::Conflict(STALE.into()));
    }
    // Unstage/Discard reverse-apply the patch, so the selection must be
    // built with the reverse rules (see `build_reverse_hunk_patch_bytes`).
    let build = match req.op {
        HunkOp::Stage => build_hunk_patch_bytes,
        HunkOp::Unstage | HunkOp::Discard => build_reverse_hunk_patch_bytes,
    };
    let patch = build(
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
        // Capped like the per-file `/diff?path=&full=true` the UI loaded the
        // file with (a "Load anyway" file must not come back as 40k lines of
        // JSON on every hunk click; past the ceiling it is `too_large`).
        diff: git
            .diff_with(
                &target,
                &crate::local::DiffOpts {
                    path: Some(req.path.clone()),
                    caps: Some(crate::parse::DiffCaps::FULL_FILE),
                    ..Default::default()
                },
            )
            .await?,
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

    /// A symlink → file TYPE CHANGE: two blocks for one path.
    const TYPECHANGE: &str = "\
diff --git a/link b/link
deleted file mode 120000
index 1111111..0000000
--- a/link
+++ /dev/null
@@ -1 +0,0 @@
-plain.rs
\\ No newline at end of file
diff --git a/link b/link
new file mode 100644
index 0000000..2222222
--- /dev/null
+++ b/link
@@ -0,0 +1 @@
+now a file
";

    /// The UI shows a type change as ONE file whose hunks run across both
    /// blocks (`fileFor`): index 0 is the old side's deletion, index 1 the
    /// new side's creation — which used to resolve to the FIRST block again
    /// (a stale-hunk error, or the wrong half).
    #[test]
    fn typechange_hunk_index_spans_both_blocks() {
        let first = build_hunk_patch(TYPECHANGE, "link", 0, "@@ -1 +0,0 @@", None).unwrap();
        assert!(first.contains("deleted file mode 120000"), "{first}");
        assert!(first.contains("-plain.rs"));
        assert!(!first.contains("now a file"));
        let second = build_hunk_patch(TYPECHANGE, "link", 1, "@@ -0,0 +1 @@", None).unwrap();
        assert!(second.contains("new file mode 100644"), "{second}");
        assert!(second.contains("+now a file"));
        assert!(!second.contains("plain.rs"));
        // Past the last hunk is stale, never a wrap-around.
        assert!(matches!(
            build_hunk_patch(TYPECHANGE, "link", 2, "@@ -0,0 +1 @@", None),
            Err(Error::Conflict(_))
        ));
    }

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

    /// S2-10: a PARTIAL reverse selection on a created file keeps unselected
    /// lines → the block is rewritten as a modification, not a removal.
    #[test]
    fn partial_reverse_on_new_file_becomes_modification() {
        let p = build_reverse_hunk_patch_bytes(
            NEW_FILE.as_bytes(),
            "n.txt",
            0,
            "@@ -0,0 +1,3 @@",
            Some(&[0]),
        )
        .unwrap();
        let p = String::from_utf8(p).unwrap();
        assert_eq!(
            p,
            "diff --git a/n.txt b/n.txt\n--- a/n.txt\n+++ b/n.txt\n@@ -1,2 +1,3 @@\n+n1\n n2\n n3\n"
        );
    }

    /// S2-10: a PARTIAL forward selection on a deleted file leaves lines in
    /// the post-image → modification with a real `+++` path.
    #[test]
    fn partial_forward_on_deleted_file_becomes_modification() {
        let p = build_hunk_patch(DELETED_FILE, "x.txt", 0, "@@ -1,3 +0,0 @@", Some(&[0])).unwrap();
        assert_eq!(
            p,
            "diff --git a/x.txt b/x.txt\n--- a/x.txt\n+++ b/x.txt\n@@ -1,3 +1,2 @@\n-x1\n x2\n x3\n"
        );
        // The whole selection is still a deletion.
        let full = build_hunk_patch(
            DELETED_FILE,
            "x.txt",
            0,
            "@@ -1,3 +0,0 @@",
            Some(&[0, 1, 2]),
        )
        .unwrap();
        assert_eq!(full, DELETED_FILE);
    }

    #[test]
    fn swap_side_prefix_handles_quoted_paths() {
        assert_eq!(swap_side_prefix(b"b/x\n", b'b', b'a').unwrap(), b"a/x\n");
        assert_eq!(
            swap_side_prefix(b"\"b/p q\"\n", b'b', b'a').unwrap(),
            b"\"a/p q\"\n"
        );
        assert!(swap_side_prefix(b"x\n", b'b', b'a').is_err());
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

    /// A hunk op moves content only: a pending `chmod +x` must neither ride
    /// along with one staged hunk nor be reverted by one discarded hunk.
    #[test]
    fn mode_change_lines_are_not_part_of_a_hunk_patch() {
        let raw = "\
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
index 1111111..2222222
--- a/run.sh
+++ b/run.sh
@@ -1,2 +1,2 @@
 r1
-r2
+R2
";
        let p = build_hunk_patch(raw, "run.sh", 0, "@@ -1,2 +1,2 @@", None).unwrap();
        assert!(!p.contains("old mode") && !p.contains("new mode"), "{p}");
        assert!(
            p.starts_with("diff --git a/run.sh b/run.sh\nindex 1111111..2222222\n"),
            "{p}"
        );
        // `new file mode` is structural and stays.
        let p = build_hunk_patch(NEW_FILE, "n.txt", 0, "@@ -0,0 +1,3 @@", None).unwrap();
        assert!(p.contains("new file mode 100644\n"), "{p}");
    }

    /// Non-UTF-8 bytes pass through the builder untouched (no U+FFFD).
    #[test]
    fn non_utf8_lines_round_trip_bytes() {
        let raw: &[u8] = b"diff --git a/l.txt b/l.txt\n\
index 1111111..2222222 100644\n\
--- a/l.txt\n\
+++ b/l.txt\n\
@@ -1,2 +1,2 @@ caf\xe9()\n \
l1\n\
-caf\xe9\n\
+CAF\xc9\n";
        let p =
            build_hunk_patch_bytes(raw, "l.txt", 0, "@@ -1,2 +1,2 @@ caf\u{fffd}()", None).unwrap();
        assert_eq!(p, raw);
        // A partial selection converting the `-` to context keeps its bytes.
        let p =
            build_hunk_patch_bytes(raw, "l.txt", 0, "@@ -1,2 +1,2 @@ caf\u{fffd}()", Some(&[2]))
                .unwrap();
        assert!(p.ends_with(b" l1\n caf\xe9\n+CAF\xc9\n"), "{p:?}");
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
        // The RAW name — what status/summary hand the client — matches too.
        let p = build_hunk_patch(raw, "we\"ird.txt", 0, "@@ -1,2 +1,2 @@", None).unwrap();
        assert_eq!(p, raw);
        let tab = raw.replace("we\\\"ird", "t\\tab");
        let p = build_hunk_patch(&tab, "t\tab.txt", 0, "@@ -1,2 +1,2 @@", None).unwrap();
        assert_eq!(p, tab);
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
            fingerprint: String::new(),
            lines: None,
            op,
            confirm: op == HunkOp::Discard,
        }
    }

    async fn run_test_hunk_op(git: &LocalGit, req: &StageHunkReq) -> Result<StageHunkResp> {
        use sha2::{Digest, Sha256};
        let raw = git
            .diff_raw(
                if req.op == HunkOp::Unstage {
                    DiffTarget::Staged
                } else {
                    DiffTarget::Worktree
                },
                &req.path,
            )
            .await?;
        let current = StageHunkReq {
            path: req.path.clone(),
            hunk_index: req.hunk_index,
            hunk_header: req.hunk_header.clone(),
            fingerprint: hex::encode(Sha256::digest(raw.as_slice())),
            lines: req.lines.clone(),
            op: req.op,
            confirm: req.confirm,
        };
        run_hunk_op(git, &current).await
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
        let resp = run_test_hunk_op(&git, &req("two.txt", 0, &h0, HunkOp::Stage))
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

    /// A symlink → file type change end to end: both parsed blocks carry the
    /// fingerprint of the path's WHOLE raw diff (what the hunk op re-hashes),
    /// so the UI's merged file (`fileFor`) can stage — the deletion half here
    /// — instead of always failing as stale.
    #[tokio::test]
    async fn typechange_stages_with_the_merged_fingerprint() {
        use sha2::{Digest, Sha256};
        let (_tmp, dir, git) = repo();
        write(&dir, "target.txt", "t\n");
        std::os::unix::fs::symlink("target.txt", dir.join("link")).unwrap();
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        std::fs::remove_file(dir.join("link")).unwrap();
        write(&dir, "link", "now a file\n");

        let d = git.diff(DiffTarget::Worktree, Some("link")).await.unwrap();
        assert_eq!(d.files.len(), 2, "git prints a type change as two blocks");
        let raw = git.diff_raw(DiffTarget::Worktree, "link").await.unwrap();
        let whole = hex::encode(Sha256::digest(raw.as_slice()));
        assert!(d.files.iter().all(|f| f.fingerprint == whole));

        let request = StageHunkReq {
            fingerprint: d.files[1].fingerprint.clone(), // the merged file's
            ..req("link", 0, &d.files[0].hunks[0].header, HunkOp::Stage)
        };
        run_hunk_op(&git, &request)
            .await
            .expect("stages the deletion half");
        let staged = String::from_utf8(git_bytes(&dir, &["diff", "--cached"])).unwrap();
        assert!(staged.contains("deleted file mode 120000"), "{staged}");
    }

    /// `line 2`,`line 3` → `A`,`B` in one hunk; body indices:
    /// 0 ` line 1`, 1 `-line 2`, 2 `-line 3`, 3 `+A`, 4 `+B`, …
    fn replace_two_repo() -> (tempfile::TempDir, PathBuf, LocalGit) {
        let (tmp, dir, git) = repo();
        write(&dir, "r.txt", "line 1\nline 2\nline 3\nline 4\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        write(&dir, "r.txt", "line 1\nA\nB\nline 4\n");
        (tmp, dir, git)
    }

    /// Partial Unstage (line-level, reverse apply): only `line 2 → A` leaves
    /// the index; `line 3 → B` stays staged; the worktree is untouched.
    #[tokio::test]
    async fn partial_unstage_reverses_only_the_selected_lines() {
        let (_tmp, dir, git) = replace_two_repo();
        sh_git(&dir, &["add", "r.txt"]);
        let h = header_of(&git, DiffTarget::Staged, "r.txt", 0).await;
        let request = StageHunkReq {
            lines: Some(vec![1, 3]),
            ..req("r.txt", 0, &h, HunkOp::Unstage)
        };
        run_test_hunk_op(&git, &request)
            .await
            .expect("partial unstage applies");
        let index = String::from_utf8(git_bytes(&dir, &["show", ":r.txt"])).unwrap();
        assert_eq!(index, "line 1\nline 2\nB\nline 4\n");
        let disk = std::fs::read_to_string(dir.join("r.txt")).unwrap();
        assert_eq!(disk, "line 1\nA\nB\nline 4\n");
    }

    /// Partial Discard (line-level, reverse apply on the worktree): only the
    /// selected pair is reverted.
    #[tokio::test]
    async fn partial_discard_reverts_only_the_selected_lines() {
        let (_tmp, dir, git) = replace_two_repo();
        let h = header_of(&git, DiffTarget::Worktree, "r.txt", 0).await;
        let request = StageHunkReq {
            lines: Some(vec![1, 3]),
            ..req("r.txt", 0, &h, HunkOp::Discard)
        };
        run_test_hunk_op(&git, &request)
            .await
            .expect("partial discard applies");
        let disk = std::fs::read_to_string(dir.join("r.txt")).unwrap();
        assert_eq!(disk, "line 1\nline 2\nB\nline 4\n");
    }

    /// S2-10: partial Unstage of a staged NEW file leaves the unselected
    /// lines in the index (it used to fail: "removal patch leaves contents").
    #[tokio::test]
    async fn partial_unstage_of_new_file_applies() {
        let (_tmp, dir, git) = repo();
        write(&dir, "seed.txt", "seed\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        write(&dir, "n.txt", "n1\nn2\nn3\n");
        sh_git(&dir, &["add", "n.txt"]);
        let h = header_of(&git, DiffTarget::Staged, "n.txt", 0).await;
        let request = StageHunkReq {
            lines: Some(vec![0]),
            ..req("n.txt", 0, &h, HunkOp::Unstage)
        };
        run_test_hunk_op(&git, &request)
            .await
            .expect("partial unstage of a new file applies");
        let index = String::from_utf8(git_bytes(&dir, &["show", ":n.txt"])).unwrap();
        assert_eq!(index, "n2\nn3\n");
        let disk = std::fs::read_to_string(dir.join("n.txt")).unwrap();
        assert_eq!(disk, "n1\nn2\nn3\n");
    }

    /// S2-10: partial Stage of a DELETED file stages only the selected
    /// removals; the file stays in the index with the rest.
    #[tokio::test]
    async fn partial_stage_of_deleted_file_applies() {
        let (_tmp, dir, git) = repo();
        write(&dir, "x.txt", "x1\nx2\nx3\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        std::fs::remove_file(dir.join("x.txt")).unwrap();
        let h = header_of(&git, DiffTarget::Worktree, "x.txt", 0).await;
        let request = StageHunkReq {
            lines: Some(vec![0]),
            ..req("x.txt", 0, &h, HunkOp::Stage)
        };
        run_test_hunk_op(&git, &request)
            .await
            .expect("partial stage of a deleted file applies");
        let index = String::from_utf8(git_bytes(&dir, &["show", ":x.txt"])).unwrap();
        assert_eq!(index, "x2\nx3\n");
    }

    #[tokio::test]
    async fn unstage_reverses() {
        let (_tmp, dir, git) = two_hunk_repo();
        let h0 = header_of(&git, DiffTarget::Worktree, "two.txt", 0).await;
        run_test_hunk_op(&git, &req("two.txt", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();

        let sh0 = header_of(&git, DiffTarget::Staged, "two.txt", 0).await;
        run_test_hunk_op(&git, &req("two.txt", 0, &sh0, HunkOp::Unstage))
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
        let resp = run_test_hunk_op(&git, &req("two.txt", 0, &h0, HunkOp::Discard))
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
    async fn displayed_fingerprint_authorizes_only_that_file_diff() {
        let (_tmp, dir, git) = two_hunk_repo();
        write(&dir, "other.txt", "other\r\n");
        sh_git(&dir, &["add", "other.txt"]);
        let display = git.diff(DiffTarget::Worktree, None).await.unwrap();
        let file = display.files.iter().find(|f| f.path == "two.txt").unwrap();
        let request = StageHunkReq {
            path: file.path.clone(),
            hunk_index: 0,
            hunk_header: file.hunks[0].header.clone(),
            fingerprint: file.fingerprint.clone(),
            lines: None,
            op: HunkOp::Stage,
            confirm: false,
        };
        run_hunk_op(&git, &request).await.unwrap();
        assert!(matches!(
            run_hunk_op(&git, &request).await,
            Err(Error::Conflict(_))
        ));
        let staged = git.diff(DiffTarget::Staged, None).await.unwrap();
        let file = staged.files.iter().find(|f| f.path == "two.txt").unwrap();
        let request = StageHunkReq {
            path: file.path.clone(),
            hunk_index: 0,
            hunk_header: file.hunks[0].header.clone(),
            fingerprint: file.fingerprint.clone(),
            lines: None,
            op: HunkOp::Unstage,
            confirm: false,
        };
        run_hunk_op(&git, &request).await.unwrap();
        assert_eq!(
            git.diff(DiffTarget::Staged, None)
                .await
                .unwrap()
                .files
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn same_coordinates_new_content_is_rejected() {
        let (_tmp, dir, git) = two_hunk_repo();
        let raw = git.diff_raw(DiffTarget::Worktree, "two.txt").await.unwrap();
        use sha2::{Digest, Sha256};
        let fingerprint = hex::encode(Sha256::digest(raw.as_slice()));
        let header = header_of(&git, DiffTarget::Worktree, "two.txt", 0).await;
        let changed = std::fs::read_to_string(dir.join("two.txt"))
            .unwrap()
            .replace("LINE TWO", "UNREVIEWED");
        write(&dir, "two.txt", &changed);
        for op in ["stage", "discard"] {
            let request: StageHunkReq = serde_json::from_value(serde_json::json!({
                "path":"two.txt", "hunk_index":0, "hunk_header":header,
                "fingerprint":fingerprint, "op":op, "confirm":true
            }))
            .unwrap();
            assert!(matches!(
                run_hunk_op(&git, &request).await,
                Err(Error::Conflict(_))
            ));
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("two.txt")).unwrap(),
            changed
        );
        assert!(git.stash_list().await.unwrap().is_empty());
        assert!(git
            .diff(DiffTarget::Staged, None)
            .await
            .unwrap()
            .files
            .is_empty());
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

        let e = run_test_hunk_op(&git, &req("two.txt", 0, &stale, HunkOp::Stage))
            .await
            .unwrap_err();
        assert!(matches!(&e, Error::Conflict(m) if m == STALE), "{e:?}");
        let staged = String::from_utf8(git_bytes(&dir, &["diff", "--cached"])).unwrap();
        assert!(staged.trim().is_empty(), "nothing may be staged: {staged}");

        // Same for discard: bytes untouched AND no stray backup stash.
        let e = run_test_hunk_op(&git, &req("two.txt", 0, &stale, HunkOp::Discard))
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
        run_test_hunk_op(&git, &req("-notes.md", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();
        let staged = String::from_utf8(git_bytes(&dir, &["diff", "--cached"])).unwrap();
        assert!(staged.contains("+N2"), "{staged}");
    }

    /// G-2: a Latin-1 line staged through a hunk op lands in the index byte
    /// for byte — the lossy String round-trip staged `\xEF\xBF\xBD` instead.
    #[tokio::test]
    async fn non_utf8_hunk_stage_keeps_bytes() {
        let (_tmp, dir, git) = repo();
        std::fs::write(dir.join("latin.txt"), b"l1\nl2\nl3\n").unwrap();
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        std::fs::write(dir.join("latin.txt"), b"l1\ncaf\xe9\nl3\n").unwrap();

        let h0 = header_of(&git, DiffTarget::Worktree, "latin.txt", 0).await;
        run_test_hunk_op(&git, &req("latin.txt", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();
        assert_eq!(
            git_bytes(&dir, &["show", ":latin.txt"]),
            b"l1\ncaf\xe9\nl3\n"
        );

        // …and discarding it back writes the ORIGINAL bytes, not U+FFFD.
        let sh0 = header_of(&git, DiffTarget::Staged, "latin.txt", 0).await;
        run_test_hunk_op(&git, &req("latin.txt", 0, &sh0, HunkOp::Unstage))
            .await
            .unwrap();
        let h0 = header_of(&git, DiffTarget::Worktree, "latin.txt", 0).await;
        run_test_hunk_op(&git, &req("latin.txt", 0, &h0, HunkOp::Discard))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(dir.join("latin.txt")).unwrap(),
            b"l1\nl2\nl3\n"
        );
    }

    /// G-1: the path is a LITERAL name — a route folder like `app/[id]` must
    /// not glob-match `app/d` into the diff (whose two blocks made every hunk
    /// op on it a 409).
    #[tokio::test]
    async fn bracketed_route_path_is_literal() {
        let (_tmp, dir, git) = repo();
        std::fs::create_dir_all(dir.join("app/[id]")).unwrap();
        std::fs::create_dir_all(dir.join("app/d")).unwrap();
        write(&dir, "app/[id]/page.tsx", "a1\na2\n");
        write(&dir, "app/d/page.tsx", "d1\nd2\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        write(&dir, "app/[id]/page.tsx", "a1\nA2\n");
        write(&dir, "app/d/page.tsx", "d1\nD2\n");

        let raw = git
            .diff_raw(DiffTarget::Worktree, "app/[id]/page.tsx")
            .await
            .unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(!text.contains("app/d/page.tsx"), "{text}");

        let h0 = header_of(&git, DiffTarget::Worktree, "app/[id]/page.tsx", 0).await;
        run_test_hunk_op(&git, &req("app/[id]/page.tsx", 0, &h0, HunkOp::Stage))
            .await
            .unwrap();
        let staged =
            String::from_utf8(git_bytes(&dir, &["diff", "--cached", "--name-only"])).unwrap();
        assert_eq!(staged.trim(), "app/[id]/page.tsx");
    }

    #[tokio::test]
    async fn crlf_hunk_round_trips_bytes() {
        let (_tmp, dir, git) = repo();
        write(&dir, "crlf.txt", "c1\r\nc2\r\nc3\r\nc4\r\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        write(&dir, "crlf.txt", "c1\r\nC2\r\nc3\r\nc4\r\n");

        let h0 = header_of(&git, DiffTarget::Worktree, "crlf.txt", 0).await;
        run_test_hunk_op(&git, &req("crlf.txt", 0, &h0, HunkOp::Stage))
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
        run_test_hunk_op(&git, &req("nl.txt", 0, &h0, HunkOp::Stage))
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
