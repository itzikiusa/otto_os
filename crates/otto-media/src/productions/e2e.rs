//! The planner of the isolated E2E daemon (`OTTO_E2E=1`, the
//! [`otto_core::e2e::isolated`] gate every E2E seam follows): a fixed plan —
//! the owner's sword-and-monster scene in `e2e_plan.json`, as the real
//! planner wrote it (its layout and the man's 3D prompt) — in place of the
//! agent turn, so the whole Smart flow runs on the real server and executor
//! without an agent CLI. A steering turn changes the current plan the way the
//! real planner did in the second acceptance run (real planner, real Forge): "the man should look older
//! and tired" gives the man its real older prompt (he is made again), and any
//! other comment makes the monster bigger (its size only, so it is not made
//! again). A description of a pier gets `e2e_pier_plan.json` instead (a
//! setting: water, night, fog, a glowing lantern), and "daytime" on a plan
//! with a setting makes it a clear day (no item changes). A description of a
//! chest of gold gets `e2e_progress_plan.json` (a man, a chest, coins in it and
//! his line: the making screen's own walk-through). While the file
//! `$OTTO_DATA_DIR/e2e-hold-planning` exists the planner keeps thinking, so a
//! test can look at the thinking screen. A new scene of a project (its prompt
//! lists the project's pieces) reuses every piece the project offers, in the
//! project's last place. The host picks it only under that gate.

use async_trait::async_trait;
use otto_core::Result;
use serde_json::{json, Value};

use super::ProductionPlanner;

const PLAN: &str = include_str!("e2e_plan.json");
const PIER: &str = include_str!("e2e_pier_plan.json");
const PROGRESS: &str = include_str!("e2e_progress_plan.json");

pub struct FixedPlanner;

#[async_trait]
impl ProductionPlanner for FixedPlanner {
    async fn run(&self, _production_id: &str, prompt: String) -> Result<String> {
        let hold = std::env::var_os("OTTO_DATA_DIR")
            .map(|d| std::path::Path::new(&d).join("e2e-hold-planning"));
        while let Some(true) = hold.as_ref().map(|h| h.exists()) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        Ok(fixed_reply(&prompt))
    }
}

/// The man's prompt after "the man should look older and tired", as the real
/// planner wrote it (smart-acceptance-v2, `plans/plan-after-steer.json`).
pub const OLDER_MAN: &str = "a rugged old male warrior in his sixties, tired weathered face with deep wrinkles, heavy bags under sunken weary eyes, hollow cheeks, short grey hair, thick grey beard, scar across one cheek, worn brown leather armor with a fur collar, cloth trousers, heavy boots, stylized fantasy game character, full body";

/// The fixed answer: the plan, or for a steering prompt the current plan
/// changed as the comment says, plus its revision.
pub fn fixed_reply(prompt: &str) -> String {
    let part = |from: &str, to: &str| {
        prompt
            .split(from)
            .nth(1)
            .and_then(|c| c.split(to).next())
            .unwrap_or_default()
            .to_string()
    };
    if let Some(reply) = project_reply(prompt) {
        return reply;
    }
    if !prompt.contains("REVISING an existing plan") {
        let said = part("The description:\n<<<\n", "\n>>>").to_lowercase();
        return match () {
            _ if said.contains("pier") => PIER,
            _ if said.contains("chest of gold") => PROGRESS,
            _ => PLAN,
        }
        .to_string();
    }
    let comment = part("The person's comment:\n<<<\n", "\n>>>").to_lowercase();
    let mut plan: Value =
        serde_json::from_str(&part("The current plan:\n", "\n\nThe person's comment:"))
            .or_else(|_| serde_json::from_str(PLAN))
            .unwrap_or_default();
    if comment.contains("daytime") && plan.get("setting").is_some() {
        plan["setting"]["time"] = "day".into();
        plan["setting"]["weather"] = "clear".into();
        plan["setting"]["summary"] = "A wooden pier over the sea on a clear day.".into();
        return json!({
            "plan": plan,
            "revision": { "summary": "It will be a clear day.", "changed": [], "added": [], "removed": [] }
        })
        .to_string();
    }
    let older = comment.contains("older") || comment.contains("tired");
    let id = if older { "man" } else { "monster" };
    if let Some(it) = plan["items"]
        .as_array_mut()
        .and_then(|xs| xs.iter_mut().find(|i| i["id"] == id))
    {
        if older {
            it["description"] =
                "An old, tired fighter who takes the sword and fights the monster.".into();
            it["details"]["prompt"] = OLDER_MAN.into();
        } else {
            it["description"] =
                "A huge beast with a key on a cord around its neck. It guards the chest.".into();
            it["details"]["size_m"] = 4.0.into();
        }
    }
    let summary = if older {
        "The man will be made again so he looks older and more tired, with grey hair, a grey beard and a wrinkled, weary face."
    } else {
        "I’ll make the monster bigger."
    };
    json!({
        "plan": plan,
        "revision": { "summary": summary, "changed": [id], "added": [], "removed": [] }
    })
    .to_string()
}

/// A new scene of a project: every piece the prompt offers, reused (by its
/// `reuse` mark, as the planner is told), in the project's last place.
fn project_reply(prompt: &str) -> Option<String> {
    let list = prompt
        .split("(one JSON object a line):\n")
        .nth(1)?
        .split("\nReuse them:")
        .next()?;
    let pieces: Vec<Value> = list
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if pieces.is_empty() {
        return None;
    }
    let places = ["center", "left", "right", "front", "back", "far"];
    let items: Vec<Value> = pieces
        .iter()
        .enumerate()
        .map(|(n, p)| {
            json!({"id": format!("p{n}"), "kind": p["kind"], "name": p["name"], "description": p["description"],
                   "enabled": true, "details": {"reuse": p["reuse"]}})
        })
        .collect();
    let layout: Vec<Value> = (0..items.len())
        .map(|n| json!({"item": format!("p{n}"), "place": places[n % places.len()]}))
        .collect();
    let place = prompt
        .split("(plan.setting): ")
        .nth(1)
        .and_then(|s| s.lines().next())
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let mut plan = json!({"title": "Later that day", "summary": "The same people and things, later that day.",
                          "notes": [], "items": items, "layout": layout});
    if let Some(place) = place {
        plan["setting"] = place;
    }
    Some(plan.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::productions::{parse_plan_reply, plan_prompt, steer_prompt, Plan};

    #[test]
    fn a_project_scene_reuses_every_piece_it_is_offered_in_its_place() {
        let (plan, _) = parse_plan_reply(&fixed_reply(&plan_prompt(
            "a chest of gold",
            "pc",
            "plan.json",
        )))
        .unwrap();
        let reuse = crate::productions::reuse::Reuse {
            artifact_id: "a1".into(),
            ..Default::default()
        };
        let piece = crate::productions::reuse::Piece {
            kind: "object".into(),
            name: "Chest".into(),
            description: "A chest.".into(),
            details: crate::productions::Details {
                reuse: Some(reuse),
                ..Default::default()
            },
        };
        let night = parse_plan_reply(&fixed_reply(&plan_prompt(
            "A foggy pier at night.",
            "pc",
            "plan.json",
        )))
        .unwrap()
        .0;
        let project = crate::productions::reuse::prompt(&[piece], night.setting.as_ref());
        let asked = crate::productions::project_plan_prompt("later", "pc", "plan.json", &project);
        let (later, _) =
            parse_plan_reply(&fixed_reply(&asked)).expect("the project plan validates");
        assert_eq!(later.items.len(), 1);
        assert_eq!(
            later.items[0].details.reuse.as_ref().unwrap().artifact_id,
            "a1"
        );
        let (was, now) = (night.setting.unwrap(), later.setting.unwrap());
        assert_eq!(
            (now.ground, now.time, now.weather),
            (was.ground, was.time, was.weather)
        );
        assert!(now.lights.is_empty(), "the lights named that scene's items");
        assert_ne!(later.title, plan.title);
    }

    #[test]
    fn a_pier_gets_the_night_setting_and_daytime_changes_only_it() {
        let night = plan_prompt(
            "A robot keeper on a foggy pier at night.",
            "pc",
            "plan.json",
        );
        let (plan, _) = parse_plan_reply(&fixed_reply(&night)).expect("the pier plan validates");
        let set = plan.setting.clone().expect("a setting");
        assert_eq!(
            (set.ground.as_str(), set.time.as_str(), set.weather.as_str()),
            ("water", "night", "fog")
        );
        let steer = steer_prompt(&plan, "make it daytime", &[], "plan.json");
        let (day, rev) = parse_plan_reply(&fixed_reply(&steer)).expect("validates");
        assert_eq!(day.items, plan.items);
        assert_eq!(day.setting.unwrap().time, "day");
        assert!(rev.unwrap().changed.is_empty());
        // A chest of gold is the making screen's walk-through.
        let gold = plan_prompt("A tired man next to a chest of gold.", "pc", "plan.json");
        let (gold, _) = parse_plan_reply(&fixed_reply(&gold)).expect("the progress plan validates");
        assert_eq!(gold.items.len(), 4);
        // Any other description is the owner's sword scene.
        let sword = plan_prompt("a man pulls a sword", "pc", "plan.json");
        assert!(parse_plan_reply(&fixed_reply(&sword))
            .unwrap()
            .0
            .setting
            .is_none());
    }

    #[test]
    fn the_fixed_plan_is_valid_and_a_steer_changes_one_piece_of_the_current_plan() {
        let (plan, rev) =
            parse_plan_reply(&fixed_reply("plan this")).expect("fixed plan validates");
        assert!(rev.is_none());
        assert_eq!(plan.items.len(), 10);
        let changed = |a: &Plan, b: &Plan| -> Vec<String> {
            a.items
                .iter()
                .zip(&b.items)
                .filter(|(x, y)| x != y)
                .map(|(x, _)| x.id.clone())
                .collect()
        };
        let steer = steer_prompt(&plan, "make the monster bigger", &[], "plan.json");
        let (bigger, rev) = parse_plan_reply(&fixed_reply(&steer)).expect("steered plan validates");
        assert_eq!(rev.expect("a revision").changed, ["monster"]);
        assert_eq!(changed(&plan, &bigger), ["monster"]);
        let monster = |p: &Plan| p.items[1].details.clone();
        assert_eq!(monster(&bigger).size_m, Some(4.0));
        assert_eq!(monster(&bigger).prompt, monster(&plan).prompt);
        // A look change on the man keeps the bigger monster of the plan it changes.
        let older = steer_prompt(
            &bigger,
            "The man should look older and tired.",
            &[],
            "plan.json",
        );
        let (old, rev) = parse_plan_reply(&fixed_reply(&older)).expect("validates");
        assert_eq!(rev.expect("a revision").changed, ["man"]);
        assert_eq!(changed(&bigger, &old), ["man"]);
        assert_eq!(old.items[0].details.prompt.as_deref(), Some(OLDER_MAN));
        assert_eq!(monster(&old).size_m, Some(4.0));
    }
}
