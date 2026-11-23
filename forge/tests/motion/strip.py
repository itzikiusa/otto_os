"""Blender worker: frame strips of one clip from the front and the side, plus hand-to-anchor distances.

Blender -b --factory-startup -P tests/motion/strip.py -- --glb CLIP.glb --clip NAME --out STRIP.png
    [--frames 8] [--height 1.8] [--report R.json]
    [--ref REF.glb --ref-clip NAME --anchor NODE [--anchor-bone BONE] [--ref-frame F]]

With --ref, the anchor node (e.g. sword.grip, quiver.opening from a forge-fit reach output on the same
character) is read at --ref-frame of the reference clip, either in world space or, with --anchor-bone,
relative to that bone (a quiver on the back moves with the chest). The report gives, per hand, the
closest approach of the palm centre to the anchor over the clip, in metres at --height.
"""

import argparse
import json
import re
import sys
from pathlib import Path

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "forge/fit"))
import render  # noqa: E402

a = argparse.ArgumentParser()
for k in ("glb", "clip", "out"):
    a.add_argument("--" + k, required=True)
a.add_argument("--frames", type=int, default=8)
a.add_argument("--height", type=float, default=1.8)
a.add_argument("--report")
a.add_argument("--ref")
a.add_argument("--ref-clip")
a.add_argument("--anchor")
a.add_argument("--anchor-bone")
a.add_argument("--ref-frame", type=float, default=0)
args = a.parse_args(sys.argv[sys.argv.index("--") + 1:])


def load(path, clip):
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path)
    objs = set(bpy.data.objects) - before
    rng = None
    for ob in objs:
        ad = ob.animation_data
        if not ad:
            continue
        pick = next(((t.strips[0].action, t.strips[0].action_slot) for t in ad.nla_tracks
                     if t.name == clip and len(t.strips)), None)
        while ad.nla_tracks:
            ad.nla_tracks.remove(ad.nla_tracks[0])
        if pick:
            ad.action, ad.action_slot = pick
            if ob.type == "ARMATURE":
                rng = tuple(pick[0].frame_range)
    rig = next(o for o in objs if o.type == "ARMATURE")
    if rng is None:
        raise SystemExit(f"{path}: no clip named {clip!r}")
    return rig, objs, rng


def bone(rig, *words):
    for b in rig.pose.bones:
        n = re.sub(r"[^a-z0-9]", "", b.name.split(":")[-1].lower())
        if all(w(n) if callable(w) else w in n for w in words):
            return b
    return None


def palms(rig):
    out = {}
    for side in ("left", "right"):
        hand = bone(rig, side, lambda n: n.endswith("hand"))
        mid = bone(rig, side, "middle", lambda n: "proximal" in n or n.endswith("1"))
        out[side] = (hand, mid)
    return out


def palm_point(rig, pair):
    hand, mid = pair
    p = rig.matrix_world @ hand.head
    return (p + rig.matrix_world @ mid.head) / 2 if mid else p


bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.fps = 60
anchor = None
if args.ref:
    ref, ref_objs, _ = load(args.ref, args.ref_clip)
    scene.frame_set(int(args.ref_frame))
    node = next(o for o in ref_objs if o.name == args.anchor)
    anchor = node.matrix_world.translation.copy()
    if args.anchor_bone:
        rb = ref.pose.bones[args.anchor_bone]
        anchor = (ref.matrix_world @ rb.matrix).inverted() @ anchor
    for o in ref_objs:
        bpy.data.objects.remove(o, do_unlink=True)

rig, objs, (s0, s1) = load(args.glb, args.clip)
meshes = [o for o in objs if o.type == "MESH" and o.find_armature() == rig]  # not the bone display icosphere
zs = [(o.matrix_world @ Vector(c)).z for o in meshes for c in o.bound_box]
units_per_m = (max(zs) - min(zs)) / args.height
foot, toe = bone(rig, "left", "foot"), bone(rig, "left", "toe")
fwd = (toe.head - foot.head) if toe and foot else Vector((0, -1, 0))
fwd = (rig.matrix_world.to_3x3() @ fwd)
fwd.z = 0
fwd = fwd.normalized() if fwd.length > 1e-6 else Vector((0, -1, 0))
side = fwd.cross(Vector((0, 0, 1))).normalized()

frames = [s0 + (s1 - s0) * i / (args.frames - 1) for i in range(args.frames)]
hp = palms(rig)
track = []
for f in [s0 + (s1 - s0) * i / 120 for i in range(121)]:
    scene.frame_set(int(f), subframe=f - int(f))
    hips = rig.matrix_world @ bone(rig, "hips").head
    row = {"frame": f, "t": (f - s0) / scene.render.fps, "hips": list(hips)}
    for k, pair in hp.items():
        row[k] = list(palm_point(rig, pair))
    if anchor is not None:
        at = anchor
        if args.anchor_bone:
            at = rig.matrix_world @ rig.pose.bones[args.anchor_bone].matrix @ anchor
        row["anchor"] = list(at)
        for k in hp:
            row[k + "_to_anchor_m"] = (Vector(row[k]) - at).length / units_per_m
    track.append(row)

xs = [r["hips"] for r in track]
centre = Vector([sum(c) / len(xs) for c in zip(*xs)])
centre.z = (max(zs) + min(zs)) / 2
travel = max((Vector(x) - Vector(xs[0])).to_2d().length for x in xs)  # horizontal
span = (max(zs) - min(zs)) * 1.25 + travel
cam, label = render.setup(res=300)
shots = []
work = Path(args.out).with_suffix("")
work.mkdir(parents=True, exist_ok=True)
for name, direction in (("front", fwd), ("side", side)):
    for i, f in enumerate(frames):
        scene.frame_set(int(f), subframe=f - int(f))
        render.view(cam, label, centre, direction, span, f"{name} t={(f - s0) / scene.render.fps:.2f}s")
        shots.append(render.shot(work / f"{name}_{i}.png"))
render.sheet(shots, args.frames, args.out)
for p in shots:
    Path(p).unlink()
work.rmdir()
report = {"clip": args.clip, "duration_s": (s1 - s0) / scene.render.fps, "units_per_m": units_per_m,
          "hips_travel_m": travel / units_per_m}
sep = [((Vector(r["left"]) - Vector(r["right"])).length / units_per_m, r["t"]) for r in track]
report["max_hand_separation_m"], report["max_hand_separation_t"] = (round(x, 3) for x in max(sep))
lowest = min(track, key=lambda r: min(r["left"][2], r["right"][2]))
report["lowest_hand_height_m"] = round(min(lowest["left"][2], lowest["right"][2]) / units_per_m - min(zs) / units_per_m, 3)
if anchor is not None:
    for k in hp:
        best = min(track, key=lambda r: r[k + "_to_anchor_m"])
        report[k + "_closest_m"] = round(best[k + "_to_anchor_m"], 3)
        report[k + "_closest_t"] = round(best["t"], 2)
    report["anchor"] = args.anchor
if args.report:
    Path(args.report).write_text(json.dumps({**report, "track": track}, indent=1))
print("FORGE_STRIP", json.dumps(report))
