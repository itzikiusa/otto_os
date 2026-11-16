//! Safe, non-executing discovery of a Claude-family custom provider's effective
//! `CLAUDE_CONFIG_DIR` (e.g. `~/.claude-work` for `claudework` -> `claude-work`).
//!
//! Resolution precedence:
//!   1. Explicit `CLAUDE_CONFIG_DIR` in the provider's `env`, or a leading
//!      `CLAUDE_CONFIG_DIR=…` assignment in `cmd` / `args`.
//!   2. Static inspection of the resolved wrapper script on `$PATH`/filesystem
//!      (regular UTF-8 file `<= 16 KiB`, no NUL bytes) or the user's shell rc
//!      alias / function body (`~/.zshrc`, etc.) for `CLAUDE_CONFIG_DIR=…` —
//!      never executing arbitrary code.
//!   3. Conventional fallback: when the program basename is `claude-<suffix>`
//!      (e.g. `claude-work`) or the provider slug is `claude-<suffix>` /
//!      `claude<suffix>` (e.g. `claudework`) and `<home>/.claude-<suffix>`
//!      exists on disk, infer `<home>/.claude-<suffix>`.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Maximum wrapper-script size inspected for `CLAUDE_CONFIG_DIR=…` (16 KiB).
const MAX_WRAPPER_BYTES: u64 = 16 * 1024;
/// Maximum shell rc file size scanned for alias/function definitions (256 KiB).
const MAX_RC_BYTES: u64 = 256 * 1024;

/// Expand `~`, `~/…`, `$HOME`, `$HOME/…`, `${HOME}`, `${HOME}/…` against `home`,
/// stripping a single pair of matching outer `'…'` or `"…"` quotes.
pub fn expand_home_path(raw: &str, home: &Path) -> Option<PathBuf> {
    let trimmed = strip_matching_quotes(raw.trim()).trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "~" || trimmed == "$HOME" || trimmed == "${HOME}" {
        return Some(home.to_path_buf());
    }
    for prefix in ["~/", "$HOME/", "${HOME}/"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim_start_matches('/');
            return Some(if rest.is_empty() {
                home.to_path_buf()
            } else {
                home.join(rest)
            });
        }
    }
    Some(PathBuf::from(trimmed))
}

/// Tokenize a shell-like command string without executing anything. Supports
/// single quotes, double quotes, and backslash escapes outside single quotes.
pub fn split_command_tokens(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars().peekable();

    while let Some(c) = chars.next() {
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            Some('"') => {
                if c == '"' {
                    quote = None;
                } else if c == '\\' {
                    if let Some(&next) = chars.peek() {
                        if matches!(next, '"' | '\\' | '$' | '`') {
                            cur.push(chars.next().unwrap_or(next));
                        } else {
                            cur.push(c);
                        }
                    } else {
                        cur.push(c);
                    }
                } else {
                    cur.push(c);
                }
            }
            Some(_) => unreachable!(),
            None => match c {
                '\'' | '"' => {
                    in_token = true;
                    quote = Some(c);
                }
                '\\' => {
                    in_token = true;
                    if let Some(next) = chars.next() {
                        cur.push(next);
                    }
                }
                c if c.is_whitespace() => {
                    if in_token {
                        tokens.push(std::mem::take(&mut cur));
                        in_token = false;
                    }
                }
                _ => {
                    in_token = true;
                    cur.push(c);
                }
            },
        }
    }
    if in_token {
        tokens.push(cur);
    }
    tokens
}

/// Parse `KEY=VALUE` when `KEY` is a valid POSIX environment variable identifier.
pub fn parse_env_assignment(token: &str) -> Option<(&str, &str)> {
    let (key, val) = token.split_once('=')?;
    let mut chars = key.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((key, val))
}

/// Resolve a Claude-family provider's effective `CLAUDE_CONFIG_DIR` using the
/// process `$PATH` and standard shell rc files under `home` / `$ZDOTDIR`.
pub fn resolve_claude_config_dir<'a, I>(
    slug: &str,
    cmd: &str,
    args: &[String],
    env: I,
    home: &Path,
) -> Option<PathBuf>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let path_dirs = default_search_path_dirs(home);
    let rc_files = default_rc_files(home);
    resolve_claude_config_dir_with(slug, cmd, args, env, home, &path_dirs, &rc_files)
}

/// Testable core of [`resolve_claude_config_dir`] with injected `PATH`
/// directories and shell rc file paths.
pub fn resolve_claude_config_dir_with<'a, I>(
    slug: &str,
    cmd: &str,
    args: &[String],
    env: I,
    home: &Path,
    path_dirs: &[PathBuf],
    rc_files: &[PathBuf],
) -> Option<PathBuf>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    // 1a. Explicit `env.get("CLAUDE_CONFIG_DIR")`.
    for (k, v) in env {
        if k == "CLAUDE_CONFIG_DIR" {
            if let Some(dir) = expand_home_path(v, home) {
                return Some(dir);
            }
        }
    }

    // 1b. Leading `CLAUDE_CONFIG_DIR=…` in `cmd` or `args` (allowing `env …`).
    let cmd_tokens = split_command_tokens(cmd);
    let mut program: Option<String> = None;
    for tok in cmd_tokens.iter().chain(args.iter()) {
        if program.is_none() && tok == "env" {
            continue;
        }
        if program.is_none() {
            if let Some((k, v)) = parse_env_assignment(tok) {
                if k == "CLAUDE_CONFIG_DIR" {
                    if let Some(dir) = expand_home_path(v, home) {
                        return Some(dir);
                    }
                }
                continue;
            }
            program = Some(tok.clone());
            break;
        }
    }

    let raw_prog = program.unwrap_or_default();
    let expanded_prog = if raw_prog.starts_with("~/") || raw_prog == "~" {
        expand_home_path(&raw_prog, home)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| raw_prog.clone())
    } else {
        raw_prog.clone()
    };

    // 2a. Small UTF-8 wrapper script (`<= 16 KiB`) on filesystem or `PATH`.
    if !expanded_prog.is_empty()
        && !expanded_prog.contains("{cwd}")
        && !expanded_prog.contains("{sid}")
    {
        if let Some(exec_path) = locate_program_file(&expanded_prog, cmd.trim(), home, path_dirs) {
            if let Some(script) = read_small_utf8_script(&exec_path) {
                if let Some(dir) = extract_claude_config_dir_from_script(&script, home) {
                    return Some(dir);
                }
            }
        }

        // 2b. Shell alias or function definition in rc files (`~/.zshrc`, …).
        if !expanded_prog.contains('/') {
            for rc in rc_files {
                if let Some(text) = read_capped_utf8(rc, MAX_RC_BYTES) {
                    if let Some(dir) =
                        extract_claude_config_dir_from_rc(&text, &expanded_prog, home)
                    {
                        return Some(dir);
                    }
                }
            }
        }
    }

    // 3. Conventional `<home>/.claude-<suffix>` fallback when it exists on disk.
    infer_conventional_claude_dir(slug, &expanded_prog, home)
}

fn default_search_path_dirs(home: &Path) -> Vec<PathBuf> {
    let real_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let isolated_home = real_home
        .as_deref()
        .is_some_and(|rh| !rh.as_os_str().is_empty() && rh != home);
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|d| {
                    !isolated_home || !real_home.as_deref().is_some_and(|rh| d.starts_with(rh))
                })
                .collect()
        })
        .unwrap_or_default();
    for extra in [
        home.join(".local").join("bin"),
        home.join("bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ] {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}

fn default_rc_files(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(11);
    let same_as_process_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .is_none_or(|rh| Path::new(&rh) == home);
    if same_as_process_home {
        if let Some(zdot) = std::env::var_os("ZDOTDIR").filter(|v| !v.is_empty()) {
            let zdir = PathBuf::from(zdot);
            for name in [".zshrc", ".zshenv", ".zprofile"] {
                out.push(zdir.join(name));
            }
        }
    }
    for name in [
        ".zshrc",
        ".zshenv",
        ".zprofile",
        ".bashrc",
        ".bash_profile",
        ".bash_aliases",
        ".aliases",
        ".profile",
    ] {
        let p = home.join(name);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

fn locate_program_file(
    expanded_prog: &str,
    raw_cmd: &str,
    home: &Path,
    path_dirs: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(whole) = expand_home_path(raw_cmd, home).filter(|p| p.is_file()) {
        return Some(whole);
    }
    let p = Path::new(expanded_prog);
    if expanded_prog.contains('/') {
        return p.is_file().then(|| p.to_path_buf());
    }
    path_dirs
        .iter()
        .map(|d| d.join(expanded_prog))
        .find(|cand| cand.is_file())
}

fn read_small_utf8_script(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_WRAPPER_BYTES {
        return None;
    }
    read_capped_utf8(path, MAX_WRAPPER_BYTES)
}

fn read_capped_utf8(path: &Path, max_bytes: u64) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(max_bytes).read_to_end(&mut bytes).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn strip_matching_quotes(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Extract `CLAUDE_CONFIG_DIR` from a shell snippet or wrapper script without
/// executing anything. Understands:
/// - `export CLAUDE_CONFIG_DIR=...`
/// - `CLAUDE_CONFIG_DIR=... exec claude "$@"`
/// - `CLAUDE_CONFIG_DIR="${CLAUDE_CONFIG_DIR:-$HOME/.claude-work}"`
/// - `: "${CLAUDE_CONFIG_DIR:=$HOME/.claude-work}"`
pub fn extract_claude_config_dir_from_script(script: &str, home: &Path) -> Option<PathBuf> {
    for raw_line in script.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(dir) = extract_claude_config_dir_from_statement(line, home) {
            return Some(dir);
        }
    }
    None
}

fn extract_claude_config_dir_from_statement(stmt: &str, home: &Path) -> Option<PathBuf> {
    for seg in stmt.split([';', '&', '|']) {
        let seg = seg.trim();
        if seg.is_empty() || seg.starts_with('#') {
            continue;
        }
        for tok in split_command_tokens(seg) {
            if tok == "export" || tok == "env" || tok == ":" {
                continue;
            }
            if let Some((k, v)) = parse_env_assignment(&tok) {
                if k == "CLAUDE_CONFIG_DIR" {
                    if let Some(dir) = clean_config_value(v, home) {
                        return Some(dir);
                    }
                }
                continue;
            }
            if let Some(inner) = tok
                .strip_prefix("${CLAUDE_CONFIG_DIR:-")
                .or_else(|| tok.strip_prefix("${CLAUDE_CONFIG_DIR:="))
                .and_then(|s| s.strip_suffix('}'))
            {
                if let Some(dir) = clean_config_value(inner, home) {
                    return Some(dir);
                }
            }
            break;
        }
    }
    None
}

fn clean_config_value(raw: &str, home: &Path) -> Option<PathBuf> {
    let trimmed = strip_matching_quotes(raw.trim()).trim();
    if let Some(inner) = trimmed
        .strip_prefix("${CLAUDE_CONFIG_DIR:-")
        .or_else(|| trimmed.strip_prefix("${CLAUDE_CONFIG_DIR:="))
        .and_then(|s| s.strip_suffix('}'))
    {
        return expand_home_path(inner, home);
    }
    // Reject unexpanded dynamic parameter expressions like `$1` or `$(...)`.
    let candidate = trimmed
        .strip_prefix("$HOME")
        .or_else(|| trimmed.strip_prefix("${HOME}"))
        .unwrap_or(trimmed);
    if candidate.contains('$') || candidate.contains('`') {
        return None;
    }
    expand_home_path(trimmed, home)
}

/// Extract `CLAUDE_CONFIG_DIR` from an `alias <program>=…` or `<program>()`
/// function body in shell rc content.
pub fn extract_claude_config_dir_from_rc(
    rc_content: &str,
    program: &str,
    home: &Path,
) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let mut in_target_fn = false;
    let mut brace_depth: i32 = 0;

    for raw_line in rc_content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if in_target_fn {
            if let Some(dir) = extract_claude_config_dir_from_statement(line, home) {
                return Some(dir);
            }
            brace_depth += count_braces(line);
            if brace_depth <= 0 {
                in_target_fn = false;
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("alias ") {
            let rest = rest.trim_start();
            for prefix in [
                format!("{program}="),
                format!("'{program}'="),
                format!("\"{program}\"="),
            ] {
                if let Some(rhs) = rest.strip_prefix(&prefix) {
                    let body = strip_matching_quotes(rhs.trim());
                    if let Some(dir) = extract_claude_config_dir_from_statement(body, home) {
                        return Some(dir);
                    }
                }
            }
        }

        let fn_body_after = line
            .strip_prefix("function ")
            .map(str::trim_start)
            .and_then(|s| s.strip_prefix(program))
            .filter(|after| {
                after.is_empty()
                    || after.starts_with(char::is_whitespace)
                    || after.starts_with('(')
                    || after.starts_with('{')
            })
            .or_else(|| {
                line.strip_prefix(program)
                    .map(str::trim_start)
                    .filter(|after| after.starts_with("()"))
            });

        if let Some(after) = fn_body_after {
            let body = after
                .trim_start_matches("()")
                .trim_start()
                .trim_start_matches('{')
                .trim_end_matches('}');
            if let Some(dir) = extract_claude_config_dir_from_statement(body, home) {
                return Some(dir);
            }
            brace_depth = count_braces(line);
            if brace_depth > 0 || !line.contains('}') {
                in_target_fn = true;
                if brace_depth == 0 {
                    brace_depth = 1;
                }
            }
        }
    }
    None
}

fn count_braces(line: &str) -> i32 {
    let mut delta = 0i32;
    for b in line.bytes() {
        if b == b'{' {
            delta += 1;
        } else if b == b'}' {
            delta -= 1;
        }
    }
    delta
}

fn infer_conventional_claude_dir(slug: &str, program: &str, home: &Path) -> Option<PathBuf> {
    let prog_base = Path::new(program)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let slug_lc = slug.trim().to_ascii_lowercase();

    for candidate in [prog_base.as_str(), slug_lc.as_str()] {
        if candidate.is_empty() || candidate == "claude" {
            continue;
        }
        if let Some(suffix) = candidate
            .strip_prefix("claude-")
            .or_else(|| candidate.strip_prefix("claude_"))
            .filter(|s| !s.is_empty())
        {
            let dir = home.join(format!(".claude-{suffix}"));
            if dir.is_dir() {
                return Some(dir);
            }
        } else if let Some(suffix) = candidate.strip_prefix("claude").filter(|s| !s.is_empty()) {
            let dashed = home.join(format!(".claude-{suffix}"));
            if dashed.is_dir() {
                return Some(dashed);
            }
            let flat = home.join(format!(".claude{suffix}"));
            if flat.is_dir() {
                return Some(flat);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_explicit_env_and_inline_cmd_assignments() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        let from_env = resolve_claude_config_dir_with(
            "claudework",
            "claude",
            &[],
            [("CLAUDE_CONFIG_DIR", "~/.claude-work")],
            home,
            &[],
            &[],
        );
        assert_eq!(from_env, Some(home.join(".claude-work")));

        let from_cmd = resolve_claude_config_dir_with(
            "claudework",
            "CLAUDE_CONFIG_DIR=\"$HOME/.claude-team\" claude --dangerously-skip-permissions",
            &[],
            std::iter::empty(),
            home,
            &[],
            &[],
        );
        assert_eq!(from_cmd, Some(home.join(".claude-team")));
    }

    #[test]
    fn resolves_wrapper_script_and_zshrc_alias_without_executing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&bin).unwrap();

        let wrapper = bin.join("claude-work");
        std::fs::write(
            &wrapper,
            "#!/usr/bin/env bash\n# wrapper\nexport CLAUDE_CONFIG_DIR=\"$HOME/.claude-work\"\nexec claude \"$@\"\n",
        )
        .unwrap();

        let from_wrapper = resolve_claude_config_dir_with(
            "claudework",
            "claude-work --dangerously-skip-permissions",
            &[],
            std::iter::empty(),
            &home,
            std::slice::from_ref(&bin),
            &[],
        );
        assert_eq!(from_wrapper, Some(home.join(".claude-work")));

        let zshrc = home.join(".zshrc");
        std::fs::write(
            &zshrc,
            "alias claude-alias='CLAUDE_CONFIG_DIR=~/.claude-alias claude'\nclaude-fn() {\n  export CLAUDE_CONFIG_DIR=\"${HOME}/.claude-fn\"\n  claude \"$@\"\n}\n",
        )
        .unwrap();

        assert_eq!(
            resolve_claude_config_dir_with(
                "claudealias",
                "claude-alias --dangerously-skip-permissions",
                &[],
                std::iter::empty(),
                &home,
                &[],
                std::slice::from_ref(&zshrc),
            ),
            Some(home.join(".claude-alias"))
        );
        assert_eq!(
            resolve_claude_config_dir_with(
                "claudefn",
                "claude-fn",
                &[],
                std::iter::empty(),
                &home,
                &[],
                std::slice::from_ref(&zshrc),
            ),
            Some(home.join(".claude-fn"))
        );
    }

    #[test]
    fn conventional_fallback_matches_existing_dir_and_ignores_builtin_or_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".claude-work")).unwrap();

        assert_eq!(
            resolve_claude_config_dir_with(
                "claudework",
                "claude-work --dangerously-skip-permissions",
                &[],
                std::iter::empty(),
                home,
                &[],
                &[],
            ),
            Some(home.join(".claude-work"))
        );
        assert_eq!(
            resolve_claude_config_dir_with(
                "claudework",
                "claude",
                &[],
                std::iter::empty(),
                home,
                &[],
                &[],
            ),
            Some(home.join(".claude-work"))
        );
        // Built-in `claude` never guesses `.claude-work`, and missing `.claude-sub` returns None.
        assert_eq!(
            resolve_claude_config_dir_with(
                "claude",
                "claude",
                &[],
                std::iter::empty(),
                home,
                &[],
                &[],
            ),
            None
        );
        assert_eq!(
            resolve_claude_config_dir_with(
                "claude-sub",
                "claude-sub",
                &[],
                std::iter::empty(),
                home,
                &[],
                &[],
            ),
            None
        );
    }
}
