"""Run inside locked Blender from the Forge root after generating the three rig files."""

import json
import math
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector
from mathutils.kdtree import KDTree

ROOT = Path(__file__).resolve().parents[2]
OUT = Path(__file__).resolve().parent / "work"
settings = ("0", "0.5", "1")
bpy.ops.wm.read_factory_settings(use_empty=True)
sc = bpy.context.scene
sc.render.fps = 30


def import_rig(path):
    before = set(sc.objects)
    bpy.ops.import_scene.gltf(filepath=str(path))
    obs = set(sc.objects) - before
    arm = next(o for o in obs if o.type == "ARMATURE")
    for o in obs:
        o.hide_render = True
    return arm, next(o for o in obs if o.type == "MESH" and o.find_armature() == arm)


def positions(mesh):
    ob = mesh.evaluated_get(bpy.context.evaluated_depsgraph_get())
    co = np.empty(len(ob.data.vertices) * 3, dtype=np.float32)
    ob.data.vertices.foreach_get("co", co)
    mat = np.array(ob.matrix_world)
    return co.reshape(-1, 3) @ mat[:3, :3].T + mat[:3, 3]


rig, reference = import_rig(ROOT / "out/p0/demo/retarget_v2/knight_final.glb")
reference.hide_render = True
ad = rig.animation_data
clips = {t.name: t.strips[0].action for t in ad.nla_tracks}
for t in list(ad.nla_tracks):
    ad.nla_tracks.remove(t)
ad.action = None
rig.data.pose_position = "REST"
meshes = {}
for key in settings:
    old, mesh = import_rig(OUT / f"knight-{key}.glb")
    world = mesh.matrix_world.copy()
    mesh.parent = rig
    mesh.matrix_world = world
    next(m for m in mesh.modifiers if m.type == "ARMATURE").object = rig
    mesh.hide_render = True
    meshes[key] = mesh
bpy.context.view_layer.update()
rest = positions(meshes["0"])
for mesh in meshes.values():
    np.testing.assert_allclose(positions(mesh), rest, atol=1e-6)
ref_rest = positions(reference)
# Bind vertex order differs across Blender's v2 re-export (duplicated UV seams).
# Validate that the driven original mesh reproduces that export throughout both actions.
tree = KDTree(len(ref_rest))
for i, v in enumerate(ref_rest):
    tree.insert(v, i)
tree.balance()
mapping = np.array([tree.find(v)[1] for v in rest])
np.testing.assert_allclose(rest, ref_rest[mapping], atol=1e-6)
low, high = rest.min(axis=0), rest.max(axis=0)
height = float(high[2] - low[2])
center = Vector((low + high) / 2)
rig.data.pose_position = "POSE"
mesh = meshes["0"]
groups = [g.name for g in mesh.vertex_groups]
dominant = [max(v.groups, key=lambda g: g.weight) for v in mesh.data.vertices]
owner = np.array([groups[g.group] for g in dominant])
weight = np.array([g.weight for g in dominant])
pairs = {}
rng = np.random.default_rng(582)
cohorts = {
    "torso": ["Spine", "Chest", "UpperChest"],
    "forearms": ["LeftLowerArm", "RightLowerArm"],
    "shins": ["LeftLowerLeg", "RightLowerLeg"],
}
for region, names in cohorts.items():
    batches = []
    for name in names:
        ids = np.flatnonzero((owner == name) & (weight >= 0.60))
        if len(ids) < 2:
            continue
        candidates = rng.choice(ids, (8192, 2))
        delta = rest[candidates[:, 0]] - rest[candidates[:, 1]]
        ij = candidates[np.linalg.norm(delta, axis=1) > 0.04 * height][:1024]
        if len(ij):
            batches.append(ij)
    pairs[region] = np.vstack(batches)
# Different dominant spine bones can bend a single breastplate across their boundary.
torso_ids = np.flatnonzero(np.isin(owner, cohorts["torso"]) & (weight >= 0.60))
cross = rng.choice(torso_ids, (16384, 2))
dist = np.linalg.norm(rest[cross[:, 0]] - rest[cross[:, 1]], axis=1)
pairs["across_torso_bones"] = cross[
    (owner[cross[:, 0]] != owner[cross[:, 1]]) & (dist > 0.04 * height)
][:2048]
assert len(pairs["across_torso_bones"]) > 0
lengths = {
    k: np.linalg.norm(rest[ij[:, 0]] - rest[ij[:, 1]], axis=1)
    for k, ij in pairs.items()
}
joints = {}
edges = np.array([e.vertices[:] for e in mesh.data.edges])
edge_len = np.linalg.norm(rest[edges[:, 0]] - rest[edges[:, 1]], axis=1)
for side in ("Left", "Right"):
    for region, suffix in [
        ("elbow", "LowerArm"),
        ("knee", "LowerLeg"),
        ("hip", "UpperLeg"),
    ]:
        bone = rig.data.bones[side + suffix]
        pivot = np.array(rig.matrix_world @ bone.head_local)
        mask = np.linalg.norm(rest - pivot, axis=1) <= 0.065 * height
        ids = np.flatnonzero(mask)
        assert len(ids) >= 10
        ie = np.flatnonzero(
            mask[edges[:, 0]] & mask[edges[:, 1]] & (edge_len > height * 0.0001)
        )
        joints[side + region] = (ids, ie)


def volume(points):
    return np.sqrt(np.linalg.det(np.cov(points.T)))


def measure(co):
    strain = {
        k: abs(np.linalg.norm(co[ij[:, 0]] - co[ij[:, 1]], axis=1) / lengths[k] - 1)
        for k, ij in pairs.items()
    }
    regional = {}
    for name, (ids, ie) in joints.items():
        ij = edges[ie]
        ratio = np.linalg.norm(co[ij[:, 0]] - co[ij[:, 1]], axis=1) / edge_len[ie]
        regional[name] = [
            volume(co[ids]) / volume(rest[ids]),
            np.percentile(ratio, 5),
            np.percentile(ratio, 95),
        ]
    return strain, regional


sc.render.engine = "CYCLES"
sc.cycles.device = "CPU"
sc.cycles.samples = 8
sc.cycles.use_denoising = True
sc.render.threads_mode = "FIXED"
sc.render.threads = 6
sc.render.resolution_x, sc.render.resolution_y = 504, 640
sc.render.resolution_percentage = 100
sc.world = bpy.data.worlds.new("Neutral lighting")
sc.world.use_nodes = True
sc.world.node_tree.nodes["Background"].inputs[0].default_value = (0.46, 0.49, 0.54, 1)
sc.world.node_tree.nodes["Background"].inputs[1].default_value = 0.62
sc.view_settings.view_transform = "AgX"
cam = bpy.data.objects.new("Evidence camera", bpy.data.cameras.new("Orthographic"))
sc.collection.objects.link(cam)
cam.data.type = "ORTHO"
cam.data.ortho_scale = height * 1.35
cam.data.clip_start = height * 0.001
sc.camera = cam


def aim(ob, dest):
    ob.rotation_euler = (dest - ob.location).to_track_quat("-Z", "Y").to_euler()


for direction, power in [
    ((-1.3, -2.2, 2.4), 88),
    ((1.6, -1, 0.9), 46),
    ((0.7, 1.7, 1.7), 64),
]:
    light = bpy.data.lights.new("Softbox", "AREA")
    ob = bpy.data.objects.new(light.name, light)
    sc.collection.objects.link(ob)
    ob.location = center + Vector(direction) * height
    light.energy, light.shape, light.size = power * height**2, "DISK", 1.7 * height
    aim(ob, center)


def render(name, yaw, upward=0):
    a = math.radians(yaw)
    cam.location = center + Vector((math.sin(a), -math.cos(a), upward)) * height * 3
    aim(cam, center)
    sc.render.filepath = str(OUT / (name + ".png"))
    bpy.ops.render.render(write_still=True)


def material(name, rgb):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (*rgb, 1)
    mat.use_nodes = True
    node = mat.node_tree.nodes.get("Principled BSDF")
    node.inputs["Base Color"].default_value = (*rgb, 1)
    node.inputs["Roughness"].default_value = 0.85
    return mat


# A fixed open grid marks the lowest rest-pose vertex; feet below it stay visible.
curve = bpy.data.curves.new("Bind-sole plane", "CURVE")
curve.dimensions, curve.bevel_depth = "3D", height * 0.00055
curve.resolution_u, curve.bevel_resolution = 1, 0
for k in range(-8, 9):
    for direction in (0, 1):
        line = curve.splines.new("POLY")
        line.points.add(1)
        for i, side in enumerate((-1, 1)):
            xy = [center.x, center.y]
            xy[direction] += k * height * 0.1
            xy[1 - direction] += side * height * 0.8
            line.points[i].co = (*xy, low[2], 1)
grid = bpy.data.objects.new("Ground reference only", curve)
sc.collection.objects.link(grid)
curve.materials.append(material("Ground lines", (0.095, 0.14, 0.2)))
ref_error = 0.0
report = {
    "fps": 30,
    "region_pairs": {k: len(v) for k, v in pairs.items()},
    "joint_vertices": {k: len(ids) for k, (ids, _) in joints.items()},
    "clips": {},
}
for clip, short in [("walk", "walk"), ("1h-melee-attack-chop", "chop")]:
    action = clips[clip]
    ad.action = action
    ad.action_slot = action.slots[0]
    first, last = [round(v) for v in action.frame_range]
    clearance = {s: [] for s in settings}
    rows = {s: [] for s in settings}
    for frame in range(first, last + 1):
        sc.frame_set(frame)
        expected = positions(reference)[mapping]
        original = positions(meshes["0"])
        err = np.max(np.linalg.norm(original - expected, axis=1))
        ref_error = max(ref_error, float(err))
        assert err < 0.0001, (clip, frame, err)
        for s, mesh in meshes.items():
            co = positions(mesh)
            rows[s].append(measure(co))
            clearance[s].append(float(co[:, 2].min() - low[2]))
            if frame in (3, 14, 26) and "--metrics-only" not in sys.argv:
                mesh.hide_render = False
                for view, yaw in [("oblique", 27), ("side", 83)]:
                    render(f"{short}_{frame:03}_{view}_{s}", yaw, 0.075)
                mesh.hide_render = True
    table = {}
    for s, samples in rows.items():
        strain = {
            k: float(np.percentile(np.concatenate([a[k] for a, _ in samples]), 95))
            for k in pairs
        }
        quality = {
            k: {
                "min_spread_ratio": float(min(b[k][0] for _, b in samples)),
                "min_edge_p05": float(min(b[k][1] for _, b in samples)),
                "max_edge_p95": float(max(b[k][2] for _, b in samples)),
            }
            for k in joints
        }
        table[s] = {
            "pair_strain_p95": strain,
            "joints": quality,
            "min_clearance_m": min(clearance[s]),
        }
    report["clips"][short] = {"frames": [first, last], "settings": table}
report["baseline_vs_v2_max_vertex_error_m"] = ref_error
(OUT / "metrics.json").write_text(json.dumps(report, indent=2) + "\n")
