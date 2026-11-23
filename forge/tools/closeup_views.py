"""Shared close-up cameras for Otto Forge evidence renders (one file, so every worker renders identical views).

    Blender -b --factory-startup --python tools/closeup_views.py -- MODEL.glb OUTDIR
            [--views VIEWS.json] [--modes lit,base,clay,shade,normals] [--turnaround] [--write-views VIEWS.json]

VIEWS.json maps a view name to [centre_xyz, ortho_width] or [centre_xyz, ortho_width, direction_xyz] in Blender
world coordinates (Z up; direction points from the centre to the camera), the format of
out/text_to_asset/character/crops.json. Pass the SAME file for every variant of an asset so the cameras are
identical. Without --views the views come from the model's bounding box (upright humanoid guess: face, both
hands, both feet), and --write-views saves them for the next variant. 'three_quarter' is always added.

Modes (one PNG per view in OUTDIR/<mode>/):
  lit      Cycles, three soft area lights, AgX: the look of the existing close-ups.
  base     unlit base colour only (each material's Base Color routed to emission): paint.
  clay     one grey material, no maps, single grazing sun from the camera's side: geometry and smoothing
           (transparent film: alpha is the silhouette).
  shade    grey base colour but each material's own normal map, lit like 'lit': normal-map shading.
  normals  true face normals as colour, back faces red: flipped faces and openings into the inside.
OUTDIR/counts.json: per mode and view, near-black pixels (all channels <= 25/255; the backdrop is grey) and,
for 'normals', back-facing (red) pixels.
--turnaround also writes OUTDIR/turnaround.png (front, left, back, right; model's own sides) in 'lit'.
"""
import json
import math
import os
import sys

import bpy
import numpy as np
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
MODEL, OUT = argv[0], argv[1]
opt = lambda k, d=None: argv[argv.index(k) + 1] if k in argv else d
MODES = opt("--modes", "lit,base,clay,shade,normals").split(",")
RES = 640
DARK = 25 / 255


def load():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=os.path.abspath(MODEL))
    scene = bpy.context.scene
    rig = next((o for o in scene.objects if o.type == "ARMATURE"), None)
    if rig:
        rig.data.pose_position = "REST"
    shapes = {pb.custom_shape for o in scene.objects if o.type == "ARMATURE" for pb in o.pose.bones}
    meshes = [o for o in scene.objects if o.type == "MESH" and o not in shapes and not o.name.startswith("UCX_")
              and "_LOD" not in o.name]
    for o in scene.objects:
        o.hide_render = o.type == "MESH" and o not in meshes
    bpy.context.view_layer.update()
    dg = bpy.context.evaluated_depsgraph_get()
    pts = []
    for o in meshes:
        me = o.evaluated_get(dg).data
        c = np.empty(len(me.vertices) * 3, np.float32)
        me.vertices.foreach_get("co", c)
        m = np.asarray(o.matrix_world)
        pts.append(c.reshape(-1, 3) @ m[:3, :3].T + m[:3, 3])
    return scene, meshes, np.vstack(pts)


def auto_views(pts):
    """Upright humanoid guess from the vertex cloud: head = top 13% of the height, hands = the outermost 8% in
    X at either side, feet = the lowest 12% on either side of the centre line."""
    lo, hi = pts.min(0), pts.max(0)
    h, cx = float(hi[2] - lo[2]), (lo[0] + hi[0]) / 2
    views = {}
    head = pts[pts[:, 2] > hi[2] - .13 * h]
    views["face"] = [np.percentile(head, 50, 0).tolist(), .23 * h]
    w = float(hi[0] - lo[0])
    for name, sel in (("left_hand", pts[:, 0] > hi[0] - .08 * w), ("right_hand", pts[:, 0] < lo[0] + .08 * w)):
        views[name] = [np.median(pts[sel], 0).tolist(), .18 * h]
    low = pts[:, 2] < lo[2] + .12 * h
    for name, sel in (("left_foot", low & (pts[:, 0] > cx)), ("right_foot", low & (pts[:, 0] <= cx))):
        views[name] = [np.median(pts[sel], 0).tolist(), .25 * h]
    return views


def aim(ob, at):
    ob.rotation_euler = (Vector(at) - ob.location).to_track_quat("-Z", "Y").to_euler()


def camera_dir(name):  # the directions of out/text_to_asset/render_details.py
    if name == "three_quarter":
        return Vector((.48, -1, .22))
    side = .18 if name.startswith("left") else -.18
    return Vector((side, -1, .25 if ("foot" in name or "hand" in name) else .03))


def set_world(scene, rgb, strength):
    w = scene.world = bpy.data.worlds.new("w")
    w.use_nodes = True
    bg = w.node_tree.nodes["Background"]
    bg.inputs[0].default_value = (*rgb, 1)
    bg.inputs[1].default_value = strength


def area_lights(scene, centre, height):
    for d, power in [((-1.3, -2.2, 2.4), 88), ((1.6, -1, .9), 46), ((.7, 1.7, 1.7), 64)]:
        lamp = bpy.data.lights.new("softbox", "AREA")
        ob = bpy.data.objects.new("softbox", lamp)
        scene.collection.objects.link(ob)
        ob.location = Vector(centre) + Vector(d) * height
        lamp.energy, lamp.shape, lamp.size = power * height ** 2, "DISK", 1.7 * height
        aim(ob, centre)


def emission_material(nt, colour_socket=None, colour=None):
    out = next(n for n in nt.nodes if n.type == "OUTPUT_MATERIAL" and n.is_active_output)
    em = nt.nodes.new("ShaderNodeEmission")
    if colour_socket is not None:
        nt.links.new(colour_socket, em.inputs["Color"])
    elif colour is not None:
        em.inputs["Color"].default_value = colour
    nt.links.new(em.outputs["Emission"], out.inputs["Surface"])


def prepare(scene, mode, meshes, centre, height):
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.render.threads_mode, scene.render.threads = "FIXED", 6
    scene.render.film_transparent = mode == "clay"  # clay's alpha is the exact silhouette for overlays
    scene.cycles.samples, scene.cycles.use_denoising = (12, True) if mode in ("lit", "shade") else (4, False)
    scene.view_settings.view_transform = "AgX" if mode in ("lit", "shade") else "Standard"
    mats = {m for o in meshes for m in o.data.materials if m and m.use_nodes}
    if mode in ("lit", "shade"):
        set_world(scene, (.43, .46, .50), .65)
        area_lights(scene, centre, height)
    if mode == "shade":
        for m in mats:
            b = next((n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
            if b:
                for name, v in (("Base Color", (.6, .6, .6, 1)), ("Metallic", 0.0), ("Roughness", .5)):
                    for link in list(b.inputs[name].links):
                        m.node_tree.links.remove(link)
                    b.inputs[name].default_value = v
    if mode == "base":
        set_world(scene, (.5, .5, .52), 1.0)
        for m in mats:
            b = next((n for n in m.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
            sock = b.inputs["Base Color"] if b else None
            if sock is not None and sock.is_linked:
                emission_material(m.node_tree, colour_socket=sock.links[0].from_socket)
            else:
                emission_material(m.node_tree, colour=tuple(sock.default_value) if sock else (.8, .8, .8, 1))
    if mode == "clay":
        set_world(scene, (.5, .5, .52), .12)
        clay = bpy.data.materials.new("clay")
        clay.use_nodes = True
        b = next(n for n in clay.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
        b.inputs["Base Color"].default_value, b.inputs["Roughness"].default_value = (.62, .62, .62, 1), .55
        bpy.context.view_layer.material_override = clay
        sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
        sun.data.energy = 4.0
        scene.collection.objects.link(sun)
        return sun
    if mode == "normals":
        set_world(scene, (.5, .5, .52), 1.0)
        mat = bpy.data.materials.new("normals")
        mat.use_nodes = True
        nt = mat.node_tree
        geo = nt.nodes.new("ShaderNodeNewGeometry")
        mul = nt.nodes.new("ShaderNodeVectorMath")
        mul.operation = "MULTIPLY_ADD"
        mul.inputs[1].default_value, mul.inputs[2].default_value = (.5, .5, .5), (.5, .5, .5)
        nt.links.new(geo.outputs["True Normal"], mul.inputs[0])
        mix = nt.nodes.new("ShaderNodeMix")
        mix.data_type = "RGBA"
        nt.links.new(geo.outputs["Backfacing"], mix.inputs["Factor"])
        nt.links.new(mul.outputs["Vector"], mix.inputs["A"])
        mix.inputs["B"].default_value = (1, 0, 0, 1)
        emission_material(nt, colour_socket=mix.outputs["Result"])
        bpy.context.view_layer.material_override = mat
    return None


def count(path, mode):
    img = bpy.data.images.load(path)
    px = np.empty(len(img.pixels), np.float32)
    img.pixels.foreach_get(px)
    bpy.data.images.remove(img)
    px = px.reshape(-1, 4)
    px = px[px[:, 3] > .5, :3]
    res = {"dark": int((px.max(1) <= DARK).sum())}
    if mode == "normals":
        res["backfacing"] = int(((px[:, 0] > .95) & (px[:, 1] < .05) & (px[:, 2] < .05)).sum())
    return res


def main():
    scene, meshes, pts = load()
    lo, hi = pts.min(0), pts.max(0)
    centre, height = (lo + hi) / 2, float(hi[2] - lo[2])
    views = json.load(open(opt("--views"))) if opt("--views") else auto_views(pts)
    views.setdefault("three_quarter", [centre.tolist(), float(max(hi - lo)) * 1.23])
    if opt("--write-views"):
        json.dump(views, open(opt("--write-views"), "w"), indent=1)
    os.makedirs(OUT, exist_ok=True)
    counts = {}
    for mode in MODES:
        scene, meshes, _ = load()
        sun = prepare(scene, mode, meshes, centre, height)
        cam = bpy.data.objects.new("camera", bpy.data.cameras.new("cam"))
        scene.collection.objects.link(cam)
        scene.camera, cam.data.type, cam.data.clip_start = cam, "ORTHO", height * .001
        scene.render.resolution_x = scene.render.resolution_y = RES
        os.makedirs(os.path.join(OUT, mode), exist_ok=True)
        counts[mode] = {}
        for name, (mid, width, *direction) in views.items():
            d = (Vector(direction[0]) if direction else camera_dir(name)).normalized()
            cam.data.ortho_scale = width
            cam.location = Vector(mid) + d * height * 3
            aim(cam, mid)
            if sun:  # grazing: mostly from the camera's right, a little from above and in front
                right = d.cross(Vector((0, 0, 1))).normalized()
                to_light = (right * .92 + d * .25 + Vector((0, 0, .3))).normalized()
                sun.rotation_euler = (-to_light).to_track_quat("-Z", "Y").to_euler()
            path = os.path.join(os.path.abspath(OUT), mode, name + ".png")
            scene.render.filepath = path
            bpy.ops.render.render(write_still=True)
            counts[mode][name] = count(path, mode)
        if mode == "lit" and "--turnaround" in argv:
            turnaround(scene, cam, centre, lo, hi)
    json.dump(counts, open(os.path.join(OUT, "counts.json"), "w"), indent=1)
    print("COUNTS", json.dumps(counts))


def turnaround(scene, cam, centre, lo, hi):
    size = hi - lo
    scene.render.resolution_x, scene.render.resolution_y = 512, 768
    cam.data.ortho_scale = max(size[2], max(size[0], size[1]) * 1.5) * 1.1
    tiles, dist = [], float(max(size)) * 3
    for k, angle in enumerate((0, 90, 180, 270)):  # front, model's left, back, model's right
        a = math.radians(angle)
        cam.location = Vector(centre) + Vector((dist * math.sin(a), -dist * math.cos(a), 0))
        cam.rotation_euler = (math.radians(90), 0, a)
        path = os.path.join(os.path.abspath(OUT), f"turnaround_{k}.png")
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        img = bpy.data.images.load(path)
        px = np.empty(len(img.pixels), np.float32)
        img.pixels.foreach_get(px)
        tiles.append(px.reshape(img.size[1], img.size[0], 4))
        bpy.data.images.remove(img)
        os.remove(path)
    strip = np.concatenate(tiles, axis=1)
    out = bpy.data.images.new("strip", strip.shape[1], strip.shape[0], alpha=True)
    out.pixels.foreach_set(strip.ravel())
    out.filepath_raw, out.file_format = os.path.join(os.path.abspath(OUT), "turnaround.png"), "PNG"
    out.save()


main()
