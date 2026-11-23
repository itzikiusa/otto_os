//! A parked vehicle can turn to show its measured wheelbase. Keep a moving,
//! directed or loaded vehicle in the orientation specified by its plan.
use super::{BTreeMap, Model, OpeningProjection, Plan, Spot, GAP};

pub(super) fn arrange(
    plan: &Plan,
    models: &[Model],
    spots: &mut BTreeMap<&str, Spot>,
    groups: &BTreeMap<String, Vec<String>>,
    eye: [f64; 3],
    target: [f64; 3],
) -> Option<(f64, f64)> {
    if !models
        .iter()
        .any(|m| m.joints.contains_key("wheel_contact/0"))
    {
        return None;
    }
    let offset = [0, 1, 2].map(|k| eye[k] - target[k]);
    let az = offset[0].atan2(offset[2]);
    let pitch = offset[1].atan2(offset[0].hypot(offset[2]));
    let (sn, cs) = az.sin_cos();
    let (sa, ca) = pitch.sin_cos();
    let right = [cs, 0., -sn];
    let up = [-sn * sa, ca, -cs * sa];
    let back = [sn * ca, sa, cs * ca];
    let dot = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| a[k] * b[k]).sum::<f64>();
    // Use the fitted opening camera. The compact viewport centres the group
    // and can see a small foreground figure from a different direction.
    let corners: Vec<_> = spots.values().flat_map(Spot::corners).collect();
    let center = [0, 1, 2].map(|k| {
        let (lo, hi) = corners
            .iter()
            .map(|p| p[k])
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            });
        (lo + hi) * 0.5
    });
    let basis = [right, up, back];
    let fit = OpeningProjection::fit(&corners, center, basis);
    let eye = [0, 1, 2].map(|k| {
        (0..3)
            .map(|axis| basis[axis][k] * fit.eye[axis])
            .sum::<f64>()
    });
    let project = |p: [f64; 3]| {
        let d = [0, 1, 2].map(|k| p[k] - eye[k]);
        let depth = (-dot(d, back)).max(0.01);
        ([dot(d, right) / depth, dot(d, up) / depth], depth)
    };
    let mut moved = false;
    for model in models {
        let contacts: Vec<_> = model
            .joints
            .iter()
            .filter(|(k, _)| k.starts_with("wheel_contact/"))
            .map(|(_, p)| *p)
            .collect();
        if !matches!(contacts.len(), 4 | 6 | 8) {
            continue;
        }
        let Some(item) = plan
            .items
            .iter()
            .find(|i| i.id == model.item && i.kind == "object")
        else {
            continue;
        };
        let Some(original) = spots.get(model.item.as_str()).copied() else {
            continue;
        };
        if original.tilt.is_some()
            || original.floor.abs() > 0.001
            || item.details.attach_to.is_some()
            || !item.details.moves.is_empty()
            || plan.layout.iter().any(|p| {
                p.item == item.id
                    && (p.facing.is_some() || p.attach_to.is_some() || p.rel.is_some())
            })
        {
            continue;
        }
        // Move only an independent floor prop. A person driving it, leaning on
        // its hood, or facing it depends on the vehicle's existing pose.
        let neighbours: Vec<_> = plan
            .layout
            .iter()
            .filter(|p| p.rel.as_ref().is_some_and(|r| r.to == item.id))
            .collect();
        if neighbours.len() > 1
            || groups
                .get(&item.id)
                .is_some_and(|v| v.len() != neighbours.len())
        {
            continue;
        }
        let neighbour = neighbours.first().copied();
        let side = match neighbour.and_then(|n| n.rel.as_ref()) {
            Some(r) if r.how == "beside" && r.side.as_deref() == Some("left") => 1.,
            Some(r) if r.how == "beside" && r.side.as_deref() == Some("right") => -1.,
            None => 0.,
            _ => continue,
        };
        let contact = |i: &super::Item, id: &str| {
            i.details.holds.iter().any(|h| h.item == id)
                || i.details.pose.as_ref().is_some_and(|p| {
                    [
                        p.plant.as_deref(),
                        p.play.as_deref(),
                        p.sing_into.as_deref(),
                        p.on.as_deref(),
                    ]
                    .contains(&Some(id))
                })
        };
        if plan.items.iter().any(|i| {
            contact(i, &item.id)
                || i.details.pose.as_ref().and_then(|p| p.look_at.as_ref()) == Some(&item.id)
        }) || plan
            .layout
            .iter()
            .any(|p| p.facing.as_ref() == Some(&item.id))
        {
            continue;
        }
        let actor = neighbour.and_then(|n| {
            plan.items
                .iter()
                .find(|i| i.kind == "character" && contact(i, &n.item))
        });
        let group = neighbour
            .iter()
            .map(|n| n.item.as_str())
            .chain(actor.iter().map(|a| a.id.as_str()))
            .chain(
                actor
                    .iter()
                    .flat_map(|a| a.details.holds.iter().map(|h| h.item.as_str())),
            )
            .chain(
                plan.layout
                    .iter()
                    .filter(|p| {
                        actor.is_some_and(|a| {
                            p.rel
                                .as_ref()
                                .is_some_and(|r| r.to == a.id && r.how == "worn")
                        })
                    })
                    .map(|p| p.item.as_str()),
            );
        let edge = group
            .filter_map(|id| {
                let s = spots.get(id)?;
                let geom = models.iter().find(|m| m.item == id)?;
                let screen = |p| side * project(p).0[0];
                Some(if geom.tris.is_empty() {
                    s.corners()
                        .into_iter()
                        .map(screen)
                        .fold(f64::NEG_INFINITY, f64::max)
                } else {
                    geom.tris
                        .iter()
                        .flatten()
                        .map(|p| screen(s.to_world(*p)))
                        .fold(f64::NEG_INFINITY, f64::max)
                })
            })
            .fold(f64::NEG_INFINITY, f64::max);
        let mut best = None;
        for degrees in (-180..180).step_by(2) {
            let mut s = Spot {
                rot_y: f64::from(degrees),
                ..original
            };
            let shift = s
                .corners()
                .map(project)
                .into_iter()
                .map(|(q, z)| (edge - side * q[0]) * z + GAP / 2.)
                .fold(0., f64::max);
            if shift > 2. * s.radius() {
                continue;
            }
            s.center = [
                s.center[0] + side * right[0] * shift,
                s.center[1] + side * right[2] * shift,
            ];
            if spots.iter().any(|(id, o)| {
                *id != item.id
                    && neighbour.is_none_or(|n| *id != n.item)
                    && actor.is_none_or(|a| *id != a.id)
                    && o.floor < s.top()
                    && (s.center[0] - o.center[0]).hypot(s.center[1] - o.center[1])
                        < s.radius() + o.radius()
            }) {
                continue;
            }
            let (sin, cos) = s.rot_y.to_radians().sin_cos();
            let dx = eye[0] - s.center[0];
            let dz = eye[2] - s.center[1];
            let facing = (cos * dx - sin * dz) / dx.hypot(dz).max(0.001);
            // Directional lighting in the authored setting comes from camera left.
            let lit = (facing.signum() * (cos * (sn - cs) - sin * (cs + sn)) / 2f64.sqrt()).max(0.);
            let points: Vec<_> = contacts
                .iter()
                .map(|p| {
                    let mut p = s.to_world(*p);
                    p[1] += s.size(1) * 0.20;
                    project(p).0
                })
                .collect();
            let separation = points
                .iter()
                .enumerate()
                .flat_map(|(i, a)| {
                    points[..i]
                        .iter()
                        .map(move |b| (a[0] - b[0]).hypot(a[1] - b[1]))
                })
                .fold(f64::INFINITY, f64::min);
            let score = separation * facing.abs() * (0.12 + 0.88 * lit);
            if best.is_none_or(|(was, _)| score > was) {
                best = Some((score, s));
            }
        }
        if let Some((_, s)) = best {
            *spots.get_mut(model.item.as_str())? = s;
            moved = true;
        }
    }
    moved.then(|| (az.to_degrees(), pitch.to_degrees()))
}
