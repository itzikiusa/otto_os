"""Blender headless: build and render a wooden treasure chest as a stand-in prop concept image.

    Blender -b --python tools/make_standin_chest.py -- out.png
"""

import math
import sys

import bpy
from mathutils import Vector

out = sys.argv[sys.argv.index("--") + 1]
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene


def material(name, rgb, metal=0.0, rough=0.6):
    m = bpy.data.materials.new(name)
    p = m.node_tree.nodes["Principled BSDF"] if m.node_tree else None
    if p is None:
        m.use_nodes = True
        p = m.node_tree.nodes["Principled BSDF"]
    p.inputs["Base Color"].default_value = (*rgb, 1)
    p.inputs["Metallic"].default_value = metal
    p.inputs["Roughness"].default_value = rough
    return m


WOOD = material("wood", (0.36, 0.2, 0.09), 0.0, 0.75)
DARK = material("darkwood", (0.22, 0.12, 0.05), 0.0, 0.8)
IRON = material("iron", (0.25, 0.25, 0.27), 0.9, 0.4)
GOLD = material("gold", (0.85, 0.62, 0.18), 1.0, 0.3)


def box(loc, scale, mat, bevel=0.01):
    bpy.ops.mesh.primitive_cube_add(location=loc)
    ob = bpy.context.object
    ob.scale = scale
    ob.data.materials.append(mat)
    mod = ob.modifiers.new("b", "BEVEL")
    mod.width, mod.segments = bevel, 3
    return ob


def lid_cyl(loc, radius, depth, mat):
    bpy.ops.mesh.primitive_cylinder_add(vertices=64, radius=radius, depth=depth, location=loc,
                                        rotation=(0, math.radians(90), 0))
    ob = bpy.context.object
    ob.data.materials.append(mat)
    bpy.ops.object.shade_smooth()
    return ob


# body: planks + iron bands + corner caps
box((0, 0, 0.3), (0.6, 0.38, 0.3), WOOD)
for z in (0.12, 0.3, 0.48):
    box((0, -0.381, z), (0.58, 0.004, 0.006), DARK, 0.002)   # plank grooves
for x in (-0.42, 0.42):
    box((x, 0, 0.3), (0.035, 0.39, 0.31), IRON, 0.008)
for x in (-0.6, 0.6):
    for y in (-0.38, 0.38):
        box((x, y, 0.3), (0.03, 0.03, 0.31), IRON, 0.008)
# rounded lid
lid_cyl((0, 0, 0.6), 0.375, 1.19, WOOD)
box((0, 0, 0.6), (0.6, 0.38, 0.01), DARK, 0.002)
for x in (-0.42, 0.42):
    lid_cyl((x, 0, 0.6), 0.395, 0.07, IRON)
# the lid's lower half sits inside the body box and is never seen
# lock plate + keyhole
box((0, -0.39, 0.55), (0.07, 0.015, 0.09), GOLD, 0.01)
box((0, -0.406, 0.53), (0.012, 0.003, 0.03), material("hole", (0.02, 0.02, 0.02)), 0.002)

world = bpy.data.worlds.new("w")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.78, 0.78, 0.8, 1)
scene.world = world
key = bpy.data.objects.new("key", bpy.data.lights.new("key", "AREA"))
key.data.energy, key.data.size = 300, 4
key.location = (2, -3, 3.5)
key.rotation_euler = (-key.location).to_track_quat("-Z", "Y").to_euler()
scene.collection.objects.link(key)

cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
cam.data.angle = math.radians(20)
target = Vector((0, 0, 0.45))
cam.location = target + Vector((3.2, -5.0, 2.6))
cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
scene.collection.objects.link(cam)
scene.camera = cam
scene.render.engine = "CYCLES"
scene.cycles.samples = 64
scene.cycles.device = "GPU"
scene.render.resolution_x = scene.render.resolution_y = 1024
scene.view_settings.view_transform = "Standard"
scene.render.filepath = out
bpy.ops.render.render(write_still=True)
print("wrote", out)
