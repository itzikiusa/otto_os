"""Blender worker: one clip of any body as a strip of frames, side and three-quarter, walking over a floor.

Blender -b --factory-startup -P tests/rig_any/strip.py -- --glb ANIMATED.glb --clip walk --out STRIP.png
    [--frames 8] [--forward x,y,z (glTF)] [--speed M_PER_S]

With --speed the model is moved forward at that speed (the speed the manifest gives a game controller),
over a striped floor: a planted foot must stay on the same stripe from frame to frame, or it slides.
"""

import argparse
import sys
from pathlib import Path

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "forge/fit"))
import render  # noqa: E402

a = argparse.ArgumentParser()
a.add_argument("--glb", required=True)
a.add_argument("--clip", required=True)
a.add_argument("--out", required=True)
a.add_argument("--frames", type=int, default=8)
a.add_argument("--forward", default="0,0,1")
a.add_argument("--speed", type=float, default=0.0)
args = a.parse_args(sys.argv[sys.argv.index("--") + 1:])

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.render.fps = 60
before = set(bpy.data.objects)
bpy.ops.import_scene.gltf(filepath=args.glb)
objs = set(bpy.data.objects) - before
rig = next(o for o in objs if o.type == "ARMATURE")
rng = None
for ob in objs:
    ad = ob.animation_data
    if not ad:
        continue
    pick = next(((t.strips[0].action, t.strips[0].action_slot) for t in ad.nla_tracks if t.name == args.clip and len(t.strips)), None)
    while ad.nla_tracks:
        ad.nla_tracks.remove(ad.nla_tracks[0])
    if pick:
        ad.action, ad.action_slot = pick
        if ob.type == "ARMATURE":
            rng = tuple(pick[0].frame_range)
if rng is None:
    raise SystemExit(f"{args.glb}: no clip named {args.clip!r}")
s0, s1 = rng
meshes = [o for o in objs if o.type == "MESH" and o.find_armature() == rig]
for o in objs:
    if o.type == "MESH" and o not in meshes:
        o.hide_render = True  # LOD copies, collision hulls, the bone display icosphere
fx, fy, fz = (float(v) for v in args.forward.split(","))
fwd = Vector((fx, -fz, 0)).normalized()  # glTF (x, y, z) is Blender (x, -z, y)
side = fwd.cross(Vector((0, 0, 1))).normalized()
duration = (s1 - s0) / scene.render.fps
root = rig  # the armature object carries the root motion for the picture
base = root.location.copy()

pts = [o.matrix_world @ Vector(c) for o in meshes for c in o.bound_box]
lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
size = max(hi - lo)
travel = args.speed * duration
centre = (lo + hi) / 2 + fwd * travel / 2
floor_z = lo.z

# a striped floor across the walking direction
mat_a = bpy.data.materials.new("FloorA")
mat_a.diffuse_color = (0.30, 0.32, 0.36, 1)
mat_b = bpy.data.materials.new("FloorB")
mat_b.diffuse_color = (0.85, 0.55, 0.20, 1)
span = size * 1.6 + travel
step = max(size / 12, 0.01)
count = int(span / step) + 2
for i in range(-count, count):
    bpy.ops.mesh.primitive_cube_add(size=1)
    tile = bpy.context.active_object
    tile.scale = (step * 0.5, size * 3, 0.002 * size) if abs(fwd.x) > abs(fwd.y) else (size * 3, step * 0.5, 0.002 * size)
    tile.location = centre + fwd * (i * step) + Vector((0, 0, floor_z - centre.z - 0.002 * size))
    tile.data.materials.append(mat_a if i % 2 else mat_b)

cam, label = render.setup(res=300)
frames = [s0 + (s1 - s0) * i / (args.frames - 1) for i in range(args.frames)]
shots = []
work = Path(args.out).with_suffix("")
work.mkdir(parents=True, exist_ok=True)
view_span = size * 1.35 + travel
for name, direction in (("side", side), ("3/4", (side + fwd * 0.9 + Vector((0, 0, 0.45))).normalized())):
    for i, f in enumerate(frames):
        scene.frame_set(int(f), subframe=f - int(f))
        t = (f - s0) / scene.render.fps
        root.location = base + fwd * (args.speed * t)
        bpy.context.view_layer.update()
        render.view(cam, label, centre, direction, view_span, f"{args.clip} {name} t={t:.2f}s")
        shots.append(render.shot(work / f"{len(shots):02d}.png"))
render.sheet(shots, args.frames, args.out)
for p in shots:
    Path(p).unlink()
work.rmdir()
print("FORGE_STRIP", args.out)
