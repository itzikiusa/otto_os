use serde_json::{json, Value};

use super::*;

/// The owner's example, as a planner would answer it.
pub(crate) fn good_plan() -> Value {
    json!({
        "title": "The sword and the monster",
        "summary": "A man pulls a sword from the ground and fights the monster guarding a chest.",
        "items": [
            {"id": "man", "kind": "character", "name": "Man", "description": "A tired warrior in a cloak.",
             "enabled": true, "details": {"prompt": "tired warrior, cloak, boots", "voice": "deep, a bit broken",
             "moves": ["idle", "walk"], "holds": [{"item": "sword", "socket": "right_hand"}], "actions": ["pull_sword"]}},
            {"id": "sword", "kind": "object", "name": "Sword", "description": "An old sword stuck in the ground.",
             "enabled": true, "details": {"prompt": "old longsword"}},
            {"id": "monster", "kind": "character", "name": "Monster", "description": "A big horned beast.",
             "enabled": true, "details": {"prompt": "horned beast", "voice": "growling"}},
            {"id": "chest", "kind": "object", "name": "Chest", "description": "A wooden chest full of gold coins.",
             "enabled": true, "details": {"prompt": "wooden treasure chest"}},
            {"id": "key", "kind": "object", "name": "Key", "description": "A big iron key.",
             "enabled": true, "details": {"prompt": "iron key", "attach_to": {"item": "monster", "where": "neck"}}},
            {"id": "man-line", "kind": "voice_line", "name": "Man speaks", "description": "The man says he is ready.",
             "enabled": true, "details": {"speaker": "man", "text": "Come then. I am ready."}},
            {"id": "clash", "kind": "sound", "name": "Swords clash", "description": "Metal hitting metal.",
             "enabled": true, "details": {"prompt": "sword clash", "seconds": 2}},
            {"id": "theme", "kind": "music", "name": "Fight music", "description": "Fast drums for the fight.",
             "enabled": true, "details": {"prompt": "epic drums", "seconds": 60, "loop": true}}
        ],
        "notes": ["I can't make a 'pulling the sword out' move yet, so I used 'idle' and 'walk'."],
        "layout": [
            {"item": "man", "place": "center"},
            {"item": "sword", "place": "center-right"},
            {"item": "monster", "place": "front", "facing": "man", "scale_hint": "big"},
            {"item": "chest", "place": "front-right"},
            {"item": "key", "place": "front", "attach_to": {"item": "monster", "where": "neck"}}
        ]
    })
}

fn with(mut v: Value, f: impl FnOnce(&mut Value)) -> String {
    f(&mut v);
    v.to_string()
}

#[test]
fn a_good_plan_passes_and_progress_stays_the_servers() {
    let text = with(good_plan(), |v| {
        v["items"][0]["status"] = json!("done");
        v["items"][0]["result"] = json!({"artifact_id": "planted"});
    });
    let (plan, rev) = parse_plan_reply(&text).unwrap();
    assert!(rev.is_none());
    assert_eq!(plan.items.len(), 8);
    assert!(plan
        .items
        .iter()
        .all(|i| i.status == "waiting" && i.result.is_none()));
    assert_eq!(plan.items[0].details.holds[0].socket, "right_hand");
    assert_eq!(plan.items[7].details.looped, Some(true));
    // A fenced block in prose is read too.
    let fenced = format!("Here it is:\n\n```json\n{}\n```\nDone.", good_plan());
    assert!(parse_plan_reply(&fenced).is_ok());
}

#[test]
fn malformed_answers_are_refused() {
    for bad in [
        "no plan here",
        "[1, 2]",
        r#"{"title": 3, "items": []}"#,
        r#"{"title": "x", "items": []}"#,
    ] {
        assert!(parse_plan_reply(bad).is_err(), "{bad}");
    }
}

/// A move the engine cannot make is no reason to re-plan: the plan passes
/// and the executor leaves it out of `{clips}` with a plain note.
#[test]
fn an_unknown_move_passes_validation() {
    let mut v = good_plan();
    v["items"][0]["details"]["moves"] = json!(["idle", "backflip"]);
    assert!(parse_plan_reply(&v.to_string()).is_ok());
}

#[test]
fn unknown_kinds_actions_sockets_and_dangling_ids_are_refused() {
    type Edit = Box<dyn FnOnce(&mut Value)>;
    let cases: Vec<(&str, Edit)> = vec![
        (
            "kind",
            Box::new(|v| v["items"][1]["kind"] = json!("vehicle")),
        ),
        (
            "action",
            Box::new(|v| v["items"][0]["details"]["actions"] = json!(["juggle"])),
        ),
        (
            "socket",
            Box::new(|v| v["items"][0]["details"]["holds"][0]["socket"] = json!("mouth")),
        ),
        (
            "holds a character",
            Box::new(|v| v["items"][0]["details"]["holds"][0]["item"] = json!("monster")),
        ),
        (
            "speaker",
            Box::new(|v| v["items"][5]["details"]["speaker"] = json!("chest")),
        ),
        (
            "no text",
            Box::new(|v| v["items"][5]["details"]["text"] = Value::Null),
        ),
        (
            "object moves",
            Box::new(|v| v["items"][1]["details"]["moves"] = json!(["idle"])),
        ),
        (
            "duplicate id",
            Box::new(|v| v["items"][2]["id"] = json!("man")),
        ),
        (
            "bad id",
            Box::new(|v| v["items"][1]["id"] = json!("Big Sword!")),
        ),
        (
            "place",
            Box::new(|v| v["layout"][0]["place"] = json!("upstairs")),
        ),
        (
            "layout of a sound",
            Box::new(|v| v["layout"][0]["item"] = json!("clash")),
        ),
        (
            "facing",
            Box::new(|v| v["layout"][2]["facing"] = json!("ghost")),
        ),
        (
            "empty name",
            Box::new(|v| v["items"][1]["name"] = json!(" ")),
        ),
    ];
    for (what, f) in cases {
        let err = parse_plan_reply(&with(good_plan(), f));
        assert!(err.is_err(), "{what} passed");
    }
}

#[test]
fn too_many_items_are_refused() {
    let text = with(good_plan(), |v| {
        let one = v["items"][1].clone();
        let items = v["items"].as_array_mut().unwrap();
        for n in 0..MAX_ITEMS {
            let mut it = one.clone();
            it["id"] = json!(format!("rock-{n}"));
            items.push(it);
        }
    });
    let err = parse_plan_reply(&text).unwrap_err();
    assert!(err.contains("1-24 items"), "{err}");
}

#[test]
fn a_steering_answer_carries_its_revision() {
    let text = json!({
        "plan": good_plan(),
        "revision": {"summary": "The monster gets bigger horns.", "changed": ["monster"], "added": [], "removed": []}
    })
    .to_string();
    let (plan, rev) = parse_plan_reply(&text).unwrap();
    assert_eq!(plan.items.len(), 8);
    assert_eq!(rev.unwrap().changed, vec!["monster"]);
}
