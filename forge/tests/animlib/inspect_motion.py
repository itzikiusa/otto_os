"""Independent end-to-end oracle: evaluate exported glTF node TRS in world space."""

import json
import math
import struct
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Quaternion, Vector


def read(path):
    with path.open("rb") as f:
        f.read(12)
        size, kind = struct.unpack("<I4s", f.read(8))
        assert kind == b"JSON"
        doc = json.loads(f.read(size))
        f.read(8)
        data = f.read()
    return doc, data


def accessor(asset, idx):
    doc, data = asset
    acc = doc["accessors"][idx]
    v = doc["bufferViews"][acc["bufferView"]]
    n = {"SCALAR": 1, "VEC3": 3, "VEC4": 4}[acc["type"]]
    assert acc["componentType"] == 5126
    return np.ndarray(
        (acc["count"], n),
        dtype="<f4",
        buffer=data,
        offset=v.get("byteOffset", 0) + acc.get("byteOffset", 0),
        strides=(v.get("byteStride", 4 * n), 4),
    )


def quat(xyzw):
    return Quaternion((xyzw[3], *xyzw[:3])).normalized()


def sample(asset, animation, t):
    doc, _ = asset
    nodes = [dict(node) for node in doc["nodes"]]
    start = min(float(accessor(asset, s["input"])[0, 0]) for s in animation["samplers"])
    end = max(float(accessor(asset, s["input"])[-1, 0]) for s in animation["samplers"])
    cursor = start + t * (end - start)
    for channel in animation["channels"]:
        s = animation["samplers"][channel["sampler"]]
        tt = accessor(asset, s["input"])[:, 0]
        vv = accessor(asset, s["output"])
        lo = max(0, int(np.searchsorted(tt, cursor, side="right")) - 1)
        hi = min(lo + 1, len(tt) - 1)
        u = (
            min(1, max(0, (cursor - tt[lo]) / float(tt[hi] - tt[lo])))
            if hi != lo
            else 0
        )
        mode = s.get("interpolation", "LINEAR")
        target = channel["target"]
        tag = target["path"]
        if mode == "STEP":
            val = vv[lo]
        elif mode == "CUBICSPLINE":
            dt = float(tt[hi] - tt[lo])
            val = (
                (2 * u**3 - 3 * u**2 + 1) * vv[3 * lo + 1]
                + (u**3 - 2 * u * u + u) * dt * vv[3 * lo + 2]
                + (-2 * u**3 + 3 * u**2) * vv[3 * hi + 1]
                + (u**3 - u * u) * dt * vv[3 * hi]
            )
        elif tag == "rotation":
            a = quat(vv[lo]).slerp(quat(vv[hi]), float(u))
            val = (a.x, a.y, a.z, a.w)
        else:
            val = vv[lo] * (1 - u) + vv[hi] * u
        nodes[target["node"]][tag] = val
    parents = {c: p for p, node in enumerate(nodes) for c in node.get("children", [])}
    world = {}

    def matrix(i):
        if i not in world:
            n = nodes[i]
            if "matrix" in n:
                local = Matrix(np.array(n["matrix"]).reshape(4, 4).T.tolist())
            else:
                local = Matrix.LocRotScale(
                    Vector(n.get("translation", (0, 0, 0))),
                    quat(n.get("rotation", (0, 0, 0, 1))),
                    Vector(n.get("scale", (1, 1, 1))),
                )
            world[i] = matrix(parents[i]) @ local if i in parents else local
        return world[i]

    return {
        n["name"]: matrix(i).translation for i, n in enumerate(nodes) if "name" in n
    }, end - start


def verify(directory):
    source = read(directory / "mixamo.glb")
    target = read(directory / "animated.glb")
    landmarks = ["LeftHand", "RightHand", "LeftFoot", "RightFoot", "Head"]
    result = {}
    for src in source[0]["animations"]:
        name = src["name"]
        dst = next(a for a in target[0]["animations"] if a["name"] == name)
        errors = {j: [] for j in landmarks}
        positions = []
        source_positions = []
        travel_checks = []
        for step in range(25):
            s, ds = sample(source, src, step / 24)
            d, dd = sample(target, dst, step / 24)
            assert abs(ds - dd) < 0.045, (ds, dd)
            for joint in landmarks:
                errors[joint].append((s["mixamorig:" + joint] - d[joint]).length)
            if step <= 6:
                positions.append(d["LeftFoot"])
                source_positions.append(s["mixamorig:LeftFoot"])
            travel_checks.append(
                s["mixamorig:RightHand" if name == "gesture" else "mixamorig:Hips"]
            )
        movement = max((p - travel_checks[0]).length for p in travel_checks)
        assert movement > 0.14, (name, movement)
        result[name] = {
            "duration": dd,
            "source_motion_m": movement,
            "max_world_error_m": {k: max(v) for k, v in errors.items()},
        }
        if name == "stride":
            result[name]["source_planted_foot_displacement_m"] = max(
                (p - source_positions[0]).length for p in source_positions
            )
            result[name]["planted_foot_displacement_m"] = max(
                (p - positions[0]).length for p in positions
            )
    (directory / "measured.json").write_text(json.dumps(result, indent=2))
    for name, r in result.items():
        assert max(r["max_world_error_m"].values()) < 0.04, (name, r)
    assert result["stride"]["planted_foot_displacement_m"] < 0.015, result
    print(json.dumps(result))


def verify_knight(source_path, output_path, directory):
    """Head, hands and feet should reproduce authored accessory-rig locomotion."""
    source, target = read(source_path), read(output_path)
    errors = {}
    for name, original in [
        ("walk", "Walking_A"),
        ("run", "Running_A"),
        ("idle", "Idle"),
    ]:
        a = next(a for a in source[0]["animations"] if a["name"] == original)
        b = next(a for a in target[0]["animations"] if a["name"] == name)
        peaks = dict.fromkeys(("head", "hand.l", "hand.r", "foot.l", "foot.r"), 0.0)
        for point in range(25):
            s, _ = sample(source, a, point / 24)
            t, _ = sample(target, b, point / 24)
            for joint in peaks:
                peaks[joint] = max(peaks[joint], (s[joint] - t[joint]).length)
        errors[name] = peaks
    (directory / "knight-errors.json").write_text(json.dumps(errors, indent=2))
    assert all(max(e.values()) < 0.01 for e in errors.values()), errors


def render(path, directory, clip, indices):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.render.fps = 30
    bpy.ops.import_scene.gltf(filepath=str(path))
    rig = next(o for o in bpy.context.scene.objects if o.type == "ARMATURE")
    ad = rig.animation_data
    track = next(t for t in ad.nla_tracks if t.name == clip)
    ad.action = None
    for t in ad.nla_tracks:
        t.mute = t != track
    strip = track.strips[0]
    scene = bpy.context.scene
    bpy.ops.object.camera_add()
    cam = bpy.context.object
    scene.camera = cam
    cam.data.type = "ORTHO"
    points = [
        obj.matrix_world @ Vector(v)
        for obj in scene.objects
        if obj.type == "MESH"
        for v in obj.bound_box
    ]
    low = Vector([min(p[i] for p in points) for i in range(3)])
    high = Vector([max(p[i] for p in points) for i in range(3)])
    center = (low + high) * 0.5
    span = max((high - low).z, (high - low).x, 0.1)
    cam.location = center + Vector((0.43, -1, 0.24)) * span * 3
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    cam.data.ortho_scale = span * 1.4
    cam.data.clip_end = max(100, span * 20)
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 4
    world = bpy.data.worlds.new("Studio")
    scene.world = world
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (
        0.50,
        0.55,
        0.62,
        1,
    )
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.62
    for direction, power in [((1, -2, 3), 85), ((-1, -1, 1), 35)]:
        bpy.ops.object.light_add(
            type="AREA", location=center + span * Vector(direction)
        )
        light = bpy.context.object
        light.data.energy = power * span * span * 5
        light.data.shape = "DISK"
        light.data.size = span * 2
        light.rotation_euler = (
            (center - light.location).to_track_quat("-Z", "Y").to_euler()
        )
    scene.render.resolution_x = 360
    scene.render.resolution_y = 360
    scene.render.resolution_percentage = 100
    for i, t in enumerate(indices):
        frame = strip.frame_start + t * (strip.frame_end - strip.frame_start)
        scene.frame_set(math.floor(frame), subframe=frame % 1)
        scene.render.filepath = str(directory / f"{path.stem}-{clip}-{i}.png")
        bpy.ops.render.render(write_still=True)


if __name__ == "__main__":
    args = sys.argv[sys.argv.index("--") + 1 :]
    directory = Path(args[0]).resolve()
    if len(args) == 1:
        verify(directory)
        render(directory / "animated.glb", directory, "gesture", [0, 0.5, 1])
        render(directory / "animated.glb", directory, "stride", [0.20, 0.72])
    elif args[1] == "--compare-knight":
        verify_knight(Path(args[2]), Path(args[3]), directory)
    else:
        render(Path(args[1]).resolve(), directory, args[2], [0, 0.25, 0.5])
