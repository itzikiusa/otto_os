//! The assembled scene of a production: one `scene3d` (v2) document that
//! embeds every made model as `otto://design/<id>@latest`, placed on the
//! ground from the plan's layout hints, sized by the scale rule below, spaced
//! by each model's world-space bounds and framed by the camera.
//!
//! BOUNDS: each primitive's POSITION bounds in world space. A rotation mixing
//! axes uses transformed vertices when available to avoid measuring empty
//! corners. Other transforms use the accessor's min/max box. A skinned mesh is placed by its skeleton
//! instead: glTF ignores a skinned mesh node's own transform, and a joint's
//! world matrix times its inverse bind matrix is the matrix of the skeleton
//! root's parent (the armature, which carries Blender's armature scale) at
//! bind, so that node's world matrix is used.
//!
//! SIZE RULE ([`size_m`]): every model stands at its real size in metres, a
//! character's height or an object's longest side: the plan's
//! `details.size_m`, else a default from what it is (a person 1.8 m, a
//! monster or a `big` character 2.7 m, a `small` one 1.0 m; a chest 0.6 m, a
//! sword 1.0 m, a key 0.2 m, a coin pile 0.3 m, another object 0.6 m,
//! `small` 0.25 m, `big` 1.5 m). Forge makes a model about one unit big
//! unless it is told the size, so the model is scaled to it (a model Forge
//! made at that size scales by 1); a library piece is already real and keeps
//! its size, unless the plan gives its metres.
//!
//! PLACES: free models stand on a grid of rows (back, middle, front) and
//! columns (left, centre, right) from their `place`. A piece behind a taller
//! one moves sideways until the camera sees it clear of it. One that faces
//! another turns toward it, but no more than 60° away from the camera. An
//! attached piece (and a prop a character holds) goes after its target, a
//! chain in order: in it ("in", "inside": heaped on top when it would vanish
//! inside), at a socket bone (a hand, the back, a hip, the neck), beside a
//! holder without that bone on that side, or ON its top surface, several
//! there laid out by their own places on its footprint, never into each
//! other. The camera frames every piece's box; the ground is the group's
//! footprint plus half a metre.

mod attachments;
mod opening;
mod performers;
use opening::{scene_doc, OpeningProjection};
#[cfg(test)]
use opening::{shin_gap, side_by_side};
pub(super) use performers::shaft_axis;
use performers::*;
mod bedding;
mod drumsticks;
mod wheels;

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::geometry::{levelling, meets_box, turn_point, Occupancy, Surface, Tri, Turn};
use super::pose::{GripAt, Posed};
use super::{Attach, Item, Plan, SOCKETS};

/// A model's scale: uniform to its real size (the SIZE RULE), per axis
/// toward the plan's `dims_m` when it gives them, within ×0.4–×2.5 of
/// uniform (past that it keeps its own shape: `true`, said in a note).
fn scale_of(plan: &Plan, m: &Model) -> ([f64; 3], bool) {
    let item = |id: &str| plan.items.iter().find(|i| i.id == id);

    let b = &m.bounds;
    let Some(it) = item(&m.item) else {
        return ([1.0; 3], false);
    };
    let dims = it.details.dims_m;
    if m.real_size && it.details.size_m.is_none() && dims.is_none() {
        return ([1.0; 3], false);
    }
    let character = it.kind == "character";
    let size = match character {
        true => b.size(1),
        false => b.size(0).max(b.size(1)).max(b.size(2)),
    };
    let u = match (dims, it.details.size_m) {
        (Some(d), None) if character => d[1] / size,
        (Some(d), None) => d[0].max(d[1]).max(d[2]) / size,
        _ => size_m(plan, it) / size,
    };
    // Furniture is used at its height (a table top at the hand, a stove at
    // the hip): a uniform scale keeps it at the plan's height, else its kind's.
    let tall = (!character)
        .then(|| words(&it.name).last().and_then(|w| furniture_height(w)))
        .flatten()
        .map(|h| dims.map_or(h, |d| d[1]) / b.size(1).max(1e-6));
    let Some(d) = dims else {
        return ([tall.unwrap_or(u); 3], false);
    };
    let want = [0, 1, 2].map(|k| d[k] / b.size(k));
    // A hanging textile can come back as a thick solid. Its width, drop and
    // fold depth are independently adjustable when the plan specifies them.
    if hanging_textile(it) && b.size(1) >= b.size(2) {
        return (want, false);
    }
    match want.iter().all(|w| (0.4 * u..=2.5 * u).contains(w)) {
        true => (want, false),
        false
            if m.real_size
                && words(&it.name)
                    .last()
                    .is_some_and(|w| w == "table" || w == "desk") =>
        {
            // The same catalog table can be requested as a desk or a bedside table.
            // Its top stays at the specified height. Keep each horizontal adjustment bounded.
            let height = want[1];
            (
                [
                    want[0].clamp(0.4 * height, 2.5 * height),
                    height,
                    want[2].clamp(0.4 * height, 2.5 * height),
                ],
                true,
            )
        }
        false => ([tall.unwrap_or(u); 3], true),
    }
}

/// The ordinary height of furniture whose use depends on it (metres).
fn furniture_height(head: &str) -> Option<f64> {
    match head {
        "table" | "desk" => Some(0.75),
        "stove" | "oven" | "cooker" | "counter" | "sink" => Some(0.9),
        "chair" => Some(0.9),
        "door" => Some(2.0),
        _ => None,
    }
}

/// A newly posed swimmer may have a corrected rest heading. Otherwise use
/// the rig's surveyed forward vector, defaulting to +Z on older humanoids.
pub(super) fn rest_heading(plan: &Plan, id: &str) -> f64 {
    let Some(result) = plan
        .items
        .iter()
        .find(|i| i.id == id)
        .and_then(|i| i.result.as_ref())
    else {
        return 0.;
    };
    [
        result.pose.as_ref().map(|r| &r["manifest"]["forward"]),
        result.rig.as_ref().map(|r| &r["forward"]),
    ]
    .into_iter()
    .flatten()
    .filter_map(nums::<3>)
    .find(|v| v[0].hypot(v[2]) > 1e-6)
    .map_or(0.0, |v| v[0].atan2(v[2]).to_degrees())
}

/// The performer was explicitly placed at this station and faces it.
fn station_of<'p>(layout: &'p [super::Placement], performer: &str) -> Option<&'p str> {
    let placement = layout.iter().find(|p| p.item == performer)?;
    let station = placement.rel.as_ref()?;
    (matches!(station.how.as_str(), "behind" | "in_front_of" | "beside")
        && placement.facing.as_deref() == Some(station.to.as_str()))
    .then_some(station.to.as_str())
}

/// Turn each model toward its facing target, carrying the yaw to its riders.
fn face_targets(
    plan: &Plan,
    models: &[Model],
    spots: &mut BTreeMap<&str, Spot>,
    riders: Vec<(&Model, String)>,
) {
    for m in models {
        let Some(f) = plan
            .layout
            .iter()
            .find(|p| p.item == m.item)
            .and_then(|p| p.facing.as_deref())
        else {
            continue;
        };
        let target = spots.get(f).or_else(|| {
            let station = station_of(&plan.layout, f).filter(|id| *id != m.item)?;
            spots.get(station)
        });
        let (Some(me), Some(t)) = (spots.get(m.item.as_str()), target) else {
            continue;
        };
        let yaw = (t.center[0] - me.center[0])
            .atan2(t.center[1] - me.center[1])
            .to_degrees();
        if let Some(me) = spots.get_mut(m.item.as_str()) {
            me.rot_y = ((yaw - rest_heading(plan, &m.item)) * 10.0).round() / 10.0;
        }
    }
    // A piece in or under another turns with it.
    for (m, target) in riders {
        let rot_y = spots[target.as_str()].rot_y;
        if let Some(s) = spots.get_mut(m.item.as_str()) {
            s.rot_y = rot_y;
        }
    }
}

/// When a watcher is beside a performer's station, leave room for the posed
/// performer too. A narrow instrument can otherwise put the watcher inside
/// the span of the playing arms.
fn space_station_watchers(
    plan: &Plan,
    spots: &mut BTreeMap<&str, Spot>,
    supports: &BTreeMap<&str, String>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    for placement in &plan.layout {
        let (Some(rel), Some(subject)) = (&placement.rel, placement.facing.as_deref()) else {
            continue;
        };
        if rel.how != "beside"
            || station_of(&plan.layout, subject) != Some(&rel.to)
            || supports.contains_key(placement.item.as_str())
        {
            continue;
        }
        let side = match rel.side.as_deref() {
            Some("left") => -1.,
            Some("right") => 1.,
            _ => continue,
        };
        let (Some(actor), Some(station)) =
            (spots.get(subject).copied(), spots.get(rel.to.as_str()))
        else {
            continue;
        };
        let edge = |s: &Spot| {
            s.corners()
                .iter()
                .map(|p| side * p[0])
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let extra = edge(&actor) - edge(station);
        if extra <= 0. {
            continue;
        }
        let Some(watcher) = spots.get_mut(placement.item.as_str()) else {
            continue;
        };
        let before = *watcher;
        // Carry the whole row out by the same distance, preserving the gaps
        // between two or more spectators placed on the same side.
        watcher.center[0] += side * extra;
        let yaw = (actor.center[0] - watcher.center[0]).atan2(actor.center[1] - watcher.center[1]);
        watcher.rot_y = yaw.to_degrees() - rest_heading(plan, &placement.item);
        carry(spots, groups, &[], &placement.item, before);
    }
}

/// The camera's first turn (degrees about the up axis): it looks at faces,
/// a three-quarter front view of the main character (the plan's first,
/// else the first line's speaker), side-on to two that face each other,
/// from the side of the hand he holds something in, else the side nearer
/// the usual view; and whether it is such a pair (the other side shows the
/// other profile, not a back).
fn face_turn(plan: &Plan, spots: &BTreeMap<&str, Spot>) -> (f64, bool) {
    let item = |id: &str| plan.items.iter().find(|i| i.id == id);
    let layout = |id: &str| plan.layout.iter().find(|p| p.item == id);
    let usual = VIEW[0].atan2(VIEW[2]).to_degrees();
    let placed = |id: &str| spots.contains_key(id);
    // An upstage curtain establishes the audience side even when the first
    // musician looks across the stage to play an instrument.
    if let Some(stage) = plan.layout.iter().find_map(|p| {
        let rel = p.rel.as_ref()?;
        (rel.how == "on"
            && rel.spot.as_deref().map(slots).is_some_and(|(_, z)| z < 0)
            && placed(&p.item)
            && item(&p.item).is_some_and(hanging_textile)
            && item(&rel.to).is_some_and(|i| stage_name(&i.name)))
        .then(|| spots.get(rel.to.as_str()))
        .flatten()
    }) {
        return (stage.rot_y, false);
    }
    main_character(plan, spots).map_or((usual, false), |id| {
        let facing = layout(id).and_then(|p| p.facing.clone());
        let pair =
            facing.is_some_and(|f| placed(&f) && item(&f).is_some_and(|i| i.kind == "character"));
        // A swimming fish is identifiable in profile by its tail, stripes and fins.
        let profile = item(id).is_some_and(|i| i.details.body.as_deref() == Some("fish"));
        let off = if pair || profile { 90.0 } else { 35.0 };
        let yaw = spots[id].rot_y + rest_heading(plan, id);
        // His own right is his -x: a camera on that side turns by -off.
        let words = item(id)
            .into_iter()
            .flat_map(|i| i.details.holds.iter().map(|h| h.socket.as_str()))
            .chain(plan.layout.iter().filter_map(|p| {
                let rel = p.rel.as_ref().filter(|r| r.to == id && r.how == "held");
                let words = p.attach_to.as_ref().filter(|a| a.item == id);
                rel.and_then(|r| r.hand.as_deref())
                    .or(words.map(|a| a.at.as_str()))
            }));
        let hand = words
            .filter(|w| w.contains("hand") || matches!(*w, "right" | "left"))
            .find_map(|w| match w.contains("right") {
                true => Some(-1.0),
                false => w.contains("left").then_some(1.0),
            });
        let from_usual = |a: f64| ((a - usual + 180.0).rem_euclid(360.0) - 180.0).abs();
        let az = match hand {
            Some(side) => yaw + side * off,
            None if from_usual(yaw + off) <= from_usual(yaw - off) => yaw + off,
            None => yaw - off,
        };
        (az, pair)
    })
}

/// The scene's main character: the plan's first placed character, else the
/// first line's placed speaker.
fn main_character<'p>(plan: &'p Plan, spots: &BTreeMap<&str, Spot>) -> Option<&'p str> {
    let placed = |id: &str| spots.contains_key(id);
    plan.items
        .iter()
        .find(|i| i.kind == "character" && placed(&i.id))
        .map(|i| i.id.as_str())
        .or_else(|| {
            plan.items
                .iter()
                .filter_map(|i| i.details.speaker.as_deref())
                .find(|s| placed(s))
        })
}

/// Where on a target's top each of the pieces on it goes: the spot its
/// words name; with others there, else its `place` (`slot_of`); "beside the
/// keeper": in that piece's cell, next to it.
fn surface_slots(
    kids: &[(&Model, Link)],
    slot_of: impl Fn(&str) -> (i32, i32),
    name: impl Fn(&str) -> String,
) -> Vec<(i32, i32)> {
    let several = kids.len() > 1;
    let mut slot: Vec<(i32, i32)> = kids
        .iter()
        .map(|(m, l)| match l.spot {
            Some(s) => s,
            None if several => slot_of(&m.item),
            None => (0, 0),
        })
        .collect();
    // "beside the keeper": in that piece's cell, next to it.
    for i in 0..kids.len() {
        let at = &kids[i].1.words;
        if slot[i] != (0, 0) || !at.iter().any(|w| w == "beside" || w == "next") {
            continue;
        }
        let named = |id: &str| {
            at.contains(&id.to_string())
                || words(&name(id))
                    .iter()
                    .any(|w| w.len() > 2 && at.contains(w))
        };
        if let Some(j) = (0..kids.len()).find(|&j| j != i && named(&kids[j].0.item)) {
            slot[i] = slot[j];
        }
    }
    slot
}

/// How a piece really goes with its target's real shape (`sf`; `here` and
/// `base` in its space, `height` in the world): in needs a real cavity (else
/// on top, said), under needs real clearance (else beside, said), and "on
/// the floor in front of the bed" in a room is on that room's floor.
fn real_how(
    l: &Link,
    sf: Option<&Surface>,
    t: &Spot,
    here: [f64; 2],
    base: [f64; 2],
    height: f64,
) -> (How, Option<&'static str>) {
    let Some(sf) = sf else {
        return (l.how, None);
    };
    let floor_words = l.words.iter().any(|w| w == "floor" || w == "ground");
    let middle = t.to_model(t.center);
    match l.how {
        How::In if sf.cavity(here[0], here[1]).is_none() => (
            How::On,
            Some("The {to} has no room inside, so the {it} sits on top of it."),
        ),
        How::Beside if floor_words && sf.cavity(middle[0], middle[1]).is_some() => (How::In, None),
        How::Under
            if sf
                .clearance(here[0], here[1], base, t.b.min[1])
                .is_some_and(|room| room * t.scale[1] < height) =>
        {
            (
                How::Beside,
                Some("The {it} does not fit under the {to}, so it stands beside it."),
            )
        }
        how => (how, None),
    }
}

/// Where a piece goes in a container: where its words say, else (no room
/// there) the container's middle; other relations keep their spot.
fn in_spot(
    l: &Link,
    sf: Option<&Surface>,
    model: impl Fn(Option<(i32, i32)>) -> [f64; 2],
) -> Option<(i32, i32)> {
    match (l.how, sf) {
        (How::In, Some(sf)) => {
            let m = model(l.spot);
            l.spot.filter(|_| sf.cavity(m[0], m[1]).is_some())
        }
        _ => l.spot,
    }
}

/// Pieces with their spots.
type Laid<'m> = Vec<(&'m str, Spot)>;

/// Names of a base others stand on.
const BASE_WORDS: &[&str] = &[
    "deck", "pier", "dock", "jetty", "stage", "platform", "floor", "road", "street", "bridge",
    "raft",
];

pub(super) fn is_spotlight(name: &str) -> bool {
    let w = words(name);
    w.iter().any(|s| s == "spotlight") || w.windows(2).any(|pair| pair == ["spot", "light"])
}

/// Gap between placed models (metres).
const GAP: f64 = 0.5;
/// The camera's vertical field of view (degrees), written into the scene.
const FOV: f64 = 50.0;
/// How far from the middle of a top the small things on it are laid (metres).
const ARM: f64 = 0.6;
/// Where the camera looks from (front, a little right, above): rows one
/// behind the other also stand apart on screen.
const VIEW: [f64; 3] = [0.6, 0.5, 1.0];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Bounds {
    /// A model whose bounds are unknown: a 1 m cube standing on the ground.
    pub const UNIT: Bounds = Bounds {
        min: [-0.5, 0.0, -0.5],
        max: [0.5, 1.0, 0.5],
    };

    fn from_points(mut points: impl Iterator<Item = [f64; 3]>) -> Option<Self> {
        let first = points.next()?;
        Some(points.fold(
            Self {
                min: first,
                max: first,
            },
            Self::add,
        ))
    }

    fn size(&self, k: usize) -> f64 {
        (self.max[k] - self.min[k]).max(0.01)
    }

    fn mid(&self, k: usize) -> f64 {
        (self.max[k] + self.min[k]) / 2.0
    }

    fn add(self, p: [f64; 3]) -> Bounds {
        Bounds {
            min: [0, 1, 2].map(|k| self.min[k].min(p[k])),
            max: [0, 1, 2].map(|k| self.max[k].max(p[k])),
        }
    }
}

/// A column-major 4x4 matrix, as glTF writes it.
type Mat = [f64; 16];
const IDENTITY: Mat = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut m = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            m[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
        }
    }
    m
}

fn apply(m: &Mat, p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r])
}

fn nums<const N: usize>(v: &Value) -> Option<[f64; N]> {
    let a = v.as_array().filter(|a| a.len() == N)?;
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x.as_f64()?;
    }
    Some(out)
}

/// A node's local matrix: `matrix`, else T * R * S.
fn local(node: &Value) -> Mat {
    if let Some(m) = nums::<16>(&node["matrix"]) {
        return m;
    }
    let [tx, ty, tz] = nums(&node["translation"]).unwrap_or([0.0; 3]);
    let [x, y, z, w] = nums(&node["rotation"]).unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let [sx, sy, sz] = nums(&node["scale"]).unwrap_or([1.0; 3]);
    #[rustfmt::skip]
    let m = [
        (1.0 - 2.0 * (y * y + z * z)) * sx, (2.0 * (x * y + z * w)) * sx, (2.0 * (x * z - y * w)) * sx, 0.0,
        (2.0 * (x * y - z * w)) * sy, (1.0 - 2.0 * (x * x + z * z)) * sy, (2.0 * (y * z + x * w)) * sy, 0.0,
        (2.0 * (x * z + y * w)) * sz, (2.0 * (y * z - x * w)) * sz, (1.0 - 2.0 * (x * x + y * y)) * sz, 0.0,
        tx, ty, tz, 1.0,
    ];
    m
}

fn glb_json(bytes: &[u8]) -> Option<Value> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" || &bytes[16..20] != b"JSON" {
        return None;
    }
    let len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    serde_json::from_slice(bytes.get(20..20 + len)?).ok()
}

/// A GLB's world-space bounds (see BOUNDS above); `None` for bytes that are no
/// GLB or place no mesh with positions.
pub fn glb_bounds(bytes: &[u8]) -> Option<Bounds> {
    gltf_bounds(&glb_json(bytes)?, glb_bin(bytes))
}

/// A socket's skeleton bones, best first: a name (lowercase, letters only)
/// that ends so (the neck: that contains "neck", else "head").
const JOINTS: &[(&str, &[&str])] = &[
    ("neck", &["neck", "head"]),
    ("head", &["head"]),
    ("right_hand", &["righthand", "handr"]),
    ("left_hand", &["lefthand", "handl"]),
    ("back", &["upperchest", "chest", "spine"]),
    ("hip_left", &["leftupperleg", "thighl"]),
    ("hip_right", &["rightupperleg", "thighr"]),
    ("lid_hinge", &["lidhinge", "chestlid", "chesttop"]),
];

/// Resting socket bones and measured wheel contacts, in the space of [`glb_bounds`].
pub fn glb_joints(bytes: &[u8]) -> BTreeMap<String, [f64; 3]> {
    let mut out = BTreeMap::new();
    let Some(doc) = glb_json(bytes) else {
        return out;
    };
    let Some((nodes, _, world)) = node_worlds(&doc) else {
        return out;
    };
    // Named contact landmarks move with the packaged GLB, including size
    // conversions. An older package without them retains its original layout.
    let count = doc["asset"]["extras"]["wheel_contacts"]["count"]
        .as_u64()
        .unwrap_or(0);
    if matches!(count, 4 | 6 | 8) {
        let mut contacts = BTreeMap::new();
        for (node, matrix) in nodes.iter().zip(&world) {
            if let Some(i) = node["name"]
                .as_str()
                .and_then(|s| s.strip_prefix("WheelContact_"))
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|i| *i < count)
            {
                if let Some(m) = matrix {
                    let point = [m[12], m[13], m[14]];
                    if point.iter().all(|x| x.is_finite()) {
                        contacts.insert(format!("wheel_contact/{i}"), point);
                    }
                }
            }
        }
        if contacts.len() == count as usize {
            out.extend(contacts);
        }
    }
    let joints: Vec<usize> = doc["skins"][0]["joints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|j| Some(j.as_u64()? as usize))
        .collect();
    let name = |j: usize| {
        let n = nodes
            .get(j)
            .and_then(|n| n["name"].as_str())
            .unwrap_or_default();
        n.to_lowercase()
            .chars()
            .filter(char::is_ascii_alphabetic)
            .collect::<String>()
    };
    for (socket, bones) in JOINTS {
        let hit = bones.iter().find_map(|b| {
            joints.iter().copied().find(|j| match *socket {
                "neck" => name(*j).contains(b),
                _ => name(*j).ends_with(b),
            })
        });
        if let Some(m) = hit.and_then(|j| world.get(j).copied().flatten()) {
            out.insert(socket.to_string(), [m[12], m[13], m[14]]);
        }
    }
    // Preserve one lowest joint per leg so the camera can tell when a near
    // leg hides the next one. Duplicate joints on a split foot move together.
    for joint in joints {
        let (Some(node), Some(m)) = (nodes.get(joint), world.get(joint).copied().flatten()) else {
            continue;
        };
        let label = node["name"].as_str().unwrap_or_default();
        if label.starts_with("leg_")
            && !label.contains("_twin")
            && !node["children"].as_array().into_iter().flatten().any(|c| {
                c.as_u64()
                    .and_then(|i| nodes.get(i as usize))
                    .and_then(|n| n["name"].as_str())
                    .is_some_and(|n| n.starts_with("leg_"))
            })
        {
            out.insert(format!("foot/{label}"), [m[12], m[13], m[14]]);
        }
    }
    // Forge stores the first-frame sole centres in the model's coordinate
    // space, after skinning. A folded ankle can be far from the visible foot.
    if let Some(feet) = doc["animations"][0]["extras"]["foot_centres"].as_array() {
        for (i, foot) in feet.iter().enumerate() {
            if let Some(p) = nums::<3>(foot) {
                out.insert(format!("moving_foot/{i}"), p);
            }
        }
    }
    if let Some(axes) = doc["animations"][0]["extras"]["lower_leg_axes"].as_array() {
        for (i, axis) in axes.iter().enumerate() {
            if let (Some(low), Some(high)) = (nums::<3>(&axis[0]), nums::<3>(&axis[1])) {
                out.insert(format!("lower_leg_start/{i}"), low);
                out.insert(format!("lower_leg_end/{i}"), high);
            }
        }
    }
    out
}

/// Where a GLB's neck is (see [`glb_joints`]).
pub fn glb_neck(bytes: &[u8]) -> Option<[f64; 3]> {
    glb_joints(bytes).remove("neck")
}

/// Each node's parent and world matrix (from the scene's roots).
#[allow(clippy::type_complexity)]
fn node_worlds(doc: &Value) -> Option<(&Vec<Value>, Vec<Option<usize>>, Vec<Option<Mat>>)> {
    let nodes = doc["nodes"].as_array()?;
    let mut parent = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for c in n["children"].as_array().into_iter().flatten() {
            if let Some(p) = c.as_u64().and_then(|c| parent.get_mut(c as usize)) {
                *p = Some(i);
            }
        }
    }
    let roots: Vec<usize> =
        match doc["scenes"][doc["scene"].as_u64().unwrap_or(0) as usize]["nodes"].as_array() {
            Some(r) => r
                .iter()
                .filter_map(|v| Some(v.as_u64()? as usize))
                .collect(),
            None => (0..nodes.len()).filter(|i| parent[*i].is_none()).collect(),
        };
    let mut world: Vec<Option<Mat>> = vec![None; nodes.len()];
    let mut stack: Vec<(usize, Mat)> = roots.into_iter().map(|r| (r, IDENTITY)).collect();
    while let Some((i, up)) = stack.pop() {
        let Some(n) = nodes.get(i).filter(|_| world[i].is_none()) else {
            continue;
        };
        let m = mul(&up, &local(n));
        world[i] = Some(m);
        for c in n["children"].as_array().into_iter().flatten() {
            stack.extend(c.as_u64().map(|c| (c as usize, m)));
        }
    }
    Some((nodes, parent, world))
}

fn gltf_bounds(doc: &Value, bin: Option<&[u8]>) -> Option<Bounds> {
    let (nodes, parent, world) = node_worlds(doc)?;
    let mut out: Option<Bounds> = None;
    for (i, n) in nodes.iter().enumerate() {
        let (Some(mesh), Some(own)) = (n["mesh"].as_u64(), world[i]) else {
            continue;
        };
        let m = match n["skin"].as_u64() {
            Some(s) => skeleton_matrix(doc, s as usize, &parent, &world).unwrap_or(own),
            None => own,
        };
        // Rotating the corners of a local bounding box encloses empty space
        // around a diagonal mesh. Only mixed axes need actual vertices.
        let mixed_axes =
            (0..3).any(|row| (0..3).filter(|&k| m[k * 4 + row].abs() > 1e-9).count() > 1);
        let mut add = |point| {
            let w = apply(&m, point);
            out = Some(out.map_or(Bounds { min: w, max: w }, |b| b.add(w)));
        };
        let prims = doc["meshes"][mesh as usize]["primitives"].as_array();
        for p in prims.into_iter().flatten() {
            let Some(acc) = p["attributes"]["POSITION"].as_u64() else {
                continue;
            };
            if mixed_axes {
                if let Some(points) = bin.and_then(|b| floats(doc, b, acc as usize)) {
                    if !points.is_empty() {
                        points.into_iter().for_each(&mut add);
                        continue;
                    }
                }
            }
            let a = &doc["accessors"][acc as usize];
            let (Some(lo), Some(hi)) = (nums::<3>(&a["min"]), nums::<3>(&a["max"])) else {
                continue;
            };
            for k in 0..8 {
                let corner = [0, 1, 2].map(|d| if k >> d & 1 == 0 { lo[d] } else { hi[d] });
                add(corner);
            }
        }
    }
    out
}

/// A GLB's triangles in the same space as [`glb_bounds`]: its display meshes
/// only (no `_LOD1`… copies, no `UCX_` collision hulls), float positions,
/// triangle lists; empty when they can't be read (no binary chunk, a
/// compressed mesh). At most `MAX_TRIS`, evenly picked.
pub fn glb_tris(bytes: &[u8]) -> Vec<Tri> {
    const MAX_TRIS: usize = 200_000;
    let (Some(doc), Some(bin)) = (glb_json(bytes), glb_bin(bytes)) else {
        return Vec::new();
    };
    let Some((nodes, parent, world)) = node_worlds(&doc) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let name = n["name"].as_str().unwrap_or_default();
        let lod = name
            .rsplit_once("_LOD")
            .is_some_and(|(_, k)| k.parse::<u32>().is_ok_and(|k| k > 0));
        let (Some(mesh), Some(own)) = (n["mesh"].as_u64(), world[i]) else {
            continue;
        };
        if lod || name.starts_with("UCX_") {
            continue;
        }
        let m = match n["skin"].as_u64() {
            Some(s) => skeleton_matrix(&doc, s as usize, &parent, &world).unwrap_or(own),
            None => own,
        };
        for p in doc["meshes"][mesh as usize]["primitives"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if p["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let Some(pos) = p["attributes"]["POSITION"]
                .as_u64()
                .and_then(|a| floats(&doc, bin, a as usize))
            else {
                continue;
            };
            let idx: Vec<usize> = match p["indices"].as_u64() {
                Some(a) => match indices(&doc, bin, a as usize) {
                    Some(i) => i,
                    None => continue,
                },
                None => (0..pos.len()).collect(),
            };
            for t in idx.as_chunks::<3>().0 {
                let tri = [0, 1, 2].map(|k| pos.get(t[k]).map(|p| apply(&m, *p)));
                if let [Some(a), Some(b), Some(c)] = tri {
                    out.push([a, b, c]);
                }
            }
        }
    }
    if out.len() > MAX_TRIS {
        let step = out.len().div_ceil(MAX_TRIS);
        out = out.into_iter().step_by(step).collect();
    }
    out
}

/// The GLB's binary chunk (right after the JSON one).
fn glb_bin(bytes: &[u8]) -> Option<&[u8]> {
    let json = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize;
    let at = 20 + json;
    let len = u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
    (bytes.get(at + 4..at + 8)? == b"BIN\0").then_some(())?;
    bytes.get(at + 8..at + 8 + len)
}

/// An accessor's bytes: (data from its first element, element stride, count).
fn accessor<'b>(
    doc: &Value,
    bin: &'b [u8],
    a: usize,
    width: usize,
) -> Option<(&'b [u8], usize, usize)> {
    let acc = &doc["accessors"][a];
    if acc.get("sparse").is_some() {
        return None;
    }
    let view = &doc["bufferViews"][acc["bufferView"].as_u64()? as usize];
    if view["buffer"].as_u64().unwrap_or(0) != 0 {
        return None;
    }
    let start = (view["byteOffset"].as_u64().unwrap_or(0) as usize)
        .checked_add(acc["byteOffset"].as_u64().unwrap_or(0) as usize)?;
    let stride = view["byteStride"].as_u64().map_or(width, |s| s as usize);
    let count = acc["count"].as_u64()? as usize;
    if stride < width {
        return None;
    }
    let end = start
        .checked_add(stride.checked_mul(count.saturating_sub(1))?)?
        .checked_add(width)?;
    Some((bin.get(start..end)?, stride, count))
}

fn floats(doc: &Value, bin: &[u8], a: usize) -> Option<Vec<[f64; 3]>> {
    let acc = &doc["accessors"][a];
    if acc["componentType"].as_u64()? != 5126 || acc["type"].as_str()? != "VEC3" {
        return None;
    }
    let (data, stride, count) = accessor(doc, bin, a, 12)?;
    let f = |o: usize| f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as f64;
    Some(
        (0..count)
            .map(|i| [f(i * stride), f(i * stride + 4), f(i * stride + 8)])
            .collect(),
    )
}

fn indices(doc: &Value, bin: &[u8], a: usize) -> Option<Vec<usize>> {
    let width = match doc["accessors"][a]["componentType"].as_u64()? {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        _ => return None,
    };
    let (data, stride, count) = accessor(doc, bin, a, width)?;
    Some(
        (0..count)
            .map(|i| {
                let o = i * stride;
                match width {
                    1 => data[o] as usize,
                    2 => u16::from_le_bytes([data[o], data[o + 1]]) as usize,
                    _ => u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
                        as usize,
                }
            })
            .collect(),
    )
}

/// The world matrix of a skin's skeleton root's parent (identity at the top).
fn skeleton_matrix(
    doc: &Value,
    skin: usize,
    parent: &[Option<usize>],
    world: &[Option<Mat>],
) -> Option<Mat> {
    let joints: Vec<usize> = doc["skins"][skin]["joints"]
        .as_array()?
        .iter()
        .filter_map(|j| Some(j.as_u64()? as usize))
        .collect();
    let root = doc["skins"][skin]["skeleton"]
        .as_u64()
        .map(|r| r as usize)
        .or_else(|| {
            joints.iter().copied().find(|j| {
                parent
                    .get(*j)
                    .copied()
                    .flatten()
                    .is_none_or(|p| !joints.contains(&p))
            })
        })?;
    match parent.get(root).copied().flatten() {
        Some(p) => world.get(p).copied().flatten(),
        None => Some(IDENTITY),
    }
}

/// One made model: its item id, its artifact and its bounds.
#[derive(Clone)]
pub struct Model {
    pub item: String,
    pub artifact_id: String,
    /// The version it embeds (`@v<seq>`), else `@latest`.
    pub version: Option<i64>,
    pub bounds: Bounds,
    /// Its skeleton's socket bones (`neck`, `right_hand`, `left_hand`,
    /// `back`, `hip_left`, `hip_right`), in its own space (see [`glb_joints`]).
    pub joints: BTreeMap<String, [f64; 3]>,
    /// Already at its real size (a library piece): never scaled.
    pub real_size: bool,
    /// Its triangles (see [`glb_tris`]); empty: only its box is known.
    pub tris: Vec<Tri>,
    /// Posed for this scene ([`super::pose`]): its posed box, seat and palm.
    pub pose: Option<Posed>,
}

impl Model {
    /// Keep real surfaces only when they describe the mesh in its opening pose.
    pub fn apply_pose(&mut self, pose: Posed, receipt: &Value, body: Option<&str>) {
        let manifest = &receipt["manifest"];
        let unchanged = manifest["rest_mesh_unchanged"]
            .as_bool()
            .unwrap_or_else(|| {
                // Older Forge receipts did not store the flag. Its plain stand
                // for a legless body leaves every joint at rest. Bounds in those
                // receipts are rounded to four decimal places.
                body == Some("no_legs")
                    && receipt["spec"] == json!({"kind":"stand"})
                    && manifest["contact"]["feet"] == "none"
                    && (0..3).all(|k| {
                        (pose.min[k] - self.bounds.min[k]).abs() < 0.0001
                            && (pose.max[k] - self.bounds.max[k]).abs() < 0.0001
                    })
            });
        if !unchanged {
            self.tris.clear();
        }
        self.joints.retain(|key, _| !key.starts_with("limb_tip/"));
        if let Some(tips) = manifest["limb_tips"].as_array() {
            for (i, tip) in tips.iter().enumerate() {
                if let Some(point) = nums::<3>(tip) {
                    self.joints.insert(format!("limb_tip/{i}"), point);
                }
            }
        }
        self.pose = Some(pose);
    }

    /// Its box as it stands in the scene: posed, else as made (its size
    /// comes from the latter: a seated person is as tall as standing).
    pub fn now_box(&self) -> Bounds {
        self.pose.map_or(self.bounds, |p| Bounds {
            min: p.min,
            max: p.max,
        })
    }
}

/// Where a held piece goes on its holder: a posed palm held out (`true`: it
/// rests on it), else the hand's (or socket's) bone.
fn socket_point(models: &[Model], l: &Link) -> Option<([f64; 3], bool)> {
    let m = models.iter().find(|x| x.item == l.to)?;
    let hand = |p: &Posed| {
        if p.hand_left {
            "left_hand"
        } else {
            "right_hand"
        }
    };
    let palm = m.pose.and_then(|p| p.palm.filter(|_| l.socket == hand(&p)));
    palm.map(|p| (p, true))
        .or_else(|| body_point(m, l.socket).map(|j| (j, false)))
}

/// Where a body part of a model is now, in its own space: its rest joint
/// ("waist": between its hips), moved with its head when it is posed (the
/// displacement of the head joint from rest). A hand stays at rest.
fn body_point(m: &Model, socket: &str) -> Option<[f64; 3]> {
    let j = |s: &str| m.joints.get(s).copied();
    let rest = match socket {
        "waist" => {
            let (l, r) = (j("hip_left")?, j("hip_right")?);
            [0, 1, 2].map(|k| (l[k] + r[k]) / 2.0)
        }
        s => j(s)?,
    };
    let Some(head) = m
        .pose
        .and_then(|p| p.head)
        .filter(|_| !socket.contains("hand"))
    else {
        return Some(rest);
    };
    let head_at_rest = j("head").or_else(|| {
        let mut neck = j("neck")?;
        neck[1] += 0.5 * head_size(m)?;
        Some(neck)
    });
    Some(head_at_rest.map_or(rest, |base| [0, 1, 2].map(|k| rest[k] + head[k] - base[k])))
}

/// A worn piece (`own`) fitted to the body part of its wearer (on `t`): a
/// hat to the head, a backpack or an apron to the hips' width, a neck piece
/// no wider than the head (than most of the wearer, without a skeleton).
/// Also that part's world size (0: unknown). A held piece keeps its size.
fn fit_worn(models: &[Model], l: &Link, t: Spot, mut own: Spot, held: bool) -> (Spot, f64) {
    let wearer = models.iter().find(|x| x.item == l.to);
    let part = wearer.and_then(|w| match l.socket {
        _ if l.how == How::Head && misplaced_head_patch(w) => None,
        "back" | "waist" => hip_span(w).map(|s| s * t.scale[0]),
        _ => head_size(w).map(|s| s * t.scale[1]),
    });
    let wide = own.size(0).max(own.size(2));
    let want = match (l.how, l.socket, part) {
        _ if held => None,
        (How::Head, _, Some(p)) => Some(1.15 * p),
        (How::EnclosedHead, _, Some(p)) => Some(1.36 * p),
        (_, "back", Some(p)) => Some(1.8 * p),
        (_, "waist", Some(p)) => Some(2.0 * p),
        (_, "neck", p) => {
            Some(p.map_or(0.6 * t.size(0).max(t.size(2)), |p| 1.2 * p)).filter(|w| *w < wide)
        }
        _ => None,
    };
    if let Some(w) = want {
        own.scale = own.scale.map(|s| s * w / wide);
    }
    (own, part.unwrap_or(0.0))
}

/// Fit the shell around the posed head surface with clearance on each axis.
fn enclose_head(wearer: Spot, mut helmet: Spot, head: Bounds) -> Spot {
    let fit = (0..3)
        .map(|k| head.size(k) * wearer.scale[k] / helmet.size(k))
        .fold(0.0, f64::max);
    helmet.scale = helmet.scale.map(|s| 1.12 * fit * s);
    let center = wearer.to_world([0, 1, 2].map(|k| head.mid(k)));
    helmet.rot_y = wearer.rot_y;
    helmet.center = [center[0], center[2]];
    helmet.floor = center[1] - helmet.size(1) / 2.0;
    helmet
}

/// Only a helmet meant to surround the face seats down over it. A tall hat
/// or a cycling helmet still belongs on the crown.
fn encloses_head(item: &Item) -> bool {
    let name = words(&item.name);
    if !name.iter().any(|w| w == "helmet") {
        return false;
    }
    let text = format!(
        "{} {} {}",
        item.name,
        item.description,
        item.details.prompt.as_deref().unwrap_or("")
    )
    .to_lowercase();
    [
        "space",
        "astronaut",
        "full-face",
        "full face",
        "diving",
        "enclosed visor",
    ]
    .iter()
    .any(|term| text.contains(term))
}

/// How tall its head is (its neck to its top), in its own space.
fn head_size(m: &Model) -> Option<f64> {
    Some(m.bounds.max[1] - m.joints.get("neck")?[1]).filter(|h| *h > 0.0)
}

/// A rig can label the tip of an outstretched limb as the head. A tiny
/// patch well below the crown cannot support a hat on an upright figure.
fn misplaced_head_patch(m: &Model) -> bool {
    let body = m.now_box();
    m.pose.and_then(|p| p.head_bounds).is_some_and(|head| {
        !m.tris.is_empty()
            && body.size(1) >= body.size(0).max(body.size(2))
            && head.max[1] < body.max[1] - 0.25 * body.size(1)
            && (0..3).all(|k| head.size(k) < 0.035 * body.size(1))
    })
}

/// A turn (degrees about x, y, z, applied as the viewer does) as a matrix.
fn turn_matrix(e: [f64; 3]) -> [[f64; 3]; 3] {
    let [(sa, ca), (sb, cb), (sc, cc)] = e.map(|d| d.to_radians().sin_cos());
    [
        [cb * cc, -cb * sc, sb],
        [ca * sc + sa * sb * cc, ca * cc - sa * sb * sc, -sa * cb],
        [sa * sc - ca * sb * cc, sa * cc + ca * sb * sc, ca * cb],
    ]
}

fn compose_turns(a: Turn, b: Turn) -> Turn {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// The turn (degrees, x-y-z) of a rotation matrix.
fn turn_of(r: [[f64; 3]; 3]) -> [f64; 3] {
    let y = r[0][2].clamp(-1.0, 1.0).asin();
    let (x, z) = match r[0][2].abs() < 0.999_999 {
        true => ((-r[1][2]).atan2(r[2][2]), (-r[0][1]).atan2(r[0][0])),
        false => (r[2][1].atan2(r[1][1]), 0.0),
    };
    [x, y, z].map(f64::to_degrees)
}

fn turned(r: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2])
}

/// The world orientation of an upright handle held along the grip axis.
fn grip_rotation(t: Spot, g: GripAt) -> Turn {
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let yaw = t.rot_y.to_radians();
    let turn = |v: [f64; 3]| {
        [
            v[0] * yaw.cos() + v[2] * yaw.sin(),
            v[1],
            -v[0] * yaw.sin() + v[2] * yaw.cos(),
        ]
    };
    let unit = |v: [f64; 3]| {
        let n = dot(v, v).sqrt().max(1e-9);
        v.map(|x| x / n)
    };
    let a = unit(turn(g.axis));
    let y = if g.carry { a.map(|x| -x) } else { a };
    let f = turn([0.0, 0.0, 1.0]);
    let z = match [0, 1, 2].map(|k| f[k] - y[k] * dot(f, y)) {
        z if dot(z, z) > 1e-6 => unit(z),
        _ => unit(turn([1.0, 0.0, 0.0])),
    };
    let x = [
        y[1] * z[2] - y[2] * z[1],
        y[2] * z[0] - y[0] * z[2],
        y[0] * z[1] - y[1] * z[0],
    ];
    [0, 1, 2].map(|i| [x[i], y[i], z[i]])
}

/// Mount an upright handle one sixth of the way up, or suspend a carried load from its top.
fn grip_spot(t: Spot, own: Spot, g: GripAt) -> Spot {
    let r = grip_rotation(t, g);
    let (b, s) = (own.b, own.scale);
    let gy = match g.carry {
        true => b.max[1],
        false => b.min[1] + (b.max[1] - b.min[1]) / 6.0,
    };
    mounted(
        t,
        own,
        r,
        [b.mid(0) * s[0], gy * s[1], b.mid(2) * s[2]],
        g.point,
    )
}

fn wind_spot(t: Spot, own: Spot, wind: super::pose::WindMount) -> Spot {
    let yaw = turn_matrix([0., t.rot_y, 0.]);
    let r =
        [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| yaw[i][k] * wind.rotation[k][j]).sum()));
    mounted(
        t,
        own,
        r,
        [0, 1, 2].map(|i| wind.tip[i] * own.scale[i]),
        wind.point,
    )
}

fn mounted(t: Spot, own: Spot, r: [[f64; 3]; 3], anchor: [f64; 3], point: [f64; 3]) -> Spot {
    let at = turned(r, anchor);
    let p = t.to_world(point);
    let origin = [0, 1, 2].map(|k| p[k] - at[k]);
    let s = Spot {
        rot_y: t.rot_y,
        tilt: Some((origin, turn_of(r))),
        ..own
    };
    let c = s.corners();
    Spot {
        center: [0, 2].map(|k| c.iter().map(|p| p[k]).sum::<f64>() / 8.0),
        floor: c.iter().map(|p| p[1]).fold(f64::MAX, f64::min),
        ..s
    }
}

/// The far end of a piece held up in a fist (the top of it as made).
fn tip(s: &Spot) -> Option<[f64; 3]> {
    let (origin, e) = s.tilt?;
    let (b, sc) = (s.b, s.scale);
    let w = turned(
        turn_matrix(e),
        [b.mid(0) * sc[0], b.max[1] * sc[1], b.mid(2) * sc[2]],
    );
    Some([0, 1, 2].map(|k| origin[k] + w[k]))
}

/// Scenery standing free (an object with no relation to anything and
/// nothing on it) between the first view's camera (`cam`) and what the
/// cast gathers round goes to the far side of it, mirrored through it,
/// when nothing stands there: a tent never stands in front of the
/// campfire. Whether any moved (the view is then looked for again).
fn behind_focus(
    plan: &Plan,
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
    cam: [f64; 3],
    is_char: impl Fn(&str) -> bool,
) -> bool {
    let focus = focus_of(plan, spots, is_char);
    let Some(f) = focus.and_then(|f| spots.get(f).copied()) else {
        return false;
    };
    let d = [cam[0] - f.center[0], cam[2] - f.center[1]];
    let d = d.map(|x| x / d[0].hypot(d[1]).max(1e-9));
    let mut moved = false;
    let lone = |id: &str| {
        let item = plan.items.iter().find(|i| i.id == id);
        let at = plan.layout.iter().find(|p| p.item == id);
        item.is_some_and(|i| i.kind == "object" && i.details.attach_to.is_none())
            && at.is_some_and(|p| p.rel.is_none() && p.attach_to.is_none())
            && !groups.values().flatten().any(|c| c == id)
    };
    let ids: Vec<&str> = spots
        .keys()
        .copied()
        .filter(|id| Some(*id) != focus && lone(id))
        .collect();
    for id in ids {
        let s = spots[id];
        let ahead = (s.center[0] - f.center[0]) * d[0] + (s.center[1] - f.center[1]) * d[1];
        // What rests on it goes with it (an owl's branch on the tent).
        let mut with = vec![id.to_string()];
        let mut i = 0;
        while i < with.len() {
            with.extend(groups.get(&with[i]).into_iter().flatten().cloned());
            i += 1;
        }
        // Its mirror place, else farther out on that side until clear.
        let clear = |to: [f64; 2]| {
            (spots.iter())
                .filter(|(o, _)| !with.iter().any(|w| w == *o))
                .all(|(_, o)| {
                    (o.center[0] - to[0]).hypot(o.center[1] - to[1]) > o.radius() + s.radius()
                })
        };
        let to = (0..9)
            .map(|k| {
                let back = 2.0 * ahead + 0.5 * k as f64;
                [s.center[0] - back * d[0], s.center[1] - back * d[1]]
            })
            .find(|to| clear(*to));
        if let Some(to) = to.filter(|_| ahead > 0.0) {
            let by = [to[0] - s.center[0], to[1] - s.center[1]];
            for w in &with {
                if let Some(s) = spots.get_mut(w.as_str()) {
                    s.center = [s.center[0] + by[0], s.center[1] + by[1]];
                    moved = true;
                }
            }
        }
    }
    moved
}

/// The fist a held piece is in (`l`: held in a hand): its holder posed
/// gripping with that hand.
fn fist(models: &[Model], l: &Link) -> Option<GripAt> {
    let pose = models.iter().find(|m| m.item == l.to)?.pose?;
    [pose.grip, pose.other_grip]
        .into_iter()
        .flatten()
        .find(|g| {
            l.how == How::Socket && l.socket == if g.left { "left_hand" } else { "right_hand" }
        })
}

/// What rests on a piece held up in a fist (a marshmallow on a stick) sits
/// on its far end.
fn onto_tips(
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
    held: &[&str],
) {
    for (target, kids) in groups {
        let Some(t) = spots.get(target.as_str()).and_then(tip) else {
            continue;
        };
        for k in kids.iter().filter(|k| !held.contains(&k.as_str())) {
            if let Some(s) = spots.get_mut(k.as_str()) {
                (s.center, s.floor) = ([t[0], t[2]], t[1] - s.size(1) / 2.0);
            }
        }
    }
}

/// A holder whose piece held toward another (a stick over the fire) ends
/// short of it moves closer by what is missing, with all it holds: sliding
/// along its seat while its sitting point stays on the seat's top, else (on
/// its feet) by up to half its own width, never into the target.
fn step_closer(
    plan: &Plan,
    models: &[Model],
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    for p in &plan.layout {
        let Some((holder, to)) = p
            .rel
            .as_ref()
            .and_then(|r| Some((r.to.as_str(), r.toward.as_deref()?)))
        else {
            continue;
        };
        let (Some(end), Some(t), Some(h)) = (
            spots.get(p.item.as_str()).and_then(tip),
            spots.get(to).copied(),
            spots.get(holder).copied(),
        ) else {
            continue;
        };
        let v = [t.center[0] - end[0], t.center[1] - end[2]];
        let need = v[0].hypot(v[1]) - 0.5 * t.radius();
        if need <= 0.0 {
            continue;
        }
        let dir = v.map(|x| x / v[0].hypot(v[1]));
        let seat = (plan.layout.iter())
            .find(|q| q.item == holder)
            .and_then(|q| q.rel.as_ref().filter(|r| r.how == "on"))
            .and_then(|r| spots.get(r.to.as_str()).copied())
            .zip(
                models
                    .iter()
                    .find(|m| m.item == holder)
                    .and_then(|m| m.pose?.seat),
            );
        let fits = |d: f64| match seat {
            Some((s, at)) => {
                let w = h.to_world(at);
                let m = s.to_model([w[0] + dir[0] * d, w[2] + dir[1] * d]);
                let inset = |k: usize| 0.1 * (s.b.max[k] - s.b.min[k]);
                [(0, m[0]), (2, m[1])]
                    .iter()
                    .all(|&(k, x)| x > s.b.min[k] + inset(k) && x < s.b.max[k] - inset(k))
            }
            None => {
                let c = [h.center[0] + dir[0] * d, h.center[1] + dir[1] * d];
                d <= 0.5 * h.radius()
                    && (c[0] - t.center[0]).hypot(c[1] - t.center[1]) > h.radius() + t.radius()
            }
        };
        let mut d = need;
        if !fits(d) {
            // The last part of a log can be enough for the stick to reach.
            // Keep the largest safe slide, to within a millimetre on a typical seat.
            let (mut near, mut far) = (0.0, d);
            for _ in 0..12 {
                let mid = 0.5 * (near + far);
                if fits(mid) {
                    near = mid;
                } else {
                    far = mid;
                }
            }
            d = near;
        }
        if d < 0.001 {
            continue;
        }
        let mut moved = vec![holder.to_string()];
        while let Some(id) = moved.pop() {
            if let Some(s) = spots.get_mut(id.as_str()) {
                s.center = [s.center[0] + dir[0] * d, s.center[1] + dir[1] * d];
                if let Some((o, _)) = s.tilt.as_mut() {
                    (o[0], o[2]) = (o[0] + dir[0] * d, o[2] + dir[1] * d);
                }
            }
            moved.extend(groups.get(&id).into_iter().flatten().cloned());
        }
    }
}

/// A piece held toward another (a stick over the fire) whose far end does
/// not reach over it, said plainly.
fn short_of(
    plan: &Plan,
    spots: &BTreeMap<&str, Spot>,
    name: &dyn Fn(&str) -> String,
) -> Vec<String> {
    let mut said = Vec::new();
    for p in &plan.layout {
        let Some(to) = p.rel.as_ref().and_then(|r| r.toward.as_deref()) else {
            continue;
        };
        let (Some(end), Some(t)) = (spots.get(p.item.as_str()).and_then(tip), spots.get(to)) else {
            continue;
        };
        if (end[0] - t.center[0]).hypot(end[2] - t.center[1]) > t.radius() {
            said.push(format!(
                "The {} does not reach over the {}: its holder is too far from it.",
                name(&p.item),
                name(to)
            ));
        }
    }
    said
}

/// How far apart its hips are, in its own space.
fn hip_span(m: &Model) -> Option<f64> {
    let (l, r) = (m.joints.get("hip_left")?, m.joints.get("hip_right")?);
    Some((l[0] - r[0]).hypot(l[2] - r[2])).filter(|s| *s > 0.0)
}

/// Object defaults of the SIZE RULE (module doc), by word.
const OBJECT_SIZES: &[(&str, f64)] = &[("chest", 0.6), ("sword", 1.0), ("key", 0.2), ("coin", 0.3)];

/// Lowercase words, a plural's `s` dropped.
pub(super) fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphabetic())
        .map(|w| w.strip_suffix('s').unwrap_or(w).to_string())
        .collect()
}

/// The SIZE RULE (module doc): what `it` measures in metres, its height for a
/// character, its longest side for an object.
pub fn size_m(plan: &Plan, it: &Item) -> f64 {
    if let Some(m) = it.details.size_m {
        return m;
    }
    let hint = plan
        .layout
        .iter()
        .find(|p| p.item == it.id)
        .and_then(|p| p.scale_hint.as_deref());
    let prompt = it.details.prompt.as_deref().unwrap_or_default();
    if it.kind == "character" {
        let monster = ["monster", "giant", "ogre", "troll", "dragon", "beast"];
        let big = [&it.name, prompt]
            .iter()
            .any(|t| words(t).iter().any(|w| monster.contains(&w.as_str())));
        return match hint {
            Some("small") => 1.0,
            Some("big") => 2.7,
            _ if big => 2.7,
            _ => 1.8,
        };
    }
    let known = |w: &String| OBJECT_SIZES.iter().find(|(k, _)| k == w).map(|(_, m)| *m);
    // A name's head word comes last ("Chest Key"); a sentence names its
    // subject first ("A chest full of gold coins").
    let named = words(&it.name).iter().rev().find_map(known);
    let said = || {
        [it.description.as_str(), prompt]
            .iter()
            .find_map(|t| words(t).iter().find_map(known))
    };
    named.or_else(said).unwrap_or(match hint {
        Some("small") => 0.25,
        Some("big") => 1.5,
        _ => 0.6,
    })
}

#[derive(Clone, Copy)]
struct Spot {
    /// The footprint centre on the ground (x, z) the model is turned about.
    center: [f64; 2],
    /// The height of the model's lowest point.
    floor: f64,
    rot_y: f64,
    /// Per axis (x, y, z): uniform but for a piece the plan gives `dims_m`.
    scale: [f64; 3],
    b: Bounds,
    /// Held in a fist ([`grip_spot`]): its origin and its turn (degrees,
    /// x-y-z) instead of standing upright at `center` and `floor`.
    tilt: Option<([f64; 3], [f64; 3])>,
}

impl Spot {
    fn for_model(m: &Model, scale: [f64; 3]) -> Self {
        Spot {
            center: [0.0; 2],
            floor: 0.0,
            rot_y: 0.0,
            scale,
            b: m.now_box(),
            tilt: None,
        }
    }

    /// Its world size along its own axis `k`.
    fn size(&self, k: usize) -> f64 {
        self.b.size(k) * self.scale[k]
    }

    /// The scene object's position: its origin, so that the turned model's
    /// box centre lands on `center` and its lowest point on `floor`.
    fn pos(&self) -> [f64; 3] {
        let (s, t) = (self.scale, self.rot_y.to_radians());
        let (mx, mz) = (self.b.mid(0) * s[0], self.b.mid(2) * s[2]);
        [
            self.center[0] - (mx * t.cos() + mz * t.sin()),
            self.floor - self.b.min[1] * s[1],
            self.center[1] - (-mx * t.sin() + mz * t.cos()),
        ]
    }

    /// The radius of the footprint around `center` (any turn stays inside).
    fn radius(&self) -> f64 {
        self.size(0).hypot(self.size(2)) / 2.0
    }

    fn top(&self) -> f64 {
        self.floor + self.size(1)
    }

    /// The 8 world corners of the model's box (through its scale and turn).
    fn corners(&self) -> [[f64; 3]; 8] {
        std::array::from_fn(|k| {
            self.to_world([0, 1, 2].map(|d| {
                if k >> d & 1 == 0 {
                    self.b.min[d]
                } else {
                    self.b.max[d]
                }
            }))
        })
    }

    /// A world point (x, z) in the model's own space.
    fn to_model(self, p: [f64; 2]) -> [f64; 2] {
        let (o, t) = (self.pos(), self.rot_y.to_radians());
        let (dx, dz) = (p[0] - o[0], p[1] - o[2]);
        [
            (dx * t.cos() - dz * t.sin()) / self.scale[0],
            (dx * t.sin() + dz * t.cos()) / self.scale[2],
        ]
    }

    /// A model point in the world (held in a fist: through its full turn).
    fn to_world(self, v: [f64; 3]) -> [f64; 3] {
        if let Some((origin, e)) = self.tilt {
            let w = turned(turn_matrix(e), [0, 1, 2].map(|k| v[k] * self.scale[k]));
            return [0, 1, 2].map(|k| origin[k] + w[k]);
        }
        let (o, t) = (self.pos(), self.rot_y.to_radians());
        let (x, z) = (v[0] * self.scale[0], v[2] * self.scale[2]);
        [
            o[0] + x * t.cos() + z * t.sin(),
            o[1] + v[1] * self.scale[1],
            o[2] - x * t.sin() + z * t.cos(),
        ]
    }

    /// A model height in the world.
    fn world_y(&self, y: f64) -> f64 {
        self.pos()[1] + y * self.scale[1]
    }

    /// A model rectangle (min x, min z, max x, max z) as a world centre and
    /// half sizes (of its turned box).
    fn world_rect(&self, r: [f64; 4]) -> ([f64; 2], [f64; 2]) {
        let ps = [(r[0], r[1]), (r[2], r[1]), (r[0], r[3]), (r[2], r[3])]
            .map(|(x, z)| self.to_world([x, 0.0, z]));
        let lo = [0, 2].map(|k| ps.iter().map(|p| p[k]).fold(f64::MAX, f64::min));
        let hi = [0, 2].map(|k| ps.iter().map(|p| p[k]).fold(f64::MIN, f64::max));
        (
            [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0],
            [(hi[0] - lo[0]) / 2.0, (hi[1] - lo[1]) / 2.0],
        )
    }

    /// Half the world footprint across x and z (its turned box).
    fn half(&self) -> [f64; 2] {
        let t = self.rot_y.to_radians();
        let (w, d) = (self.size(0), self.size(2));
        let (c, n) = (t.cos().abs(), t.sin().abs());
        [(w * c + d * n) / 2.0, (w * n + d * c) / 2.0]
    }
}

/// Free footprints never meet: the circles any turn stays inside.
fn clear(spots: &[Spot]) -> bool {
    spots.iter().enumerate().all(|(i, a)| {
        spots[i + 1..].iter().all(|b| {
            let d = (a.center[0] - b.center[0]).hypot(a.center[1] - b.center[1]);
            d + 1e-9 >= a.radius() + b.radius()
        })
    })
}

/// A cell's models in a row, each in its circle, `GAP` apart, around `x`.
fn cell_width(cell: &[(&Model, Spot)]) -> f64 {
    cell.iter().map(|(_, s)| 2.0 * s.radius()).sum::<f64>()
        + GAP * cell.len().saturating_sub(1) as f64
}

/// How wide each column (left, centre, right) and how deep each row (back,
/// middle, front) of a grid of cells is.
fn grid_sizes(cells: &BTreeMap<(i32, i32), Vec<(&Model, Spot)>>) -> ([f64; 3], [f64; 3]) {
    let (mut col_w, mut row_d) = ([0.0f64; 3], [0.0f64; 3]);
    for ((x, z), cell) in cells {
        let (c, r) = ((x + 1) as usize, (z + 1) as usize);
        col_w[c] = col_w[c].max(cell_width(cell));
        row_d[r] = cell
            .iter()
            .map(|(_, s)| 2.0 * s.radius())
            .fold(row_d[r], f64::max);
    }
    (col_w, row_d)
}

/// Each cell packed left to right around its column's `col_x` and row's `row_z`.
fn pack<'m>(
    cells: &BTreeMap<(i32, i32), Vec<(&'m Model, Spot)>>,
    col_x: [f64; 3],
    row_z: [f64; 3],
) -> Vec<(&'m str, Spot)> {
    let mut out = Vec::new();
    for ((x, z), cell) in cells {
        let mut cursor = col_x[(x + 1) as usize] - cell_width(cell) / 2.0;
        for (m, s) in cell {
            let w = 2.0 * s.radius();
            let center = [cursor + w / 2.0, row_z[(z + 1) as usize]];
            cursor += w + GAP;
            out.push((m.item.as_str(), Spot { center, ..*s }));
        }
    }
    out
}

/// Pieces ON a target `t` in their cells: laid on its real top (the
/// standable footprint of its mesh `sf`, else its box), each base over real
/// surface (a look straight down under it); one that can't stand where asked
/// goes to the nearest place that holds it. Plain notes, `{it}` / `{to}` the
/// piece's and the target's names.
fn on_top<'m>(
    t: Spot,
    sf: Option<&Surface>,
    cells: &BTreeMap<(i32, i32), Vec<(&'m Model, Spot)>>,
    packed: bool,
    character: impl Fn(&str) -> Option<(&'m Model, bool)>,
) -> (Laid<'m>, Vec<(&'m str, &'static str)>) {
    let (c, h) = match sf {
        Some(sf) => t.world_rect(sf.footprint()),
        None => (t.center, t.half()),
    };
    // Small things on a long top stay within arm's reach of each other, in
    // its middle (cups beside the teapot, never at a 3 m table's two ends).
    let big = (cells.values().flatten())
        .map(|(_, s)| s.half()[0].max(s.half()[1]))
        .fold(0.0, f64::max);
    let area = match big <= ARM / 2.0 {
        true => (c, h.map(|h| h.min(ARM))),
        false => (c, h),
    };
    // Free pieces on a floor-like base stand together in its middle.
    let (laid, roomy) = match packed {
        true => {
            let (col_w, row_d) = grid_sizes(cells);
            let (col_x, row_z) = spread(col_w, row_d);
            let shift = |v: [f64; 3], by: f64| v.map(|x| x + by);
            (
                pack(cells, shift(col_x, area.0[0]), shift(row_z, area.0[1])),
                true,
            )
        }
        false => lay_on(area, cells),
    };
    let mut said = Vec::new();
    if !roomy {
        said.push((
            "",
            "There was not much room on the {to}, so the things on it stand close together.",
        ));
    }
    let mut standing: Vec<(&str, Spot)> = Vec::new();
    for (id, s) in laid {
        let actor = character(id);
        let s = Spot {
            floor: t.top(),
            rot_y: t.rot_y,
            ..s
        };
        let others: Vec<Spot> = standing.iter().map(|(_, s)| *s).collect();
        let s = match sf.map(|sf| {
            if let Some((m, true)) = actor {
                // An asymmetric bird can have its feet far from the centre of
                // its wings and tail. Place its actual feet over the crown.
                let [x, z] = sf.crown();
                let crown = t.to_world([x, 0., z]);
                let feet = standing_base(s, m);
                let shift = [crown[0] - feet.center[0], crown[2] - feet.center[1]];
                let perched = Spot {
                    center: [0, 1].map(|k| s.center[k] + shift[k]),
                    ..s
                };
                if let Some(spot) = stand_on_base(sf, &t, perched, m, &others) {
                    return Some(spot);
                }
            }
            stand_on(sf, &t, s, &others)
                .or_else(|| actor.and_then(|(m, _)| stand_on_base(sf, &t, s, m, &others)))
        }) {
            Some(Some((s, moved))) => {
                if moved {
                    said.push((id, "The {it} could not stand where the plan asked on the {to}, so it stands at the nearest place that holds it."));
                }
                s
            }
            // A real top too uneven for the whole base (a stove's burners, a
            // plate's dip) still holds a piece that fits on it, at its middle point.
            Some(None) if uneven(sf, &t, s, &others).is_some() => {
                uneven(sf, &t, s, &others).unwrap_or(s)
            }
            Some(None) => {
                said.push((
                    id,
                    "There is no room for the {it} on the {to}, so it stands beside it.",
                ));
                beside(&t, s)
            }
            None => s,
        };
        standing.push((id, s));
    }
    (standing, said)
}

/// What a character sits on, when no pose can sit it there.
const SEATS: &[&str] = &[
    "chair", "log", "bench", "stool", "sofa", "couch", "seat", "armchair",
];

/// Without a sitting pose a character "on" a seat (a log, a chair, a bench)
/// stands just in front of it, between it and what it faces, on the seat's
/// floor, never on top of it; what it carries goes with it.
fn before_seats(
    plan: &Plan,
    models: &[Model],
    surfaces: &BTreeMap<&str, Surface>,
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let item = |id: &str| plan.items.iter().find(|i| i.id == id);
    let mut notes = Vec::new();
    for p in &plan.layout {
        let Some(c) = item(&p.item).filter(|c| c.kind == "character") else {
            continue;
        };
        // A bird's feet rest on its perch. Moving an unposed bird in front
        // of the support would move it all the way down to the ground.
        if c.details.body.as_deref() == Some("bird") {
            continue;
        }
        let pose = c.details.pose.as_ref();
        let rel = (p.rel.as_ref())
            .filter(|r| matches!(r.how.as_str(), "on" | "in"))
            .map(|r| r.to.as_str());
        let Some(seat) = pose.and_then(|x| x.on.as_deref()).or(rel).and_then(item) else {
            continue;
        };
        let (Some(&was), Some(&t)) = (spots.get(p.item.as_str()), spots.get(seat.id.as_str()))
        else {
            continue;
        };
        // Seated by its pose: the point its body rests on goes on the seat's top middle.
        if let Some(at) = models
            .iter()
            .find(|m| m.item == p.item)
            .and_then(|m| m.pose?.seat)
        {
            let w = was.to_world(at);
            let [x, z] = t.to_model(t.center);
            let seat_y = surfaces.get(seat.id.as_str()).map_or(t.top(), |surface| {
                // A slat gap at the centre still has a seat on either side.
                let y = surface.top_at(x, z).unwrap_or(surface.max_top);
                t.world_y(y)
            });
            if let Some(s) = spots.get_mut(p.item.as_str()) {
                s.center = [
                    was.center[0] + t.center[0] - w[0],
                    was.center[1] + t.center[1] - w[2],
                ];
                s.floor = was.floor + seat_y - w[1];
            }
            carry(spots, groups, &[], &p.item, was);
            continue;
        }
        // A legless rider already rests on its base. There are no knees
        // to bend, and moving it to the floor breaks the support relation.
        if c.details.body.as_deref() == Some("no_legs") {
            continue;
        }
        let sits = pose.is_some_and(|x| x.kind == "sit")
            || words(&seat.name)
                .iter()
                .any(|w| SEATS.contains(&w.as_str()));
        if !sits {
            continue;
        }
        // Toward what it faces (the fire), else toward the camera's side.
        let to = (p.facing.as_deref().and_then(|f| spots.get(f)))
            .map_or([t.center[0], t.center[1] + 1.0], |f| f.center);
        let (dx, dz) = (to[0] - t.center[0], to[1] - t.center[1]);
        let n = dx.hypot(dz).max(1e-6);
        // Its depth along the way (not its arm span) just clear of the seat.
        let along = |s: &Spot| (dx / n * s.half()[0]).abs() + (dz / n * s.half()[1]).abs();
        let d = along(&t) + along(&was) + 0.05;
        if let Some(s) = spots.get_mut(p.item.as_str()) {
            s.center = [t.center[0] + dx / n * d, t.center[1] + dz / n * d];
            s.floor = t.floor.max(0.0);
        }
        carry(spots, groups, &[], &p.item, was);
        notes.push(format!(
            "Otto cannot make the {} sit yet, so it stands in front of the {}.",
            c.name, seat.name
        ));
    }
    notes
}

/// Arrange the requested performers across their shared floor or platform.
/// Move a performer's stand and all its riders as one station, preserving
/// hand and instrument contacts when the final poses are placed.
fn line_up(
    plan: &Plan,
    spots: &mut BTreeMap<&str, Spot>,
    supports: &BTreeMap<&str, String>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    let mut stations = Vec::new();
    for id in &plan.lineup {
        let Some(actor) = spots.get(id.as_str()).copied() else {
            continue;
        };
        let near = plan
            .layout
            .iter()
            .find(|p| p.item == *id)
            .and_then(|p| p.rel.as_ref())
            .filter(|r| {
                matches!(r.how.as_str(), "beside" | "behind" | "in_front_of")
                    && !plan.lineup.contains(&r.to)
            });
        let root = near.map_or(id.as_str(), |r| r.to.as_str());
        let Some(anchor) = spots.get(root).copied() else {
            continue;
        };
        let lo = (actor.center[0] - actor.half()[0]).min(anchor.center[0] - anchor.half()[0]);
        let hi = (actor.center[0] + actor.half()[0]).max(anchor.center[0] + anchor.half()[0]);
        stations.push((root, actor, anchor, lo, hi));
    }
    if stations.len() < 2 {
        return;
    }
    let base = supports.get(stations[0].0);
    if stations.iter().any(|s| supports.get(s.0) != base)
        || stations
            .iter()
            .map(|s| s.0)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != stations.len()
    {
        return;
    }
    let width = stations.iter().map(|s| s.4 - s.3).sum::<f64>();
    let depth = stations.iter().map(|s| s.1.center[1]).sum::<f64>() / stations.len() as f64;
    let (center, gap) = match base.and_then(|id| spots.get(id.as_str())) {
        Some(deck) => (
            deck.center[0],
            ((deck.half()[0] * 2.0 - width) / (stations.len() + 1) as f64).clamp(0.0, 0.40),
        ),
        None => (
            stations.iter().map(|s| s.1.center[0]).sum::<f64>() / stations.len() as f64,
            0.25,
        ),
    };
    // If the requested stations do not fit on a deck, preserve the valid
    // supported layout. The planner can choose a larger platform.
    if base
        .and_then(|id| spots.get(id.as_str()))
        .is_some_and(|s| width > s.half()[0] * 2.0)
    {
        return;
    }
    let mut x = center - (width + gap * (stations.len() - 1) as f64) / 2.0;
    for (root, actor, anchor, lo, hi) in stations {
        let s = spots.get_mut(root).expect("station exists");
        s.center[0] += x - lo;
        s.center[1] += depth - actor.center[1];
        carry(spots, groups, &[], root, anchor);
        x += hi - lo + gap;
    }
}

/// After moving the band across the platform, sample the new spot under
/// each station and each pair of feet. Descendants of a stand move with it.
fn settle_lineup(
    plan: &Plan,
    models: &[Model],
    geo: &BTreeMap<&str, Surface>,
    supports: &BTreeMap<&str, String>,
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    let mut ids = Vec::new();
    for id in &plan.lineup {
        let r = plan
            .layout
            .iter()
            .find(|p| p.item == *id)
            .and_then(|p| p.rel.as_ref());
        if let Some(r) = r.filter(|r| {
            matches!(r.how.as_str(), "beside" | "behind" | "in_front_of")
                && !plan.lineup.contains(&r.to)
        }) {
            if !ids.contains(&r.to.as_str()) {
                ids.push(r.to.as_str())
            }
        }
    }
    ids.extend(plan.lineup.iter().map(String::as_str));
    for id in ids {
        let Some(base) = supports.get(id) else {
            continue;
        };
        let (Some(surface), Some(deck), Some(was), Some(model)) = (
            geo.get(base.as_str()),
            spots.get(base.as_str()).copied(),
            spots.get(id).copied(),
            models.iter().find(|m| m.item == id),
        ) else {
            continue;
        };
        let feet = standing_base(was, model);
        let at = deck.to_model(feet.center);
        let Some(y) = surface.top_at(at[0], at[1]) else {
            continue;
        };
        let delta = deck.world_y(y) - feet.floor;
        spots.get_mut(id).expect("placed performer").floor += delta;
        carry(spots, groups, &[], id, was);
    }
}

/// What rests on a piece that moved from `was` (a robin on a worn hat) goes
/// with it; a held or worn piece among them is put at its own bone.
fn carry(
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
    held: &[&str],
    id: &str,
    was: Spot,
) {
    let now = spots[id];
    let d = [
        now.center[0] - was.center[0],
        now.center[1] - was.center[1],
        now.floor - was.floor,
    ];
    let kids = |k: &str| groups.get(k).into_iter().flatten().map(String::as_str);
    let mut stack: Vec<&str> = kids(id).collect();
    while let Some(k) = stack.pop() {
        if held.contains(&k) {
            continue;
        }
        if let Some(s) = spots.get_mut(k) {
            (s.center[0], s.center[1], s.floor) =
                (s.center[0] + d[0], s.center[1] + d[1], s.floor + d[2]);
        }
        stack.extend(kids(k));
    }
}

/// The area a standing performer needs underfoot. Arms can extend over an
/// instrument or the edge of a platform while both feet stay supported.
fn standing_base(body: Spot, model: &Model) -> Spot {
    let y = model.bounds.min[1] + model.bounds.size(1) * 0.1;
    let points = model.tris.iter().flatten().filter(|p| p[1] <= y);
    let Some(mut feet) = points.copied().fold(None::<Bounds>, |b, p| {
        Some(b.map_or(Bounds { min: p, max: p }, |b| b.add(p)))
    }) else {
        return body;
    };
    feet.min[1] = body.b.min[1];
    feet.max[1] = body.b.max[1];
    let at = body.to_world([feet.mid(0), feet.min[1], feet.mid(2)]);
    Spot {
        center: [at[0], at[2]],
        b: feet,
        ..body
    }
}

/// A character may overhang the support with arms or wings. Its standing
/// footprint must fit, with a real surface directly below it.
fn stand_on_base(
    sf: &Surface,
    t: &Spot,
    body: Spot,
    model: &Model,
    others: &[Spot],
) -> Option<(Spot, bool)> {
    let feet = standing_base(body, model);
    nearest_on_top(sf, t, feet.center, |center| {
        let base = uneven(Some(sf), t, Spot { center, ..feet }, &[])?;
        let perched = Spot {
            center: [0, 1].map(|k| body.center[k] + center[k] - feet.center[k]),
            floor: base.floor,
            ..body
        };
        others
            .iter()
            .all(|o| clear(&[*o, perched]))
            .then_some(perched)
    })
}

/// A piece on a real top too uneven for its whole base (a stove's burners,
/// a plate's dip): it still stands on it, on its middle point, when it fits
/// inside the top clear of the pieces already there.
fn uneven(sf: Option<&Surface>, t: &Spot, s: Spot, others: &[Spot]) -> Option<Spot> {
    let fits =
        (0..2).all(|k| (s.center[k] - t.center[k]).abs() + s.half()[k] <= t.half()[k] + 1e-6);
    let m = t.to_model(s.center);
    let y = sf?.top_at(m[0], m[1])?;
    let s = Spot {
        floor: t.world_y(y),
        ..s
    };
    let mut all = others.to_vec();
    all.push(s);
    (fits && clear(&all)).then_some(s)
}

/// A piece the target has no room for: on the target's floor just outside
/// it, on the side it was laid toward, never in the air beside it.
fn beside(t: &Spot, s: Spot) -> Spot {
    let (dx, dz) = (s.center[0] - t.center[0], s.center[1] - t.center[1]);
    let n = dx.hypot(dz);
    let dir = if n < 1e-6 {
        [1.0, 0.0]
    } else {
        [dx / n, dz / n]
    };
    let [hx, hz] = t.half();
    let d = (dir[0] * hx).abs() + (dir[1] * hz).abs() + s.radius() + GAP / 2.0;
    Spot {
        center: [t.center[0] + dir[0] * d, t.center[1] + dir[1] * d],
        floor: t.floor.max(0.0),
        ..s
    }
}

fn stage_contact(
    sf: &Surface,
    deck: &Spot,
    s: Spot,
    center: [f64; 2],
    others: &[Spot],
) -> Option<Spot> {
    let s = Spot { center, ..s };
    resting_at(sf, deck, s, center, others).or_else(|| {
        // Planks have gaps: require floor under the feet on each side,
        // even where the rectangular patch between them is broken.
        let half = s.half();
        let mut heights = Vec::new();
        for offset in [[0.0, 0.0], [-1.0, 0.0], [1.0, 0.0], [0.0, -1.0], [0.0, 1.0]] {
            let q = deck.to_model([0, 1].map(|k| center[k] + half[k] * offset[k]));
            heights.push(deck.world_y(sf.top_at(q[0], q[1])?));
        }
        let high = heights.iter().copied().fold(f64::MIN, f64::max);
        let low = heights.iter().copied().fold(f64::MAX, f64::min);
        let s = Spot { floor: high, ..s };
        (high - low < 0.12 && others.iter().all(|o| clear(&[*o, s]))).then_some(s)
    })
}

fn resting_at(sf: &Surface, t: &Spot, s: Spot, center: [f64; 2], others: &[Spot]) -> Option<Spot> {
    let [hx, hz] = s.half();
    let r = hx.max(hz);
    let base = [r / t.scale[0], r / t.scale[2]];
    let tol = (0.1 + 0.35 * 2.0 * r) / t.scale[1];
    let [x, z] = t.to_model(center);
    let floor = t.world_y(sf.support(x, z, base, tol)?);
    let s = Spot { center, floor, ..s };
    others.iter().all(|other| clear(&[*other, s])).then_some(s)
}

/// A piece on a target's real surface: its base (5 × 5 points) all over
/// standable surface of the target's mesh, standing on the highest of them; else the nearest
/// such place (rings outward a grid step apart) clear of the pieces already
/// `standing` there, `true` when it moved; `None` when none holds it.
fn stand_on(sf: &Surface, t: &Spot, s: Spot, standing: &[Spot]) -> Option<(Spot, bool)> {
    nearest_on_top(sf, t, s.center, |center| {
        resting_at(sf, t, s, center, standing)
    })
}

fn nearest_on_top<T>(
    sf: &Surface,
    t: &Spot,
    start: [f64; 2],
    at: impl Fn([f64; 2]) -> Option<T>,
) -> Option<(T, bool)> {
    if let Some(value) = at(start) {
        return Some((value, false));
    }
    let step = (sf.cell() * t.scale[0].min(t.scale[2])).max(0.02);
    let reach = (t.half()[0].max(t.half()[1]) * 2.0 / step).ceil() as i32;
    for ring in 1..=reach {
        for i in -ring..=ring {
            for (dx, dz) in [(i, -ring), (i, ring), (-ring, i), (ring, i)] {
                let c = [start[0] + dx as f64 * step, start[1] + dz as f64 * step];
                if let Some(value) = at(c) {
                    return Some((value, true));
                }
            }
        }
    }
    None
}

/// Cells of pieces on a target's top (`t`): the columns and rows from the
/// edges of its footprint, a margin in; `false` when they would meet even
/// moved toward the middle, and are packed close together like the ground.
fn lay_on<'m>(
    (c, [hx, hz]): ([f64; 2], [f64; 2]),
    cells: &BTreeMap<(i32, i32), Vec<(&'m Model, Spot)>>,
) -> (Vec<(&'m str, Spot)>, bool) {
    let (col_w, row_d) = grid_sizes(cells);
    // The columns and rows from the edges of its top, a margin in.
    let margin = (0.1 * hx.min(hz)).min(0.25);
    let d = |h: f64, w: [f64; 3]| [-h + margin + w[0] / 2.0, 0.0, h - margin - w[2] / 2.0];
    let shift = |v: [f64; 3], by: f64| v.map(|x| x + by);
    // A corner piece comes in a little (a round top has no corners);
    // pieces that would meet move toward the middle, never off the edge.
    let corner = |k: f64| {
        let cols = shift(d(hx, col_w).map(|x| x * k), c[0]);
        let rows = shift(d(hz, row_d).map(|z| z * k), c[1]);
        pack(cells, cols, rows)
            .into_iter()
            .map(|(id, s)| {
                let (dx, dz) = (s.center[0] - c[0], s.center[1] - c[1]);
                let f = if dx.abs() > 1e-6 && dz.abs() > 1e-6 {
                    0.75
                } else {
                    1.0
                };
                let center = [c[0] + dx * f, c[1] + dz * f];
                (id, Spot { center, ..s })
            })
            .collect::<Vec<_>>()
    };
    let spots_of = |l: &[(&str, Spot)]| l.iter().map(|(_, s)| *s).collect::<Vec<_>>();
    let mut laid = corner(1.0);
    for k in [0.85, 0.7] {
        if clear(&spots_of(&laid)) {
            break;
        }
        laid = corner(k);
    }
    if clear(&spots_of(&laid)) {
        return (laid, true);
    }
    // Too little room for the places asked: packed like the ground.
    let (col_x, row_z) = spread(col_w, row_d);
    (pack(cells, shift(col_x, c[0]), shift(row_z, c[1])), false)
}

/// A held or worn piece (`own`) on its holder `t` after the holder turned:
/// at its `bone` (a hand's prop centred on it, upright; the neck's hanging
/// in front of the throat; a hat on the head, `part` its head's height; a
/// backpack behind the back; an apron in front of the hips, hanging from
/// the waist), else beside it on that side on its floor (its right is its
/// own -x; a neck piece at the front of its head end), with a plain note
/// (`{it}` / `{to}`: the two names) when that is not what was asked.
fn at_socket(
    l: &Link,
    t: Spot,
    own: Spot,
    bone: Option<([f64; 3], bool)>,
    held: bool,
    creature: bool,
    part: f64,
) -> (Spot, Option<String>) {
    let height = own.size(1);
    let (s, yaw, o) = (t.scale, t.rot_y.to_radians(), t.pos());
    let turn = |x: f64, z: f64| {
        [
            x * yaw.cos() + z * yaw.sin(),
            -x * yaw.sin() + z * yaw.cos(),
        ]
    };
    let world = |n: [f64; 3]| {
        let [x, z] = turn(s[0] * n[0], s[2] * n[2]);
        [o[0] + x, o[1] + s[1] * n[1], o[2] + z]
    };
    let ((center, floor), note) = match (l.how, l.socket, bone) {
        (How::Head, _, Some((n, _))) if part > 0.0 => {
            let p = world(n);
            (([p[0], p[2]], p[1] + 0.8 * part), None)
        }
        // An enclosed shell needs room above the crown as well as around
        // the face. A cap rests on the crown and leaves the face exposed.
        (How::EnclosedHead, _, Some((n, _))) if part > 0.0 => {
            let p = world(n);
            (([p[0], p[2]], p[1] + 1.23 * part - height), None)
        }
        (How::Head, _, _) => ((t.center, t.top() - 0.15 * height), None),
        (How::EnclosedHead, _, _) => ((t.center, t.top() - 0.85 * height), None),
        (_, "back", Some((n, _))) => {
            let p = world(n);
            let d = -(0.07 * t.size(1) + own.size(2) / 2.0);
            let at = [p[0] + d * yaw.sin(), p[2] + d * yaw.cos()];
            ((at, p[1] - height / 2.0), None)
        }
        (_, "waist", Some((n, _))) => {
            let p = world(n);
            let d = 0.06 * t.size(1) + own.size(2) / 2.0;
            let at = [p[0] + d * yaw.sin(), p[2] + d * yaw.cos()];
            ((at, p[1] + 0.06 * t.size(1) - height), None)
        }
        (_, "neck", Some((n, _))) => {
            let p = world(n);
            let ahead = 0.08 * (t.top() - t.floor) + own.radius() / 2.0;
            (
                (
                    [p[0] + ahead * yaw.sin(), p[2] + ahead * yaw.cos()],
                    p[1] - height,
                ),
                None,
            )
        }
        (_, "neck", None) => {
            // Its head end: the front of it, three quarters up.
            let ahead = 0.375 * t.size(2);
            let at = [
                t.center[0] + ahead * yaw.sin(),
                t.center[1] + ahead * yaw.cos(),
            ];
            let note =
                "I put the {it} at the front of the {to} because I can't find the {to}'s neck.";
            (
                (at, t.floor + 0.75 * t.size(1) - height / 2.0),
                Some(note.to_string()),
            )
        }
        // A tiny creature in a big one's hand stands at it as a prop does (a
        // lantern); a hanging hand holds no palm out, said plainly.
        // A palm held out (a posed hand): it stands on the palm.
        (_, _, Some((n, true))) => {
            let p = world(n);
            (([p[0], p[2]], p[1]), None)
        }
        (_, hand, Some((n, false)))
            if creature && hand.contains("hand") && height < 0.1 * t.size(1) =>
        {
            let p = world(n);
            let note = "The {to}'s arm hangs by the side; holding out a palm for the {it} needs poses, which are not here yet.";
            (
                ([p[0], p[2]], (p[1] - height / 2.0).max(t.floor)),
                Some(note.into()),
            )
        }
        (_, _, Some((n, _))) => {
            let p = world(n);
            (([p[0], p[2]], (p[1] - height / 2.0).max(t.floor)), None)
        }
        (_, socket, None) => {
            let (lx, lz) = match socket {
                s if s.contains("right") => (-1.0, 0.0),
                s if s.contains("left") => (1.0, 0.0),
                _ => (0.0, -1.0),
            };
            let d = t.radius() + GAP / 2.0 + own.radius();
            let [x, z] = turn(lx * d, lz * d);
            let note = (!held).then(|| {
                "I put the {it} next to the {to} because it cannot hold things yet.".to_string()
            });
            (([t.center[0] + x, t.center[1] + z], t.floor), note)
        }
    };
    let s = Spot {
        center,
        floor,
        rot_y: t.rot_y,
        ..own
    };
    (s, note)
}

/// The free piece that is a floor-like base: the widest, flat (no taller
/// than 0.3 of its shorter side), bigger than all the other free pieces together.
fn flat_base(free: &[(&str, Spot)]) -> Option<String> {
    let area = |s: &Spot| {
        let [hx, hz] = s.half();
        4.0 * hx * hz
    };
    let flat = |s: &Spot| {
        let [hx, hz] = s.half();
        s.size(1) <= 0.3 * 2.0 * hx.min(hz)
    };
    let (id, b) = free
        .iter()
        .filter(|(_, s)| flat(s))
        .max_by(|a, b| area(&a.1).total_cmp(&area(&b.1)))?;
    // The others it must carry (a piece far bigger than most, an "Earth in
    // the sky", is not one of them).
    let span = |s: &Spot| s.size(0).max(s.size(1)).max(s.size(2));
    let mut spans: Vec<f64> = free.iter().map(|(_, s)| span(s)).collect();
    spans.sort_by(f64::total_cmp);
    let median = spans[spans.len() / 2];
    let rest: f64 = free
        .iter()
        .filter(|(i, s)| i != id && span(s) <= 4.0 * median)
        .map(|(_, s)| area(s))
        .sum();
    (free.len() > 1 && area(b) >= rest).then(|| id.to_string())
}

/// The ground grid's columns and rows around (0, 0): the middle ones in the
/// middle, the others beside them, `GAP` apart.
fn spread(col_w: [f64; 3], row_d: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let side = if col_w[1] > 0.0 {
        col_w[1] / 2.0 + GAP
    } else {
        GAP / 2.0
    };
    (
        [-(side + col_w[0] / 2.0), 0.0, side + col_w[2] / 2.0],
        [
            -(row_d[1] / 2.0 + GAP + row_d[0] / 2.0),
            0.0,
            row_d[1] / 2.0 + GAP + row_d[2] / 2.0,
        ],
    )
}

/// How a piece stands to another: the plan's `rel` or, in an older plan,
/// read from its `attach_to` words (or a hold: in that hand).
#[derive(Debug, Clone)]
struct Link {
    to: String,
    how: How,
    /// On, in or under: where on the target (its place words); `None`: say none.
    spot: Option<(i32, i32)>,
    /// Beside: on which side as the camera sees it (x: left -1 / right 1,
    /// z: back -1 / front 1); `None`: toward the middle of the scene.
    side: Option<(i32, i32)>,
    /// Held or worn: the socket bone.
    socket: &'static str,
    /// Its words, for "beside the keeper" on the same surface.
    words: Vec<String>,
    /// Put there for it (a free piece on the floor-like base): packed together
    /// in the middle, by its place, not spread to the edges.
    packed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum How {
    On,
    In,
    Under,
    Beside,
    Above,
    Around,
    Socket,
    Head,
    EnclosedHead,
}

fn place_of(words: &str) -> Option<(i32, i32)> {
    Some(slots(words)).filter(|s| *s != (0, 0))
}

impl Link {
    /// A structured relation takes precedence over a legacy attachment.
    fn declared(place: Option<&super::Placement>, item: Option<&Item>) -> Option<Self> {
        let mut link = place
            .and_then(|p| p.rel.as_ref())
            .map(Self::of_rel)
            .or_else(|| {
                place
                    .and_then(|p| p.attach_to.as_ref())
                    .or_else(|| item?.details.attach_to.as_ref())
                    .map(Self::of_words)
            })?;
        if link.how == How::Head && item.is_some_and(encloses_head) {
            link.how = How::EnclosedHead;
        }
        Some(link)
    }

    fn of_rel(r: &super::Rel) -> Link {
        let side = r.side.as_deref().and_then(place_of);
        let (how, side, socket) = match r.how.as_str() {
            "in" => (How::In, None, ""),
            "under" => (How::Under, None, ""),
            "beside" => (How::Beside, side, ""),
            "behind" => (How::Beside, Some((0, -1)), ""),
            "in_front_of" => (How::Beside, Some((0, 1)), ""),
            "above" => (How::Above, None, ""),
            "around" => (How::Around, None, ""),
            "held" => {
                let left = r.hand.as_deref() == Some("left_hand");
                (
                    How::Socket,
                    None,
                    if left { "left_hand" } else { "right_hand" },
                )
            }
            "worn" => match r.part.as_deref() {
                Some("head") => (How::Head, None, "neck"),
                Some("neck") => (How::Socket, None, "neck"),
                Some("hip") => (How::Socket, None, "waist"),
                _ => (How::Socket, None, "back"),
            },
            _ => (How::On, None, ""),
        };
        Link {
            to: r.to.clone(),
            how,
            spot: r.spot.as_deref().map(slots),
            side,
            socket,
            words: Vec::new(),
            packed: false,
        }
    }

    /// An older plan's free words, best effort: hand and body-part words, then
    /// under (beneath, between its legs), in front of, behind, beside (next
    /// to, near: on the ground beside it, unless it says on the target, then
    /// on it beside another piece), above, around, in (not "in the middle
    /// of"), else on it; its place words say where.
    fn of_words(a: &Attach) -> Link {
        let text = a.at.trim().to_lowercase();
        let w = words(&text);
        let has = |x: &str| w.iter().any(|y| y == x);
        let any = |xs: &[&str]| xs.iter().any(|x| has(x));
        let side = place_of(&text);
        let hand = (has("hand") || has("palm")) && any(&["right", "left"]);
        let floor = any(&["floor", "ground"]);
        let near = text.contains("in front of") || any(&["behind", "beside", "next", "near"]);
        let on_it = (any(&["on", "top", "onto"]) || text.contains("in the ")) && !floor;
        let (how, socket) = match socket_of(&text) {
            Some(s) => (How::Socket, s),
            None if hand => (
                How::Socket,
                if has("left") {
                    "left_hand"
                } else {
                    "right_hand"
                },
            ),
            None if any(&["under", "beneath", "below", "underneath"])
                || (has("between") && has("leg")) =>
            {
                (How::Under, "")
            }
            // "On the sand in front of the octopus", "in the dust beside the
            // rover": on the target, next to another piece.
            None if near && on_it => (How::On, ""),
            None if near => (How::Beside, ""),
            None if has("above") => (How::Above, ""),
            None if has("around") => (How::Around, ""),
            None if ["on his head", "on her head", "on its head", "on their head"]
                .iter()
                .any(|p| text.contains(p))
                || any(&["wear", "wearing", "wears"]) =>
            {
                (How::Head, "")
            }
            None if text.contains("in the middle") || text.contains("in the cent") => (How::On, ""),
            None if any(&["in", "inside", "into", "within"]) => (How::In, ""),
            // "On the floor" of a room: inside it, on its floor.
            None if floor => (How::In, ""),
            None => (How::On, ""),
        };
        let side = match () {
            _ if text.contains("in front of") => Some((0, 1)),
            _ if has("behind") => Some((0, -1)),
            _ => side,
        };
        Link {
            to: a.item.clone(),
            how,
            spot: place_of(&text),
            side,
            socket,
            words: w,
            packed: false,
        }
    }
}

/// The socket an attachment names: one of [`SOCKETS`] as the whole `where`
/// ("right_hand", "right hand"), a two-word one inside it ("in his right
/// hand"), or the neck.
fn socket_of(at: &str) -> Option<&'static str> {
    let at = at.trim().to_lowercase().replace([' ', '-'], "_");
    SOCKETS.iter().copied().find(|s| {
        at == *s || (s.contains('_') && at.contains(s)) || (*s == "neck" && at.contains("neck"))
    })
}

fn slots(place: &str) -> (i32, i32) {
    let has = |w: &str| place.split(['-', ' ', '_', '+']).any(|p| p == w);
    let x = if has("left") {
        -1
    } else if has("right") {
        1
    } else {
        0
    };
    let z = if has("front") {
        1
    } else if has("back") {
        -1
    } else {
        0
    };
    (x, z)
}

/// The physical contact heights of each levelled piece in the viewer. An
/// axis-aligned box can extend below the runners of a sled or above its slats.
struct FittedModels {
    models: Vec<Model>,
    scales: BTreeMap<String, [f64; 3]>,
    turns: BTreeMap<String, Turn>,
    notes: Vec<String>,
}

pub(super) fn stage_name(name: &str) -> bool {
    words(name)
        .iter()
        .any(|w| matches!(w.as_str(), "stage" | "platform"))
}

fn hanging_textile(it: &Item) -> bool {
    it.kind == "object"
        && it
            .details
            .dims_m
            .is_some_and(|[w, h, d]| d < 0.2 * w.min(h))
        && words(&it.name)
            .last()
            .is_some_and(|w| matches!(w.as_str(), "curtain" | "drape" | "tapestry"))
}

fn textile_dims(it: &Item) -> Option<[f64; 3]> {
    let dims = it.details.dims_m?;
    let named = words(&it.name)
        .iter()
        .any(|w| matches!(w.as_str(), "rug" | "carpet" | "tablecloth"));
    (it.kind == "object"
        && (named || bedcover(it))
        && dims.iter().all(|d| d.is_finite() && *d > 0.)
        && dims[1] < 0.25 * dims[0].min(dims[2]))
    .then_some(dims)
}

fn bedcover(it: &Item) -> bool {
    it.kind == "object"
        && words(&it.name).iter().any(|word| {
            matches!(
                word.as_str(),
                "bedspread" | "quilt" | "blanket" | "duvet" | "comforter"
            )
        })
}

/// A plan can put the covering and its occupant on the same mattress.
/// Retain that support in partial scenes until the named covering is present.
fn tucked_covers<'a>(plan: &'a Plan, models: &[Model]) -> BTreeMap<&'a str, &'a str> {
    let support = |id: &str| {
        plan.layout
            .iter()
            .find(|p| p.item == id)
            .and_then(|p| p.rel.as_ref())
            .filter(|r| r.how == "on")
            .map(|r| r.to.as_str())
    };
    let mut targets = BTreeMap::new();
    for toy in plan.items.iter().filter(|it| {
        it.enabled && it.kind == "object" && words(&it.description).iter().any(|w| w == "tucked")
    }) {
        let Some(bed) = support(&toy.id) else {
            continue;
        };
        let lowercase_name = bare_name(&toy.name).to_lowercase();
        let name: Vec<_> = lowercase_name.split_whitespace().collect();
        if name.is_empty() {
            continue;
        }
        let mut covers = plan.items.iter().filter(|it| {
            if it.id == toy.id || !it.enabled || !bedcover(it) || support(&it.id) != Some(bed) {
                return false;
            }
            let description = it.description.to_lowercase();
            let detail: Vec<_> = description
                .split(|c: char| !c.is_alphanumeric())
                .filter(|s| !s.is_empty())
                .collect();
            detail.iter().enumerate().any(|(k, w)| {
                if !matches!(*w, "covers" | "covering") || k > 0 && detail[k - 1] == "not" {
                    return false;
                }
                let object = &detail[k + 1..];
                let object = match object {
                    ["the" | "a" | "an", rest @ ..] => rest,
                    _ => object,
                };
                object.starts_with(&name)
            })
        });
        if let Some(cloth) = covers.next() {
            if covers.next().is_none() && models.iter().any(|m| m.item == cloth.id) {
                targets.insert(toy.id.as_str(), cloth.id.as_str());
            }
        }
    }
    targets
}

/// A flexible cover can overhang the mattress or table edge. Its centre
/// must still rest on real material within a similarly sized support.
fn cover_height(sf: Option<&Surface>, target: Spot, cover: Spot, centre: [f64; 2]) -> Option<f64> {
    let [w, d] = cover.half();
    let [tw, td] = target.half();
    if w > 1.3 * tw || d > 1.3 * td {
        return None;
    }
    let [x, z] = target.to_model(centre);
    Some(target.world_y(sf?.top_at(x, z)?))
}
/// Place a flexible layer on its support, or tuck an explicitly peeking
/// object under a supported bedcover with its head at the requested edge.
fn lay_textile(
    l: &Link,
    items: (Option<&Item>, Option<&Item>),
    sf: Option<&Surface>,
    pair: (Spot, Spot),
    centre: [f64; 2],
    supported: bool,
) -> Option<Spot> {
    let (own, t) = pair;
    let it = items.0?;
    if l.how == How::On && (textile_dims(it).is_some() || hanging_textile(it)) {
        return Some(Spot {
            center: centre,
            floor: cover_height(sf, t, own, centre)?,
            rot_y: t.rot_y,
            ..own
        });
    }
    if l.how != How::Under || !supported || it.kind != "object" {
        return None;
    }
    let target = items.1?;
    if !bedcover(target) {
        return None;
    }
    let description = format!(
        "{} {}",
        it.description,
        it.details.prompt.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    if !words(&description)
        .iter()
        .any(|w| matches!(w.as_str(), "peek" | "peeks" | "peeking" | "tucked"))
    {
        return None;
    }
    let (x, z) = l.spot.filter(|&p| p != (0, 0)).unwrap_or((0, -1));
    let dir = [f64::from(x), f64::from(z)];
    let mut placed = Spot {
        center: [0, 1].map(|k| t.center[k] + dir[k] * t.half()[k]),
        floor: t.floor,
        ..own
    };
    let reclines = description.contains("on its back")
        || ((description.contains("tucked in") || description.contains("reclining"))
            && !["sitting up", "sits up", "upright", "standing"]
                .iter()
                .any(|s| description.contains(s)));
    // A seated plush can be a little deeper than it is tall because its feet
    // point forward. A model already lying flat keeps its original orientation.
    if reclines && own.b.size(1) > 0.8 * own.b.size(2) {
        // The requested dimensions describe the toy lying down. An upright
        // source mesh needs that length along its spine before the quarter turn.
        if it.details.dims_m.is_some() && own.size(1) < own.size(2) && own.b.size(2) > 1e-6 {
            placed.scale[1] = own.size(2) / own.b.size(1);
            placed.scale[2] = own.size(1) / own.b.size(2);
        }
        // The face points up, the head beyond the edge and most of the body under it.
        let yaw = (-dir[0]).atan2(-dir[1]).to_degrees();
        let pitch = turn_matrix([-85.0, 0.0, 0.0]);
        let e = rotation(yaw, Some(&pitch));
        let corners = Spot {
            tilt: Some(([0.0; 3], e)),
            ..placed
        }
        .corners();
        let mid = |k: usize| corners.iter().map(|p| p[k]).sum::<f64>() / 8.0;
        let bottom = corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
        let inset = 0.20 * placed.size(1);
        placed.center = [0, 1].map(|k| placed.center[k] - dir[k] * inset);
        placed.tilt = Some((
            [
                placed.center[0] - mid(0),
                t.floor - bottom,
                placed.center[1] - mid(2),
            ],
            e,
        ));
    }
    Some(placed)
}

/// An inflated or wrapped cloth may have a hole through the view from above.
/// Choose the face that spans the support. A quarter turn preserves the
/// order of the scale and rotation that the app applies to its source mesh.
fn spread_folded_textile(model: &mut Model, dims: [f64; 3]) -> Option<(Turn, [f64; 3])> {
    let front = [[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]];
    let side = [[0., 0., -1.], [1., 0., 0.], [0., -1., 0.]];
    let up = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let mut best = None;
    // Prefer the original top when it has just as much material across it.
    for (turn, axes) in [(up, [0, 1, 2]), (front, [0, 2, 1]), (side, [2, 0, 1])] {
        let tris: Vec<_> = model
            .tris
            .iter()
            .map(|t| t.map(|v| turn_point(&turn, v)))
            .collect();
        let b = Bounds::from_points(tris.iter().flatten().copied())?;
        let Some(surface) = Surface::of(&tris, b.min, b.max) else {
            continue;
        };
        let at = |axis: usize, k: i32| b.min[axis] + b.size(axis) * (f64::from(k) + 0.5) / 7.;
        let covered: usize = (0..7)
            .map(|x| {
                (0..7)
                    .filter(|&z| surface.top_at(at(0, x), at(2, z)).is_some())
                    .count()
            })
            .sum();
        if covered > 0 && best.as_ref().is_none_or(|(score, _, _)| covered > *score) {
            best = Some((covered, turn, axes));
        }
    }
    let (_, turn, axes) = best?;
    let mut scale = [1.; 3];
    for k in 0..3 {
        let extent = model.bounds.max[axes[k]] - model.bounds.min[axes[k]];
        if extent <= 1e-6 {
            return None;
        }
        scale[axes[k]] = dims[k] / extent;
    }
    // The app scales the source mesh before turning it. The contact solver
    // scales the already fitted model, so undo its scale in that frame.
    for p in model.tris.iter_mut().flatten() {
        let fitted = turn_point(&turn, [0, 1, 2].map(|k| p[k] * scale[k]));
        *p = [0, 1, 2].map(|k| fitted[k] / scale[k]);
    }
    model.bounds = Bounds::from_points(model.tris.iter().flatten().copied())?;
    Some((turn, scale))
}

/// A floor covering or bedspread can arrive standing on one edge. When
/// the plan calls for a thin horizontal textile, lay its actual broad face
/// flat and fit the whole sheet within its requested footprint.
fn horizontal_textile(it: &Item, model: &mut Model) -> Option<(Turn, [f64; 3])> {
    let dims = textile_dims(it)?;
    let turn = super::geometry::sheet_turn(&model.tris)?;
    let b = Bounds::from_points(model.tris.iter().flatten().map(|v| turn_point(&turn, *v)))?;
    if b.size(1) > 0.35 * b.size(0).min(b.size(2)) {
        return spread_folded_textile(model, dims);
    }
    let scale = (dims[0] / b.size(0)).min(dims[2] / b.size(2));
    model.bounds = b;
    for v in model.tris.iter_mut().flatten() {
        *v = turn_point(&turn, *v);
    }
    Some((turn, [scale; 3]))
}

/// The width of a rectangular stage is measured along the boards, not across
/// the diagonal of the imported model's box. Keep its level top and proportions.
fn fit_rectangular_stage(
    it: &Item,
    model: &mut Model,
    level: Turn,
    notes: &mut Vec<String>,
) -> Option<(Turn, [f64; 3])> {
    let [width, height, depth] = it.details.dims_m?;
    if !stage_name(&it.name)
        || height > 0.35 * width.min(depth)
        || !words(it.details.prompt.as_deref().unwrap_or_default())
            .iter()
            .any(|w| w == "rectangular" || w == "rectangle")
    {
        return None;
    }
    let angle = super::geometry::deck_axis(&model.tris, &level)?;
    if angle.to_degrees().abs() < 6.0 {
        return None;
    }
    let aligned = |a: f64| {
        let (s, c) = a.sin_cos();
        [
            std::array::from_fn(|k| c * level[0][k] + s * level[2][k]),
            level[1],
            std::array::from_fn(|k| -s * level[0][k] + c * level[2][k]),
        ]
    };
    let extent = |r: &Turn| {
        let mut vs = model.tris.iter().flatten().map(|v| turn_point(r, *v));
        let v = vs.next()?;
        Some(vs.fold(Bounds { min: v, max: v }, |b, v| b.add(v)))
    };
    let mut turn = aligned(angle);
    let mut bounds = extent(&turn)?;
    if (width > depth && bounds.size(0) < bounds.size(2))
        || (depth > width && bounds.size(2) < bounds.size(0))
    {
        turn = aligned(angle + std::f64::consts::FRAC_PI_2);
        bounds = extent(&turn)?;
    }
    let scale = width.max(depth) / bounds.size(0).max(bounds.size(2));
    let actual_width = bounds.size(0) * scale;
    let actual_depth = bounds.size(2) * scale;
    if (actual_width - width).abs() > 0.2 * width || (actual_depth - depth).abs() > 0.2 * depth {
        notes.push(format!(
            "{} measures {actual_width:.1} by {actual_depth:.1} m after fitting the boards to the planned length.",
            it.name
        ));
    }
    for v in model
        .tris
        .iter_mut()
        .flatten()
        .chain(model.joints.values_mut())
    {
        *v = turn_point(&turn, *v);
    }
    model.bounds = bounds;
    Some((turn, [scale; 3]))
}

/// Fit each model once. A support with a sloping top can be levelled up to
/// 25 degrees. Retain its planned footprint when grounding its real mesh.
fn fit_models(plan: &Plan, models: &[Model]) -> FittedModels {
    let anchors: Vec<String> = plan
        .items
        .iter()
        .filter_map(|item| {
            let p = plan.layout.iter().find(|p| p.item == item.id);
            let link = Link::declared(p, Some(item))?;
            matches!(
                link.how,
                How::On | How::In | How::Around | How::Above | How::Under
            )
            .then_some(link.to)
        })
        .collect();
    let mut result = FittedModels {
        models: Vec::with_capacity(models.len()),
        scales: BTreeMap::new(),
        turns: BTreeMap::new(),
        notes: Vec::new(),
    };
    let mut slope_notes = Vec::new();
    for made in models {
        let mut model = made.clone();
        let item = plan.items.iter().find(|i| i.id == made.item);
        if let Some((turn, scale)) = item.and_then(|i| horizontal_textile(i, &mut model)) {
            result.turns.insert(model.item.clone(), turn);
            result.scales.insert(model.item.clone(), scale);
            result.models.push(model);
            continue;
        }
        let name = item.map_or(made.item.as_str(), |i| i.name.as_str());
        let name = name
            .strip_prefix("The ")
            .or_else(|| name.strip_prefix("the "))
            .unwrap_or(name);
        let base = item.is_some_and(|i| {
            i.kind == "object"
                && (anchors.contains(&i.id)
                    || words(&i.name)
                        .iter()
                        .any(|w| BASE_WORDS.contains(&w.as_str())))
        });
        let level = base
            .then(|| levelling(&made.tris, made.bounds.min, made.bounds.max))
            .flatten();
        let turn = match level {
            Some((t, slope)) if (1.0..=25.0).contains(&slope) => Some(t),
            Some((_, slope)) if slope > 25.0 => {
                slope_notes.push(format!("The {name}'s top came out sloping {slope:.0}°, too steep to set level, so it stays as it was made."));
                None
            }
            _ => None,
        };
        if level.is_some_and(|(_, slope)| slope <= 25.0) {
            let level = turn.unwrap_or([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
            if let Some((deck_turn, scale)) =
                item.and_then(|it| fit_rectangular_stage(it, &mut model, level, &mut result.notes))
            {
                result.turns.insert(model.item.clone(), deck_turn);
                result.scales.insert(model.item.clone(), scale);
                result.models.push(model);
                continue;
            }
        }
        if let Some(t) = turn {
            let b = made.bounds;
            let corner = |k: usize| {
                turn_point(
                    &t,
                    [0, 1, 2].map(|d| if k >> d & 1 == 0 { b.min[d] } else { b.max[d] }),
                )
            };
            let first = corner(0);
            model.bounds = (1..8).fold(
                Bounds {
                    min: first,
                    max: first,
                },
                |b, k| b.add(corner(k)),
            );
        }
        let (scale, reshaped) = scale_of(plan, &model);
        if reshaped {
            let note = if scale[0] != scale[1] || scale[2] != scale[1] {
                format!(
                    "The {name} was fitted to the requested height. Its original shape limits how closely it fits the requested width and depth."
                )
            } else {
                format!(
                    "The {name} came out a different shape than planned, so it keeps its own shape."
                )
            };
            result.notes.push(note);
        }
        if let Some(t) = turn {
            // Three scales before turning. Keep that order when measuring the
            // surfaces that carry riders and the lowest point on the ground.
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for v in model.tris.iter_mut().flatten() {
                let w = turn_point(&t, [0, 1, 2].map(|k| v[k] * scale[k]));
                *v = [0, 1, 2].map(|k| w[k] / scale[k]);
                lo = lo.min(v[1]);
                hi = hi.max(v[1]);
            }
            model.bounds.min[1] = lo;
            model.bounds.max[1] = hi;
            result.turns.insert(model.item.clone(), t);
        }
        result.scales.insert(model.item.clone(), scale);
        result.models.push(model);
    }
    result.notes.extend(slope_notes);
    result
}

/// A piece behind a taller one stands clear of it as the camera sees it (a
/// chest just behind a man reads as a case in his hand): it moves outward
/// (away from the middle; from the middle, to the side it already shows),
/// with the rest of its row on that side. `rows`: each free piece's row.
fn clear_rows<'a>(rows: &[(&'a str, i32)], spots: &mut BTreeMap<&'a str, Spot>) {
    let across = |c: [f64; 2]| (c[0] * VIEW[2] - c[1] * VIEW[0]) / VIEW[0].hypot(VIEW[2]);
    for row in [0, -1] {
        for _ in 0..rows.len() {
            let blocked = rows.iter().filter(|(_, r)| *r == row).find_map(|(p, _)| {
                let p = spots[p];
                rows.iter().filter(|(_, r)| *r > row).find_map(|(q, _)| {
                    let q = spots[q];
                    let d = across(p.center) - across(q.center);
                    // Two gaps: the nearer piece is drawn wider than its ground circle.
                    let need = p.radius() + q.radius() + 2.0 * GAP;
                    let side = match p.center[0] {
                        x if x.abs() > 1e-6 => x.signum(),
                        _ => d.signum(),
                    };
                    (q.top() > p.top() && d.abs() < need)
                        .then(|| (p.center[0], side, need - side * d))
                })
            });
            let Some((x0, side, short)) = blocked else {
                break;
            };
            let dx = side * short * VIEW[0].hypot(VIEW[2]) / VIEW[2];
            for (id, _) in rows.iter().filter(|(_, r)| *r == row) {
                let s = spots.get_mut(id).expect("placed");
                if (s.center[0] - x0) * side >= 0.0 {
                    s.center[0] += dx;
                }
            }
        }
    }
}

/// The action the camera frames: every character, what one of them holds,
/// wears or has on, in or by it, and what one of them stands on unless it
/// is big (more than twice the tallest of them and more than 4 m: a pier,
/// a road, a room). `to`: a piece's link target.
/// A tiny one off the ground on a big one of the cast (a mouse on a giant's
/// shoulder): the box the first view frames instead of the big one's whole
/// self, the tiny one and his head (the viewer frames it too: `frame`).
/// [`close_up`]: the big one, the box framed instead of him, and the camera's
/// turn when the tiny one is on a palm he holds out.
type CloseUp<'s> = (&'s str, [f64; 3], [f64; 3], Option<f64>);

fn close_up<'s>(
    models: &[Model],
    spots: &BTreeMap<&str, Spot>,
    groups: &'s BTreeMap<String, Vec<String>>,
    cast: &[&str],
    characters: &[&str],
) -> Option<CloseUp<'s>> {
    let ext = |s: &Spot, k: usize| {
        let v = s.corners().map(|p| p[k]);
        (
            v.iter().copied().fold(f64::MAX, f64::min),
            v.iter().copied().fold(f64::MIN, f64::max),
        )
    };
    groups.iter().find_map(|(h, kids)| {
        let big = spots
            .get(h.as_str())
            .filter(|_| cast.contains(&h.as_str()))?;
        let tall = big.size(1);
        let (kid_id, kid) = (kids.iter().filter(|k| cast.contains(&k.as_str())))
            .filter_map(|k| Some((k.as_str(), spots.get(k.as_str())?)))
            .find(|(id, s)| {
                s.size(0).max(s.size(1)).max(s.size(2)) < 0.1 * tall
                    && s.floor > 0.2 * tall
                    // A small prop can fill a solo portrait. Keep the group in
                    // view when a second performer is doing something elsewhere.
                    && (characters.contains(id) || !characters.iter().any(|c| {
                        *c != h && spots.contains_key(*c) && !kids.iter().any(|k| k == c)
                    }))
            })?;
        // His head: the top quarter, a tenth of his height round his middle.
        let c = [big.center[0], big.top(), big.center[1]];
        let head = |k: usize, s: f64| match k {
            1 => c[1] - (1.0 - s) * 0.125 * tall,
            _ => c[k] + s * 0.1 * tall,
        };
        let lo = [0, 1, 2].map(|k| ext(kid, k).0.min(head(k, -1.0)));
        let hi = [0, 1, 2].map(|k| ext(kid, k).1.max(head(k, 1.0)));
        // On a palm he holds out (posed): the tiny one and the palm round it
        // framed (readable at its true size), seen from a little outside the
        // hand, looking back along his face's line to it: his face rises
        // behind it, eyes and all (the frame reaches 0.6 of the way up to it;
        // farther back, it looks smaller).
        let posed = (models.iter().find(|m| m.item == *h)).and_then(|m| m.pose);
        let palm = posed.filter(|p| p.palm.is_some() && characters.contains(&kid_id));
        let face_at = posed
            .and_then(|p| p.head)
            .map_or([c[0], head(1, 0.0), c[2]], |p| big.to_world(p));
        let face = face_at[1];
        let r = 1.25 * kid.size(0).max(kid.size(1)).max(kid.size(2));
        let up = (1.2 * r).max(0.6 * (face - ext(kid, 1).1));
        let (lo, hi) = match palm {
            Some(_) => (
                [0, 1, 2].map(|k| ext(kid, k).0 - if k == 1 { 0.4 * r } else { r }),
                [0, 1, 2].map(|k| ext(kid, k).1 + if k == 1 { up } else { r }),
            ),
            None => (lo, hi),
        };
        let az = (kid.center[0] - face_at[0]).atan2(kid.center[1] - face_at[2]);
        Some((h.as_str(), lo, hi, palm.map(|_| az.to_degrees())))
    })
}

/// What two or more characters gather round or face (the fire they sit
/// round, the table they stand at): the scene's focus.
fn focus_of<'p>(
    plan: &'p Plan,
    spots: &BTreeMap<&str, Spot>,
    is_char: impl Fn(&str) -> bool,
) -> Option<&'p str> {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for p in plan.layout.iter().filter(|p| is_char(&p.item)) {
        let around = (p.rel.as_ref()).filter(|r| matches!(r.how.as_str(), "around" | "beside"));
        let to = p.facing.as_deref().or(around.map(|r| r.to.as_str()));
        let visible = to.and_then(|id| {
            if spots.contains_key(id) {
                Some(id)
            } else {
                station_of(&plan.layout, id).filter(|t| spots.contains_key(t))
            }
        });
        if let Some(t) = visible.filter(|t| !is_char(t)) {
            *count.entry(t).or_default() += 1;
        }
    }
    (count.into_iter())
        .filter(|(_, n)| *n >= 2)
        .max_by_key(|(_, n)| *n)
        .map(|(t, _)| t)
}

fn cast_of<'m>(
    models: &'m [Model],
    spots: &BTreeMap<&str, Spot>,
    is_char: impl Fn(&str) -> bool,
    to: impl Fn(&str) -> Option<String>,
) -> Vec<&'m str> {
    let ids = models.iter().map(|m| m.item.as_str());
    let chars: Vec<&str> = ids.clone().filter(|id| is_char(id)).collect();
    let tall = chars
        .iter()
        .filter_map(|c| spots.get(c))
        .map(|s| s.size(1))
        .fold(0.0, f64::max);
    let small = |s: &Spot| s.size(0).max(s.size(1)).max(s.size(2)) <= (2.0 * tall).max(4.0);
    ids.filter(|id| {
        let stood_on = chars.iter().any(|c| to(c).as_deref() == Some(*id));
        chars.contains(id)
            || to(id).is_some_and(|t| chars.contains(&t.as_str()))
            || (stood_on && spots.get(id).is_some_and(small))
    })
    .collect()
}

/// A scene object's rotation (degrees, three.js's XYZ order): its turn about
/// the up axis after its levelling turn, if any.
fn rotation(yaw: f64, turn: Option<&Turn>) -> [f64; 3] {
    let Some(r) = turn else {
        return [0.0, yaw, 0.0];
    };
    let (c, s) = (yaw.to_radians().cos(), yaw.to_radians().sin());
    let ry = [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]];
    let m = compose_turns(ry, *r);
    let y = m[0][2].clamp(-1.0, 1.0).asin();
    let (x, z) = match m[0][2].abs() < 0.999_999_9 {
        true => ((-m[1][2]).atan2(m[2][2]), (-m[0][1]).atan2(m[0][0])),
        false => (m[2][1].atan2(m[1][1]), 0.0),
    };
    [x, y, z].map(|a| (a.to_degrees() * 100.0).round() / 100.0)
}

fn bare_name(name: &str) -> &str {
    name.strip_prefix("The ")
        .or_else(|| name.strip_prefix("the "))
        .unwrap_or(name)
}

/// Shared positions needed to put a neighbour beside its target, including
/// both positions when a player and instrument share a platform.
struct Neighbours<'a, 'm> {
    plan: &'a Plan,
    models: &'m [Model],
    geo: &'a BTreeMap<&'m str, Surface>,
    spots: &'a mut BTreeMap<&'m str, Spot>,
    supports: &'a mut BTreeMap<&'m str, String>,
    rows: &'a mut BTreeMap<(String, (i32, i32)), f64>,
    notes: &'a mut Vec<String>,
}
impl<'m> Neighbours<'_, 'm> {
    fn place(&mut self, m: &'m Model, l: &Link, own: Spot, t: Spot, first_rider: bool) {
        let item = |id: &str| self.plan.items.iter().find(|i| i.id == id);
        let layout = |id: &str| self.plan.layout.iter().find(|p| p.item == id);
        let name = |id: &str| item(id).map_or(id.to_string(), |i| i.name.clone());
        let [thx, thz] = t.half();
        let [ohx, ohz] = own.half();
        let side = l
            .side
            .unwrap_or(if t.center[0] > 1e-6 { (-1, 0) } else { (1, 0) });
        let (sx, sz) = (side.0 as f64, side.1 as f64);
        let used = self.rows.entry((l.to.clone(), side)).or_insert(0.0);
        let (tx, ox) = match side.0 != 0 {
            true => (thx, ohx),
            false => (thz, ohz),
        };
        let d = tx + *used + GAP / 2.0 + ox;
        *used += GAP / 2.0 + 2.0 * ox;
        let mut neighbor = Spot {
            center: [t.center[0] + sx * d, t.center[1] + sz * d],
            floor: t.floor.max(0.0),
            ..own
        };
        // A performer beside an instrument on a platform also needs
        // standing room on the platform, even when the instrument is at its edge.
        let base = self
            .supports
            .get(l.to.as_str())
            .filter(|base| {
                item(&m.item).is_some_and(|i| i.kind == "character")
                    && words(&name(base))
                        .iter()
                        .any(|w| BASE_WORDS.contains(&w.as_str()))
            })
            .cloned();
        let surface = base.and_then(|base| {
            Some((
                self.geo.get(base.as_str())?,
                *self.spots.get(base.as_str())?,
                base,
            ))
        });
        if let Some((surface, deck, base)) = surface {
            let others: Vec<_> = self
                .supports
                .iter()
                .filter(|(_, id)| **id == base)
                .filter_map(|(id, _)| self.spots.get(id).copied())
                .filter(|s| {
                    let [x, z] = deck.to_model(s.center);
                    surface.top_at(x, z).is_some()
                })
                .collect();
            // A player facing this instrument can reach over its base.
            // Leave space for their feet, instead of a gap beyond both arms.
            if layout(&m.item).and_then(|p| p.facing.as_deref()) == Some(l.to.as_str()) {
                let feet = standing_base(neighbor, m);
                let half = feet.half()[usize::from(side.0 == 0)];
                let distance = tx + half;
                neighbor.center = [t.center[0] + sx * distance, t.center[1] + sz * distance];
            }
            let players: Vec<_> = self
                .supports
                .iter()
                .filter(|(id, parent)| {
                    **parent == base && item(id).is_some_and(|i| i.kind == "character")
                })
                .filter_map(|(id, _)| {
                    Some(standing_base(
                        *self.spots.get(id)?,
                        self.models.iter().find(|x| x.item == **id)?,
                    ))
                })
                .collect();
            let feet = standing_base(neighbor, m);
            // An already supported playing position stays beside the
            // instrument. A new position must clear the other players.
            let mut resting = stage_contact(surface, &deck, feet, feet.center, &players);
            // Before separating a player and their instrument, try sliding
            // the pair to a clear patch of deck. Preserve the requested side.
            if resting.is_none() && first_rider {
                let occupied: Vec<_> = others
                    .iter()
                    .copied()
                    .filter(|o| o.center != t.center)
                    .collect();
                let pair = nearest_on_top(surface, &deck, feet.center, |center| {
                    let anchor = [0, 1].map(|k| t.center[k] + center[k] - feet.center[k]);
                    Some((
                        stage_contact(surface, &deck, feet, center, &occupied)?,
                        stage_contact(surface, &deck, t, anchor, &occupied)?,
                    ))
                });
                if let Some(((player, anchor), _)) = pair {
                    *self
                        .spots
                        .get_mut(l.to.as_str())
                        .expect("placed instrument") = anchor;
                    resting = Some(player);
                }
            }
            let resting = resting.or_else(|| stand_on(surface, &deck, feet, &others).map(|r| r.0));
            if let Some(on_deck) = resting {
                neighbor.center =
                    [0, 1].map(|k| neighbor.center[k] + on_deck.center[k] - feet.center[k]);
                neighbor.floor = on_deck.floor;
                self.supports.insert(m.item.as_str(), base);
            } else {
                neighbor = beside(&deck, neighbor);
                self.notes.push(format!(
                    "There is no standing room for the {} on the {}; it stands beside it.",
                    bare_name(&name(&m.item)),
                    bare_name(&name(&base))
                ));
            }
        }
        self.spots.insert(&m.item, neighbor);
    }
}

/// The upper end of a narrow stand or boom accepts a clipped microphone.
fn microphone_mount(models: &[Model], id: &str, stand: Spot) -> [f64; 3] {
    models
        .iter()
        .find(|m| m.item == id)
        .and_then(|m| m.tris.iter().flatten().max_by(|a, b| a[1].total_cmp(&b[1])))
        .map(|v| stand.to_world(*v))
        .unwrap_or([stand.center[0], stand.top(), stand.center[1]])
}

/// An overhead spotlight clears the cast, including actors placed after it.
fn overhead_clearance(
    name: &str,
    plan: &Plan,
    models: &[Model],
    scales: &BTreeMap<String, [f64; 3]>,
) -> f64 {
    if !is_spotlight(name) {
        return GAP / 2.0;
    }
    models
        .iter()
        .filter(|m| {
            plan.items
                .iter()
                .any(|i| i.id == m.item && i.kind == "character")
        })
        .map(|m| m.now_box().size(1) * scales[&m.item][1])
        .fold(0.0, f64::max)
        + GAP
}

fn model_surfaces(models: &[Model]) -> BTreeMap<&str, Surface> {
    models
        .iter()
        .filter_map(|m| {
            Some((
                m.item.as_str(),
                Surface::of(&m.tris, m.bounds.min, m.bounds.max)?,
            ))
        })
        .collect()
}

/// Sink a tall stage or pier so its usable surface stays at a standing height.
fn lower_bases(
    spots: &mut BTreeMap<&str, Spot>,
    geo: &BTreeMap<&str, Surface>,
    name: impl Fn(&str) -> String,
    carries: impl Fn(&str) -> bool,
    notes: &mut Vec<String>,
) {
    for (id, s) in spots.iter_mut() {
        let base = words(&name(id))
            .iter()
            .any(|w| BASE_WORDS.contains(&w.as_str()));
        let level = geo.get(id).map_or(s.top(), |g| s.world_y(g.max_top));
        if base && carries(id) && level > 1.2 {
            s.floor -= level - 0.8;
            notes.push(format!(
                "The {} came out much taller than planned, so it stands lower, its top 0.8 m above the ground.",
                bare_name(&name(id))
            ));
        }
    }
}

fn holder_of<'p>(plan: &'p Plan, models: &[Model], id: &str) -> Option<&'p Item> {
    plan.items.iter().find(|character| {
        character.kind == "character"
            && models.iter().any(|m| m.item == character.id)
            && character.details.holds.iter().any(|held| held.item == id)
    })
}

pub fn build_scene(plan: &Plan, models: &[Model]) -> (Value, Vec<String>) {
    let FittedModels {
        models: fitted,
        scales,
        turns,
        mut notes,
    } = fit_models(plan, models);
    let models = &fitted[..];
    let item = |id: &str| plan.items.iter().find(|i| i.id == id);
    let name = |id: &str| item(id).map_or(id.to_string(), |i| i.name.clone());
    // "the The Monster" never: a name's own leading "The" goes.
    let bare = |id: &str| bare_name(&name(id)).to_string();
    let layout = |id: &str| plan.layout.iter().find(|p| p.item == id);
    let made = |id: &str| models.iter().any(|m| m.item == id);
    let holder = |id: &str| holder_of(plan, models, id);
    let covers = tucked_covers(plan, models);
    // How each model stands to another (see [`link_of`]): the plan's `rel`,
    // else its `attach_to` words, else a prop a character holds, in its hand.
    let attach_of = |id: &str| -> Option<Link> {
        let held = || {
            let c = holder(id)?;
            let h = c.details.holds.iter().find(|h| h.item == id)?;
            Some(Attach {
                item: c.id.clone(),
                at: h.socket.clone(),
            })
        };
        // An otherwise free floor piece named or described beside a neighbour (a bedside rug,
        // a poolside mat) belongs beside that object even in a quadrant layout.
        let floor_neighbour = || {
            let i = item(id)?;
            let [w, h, d] = i.details.dims_m?;
            if i.kind != "object" || h > 0.15 * w.min(d) {
                return None;
            }
            let label = words(&i.name);
            let description = words(&i.description);
            let neighbour = label
                .iter()
                .find_map(|w| w.strip_suffix("side").filter(|n| n.len() >= 3))
                .or_else(|| {
                    let at = description
                        .iter()
                        .position(|w| w == "near" || w == "beside")?;
                    description[at + 1..]
                        .iter()
                        .find(|w| !matches!(w.as_str(), "a" | "an" | "the"))
                        .map(String::as_str)
                })?;
            let mut targets = plan.items.iter().filter(|other| {
                other.id != id
                    && made(&other.id)
                    && words(&other.name).iter().any(|w| w == neighbour)
                    && Link::declared(layout(&other.id), Some(other)).is_none()
            });
            let target = targets.next()?;
            targets.next().is_none().then(|| Link {
                to: target.id.clone(),
                how: How::Beside,
                side: layout(id).and_then(|p| place_of(&p.place)),
                spot: None,
                socket: "",
                words: Vec::new(),
                packed: false,
            })
        };
        let mut link = Link::declared(layout(id), item(id))
            .or_else(|| held().as_ref().map(Link::of_words))
            .or_else(floor_neighbour)?;
        if let Some(cover) = covers.get(id) {
            link.to = (*cover).to_owned();
            link.how = How::Under;
        }
        Some(link).filter(|l| l.to != id && made(&l.to))
    };
    let spot = |m: &Model| Spot::for_model(m, scales[m.item.as_str()]);
    // Each model's real surfaces (seen from above), when its mesh is known.
    let geo = model_surfaces(models);
    // Room placement hangs these pieces after the floor layout is built.
    // A wall fixture neither occupies a floor cell nor stands on a floor prop.
    let wall_fixture = |id: &str| {
        plan.setting.as_ref().is_some_and(|s| s.indoor)
            && (layout(id).is_some_and(|p| words(&p.place).iter().any(|w| w == "wall"))
                || item(id).is_some_and(|i| {
                    i.kind == "object" && words(&i.name).iter().any(|w| w == "window")
                }))
    };
    // A floor-like base (a deck, a stage, a rug: the widest free piece, flat,
    // bigger than all the other free pieces together) is a surface the other
    // free pieces stand on, each by its place.
    let free: Vec<(&str, Spot)> = models
        .iter()
        .filter(|m| !wall_fixture(&m.item) && attach_of(&m.item).is_none())
        .map(|m| (m.item.as_str(), spot(m)))
        .collect();
    let base = flat_base(&free);
    // A spectator can watch either a performer or the stage as a whole.
    // An explicit place on the stage still wins through attach_of.
    let stage = base.as_ref().filter(|b| stage_name(&name(b)));
    let audience: Vec<&str> = free
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| {
            let Some((person, target)) = item(id).zip(layout(id).and_then(|p| p.facing.as_ref()))
            else {
                return false;
            };
            stage.is_some()
                && person.kind == "character"
                && !plan.lineup.is_empty()
                && !plan.lineup.iter().any(|player| player == id)
                && (plan.lineup.contains(target)
                    || stage == Some(target)
                        && words(&person.name)
                            .into_iter()
                            .chain(words(&person.description))
                            .any(|w| {
                                matches!(
                                    w.as_str(),
                                    "spectator" | "audience" | "onlooker" | "viewer"
                                )
                            }))
        })
        .collect();
    let link = |id: &str| {
        attach_of(id).or_else(|| {
            let base = base
                .as_ref()
                .filter(|b| b.as_str() != id && !audience.contains(&id) && !wall_fixture(id))?;
            Some(Link {
                to: base.clone(),
                how: How::On,
                spot: None,
                side: None,
                socket: "",
                words: Vec::new(),
                packed: true,
            })
        })
    };
    let mut spots: BTreeMap<&str, Spot> = BTreeMap::new();

    // Free models on a grid: rows back, middle, front (the z slot) and
    // columns left, centre, right (the x slot). A cell packs its models left
    // to right, each in a circle wide enough for any turn; a column is as
    // wide as its widest cell, so a piece "front-left" stands in front of the
    // one "left".
    let slot_of = |id: &str| layout(id).map_or((0, 0), |p| slots(&p.place));
    let mut cells: BTreeMap<(i32, i32), Vec<(&Model, Spot)>> = BTreeMap::new();
    let mut wall_cells = BTreeMap::new();
    for m in models.iter().filter(|m| link(&m.item).is_none()) {
        let grid = if wall_fixture(&m.item) {
            &mut wall_cells
        } else {
            &mut cells
        };
        grid.entry(slot_of(&m.item)).or_default().push((m, spot(m)));
    }
    for grid in [&cells, &wall_cells] {
        let (col_w, row_d) = grid_sizes(grid);
        let (col_x, row_z) = spread(col_w, row_d);
        spots.extend(pack(grid, col_x, row_z));
    }
    lower_bases(
        &mut spots,
        &geo,
        name,
        |id| {
            models
                .iter()
                .any(|m| link(&m.item).is_some_and(|l| l.to == id && l.how == How::On))
        },
        &mut notes,
    );
    let rows: Vec<(&str, i32)> = cells
        .iter()
        .flat_map(|((_, z), cell)| cell.iter().map(move |(m, _)| (m.item.as_str(), *z)))
        .collect();
    clear_rows(&rows, &mut spots);
    let attachments::Attachments {
        groups,
        supports,
        socketed,
    } = attachments::place(plan, models, &mut spots, &scales, &geo, link, &mut notes);
    at_planted_shafts(plan, models, &mut spots, &groups);
    let playing_sticks = drumsticks::measure(plan, models, &spots);
    at_percussion(plan, models, &geo, &playing_sticks, &mut spots, &groups);
    at_microphones(plan, models, &mut spots, &groups);
    at_upright_strings(plan, models, &mut spots, &groups);
    lean_against(plan, models, &mut spots, &groups);
    let mut profile_subjects = place_followers(plan, &mut spots, &groups);
    settle_lineup(plan, models, &geo, &supports, &mut spots, &groups);
    space_station_watchers(plan, &mut spots, &supports, &groups);
    // Held or worn, through the holder's final scale, turn and place: at its
    // bone (a hand's prop centred on it, upright; the neck's hanging in front
    // of the throat; a hat on top of the head), else beside it on that side
    // on its floor (its right is its own -x; the neck's toward the middle of
    // the scene).
    let is_char = |id: &str| item(id).is_some_and(|i| i.kind == "character");
    let place_held = |spots: &mut BTreeMap<_, Spot>, i: usize| {
        let (m, l): (&Model, &Link) = (socketed[i].0, &socketed[i].1);
        let bone = socket_point(models, l);
        let (held, t) = (holder(&m.item).is_some_and(|c| c.id == l.to), spots[&*l.to]);
        let (own, part) = fit_worn(models, l, t, spot(m), held);
        let wearer = models.iter().find(|a| a.item == l.to);
        let pose = wearer.and_then(|a| a.pose);
        let crown = wearer
            .filter(|w| l.how == How::Head && !held && misplaced_head_patch(w))
            .and_then(|w| geo.get(w.item.as_str()))
            .and_then(|surface| {
                let [x, z] = surface.crown();
                Some(t.to_world([x, surface.top_at(x, z)?, z]))
            });
        let head = pose
            .and_then(|p| p.head_bounds)
            .filter(|_| l.how == How::EnclosedHead && !held);
        let wind = pose.and_then(|p| p.wind).filter(|_| {
            l.how == How::Socket
                && l.socket.ends_with("hand")
                && item(&m.item).is_some_and(super::pose::wind_instrument)
        });
        let (s, note) = if let Some(bounds) = head {
            (enclose_head(t, own, bounds), None)
        } else if let Some(p) = crown {
            let mut hat = own;
            hat.center = [p[0], p[2]];
            hat.floor = p[1] - 0.15 * hat.size(1);
            hat.rot_y = t.rot_y;
            (hat, None)
        } else if let Some(wind) = wind {
            (wind_spot(t, own, wind), None)
        } else if let Some(g) = fist(models, l).filter(|_| !is_char(&m.item)) {
            (
                playing_sticks
                    .get(m.item.as_str())
                    .map_or_else(|| grip_spot(t, own, g), |s| s.mount(t, own, g)),
                None,
            )
        } else {
            at_socket(l, t, own, bone, held, is_char(&m.item), part)
        };
        spots.insert(m.item.as_str(), s);
        note.map(|n| {
            n.replace("{it}", &bare(&m.item))
                .replace("{to}", &bare(&l.to))
        })
    };
    notes.extend(before_seats(plan, models, &geo, &mut spots, &groups));
    let held: Vec<&str> = socketed.iter().map(|(m, _)| m.item.as_str()).collect();
    for (i, id) in held.iter().enumerate() {
        let was = spots[id];
        notes.extend(place_held(&mut spots, i));
        carry(&mut spots, &groups, &held, id, was);
    }
    onto_tips(&mut spots, &groups, &held);
    step_closer(plan, models, &mut spots, &groups);
    notes.extend(short_of(plan, &spots, &bare));

    let (az0, pair) = face_turn(plan, &spots);
    let audience_to_omit = if plan.lineup.iter().any(|id| made(id)) {
        audience.as_slice()
    } else {
        &[]
    };
    let featured = |id: &str| is_char(id) && !audience_to_omit.contains(&id);
    let focus = focus_of(plan, &spots, featured);
    let mut cast = cast_of(models, &spots, featured, |id| link(id).map(|l| l.to));
    if let Some(station) = focus.filter(|id| !cast.contains(id)) {
        cast.push(station);
    }
    let face_actions = plan
        .items
        .iter()
        .filter_map(|i| {
            let pose = super::pose::effective(plan, i)?;
            (pose.arms.as_deref() == Some("crossed") || pose.plant.is_some() || pose.kind == "lean")
                .then_some((i.id.as_str(), pose))
        })
        .collect::<Vec<_>>();
    let characters = plan
        .items
        .iter()
        .filter(|i| featured(&i.id))
        .map(|i| i.id.as_str())
        .collect::<Vec<_>>();
    profile_subjects.extend(
        plan.items
            .iter()
            .filter(|i| i.details.body.as_deref() == Some("fish"))
            .map(|i| (i.id.as_str(), rest_heading(plan, &i.id))),
    );
    let look = Look {
        audience: audience_to_omit,
        profile_subjects: &profile_subjects,
        max_pitch: plan
            .setting
            .as_ref()
            .and_then(|s| s.max_camera_pitch(models)),
        characters: &characters,
        layout: &plan.layout,
        lineup: &plan.lineup,
        face_actions: &face_actions,
        supports: &supports,
        az0,
        pair,
        main: main_character(plan, &spots),
        turns: &turns,
        fixed: false,
        fixed_pitch: None,
        cast: &cast,
        focus,
    };
    // A held piece the face view hardly sees (behind its holder's body)
    // comes into view by the smallest change that keeps the plan true: its
    // holder turns toward the camera, up to 35°, so the hand is on the
    // camera's side (before the camera would rather go round to his back).
    let face = Look {
        fixed: true,
        ..look
    };
    let mut made = scene_doc(models, &spots, &groups, &name, &face);
    for (m, l) in &socketed {
        let id = m.item.as_str();
        if playing_sticks.values().any(|stick| stick.player == l.to)
            || made.1.get(id).is_none_or(|v| *v >= 0.5)
        {
            continue;
        }
        let (h, p, cam) = (spots[l.to.as_str()], spots[id], made.2);
        let angle = |x: f64, z: f64| (x - h.center[0]).atan2(z - h.center[1]).to_degrees();
        let want = angle(cam[0], cam[2]) - angle(p.center[0], p.center[1]);
        let want = ((want + 540.0).rem_euclid(360.0) - 180.0).clamp(-35.0, 35.0);
        let mut trial = spots.clone();
        if let Some(h) = trial.get_mut(l.to.as_str()) {
            h.rot_y += want.round();
        }
        // What it holds, and what that wears (a mouse's scarf), go with it.
        let mut moved = vec![l.to.as_str()];
        for (j, (m, l)) in socketed.iter().enumerate() {
            if moved.contains(&l.to.as_str()) {
                place_held(&mut trial, j);
                moved.push(m.item.as_str());
            }
        }
        onto_tips(&mut trial, &groups, &held);
        let again = scene_doc(models, &trial, &groups, &name, &face);
        if again.1.get(id).copied().unwrap_or(0.0) > made.1[id] + 0.1 {
            (spots, made) = (trial, again);
        }
    }
    let mut made = scene_doc(models, &spots, &groups, &name, &look);
    if behind_focus(plan, &mut spots, &groups, made.2, is_char) {
        made = scene_doc(models, &spots, &groups, &name, &look);
    }
    if let Some((az, pitch)) = nums::<3>(&made.0["camera"]["target"])
        .and_then(|target| wheels::arrange(plan, models, &mut spots, &groups, made.2, target))
    {
        let placed = Look {
            az0: az,
            fixed: true,
            fixed_pitch: Some(pitch),
            ..look
        };
        made = scene_doc(models, &spots, &groups, &name, &placed);
    }
    (made.0, notes.into_iter().chain(made.3).collect())
}

/// How the scene is looked at: the camera's first turn `az0` (degrees
/// about the up axis; 0 looks at a piece's face), the main character whose
/// chest it aims at, and each levelled model's turn.
#[derive(Clone, Copy)]
struct Look<'a> {
    audience: &'a [&'a str],
    profile_subjects: &'a [(&'a str, f64)],
    max_pitch: Option<f64>,
    characters: &'a [&'a str],
    az0: f64,
    /// Side-on to two who face each other: the other side is the other
    /// profile (a lone character's other side is his back: never tried).
    pair: bool,
    main: Option<&'a str>,
    turns: &'a BTreeMap<String, Turn>,
    /// Only the first view (the face view), not a search.
    fixed: bool,
    /// Reframe moved scenery around the composition selected for the cast.
    fixed_pitch: Option<f64>,
    /// The cast (every character): with one, the camera frames the action
    /// and a big base may run out of the frame; empty: every piece.
    cast: &'a [&'a str],
    /// Pieces the cast actually stands on or inside.
    supports: &'a BTreeMap<&'a str, String>,
    /// Pieces explicitly requested beside each other on the left or right.
    layout: &'a [super::Placement],
    lineup: &'a [String],
    face_actions: &'a [(&'a str, super::pose::Pose)],
    /// What the cast gathers round or faces (a fire, a table): the first
    /// view shows it unhidden, looking through a gap between them.
    focus: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::productions::geometry::hits;
    use crate::productions::{Details, Item, Placement};

    /// A GLB carrying only `doc` as its JSON chunk (bounds read no buffer).
    fn glb_of(doc: &Value) -> Vec<u8> {
        let mut text = serde_json::to_vec(doc).unwrap();
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        let mut b = b"glTF".to_vec();
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&(20 + text.len() as u32).to_le_bytes());
        b.extend_from_slice(&(text.len() as u32).to_le_bytes());
        b.extend_from_slice(b"JSON");
        b.extend_from_slice(&text);
        b
    }

    /// A GLB whose one mesh, on one untransformed node, spans `min`..`max`.
    pub fn fixture_glb(min: [f64; 3], max: [f64; 3]) -> Vec<u8> {
        glb_of(&json!({
            "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "accessors": [{"componentType": 5126, "count": 3, "type": "VEC3", "min": min, "max": max}]
        }))
    }

    /// The JSON chunks of two real Otto Forge outputs (out/text_to_asset/
    /// character/animlib.glb, a rigged, animated character, and
    /// prop_check/fixed.glb, a game-ready prop with LODs and collision
    /// hulls), trimmed to the nodes, skins, meshes and POSITION accessors.
    fn forge(name: &str) -> Bounds {
        let raw = match name {
            "character" => include_str!("fixtures/forge_character.gltf.json"),
            _ => include_str!("fixtures/forge_prop.gltf.json"),
        };
        glb_bounds(&glb_of(&serde_json::from_str(raw).unwrap())).unwrap()
    }

    fn close(a: [f64; 3], b: [f64; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-6)
    }

    #[test]
    fn bounds_are_in_world_space() {
        let b = glb_bounds(&fixture_glb([-1.0, 0.0, -2.0], [1.0, 3.0, 2.0])).unwrap();
        assert_eq!(
            b,
            Bounds {
                min: [-1.0, 0.0, -2.0],
                max: [1.0, 3.0, 2.0]
            }
        );
        assert!(glb_bounds(b"not a glb at all, really").is_none());

        // A parent's translation and scale, then a child's 90° turn about x
        // (y goes to z), as TRS and as a matrix.
        let half = std::f64::consts::FRAC_1_SQRT_2;
        for child in [
            json!({"mesh": 0, "rotation": [half, 0.0, 0.0, half]}),
            json!({"mesh": 0, "matrix": [1, 0, 0, 0, 0, 0, 1, 0, 0, -1, 0, 0, 0, 0, 0, 1]}),
        ] {
            let doc = json!({
                "scenes": [{"nodes": [0]}],
                "nodes": [{"children": [1], "translation": [10.0, 0.0, 0.0], "scale": [2.0, 2.0, 2.0]}, child],
                "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
                "accessors": [{"min": [0.0, 0.0, 0.0], "max": [1.0, 3.0, 0.5]}]
            });
            let b = glb_bounds(&glb_of(&doc)).unwrap();
            assert!(
                close(b.min, [10.0, -1.0, 0.0]) && close(b.max, [12.0, 0.0, 6.0]),
                "{b:?}"
            );
        }

        // A skinned mesh in centimetres under an armature scaled 0.01: placed
        // by the armature (its own node scale is ignored, as glTF says).
        let doc = json!({
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name": "Armature", "children": [1, 2], "scale": [0.01, 0.01, 0.01]},
                {"name": "Body", "mesh": 0, "skin": 0, "scale": [5.0, 5.0, 5.0]},
                {"name": "Hips", "translation": [0.0, 90.0, 0.0]}
            ],
            "skins": [{"joints": [2]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "accessors": [{"min": [-30.0, 0.0, -20.0], "max": [30.0, 180.0, 20.0]}]
        });
        let b = glb_bounds(&glb_of(&doc)).unwrap();
        assert!(
            close(b.min, [-0.3, 0.0, -0.2]) && close(b.max, [0.3, 1.8, 0.2]),
            "{b:?}"
        );

        // The real Forge character is about 0.9 units tall, the prop about 0.9 wide.
        let (c, p) = (forge("character"), forge("prop"));
        assert!((0.85..0.9).contains(&c.size(1)), "{c:?}");
        assert!((0.85..0.95).contains(&p.size(0)), "{p:?}");
    }

    fn item(id: &str, kind: &str) -> Item {
        Item {
            id: id.into(),
            kind: kind.into(),
            name: id.replace('_', " "),
            description: "x".into(),
            enabled: true,
            details: Details::default(),
            status: "done".into(),
            progress_pct: 100,
            message: None,
            result: None,
            previous_result: None,
            step: None,
            working_since: None,
            waiting: None,
            has_preview: false,
            preview_rev: 0,
            made: None,
            seed: None,
        }
    }

    fn place(item: &str, place: &str) -> Placement {
        Placement {
            item: item.into(),
            rel: None,
            place: place.into(),
            facing: None,
            attach_to: None,
            scale_hint: None,
        }
    }

    fn model(id: &str, bounds: Bounds) -> Model {
        Model {
            item: id.into(),
            artifact_id: format!("a-{id}"),
            version: None,
            bounds,
            joints: BTreeMap::new(),
            real_size: false,
            tris: Vec::new(),
            pose: None,
        }
    }

    /// Worn pieces fit the body: a hat on the head as wide as it, a backpack
    /// behind the back and an apron in front of the hips as wide as them,
    /// whatever size they were made; a scarf on a wearer with no skeleton
    /// goes at the front of its head end, no wider than it, said.
    #[test]
    fn worn_pieces_fit_the_body_part() {
        let ids = ["gran", "hat", "pack", "apron", "mouse", "scarf"];
        let mut plan = Plan {
            title: "Gran".into(),
            items: ids.map(|i| item(i, "object")).to_vec(),
            ..Default::default()
        };
        (plan.items[0].kind, plan.items[4].kind) = ("character".into(), "character".into());
        let worn = |id: &str, to: &str, part: &str| {
            let mut p = place(id, "");
            p.rel = Some(
                serde_json::from_value(json!({"to": to, "how": "worn", "part": part})).unwrap(),
            );
            p
        };
        plan.layout = vec![
            place("gran", "center"),
            worn("hat", "gran", "head"),
            worn("pack", "gran", "back"),
            worn("apron", "gran", "hip"),
            place("mouse", "left"),
            worn("scarf", "mouse", "neck"),
        ];
        let b = |lo: [f64; 3], hi: [f64; 3]| Bounds { min: lo, max: hi };
        let mut gran = model("gran", b([-0.3, 0.0, -0.15], [0.3, 1.8, 0.15]));
        for (j, p) in [
            ("neck", [0.0, 1.55, 0.0]),
            ("back", [0.0, 1.3, 0.0]),
            ("hip_left", [0.1, 0.95, 0.0]),
            ("hip_right", [-0.1, 0.95, 0.0]),
        ] {
            gran.joints.insert(j.into(), p);
        }
        let mut models = vec![
            gran,
            model("hat", b([-0.5, 0.0, -0.5], [0.5, 0.3, 0.5])),
            model("pack", b([-0.3, 0.0, -0.15], [0.3, 0.5, 0.15])),
            model("apron", b([-0.3, 0.0, -0.02], [0.3, 0.6, 0.02])),
            model("mouse", b([-0.05, 0.0, -0.1], [0.05, 0.08, 0.1])),
            model("scarf", b([-0.2, 0.0, -0.02], [0.2, 0.1, 0.02])),
        ];
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, notes) = build_scene(&plan, &models);
        let w = |id| world_box(obj(&doc, id), &models);
        let wide = |id| w(id).size(0).max(w(id).size(2));
        let o = obj(&doc, "gran");
        let (p, yaw) = (
            nums::<3>(&o["position"]).unwrap(),
            o["rotation"][1].as_f64().unwrap().to_radians(),
        );
        let joint = |v: [f64; 3]| {
            [
                p[0] + v[0] * yaw.cos() + v[2] * yaw.sin(),
                p[1] + v[1],
                p[2] - v[0] * yaw.sin() + v[2] * yaw.cos(),
            ]
        };
        // How far in front of a joint a piece's middle is (behind: below 0).
        let ahead = |id, j: [f64; 3]| {
            let (c, j) = (w(id), joint(j));
            (c.mid(0) - j[0]) * yaw.sin() + (c.mid(2) - j[2]) * yaw.cos()
        };
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        // The head is 0.25 high: the hat 1.15 of it, down over the crown.
        assert!(near(wide("hat"), 1.15 * 0.25), "{}", wide("hat"));
        assert!(
            near(w("hat").min[1], joint([0.0, 1.55, 0.0])[1] + 0.2),
            "{:?}",
            w("hat")
        );
        assert!(ahead("hat", [0.0, 1.55, 0.0]).abs() < 1e-3);
        // The hips are 0.2 apart: the backpack 1.8 of it behind the back,
        // the apron 2.0 of it in front, hanging from just over the hips.
        assert!(
            near(wide("pack"), 0.36) && ahead("pack", [0.0, 1.3, 0.0]) < -0.12,
            "{:?}",
            w("pack")
        );
        assert!(
            near(w("apron").size(0), 0.4) && ahead("apron", [0.0, 0.95, 0.0]) > 0.1,
            "{:?}",
            w("apron")
        );
        assert!(
            near(w("apron").max[1], 0.95 + 0.06 * 1.8),
            "{:?}",
            w("apron")
        );
        // No skeleton: the scarf at the mouse's front, three quarters up.
        let (scarf, mouse) = (w("scarf"), w("mouse"));
        assert!(near(wide("scarf"), 0.6 * 0.2), "{}", wide("scarf"));
        assert!(
            scarf.mid(1) > mouse.min[1] + 0.5 * mouse.size(1) && scarf.mid(1) < mouse.max[1] + 0.05
        );
        assert!(
            notes.contains(
                &"I put the scarf at the front of the mouse because I can't find the mouse's neck."
                    .to_string()
            ),
            "{notes:?}"
        );
    }

    /// A piece held in a posed fist: a stick through the fist a sixth of the
    /// way up, its length along the handle's way out, the marshmallow on it
    /// at its far end, said when it does not reach over the fire; a lantern
    /// hangs by its top from the other child's fist.
    #[test]
    fn a_held_piece_goes_through_the_fist_along_the_handle() {
        let ids = ["kid", "stick", "puff", "fire", "keeper", "lantern"];
        let mut plan = Plan {
            title: "Camp".into(),
            items: ids.map(|i| item(i, "object")).to_vec(),
            ..Default::default()
        };
        (plan.items[0].kind, plan.items[4].kind) = ("character".into(), "character".into());
        let rel = |id: &str, r: Value| {
            let mut p = place(id, "");
            p.rel = Some(serde_json::from_value(r).unwrap());
            p
        };
        plan.layout = vec![
            place("kid", "left"),
            rel(
                "stick",
                json!({"to": "kid", "how": "held", "hand": "right_hand", "toward": "fire"}),
            ),
            rel("puff", json!({"to": "stick", "how": "on"})),
            place("fire", "center"),
            place("keeper", "right"),
            rel(
                "lantern",
                json!({"to": "keeper", "how": "held", "hand": "left_hand"}),
            ),
        ];
        let b = |lo: [f64; 3], hi: [f64; 3]| Bounds { min: lo, max: hi };
        let posed = |left: bool, carry: bool, axis: [f64; 3]| Posed {
            min: [-0.3, 0.0, -0.2],
            max: [0.3, 1.2, 0.4],
            seat: None,
            palm: None,
            hand_left: false,
            back: None,
            wind: None,
            shaft: None,
            other_grip: None,
            head_bounds: None,
            mouth: None,
            head: None,
            grip: Some(GripAt {
                left,
                carry,
                point: [-0.2, 0.7, 0.3],
                axis,
            }),
        };
        let mut kid = model("kid", b([-0.3, 0.0, -0.2], [0.3, 1.2, 0.2]));
        let s = std::f64::consts::FRAC_1_SQRT_2;
        kid.pose = Some(posed(false, false, [0.0, s, s]));
        let mut keeper = model("keeper", b([-0.3, 0.0, -0.2], [0.3, 1.8, 0.2]));
        keeper.pose = Some(posed(true, true, [0.0, -1.0, 0.0]));
        let mut models = vec![
            kid,
            model("stick", b([-0.01, 0.0, -0.01], [0.01, 0.6, 0.01])),
            model("puff", b([-0.02, 0.0, -0.02], [0.02, 0.04, 0.02])),
            model("fire", b([-0.4, 0.0, -0.4], [0.4, 0.5, 0.4])),
            keeper,
            model("lantern", b([-0.1, 0.0, -0.1], [0.1, 0.3, 0.1])),
        ];
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, notes) = build_scene(&plan, &models);
        // A model point of a placed piece, in the world (its full turn).
        let at = |id: &str, v: [f64; 3]| {
            let o = obj(&doc, id);
            let (p, e, sc) = (
                nums::<3>(&o["position"]).unwrap(),
                nums::<3>(&o["rotation"]).unwrap(),
                o["scale"][0].as_f64().unwrap(),
            );
            let w = turned(turn_matrix(e), v.map(|x| x * sc));
            [0, 1, 2].map(|k| p[k] + w[k])
        };
        let holder = |id: &str, v: [f64; 3]| {
            let o = obj(&doc, id);
            let (p, yaw) = (
                nums::<3>(&o["position"]).unwrap(),
                o["rotation"][1].as_f64().unwrap().to_radians(),
            );
            [
                p[0] + v[0] * yaw.cos() + v[2] * yaw.sin(),
                p[1] + v[1],
                p[2] - v[0] * yaw.sin() + v[2] * yaw.cos(),
            ]
        };
        let near = |a: [f64; 3], b: [f64; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-3);
        // The stick: through the fist a sixth of the way up, along the handle.
        let fist = holder("kid", [-0.2, 0.7, 0.3]);
        assert!(
            near(at("stick", [0.0, 0.1, 0.0]), fist),
            "{:?} vs {fist:?}",
            at("stick", [0.0, 0.1, 0.0])
        );
        let yaw = obj(&doc, "kid")["rotation"][1]
            .as_f64()
            .unwrap()
            .to_radians();
        let way = [s * yaw.sin(), s, s * yaw.cos()];
        let up = at("stick", [0.0, 0.6, 0.0]);
        let along = [0, 1, 2].map(|k| (up[k] - fist[k]) / 0.5);
        assert!(near(along, way), "{along:?} vs {way:?}");
        // The marshmallow on its far end.
        let puff = world_box(obj(&doc, "puff"), &models);
        assert!(
            (puff.mid(0) - up[0]).abs() < 1e-3 && (puff.mid(2) - up[2]).abs() < 1e-3,
            "{puff:?} vs {up:?}"
        );
        // The lantern hangs by its top from the keeper's fist, upright.
        let fist = holder("keeper", [-0.2, 0.7, 0.3]);
        assert!(
            near(at("lantern", [0.0, 0.3, 0.0]), fist),
            "{:?}",
            at("lantern", [0.0, 0.3, 0.0])
        );
        assert!(near(
            at("lantern", [0.0, 0.0, 0.0]),
            [fist[0], fist[1] - 0.3, fist[2]]
        ));
        // A child across the scene from the fire: the stick does not reach.
        assert!(
            notes
                .iter()
                .any(|n| n
                    == "The stick does not reach over the fire: its holder is too far from it."),
            "{notes:?}"
        );
    }

    /// A seated holder whose stick ends short of the fire slides along its
    /// log toward it, its sitting point still on the log, with what it holds.
    #[test]
    fn a_seated_holder_slides_along_its_seat_toward_what_it_holds_its_stick_over() {
        let ids = ["log", "kid", "stick", "fire"];
        let mut plan = Plan {
            title: "Camp".into(),
            items: ids.map(|i| item(i, "object")).to_vec(),
            ..Default::default()
        };
        plan.items[1].kind = "character".into();
        let rel = |id: &str, r: Value| {
            let mut p = place(id, "");
            p.rel = Some(serde_json::from_value(r).unwrap());
            p
        };
        plan.layout = vec![
            place("log", "left"),
            rel("kid", json!({"to": "log", "how": "on"})),
            rel(
                "stick",
                json!({"to": "kid", "how": "held", "hand": "right_hand", "toward": "fire"}),
            ),
            place("fire", "center"),
        ];
        let b = |lo: [f64; 3], hi: [f64; 3]| Bounds { min: lo, max: hi };
        let mut kid = model("kid", b([-0.2, 0.0, -0.15], [0.2, 1.2, 0.15]));
        kid.pose = Some(Posed {
            min: [-0.2, 0.0, -0.3],
            max: [0.2, 0.8, 0.3],
            seat: Some([0.0, 0.45, -0.1]),
            palm: None,
            hand_left: false,
            back: None,
            wind: None,
            shaft: None,
            other_grip: None,
            head_bounds: None,
            mouth: None,
            head: None,
            grip: Some(GripAt {
                left: false,
                carry: false,
                point: [-0.15, 0.6, 0.25],
                axis: [0.0, 0.0, 1.0],
            }),
        });
        let mut models = vec![
            model("log", b([-1.5, 0.0, -0.25], [1.5, 0.4, 0.25])),
            kid,
            model("stick", b([-0.01, 0.0, -0.01], [0.01, 0.6, 0.01])),
            model("fire", b([-0.4, 0.0, -0.4], [0.4, 0.5, 0.4])),
        ];
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, notes) = build_scene(&plan, &models);
        let o = obj(&doc, "kid");
        let (p, yaw) = (
            nums::<3>(&o["position"]).unwrap(),
            o["rotation"][1].as_f64().unwrap().to_radians(),
        );
        let v = [0.0, 0.45, -0.1];
        let seat = [
            p[0] + v[0] * yaw.cos() + v[2] * yaw.sin(),
            p[2] - v[0] * yaw.sin() + v[2] * yaw.cos(),
        ];
        let log = world_box(obj(&doc, "log"), &models);
        assert!(
            seat[0] > log.min[0]
                && seat[0] < log.max[0]
                && seat[1] > log.min[2]
                && seat[1] < log.max[2],
            "{seat:?} on {log:?}"
        );
        assert!(
            (seat[0] - log.mid(0)).hypot(seat[1] - log.mid(2)) > 0.1,
            "it slid: {seat:?} on {log:?}"
        );
        // Its stick went with it: still in the fist.
        let s = obj(&doc, "stick");
        let fist = [
            p[0] - 0.15 * yaw.cos() + 0.25 * yaw.sin(),
            p[1] + 0.6,
            p[2] + 0.15 * yaw.sin() + 0.25 * yaw.cos(),
        ];
        let (sp, e) = (
            nums::<3>(&s["position"]).unwrap(),
            nums::<3>(&s["rotation"]).unwrap(),
        );
        let g = turned(turn_matrix(e), [0.0, 0.1, 0.0]);
        let grip = [0, 1, 2].map(|k| sp[k] + g[k]);
        assert!(
            (0..3).all(|k| (grip[k] - fist[k]).abs() < 1e-3),
            "{grip:?} vs {fist:?}"
        );
        let _ = notes;
    }

    /// Free scenery between the opening camera and what the cast gathers
    /// round goes to its far side: a tent is never in front of the fire.
    #[test]
    fn free_scenery_stands_beyond_the_focus_from_the_camera() {
        let ids = ["fire", "a", "b", "tent"];
        let mut plan = Plan {
            title: "Camp".into(),
            items: ids.map(|i| item(i, "object")).to_vec(),
            ..Default::default()
        };
        (plan.items[1].kind, plan.items[2].kind) = ("character".into(), "character".into());
        let around = |id: &str| {
            let mut p = place(id, "");
            p.rel = Some(serde_json::from_value(json!({"to": "fire", "how": "around"})).unwrap());
            p
        };
        plan.layout = vec![
            place("fire", "center"),
            around("a"),
            around("b"),
            place("tent", "front"),
        ];
        let b = |lo: [f64; 3], hi: [f64; 3]| Bounds { min: lo, max: hi };
        let mut models = vec![
            model("fire", b([-0.4, 0.0, -0.4], [0.4, 0.5, 0.4])),
            model("a", b([-0.25, 0.0, -0.2], [0.25, 1.2, 0.2])),
            model("b", b([-0.25, 0.0, -0.2], [0.25, 1.2, 0.2])),
            model("tent", b([-0.8, 0.0, -0.8], [0.8, 1.2, 0.8])),
        ];
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, _) = build_scene(&plan, &models);
        let (cam, fire, tent) = (
            nums::<3>(&doc["camera"]["position"]).unwrap(),
            world_box(obj(&doc, "fire"), &models),
            world_box(obj(&doc, "tent"), &models),
        );
        let toward = |p: [f64; 2]| {
            (p[0] - fire.mid(0)) * (cam[0] - fire.mid(0))
                + (p[1] - fire.mid(2)) * (cam[2] - fire.mid(2))
        };
        assert!(
            toward([tent.mid(0), tent.mid(2)]) < 0.0,
            "tent {tent:?} fire {fire:?} camera {cam:?}"
        );
    }

    fn sword_scene() -> (Plan, Vec<Model>) {
        let mut plan = Plan {
            title: "Sword".into(),
            items: ["man", "sword", "monster", "chest", "key", "coins"]
                .map(|i| item(i, "object"))
                .to_vec(),
            ..Default::default()
        };
        plan.items[0].kind = "character".into();
        plan.items[2].kind = "character".into();
        let mut monster = place("monster", "front");
        monster.facing = Some("man".into());
        monster.scale_hint = Some("big".into());
        let mut key = place("key", "front");
        key.attach_to = Some(Attach {
            item: "monster".into(),
            at: "neck".into(),
        });
        key.scale_hint = Some("small".into());
        let mut coins = place("coins", "back");
        coins.attach_to = Some(Attach {
            item: "chest".into(),
            at: String::new(),
        });
        plan.layout = vec![
            place("man", "center"),
            place("sword", "center-right"),
            monster,
            place("chest", "front-right"),
            key,
            coins,
        ];
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let models = vec![
            model("man", b([-0.3, 0.0, -0.2], [0.3, 1.8, 0.2])),
            model("sword", b([-0.05, -0.1, -0.05], [0.05, 1.0, 0.05])),
            model("monster", b([-1.0, 0.0, -0.8], [1.0, 2.5, 0.8])),
            model("chest", b([-0.5, 0.0, -0.3], [0.5, 0.6, 0.3])),
            model("key", b([-0.05, -0.05, -0.01], [0.05, 0.1, 0.01])),
            model("coins", b([-0.3, 0.0, -0.2], [0.3, 0.2, 0.2])),
        ];
        (plan, models)
    }

    /// A placed object's 8 world corners (its box through scale, turn, position).
    fn corners(o: &Value, models: &[Model]) -> Vec<[f64; 3]> {
        let b = models.iter().find(|m| m.item == o["id"]).unwrap().bounds;
        let p = nums::<3>(&o["position"]).unwrap();
        let s = nums::<3>(&o["scale"]).unwrap();
        let t = o["rotation"][1].as_f64().unwrap().to_radians();
        (0..8)
            .map(|k| {
                let c = [0, 1, 2].map(|d| s[d] * if k >> d & 1 == 0 { b.min[d] } else { b.max[d] });
                [
                    p[0] + c[0] * t.cos() + c[2] * t.sin(),
                    p[1] + c[1],
                    p[2] - c[0] * t.sin() + c[2] * t.cos(),
                ]
            })
            .collect()
    }

    fn world_box(o: &Value, models: &[Model]) -> Bounds {
        let cs = corners(o, models);
        cs[1..].iter().fold(
            Bounds {
                min: cs[0],
                max: cs[0],
            },
            |b, c| b.add(*c),
        )
    }

    fn obj<'a>(doc: &'a Value, id: &str) -> &'a Value {
        doc["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == id)
            .unwrap()
    }

    /// No two free pieces (all but `attached`) overlap or float, and every
    /// corner of every piece is inside the camera's view cone.
    fn check_layout(doc: &Value, models: &[Model], attached: &[&str]) {
        otto_design::format::scene3d::validate(doc).unwrap();
        let objects: Vec<&Value> = doc["objects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["type"] == "gltf")
            .collect();
        let free: Vec<&Value> = objects
            .iter()
            .copied()
            .filter(|o| !attached.iter().any(|a| o["id"] == *a))
            .collect();
        for (i, a) in free.iter().enumerate() {
            let ab = world_box(a, models);
            assert!(
                ab.min[1].abs() < 1e-3,
                "{} stands on the ground: {ab:?}",
                a["id"]
            );
            for b in &free[i + 1..] {
                let bb = world_box(b, models);
                let overlap = ab.min[0] < bb.max[0]
                    && bb.min[0] < ab.max[0]
                    && ab.min[2] < bb.max[2]
                    && bb.min[2] < ab.max[2];
                assert!(!overlap, "{} overlaps {}", a["id"], b["id"]);
            }
        }
        let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
        let target = nums::<3>(&doc["camera"]["target"]).unwrap();
        let half = (doc["camera"]["fov"].as_f64().unwrap() / 2.0).to_radians();
        let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let len = |v: [f64; 3]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
        let axis = sub(target, cam);
        // Every piece shows: its middle is in view (a big base may run out
        // of the frame around the action).
        for o in objects {
            let c = corners(o, models);
            let mid = [0, 1, 2].map(|k| c.iter().map(|p| p[k]).sum::<f64>() / 8.0);
            let v = sub(mid, cam);
            let cos = (0..3).map(|k| v[k] * axis[k]).sum::<f64>() / (len(v) * len(axis));
            assert!(
                cos >= half.cos(),
                "{} middle {mid:?} is out of view",
                o["id"]
            );
        }
    }

    /// `a` sits on top of `t`, over its centre.
    fn on_top(doc: &Value, models: &[Model], a: &str, t: &str) {
        let (ab, tb) = (
            world_box(obj(doc, a), models),
            world_box(obj(doc, t), models),
        );
        assert!(
            (ab.min[1] - tb.max[1]).abs() < 0.01,
            "{a} on top of {t}: {ab:?} {tb:?}"
        );
        for k in [0, 2] {
            assert!(
                (ab.mid(k) - tb.mid(k)).abs() < 0.01,
                "{a} over the middle of {t}"
            );
        }
    }

    #[test]
    fn placed_models_stand_apart_attachments_ride_their_target_and_it_is_deterministic() {
        let (plan, models) = sword_scene();
        let (doc, notes) = build_scene(&plan, &models);
        assert_eq!(build_scene(&plan, &models).0, doc, "deterministic");
        assert_eq!(doc["objects"].as_array().unwrap().len(), 7);
        // Everything stands on a ground under the whole group.
        let ground = obj(&doc, "ground");
        assert_eq!(ground["type"], "plane");
        assert!(ground["scale"][0].as_f64().unwrap() > 4.0, "{ground}");
        check_layout(&doc, &models, &["key", "coins"]);
        on_top(&doc, &models, "coins", "chest");

        // The size rule: a person 1.8 m, a big monster 2.7 m, a chest's
        // longest side 0.6 m, a key 0.2 m, coins 0.3 m.
        let b = |id| world_box(obj(&doc, id), &models);
        let longest = |id| {
            [0, 1, 2]
                .map(|k| b(id).size(k))
                .into_iter()
                .fold(0.0, f64::max)
        };
        assert!((b("man").size(1) - 1.8).abs() < 1e-3);
        assert!((b("monster").size(1) - 2.7).abs() < 1e-3);
        for (id, m) in [("chest", 0.6), ("key", 0.2), ("coins", 0.3), ("sword", 1.0)] {
            assert!((longest(id) - m).abs() < 1e-3, "{id}: {}", longest(id));
        }

        let groups = doc["groups"].as_array().unwrap();
        assert!(groups.contains(
            &json!({"id": "with-monster", "name": "monster", "children": ["monster", "key"]})
        ));
        assert!(groups.contains(
            &json!({"id": "with-chest", "name": "chest", "children": ["chest", "coins"]})
        ));
        // No skeleton: the key hangs at the monster's front, three quarters up.
        assert_eq!(
            notes,
            vec![
                "I put the key at the front of the monster because I can't find the monster's neck."
            ]
        );
        assert!(
            b("key").mid(1) > b("monster").min[1] + 0.5 * b("monster").size(1),
            "{:?}",
            b("key")
        );

        // The monster, in front, turns toward the man behind it.
        let (monster, man) = (obj(&doc, "monster"), obj(&doc, "man"));
        let yaw = monster["rotation"][1].as_f64().unwrap();
        assert!((yaw - toward(&doc, "monster", "man")).abs() < 0.1, "{yaw}");
        assert!(monster["position"][2].as_f64().unwrap() > man["position"][2].as_f64().unwrap());
        assert_eq!(monster["src"], "otto://design/a-monster@latest");
    }

    /// Acceptance defects 4-6: sizes from what things are (or the plan's
    /// metres), a library piece at its own size, coins inside the chest, the
    /// key at a skinned monster's neck, the held sword in front of the man,
    /// and every piece in its own left/right column.
    #[test]
    fn sizes_slots_and_attachments_follow_the_plan() {
        let mut plan = Plan {
            title: "Sword".into(),
            items: vec![
                item("man", "character"),
                item("monster", "character"),
                item("sword", "object"),
                item("chest", "object"),
                item("coins", "object"),
                item("key", "object"),
                item("lantern", "object"),
                item("rock", "object"),
            ],
            ..Default::default()
        };
        plan.items[1].name = "The Monster".into();
        plan.items[0].details.holds = vec![crate::productions::Hold {
            item: "sword".into(),
            socket: "right_hand".into(),
        }];
        (plan.items[4].name, plan.items[5].name) = ("Gold Coins".into(), "Chest Key".into());
        plan.items[6].details.size_m = Some(0.4);
        let attach = |id: &str, to: &str, at: &str| {
            let mut p = place(id, "front");
            p.attach_to = Some(Attach {
                item: to.into(),
                at: at.into(),
            });
            p
        };
        let mut monster = place("monster", "right");
        monster.facing = Some("man".into());
        plan.layout = vec![
            place("man", "left"),
            monster,
            place("sword", "center"),
            place("chest", "back-right"),
            attach("coins", "chest", "inside"),
            attach("key", "monster", "neck"),
            place("lantern", "front-right"),
            place("rock", "back-left"),
        ];
        let unit = Bounds {
            min: [-0.3, 0.0, -0.2],
            max: [0.3, 0.9, 0.2],
        };
        let mut models: Vec<Model> = plan.items.iter().map(|i| model(&i.id, unit)).collect();
        models[1].joints.insert("neck".into(), [0.0, 0.75, 0.0]);
        models[7].real_size = true;
        let (doc, notes) = build_scene(&plan, &models);
        check_layout(&doc, &models, &["coins", "key"]);
        assert!(notes.is_empty(), "{notes:?}");
        let b = |id| world_box(obj(&doc, id), &models);
        let longest = |id| {
            [0, 1, 2]
                .map(|k| b(id).size(k))
                .into_iter()
                .fold(0.0, f64::max)
        };
        assert!((b("man").size(1) - 1.8).abs() < 1e-3);
        assert!(
            (b("monster").size(1) - 2.7).abs() < 1e-3,
            "a monster is big"
        );
        for (id, m) in [
            ("sword", 1.0),
            ("chest", 0.6),
            ("coins", 0.3),
            ("key", 0.2),
            ("lantern", 0.4),
        ] {
            assert!((longest(id) - m).abs() < 1e-3, "{id}: {}", longest(id));
        }
        assert_eq!(obj(&doc, "rock")["scale"], json!([1.0, 1.0, 1.0]));

        // Columns: left of the middle, right of it; the sword he holds (no
        // skeleton) beside him on his right (his own -x), on the ground.
        let (man, sword) = (b("man"), b("sword"));
        assert!(man.mid(0) < 0.0 && b("monster").mid(0) > 0.0 && b("chest").mid(0) > 0.0);
        assert!(b("rock").mid(0) < 0.0 && b("lantern").mid(0) > 0.0);
        assert!(
            sword.max[0] <= man.min[0] + 1e-3 && sword.min[1].abs() < 1e-3,
            "{sword:?}"
        );

        // Coins in the chest rest on its floor inside it; the key at the
        // monster's neck on the man's side.
        let (coins, chest) = (b("coins"), b("chest"));
        assert!(
            (coins.min[1] - (chest.min[1] + 0.05 * chest.size(1))).abs() < 1e-3,
            "{coins:?} {chest:?}"
        );
        for k in [0, 2] {
            assert!((coins.mid(k) - chest.mid(k)).abs() < 1e-3);
        }
        let (key, monster) = (b("key"), b("monster"));
        assert!(
            (key.max[1] - 0.75 * 3.0).abs() < 1e-3,
            "hangs from the neck bone: {key:?}"
        );
        assert!(
            key.mid(0) < monster.mid(0) - 0.1,
            "in front, toward the man: {key:?}"
        );
    }

    /// The turn (degrees) that faces `a` toward `b` (glTF faces +z).
    fn toward(doc: &Value, a: &str, b: &str) -> f64 {
        let p = |id| nums::<3>(&obj(doc, id)["position"]).unwrap();
        let (pa, pb) = (p(a), p(b));
        (pb[0] - pa[0]).atan2(pb[2] - pa[2]).to_degrees()
    }

    /// Where a placed piece spans across the picture (perspective, from the
    /// document's camera, in units of the view's half width).
    fn on_screen(doc: &Value, models: &[Model], id: &str) -> (f64, f64) {
        let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
        let target = nums::<3>(&doc["camera"]["target"]).unwrap();
        let fwd = [0, 1, 2].map(|k| target[k] - cam[k]);
        let right = [-fwd[2], 0.0, fwd[0]];
        let n = |v: [f64; 3]| (0..3).map(|k| v[k] * v[k]).sum::<f64>().sqrt();
        let (fwd, right) = (fwd.map(|x| x / n(fwd)), right.map(|x| x / n(right)));
        let xs: Vec<f64> = corners(obj(doc, id), models)
            .iter()
            .map(|c| {
                let q = [0, 1, 2].map(|k| c[k] - cam[k]);
                let dot = |a: [f64; 3]| (0..3).map(|k| a[k] * q[k]).sum::<f64>();
                dot(right) / dot(fwd)
            })
            .collect();
        (
            xs.iter().copied().fold(f64::MAX, f64::min),
            xs.iter().copied().fold(f64::MIN, f64::max),
        )
    }

    /// How much of the view the pieces span, across or up (1 = edge to edge).
    fn fill(doc: &Value, models: &[Model]) -> f64 {
        let ids: Vec<String> = models.iter().map(|m| m.item.clone()).collect();
        let (lo, hi) = ids
            .iter()
            .map(|id| on_screen(doc, models, id))
            .fold((f64::MAX, f64::MIN), |(lo, hi), (a, b)| {
                (lo.min(a), hi.max(b))
            });
        let half = (doc["camera"]["fov"].as_f64().unwrap() / 2.0)
            .to_radians()
            .tan();
        (hi - lo) / (2.0 * half)
    }

    /// The owner's pier scene (his real plan): the keeper, the crate, the
    /// telescope and the rope stand ON the pier's deck where the plan says
    /// (middle; left, beside the keeper; back right; front right), never in a
    /// pile and never into each other; the fox on the crate; the lantern at
    /// the keeper's right hand (beside him on that side without a skeleton);
    /// no "I can't find" notes; a ground no bigger than the group plus a
    /// margin; the group fills the view. Without the fox the rest is the same.
    #[test]
    fn several_pieces_on_one_surface_stand_where_the_plan_says() {
        let plan: Plan =
            serde_json::from_str(include_str!("fixtures/owner-pier-plan.json")).unwrap();
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let all = [
            model("pier", b([-4.0, 0.0, -2.0], [4.0, 0.5, 2.0])),
            model("keeper", b([-0.4, 0.0, -0.3], [0.4, 2.0, 0.3])),
            model("lantern", b([-0.1, 0.0, -0.1], [0.1, 0.34, 0.1])),
            model("crate", b([-0.5, 0.0, -0.3], [0.5, 0.6, 0.3])),
            model("fox", b([-0.1, 0.0, -0.2], [0.1, 0.38, 0.2])),
            model("telescope", b([-0.3, 0.0, -0.3], [0.3, 1.5, 0.3])),
            model("rope", b([-0.55, 0.0, -0.55], [0.55, 0.2, 0.55])),
        ];
        for (with_fox, with_hand) in [(true, false), (false, false), (true, true)] {
            let mut models: Vec<Model> = all
                .iter()
                .filter(|m| with_fox || m.item != "fox")
                .map(|m| Model {
                    item: m.item.clone(),
                    artifact_id: m.artifact_id.clone(),
                    joints: BTreeMap::new(),
                    tris: Vec::new(),
                    ..*m
                })
                .collect();
            if with_hand {
                models[1]
                    .joints
                    .insert("right_hand".into(), [-0.45, 1.0, 0.1]);
            }
            let (doc, notes) = build_scene(&plan, &models);
            assert!(notes.is_empty(), "{notes:?}");
            let w = |id| world_box(obj(&doc, id), &models);
            let (pier, keeper) = (w("pier"), w("keeper"));
            let deck = ["keeper", "crate", "telescope", "rope"];
            for id in deck {
                let o = w(id);
                assert!(
                    (o.min[1] - pier.max[1]).abs() < 1e-3,
                    "{id} stands on the deck: {o:?}"
                );
                for k in [0, 2] {
                    assert!(
                        o.min[k] >= pier.min[k] - 1e-3 && o.max[k] <= pier.max[k] + 1e-3,
                        "{id} on the deck: {o:?}"
                    );
                }
            }
            assert!(
                (keeper.mid(0) - pier.mid(0)).abs() < 1e-3
                    && (keeper.mid(2) - pier.mid(2)).abs() < 1e-3
            );
            assert!(w("crate").mid(0) < keeper.min[0], "the crate to his left");
            let (t, r) = (w("telescope"), w("rope"));
            assert!(
                t.min[0] > keeper.max[0] && t.mid(2) < keeper.mid(2),
                "back right: {t:?}"
            );
            assert!(
                r.min[0] > keeper.max[0] && r.mid(2) > keeper.mid(2),
                "front right: {r:?}"
            );
            for (i, a) in deck.iter().enumerate() {
                for c in &deck[i + 1..] {
                    let (p, q) = (w(a), w(c));
                    let meet = p.min[0] < q.max[0]
                        && q.min[0] < p.max[0]
                        && p.min[2] < q.max[2]
                        && q.min[2] < p.max[2];
                    assert!(!meet, "{a} overlaps {c}");
                }
            }
            if with_fox {
                on_top(&doc, &models, "fox", "crate");
            }
            let lantern = w("lantern");
            let from = (lantern.mid(0) - keeper.mid(0)).hypot(lantern.mid(2) - keeper.mid(2));
            if with_hand {
                // In view: nothing stands between the camera and it.
                let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
                let to = [0, 1, 2].map(|k| lantern.mid(k) - cam[k]);
                for id in ["keeper", "crate", "telescope", "rope"] {
                    assert!(
                        !blocks(&doc, &models, id, cam, to),
                        "{id} hides the lantern"
                    );
                }
                // Centred on the hand bone (through his turn toward the fox).
                assert!((from - 0.45f64.hypot(0.1)).abs() < 1e-3, "{lantern:?}");
                assert!(
                    (lantern.mid(1) - (pier.max[1] + 1.0)).abs() < 1e-3,
                    "{lantern:?}"
                );
            } else {
                // Beside him on his right, standing on the deck, not on his head.
                assert!((lantern.min[1] - pier.max[1]).abs() < 1e-3, "{lantern:?}");
                let meet = lantern.min[0] < keeper.max[0]
                    && keeper.min[0] < lantern.max[0]
                    && lantern.min[2] < keeper.max[2]
                    && keeper.min[2] < lantern.max[2];
                assert!(!meet, "{lantern:?} {keeper:?}");
                if !with_fox {
                    assert!(
                        lantern.max[0] <= keeper.min[0] + 1e-3,
                        "his right is his own -x"
                    );
                }
            }
            // The ground is the group's size plus a margin, and the group fills the view.
            let ground = obj(&doc, "ground");
            let group = doc["objects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|o| o["type"] == "gltf")
                .map(|o| world_box(o, &models))
                .reduce(|a, b| a.add(b.min).add(b.max))
                .unwrap();
            assert!(
                ground["scale"][0].as_f64().unwrap() <= group.size(0) + 1.0 + 1e-3,
                "{ground}"
            );
            assert!(
                ground["scale"][1].as_f64().unwrap() <= group.size(2) + 1.0 + 1e-3,
                "{ground}"
            );
            assert!(fill(&doc, &models) >= 0.6, "fills {}", fill(&doc, &models));
        }
    }

    /// Every relation of `layout[].rel`, and the same read from an older
    /// plan's `attach_to` words: on (at a spot), in (on its floor inside),
    /// under, beside / behind / in front of, above, around, held (at the hand
    /// bone) and worn (a hat on the head).
    #[test]
    fn every_relation_places_the_piece_where_it_says() {
        let ids = [
            "table", "cup", "box", "ball", "cat", "dog", "toy", "lamp", "bird", "kid", "hat",
            "stick", "rock1", "rock2",
        ];
        let mut plan = Plan {
            title: "Rel".into(),
            items: ids.map(|i| item(i, "object")).to_vec(),
            ..Default::default()
        };
        plan.items[9].kind = "character".into();
        let rel = |id: &str, to: &str, how: &str, extra: Value| {
            let mut r = json!({"to": to, "how": how});
            r.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let mut p = place(id, "center");
            p.rel = Some(serde_json::from_value(r).unwrap());
            p
        };
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let small = b([-0.1, 0.0, -0.1], [0.1, 0.2, 0.1]);
        let mut models: Vec<Model> = ids.iter().map(|i| model(i, small)).collect();
        models[0].bounds = b([-1.0, 0.0, -0.5], [1.0, 0.8, 0.5]);
        models[2].bounds = b([-0.3, 0.0, -0.3], [0.3, 0.4, 0.3]);
        models[9].bounds = b([-0.25, 0.0, -0.2], [0.25, 1.2, 0.2]);
        models[9]
            .joints
            .insert("right_hand".into(), [-0.3, 0.6, 0.0]);
        for (i, m) in [(0, 2.0), (2, 0.6), (9, 1.2)] {
            plan.items[i].details.size_m = Some(m);
        }
        for i in [1, 3] {
            plan.items[i].details.size_m = Some(0.2);
        }
        plan.layout = vec![
            place("table", "center"),
            rel("cup", "table", "on", json!({"spot": "front-left"})),
            place("box", "right"),
            rel("ball", "box", "in", json!({})),
            rel("cat", "table", "under", json!({})),
            rel("dog", "table", "beside", json!({"side": "left"})),
            rel("toy", "table", "behind", json!({})),
            rel("lamp", "table", "above", json!({})),
            rel("bird", "table", "in_front_of", json!({})),
            place("kid", "left"),
            rel("hat", "kid", "worn", json!({"part": "head"})),
            rel("stick", "kid", "held", json!({"hand": "right_hand"})),
            rel("rock1", "box", "around", json!({})),
            rel("rock2", "box", "around", json!({})),
        ];
        let check = |plan: &Plan| {
            let (doc, notes) = build_scene(plan, &models);
            assert!(notes.is_empty(), "{notes:?}");
            let w = |id| world_box(obj(&doc, id), &models);
            let (table, bx, kid) = (w("table"), w("box"), w("kid"));
            let cup = w("cup");
            assert!(
                (cup.min[1] - table.max[1]).abs() < 1e-3
                    && cup.mid(0) < table.mid(0)
                    && cup.mid(2) > table.mid(2),
                "on, front-left: {cup:?}"
            );
            let ball = w("ball");
            assert!(
                ball.min[1] > bx.min[1]
                    && ball.max[1] < bx.max[1]
                    && (ball.mid(0) - bx.mid(0)).abs() < 1e-3,
                "in: {ball:?}"
            );
            let cat = w("cat");
            assert!(
                cat.min[1].abs() < 1e-3
                    && cat.max[1] < table.max[1]
                    && (cat.mid(0) - table.mid(0)).abs() < 1e-3,
                "under: {cat:?}"
            );
            let dog = w("dog");
            assert!(
                dog.max[0] <= table.min[0] + 1e-3 && dog.min[1].abs() < 1e-3,
                "beside left: {dog:?}"
            );
            assert!(w("toy").max[2] <= table.min[2] + 1e-3, "behind");
            assert!(w("bird").min[2] >= table.max[2] - 1e-3, "in front");
            assert!(w("lamp").min[1] > table.max[1], "above");
            let hat = w("hat");
            assert!(
                hat.min[1] < kid.max[1]
                    && hat.max[1] > kid.max[1]
                    && (hat.mid(0) - kid.mid(0)).abs() < 1e-3,
                "worn on the head: {hat:?}"
            );
            let stick = w("stick");
            assert!(
                (stick.mid(1) - 0.6).abs() < 1e-3 && stick.min[1] < kid.max[1],
                "held at the hand: {stick:?}"
            );
            for r in ["rock1", "rock2"] {
                let rb = w(r);
                let meet = rb.min[0] < bx.max[0]
                    && bx.min[0] < rb.max[0]
                    && rb.min[2] < bx.max[2]
                    && bx.min[2] < rb.max[2];
                assert!(!meet && rb.min[1].abs() < 1e-3, "around: {r} {rb:?}");
            }
        };
        check(&plan);
        // The same plan in an older plan's words.
        let words = [
            ("cup", "table", "front left of the tabletop"),
            ("ball", "box", "inside the box"),
            ("cat", "table", "on the floor between the table legs"),
            ("dog", "table", "on the floor beside the table, on its left"),
            ("toy", "table", "behind the table"),
            ("lamp", "table", "above the table"),
            ("bird", "table", "in front of the table"),
            ("hat", "kid", "on her head"),
            ("stick", "kid", "in her right hand"),
            ("rock1", "box", "around the box"),
            ("rock2", "box", "around the box"),
        ];
        for p in plan.layout.iter_mut() {
            if let Some((_, to, at)) = words.iter().find(|(id, ..)| *id == p.item) {
                p.rel = None;
                p.attach_to = Some(Attach {
                    item: to.to_string(),
                    at: at.to_string(),
                });
            }
        }
        check(&plan);
    }

    /// A flat base bigger than the rest (a stage) carries the other free
    /// pieces, each where its place says on it.
    #[test]
    fn free_pieces_stand_on_a_floor_like_base() {
        let mut plan = Plan {
            title: "Band".into(),
            items: ["stage", "bear", "drum"]
                .map(|i| item(i, "object"))
                .to_vec(),
            ..Default::default()
        };
        plan.items[0].details.size_m = Some(6.0);
        plan.layout = vec![
            place("stage", "center"),
            place("bear", "left"),
            place("drum", "right"),
        ];
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let models = vec![
            model("stage", b([-3.0, 0.0, -2.0], [3.0, 0.4, 2.0])),
            model("bear", b([-0.3, 0.0, -0.3], [0.3, 0.6, 0.3])),
            model("drum", b([-0.3, 0.0, -0.3], [0.3, 0.6, 0.3])),
        ];
        let (doc, _) = build_scene(&plan, &models);
        let w = |id| world_box(obj(&doc, id), &models);
        let stage = w("stage");
        for id in ["bear", "drum"] {
            assert!(
                (w(id).min[1] - stage.max[1]).abs() < 1e-3,
                "{id} on the stage"
            );
        }
        assert!(w("bear").mid(0) < stage.mid(0) && w("drum").mid(0) > stage.mid(0));
    }

    #[test]
    fn a_rectangular_stage_uses_the_working_width_of_its_boards() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[{"id":"deck","kind":"object","name":"Low stage","description":"",
                 "details":{"prompt":"Rectangular wooden platform","dims_m":[5.0,0.4,3.0]}}],
            "layout":[{"item":"deck","place":"center"}]
        }))
        .unwrap();
        let corners = [
            [-2., 0., -1.2],
            [-2., 0., 1.2],
            [2., 0., 1.2],
            [2., 0., -1.2],
        ];
        let basis = turn_matrix([8., 38., 0.]);
        let rotated = |p| turn_point(&basis, p);
        let mut tris = quad(-2., 2., -1.2, 1.2, 0.32, true).to_vec();
        for i in 0..4 {
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            let top = |p: [f64; 3]| [p[0], 0.32, p[2]];
            tris.extend([[a, b, top(a)], [b, top(b), top(a)]]);
        }
        for v in tris.iter_mut().flatten() {
            *v = rotated(*v);
        }
        let models = [meshed("deck", &tris)];
        let (doc, _) = build_scene(&plan, &models);
        let o = obj(&doc, "deck");
        let turn = turn_matrix(nums(&o["rotation"]).unwrap());
        let scale: [f64; 3] = nums(&o["scale"]).unwrap();
        let placed = |v| {
            let v = rotated(v);
            turn_point(&turn, [0, 1, 2].map(|k| v[k] * scale[k]))
        };
        let a = placed(corners[0]);
        let b = placed(corners[3]);
        let c = placed(corners[1]);
        assert!(
            (a[0] - b[0]).abs() > 4.9 && (a[2] - b[2]).abs() < 0.12,
            "A 5 m stage needs a 5 m lateral edge, got {a:?} to {b:?}"
        );
        assert!((a[2] - c[2]).abs() > 2.9 && (a[1] - c[1]).abs() < 0.04);
    }

    #[test]
    fn a_circular_platform_has_no_rectangular_axis() {
        let rim = |i: usize, y| {
            let (s, c) = (i as f64 * std::f64::consts::TAU / 24.).sin_cos();
            [s, y, c]
        };
        let sides: Vec<_> = (0..24)
            .flat_map(|i| {
                let a = rim(i, 0.);
                let b = rim(i + 1, 0.);
                let c = rim(i, 0.3);
                let d = rim(i + 1, 0.3);
                [[a, b, c], [b, d, c]]
            })
            .collect();
        assert!(super::super::geometry::deck_axis(&sides, &turn_matrix([0.; 3])).is_none());
    }

    #[test]
    fn a_wide_curtain_spans_the_rear_of_its_support() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[
                {"id":"platform","kind":"object","name":"Raised platform","description":""},
                {"id":"back","kind":"object","name":"Velvet curtains","description":"",
                 "details":{"dims_m":[5.5,2.8,0.25]}}
            ],
            "layout":[{"item":"platform","place":"center"},
                      {"item":"back","rel":{"how":"on","to":"platform","spot":"back"}}]
        }))
        .unwrap();
        let mut deck = quad(-2.75, 2.75, -1.5, 1.5, 0.3, true).to_vec();
        deck.extend(quad(-2.75, 2.75, -1.5, 1.5, 0.0, false));
        let models = [
            Model {
                real_size: true,
                ..meshed("platform", &deck)
            },
            model(
                "back",
                Bounds {
                    min: [-2.39, 0., -1.64],
                    max: [2.39, 5.5, 1.64],
                },
            ),
        ];
        let (doc, _) = build_scene(&plan, &models);
        let curtain = world_box(obj(&doc, "back"), &models);
        let deck = world_box(obj(&doc, "platform"), &models);
        assert!((curtain.size(2) - 0.25).abs() < 0.01, "{curtain:?}");
        assert!((curtain.size(0) - deck.size(0)).abs() < 0.01);
        assert!(curtain.max[2] < deck.mid(2) && curtain.min[2] >= deck.min[2]);
        assert!((curtain.min[1] - deck.max[1]).abs() < 0.01);
    }

    #[test]
    fn a_stage_with_rear_curtains_opens_from_the_audience_side() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[
                {"id":"deck","kind":"object","name":"Stage","description":""},
                {"id":"back","kind":"object","name":"Curtains","description":"",
                 "details":{"dims_m":[5.0,2.4,0.15]}},
                {"id":"player","kind":"character","name":"Player","description":""},
                {"id":"singer","kind":"character","name":"Singer","description":""}
            ],
            "layout":[{"item":"deck","place":"center"},
                      {"item":"back","rel":{"how":"on","to":"deck","spot":"back"}},
                      {"item":"player","facing":"back","rel":{"how":"on","to":"deck","spot":"left"}},
                      {"item":"singer","rel":{"how":"on","to":"deck","spot":"right"}}]
        })).unwrap();
        let models = [
            ("deck", [-2.6, 0., -1.5], [2.6, 0.3, 1.5]),
            ("back", [-2.5, 0., -0.075], [2.5, 2.4, 0.075]),
            ("player", [-0.3, 0., -0.2], [0.3, 1.7, 0.2]),
            ("singer", [-0.25, 0., -0.2], [0.25, 1.5, 0.2]),
        ]
        .map(|(id, min, max)| Model {
            real_size: true,
            ..model(id, Bounds { min, max })
        });
        let (doc, _) = build_scene(&plan, &models);
        let eye = nums::<3>(&doc["camera"]["position"]).unwrap();
        let aim = nums::<3>(&doc["camera"]["target"]).unwrap();
        assert!(
            eye[2] - aim[2] > 0.5 * (eye[0] - aim[0]).abs(),
            "behind or end-on: {eye:?} toward {aim:?}"
        );
    }

    #[test]
    fn a_spectator_facing_the_lineup_keeps_its_place_in_front_of_the_stage() {
        let mut plan: Plan = serde_json::from_value(json!({
            "items":[
                {"id":"deck","kind":"object","name":"Raised stage","description":""},
                {"id":"singer","kind":"character","name":"Singer","description":""},
                {"id":"guest","kind":"character","name":"Guest","description":""}
            ],
            "lineup":["singer"],
            "layout":[{"item":"deck","place":"center"},
                      {"item":"singer","rel":{"how":"on","to":"deck"}},
                      {"item":"guest","place":"front-left","facing":"singer"}]
        }))
        .unwrap();
        let models = [
            ("deck", [-2., 0., -1.], [2., 0.35, 1.]),
            ("singer", [-0.25, 0., -0.2], [0.25, 1.6, 0.2]),
            ("guest", [-0.2, 0., -0.25], [0.2, 1.4, 0.25]),
        ]
        .map(|(id, min, max)| Model {
            real_size: true,
            ..model(id, Bounds { min, max })
        });
        let (doc, _) = build_scene(&plan, &models);
        let bounds = |id| world_box(obj(&doc, id), &models);
        assert_eq!(
            obj(&doc, "guest")["backdrop"],
            true,
            "frame the performance"
        );
        let guest = bounds("guest");
        let deck = bounds("deck");
        assert!(
            guest.min[1].abs() < 0.01 && guest.min[2] >= deck.max[2],
            "{guest:?} vs {deck:?}"
        );
        assert!((bounds("singer").min[1] - deck.max[1]).abs() < 0.01);

        let (early, _) = build_scene(&plan, &[models[0].clone(), models[2].clone()]);
        let early_guest = obj(&early, "guest");
        assert!(world_box(early_guest, &models).min[1].abs() < 0.01);
        assert!(early_guest.get("backdrop").is_none());

        // A planner may name the stage, rather than one specific player,
        // as the object of an audience member's attention.
        plan.items[2].description = "Spectator watching the band.".into();
        plan.layout[2].facing = Some("deck".into());
        let (watching_stage, _) = build_scene(&plan, &models);
        let guest = obj(&watching_stage, "guest");
        assert_eq!(guest["backdrop"], true);
        assert!(world_box(guest, &models).min[2] >= deck.max[2]);

        plan.layout[2].rel = plan.layout[1].rel.clone();
        let (doc, _) = build_scene(&plan, &models);
        assert!((world_box(obj(&doc, "guest"), &models).min[1] - deck.max[1]).abs() < 0.01);
    }

    #[test]
    fn rotating_a_thin_mesh_does_not_enlarge_its_measured_height() {
        let src = glb_of_tris(&[[[0., 0., 0.], [0., 1., 1.], [1., 0.5, 0.5]]]);
        let mut doc = glb_json(&src).unwrap();
        let half = 22.5_f64.to_radians();
        doc["nodes"][0]["rotation"] = json!([half.sin(), 0., 0., half.cos()]);
        let mut bytes = glb_of(&doc);
        let data = glb_bin(&src).unwrap();
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"BIN\0");
        bytes.extend_from_slice(data);
        let length = bytes.len() as u32;
        bytes[8..12].copy_from_slice(&length.to_le_bytes());
        let b = glb_bounds(&bytes).unwrap();
        assert!(
            b.max[1] - b.min[1] < 0.00001,
            "transformed triangle is flat: {b:?}"
        );
    }

    /// A GLB holding `tris` as one mesh (float positions, no indices).
    fn glb_of_tris(tris: &[Tri]) -> Vec<u8> {
        let mut bin: Vec<u8> = tris
            .iter()
            .flatten()
            .flat_map(|p| p.iter().flat_map(|v| (*v as f32).to_le_bytes()))
            .collect();
        let (lo, hi) = tris
            .iter()
            .flatten()
            .fold(([f64::MAX; 3], [f64::MIN; 3]), |(l, h), p| {
                (
                    [0, 1, 2].map(|k| l[k].min(p[k])),
                    [0, 1, 2].map(|k| h[k].max(p[k])),
                )
            });
        let doc = json!({
            "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0, "name": "body"}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": bin.len()}],
            "accessors": [{"bufferView": 0, "componentType": 5126, "count": tris.len() * 3, "type": "VEC3", "min": lo, "max": hi}]
        });
        let mut text = serde_json::to_vec(&doc).unwrap();
        while !text.len().is_multiple_of(4) {
            text.push(b' ');
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let mut b = b"glTF".to_vec();
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&((28 + text.len() + bin.len()) as u32).to_le_bytes());
        b.extend_from_slice(&(text.len() as u32).to_le_bytes());
        b.extend_from_slice(b"JSON");
        b.extend_from_slice(&text);
        b.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        b.extend_from_slice(b"BIN\0");
        b.extend_from_slice(&bin);
        b
    }

    /// A flat rectangle at height `y` over x0..x1, z0..z1, facing up (or down).
    fn quad(x0: f64, x1: f64, z0: f64, z1: f64, y: f64, up: bool) -> [Tri; 2] {
        let (a, b, c, d) = ([x0, y, z0], [x0, y, z1], [x1, y, z1], [x1, y, z0]);
        match up {
            true => [[a, b, c], [a, c, d]],
            false => [[a, c, b], [a, d, c]],
        }
    }

    /// Whether the placed piece `id` (its turned box) stands across the
    /// segment from `cam` along `to`.
    fn blocks(doc: &Value, models: &[Model], id: &str, cam: [f64; 3], to: [f64; 3]) -> bool {
        let c = corners(obj(doc, id), models);
        const FACES: [[usize; 4]; 6] = [
            [0, 1, 3, 2],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 3, 7, 5],
        ];
        FACES.iter().any(|f| {
            hits(cam, to, &[c[f[0]], c[f[1]], c[f[2]]])
                || hits(cam, to, &[c[f[0]], c[f[2]], c[f[3]]])
        })
    }

    /// A model from its triangles (bounds and triangles read back from a GLB).
    fn meshed(id: &str, tris: &[Tri]) -> Model {
        let bytes = glb_of_tris(tris);
        Model {
            bounds: glb_bounds(&bytes).unwrap(),
            tris: glb_tris(&bytes),
            real_size: true,
            ..model(id, Bounds::UNIT)
        }
    }

    #[test]
    fn a_microphone_attaches_to_the_top_of_its_stand() {
        let mut tris = quad(-0.2, 0.2, -0.2, 0.2, 0.02, true).to_vec();
        tris.extend(quad(-0.18, 0.18, -0.18, 0.18, 0.0, false));
        // A boom's narrow mounting point sits off-centre.
        tris.push([[0.0, 0.02, 0.0], [0.12, 1.2, 0.0], [0.10, 1.18, 0.0]]);
        let stand = meshed("stand", &tris);
        let mic = Model {
            real_size: true,
            ..model(
                "mic",
                Bounds {
                    min: [-0.02, 0.0, -0.02],
                    max: [0.02, 0.12, 0.02],
                },
            )
        };
        let mut stand_item = item("stand", "object");
        stand_item.name = "Microphone stand".into();
        let mut mic_item = item("mic", "object");
        mic_item.name = "Microphone".into();
        let mut attach = place("mic", "");
        attach.rel = Some(serde_json::from_value(json!({"how":"on","to":"stand"})).unwrap());
        let plan = Plan {
            items: vec![stand_item, mic_item],
            layout: vec![place("stand", "center"), attach],
            ..Default::default()
        };
        let models = [stand, mic];
        let (d, _) = build_scene(&plan, &models);
        let base = world_box(obj(&d, "stand"), &models);
        let head = world_box(obj(&d, "mic"), &models);
        assert!(
            (head.min[1] - base.max[1]).abs() < 0.01,
            "mic at {}, stand top {}",
            head.min[1],
            base.max[1]
        );
        assert!((head.mid(0) - 0.12).abs() < 0.03);
    }

    #[test]
    fn a_bass_placed_beside_its_player_moves_into_reach_on_the_stage() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[
                {"id":"stage","kind":"object","name":"Small stage","description":""},
                {"id":"bear","kind":"character","name":"Bear","description":"","details":{"pose":{"kind":"stand","play":"bass"}}},
                {"id":"bass","kind":"object","name":"Double bass","description":""}
            ],
            "lineup":["bear"],
            "layout":[{"item":"stage","place":"center"},
                      {"item":"bear","rel":{"how":"on","to":"stage","spot":"left"}},
                      {"item":"bass","rel":{"how":"beside","to":"bear","side":"left"}}]
        })).unwrap();
        let mut deck = quad(-2.0, 2.0, -1.0, 1.0, 0.3, true).to_vec();
        deck.extend(quad(-2.0, 2.0, -1.0, 1.0, 0.0, false));
        let mut models = [
            meshed("stage", &deck),
            model(
                "bear",
                Bounds {
                    min: [-0.4, 0., -0.3],
                    max: [0.4, 1.8, 0.3],
                },
            ),
            model(
                "bass",
                Bounds {
                    min: [-0.25, 0., -0.15],
                    max: [0.25, 1.65, 0.15],
                },
            ),
        ];
        for m in &mut models {
            m.real_size = true;
        }
        let (doc, _) = build_scene(&plan, &models);
        let bounds = |id| world_box(obj(&doc, id), &models);
        let (bear, bass, deck) = (bounds("bear"), bounds("bass"), bounds("stage"));
        let separation = (bear.mid(0) - bass.mid(0)).hypot(bear.mid(2) - bass.mid(2));
        assert!(
            separation < 0.5,
            "the strings must be within arm's reach: {separation} m"
        );
        assert!(bass.min[0] >= deck.min[0] && bass.max[0] <= deck.max[0]);
        assert!((bass.min[1] - deck.max[1]).abs() < 0.01);
    }

    #[test]
    fn the_two_striking_tips_stay_above_their_drum_heads_when_posing_changes_the_box() {
        let b = |min, max| Bounds { min, max };
        let tips = [[-0.19, 0.36, 0.55], [0.14, 0.42, 0.48]];
        let anchor = [0.08, -0.13];
        let manifest = json!({"bounds":{"min":[-0.6,0.,-0.8],"max":[0.5,0.7,0.6]},
            "contact":{"sticks":tips},"drum_contact":{"kit":"kit","xz":anchor}});
        let receipt = json!({"manifest":manifest,"spec":{"kind":"stand","play":"kit"}});
        let mut plan:Plan=serde_json::from_value(json!({
            "items":[{"id":"player","kind":"character","name":"Percussionist","description":"",
                      "details":{"body":"many_legs","pose":{"kind":"stand","play":"kit"}},"result":{"pose":receipt}},
                     {"id":"kit","kind":"object","name":"Drum kit","description":""}],
            "layout":[{"item":"kit","place":"center"},{"item":"player","facing":"kit","rel":{"how":"behind","to":"kit"}}]
        })).unwrap();
        let mut player = model("player", b([-0.6, 0., -0.6], [0.6, 0.65, 0.7]));
        player.apply_pose(Posed::of(&manifest).unwrap(), &receipt, Some("many_legs"));
        let mut kit = model("kit", b([-0.5, 0., -0.35], [0.5, 0.5, 0.35]));
        kit.tris = quad(-0.5, 0.5, -0.35, 0.35, 0.48, true).to_vec();
        kit.real_size = true;
        player.real_size = true;
        let models = [player, kit];
        let (doc, _) = build_scene(&plan, &models);
        let performer = super::super::setting::upright_transform(obj(&doc, "player")).unwrap();
        let instrument = super::super::setting::upright_transform(obj(&doc, "kit")).unwrap();
        let want = instrument([anchor[0], 0., anchor[1]]);
        let [first, second] = tips.map(performer);
        for k in [0, 2] {
            assert!(((first[k] + second[k]) / 2. - want[k]).abs() < 0.002);
        }

        // A swimmer can lift its body to complete a stroke. Its two tips would
        // otherwise land below the actual head even with their X/Z centred.
        plan.setting = Some(super::super::setting::Setting {
            world: "underwater".into(),
            ..Default::default()
        });
        let (water, _) = build_scene(&plan, &models);
        let swimmer = super::super::setting::upright_transform(obj(&water, "player")).unwrap();
        let kit = super::super::setting::upright_transform(obj(&water, "kit")).unwrap();
        let surface = kit([0., 0.48, 0.])[1];
        let lowest = tips
            .map(|tip| swimmer(tip)[1])
            .into_iter()
            .fold(f64::MAX, f64::min);
        assert!(
            (lowest - surface).abs() < 0.002,
            "tip {lowest} vs drumhead {surface}"
        );
    }

    #[test]
    fn a_kit_aimed_at_by_held_sticks_stays_in_reach() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[
                {"id":"player","kind":"character","name":"Drummer","description":"Plays the drums.","details":{"body":"two_legs"}},
                {"id":"kit","kind":"object","name":"Blue drum kit","description":""},
                {"id":"stick","kind":"object","name":"Drumstick","description":""}
            ],
            "layout":[{"item":"player","place":"center"},
                      {"item":"kit","rel":{"how":"in_front_of","to":"player"}},
                      {"item":"stick","rel":{"how":"held","hand":"left_hand","to":"player","toward":"kit"}}]
        })).unwrap();
        let models = [
            ("player", [-0.35, 0., -0.30], [0.35, 0.95, 0.30]),
            ("kit", [-0.55, 0., -0.325], [0.55, 0.65, 0.325]),
        ]
        .map(|(id, min, max)| Model {
            real_size: true,
            ..model(id, Bounds { min, max })
        });
        let (doc, _) = build_scene(&plan, &models);
        let player = world_box(obj(&doc, "player"), &models);
        let kit = world_box(obj(&doc, "kit"), &models);
        let reach = kit.min[2] - player.mid(2);
        assert!(
            (0.1..0.30).contains(&reach),
            "nearest drum is {reach} m from the body"
        );
    }

    #[test]
    fn a_drummer_faces_the_heads_and_holds_the_sloping_stick_along_its_shaft() {
        let manifest = json!({"bounds":{"min":[-0.2,0.,-0.15],"max":[0.2,0.9,0.15]},
            "grip":{"hand":"left","style":"raise","point":[0.12,0.48,0.17],"axis":[0.,0.,1.]}});
        let receipt = json!({"manifest":manifest,"spec":{"kind":"stand"}});
        let plan: Plan = serde_json::from_value(json!({
            "items":[{"id":"player","kind":"character","name":"Drummer","description":"",
                      "details":{"body":"two_legs"},"result":{"pose":receipt}},
                     {"id":"kit","kind":"object","name":"Drum kit","description":""},
                     {"id":"stick","kind":"object","name":"Drumstick","description":""}],
            "layout":[{"item":"player","place":"center"},
                      {"item":"kit","rel":{"how":"beside","to":"player","side":"left"}},
                      {"item":"stick","rel":{"how":"held","hand":"left_hand","to":"player","toward":"kit"}}]
        })).unwrap();
        let mut player = model(
            "player",
            Bounds {
                min: [-0.2, 0., -0.15],
                max: [0.2, 0.9, 0.15],
            },
        );
        player.real_size = true;
        player.apply_pose(Posed::of(&manifest).unwrap(), &receipt, Some("two_legs"));
        let kit = meshed("kit", &quad(-0.4, 0.4, -0.35, 0.35, 0.5, true));
        let ends = [[0., 0., 0.], [0., 0.28, -0.18]];
        let stick = meshed(
            "stick",
            &[
                [[-0.005, 0., 0.], [0.005, 0., 0.], [0.005, 0.28, -0.18]],
                [
                    [-0.005, 0., 0.],
                    [0.005, 0.28, -0.18],
                    [-0.005, 0.28, -0.18],
                ],
            ],
        );
        let (doc, _) = build_scene(&plan, &[player, kit, stick]);
        let o = obj(&doc, "player");
        let at = nums::<3>(&o["position"]).unwrap();
        let kit = nums::<3>(&obj(&doc, "kit")["position"]).unwrap();
        let yaw = o["rotation"][1].as_f64().unwrap().to_radians();
        let expected = [yaw.sin(), 0., yaw.cos()];
        let (dx, dz) = (kit[0] - at[0], kit[2] - at[2]);
        assert!(
            (dx * expected[0] + dz * expected[2]) / dx.hypot(dz) > 0.85,
            "back to the kit: {o}"
        );
        let stick = obj(&doc, "stick");
        let scale = nums::<3>(&stick["scale"]).unwrap();
        let rotation = turn_matrix(nums::<3>(&stick["rotation"]).unwrap());
        let delta = turned(
            rotation,
            [0, 1, 2].map(|k| (ends[1][k] - ends[0][k]) * scale[k]),
        );
        let length = delta[0].hypot(delta[1]).hypot(delta[2]);
        assert!(
            delta[0] * expected[0] + delta[2] * expected[2] > 0.98 * length,
            "stick across fist: {delta:?}"
        );
    }

    #[test]
    fn the_far_plucking_arm_can_reach_the_front_of_an_upright_bass() {
        let plan: Plan = serde_json::from_value(json!({
            "title": "String duet",
            "items": [
                {"id":"bass","name":"Double bass","kind":"object","description":"A standing instrument"},
                {"id":"player","name":"Musician","kind":"character","description":"Playing the bass",
                 "details":{"pose":{"kind":"stand","play":"bass"}}}
            ],
            "layout": [
                {"item":"bass","place":"center"},
                {"item":"player","rel":{"to":"bass","how":"beside","side":"left"}}
            ]
        })).unwrap();
        // Metres from the generated player's rest rig and its floor-standing instrument.
        let models = [
            Model {
                real_size: true,
                ..model(
                    "player",
                    Bounds {
                        min: [-0.811, -1.017, -0.605],
                        max: [0.810, 0.933, 0.599],
                    },
                )
            },
            Model {
                real_size: true,
                ..model(
                    "bass",
                    Bounds {
                        min: [-0.302, -0.919, -0.586],
                        max: [0.308, 0.931, 0.237],
                    },
                )
            },
        ];
        let (doc, _) = build_scene(&plan, &models);
        let player = super::super::setting::upright_transform(obj(&doc, "player")).unwrap();
        let shoulder = player([-0.309, 0.403, -0.159]);
        let bass = models[1].bounds;
        let strings = super::super::setting::upright_transform(obj(&doc, "bass")).unwrap()([
            bass.mid(0),
            bass.min[1] + 0.47 * bass.size(1),
            bass.mid(2) + 0.4 * bass.size(2),
        ]);
        let distance = (shoulder[0] - strings[0])
            .hypot(shoulder[1] - strings[1])
            .hypot(shoulder[2] - strings[2]);
        // Shoulder–elbow, elbow–wrist and wrist–knuckle segments.
        assert!(
            distance <= 0.369 + 0.399 + 0.175,
            "right hand cannot reach the strings: {distance} m"
        );
    }

    #[test]
    fn a_performer_beside_an_instrument_stays_on_the_stage() {
        let mut deck = quad(-1.35, 1.35, -2.0, 2.0, 0.7, true).to_vec();
        deck.extend(quad(-1.35, 1.35, -2.0, 2.0, 0.0, false));
        let solid = |id, min, max| Model {
            real_size: true,
            ..model(id, Bounds { min, max })
        };
        let models = [
            meshed("stage", &deck),
            solid("bass", [-0.25, 0., -0.15], [0.25, 1.65, 0.15]),
            solid("bear", [-0.4, 0., -0.3], [0.4, 1.8, 0.3]),
        ];
        let mut bass = place("bass", "left");
        bass.rel =
            Some(serde_json::from_value(json!({"to":"stage","how":"on","spot":"left"})).unwrap());
        let mut bear = place("bear", "");
        bear.rel = Some(
            serde_json::from_value(json!({"to":"bass","how":"beside","side":"left"})).unwrap(),
        );
        let plan = Plan {
            items: vec![
                item("stage", "object"),
                item("bass", "object"),
                item("bear", "character"),
            ],
            layout: vec![place("stage", "center"), bass, bear],
            ..Default::default()
        };
        let (doc, _) = build_scene(&plan, &models);
        let bounds = |id| world_box(obj(&doc, id), &models);
        let (stage, player, bass) = (bounds("stage"), bounds("bear"), bounds("bass"));
        assert!(
            player.min[0] >= stage.min[0] - 0.001 && player.max[0] <= stage.max[0] + 0.001,
            "player stands beyond the platform: {player:?} {stage:?}"
        );
        assert!((player.min[1] - stage.max[1]).abs() < 0.01);
        assert!((player.mid(0) - bass.mid(0)).hypot(player.mid(2) - bass.mid(2)) < 1.5);
    }

    /// A deck whose box is far bigger than its planks (a tall sign post at
    /// its side): what stands on it stands on the planks, at their height,
    /// each where its place says, never over air; and a deck that came out
    /// standing high sinks so its planks are at 0.8 m.
    #[test]
    fn pieces_on_a_deck_stand_on_its_planks_not_on_its_box() {
        for (lift, name) in [(0.0, "Deck"), (2.0, "Wooden pier")] {
            // Planks, a post's cap above them and its foot on the ground.
            let mut tris: Vec<Tri> = quad(-3.0, 3.0, -1.5, 1.5, 1.0 + lift, true).to_vec();
            tris.extend(quad(3.9, 4.1, -0.1, 0.1, 2.0 + lift, true));
            tris.extend(quad(3.9, 4.1, -0.1, 0.1, 0.0, false));
            let mut plan = Plan {
                title: "Deck".into(),
                items: ["deck", "crate", "barrel", "lamp"]
                    .map(|i| item(i, "object"))
                    .to_vec(),
                ..Default::default()
            };
            plan.items[0].name = name.into();
            let on = |id: &str, place_at: &str| {
                let mut p = place(id, place_at);
                p.rel = Some(serde_json::from_value(json!({"to": "deck", "how": "on"})).unwrap());
                p
            };
            plan.layout = vec![
                place("deck", "center"),
                on("crate", "left"),
                on("barrel", "back-right"),
                on("lamp", "right"),
            ];
            let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
            let mut models = vec![meshed("deck", &tris)];
            for id in ["crate", "barrel", "lamp"] {
                let mut m = model(id, b([-0.3, 0.0, -0.3], [0.3, 0.6, 0.3]));
                m.real_size = true;
                models.push(m);
            }
            let (doc, notes) = build_scene(&plan, &models);
            let w = |id| world_box(obj(&doc, id), &models);
            let deck_box = w("deck");
            let planks = deck_box.min[1] + 1.0 + lift;
            let planks = if lift > 0.0 { 0.8 } else { planks };
            for id in ["crate", "barrel", "lamp"] {
                let o = w(id);
                assert!(
                    (o.min[1] - planks).abs() < 0.03,
                    "{name}: {id} stands on the planks: {o:?}, {notes:?}"
                );
                let (x0, z0) = (deck_box.min[0], deck_box.mid(2));
                assert!(
                    o.min[0] >= x0 - 1e-6 && o.max[0] <= x0 + 6.0 + 1e-6,
                    "{id} over the planks, not the post: {o:?}"
                );
                assert!(
                    o.min[2] >= z0 - 1.5 - 1e-6 && o.max[2] <= z0 + 1.5 + 1e-6,
                    "{id}: {o:?}"
                );
            }
            assert!(w("crate").mid(0) < w("lamp").mid(0) && w("barrel").mid(2) < w("crate").mid(2));
            if lift > 0.0 {
                assert!(
                    notes.iter().any(|n| n.contains("stands lower")),
                    "{notes:?}"
                );
            }
        }
    }

    #[test]
    fn adding_a_wall_fixture_does_not_move_the_floor_furniture_apart() {
        let plan: Plan = serde_json::from_value(json!({
            "title":"A practice room","setting":{"indoor":true,"ground":"wood_floor"},
            "items":[{"id":"piano","kind":"object","name":"Piano","description":"A piano.","details":{"dims_m":[1.5,1.2,0.8]}},
                {"id":"stool","kind":"object","name":"Low stool","description":"A stool.","details":{"dims_m":[0.4,0.5,0.4]}},
                {"id":"panel","kind":"object","name":"Acoustic panel","description":"A felt panel.","details":{"dims_m":[1.2,0.8,0.1]}}],
            "layout":[{"item":"piano","place":"back-left"},{"item":"stool","place":"front-right"},
                {"item":"panel","place":"wall"}]
        })).unwrap();
        let models: Vec<_> = ["piano", "stool", "panel"]
            .iter()
            .map(|id| model(id, Bounds::UNIT))
            .collect();
        let offset = |models: &[Model]| {
            let (doc, _) = build_scene(&plan, models);
            let piano = nums::<3>(&obj(&doc, "piano")["position"]).unwrap();
            let stool = nums::<3>(&obj(&doc, "stool")["position"]).unwrap();
            [stool[0] - piano[0], stool[2] - piano[2]]
        };
        let floor = offset(&models[..2]);
        assert!(floor[0] > 0. && floor[1] > 0.);
        assert_eq!(
            offset(&models),
            floor,
            "a wall panel occupies no floor-grid cell"
        );
    }

    #[test]
    fn a_toy_named_as_peeking_stays_at_the_bedcovers_edge() {
        let mut cover = item("quilt", "object");
        cover.details.dims_m = Some([0.8, 0.06, 1.4]);
        let mut toy = item("bear", "object");
        toy.description = "A little bear on its back peeks from under the quilt.".into();
        let spot = Spot {
            center: [0.0; 2],
            floor: 0.6,
            rot_y: 0.0,
            scale: [1.0; 3],
            tilt: None,
            b: Bounds {
                min: [-0.4, 0.0, -0.7],
                max: [0.4, 0.06, 0.7],
            },
        };
        let bear = Spot {
            b: Bounds {
                min: [-0.09, 0.0, -0.06],
                max: [0.09, 0.3, 0.06],
            },
            ..spot
        };
        let l = Link::of_rel(
            &serde_json::from_value(json!({"to":"quilt","how":"under","spot":"back"})).unwrap(),
        );
        let tuck = |it: &Item| {
            lay_textile(
                &l,
                (Some(it), Some(&cover)),
                None,
                (bear, spot),
                spot.center,
                true,
            )
        };
        let result = tuck(&toy).expect("the bear peeks over the folded edge");
        let head = result.to_world([0.0, 0.27, 0.04]);
        let foot = result.to_world([0.0, 0.03, 0.0]);
        assert!(
            head[2] < -0.7 && foot[2] > -0.7,
            "head {head:?}, foot {foot:?}"
        );
        assert!(head[1] > spot.floor && result.tilt.is_some());
        // The plan gives the final bed-space dimensions. A seated source mesh
        // may already have been squeezed to that height before we turn it.
        let seated = Spot {
            b: Bounds {
                min: [-0.145, -0.112, -0.127],
                max: [0.170, 0.128, 0.102],
            },
            scale: [0.20 / 0.315, 0.12 / 0.24, 0.31 / 0.229],
            ..bear
        };
        toy.details.dims_m = Some([0.20, 0.12, 0.31]);
        let laid = lay_textile(
            &l,
            (Some(&toy), Some(&cover)),
            None,
            (seated, spot),
            spot.center,
            true,
        )
        .unwrap();
        let nose = laid.to_world([0.0, 0.105, 0.08]);
        let toes = laid.to_world([0.0, -0.095, 0.08]);
        assert!(
            nose[2] < -0.7 && toes[2] > -0.7,
            "nose {nose:?}, toes {toes:?}"
        );
        assert!(
            (nose[2] - toes[2]).abs() > 0.23,
            "the laid body keeps its planned length"
        );
        toy.description = "A rabbit sits upright, tucked in under the quilt.".into();
        assert!(tuck(&toy).unwrap().tilt.is_none());
        toy.description = "A coin completely hidden under the cover.".into();
        assert!(tuck(&toy).is_none());
    }

    #[test]
    fn a_tucked_toy_and_its_cover_can_both_be_placed_on_the_same_cot() {
        let support = quad(-0.5, 0.5, -1., 1., 0.48, true);
        let sheet = quad(-0.39, 0.39, -0.64, 0.64, 0.02, true);
        let models = [
            meshed("cot", &support),
            meshed("cloth", &sheet),
            model(
                "toy",
                Bounds {
                    min: [-0.10, 0., -0.07],
                    max: [0.10, 0.28, 0.07],
                },
            ),
        ];
        let mut plan: Plan = serde_json::from_value(json!({
            "items":[{"kind":"object","id":"cot","name":"Camping cot","description":"A cot."},
                {"kind":"object","id":"cloth","name":"Green blanket","description":"The blanket covers the plush rabbit up to its chin.","details":{"dims_m":[0.78,0.04,1.28]}},
                {"kind":"object","id":"toy","name":"Plush rabbit","description":"A small rabbit lies tucked in with its head showing.","details":{"prompt":"A plush rabbit lying flat on its back.","dims_m":[0.20,0.10,0.28]}}],
            "layout":[{"item":"cot","place":"center"},{"item":"cloth","rel":{"how":"on","to":"cot","spot":"front"}},
                {"item":"toy","rel":{"how":"on","to":"cot","spot":"back"}}]
        })).unwrap();
        let sibling = build_scene(&plan, &models).0;
        let partial = build_scene(&plan, &[models[0].clone(), models[2].clone()]).0;
        let rel = plan.layout[2].rel.as_mut().unwrap();
        rel.to = "cloth".into();
        rel.how = "under".into();
        let nested = build_scene(&plan, &models).0;
        assert_eq!(
            obj(&sibling, "toy"),
            obj(&nested, "toy"),
            "the visible head belongs at the blanket edge"
        );
        let rel = plan.layout[2].rel.as_mut().unwrap();
        rel.to = "cot".into();
        rel.how = "on".into();
        plan.items[1].description = "The blanket covers the pillow beside the plush rabbit.".into();
        let indirect = build_scene(&plan, &models).0;
        assert_ne!(obj(&indirect, "toy"), obj(&nested, "toy"));
        let mut ambiguous = plan.clone();
        ambiguous.items[1].description = "The blanket covers the plush rabbit.".into();
        let mut other = ambiguous.items[1].clone();
        other.id = "second".into();
        ambiguous.items.push(other);
        let mut location = ambiguous.layout[1].clone();
        location.item = "second".into();
        ambiguous.layout.push(location);
        assert!(tucked_covers(&ambiguous, &models).is_empty());
        plan.items[1].description = "The blanket covers the plush rabbit.".into();
        plan.items[2].description = "A rabbit resting on top of the blanket.".into();
        let resting = build_scene(&plan, &models).0;
        assert_ne!(obj(&resting, "toy"), obj(&nested, "toy"));
        assert_eq!(
            partial,
            build_scene(&plan, &[models[0].clone(), models[2].clone()]).0
        );
    }

    #[test]
    fn a_tucked_plush_rests_its_head_on_the_pillow_with_its_feet_below_the_cover() {
        let mut mattress = quad(-0.5, 0.5, -0.95, 0.95, 0.5, true).to_vec();
        mattress.extend(quad(-0.5, 0.5, -0.95, 0.95, 0., false));
        mattress.extend(quad(-0.33, 0.33, -0.9, -0.62, 0.65, true));
        let mut cover = quad(-0.45, 0.45, -0.69, 0.69, 0.04, true).to_vec();
        cover.extend(quad(-0.45, 0.45, -0.69, 0.69, 0., false));
        // The asset's broad face arrived upright. The emitted transform must
        // both lay it down and raise its head edge over the tucked body.
        let cover: Vec<Tri> = cover
            .iter()
            .map(|t| t.map(|[x, y, z]| [x, -z, y]))
            .collect();
        let mut rabbit = quad(-0.04, 0.04, -0.03, 0.05, 0.28, true).to_vec();
        rabbit.extend(quad(-0.04, 0.04, -0.03, 0.05, 0.20, false));
        rabbit.extend(quad(-0.08, 0.08, -0.05, 0.065, 0.08, true));
        rabbit.extend(quad(-0.08, 0.08, -0.05, 0.065, 0.0, false));
        let mut models = [
            meshed("cot", &mattress),
            meshed("throw", &cover),
            meshed("plush", &rabbit),
        ];
        let mut plan: Plan = serde_json::from_value(json!({
            "items":[{"kind":"object","id":"cot","name":"Cot","description":"A cot with a pillow.","details":{"dims_m":[1.0,0.65,1.90]}},
                     {"kind":"object","id":"throw","name":"Blanket","description":"A thin cotton blanket.","details":{"dims_m":[0.9,0.04,1.38]}},
                     {"kind":"object","id":"plush","name":"Rabbit","description":"The rabbit is tucked in on its back.","details":{"dims_m":[0.16,0.10,0.28]}}],
            "layout":[{"item":"cot","place":"center"},{"item":"throw","rel":{"how":"on","to":"cot","spot":"center"}},
                      {"item":"plush","rel":{"how":"under","to":"throw","spot":"back"}}]
        })).unwrap();
        let check = |plan: &Plan, models: &[Model], pillow_z: [f64; 2]| {
            let doc = build_scene(plan, models).0;
            let at = |id, p: [f64; 3]| {
                let o = obj(&doc, id);
                let scale = nums::<3>(&o["scale"]).unwrap();
                let p = turned(
                    turn_matrix(nums::<3>(&o["rotation"]).unwrap()),
                    [0, 1, 2].map(|k| p[k] * scale[k]),
                );
                let offset = nums::<3>(&o["position"]).unwrap();
                [0, 1, 2].map(|k| p[k] + offset[k])
            };
            let head = at("plush", [0., 0.25, 0.05]);
            let feet = at("plush", [0., 0.04, 0.06]);
            let pillow = at("cot", [0., 0.65, -0.75]);
            assert!(
                head[1] > pillow[1] + 0.01,
                "face {head:?} inside pillow {pillow:?}"
            );
            let limits = pillow_z.map(|z| at("cot", [0., 0.65, z])[2]);
            assert!(
                (limits[0]..=limits[1]).contains(&head[2]),
                "head at {} misses the pillow at {limits:?}",
                head[2]
            );
            let actual: Vec<Tri> = cover.iter().map(|t| t.map(|v| at("throw", v))).collect();
            let bounds = Bounds::from_points(actual.iter().flatten().copied()).unwrap();
            let surface = Surface::of(&actual, bounds.min, bounds.max).unwrap();
            let top = surface.top_at(feet[0], feet[2]).unwrap();
            assert!(
                feet[1] <= top + 0.012,
                "feet {feet:?} above the blanket at {top}"
            );
            assert!(
                surface.top_at(head[0], head[2]).is_none(),
                "the blanket should end at the neck"
            );
        };
        check(&plan, &models, [-0.9, -0.62]);
        plan.items[2].description = "A plush rabbit is tucked in with its head showing.".into();
        plan.items[2].details.dims_m = None;
        plan.items[2].details.size_m = Some(0.28);
        check(&plan, &models, [-0.9, -0.62]);
        // A longer cover can end past the pillow, above bare mattress.
        mattress.truncate(4);
        mattress.extend(quad(-0.33, 0.33, -0.70, -0.46, 0.65, true));
        models[0] = meshed("cot", &mattress);
        check(&plan, &models, [-0.70, -0.46]);
    }

    #[test]
    fn a_quilt_drawn_at_an_angle_lies_flat_on_the_bed() {
        let mut bed = quad(-0.5, 0.5, -1.0, 1.0, 0.55, true).to_vec();
        bed.extend(quad(-0.5, 0.5, -1.0, 1.0, 0.0, false));
        let (sn, cs) = 63.0f64.to_radians().sin_cos();
        let cloth: Vec<_> = quad(-0.45, 0.45, -0.7, 0.7, 0.02, true)
            .iter()
            .chain(quad(-0.45, 0.45, -0.7, 0.7, 0.0, false).iter())
            .map(|tri| tri.map(|[x, y, z]| [x, y * cs - z * sn, y * sn + z * cs]))
            .collect();
        let models = [meshed("bed", &bed), meshed("quilt", &cloth)];
        let mut quilt = item("quilt", "object");
        quilt.details.dims_m = Some([0.90, 0.03, 1.4]);
        let mut loc = place("quilt", "");
        loc.rel = Some(serde_json::from_value(json!({"to":"bed","how":"on"})).unwrap());
        let plan = Plan {
            items: vec![item("bed", "object"), quilt],
            layout: vec![place("bed", "center"), loc],
            ..Default::default()
        };
        let (doc, _) = build_scene(&plan, &models);
        let bed = world_box(obj(&doc, "bed"), &models);
        let obj = obj(&doc, "quilt");
        let scale = nums::<3>(&obj["scale"]).unwrap();
        let shift = nums::<3>(&obj["position"]).unwrap();
        let rot = turn_matrix(nums::<3>(&obj["rotation"]).unwrap());
        let ys: Vec<_> = models[1]
            .tris
            .iter()
            .flatten()
            .map(|v| turned(rot, [0, 1, 2].map(|k| v[k] * scale[k]))[1] + shift[1])
            .collect();
        let lo = ys.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (lo - bed.max[1]).abs() < 0.02 && hi - lo < 0.08,
            "cloth {lo}..{hi} versus bed {bed:?}"
        );
    }

    #[test]
    fn a_draped_tablecloth_spreads_to_its_requested_size_on_the_table() {
        let mut table = quad(-0.52, 0.52, -0.8, 0.8, 0.72, true).to_vec();
        table.extend(quad(-0.52, 0.52, -0.8, 0.8, 0., false));
        // A cloth doubled around a form, with fabric on the front and back
        // and an open middle when seen from above.
        let cloth: Vec<Tri> = [(-0.66, false), (0.66, true)]
            .into_iter()
            .flat_map(|(depth, front)| quad(-0.7, 0.7, -0.65, 0.65, depth, front))
            .map(|tri| tri.map(|[x, y, z]| [x, -z, y]))
            .collect();
        let models = [meshed("table", &table), meshed("cloth", &cloth)];
        let plan: Plan = serde_json::from_value(json!({
            "items":[{"kind":"object","id":"table","name":"Dining table","description":"A dining table.","details":{"dims_m":[1.04,0.72,1.60]}},
                     {"kind":"object","id":"cloth","name":"Striped tablecloth","description":"A cotton cloth covers the tabletop.","details":{"dims_m":[0.96,0.05,1.40]}}],
            "layout":[{"item":"table","place":"center"},{"item":"cloth","rel":{"how":"on","to":"table","spot":"center"}}]
        })).unwrap();
        let doc = build_scene(&plan, &models).0;
        let table = world_box(obj(&doc, "table"), &models);
        let o = obj(&doc, "cloth");
        let turn = turn_matrix(nums::<3>(&o["rotation"]).unwrap());
        let scale = nums::<3>(&o["scale"]).unwrap();
        let shift = nums::<3>(&o["position"]).unwrap();
        let actual: Vec<Tri> = models[1]
            .tris
            .iter()
            .map(|t| {
                t.map(|p| {
                    let p = turned(turn, [0, 1, 2].map(|k| p[k] * scale[k]));
                    [0, 1, 2].map(|k| p[k] + shift[k])
                })
            })
            .collect();
        let cloth = Bounds::from_points(actual.iter().flatten().copied()).unwrap();
        assert!(
            (cloth.size(1) - 0.05).abs() < 0.005,
            "fold height {cloth:?}"
        );
        assert!((cloth.size(0) - 0.96).abs() < 0.005 && (cloth.size(2) - 1.40).abs() < 0.005);
        assert!((cloth.min[1] - table.max[1]).abs() < 0.005);
        let middle = |k| (table.min[k] + table.max[k]) * 0.5;
        let surface = Surface::of(&actual, cloth.min, cloth.max);
        assert!(
            surface
                .and_then(|s| s.top_at(middle(0), middle(2)))
                .is_some(),
            "the fabric itself must span the centre of the table"
        );
    }

    #[test]
    fn unchanged_pose_meshes_keep_their_contact_surfaces() {
        let shape = quad(-0.3, 0.3, -0.4, 0.4, 0.0, false);
        let piece = meshed("figure", &shape);
        let legacy = json!({"spec":{"kind":"stand"},"manifest":{
            "bounds":{"min":piece.bounds.min,"max":piece.bounds.max},"contact":{"feet":"none"}
        }});
        for (value, body, keep) in [
            (None, Some("no_legs"), true),
            (None, Some("two_legs"), false),
            (Some(true), Some("two_legs"), true),
            (Some(false), Some("no_legs"), false),
        ] {
            let mut report = legacy.clone();
            if let Some(same) = value {
                report["manifest"]["rest_mesh_unchanged"] = json!(same);
            }
            let mut model = meshed("figure", &shape);
            model.apply_pose(Posed::of(&report["manifest"]).unwrap(), &report, body);
            assert_eq!(!model.tris.is_empty(), keep, "{report}");
        }
    }

    #[test]
    fn a_character_can_stand_on_a_platform_narrower_than_its_arms() {
        let mut figure = item("figure", "character");
        figure.details.body = Some("no_legs".into());
        figure.details.pose =
            Some(serde_json::from_value(json!({"kind":"sit","on":"deck"})).unwrap());
        let mut placement = place("figure", "");
        placement.rel = Some(serde_json::from_value(json!({"to":"deck","how":"on"})).unwrap());
        let plan = Plan {
            items: vec![figure, item("deck", "object")],
            layout: vec![place("deck", "center"), placement],
            ..Default::default()
        };
        let mut deck = quad(-0.3, 0.3, -0.5, 0.5, 0.24, true).to_vec();
        deck.extend(quad(-0.3, 0.3, -0.5, 0.5, 0.0, false));
        let mut figure = quad(-0.14, 0.14, -0.12, 0.12, 0.0, false).to_vec();
        figure.extend(quad(-0.8, 0.8, -0.15, 0.15, 1.2, true));
        let models = [meshed("deck", &deck), meshed("figure", &figure)];
        let (doc, _) = build_scene(&plan, &models);
        let support = world_box(obj(&doc, "deck"), &models);
        let actor = world_box(obj(&doc, "figure"), &models);
        assert!(
            (actor.min[1] - support.max[1]).abs() < 0.02,
            "actor {actor:?}, deck {support:?}"
        );
    }

    #[test]
    fn a_bird_sitting_on_a_high_perch_keeps_its_feet_on_top() {
        let mut bird = item("bird", "character");
        bird.details.body = Some("bird".into());
        bird.details.pose =
            Some(serde_json::from_value(json!({"kind":"sit","on":"cactus"})).unwrap());
        let mut perch = place("bird", "");
        perch.rel = Some(
            serde_json::from_value(json!({"to":"cactus","how":"on","spot":"center"})).unwrap(),
        );
        let plan = Plan {
            items: vec![item("cactus", "object"), bird],
            layout: vec![place("cactus", "center"), perch],
            ..Default::default()
        };
        let fixed = |id, min, max| Model {
            real_size: true,
            ..model(id, Bounds { min, max })
        };
        let models = [
            fixed("cactus", [-0.45, 0.0, -0.45], [0.45, 4.2, 0.45]),
            fixed("bird", [-0.18, 0.0, -0.12], [0.18, 0.7, 0.12]),
        ];
        let (doc, _) = build_scene(&plan, &models);
        let bird = world_box(obj(&doc, "bird"), &models);
        let plant = world_box(obj(&doc, "cactus"), &models);
        assert!(
            (bird.min[1] - plant.max[1]).abs() < 0.01,
            "{bird:?} on {plant:?}"
        );
        assert!(super::super::pose::effective(&plan, &plan.items[1]).is_none_or(|p| p.on.is_none()));
    }

    #[test]
    fn a_bird_on_top_of_a_tall_plant_uses_its_crown() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[{"id":"plant","kind":"object","name":"Plant","description":"One tall plant."},
                     {"id":"bird","kind":"character","name":"Bird",
                      "description":"A bird is perched on top of the tall plant.",
                      "details":{"body":"bird"}}],
            "layout":[{"item":"plant","place":"center"},
                      {"item":"bird","rel":{"how":"on","to":"plant","spot":"center"}}]
        }))
        .unwrap();
        let mut surface = quad(-0.45, 0.45, -0.12, 0.12, 2., true).to_vec();
        surface.extend(quad(-0.45, -0.15, -0.12, 0.12, 3., true));
        surface.extend(quad(-0.45, 0.45, -0.12, 0.12, 0., false));
        let models = [
            Model {
                real_size: true,
                ..meshed("plant", &surface)
            },
            Model {
                real_size: true,
                ..model(
                    "bird",
                    Bounds {
                        min: [-0.07, 0., -0.05],
                        max: [0.07, 0.25, 0.05],
                    },
                )
            },
        ];
        let (doc, _) = build_scene(&plan, &models);
        let bird = world_box(obj(&doc, "bird"), &models);
        let plant = world_box(obj(&doc, "plant"), &models);
        assert!(
            (bird.min[1] - plant.max[1]).abs() < 0.02,
            "{bird:?} on {plant:?}"
        );

        // A long tail moves the body centre away from the small pair of feet.
        let mut wings = quad(-0.35, 0.35, -0.24, 0.24, 0.24, true).to_vec();
        wings.extend(quad(0.19, 0.23, -0.02, 0.02, 0., false));
        let models = [
            models[0].clone(),
            Model {
                real_size: true,
                ..meshed("bird", &wings)
            },
        ];
        let (doc, _) = build_scene(&plan, &models);
        let bird = world_box(obj(&doc, "bird"), &models);
        assert!(
            (bird.min[1] - plant.max[1]).abs() < 0.02,
            "off-centre feet landed below the crown: {bird:?} on {plant:?}"
        );
    }

    #[test]
    fn a_perch_uses_the_branch_under_the_bird_below_its_highest_twigs() {
        let mut tris = quad(-1.0, 1.0, -0.1, 0.1, 0.2, true).to_vec();
        tris.extend(quad(0.8, 1.0, -0.1, 0.1, 1.0, true));
        let b = Bounds {
            min: [-1.0, 0.0, -0.1],
            max: [1.0, 1.0, 0.1],
        };
        let branch = Spot {
            center: [0.0; 2],
            floor: 0.0,
            rot_y: 0.0,
            scale: [1.0; 3],
            b,
            tilt: None,
        };
        let bird = Spot {
            b: Bounds {
                min: [-0.05, 0.0, -0.05],
                max: [0.05, 0.1, 0.05],
            },
            ..branch
        };
        let surface = Surface::of(&tris, b.min, b.max).unwrap();
        let perched = uneven(Some(&surface), &branch, bird, &[]).unwrap();
        assert!(
            (perched.floor - 0.2).abs() < 0.001,
            "perched at {}",
            perched.floor
        );
    }

    /// A robin on a worn hat (the snowman): it goes up with the hat to the
    /// head, never left where the hat stood before it was put on.
    #[test]
    fn a_piece_on_a_worn_piece_goes_with_it() {
        let mut plan = Plan {
            title: "Snowman".into(),
            items: vec![
                item("snowman", "character"),
                item("hat", "object"),
                item("robin", "object"),
            ],
            ..Default::default()
        };
        let rel = |id: &str, r: Value| {
            let mut p = place(id, "");
            p.rel = Some(serde_json::from_value(r).unwrap());
            p
        };
        plan.layout = vec![
            place("snowman", "center"),
            rel(
                "hat",
                json!({"to": "snowman", "how": "worn", "part": "head"}),
            ),
            rel("robin", json!({"to": "hat", "how": "on"})),
        ];
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let mut models = vec![
            model("snowman", b([-0.3, 0.0, -0.3], [0.3, 1.6, 0.3])),
            model("hat", b([-0.15, 0.0, -0.15], [0.15, 0.2, 0.15])),
            model("robin", b([-0.05, 0.0, -0.05], [0.05, 0.1, 0.05])),
        ];
        for m in &mut models {
            m.real_size = true;
        }
        // A skeleton makes the hat fit the head. Its new height is the
        // surface the robin perches on.
        models[0].joints.insert("neck".into(), [0.0, 1.12, 0.0]);
        models[0].joints.insert("head".into(), [0.0, 1.24, 0.0]);
        models[0].pose = Some(Posed {
            min: [-0.3, 0.0, -0.3],
            max: [0.3, 1.6, 0.3],
            seat: None,
            palm: None,
            hand_left: false,
            back: None,
            wind: None,
            shaft: None,
            other_grip: None,
            head_bounds: None,
            mouth: None,
            head: Some([0.0, 1.24, 0.0]),
            grip: None,
        });
        let neck = body_point(&models[0], "neck").unwrap();
        assert!(
            (neck[1] - 1.12).abs() < 0.001,
            "unchanged head moved neck: {neck:?}"
        );
        let (doc, notes) = build_scene(&plan, &models);
        let w = |id| world_box(obj(&doc, id), &models);
        let (hat, robin) = (w("hat"), w("robin"));
        assert!(hat.min[1] > 1.2, "the hat is on the head: {hat:?}");
        assert!(
            (robin.min[1] - hat.max[1]).abs() < 0.03,
            "{robin:?} on {hat:?}: {notes:?}"
        );
        assert!((robin.mid(0) - hat.mid(0)).abs() < 0.2 && (robin.mid(2) - hat.mid(2)).abs() < 0.2);

        // On a generated stick-armed figure the neck and its tiny patch of
        // "head" skin can both land on a twig. The actual crown is off-centre.
        let figure = &mut models[0];
        figure.tris = quad(0.04, 0.20, -0.08, 0.08, 1.6, true).to_vec();
        figure.joints.insert("neck".into(), [-0.27, 0.9, 0.]);
        figure.joints.insert("head".into(), [-0.27, 0.95, 0.]);
        let pose = figure.pose.as_mut().unwrap();
        pose.head = Some([-0.27, 0.95, 0.]);
        pose.head_bounds = Some(Bounds {
            min: [-0.28, 0.95, 0.],
            max: [-0.27, 0.96, 0.01],
        });
        let (doc, _) = build_scene(&plan, &models);
        let hat = world_box(obj(&doc, "hat"), &models);
        let robin = world_box(obj(&doc, "robin"), &models);
        let figure = world_box(obj(&doc, "snowman"), &models);
        assert!((hat.mid(0) - figure.mid(0) - 0.12).abs() < 0.025);
        assert!((hat.min[1] - figure.max[1]).abs() < 0.05);
        assert!((robin.min[1] - hat.max[1]).abs() < 0.03);
    }

    /// More on a small stage than it holds (the animal band): one with no
    /// room stands on the floor beside it, said plainly, never in the air at
    /// the stage's height.
    /// Posed (`forge pose`): a child sits with the point its body rests on at
    /// the log's top middle, and a giant holds his palm out with the mouse
    /// standing on it (no word of missing poses).
    #[test]
    fn a_posed_character_sits_on_its_seat_and_a_palm_holds_what_it_holds() {
        let at = |doc: &Value, id: &str, v: [f64; 3]| {
            let o = obj(doc, id);
            let (p, yaw) = (
                nums::<3>(&o["position"]).unwrap(),
                o["rotation"][1].as_f64().unwrap().to_radians(),
            );
            let s = o["scale"][1].as_f64().unwrap();
            let (x, z) = (v[0] * s, v[2] * s);
            [
                p[0] + x * yaw.cos() + z * yaw.sin(),
                p[1] + v[1] * s,
                p[2] - x * yaw.sin() + z * yaw.cos(),
            ]
        };
        let b = |lo: [f64; 3], hi: [f64; 3]| Bounds { min: lo, max: hi };
        let rel = |to: &str, how: &str| {
            serde_json::from_value(json!({"to": to, "how": how, "hand": "right_hand"})).unwrap()
        };
        // A child seated on a log.
        let mut plan = Plan {
            title: "Camp".into(),
            items: vec![item("log", "object"), item("kid", "character")],
            ..Default::default()
        };
        plan.items[0].name = "Log seat".into();
        let mut kid = place("kid", "");
        kid.rel = Some(rel("log", "on"));
        plan.layout = vec![place("log", "center"), kid];
        let mut kid = model("kid", b([-0.2, 0.0, -0.15], [0.2, 1.2, 0.15]));
        kid.pose = Some(Posed {
            min: [-0.2, 0.0, -0.3],
            max: [0.2, 0.8, 0.3],
            seat: Some([0.0, 0.45, -0.1]),
            palm: None,
            hand_left: false,
            back: None,
            wind: None,
            shaft: None,
            other_grip: None,
            head_bounds: None,
            mouth: None,
            head: None,
            grip: None,
        });
        let mut models = vec![model("log", b([-0.6, 0.0, -0.2], [0.6, 0.4, 0.2])), kid];
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, notes) = build_scene(&plan, &models);
        let (seat, log) = (
            at(&doc, "kid", [0.0, 0.45, -0.1]),
            world_box(obj(&doc, "log"), &models),
        );
        for (k, want) in [(0, log.mid(0)), (1, log.max[1]), (2, log.mid(2))] {
            assert!((seat[k] - want).abs() < 1e-3, "seated: {seat:?} on {log:?}");
        }
        assert!(
            !notes.iter().any(|n| n.contains("in front of")),
            "{notes:?}"
        );
        // A raised runner or chair back can extend above the supporting seat.
        // The posed rider touches the slats beneath it.
        let mut slats = quad(-0.7, 0.7, -0.3, 0.3, 0.25, true).to_vec();
        slats.extend(quad(-0.7, 0.7, -0.3, 0.3, 0.0, false));
        slats.extend(quad(0.6, 0.7, -0.3, 0.3, 0.55, true));
        models[0] = meshed("log", &slats);
        let (doc, _) = build_scene(&plan, &models);
        let contact = at(&doc, "kid", [0.0, 0.45, -0.1]);
        assert!(
            (contact[1] - 0.25).abs() < 0.001,
            "seat contact {contact:?}"
        );
        // A giant holding his palm out, the mouse on it.
        let mut plan = Plan {
            title: "Giant".into(),
            items: vec![item("giant", "character"), item("mouse", "character")],
            ..Default::default()
        };
        let mut mouse = place("mouse", "");
        mouse.rel = Some(rel("giant", "held"));
        plan.layout = vec![place("giant", "center"), mouse];
        let mut giant = model("giant", b([-1.2, 0.0, -0.6], [1.2, 8.0, 0.6]));
        giant.joints.insert("right_hand".into(), [-1.3, 3.6, 0.3]);
        giant.pose = Some(Posed {
            min: [-1.2, 0.0, -0.6],
            max: [1.2, 8.0, 1.8],
            seat: None,
            palm: Some([-0.6, 4.5, 1.5]),
            hand_left: false,
            back: None,
            wind: None,
            shaft: None,
            other_grip: None,
            head_bounds: None,
            mouth: None,
            head: None,
            grip: None,
        });
        let mut models = vec![
            giant,
            model("mouse", b([-0.05, 0.0, -0.03], [0.05, 0.1, 0.03])),
        ];
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, notes) = build_scene(&plan, &models);
        let (palm, m) = (
            at(&doc, "giant", [-0.6, 4.5, 1.5]),
            world_box(obj(&doc, "mouse"), &models),
        );
        assert!(
            (m.min[1] - palm[1]).abs() < 1e-3,
            "on the palm: {m:?} {palm:?}"
        );
        assert!(
            (m.mid(0) - palm[0]).abs() < 1e-3 && (m.mid(2) - palm[2]).abs() < 1e-3,
            "{m:?} {palm:?}"
        );
        assert!(
            !notes.iter().any(|n| n.contains("needs poses")),
            "{notes:?}"
        );
    }

    /// A mouse on a giant's palm (the real giant and the mouse): the first view
    /// frames the mouse and his head, never his feet, from outside him, and the
    /// viewer is told to frame that box instead of his whole self.
    #[test]
    fn a_tiny_one_on_a_big_ones_palm_is_framed_with_his_head() {
        let mut plan = Plan {
            title: "Giant".into(),
            items: vec![item("giant", "character"), item("mouse", "character")],
            ..Default::default()
        };
        let mut held = place("mouse", "");
        held.rel = Some(
            serde_json::from_value(json!({"to": "giant", "how": "held", "hand": "right_hand"}))
                .unwrap(),
        );
        plan.layout = vec![place("giant", "center"), held];
        // A body to his shoulders (6.4 m), a head on top: closed boxes.
        let cube = |lo: [f64; 3], hi: [f64; 3]| {
            let c = |k: usize| [0, 1, 2].map(|a| if k >> a & 1 == 1 { hi[a] } else { lo[a] });
            let faces = [
                [0, 2, 3, 1],
                [4, 5, 7, 6],
                [0, 1, 5, 4],
                [2, 6, 7, 3],
                [0, 4, 6, 2],
                [1, 3, 7, 5],
            ];
            faces
                .map(|f| [[c(f[0]), c(f[1]), c(f[2])], [c(f[0]), c(f[2]), c(f[3])]])
                .concat()
        };
        let mut tris = cube([-1.2, 0.0, -0.6], [1.2, 6.4, 0.6]);
        tris.extend(cube([-0.4, 6.4, -0.4], [0.4, 8.0, 0.4]));
        let mut giant = meshed("giant", &tris);
        giant.joints.insert("right_hand".into(), [-1.3, 3.6, 0.3]);
        let mouse = model(
            "mouse",
            Bounds {
                min: [-0.05, 0.0, -0.03],
                max: [0.05, 0.1, 0.03],
            },
        );
        let mut models = vec![giant, mouse];
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, notes) = build_scene(&plan, &models);
        let g = obj(&doc, "giant");
        assert_eq!(g["backdrop"], true, "{g}");
        let frame = [
            nums::<3>(&g["frame"][0]).unwrap(),
            nums::<3>(&g["frame"][1]).unwrap(),
        ];
        let (m, gb) = (
            world_box(obj(&doc, "mouse"), &models),
            world_box(g, &models),
        );
        assert!(
            frame[0][1] <= m.min[1] + 1e-3 && frame[1][1] >= gb.max[1] - 1e-3,
            "{frame:?}"
        );
        assert!(frame[0][1] > 0.5, "not his feet: {frame:?}");
        let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
        assert!(
            (0..3).any(|k| cam[k] < gb.min[k] || cam[k] > gb.max[k]),
            "outside him: {cam:?}"
        );
        // At his hand, as a lantern is (no pose holds a palm out, said).
        assert!((m.mid(1) - 3.6).abs() < 1e-3, "at his hand: {m:?}");
        assert!(notes.iter().any(|n| n.contains("needs poses")), "{notes:?}");
    }

    #[test]
    fn holding_a_small_prop_leaves_the_whole_group_in_view() {
        let mut ball = place("ball", "");
        ball.rel = Some(
            serde_json::from_value(json!({"to":"child", "how":"held", "hand":"right_hand"}))
                .unwrap(),
        );
        let b = |min, max| Bounds { min, max };
        let mut plan = Plan {
            items: vec![item("child", "character"), item("ball", "object")],
            layout: vec![place("child", "left"), ball],
            ..Default::default()
        };
        let mut child = model("child", b([-0.25, 0.0, -0.15], [0.25, 1.5, 0.15]));
        child.joints.insert("right_hand".into(), [0.3, 1.0, 0.1]);
        let ball = model("ball", b([-0.04, 0.0, -0.04], [0.04, 0.08, 0.04]));
        let mut models = vec![child, ball];
        for m in &mut models {
            m.real_size = true;
        }
        let (solo, _) = build_scene(&plan, &models);
        assert!(obj(&solo, "child").get("frame").is_some());
        plan.items.push(item("snowman", "character"));
        plan.layout.push(place("snowman", "right"));
        let mut snowman = model("snowman", b([-0.4, 0.0, -0.3], [0.4, 1.2, 0.3]));
        snowman.real_size = true;
        models.push(snowman);
        let (group, _) = build_scene(&plan, &models);
        assert!(obj(&group, "child").get("frame").is_none());
    }

    /// A child "on" a log seat round a fire, with no sitting pose: it stands
    /// just in front of the log, between it and the fire, on the ground.
    #[test]
    fn a_character_on_a_seat_stands_in_front_of_it() {
        let ids = ["campfire", "log1", "child1"];
        let mut plan = Plan {
            title: "Camp".into(),
            items: ids
                .map(|i| item(i, if i == "child1" { "character" } else { "object" }))
                .to_vec(),
            ..Default::default()
        };
        plan.items[1].name = "Log seat 1".into();
        let rel =
            |to: &str, how: &str| serde_json::from_value(json!({"to": to, "how": how})).unwrap();
        let mut log = place("log1", "");
        log.rel = Some(rel("campfire", "around"));
        let mut child = place("child1", "");
        (child.rel, child.facing) = (Some(rel("log1", "on")), Some("campfire".into()));
        plan.layout = vec![place("campfire", "center"), log, child];
        let b = |lo: [f64; 3], hi: [f64; 3]| Bounds { min: lo, max: hi };
        let mut models = vec![
            model("campfire", b([-0.45, 0.0, -0.45], [0.45, 0.8, 0.45])),
            model("log1", b([-0.6, 0.0, -0.2], [0.6, 0.4, 0.2])),
            model("child1", b([-0.2, 0.0, -0.15], [0.2, 1.2, 0.15])),
        ];
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, notes) = build_scene(&plan, &models);
        let w = |id| world_box(obj(&doc, id), &models);
        let (fire, seat, kid) = (w("campfire"), w("log1"), w("child1"));
        assert!(kid.min[1].abs() < 1e-3, "on the ground: {kid:?}");
        let apart = (0..3)
            .step_by(2)
            .any(|k| kid.min[k] >= seat.max[k] - 1e-3 || kid.max[k] <= seat.min[k] + 1e-3);
        assert!(apart, "not on the log: {kid:?} {seat:?}");
        let to_fire = |x: &Bounds| (x.mid(0) - fire.mid(0)).hypot(x.mid(2) - fire.mid(2));
        assert!(
            to_fire(&kid) < to_fire(&seat),
            "between the log and the fire"
        );
        assert!(
            notes.iter().any(|n| n.contains("stands in front of")),
            "{notes:?}"
        );
    }

    /// Three children round a campfire (the real campfire's ring): the first
    /// view shows the fire, unhidden, through a gap between them, and a face.
    #[test]
    fn the_first_view_shows_what_the_cast_gathers_round() {
        let ids = ["campfire", "child1", "child2", "child3"];
        let mut plan = Plan {
            title: "Camp".into(),
            items: ids
                .map(|i| {
                    item(
                        i,
                        if i == "campfire" {
                            "object"
                        } else {
                            "character"
                        },
                    )
                })
                .to_vec(),
            ..Default::default()
        };
        let round = |id: &str| {
            let mut p = place(id, "");
            p.facing = Some("campfire".into());
            p.rel =
                Some(serde_json::from_value(json!({"to": "campfire", "how": "around"})).unwrap());
            p
        };
        plan.layout = vec![
            place("campfire", "center"),
            round("child1"),
            round("child2"),
            round("child3"),
        ];
        let mut models = vec![model(
            "campfire",
            Bounds {
                min: [-0.45, 0.0, -0.45],
                max: [0.45, 0.8, 0.45],
            },
        )];
        for id in &ids[1..] {
            models.push(model(
                id,
                Bounds {
                    min: [-0.2, 0.0, -0.15],
                    max: [0.2, 1.2, 0.15],
                },
            ));
        }
        for m in models.iter_mut() {
            m.real_size = true;
        }
        let (doc, _) = build_scene(&plan, &models);
        let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
        let fire = world_box(obj(&doc, "campfire"), &models);
        let (fx, fz) = (fire.mid(0) - cam[0], fire.mid(2) - cam[2]);
        let n = fx.hypot(fz);
        for (s, y) in [(-0.3, 0.4), (0.0, 0.2), (0.0, 0.4), (0.0, 0.6), (0.3, 0.4)] {
            let to = [
                fire.mid(0) - fz / n * s - cam[0],
                y - cam[1],
                fire.mid(2) + fx / n * s - cam[2],
            ];
            for id in &ids[1..] {
                assert!(!blocks(&doc, &models, id, cam, to), "{id} hides the fire");
            }
        }
        let faces = ids[1..].iter().filter(|id| {
            let o = obj(&doc, id);
            let (p, yaw) = (
                nums::<3>(&o["position"]).unwrap(),
                o["rotation"][1].as_f64().unwrap(),
            );
            let look = (cam[0] - p[0]).atan2(cam[2] - p[2]).to_degrees();
            ((look - yaw + 180.0).rem_euclid(360.0) - 180.0).abs() < 90.0
        });
        assert!(faces.count() >= 1, "a face in the first view");
    }

    #[test]
    fn small_things_on_a_long_top_stay_within_reach_of_each_other() {
        let ids = ["table", "cup1", "cup2"];
        let mut plan = Plan {
            title: "Kitchen".into(),
            items: ids.map(|i| item(i, "object")).to_vec(),
            ..Default::default()
        };
        let on = |id: &str, spot: &str| {
            let mut p = place(id, "");
            p.rel = Some(
                serde_json::from_value(json!({"to": "table", "how": "on", "spot": spot})).unwrap(),
            );
            p
        };
        plan.layout = vec![
            place("table", "center"),
            on("cup1", "left"),
            on("cup2", "right"),
        ];
        let mut table = model(
            "table",
            Bounds {
                min: [-1.5, 0.0, -0.4],
                max: [1.5, 0.75, 0.4],
            },
        );
        table.real_size = true;
        let mut models = vec![table];
        for id in &ids[1..] {
            let mut m = model(
                id,
                Bounds {
                    min: [-0.05, 0.0, -0.05],
                    max: [0.05, 0.1, 0.05],
                },
            );
            m.real_size = true;
            models.push(m);
        }
        let (doc, _) = build_scene(&plan, &models);
        let (a, b) = (
            world_box(obj(&doc, "cup1"), &models),
            world_box(obj(&doc, "cup2"), &models),
        );
        let apart = (a.mid(0) - b.mid(0)).hypot(a.mid(2) - b.mid(2));
        assert!(
            apart > 0.1 && apart <= 2.0 * ARM,
            "the cups {apart} m apart"
        );
    }

    #[test]
    fn a_floor_covering_near_a_named_object_brings_its_contents_along() {
        let mut plan: Plan = serde_json::from_value(json!({
            "items":[{"id":"bench","kind":"object","name":"Bench","description":""},
                     {"id":"mat","kind":"object","name":"Benchside mat","description":"A woven mat lies near the bench.",
                      "details":{"dims_m":[1.3,0.03,0.8]}},
                     {"id":"toy","kind":"object","name":"Toy","description":""}],
            "layout":[{"item":"bench","place":"center-left"},{"item":"mat","place":"front-right"},
                      {"item":"toy","rel":{"to":"mat","how":"on","spot":"center"}}]
        })).unwrap();
        let models = [
            ("bench", [-0.8, 0., -0.4], [0.8, 0.7, 0.4]),
            ("mat", [-0.65, 0., -0.4], [0.65, 0.03, 0.4]),
            ("toy", [-0.1, 0., -0.05], [0.1, 0.12, 0.05]),
        ]
        .map(|(id, min, max)| Model {
            real_size: true,
            ..model(id, Bounds { min, max })
        });
        let named = build_scene(&plan, &models).0;
        plan.items[1].name = "Mat".into();
        let described = build_scene(&plan, &models).0;
        for id in ["mat", "toy"] {
            assert_eq!(obj(&named, id)["position"], obj(&described, id)["position"]);
        }
    }

    #[test]
    fn a_library_table_keeps_its_height_and_fits_a_bedside_footprint() {
        let mut table = item("table", "object");
        table.name = "Bedside table".into();
        table.details.size_m = Some(0.55);
        table.details.dims_m = Some([0.5, 0.55, 0.45]);
        let plan = Plan {
            items: vec![table],
            ..Default::default()
        };
        let m = Model {
            real_size: true,
            ..model(
                "table",
                Bounds {
                    min: [0.0; 3],
                    max: [2.85, 0.815, 1.10],
                },
            )
        };
        let (s, _) = scale_of(&plan, &m);
        assert!((s[1] * 0.815 - 0.55).abs() < 1e-6);
        assert!(
            s[0] * 2.85 <= 0.8 && s[2] * 1.10 <= 0.5,
            "table scale {s:?}"
        );
    }

    #[test]
    fn furniture_stands_at_its_height_whatever_shape_it_came_out() {
        // A long low table model the plan's round 1.2 m table does not fit.
        let mut plan = Plan {
            title: "Kitchen".into(),
            items: vec![item("table", "object")],
            ..Default::default()
        };
        plan.items[0].name = "Round wooden table".into();
        plan.items[0].details.dims_m = Some([1.2, 0.75, 1.2]);
        let m = model(
            "table",
            Bounds {
                min: [-1.5, 0.0, -0.4],
                max: [1.5, 0.5, 0.4],
            },
        );
        let (s, kept) = scale_of(&plan, &m);
        assert!(kept, "it keeps its own shape");
        assert!((0.5 * s[1] - 0.75).abs() < 1e-9, "its top at 75 cm: {s:?}");
        // Without metres: an ordinary table's height.
        plan.items[0].details.dims_m = None;
        assert!((0.5 * scale_of(&plan, &m).0[1] - 0.75).abs() < 1e-9);
    }

    #[test]
    fn a_piece_with_no_room_on_a_surface_stands_beside_it_on_the_floor() {
        let mut tris: Vec<Tri> = quad(-0.75, 0.75, -0.75, 0.75, 0.5, true).to_vec();
        tris.extend(quad(-0.75, 0.75, -0.75, 0.75, 0.0, false));
        let ids = ["stage", "bear", "frog", "cat", "rabbit"];
        let mut plan = Plan {
            title: "Band".into(),
            items: ids.map(|i| item(i, "object")).to_vec(),
            ..Default::default()
        };
        let on = |id: &str, place_at: &str| {
            let mut p = place(id, place_at);
            p.rel = Some(serde_json::from_value(json!({"to": "stage", "how": "on"})).unwrap());
            p
        };
        plan.layout = vec![
            place("stage", "center"),
            on("bear", "back-left"),
            on("frog", "back-right"),
            on("cat", "front-left"),
            on("rabbit", "front-right"),
        ];
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let mut models = vec![meshed("stage", &tris)];
        for id in &ids[1..] {
            let mut m = model(id, b([-0.4, 0.0, -0.4], [0.4, 1.0, 0.4]));
            m.real_size = true;
            models.push(m);
        }
        let (doc, notes) = build_scene(&plan, &models);
        let w = |id| world_box(obj(&doc, id), &models);
        let stage = w("stage");
        let mut beside = 0;
        for id in &ids[1..] {
            let o = w(id);
            let over = o.max[0] > stage.min[0] + 1e-6
                && o.min[0] < stage.max[0] - 1e-6
                && o.max[2] > stage.min[2] + 1e-6
                && o.min[2] < stage.max[2] - 1e-6;
            match over {
                true => assert!(
                    (o.min[1] - stage.max[1]).abs() < 0.03,
                    "{id} on the stage: {o:?}"
                ),
                false => {
                    beside += 1;
                    assert!(
                        o.min[1].abs() < 0.03,
                        "{id} on the floor, not in the air: {o:?}"
                    );
                }
            }
        }
        assert!(
            beside > 0,
            "a 1.5 m stage holds no four 0.8 m pieces: {notes:?}"
        );
        assert!(
            notes.iter().any(|n| n.contains("so it stands beside it")),
            "{notes:?}"
        );
    }

    /// A deck that came out sloping (the generator keeps its picture's tilt)
    /// is set level before anything stands on it: two crates at its two ends
    /// stand at one height. One sloping past 25° stays as made, said plainly.
    #[test]
    fn a_sloping_base_is_set_level_before_pieces_stand_on_it() {
        for (slope, levelled) in [(10.0_f64, true), (30.0, false)] {
            let k = slope.to_radians().tan();
            let tilt = |t: Tri| t.map(|v| [v[0], v[1] + v[0] * k, v[2]]);
            let mut tris: Vec<Tri> = quad(-3.0, 3.0, -1.5, 1.5, 1.0, true).map(tilt).to_vec();
            tris.extend(quad(-3.0, 3.0, -1.5, 1.5, -2.0, false));
            let mut plan = Plan {
                title: "Deck".into(),
                items: ["deck", "left", "right"]
                    .map(|i| item(i, "object"))
                    .to_vec(),
                ..Default::default()
            };
            let on = |id: &str, at: &str| {
                let mut p = place(id, at);
                p.rel = Some(serde_json::from_value(json!({"to": "deck", "how": "on"})).unwrap());
                p
            };
            plan.layout = vec![
                place("deck", "center"),
                on("left", "left"),
                on("right", "right"),
            ];
            let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
            let mut models = vec![meshed("deck", &tris)];
            for id in ["left", "right"] {
                let mut m = model(id, b([-0.3, 0.0, -0.3], [0.3, 0.6, 0.3]));
                m.real_size = true;
                models.push(m);
            }
            let (doc, notes) = build_scene(&plan, &models);
            let rot = nums::<3>(&obj(&doc, "deck")["rotation"]).unwrap();
            let floor = |id: &str| world_box(obj(&doc, id), &models).min[1];
            if levelled {
                assert!(
                    (rot[2].abs() - slope).abs() < 0.5 && rot[0].abs() < 0.5,
                    "turned level: {rot:?}"
                );
                assert!(
                    (floor("left") - floor("right")).abs() < 0.05,
                    "one height: {} {}",
                    floor("left"),
                    floor("right")
                );
            } else {
                assert_eq!(rot, [0.0, rot[1], 0.0], "left as made");
                assert!(notes.iter().any(|n| n.contains("too steep")), "{notes:?}");
            }
        }
    }

    #[test]
    fn a_levelled_sled_rests_on_its_actual_runners() {
        let turn = turn_matrix([17.0, 0.0, 0.0]);
        let mut mesh = quad(-0.4, 0.4, -0.9, 0.9, 0.3, true).to_vec();
        mesh.extend(quad(-0.4, 0.4, -0.9, 0.9, 0.0, false));
        let mesh: Vec<_> = mesh.iter().map(|t| t.map(|v| turned(turn, v))).collect();
        let mut plan = Plan {
            items: vec![item("sled", "object"), item("rider", "character")],
            ..Default::default()
        };
        let mut rider = place("rider", "");
        rider.rel = Some(serde_json::from_value(json!({"to":"sled","how":"on"})).unwrap());
        plan.layout = vec![place("sled", "center"), rider];
        let (doc, _) = build_scene(
            &plan,
            &[meshed("sled", &mesh), model("rider", Bounds::UNIT)],
        );
        let sled = obj(&doc, "sled");
        let rotation = turn_matrix(nums(&sled["rotation"]).unwrap());
        let scale: [f64; 3] = nums(&sled["scale"]).unwrap();
        let origin = sled["position"][1].as_f64().unwrap();
        let runner = mesh
            .iter()
            .flatten()
            .map(|p| origin + turned(rotation, [0, 1, 2].map(|k| p[k] * scale[k]))[1])
            .fold(f64::INFINITY, f64::min);
        assert!(
            runner.abs() < 0.001,
            "lowest runner {runner} m above ground"
        );
    }

    #[test]
    fn a_supported_lean_shows_the_body_slope_and_the_crossed_arms() {
        let plan: Plan = serde_json::from_value(json!({"title":"Waiting", "items":[
            {"id":"porter","kind":"character","name":"Porter","description":"Rests against the counter.",
             "details":{"size_m":1.8,"pose":{"kind":"lean","on":"counter","arms":"crossed"}}},
            {"id":"counter","kind":"object","name":"Counter","description":"A low counter.","details":{"size_m":2.0}}],
            "layout":[{"item":"counter","place":"center"},{"item":"porter","rel":{"how":"beside","to":"counter","side":"left"}}]})).unwrap();
        let mut person = model(
            "porter",
            Bounds {
                min: [-0.3, 0., -0.4],
                max: [0.3, 1.8, 0.25],
            },
        );
        person.pose = Posed::of(
            &json!({"bounds":{"min":person.bounds.min,"max":person.bounds.max},
            "contact":{"back":[[0.,0.8,-0.15],[0.,1.0,-0.2],[0.,1.4,-0.35]]}}),
        );
        let mut counter = model(
            "counter",
            Bounds {
                min: [-0.3, 0., -1.0],
                max: [0.3, 1.2, 1.0],
            },
        );
        counter.tris = vec![
            [[-0.3, 0., -1.], [-0.3, 1.2, -1.], [-0.3, 1.2, 1.]],
            [[-0.3, 0., -1.], [-0.3, 1.2, 1.], [-0.3, 0., 1.]],
        ];
        let (doc, _) = build_scene(&plan, &[person, counter]);
        let actor = obj(&doc, "porter");
        let p = nums::<3>(&actor["position"]).unwrap();
        let c = nums::<3>(&doc["camera"]["position"]).unwrap();
        let angle =
            (c[0] - p[0]).atan2(c[2] - p[2]) - actor["rotation"][1].as_f64().unwrap().to_radians();
        assert!(
            angle.sin().abs() > 0.55 && angle.cos() > 0.45,
            "lean and crossed forearms must both read from the opening, relative angle {} degrees",
            angle.to_degrees()
        );
    }

    #[test]
    fn a_planted_shaft_meets_the_actual_posed_fist() {
        let plan: Plan = serde_json::from_value(json!({"title":"Survey", "items":[
            {"id":"surveyor", "kind":"character", "name":"Surveyor", "description":"Plants the surveying stake.", "details":{"size_m":1.7,"pose":{"kind":"stand","plant":"stake"}}},
            {"id":"stake", "kind":"object", "name":"Survey stake", "description":"A thin pole.", "details":{"size_m":1.5}}],
            "layout":[{"item":"stake","place":"center"},{"item":"surveyor","rel":{"how":"beside","to":"stake","side":"left"}}]})).unwrap();
        let mut person = model(
            "surveyor",
            Bounds {
                min: [-0.3, 0., -0.25],
                max: [0.3, 1.7, 0.25],
            },
        );
        let hand = [0.14, 1.2, 0.32];
        person.pose = Posed::of(
            &json!({"bounds":{"min":person.bounds.min,"max":person.bounds.max},"contact":{"shaft":hand}}),
        );
        let stake = model(
            "stake",
            Bounds {
                min: [-0.02, 0., -0.02],
                max: [0.02, 1.5, 0.02],
            },
        );
        let (doc, _) = build_scene(&plan, &[person, stake]);
        let actor = obj(&doc, "surveyor");
        let pole = obj(&doc, "stake");
        let o = nums::<3>(&actor["position"]).unwrap();
        let s = nums::<3>(&actor["scale"]).unwrap();
        let a = actor["rotation"][1].as_f64().unwrap().to_radians();
        let x = o[0] + hand[0] * s[0] * a.cos() + hand[2] * s[2] * a.sin();
        let z = o[2] - hand[0] * s[0] * a.sin() + hand[2] * s[2] * a.cos();
        let origin = nums::<3>(&pole["position"]).unwrap();
        assert!(
            (x - origin[0]).hypot(z - origin[2]) < 0.002,
            "fist at {x},{z}, shaft at {origin:?}"
        );
    }

    #[test]
    fn a_planted_flags_cloth_extends_away_from_the_planters_face() {
        let plan: Plan = serde_json::from_value(json!({"items":[
            {"id":"walker","kind":"character","name":"Walker","description":"Plants a flag.",
             "details":{"pose":{"kind":"stand","plant":"pole"}}},
            {"id":"pole","kind":"object","name":"Trail flag","description":"A flag on a thin pole."}],
            "layout":[{"item":"pole","place":"center"},
                      {"item":"walker","rel":{"how":"beside","to":"pole","side":"left"}}]})).unwrap();
        let actor = model("walker", Bounds::UNIT);
        let mut flag = model(
            "pole",
            Bounds {
                min: [-0.55, 0., -0.02],
                max: [0.02, 1.6, 0.02],
            },
        );
        flag.real_size = true;
        flag.tris = vec![
            [[0., 0.20, 0.], [0., 0.50, 0.], [0., 0.65, 0.]],
            [[0., 1.5, 0.], [-0.54, 1.5, 0.], [-0.54, 1.15, 0.]],
        ];
        let (doc, _) = build_scene(&plan, &[actor, flag]);
        let mast = obj(&doc, "pole");
        let origin = nums::<3>(&mast["position"]).unwrap();
        let head = nums::<3>(&obj(&doc, "walker")["position"]).unwrap();
        let yaw = mast["rotation"][1].as_f64().unwrap().to_radians();
        let away = [origin[0] - head[0], origin[2] - head[2]];
        let cosine = (-yaw.cos() * away[0] + yaw.sin() * away[1]) / away[0].hypot(away[1]);
        assert!(cosine > 0.8, "cloth projects toward its planter: {cosine}");
    }

    #[test]
    fn a_vehicle_beside_the_cast_stays_in_the_opening_view() {
        // The Moon production places the flag beside the rover and the astronaut
        // beside the flag. Neither piece stands on the rover.
        let mut plan = Plan {
            items: vec![
                item("astronaut", "character"),
                item("flag", "object"),
                item("rover", "object"),
            ],
            ..Default::default()
        };
        let beside = |id: &str, target: &str| {
            let mut p = place(id, "");
            p.rel = Some(
                serde_json::from_value(json!({"to": target, "how": "beside", "side": "left"}))
                    .unwrap(),
            );
            p
        };
        plan.layout = vec![
            place("rover", "right"),
            beside("flag", "rover"),
            beside("astronaut", "flag"),
        ];
        let mut models: Vec<_> = [
            ("astronaut", [0.98, 1.5, 0.45]),
            ("flag", [0.60, 1.66, 0.47]),
            ("rover", [1.60, 1.15, 2.60]),
        ]
        .into_iter()
        .map(|(id, size)| {
            model(
                id,
                Bounds {
                    min: [-size[0] / 2.0, 0.0, -size[2] / 2.0],
                    max: [size[0] / 2.0, size[1], size[2] / 2.0],
                },
            )
        })
        .collect();
        for m in &mut models {
            m.real_size = true;
        }
        let (doc, _) = build_scene(&plan, &models);
        assert_ne!(
            obj(&doc, "rover")["backdrop"],
            true,
            "the viewer omits backdrop bounds when fitting its camera"
        );
    }

    #[test]
    fn a_prop_with_a_sloped_top_beside_someone_is_not_a_platform() {
        let slope = 10.0_f64.to_radians().tan();
        let roof = quad(-2.0, 2.0, -1.0, 1.0, 1.0, true)
            .map(|tri| tri.map(|v| [v[0], v[1] + slope * v[0], v[2]]));
        let mut tris = roof.to_vec();
        tris.extend(quad(-2.0, 2.0, -1.0, 1.0, -1.0, false));
        let rover = meshed("rover", &tris);
        let mut flag = place("flag", "");
        flag.rel = Some(
            serde_json::from_value(json!({"to":"rover","how":"beside","side":"left"})).unwrap(),
        );
        let plan = Plan {
            items: vec![item("rover", "object"), item("flag", "object")],
            layout: vec![place("rover", "right"), flag],
            ..Default::default()
        };
        let flag = model(
            "flag",
            Bounds {
                min: [-0.2, 0.0, -0.1],
                max: [0.2, 1.5, 0.1],
            },
        );
        let turns = fit_models(&plan, &[rover, flag]).turns;
        assert!(
            !turns.contains_key("rover"),
            "a vehicle's sloping roof does not tilt its wheels off the ground"
        );
    }

    #[test]
    fn an_enclosed_space_helmet_surrounds_the_face() {
        let mut helmet = item("helmet", "object");
        helmet.name = "Space helmet".into();
        let mut at = place("helmet", "");
        at.rel = Some(
            serde_json::from_value(json!({"to": "astronaut", "how": "worn", "part": "head"}))
                .unwrap(),
        );
        let plan = Plan {
            items: vec![item("astronaut", "character"), helmet],
            layout: vec![place("astronaut", "center"), at],
            ..Default::default()
        };
        let mut astronaut = model(
            "astronaut",
            Bounds {
                min: [-0.49, -0.746, -0.22],
                max: [0.49, 0.754, 0.24],
            },
        );
        astronaut
            .joints
            .insert("neck".into(), [0.005, 0.405, -0.017]);
        let mut helmet = model(
            "helmet",
            Bounds {
                min: [-0.17, -0.163, -0.174],
                max: [0.177, 0.199, 0.158],
            },
        );
        astronaut.real_size = true;
        helmet.real_size = true;
        let models = [astronaut, helmet];
        let (doc, _) = build_scene(&plan, &models);
        let head_top = world_box(obj(&doc, "astronaut"), &models).max[1];
        let shell = world_box(obj(&doc, "helmet"), &models);
        assert!(
            shell.min[1] <= head_top - 0.30 && shell.max[1] >= head_top,
            "helmet {shell:?}, crown {head_top}"
        );

        // A posed head can project well forward of the neck. Its shell has to
        // surround the actual skin horizontally as well as clear the crown.
        let mut models = models;
        models[0].pose = Posed::of(&json!({
            "bounds": {"min":[-0.4357,-0.7268,-0.197], "max":[0.3158,0.7077,0.5425]},
            "head":{"point":[0.0025,0.5272,0.0453],
                    "bounds":{"min":[-0.0926,0.4680,0.0062], "max":[0.0961,0.7077,0.2213]}}
        }));
        let (doc, _) = build_scene(&plan, &models);
        let shell = world_box(obj(&doc, "helmet"), &models);
        let wearer = obj(&doc, "astronaut");
        let pos = nums::<3>(&wearer["position"]).unwrap();
        let scale = nums::<3>(&wearer["scale"]).unwrap();
        let turn = turn_matrix(nums::<3>(&wearer["rotation"]).unwrap());
        let head_mid = [0.00175, 0.58785, 0.11375];
        for axis in 0..3 {
            let world = pos[axis]
                + (0..3)
                    .map(|j| turn[axis][j] * scale[j] * head_mid[j])
                    .sum::<f64>();
            assert!(
                (shell.mid(axis) - world).abs() < 0.004,
                "shell {shell:?} misses head centre on axis {axis}"
            );
        }
    }

    /// With a cast the camera frames the action, not a big base: a keeper
    /// on a 12 m deck fills the view and the lantern in his hand is at least
    /// 3 % of the picture's height, while the deck runs out of the frame.
    #[test]
    fn the_camera_frames_the_action_not_the_base() {
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let mut plan = Plan {
            title: "Pier".into(),
            items: vec![
                item("keeper", "character"),
                item("lamp", "object"),
                item("deck", "object"),
            ],
            ..Default::default()
        };
        plan.items[0].details.holds = vec![crate::productions::Hold {
            item: "lamp".into(),
            socket: "right_hand".into(),
        }];
        let on = |id: &str| {
            let mut p = place(id, "center");
            p.rel = Some(serde_json::from_value(json!({"to": "deck", "how": "on"})).unwrap());
            p
        };
        plan.layout = vec![
            place("deck", "center"),
            on("keeper"),
            place("lamp", "center"),
        ];
        let mut keeper = model("keeper", b([-0.3, 0.0, -0.2], [0.3, 1.8, 0.2]));
        keeper.joints.insert("right_hand".into(), [-0.35, 0.9, 0.1]);
        let mut deck = model("deck", b([-6.0, 0.0, -6.0], [6.0, 0.5, 6.0]));
        let mut lamp = model("lamp", b([-0.1, 0.0, -0.1], [0.1, 0.3, 0.1]));
        for m in [&mut keeper, &mut deck, &mut lamp] {
            m.real_size = true;
        }
        let models = vec![keeper, lamp, deck];
        let (doc, notes) = build_scene(&plan, &models);
        let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
        let at = nums::<3>(&doc["camera"]["target"]).unwrap();
        let f = [0, 1, 2].map(|k| at[k] - cam[k]);
        let n = f.iter().map(|x| x * x).sum::<f64>().sqrt();
        let f = f.map(|x| x / n);
        let r = [f[2], 0.0, -f[0]].map(|x| x / f[0].hypot(f[2]));
        let up = [
            r[1] * f[2] - r[2] * f[1],
            r[2] * f[0] - r[0] * f[2],
            r[0] * f[1] - r[1] * f[0],
        ];
        let half = (doc["camera"]["fov"].as_f64().unwrap() / 2.0)
            .to_radians()
            .tan();
        let screen = |p: [f64; 3]| {
            let q = [0, 1, 2].map(|k| p[k] - cam[k]);
            let z = (0..3).map(|k| q[k] * f[k]).sum::<f64>();
            let x = (0..3).map(|k| q[k] * r[k]).sum::<f64>() / z / half;
            let y = (0..3).map(|k| q[k] * up[k]).sum::<f64>() / z / half;
            (x, y)
        };
        let ys: Vec<f64> = corners(obj(&doc, "lamp"), &models)
            .iter()
            .map(|c| screen(*c).1)
            .collect();
        let tall = (ys.iter().cloned().fold(f64::MIN, f64::max)
            - ys.iter().cloned().fold(f64::MAX, f64::min))
            / 2.0;
        assert!(
            tall >= 0.03,
            "the lantern is {tall} of the picture's height"
        );
        let out = corners(obj(&doc, "deck"), &models)
            .iter()
            .any(|c| screen(*c).0.abs() > 16.0 / 9.0 || screen(*c).1.abs() > 1.0);
        assert!(out, "the deck runs out of the frame");
        assert!(
            notes.iter().all(|n| !n.contains("outside the first view")),
            "{notes:?}"
        );
    }

    /// A lantern in the keeper's right hand: the camera looks from his right,
    /// and a lantern his body still hides (behind his back) is brought into
    /// view: he turns toward the camera (never more than 35°), the camera
    /// never goes round to his back, and nothing stands between it and the
    /// lantern.
    #[test]
    fn a_held_piece_behind_its_holder_comes_into_view() {
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let mut plan = Plan {
            title: "Keeper".into(),
            items: vec![item("keeper", "character"), item("lamp", "object")],
            ..Default::default()
        };
        plan.items[0].details.holds = vec![crate::productions::Hold {
            item: "lamp".into(),
            socket: "right_hand".into(),
        }];
        plan.layout = vec![place("keeper", "center"), place("lamp", "center")];
        let mut keeper = model("keeper", b([-0.3, 0.0, -0.15], [0.3, 1.8, 0.15]));
        keeper
            .joints
            .insert("right_hand".into(), [-0.05, 1.0, -0.3]);
        keeper.real_size = true;
        let mut lamp = model("lamp", b([-0.1, 0.0, -0.1], [0.1, 0.25, 0.1]));
        lamp.real_size = true;
        let models = vec![keeper, lamp];
        let (doc, _) = build_scene(&plan, &models);
        let yaw = obj(&doc, "keeper")["rotation"][1].as_f64().unwrap();
        assert!(yaw.abs() > 1.0 && yaw.abs() <= 35.0 + 1e-6, "turned: {yaw}");
        let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
        let lamp = world_box(obj(&doc, "lamp"), &models);
        let to = [0, 1, 2].map(|k| lamp.mid(k) - cam[k]);
        assert!(
            !blocks(&doc, &models, "keeper", cam, to),
            "the lantern shows"
        );
        let p = nums::<3>(&obj(&doc, "keeper")["position"]).unwrap();
        let look = (cam[0] - p[0]).atan2(cam[2] - p[2]).to_degrees();
        let off = ((look - yaw + 180.0).rem_euclid(360.0) - 180.0).abs();
        // Side-on at most (a lantern behind his back): never his back.
        assert!(off < 135.0, "not his back: {off}");
    }

    #[test]
    fn a_bubble_trail_rises_from_inside_its_open_container() {
        let mut vessel = quad(-0.5, 0.5, -0.5, 0.5, 0.1, true).to_vec();
        vessel.extend(quad(-0.5, 0.5, 0.4, 0.5, 0.8, true));
        let mut bubbles = item("stream", "object");
        bubbles.name = "Bubble trail".into();
        let mut stream = place("stream", "");
        stream.rel = Some(serde_json::from_value(json!({"to":"vessel","how":"above"})).unwrap());
        let mut plan = Plan {
            items: vec![item("vessel", "object"), bubbles],
            layout: vec![place("vessel", "center"), stream],
            ..Default::default()
        };
        let mut models = [meshed("vessel", &vessel), model("stream", Bounds::UNIT)];
        let lower = |plan: &Plan, models: &[Model]| {
            let (doc, _) = build_scene(plan, models);
            world_box(obj(&doc, "stream"), models).min[1]
                - world_box(obj(&doc, "vessel"), models).min[1]
        };
        assert!(
            lower(&plan, &models) < 0.2,
            "trail starts above the mouth at {}",
            lower(&plan, &models)
        );
        plan.items[0].name = "Open chest".into();
        models[0]
            .joints
            .insert("lid_hinge".into(), [0.0, 0.45, -0.45]);
        assert!(
            (lower(&plan, &models) - 0.35).abs() < 0.02,
            "at the articulated lid hinge"
        );
        plan.items[1].name = "Floating lantern".into();
        assert!(
            lower(&plan, &models) > 0.7,
            "an independent object stays above the lid"
        );
    }

    /// In needs a real cavity (an open basket: on its inner floor; a closed
    /// chest: on top, said); under needs real clearance (a cat under a table;
    /// a big dog beside it, said).
    #[test]
    fn in_and_under_follow_the_real_shape() {
        let mut basket: Vec<Tri> = quad(-0.5, 0.5, -0.5, 0.5, 0.05, true).to_vec();
        for (x0, x1, z0, z1) in [
            (-0.5, 0.5, -0.5, -0.45),
            (-0.5, 0.5, 0.45, 0.5),
            (-0.5, -0.45, -0.5, 0.5),
            (0.45, 0.5, -0.5, 0.5),
        ] {
            basket.extend(quad(x0, x1, z0, z1, 0.4, true));
        }
        basket.extend(quad(-0.5, 0.5, -0.5, 0.5, 0.0, false));
        let mut chest: Vec<Tri> = quad(-0.5, 0.5, -0.3, 0.3, 0.6, true).to_vec();
        chest.extend(quad(-0.5, 0.5, -0.3, 0.3, 0.0, false));
        let mut table: Vec<Tri> = quad(-0.8, 0.8, -0.5, 0.5, 0.75, true).to_vec();
        table.extend(quad(-0.8, 0.8, -0.5, 0.5, 0.7, false));
        for (x, z) in [(-0.75, -0.45), (0.75, -0.45), (-0.75, 0.45), (0.75, 0.45)] {
            table.extend(quad(x - 0.05, x + 0.05, z - 0.05, z + 0.05, 0.0, false));
        }
        let mut plan = Plan {
            title: "Room".into(),
            items: ["basket", "chest", "table", "puppy", "coins", "cat", "dog"]
                .map(|i| item(i, "object"))
                .to_vec(),
            ..Default::default()
        };
        let rel = |id: &str, to: &str, how: &str| {
            let mut p = place(id, "center");
            p.rel = Some(serde_json::from_value(json!({"to": to, "how": how})).unwrap());
            p
        };
        plan.layout = vec![
            place("basket", "left"),
            place("chest", "center"),
            place("table", "right"),
            rel("puppy", "basket", "in"),
            rel("coins", "chest", "in"),
            rel("cat", "table", "under"),
            rel("dog", "table", "under"),
        ];
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let mut models = vec![
            meshed("basket", &basket),
            meshed("chest", &chest),
            meshed("table", &table),
        ];
        for (id, h) in [("puppy", 0.2), ("coins", 0.1), ("cat", 0.3), ("dog", 1.0)] {
            let mut m = model(id, b([-0.1, 0.0, -0.1], [0.1, h, 0.1]));
            m.real_size = true;
            models.push(m);
        }
        let (doc, notes) = build_scene(&plan, &models);
        let w = |id| world_box(obj(&doc, id), &models);
        assert!(
            (w("puppy").min[1] - (w("basket").min[1] + 0.05)).abs() < 0.02,
            "on the basket's floor: {:?}",
            w("puppy")
        );
        assert!(
            (w("coins").min[1] - w("chest").max[1]).abs() < 0.02,
            "on the closed chest: {:?}",
            w("coins")
        );
        let (cat, table) = (w("cat"), w("table"));
        assert!(
            cat.min[1].abs() < 1e-3 && cat.max[1] < 0.7 && (cat.mid(0) - table.mid(0)).abs() < 0.3,
            "under: {cat:?}"
        );
        let dog = w("dog");
        let meet = dog.min[0] < table.max[0]
            && table.min[0] < dog.max[0]
            && dog.min[2] < table.max[2]
            && table.min[2] < dog.max[2];
        assert!(!meet && dog.min[1].abs() < 1e-3, "beside: {dog:?}");
        assert!(
            notes.contains(
                &"The chest has no room inside, so the coins sits on top of it.".to_string()
            ),
            "{notes:?}"
        );
        assert!(
            notes.contains(
                &"The dog does not fit under the table, so it stands beside it.".to_string()
            ),
            "{notes:?}"
        );
    }

    /// `dims_m` scales the made model toward the planned proportions per
    /// axis, within ×0.4–×2.5 of its uniform scale; past that it keeps its
    /// own shape, said in a note.
    #[test]
    fn planned_proportions_scale_each_axis_within_a_limit() {
        for (dims, want, noted) in [
            ([8.0, 2.0, 4.0], [8.0, 2.0, 4.0], false),
            ([8.0, 0.5, 4.0], [8.0, 4.0, 8.0], true),
        ] {
            let mut plan = Plan {
                title: "Pier".into(),
                items: vec![item("pier", "object")],
                ..Default::default()
            };
            plan.items[0].details.dims_m = Some(dims);
            plan.layout = vec![place("pier", "center")];
            let b = glb_bounds(&fixture_glb([-4.0, 0.0, -4.0], [4.0, 4.0, 4.0])).unwrap();
            let models = vec![model("pier", b)];
            let (doc, notes) = build_scene(&plan, &models);
            let o = world_box(obj(&doc, "pier"), &models);
            for (k, w) in want.iter().enumerate() {
                assert!((o.size(k) - w).abs() < 1e-3, "{dims:?} → {o:?}");
            }
            assert_eq!(!notes.is_empty(), noted, "{notes:?}");
        }
    }

    /// The camera looks at faces: three-quarter front on the main character,
    /// side-on to two characters that face each other.
    #[test]
    fn the_camera_looks_at_the_main_characters_faces() {
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let person = b([-0.3, 0.0, -0.2], [0.3, 1.8, 0.2]);
        let off = |doc: &Value, id: &str| {
            let o = obj(doc, id);
            let (p, yaw) = (
                nums::<3>(&o["position"]).unwrap(),
                o["rotation"][1].as_f64().unwrap(),
            );
            let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
            let look = (cam[0] - p[0]).atan2(cam[2] - p[2]).to_degrees();
            ((look - yaw + 180.0).rem_euclid(360.0) - 180.0).abs()
        };
        // One man turned toward a chest behind him: the camera finds his face.
        let mut plan = Plan {
            title: "Man".into(),
            items: vec![item("man", "character"), item("chest", "object")],
            ..Default::default()
        };
        let mut man = place("man", "front");
        man.facing = Some("chest".into());
        plan.layout = vec![man, place("chest", "back")];
        let models = vec![
            model("man", person),
            model("chest", b([-0.5, 0.0, -0.3], [0.5, 0.6, 0.3])),
        ];
        let (doc, _) = build_scene(&plan, &models);
        assert!(
            off(&doc, "man") <= 60.0,
            "three-quarter front: {}",
            off(&doc, "man")
        );
        // From about eye level, at his chest (like a person near the scene).
        let (cam, at) = (
            nums::<3>(&doc["camera"]["position"]).unwrap(),
            nums::<3>(&doc["camera"]["target"]).unwrap(),
        );
        let up = (cam[1] - at[1])
            .atan2((cam[0] - at[0]).hypot(cam[2] - at[2]))
            .to_degrees();
        assert!((10.0..=20.0).contains(&up), "eye level: {up}");
        assert!((at[1] - 0.72 * 1.8).abs() < 0.05, "his chest: {at:?}");
        // Two who face each other: seen side-on, both faces in profile.
        let mut plan = Plan {
            title: "Talk".into(),
            items: vec![item("a", "character"), item("b", "character")],
            ..Default::default()
        };
        let (mut pa, mut pb) = (place("a", "left"), place("b", "right"));
        (pa.facing, pb.facing) = (Some("b".into()), Some("a".into()));
        plan.layout = vec![pa, pb];
        let models = vec![model("a", person), model("b", person)];
        let (doc, _) = build_scene(&plan, &models);
        for id in ["a", "b"] {
            assert!(
                (off(&doc, id) - 90.0).abs() <= 30.0,
                "{id} side-on: {}",
                off(&doc, id)
            );
        }
    }

    #[test]
    fn a_floor_level_fan_of_playing_arms_is_seen_above_its_foreground_tips() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[{"id":"player","kind":"character","name":"Player","description":"Playing percussion."},
                {"id":"kelp","kind":"object","name":"Kelp","description":"Tall seaweed."}],
            "setting":{"world":"underwater","ground":"sea_floor","time":"day"},
            "layout":[{"item":"player","place":"center"},{"item":"kelp","place":"far behind"}]
        })).unwrap();
        let mut actor = model(
            "player",
            Bounds {
                min: [-0.8, 0., -0.8],
                max: [0.8, 0.36, 0.8],
            },
        );
        let tips: Vec<_> = (0..8)
            .map(|i| {
                let angle = std::f64::consts::FRAC_PI_4 * f64::from(i);
                [0.8 * angle.cos(), 0.05, 0.8 * angle.sin()]
            })
            .collect();
        let report = json!({"manifest":{"bounds":{"min":actor.bounds.min,"max":actor.bounds.max},
            "limb_tips":tips, "contact":{"sticks":[tips[2],tips[3]]}}});
        actor.apply_pose(Posed::of(&report["manifest"]).unwrap(), &report, None);
        let (doc, _) =
            super::super::setting::build_scene(&plan, &[actor, model("kelp", Bounds::UNIT)]);
        assert_eq!(obj(&doc, "kelp")["backdrop"], true);
        let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
        let at = nums::<3>(&doc["camera"]["target"]).unwrap();
        let pitch = (cam[1] - at[1])
            .atan2((cam[0] - at[0]).hypot(cam[2] - at[2]))
            .to_degrees();
        assert!(
            (20.0..25.0).contains(&pitch),
            "floor-level arms at {pitch} degrees"
        );
    }

    #[test]
    fn an_existing_swimmer_turns_its_head_toward_the_station_after_reposing() {
        let plan: Plan = serde_json::from_value(json!({
            "items":[{"id":"fish","name":"Reef fish","description":"","kind":"character",
                "details":{"body":"fish"},
                "result":{"rig":{"forward":[1,0,0]},"pose":{"manifest":{"forward":[-1,0,0]}}}},
                {"id":"kit","kind":"object","name":"Drums","description":""}],
            "layout":[{"item":"kit","place":"center"},{"item":"fish","place":"right","facing":"kit"}]
        })).unwrap();
        let models = [
            model(
                "fish",
                Bounds {
                    min: [-0.4, 0., -0.15],
                    max: [0.4, 0.3, 0.15],
                },
            ),
            model(
                "kit",
                Bounds {
                    min: [-0.5, 0., -0.5],
                    max: [0.5, 0.6, 0.5],
                },
            ),
        ];
        let (doc, _) = build_scene(&plan, &models);
        let fish = obj(&doc, "fish");
        let head = fish["rotation"][1].as_f64().unwrap().to_radians();
        let here = nums::<3>(&fish["position"]).unwrap();
        let target = nums::<3>(&obj(&doc, "kit")["position"]).unwrap();
        let towards = [target[0] - here[0], target[2] - here[2]];
        let dot = -head.cos() * towards[0] + head.sin() * towards[1];
        assert!(
            dot > 0.99 * towards[0].hypot(towards[1]),
            "tail towards the kit: {fish}"
        );
    }

    #[test]
    fn an_independent_parked_cart_shows_its_wheelbase_and_a_directed_cart_keeps_its_heading() {
        let mut cart = model(
            "delivery-cart",
            Bounds {
                min: [-0.6, 0., -1.1],
                max: [0.6, 0.8, 1.1],
            },
        );
        for (i, p) in [
            [-0.52, 0., -0.9],
            [0.52, 0., -0.9],
            [-0.52, 0., 0.9],
            [0.52, 0., 0.9],
        ]
        .into_iter()
        .enumerate()
        {
            cart.joints.insert(format!("wheel_contact/{i}"), p);
        }
        let unmarked = model("delivery-cart", cart.bounds);
        let mut plan = Plan {
            items: vec![item("delivery-cart", "object")],
            layout: vec![place("delivery-cart", "center")],
            ..Default::default()
        };
        let make = |plan: &Plan, cart: &Model| build_scene(plan, std::slice::from_ref(cart)).0;
        let angle = |doc: &Value| obj(doc, "delivery-cart")["rotation"][1].as_f64().unwrap();
        let parked = make(&plan, &cart);
        assert_ne!(
            angle(&parked),
            angle(&make(&plan, &unmarked)),
            "a measured wheelbase can be shown even when it is the only item"
        );
        plan.items[0].details.moves.push("roll".into());
        assert_eq!(make(&plan, &cart), make(&plan, &unmarked));
    }

    #[test]
    fn reciprocal_beside_instructions_require_the_same_screen_order() {
        let left = Spot {
            center: [-1., 0.],
            floor: 0.,
            rot_y: 0.,
            scale: [1.; 3],
            b: Bounds {
                min: [-0.2; 3],
                max: [0.2; 3],
            },
            tilt: None,
        };
        let right = Spot {
            center: [1., 0.],
            ..left
        };
        let spots = BTreeMap::from([("stool", left), ("shelf", right)]);
        for (item, target, side) in [("stool", "shelf", "left"), ("shelf", "stool", "right")] {
            let layout: Vec<crate::productions::Placement> = serde_json::from_value(json!([{
                "item": item, "rel": {"how": "beside", "to": target, "side": side}
            }]))
            .unwrap();
            let pairs = side_by_side(&layout, &spots);
            assert_eq!(pairs.len(), 1);
            assert_eq!(
                (pairs[0].0.center, pairs[0].1.center),
                (left.center, right.center)
            );
        }
    }

    #[test]
    fn spectators_on_opposite_sides_leave_the_performer_between_them_on_screen() {
        let mut plan: Plan = serde_json::from_value(json!({
            "setting":{"world":"underwater","ground":"sea_floor","time":"day","weather":"clear"},
            "items":[
                {"id":"player","kind":"character","name":"Performer","description":"","details":{"body":"many_legs","size_m":0.55,"pose":{"kind":"stand","play":"kit"}},"result":{"rig":{"forward":[0.19,0,-0.98]}}},
                {"id":"kit","kind":"object","name":"Drum kit","description":""},
                {"id":"yellow","kind":"character","name":"Yellow fish","description":"","details":{"body":"fish"},"result":{"rig":{"forward":[-0.84,0,0.55]}}},
                {"id":"blue","kind":"character","name":"Blue fish","description":"","details":{"body":"fish"},"result":{"rig":{"forward":[-0.92,0,0.40]}}},
                {"id":"chest","kind":"object","name":"Chest","description":""}
            ],
            "layout":[{"item":"kit","place":"center"},
                      {"item":"player","facing":"kit","rel":{"how":"behind","to":"kit"}},
                      {"item":"yellow","place":"front-left","facing":"player"},
                      {"item":"blue","place":"front-right","facing":"player"},
                      {"item":"chest","place":"left"}]
        })).unwrap();
        let models = [
            ("player", [1.8, 0.55, 1.9]),
            ("kit", [1.0, 0.50, 0.8]),
            ("yellow", [0.13, 0.24, 0.49]),
            ("blue", [0.16, 0.27, 0.58]),
            ("chest", [0.70, 0.68, 0.65]),
        ]
        .map(|(id, size)| Model {
            real_size: true,
            ..model(
                id,
                Bounds {
                    min: [-size[0] * 0.5, 0., -size[2] * 0.5],
                    max: [size[0] * 0.5, size[1], size[2] * 0.5],
                },
            )
        });
        for beside_station in [false, true] {
            if beside_station {
                for (spectator, side) in [("yellow", "left"), ("blue", "right")] {
                    let p = plan
                        .layout
                        .iter_mut()
                        .find(|p| p.item == spectator)
                        .unwrap();
                    p.place.clear();
                    p.rel = Some(
                        serde_json::from_value(json!({"how":"beside","to":"kit","side":side}))
                            .unwrap(),
                    );
                }
            }
            let (doc, _) = build_scene(&plan, &models);
            let (left, right) = (
                on_screen(&doc, &models, "yellow"),
                on_screen(&doc, &models, "blue"),
            );
            for id in ["player", "kit"] {
                let (lo, hi) = on_screen(&doc, &models, id);
                assert!(
                    left.1 < lo && hi < right.0,
                    "{id} spans {lo}..{hi}, spectators {left:?} and {right:?}"
                );
            }
        }
    }

    #[test]
    fn spectators_face_the_station_of_a_missing_performer() {
        let mut audience = place("audience", "left");
        audience.facing = Some("player".into());
        let mut player = place("player", "");
        player.facing = Some("kit".into());
        player.rel = Some(serde_json::from_value(json!({"to": "kit", "how": "behind"})).unwrap());
        let plan = Plan {
            items: vec![
                item("audience", "character"),
                item("player", "character"),
                item("kit", "object"),
            ],
            layout: vec![audience, player, place("kit", "right")],
            ..Default::default()
        };
        let models = ["audience", "kit"].map(|id| model(id, Bounds::UNIT));
        let (doc, _) = build_scene(&plan, &models);
        let viewer = obj(&doc, "audience");
        let from = nums::<3>(&viewer["position"]).unwrap();
        let to = nums::<3>(&obj(&doc, "kit")["position"]).unwrap();
        let expected = (to[0] - from[0]).atan2(to[2] - from[2]).to_degrees();
        let actual = viewer["rotation"][1].as_f64().unwrap();
        assert!(
            (expected - actual).abs() < 0.2,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn animals_face_and_follow_with_their_own_mesh_headings() {
        assert_trailing_heading("Following the fox at a trot");
    }

    #[test]
    fn a_cub_trotting_behind_shares_its_leaders_path() {
        assert_trailing_heading("A cub trots behind the fox.");
    }

    #[test]
    fn a_small_follower_shows_its_flank_in_the_opening() {
        let mut guide = item("walker", "character");
        guide.details.size_m = Some(1.6);
        let mut hound = item("hound", "character");
        hound.description = "A hound trots behind the walker.".into();
        hound.details.size_m = Some(0.55);
        let mut following = place("hound", "");
        following.rel = serde_json::from_value(json!({"how":"behind","to":"walker"})).ok();
        let plan = Plan {
            items: vec![guide, hound],
            layout: vec![place("walker", "center"), following],
            ..Default::default()
        };
        let models = [
            model("walker", Bounds::UNIT),
            model(
                "hound",
                Bounds {
                    min: [-0.12, 0., -0.4],
                    max: [0.12, 0.55, 0.4],
                },
            ),
        ];
        let (doc, _) = build_scene(&plan, &models);
        let dog = obj(&doc, "hound");
        let pos = nums::<3>(&dog["position"]).unwrap();
        let camera = nums::<3>(&doc["camera"]["position"]).unwrap();
        let angle = (camera[0] - pos[0]).atan2(camera[2] - pos[2])
            - dog["rotation"][1].as_f64().unwrap().to_radians();
        assert!(
            angle.sin().abs() > 0.65,
            "only {:.1}% of the flank is projected",
            100. * angle.sin().abs()
        );
    }

    fn assert_trailing_heading(description: &str) {
        let animal = |id: &str, yaw: f64| {
            let mut it = item(id, "character");
            let angle = yaw.to_radians();
            it.result = Some(super::super::ItemResult {
                rig: Some(
                    json!({"body_plan":"quadruped", "forward":[angle.sin(), 0., angle.cos()]}),
                ),
                ..Default::default()
            });
            it
        };
        let leader = animal("fox", 62.5);
        let mut follower = animal("cub", -90.);
        follower.description = description.into();
        let mut left = place("fox", "left");
        left.facing = Some("tree".into());
        let mut behind = place("cub", "back");
        behind.rel = Some(serde_json::from_value(json!({"to":"fox","how":"behind"})).unwrap());
        let plan = Plan {
            items: vec![leader, follower, item("tree", "object")],
            layout: vec![left, behind, place("tree", "right")],
            ..Default::default()
        };
        let models = ["fox", "cub", "tree"].map(|id| model(id, Bounds::UNIT));
        let (doc, _) = build_scene(&plan, &models);
        let object = |id| {
            doc["objects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["id"] == id)
                .unwrap()
        };
        let point = |id| nums::<3>(&object(id)["position"]).unwrap();
        let yaw = |id| object(id)["rotation"][1].as_f64().unwrap();
        let (fox, tree, cub) = (point("fox"), point("tree"), point("cub"));
        let toward = (tree[0] - fox[0]).atan2(tree[2] - fox[2]);
        let error =
            |angle: f64| ((angle - toward.to_degrees() + 180.).rem_euclid(360.) - 180.).abs();
        assert!(
            error(yaw("fox") + 62.5) < 0.2,
            "fox faces away from the tree"
        );
        assert!(
            error(yaw("cub") - 90.) < 0.2,
            "cub faces away from the fox's path"
        );
        assert!((cub[0] - fox[0]) * toward.sin() + (cub[2] - fox[2]) * toward.cos() < 0.);
    }

    #[test]
    fn wheel_contacts_follow_their_parent_and_require_the_entire_declared_set() {
        let mut nodes = vec![json!({"translation":[3.,2.,1.],"children":[1,2,3,4]})];
        for i in 0..4 {
            nodes.push(json!({"name":format!("WheelContact_{i}"),"translation":[i,0,0]}));
        }
        let mut doc = json!({"asset":{"extras":{"wheel_contacts":{"count":4}}},
            "scenes":[{"nodes":[0]}],"nodes":nodes});
        let joints = glb_joints(&glb_of(&doc));
        assert_eq!(joints.len(), 4);
        assert_eq!(joints["wheel_contact/2"], [5., 2., 1.]);
        doc["asset"]["extras"]["wheel_contacts"]["count"] = json!(6);
        assert!(glb_joints(&glb_of(&doc)).is_empty());
    }

    #[test]
    fn the_camera_gets_one_tip_per_weight_bearing_leg_chain() {
        let doc = json!({
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name":"root", "children":[1,3,4]},
                {"name":"leg_left_1_1", "children":[2]},
                {"name":"leg_left_1_2", "translation":[0.2,0.0,0.3]},
                {"name":"leg_right_1_2", "translation":[-0.2,0.0,0.3]},
                {"name":"leg_left_1_twin1_2", "translation":[0.19,0.0,0.29]}
            ],
            "skins": [{"joints":[0,1,2,3,4]}]
        });
        let joints = glb_joints(&glb_of(&doc));
        assert_eq!(joints.get("foot/leg_left_1_2"), Some(&[0.2, 0.0, 0.3]));
        assert_eq!(
            joints.keys().filter(|key| key.starts_with("foot/")).count(),
            2
        );
    }

    #[test]
    fn the_camera_reads_the_first_gait_clips_posed_soles() {
        let marks = vec![[0.10, 0.04, 0.20]; 6];
        let segments = vec![[[0.09, 0.04, 0.20], [0.12, 0.09, 0.21]]; 6];
        let doc = json!({"scenes":[{"nodes":[0]}],"nodes":[{"name":"root"}],
            "skins":[{"joints":[0]}],"animations":[
                {"name":"run","extras":{"foot_centres":marks,"lower_leg_axes":segments}},
                {"name":"walk","extras":{"foot_centres":[[0.,0.,0.]]}}]});
        let joints = glb_joints(&glb_of(&doc));
        assert_eq!(joints.get("moving_foot/5"), Some(&marks[5]));
        assert_eq!(joints.get("lower_leg_start/5"), Some(&segments[5][0]));
        assert_eq!(joints.get("lower_leg_end/5"), Some(&segments[5][1]));
    }

    #[test]
    fn two_separated_feet_can_have_crossing_shins() {
        let a = [[-0.1, 0.1], [0.1, 0.3]];
        let b = [[0.1, 0.1], [-0.1, 0.3]];
        assert_eq!(shin_gap(a, b), 0.);
        assert!((shin_gap([[0., 0.], [0., 2.]], [[0.25, 1.], [0.25, 3.]]) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn foreground_bounds_recentre_the_opening_before_projecting_the_feet() {
        let corners = [[-2., -1., -1.], [-2., 1., -1.], [2., -1., 1.], [2., 1., 1.]];
        let fit = OpeningProjection::fit(
            &corners,
            [0.; 3],
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
        );
        let left = fit.project(corners[0])[0];
        let right = fit.project(corners[2])[0];
        let limit = 0.88 * (FOV / 2.).to_radians().tan();
        assert!(fit.eye[0] > 0.35);
        assert!(left.abs() <= limit + 1e-6 && right.abs() <= limit + 1e-6);
        assert!((left + right).abs() < 0.001);
    }

    /// A GLB with a skeleton whose "Neck" joint sits under a scaled armature.
    #[test]
    fn the_neck_is_the_skeletons_neck_bone() {
        let doc = json!({
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name": "Armature", "children": [1, 2], "scale": [0.01, 0.01, 0.01]},
                {"name": "Body", "mesh": 0, "skin": 0},
                {"name": "Hips", "translation": [0.0, 90.0, 0.0], "children": [3]},
                {"name": "Neck", "translation": [0.0, 60.0, 20.0]}
            ],
            "skins": [{"joints": [2, 3]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "accessors": [{"min": [-30.0, 0.0, -20.0], "max": [30.0, 180.0, 20.0]}]
        });
        let n = glb_neck(&glb_of(&doc)).unwrap();
        assert!(close(n, [0.0, 1.5, 0.2]), "{n:?}");
        assert!(glb_neck(&fixture_glb([0.0; 3], [1.0; 3])).is_none());
    }

    /// Review v4: in the scene's own view the chest read as a case in the
    /// man's hand, the monster showed its back, and the key and the gold
    /// could not be seen. The real planner's layout for the owner's scene: the
    /// chest behind the monster stands clear of it, the man and the monster
    /// turn toward each other with their faces to the camera, the key hangs
    /// at the (skinned) monster's neck at a findable size, and the coins are
    /// heaped on the chest.
    #[test]
    fn the_owners_scene_shows_every_piece_from_the_camera() {
        let mut plan = Plan {
            title: "Sword".into(),
            items: ["man", "sword", "monster", "chest", "coins", "key"]
                .map(|i| item(i, "object"))
                .to_vec(),
            ..Default::default()
        };
        for c in [0, 2] {
            plan.items[c].kind = "character".into();
        }
        plan.items[0].details.holds = vec![crate::productions::Hold {
            item: "sword".into(),
            socket: "right_hand".into(),
        }];
        let facing = |id: &str, place_at: &str, to: &str| {
            let mut p = place(id, place_at);
            p.facing = Some(to.into());
            p
        };
        let attach = |id: &str, to: &str, at: &str| {
            let mut p = place(id, "back-right");
            p.attach_to = Some(Attach {
                item: to.into(),
                at: at.into(),
            });
            p
        };
        let mut monster = facing("monster", "right", "man");
        monster.scale_hint = Some("big".into());
        plan.layout = vec![
            facing("man", "left", "monster"),
            place("sword", "front-left"),
            monster,
            place("chest", "back-right"),
            attach("coins", "chest", "inside"),
            attach("key", "monster", "neck"),
        ];
        let b = |min, max| glb_bounds(&fixture_glb(min, max)).unwrap();
        let mut models = vec![
            model("man", b([-0.3, 0.0, -0.2], [0.3, 1.8, 0.2])),
            model("sword", b([-0.05, -0.1, -0.05], [0.05, 1.0, 0.05])),
            model("monster", b([-1.0, 0.0, -0.8], [1.0, 2.5, 0.8])),
            model("chest", b([-0.5, 0.0, -0.3], [0.5, 0.6, 0.3])),
            model("coins", b([-0.3, 0.0, -0.2], [0.3, 0.1, 0.2])),
            model("key", b([-0.05, -0.05, -0.01], [0.05, 0.1, 0.01])),
        ];
        models[2].joints.insert("neck".into(), [0.0, 2.0, 0.3]);
        // A library pile keeps its size unless the plan gives its metres.
        models[4].real_size = true;
        plan.items[4].details.size_m = Some(0.6);
        let (doc, notes) = build_scene(&plan, &models);
        assert!(notes.is_empty(), "{notes:?}");
        check_layout(&doc, &models, &["coins", "key"]);

        // The camera sees the chest: the man and the monster are not between.
        let cam = nums::<3>(&doc["camera"]["position"]).unwrap();
        let chest = world_box(obj(&doc, "chest"), &models);
        let to = [0, 1, 2].map(|k| chest.mid(k) - cam[k]);
        for id in ["man", "monster"] {
            let o = world_box(obj(&doc, id), &models);
            assert!(!meets_box(cam, to, o.min, o.max), "{id} hides the chest");
        }
        // They really face each other.
        let yaw = |id| obj(&doc, id)["rotation"][1].as_f64().unwrap();
        assert!((yaw("man") - toward(&doc, "man", "monster")).abs() < 0.1);
        assert!((yaw("monster") - toward(&doc, "monster", "man")).abs() < 0.1);
        // The key at the monster's neck, 0.2 m; the coins heaped on the chest, most of them above its top.
        let wb = |id| world_box(obj(&doc, id), &models);
        // The camera frames the action, the chest partly at its edge (the
        // first real run's camera left the group at 12 % of the frame).
        assert!(fill(&doc, &models) > 0.5, "{}", fill(&doc, &models));
        assert!((wb("key").size(0).max(wb("key").size(1)) - 0.2).abs() < 1e-3);
        assert!(
            (wb("key").max[1] - 2.0 * 2.7 / 2.5).abs() < 1e-3,
            "hangs from the neck bone"
        );
        let (coins, chest) = (wb("coins"), wb("chest"));
        assert!(
            (coins.size(0).max(coins.size(2)) - 0.6).abs() < 1e-3,
            "{coins:?}"
        );
        assert!(
            coins.min[1] > chest.min[1] && coins.max[1] < chest.max[1],
            "inside: {coins:?}"
        );
    }

    /// The real Forge outputs: a 0.9-unit character becomes a 1.8 m person
    /// with props at a plausible size beside it, all in view.
    #[test]
    fn real_forge_models_are_sized_spaced_and_framed() {
        let mut plan = Plan {
            title: "Knight".into(),
            items: vec![
                item("knight", "character"),
                item("chest", "object"),
                item("box", "object"),
                item("ogre", "character"),
            ],
            ..Default::default()
        };
        let mut boxed = place("box", "center");
        boxed.attach_to = Some(Attach {
            item: "chest".into(),
            at: String::new(),
        });
        boxed.scale_hint = Some("small".into());
        let mut ogre = place("ogre", "left");
        ogre.scale_hint = Some("big".into());
        ogre.facing = Some("knight".into());
        plan.layout = vec![
            place("knight", "center"),
            place("chest", "right"),
            boxed,
            ogre,
        ];
        let models = vec![
            model("knight", forge("character")),
            model("chest", forge("prop")),
            model("box", forge("prop")),
            model("ogre", forge("character")),
        ];
        let (doc, _) = build_scene(&plan, &models);
        check_layout(&doc, &models, &["box"]);
        on_top(&doc, &models, "box", "chest");
        let knight = world_box(obj(&doc, "knight"), &models);
        assert!((knight.size(1) - 1.8).abs() < 1e-3, "{knight:?}");
        let chest = world_box(obj(&doc, "chest"), &models);
        assert!(
            chest.size(1) < knight.size(1) / 2.0,
            "a prop is smaller than a person"
        );
    }
}
