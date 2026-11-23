//! Where a piece is relative to another (`layout[].rel`), in closed words the
//! scene builder can place: on, under, in, beside, behind, in front of, above
//! or around another piece, or held or worn by a character. It replaces the
//! free-prose `attach_to.where` (still read for older plans); a piece with a `rel`
//! has no `place` of its own (a held or worn one never). Also the plan's
//! shape words for a piece: `details.dims_m` and a character's `details.body`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::Plan;

pub const HOWS: &[&str] = &[
    "on",
    "under",
    "in",
    "beside",
    "behind",
    "in_front_of",
    "above",
    "around",
    "held",
    "worn",
];
/// `spot` words: one across, one deep, joined with "-" ("front-left").
pub const ACROSS: &[&str] = &["left", "center", "right"];
pub const DEEP: &[&str] = &["front", "middle", "back"];
pub const SIDES: &[&str] = &["left", "right", "front", "back"];
pub const HANDS: &[&str] = &["right_hand", "left_hand", "both"];
pub const PARTS: &[&str] = &["head", "neck", "back", "hip", "shoulders"];
/// A character's body, so the scene can pose and place it (a fish floats, a
/// bird perches).
/// A voice line's pace.
pub const PACES: &[&str] = &["slow", "normal", "fast"];
pub const BODIES: &[&str] = &[
    "two_legs",
    "four_legs",
    "bird",
    "fish",
    "no_legs",
    "many_legs",
    "wheels",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rel {
    pub to: String,
    pub how: String,
    /// Where on the target (on, in, under).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot: Option<String>,
    /// Which side of the target (beside).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// Which hand (held).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand: Option<String>,
    /// Which body part (worn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// What it is held over or pointed at (held): an item id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toward: Option<String>,
}

/// The relation checks (part of [`super::validate_plan`]).
pub fn validate(plan: &Plan) -> Result<(), String> {
    let kind = |id: &str| {
        plan.items
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.kind.as_str())
    };
    let mut members = HashSet::new();
    for id in &plan.lineup {
        if kind(id) != Some("character") || !members.insert(id) {
            return Err("lineup must contain distinct character item ids".into());
        }
    }
    let worn: HashSet<&str> = plan
        .layout
        .iter()
        .filter(|p| p.rel.as_ref().is_some_and(|r| r.how == "worn"))
        .map(|p| p.item.as_str())
        .collect();
    for it in &plan.items {
        let at = format!("item {}", it.id);
        let d = &it.details;
        if d.effect.is_some() && it.kind != "object" {
            return Err(format!(
                "{at}: details.effect belongs on the object that produces the effect"
            ));
        }
        if let Some(b) = &d.body {
            if it.kind != "character" || !BODIES.contains(&b.as_str()) {
                return Err(format!(
                    "{at}: details.body is for a character, one of {}",
                    BODIES.join(", ")
                ));
            }
        }
        if d.dims_m
            .is_some_and(|m| m.iter().any(|v| !(0.005..=500.0).contains(v)))
        {
            return Err(format!(
                "{at}: details.dims_m is [width, height, depth] in metres, each 0.005-500"
            ));
        }
        if d.featured == Some(true) && it.kind != "music" {
            return Err(format!("{at}: details.featured is for a music performance"));
        }
        let line = it.kind == "voice_line";
        if (d.pace.is_some() || d.mood.is_some() || d.replies_to.is_some()) && !line {
            return Err(format!(
                "{at}: pace, mood and replies_to are for a voice line"
            ));
        }
        if d.pace.as_deref().is_some_and(|p| !PACES.contains(&p)) {
            return Err(format!("{at}: details.pace is slow, normal or fast"));
        }
        if d.mood.as_deref().is_some_and(|m| !super::text_ok(m, 60)) {
            return Err(format!(
                "{at}: details.mood is a few plain words (1-60 characters)"
            ));
        }
        if let Some(r) = &d.replies_to {
            if *r == it.id || kind(r) != Some("voice_line") {
                return Err(format!(
                    "{at}: replies_to {r:?} must name another voice line"
                ));
            }
        }
        if let Some(h) = d.holds.iter().find(|h| worn.contains(h.item.as_str())) {
            return Err(format!(
                "{at}: {:?} is worn (rel how \"worn\"), so it is not in details.holds",
                h.item
            ));
        }
    }
    for p in &plan.layout {
        let Some(r) = &p.rel else {
            continue;
        };
        let at = format!("layout {} rel", p.item);
        if r.to == p.item || !matches!(kind(&r.to), Some("character" | "object")) {
            return Err(format!(
                "{at}: to {:?} must name another character or object",
                r.to
            ));
        }
        if !HOWS.contains(&r.how.as_str()) {
            return Err(format!("{at}: how must be one of {}", HOWS.join(", ")));
        }
        let only = |field: &str, v: &Option<String>, hows: &[&str], allowed: &[&str]| match v {
            Some(v) if !hows.contains(&r.how.as_str()) => Err(format!(
                "{at}: {field} is only for how {}",
                hows.join(" or ")
            )),
            Some(v) if !allowed.contains(&v.as_str()) => Err(format!(
                "{at}: {field} {v:?} must be one of {}",
                allowed.join(", ")
            )),
            _ => Ok(()),
        };
        only("side", &r.side, &["beside"], SIDES)?;
        only("hand", &r.hand, &["held"], HANDS)?;
        only("part", &r.part, &["worn"], PARTS)?;
        if let Some(t) = &r.toward {
            if r.how != "held" || *t == p.item || *t == r.to || kind(t).is_none() {
                return Err(format!(
                    "{at}: toward is only for how held, and names another item"
                ));
            }
        }
        if let Some(s) = &r.spot {
            let words: Vec<&str> = s.split('-').collect();
            let fits = |set: &[&str]| words.iter().filter(|w| set.contains(w)).count() <= 1;
            if !["on", "in", "under"].contains(&r.how.as_str())
                || words
                    .iter()
                    .any(|w| !ACROSS.contains(w) && !DEEP.contains(w))
                || !fits(ACROSS)
                || !fits(DEEP)
            {
                return Err(format!(
                    "{at}: spot is for on, in or under: left|center|right and/or front|middle|back joined with \"-\""
                ));
            }
        }
        if matches!(r.how.as_str(), "held" | "worn") {
            if kind(&r.to) != Some("character") {
                return Err(format!("{at}: only a character holds or wears a piece"));
            }
            if !p.place.trim().is_empty() {
                return Err(format!(
                    "{at}: a held or worn piece has no place of its own"
                ));
            }
        }
    }
    // A conversation runs one way: no line answers itself through others.
    for it in &plan.items {
        let mut at = it.id.as_str();
        for _ in 0..=plan.items.len() {
            let next = plan
                .items
                .iter()
                .find(|i| i.id == at)
                .and_then(|i| i.details.replies_to.as_deref());
            match next {
                Some(n) if n == it.id => {
                    return Err(format!("item {}: replies_to leads back to itself", it.id));
                }
                Some(n) => at = n,
                None => break,
            }
        }
    }
    // A piece never rests (through rel or attach_to) on itself.
    let target = |id: &str| {
        let p = plan.layout.iter().find(|p| p.item == id);
        p.and_then(|p| p.rel.as_ref().map(|r| r.to.clone()))
            .or_else(|| p.and_then(|p| p.attach_to.as_ref().map(|a| a.item.clone())))
            .or_else(|| {
                let it = plan.items.iter().find(|i| i.id == id)?;
                it.details.attach_to.as_ref().map(|a| a.item.clone())
            })
    };
    for p in &plan.layout {
        let mut at = p.item.clone();
        for _ in 0..=plan.items.len() {
            match target(&at) {
                Some(t) if t == p.item => {
                    return Err(format!("layout {}: its rel leads back to itself", p.item));
                }
                Some(t) => at = t,
                None => break,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::productions::parse_plan_reply;
    use serde_json::{json, Value};

    /// Breakfast as a planner writes it now: the pancakes on their plate on
    /// the table, the cat under it, the spoon held, the apron worn.
    fn breakfast() -> Value {
        let it = |id: &str, kind: &str| json!({"id": id, "kind": kind, "name": id, "description": format!("The {id}."), "enabled": true, "details": {"prompt": id}});
        let mut gran = it("grandma", "character");
        gran["details"]["holds"] = json!([{"item": "spoon", "socket": "right_hand"}]);
        gran["details"]["body"] = json!("two_legs");
        let mut cat = it("cat", "character");
        cat["details"]["body"] = json!("four_legs");
        let mut table = it("table", "object");
        table["details"]["dims_m"] = json!([1.2, 0.75, 1.2]);
        json!({
            "title": "Breakfast",
            "items": [gran, cat, table, it("plate", "object"), it("pancakes", "object"), it("spoon", "object"), it("apron", "object")],
            "layout": [
                {"item": "table", "place": "center"},
                {"item": "plate", "rel": {"to": "table", "how": "on", "spot": "front-left"}},
                {"item": "pancakes", "rel": {"to": "plate", "how": "on"}},
                {"item": "cat", "rel": {"to": "table", "how": "under", "spot": "center"}},
                {"item": "grandma", "rel": {"to": "table", "how": "beside", "side": "right"}, "facing": "table"},
                {"item": "spoon", "rel": {"to": "grandma", "how": "held", "hand": "right_hand"}},
                {"item": "apron", "rel": {"to": "grandma", "how": "worn", "part": "hip"}}
            ]
        })
    }

    fn refused(f: impl FnOnce(&mut Value)) -> String {
        let mut v = breakfast();
        f(&mut v);
        parse_plan_reply(&v.to_string()).unwrap_err()
    }

    #[test]
    fn a_featured_performance_survives_plan_validation() {
        let mut v = breakfast();
        v["items"].as_array_mut().unwrap().push(json!({
            "id":"jazz", "kind":"music", "name":"Jazz", "description":"Fast swing.",
            "enabled":true, "details":{"prompt":"Play fast swing.", "seconds":48, "featured":true}
        }));
        let (plan, _) = parse_plan_reply(&v.to_string()).unwrap();
        assert_eq!(plan.items.last().unwrap().details.featured, Some(true));
        assert!(
            refused(|v| v["items"][0]["details"]["featured"] = json!(true))
                .contains("for a music performance")
        );
    }

    #[test]
    fn structured_relations_and_shapes_pass() {
        let (plan, _) = parse_plan_reply(&breakfast().to_string()).expect("valid");
        let r = plan.layout[1].rel.as_ref().unwrap();
        assert_eq!(
            (r.to.as_str(), r.how.as_str(), r.spot.as_deref()),
            ("table", "on", Some("front-left"))
        );
        assert!(plan.layout[1].place.is_empty());
        assert_eq!(plan.items[2].details.dims_m, Some([1.2, 0.75, 1.2]));
        assert_eq!(plan.items[1].details.body.as_deref(), Some("four_legs"));
    }

    #[test]
    fn prose_mixed_fields_and_double_places_are_refused() {
        assert!(
            refused(|v| v["layout"][1]["rel"]["how"] = json!("on top of"))
                .contains("how must be one of")
        );
        assert!(
            refused(|v| v["layout"][1]["rel"]["spot"] = json!("front left of the tabletop"))
                .contains("spot is for on")
        );
        assert!(
            refused(|v| v["layout"][1]["rel"]["spot"] = json!("left-right"))
                .contains("spot is for on")
        );
        assert!(
            refused(|v| v["layout"][4]["rel"]["spot"] = json!("front")).contains("spot is for on")
        );
        assert!(
            refused(|v| v["layout"][4]["rel"]["hand"] = json!("right_hand"))
                .contains("hand is only for how held")
        );
        assert!(
            refused(|v| v["layout"][5]["rel"]["hand"] = json!("right palm"))
                .contains("must be one of")
        );
        assert!(
            refused(|v| v["layout"][6]["rel"]["part"] = json!("waist")).contains("must be one of")
        );
        assert!(
            refused(|v| v["layout"][5]["place"] = json!("front")).contains("no place of its own")
        );
        assert!(
            refused(|v| v["layout"][6]["rel"]["to"] = json!("table")).contains("only a character")
        );
        assert!(refused(|v| v["layout"][1]["rel"]["to"] = json!("plate")).contains("another"));
        // Clothing is worn, never held.
        assert!(refused(
            |v| v["items"][0]["details"]["holds"] = json!([{"item": "apron", "socket": "neck"}])
        )
        .contains("is worn"));
        // A piece never rests on itself through others.
        assert!(refused(|v| v["layout"][0] =
            json!({"item": "table", "rel": {"to": "pancakes", "how": "under"}}))
        .contains("back to itself"));
        assert!(
            refused(|v| v["items"][2]["details"]["body"] = json!("four_legs"))
                .contains("for a character")
        );
        assert!(
            refused(|v| v["items"][1]["details"]["body"] = json!("paws"))
                .contains("for a character")
        );
        assert!(
            refused(|v| v["items"][2]["details"]["dims_m"] = json!([1.2, 0, 1.2]))
                .contains("dims_m")
        );
        // A piece with neither a place nor a rel is not placed.
        assert!(refused(|v| v["layout"][0] = json!({"item": "table"}))
            .contains("or the piece has a rel"));
    }

    /// A conversation: each line's pace, mood and the line it answers.
    #[test]
    fn voice_lines_carry_pace_mood_and_what_they_answer() {
        let line = |id: &str, replies: Option<&str>| {
            let mut l = json!({"id": id, "kind": "voice_line", "name": id, "description": "A line.", "enabled": true,
                "details": {"speaker": "grandma", "text": "Pancakes are ready!", "pace": "normal", "mood": "warm"}});
            if let Some(r) = replies {
                l["details"]["replies_to"] = json!(r);
            }
            l
        };
        let with = |a: Value, b: Value| {
            let mut v = breakfast();
            v["items"].as_array_mut().unwrap().extend([a, b]);
            v
        };
        let ok = with(line("hello", None), line("meow", Some("hello")));
        let (plan, _) = parse_plan_reply(&ok.to_string()).expect("valid");
        let meow = plan.items.iter().find(|i| i.id == "meow").unwrap();
        assert_eq!(meow.details.replies_to.as_deref(), Some("hello"));
        assert_eq!(meow.details.pace.as_deref(), Some("normal"));
        let bad = |v: Value| parse_plan_reply(&v.to_string()).unwrap_err();
        let mut fast = line("hello", None);
        fast["details"]["pace"] = json!("quick-ish");
        assert!(bad(with(fast, line("meow", None))).contains("pace"));
        assert!(bad(with(
            line("hello", Some("meow")),
            line("meow", Some("hello"))
        ))
        .contains("back to itself"));
        assert!(bad(with(line("hello", Some("table")), line("meow", None)))
            .contains("another voice line"));
        let mut v = breakfast();
        v["items"][2]["details"]["mood"] = json!("happy");
        assert!(bad(v).contains("for a voice line"));
    }

    /// An older plan's free-prose attach_to still loads.
    #[test]
    fn an_old_attach_to_still_loads() {
        let mut v = breakfast();
        v["layout"][1] = json!({"item": "plate", "place": "center", "attach_to": {"item": "table", "where": "front left of the tabletop"}});
        v["layout"][2] =
            json!({"item": "pancakes", "attach_to": {"item": "plate", "where": "top"}});
        let (plan, _) = parse_plan_reply(&v.to_string()).expect("an old plan loads");
        assert!(plan.layout[1].rel.is_none());
        assert_eq!(
            plan.layout[1].attach_to.as_ref().unwrap().at,
            "front left of the tabletop"
        );
    }
}
