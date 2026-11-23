"""Blender headless: build and render a simple stylised knight mannequin as a stand-in concept image.

    Blender -b --python tools/make_standin.py -- out.png

Primitives only (no downloaded assets), A-pose, open hands, plain light grey background,
soft even light, 20-degree perspective camera (the gauge camera Pixal3D's single view expects).
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
    m.use_nodes = True
    p = m.node_tree.nodes["Principled BSDF"]
    p.inputs["Base Color"].default_value = (*rgb, 1)
    p.inputs["Metallic"].default_value = metal
    p.inputs["Roughness"].default_value = rough
    return m


STEEL = material("steel", (0.45, 0.47, 0.52), 0.6, 0.45)
CLOTH = material("cloth", (0.12, 0.2, 0.45), 0.0, 0.8)
LEATHER = material("leather", (0.32, 0.18, 0.08), 0.0, 0.7)
SKIN = material("skin", (0.85, 0.62, 0.48), 0.0, 0.55)
GOLD = material("gold", (0.85, 0.65, 0.2), 0.9, 0.3)


def part(kind, loc, scale, mat, rot=(0, 0, 0), bevel=0.0):
    if kind == "sphere":
        bpy.ops.mesh.primitive_uv_sphere_add(segments=48, ring_count=24, location=loc)
    elif kind == "cyl":
        bpy.ops.mesh.primitive_cylinder_add(vertices=48, location=loc)
    else:
        bpy.ops.mesh.primitive_cube_add(location=loc)
    ob = bpy.context.object
    ob.scale = scale
    ob.rotation_euler = rot
    ob.data.materials.append(mat)
    if bevel:
        mod = ob.modifiers.new("b", "BEVEL")
        mod.width, mod.segments = bevel, 4
    bpy.ops.object.shade_smooth()
    return ob


def limb(a, b, radius, mat, kind="cyl"):
    a, b = Vector(a), Vector(b)
    d = b - a
    ob = part(kind, (a + b) / 2, (radius, radius, d.length / 2), mat)
    ob.rotation_euler = d.to_track_quat("Z", "Y").to_euler()
    for end in (a, b):
        part("sphere", end, (radius, radius, radius), mat)
    return ob


# legs (feet slightly apart), boots
for s in (-1, 1):
    limb((0.11 * s, 0, 0.95), (0.12 * s, 0, 0.52), 0.075, CLOTH)
    limb((0.12 * s, 0, 0.52), (0.13 * s, 0, 0.12), 0.065, STEEL)
    part("cube", (0.13 * s, -0.04, 0.05), (0.07, 0.13, 0.05), LEATHER, bevel=0.03)
# hips, belt, torso, breastplate, pauldrons
part("cube", (0, 0, 0.98), (0.2, 0.12, 0.09), CLOTH, bevel=0.05)
part("cube", (0, 0, 1.08), (0.21, 0.13, 0.035), LEATHER, bevel=0.02)
part("cube", (0, 0, 1.08), (0.04, 0.135, 0.03), GOLD, bevel=0.01)
part("cube", (0, 0, 1.33), (0.22, 0.14, 0.22), STEEL, bevel=0.08)
for s in (-1, 1):
    part("sphere", (0.25 * s, 0, 1.5), (0.1, 0.1, 0.08), STEEL)
# arms in an A-pose: about 45 degrees down, a clear gap from the body; open hands
for s in (-1, 1):
    shoulder = Vector((0.27 * s, 0, 1.47))
    down = Vector((math.sin(math.radians(45)) * s, 0, -math.cos(math.radians(45))))
    elbow = shoulder + down * 0.28
    wrist = elbow + down * 0.26
    limb(shoulder, elbow, 0.055, CLOTH)
    limb(elbow, wrist, 0.05, STEEL)
    across = Vector((-down.z, 0, down.x))  # in the picture plane, across the palm
    hand = wrist + down * 0.07
    palm = part("cube", hand, (0.05, 0.022, 0.05), LEATHER, bevel=0.015)
    palm.rotation_euler = (0, -math.radians(45) * s, 0)
    for i in range(4):
        base = hand + down * 0.045 + across * (-0.033 + i * 0.022)
        limb(base, base + down * 0.07, 0.01, LEATHER)
    thumb = hand - across * 0.05 * s + down * 0.0
    limb(thumb, thumb + (down * 0.03 - across * 0.04 * s), 0.011, LEATHER)
# neck, head, helmet with an open face
limb((0, 0, 1.55), (0, 0, 1.63), 0.05, SKIN)
part("sphere", (0, -0.005, 1.74), (0.105, 0.115, 0.13), SKIN)
for s in (-1, 1):
    part("sphere", (0.038 * s, -0.105, 1.76), (0.016, 0.01, 0.016), material("eye", (0.05, 0.05, 0.08)))
helmet = part("sphere", (0, 0.02, 1.79), (0.13, 0.13, 0.12), STEEL)
part("cube", (0, 0.0, 1.9), (0.015, 0.14, 0.04), GOLD, bevel=0.01)

# light grey studio: world light + one soft key, no hard shadows
world = bpy.data.worlds.new("w")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs[0].default_value = (0.78, 0.78, 0.8, 1)
world.node_tree.nodes["Background"].inputs[1].default_value = 1.0
scene.world = world
key = bpy.data.objects.new("key", bpy.data.lights.new("key", "AREA"))
key.data.energy, key.data.size = 300, 4
key.location = (1.5, -3, 3)
key.rotation_euler = Vector((-1.5, 3, -2)).to_track_quat("-Z", "Y").to_euler()
scene.collection.objects.link(key)

cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
cam.data.angle = math.radians(20)
cam.location = (0, -6.5, 1.0)
cam.rotation_euler = (math.radians(90), 0, 0)
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
