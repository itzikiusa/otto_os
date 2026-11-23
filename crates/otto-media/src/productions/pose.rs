//! A character's pose in one scene (`details.pose`): sit on a seat, kneel,
//! crouch, lie, curl up, hold a palm out, look at something. Made by Otto
//! Forge's `forge pose` on the character's rigged model (seconds, CPU): the
//! posed model is this scene's own artifact (the project's character stays
//! unposed), its default clip is the pose, and its manifest says where it
//! sits, where its palm is and where it looks ([`Posed`]), which the scene
//! builder uses to sit it on its seat and put what it holds on its palm.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use otto_design::service::{Author, CreateInput, SaveOpts};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::exec::forge_help;
use super::scene::{rest_heading, Bounds, Model};
use super::setting::{build_scene, world_box};
use super::{Item, Plan, Production};
use crate::gen3d::{read_optional_manifest, run_cli_argv, validate_3d_output};
use crate::service::MediaService;

/// The pose kinds a plan may name.
pub const POSES: &[&str] = &[
    "stand", "sit", "kneel", "crouch", "hold_out", "lie", "curl", "lean", "peek",
];

/// What the plan asks of a character's body in this scene.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arms: Option<String>,
    /// A standing instrument played with both hands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play: Option<String>,
    /// Microphone the performer sings into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sing_into: Option<String>,
    /// Independently grounded shaft the performer is pressing into the surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plant: Option<String>,
    /// The seat it sits on (an item id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// What it looks at: an item id, or "camera".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look_at: Option<String>,
    /// The hand it holds out: left or right (default right).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand: Option<String>,
    /// What one hand closes round (from what it holds, see [`effective`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grip: Option<Grip>,
}

/// A fist round a held piece: `forge pose …,hold`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Grip {
    /// left or right.
    pub hand: String,
    /// raise (a stick, a spoon, a sword: up and forward) or carry (a lantern
    /// or a bucket hanging from the fist).
    pub style: String,
    /// What it is held over or pointed at (raise only): an item id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toward: Option<String>,
    /// Axial instrument whose real mouthpiece and valves determine the pose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrument: Option<String>,
    /// A second, separately held object in the opposite hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<OtherGrip>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OtherGrip {
    pub style: String,
    pub toward: Option<String>,
}

pub(super) fn upright_strings(it: &Item) -> bool {
    let n = it.name.to_lowercase();
    it.kind == "object"
        && (n.contains("double bass")
            || n.contains("upright bass")
            || super::scene::words(&n).iter().any(|w| w == "cello"))
}

pub(super) fn drum_kit(it: &Item) -> bool {
    it.kind == "object" && super::scene::words(&it.name).iter().any(|w| w == "drum")
}

pub(super) fn wind_instrument(it: &Item) -> bool {
    it.kind == "object"
        && super::scene::words(&it.name).iter().any(|w| {
            matches!(
                w.as_str(),
                "trumpet" | "cornet" | "bugle" | "clarinet" | "recorder"
            )
        })
}

/// What hangs from a fist rather than being held up.
const CARRIED: &[&str] = &[
    "lantern", "lamp", "bucket", "pail", "basket", "bag", "cage", "kettle",
];

/// Older plans sometimes put the action in the scene summary and only say "beside" in layout.
fn planting(plan: &Plan, actor: &Item, pole: &Item) -> bool {
    let names = super::scene::words(&pole.name);
    let shaft = |w: &str| {
        matches!(w, "flag" | "flagpole" | "stake" | "pole") && names.iter().any(|n| n == w)
    };
    let actor_names = super::scene::words(&actor.name);
    let matches = |text: &str, require_actor: bool| {
        let w = super::scene::words(text);
        w.iter().enumerate().any(|(i, verb)| {
            ["plant", "drive", "press"]
                .iter()
                .any(|v| verb.starts_with(v))
                && w.iter().skip(i + 1).take(4).any(|v| shaft(v))
                && (!require_actor
                    || w[i.saturating_sub(5)..i]
                        .iter()
                        .any(|s| s.len() > 2 && actor_names.contains(s)))
        })
    };
    matches(&actor.description, false) || matches(&plan.summary, true)
}

/// The pose a character is made in: its plan's pose, plus a fist round the
/// first object it holds in a hand (its `holds`, or a piece held by it in
/// the layout), the way that object is held. A palm held out, lying or
/// curled up holds nothing in a fist.
pub fn effective(plan: &Plan, it: &Item) -> Option<Pose> {
    let mut base = it.details.pose.clone();
    if it.kind != "character" {
        return base;
    }
    let place = plan.layout.iter().find(|p| p.item == it.id);
    let nearby = place.and_then(|p| p.rel.as_ref()).and_then(|r| {
        plan.items
            .iter()
            .find(|i| i.id == r.to && i.kind == "object")
    });
    let target = base
        .as_ref()
        .and_then(|p| p.look_at.as_ref())
        .or_else(|| place?.facing.as_ref())
        .and_then(|id| {
            plan.items
                .iter()
                .find(|i| &i.id == id && i.kind == "object")
        });
    let description = super::scene::words(&it.description);
    let has = |terms: &[&str]| description.iter().any(|w| terms.contains(&w.as_str()));
    let mut pose = base.take().unwrap_or_else(|| Pose {
        kind: "stand".into(),
        ..Default::default()
    });
    if pose.play.is_none()
        && has(&[
            "play", "playing", "player", "bassist", "cellist", "pluck", "plucking",
        ])
    {
        pose.play = target
            .filter(|i| upright_strings(i))
            .or_else(|| nearby.filter(|i| upright_strings(i)))
            .map(|i| i.id.clone());
    }
    if pose.play.is_none()
        && it.details.body.as_deref() == Some("many_legs")
        && super::scene::words(&format!("{} {}", it.name, it.description))
            .iter()
            .any(|w| matches!(w.as_str(), "drummer" | "drum" | "drumming"))
    {
        pose.play = target
            .filter(|i| drum_kit(i))
            .or_else(|| nearby.filter(|i| drum_kit(i)))
            .map(|i| i.id.clone());
    }
    if pose.sing_into.is_none() && has(&["sing", "singing", "singer", "vocalist"]) {
        pose.sing_into = target
            .filter(|i| {
                super::scene::words(&i.name)
                    .iter()
                    .any(|w| w == "mic" || w == "microphone")
            })
            .map(|i| i.id.clone());
    }
    if pose.sing_into.is_some()
        && pose.look_at.is_none()
        && (plan.lineup.contains(&it.id)
            || nearby.is_some_and(|support| super::scene::stage_name(&support.name)))
    {
        pose.look_at = Some("camera".into());
    }
    if pose.plant.is_none()
        && place
            .and_then(|p| p.rel.as_ref())
            .is_some_and(|r| r.how == "beside")
    {
        pose.plant = nearby
            .filter(|o| planting(plan, it, o))
            .map(|i| i.id.clone());
    }
    if super::reveal::under_cover(plan, it).is_some() && pose.kind == "stand" {
        pose.kind = "peek".into();
    }
    if it.details.pose.is_some()
        || pose.play.is_some()
        || pose.sing_into.is_some()
        || pose.plant.is_some()
        || pose.kind != "stand"
    {
        base = Some(pose);
    }
    // A perched bird stands on its claws. The height of its branch above
    // the terrain is unrelated to the length of its legs.
    if matches!(
        it.details.body.as_deref(),
        Some("bird" | "no_legs" | "fish")
    ) {
        if let Some(p) = base.as_mut().filter(|p| p.kind == "sit" && p.on.is_some()) {
            p.kind = "stand".into();
            p.on = None;
        }
    }
    if base
        .as_ref()
        .is_some_and(|p| matches!(p.kind.as_str(), "hold_out" | "lie" | "curl" | "peek"))
    {
        return base;
    }
    let object = |id: &str| plan.items.iter().find(|i| i.id == id && i.kind == "object");
    let rel = |id: &str| {
        (plan.layout.iter())
            .find(|p| p.item == id)
            .and_then(|p| p.rel.as_ref())
            .filter(|r| r.how == "held" && r.to == it.id)
    };
    let held = it
        .details
        .holds
        .iter()
        .filter(|h| h.socket.ends_with("hand") && object(&h.item).is_some())
        .map(|h| (h.item.as_str(), h.socket.as_str()))
        .chain(plan.layout.iter().filter_map(|p| {
            let r = rel(&p.item)?;
            object(&p.item)?;
            Some((p.item.as_str(), r.hand.as_deref().unwrap_or("right_hand")))
        }))
        .collect::<Vec<_>>();
    let Some(&(id, socket)) = held.first() else {
        return base;
    };
    let handling = |id: &str| {
        let o = object(id)?;
        let words = super::scene::words(&format!(
            "{} {}",
            o.name,
            o.details.prompt.as_deref().unwrap_or_default()
        ));
        let carry = words.iter().any(|w| CARRIED.contains(&w.as_str()));
        Some(OtherGrip {
            style: if carry { "carry" } else { "raise" }.into(),
            toward: rel(id).and_then(|r| r.toward.clone()).filter(|_| !carry),
        })
    };
    let mut primary = handling(id)?;
    let other = held
        .iter()
        .find(|(next, side)| *side != socket && *next != id)
        .and_then(|(next, _)| handling(next));
    let o = object(id)?;
    let mut p = base.unwrap_or_else(|| Pose {
        kind: "stand".into(),
        ..Default::default()
    });
    if p.arms.as_deref() == Some("crossed") {
        return Some(p);
    }
    let wind = wind_instrument(o);
    if other.is_none()
        && super::scene::words(&o.name).iter().any(|w| {
            matches!(
                w.as_str(),
                "ball" | "snowball" | "orb" | "apple" | "orange" | "peach"
            )
        })
    {
        primary.style = "cup".into();
    }
    p.grip = Some(Grip {
        hand: if socket == "left_hand" {
            "left"
        } else {
            "right"
        }
        .into(),
        style: if wind { "wind".into() } else { primary.style },
        instrument: wind.then(|| id.to_string()),
        other: if wind { None } else { other },
        toward: primary.toward,
    });
    Some(p)
}

impl Pose {
    /// `forge pose --pose`: a palm held out while sitting is "sit,hold-out".
    pub fn arg(&self) -> String {
        let base: String = match (self.kind.as_str(), self.on.is_some()) {
            ("hold_out", true) => "sit,hold-out".into(),
            ("hold_out", false) => "hold-out".into(),
            (k, _) => k.into(),
        };
        if self.arms.as_deref() == Some("crossed") {
            return format!("{base},cross-arms");
        }
        match &self.grip {
            Some(g) if g.instrument.is_some() => base,
            Some(g) if g.style == "cup" => format!("{base},hold-out"),
            Some(_) => format!("{base},hold"),
            None => base,
        }
    }

    pub fn hand_left(&self) -> bool {
        self.kind == "hold_out" && self.hand.as_deref() == Some("left")
    }
}

/// The pose checks (part of [`super::validate_plan`]).
pub fn validate(plan: &Plan) -> Result<(), String> {
    let known = |id: &str| plan.items.iter().any(|i| i.id == id);
    for it in plan.items.iter() {
        let Some(p) = &it.details.pose else { continue };
        let at = format!("item {}", it.id);
        if it.kind != "character" {
            return Err(format!("{at}: details.pose is for a character"));
        }
        if p.plant
            .as_ref()
            .is_some_and(|id| !plan.items.iter().any(|i| &i.id == id && i.kind == "object"))
        {
            return Err(format!(
                "{at}: details.pose.plant names a grounded pole item"
            ));
        }
        if p.play
            .as_ref()
            .is_some_and(|id| !plan.items.iter().any(|i| &i.id == id && i.kind == "object"))
        {
            return Err(format!("{at}: details.pose.play names an instrument item"));
        }
        if p.sing_into
            .as_ref()
            .is_some_and(|id| !plan.items.iter().any(|i| &i.id == id && i.kind == "object"))
        {
            return Err(format!(
                "{at}: details.pose.sing_into names a microphone item"
            ));
        }
        if p.arms.as_deref().is_some_and(|a| a != "crossed") {
            return Err(format!("{at}: details.pose.arms is crossed"));
        }
        if p.kind == "lean" && p.on.is_none() {
            return Err(format!(
                "{at}: a leaning character needs a support in details.pose.on"
            ));
        }
        if !POSES.contains(&p.kind.as_str()) {
            return Err(format!(
                "{at}: details.pose.kind is one of {}",
                POSES.join(", ")
            ));
        }
        if p.on.as_deref().is_some_and(|o| !known(o) || o == it.id) {
            return Err(format!("{at}: details.pose.on names the seat's item id"));
        }
        if p.look_at
            .as_deref()
            .is_some_and(|o| o != "camera" && (!known(o) || o == it.id))
        {
            return Err(format!(
                "{at}: details.pose.look_at is an item id or \"camera\""
            ));
        }
        if p.hand
            .as_deref()
            .is_some_and(|h| !matches!(h, "left" | "right"))
        {
            return Err(format!("{at}: details.pose.hand is left or right"));
        }
    }
    Ok(())
}

/// What the posed model's manifest says, in its own frame (metres): where its
/// body rests on a seat, its palm, its head, and its posed box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Posed {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub seat: Option<[f64; 3]>,
    pub back: Option<[[f64; 3]; 3]>,
    pub palm: Option<[f64; 3]>,
    /// Where a grounded shaft passes through the posed fist.
    pub shaft: Option<[f64; 3]>,
    pub hand_left: bool,
    /// The middle of its head.
    pub head: Option<[f64; 3]>,
    pub head_bounds: Option<super::scene::Bounds>,
    pub mouth: Option<[f64; 3]>,
    /// Its fist round a held piece: where the handle passes through, the
    /// handle's way out of it (the free end leaves along it) and the thumb's side.
    pub grip: Option<GripAt>,
    pub other_grip: Option<GripAt>,
    pub wind: Option<WindMount>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct WindMount {
    pub point: [f64; 3],
    pub tip: [f64; 3],
    pub rotation: [[f64; 3]; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GripAt {
    pub left: bool,
    pub carry: bool,
    pub point: [f64; 3],
    pub axis: [f64; 3],
}

impl Posed {
    /// From the manifest stored on the item's result (`result.pose.manifest`).
    pub fn of(manifest: &Value) -> Option<Posed> {
        let v3 = |v: &Value| -> Option<[f64; 3]> {
            let a = v.as_array()?;
            Some([
                a.first()?.as_f64()?,
                a.get(1)?.as_f64()?,
                a.get(2)?.as_f64()?,
            ])
        };
        let grip = |g: &Value| {
            Some(GripAt {
                left: g["hand"] == "left",
                carry: g["style"] == "carry",
                point: v3(&g["point"])?,
                axis: v3(&g["axis"])?,
            })
        };
        let b = &manifest["bounds"];
        Some(Posed {
            min: v3(&b["min"])?,
            max: v3(&b["max"])?,
            back: serde_json::from_value(manifest["contact"]["back"].clone()).ok(),
            wind: serde_json::from_value(manifest["wind_mount"].clone()).ok(),
            seat: v3(&manifest["contact"]["seat"]),
            palm: v3(&manifest["palm"]["point"]),
            shaft: v3(&manifest["contact"]["shaft"]),
            hand_left: manifest["palm"]["hand"] == "left",
            mouth: v3(&manifest["head"]["mouth"]),
            head: v3(&manifest["head"]["point"]),
            head_bounds: v3(&manifest["head"]["bounds"]["min"])
                .zip(v3(&manifest["head"]["bounds"]["max"]))
                .filter(|(min, max)| (0..3).all(|k| min[k] < max[k]))
                .map(|(min, max)| super::scene::Bounds { min, max }),
            grip: grip(&manifest["grip"]),
            other_grip: grip(&manifest["other_grip"]),
        })
    }
}

/// The pose an item's result was made with (`result.pose.spec`), if any.
pub fn made_with(it: &Item) -> Option<Pose> {
    let v = it.result.as_ref()?.pose.as_ref()?;
    serde_json::from_value(v.get("spec")?.clone()).ok()
}

/// Whether this Forge has a subcommand (its `--help` lists it in `{…}`).
pub(super) async fn forge_has(bin: &str, command: &str) -> bool {
    let help = forge_help(bin, &["--help"]).await;
    let list = help
        .split('{')
        .nth(1)
        .and_then(|s| s.split('}').next())
        .unwrap_or("");
    list.split(',').any(|c| c.trim() == command)
}

/// Two separate playing surfaces on the player's side of a complete kit.
pub(super) fn near_drumheads(
    tris: &[super::geometry::Tri],
    b: Bounds,
    player: [f64; 2],
) -> [[f64; 3]; 2] {
    let mut q = [0, 1, 2].map(|k| (b.min[k] + b.max[k]) / 2.);
    let dx = player[0] - q[0];
    let dz = player[1] - q[2];
    let n = dx.hypot(dz).max(0.001);
    let width = b.max[0] - b.min[0];
    let depth = b.max[2] - b.min[2];
    q[0] += 0.29 * width * dx / n;
    q[2] += 0.29 * depth * dz / n;
    q[1] = b.min[1] + 0.85 * (b.max[1] - b.min[1]);
    let seed = [1., -1.].map(|side| {
        [
            q[0] - side * 0.22 * width * dz / n,
            q[1],
            q[2] + side * 0.22 * depth * dx / n,
        ]
    });
    drum_surfaces(tris, b, seed)
}

/// Choose a patch of actual drumhead for each hand. A kit's highest point
/// may be a cymbal stand and there is often air between the two nearest toms.
fn drum_surfaces(
    tris: &[super::geometry::Tri],
    bounds: Bounds,
    seed: [[f64; 3]; 2],
) -> [[f64; 3]; 2] {
    let Some(surface) = super::geometry::Surface::of(tris, bounds.min, bounds.max) else {
        return seed;
    };
    let cell = surface.cell();
    let span = (bounds.max[0] - bounds.min[0]).max(bounds.max[2] - bounds.min[2]);
    let height = bounds.max[1] - bounds.min[1];
    let radius = (0.045 * span).max(cell);
    let mut points = seed;
    let mut costs = [f64::INFINITY; 2];
    let separate = (seed[0][0] - seed[1][0]).hypot(seed[0][2] - seed[1][2]);
    // Prefer a broad playing surface for each hand. A small head on the
    // other side of the kit can still support a fingertip or stick tip.
    for half in [radius, (0.5 * radius).max(cell)] {
        let found = costs.map(f64::is_finite);
        if found == [true; 2] {
            break;
        }
        for ix in 0..((bounds.max[0] - bounds.min[0]) / cell).ceil() as usize {
            for iz in 0..((bounds.max[2] - bounds.min[2]) / cell).ceil() as usize {
                let x = bounds.min[0] + (ix as f64 + 0.5) * cell;
                let z = bounds.min[2] + (iz as f64 + 0.5) * cell;
                let dist = seed.map(|at| (x - at[0]).hypot(z - at[2]));
                let i = usize::from(dist[1] < dist[0]);
                if found[i]
                    || dist[i] > 0.8 * separate
                    || dist[1 - i] < 0.65 * separate
                    || dist[i] * dist[i] >= costs[i]
                {
                    continue;
                }
                if let Some(y) = surface.support(x, z, [half; 2], 0.08 * height) {
                    let cost = dist[i] * dist[i] + 0.3 * (y - seed[i][1]).powi(2);
                    if cost < costs[i] {
                        points[i] = [x, surface.top_at(x, z).unwrap_or(y), z];
                        costs[i] = cost;
                    }
                }
            }
        }
    }
    points
}

/// A point of the scene as `forge pose --look-at` takes it: in the model's
/// own metres from the point on its floor (`floor`, world y) under its
/// origin, x its left, y up, z the way it faces.
fn local(o: &Value, at: [f64; 3], floor: f64, rest_heading: f64) -> Option<[f64; 3]> {
    let n = |v: &Value, k: usize| v[k].as_f64();
    let (p, yaw, s) = (
        &o["position"],
        (n(&o["rotation"], 1)? + rest_heading).to_radians(),
        n(&o["scale"], 1)?,
    );
    let d = [at[0] - n(p, 0)?, at[1] - floor, at[2] - n(p, 2)?];
    let (x, z) = (
        d[0] * yaw.cos() - d[2] * yaw.sin(),
        d[0] * yaw.sin() + d[2] * yaw.cos(),
    );
    Some([x / s, d[1] / s, z / s])
}

impl MediaService {
    /// Pose every character whose plan asks a pose it was not made with,
    /// once it is done (its skeleton made) and what the pose names (its seat, what it
    /// looks at) is made: `forge pose` on its rigged model, the seat's real
    /// height and the look target in its own frame from the scene as laid
    /// out now. Seconds each. An engine without `pose` leaves it as it was
    /// (it stands, said by the scene); a pose the engine refuses is said.
    pub(super) async fn pose_characters(&self, p: &mut Production, user_id: &str) {
        let made = |id: &str| {
            id == "camera"
                || p.plan
                    .items
                    .iter()
                    .any(|i| i.id == id && i.result.is_some())
        };
        let todo: Vec<usize> = (0..p.plan.items.len())
            .filter(|&ix| {
                let it = &p.plan.items[ix];
                let Some(pose) = effective(&p.plan, it) else {
                    return false;
                };
                // Done: its skeleton (and moves) made, or reused as it is; a
                // model without a skeleton is refused by the engine, said.
                let rigged = it.status == "done" && it.result.is_some();
                let toward = pose.grip.as_ref().and_then(|g| g.toward.as_ref());
                let instrument = pose.grip.as_ref().and_then(|g| g.instrument.as_ref());
                let other = pose
                    .grip
                    .as_ref()
                    .and_then(|g| g.other.as_ref()?.toward.as_ref());
                let named = [
                    pose.plant.as_ref(),
                    pose.play.as_ref(),
                    pose.sing_into.as_ref(),
                    pose.on.as_ref(),
                    pose.look_at.as_ref(),
                    toward,
                    instrument,
                    other,
                ]
                .into_iter()
                .flatten()
                .all(|id| made(id));
                it.kind == "character" && rigged && named && made_with(it) != Some(pose)
            })
            .collect();
        let Some(bin) = (if todo.is_empty() {
            None
        } else {
            self.forge_bin().await
        }) else {
            return;
        };
        if !forge_has(&bin, "pose").await {
            return;
        }
        let models = self.scene_models(p).await;
        let (doc, _) = build_scene(&p.plan, &models);
        for ix in todo {
            let made = self.pose_one(p, ix, &bin, &models, &doc, user_id).await;
            let spec = json!(effective(&p.plan, &p.plan.items[ix]));
            let it = &mut p.plan.items[ix];
            // Its last pose's refusal is no longer true.
            let refused = format!("Otto could not pose {} this time (", it.name);
            let note = match made {
                Ok(mut v) => {
                    v["spec"] = spec;
                    if let Some(r) = it.result.as_mut() {
                        r.pose = Some(v);
                    }
                    None
                }
                Err(why) => {
                    if let Some(r) = it.result.as_mut() {
                        r.pose = Some(json!({ "spec": spec, "failed": why }));
                    }
                    Some(format!(
                        "Otto could not pose {} this time ({why}), so it stands.",
                        it.name
                    ))
                }
            };
            p.plan.notes.retain(|n| !n.starts_with(&refused));
            if let Some(n) = note {
                p.plan.notes.push(n);
            }
        }
    }

    async fn pose_input(&self, it: &Item) -> Result<Vec<u8>, String> {
        let design = self.design_service();
        let r = it.result.as_ref().ok_or("the model is not made")?;
        let a = design
            .store()
            .require_artifact(r.artifact_id.as_deref().unwrap_or_default())
            .await
            .map_err(|e| e.to_string())?;
        let v = match r.version_id.as_deref() {
            Some(v) => design.resolve_version(&a, v).await,
            None => design.head_version(&a).await,
        }
        .map_err(|e| e.to_string())?;
        design.version_bytes(&v).await.map_err(|e| e.to_string())
    }

    /// One character posed: the posed model saved as this scene's own
    /// artifact (a new version when it was posed before), and what the
    /// scene needs from its manifest. `Err`: why, in plain words.
    async fn pose_one(
        &self,
        p: &Production,
        ix: usize,
        bin: &str,
        models: &[Model],
        doc: &Value,
        user_id: &str,
    ) -> std::result::Result<Value, String> {
        let it = &p.plan.items[ix];
        let pose = effective(&p.plan, it).unwrap_or_default();
        let obj = |id: &str| {
            doc["objects"]
                .as_array()?
                .iter()
                .find(|o| o["id"] == id)
                .cloned()
        };
        let model = |id: &str| models.iter().find(|m| m.item == id);
        let me = obj(&it.id).ok_or("it is not in the scene yet")?;
        let mut drum_contact = None;
        let mut args = vec![
            "pose".to_string(),
            "--mesh".into(),
            String::new(),
            "--pose".into(),
            pose.arg(),
        ];
        if it.details.body.as_deref() == Some("fish")
            && forge_help(bin, &["pose", "--help"])
                .await
                .contains("--swimming-fish")
        {
            args.push("--swimming-fish".into());
        }
        if pose.kind == "peek" {
            // A reused rig can predate the material finish for pale eyes in shadow.
            let prompt = it.details.prompt.as_deref().unwrap_or(&it.description);
            args.extend(["--prompt".into(), prompt.into()]);
        }
        // The seat's top above the ground, in the character's own metres;
        // the floor it stands on: the seat's ground, else under its feet now.
        let seat = (pose.on.as_deref())
            .and_then(|id| obj(id).zip(model(id)))
            .and_then(|(o, m)| world_box(&o, &m.bounds));
        let s = me["scale"][1].as_f64().unwrap_or(1.0);
        let mine = model(&it.id).and_then(|m| world_box(&me, &m.bounds));
        let floor = seat.map_or(mine.map_or(0.0, |b| b.min[1]), |b| b.min[1].max(0.0));
        let heading = rest_heading(&p.plan, &it.id);
        let body_point = |at| local(&me, at, floor, heading);
        let player_xz = [
            me["position"][0].as_f64().unwrap_or(0.),
            me["position"][2].as_f64().unwrap_or(0.),
        ];
        let kit_heads = |kit: &Value, m: &Model, b: Bounds| {
            let world = super::setting::upright_transform(kit).ok_or("missing kit transform")?;
            let tris: Vec<_> = m.tris.iter().map(|tri| tri.map(&world)).collect();
            Ok::<_, String>(near_drumheads(&tris, b, player_xz))
        };
        if let Some(b) = seat.filter(|_| pose.kind != "lean") {
            args.extend([
                "--seat-height".into(),
                format!("{:.3}", (b.max[1] - floor) / s),
            ]);
        }
        let aim = |at: Option<&str>| match at {
            Some("camera") => (doc["camera"]["position"].as_array())
                .and_then(|a| Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])),
            Some(id) => obj(id).zip(model(id)).and_then(|(o, m)| {
                let b: Bounds = world_box(&o, &m.now_box())?;
                let top = if m.pose.is_some()
                    || p.plan
                        .items
                        .iter()
                        .any(|i| i.id == id && i.kind == "character")
                {
                    b.max[1] - 0.1 * (b.max[1] - b.min[1])
                } else {
                    (b.min[1] + b.max[1]) / 2.0
                };
                Some([
                    (b.min[0] + b.max[0]) / 2.0,
                    top,
                    (b.min[2] + b.max[2]) / 2.0,
                ])
            }),
            None => None,
        };
        let target = aim(pose.look_at.as_deref())
            .filter(|_| !(pose.kind == "peek" && pose.look_at.as_deref() == Some("camera")));
        if let Some(l) = target.and_then(body_point) {
            args.extend([
                "--look-at".into(),
                format!("{:.3},{:.3},{:.3}", l[0], l[1], l[2]),
            ]);
        }
        if pose.kind == "hold_out" {
            args.extend([
                "--hand".into(),
                pose.hand.clone().unwrap_or_else(|| "right".into()),
            ]);
        }
        if let Some(g) = pose.grip.as_ref().filter(|g| g.style == "cup") {
            args.extend(["--hand".into(), g.hand.clone()]);
        }
        if let Some(g) = pose
            .grip
            .as_ref()
            .filter(|g| g.instrument.is_none() && g.style != "cup")
        {
            args.extend([
                "--hand".into(),
                g.hand.clone(),
                "--hold-style".into(),
                g.style.clone(),
            ]);
            if let Some(other) = &g.other {
                args.extend(["--other-hold-style".into(), other.style.clone()]);
            }
            for (flag, target, left) in [
                ("--hold-toward", g.toward.as_ref(), g.hand == "left"),
                (
                    "--other-hold-toward",
                    g.other.as_ref().and_then(|g| g.toward.as_ref()),
                    g.hand != "left",
                ),
            ] {
                let Some(id) = target else { continue };
                let Some((o, m)) = obj(id).zip(model(id)) else {
                    continue;
                };
                let Some(b) = world_box(&o, &m.now_box()) else {
                    continue;
                };
                let top = [
                    (b.min[0] + b.max[0]) / 2.,
                    b.max[1],
                    (b.min[2] + b.max[2]) / 2.,
                ];
                let hand_target = if p
                    .plan
                    .items
                    .iter()
                    .any(|item| item.id == *id && drum_kit(item))
                {
                    let heads = kit_heads(&o, m, b)?;
                    let [Some(a), Some(b)] = heads.map(body_point) else {
                        continue;
                    };
                    Some(if (a[0] > b[0]) == left { a } else { b })
                } else {
                    body_point(top)
                };
                if let Some(l) = hand_target {
                    args.extend([flag.into(), format!("{:.3},{:.3},{:.3}", l[0], l[1], l[2])]);
                }
            }
        }
        if let Some(id) = pose.play.as_deref().filter(|id| {
            p.plan
                .items
                .iter()
                .any(|i| i.id == *id && upright_strings(i))
        }) {
            let o = obj(id).ok_or("missing instrument in the layout")?;
            let bass = model(id).ok_or("missing instrument model")?;
            let b = world_box(&o, &bass.bounds).ok_or("missing instrument bounds")?;
            let h = b.max[1] - b.min[1];
            for (flag, level) in [("--fingerboard", 0.75), ("--strings", 0.47)] {
                let y = b.min[1] + level * h;
                let yaw = o["rotation"][1].as_f64().unwrap_or_default().to_radians();
                let front = 0.4
                    * (bass.bounds.max[2] - bass.bounds.min[2])
                    * o["scale"][2].as_f64().unwrap_or(1.);
                let target = [
                    (b.max[0] + b.min[0]) / 2. + yaw.sin() * front,
                    y,
                    (b.max[2] + b.min[2]) / 2. + yaw.cos() * front,
                ];
                let l = body_point(target).ok_or("missing performer transform")?;
                args.extend([flag.into(), format!("{:.4},{:.4},{:.4}", l[0], l[1], l[2])]);
            }
        }
        if let Some(id) = pose.play.as_deref().filter(|id| {
            p.plan.items.iter().any(|i| i.id == *id && drum_kit(i))
                && it.details.body.as_deref() == Some("many_legs")
        }) {
            let kit = obj(id).ok_or("missing kit in the layout")?;
            let m = model(id).ok_or("missing drum model")?;
            let b = world_box(&kit, &m.bounds).ok_or("missing drum bounds")?;
            let world = super::setting::upright_transform(&kit).ok_or("missing kit transform")?;
            let points = kit_heads(&kit, m, b)?;
            let center = [0, 1, 2].map(|k| (points[0][k] + points[1][k]) / 2.);
            let origin = world([0.; 3]);
            let xz = [0, 2].map(|axis| {
                let mut unit = [0.; 3];
                unit[axis] = 1.;
                let v = world(unit);
                let dx = v[0] - origin[0];
                let dz = v[2] - origin[2];
                ((center[0] - origin[0]) * dx + (center[2] - origin[2]) * dz)
                    / (dx * dx + dz * dz).max(1e-12)
            });
            drum_contact = Some(json!({"kit":id,"xz":xz}));
            for (flag, at) in ["--strike-left", "--strike-right"].into_iter().zip(points) {
                let l = body_point(at).ok_or("missing drummer transform")?;
                args.extend([flag.into(), format!("{:.4},{:.4},{:.4}", l[0], l[1], l[2])]);
            }
        }
        if let Some(id) = pose.plant.as_deref() {
            let shaft = model(id).ok_or("missing planted shaft")?;
            let o = obj(id).ok_or("missing grounded pole in the layout")?;
            let raw = super::scene::shaft_axis(shaft);
            let [x, z] = [0, 2].map(|k| raw[k / 2] * o["scale"][k].as_f64().unwrap_or(1.));
            let yaw = o["rotation"][1].as_f64().unwrap_or(0.).to_radians();
            let h = model(&it.id).ok_or("missing performer")?.bounds;
            let world = [
                o["position"][0].as_f64().unwrap_or(0.) + x * yaw.cos() + z * yaw.sin(),
                // A chest-high grip leaves the elbow bent to press the pole down.
                floor + (h.max[1] - h.min[1]) * s * 0.70,
                o["position"][2].as_f64().unwrap_or(0.) - x * yaw.sin() + z * yaw.cos(),
            ];
            let l = body_point(world).ok_or("missing performer transform")?;
            args.extend([
                "--plant-at".into(),
                format!("{:.4},{:.4},{:.4}", l[0], l[1], l[2]),
                "--hand".into(),
                pose.hand.clone().unwrap_or_else(|| {
                    if l[0] > 0. {
                        "left".into()
                    } else {
                        "right".into()
                    }
                }),
            ]);
        }
        // The rigged model on disk, the posed one beside it.
        let design = self.design_service();
        let r = it.result.as_ref().ok_or("it is not made")?;
        let bytes = self.pose_input(it).await?;
        let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
        let (input, out) = (tmp.path().join("rigged.glb"), tmp.path().join("posed.glb"));
        tokio::fs::write(&input, &bytes)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(id) = pose.grip.as_ref().and_then(|g| g.instrument.as_deref()) {
            let instrument = p
                .plan
                .items
                .iter()
                .find(|i| i.id == id)
                .ok_or("missing held instrument")?;
            let scale = obj(id)
                .and_then(|o| o["scale"][0].as_f64())
                .ok_or("missing instrument scale")?;
            let path = tmp.path().join("instrument.glb");
            tokio::fs::write(&path, self.pose_input(instrument).await?)
                .await
                .map_err(|e| e.to_string())?;
            args.extend([
                "--wind-instrument".into(),
                path.to_string_lossy().into(),
                "--wind-scale".into(),
                (scale / s).to_string(),
            ]);
        }
        args[2] = input.to_string_lossy().into();
        args.extend(["--output".into(), out.to_string_lossy().into()]);
        let never = AtomicBool::new(false);
        run_cli_argv(
            Path::new(bin),
            &args,
            None,
            Duration::from_secs(180),
            &never,
            None,
        )
        .await
        .map_err(|e| super::exec::plain(&e))?;
        let posed = tokio::fs::read(&out)
            .await
            .map_err(|_| "the engine wrote no posed model".to_string())?;
        validate_3d_output("glb", &posed, 512 * 1024 * 1024).map_err(|e| e.to_string())?;
        let manifest = read_optional_manifest(&out).await.unwrap_or_default();
        Posed::of(&manifest).ok_or("the engine said nothing of the pose")?;
        let message = format!("Posed: {}", pose.arg());
        let provenance = json!({ "source": "pose", "pose": manifest["pose"], "of": r.artifact_id, "production": p.id });
        let before = r.pose.as_ref().and_then(|v| v["artifact_id"].as_str());
        let saved = match before.map(|id| design.store().require_artifact(id)) {
            Some(f) => {
                let a = f.await.map_err(|e| e.to_string())?;
                let opts = SaveOpts {
                    base: None,
                    kind: "named".into(),
                    author: Author::user(user_id),
                    message,
                    provenance,
                    force: false,
                    validate: true,
                    change: "content",
                };
                design.commit_bytes(&a, posed, opts).await
            }
            None => {
                design
                    .create_artifact(CreateInput {
                        workspace_id: p.workspace_id.clone(),
                        project_id: None,
                        studio: Some("3d".into()),
                        format: "glb".into(),
                        title: format!("{} ({})", it.name, pose.kind.replace('_', " ")),
                        tags: vec!["smart-mode".into(), "pose".into()],
                        meta: json!({ "source": "pose" }),
                        content: Some(posed),
                        story_id: None,
                        derived_from: None,
                        created_by: user_id.to_string(),
                        author: Author::user(user_id),
                        message: Some(message),
                        source: None,
                        created_at: None,
                        version_kind: Some("named".into()),
                        validate: true,
                        provenance: Some(provenance),
                    })
                    .await
            }
        }
        .map_err(|e| e.to_string())?;
        let keep = [
            "bounds",
            "forward",
            "rest_mesh_unchanged",
            "contact",
            "palm",
            "head",
            "grip",
            "wind_mount",
            "other_grip",
            "limb_tips",
            "default_clip",
        ];
        let manifest: serde_json::Map<String, Value> = keep
            .iter()
            .filter_map(|k| Some((k.to_string(), manifest.get(*k)?.clone())))
            .collect();
        let mut receipt = json!({ "artifact_id": saved.artifact.id, "version_id": saved.version.id, "manifest": manifest });
        if let Some(contact) = drum_contact {
            receipt["manifest"]["drum_contact"] = contact;
        }
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_narrow_drumhead_still_provides_a_strike_point() {
        let bounds = Bounds {
            min: [-0.65, 0., -0.4],
            max: [0.65, 0.7, 0.4],
        };
        let mut tris = Vec::new();
        for (x, half) in [(-0.32, 0.18), (0.32, 0.048)] {
            let a = [x - half, 0.48, 0.1 - half];
            let b = [x - half, 0.48, 0.1 + half];
            let c = [x + half, 0.48, 0.1 + half];
            let d = [x + half, 0.48, 0.1 - half];
            tris.extend([[a, b, c], [a, c, d]]);
        }
        let seed = [[-0.24, 0.6, -0.16], [0.24, 0.6, -0.16]];
        let [large, small] = drum_surfaces(&tris, bounds, seed);
        assert!((large[0] + 0.32).abs() < 0.18);
        assert!((small[0] - 0.32).abs() < 0.048);
        assert!((small[2] - 0.1).abs() < 0.048);
        assert!((small[1] - 0.48).abs() < 1e-6);
    }

    #[test]
    fn two_drum_strokes_land_on_the_separate_raised_playing_surfaces() {
        // The two heads sit at different heights, forward of the space
        // where a fixed fraction of the kit box would put each stroke.
        let mut tris = Vec::new();
        for (x, y) in [(-0.26, 0.31), (0.30, 0.64)] {
            let a = [x - 0.11, y, 0.06];
            let b = [x - 0.11, y, 0.32];
            let c = [x + 0.11, y, 0.32];
            let d = [x + 0.11, y, 0.06];
            tris.extend([[a, b, c], [a, c, d]]);
        }
        let bounds = Bounds {
            min: [-0.50, 0., -0.4],
            max: [0.50, 0.7, 0.4],
        };
        let seed = [[-0.22, 0.60, -0.12], [0.22, 0.60, -0.12]];
        let points = drum_surfaces(&tris, bounds, seed);
        for (p, (x, y)) in points.iter().zip([(-0.26, 0.31), (0.30, 0.64)]) {
            assert!(
                (p[0] - x).abs() < 0.11 && (0.06..0.32).contains(&p[2]),
                "{p:?}"
            );
            assert!((p[1] - y).abs() < 1e-6, "{p:?}");
        }
        assert_eq!(drum_surfaces(&[], bounds, seed), seed);
    }

    #[test]
    fn pose_targets_follow_the_rigs_anatomical_forward() {
        // The rig faces +X at rest. Its placement turns that axis to world +Z.
        let actor =
            json!({"position": [5., 2., 3.], "rotation": [0., -90., 0.], "scale": [2., 2., 2.]});
        let target = local(&actor, [5., 3., 7.], 1., 90.).unwrap();
        assert!(
            target[0].abs() < 1e-6
                && (target[1] - 1.).abs() < 1e-6
                && (target[2] - 2.).abs() < 1e-6,
            "{target:?}"
        );
    }

    #[test]
    fn explicit_peek_and_independent_instrument_contacts_survive_plain_item_descriptions() {
        let item = |id: &str, kind: &str, name: &str, desc: &str, details: Value| json!({"id":id, "kind":kind, "name":name, "description":desc, "enabled":true, "details":details});
        let plan: Plan = serde_json::from_value(json!({
            "title":"Cellar rehearsal", "summary":"", "notes":[],
            "items":[
                item("toy", "character", "Velvet rabbit", "A small plush toy.", json!({"pose":{"kind":"peek"}})),
                item("chair", "object", "Armchair", "Brown leather.", json!({})),
                item("cellist", "character", "Cellist", "Plays the wooden cello.", json!({"pose":{"kind":"stand","look_at":"score"}})),
                item("cello", "object", "Cello", "", json!({})),
                item("score", "object", "Sheet music", "", json!({}))
            ],
            "layout":[
                {"item":"toy","rel":{"how":"under","to":"chair"}},
                {"item":"cellist","rel":{"how":"beside","to":"cello"}}
            ]
        })).unwrap();
        let cover = super::super::reveal::under_cover(&plan, &plan.items[0]).unwrap();
        assert_eq!(cover.to, "chair");
        let pose = effective(&plan, &plan.items[2]).unwrap();
        assert_eq!(pose.play.as_deref(), Some("cello"));
        assert_eq!(pose.look_at.as_deref(), Some("score"));
    }

    #[test]
    fn a_stage_singer_faces_the_audience_unless_given_a_gaze_target() {
        let mut plan: Plan = serde_json::from_value(json!({
            "title":"Performance",
            "items":[
                {"id":"singer","kind":"character","name":"Singer","description":"Sings.","details":{"pose":{"kind":"stand","sing_into":"mic"}}},
                {"id":"mic","kind":"object","name":"Microphone","description":"On a stand."},
                {"id":"duet","kind":"character","name":"Duet partner","description":"Waits."}
            ]
        })).unwrap();
        assert!(effective(&plan, &plan.items[0]).unwrap().look_at.is_none());
        plan.lineup = vec!["singer".into(), "duet".into()];
        assert_eq!(
            effective(&plan, &plan.items[0]).unwrap().look_at.as_deref(),
            Some("camera")
        );
        plan.items[0].details.pose.as_mut().unwrap().look_at = Some("duet".into());
        assert_eq!(
            effective(&plan, &plan.items[0]).unwrap().look_at.as_deref(),
            Some("duet")
        );
    }

    /// A character holding an object in a hand is posed gripping it: held up
    /// (toward what the layout says), or carried hanging (a lantern); a palm
    /// held out keeps its own pose.
    #[test]
    fn a_held_object_gives_its_holder_a_fist() {
        let plan: Plan = serde_json::from_value(json!({
            "title": "Camp", "summary": "", "notes": [],
            "items": [
                {"id": "kid", "kind": "character", "name": "Kid", "description": "", "enabled": true,
                 "details": {"pose": {"kind": "sit", "on": "log"}, "holds": [{"item": "stick", "socket": "right_hand"}]}},
                {"id": "keeper", "kind": "character", "name": "Keeper", "description": "", "enabled": true, "details": {}},
                {"id": "giant", "kind": "character", "name": "Giant", "description": "", "enabled": true,
                 "details": {"pose": {"kind": "hold_out"}}},
                {"id": "mouse", "kind": "character", "name": "Mouse", "description": "", "enabled": true, "details": {}},
                {"id": "stick", "kind": "object", "name": "Toasting stick", "description": "", "enabled": true, "details": {}},
                {"id": "lantern", "kind": "object", "name": "Old lantern", "description": "", "enabled": true, "details": {}},
                {"id": "log", "kind": "object", "name": "Log", "description": "", "enabled": true, "details": {}},
                {"id": "fire", "kind": "object", "name": "Fire", "description": "", "enabled": true, "details": {}}
            ],
            "layout": [
                {"item": "stick", "rel": {"to": "kid", "how": "held", "hand": "right_hand", "toward": "fire"}},
                {"item": "lantern", "rel": {"to": "keeper", "how": "held", "hand": "left_hand", "toward": "fire"}},
                {"item": "mouse", "rel": {"to": "giant", "how": "held", "hand": "right_hand"}}
            ]
        }))
        .unwrap();
        let of = |id: &str| effective(&plan, plan.items.iter().find(|i| i.id == id).unwrap());
        let kid = of("kid").unwrap();
        assert_eq!(kid.arg(), "sit,hold");
        assert_eq!(
            kid.grip,
            Some(Grip {
                hand: "right".into(),
                style: "raise".into(),
                instrument: None,
                other: None,
                toward: Some("fire".into())
            })
        );
        let keeper = of("keeper").unwrap();
        assert_eq!(keeper.arg(), "stand,hold");
        assert_eq!(
            keeper.grip,
            Some(Grip {
                hand: "left".into(),
                style: "carry".into(),
                instrument: None,
                other: None,
                toward: None
            })
        );
        assert_eq!(of("giant").unwrap().arg(), "hold-out");
        assert_eq!(of("mouse"), None);
    }
}
