//! Open a sight line into a cavity when the requested subject is peeking out.
use super::{
    geometry::hit_distance,
    scene::{rest_heading, words, Bounds, Model},
    Plan,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) fn under_cover<'p>(plan: &'p Plan, it: &super::Item) -> Option<&'p super::Rel> {
    let peeks = it.details.pose.as_ref().is_some_and(|p| p.kind == "peek")
        || words(&it.description).iter().any(|w| {
            matches!(
                w.as_str(),
                "peek" | "peeking" | "hide" | "hiding" | "concealed"
            )
        });
    if it.kind != "character" || !peeks {
        return None;
    }
    plan.layout
        .iter()
        .find(|p| p.item == it.id)?
        .rel
        .as_ref()
        .filter(|r| r.how == "under")
}

pub(super) fn frame(
    plan: &Plan,
    models: &[Model],
    doc: &mut Value,
    boxes: &mut BTreeMap<String, Bounds>,
) {
    let Some((actor, rel)) = plan
        .items
        .iter()
        .find_map(|it| Some((it, under_cover(plan, it)?)))
    else {
        return;
    };
    let (Some(support), Some(body)) = (boxes.get(&rel.to).copied(), boxes.get(&actor.id).copied())
    else {
        return;
    };
    let Some(mesh) = models
        .iter()
        .find(|m| m.item == rel.to && !m.tris.is_empty())
    else {
        return;
    };
    let Some(objects) = doc["objects"].as_array() else {
        return;
    };
    let Some(obj) = objects.iter().find(|o| o["id"] == rel.to) else {
        return;
    };
    let subject = models.iter().find(|m| m.item == actor.id);
    let body = subject
        .and_then(|m| {
            super::setting::world_box(objects.iter().find(|o| o["id"] == actor.id)?, &m.now_box())
        })
        .unwrap_or(body);
    let v3 = |v: &Value| serde_json::from_value::<[f64; 3]>(v.clone()).ok();
    let (Some(pos), Some(scale), Some(rot)) = (
        v3(&obj["position"]),
        v3(&obj["scale"]),
        v3(&obj["rotation"]),
    ) else {
        return;
    };
    if rot[0].abs() > 5. || rot[2].abs() > 5. {
        return;
    }
    let (sn, cs) = rot[1].to_radians().sin_cos();
    let tris = mesh
        .tris
        .iter()
        .map(|t| {
            t.map(|p| {
                let [x, y, z] = [0, 1, 2].map(|k| p[k] * scale[k]);
                [
                    pos[0] + cs * x + sn * z,
                    pos[1] + y,
                    pos[2] - sn * x + cs * z,
                ]
            })
        })
        .collect::<Vec<_>>();
    let all = boxes.values().fold(support, |a, b| Bounds {
        min: [0, 1, 2].map(|k| a.min[k].min(b.min[k])),
        max: [0, 1, 2].map(|k| a.max[k].max(b.max[k])),
    });
    let target = [0, 1, 2].map(|k| (all.max[k] + all.min[k]) / 2.);
    let fov = doc["camera"]["fov"].as_f64().unwrap_or(50.).to_radians();
    let span = (all.max[0] - all.min[0]).max(all.max[1] - all.min[1]);
    let dist = 0.55 * span / (0.5 * fov).tan() + 0.5 * (all.max[2] - all.min[2]);
    let face = if doc["camera"]["position"][2]
        .as_f64()
        .unwrap_or(target[2] + 1.)
        >= target[2]
    {
        1.
    } else {
        -1.
    };
    let size = [0, 1, 2].map(|k| body.max[k] - body.min[k]);
    let heading = rest_heading(plan, &actor.id);
    let actor_obj = objects.iter().find(|o| o["id"] == actor.id);
    // Position the sight lines on the posed face. A face can be well away
    // from the middle of a crouched animal's body and its +Z axis.
    let eyes = subject.and_then(|m| {
        let posed = m.pose?;
        let head = posed.head?;
        let bounds = posed.head_bounds?;
        let scale = v3(&actor_obj?["scale"])?;
        let (sn, cs) = heading.to_radians().sin_cos();
        let diameter = ((bounds.max[0] - bounds.min[0]) * scale[0])
            .min((bounds.max[2] - bounds.min[2]) * scale[2]);
        let middle = [0, 1, 2].map(|k| 0.5 * (posed.min[k] + posed.max[k]));
        let x = (head[0] - middle[0]) * scale[0] + sn * 0.45 * diameter;
        let z = (head[2] - middle[2]) * scale[2] + cs * 0.45 * diameter;
        let y = (head[1] - posed.min[1] + 0.12 * (bounds.max[1] - bounds.min[1])) * scale[1];
        Some(([x, y, z], 0.22 * diameter))
    });
    let mid = [0, 1, 2].map(|k| (support.max[k] + support.min[k]) / 2.);
    let front = if face > 0. {
        support.max[2]
    } else {
        support.min[2]
    };
    let mut best = None;
    for pitch in [16.0_f64, 12., 8., 5.] {
        let e = pitch.to_radians();
        let camera = [
            target[0],
            target[1] + dist * e.sin(),
            target[2] + face * dist * e.cos(),
        ];
        for inset in [1.40, 1.0, 0.80, 0.65, 0.45, 0.25] {
            for side in [0., 0.075, -0.075, 0.15, -0.15, 0.32, -0.32] {
                let center = [
                    mid[0] + side * (support.max[0] - support.min[0]),
                    body.min[1],
                    front - face * size[2] * inset,
                ];
                let yaw = (camera[0] - center[0]).atan2(camera[2] - center[2]);
                let rotation = yaw - heading.to_radians();
                let (sn, cs) = rotation.sin_cos();
                let (eye, half_width) = eyes.map_or(
                    (
                        [
                            center[0],
                            center[1] + 0.80 * size[1],
                            center[2] + face * 0.40 * size[2],
                        ],
                        0.23 * size[0],
                    ),
                    |([x, y, z], w)| {
                        (
                            [
                                center[0] + cs * x + sn * z,
                                center[1] + y,
                                center[2] - sn * x + cs * z,
                            ],
                            w,
                        )
                    },
                );
                let entry = [eye[0], eye[1], front + face * 0.2];
                let through = [
                    0.,
                    0.,
                    -face * (0.2 + 0.22 * (support.max[2] - support.min[2])),
                ];
                if tris
                    .iter()
                    .any(|tri| hit_distance(entry, through, tri).is_some())
                {
                    continue;
                }
                let visible = |at: [f64; 3]| {
                    let dir = [0, 1, 2].map(|k| at[k] - camera[k]);
                    !tris
                        .iter()
                        .any(|tri| hit_distance(camera, dir, tri).is_some())
                };
                // Include a margin round each eye so a tiny slit through the
                // furniture cannot count as a visible face.
                if ![-1., 1.].iter().all(|side| {
                    let x = 1.35 * side * half_width;
                    [-0.35, 0.35].iter().all(|height| {
                        visible([
                            eye[0] + yaw.cos() * x,
                            eye[1] + height * half_width,
                            eye[2] - yaw.sin() * x,
                        ])
                    })
                }) {
                    continue;
                }
                let covered = [0.15, 0.32, 0.5, 0.76, 0.90]
                    .into_iter()
                    .filter(|&y| !visible([center[0], center[1] + y * size[1], center[2]]))
                    .count();
                let value =
                    covered as f64 * 0.30 + inset * 0.08 + pitch * 0.012 - side.abs() * 0.10;
                if best.as_ref().is_none_or(|(s, _, _, _)| value > *s) {
                    best = Some((value, center, camera, rotation))
                }
            }
        }
    }
    let Some((_, center, camera, rotation)) = best else {
        return;
    };
    let Some(obj) = doc["objects"]
        .as_array_mut()
        .and_then(|a| a.iter_mut().find(|o| o["id"] == actor.id))
    else {
        return;
    };
    let Some(mut p) = v3(&obj["position"]) else {
        return;
    };
    obj["rotation"] = json!([0., rotation.to_degrees(), 0.]);
    // The offset of an asymmetric mesh changes when its heading changes.
    let turned = subject
        .and_then(|m| super::setting::world_box(obj, &m.now_box()))
        .unwrap_or(body);
    for axis in [0, 2] {
        p[axis] += center[axis] - 0.5 * (turned.min[axis] + turned.max[axis]);
    }
    obj["position"] = json!(p);
    if let Some(bounds) = subject.and_then(|m| super::setting::world_box(obj, &m.now_box())) {
        boxes.insert(actor.id.clone(), bounds);
    }
    doc["camera"]["position"] = json!(camera);
    doc["camera"]["target"] = json!(target);
}
