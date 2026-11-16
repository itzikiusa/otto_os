//! Document formatting helpers (`goimports`/`gofmt`, `rustfmt`, `prettier`,
//! `ruff`/`black`, `google-java-format`, `shfmt`, `buf`) and deterministic
//! fallback formatters (including RFC 4180 multiline CSV/TSV formatting).

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use super::{code_mask_for_line, symbols_data};
use crate::target::ResolvedTarget;
use crate::IdeCtx;

pub(super) fn formatter_candidates(
    lang: &str,
    path: &str,
    organize_imports: bool,
    caller_is_root: bool,
) -> Vec<(&'static str, String, Vec<String>)> {
    let _ = organize_imports;
    match lang {
        "go" => vec![
            ("goimports", "goimports".into(), vec![]),
            ("gofmt", "gofmt".into(), vec![]),
        ],
        "rust" => vec![(
            "rustfmt",
            "rustfmt".into(),
            vec!["--edition".into(), "2021".into()],
        )],
        "python" => vec![
            (
                "ruff",
                "ruff".into(),
                vec![
                    "format".into(),
                    "--stdin-filename".into(),
                    path.to_string(),
                    "-".into(),
                ],
            ),
            ("black", "black".into(), vec!["-q".into(), "-".into()]),
        ],
        "java" => vec![(
            "google-java-format",
            "google-java-format".into(),
            vec!["-".into()],
        )],
        "shell" | "bash" | "sh" | "zsh" => vec![(
            "shfmt",
            "shfmt".into(),
            vec!["-i".into(), "2".into(), "-ci".into(), "-".into()],
        )],
        "javascript" | "typescript" | "json" | "jsonc" | "css" | "scss" | "html" | "markdown"
        | "yaml" | "graphql" | "svelte" => {
            let mut prettier_args = Vec::new();
            if !caller_is_root {
                prettier_args.push("--no-config".into());
            }
            prettier_args.extend(["--stdin-filepath".into(), path.to_string()]);
            vec![
                ("prettier", "prettier".into(), prettier_args),
                (
                    "biome",
                    "biome".into(),
                    vec![
                        "format".into(),
                        "--stdin-file-path".into(),
                        path.to_string(),
                    ],
                ),
            ]
        }
        "protobuf" => vec![("buf", "buf".into(), vec!["format".into(), "-".into()])],
        _ => Vec::new(),
    }
}

pub(super) fn try_native_data_format(lang: &str, path: &str, content: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    if lang == "jsonl" || lower.ends_with(".jsonl") || lower.ends_with(".ndjson") {
        let mut out = Vec::new();
        for raw in content.lines() {
            let t = raw.trim();
            if t.is_empty() {
                continue;
            }
            let val: serde_json::Value = serde_json::from_str(t).ok()?;
            out.push(serde_json::to_string(&val).ok()?);
        }
        return Some(if out.is_empty() {
            String::new()
        } else {
            format!("{}\n", out.join("\n"))
        });
    }
    if lang == "json" && !lower.ends_with(".jsonc") {
        let val: serde_json::Value = serde_json::from_str(content).ok()?;
        let pretty = serde_json::to_string_pretty(&val).ok()?;
        return Some(format!("{pretty}\n"));
    }
    if matches!(lang, "csv" | "tsv") {
        let first = content.lines().find(|l| !l.trim().is_empty())?;
        let delim = symbols_data::detect_csv_delimiter(path, lang, first);
        let parsed_rows = parse_rfc4180_records(content, delim);
        let mut rows = Vec::with_capacity(parsed_rows.len());
        let delim_str = delim.to_string();
        for raw_cells in parsed_rows {
            let cells: Vec<String> = raw_cells
                .into_iter()
                .map(|(raw_cell, was_quoted)| {
                    let cell = if was_quoted {
                        raw_cell
                    } else {
                        raw_cell.trim().to_string()
                    };
                    let needs_quotes = cell.contains(delim)
                        || cell.contains('"')
                        || cell.contains('\n')
                        || cell.contains('\r')
                        || (was_quoted && cell.trim() != cell);
                    if needs_quotes {
                        format!("\"{}\"", cell.replace('"', "\"\""))
                    } else {
                        cell
                    }
                })
                .collect();
            rows.push(cells.join(&delim_str));
        }
        return Some(if rows.is_empty() {
            String::new()
        } else {
            format!("{}\n", rows.join("\n"))
        });
    }
    None
}

/// Quote-aware RFC 4180 record scanner that preserves `\r` and `\n` inside quoted
/// fields within a single record and distinguishes quoted vs unquoted cells.
fn parse_rfc4180_records(content: &str, delim: char) -> Vec<Vec<(String, bool)>> {
    let mut rows: Vec<Vec<(String, bool)>> = Vec::new();
    let mut cur_row: Vec<(String, bool)> = Vec::new();
    let mut cur_cell = String::new();
    let mut cell_was_quoted = false;
    let mut in_quotes = false;
    let mut chars = content.chars().peekable();

    let push_row_if_non_blank = |rows: &mut Vec<Vec<(String, bool)>>, row: Vec<(String, bool)>| {
        if row.len() == 1 && !row[0].1 && row[0].0.trim().is_empty() {
            return;
        }
        if !row.is_empty() {
            rows.push(row);
        }
    };

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    let _ = chars.next();
                    cur_cell.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cur_cell.push(ch);
            }
            continue;
        }

        if ch == '"' && cur_cell.trim().is_empty() && !cell_was_quoted {
            cur_cell.clear();
            in_quotes = true;
            cell_was_quoted = true;
        } else if ch == delim {
            cur_row.push((std::mem::take(&mut cur_cell), cell_was_quoted));
            cell_was_quoted = false;
        } else if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            cur_row.push((std::mem::take(&mut cur_cell), cell_was_quoted));
            cell_was_quoted = false;
            push_row_if_non_blank(&mut rows, std::mem::take(&mut cur_row));
        } else if ch == '\n' {
            cur_row.push((std::mem::take(&mut cur_cell), cell_was_quoted));
            cell_was_quoted = false;
            push_row_if_non_blank(&mut rows, std::mem::take(&mut cur_row));
        } else {
            cur_cell.push(ch);
        }
    }

    if !cur_row.is_empty() || cell_was_quoted || !cur_cell.trim().is_empty() {
        cur_row.push((cur_cell, cell_was_quoted));
        push_row_if_non_blank(&mut rows, cur_row);
    }

    rows
}

pub(super) async fn run_stdin_formatter<S: IdeCtx>(
    ctx: &S,
    resolved: &ResolvedTarget,
    cmd: &str,
    args: &[String],
    env_map: &BTreeMap<String, String>,
    input: &str,
) -> Option<String> {
    let cwd = match resolved {
        ResolvedTarget::Local { root, .. } => Some(root.to_string_lossy().into_owned()),
        ResolvedTarget::Ssh { root, .. } => Some(root.clone()),
    };
    let mut child = ctx
        .ide()
        .targets
        .spawn_piped(resolved, cmd, args, cwd.as_deref(), env_map)
        .await
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(input.as_bytes()).await.is_err() {
            let _ = child.kill().await;
            return None;
        }
        drop(stdin);
    }
    let out = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8(out.stdout).ok())
        .flatten()
}

pub(super) fn organize_imports_if_requested(lang: &str, content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if let Some((_, organized)) =
        super::code_actions::build_organize_imports_action(lang, &lines, content)
    {
        organized
    } else {
        content.to_string()
    }
}

pub fn format_builtin_fallback(lang: &str, content: &str) -> String {
    format_builtin_fallback_opts(lang, content, false)
}

pub fn format_builtin_fallback_opts(lang: &str, content: &str, organize_imports: bool) -> String {
    let formatted = format_builtin_fallback_inner(lang, content);
    if organize_imports {
        organize_imports_if_requested(lang, &formatted)
    } else {
        formatted
    }
}

fn format_builtin_fallback_inner(lang: &str, content: &str) -> String {
    if matches!(lang, "json" | "jsonl" | "csv" | "tsv") {
        let fake_path = format!("doc.{lang}");
        if let Some(formatted) = try_native_data_format(lang, &fake_path, content) {
            return formatted;
        }
    }
    if matches!(lang, "shell" | "bash" | "sh" | "zsh") {
        return format_shell_fallback(content);
    }
    let normalized = content.replace("\r\n", "\n");
    let brace_indent = matches!(
        lang,
        "java"
            | "kotlin"
            | "javascript"
            | "typescript"
            | "json"
            | "jsonc"
            | "rust"
            | "go"
            | "c"
            | "cpp"
            | "protobuf"
    );
    let unit = if lang == "go" {
        "\t"
    } else if matches!(lang, "json" | "jsonc") {
        "  "
    } else {
        "    "
    };
    let mut depth = 0usize;
    let mut out_lines: Vec<String> = Vec::new();
    let mut blanks = 0usize;
    let mut in_block = false;

    for raw in normalized.lines() {
        let trimmed_end = raw.trim_end();
        if trimmed_end.trim().is_empty() {
            blanks += 1;
            if blanks <= 2 {
                out_lines.push(String::new());
            }
            continue;
        }
        blanks = 0;
        if !brace_indent {
            out_lines.push(trimmed_end.to_string());
            continue;
        }
        let trimmed = trimmed_end.trim_start();
        let mask = code_mask_for_line(lang, trimmed, &mut in_block);
        let code: String = trimmed
            .char_indices()
            .filter(|(i, _)| mask.get(*i).copied().unwrap_or(false))
            .map(|(_, c)| c)
            .collect();
        let leading = code
            .chars()
            .take_while(|&c| matches!(c, '}' | ')' | ']'))
            .count();
        out_lines.push(format!(
            "{}{}",
            unit.repeat(depth.saturating_sub(leading)),
            trimmed
        ));
        let opens = code.chars().filter(|&c| c == '{').count();
        let closes = code.chars().filter(|&c| c == '}').count();
        depth = if opens >= closes {
            depth + (opens - closes)
        } else {
            depth.saturating_sub(closes - opens)
        };
    }

    while out_lines.last().is_some_and(|l| l.is_empty()) {
        out_lines.pop();
    }
    if out_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", out_lines.join("\n"))
    }
}

fn format_shell_fallback(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    let mut depth = 0usize;
    let mut out_lines: Vec<String> = Vec::new();
    let mut blanks = 0usize;
    for raw in normalized.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            blanks += 1;
            if blanks <= 2 {
                out_lines.push(String::new());
            }
            continue;
        }
        blanks = 0;
        let deindent_before = matches!(trimmed, "fi" | "done" | "esac" | "}")
            || trimmed.starts_with("elif ")
            || trimmed == "else"
            || trimmed.starts_with(";;");
        let line_depth = if deindent_before {
            depth.saturating_sub(1)
        } else {
            depth
        };
        out_lines.push(format!("{}{}", "  ".repeat(line_depth), trimmed));

        if matches!(trimmed, "fi" | "done" | "esac" | "}") {
            depth = depth.saturating_sub(1);
        }
        let code_part = trimmed.split('#').next().unwrap_or("").trim();
        if code_part.ends_with("; then")
            || code_part == "then"
            || code_part.ends_with("; do")
            || code_part == "do"
            || code_part.ends_with('{')
            || (code_part.starts_with("case ") && code_part.ends_with(" in"))
        {
            depth += 1;
        }
    }
    while out_lines.last().is_some_and(|l| l.is_empty()) {
        out_lines.pop();
    }
    if out_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", out_lines.join("\n"))
    }
}
