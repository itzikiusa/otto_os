"""Evidence renders for the gameready stage: 4-view turnaround of the source high-poly, the LOD0 with its baked
maps, and LOD0 as wireframe, stitched into one PNG (rows in that order; views front, right, back, left).
  Blender -b --factory-startup --python turnaround.py -- SOURCE.glb GAMEREADY.glb|LOD0.fbx OUT.png [--size 512]
"""
import math
import os
import re
import sys

import bpy
import numpy as np
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
SRC, LOW, PNG = argv[:3]
SIZE = int(argv[argv.index("--size") + 1]) if "--size" in argv else 512


def load(path, keep_lod0_only):
    before = set(bpy.data.objects)
    if path.lower().endswith(".fbx"):  # FBX keeps quads, so the wireframe shows the real topology
        bpy.ops.import_scene.fbx(filepath=path)
    else:
        bpy.ops.import_scene.gltf(filepath=path)
    objs = [o for o in set(bpy.data.objects) - before if o.type == "MESH"]
    if keep_lod0_only:
        objs = [o for o in objs if not o.name.startswith("UCX_") and not re.search(r"_LOD\d+$", o.name)]
    return objs


def frame(objs):
    pts = [o.matrix_world @ Vector(c) for o in objs for c in o.bound_box]
    lo = Vector([min(p[i] for p in pts) for i in range(3)])
    hi = Vector([max(p[i] for p in pts) for i in range(3)])
    return (lo + hi) / 2, (hi - lo).length


def render(objs, engine, tag):
    scene = bpy.context.scene
    for o in bpy.data.objects:
        if o.type == "MESH":
            o.hide_render = o not in objs
    scene.render.engine = engine
    scene.render.resolution_x = scene.render.resolution_y = SIZE
    scene.render.film_transparent = False
    centre, diag = frame(objs)
    cam = scene.camera
    cam.data.type = "ORTHO"
    cam.data.ortho_scale = diag * 0.85
    tiles = []
    for k, az in enumerate((-90, 0, 90, 180)):  # front (-Y), right (+X), back, left
        a, el = math.radians(az), math.radians(12)
        cam.location = centre + Vector((math.cos(a) * math.cos(el), math.sin(a) * math.cos(el), math.sin(el))) * diag * 2
        cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = f"{PNG}.{tag}{k}.png"
        bpy.ops.render.render(write_still=True)
        img = bpy.data.images.load(scene.render.filepath)
        px = np.empty(SIZE * SIZE * 4, np.float32)
        img.pixels.foreach_get(px)
        tiles.append(px.reshape(SIZE, SIZE, 4))
        bpy.data.images.remove(img)
        os.remove(scene.render.filepath)
    return np.concatenate(tiles, axis=1)


bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
scene.collection.objects.link(cam)
scene.camera = cam
sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
sun.data.energy = 3.0
sun.rotation_euler = (math.radians(50), 0, math.radians(30))
scene.collection.objects.link(sun)
scene.world = bpy.data.worlds.new("w")
scene.world.color = (0.35, 0.35, 0.37)
scene.world.use_nodes = True
scene.world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.35, 0.35, 0.37, 1)
scene.view_settings.view_transform = "Standard"

high = load(SRC, False)
low = load(LOW, True)
rows = [render(high, "BLENDER_EEVEE", "high"), render(low, "BLENDER_EEVEE", "low")]

wire_mat = bpy.data.materials.new("wire")
wire_mat.diffuse_color = (0.02, 0.02, 0.02, 1)
for o in low:
    for m in o.data.materials:
        m.diffuse_color = (0.8, 0.8, 0.8, 1)
    o.data.materials.append(wire_mat)
    w = o.modifiers.new("wire", "WIREFRAME")
    w.use_replace, w.material_offset = False, len(o.data.materials) - 1
    w.thickness = frame(low)[1] * 0.0012
scene.display.shading.color_type = "MATERIAL"
scene.display.shading.light = "STUDIO"
rows.append(render(low, "BLENDER_WORKBENCH", "wire"))

sheet = np.concatenate(rows[::-1], axis=0)  # Blender images are bottom-up: first row ends on top
out = bpy.data.images.new("sheet", sheet.shape[1], sheet.shape[0], alpha=True)
out.pixels.foreach_set(sheet.ravel())
out.filepath_raw = PNG
out.file_format = "PNG"
out.save()
print("wrote", PNG)
