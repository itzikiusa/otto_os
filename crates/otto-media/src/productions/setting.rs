//! The SETTING of a production: where and when its scene is (a pier over the
//! sea at night in fog, a kitchen in the morning), worked out by the planner
//! from the description even when it is only implied, in closed words so it
//! can be checked (`plan.setting`, docs/contracts/api.md). A plan without
//! one builds exactly as before; with one, [`build_scene`] dresses the scene
//! [`scene::build_scene`] laid out:
//!
//! - BACKDROP: a piece placed `far` (`far-left`, `far-right`; a lighthouse behind
//!   the pier) is left out of the layout and stands behind the group as the
//!   camera sees it: the camera, lowered to 12°, frames the group as before,
//!   and the piece stands far enough back that its top stays in the picture
//!   (perspective makes it smaller), on a low grassy far shore when the
//!   ground is water. Indoors it stands against the back of the room.
//! - GROUND: the ground plane goes; `environment.ground` names its kind, centre
//!   and size (far past the group outdoors, so the fog hides its edge; the
//!   room's floor indoors, with two walls behind the group). On WATER the
//!   base piece (the largest free object: a pier, a boat) stands IN it, the
//!   surface at 40 % of its height, above its feet and below its deck.
//! - SKY, FOG, LIGHT: sky colours, fog, sun or moon and sky light from the
//!   time and weather; a warm (or cool) point light with a halo (`glow`) at
//!   each glowing item, at its middle or near the top of a tall one (a
//!   lighthouse's lamp).
//!
//! Steering a setting ("make it daytime", "no fog") changes no item, so
//! nothing is made again and the scene is rebuilt at once; [`said`] puts the
//! change in plain words.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::scene::{self, size_m, Bounds, Model};
use super::{Item, Plan};

pub const GROUNDS: &[&str] = &[
    "water",
    "grass",
    "meadow",
    "sand",
    "soil",
    "stone",
    "snow",
    "moon_dust",
    "sea_floor",
    "wood_floor",
    "carpet",
    "tiles",
    "stage",
    "road",
    "none",
];
/// Where the scene is: on Earth (under its sky), in space (a black sky, no
/// air), or under water (blue light, the sea floor).
pub const WORLDS: &[&str] = &["earth", "space", "underwater"];
/// What can be seen in the sky.
pub const SKY_THINGS: &[&str] = &["sun", "moon", "earth", "planet", "stars"];
pub const TIMES: &[&str] = &["day", "dusk", "night", "dawn"];
pub const WEATHERS: &[&str] = &["clear", "fog", "rain", "snow"];
pub const GLOW_COLORS: &[&str] = &["warm", "cool"];
/// Pieces placed "wall" that hang on it (else they stand at it).
const HANGS: &[&str] = &[
    "window", "painting", "picture", "poster", "clock", "mirror", "frame",
];
/// Pieces a road runs past (the viewer draws its 7 m strip, `ROAD_WIDTH` in setting.ts).
const VEHICLES: &[&str] = &[
    "car", "truck", "bus", "van", "jeep", "taxi", "lorry", "tractor",
];
const ROAD_WIDTH: f64 = 7.0;
pub const STRENGTHS: &[&str] = &["low", "medium", "high"];
const MAX_LIGHTS: usize = 8;
/// How far the camera looks down on a scene with backdrop pieces (degrees).
const PITCH: f64 = 12.0;
/// The highest a room's camera looks down at the action (degrees).
const ROOM_PITCH: f64 = 25.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Setting {
    /// One plain sentence: "A wooden pier over the sea at night, in fog."
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub ground: String,
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub weather: String,
    #[serde(default)]
    pub indoor: bool,
    /// The items that glow (a lantern, a campfire, a lamp).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lights: Vec<Glow>,
    /// earth (the default), space or underwater.
    #[serde(default, skip_serializing_if = "is_earth")]
    pub world: String,
    /// What the sky shows ([`SKY_THINGS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_sky: Vec<String>,
}

fn is_earth(w: &String) -> bool {
    w.is_empty() || w == "earth"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Glow {
    pub item: String,
    #[serde(default = "warm", alias = "colour")]
    pub color: String,
    #[serde(default = "medium")]
    pub strength: String,
}

fn warm() -> String {
    "warm".into()
}
fn medium() -> String {
    "medium".into()
}

/// The planner's rules for the setting (after the plan rules).
pub const RULES: &str = r#"The setting: where and when the scene is. Write it as plan.setting:
{"summary": one plain sentence ("A wooden pier over the sea at night, in thick fog."),
 "ground": water|grass|meadow|sand|soil|stone|snow|moon_dust|sea_floor|wood_floor|carpet|tiles|stage|road|none,
 "time": day|dusk|night|dawn, "weather": clear|fog|rain|snow, "indoor": true|false,
 "world"?: earth|space|underwater (default earth), "in_sky"?: [sun|moon|earth|planet|stars],
 "lights": [{"item": <id of an item that glows>, "color": warm|cool, "strength": low|medium|high}]}
- The setting IS the place: the ground, the room's floor and walls, the sky, the time, the weather,
  the world. Never make the place an item (no "Desert road", "Green meadow", "Bedroom", "Moon
  surface", "Sea floor", "Kitchen" or "Earth in the sky" items): a road is ground road, a meadow
  ground meadow, a room is indoor with its floor (wood_floor, carpet, tiles; a theatre stage floor
  is stage), the moon's surface is ground moon_dust with world space, the sea floor is ground
  sea_floor with world underwater, the Earth or the Moon in the sky is in_sky.
- Work the place out from the description even when it is only implied: a pier, a boat or a harbour
  is on water; a campfire is outdoors on soil; a forest floor is soil or grass; a beach is sand; a
  garden or a field is grass or meadow; a kitchen table or a bedroom is indoors; a road is road
  (Otto draws it behind a car, a truck or a bus on its verge, so it is stopped at the side of the
  road, never across it).
  "none" is only for an empty void.
- Time and weather come from the words: night, dark, moonlit is night; evening, sunset is dusk;
  morning, sunrise is dawn; noon, sunny is day; foggy, misty is fog; rain, storm is rain; snowing is
  snow. A campfire with no time said is dusk. Nothing said: day and clear. Indoors and in space or
  under water the weather is clear. A clear night sky has stars.
- Only the ground, sky, walls, lighting and planned pieces appear in the picture. A name in
  setting.summary alone is invisible. If the requested place would still look like an empty
  floor or plain landscape, include one recognizable physical feature at its edge. For example,
  a ticket machine can identify a station platform even when snow covers the ground. Use a piece
  already in the plan if it identifies the place. Honor requests for a bare or empty setting.
  Other physical features implied by the description can be items too: a lighthouse behind a lighthouse keeper's pier, a boat at a harbour, a blackboard in a
  classroom, trees around a forest camp (up to three trees, "far-left", "far" and "far-right": a
  ring of trees is never one item), a window in a sunny kitchen (place "wall"). At most three, never against
  the text, each with its real size (details.size_m: a lighthouse 15-25, a tree 6-12, a house 8) and
  its place: "far" (or "far-left", "far-right") for one in the background, a normal place or a rel
  for one that is part of the scene. Say each in plan.notes in one plain sentence that starts
  "I added" and says why ("I added a lighthouse behind the pier because he is its keeper."), so the
  person can take it out.
- lights names every item that glows or gives light (a lantern, a campfire, a lamp, a candle, a lit
  window, a spotlight, a lighthouse at night): warm for fire and bulbs, cool for screens, ice and
  magic; strength low for a candle, medium for a lantern or a lamp, high for a campfire, a spotlight
  or a lighthouse. Only items.
- Otto draws the setting (water, ground, sky, the Sun, Moon, Earth and stars, time of day, fog, rain,
  snow, room walls, under water, the glow): never write a note that Otto cannot show night, fog,
  water, the sky or the place."#;

/// The steering rule for the setting (after the other steering rules).
pub const STEER: &str =
    "A change to where or when (\"make it daytime\", \"no fog\", \"put it on a \
beach\") changes plan.setting and keeps every item as it is: add no item for it unless asked, and \
take out an item you added for the setting only when the person asks.";

/// The setting check (part of [`super::validate_plan`]).
pub fn validate(plan: &Plan) -> Result<(), String> {
    let Some(s) = &plan.setting else {
        return Ok(());
    };
    if !super::text_ok(&s.summary, 200) {
        return Err("setting.summary is one plain sentence of 1-200 characters".into());
    }
    for (name, v, allowed) in [
        ("ground", &s.ground, GROUNDS),
        ("time", &s.time, TIMES),
        ("weather", &s.weather, WEATHERS),
    ] {
        if !allowed.contains(&v.as_str()) {
            return Err(format!(
                "setting.{name} must be one of {}",
                allowed.join(", ")
            ));
        }
    }
    if !is_earth(&s.world) && !WORLDS.contains(&s.world.as_str()) {
        return Err(format!(
            "setting.world must be one of {}",
            WORLDS.join(", ")
        ));
    }
    let mut sky = HashSet::new();
    if s.in_sky
        .iter()
        .any(|t| !SKY_THINGS.contains(&t.as_str()) || !sky.insert(t))
    {
        return Err(format!(
            "setting.in_sky lists each of {} at most once",
            SKY_THINGS.join(", ")
        ));
    }
    if s.lights.len() > MAX_LIGHTS {
        return Err(format!("setting.lights: at most {MAX_LIGHTS}"));
    }
    let mut seen = HashSet::new();
    for g in &s.lights {
        let kind = plan
            .items
            .iter()
            .find(|i| i.id == g.item)
            .map(|i| i.kind.as_str());
        if !matches!(kind, Some("character" | "object")) || !seen.insert(g.item.as_str()) {
            return Err(format!(
                "setting.lights: {:?} must name a character or object item, once",
                g.item
            ));
        }
        if !GLOW_COLORS.contains(&g.color.as_str()) || !STRENGTHS.contains(&g.strength.as_str()) {
            return Err(format!(
                "setting.lights {}: color is warm or cool, strength low, medium or high",
                g.item
            ));
        }
    }
    Ok(())
}

/// What a steering round did to the setting, one plain sentence a change
/// (the revision's `done`).
pub fn said(old: &Plan, new: &Plan) -> Vec<String> {
    let Some(n) = &new.setting else {
        return Vec::new();
    };
    let o = old.setting.clone().unwrap_or_default();
    let mut out = Vec::new();
    if o.time != n.time {
        out.push(match n.time.as_str() {
            "day" => "It is daytime now.",
            "dusk" => "It is evening now.",
            "dawn" => "It is early morning now.",
            _ => "It is night now.",
        });
    }
    if o.weather != n.weather {
        out.push(match (n.weather.as_str(), o.weather.as_str()) {
            ("fog", _) => "There is fog now.",
            ("rain", _) => "It is raining now.",
            ("snow", _) => "It is snowing now.",
            (_, "fog") => "The fog is gone.",
            (_, "rain") => "The rain has stopped.",
            (_, "snow") => "The snow has stopped.",
            _ => "The sky is clear now.",
        });
    }
    if is_earth(&o.world) != is_earth(&n.world) || (!is_earth(&n.world) && o.world != n.world) {
        out.push(match n.world.as_str() {
            "space" => "It is in space now.",
            "underwater" => "It is under water now.",
            _ => "It is back on Earth now.",
        });
    }
    if o.indoor != n.indoor {
        out.push(if n.indoor {
            "It is indoors now."
        } else {
            "It is outdoors now."
        });
    }
    if o.ground != n.ground {
        out.push(match n.ground.as_str() {
            "water" => "It stands in water now.",
            "grass" => "It stands on grass now.",
            "sand" => "It stands on sand now.",
            "soil" => "It stands on earth now.",
            "stone" => "It stands on stone now.",
            "snow" => "It stands in snow now.",
            "wood_floor" => "It stands on a wooden floor now.",
            "tiles" => "It stands on a tiled floor now.",
            "road" => "It stands on a road now.",
            "meadow" => "It stands in a meadow now.",
            "moon_dust" => "It stands on moon dust now.",
            "sea_floor" => "It stands on the sea floor now.",
            "carpet" => "It stands on a carpet now.",
            "stage" => "It stands on a stage now.",
            _ => "It floats in empty space now.",
        });
    }
    let mut out: Vec<String> = out.into_iter().map(str::to_string).collect();
    let name = |id: &str| {
        new.items
            .iter()
            .chain(&old.items)
            .find(|i| i.id == id)
            .map_or(id.to_string(), |i| i.name.clone())
    };
    let lit = |s: &Setting, id: &str| s.lights.iter().any(|g| g.item == id);
    for g in &n.lights {
        if !lit(&o, &g.item) {
            out.push(format!("{} glows now.", name(&g.item)));
        }
    }
    for g in &o.lights {
        if !lit(n, &g.item) {
            out.push(format!("{} does not glow now.", name(&g.item)));
        }
    }
    out
}

/// The setting a revision replaced, kept so a revert puts it back (an empty
/// setting stands for "none"); `None` when it did not change.
pub fn before(old: &Plan, new: &Plan) -> Option<Setting> {
    (old.setting != new.setting).then(|| old.setting.clone().unwrap_or_default())
}

/// A revert's setting from what [`before`] kept.
pub fn restore(kept: Setting) -> Option<Setting> {
    (kept != Setting::default()).then_some(kept)
}

// ---------------------------------------------------------------------------
// The scene
// ---------------------------------------------------------------------------

fn place_words(plan: &Plan, id: &str) -> Vec<String> {
    plan.layout
        .iter()
        .find(|p| p.item == id)
        .map(|p| {
            p.place
                .split(['-', ' ', '_', '+'])
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A backdrop piece (`far`), seen but never framed. One that something else
/// stands on, is in or is held by (the owl's branch on a far oak) is part of
/// the scene, laid out with it.
fn is_far(plan: &Plan, id: &str) -> bool {
    place_words(plan, id).iter().any(|w| w == "far")
        && !plan
            .layout
            .iter()
            .any(|p| rests_on(plan, &p.item).as_deref() == Some(id))
}

/// The scene document of [`scene::build_scene`] dressed with the plan's
/// setting (module doc); without one, exactly that scene.
pub fn build_scene(plan: &Plan, models: &[Model]) -> (Value, Vec<String>) {
    let Some(set) = &plan.setting else {
        return scene::build_scene(plan, models);
    };
    let near: Vec<Model> = models
        .iter()
        .filter(|m| !is_far(plan, &m.item))
        .cloned()
        .collect();
    let (back, near): (Vec<&Model>, &[Model]) = match near.is_empty() {
        true => (Vec::new(), models),
        false => (
            models.iter().filter(|m| is_far(plan, &m.item)).collect(),
            &near,
        ),
    };
    let (mut doc, notes) = scene::build_scene(plan, near);
    dress(plan, set, near, &back, &mut doc);
    (doc, notes)
}

fn r3(v: [f64; 3]) -> [f64; 3] {
    v.map(|x| (x * 1000.0).round() / 1000.0)
}

/// What a model measures in the scene: its size rule's scale (a library piece
/// keeps its own size unless the plan gives one), as in [`scene::build_scene`].
fn scale_of(plan: &Plan, it: Option<&Item>, m: &Model) -> f64 {
    let b = &m.bounds;
    let size = |k: usize| (b.max[k] - b.min[k]).max(0.01);
    match it {
        Some(it) if !m.real_size || it.details.size_m.is_some() => {
            let own = match it.kind.as_str() {
                "character" => size(1),
                _ => size(0).max(size(1)).max(size(2)),
            };
            size_m(plan, it) / own
        }
        _ => 1.0,
    }
}

/// A point on an upright scene object, through its scale, yaw and position.
pub(super) fn upright_transform(obj: &Value) -> Option<impl Fn([f64; 3]) -> [f64; 3]> {
    let p: [f64; 3] = serde_json::from_value(obj["position"].clone()).ok()?;
    // Per axis: a piece the plan gives dims_m is scaled toward them.
    let s = [0, 1, 2].map(|d| obj["scale"][d].as_f64().unwrap_or(1.0));
    let t = obj["rotation"][1].as_f64().unwrap_or(0.0).to_radians();
    let (sn, cs) = t.sin_cos();
    Some(move |point: [f64; 3]| {
        let [x, y, z] = [0, 1, 2].map(|k| point[k] * s[k]);
        [p[0] + x * cs + z * sn, p[1] + y, p[2] - x * sn + z * cs]
    })
}

pub(super) fn world_box(obj: &Value, b: &Bounds) -> Option<Bounds> {
    let world = upright_transform(obj)?;
    let mut out: Option<Bounds> = None;
    for k in 0..8 {
        let w = world([0, 1, 2].map(|d| if k >> d & 1 == 0 { b.min[d] } else { b.max[d] }));
        out = Some(match out {
            None => Bounds { min: w, max: w },
            Some(o) => Bounds {
                min: [0, 1, 2].map(|d| o.min[d].min(w[d])),
                max: [0, 1, 2].map(|d| o.max[d].max(w[d])),
            },
        });
    }
    out
}

fn union(boxes: &[Bounds]) -> Bounds {
    boxes
        .iter()
        .copied()
        .reduce(|a, b| Bounds {
            min: [0, 1, 2].map(|d| a.min[d].min(b.min[d])),
            max: [0, 1, 2].map(|d| a.max[d].max(b.max[d])),
        })
        .unwrap_or(Bounds::UNIT)
}

fn mid(b: &Bounds) -> [f64; 3] {
    [0, 1, 2].map(|k| (b.min[k] + b.max[k]) / 2.0)
}

/// The doc camera's `position` or `target`.
fn camera_at(doc: &Value, k: &str, or: [f64; 3]) -> [f64; 3] {
    serde_json::from_value(doc["camera"][k].clone()).unwrap_or(or)
}

fn hex(c: &str) -> [f64; 3] {
    let n = u32::from_str_radix(c.trim_start_matches('#'), 16).unwrap_or(0);
    [16, 8, 0].map(|s| ((n >> s) & 0xff) as f64)
}

/// `a` toward `b` by `t` (hex colours).
fn mix(a: &str, b: &str, t: f64) -> String {
    let (a, b) = (hex(a), hex(b));
    let c = [0, 1, 2].map(|k| (a[k] + (b[k] - a[k]) * t).round().clamp(0.0, 255.0) as u32);
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// The look of a time and weather: sky (zenith, horizon, below), the
/// environment preset whose highlights suit it, its light (IBL) strength,
/// the sky light (colour, ground colour, strength) and the sun or moon
/// (colour, strength, height in degrees).
struct Look {
    sky: [String; 3],
    preset: &'static str,
    ibl: f64,
    hemi: (&'static str, &'static str, f64),
    sun: (&'static str, f64, f64),
}

fn look(set: &Setting) -> Look {
    let s = |a: &str, b: &str, c: &str| [a.to_string(), b.to_string(), c.to_string()];
    let mut l = match set.time.as_str() {
        "night" => Look {
            sky: s("#04060e", "#141c33", "#020308"),
            preset: "night",
            ibl: 0.5,
            hemi: ("#6f80b8", "#1a1e2c", 0.9),
            sun: ("#b8c8ff", 1.1, 40.0),
        },
        "dusk" => Look {
            sky: s("#2c2c5c", "#ef9466", "#3a2a35"),
            preset: "sunset",
            ibl: 0.6,
            hemi: ("#ffb48a", "#3a2f45", 0.5),
            sun: ("#ff9a5a", 1.4, 10.0),
        },
        "dawn" => Look {
            sky: s("#6f86c6", "#f6c29a", "#5a4c55"),
            preset: "sunset",
            ibl: 0.65,
            hemi: ("#ffd7c2", "#4a4256", 0.55),
            sun: ("#ffb88a", 1.4, 12.0),
        },
        _ => Look {
            sky: s("#4f8fd8", "#cfe2f2", "#8f9aa0"),
            preset: "studio-soft",
            ibl: 0.9,
            hemi: ("#d6e8ff", "#8a7f6a", 1.0),
            sun: ("#fff4e0", 2.4, 55.0),
        },
    };
    match set.world.as_str() {
        // No air: a black sky, a hard white sun, almost no sky light.
        "space" => {
            l.sky = s("#000000", "#06070b", "#000000");
            l.preset = "night";
            l.ibl = 0.12;
            l.hemi = ("#9aa0b0", "#202020", 0.12);
            l.sun = ("#ffffff", 3.0, 25.0);
            return l;
        }
        // Blue light from above, the water's blue all round.
        "underwater" => {
            l.sky = s("#0a3a5c", "#1f6f94", "#06263d");
            l.preset = "studio-soft";
            l.ibl = 0.55;
            l.hemi = ("#7fd0ff", "#0b3550", 0.8);
            l.sun = ("#bfe8ff", 1.4, 70.0);
            return l;
        }
        _ => {}
    }
    if set.indoor {
        // A room: walls, not sky; light through a window by day, little at night.
        let night = set.time == "night";
        l.sky = match night {
            true => s("#14120f", "#1d1a16", "#0c0b09"),
            false => s("#e9e1d4", "#cfc3b2", "#8d7e6c"),
        };
        l.preset = "studio-soft";
        l.ibl = if night { 0.32 } else { 0.7 };
        l.hemi = ("#fff1dc", "#6b5a48", if night { 0.38 } else { 0.9 });
        l.sun = ("#fff2dc", if night { 0.0 } else { 1.4 }, 35.0);
        return l;
    }
    let grey = |l: &mut Look, fog: &str, keep: f64, sun: f64| {
        l.sky = [
            mix(&l.sky[0], fog, 0.6),
            fog.to_string(),
            mix(&l.sky[2], fog, 0.5),
        ];
        l.sun.1 *= sun;
        l.hemi.2 *= keep;
    };
    let night = set.time == "night";
    match set.weather.as_str() {
        "fog" => {
            let fog = match set.time.as_str() {
                "night" => "#1e2535",
                "dusk" => "#8f7480",
                "dawn" => "#d6c3b8",
                _ => "#c3c9cf",
            };
            grey(&mut l, fog, 1.1, 0.45);
        }
        "rain" => grey(&mut l, if night { "#11161f" } else { "#9aa3ab" }, 0.9, 0.3),
        "snow" => grey(&mut l, if night { "#1a2030" } else { "#e3e8ee" }, 1.2, 0.5),
        _ => {}
    }
    l
}

/// Writes the setting into `doc` (module doc).
/// The piece `id` stands on, is in or is held by (its `rel`, its older
/// `attach_to`, or the character that holds it).
fn rests_on(plan: &Plan, id: &str) -> Option<String> {
    anchor_for(plan, id, |_| true)
}

fn anchor_for(plan: &Plan, id: &str, accepts: impl Fn(&str) -> bool) -> Option<String> {
    let at = plan.layout.iter().find(|p| p.item == id);
    if let Some(rel) = at.and_then(|p| p.rel.as_ref()) {
        return accepts(&rel.how).then(|| rel.to.clone());
    }
    at.and_then(|p| p.attach_to.as_ref().map(|a| a.item.clone()))
        .or_else(|| {
            let it = plan.items.iter().find(|i| i.id == id)?;
            it.details.attach_to.as_ref().map(|a| a.item.clone())
        })
        .or_else(|| {
            let holder = plan
                .items
                .iter()
                .find(|c| c.details.holds.iter().any(|h| h.item == id));
            holder.map(|c| c.id.clone())
        })
}

/// How far a piece goes up out of the water: what it rests on goes up, and it
/// with that; then, still under the surface, up to it. The base piece stays.
fn lift(
    plan: &Plan,
    boxes: &BTreeMap<String, Bounds>,
    base: Option<&str>,
    level: f64,
    id: &str,
    ups: &mut BTreeMap<String, f64>,
) -> f64 {
    if let Some(up) = ups.get(id) {
        return *up;
    }
    // Marked first: a loop of relations (refused by the plan check) ends here.
    ups.insert(id.to_string(), 0.0);
    let (Some(b), false) = (boxes.get(id), base == Some(id)) else {
        return 0.0;
    };
    let under = rests_on(plan, id)
        .filter(|to| boxes.contains_key(to))
        .map_or(0.0, |to| lift(plan, boxes, base, level, &to, ups));
    let up = under + (level - (b.min[1] + under)).max(0.0);
    ups.insert(id.to_string(), up);
    up
}

/// A room's floor (its middle and size): sized to what stands in it. A
/// window, a piece placed "wall" and a free piece placed at the back stand
/// against its back wall (the far side from the camera, where the viewer
/// draws it); a window, a painting, a clock or a mirror hangs on it.
fn room(
    plan: &Plan,
    boxes: &mut BTreeMap<String, Bounds>,
    doc: &mut Value,
    cam: [f64; 3],
    all: Bounds,
) -> ([f64; 3], [f64; 2]) {
    let item = |id: &str| plan.items.iter().find(|i| i.id == id);
    let free =
        |id: &str| item(id).is_some_and(|i| i.kind == "object") && rests_on(plan, id).is_none();
    let named = |id: &str, words: &[&str]| {
        item(id).is_some_and(|i| {
            let w = scene::words(&i.name);
            (words.iter()).any(|h| w.iter().any(|x| Some(x) == scene::words(h).first()))
        })
    };
    let walled: Vec<String> = (boxes.keys())
        .filter(|id| {
            let p = place_words(plan, id);
            named(id, &["window"])
                || p.iter().any(|w| w == "wall")
                || (free(id) && p.iter().any(|w| w == "back"))
        })
        .cloned()
        .collect();
    // Descendants move with the fixture. Their provisional depth must not
    // push the back wall away when the fixture is already counted there.
    let wall_parent = |id: &str| {
        let mut at = id.to_owned();
        for _ in 0..8 {
            if let Some(parent) = walled.iter().find(|w| **w == at) {
                return Some(parent);
            }
            at = rests_on(plan, &at)?;
        }
        None
    };
    // The room is sized to what stands in it: a metre round the group, its
    // back wall just behind it with room for what stands there.
    let rest: Vec<Bounds> = (boxes.iter())
        .filter(|(id, _)| wall_parent(id).is_none())
        .map(|(_, b)| *b)
        .collect();
    let r = if rest.is_empty() { all } else { union(&rest) };
    let deep = (walled.iter().filter_map(|id| boxes.get(id)))
        .map(|b| b.max[2] - b.min[2])
        .fold(0.0, f64::max);
    let ahead = cam[2] >= mid(&r)[2];
    let (lo, hi) = match ahead {
        true => (r.min[2] - deep - 0.4, r.max[2] + 1.5),
        false => (r.min[2] - 1.5, r.max[2] + deep + 0.4),
    };
    // A wall hanging can share the horizontal span of the furniture below it.
    // The initial free-object grid has no walls and often puts one metres away.
    let mut hanging = walled
        .iter()
        .filter(|id| {
            named(id, HANGS)
                && !place_words(plan, id)
                    .iter()
                    .any(|w| matches!(w.as_str(), "left" | "right"))
        })
        .collect::<Vec<_>>();
    hanging.sort_by(|a, b| mid(&boxes[*a])[0].total_cmp(&mid(&boxes[*b])[0]));
    let total = hanging
        .iter()
        .map(|id| boxes[*id].max[0] - boxes[*id].min[0])
        .sum::<f64>()
        + 0.18 * hanging.len().saturating_sub(1) as f64;
    let mut next = mid(&r)[0] - total / 2.;
    let mut horizontal = BTreeMap::new();
    for id in hanging {
        let b = &boxes[id];
        horizontal.insert(id.as_str(), next - b.min[0]);
        next += b.max[0] - b.min[0] + 0.18;
    }
    // Pinning two floor-standing fixtures to one wall can bring previously
    // separate footprints into the same depth range. Keep their left-to-right order.
    let mut furniture: Vec<_> = walled
        .iter()
        .filter(|id| !named(id, HANGS) && rests_on(plan, id).is_none())
        .collect();
    furniture.sort_by(|a, b| mid(&boxes[*a])[0].total_cmp(&mid(&boxes[*b])[0]));
    let mut right = f64::NEG_INFINITY;
    for id in furniture {
        let b = &boxes[id];
        let dx = (right - b.min[0]).max(0.0);
        horizontal.insert(id.as_str(), dx);
        right = b.max[0] + dx + 0.08;
    }
    let x0 = boxes
        .iter()
        .map(|(id, b)| b.min[0] + horizontal.get(id.as_str()).copied().unwrap_or_default())
        .fold(f64::MAX, f64::min)
        - 1.;
    let x1 = boxes
        .iter()
        .map(|(id, b)| b.max[0] + horizontal.get(id.as_str()).copied().unwrap_or_default())
        .fold(f64::MIN, f64::max)
        + 1.;
    let (floor_mid, floor_size) = ([(x0 + x1) / 2.0, 0.0, (lo + hi) / 2.0], [x1 - x0, hi - lo]);
    let back = if ahead { -1.0 } else { 1.0 };
    let wall = floor_mid[2] + back * floor_size[1] / 2.0;
    let moves: BTreeMap<String, [f64; 3]> = (walled.iter())
        .map(|id| {
            let b = boxes[id];
            let dz = wall - if back < 0.0 { b.min[2] } else { b.max[2] };
            let dy = match named(id, HANGS) {
                true => (1.0 - b.min[1]).max(0.0),
                false => 0.0,
            };
            (
                id.clone(),
                [
                    horizontal.get(id.as_str()).copied().unwrap_or_default(),
                    dy,
                    dz,
                ],
            )
        })
        .collect();
    let moved = |id: &str| moves.get(wall_parent(id)?).copied();
    for o in doc["objects"].as_array_mut().into_iter().flatten() {
        let Some(id) = o["id"].as_str().map(str::to_string) else {
            continue;
        };
        let Some([dx, dy, dz]) = moved(&id) else {
            continue;
        };
        // Its box moves too: a window's light glows at the wall, not where it stood.
        if let Some(b) = boxes.get_mut(&id) {
            b.min[0] += dx;
            b.max[0] += dx;
            (b.min[1], b.max[1], b.min[2], b.max[2]) =
                (b.min[1] + dy, b.max[1] + dy, b.min[2] + dz, b.max[2] + dz);
        }
        let p = &mut o["position"];
        p[0] = json!(((p[0].as_f64().unwrap_or_default() + dx) * 1000.).round() / 1000.);
        p[1] = json!(r3([0.0, p[1].as_f64().unwrap_or(0.0) + dy, 0.0])[1]);
        p[2] = json!(r3([0.0, 0.0, p[2].as_f64().unwrap_or(0.0) + dz])[2]);
    }
    (floor_mid, floor_size)
}

/// Run the lane along the longest parked vehicle and put it beside the
/// vehicles on the far side from the viewer. Old scenes use the x axis.
fn place_road(
    plan: &Plan,
    boxes: &BTreeMap<String, Bounds>,
    camera: [f64; 3],
    centre: &mut [f64; 3],
) -> bool {
    let cars: Vec<_> = plan
        .items
        .iter()
        .filter(|i| {
            scene::words(&i.name)
                .iter()
                .any(|w| VEHICLES.contains(&w.as_str()))
        })
        .filter_map(|i| boxes.get(&i.id).copied())
        .collect();
    let span = |b: &Bounds, axis: usize| b.max[axis] - b.min[axis];
    let Some(longest) = cars.iter().max_by(|a, b| {
        span(a, 0)
            .max(span(a, 2))
            .total_cmp(&span(b, 0).max(span(b, 2)))
    }) else {
        return false;
    };
    let along_z = span(longest, 2) > 1.15 * span(longest, 0);
    let across = if along_z { 0 } else { 2 };
    let cars = union(&cars);
    let half = ROAD_WIDTH / 2.0 + 0.3;
    let sign = if camera[across] < (cars.min[across] + cars.max[across]) / 2.0 {
        1.
    } else {
        -1.
    };
    let edge = |b: Bounds| (sign * b.max[across]).max(sign * b.min[across]);
    let lane_center = edge(cars) + half;
    let mut shoulder = edge(cars);
    // Nearby trees and shrubs on the parked vehicle's verge also need soil.
    // Keep planting on the opposite side of the lane in its own place.
    for item in plan.items.iter().filter(|i| i.kind == "object") {
        let name = item.name.to_ascii_lowercase();
        let rooted = name.split(|c: char| !c.is_ascii_alphabetic()).any(|w| {
            matches!(
                w,
                "tree"
                    | "trees"
                    | "cactus"
                    | "cacti"
                    | "cactuses"
                    | "bush"
                    | "bushes"
                    | "shrub"
                    | "shrubs"
                    | "hedge"
                    | "hedges"
            )
        });
        let description = item.description.to_ascii_lowercase();
        if !rooted
            || description.contains("median")
            || description.contains("traffic island")
            || plan
                .layout
                .iter()
                .find(|p| p.item == item.id)
                .and_then(|p| p.rel.as_ref())
                .is_some_and(|r| matches!(r.how.as_str(), "on" | "in" | "held"))
        {
            continue;
        }
        if let Some(b) = boxes.get(&item.id) {
            let root = sign * (b.min[across] + b.max[across]) / 2.;
            if root < lane_center {
                shoulder = shoulder.max(edge(*b));
            }
        }
    }
    centre[across] = sign * (shoulder + half);
    along_z
}

impl Setting {
    fn show_in_sky(&self, body: &str) -> bool {
        body != "stars"
            && (body != "sun"
                || self.world == "space"
                || matches!(self.time.as_str(), "dawn" | "dusk"))
    }

    fn sandy_shore(&self) -> bool {
        if self.ground != "sand" || self.indoor || !matches!(self.world.as_str(), "" | "earth") {
            return false;
        }
        let words = scene::words(&self.summary);
        words.iter().any(|w| {
            matches!(
                w.as_str(),
                "beach"
                    | "shore"
                    | "shoreline"
                    | "seashore"
                    | "lakeshore"
                    | "seaside"
                    | "coast"
                    | "coastline"
            )
        }) || words.windows(3).any(|w| {
            matches!(w[0].as_str(), "by" | "beside" | "along")
                && w[1] == "the"
                && matches!(w[2].as_str(), "sea" | "ocean" | "lake")
        })
    }

    fn has_underwater_arm_fan(&self, models: &[Model]) -> bool {
        self.world == "underwater" && models.iter().any(|m| m.joints.contains_key("limb_tip/3"))
    }

    /// A room frames its occupants across the floor. Open water and sky subjects
    /// need some of the 50-degree view above the horizon.
    pub(super) fn max_camera_pitch(&self, models: &[Model]) -> Option<f64> {
        if self.indoor {
            Some(ROOM_PITCH)
        } else if self.sandy_shore() || self.in_sky.iter().any(|b| self.show_in_sky(b)) {
            Some(PITCH)
        } else if self.world == "underwater" {
            // A flat spread of playing arms needs more elevation. Keep a
            // strip of open water at the top of the 50-degree view.
            Some(if self.has_underwater_arm_fan(models) {
                24.
            } else {
                20.
            })
        } else {
            None
        }
    }
}

fn light_target<'a>(plan: &'a Plan, id: &str) -> Option<&'a str> {
    let p = plan.layout.iter().find(|p| p.item == id)?;
    p.facing.as_deref().or_else(|| {
        p.rel
            .as_ref()
            .filter(|r| r.how == "above")
            .map(|r| r.to.as_str())
    })
}

fn aimed_at_stage(plan: &Plan, id: &str) -> bool {
    light_target(plan, id)
        .and_then(|target| plan.items.iter().find(|i| i.id == target))
        .is_some_and(|i| scene::words(&i.name).iter().any(|w| w == "stage"))
}

fn item_lights(
    plan: &Plan,
    set: &Setting,
    boxes: &BTreeMap<String, Bounds>,
    centre: [f64; 3],
    lights: &mut Vec<Value>,
) {
    let item = |id: &str| plan.items.iter().find(|i| i.id == id);
    let round = |x: f64| (x * 1000.0).round() / 1000.0;
    let dark = match set.time.as_str() {
        "night" => 1.0,
        "dusk" | "dawn" => 0.8,
        _ if set.indoor => 0.6,
        _ => 0.45,
    };
    for g in &set.lights {
        let Some(b) = boxes.get(&g.item) else {
            continue;
        };
        let (w, h) = (
            (b.max[0] - b.min[0]).max(b.max[2] - b.min[2]),
            b.max[1] - b.min[1],
        );
        let tall = h > 2.5 * w;
        let c = mid(b);
        let at = [c[0], if tall { b.min[1] + 0.88 * h } else { c[1] }, c[2]];
        let size = w.max(h);
        let base = match g.strength.as_str() {
            "low" => 1.0,
            "high" => 8.0,
            _ => 3.0,
        };
        if item(&g.item).is_some_and(|i| scene::is_spotlight(&i.name)) {
            let target = light_target(plan, &g.item)
                .and_then(|id| boxes.get(id))
                .map(mid)
                .unwrap_or(centre);
            let distance_squared = (0..3).map(|k| (at[k] - target[k]).powi(2)).sum::<f64>();
            let mut angle = 32.0_f64;
            if aimed_at_stage(plan, &g.item) && distance_squared > 1e-6 {
                // A beam on the stage covers the row of performers. A spot
                // aimed at a single singer keeps its narrow angle.
                for b in plan.lineup.iter().filter_map(|id| boxes.get(id)) {
                    let mut chest = mid(b);
                    chest[1] = b.min[1] + 0.8 * (b.max[1] - b.min[1]);
                    let ray = [0, 1, 2].map(|k| chest[k] - at[k]);
                    let norm = ray.iter().map(|v| v * v).sum::<f64>().sqrt();
                    if norm > 0.001 {
                        let dot = (0..3).map(|k| ray[k] * (target[k] - at[k])).sum::<f64>();
                        let cos = (dot / (norm * distance_squared.sqrt())).clamp(-1.0, 1.0);
                        angle = angle.max((cos.acos().to_degrees() + 5.0).min(70.0));
                    }
                }
            }
            lights.push(json!({
                "id": format!("glow-{}",g.item), "name": "Spotlight", "type":"spot",
                "position": r3(at), "target":r3(target), "angle":round(angle),
                "color":if g.color=="cool" {"#d1e5ff"} else {"#ffe3b2"},
                "intensity":round(base * distance_squared.max(1.0)), "shadow":true,
            }));
            continue;
        }
        // Never blown out: a character near the light (not one holding it)
        // is lit no brighter than by a low sun, and its halo stops short of it.
        let holder = rests_on(plan, &g.item);
        let near = (plan.items.iter())
            .filter(|i| i.kind == "character" && holder.as_deref() != Some(i.id.as_str()))
            .filter_map(|i| boxes.get(&i.id))
            .map(|b| {
                let d = [0, 1, 2].map(|k| (b.min[k] - at[k]).max(at[k] - b.max[k]).max(0.0));
                d.iter().map(|x| x * x).sum::<f64>().sqrt()
            })
            .fold(f64::MAX, f64::min)
            .max(0.3);
        let intensity =
            (base * (size / 0.4).powi(2).clamp(1.0, 16.0) * dark).min(4.0 * near * near);
        let halo =
            (if tall { 0.9 * w } else { 0.75 * size }.clamp(0.12, 4.0) * dark).min(0.8 * near);
        lights.push(json!({
            "id": format!("glow-{}", g.item),
            "name": format!("{} light", item(&g.item).map_or(g.item.as_str(), |i| i.name.as_str())),
            "type": "point", "position": r3(at),
            "color": if g.color == "cool" { "#9cc4ff" } else { "#ffb35c" },
            "intensity": (intensity * 100.0).round() / 100.0, "glow": (halo * 100.0).round() / 100.0,
        }));
    }
}

fn dress(plan: &Plan, set: &Setting, near: &[Model], back: &[&Model], doc: &mut Value) {
    let item = |id: &str| plan.items.iter().find(|i| i.id == id);
    let fov = doc["camera"]["fov"].as_f64().unwrap_or(50.0);
    let objects = doc["objects"].as_array().cloned().unwrap_or_default();
    // Where each placed model is (the ground plane is not one).
    let mut boxes: BTreeMap<String, Bounds> = near
        .iter()
        .filter_map(|m| {
            let o = objects.iter().find(|o| o["id"] == m.item.as_str())?;
            Some((m.item.clone(), world_box(o, &m.bounds)?))
        })
        .collect();
    let group = union(&boxes.values().copied().collect::<Vec<_>>());
    let centre = mid(&group);
    let span = (group.max[0] - group.min[0]).max(group.max[2] - group.min[2]);
    let reach = (group.max[0] - group.min[0]).hypot(group.max[2] - group.min[2]) / 2.0;
    let (cam, tgt): ([f64; 3], [f64; 3]) = (
        camera_at(doc, "position", [6.0, 5.0, 10.0]),
        camera_at(doc, "target", centre),
    );
    // The camera's side of the group, on the ground (unit), and its right.
    let (bx, bz) = (cam[0] - tgt[0], cam[2] - tgt[2]);
    let hl = bx.hypot(bz).max(1e-6);
    let toward = [bx / hl, 0.0, bz / hl];
    let right = [toward[2], 0.0, -toward[0]];
    // The sun shows only low, at dawn or dusk (at noon it is overhead, out of the picture).
    let bodies: Vec<&String> = set.in_sky.iter().filter(|t| set.show_in_sky(t)).collect();
    // Lower, so a tall piece behind the group, or the Moon, shows against the
    // sky; in a room at about eye level across the action, never a map's view
    // of the floor. The builder's framing (its target and distance) stays.
    // A close-up the builder framed (a tiny one on a palm held out) keeps its view.
    let close = objects.iter().any(|o| o.get("frame").is_some());
    let pitch = match set.indoor {
        true => Some(ROOM_PITCH),
        false => (!bodies.is_empty() || (!back.is_empty() && !set.has_underwater_arm_fan(near)))
            .then_some(PITCH),
    };
    if let Some(pitch) = pitch.filter(|_| !close) {
        let d = (0..3)
            .map(|k| (cam[k] - tgt[k]).powi(2))
            .sum::<f64>()
            .sqrt()
            .max(1e-6);
        let e = pitch.to_radians().min(((cam[1] - tgt[1]) / d).asin());
        let dir = [toward[0] * e.cos(), e.sin(), toward[2] * e.cos()];
        doc["camera"]["position"] = json!(r3([0, 1, 2].map(|k| tgt[k] + dir[k] * d)));
    }
    let (cam, tgt) = (
        camera_at(doc, "position", cam),
        camera_at(doc, "target", tgt),
    );
    let view = (0..3)
        .map(|k| (cam[k] - tgt[k]).powi(2))
        .sum::<f64>()
        .sqrt()
        .max(1.0);

    // Water: the base piece stands in it.
    let free =
        |id: &str| item(id).is_some_and(|i| i.kind == "object") && rests_on(plan, id).is_none();
    let water = set.ground == "water";
    let base = near
        .iter()
        .filter(|m| water && free(&m.item))
        .filter_map(|m| Some((m.item.clone(), *boxes.get(&m.item)?)))
        .max_by(|(_, a), (_, b)| {
            let area = |x: &Bounds| (x.max[0] - x.min[0]) * (x.max[2] - x.min[2]);
            area(a).total_cmp(&area(b))
        });
    // A tall base the scene lowered (its legs under the ground) stands in water at the ground.
    let level = base.as_ref().map_or(0.0, |(_, b)| {
        (b.min[1] + 0.4 * (b.max[1] - b.min[1])).max(0.0)
    });
    if base.is_some() {
        // Nothing else drowns: a piece whose feet are under the water stands
        // on its surface (one on the pier's deck is above it already), and
        // what rests on it goes up with it (the fox on a floating crate).
        let base_id = base.as_ref().map(|(k, _)| k.as_str());
        let mut ups = BTreeMap::new();
        for id in boxes.keys() {
            lift(plan, &boxes, base_id, level, id, &mut ups);
        }
        for o in doc["objects"].as_array_mut().into_iter().flatten() {
            let Some(id) = o["id"].as_str().map(str::to_string) else {
                continue;
            };
            let up = ups.get(&id).copied().unwrap_or(0.0);
            let Some(b) = boxes.get_mut(&id).filter(|_| up > 1e-6) else {
                continue;
            };
            b.min[1] += up;
            b.max[1] += up;
            o["position"][1] =
                json!(((o["position"][1].as_f64().unwrap_or(0.0) + up) * 1000.0).round() / 1000.0);
        }
    }
    // A fish swims: one standing free under water floats over the floor.
    if set.world == "underwater" {
        for o in doc["objects"].as_array_mut().into_iter().flatten() {
            let Some(id) = o["id"].as_str().map(str::to_string) else {
                continue;
            };
            let fish = item(&id).is_some_and(|i| i.details.body.as_deref() == Some("fish"));
            if !fish {
                continue;
            }
            // Swimming beside another piece sets the horizontal position only.
            // Keep explicit heights and attachments, such as a fish in a bowl.
            let anchor = anchor_for(plan, &id, |how| {
                !matches!(how, "beside" | "behind" | "in_front_of" | "around")
            });
            if let (None, Some(b)) = (anchor, boxes.get(&id)) {
                let up = (b.max[1] - b.min[1]).max(0.6);
                let y = o["position"][1].as_f64().unwrap_or(0.0) + up;
                o["position"][1] = json!((y * 1000.0).round() / 1000.0);
            }
        }
    }
    // On water the backdrop pieces stand on a low far shore.
    let shore_top = if water { level + 0.5 } else { 0.0 };

    // Backdrop pieces: behind the group as the camera sees it. Outdoors far
    // enough back that a tall one's top stays in the picture (its top at
    // most 65 % of the way up the view's upper half), "far-left" /
    // "far-right" to that side; indoors against the back of the room.
    let ahead = (tgt[0] - cam[0]) * -toward[0] + (tgt[2] - cam[2]) * -toward[2];
    let top = (0.65 * (fov / 2.0 - PITCH)).to_radians().tan();
    let mut cursor: BTreeMap<i32, f64> = BTreeMap::new();
    let (mut extra, mut far_boxes, mut farthest) = (Vec::new(), Vec::new(), 0f64);
    for m in back {
        let it = item(&m.item);
        let s = scale_of(plan, it, m);
        let b = &m.bounds;
        let size = |k: usize| (b.max[k] - b.min[k]).max(0.01) * s;
        let (height, radius) = (size(1), size(0).hypot(size(2)) / 2.0);
        let words = place_words(plan, &m.item);
        let side = match (
            words.iter().any(|w| w == "left"),
            words.iter().any(|w| w == "right"),
        ) {
            (true, false) => -1,
            (false, true) => 1,
            _ => 0,
        };
        let floor = if set.indoor { 0.0 } else { shore_top };
        let back_by = match set.indoor {
            true => reach + radius + 0.3,
            // ...and never so near that it fills more than 30 % of the view.
            false => {
                (ahead + reach + radius + 1.0)
                    .max((floor + height - cam[1]) / top)
                    .max(height.max(2.0 * radius) / (0.3 * fov).to_radians().tan())
                    - ahead
            }
        };
        let at = cursor.entry(side).or_insert(if side == 0 {
            0.0
        } else {
            reach.max(0.25 * (ahead + back_by))
        });
        let off = side as f64 * (*at + radius);
        *at += 2.0 * radius + 1.0;
        let c = [0, 2].map(|k| tgt[k] - toward[k] * back_by + right[k] * off);
        let (mx, mz) = (
            (b.min[0] + b.max[0]) / 2.0 * s,
            (b.min[2] + b.max[2]) / 2.0 * s,
        );
        let obj = json!({
            "id": m.item, "name": it.map_or(m.item.clone(), |i| i.name.clone()), "type": "gltf",
            "src": match m.version {
                Some(n) => format!("otto://design/{}@v{n}", m.artifact_id),
                None => format!("otto://design/{}@latest", m.artifact_id),
            },
            "position": r3([c[0] - mx, floor - b.min[1] * s, c[1] - mz]), "rotation": [0.0, 0.0, 0.0], "scale": [s, s, s],
            "backdrop": true,
        });
        if let Some(w) = world_box(&obj, b) {
            boxes.insert(m.item.clone(), w);
            far_boxes.push(w);
        }
        farthest = farthest.max(ahead + back_by);
        extra.push(obj);
    }
    let all = union(&boxes.values().copied().collect::<Vec<_>>());
    let objs = doc["objects"].as_array_mut().expect("a scene has objects");
    // The ground plane goes: the environment's ground (or none) replaces it.
    objs.retain(|o| !(o["id"] == "ground" && o["type"] == "plane"));
    objs.extend(extra);

    let mut l = look(set);
    if set.indoor
        && matches!(set.time.as_str(), "dusk" | "night")
        && set.lights.iter().any(|g| {
            boxes.contains_key(&g.item)
                && item(&g.item).is_some_and(|i| scene::is_spotlight(&i.name))
                && (aimed_at_stage(plan, &g.item)
                    || light_target(plan, &g.item)
                        .is_some_and(|id| plan.lineup.iter().any(|actor| actor == id)))
        })
    {
        // At showtime the directed beam picks out the performers. A daytime
        // fill from a second direction would light the entire auditorium.
        l.sun.1 = 0.0;
        l.hemi.2 = l.hemi.2.min(0.30);
        l.ibl = l.ibl.min(0.32);
    }
    // Outdoors the ground reaches far past the camera and the backdrop (its
    // edge in the haze); indoors it is the room's floor under everything.
    let extent = (20.0 * view).max(6.0 * farthest).max(200.0);
    let (floor_mid, floor_size) = match set.indoor {
        true => room(plan, &mut boxes, doc, cam, all),
        false => (centre, [extent, extent]),
    };
    if set.indoor {
        super::reveal::frame(plan, near, doc, &mut boxes);
    }
    let cam = camera_at(doc, "position", cam);
    let mut floor_mid = floor_mid;
    let effects: Vec<_> = near
        .iter()
        .filter_map(|m| {
            let obj = doc["objects"]
                .as_array()?
                .iter()
                .find(|o| o["id"] == m.item.as_str())?;
            super::effects::placed(item(&m.item)?, m, obj, cam)
        })
        .take(32)
        .collect();
    let road_along_z = set.ground == "road" && place_road(plan, &boxes, cam, &mut floor_mid);
    let mut env = json!({
        "preset": l.preset, "intensity": l.ibl, "background": true,
        "sky": l.sky, "exposure": if set.time == "night" { 1.2 } else { 1.0 },
    });
    if !effects.is_empty() {
        env["local_effects"] = json!(effects);
    }
    let round = |x: f64| (x * 1000.0).round() / 1000.0;
    if set.ground != "none" {
        env["ground"] = json!({
            "kind": set.ground,
            "center": r3([floor_mid[0], level, floor_mid[2]]),
            "size": floor_size.map(round),
        });
    }
    if road_along_z {
        env["ground"]["axis"] = json!("z");
    }
    if set.sandy_shore() {
        // Cover the foreground with sand up to a little beyond the cast.
        // The water continues past its far edge without lifting any actor.
        let offset = [cam[0] - tgt[0], cam[2] - tgt[2]];
        let axis = usize::from(offset[1].abs() > offset[0].abs());
        let k = 2 * axis;
        let landward = if offset[axis] >= 0. { 1. } else { -1. };
        let edge = if landward > 0. {
            all.min[k]
        } else {
            all.max[k]
        } - landward * (0.20 * (all.max[k] - all.min[k])).max(0.5);
        let mut sand = [floor_mid[0], level, floor_mid[2]];
        sand[k] = edge + landward * floor_size[axis] * 0.5;
        env["ground"]["kind"] = json!("water");
        env["ground"]["center"][1] = json!(round(level - 0.04));
        env["shore"] = json!({"kind":"sand", "center":r3(sand), "size":floor_size.map(round)});
    }
    if water && !far_boxes.is_empty() {
        let u = union(&far_boxes);
        env["shore"] = json!({
            "kind": if set.weather == "snow" { "snow" } else { "grass" },
            "center": r3([(u.min[0] + u.max[0]) / 2.0, shore_top, (u.min[2] + u.max[2]) / 2.0]),
            "size": [round(u.max[0] - u.min[0] + 6.0), round(u.max[2] - u.min[2] + 6.0)],
        });
    }
    if set.indoor {
        let tallest = all.max[1] - all.min[1];
        env["room"] = json!({ "height": round(3f64.max(1.4 * tallest)) });
    } else {
        // Fog thick enough to read as fog (the group a quarter hazed, a
        // backdrop piece a ghost); else a haze that only hides the ground's
        // far edge.
        let thick = match (set.world.as_str(), set.weather.as_str()) {
            ("underwater", _) => 0.4,
            (_, "fog") => 0.55,
            (_, "rain" | "snow") => 0.3,
            _ => 0.0,
        };
        let density = match (thick > 0.0, farthest > 0.0) {
            (true, true) => (thick / view).min(2.0 * thick / farthest),
            (true, false) => thick / view,
            _ => 2.0 / extent,
        };
        // Space has no air to haze.
        if set.world != "space" {
            env["fog"] = json!({ "color": l.sky[1], "density": (density * 1e5).round() / 1e5 });
        }
        if matches!(set.weather.as_str(), "rain" | "snow") && is_earth(&set.world) {
            env["weather"] = json!(set.weather);
        }
        // The Sun, Moon, Earth or a planet low in the sky ahead, one beside
        // the other; stars on a clear night and in space.
        let up = (0.6 * (fov / 2.0 - PITCH)).to_radians();
        let sky: Vec<Value> = bodies
            .iter()
            .zip([-10f64, 9.0, -22.0, 20.0])
            .map(|(kind, turn)| {
                let (a, f) = (turn.to_radians(), [-toward[0], -toward[2]]);
                let h = [f[0] * a.cos() - f[1] * a.sin(), f[0] * a.sin() + f[1] * a.cos()];
                json!({ "kind": kind, "direction": r3([h[0] * up.cos(), up.sin(), h[1] * up.cos()]) })
            })
            .collect();
        if !sky.is_empty() {
            env["sky_bodies"] = json!(sky);
        }
        let night = set.time == "night" && set.weather == "clear";
        if set.world == "space" || night || set.in_sky.iter().any(|t| t == "stars") {
            env["stars"] = json!(true);
        }
    }

    // Lights: the sky light, the sun or moon from the camera's left, and a
    // point light with a halo at each glowing item.
    let far = extent.min(60.0).max(span * 1.5).max(10.0);
    let (sun_col, sun_i, height) = l.sun;
    let az = [
        toward[0] * 0.7 - right[0] * 0.7,
        0.0,
        toward[2] * 0.7 - right[2] * 0.7,
    ];
    let el = height.to_radians();
    let sun_at = [
        centre[0] + az[0] * el.cos() * far,
        centre[1] + el.sin() * far,
        centre[2] + az[2] * el.cos() * far,
    ];
    let mut lights = vec![json!({
        "id": "sky", "name": "Sky light", "type": "hemisphere",
        "color": l.hemi.0, "ground_color": l.hemi.1, "intensity": l.hemi.2,
    })];
    if sun_i > 0.0 {
        let name = if set.time == "night" { "Moon" } else { "Sun" };
        let mut sun = json!({
            "id": "sun", "name": name, "type": "directional", "position": r3(sun_at),
            "target": r3(centre), "color": sun_col, "intensity": (sun_i * 100.0).round() / 100.0, "shadow": true,
        });
        // Lit dust reflects light back onto nearby models in an airless scene.
        if set.world == "space" && set.ground == "moon_dust" {
            sun["shadow_intensity"] = json!(0.76);
        }
        lights.push(sun);
    }
    item_lights(plan, set, &boxes, centre, &mut lights);
    if set.indoor && set.lights.is_empty() {
        // A room with nothing that glows still has its ceiling light on.
        let h = env["room"]["height"].as_f64().unwrap_or(3.0) - 0.3;
        lights.push(json!({
            "id": "room-light", "name": "Room light", "type": "point",
            "position": r3([centre[0], h, centre[2]]), "color": "#ffe6c4",
            "intensity": (1.5 * h * h * 100.0).round() / 100.0,
        }));
    }
    doc["lights"] = json!(lights);
    doc["camera"]["far"] = json!(round(extent.max(500.0)));
    doc["environment"] = env;
    doc["background"] = json!(l.sky[1]);
    doc["grid"] = json!(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::productions::parse_plan_reply;

    fn model(item: &str, min: [f64; 3], max: [f64; 3]) -> Model {
        Model {
            item: item.into(),
            artifact_id: format!("a-{item}"),
            version: Some(1),
            bounds: Bounds { min, max },
            joints: Default::default(),
            real_size: false,
            tris: Vec::new(),
            pose: None,
        }
    }

    #[test]
    fn peeking_through_a_gap_faces_the_camera_from_the_rigs_own_forward_axis() {
        let mut plan: Plan = serde_json::from_value(json!({
            "items": [
                {"id":"table","kind":"object","name":"Table","description":""},
                {"id":"pup","kind":"character","name":"Pup","description":"",
                 "details":{"pose":{"kind":"peek"}}}
            ],
            "layout":[{"item":"pup","rel":{"how":"under","to":"table"}}]
        }))
        .unwrap();
        let mut table = model("table", [-1., 0., -0.5], [1., 0.75, 0.5]);
        table.tris = vec![
            [[-1., 0.75, -0.5], [-1., 0.75, 0.5], [1., 0.75, 0.5]],
            [[-1., 0.75, -0.5], [1., 0.75, 0.5], [1., 0.75, -0.5]],
        ];
        let models = [table, model("pup", [-0.1, 0., -0.1], [0.1, 0.2, 0.1])];
        let yaw = |plan: &Plan| {
            let mut doc = json!({"objects":[
                {"id":"table","position":[0,0,0],"scale":[1,1,1],"rotation":[0,0,0]},
                {"id":"pup","position":[0,0,0],"scale":[1,1,1],"rotation":[0,0,0]}],
                "camera":{"position":[0,1,4],"target":[0,0,0],"fov":50}});
            let mut boxes = models.iter().map(|m| (m.item.clone(), m.bounds)).collect();
            super::super::reveal::frame(plan, &models, &mut doc, &mut boxes);
            assert!(doc["objects"][1]["position"][2].as_f64().unwrap() > 0.);
            doc["objects"][1]["rotation"][1].as_f64().unwrap()
        };
        let forward_z = yaw(&plan);
        plan.items[1].result = Some(super::super::ItemResult {
            rig: Some(json!({"forward":[1.,0.,0.]})),
            ..Default::default()
        });
        assert!((yaw(&plan) + 90. - forward_z).abs() < 0.01);
    }

    /// The owner's pier as a planner writes it with a setting: night, fog,
    /// water, the lantern glowing, and the lighthouse it implies far behind.
    fn pier(setting: Value) -> Plan {
        let it = |id: &str, kind: &str, name: &str, size: f64| {
            json!({"id": id, "kind": kind, "name": name, "description": format!("The {name}."),
                   "enabled": true, "details": {"prompt": name, "size_m": size}})
        };
        let mut v = json!({
            "title": "Stay Close to the Light",
            "items": [it("keeper", "character", "Robot keeper", 2.0), it("lantern", "object", "Glowing lantern", 0.34),
                      it("pier", "object", "Wooden pier", 8.0), it("crate", "object", "Wooden crate", 1.0),
                      it("lighthouse", "object", "Lighthouse", 20.0)],
            "notes": ["I added a lighthouse behind the pier because he is its keeper."],
            "layout": [
                {"item": "pier", "place": "center"},
                {"item": "keeper", "place": "center", "attach_to": {"item": "pier", "where": "top"}},
                {"item": "lantern", "place": "center", "attach_to": {"item": "keeper", "where": "top"}},
                {"item": "crate", "place": "left", "attach_to": {"item": "pier", "where": "top"}},
                {"item": "lighthouse", "place": "far"}
            ],
        });
        if !setting.is_null() {
            v["setting"] = setting;
        }
        parse_plan_reply(&v.to_string())
            .expect("the pier plan validates")
            .0
    }

    fn night_fog() -> Value {
        json!({"summary": "A wooden pier over the sea at night, in thick fog.", "ground": "water",
               "time": "night", "weather": "fog", "indoor": false,
               "lights": [{"item": "lantern", "color": "warm", "strength": "medium"},
                          {"item": "lighthouse", "color": "warm", "strength": "high"}]})
    }

    /// Forge-sized models: about a unit big, the pier long and low, the
    /// lighthouse tall and thin.
    fn models() -> Vec<Model> {
        vec![
            model("keeper", [-0.3, 0.0, -0.2], [0.3, 1.0, 0.2]),
            model("lantern", [-0.1, 0.0, -0.1], [0.1, 0.2, 0.1]),
            model("pier", [-0.5, 0.0, -0.25], [0.5, 0.2, 0.25]),
            model("crate", [-0.5, 0.0, -0.5], [0.5, 0.8, 0.5]),
            model("lighthouse", [-0.1, 0.0, -0.1], [0.1, 1.0, 0.1]),
        ]
    }

    fn obj<'a>(doc: &'a Value, id: &str) -> &'a Value {
        doc["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == id)
            .unwrap()
    }

    fn light<'a>(doc: &'a Value, id: &str) -> Option<&'a Value> {
        doc["lights"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["id"] == id)
    }

    fn v3(v: &Value) -> [f64; 3] {
        serde_json::from_value(v.clone()).unwrap()
    }

    #[test]
    fn a_setting_is_checked_in_plain_closed_words() {
        let ok = pier(night_fog());
        assert_eq!(ok.setting.as_ref().unwrap().lights.len(), 2);
        let refused = |f: &dyn Fn(&mut Value)| {
            let mut s = night_fog();
            f(&mut s);
            let mut plan = pier(Value::Null);
            plan.setting = serde_json::from_value(s).unwrap();
            validate(&plan).unwrap_err()
        };
        assert!(refused(&|s| s["ground"] = json!("lava"))
            .starts_with("setting.ground must be one of water"));
        assert!(refused(&|s| s["time"] = json!("noon")).starts_with("setting.time"));
        assert!(refused(&|s| s["weather"] = json!("")).starts_with("setting.weather"));
        assert!(refused(&|s| s["summary"] = json!(" ")).starts_with("setting.summary"));
        assert!(
            refused(&|s| s["lights"][0]["item"] = json!("ghost")).contains("\"ghost\" must name")
        );
        assert!(refused(&|s| s["lights"][1]["item"] = json!("lantern")).contains("once"));
        assert!(
            refused(&|s| s["lights"][0]["strength"] = json!("blinding")).contains("strength low")
        );
        // A light's colour and strength may be left out (warm, medium).
        let mut s = night_fog();
        s["lights"] = json!([{"item": "lantern"}]);
        let g = &pier(s).setting.unwrap().lights[0];
        assert_eq!((g.color.as_str(), g.strength.as_str()), ("warm", "medium"));
        // "far" is a place.
        assert!(validate_far("far-left").is_ok() && validate_far("very-far").is_err());
    }

    #[test]
    fn an_indoor_evening_performance_is_lit_by_its_spotlight() {
        let mut p = pier(
            json!({"summary":"Theater at showtime", "ground":"wood_floor", "time":"dusk", "weather":"clear",
             "indoor":true, "lights":[{"item":"lantern","strength":"high"}]}),
        );
        p.items.iter_mut().find(|i| i.id == "lantern").unwrap().name = "Theatre spotlight".into();
        p.items.iter_mut().find(|i| i.id == "pier").unwrap().name = "Small stage".into();
        let lamp = p.layout.iter_mut().find(|l| l.item == "lantern").unwrap();
        lamp.attach_to = None;
        lamp.rel = Some(serde_json::from_value(json!({"how":"above","to":"pier"})).unwrap());
        lamp.facing = Some("pier".into());
        let (lit, _) = build_scene(&p, &models());
        assert!(
            light(&lit, "sun").is_none(),
            "daylight washes out the stage beam"
        );
        assert_eq!(light(&lit, "glow-lantern").unwrap()["type"], "spot");
        p.lineup.push("keeper".into());
        p.layout
            .iter_mut()
            .find(|l| l.item == "lantern")
            .unwrap()
            .facing = Some("keeper".into());
        let (solo, _) = build_scene(&p, &models());
        assert!(light(&solo, "sun").is_none());
        assert_eq!(light(&solo, "glow-lantern").unwrap()["angle"], 32.0);
        p.setting.as_mut().unwrap().lights.clear();
        let (room, _) = build_scene(&p, &models());
        assert!(light(&room, "sun").is_some());
        assert!(
            lit["environment"]["intensity"].as_f64().unwrap()
                < room["environment"]["intensity"].as_f64().unwrap()
        );
    }

    #[test]
    fn an_overhead_spotlight_illuminates_its_named_performer() {
        let mut p = pier(night_fog());
        let lamp = p.items.iter_mut().find(|i| i.id == "lantern").unwrap();
        lamp.name = "Theatre spotlight".into();
        let at = p.layout.iter_mut().find(|l| l.item == "lantern").unwrap();
        at.rel = Some(serde_json::from_value(json!({"how":"above","to":"pier"})).unwrap());
        at.attach_to = None;
        at.facing = Some("keeper".into());
        let m = models();
        let (doc, _) = build_scene(&p, &m);
        let beam = light(&doc, "glow-lantern").unwrap();
        assert_eq!(beam["type"], "spot");
        let performer = world_box(obj(&doc, "keeper"), &m[0].bounds).unwrap();
        assert!(v3(&beam["position"])[1] > performer.max[1]);
        let target = v3(&beam["target"]);
        for (actual, expected) in target.iter().zip(mid(&performer)) {
            assert!((actual - expected).abs() < 0.01);
        }
    }

    fn validate_far(place: &str) -> Result<(), String> {
        let mut plan = pier(night_fog());
        plan.layout.last_mut().unwrap().place = place.into();
        super::super::validate_plan(&plan)
    }

    /// An old plan (no setting) builds exactly the scene it built before.
    #[test]
    fn an_old_plan_builds_as_before() {
        let plan = pier(Value::Null);
        let ms = models();
        assert_eq!(build_scene(&plan, &ms).0, scene::build_scene(&plan, &ms).0);
        let doc = build_scene(&plan, &ms).0;
        assert_eq!(obj(&doc, "ground")["type"], "plane");
        assert_eq!(doc["environment"], json!({"preset": "studio-soft"}));
    }

    #[test]
    fn a_sandy_shore_keeps_the_cast_on_land_with_water_beyond() {
        let mut plan = pier(
            json!({"summary":"A small stretch of sandy shore by the sea under a clear night sky.", "ground":"sand", "time":"night", "weather":"clear"}),
        );
        plan.layout.retain(|p| p.item != "lighthouse");
        let cast: Vec<_> = models()
            .into_iter()
            .filter(|m| matches!(m.item.as_str(), "keeper" | "crate"))
            .collect();
        let (doc, _) = build_scene(&plan, &cast);
        let env = &doc["environment"];
        assert_eq!(env["ground"]["kind"], "water");
        assert_eq!(env["shore"]["kind"], "sand");
        let sand = v3(&env["shore"]["center"]);
        let size = &env["shore"]["size"];
        assert!(v3(&env["ground"]["center"])[1] < sand[1]);
        for m in &cast {
            let bounds = world_box(obj(&doc, &m.item), &m.bounds).unwrap();
            for (axis, k) in [0, 2].into_iter().enumerate() {
                let half = size[axis].as_f64().unwrap() / 2.;
                assert!(bounds.min[k] >= sand[k] - half && bounds.max[k] <= sand[k] + half);
            }
        }
        let eye = v3(&doc["camera"]["position"]);
        let at = v3(&doc["camera"]["target"]);
        let depth = usize::from((eye[2] - at[2]).abs() > (eye[0] - at[0]).abs());
        let k = 2 * depth;
        let inland = (eye[k] - at[k]).signum();
        let edge = inland * sand[k] - size[depth].as_f64().unwrap() / 2.;
        let rear = cast
            .iter()
            .map(|m| {
                let b = world_box(obj(&doc, &m.item), &m.bounds).unwrap();
                (inland * b.min[k]).min(inland * b.max[k])
            })
            .fold(f64::INFINITY, f64::min);
        assert!(
            (0.0..2.0).contains(&(rear - edge)),
            "the sea starts close behind the cast"
        );
        plan.setting.as_mut().unwrap().summary = "A desert of sand dunes at night.".into();
        let (desert, _) = build_scene(&plan, &cast);
        assert_eq!(desert["environment"]["ground"]["kind"], "sand");
        assert!(desert["environment"]["shore"].is_null());
    }

    /// The owner's pier: water the pier stands in, night, fog, the lantern
    /// lit, the lighthouse behind it with the camera low enough to see it.
    #[test]
    fn the_pier_stands_in_water_at_night_in_fog_with_the_lantern_lit() {
        let plan = pier(night_fog());
        let (doc, _) = build_scene(&plan, &models());
        // Lowered for the lighthouse, the camera keeps the builder's framing
        // of the cast: its target and its distance.
        let near: Vec<Model> = models()
            .into_iter()
            .filter(|m| m.item != "lighthouse")
            .collect();
        let plain = scene::build_scene(&plan, &near).0;
        assert_eq!(doc["camera"]["target"], plain["camera"]["target"]);
        let dist = |d: &Value| {
            let (p, t) = (v3(&d["camera"]["position"]), v3(&d["camera"]["target"]));
            (0..3).map(|k| (p[k] - t[k]).powi(2)).sum::<f64>().sqrt()
        };
        assert!(
            (dist(&doc) - dist(&plain)).abs() < 0.01,
            "{} {}",
            dist(&doc),
            dist(&plain)
        );
        let env = &doc["environment"];
        assert!(
            doc["objects"]
                .as_array()
                .unwrap()
                .iter()
                .all(|o| o["type"] != "plane"),
            "no slab"
        );
        assert_eq!(env["ground"]["kind"], "water");
        // The pier is 8 m long, so 1.6 m high: the water is above its feet
        // and below its deck (40 % up it, or at the ground once the scene sank
        // the tall deck to 0.8 m).
        let p = v3(&obj(&doc, "pier")["position"]);
        let level = env["ground"]["center"][1].as_f64().unwrap();
        assert!(
            (level - (p[1] + 0.4 * 1.6).max(0.0)).abs() < 1e-3,
            "{level} {p:?}"
        );
        assert!(p[1] < level && level < p[1] + 1.6, "{level} {p:?}");
        assert!(env["ground"]["size"][0].as_f64().unwrap() >= 200.0);
        // Night: a dark sky, the fog is the horizon's colour, a weak moon.
        let sky: Vec<String> = serde_json::from_value(env["sky"].clone()).unwrap();
        assert!(hex(&sky[0]).iter().sum::<f64>() < 100.0, "{sky:?}");
        assert_eq!(env["fog"]["color"], json!(sky[1]));
        let fog = env["fog"]["density"].as_f64().unwrap();
        assert!(fog > 0.005, "{fog}");
        assert_eq!(env["preset"], "night");
        assert_eq!(light(&doc, "sun").unwrap()["name"], "Moon");
        assert!(light(&doc, "sun").unwrap()["intensity"].as_f64().unwrap() < 0.6);
        assert_eq!(doc["grid"], false);
        // The lantern glows warm, inside its own box.
        let lamp = light(&doc, "glow-lantern").expect("the lantern is a light");
        assert_eq!(
            (lamp["type"].as_str(), lamp["color"].as_str()),
            (Some("point"), Some("#ffb35c"))
        );
        assert!(lamp["glow"].as_f64().unwrap() > 0.1);
        let lantern = world_box(obj(&doc, "lantern"), &models()[1].bounds).unwrap();
        let at = v3(&lamp["position"]);
        assert!((0..3).all(|k| lantern.min[k] - 1e-6 <= at[k] && at[k] <= lantern.max[k] + 1e-6));
        // The lighthouse stands behind the pier as the camera sees it, its
        // light near its top, and the camera looks up at 12°.
        let (cam, tgt) = (v3(&doc["camera"]["position"]), v3(&doc["camera"]["target"]));
        let toward = [cam[0] - tgt[0], cam[2] - tgt[2]];
        let lh = world_box(obj(&doc, "lighthouse"), &models()[4].bounds).unwrap();
        let (lc, pc) = (
            mid(&lh),
            mid(&world_box(obj(&doc, "pier"), &models()[2].bounds).unwrap()),
        );
        let behind = -((lc[0] - pc[0]) * toward[0] + (lc[2] - pc[2]) * toward[1])
            / toward[0].hypot(toward[1]);
        assert!(behind > 20.0, "{behind}");
        assert!((lh.max[1] - lh.min[1] - 20.0).abs() < 1e-6);
        assert_eq!(
            obj(&doc, "lighthouse")["backdrop"],
            true,
            "the camera fit leaves it out"
        );
        // The pier is the cast's base, not framed whole (it may run out of the
        // picture); the cast is framed.
        assert_eq!(obj(&doc, "pier")["backdrop"], true);
        assert!(obj(&doc, "keeper").get("backdrop").is_none());
        // It stands on the far shore, above the water, and its top is in the picture.
        let shore = &env["shore"];
        assert_eq!(shore["kind"], "grass");
        assert!((lh.min[1] - (level + 0.5)).abs() < 1e-3, "{lh:?}");
        let s = v3(&shore["center"]);
        assert!((s[0] - lc[0]).abs() < 1e-3 && (s[2] - lc[2]).abs() < 1e-3);
        let along = -((lc[0] - cam[0]) * toward[0] + (lc[2] - cam[2]) * toward[1])
            / toward[0].hypot(toward[1]);
        let above = ((lh.max[1] - cam[1]) / along).atan().to_degrees();
        assert!(above < 25.0 - 12.0, "the top is {above}° above the horizon");
        assert!(
            fog * along < 1.6,
            "the lighthouse shows through the fog: {}",
            fog * along
        );
        let top = v3(&light(&doc, "glow-lighthouse").unwrap()["position"]);
        assert!(top[1] > 17.0, "{top:?}");
        let up = (cam[1] - tgt[1])
            / (0..3)
                .map(|k| (cam[k] - tgt[k]).powi(2))
                .sum::<f64>()
                .sqrt();
        assert!((up - 12f64.to_radians().sin()).abs() < 1e-3, "{up}");
        // A free piece (the crate is on the pier here; a free one would not drown) never sinks below the water.
        for id in ["keeper", "lantern", "crate"] {
            let b = world_box(
                obj(&doc, id),
                &models().iter().find(|m| m.item == id).unwrap().bounds,
            )
            .unwrap();
            assert!(b.min[1] >= level - 1e-3, "{id} is under the water: {b:?}");
        }
        // The near group is laid out as if the lighthouse were not there.
        let near: Vec<Model> = models()
            .into_iter()
            .filter(|m| m.item != "lighthouse")
            .collect();
        assert_eq!(
            obj(&scene::build_scene(&plan, &near).0, "pier")["position"],
            obj(&doc, "pier")["position"]
        );
    }

    /// A piece standing free beside the pier (not on it) is lifted to the
    /// water's surface instead of drowning; the pier stays in the water.
    #[test]
    fn nothing_but_the_base_piece_stands_under_the_water() {
        let mut plan = pier(night_fog());
        plan.layout.retain(|p| p.item != "crate");
        plan.layout
            .push(serde_json::from_value(json!({"item": "crate", "place": "front-left"})).unwrap());
        let (doc, _) = build_scene(&plan, &models());
        let level = doc["environment"]["ground"]["center"][1].as_f64().unwrap();
        let crate_ = world_box(obj(&doc, "crate"), &models()[3].bounds).unwrap();
        assert!((crate_.min[1] - level).abs() < 1e-3, "{crate_:?} {level}");
        let pier_ = world_box(obj(&doc, "pier"), &models()[2].bounds).unwrap();
        assert!(pier_.min[1] < level, "the pier stands in the water");
        // A crate on the water beside the pier floats, and the lantern on it
        // goes up with it: on the crate, not inside it.
        let rel = |item: &str, to: &str, how: &str| {
            serde_json::from_value(json!({"item": item, "rel": {"to": to, "how": how}})).unwrap()
        };
        plan.layout
            .retain(|p| !matches!(p.item.as_str(), "crate" | "lantern"));
        plan.layout.push(rel("crate", "pier", "beside"));
        plan.layout.push(rel("lantern", "crate", "on"));
        let (doc, _) = build_scene(&plan, &models());
        let crate_ = world_box(obj(&doc, "crate"), &models()[3].bounds).unwrap();
        let lantern = world_box(obj(&doc, "lantern"), &models()[1].bounds).unwrap();
        assert!((crate_.min[1] - level).abs() < 1e-3, "{crate_:?} {level}");
        assert!(
            (lantern.min[1] - crate_.max[1]).abs() < 1e-3,
            "{lantern:?} on {crate_:?}"
        );
    }

    #[test]
    fn day_rain_and_rooms_get_their_own_sky_fog_and_light() {
        let mut s = night_fog();
        s["time"] = json!("day");
        s["weather"] = json!("clear");
        s["ground"] = json!("grass");
        let (day, _) = build_scene(&pier(s.clone()), &models());
        let night = build_scene(&pier(night_fog()), &models()).0;
        let fog = |d: &Value| d["environment"]["fog"]["density"].as_f64().unwrap();
        assert!(fog(&day) < fog(&night) / 2.0, "a clear day is only hazed");
        assert!(light(&day, "sun").unwrap()["intensity"].as_f64().unwrap() > 2.0);
        assert_eq!(
            day["environment"]["ground"]["center"][1], 0.0,
            "grass is the floor"
        );
        assert!(day["environment"].get("weather").is_none());
        s["weather"] = json!("rain");
        let rain = build_scene(&pier(s.clone()), &models()).0;
        assert_eq!(rain["environment"]["weather"], "rain");
        // Indoors: a room, no fog, no rain, a ceiling light when nothing glows.
        s["indoor"] = json!(true);
        s["ground"] = json!("wood_floor");
        s["lights"] = json!([]);
        let room = build_scene(&pier(s), &models()).0;
        let env = &room["environment"];
        assert!(env["room"]["height"].as_f64().unwrap() >= 3.0);
        assert!(env.get("fog").is_none() && env.get("weather").is_none());
        assert!(light(&room, "room-light").is_some());
        // Nothing underfoot in a void.
        let mut s = night_fog();
        s["ground"] = json!("none");
        assert!(build_scene(&pier(s), &models()).0["environment"]
            .get("ground")
            .is_none());
    }

    #[test]
    fn the_underwater_first_view_leaves_blue_water_above_the_seabed() {
        let mut set = night_fog();
        set["world"] = json!("underwater");
        set["ground"] = json!("sea_floor");
        set["in_sky"] = json!([]);
        let plan = pier(set);
        // With just the first model ready, a high default camera can fill the frame with sand.
        let only_prop: Vec<_> = models().into_iter().filter(|m| m.item == "crate").collect();
        assert_eq!(only_prop.len(), 1);
        let doc = build_scene(&plan, &only_prop).0;
        let cam = v3(&doc["camera"]["position"]);
        let target = v3(&doc["camera"]["target"]);
        let pitch = (cam[1] - target[1])
            .atan2((cam[0] - target[0]).hypot(cam[2] - target[2]))
            .to_degrees();
        assert!(pitch <= 20.1, "camera points {pitch} degrees down");
    }

    /// The moon: a black airless sky with stars, the Earth low in the view
    /// ahead; under the sea: blue all round and the sea floor.
    #[test]
    fn space_and_under_water_have_their_own_sky() {
        let mut s = night_fog();
        s["world"] = json!("space");
        s["ground"] = json!("moon_dust");
        s["time"] = json!("day");
        s["weather"] = json!("clear");
        s["in_sky"] = json!(["earth"]);
        s["lights"] = json!([]);
        let moon = build_scene(&pier(s.clone()), &models()).0;
        let env = &moon["environment"];
        assert_eq!(env["sky"][0], "#000000");
        assert!(env.get("fog").is_none(), "no air");
        assert_eq!(env["stars"], true);
        assert_eq!(env["ground"]["kind"], "moon_dust");
        let earth = &env["sky_bodies"][0];
        assert_eq!(earth["kind"], "earth");
        // Ahead of the camera, a little above the horizon, inside the view.
        let d = v3(&earth["direction"]);
        let (cam, tgt) = (
            v3(&moon["camera"]["position"]),
            v3(&moon["camera"]["target"]),
        );
        let look = [tgt[0] - cam[0], tgt[2] - cam[2]];
        assert!(d[0] * look[0] + d[2] * look[1] > 0.0);
        let up = d[1].asin().to_degrees();
        assert!(up > 3.0 && up < 25.0 - 12.0, "{up}");
        assert!(light(&moon, "sun").unwrap()["intensity"].as_f64().unwrap() >= 3.0);
        assert_eq!(light(&moon, "sun").unwrap()["shadow_intensity"], 0.76);
        // The noon sun is overhead, not a disc low ahead; a dusk one is.
        let mut sky = night_fog();
        sky["time"] = json!("day");
        sky["in_sky"] = json!(["sun"]);
        assert!(build_scene(&pier(sky.clone()), &models()).0["environment"]
            .get("sky_bodies")
            .is_none());
        sky["time"] = json!("dusk");
        assert_eq!(
            build_scene(&pier(sky), &models()).0["environment"]["sky_bodies"][0]["kind"],
            "sun"
        );
        s["world"] = json!("underwater");
        s["ground"] = json!("sea_floor");
        s["in_sky"] = json!([]);
        let sea = build_scene(&pier(s.clone()), &models()).0;
        let env = &sea["environment"];
        let b = hex(env["fog"]["color"].as_str().unwrap());
        assert!(b[2] > b[0] + 60.0, "blue water all round: {b:?}");
        assert!(env.get("stars").is_none() && env.get("sky_bodies").is_none());
        // A fish standing free swims over the sea floor, never lies on it.
        let mut fish = pier(s);
        fish.items[0].details.body = Some("fish".into());
        fish.layout.retain(|p| p.item != "keeper");
        fish.layout
            .push(serde_json::from_value(json!({"item": "keeper", "place": "left"})).unwrap());
        let doc = build_scene(&fish, &models()).0;
        let swim = world_box(obj(&doc, "keeper"), &models()[0].bounds).unwrap();
        assert!(swim.min[1] >= 0.6, "{swim:?}");
        for relation in ["beside", "behind", "in_front_of", "around", "above", "on"] {
            fish.layout.last_mut().unwrap().rel =
                Some(serde_json::from_value(json!({"to":"crate","how":relation})).unwrap());
            let doc = build_scene(&fish, &models()).0;
            let swim = world_box(obj(&doc, "keeper"), &models()[0].bounds).unwrap();
            let plain = scene::build_scene(&fish, &models()).0;
            let placed = world_box(obj(&plain, "keeper"), &models()[0].bounds).unwrap();
            if matches!(relation, "above" | "on") {
                assert!(
                    (swim.min[1] - placed.min[1]).abs() < 0.002,
                    "{relation}: {swim:?}"
                );
            } else {
                assert!(swim.min[1] >= placed.min[1] + 0.599, "{relation}: {swim:?}");
            }
        }
        // A clear night has stars.
        let mut s = night_fog();
        s["weather"] = json!("clear");
        assert_eq!(
            build_scene(&pier(s), &models()).0["environment"]["stars"],
            true
        );
    }

    fn free_crate(plan: &mut Plan, name: &str, place: &str) {
        plan.items[3].name = name.into();
        plan.layout.retain(|p| p.item != "crate");
        plan.layout
            .push(serde_json::from_value(json!({"item": "crate", "place": place})).unwrap());
    }

    /// A window placed "wall" hangs on the room's back wall; a car on a road
    /// stands on its verge with the road behind it, never in its lane.
    #[test]
    fn a_window_hangs_on_the_wall_and_a_car_stands_beside_the_road() {
        let mut s = night_fog();
        (s["indoor"], s["ground"], s["weather"], s["time"]) = (
            json!(true),
            json!("wood_floor"),
            json!("clear"),
            json!("day"),
        );
        // A painting placed "wall", a window wherever the plan put it, and a
        // stove placed at the back (it stands against the wall, on the floor).
        for (name, place) in [
            ("Old painting", "wall"),
            ("Kitchen window", "front"),
            ("Old stove", "back-left"),
        ] {
            let mut lit = s.clone();
            lit["lights"] = json!([{"item": "crate"}]);
            let mut room = pier(lit);
            free_crate(&mut room, name, place);
            let doc = build_scene(&room, &models()).0;
            let g = &doc["environment"]["ground"];
            let (cz, dz) = (
                g["center"][2].as_f64().unwrap(),
                g["size"][1].as_f64().unwrap(),
            );
            let cam = v3(&doc["camera"]["position"]);
            let wall = cz + if cam[2] >= cz { -dz / 2.0 } else { dz / 2.0 };
            let w = world_box(obj(&doc, "crate"), &models()[3].bounds).unwrap();
            assert!(
                (w.min[2] - wall).abs() < 1e-3 || (w.max[2] - wall).abs() < 1e-3,
                "{name} against the wall at {wall}: {w:?}"
            );
            let hangs = !name.contains("stove");
            assert_eq!(w.min[1] >= 0.99, hangs, "{name} hangs: {w:?}");
            // At about eye level, never a map's view of the room.
            let tgt = v3(&doc["camera"]["target"]);
            let d = (0..3)
                .map(|k| (cam[k] - tgt[k]).powi(2))
                .sum::<f64>()
                .sqrt();
            assert!(
                ((cam[1] - tgt[1]) / d).asin().to_degrees() <= ROOM_PITCH + 0.5,
                "{cam:?}"
            );
            // A glowing window lights the room from the wall, where it hangs.
            let z = v3(&light(&doc, "glow-crate").expect("it glows")["position"])[2];
            assert!(
                z >= w.min[2] - 1e-3 && z <= w.max[2] + 1e-3,
                "{name}'s glow at {z}, the window at {w:?}"
            );
        }

        let mut s = night_fog();
        (s["ground"], s["weather"], s["time"]) = (json!("road"), json!("clear"), json!("day"));
        let mut road = pier(s);
        free_crate(&mut road, "Old yellow car", "right");
        let doc = build_scene(&road, &models()).0;
        let cz = doc["environment"]["ground"]["center"][2].as_f64().unwrap();
        let car = world_box(obj(&doc, "crate"), &models()[3].bounds).unwrap();
        assert!(
            cz + 3.5 <= car.min[2] + 1e-6 || cz - 3.5 >= car.max[2] - 1e-6,
            "the 7 m road at z {cz} misses the car {car:?}"
        );
    }

    #[test]
    fn separate_floor_furniture_stays_side_by_side_at_the_back_wall() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[{"id":"bench","name":"Bench","kind":"object","description":""},
                {"id":"shelf","name":"Shelf","kind":"object","description":""}],
            "layout":[{"item":"bench","place":"back-left"},{"item":"shelf","place":"wall"}]
        }))
        .unwrap();
        let mut boxes = BTreeMap::from([
            (
                "bench".into(),
                Bounds {
                    min: [-1., 0., -2.],
                    max: [0., 0.8, 0.],
                },
            ),
            (
                "shelf".into(),
                Bounds {
                    min: [-0.3, 0., 0.5],
                    max: [0.4, 1., 0.8],
                },
            ),
        ]);
        let all = union(&boxes.values().copied().collect::<Vec<_>>());
        let mut doc = json!({"objects":[{"id":"bench","position":[0.,0.,0.]},{"id":"shelf","position":[0.,0.,0.]}]});
        room(&plan, &mut boxes, &mut doc, [0., 2., 4.], all);
        assert!((boxes["bench"].min[2] - boxes["shelf"].min[2]).abs() < 0.002);
        assert!(
            boxes["bench"].max[0] <= boxes["shelf"].min[0],
            "wall furniture overlaps: {boxes:?}"
        );
    }

    #[test]
    fn a_piece_on_wall_furniture_does_not_make_the_room_deeper() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[{"id":"bench","name":"Bench","kind":"object","description":""},
                {"id":"bag","name":"Bag","kind":"object","description":""},
                {"id":"mat","name":"Mat","kind":"object","description":""}],
            "layout":[{"item":"bench","place":"back-left"},{"item":"mat","place":"front-right"},
                {"item":"bag","rel":{"to":"bench","how":"on"}}]
        }))
        .unwrap();
        let mut boxes = BTreeMap::from([
            (
                "bench".into(),
                Bounds {
                    min: [-1., 0., -1.6],
                    max: [0., 1., -0.6],
                },
            ),
            (
                "mat".into(),
                Bounds {
                    min: [0.2, 0., 0.1],
                    max: [1.2, 0.05, 1.0],
                },
            ),
        ]);
        let all = union(&boxes.values().copied().collect::<Vec<_>>());
        let empty = room(
            &plan,
            &mut boxes.clone(),
            &mut json!({"objects":[]}),
            [0., 2., 4.],
            all,
        );
        boxes.insert(
            "bag".into(),
            Bounds {
                min: [-0.8, 1., -1.4],
                max: [-0.3, 1.3, -0.8],
            },
        );
        assert_eq!(
            room(
                &plan,
                &mut boxes,
                &mut json!({"objects":[]}),
                [0., 2., 4.],
                all
            ),
            empty
        );
    }

    #[test]
    fn the_near_verge_includes_rooted_scenery_beside_a_parked_vehicle() {
        let (mut plan, _) = parse_plan_reply(&json!({
            "title": "A car beside a tree",
            "items": [
                {"id":"car","kind":"object","name":"Old yellow car","description":"Parked on the shoulder."},
                {"id":"tree","kind":"object","name":"Birch tree","description":"A mature birch."}
            ]
        }).to_string()).unwrap();
        let car = Bounds {
            min: [-0.9, 0., -2.3],
            max: [0.9, 1.8, 2.3],
        };
        let mut boxes = BTreeMap::from([("car".into(), car)]);
        for (name, root, description, expected_edge) in [
            ("Tall cactus", 1.8, "Beside the car.", 2.0),
            ("Birch trees", 1.8, "Beside the car.", 2.0),
            ("Bushes", 12., "Across the road.", 0.9),
            ("Bushes", 1.8, "On a traffic island.", 0.9),
        ] {
            plan.items[1].name = name.into();
            plan.items[1].description = description.into();
            boxes.insert(
                "tree".into(),
                Bounds {
                    min: [root - 0.2, 0., -0.2],
                    max: [root + 0.2, 4., 0.2],
                },
            );
            let mut center = [0.; 3];
            assert!(place_road(&plan, &boxes, [-5., 2., 6.], &mut center));
            assert!(
                (center[0] - ROAD_WIDTH / 2.0 - 0.3 - expected_edge).abs() < 1e-6,
                "Road at {center:?} with {name} rooted at {root}"
            );
        }
    }

    #[test]
    fn a_road_runs_alongside_a_long_vehicle() {
        let mut setting = night_fog();
        setting["ground"] = json!("road");
        let mut plan = pier(setting);
        free_crate(&mut plan, "Old yellow car", "right");
        let mut models = models();
        models[3].bounds = Bounds {
            min: [-0.85, 0.0, -2.3],
            max: [0.85, 1.8, 2.3],
        };
        models[3].real_size = true;
        let (doc, _) = build_scene(&plan, &models);
        let ground = &doc["environment"]["ground"];
        let car = world_box(obj(&doc, "crate"), &models[3].bounds).unwrap();
        assert!(car.max[2] - car.min[2] > car.max[0] - car.min[0]);
        assert_eq!(ground["axis"], "z");
        let cx = ground["center"][0].as_f64().unwrap();
        assert!(cx + ROAD_WIDTH / 2.0 <= car.min[0] || cx - ROAD_WIDTH / 2.0 >= car.max[0]);
    }

    /// A far piece something stands on is part of the scene (the owl's
    /// branch on a far oak): laid out with it, not a backdrop.
    #[test]
    fn a_far_piece_something_stands_on_is_laid_out_with_the_scene() {
        let mut plan = pier(night_fog());
        plan.layout.retain(|p| p.item != "crate");
        plan.layout.push(
            serde_json::from_value(
                json!({"item": "crate", "rel": {"to": "lighthouse", "how": "on"}}),
            )
            .unwrap(),
        );
        let doc = build_scene(&plan, &models()).0;
        let at = |d: &Value| {
            let p = v3(&obj(d, "lighthouse")["position"]);
            p[0].hypot(p[2])
        };
        assert!(at(&doc) < 15.0, "laid out with the scene: {doc}");
        let (tower, crate_) = (
            world_box(obj(&doc, "lighthouse"), &models()[4].bounds).unwrap(),
            world_box(obj(&doc, "crate"), &models()[3].bounds).unwrap(),
        );
        assert!(
            (crate_.min[1] - tower.max[1]).abs() < 0.05,
            "{crate_:?} on {tower:?}"
        );
        // Without it the lighthouse is a backdrop again, far behind.
        assert!(at(&build_scene(&pier(night_fog()), &models()).0) > 20.0);
    }

    #[test]
    fn a_setting_change_is_said_plainly_and_can_be_put_back() {
        let night = pier(night_fog());
        let mut s = night_fog();
        s["time"] = json!("day");
        s["weather"] = json!("clear");
        s["lights"] = json!([{"item": "lantern"}]);
        let day = pier(s);
        assert_eq!(
            said(&night, &day),
            [
                "It is daytime now.",
                "The fog is gone.",
                "Lighthouse does not glow now."
            ]
        );
        assert!(said(&night, &night).is_empty());
        let old = pier(Value::Null);
        assert_eq!(said(&old, &night)[0], "It is night now.");
        assert!(said(&old, &night).contains(&"It stands in water now.".to_string()));
        // What a revert puts back: the night, or no setting at all.
        assert_eq!(before(&night, &night), None);
        assert_eq!(
            before(&night, &day).map(restore),
            Some(night.setting.clone())
        );
        assert_eq!(before(&old, &night).map(restore), Some(None));
    }
}
