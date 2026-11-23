//! Select and frame the default camera view from the placed models.
use super::*;

/// One placed piece as the camera sees it: where its surface is (see
/// [`Occupancy`]; its box's when its mesh is unknown), points on it to look
/// for with which way each faces, and its world box.
struct Piece {
    grid: Occupancy,
    samples: Vec<([f64; 3], [f64; 3])>,
    min: [f64; 3],
    max: [f64; 3],
}

const SAMPLES: usize = 16;

impl Piece {
    fn of(s: &Spot, tris: &[Tri]) -> Piece {
        let c = s.corners();
        let min = [0, 1, 2].map(|k| c.iter().map(|p| p[k]).fold(f64::MAX, f64::min));
        let max = [0, 1, 2].map(|k| c.iter().map(|p| p[k]).fold(f64::MIN, f64::max));
        let world: Vec<Tri> = match tris.is_empty() {
            true => {
                // The box's 12 triangles (corner k: bit 0 x, bit 1 y, bit 2 z), outward.
                const FACES: [[usize; 4]; 6] = [
                    [0, 2, 3, 1],
                    [4, 5, 7, 6],
                    [0, 1, 5, 4],
                    [2, 6, 7, 3],
                    [0, 4, 6, 2],
                    [1, 3, 7, 5],
                ];
                FACES
                    .iter()
                    .flat_map(|f| [[c[f[0]], c[f[1]], c[f[2]]], [c[f[0]], c[f[2]], c[f[3]]]])
                    .collect()
            }
            false => tris.iter().map(|t| t.map(|v| s.to_world(v))).collect(),
        };
        let step = world.len().div_ceil(SAMPLES).max(1);
        let samples = world
            .iter()
            .step_by(step)
            .map(|t| {
                let (a, b) = (
                    [0, 1, 2].map(|k| t[1][k] - t[0][k]),
                    [0, 1, 2].map(|k| t[2][k] - t[0][k]),
                );
                let n = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                ([0, 1, 2].map(|k| (t[0][k] + t[1][k] + t[2][k]) / 3.0), n)
            })
            .collect();
        Piece {
            grid: Occupancy::of(&world, min, max),
            samples,
            min,
            max,
        }
    }
}

/// Project in the compact opening view. The viewport centres each piece's
/// bounding box in perspective (scene3d/cameraFit.ts); the authored eye can
/// have a very different sightline to the feet of a small foreground animal.
pub(super) struct OpeningProjection {
    basis: [[f64; 3]; 3],
    pub(super) eye: [f64; 3], // in the view basis
}

impl OpeningProjection {
    pub(super) fn fit(corners: &[[f64; 3]], center: [f64; 3], basis: [[f64; 3]; 3]) -> Self {
        let coords = |p: [f64; 3]| basis.map(|b| p[0] * b[0] + p[1] * b[1] + p[2] * b[2]);
        let pts: Vec<_> = corners.iter().copied().map(coords).collect();
        let origin = coords(center);
        let mut eye = origin;
        let margin = 0.88 * (FOV / 2.).to_radians().tan();
        for pass in 0..=4 {
            eye[2] = pts
                .iter()
                .map(|p| p[2] + (p[0] - eye[0]).abs().max((p[1] - eye[1]).abs()) / margin)
                .fold(f64::NEG_INFINITY, f64::max);
            if pass == 4 {
                break;
            }
            let distance = eye[2] - origin[2];
            for k in 0..2 {
                let (lo, hi) = pts
                    .iter()
                    .map(|p| (p[k] - eye[k]) / (eye[2] - p[2]).max(1e-6))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                        (lo.min(x), hi.max(x))
                    });
                eye[k] += 0.5 * (lo + hi) * distance;
            }
        }
        Self { basis, eye }
    }

    pub(super) fn project(&self, p: [f64; 3]) -> [f64; 2] {
        let q = self.basis.map(|b| p[0] * b[0] + p[1] * b[1] + p[2] * b[2]);
        [0, 1].map(|k| (q[k] - self.eye[k]) / (self.eye[2] - q[2]).max(1e-6))
    }
}

/// Closest separation of two projected shins. Separated feet can still lie
/// at the ends of crossing segments, in which case the separation is zero.
pub(super) fn shin_gap(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> f64 {
    let sub = |p: [f64; 2], q: [f64; 2]| [p[0] - q[0], p[1] - q[1]];
    let cross = |p: [f64; 2], q: [f64; 2]| p[0] * q[1] - p[1] * q[0];
    let u = sub(a[1], a[0]);
    let v = sub(b[1], b[0]);
    let w = sub(b[0], a[0]);
    let den = cross(u, v);
    if den.abs() > 1e-14
        && (0.0..=1.0).contains(&(cross(w, v) / den))
        && (0.0..=1.0).contains(&(cross(w, u) / den))
    {
        return 0.;
    }
    let to_segment = |p: [f64; 2], line: [[f64; 2]; 2]| {
        let d = sub(line[1], line[0]);
        let r = sub(p, line[0]);
        let t =
            ((r[0] * d[0] + r[1] * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-15)).clamp(0., 1.);
        (r[0] - t * d[0]).hypot(r[1] - t * d[1])
    };
    to_segment(a[0], b)
        .min(to_segment(a[1], b))
        .min(to_segment(b[0], a))
        .min(to_segment(b[1], a))
}
/// Placed pairs in their requested left-to-right order in the opening view.
pub(super) fn side_by_side(
    layout: &[crate::productions::Placement],
    spots: &BTreeMap<&str, Spot>,
) -> Vec<(Spot, Spot)> {
    layout
        .iter()
        .filter_map(|p| {
            let r = p.rel.as_ref()?;
            if r.how != "beside" {
                return None;
            }
            let subject = *spots.get(p.item.as_str())?;
            let target = *spots.get(r.to.as_str())?;
            match r.side.as_deref() {
                Some("left") => Some((subject, target)),
                Some("right") => Some((target, subject)),
                _ => None,
            }
        })
        .collect()
}

/// When spectators watch the same subject from opposite sides, keep the
/// subject visible between them. A side view can put a spectator over its face.
fn spectator_order(
    layout: &[crate::productions::Placement],
    spots: &BTreeMap<&str, Spot>,
) -> Vec<(Spot, Spot)> {
    let mut watching = BTreeMap::<&str, [Vec<Spot>; 2]>::new();
    for p in layout {
        let (Some(subject), Some(&watcher)) = (p.facing.as_deref(), spots.get(p.item.as_str()))
        else {
            continue;
        };
        let side = match &p.rel {
            None => slots(&p.place).0,
            Some(r) if r.how == "beside" => {
                if r.to != subject && station_of(layout, subject) != Some(&r.to) {
                    continue;
                }
                r.side.as_deref().map(slots).unwrap_or_default().0
            }
            _ => continue,
        };
        match side {
            -1 => watching.entry(subject).or_default()[0].push(watcher),
            1 => watching.entry(subject).or_default()[1].push(watcher),
            _ => {}
        }
    }
    let mut order = Vec::new();
    for (id, [left, right]) in watching {
        if let Some(&subject) = spots
            .get(id)
            .filter(|_| !left.is_empty() && !right.is_empty())
        {
            order.extend(left.into_iter().map(|l| (l, subject)));
            order.extend(right.into_iter().map(|r| (subject, r)));
        }
    }
    order
}

/// The scene document: the placed models (`objects`), their groups, the
/// ground and the camera; with how much of each piece the camera sees (0–1)
/// and where it stands. The camera starts at `look.az0` and tries other
/// turns and heights: the view where the least-visible piece shows most
/// wins (rays from the camera to points on each piece, blocked by the other
/// pieces' triangles), the first one on a tie; each view frames the pieces
/// the same way. With a main character it aims at his chest from about eye
/// level (10–20° up, like a person standing near the scene), higher only
/// when that hides a piece.
pub(super) fn scene_doc(
    models: &[Model],
    spots: &BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
    names: &dyn Fn(&str) -> String,
    look: &Look,
) -> (Value, BTreeMap<String, f64>, [f64; 3], Vec<String>) {
    let r3 = |v: [f64; 3]| v.map(|x| (x * 1000.0).round() / 1000.0);
    let name = |id: &str| names(id);
    let mut objects: Vec<Value> = models
        .iter()
        .filter_map(|m| {
            let s = spots.get(m.item.as_str())?;
            Some(json!({
                "id": m.item, "name": name(&m.item), "type": "gltf",
                "src": match m.version {
                    Some(n) => format!("otto://design/{}@v{n}", m.artifact_id),
                    None => format!("otto://design/{}@latest", m.artifact_id),
                },
                "position": r3(s.tilt.map_or(s.pos(), |t| t.0)),
                "rotation": match (s.tilt, look.turns.get(&m.item)) {
                    (Some((_,e)), Some(fitted)) => r3(turn_of(compose_turns(turn_matrix(e), *fitted))),
                    (Some((_,e)), None) => r3(e),
                    (None, fitted) => rotation(s.rot_y, fitted),
                },
                "scale": s.scale,
            }))
        })
        .collect();
    let group_docs: Vec<Value> = groups
        .iter()
        .map(|(target, children)| {
            let children: Vec<&String> = std::iter::once(target).chain(children).collect();
            json!({ "id": format!("with-{target}"), "name": name(target), "children": children })
        })
        .collect();
    let (ids, pieces): (Vec<&str>, Vec<Piece>) = models
        .iter()
        .filter_map(|m| {
            let s = spots.get(m.item.as_str())?;
            Some((m.item.as_str(), Piece::of(s, &m.tris)))
        })
        .unzip();
    let pieces = &pieces;
    // Frame the pieces by their turned boxes; a backdrop several times bigger
    // than the rest (a meadow, a road, a room) is left out of the framing, so
    // it never shrinks the action to a speck.
    let span = |s: &Spot| s.size(0).max(s.size(1)).max(s.size(2));
    let mut spans: Vec<f64> = spots.values().map(span).collect();
    spans.sort_by(f64::total_cmp);
    let median = spans.get(spans.len() / 2).copied().unwrap_or(1.0);
    // A floor-like piece (flat, far wider than the tallest other) is the
    // ground the action stands on, not the action.
    let tallest = spots.values().map(|s| s.size(1)).fold(0.0, f64::max);
    // The cast is the action, never a backdrop (a grandmother among cups).
    let in_cast = |id: &str| look.cast.contains(&id);
    let backdrop = |id: &str, s: &Spot| {
        let flat = s.size(1) <= 0.1 * s.size(0).max(s.size(2));
        look.audience.contains(&id)
            || !in_cast(id)
                && spots.len() >= 3
                && (span(s) > 4.0 * median || (flat && span(s) > 3.0 * tallest))
    };
    let whole: Vec<&Spot> = spots
        .iter()
        .filter(|(id, s)| !backdrop(id, s))
        .map(|(_, s)| s)
        .collect();
    let whole = if whole.is_empty() {
        spots.values().collect()
    } else {
        whole
    };
    // With a cast, the camera frames the action (see [`cast_of`]); the rest
    // comes in only as far as it fits (below).
    let close = close_up(models, spots, groups, look.cast, look.characters);
    let framed: Vec<&Spot> = match look.cast.is_empty() {
        true => whole.clone(),
        false => spots
            .iter()
            .filter(|(id, _)| in_cast(id) && close.is_none_or(|c| c.0 != **id))
            .map(|(_, s)| s)
            .collect(),
    };
    let mut corners: Vec<[f64; 3]> = framed.iter().flat_map(|s| s.corners()).collect();
    if let Some((_, lo, hi, _)) = close {
        corners
            .extend((0..8).map(|c| [0, 1, 2].map(|k| if c >> k & 1 == 1 { hi[k] } else { lo[k] })));
    }

    let all = bounds(&corners);
    let mut target = [0, 1, 2].map(|k| all.mid(k));
    let chest = look
        .main
        .and_then(|id| spots.get(id))
        .map(|s| s.floor + 0.72 * s.size(1));
    if let Some(y) = chest {
        target[1] = y.clamp(all.min[1], all.max[1]);
    }

    let tan = 0.7 * (FOV / 2.0).to_radians().tan();
    // From a turn and a height: which way the camera looks from the target,
    // and how far back it stands so every point is inside 70 % of the view.

    let reach = |pts: &[[f64; 3]], az: f64, el: f64| {
        let (dir, a) = (dir_of(az, el), az.to_radians());
        let right = [a.cos(), 0.0, -a.sin()];
        let up = [
            dir[1] * right[2],
            dir[2] * right[0] - dir[0] * right[2],
            -dir[1] * right[0],
        ];
        pts.iter()
            .map(|c| {
                let q = [0, 1, 2].map(|k| c[k] - target[k]);
                dot(q, dir) + dot(q, right).abs().max(dot(q, up).abs()) / tan
            })
            .fold(0.1, f64::max)
    };
    let max_pitch = look.max_pitch.filter(|_| close.is_none()).unwrap_or(90.);
    let first_el = match chest {
        Some(_) => 15.0,
        None => VIEW[1].atan2(VIEW[0].hypot(VIEW[2])).to_degrees(),
    }
    .min(max_pitch);
    // Every other piece is in the first view: whole when that takes the
    // camera back no more than a third further, else by its part nearest
    // the action (a big base runs out of the frame) when that is within 4 m
    // of it or the camera goes no more than twice as far; else it is left
    // to looking around, said.
    let mut notes = Vec::new();
    let mut unframed: Vec<&str> = spots
        .iter()
        .filter(|(id, s)| backdrop(id, s))
        .map(|(id, _)| *id)
        .collect();
    if !look.cast.is_empty() {
        let base = reach(&corners, look.az0, first_el);
        let others: Vec<(&&str, &Spot)> = spots
            .iter()
            .filter(|(id, s)| !in_cast(id) && !backdrop(id, s))
            .collect();
        let near_of = |s: &Spot| {
            let c = s.corners();
            [0, 1, 2].map(|k| {
                let lo = c.iter().map(|p| p[k]).fold(f64::MAX, f64::min);
                let hi = c.iter().map(|p| p[k]).fold(f64::MIN, f64::max);
                target[k].clamp(lo, hi)
            })
        };
        let mut out = Vec::new();
        for (id, s) in &others {
            let near = near_of(s);
            if reach(&s.corners(), look.az0, first_el) <= 1.34 * base {
                corners.extend(s.corners());
                continue;
            }
            // A base the cast stands on, not framed whole: a viewer that fits
            // its own view leaves it out too (it may run out of the picture).
            if look
                .supports
                .iter()
                .any(|(child, support)| in_cast(child) && support == **id)
            {
                unframed.push(**id);
            }
            if reach(&[near], look.az0, first_el) <= 2.0 * base
                || (near[0] - target[0]).hypot(near[2] - target[2]) <= 4.0
            {
                let mid = [s.center[0], s.floor + s.size(1) / 2.0, s.center[1]];
                // Keep the middle of a nearby freestanding piece visible when
                // it fits the same distance budget. A large base can run out.
                corners.push(
                    if close.is_none()
                        && !unframed.contains(id)
                        && reach(&[mid], look.az0, first_el) <= 2.0 * base
                    {
                        mid
                    } else {
                        near
                    },
                );
            } else {
                out.push(**id);
                unframed.push(**id);
            }
        }
        // A piece in or on one that shows comes along with it.
        for &id in &out {
            let with = groups
                .iter()
                .any(|(t, kids)| kids.iter().any(|k| k == id) && !out.contains(&t.as_str()));
            match with {
                true => corners.push(near_of(&spots[id])),
                false => notes.push(format!(
                    "The {} is outside the first view; drag the view to look around.",
                    name(id)
                )),
            }
        }
    }
    let view = |az: f64, el: f64| {
        let (dir, dist) = (dir_of(az, el), reach(&corners, az, el));
        [0, 1, 2].map(|k| target[k] + dir[k] * dist)
    };
    // How much of each piece the camera sees.
    let seen_each = |cam: [f64; 3]| -> Vec<f64> {
        pieces
            .iter()
            .enumerate()
            .map(|(i, p)| {
                // The points on its side that faces the camera (all, if none do).
                let toward = |(q, n): &&([f64; 3], [f64; 3])| {
                    dot(*n, [0, 1, 2].map(|k| cam[k] - q[k])) > 0.0
                };
                let mut pts: Vec<&([f64; 3], [f64; 3])> = p.samples.iter().filter(toward).collect();
                if pts.is_empty() {
                    pts = p.samples.iter().collect();
                }
                // What it wears, holds or carries never hides it (a suit, a coat).
                // Nor does a fist hide what it holds, nor a stick held in one
                // what is on its end (a marshmallow).
                let own = |j: usize| {
                    let of = |a: usize, b: usize| {
                        groups
                            .get(ids[a])
                            .is_some_and(|k| k.iter().any(|c| c == ids[b]))
                    };
                    let fist = |a: usize| spots[ids[a]].tilt.is_some();
                    of(i, j) || (of(j, i) && (fist(i) || fist(j)))
                };
                let seen = pts.iter().filter(|(q, _)| {
                    let d = [0, 1, 2].map(|k| q[k] - cam[k]);
                    !pieces.iter().enumerate().any(|(j, o)| {
                        j != i
                            && !own(j)
                            && meets_box(cam, d, o.min, o.max)
                            && o.grid.crosses(cam, d)
                    })
                });
                seen.count() as f64 / pts.len().max(1) as f64
            })
            .collect()
    };
    // The camera looks for the cast and the pieces of size: a cup hidden
    // behind the teapot never lifts it to a map's view of the room.
    let counts = |i: usize| {
        !look.audience.contains(&ids[i])
            && (look.cast.is_empty() || in_cast(ids[i]) || span(&spots[ids[i]]) >= 0.4)
    };
    // The focus counts only when whole: anyone in front of it puts it below
    // what is enough.
    let focus = look.focus.and_then(|f| ids.iter().position(|id| *id == f));
    // Its whole width too, clear of everyone's box (a girl whose arms miss
    // the fire's middle still hides it), and its middle unhidden by anything.
    let middle_seen = |cam: [f64; 3], i: usize| {
        let p = &pieces[i];
        let c = [0, 1, 2].map(|k| (p.min[k] + p.max[k]) / 2.0);
        let (fx, fz) = (c[0] - cam[0], c[2] - cam[2]);
        let n = fx.hypot(fz).max(1e-6);
        let w = 0.4 * (p.max[0] - p.min[0]).max(p.max[2] - p.min[2]);
        let to = |s: f64, h: f64| {
            let y = p.min[1] + h * (p.max[1] - p.min[1]);
            [
                c[0] - fz / n * s * w - cam[0],
                y - cam[1],
                c[2] + fx / n * s * w - cam[2],
            ]
        };
        let d = [0, 1, 2].map(|k| c[k] - cam[k]);
        // A stick held into the fire is not in front of it.
        let touches = |o: &Piece| (0..3).all(|k| o.min[k] < p.max[k] && p.min[k] < o.max[k]);
        let wide = |o: &Piece| {
            [-1.0, 0.0, 1.0].iter().any(|s| {
                [0.5, 0.85]
                    .iter()
                    .any(|h| meets_box(cam, to(*s, *h), o.min, o.max))
            })
        };
        !pieces.iter().enumerate().any(|(j, o)| {
            j != i
                && ((meets_box(cam, d, o.min, o.max) && o.grid.crosses(cam, d))
                    || (in_cast(ids[j]) && !touches(o) && wide(o)))
        })
    };
    // A view that hides the focus loses to every view that shows it.
    let least = |cam: [f64; 3]| {
        let seen = seen_each(cam);
        let hid = focus.is_some_and(|i| seen[i] < 0.97 || !middle_seen(cam, i));
        (seen.into_iter().enumerate())
            .filter(|(i, _)| counts(*i))
            .fold(1.0f64, |m, (_, v)| m.min(v))
            - if hid { 1.0 } else { 0.0 }
    };
    let best = Opening {
        models,
        spots,
        groups,
        look,
        pieces,
        ids: &ids,
        target,
        close,
        unframed: &unframed,
        chest,
        max_pitch,
    }
    .choose(view, least, backdrop);
    // A tiny one on a palm held out: from just above the palm (lower down,
    // the hand hides what stands on it; higher up, his face leaves the top).
    let cam = close.and_then(|c| c.3).map_or(best, |az| view(az, 2.0));
    let seen: BTreeMap<String, f64> = ids
        .iter()
        .map(|id| id.to_string())
        .zip(seen_each(cam))
        .collect();
    // A soft ground half a metre past the group's boxes, so every piece
    // stands on something and the sun's shadow falls on it, never a slab that
    // dwarfs them (a viewer leaves a plane out of the framing).
    // A base whose legs were sunk: the ground reaches far enough that the
    // camera never sees under it.
    let sunk = spots.values().map(|s| -s.floor).fold(0.0, f64::max);
    let margin = 0.5 + 4.0 * sunk;
    let under: Vec<[f64; 3]> = whole.iter().flat_map(|s| s.corners()).collect();
    let under = bounds(&under);
    let (wide, deep) = (under.size(0) + 2.0 * margin, under.size(2) + 2.0 * margin);
    // Seen, not framed (`backdrop`): a viewer fitting its own view frames the rest.
    for o in objects.iter_mut() {
        if o["id"].as_str().is_some_and(|id| unframed.contains(&id)) {
            o["backdrop"] = json!(true);
        }
        if let Some((_, lo, hi, _)) = close.filter(|c| o["id"] == c.0) {
            (o["backdrop"], o["frame"]) = (json!(true), json!([r3(lo), r3(hi)]));
        }
    }
    objects.push(json!({
        "id": "ground", "name": "Ground", "type": "plane",
        "position": r3([under.mid(0), 0.0, under.mid(2)]), "rotation": [-90.0, 0.0, 0.0],
        "scale": [wide, deep, 1.0], "material": { "color": "#a39a8a", "roughness": 1.0 },
    }));
    let doc = json!({
        "type": "otto-scene3d", "version": 2, "grid": true,
        "camera": { "position": r3(cam), "target": r3(target), "fov": FOV },
        "environment": { "preset": "studio-soft" },
        "lights": [
            { "id": "sky", "type": "hemisphere", "intensity": 0.8 },
            { "id": "sun", "type": "directional", "position": [4.0, 8.0, 6.0], "intensity": 1.5, "shadow": true }
        ],
        "objects": objects,
        "groups": group_docs,
    });
    (doc, seen, cam, notes)
}

fn bounds(pts: &[[f64; 3]]) -> Bounds {
    pts.iter()
        .fold(None, |b: Option<Bounds>, c| {
            Some(b.map_or(Bounds { min: *c, max: *c }, |b| b.add(*c)))
        })
        .unwrap_or(Bounds::UNIT)
}

fn dir_of(az: f64, el: f64) -> [f64; 3] {
    let (a, e) = (az.to_radians(), el.to_radians());
    [a.sin() * e.cos(), e.sin(), a.cos() * e.cos()]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
struct LimbProjection {
    feet: Vec<(Vec<[f64; 3]>, bool)>,
    shins: Vec<Vec<[[f64; 3]; 2]>>,
    fit_corners: Vec<[f64; 3]>,
    fit_center: [f64; 3],
    fans: Vec<Vec<[f64; 3]>>,
}
fn limb_projection(
    models: &[Model],
    spots: &BTreeMap<&str, Spot>,
    close: Option<CloseUp<'_>>,
    unframed: &[&str],
) -> LimbProjection {
    let feet: Vec<_> = models
        .iter()
        .filter_map(|model| {
            let spot = spots.get(model.item.as_str())?;
            let prefix = if model.pose.is_none() && model.joints.contains_key("moving_foot/5") {
                "moving_foot/"
            } else {
                "foot/"
            };
            let tips: Vec<_> = model
                .joints
                .iter()
                .filter(|(name, _)| name.starts_with(prefix))
                .map(|(_, point)| spot.to_world(*point))
                .collect();
            (tips.len() >= 6).then_some((tips, prefix == "moving_foot/"))
        })
        .collect();
    let shins: Vec<Vec<_>> = models
        .iter()
        .filter(|m| m.pose.is_none())
        .filter_map(|m| {
            let spot = spots.get(m.item.as_str())?;
            let axes: Vec<_> = (0..m.joints.len() / 2)
                .map_while(|i| {
                    let a = m.joints.get(&format!("lower_leg_start/{i}"))?;
                    let b = m.joints.get(&format!("lower_leg_end/{i}"))?;
                    Some([spot.to_world(*a), spot.to_world(*b)])
                })
                .collect();
            (axes.len() >= 6).then_some(axes)
        })
        .collect();
    let mut fit_corners = Vec::new();
    if !shins.is_empty() {
        for m in models {
            let Some(spot) = spots.get(m.item.as_str()) else {
                continue;
            };
            let b = if let Some((_, min, max, _)) = close.filter(|c| c.0 == m.item) {
                Bounds { min, max }
            } else if unframed.contains(&m.item.as_str()) {
                continue;
            } else if let Some(first) = m.tris.first() {
                let p = spot.to_world(first[0]);
                m.tris
                    .iter()
                    .flatten()
                    .fold(Bounds { min: p, max: p }, |b, p| b.add(spot.to_world(*p)))
            } else {
                bounds(&spot.corners())
            };
            fit_corners.extend((0..8).map(|i| {
                [0, 1, 2].map(|k| {
                    if (i >> k) & 1 == 1 {
                        b.max[k]
                    } else {
                        b.min[k]
                    }
                })
            }));
        }
    }
    let fit_center = bounds(&fit_corners);
    let fit_center = [0, 1, 2].map(|k| fit_center.mid(k));
    let fans: Vec<Vec<_>> = models
        .iter()
        .filter_map(|m| {
            let spot = spots.get(m.item.as_str())?;
            let tips: Vec<_> = m
                .joints
                .iter()
                .filter(|(key, _)| key.starts_with("limb_tip/"))
                .map(|(_, point)| spot.to_world(*point))
                .collect();
            (tips.len() >= 4).then_some(tips)
        })
        .collect();
    LimbProjection {
        feet,
        shins,
        fit_corners,
        fit_center,
        fans,
    }
}
struct Opening<'a> {
    models: &'a [Model],
    spots: &'a BTreeMap<&'a str, Spot>,
    groups: &'a BTreeMap<String, Vec<String>>,
    look: &'a Look<'a>,
    pieces: &'a [Piece],
    ids: &'a [&'a str],
    target: [f64; 3],
    close: Option<CloseUp<'a>>,
    unframed: &'a [&'a str],
    chest: Option<f64>,
    max_pitch: f64,
}
impl Opening<'_> {
    fn choose(
        self,
        view: impl Fn(f64, f64) -> [f64; 3],
        least: impl Fn([f64; 3]) -> f64,
        backdrop: impl Fn(&str, &Spot) -> bool,
    ) -> [f64; 3] {
        let Self {
            models,
            spots,
            groups,
            look,
            pieces,
            ids,
            target,
            close,
            unframed,
            chest,
            max_pitch,
        } = self;
        let az0 = look.az0;
        // Passes of turns and heights, each kept only when it shows the
        // least-visible piece better: the face side first (with a character at
        // about eye level, then higher), the other side last (two side-on: the
        // other profile). A pass that shows every piece well enough ends it.
        let usual = VIEW[1].atan2(VIEW[0].hypot(VIEW[2])).to_degrees();
        let front: &[f64] = &[0.0, 20.0, -20.0, 40.0, -40.0, 60.0, -60.0];
        let back: &[f64] = &[180.0, 160.0, -160.0];
        // A group round a focus: a gap between them anywhere round it.
        let round: &[f64] = &[90.0, -90.0, 120.0, -120.0, 150.0, -150.0, 180.0];
        let LimbProjection {
            feet,
            shins,
            fit_corners,
            fit_center,
            fans,
        } = limb_projection(models, spots, close, unframed);
        let passes: Vec<(&[f64], Vec<f64>, f64)> = match (look.fixed, chest) {
            (true, Some(_)) => vec![(&[0.0], vec![look.fixed_pitch.unwrap_or(15.0)], 1.0)],
            (true, None) => vec![(&[0.0], vec![look.fixed_pitch.unwrap_or(usual)], 1.0)],
            (false, Some(_)) => vec![
                (
                    front,
                    if fans.is_empty() {
                        vec![15.0, 10.0, 20.0]
                    } else {
                        vec![15.0, 24.0, 35.0]
                    },
                    0.5,
                ),
                (
                    if look.focus.is_some() || !feet.is_empty() || !look.profile_subjects.is_empty()
                    {
                        round
                    } else {
                        &[]
                    },
                    vec![15.0, 28.0],
                    0.5,
                ),
                (front, vec![28.0, 38.0, 50.0, 65.0], 0.5),
                (if look.pair { back } else { &[] }, vec![15.0, 28.0], 1.0),
            ],
            (false, None) => vec![
                (
                    front,
                    vec![usual, usual + 12.0, usual - 10.0, usual + 25.0],
                    1.0,
                ),
                (back, vec![usual, usual + 12.0], 1.0),
            ],
        };
        let side_pairs = side_by_side(look.layout, spots);
        let spectator_pairs = spectator_order(look.layout, spots);
        let present_lineup: Vec<_> = look
            .lineup
            .iter()
            .filter_map(|id| spots.get(id.as_str()))
            .collect();
        // A foreground prop may overlap its partner at every angle. Score the players
        // separately so that prop never masks one performer hiding another.
        let lineup_pairs = present_lineup
            .windows(2)
            .map(|pair| (*pair[0], *pair[1]))
            .collect::<Vec<_>>();
        let score = |turn: f64, el: f64| {
            let cam = view(az0 + turn, el);
            // A backdrop (a room, an "Earth") between the camera and the
            // action would fill the picture: a view behind one counts less.
            let to = [0, 1, 2].map(|k| target[k] - cam[k]);
            let walled = spots
                .iter()
                .filter(|(id, s)| backdrop(id, s))
                .any(|(_, s)| {
                    let c = s.corners();
                    let lo = [0, 1, 2].map(|k| c.iter().map(|p| p[k]).fold(f64::MAX, f64::min));
                    let hi = [0, 1, 2].map(|k| c.iter().map(|p| p[k]).fold(f64::MIN, f64::max));
                    meets_box(cam, to, lo, hi)
                });
            // Show crossed forearms from the front. A supported lean also needs
            // enough of the character's side to see the shoulders shift over the feet.
            let gesture_loss = look
                .face_actions
                .iter()
                .filter_map(|(id, pose)| spots.get(id).map(|s| (s, pose)))
                .map(|(actor, pose)| {
                    let az = (cam[0] - actor.center[0]).atan2(cam[2] - actor.center[1]);
                    let (side, front) = (az - actor.rot_y.to_radians()).sin_cos();
                    let leaning = pose.kind == "lean";
                    let min_front = match (pose.arms.as_deref(), leaning) {
                        (Some("crossed"), true) => 0.55,
                        (Some("crossed"), false) => 0.85,
                        _ => 0.0,
                    };
                    let side_loss = if leaning {
                        (0.72 - side.abs()).max(0.0)
                    } else {
                        0.0
                    };
                    0.8 * ((min_front - front).max(0.0) + side_loss)
                })
                .fold(0.0, f64::max);
            let hidden_action = look
                .face_actions
                .iter()
                .filter_map(|(id, _)| spots.get(id).map(|s| (*id, s)))
                .map(|(id, actor)| {
                    [0.25, 0.58, 0.84]
                        .iter()
                        .filter(|frac| {
                            let p = [
                                actor.center[0],
                                actor.floor + **frac * actor.size(1),
                                actor.center[1],
                            ];
                            let d = [0, 1, 2].map(|k| p[k] - cam[k]);
                            pieces.iter().enumerate().any(|(i, obstacle)| {
                                ids[i] != id
                                    && !groups
                                        .get(id)
                                        .is_some_and(|kids| kids.iter().any(|c| c == ids[i]))
                                    && meets_box(cam, d, obstacle.min, obstacle.max)
                                    && obstacle.grid.crosses(cam, d)
                            })
                        })
                        .count() as f64
                        / 3.
                })
                .fold(0., f64::max);
            // A swimmer or a following animal seen end-on loses its
            // silhouette and its direction of travel. Prefer a visible flank.
            let profile_loss = look
                .profile_subjects
                .iter()
                .filter_map(|(id, heading)| {
                    let animal = spots.get(id)?;
                    let dx = cam[0] - animal.center[0];
                    let dz = cam[2] - animal.center[1];
                    let dy = cam[1] - animal.floor - animal.size(1) * 0.5;
                    let range = dx.hypot(dz).hypot(dy).max(0.01);
                    let (sn, cs) = (animal.rot_y + heading).to_radians().sin_cos();
                    // Looking down also shortens the visible flank.
                    let profile = (dx * cs - dz * sn).abs() / range;
                    let length = animal.size(0).max(animal.size(2));
                    let fraction =
                        length * profile / (2.0 * range * (FOV / 2.0).to_radians().tan());
                    Some(
                        (1.0 - fraction / 0.10)
                            .clamp(0.0, 1.0)
                            .max((0.7 - profile).max(0.0)),
                    )
                })
                .fold(0.0, f64::max);
            // Six or more legs can all be unobstructed by other actors yet
            // hide each other. Prefer an opening with space between their tips.
            let (sn, cs) = (az0 + turn).to_radians().sin_cos();
            let dir = dir_of(az0 + turn, el);
            let across = |spot: &Spot| {
                spot.corners()
                    .iter()
                    .map(|p| {
                        let d = [0, 1, 2].map(|k| p[k] - cam[k]);
                        (d[0] * cs - d[2] * sn) / (-dot(d, dir)).max(0.01)
                    })
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                        (lo.min(x), hi.max(x))
                    })
            };
            // Props, adjacent players and spectators must stay in their requested screen order.
            // A side view can separate their silhouettes while reversing the middle pair.
            let order_loss = spectator_pairs
                .iter()
                .chain(&lineup_pairs)
                .map(|(left, right)| {
                    let (a, b) = across(left);
                    let (c, d) = across(right);
                    ((b - c) / (b - a).min(d - c).max(1e-6)).clamp(0., 2.)
                })
                .fold(0., f64::max);
            // Adjacent objects may overlap in perspective. Keep their centres in the
            // requested order, leaving a visible offset even for a small companion.
            let beside_loss = side_pairs
                .iter()
                .map(|(left, right)| {
                    let (a, b) = across(left);
                    let (c, d) = across(right);
                    let separation = (c + d - a - b) * 0.5 / (b - a).min(d - c).max(1e-6);
                    (0.5 - separation).clamp(0.0, 2.0)
                })
                .fold(0., f64::max);
            // An arm fan seen edge-on collapses into a single line. Measure
            // both dimensions in the actual image, including perspective.
            let up = [-sn * dir[1], el.to_radians().cos(), -cs * dir[1]];
            let fan_loss = fans
                .iter()
                .map(|tips| {
                    let points: Vec<_> = tips
                        .iter()
                        .map(|p| {
                            let d = [0, 1, 2].map(|k| p[k] - cam[k]);
                            let depth = (-dot(d, dir)).max(0.01);
                            [(d[0] * cs - d[2] * sn) / depth, dot(d, up) / depth]
                        })
                        .collect();
                    // The minor and major axes of the projected scatter are
                    // independent of the angle the fan makes with the screen.
                    let mean = [0, 1]
                        .map(|k| points.iter().map(|p| p[k]).sum::<f64>() / points.len() as f64);
                    let (mut xx, mut xy, mut yy) = (0., 0., 0.);
                    for p in points {
                        let (x, y) = (p[0] - mean[0], p[1] - mean[1]);
                        xx += x * x;
                        xy += x * y;
                        yy += y * y;
                    }
                    let spread = (xx - yy).hypot(2. * xy);
                    let aspect =
                        ((xx + yy - spread).max(0.) / (xx + yy + spread).max(1e-12)).sqrt();
                    (1.0 - aspect / 0.50).max(0.)
                })
                .fold(0., f64::max);
            let foot_overlap = feet
                .iter()
                .map(|(tips, in_motion)| {
                    let projected: Vec<_> = tips
                        .iter()
                        .map(|p| {
                            let d = [0, 1, 2].map(|k| p[k] - cam[k]);
                            let depth = (-dot(d, dir)).max(0.01);
                            // A lifted foot is visible above a planted one even
                            // when they lie at the same horizontal coordinate.
                            [
                                (d[0] * cs - d[2] * sn) / depth,
                                if *in_motion { dot(d, up) / depth } else { 0.0 },
                            ]
                        })
                        .collect();
                    let required =
                        (if *in_motion { 0.03 } else { 0.02 }) * (FOV / 2.).to_radians().tan();
                    let mut points = projected;
                    points.sort_by(|a, b| a[0].total_cmp(&b[0]));
                    let (mut worst, mut sum) = (0.0_f64, 0.0);
                    for (i, a) in points.iter().enumerate() {
                        for b in &points[i + 1..] {
                            let dx = b[0] - a[0];
                            if dx >= required {
                                break;
                            }
                            let loss = (1. - dx.hypot(b[1] - a[1]) / required).max(0.);
                            worst = worst.max(loss);
                            sum += loss;
                            if !*in_motion {
                                break;
                            }
                        }
                    }
                    worst + sum / (tips.len() - 1) as f64
                })
                .fold(0., f64::max);
            let seen = least(cam)
                - order_loss
                - beside_loss
                - fan_loss
                - foot_overlap
                - profile_loss
                - 1.3 * hidden_action
                - gesture_loss
                - if look.face_actions.is_empty() {
                    0.0
                } else {
                    0.018 * (el - 15.0).max(0.0)
                }
                - if walled { 0.5 } else { 0.0 };
            (seen, cam)
        };
        let mut best = (f64::MIN, view(az0, passes[0].1[0]));
        let mut best_angles = (0., 0.);
        'search: for (turns, els, enough) in passes {
            for turn in turns {
                for (index, &candidate) in els.iter().enumerate() {
                    // Evaluate the elevation that will actually be shown in this setting.
                    if candidate >= max_pitch && els[..index].iter().any(|e| *e >= max_pitch) {
                        continue;
                    }
                    let el = candidate.min(max_pitch);
                    let (seen, cam) = score(*turn, el);
                    if seen > best.0 + 1e-9 {
                        best = (seen, cam);
                        best_angles = (*turn, el);
                    }
                    if seen >= 1.0 - 1e-9 {
                        break 'search;
                    }
                }
            }
            if best.0 >= enough {
                break;
            }
        }
        // Refine the winning composition locally. A distant angle that happens
        // to separate all the feet can hide the other characters or their action.
        if (!look.fixed || look.fixed_pitch.is_some()) && !fit_corners.is_empty() {
            let (turn, el) = best_angles;
            let mut best_gap = -1.;
            let scene_score = best.0;
            // Elevation separates a lifted far foot from the near shin. Try a
            // small change in both directions, within the setting's sky limit.
            let separation = |candidate: f64, el: f64| {
                let az = az0 + candidate;
                let (sn, cs) = az.to_radians().sin_cos();
                let dir = dir_of(az, el);
                let up = [-sn * dir[1], el.to_radians().cos(), -cs * dir[1]];
                let fit =
                    OpeningProjection::fit(&fit_corners, fit_center, [[cs, 0., -sn], up, dir]);
                shins
                    .iter()
                    .flat_map(|axes| {
                        let lines: Vec<_> = axes
                            .iter()
                            .map(|axis| axis.map(|p| fit.project(p)))
                            .collect();
                        (0..lines.len()).map(move |i| {
                            lines[..i]
                                .iter()
                                .map(|&line| shin_gap(line, lines[i]))
                                .fold(f64::INFINITY, f64::min)
                        })
                    })
                    .fold(f64::INFINITY, f64::min)
            };
            for &rise in if look.fixed_pitch.is_some() {
                &[0., -4., 4., -8., 8.][..]
            } else {
                &[0.][..]
            } {
                let pitch = el + rise;
                if !(4.0..=max_pitch).contains(&pitch) {
                    continue;
                }
                for offset in [0., 2., -2., 4., -4., 6., -6., 8., -8., 10., -10., 12., -12.] {
                    let candidate = turn + offset;
                    let gap = separation(candidate, pitch);
                    // Only build visibility rays for a view that can improve the leg spacing.
                    if gap > best_gap + 1e-8 {
                        let (seen, cam) = score(candidate, pitch);
                        // Visibility is sampled from at most SAMPLES faces. On a
                        // refitted view, two samples can straddle an arm or a held prop.
                        let tolerance = if look.fixed_pitch.is_some() {
                            2.0 / SAMPLES as f64
                        } else {
                            0.10
                        };
                        if seen >= scene_score - tolerance {
                            best_gap = gap;
                            best = (seen, cam);
                        }
                    }
                }
            }
        }
        best.1
    }
}
