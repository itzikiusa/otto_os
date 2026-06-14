//! Load the self-reflection skill instructions and assemble the analysis prompt.

use std::path::Path;

use crate::digest::SessionDigest;

/// Bundled fallback copy of the skill (authored in Task 17).
const BUNDLED_SKILL: &str = include_str!("../assets/workspace-self-reflection.md");

/// Read the skill body the engine inlines into the prompt. Prefers a
/// per-workspace override at `<root>/.claude/skills/workspace-self-reflection/SKILL.md`,
/// else the bundled copy.
pub fn load_skill_instructions(root: &str) -> String {
    let override_path = Path::new(root)
        .join(".claude")
        .join("skills")
        .join("workspace-self-reflection")
        .join("SKILL.md");
    std::fs::read_to_string(&override_path).unwrap_or_else(|_| BUNDLED_SKILL.to_string())
}

/// Assemble the full analysis prompt.
pub fn build_prompt(
    skill_instructions: &str,
    workspace_name: &str,
    digests: &[SessionDigest],
    current_skills: &[(String, String)], // (name, content)
    current_memory: &[(String, String)], // (filename, content)
    skill_allowlist: &[String],
) -> String {
    let mut s = String::new();
    s.push_str(skill_instructions);
    s.push_str("\n\n========================================\n");
    s.push_str(&format!("# Workspace: {workspace_name}\n\n"));

    s.push_str("## Skills you MAY propose edits to (allow-list)\n");
    if skill_allowlist.is_empty() {
        s.push_str("(none — every skill edit will be queued for human approval)\n\n");
    } else {
        for name in skill_allowlist {
            s.push_str(&format!("- {name}\n"));
        }
        s.push('\n');
    }

    s.push_str("## Current skill files\n");
    if current_skills.is_empty() {
        s.push_str("(none in scope)\n\n");
    } else {
        for (name, content) in current_skills {
            s.push_str(&format!("### skill: {name}\n```\n{content}\n```\n\n"));
        }
    }

    s.push_str("## Current workspace memory\n");
    if current_memory.is_empty() {
        s.push_str("(empty)\n\n");
    } else {
        for (file, content) in current_memory {
            s.push_str(&format!("### memory file: {file}\n```\n{content}\n```\n\n"));
        }
    }

    s.push_str("## Recent sessions to learn from\n");
    for d in digests {
        s.push_str(&format!(
            "### session {} — \"{}\" ({} turns, {} tool errors; skills used: {})\n{}\n\n",
            d.session_id,
            d.title,
            d.turns,
            d.tool_errors,
            if d.skills_used.is_empty() { "none".to_string() } else { d.skills_used.join(", ") },
            d.text,
        ));
    }
    s.push_str(
        "\n========================================\n\
         Now output ONLY the JSON proposal object per the schema above. No prose.\n",
    );
    s
}
