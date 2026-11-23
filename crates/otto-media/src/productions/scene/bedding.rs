//! Rest a tucked plush on its actual pillow and carry the cover over its feet.
use super::{
    compose_turns, rotation, turn_matrix, turn_of, turned, Item, Link, Model, Spot, Surface,
};

fn top((spot, surface): (Spot, &Surface), point: [f64; 2]) -> Option<f64> {
    let [x, z] = spot.to_model(point);
    Some(spot.world_y(surface.top_at(x, z)?))
}

fn percentile(values: &mut [f64], percent: usize) -> Option<f64> {
    let index = values.len().checked_sub(1)? * percent / 100;
    Some(*values.select_nth_unstable_by(index, f64::total_cmp).1)
}

pub(super) fn rest(
    model: &Model,
    item: &Item,
    link: &Link,
    mut toy: Spot,
    (mut cover, fabric): (Spot, &Surface),
    support: (Spot, &Surface),
) -> Option<(Spot, Spot)> {
    // A single requested size preserves the mesh proportions. Its upright
    // height becomes length along the mattress when the plush reclines.
    let [width, depth_on_back, length] = item
        .details
        .dims_m
        .unwrap_or_else(|| [toy.size(0), toy.size(2), toy.size(1)]);
    if toy.tilt.is_none()
        || cover.tilt.is_some()
        || (item.details.dims_m.is_some() && depth_on_back >= length)
        || [width, depth_on_back, length]
            .iter()
            .any(|d| !d.is_finite() || *d <= 0.)
    {
        return None;
    }
    let (x, z) = link.spot.filter(|p| *p != (0, 0)).unwrap_or((0, -1));
    let raw = [
        f64::from(x) * cover.half()[0],
        f64::from(z) * cover.half()[1],
    ];
    let half = raw[0].hypot(raw[1]);
    if half < length {
        return None;
    }
    let dir = raw.map(|v| v / half);
    let along = |d: f64| [0, 1].map(|k| cover.center[k] + dir[k] * d);
    // The real fabric can have a curved edge inside its bounding rectangle.
    let edge_dist = (0..=32)
        .map(|n| half * (1. - f64::from(n) / 32.))
        .find(|&d| top((cover, fabric), along(d)).is_some())?;
    let head_dist = edge_dist + 0.3 * length;
    let pillow_rise = |distance| {
        let rise = top(support, along(distance))? - cover.floor;
        (0.10 * depth_on_back..=length)
            .contains(&rise)
            .then_some(rise)
    };
    // When the cloth extends over the pillow, the first probe can land on
    // bare mattress behind it. Draw the cloth back to expose the head there.
    let (inset, rise) = pillow_rise(head_dist).map(|h| (0., h)).or_else(|| {
        (1..=24)
            .filter_map(|i| {
                let inset = length * f64::from(i) / 24.;
                pillow_rise(head_dist - inset).map(|h| (inset, h))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.total_cmp(&a.0)))
    })?;
    cover.center = along(-inset);
    let along = |d: f64| [0, 1].map(|k| cover.center[k] + dir[k] * d);
    let edge = along(edge_dist);
    let yaw = (-dir[0]).atan2(-dir[1]).to_degrees();
    let pitch = -90. + rise.atan2(length * 0.5).to_degrees().clamp(5., 65.);
    let e = rotation(yaw, Some(&turn_matrix([pitch, 0., 0.])));
    let r = turn_matrix(e);
    let bounds = model.bounds;
    let dimensions = [width, length, depth_on_back];
    toy.scale = [0, 1, 2].map(|k| dimensions[k] / bounds.size(k));
    let oriented = |p: [f64; 3]| turned(r, [0, 1, 2].map(|k| p[k] * toy.scale[k]));
    let neck = oriented([
        bounds.mid(0),
        bounds.min[1] + 0.63 * bounds.size(1),
        bounds.mid(2),
    ]);
    let origin = [edge[0] - neck[0], 0., edge[1] - neck[2]];
    let pivot = along(-half);
    let mut head = Vec::new();
    let mut body = Vec::new();
    for v in model.tris.iter().flatten() {
        let p = oriented(*v);
        let xz = [origin[0] + p[0], origin[2] + p[2]];
        let h = (v[1] - bounds.min[1]) / bounds.size(1);
        let cloth = top((cover, fabric), xz);
        if h >= 0.68 && cloth.is_none() {
            if let Some(y) = top(support, xz) {
                head.push(y - p[1]);
            }
        } else if let Some(y) = cloth.filter(|_| h < 0.55) {
            let distance = (xz[0] - pivot[0]) * dir[0] + (xz[1] - pivot[1]) * dir[1];
            if distance > length {
                body.push((p[1] - y, distance));
            }
        }
    }
    // Contact the back of the head with the pillow. A few stray triangles
    // at the seam must not lift a plush or the entire opposite blanket edge.
    let y = percentile(&mut head, 93)?;
    let mut slopes: Vec<_> = body.iter().map(|(dy, d)| (y + dy) / d).collect();
    let slope = percentile(&mut slopes, 93)?.max(0.);
    if slope > 0.4 {
        return None;
    }
    toy.tilt = Some(([origin[0], y, origin[2]], e));
    let corners = toy.corners();
    let mid = |k: usize| corners.iter().map(|p| p[k]).sum::<f64>() / 8.;
    toy.center = [mid(0), mid(2)];
    toy.floor = corners.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    // Hinge the head edge up, keeping the foot edge on the mattress.
    let direction = turn_matrix([0., yaw, 0.]);
    let tilt = compose_turns(
        compose_turns(direction, turn_matrix([slope.atan().to_degrees(), 0., 0.])),
        turn_matrix([0., -yaw, 0.]),
    );
    let pivot = [pivot[0], cover.floor, pivot[1]];
    let delta = turned(tilt, [0, 1, 2].map(|k| cover.pos()[k] - pivot[k]));
    cover.tilt = Some((
        [0, 1, 2].map(|k| pivot[k] + delta[k]),
        turn_of(compose_turns(tilt, turn_matrix([0., cover.rot_y, 0.]))),
    ));
    Some((toy, cover))
}
