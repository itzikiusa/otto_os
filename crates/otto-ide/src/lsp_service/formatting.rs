//! Document formatting helpers (`gofmt`, `rustfmt`, `prettier`, `ruff`/`black`,
//! `google-java-format`, `shfmt`, `buf`) and zero-tool deterministic fallback formatters.

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use super::{code_mask_for_line, symbols_data};
use crate::target::ResolvedTarget;
use crate::IdeCtx;

pub(super) fn formatter_candidates(
    lang: &str,
    path: &str,
) -> Vec<(&'static str, String, Vec<String>)> {
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
        | "yaml" | "graphql" | "svelte" => vec![
            (
                "prettier",
                "prettier".into(),
                vec!["--stdin-filepath".into(), path.to_string()],
            ),
            (
                "biome",
                "biome".into(),
                vec![
                    "format".into(),
                    "--stdin-file-path".into(),
                    path.to_string(),
                ],
            ),
        ],
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
        let mut rows = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let cells: Vec<String> = symbols_data::split_delimited_row(line, delim)
                .into_iter()
                .map(|c| {
                    let t = c.trim();
                    if t.contains(delim) || t.contains('"') {
                        format!("\"{}\"", t.replace('"', "\"\""))
                    } else {
                        t.to_string()
                    }
                })
                .collect();
            rows.push(cells.join(&delim.to_string()));
        }
        return Some(if rows.is_empty() {
            String::new()
        } else {
            format!("{}\n", rows.join("\n"))
        });
    }
    None
}

pub(super) async fn run_stdin_formatter<S: IdeCtx>(
    ctx: &S,
    resolved: &ResolvedTarget,
    cmd: &str,
    args: &[String],
    env_map: &BTreeMap<String, String>,
    input: &str,
) -> Option<String> {
    let mut child = ctx
        .ide()
        .targets
        .spawn_piped(resolved, cmd, args, None, env_map)
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

pub fn format_builtin_fallback(lang: &str, content: &str) -> String {
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
