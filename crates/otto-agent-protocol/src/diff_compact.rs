//! Compact contextual unified-diff builder for `TurnItemKind::FileChange`.
//!
//! ACP (`agy acp`, `gemini --acp`, `opencode acp`) and full-file `write` tools
//! emit complete before/after file buffers (`old_text` + `new_text`), which can
//! reach 1.5+ MB per tool call on large source files. Converting oversized
//! before/after buffers (`> 8 KB`) into bounded contextual unified diffs
//! (`<= 48 KB`, 3 lines of surrounding context) shrinks persisted SQLite rows,
//! in-memory projections, and on-demand item fetches by 30x–500x while giving
//! `InlineDiff` real hunk line numbers.

use otto_core::agent_runtime::{TurnItem, TurnItemKind};

pub(crate) const COMPACT_FILE_CHANGE_THRESHOLD_BYTES: usize = 8 * 1024;
pub(crate) const MAX_DIFF_PATCH_BYTES: usize = 48 * 1024;
const CONTEXT_LINES: usize = 3;
const LOOKAHEAD_LINES: usize = 64;

fn truncate_utf8_lines(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(nl) = s[..end].rfind('\n') {
        end = nl + 1;
    }
    let mut out = s[..end].to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[derive(Clone, Copy)]
enum DiffOp<'a> {
    Equal(&'a str),
    Delete(&'a str),
    Insert(&'a str),
}

fn diff_slice<'a>(old_mid: &[&'a str], new_mid: &[&'a str]) -> Vec<DiffOp<'a>> {
    let mut ops = Vec::with_capacity(old_mid.len() + new_mid.len());
    let mut i = 0;
    let mut j = 0;
    while i < old_mid.len() && j < new_mid.len() {
        if old_mid[i] == new_mid[j] {
            ops.push(DiffOp::Equal(old_mid[i]));
            i += 1;
            j += 1;
            continue;
        }
        // Greedy windowed resync: look ahead up to `LOOKAHEAD_LINES` for a
        // 2-line anchor match (or 1-line at slice end) to split separate hunks.
        let max_o = (i + LOOKAHEAD_LINES).min(old_mid.len());
        let max_n = (j + LOOKAHEAD_LINES).min(new_mid.len());
        let mut best: Option<(usize, usize)> = None;
        'search: for dist in 1..=(LOOKAHEAD_LINES * 2) {
            for doff in 0..=dist {
                let noff = dist - doff;
                let oi = i + doff;
                let nj = j + noff;
                if oi >= max_o || nj >= max_n {
                    continue;
                }
                if old_mid[oi] == new_mid[nj] {
                    let next_matches = (oi + 1 < old_mid.len()
                        && nj + 1 < new_mid.len()
                        && old_mid[oi + 1] == new_mid[nj + 1])
                        || (oi + 1 == old_mid.len() && nj + 1 == new_mid.len());
                    if next_matches {
                        best = Some((oi, nj));
                        break 'search;
                    }
                }
            }
        }
        if let Some((oi, nj)) = best {
            while i < oi {
                ops.push(DiffOp::Delete(old_mid[i]));
                i += 1;
            }
            while j < nj {
                ops.push(DiffOp::Insert(new_mid[j]));
                j += 1;
            }
        } else {
            ops.push(DiffOp::Delete(old_mid[i]));
            ops.push(DiffOp::Insert(new_mid[j]));
            i += 1;
            j += 1;
        }
    }
    while i < old_mid.len() {
        ops.push(DiffOp::Delete(old_mid[i]));
        i += 1;
    }
    while j < new_mid.len() {
        ops.push(DiffOp::Insert(new_mid[j]));
        j += 1;
    }
    ops
}

/// Build a bounded unified diff section (`diff --git a/... b/...`) for a single file.
pub(crate) fn unified_file_patch(
    path: &str,
    old: Option<&str>,
    new: &str,
    max_bytes: usize,
) -> String {
    let p = path.trim_start_matches('/');
    let p = if p.is_empty() { "file" } else { p };
    let old_lines: Vec<&str> = old.map(|o| o.lines().collect()).unwrap_or_default();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut prefix = 0usize;
    let common_cap = old_lines.len().min(new_lines.len());
    while prefix < common_cap && old_lines[prefix] == new_lines[prefix] {
        prefix += 1;
    }
    if prefix == old_lines.len() && prefix == new_lines.len() {
        return String::new();
    }

    let mut old_end = old_lines.len();
    let mut new_end = new_lines.len();
    while old_end > prefix && new_end > prefix && old_lines[old_end - 1] == new_lines[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }

    let mut out = String::with_capacity(1024);
    out.push_str(&format!("diff --git a/{p} b/{p}\n"));
    if old.is_none() {
        out.push_str("--- /dev/null\n");
    } else {
        out.push_str(&format!("--- a/{p}\n"));
    }
    out.push_str(&format!("+++ b/{p}\n"));

    let ops = diff_slice(&old_lines[prefix..old_end], &new_lines[prefix..new_end]);

    // Split `ops` into hunks wherever an `Equal` run exceeds `2 * CONTEXT_LINES`.
    let mut op_idx = 0usize;
    let mut cur_old = prefix;
    let mut cur_new = prefix;

    while op_idx < ops.len() {
        // Advance past leading equals (shouldn't happen at start, may happen after split).
        while op_idx < ops.len() {
            if matches!(ops[op_idx], DiffOp::Equal(_)) {
                cur_old += 1;
                cur_new += 1;
                op_idx += 1;
            } else {
                break;
            }
        }
        if op_idx >= ops.len() {
            break;
        }

        let hunk_start_op = op_idx;
        let hunk_old_before = cur_old;
        let hunk_new_before = cur_new;

        let mut end_op = op_idx;
        let mut eq_run = 0usize;
        while end_op < ops.len() {
            match ops[end_op] {
                DiffOp::Equal(_) => {
                    eq_run += 1;
                    if eq_run > CONTEXT_LINES * 2 {
                        // Cut hunk leaving `CONTEXT_LINES` trailing context.
                        end_op = end_op + 1 - eq_run + CONTEXT_LINES;
                        break;
                    }
                }
                DiffOp::Delete(_) | DiffOp::Insert(_) => {
                    eq_run = 0;
                }
            }
            end_op += 1;
        }
        // Trim extra trailing equals at the very end of `ops` beyond `CONTEXT_LINES`.
        if end_op == ops.len() && eq_run > CONTEXT_LINES {
            end_op -= eq_run - CONTEXT_LINES;
        }

        // Pre-context comes either from preceding `Equal` ops or from `old_lines[..prefix]`.
        let pre_ctx_start_old = hunk_old_before.saturating_sub(CONTEXT_LINES);
        let pre_ctx_len = hunk_old_before - pre_ctx_start_old;
        let pre_ctx_start_new = hunk_new_before - pre_ctx_len;

        // Post-context if hunk reaches the end of `ops`: up to `CONTEXT_LINES` from suffix.
        let post_suffix_len = if end_op == ops.len() {
            (old_lines.len() - old_end).min(CONTEXT_LINES)
        } else {
            0
        };

        let mut old_count = pre_ctx_len + post_suffix_len;
        let mut new_count = pre_ctx_len + post_suffix_len;
        for op in &ops[hunk_start_op..end_op] {
            match op {
                DiffOp::Equal(_) => {
                    old_count += 1;
                    new_count += 1;
                }
                DiffOp::Delete(_) => old_count += 1,
                DiffOp::Insert(_) => new_count += 1,
            }
        }

        let old_hdr_start = if old_count == 0 {
            pre_ctx_start_old
        } else {
            pre_ctx_start_old + 1
        };
        let new_hdr_start = if new_count == 0 {
            pre_ctx_start_new
        } else {
            pre_ctx_start_new + 1
        };

        let mut hunk_buf =
            format!("@@ -{old_hdr_start},{old_count} +{new_hdr_start},{new_count} @@\n");
        for line in &old_lines[pre_ctx_start_old..hunk_old_before] {
            hunk_buf.push(' ');
            hunk_buf.push_str(line);
            hunk_buf.push('\n');
        }
        for op in &ops[hunk_start_op..end_op] {
            match op {
                DiffOp::Equal(l) => {
                    hunk_buf.push(' ');
                    hunk_buf.push_str(l);
                    hunk_buf.push('\n');
                    cur_old += 1;
                    cur_new += 1;
                }
                DiffOp::Delete(l) => {
                    hunk_buf.push('-');
                    hunk_buf.push_str(l);
                    hunk_buf.push('\n');
                    cur_old += 1;
                }
                DiffOp::Insert(l) => {
                    hunk_buf.push('+');
                    hunk_buf.push_str(l);
                    hunk_buf.push('\n');
                    cur_new += 1;
                }
            }
        }
        for line in &old_lines[old_end..(old_end + post_suffix_len)] {
            hunk_buf.push(' ');
            hunk_buf.push_str(line);
            hunk_buf.push('\n');
        }

        if out.len() + hunk_buf.len() > max_bytes {
            let remaining = max_bytes.saturating_sub(out.len());
            if remaining > 64 {
                out.push_str(&truncate_utf8_lines(&hunk_buf, remaining));
            }
            break;
        }
        out.push_str(&hunk_buf);
        op_idx = end_op;
    }

    out
}

/// Build a bounded multi-file unified diff over `(path, old, new)` entries.
pub(crate) fn multi_file_unified_patch(diffs: &[(String, Option<String>, String)]) -> String {
    let mut out = String::new();
    for (path, old, new) in diffs {
        if out.len() >= MAX_DIFF_PATCH_BYTES {
            break;
        }
        let budget = MAX_DIFF_PATCH_BYTES - out.len();
        let section = unified_file_patch(path, old.as_deref(), new, budget);
        out.push_str(&section);
    }
    out
}

/// Normalize `(diff_str, old_str, new_str)` on a `FileChange` so oversized
/// before/after file buffers (`> 8 KB`) become a bounded contextual unified
/// diff (`<= 48 KB`) with `old_str = None, new_str = None`.
pub(crate) fn compact_file_change_fields(
    file_name: &str,
    diff_str: Option<String>,
    old_str: Option<String>,
    new_str: Option<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    if let Some(d) = diff_str {
        let bounded = if d.len() > MAX_DIFF_PATCH_BYTES {
            truncate_utf8_lines(&d, MAX_DIFF_PATCH_BYTES)
        } else {
            d
        };
        let raw_bytes =
            old_str.as_ref().map_or(0, String::len) + new_str.as_ref().map_or(0, String::len);
        if raw_bytes > COMPACT_FILE_CHANGE_THRESHOLD_BYTES {
            return (Some(bounded), None, None);
        }
        return (Some(bounded), old_str, new_str);
    }

    let raw_bytes =
        old_str.as_ref().map_or(0, String::len) + new_str.as_ref().map_or(0, String::len);
    if raw_bytes <= COMPACT_FILE_CHANGE_THRESHOLD_BYTES {
        return (None, old_str, new_str);
    }

    let patch = unified_file_patch(
        file_name,
        old_str.as_deref(),
        new_str.as_deref().unwrap_or(""),
        MAX_DIFF_PATCH_BYTES,
    );
    let patch_opt = (!patch.is_empty()).then_some(patch);
    (patch_opt, None, None)
}

/// Bound an existing `TurnItem` in place (used during SQLite projection hydration
/// so legacy multi-hundred-KB `FileChange` rows immediately shrink in memory).
pub(crate) fn bound_item_in_place(item: &mut TurnItem) {
    if let TurnItemKind::FileChange {
        file_name,
        diff_str,
        old_str,
        new_str,
    } = &mut item.kind
    {
        let (d, o, n) =
            compact_file_change_fields(file_name, diff_str.take(), old_str.take(), new_str.take());
        *diff_str = d;
        *old_str = o;
        *new_str = n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_full_file_edit_becomes_small_contextual_unified_diff() {
        let mut old_lines: Vec<String> = (1..=5_000)
            .map(|n| format!("pub fn helper_{n}() -> usize {{ {n} }}"))
            .collect();
        let mut new_lines = old_lines.clone();
        new_lines[2_499] = "pub fn helper_2500() -> usize { 999_999 }".into();
        old_lines.push(String::new());
        new_lines.push(String::new());
        let old_text = old_lines.join("\n");
        let new_text = new_lines.join("\n");
        assert!(old_text.len() + new_text.len() > 300_000);

        let (diff_str, out_old, out_new) =
            compact_file_change_fields("/repo/src/lib.rs", None, Some(old_text), Some(new_text));
        assert!(out_old.is_none());
        assert!(out_new.is_none());
        let patch = diff_str.expect("expected unified diff");
        assert!(patch.len() < 1_024, "patch was {} bytes", patch.len());
        assert!(patch.contains("--- a/repo/src/lib.rs"));
        assert!(patch.contains("+++ b/repo/src/lib.rs"));
        assert!(patch.contains("@@ -2497,7 +2497,7 @@"));
        assert!(patch.contains("-pub fn helper_2500() -> usize { 2500 }"));
        assert!(patch.contains("+pub fn helper_2500() -> usize { 999_999 }"));
    }

    #[test]
    fn small_edit_preserves_old_and_new_str() {
        let (diff_str, out_old, out_new) = compact_file_change_fields(
            "src/a.ts",
            None,
            Some("const a = 1;".into()),
            Some("const a = 2;".into()),
        );
        assert!(diff_str.is_none());
        assert_eq!(out_old.as_deref(), Some("const a = 1;"));
        assert_eq!(out_new.as_deref(), Some("const a = 2;"));
    }
}
