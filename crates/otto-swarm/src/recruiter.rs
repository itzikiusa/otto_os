//! Recruiter & planner prompt design (pure). The headless `run_agent` calls that
//! use these live in otto-server; kept here so they're unit-testable and the
//! output contracts are co-located with the DTOs.

use serde_json::Value;

use crate::types::{PresetAgent, RecruitedAgent};

/// Build the recruiter prompt: given a role the user named, propose a complete
/// agent definition (title, reports-to, specialization, soul, skills with
/// must-use flags, provider/model, schedule, scope).
pub fn recruiter_prompt(
    role: &str,
    swarm_name: &str,
    swarm_mission: &str,
    existing_titles: &[String],
    available_skills: &[String],
    available_providers: &[String],
    extra_context: Option<&str>,
) -> String {
    let org = if existing_titles.is_empty() {
        "(none yet)".to_string()
    } else {
        existing_titles.join(", ")
    };
    let skills = if available_skills.is_empty() {
        "(no library skills installed)".to_string()
    } else {
        available_skills.join(", ")
    };
    let providers = available_providers.join(", ");
    let ctx = extra_context.unwrap_or("");
    format!(
        r#"You are an expert technical recruiter assembling an AI agent team ("swarm").

Swarm: {swarm_name}
Mission: {swarm_mission}
Existing roles in the org: {org}
The user wants to add this role: "{role}"
{ctx}

Available LIBRARY SKILLS you may assign (use EXACT names; do not invent skills): {skills}
Available agent providers (CLIs) you may pick from: {providers}

Design the agent. Give it a believable "soul" (background + a few characteristic
traits that shape how it works), a crisp specialization, a clear scope (what it
owns and its boundaries), and the SMALLEST set of skills it genuinely needs —
mark the ones it MUST use. Suggest the provider best suited to the role (only from
the available list) and, if the role is naturally recurring (a researcher, a PM
status report, an auditor), a sensible schedule.

Respond with EXACTLY ONE ```json block, no prose, matching this schema:
{{
  "name": "short human name",
  "title": "{role}",
  "reports_to_title": "title of the manager from the existing org, or null",
  "specialization": "one sentence",
  "soul_md": "2-5 sentences: background + characteristic traits",
  "scope_md": "what they own and their boundaries",
  "skills": [{{"name": "exact-library-skill-name", "must_use": true, "why": "short"}}],
  "suggested_provider": "one of the available providers",
  "suggested_model": null,
  "suggested_schedule": null,
  "avatar": "a single emoji"
}}
For suggested_schedule, use null OR
{{"cadence":"daily","at":"09:00","directive":"what to do each run","enabled":true}}."#
    )
}

/// Extract the first JSON object from a model reply (fenced or balanced).
pub fn extract_json(text: &str) -> Option<Value> {
    // Prefer a ```json fenced block.
    if let Some(start) = text.find("```json") {
        let rest = &text[start + 7..];
        if let Some(end) = rest.find("```") {
            if let Ok(v) = serde_json::from_str::<Value>(rest[..end].trim()) {
                return Some(v);
            }
        }
    }
    // Fall back to the first balanced { .. }.
    let bytes = text.as_bytes();
    let start = text.find('{')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'"' if !esc => in_str = !in_str,
            b'\\' if in_str => {
                esc = !esc;
                continue;
            }
            b'{' if !in_str => depth += 1,
            b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..=i]).ok();
                }
            }
            _ => {}
        }
        esc = false;
    }
    None
}

pub fn parse_recruited(text: &str) -> Option<RecruitedAgent> {
    let v = extract_json(text)?;
    serde_json::from_value(v).ok()
}

/// Build the planner prompt: break a project goal into tasks, each optionally
/// assigned to a role and with dependencies (by title) forming the DAG.
pub fn planner_prompt(project_name: &str, goal_md: &str, agents: &[PresetAgent]) -> String {
    let roles: Vec<String> = agents
        .iter()
        .map(|a| format!("- {} ({})", a.title, a.name))
        .collect();
    format!(
        r#"You are a delivery lead breaking a project into actionable tasks for an AI agent team.

Project: {project_name}
Goal:
{goal_md}

Team roles available to assign work to:
{roles}

Produce a focused task breakdown — enough to deliver the goal, not busywork.
Order matters: express dependencies so independent work can run in parallel.

Respond with EXACTLY ONE ```json block, no prose:
{{
  "tasks": [
    {{
      "title": "short imperative",
      "description": "what done looks like (acceptance)",
      "assignee_title": "a role title from the list, or null",
      "priority": "low|medium|high|urgent",
      "depends_on_titles": ["titles of tasks that must finish first"]
    }}
  ]
}}"#,
        roles = roles.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_fenced_json() {
        let t = "blah\n```json\n{\"a\": 1}\n```\ntail";
        assert_eq!(extract_json(t).unwrap()["a"], 1);
    }

    #[test]
    fn extract_balanced_json() {
        let t = "prefix {\"a\": {\"b\": 2}} suffix";
        assert_eq!(extract_json(t).unwrap()["a"]["b"], 2);
    }

    #[test]
    fn parse_recruited_minimal() {
        let t = r#"```json
{"name":"Ada","title":"CTO","reports_to_title":null,"specialization":"x",
 "soul_md":"y","scope_md":"z","skills":[{"name":"s","must_use":true,"why":"w"}],
 "suggested_provider":"claude","suggested_model":null,"suggested_schedule":null,"avatar":"🛠"}
```"#;
        let a = parse_recruited(t).unwrap();
        assert_eq!(a.title, "CTO");
        assert_eq!(a.skills.len(), 1);
        assert!(a.skills[0].must_use);
    }
}
