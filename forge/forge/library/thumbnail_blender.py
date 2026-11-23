"""Blender background catalog preview rendering. Frame the evaluated displayed mesh."""

import bpy
import json
from mathutils import Vector
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from forge.library.gltf import pack_glb
from forge.library.import_packs import LIB


def render(row, scratch):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    source = LIB / row["file"]
    tmp = scratch / "preview.glb"
    pack_glb(source, LIB / row["pack"], tmp, row.get("action"))
    bpy.ops.import_scene.gltf(filepath=str(tmp), import_pack_images=True)
    scene = bpy.context.scene
    if row.get("action"):
        for o in scene.objects:
            ad = o.animation_data
            if ad:
                for track in ad.nla_tracks:
                    track.mute = True
                action = next(
                    (a for a in bpy.data.actions if a.name == row["action"]), None
                )
                if action and o.type == "ARMATURE":
                    ad.action = action
                    if action.slots:
                        ad.action_slot = action.slots[0]
                    f0, f1 = action.frame_range
                    scene.frame_set(round(f0 + 0.37 * (f1 - f0)))
    bpy.context.view_layer.update()
    deps = bpy.context.evaluated_depsgraph_get()
    points = []
    for item in scene.objects:
        if item.type == "MESH":
            obj = item.evaluated_get(deps)
            points.extend(obj.matrix_world @ Vector(pt) for pt in obj.bound_box)
    if not points:
        raise ValueError("no display geometry: " + row["id"])
    camera = bpy.data.cameras.new("Preview")
    obj = bpy.data.objects.new("Preview", camera)
    scene.collection.objects.link(obj)
    scene.camera = obj
    direction = Vector((0.58, -1, 0.44)).normalized()
    obj.rotation_euler = (-direction).to_track_quat("-Z", "Y").to_euler()
    frame = obj.rotation_euler.to_matrix()
    coords = [frame.transposed() @ pt for pt in points]
    low = Vector([min(pt[i] for pt in coords) for i in range(3)])
    high = Vector([max(pt[i] for pt in coords) for i in range(3)])
    center = frame @ ((low + high) / 2)
    span = high - low
    scale = max(span.x, span.y, 0.001) * 1.09
    obj.location = center + direction * (span.z + scale + 1)
    camera.type = "ORTHO"
    camera.ortho_scale = scale
    camera.clip_start = 0.0001
    camera.clip_end = 4 * (span.z + scale + 1)
    scene.render.engine = "BLENDER_WORKBENCH"
    display = scene.display.shading
    display.light = "STUDIO"
    display.color_type = "TEXTURE"
    display.background_type = "VIEWPORT"
    display.background_color = (0.16, 0.18, 0.20)
    display.show_shadows = True
    display.show_cavity = False
    scene.display.render_aa = "FXAA"
    scene.view_settings.view_transform = "Standard"
    scene.render.resolution_x = 192
    scene.render.resolution_y = 192
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = False
    dest = LIB / row["thumbnail"]
    dest.parent.mkdir(exist_ok=True, parents=True)
    partial = dest.with_suffix(".partial.png")
    scene.render.filepath = str(partial)
    bpy.ops.render.render(write_still=True)
    partial.replace(dest)


if __name__ == "__main__":
    job = Path(sys.argv[-1])
    for row in json.loads(job.read_text()):
        render(row, job.parent)
