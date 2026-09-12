//! `GET /sessions/{id}/slash-commands` — the slash commands the composer can
//! complete for a session: the provider CLI's built-ins plus the user's own
//! custom commands and skills on disk (Claude: `~/.claude/commands/*.md`,
//! `<cwd>/.claude/commands/*.md`, `~/.claude/skills/*/SKILL.md`,
//! `<cwd>/.claude/skills/*/SKILL.md`; Codex: `$CODEX_HOME/skills`,
//! `<cwd>/.codex/skills`). Read-only, best-effort: a missing dir is just no
//! entries. Descriptions come from the file's frontmatter `description:` (or
//! the first non-empty line for a bare command file), clipped to 160 chars.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use axum::extract::{Path as AxPath, State};
use axum::Json;
use otto_core::Id;
use serde::Serialize;

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;
use otto_core::domain::WorkspaceRole;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SlashCommand {
    /// Without the leading slash (`compact`, `commit`, `frontend:component`).
    pub name: String,
    pub description: String,
    /// `builtin` | `user` (home dir) | `project` (session cwd).
    pub source: &'static str,
}

const CLAUDE_BUILTINS: &[(&str, &str)] = &[
    ("add-dir", "Add a working directory to this session"),
    ("agents", "Manage subagent definitions"),
    ("bug", "Report a bug to Anthropic"),
    ("clear", "Clear the conversation history"),
    ("compact", "Compact the conversation, optionally with focus instructions"),
    ("config", "Open settings"),
    ("context", "Show what is using the context window"),
    ("cost", "Show token usage and cost for this session"),
    ("doctor", "Check the Claude Code installation"),
    ("exit", "Exit the CLI"),
    ("export", "Export the conversation to a file"),
    ("help", "Show help and available commands"),
    ("init", "Create a CLAUDE.md for this project"),
    ("mcp", "Manage MCP servers"),
    ("memory", "Edit CLAUDE.md memory files"),
    ("model", "Switch the model"),
    ("permissions", "View or update permissions"),
    ("plan", "Switch to plan mode"),
    ("resume", "Resume a previous conversation"),
    ("review", "Review a pull request"),
    ("rewind", "Rewind the conversation and files to a checkpoint"),
    ("stats", "Show session statistics"),
    ("status", "Show account and system status"),
    ("terminal-setup", "Install Shift+Enter key binding"),
    ("usage", "Show plan usage limits"),
    ("vim", "Toggle vim editing mode"),
];

const CODEX_BUILTINS: &[(&str, &str)] = &[
    ("clear", "Clear the conversation"),
    ("compact", "Summarize the conversation to free context"),
    ("diff", "Show the git diff of the working tree"),
    ("init", "Create an AGENTS.md for this project"),
    ("mention", "Mention a file"),
    ("model", "Choose the model and reasoning effort"),
    ("new", "Start a new chat"),
    ("quit", "Exit Codex"),
    ("review", "Review the current changes"),
    ("status", "Show session configuration and token usage"),
];

fn frontmatter_description(body: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    for line in lines {
        let t = line.trim();
        if t == "---" {
            return None;
        }
        if let Some(rest) = t.strip_prefix("description:") {
            let v = rest.trim().trim_matches(['"', '\'']).to_string();
            return if v.is_empty() { None } else { Some(v) };
        }
    }
    None
}

fn first_line(body: &str) -> Option<String> {
    let mut in_fm = false;
    for (i, line) in body.lines().enumerate() {
        let t = line.trim();
        if i == 0 && t == "---" {
            in_fm = true;
            continue;
        }
        if in_fm {
            if t == "---" {
                in_fm = false;
            }
            continue;
        }
        if t.is_empty() || t.starts_with('#') && t.trim_start_matches('#').trim().is_empty() {
            continue;
        }
        return Some(t.trim_start_matches('#').trim().to_string());
    }
    None
}

fn clip(s: String) -> String {
    if s.chars().count() <= 160 {
        s
    } else {
        let mut out: String = s.chars().take(159).collect();
        out.push('…');
        out
    }
}

/// `<dir>/**/<name>.md` → `/name` (subdirs namespaced as `dir:name`, the
/// Claude convention). Depth-limited; symlinks are followed by `read_dir`.
fn scan_commands(dir: &Path, prefix: &str, source: &'static str, depth: u8, out: &mut BTreeMap<String, SlashCommand>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if p.is_dir() {
            if depth < 2 {
                scan_commands(&p, &format!("{prefix}{stem}:"), source, depth + 1, out);
            }
            continue;
        }
        if p.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        let body = std::fs::read_to_string(&p).unwrap_or_default();
        let description = frontmatter_description(&body).or_else(|| first_line(&body)).unwrap_or_default();
        let name = format!("{prefix}{stem}");
        out.entry(name.clone()).or_insert(SlashCommand {
            name,
            description: clip(description),
            source,
        });
    }
}

/// `<dir>/<name>/SKILL.md` → `/name`.
fn scan_skills(dir: &Path, source: &'static str, out: &mut BTreeMap<String, SlashCommand>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let skill = p.join("SKILL.md");
        let Ok(body) = std::fs::read_to_string(&skill) else {
            continue;
        };
        let description = frontmatter_description(&body).or_else(|| first_line(&body)).unwrap_or_default();
        out.entry(name.to_string()).or_insert(SlashCommand {
            name: name.to_string(),
            description: clip(description),
            source,
        });
    }
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_home().join(".codex"))
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// The list for `provider` at `cwd`. Pure over the filesystem — unit-testable
/// with temp dirs.
pub fn list(provider: &str, cwd: &Path, home: &Path, codex_home: &Path) -> Vec<SlashCommand> {
    let mut custom: BTreeMap<String, SlashCommand> = BTreeMap::new();
    let builtins: &[(&str, &str)] = match provider {
        "claude" => {
            scan_commands(&cwd.join(".claude/commands"), "", "project", 0, &mut custom);
            scan_skills(&cwd.join(".claude/skills"), "project", &mut custom);
            scan_commands(&home.join(".claude/commands"), "", "user", 0, &mut custom);
            scan_skills(&home.join(".claude/skills"), "user", &mut custom);
            CLAUDE_BUILTINS
        }
        "codex" => {
            scan_skills(&cwd.join(".codex/skills"), "project", &mut custom);
            scan_skills(&codex_home.join("skills"), "user", &mut custom);
            CODEX_BUILTINS
        }
        _ => &[],
    };
    let mut out: Vec<SlashCommand> = builtins
        .iter()
        .map(|(n, d)| SlashCommand {
            name: (*n).to_string(),
            description: (*d).to_string(),
            source: "builtin",
        })
        .collect();
    out.extend(custom.into_values());
    out
}

/// `GET /sessions/{id}/slash-commands` — Viewer + owner-or-admin (same gate
/// as the transcript; the list reveals the user's command/skill names).
pub async fn slash_commands(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<SlashCommand>>> {
    let session = super::transcript::session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let provider = super::transcript::effective_provider(&session);
    let cwd = session
        .meta
        .get("nested_cwd")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| session.cwd.clone());
    let cwd = PathBuf::from(cwd);
    let list = tokio::task::spawn_blocking(move || list(&provider, &cwd, &dirs_home(), &codex_home()))
        .await
        .unwrap_or_default();
    Ok(Json(list))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_builtins_plus_project_and_user_commands_and_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cwd = tmp.path().join("proj");
        std::fs::create_dir_all(home.join(".claude/commands/frontend")).unwrap();
        std::fs::write(home.join(".claude/commands/commit.md"), "---\ndescription: \"Write a commit\"\n---\nbody").unwrap();
        std::fs::write(home.join(".claude/commands/frontend/component.md"), "# Make a Svelte component\n\nsteps").unwrap();
        std::fs::create_dir_all(home.join(".claude/skills/pr")).unwrap();
        std::fs::write(home.join(".claude/skills/pr/SKILL.md"), "---\nname: pr\ndescription: Open a PR\n---\n").unwrap();
        std::fs::create_dir_all(cwd.join(".claude/commands")).unwrap();
        // Project shadows user for the same name.
        std::fs::write(cwd.join(".claude/commands/commit.md"), "---\ndescription: Project commit\n---\n").unwrap();
        let out = list("claude", &cwd, &home, &tmp.path().join("codex"));
        let by = |n: &str| out.iter().find(|c| c.name == n).cloned();
        assert!(by("compact").is_some_and(|c| c.source == "builtin"));
        assert_eq!(by("commit").unwrap().description, "Project commit");
        assert_eq!(by("commit").unwrap().source, "project");
        assert_eq!(by("frontend:component").unwrap().description, "Make a Svelte component");
        assert_eq!(by("pr").unwrap().description, "Open a PR");
        // Codex: only its builtins + skills dirs.
        let codex = list("codex", &cwd, &home, &tmp.path().join("codex"));
        assert!(codex.iter().any(|c| c.name == "new"));
        assert!(codex.iter().all(|c| c.name != "commit"));
        assert!(list("agy", &cwd, &home, &home).is_empty());
    }
}
