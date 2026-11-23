//! Align performers with their instruments, supports and fellow actors.
use super::*;

/// Measure the narrow bare part of the shaft, below any cloth or handle.
pub(in crate::productions) fn shaft_axis(model: &Model) -> [f64; 2] {
    let b = model.bounds;
    let low = b.min[1] + 0.10 * b.size(1);
    let high = b.min[1] + 0.48 * b.size(1);
    [0, 2].map(|k| {
        let mut coords: Vec<_> = model
            .tris
            .iter()
            .flatten()
            .filter(|v| v[1] >= low && v[1] <= high)
            .map(|v| v[k])
            .collect();
        if coords.is_empty() {
            return b.mid(k);
        }
        let mid = coords.len() / 2;
        *coords.select_nth_unstable_by(mid, f64::total_cmp).1
    })
}

pub(super) fn at_planted_shafts(
    plan: &Plan,
    models: &[Model],
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    for it in plan.items.iter().filter(|i| i.kind == "character") {
        let Some(id) = crate::productions::pose::effective(plan, it).and_then(|p| p.plant) else {
            continue;
        };
        let (Some(mut actor), Some(post), Some(model), Some(shaft)) = (
            spots.get(it.id.as_str()).copied(),
            spots.get(id.as_str()).copied(),
            models.iter().find(|m| m.item == it.id),
            models.iter().find(|m| m.item == id),
        ) else {
            continue;
        };
        let [x, z] = shaft_axis(shaft);
        let point = post.to_world([x, post.b.min[1], z]);
        let h = model.bounds.size(1) * actor.scale[1];
        let was = actor;
        let side = if actor.center[0] <= point[0] { -1. } else { 1. };
        actor.rot_y = -side * 24.;
        // The arm solver can reach a slightly different point than requested.
        // Keep the pole planted and shift the performer to the measured fist.
        let grip = match model.pose.and_then(|p| p.shaft) {
            Some(hand) => actor.to_world(hand),
            None => {
                let o = actor.pos();
                [o[0] - side * 0.22 * h, o[1], o[2] + 0.21 * h]
            }
        };
        actor.center[0] += point[0] - grip[0];
        actor.center[1] += point[2] - grip[2];
        *spots.get_mut(it.id.as_str()).expect("placed planter") = actor;
        carry(spots, groups, &[], &it.id, was);

        // Turn loose cloth away from the planter's face about the shaft,
        // preserving the measured contact with the fist.
        let flag = plan.items.iter().find(|i| i.id == id).is_some_and(|i| {
            words(&i.name)
                .iter()
                .any(|w| matches!(w.as_str(), "flag" | "banner" | "pennant" | "flagpole"))
        });
        if flag
            && post.tilt.is_none()
            && !plan.layout.iter().any(|p| {
                p.attach_to.as_ref().is_some_and(|a| a.item == id)
                    || p.rel.as_ref().is_some_and(|r| {
                        r.to == id && matches!(r.how.as_str(), "on" | "in" | "worn" | "held")
                    })
            })
            && !plan
                .layout
                .iter()
                .any(|p| p.item == id && p.facing.is_some())
        {
            let upper = shaft.bounds.min[1] + 0.55 * shaft.bounds.size(1);
            let mut extent = [0., 0.];
            let mut weight = 0.;
            for p in shaft.tris.iter().flatten().filter(|p| p[1] > upper) {
                let (dx, dz) = ((p[0] - x) * post.scale[0], (p[2] - z) * post.scale[2]);
                let w = dx.hypot(dz);
                extent[0] += dx * w;
                extent[1] += dz * w;
                weight += w;
            }
            if weight > 0. && extent[0].hypot(extent[1]) / weight > 0.08 * post.size(1) {
                let away = (point[0] - actor.center[0]).atan2(point[2] - actor.center[1]);
                let turned = spots.get_mut(id.as_str()).expect("placed flag");
                turned.rot_y = (away - extent[0].atan2(extent[1])).to_degrees();
                let pivot = turned.to_world([x, post.b.min[1], z]);
                turned.center[0] += point[0] - pivot[0];
                turned.center[1] += point[2] - pivot[2];
            }
        }
    }
}

/// A trailing animal shares its leader's heading. Leave room to see the feet
/// beyond the leader's silhouette while keeping it behind the leader.
pub(super) fn place_followers<'a>(
    plan: &'a Plan,
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) -> Vec<(&'a str, f64)> {
    let mut profiles = Vec::new();
    for it in plan.items.iter().filter(|i| i.kind == "character") {
        let action = words(&it.description);
        let follows = action
            .iter()
            .any(|w| matches!(w.as_str(), "follow" | "following" | "trail" | "trailing"))
            || action.windows(2).any(|pair| {
                pair[1] == "behind"
                    && matches!(
                        pair[0].as_str(),
                        "trot" | "trotting" | "walk" | "walking" | "run" | "running"
                    )
            });
        if !follows {
            continue;
        }
        let Some(rel) = plan
            .layout
            .iter()
            .find(|p| p.item == it.id)
            .and_then(|p| p.rel.as_ref())
            .filter(|r| r.how == "behind")
        else {
            continue;
        };
        if !plan
            .items
            .iter()
            .any(|i| i.id == rel.to && i.kind == "character")
        {
            continue;
        }
        let (Some(leader), Some(mut me)) = (
            spots.get(rel.to.as_str()).copied(),
            spots.get(it.id.as_str()).copied(),
        ) else {
            continue;
        };
        let was = me;
        let heading = leader.rot_y + rest_heading(plan, &rel.to);
        let a = heading.to_radians();
        let back = (leader.size(2) + me.size(2)) * 0.58;
        let side = (leader.size(0) + me.size(0)) * 0.66;
        let candidate = |sign: f64| {
            [
                leader.center[0] - a.sin() * back + sign * a.cos() * side,
                leader.center[1] - a.cos() * back - sign * a.sin() * side,
            ]
        };
        let clearance = |q: [f64; 2]| {
            spots
                .iter()
                .filter(|(id, s)| {
                    **id != it.id && **id != rel.to && s.floor < leader.top() && s.top() > me.floor
                })
                .map(|(_, s)| (s.center[0] - q[0]).hypot(s.center[1] - q[1]) - s.radius())
                .fold(f64::MAX, f64::min)
        };
        let left = candidate(-1.);
        let right = candidate(1.);
        me.center = if clearance(left) >= clearance(right) {
            left
        } else {
            right
        };
        let mesh_heading = rest_heading(plan, &it.id);
        me.rot_y = heading - mesh_heading;
        profiles.push((it.id.as_str(), mesh_heading));
        *spots.get_mut(it.id.as_str()).expect("placed follower") = me;
        carry(spots, groups, &[], &it.id, was);
    }
    profiles
}

/// The layout can place a musician beside an instrument, or the instrument
/// beside the musician. Keep the parent at its station and move the child
/// into playing distance, together with anything mounted on it.
fn place_at_instrument(
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
    musician: &str,
    instrument: &str,
    mut actor: Spot,
) {
    let was = spots[musician];
    if groups
        .get(musician)
        .is_some_and(|kids| kids.iter().any(|id| id == instrument))
    {
        let anchor = spots[instrument];
        let shift = [0, 1].map(|k| was.center[k] - actor.center[k]);
        let piece = spots.get_mut(instrument).expect("placed instrument");
        piece.center = [0, 1].map(|k| anchor.center[k] + shift[k]);
        actor.center = was.center;
        carry(spots, groups, &[], instrument, anchor);
    }
    *spots.get_mut(musician).expect("placed musician") = actor;
    carry(spots, groups, &[], musician, was);
}

/// Keep the drums within reach of the arms or held sticks that play them.
pub(super) fn at_percussion(
    plan: &Plan,
    models: &[Model],
    surfaces: &BTreeMap<&str, Surface>,
    sticks: &BTreeMap<&str, drumsticks::Stick>,
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    for it in plan.items.iter().filter(|i| i.kind == "character") {
        let Some(pose) = crate::productions::pose::effective(plan, it) else {
            continue;
        };
        let Some(id) = pose.play.as_deref().or_else(|| {
            let grip = pose.grip.as_ref()?;
            grip.toward
                .as_deref()
                .or_else(|| grip.other.as_ref()?.toward.as_deref())
        }) else {
            continue;
        };
        if !plan
            .items
            .iter()
            .any(|i| i.id == id && crate::productions::pose::drum_kit(i))
        {
            continue;
        }
        let (Some(kit), Some(mut actor)) =
            (spots.get(id).copied(), spots.get(it.id.as_str()).copied())
        else {
            continue;
        };
        let dir = [
            actor.center[0] - kit.center[0],
            actor.center[1] - kit.center[1],
        ];
        let distance = dir[0].hypot(dir[1]);
        if distance < 0.001 {
            continue;
        }
        let dir = dir.map(|v| v / distance);
        let half = kit.half();
        let d = (half[0] * dir[0]).abs()
            + (half[1] * dir[1]).abs()
            + if it.details.body.as_deref() == Some("many_legs") {
                0.18 * actor.size(2)
            } else {
                0.28 * actor.size(1)
            };
        actor.center = [0, 1].map(|k| kit.center[k] + dir[k] * d.min(distance));
        if let Some(playing) = drumsticks::at_heads(plan, &it.id, id, models, spots, sticks) {
            actor = playing;
        }
        // Curling a tentacle moves the centre of the posed bounding box. Keep
        // the two striking tips over the same spots on the kit after refitting.
        let mut lift = 0.0_f64;
        if let Some(receipt) = it
            .result
            .as_ref()
            .and_then(|r| r.pose.as_ref())
            .map(|r| &r["manifest"])
            .filter(|r| r["drum_contact"]["kit"] == id)
        {
            let tips = &receipt["contact"]["sticks"];
            if let (Some(xz), Some(a), Some(b)) = (
                nums::<2>(&receipt["drum_contact"]["xz"]),
                nums::<3>(&tips[0]),
                nums::<3>(&tips[1]),
            ) {
                let target = kit.to_world([xz[0], 0., xz[1]]);
                let middle = actor.to_world([0, 1, 2].map(|k| (a[k] + b[k]) / 2.));
                actor.center[0] += target[0] - middle[0];
                actor.center[1] += target[2] - middle[2];
                // An underwater performer can float. After the horizontal refit,
                // put its downstroke on the actual playing surface underneath it.
                if let Some(surface) = surfaces.get(id).filter(|_| {
                    it.details.body.as_deref() == Some("many_legs")
                        && plan
                            .setting
                            .as_ref()
                            .is_some_and(|s| s.world == "underwater")
                }) {
                    for tip in [a, b].map(|p| actor.to_world(p)) {
                        let [x, z] = kit.to_model([tip[0], tip[2]]);
                        if let Some(top) = surface.top_at(x, z) {
                            lift = lift.max(kit.world_y(top) - tip[1]);
                        }
                    }
                }
            }
        }
        place_at_instrument(spots, groups, &it.id, id, actor);
        if lift > 0.0 {
            let was = spots[it.id.as_str()];
            spots
                .get_mut(it.id.as_str())
                .expect("placed musician")
                .floor += lift;
            carry(spots, groups, &[id], &it.id, was);
        }
    }
}

/// Set a telescoping stand to the singer's mouth and bring the singer up to the capsule.
pub(super) fn at_microphones(
    plan: &Plan,
    models: &[Model],
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    for it in plan.items.iter().filter(|i| i.kind == "character") {
        let Some(id) = crate::productions::pose::effective(plan, it).and_then(|p| p.sing_into)
        else {
            continue;
        };
        let Some(stand_id) = plan
            .layout
            .iter()
            .find(|p| p.item == id)
            .and_then(|p| p.rel.as_ref())
            .filter(|r| r.how == "on")
            .map(|r| r.to.as_str())
        else {
            continue;
        };
        if !plan
            .items
            .iter()
            .any(|i| i.id == stand_id && words(&i.name).iter().any(|w| w == "stand"))
        {
            continue;
        }
        let (Some(mut stand), Some(mut mic), Some(mut actor), Some(model)) = (
            spots.get(stand_id).copied(),
            spots.get(id.as_str()).copied(),
            spots.get(it.id.as_str()).copied(),
            models.iter().find(|m| m.item == it.id),
        ) else {
            continue;
        };
        let h = model.bounds.size(1) * actor.scale[1];
        let mouth = actor.to_world(model.pose.and_then(|p| p.mouth).unwrap_or_else(|| {
            let b = model.bounds;
            [b.mid(0), b.min[1] + 0.82 * b.size(1), b.max[2] * 0.88]
        }));
        let mount = microphone_mount(models, stand_id, stand);
        let height = mount[1] - stand.floor;
        if height <= 1e-3 {
            continue;
        }
        stand.scale[1] *=
            ((mouth[1] - 0.65 * mic.size(1) - stand.floor) / height).clamp(0.65, 1.65);
        let mount = microphone_mount(models, stand_id, stand);
        mic.center = [mount[0], mount[2]];
        mic.floor = mount[1];
        let yaw = actor.rot_y.to_radians();
        let gap = (0.015 * h + 0.45 * mic.size(2)).clamp(0.01, 0.14);
        actor.center[0] += mount[0] - gap * yaw.sin() - mouth[0];
        actor.center[1] += mount[2] - gap * yaw.cos() - mouth[2];
        *spots.get_mut(stand_id).expect("placed stand") = stand;
        *spots.get_mut(id.as_str()).expect("placed microphone") = mic;
        place_at_instrument(spots, groups, &it.id, stand_id, actor);
    }
}

/// A floor-standing bass or cello sits between its player's shoulders and the audience.
pub(super) fn at_upright_strings(
    plan: &Plan,
    models: &[Model],
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    for it in plan.items.iter().filter(|i| i.kind == "character") {
        let Some(id) = crate::productions::pose::effective(plan, it).and_then(|p| p.play) else {
            continue;
        };
        if !plan
            .items
            .iter()
            .any(|i| i.id == id && crate::productions::pose::upright_strings(i))
        {
            continue;
        }
        let (Some(bass), Some(mut actor), Some(model)) = (
            spots.get(id.as_str()).copied(),
            spots.get(it.id.as_str()).copied(),
            models.iter().find(|m| m.item == it.id),
        ) else {
            continue;
        };
        let h = model.bounds.size(1) * actor.scale[1];
        let theta = bass.rot_y.to_radians();
        let side = if (actor.center[0] - bass.center[0]) * theta.cos()
            - (actor.center[1] - bass.center[1]) * theta.sin()
            > 0.
        {
            1.
        } else {
            -1.
        };
        // The left hand stops the neck and the right hand plucks. When the
        // instrument is on the player's left, bring the far shoulder around
        // the body of the instrument so the right hand reaches its strings.
        let (offset, setback, turn) = if side < 0. {
            (0.18, 0.11, 70.)
        } else {
            (0.16, 0.17, 25.)
        };
        let lateral = side * (offset * h).max(0.45 * bass.size(0));
        let behind = setback * h;
        // Use the resting model's origin. Its posed bounds can be asymmetric.
        actor.rot_y = bass.rot_y - side * turn;
        let origin = actor.pos();
        let desired = [
            bass.center[0] + lateral * theta.cos() - behind * theta.sin(),
            bass.center[1] - lateral * theta.sin() - behind * theta.cos(),
        ];
        actor.center[0] += desired[0] - origin[0];
        actor.center[1] += desired[1] - origin[2];
        place_at_instrument(spots, groups, &it.id, id.as_str(), actor);
    }
}

/// Stand with the back supported by the named mesh. Cast from the posed
/// hips, waist and shoulders; the first point to touch stops the approach.
pub(super) fn lean_against(
    plan: &Plan,
    models: &[Model],
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
) {
    for m in models {
        let Some(back) = m.pose.and_then(|p| p.back) else {
            continue;
        };
        let Some(id) = plan
            .items
            .iter()
            .find(|it| it.id == m.item)
            .and_then(|it| it.details.pose.as_ref())
            .filter(|p| p.kind == "lean")
            .and_then(|p| p.on.as_deref())
        else {
            continue;
        };
        let (Some(target), Some(mut me), Some(support)) = (
            models.iter().find(|m| m.item == id),
            spots.get(m.item.as_str()).copied(),
            spots.get(id).copied(),
        ) else {
            continue;
        };
        let was = me;
        let (x, z) = (
            support.center[0] - me.center[0],
            support.center[1] - me.center[1],
        );
        if x.hypot(z) < 1e-5 {
            continue;
        }
        let away = (-x).atan2(-z).to_degrees();
        // Rest one shoulder against the object while leaving the chest open
        // to the room. The subsequent traces still enforce real contact.
        me.rot_y = away - away.clamp(-32.0, 32.0);
        let trace = [2.0 * x, 0.0, 2.0 * z];
        let points = back.map(|p| me.to_world(p));
        let hit = target
            .tris
            .iter()
            .filter_map(|t| {
                let tri = t.map(|v| support.to_world(v));
                points
                    .iter()
                    .filter_map(|p| crate::productions::geometry::hit_distance(*p, trace, &tri))
                    .min_by(f64::total_cmp)
            })
            .min_by(f64::total_cmp);
        if let Some(t) = hit {
            me.center = [me.center[0] + trace[0] * t, me.center[1] + trace[2] * t];
            *spots.get_mut(m.item.as_str()).expect("placed character") = me;
            carry(spots, groups, &[], &m.item, was);
        }
    }
}
