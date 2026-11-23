"""Blender headless: render a GLB from front, left, back and right into one PNG strip.

    Blender -b --python tools/render_views.py -- model.glb out.png [--clip walk] [--at 0.5]

With --clip, the armature plays the action whose name contains CLIP and the strip shows the
frame at fraction --at of it (one frame mid-walk, so deformation problems are visible).
"""

import math
import sys

import bpy
import numpy as np
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
glb, out = argv[0], argv[1]
clip = argv[argv.index("--clip") + 1] if "--clip" in argv else None
at = float(argv[argv.index("--at") + 1]) if "--at" in argv else 0.5

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
scene = bpy.context.scene

if clip:
    action = next(a for a in bpy.data.actions if clip in a.name)
    start, end = action.frame_range
    for ob in scene.objects:
        if ob.type == "ARMATURE":
            ad = ob.animation_data or ob.animation_data_create()
            for track in list(ad.nla_tracks):
                ad.nla_tracks.remove(track)
            ad.action = action
            if hasattr(ad, "action_slot") and action.slots:
                ad.action_slot = action.slots[0]
    scene.frame_set(int(round(start + (end - start) * at)))
    print(f"clip {action.name}: frames {start:.0f}-{end:.0f}, showing {scene.frame_current}")

# the glTF importer adds an icosphere as the bones' display shape: not part of the model
shapes = {pb.custom_shape for ob in scene.objects if ob.type == "ARMATURE" for pb in ob.pose.bones}
for ob in scene.objects:
    if ob in shapes or ob.type == "ARMATURE" or "_LOD" in ob.name or ob.name.startswith("UCX_"):
        ob.hide_render = True
depsgraph = bpy.context.evaluated_depsgraph_get()
points = []
for ob in scene.objects:
    if ob.type == "MESH" and not ob.hide_render:
        mesh = ob.evaluated_get(depsgraph).to_mesh()
        points += [ob.matrix_world @ v.co for v in mesh.vertices]
lo = Vector([min(p[i] for p in points) for i in range(3)])
hi = Vector([max(p[i] for p in points) for i in range(3)])
centre, size = (lo + hi) / 2, hi - lo
print(f"bounds {tuple(round(x, 3) for x in size)}")

world = bpy.data.worlds.new("w")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.6, 0.6, 0.62, 1)
world.node_tree.nodes["Background"].inputs[1].default_value = 1.0
scene.world = world
sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
sun.data.energy = 3.0
sun.rotation_euler = (math.radians(50), 0, math.radians(30))
scene.collection.objects.link(sun)

scene.render.engine = "BLENDER_EEVEE"
scene.render.resolution_x, scene.render.resolution_y = 512, 768
scene.render.film_transparent = False
scene.view_settings.view_transform = "Standard"
cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
cam.data.type = "ORTHO"
cam.data.ortho_scale = max(size.z, max(size.x, size.y) * 1.5) * 1.1
scene.collection.objects.link(cam)
scene.camera = cam

tiles = []
distance = max(size) * 3
for name, angle in (("front", 0), ("left", 90), ("back", 180), ("right", 270)):  # the model's own left/right
    a = math.radians(angle)
    # glTF +Z (the model's front) is Blender -Y
    cam.location = centre + Vector((distance * math.sin(a), -distance * math.cos(a), 0))
    cam.rotation_euler = (math.radians(90), 0, a)
    path = f"{out[:-4]}_{name}.png"
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    img = bpy.data.images.load(path)
    px = np.empty(len(img.pixels), dtype=np.float32)
    img.pixels.foreach_get(px)
    tiles.append(px.reshape(img.size[1], img.size[0], 4))

strip = np.concatenate(tiles, axis=1)
result = bpy.data.images.new("strip", strip.shape[1], strip.shape[0], alpha=True)
result.pixels.foreach_set(strip.ravel())
result.filepath_raw = out
result.file_format = "PNG"
result.save()
print(f"wrote {out}")
