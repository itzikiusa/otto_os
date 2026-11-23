//! A drummer holds the measured shaft of each stick, even when its source mesh is tilted.
use super::{
    grip_rotation, mounted, rest_heading, turn_matrix, Bounds, GripAt, Model, Plan, Posed, Spot,
};
use std::collections::BTreeMap;

pub(super) struct Stick<'a> {
    pub player: &'a str,
    pub left: bool,
    pub kit: &'a str,
    // In scaled model coordinates, from the butt to the striking tip.
    ends: [[f64; 3]; 2],
}

pub(super) fn measure<'a>(
    plan: &'a Plan,
    models: &[Model],
    spots: &BTreeMap<&str, Spot>,
) -> BTreeMap<&'a str, Stick<'a>> {
    let mut sticks = BTreeMap::new();
    for placement in &plan.layout {
        let Some(rel) = placement.rel.as_ref().filter(|r| r.how == "held") else {
            continue;
        };
        let Some(kit) = rel.toward.as_deref().filter(|id| {
            plan.items
                .iter()
                .any(|i| i.id == *id && super::super::pose::drum_kit(i))
        }) else {
            continue;
        };
        let Some(m) = models.iter().find(|m| m.item == placement.item) else {
            continue;
        };
        let Some(s) = spots.get(placement.item.as_str()) else {
            continue;
        };
        let axis = (0..3)
            .max_by(|&a, &b| {
                (s.scale[a] * m.bounds.size(a)).total_cmp(&(s.scale[b] * m.bounds.size(b)))
            })
            .unwrap();
        let extent = m.bounds.max[axis] - m.bounds.min[axis];
        let mut sum = [[0.; 3]; 2];
        let mut n = [0usize; 2];
        for v in m.tris.iter().flatten() {
            let t = (v[axis] - m.bounds.min[axis]) / extent;
            let i = if t <= 0.04 {
                0
            } else if t >= 0.96 {
                1
            } else {
                continue;
            };
            for k in 0..3 {
                sum[i][k] += v[k] * s.scale[k];
            }
            n[i] += 1;
        }
        if n.contains(&0) {
            continue;
        }
        let mut ends = [0, 1].map(|i| sum[i].map(|v| v / n[i] as f64));
        if ends[1][1] < ends[0][1] {
            ends.swap(0, 1);
        }
        let stick = Stick {
            player: &rel.to,
            left: rel
                .hand
                .as_deref()
                .is_some_and(|hand| hand.starts_with("left")),
            kit,
            ends,
        };
        let length = stick.length();
        // A broad object aimed at a drum still uses the ordinary grip.
        if length <= 1e-4 {
            continue;
        }
        let dir = stick.axis().map(|v| v / length);
        let narrow = m.tris.iter().flatten().all(|p| {
            let v = [0, 1, 2].map(|k| p[k] * s.scale[k] - ends[0][k]);
            let at = (0..3).map(|k| v[k] * dir[k]).sum::<f64>();
            (0..3).map(|k| (v[k] - at * dir[k]).powi(2)).sum::<f64>() < (0.22 * length).powi(2)
        });
        if narrow {
            sticks.insert(placement.item.as_str(), stick);
        }
    }
    sticks
}

impl Stick<'_> {
    fn axis(&self) -> [f64; 3] {
        [0, 1, 2].map(|k| self.ends[1][k] - self.ends[0][k])
    }
    pub fn length(&self) -> f64 {
        let [x, y, z] = self.axis();
        x.hypot(y).hypot(z)
    }
    pub fn striking_tip(&self, player: Spot, grip: GripAt) -> [f64; 3] {
        let hand = player.to_world(grip.point);
        let r = grip_rotation(player, grip);
        [0, 1, 2].map(|k| hand[k] + 5. * self.length() / 6. * r[k][1])
    }

    pub fn mount(&self, player: Spot, own: Spot, grip: GripAt) -> Spot {
        // Straighten the tilted shaft, then orient it along the fist's grip axis.
        let [x, y, z] = self.axis();
        let straight = turn_matrix([
            (-z).atan2(x.hypot(y)).to_degrees(),
            0.,
            x.atan2(y).to_degrees(),
        ]);
        let hand = grip_rotation(player, grip);
        let rotation =
            [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| hand[i][k] * straight[k][j]).sum()));
        let anchor = [0, 1, 2].map(|i| self.ends[0][i] + (self.ends[1][i] - self.ends[0][i]) / 6.);
        mounted(player, own, rotation, anchor, grip.point)
    }
}

/// Place a player far enough behind the selected heads for a bent arm and the exposed
/// part of a stick. Keep the upper body facing its instrument throughout the pose pass.
pub(super) fn at_heads(
    plan: &Plan,
    player: &str,
    instrument: &str,
    models: &[Model],
    spots: &BTreeMap<&str, Spot>,
    all_sticks: &BTreeMap<&str, Stick>,
) -> Option<Spot> {
    let sticks: Vec<_> = all_sticks
        .values()
        .filter(|s| s.player == player && s.kit == instrument)
        .collect();
    let length = sticks.iter().map(|s| s.length()).min_by(f64::total_cmp)?;
    let mut actor = *spots.get(player)?;
    let kit = *spots.get(instrument)?;
    let model = models.iter().find(|m| m.item == instrument)?;
    let corners = kit.corners();
    let bounds = corners.into_iter().fold(
        Bounds {
            min: corners[0],
            max: corners[0],
        },
        Bounds::add,
    );
    let tris: Vec<_> = model
        .tris
        .iter()
        .map(|t| t.map(|p| kit.to_world(p)))
        .collect();
    let mut heads = super::super::pose::near_drumheads(&tris, bounds, actor.center);
    let center = [0, 2].map(|k| (heads[0][k] + heads[1][k]) / 2.);
    let dx = center[0] - actor.center[0];
    let dz = center[1] - actor.center[1];
    let distance = dx.hypot(dz);
    if distance < 0.001 {
        return None;
    }
    let reach = (0.24 * actor.size(1) + 5. * length / 6.).min(distance);
    actor.center = [
        center[0] - reach * dx / distance,
        center[1] - reach * dz / distance,
    ];
    actor.rot_y = dx.atan2(dz).to_degrees() - rest_heading(plan, player);
    if let Some(pose) = models
        .iter()
        .find(|m| m.item == player)
        .and_then(|m| m.pose)
    {
        let left = |p: &[f64; 3]| p[0] * dz - p[2] * dx;
        heads.sort_by(|a, b| left(b).total_cmp(&left(a)));
        keep_contact(&mut actor, pose, &sticks, heads, length);
    }
    Some(actor)
}

fn keep_contact(
    actor: &mut Spot,
    pose: Posed,
    sticks: &[&Stick],
    heads: [[f64; 3]; 2],
    length: f64,
) {
    let mut total = [0.; 2];
    let mut count = 0;
    for stick in sticks {
        let Some(grip) = [pose.grip, pose.other_grip]
            .into_iter()
            .flatten()
            .find(|g| g.left == stick.left)
        else {
            continue;
        };
        let tip = stick.striking_tip(*actor, grip);
        let target = heads[usize::from(!grip.left)];
        total[0] += target[0] - tip[0];
        total[1] += target[2] - tip[2];
        count += 1;
    }
    if count > 0 {
        let shift = total.map(|v| v / f64::from(count));
        // A small translation compensates for the bounding box moving as the player bends its
        // arms. A legacy pose aimed across its body should never pull the player into the kit.
        if shift[0].hypot(shift[1]) <= length / 2. {
            actor.center = [0, 1].map(|k| actor.center[k] + shift[k]);
        }
    }
}
