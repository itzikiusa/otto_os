//! Recruiter & planner prompt design (pure). The headless `run_agent` calls that
//! use these live in otto-server; kept here so they're unit-testable and the
//! output contracts are co-located with the DTOs.

use serde_json::Value;

use crate::types::{PresetAgent, RecruitedAgent};

/// Maximum number of skills injected into the recruiter prompt. Injecting the
/// full library (potentially hundreds of names) wastes tokens and can produce
/// bloated skill lists; we prioritise role-relevant skills and hard-cap the rest.
pub const RECRUITER_SKILL_CAP: usize = 40;

/// Return at most `cap` skill names from `all_skills`, ranked by how many words
/// from `role` appear in the skill name (case-insensitive prefix/infix match).
/// Within the same score tier names are sorted alphabetically for stability.
///
/// Used by the otto-server recruiter endpoint (pure so it is unit-testable).
pub fn cap_skills_for_role(all_skills: &[String], role: &str, cap: usize) -> Vec<String> {
    let role_lower = role.to_lowercase();
    let role_words: std::collections::HashSet<&str> = role_lower
        .split_whitespace()
        .collect();
    let mut scored: Vec<(usize, &String)> = all_skills
        .iter()
        .map(|name| {
            let n = name.to_lowercase();
            let score = role_words.iter().filter(|&&w| n.contains(w)).count();
            (score, name)
        })
        .collect();
    // Higher score first; stable alphabetical within a tier.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.iter().take(cap).map(|(_, n)| (*n).clone()).collect()
}

/// Build the recruiter prompt: given a role the user named, propose a complete
/// agent definition (title, reports-to, specialization, soul, skills with
/// must-use flags, provider/model, schedule, scope).
#[allow(clippy::too_many_arguments)]
pub fn recruiter_prompt(
    role: &str,
    swarm_name: &str,
    swarm_mission: &str,
    existing_titles: &[String],
    available_skills: &[String],
    available_providers: &[String],
    extra_context: Option<&str>,
    naming_theme: Option<&str>,
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
    // Optional naming theme: derive the agent's name from a domain (e.g. famous
    // footballers, NBA legends) for a cohesive, fun team roster.
    let name_line = match naming_theme.map(str::trim).filter(|t| !t.is_empty()) {
        Some(theme) => format!(
            "\nNAMING THEME: Give the agent a real, recognizable first name (or single \
             memorable moniker) drawn from \"{theme}\". It MUST fit that theme and MUST NOT \
             duplicate an existing teammate's name. Keep \"title\" as the role, not the theme.\n"
        ),
        None => String::new(),
    };
    format!(
        r#"You are an expert technical recruiter assembling an AI agent team ("swarm").

Swarm: {swarm_name}
Mission: {swarm_mission}
Existing roles in the org: {org}
The user wants to add this role: "{role}"
{ctx}
{name_line}

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
/// The JSON task-list schema both planners and the summarizer must emit.
const PLAN_SCHEMA: &str = r#"{
  "tasks": [
    {
      "title": "short imperative",
      "description": "what done looks like (acceptance)",
      "assignee_title": "a role title from the list, or null",
      "priority": "low|medium|high|urgent",
      "depends_on_titles": ["titles of tasks that must finish first"]
    }
  ]
}"#;

/// Distinct planning angles so a multi-agent plan covers the goal from several
/// perspectives. The summarizer then merges the candidates into one list.
pub const PLANNER_ANGLES: &[&str] = &[
    "Break the goal into the smallest shippable increments, end to end.",
    "Organize the work by component/area and call out cross-cutting concerns, risks, and testing.",
];

pub fn planner_prompt(
    project_name: &str,
    goal_md: &str,
    agents: &[PresetAgent],
    angle: &str,
) -> String {
    let roles: Vec<String> = agents
        .iter()
        .map(|a| format!("- {} ({})", a.title, a.name))
        .collect();
    let angle_line = if angle.trim().is_empty() {
        String::new()
    } else {
        format!("\nPlanning emphasis for this pass: {angle}\n")
    };
    format!(
        r#"You are a delivery lead breaking a project into actionable tasks for an AI agent team.

Project: {project_name}
Goal:
{goal_md}
{angle_line}
Team roles available to assign work to:
{roles}

Produce a focused task breakdown — enough to deliver the goal, not busywork.
Order matters: express dependencies so independent work can run in parallel.

Respond with EXACTLY ONE ```json block, no prose:
{PLAN_SCHEMA}"#,
        roles = roles.join("\n")
    )
}

/// The discovery task-list schema. Investigation tasks carry only a title and a
/// description (no role assignment / DAG): discovery work fans out, it is not a
/// dependency chain, and the discovery project is seeded flat.
const DISCOVERY_SCHEMA: &str = r#"{
  "tasks": [
    {
      "title": "short imperative investigation goal",
      "description": "what to investigate and what the finding should cover"
    }
  ]
}"#;

/// Build the discovery planner prompt: break a story's discovery brief into a
/// handful of *investigation* tasks (NOT implementation work). The agents that
/// pick these up are expected to research and report — map affected
/// services/files, dependencies, risks, prior art, open questions — never to
/// write production code. Mirrors `planner_prompt`'s shape (one JSON block) but
/// is framed for discovery-before-implementation.
pub fn discovery_planner_prompt(
    project_name: &str,
    brief: &str,
    agents: &[PresetAgent],
    extra: &str,
) -> String {
    let roles: Vec<String> = agents
        .iter()
        .map(|a| format!("- {} ({})", a.title, a.name))
        .collect();
    let roles_block = if roles.is_empty() {
        "(no preset agents — tasks will be picked up by whichever agents the swarm has)"
            .to_string()
    } else {
        roles.join("\n")
    };
    let extra_line = if extra.trim().is_empty() {
        String::new()
    } else {
        format!("\nExtra emphasis for this discovery: {extra}\n")
    };
    format!(
        r#"You are a discovery lead planning an INVESTIGATION (discovery before implementation).

Project: {project_name}
Discovery brief:
{brief}
{extra_line}
Team available to do the investigation:
{roles_block}

Break the brief into 3–6 focused INVESTIGATION tasks. These are research/analysis
tasks, NOT implementation work — agents must map and report, never write production
code or open PRs. Cover the angles that matter for this story: affected
services/files & data flow, dependencies & integration/contract risks, unknowns &
risks, prior art / similar work already in the codebase, and the open questions a
stakeholder must answer before implementation. Keep each task crisp and outcome-oriented.

Respond with EXACTLY ONE ```json block, no prose:
{DISCOVERY_SCHEMA}"#
    )
}

/// Merge several candidate task breakdowns (each the JSON from `planner_prompt`)
/// into one coherent, de-duplicated plan. Used by the multi-agent planner: N
/// planners run in parallel, then one summarizer reconciles their outputs.
pub fn planner_summarizer_prompt(
    project_name: &str,
    goal_md: &str,
    agents: &[PresetAgent],
    candidates: &[String],
) -> String {
    let roles: Vec<String> = agents
        .iter()
        .map(|a| format!("- {} ({})", a.title, a.name))
        .collect();
    let mut blocks = String::new();
    for (i, c) in candidates.iter().enumerate() {
        blocks.push_str(&format!("\n--- Candidate plan {} ---\n{}\n", i + 1, c.trim()));
    }
    format!(
        r#"You are the lead planner reconciling several independent task breakdowns for the same goal.

Project: {project_name}
Goal:
{goal_md}

Team roles available to assign work to:
{roles}

Below are {n} candidate plans produced independently. Merge them into ONE coherent plan:
- de-duplicate overlapping tasks (keep the clearest wording),
- keep the most valuable tasks from any candidate, drop busywork,
- reconcile and express dependencies so independent work can run in parallel,
- assign each task to the best-fit role title (or null).
{blocks}

Respond with EXACTLY ONE ```json block, no prose:
{PLAN_SCHEMA}"#,
        n = candidates.len(),
        roles = roles.join("\n"),
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
    fn discovery_planner_prompt_is_investigation_framed() {
        let p = discovery_planner_prompt("Discovery: Login", "## Brief\nstuff", &[], "");
        // Investigation framing, not implementation; asks for one JSON block.
        assert!(p.contains("INVESTIGATION"));
        assert!(p.contains("discovery before implementation"));
        assert!(p.contains("NOT implementation work"));
        assert!(p.contains("```json"));
        // The brief is embedded verbatim.
        assert!(p.contains("## Brief"));
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

    // -----------------------------------------------------------------------
    // cap_skills_for_role tests
    // -----------------------------------------------------------------------

    #[test]
    fn cap_skills_respects_hard_limit() {
        // 60 dummy skills, cap = 40 → result must be ≤ 40.
        let skills: Vec<String> = (0..60).map(|i| format!("skill-{i:02}")).collect();
        let capped = cap_skills_for_role(&skills, "engineer", RECRUITER_SKILL_CAP);
        assert!(
            capped.len() <= RECRUITER_SKILL_CAP,
            "expected ≤ {} skills, got {}",
            RECRUITER_SKILL_CAP, capped.len()
        );
    }

    #[test]
    fn cap_skills_fewer_than_cap_returns_all() {
        let skills: Vec<String> = (0..10).map(|i| format!("skill-{i}")).collect();
        let capped = cap_skills_for_role(&skills, "developer", RECRUITER_SKILL_CAP);
        assert_eq!(capped.len(), 10, "all skills should be returned when count < cap");
    }

    #[test]
    fn cap_skills_ranks_relevant_first() {
        let skills = vec![
            "code-review".to_string(),
            "deploy-ops".to_string(),
            "review-guide".to_string(),
        ];
        // Role = "code review" → words "code" and "review".
        // "code-review" contains BOTH words (score 2); "review-guide" contains "review"
        // (score 1); "deploy-ops" contains neither (score 0).
        let capped = cap_skills_for_role(&skills, "code review", 10);
        assert_eq!(capped[0], "code-review");
        assert_eq!(capped[1], "review-guide");
        assert_eq!(capped[2], "deploy-ops");
    }
}
