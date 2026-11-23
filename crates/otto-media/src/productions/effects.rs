//! Place effects on the made surface, in the final object's transform.
use super::{
    geometry::Surface,
    scene::{words, Model},
    Item,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    None,
    RainOnGlass,
    Steam,
}

fn requested(item: &Item) -> Option<Effect> {
    if item.kind != "object" {
        return None;
    }
    if let Some(effect) = item.details.effect {
        return Some(effect);
    }
    // Existing plans describe the effect in prose. Match positive descriptions
    // of the effect itself, so a steam locomotive or a raincoat stays dry.
    let desc = item.description.to_lowercase();
    if ["no steam", "without steam", "no rain", "without rain"]
        .iter()
        .any(|s| desc.contains(s))
    {
        return None;
    }
    let name = words(&item.name);
    if name.iter().any(|s| s == "window" || s == "pane")
        && ["raindrop", "rain runs", "rain on", "rain streak"]
            .iter()
            .any(|s| desc.contains(s))
    {
        return Some(Effect::RainOnGlass);
    }
    if desc.contains("steam")
        && ["rise", "rising", "curl", "billow", "steaming"]
            .iter()
            .any(|s| desc.contains(s))
    {
        return Some(Effect::Steam);
    }
    None
}

/// A raised bonnet leaves daylight between the panel and the engine block
/// in side view. The roof over the boot can be higher than the bonnet.
fn open_engine_bay(model: &Model, long: usize) -> Option<(f64, f64)> {
    let side = |p: [f64; 3]| [p[long], p[2 - long], p[1]];
    let lo = side(model.bounds.min);
    let hi = side(model.bounds.max);
    let tris: Vec<_> = model.tris.iter().map(|tri| tri.map(side)).collect();
    let silhouette = Surface::of(&tris, lo, hi)?;
    let measure = |sign: f64| {
        let (mut weight, mut columns, mut along, mut bottom) = (0., 0, 0., 0.);
        for col in 2..16 {
            let fraction = col as f64 / 48.;
            let x = lo[0] + (hi[0] - lo[0]) * if sign < 0. { fraction } else { 1. - fraction };
            let mut previous = None;
            let mut largest = (0, 0);
            for row in 8..=48 {
                let y = lo[2] + (hi[2] - lo[2]) * row as f64 / 48.;
                if silhouette.top_at(x, y).is_some() {
                    if let Some(low) = previous {
                        if row >= 25 && row - low >= 5 && row - low > largest.0 {
                            largest = (row - low, low);
                        }
                    }
                    previous = Some(row);
                }
            }
            if largest.0 > 0 {
                let gap = largest.0 as f64;
                weight += gap;
                columns += 1;
                along += gap * x;
                bottom += gap * (lo[2] + (hi[2] - lo[2]) * largest.1 as f64 / 48.);
            }
        }
        (weight, columns, along, bottom)
    };
    let (a, b) = (measure(-1.), measure(1.));
    let (open, other) = if a.0 >= b.0 { (a, b) } else { (b, a) };
    (open.1 >= 3 && open.0 > 2. * other.0).then(|| (open.2 / open.0, open.3 / open.0))
}

pub(super) fn placed(item: &Item, model: &Model, obj: &Value, camera: [f64; 3]) -> Option<Value> {
    let kind = requested(item)?;
    let b = model.bounds;
    let s: [f64; 3] = serde_json::from_value(obj["scale"].clone()).ok()?;
    let p: [f64; 3] = serde_json::from_value(obj["position"].clone()).ok()?;
    let yaw = obj["rotation"][1].as_f64().unwrap_or(0.0);
    let (sin, cos) = yaw.to_radians().sin_cos();
    let dim = [0, 1, 2].map(|k| (b.max[k] - b.min[k]) * s[k]);
    let mut at = [0, 1, 2].map(|k| (b.max[k] + b.min[k]) / 2.0);
    let (size, rotation) = match kind {
        Effect::None => return None,
        Effect::RainOnGlass => {
            let (depth, width) = if dim[2] < dim[0] { (2, 0) } else { (0, 2) };
            let rotation = yaw + if depth == 0 { 90.0 } else { 0.0 };
            let (nx, nz) = rotation.to_radians().sin_cos();
            let side = ((camera[0] - p[0]) * nx + (camera[2] - p[2]) * nz).signum();
            // The largest coplanar area parallel to the frame is its glazing.
            // A thin film on that plane stays behind the wooden mullions.
            let mut bins = [0.0; 64];
            let depth_span = (b.max[depth] - b.min[depth]).max(1e-6);
            for tri in &model.tris {
                let [a, c, d] = tri;
                if (c[depth] - a[depth]).abs().max((d[depth] - a[depth]).abs()) > 0.02 * depth_span
                {
                    continue;
                }
                let area = ((c[width] - a[width]) * (d[1] - a[1])
                    - (d[width] - a[width]) * (c[1] - a[1]))
                    .abs();
                let idx = (63.0 * (a[depth] - b.min[depth]) / depth_span)
                    .round()
                    .clamp(0.0, 63.0) as usize;
                bins[idx] += area;
            }
            if let Some((i, _)) = bins
                .iter()
                .enumerate()
                .filter(|(_, v)| **v > 0.0)
                .max_by(|a, b| a.1.total_cmp(b.1))
            {
                at[depth] = b.min[depth] + (i as f64 / 63.0) * depth_span;
            }
            at[depth] += side * (0.02 * depth_span + 0.002 / s[depth]);
            ([dim[width] * 0.89, dim[1] * 0.88], rotation)
        }
        Effect::Steam => {
            at[1] = b.max[1];
            let desc = item.description.to_lowercase();
            let long = if dim[2] >= dim[0] { 2 } else { 0 };
            let description = format!(
                "{desc} {}",
                item.details
                    .prompt
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
            );
            let has_open_hood = (description.contains("hood") || description.contains("bonnet"))
                && words(&description)
                    .iter()
                    .any(|w| matches!(w.as_str(), "raised" | "up" | "open" | "lifted"));
            if let Some((along, y)) = has_open_hood
                .then(|| open_engine_bay(model, long))
                .flatten()
            {
                at[long] = along;
                at[1] = y;
            } else if let Some(surface) =
                (desc.contains("hood") || desc.contains("bonnet") || desc.contains("engine"))
                    .then(|| Surface::of(&model.tris, b.min, b.max))
                    .flatten()
            {
                // The raised hood has a higher edge than the boot, with the
                // exposed bay directly underneath. Sample each end of the car.
                let cross = 2 - long;
                let end = |sign: f64| {
                    let mut heights = Vec::new();
                    for i in 5..=9 {
                        for j in -2..=2 {
                            let mut v = at;
                            v[long] += sign * i as f64 * 0.05 * (b.max[long] - b.min[long]);
                            v[cross] += j as f64 * 0.08 * (b.max[cross] - b.min[cross]);
                            if let Some(y) = surface.top_at(v[0], v[2]) {
                                heights.push(y);
                            }
                        }
                    }
                    heights.sort_by(f64::total_cmp);
                    (!heights.is_empty())
                        .then(|| (heights[heights.len() * 9 / 10], heights[heights.len() / 4]))
                };
                if let (Some(front), Some(back)) = (end(1.0), end(-1.0)) {
                    let (sign, y) = if front.0 >= back.0 {
                        (1.0, front.1)
                    } else {
                        (-1.0, back.1)
                    };
                    at[long] += sign * 0.31 * (b.max[long] - b.min[long]);
                    at[1] = y;
                }
            }
            let h = (0.92 * dim[1]).clamp(0.14, 3.2);
            ([h * 0.43, h], 0.0)
        }
    };
    if size.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return None;
    }
    let [x, y, z] = [0, 1, 2].map(|k| at[k] * s[k]);
    Some(
        json!({"kind":kind,"position":[p[0]+x*cos+z*sin,p[1]+y,p[2]-x*sin+z*cos],"size":size,"rotation":rotation}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::productions::scene::Bounds;

    #[test]
    fn steam_rises_from_the_open_bay_even_when_the_other_end_is_taller() {
        let item: Item = serde_json::from_value(json!({
            "id":"van", "kind":"object", "name":"van",
            "description":"Steam rises from the engine under the raised bonnet."
        }))
        .unwrap();
        let object = json!({"position":[0,0,0],"rotation":[0,0,0],"scale":[1,1,1]});
        for sign in [-1., 1.] {
            let mut panel_start = 0;
            let mut model = Model {
                item: "van".into(),
                artifact_id: "van-model".into(),
                version: Some(1),
                bounds: Bounds {
                    min: [-0.9, 0., -2.],
                    max: [0.9, 1.8, 2.],
                },
                joints: Default::default(),
                real_size: true,
                tris: Vec::new(),
                pose: None,
            };
            // A tall solid rear, a low engine block and a raised panel. Both
            // top and side faces must be present for the two views of this mesh.
            for (z0, z1, bottom, top) in [
                (-2., -0.7, 0., 1.8),
                (0.5, 2., 0., 0.65),
                (0.55, 1.91, 1.15, 1.35),
            ] {
                panel_start = model.tris.len();
                let p = |x, y, z| [x, y, z * sign];
                for side in [-0.9, 0.9] {
                    let [a, b, c, d] = [
                        p(side, bottom, z0),
                        p(side, top, z0),
                        p(side, top, z1),
                        p(side, bottom, z1),
                    ];
                    model
                        .tris
                        .extend([[a, b, c], [a, c, d], [c, b, a], [d, c, a]]);
                }
                let [a, b, c, d] = [
                    p(-0.9, top, z0),
                    p(-0.9, top, z1),
                    p(0.9, top, z1),
                    p(0.9, top, z0),
                ];
                model
                    .tris
                    .extend([[a, b, c], [a, c, d], [c, b, a], [d, c, a]]);
            }
            let position: [f64; 3] = serde_json::from_value(
                placed(&item, &model, &object, [0., 1., 5.]).unwrap()["position"].clone(),
            )
            .unwrap();
            assert!(position[2] * sign > 0.5, "{position:?}");
            assert!((position[1] - 0.65).abs() < 0.10, "{position:?}");

            // With the lid removed this is an uncovered block at one end
            // and a solid trunk at the other. Neither has an overhang.
            model.tris.truncate(panel_start);
            assert_eq!(open_engine_bay(&model, 2), None);
        }
    }
}
