"""Independent world-ray and skinned-sole measurements on exported knight GLBs."""

import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from inspect_motion import read, sample


def directions(source, target, source_name, clip):
    sa = next(a for a in source[0]["animations"] if a["name"] == source_name)
    ta = next(a for a in target[0]["animations"] if a["name"] == clip)
    errors = {}
    elevation = {s: [] for s in ("Left", "Right")}
    torso = []
    for i in range(33):
        s, _ = sample(source, sa, i / 32)
        t, _ = sample(target, ta, i / 32)
        src_down = (s["hips"] - s["head"]).normalized()
        dst_down = (t["Hips"] - t["Head"]).normalized()
        torso.append(math.degrees(src_down.angle(dst_down)))
        for side, suffix in (("Left", ".l"), ("Right", ".r")):
            elevation[side].append(
                [
                    math.degrees(
                        (s["lowerarm" + suffix] - s["upperarm" + suffix]).angle(
                            src_down
                        )
                    ),
                    math.degrees(
                        (t[side + "LowerArm"] - t[side + "UpperArm"]).angle(dst_down)
                    ),
                ]
            )
            for a, b, x, y in [
                ("upperarm", "lowerarm", "UpperArm", "LowerArm"),
                ("lowerarm", "hand", "LowerArm", "Hand"),
                ("upperleg", "lowerleg", "UpperLeg", "LowerLeg"),
                ("lowerleg", "foot", "LowerLeg", "Foot"),
            ]:
                u, v = s[b + suffix] - s[a + suffix], t[side + y] - t[side + x]
                error = math.degrees(math.atan2(u.cross(v).length, u.dot(v)))
                errors[side + x] = max(errors.get(side + x, 0), error)
    return {
        "max_world_angle_deg": errors,
        "torso_max_deg": max(torso),
        "median_elevations": {
            s: np.median(v, axis=0).tolist() for s, v in elevation.items()
        },
    }


def soles(path, clips):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.fps = 30
    bpy.ops.import_scene.gltf(filepath=str(path))
    arm = next(o for o in scene.objects if o.type == "ARMATURE")
    mesh = next(
        o for o in scene.objects if o.type == "MESH" and o.find_armature() == arm
    )
    ad = arm.animation_data
    tracks = {
        t.name: (t.strips[0].action, t.strips[0].action_slot) for t in ad.nla_tracks
    }
    ad.action = None
    for track in list(ad.nla_tracks):
        ad.nla_tracks.remove(track)
    arm.data.pose_position = "REST"
    bpy.context.view_layer.update()

    def points():
        ob = mesh.evaluated_get(bpy.context.evaluated_depsgraph_get())
        coords = np.empty(len(mesh.data.vertices) * 3, dtype=np.float32)
        ob.data.vertices.foreach_get("co", coords)
        mat = np.array(ob.matrix_world)
        return coords.reshape(-1, 3) @ mat[:3, :3].T + mat[:3, 3]

    rest = points()
    floor = rest[:, 2].min()
    height = np.ptp(rest[:, 2])
    shoes = []
    markers = []
    for side in ("Left", "Right"):
        groups = {
            g.index
            for g in mesh.vertex_groups
            if g.name in (side + "Foot", side + "Toes")
        }
        ids = [
            v.index
            for v in mesh.data.vertices
            if sum(g.weight for g in v.groups if g.group in groups) > 0.5
        ]
        ids = np.array(ids)
        shoes.append(ids)
        markers.append(ids[rest[ids, 2] <= np.percentile(rest[ids, 2], 10)])
    arm.data.pose_position = "POSE"
    result = {}
    for clip in clips:
        # A richer source animates clavicles/neck that KayKit leaves at bind.
        ad.action, ad.action_slot = tracks["100style-angry-fw"]
        scene.frame_set(0)
        ad.action, ad.action_slot = tracks[clip]
        end = round(8 * ad.action.frame_range[1])
        depths, positions, angles = [], [], []
        for f in range(end + 1):
            scene.frame_set(f // 8, subframe=(f % 8) / 8)
            if f % 8 == 0:
                bone = arm.pose.bones
                down = bone["Hips"].head - bone["Head"].head
                ray = bone["RightLowerArm"].head - bone["RightUpperArm"].head
                angles.append(math.degrees(ray.angle(down)))
            p = points()
            depths.append([p[ix, 2].min() - floor for ix in shoes])
            positions.append([p[ix, :2].mean(axis=0) for ix in markers])
        d, xy = np.array(depths), np.array(positions)
        row = {
            "minimum": float(d.min()),
            "switched_elevation": float(np.median(angles)),
        }
        if clip == "walk":
            # Same long support intervals and rigid-controller fit as the cross-stage report.
            runs = []
            for side in (0, 1):
                ix = np.flatnonzero(np.abs(d[::8, side]) <= 0.02 * height)
                for part in np.split(ix, np.where(np.diff(ix) != 1)[0] + 1):
                    if len(part) >= 9:
                        runs.append((part / 30, xy[8 * part, side]))
            assert len(runs) >= 2, "no sustained walk contact"
            vel = sum(((t - t.mean())[:, None] * p).sum(axis=0) for t, p in runs) / sum(
                ((t - t.mean()) ** 2).sum() for t, p in runs
            )
            row["slide"] = max(
                float(np.linalg.norm(r[:, None] - r[None, :], axis=-1).max())
                for t, p in runs
                for r in [p - t[:, None] * vel]
            )
        result[clip] = row
    return result


if __name__ == "__main__":
    source, output, report, *raw_path = map(Path, sys.argv[sys.argv.index("--") + 1 :])
    a, b = read(source), read(output)
    clips = {"walk": "Walking_A", "1h-melee-attack-chop": "1H_Melee_Attack_Chop"}
    metrics = {
        "rays": {k: directions(a, b, v, k) for k, v in clips.items()},
        "soles": soles(output, clips),
    }
    if raw_path:
        raw = read(raw_path[0])
        metrics["off"] = soles(raw_path[0], clips)
        for name in clips:
            on_action, off_action = [
                next(q for q in model[0]["animations"] if q["name"] == name)
                for model in (b, raw)
            ]
            corrections = []
            for t in np.linspace(0, 1, 65):
                on, _ = sample(b, on_action, t)
                off, _ = sample(raw, off_action, t)
                corrections.append(list(on["Hips"] - off["Hips"]))
            row = metrics["soles"][name]
            # glTF is Y-up. Exclude the source's own pelvis motion from the step measurement.
            delta = np.asarray(corrections)
            row["max_step_m_60hz"] = float(
                np.linalg.norm(np.diff(delta, axis=0), axis=1).max()
            )
            xy = delta[:, [0, 2]]
            row["max_lateral_step_m_60hz"] = float(
                np.linalg.norm(np.diff(xy, axis=0), axis=1).max()
            )
            row["cycle_correction_seam_m"] = float(np.linalg.norm(xy[-1] - xy[0]))
    report.write_text(json.dumps(metrics, indent=2))
