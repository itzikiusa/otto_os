//! Go Module, `go.work`, and Go Import Package Identifier Resolution.
//!
//! Priority order (mandatory for Multi-Repo Go development):
//! 1. Local `replace` target directories in `go.work` and `go.mod` (editable workspace code).
//! 2. In-workspace `go.mod` / `go.work` `use` modules across all attached project folders.
//! 3. `$GOROOT/src` standard library.
//! 4. `$GOMODCACHE` / `$GOPATH/pkg/mod` external versioned modules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) use super::go_mod_resolve::{
    discover_goroot_src_dirs, read_declared_go_package_name, resolve_go_package_dirs,
    search_go_package_dirs,
};

/// Escape uppercase letters in a Go module path per `$GOMODCACHE` layout (`GitHub` -> `!git!hub`).
pub(crate) fn escape_go_module_path(module_path: &str) -> String {
    let mut out = String::with_capacity(module_path.len() + 4);
    for ch in module_path.chars() {
        if ch.is_ascii_uppercase() {
            out.push('!');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GoModInfo {
    pub module_path: Option<String>,
    pub mod_root: PathBuf,
    /// `module_path -> version` (e.g. `"github.com/gin-gonic/gin" -> "v1.9.1"`).
    pub requires: BTreeMap<String, String>,
    /// `module_path -> local replacement dir` (canonicalized).
    pub local_replaces: BTreeMap<String, PathBuf>,
}

/// Locate and parse the nearest `go.mod` (plus any enclosing `go.work`) for `origin_file`.
#[allow(clippy::disallowed_methods)]
pub(crate) fn parse_go_mod_for_file(
    origin_file: &Path,
    project_roots: &[PathBuf],
) -> Option<GoModInfo> {
    let start_dir = if origin_file.is_dir() {
        Some(origin_file)
    } else {
        origin_file.parent()
    };
    let mut dir = start_dir;
    let mut found_mod = None;
    while let Some(d) = dir {
        let candidate = d.join("go.mod");
        if candidate.is_file() {
            found_mod = Some((d.to_path_buf(), candidate));
            break;
        }
        if project_roots.iter().any(|r| r == d) {
            break;
        }
        dir = d.parent();
    }
    if found_mod.is_none() {
        for r in project_roots {
            let candidate = r.join("go.mod");
            if candidate.is_file() {
                found_mod = Some((r.clone(), candidate));
                break;
            }
        }
    }
    let (mod_root, go_mod_file) = found_mod?;
    let mut info = parse_go_mod_file(&mod_root, &go_mod_file)?;

    // Layer any `go.work` `replace` / `use` directives (go.work overrides go.mod replaces).
    for work_file in find_go_work_files(&mod_root, project_roots) {
        if let Some(work_dir) = work_file.parent() {
            apply_go_work_replaces_and_uses(work_dir, &work_file, &mut info);
        }
    }
    Some(info)
}

#[allow(clippy::disallowed_methods)]
pub(super) fn parse_go_mod_file(mod_root: &Path, go_mod_file: &Path) -> Option<GoModInfo> {
    let text = std::fs::read_to_string(go_mod_file).ok()?;
    let mut info = GoModInfo {
        mod_root: mod_root
            .canonicalize()
            .unwrap_or_else(|_| mod_root.to_path_buf()),
        ..Default::default()
    };

    let mut in_require = false;
    let mut in_replace = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line == ")" {
            in_require = false;
            in_replace = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("module ") {
            info.module_path = Some(rest.trim().trim_matches('"').to_string());
            continue;
        }
        if line.starts_with("require") && line.ends_with('(') {
            in_require = true;
            continue;
        }
        if line.starts_with("replace") && line.ends_with('(') {
            in_replace = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ") {
            parse_go_require_spec(rest, &mut info.requires);
            continue;
        }
        if let Some(rest) = line.strip_prefix("replace ") {
            parse_go_replace_spec(rest, mod_root, &mut info.local_replaces);
            continue;
        }
        if in_require {
            parse_go_require_spec(line, &mut info.requires);
        } else if in_replace {
            parse_go_replace_spec(line, mod_root, &mut info.local_replaces);
        }
    }
    Some(info)
}

#[allow(clippy::disallowed_methods)]
fn find_go_work_files(mod_root: &Path, project_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = Some(mod_root);
    let mut depth = 0usize;
    while let Some(d) = cur {
        let gw = d.join("go.work");
        if gw.is_file() && !out.contains(&gw) {
            out.push(gw);
        }
        depth += 1;
        if depth >= 4 {
            break;
        }
        cur = d.parent();
    }
    for r in project_roots {
        for cand in [
            r.join("go.work"),
            r.parent().map(|p| p.join("go.work")).unwrap_or_default(),
        ] {
            if !cand.as_os_str().is_empty() && cand.is_file() && !out.contains(&cand) {
                out.push(cand);
            }
        }
    }
    out
}

#[allow(clippy::disallowed_methods)]
fn apply_go_work_replaces_and_uses(work_dir: &Path, work_file: &Path, info: &mut GoModInfo) {
    let Ok(text) = std::fs::read_to_string(work_file) else {
        return;
    };
    let mut in_use = false;
    let mut in_replace = false;
    let mut use_dirs = Vec::new();
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line == ")" {
            in_use = false;
            in_replace = false;
            continue;
        }
        if line.starts_with("use") && line.ends_with('(') {
            in_use = true;
            continue;
        }
        if line.starts_with("replace") && line.ends_with('(') {
            in_replace = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("use ") {
            push_go_work_use_dir(work_dir, rest.trim(), &mut use_dirs);
            continue;
        }
        if let Some(rest) = line.strip_prefix("replace ") {
            parse_go_replace_spec(rest, work_dir, &mut info.local_replaces);
            continue;
        }
        if in_use {
            push_go_work_use_dir(work_dir, line, &mut use_dirs);
        } else if in_replace {
            parse_go_replace_spec(line, work_dir, &mut info.local_replaces);
        }
    }

    for ud in use_dirs {
        if let Some(use_mod) = parse_go_mod_file(&ud, &ud.join("go.mod")) {
            if let Some(mp) = use_mod.module_path {
                info.local_replaces.entry(mp).or_insert(use_mod.mod_root);
            }
        }
    }
}

fn push_go_work_use_dir(work_dir: &Path, raw: &str, out: &mut Vec<PathBuf>) {
    let tok = raw.trim().trim_matches('"').trim_matches('\'');
    if tok.is_empty() {
        return;
    }
    let p = if Path::new(tok).is_absolute() {
        PathBuf::from(tok)
    } else {
        work_dir.join(tok)
    };
    if let Ok(c) = p.canonicalize() {
        if c.is_dir() && !out.contains(&c) {
            out.push(c);
        }
    }
}

fn parse_go_require_spec(spec: &str, out: &mut BTreeMap<String, String>) {
    let mut parts = spec.split_whitespace();
    if let (Some(m), Some(v)) = (parts.next(), parts.next()) {
        out.insert(m.trim_matches('"').to_string(), v.to_string());
    }
}

fn parse_go_replace_spec(spec: &str, base_dir: &Path, out: &mut BTreeMap<String, PathBuf>) {
    let Some((left, right)) = spec.split_once("=>") else {
        return;
    };
    let old_mod = left
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    let target = right
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .trim_matches('\'');
    if old_mod.is_empty() || target.is_empty() {
        return;
    }
    let resolved_opt = if let Some(rest) = target.strip_prefix("~/") {
        dirs::home_dir().map(|h| h.join(rest))
    } else if Path::new(target).is_absolute() {
        Some(PathBuf::from(target))
    } else if target.starts_with('.') || base_dir.join(target).join("go.mod").is_file() {
        Some(base_dir.join(target))
    } else {
        None
    };
    if let Some(resolved) = resolved_opt {
        if let Ok(canon) = resolved.canonicalize() {
            if canon.is_dir() {
                out.insert(old_mod.to_string(), canon);
            }
        }
    }
}

/// Derive valid Go package qualifier candidates from a module/package import path,
/// handling `/v2`..`/v99` semantic import versioning (`github.com/gofiber/fiber/v2` -> `fiber`),
/// `gopkg.in/yaml.v3` -> `yaml`, and `go-*` / `*-go` hyphenated repository conventions.
pub(crate) fn candidate_go_import_identifiers(import_path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let push_ident = |out: &mut Vec<String>, s: &str| {
        let clean = s.trim();
        let mut chars = clean.chars();
        let valid = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if valid && !out.iter().any(|x| x == clean) {
            out.push(clean.to_string());
        }
    };

    let segs: Vec<&str> = import_path.split('/').filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return out;
    }

    let last = segs[segs.len() - 1];
    let is_major_ver =
        last.len() >= 2 && last.starts_with('v') && last[1..].chars().all(|c| c.is_ascii_digit());
    let base_seg = if is_major_ver && segs.len() >= 2 {
        segs[segs.len() - 2]
    } else {
        last
    };

    // Strip `.vN` suffix (`gopkg.in/yaml.v3` -> `yaml`, `dd-trace-go.v1` -> `dd-trace-go`).
    let without_dot_ver = match base_seg.rsplit_once('.') {
        Some((stem, ver))
            if !stem.is_empty()
                && ver.len() >= 2
                && ver.starts_with('v')
                && ver[1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            stem
        }
        _ => base_seg,
    };

    push_ident(&mut out, without_dot_ver);
    if let Some(stripped) = without_dot_ver.strip_prefix("go-") {
        push_ident(&mut out, stripped);
    }
    if let Some(stripped) = without_dot_ver.strip_prefix("go_") {
        push_ident(&mut out, stripped);
    }
    if let Some(stripped) = without_dot_ver.strip_suffix("-go") {
        push_ident(&mut out, stripped);
        let collapsed = stripped.replace('-', "");
        push_ident(&mut out, &collapsed);
    }
    if without_dot_ver.contains('-') {
        if let Some(tail) = without_dot_ver.rsplit('-').next() {
            if tail != "go" {
                push_ident(&mut out, tail);
            }
        }
        if let Some(head) = without_dot_ver.split('-').next() {
            if head != "go" {
                push_ident(&mut out, head);
            }
        }
        let collapsed = without_dot_ver.replace('-', "");
        push_ident(&mut out, &collapsed);
    }
    if is_major_ver {
        push_ident(&mut out, last);
    }
    out
}

/// Parse `import` specs in a Go source file: `alias_or_pkg_name -> import_path`.
pub(crate) fn parse_go_imports(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut in_block = false;
    for raw in content.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("import") && line.ends_with('(') {
            in_block = true;
            continue;
        }
        if in_block {
            if line == ")" {
                in_block = false;
                continue;
            }
            insert_go_import_spec(line, &mut map);
            continue;
        }
        if let Some(rest) = line.strip_prefix("import ") {
            insert_go_import_spec(rest.trim(), &mut map);
        }
        if !in_block
            && (line.starts_with("func ") || line.starts_with("type ") || line.starts_with("var "))
        {
            break;
        }
    }
    map
}

fn insert_go_import_spec(spec: &str, out: &mut BTreeMap<String, String>) {
    let Some(q1) = spec.find('"') else {
        return;
    };
    let rest = &spec[q1 + 1..];
    let Some(q2) = rest.find('"') else {
        return;
    };
    let import_path = &rest[..q2];
    if import_path.is_empty() {
        return;
    }
    let prefix = spec[..q1].trim();
    if prefix.is_empty() || prefix == "_" || prefix == "." {
        for cand in candidate_go_import_identifiers(import_path) {
            out.entry(cand).or_insert_with(|| import_path.to_string());
        }
    } else {
        out.insert(prefix.to_string(), import_path.to_string());
    }
}

pub const MAX_GO_BUILD_TAGS: usize = 32;
pub const MAX_GO_BUILD_TAG_LEN: usize = 64;

pub fn is_valid_go_build_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= MAX_GO_BUILD_TAG_LEN
        && !tag.starts_with('-')
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// Strictly validate, split comma/whitespace tokens, and deduplicate Go build tags.
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
        // Reached `package <name>` or first non-comment statement.
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
    // Stack of boolean `negated` flags per parenthesis group.
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

/// Merge `extra_tags` cleanly into a Go CLI `args` / `build_flags` vector without
/// duplicating `-tags` (handles `-tags=a,b`, `--tags=a,b`, `-tags a,b`, `--tags a,b`,
/// and stops before `--` test-binary arg separator).
pub fn merge_go_tags_into_args(args: &mut Vec<String>, extra_tags: &[String]) {
    let mut merged: Vec<String> = Vec::new();
    let push_split = |raw: &str, dst: &mut Vec<String>| {
        for tok in raw.split(|c: char| c == ',' || c.is_whitespace()) {
            let tok = tok.trim();
            if is_valid_go_build_tag(tok) && !dst.iter().any(|x| x == tok) {
                dst.push(tok.to_string());
            }
        }
    };

    let mut cleaned = Vec::with_capacity(args.len() + 1);
    let mut insert_pos: Option<usize> = None;
    let mut i = 0usize;
    let mut after_double_dash = false;
    while i < args.len() {
        let a = &args[i];
        if !after_double_dash && a == "--" {
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
                push_split(val, &mut merged);
                i += 1;
                continue;
            }
            if a == "-tags" || a == "--tags" {
                if insert_pos.is_none() {
                    insert_pos = Some(cleaned.len());
                }
                if i + 1 < args.len() && args[i + 1] != "--" {
                    push_split(&args[i + 1], &mut merged);
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }
        }
        cleaned.push(a.clone());
        i += 1;
    }

    for tag in extra_tags {
        push_split(tag, &mut merged);
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

#[cfg(test)]
mod build_tags_tests {
    use super::{extract_go_file_build_tags, merge_go_tags_into_args, validate_go_build_tags};

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
    fn validates_and_merges_go_build_tags_cleanly() {
        let ok = validate_go_build_tags(&[
            "component, integration".into(),
            "component".into(),
            "v2.1_beta-1".into(),
        ])
        .unwrap();
        assert_eq!(ok, vec!["component", "integration", "v2.1_beta-1"]);

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
}
