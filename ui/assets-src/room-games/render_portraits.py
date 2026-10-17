"""Render menu portraits / item icons for the room games with Blender (headless).

  /Applications/Blender.app/Contents/MacOS/Blender -b --factory-startup \
      --python ui/assets-src/room-games/render_portraits.py -- <packs-dir> <out-dir>

Each job imports one Kenney CC0 model (optionally a driver posed on a kart),
frames it with a three-quarter camera and renders a transparent PNG. The
renders are derived from CC0 models and are themselves CC0.
"""
import math
import os
import sys

import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
PACKS, OUT = argv[0], argv[1]
os.makedirs(OUT, exist_ok=True)

GLB = {
    "car-kit": "car-kit/Models/GLB format",
    "mini-characters": "mini-characters/Models/GLB format",
    "platformer-kit": "platformer-kit/Models/GLB format",
    "food-kit": "food-kit/Models/GLB format",
    "toy-car-kit": "toy-car-kit/Models/GLB format",
    "blaster-kit": "blaster-kit/Models/GLB format",
}

# (output name, [(pack, file, offset, hide-node-name)], size, yaw degrees, elevation degrees)
KARTS = ["oobi", "oodi", "ooli", "oopi", "oozi"]
MINI_DRIVERS = [("kai", "character-male-c", "kart-oodi"), ("rosa", "character-female-f", "kart-oozi"), ("dex", "character-male-a", "kart-ooli")]
JOBS = []
for k in KARTS:
    JOBS.append((f"racer-{k}", [("car-kit", f"kart-{k}", (0, 0, 0), None)], 320, 35, 18))
for name, char, kart in MINI_DRIVERS:
    JOBS.append((f"racer-{name}", [("car-kit", kart, (0, 0, 0), "character"), ("mini-characters", char, (0, 0.2, -0.12), None)], 320, 35, 18))
for char in ["male-a", "male-b", "male-c", "male-d", "male-e", "male-f", "female-a", "female-b", "female-c", "female-d", "female-e", "female-f"]:
    JOBS.append((f"fighter-{char}", [("mini-characters", f"character-{char}", (0, 0, 0), None)], 320, 25, 8))
for name, pack, file in [("item-mushroom", "platformer-kit", "mushrooms"), ("item-banana", "food-kit", "banana"), ("item-bomb", "platformer-kit", "bomb"), ("item-star", "platformer-kit", "star"), ("item-box", "toy-car-kit", "item-box")]:
    JOBS.append((name, [(pack, file, (0, 0, 0), None)], 160, 30, 20))
for name, file in [("gun-pistol", "blaster-a"), ("gun-smg", "blaster-j"), ("gun-rifle", "blaster-d"), ("gun-shotgun", "blaster-l"), ("gun-sniper", "blaster-e"), ("gun-rocket", "blaster-h")]:
    JOBS.append((name, [("blaster-kit", file, (0, 0, 0), None)], 200, 90, 4))


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE_NEXT" if "BLENDER_EEVEE_NEXT" in [e.identifier for e in bpy.types.RenderSettings.bl_rna.properties["engine"].enum_items] else "BLENDER_EEVEE"
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.view_settings.view_transform = "Standard"
    world = bpy.data.worlds.new("w")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[1].default_value = 0.9
    scene.world = world
    return scene


def light(name, kind, energy, rot):
    data = bpy.data.lights.new(name, kind)
    data.energy = energy
    obj = bpy.data.objects.new(name, data)
    obj.rotation_euler = [math.radians(a) for a in rot]
    bpy.context.collection.objects.link(obj)


for out, parts, size, yaw, elev in JOBS:
    scene = reset()
    scene.render.resolution_x = scene.render.resolution_y = size
    for pack, file, offset, hide in parts:
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=os.path.join(PACKS, GLB[pack], f"{file}.glb"))
        new = [o for o in bpy.data.objects if o not in before]
        for o in new:
            # The importer adds an "Icosphere" bone-shape helper; it is not part of the model.
            if (hide and o.name.startswith(hide)) or o.name.startswith("Icosphere"):
                bpy.data.objects.remove(o, do_unlink=True)
                continue
            if o.parent is None:
                # glTF +Y up/-Z forward maps to Blender Z up; offsets are glTF (x, y, z).
                o.location = Vector((offset[0], -offset[2], offset[1]))
        # Seat mini characters with their drive pose.
        for o in [o for o in bpy.data.objects if o not in before and o.type == "ARMATURE"]:
            action = bpy.data.actions.get("drive")
            if action and file.startswith("character-") and len(parts) > 1:
                o.animation_data_create()
                o.animation_data.action = action
                scene.frame_set(1)
            elif bpy.data.actions.get("idle") and file.startswith("character-"):
                o.animation_data_create()
                o.animation_data.action = bpy.data.actions.get("idle")
                scene.frame_set(1)
    meshes = [o for o in bpy.data.objects if o.type == "MESH"]
    bpy.context.view_layer.update()
    deps = bpy.context.evaluated_depsgraph_get()
    pts = []
    for o in meshes:
        ev = o.evaluated_get(deps)
        mesh = ev.to_mesh()
        pts += [ev.matrix_world @ v.co for v in mesh.vertices]
        ev.to_mesh_clear()
    lo = Vector((min(p.x for p in pts), min(p.y for p in pts), min(p.z for p in pts)))
    hi = Vector((max(p.x for p in pts), max(p.y for p in pts), max(p.z for p in pts)))
    centre, radius = (lo + hi) / 2, (hi - lo).length / 2
    cam_data = bpy.data.cameras.new("cam")
    cam_data.lens = 50
    cam = bpy.data.objects.new("cam", cam_data)
    bpy.context.collection.objects.link(cam)
    dist = radius / math.tan(cam_data.angle / 2) * 1.08
    y, e = math.radians(yaw), math.radians(elev)
    # Front of Kenney models faces glTF +Z, i.e. Blender -Y.
    cam.location = centre + Vector((math.sin(y) * math.cos(e), -math.cos(y) * math.cos(e), math.sin(e))) * dist
    cam.rotation_euler = (centre - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = cam
    light("key", "SUN", 3.5, (50, 0, 35))
    light("fill", "SUN", 1.2, (60, 0, -120))
    scene.render.filepath = os.path.join(OUT, f"{out}.png")
    bpy.ops.render.render(write_still=True)
    print("rendered", out)
