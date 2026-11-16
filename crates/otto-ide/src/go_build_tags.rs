//! Shared Go build-tag validation, canonicalization, source-header constraint
//! extraction (`//go:build` & `// +build`), `gopls` diagnostic detection, and
//! `--`-aware CLI `-tags` flag stripping/merging.

pub const MAX_GO_BUILD_TAGS: usize = 32;
pub const MAX_GO_BUILD_TAG_LEN: usize = 64;

/// Returns `true` when `tag` is `1..=64` ASCII `[A-Za-z0-9_.-]` characters and
/// does not begin with `'-'` (preventing CLI flag injection).
pub fn is_valid_go_build_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= MAX_GO_BUILD_TAG_LEN
        && !tag.starts_with('-')
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// Strictly validate, split comma/whitespace tokens, and deduplicate Go build tags
/// preserving first-seen order.
pub fn validate_go_build_tags(raw: &[String]) -> otto_core::Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for entry in raw {
        for tok in entry.split(|c: char| c == ',' || c.is_whitespace()) {
            let tok = tok.trim();
            if tok.is_empty() {
                continue;
            }
            if !is_valid_go_build_tag(tok) {
                return Err(otto_core::Error::Invalid(format!(
                    "invalid Go build tag '{tok}' (expected 1..={MAX_GO_BUILD_TAG_LEN} chars of [A-Za-z0-9_.-], never starting with '-')"
                )));
            }
            if !out.iter().any(|existing| existing == tok) {
                if out.len() >= MAX_GO_BUILD_TAGS {
                    return Err(otto_core::Error::Invalid(format!(
                        "too many Go build tags (maximum {MAX_GO_BUILD_TAGS})"
                    )));
                }
                out.push(tok.to_string());
            }
        }
    }
    Ok(out)
}

/// Normalize Go build tags into a canonical sorted, deduplicated vector suitable
/// for pool keying (`(root, sorted_tags)`). Invalid tokens are dropped.
pub fn canonicalize_go_build_tags(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in raw {
        for tok in entry.split(|c: char| c == ',' || c.is_whitespace()) {
            let tok = tok.trim();
            if is_valid_go_build_tag(tok)
                && !out.iter().any(|existing| existing == tok)
                && out.len() < MAX_GO_BUILD_TAGS
            {
                out.push(tok.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Match `gopls` diagnostics indicating a file is excluded by active build tags.
pub fn is_gopls_build_tag_exclusion_msg(message: &str) -> bool {
    message.contains("No packages found for open file")
        || message.contains("excluded due to its build tags")
        || message.contains("build constraints exclude all Go files")
        || message.contains("no buildable Go source files")
}

fn is_standard_go_constraint_token(tok: &str) -> bool {
    if let Some(rest) = tok.strip_prefix("go1.") {
        if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
            return true;
        }
    }
    matches!(
        tok,
        "linux"
            | "darwin"
            | "windows"
            | "freebsd"
            | "openbsd"
            | "netbsd"
            | "solaris"
            | "plan9"
            | "aix"
            | "zos"
            | "js"
            | "wasip1"
            | "unix"
            | "android"
            | "ios"
            | "illumos"
            | "dragonfly"
            | "hurd"
            | "nacl"
            | "386"
            | "amd64"
            | "amd64p32"
            | "arm"
            | "armbe"
            | "arm64"
            | "arm64be"
            | "loong64"
            | "ppc"
            | "ppc64"
            | "ppc64le"
            | "mips"
            | "mipsle"
            | "mips64"
            | "mips64le"
            | "mips64p32"
            | "mips64p32le"
            | "riscv"
            | "riscv64"
            | "s390"
            | "s390x"
            | "sparc"
            | "sparc64"
            | "wasm"
            | "gc"
            | "gccgo"
            | "cgo"
            | "ignore"
    )
}

/// Extract positive non-OS/arch Go build tags from `//go:build` and `// +build`
/// header comments appearing before `package <name>`.
pub fn extract_go_file_build_tags(source: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    let mut in_block_comment = false;

    for raw_line in source.lines() {
        let mut line = raw_line.trim();
        if in_block_comment {
            if let Some(end_idx) = line.find("*/") {
                in_block_comment = false;
                line = line[end_idx + 2..].trim();
            } else {
                continue;
            }
        }
        while let Some(start_idx) = line.find("/*") {
            if start_idx > 0 {
                break;
            }
            if let Some(end_idx) = line[2..].find("*/") {
                line = line[2 + end_idx + 2..].trim();
            } else {
                in_block_comment = true;
                line = "";
                break;
            }
        }
        if in_block_comment || line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("//") {
            let trimmed = rest.trim();
            if let Some(expr) = rest
                .strip_prefix("go:build")
                .filter(|s| s.starts_with(char::is_whitespace))
            {
                collect_go_build_expr_positive_tags(expr.trim(), &mut tags);
            } else if let Some(expr) = trimmed
                .strip_prefix("+build")
                .filter(|s| s.starts_with(char::is_whitespace))
            {
                collect_plus_build_positive_tags(expr.trim(), &mut tags);
            }
            continue;
        }
        break;
    }
    tags
}

fn push_positive_go_tag(tok: &str, out: &mut Vec<String>) {
    if is_valid_go_build_tag(tok)
        && !is_standard_go_constraint_token(tok)
        && !out.iter().any(|t| t == tok)
        && out.len() < MAX_GO_BUILD_TAGS
    {
        out.push(tok.to_string());
    }
}

fn collect_go_build_expr_positive_tags(expr: &str, out: &mut Vec<String>) {
    let bytes = expr.as_bytes();
    let mut i = 0usize;
    let mut group_neg_stack: Vec<bool> = vec![false];
    let mut next_neg = false;

    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'!' {
            next_neg = !next_neg;
            i += 1;
            continue;
        }
        if b == b'(' {
            let parent_neg = *group_neg_stack.last().unwrap_or(&false);
            group_neg_stack.push(parent_neg ^ next_neg);
            next_neg = false;
            i += 1;
            continue;
        }
        if b == b')' {
            if group_neg_stack.len() > 1 {
                group_neg_stack.pop();
            }
            next_neg = false;
            i += 1;
            continue;
        }
        if b == b'&' || b == b'|' {
            next_neg = false;
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'.' | b'-'))
        {
            i += 1;
        }
        if i > start {
            let tok = &expr[start..i];
            let effective_neg = *group_neg_stack.last().unwrap_or(&false) ^ next_neg;
            if !effective_neg {
                push_positive_go_tag(tok, out);
            }
            next_neg = false;
        } else {
            next_neg = false;
            i += 1;
        }
    }
}

fn collect_plus_build_positive_tags(expr: &str, out: &mut Vec<String>) {
    for or_clause in expr.split_whitespace() {
        for term in or_clause.split(',') {
            let term = term.trim();
            if term.is_empty() || term.starts_with('!') {
                continue;
            }
            push_positive_go_tag(term, out);
        }
    }
}

fn push_valid_split_tags(raw: &str, dst: &mut Vec<String>) {
    for tok in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = tok.trim();
        if is_valid_go_build_tag(tok)
            && !dst.iter().any(|x| x == tok)
            && dst.len() < MAX_GO_BUILD_TAGS
        {
            dst.push(tok.to_string());
        }
    }
}

/// Strip `-tags` / `--tags` / `-tags=...` / `--tags=...` flags from `args`.
///
/// When `stop_at_double_dash` is `true`, scanning stops at the first `"--"`
/// separator so test-binary arguments after `"--"` are never touched or consumed
/// as the value of a preceding `-tags` flag.
///
/// Returns `(cleaned_args, extracted_valid_tags, first_tags_insert_pos)`.
pub fn strip_go_tags_from_args_with_pos(
    args: &[String],
    stop_at_double_dash: bool,
) -> (Vec<String>, Vec<String>, Option<usize>) {
    let mut cleaned = Vec::with_capacity(args.len());
    let mut extracted = Vec::new();
    let mut insert_pos: Option<usize> = None;
    let mut i = 0usize;
    let mut after_double_dash = false;

    while i < args.len() {
        let a = &args[i];
        if stop_at_double_dash && !after_double_dash && a == "--" {
            after_double_dash = true;
            cleaned.push(a.clone());
            i += 1;
            continue;
        }
        if !after_double_dash {
            if let Some(val) = a
                .strip_prefix("-tags=")
                .or_else(|| a.strip_prefix("--tags="))
            {
                if insert_pos.is_none() {
                    insert_pos = Some(cleaned.len());
                }
                push_valid_split_tags(val, &mut extracted);
                i += 1;
                continue;
            }
            if a == "-tags" || a == "--tags" {
                if insert_pos.is_none() {
                    insert_pos = Some(cleaned.len());
                }
                if let Some(next) = args.get(i + 1) {
                    if !(stop_at_double_dash && next == "--") && !next.starts_with('-') {
                        push_valid_split_tags(next, &mut extracted);
                        i += 2;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
        }
        cleaned.push(a.clone());
        i += 1;
    }

    (cleaned, extracted, insert_pos)
}

/// Strip `-tags` / `--tags` flags from `args`, stopping at `"--"` when
/// `stop_at_double_dash` is `true`.
pub fn strip_go_tags_from_args(
    args: &[String],
    stop_at_double_dash: bool,
) -> (Vec<String>, Vec<String>) {
    let (cleaned, extracted, _) = strip_go_tags_from_args_with_pos(args, stop_at_double_dash);
    (cleaned, extracted)
}

/// Merge `extra_tags` cleanly into a Go CLI `args` / `build_flags` vector without
/// duplicating `-tags` (handles `-tags=a,b`, `--tags=a,b`, `-tags a,b`, `--tags a,b`,
/// and stops before `--` test-binary arg separator).
pub fn merge_go_tags_into_args(args: &mut Vec<String>, extra_tags: &[String]) {
    let (mut cleaned, mut merged, insert_pos) = strip_go_tags_from_args_with_pos(args, true);
    for tag in extra_tags {
        push_valid_split_tags(tag, &mut merged);
    }

    if merged.is_empty() {
        if insert_pos.is_some() {
            *args = cleaned;
        }
        return;
    }

    let flag = format!("-tags={}", merged.join(","));
    let pos = insert_pos.unwrap_or_else(|| {
        cleaned
            .iter()
            .position(|x| x == "--")
            .unwrap_or(cleaned.len())
    });
    cleaned.insert(pos, flag);
    *args = cleaned;
}

/// Unify Go `-tags` across `build_flags`, `args` (stopping before `--`), and
/// `extra_tags`, placing a single deduplicated `-tags=...` flag in `build_flags`.
pub fn unify_go_build_flags_and_args(
    build_flags: Vec<String>,
    args: Vec<String>,
    extra_tags: &[String],
) -> (Vec<String>, Vec<String>) {
    let (mut clean_flags, mut merged_tags) = strip_go_tags_from_args(&build_flags, true);
    let (clean_args, arg_tags) = strip_go_tags_from_args(&args, true);
    for t in arg_tags.iter().chain(extra_tags.iter()) {
        push_valid_split_tags(t, &mut merged_tags);
    }
    merge_go_tags_into_args(&mut clean_flags, &merged_tags);
    (clean_flags, clean_args)
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_go_build_tags, extract_go_file_build_tags, merge_go_tags_into_args,
        strip_go_tags_from_args, unify_go_build_flags_and_args, validate_go_build_tags,
    };

    #[test]
    fn extracts_go_build_and_plus_build_tags_skipping_negations_and_std_os_arch() {
        let src1 = "//go:build component\n\npackage tests\n\nfunc TestAssign(t *testing.T) {}\n";
        assert_eq!(extract_go_file_build_tags(src1), vec!["component"]);

        let src2 = "//go:build (integration || e2e) && !race && linux && amd64 && !(slow || flaky) && go1.22\n// +build integration,!race component\n\npackage foo\n//go:build ignored_after_package\n";
        assert_eq!(
            extract_go_file_build_tags(src2),
            vec!["integration", "e2e", "component"]
        );
    }

    #[test]
    fn validates_canonicalizes_and_merges_go_build_tags_cleanly() {
        let ok = validate_go_build_tags(&[
            "component, integration".into(),
            "component".into(),
            "v2.1_beta-1".into(),
        ])
        .unwrap();
        assert_eq!(ok, vec!["component", "integration", "v2.1_beta-1"]);
        assert_eq!(
            canonicalize_go_build_tags(&["integration".into(), "component".into()]),
            vec!["component", "integration"]
        );

        assert!(validate_go_build_tags(&["-exec=evil".into()]).is_err());
        assert!(validate_go_build_tags(&["bad/tag".into()]).is_err());

        let mut args = vec![
            "test".into(),
            "-v".into(),
            "-tags".into(),
            "integration".into(),
            "./...".into(),
        ];
        merge_go_tags_into_args(&mut args, &["component".into(), "integration".into()]);
        assert_eq!(
            args,
            vec!["test", "-v", "-tags=integration,component", "./..."]
        );
    }

    #[test]
    fn strip_and_unify_preserve_double_dash_separator_and_post_separator_flags() {
        let raw_args = vec![
            "-v".to_string(),
            "-tags".to_string(),
            "--".to_string(),
            "-tags=keep_for_test_bin".to_string(),
            "-test.v".to_string(),
        ];
        let (clean_args, extracted) = strip_go_tags_from_args(&raw_args, true);
        assert!(extracted.is_empty());
        assert_eq!(
            clean_args,
            vec!["-v", "--", "-tags=keep_for_test_bin", "-test.v"]
        );

        let (flags, args) = unify_go_build_flags_and_args(
            vec!["-race".into(), "-tags=integration".into()],
            raw_args,
            &["component".into()],
        );
        assert_eq!(flags, vec!["-race", "-tags=integration,component"]);
        assert_eq!(args, vec!["-v", "--", "-tags=keep_for_test_bin", "-test.v"]);
    }
}
