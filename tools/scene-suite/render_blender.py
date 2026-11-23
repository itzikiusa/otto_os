"""Blender (headless): render scene3d documents with their own camera and
from the top, and measure how much of each piece the camera sees.

    Blender -b --factory-startup --python render_blender.py -- jobs.json
jobs.json: [{"doc": scene.json, "models": {object_id: glb}, "labels": {id: name},
             "out": dir, "textured": false}]
Writes <out>/camera.png, camera_labels.png, top.png and vis.json
({id: {"visible": px, "alone": px}} at 640x360 from the scene camera).
"""
import json
import math
import sys
import colorsys

import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
jobs = json.load(open(argv[0]))


def g2b(p):
    return Vector((p[0], -p[2], p[1]))


def look(cam, pos, target):
    cam.location = pos
    cam.rotation_euler = (Vector(target) - Vector(pos)).to_track_quat("-Z", "Y").to_euler()
    bpy.context.view_layer.update()


def render(sc, path, res, flat=False, transparent=False):
    sc.render.resolution_x, sc.render.resolution_y = res
    sc.render.engine = "BLENDER_WORKBENCH"
    sc.view_settings.view_transform = "Standard"
    sh = sc.display.shading
    sh.light = "FLAT" if flat else "STUDIO"
    sh.show_shadows = not flat
    sh.show_cavity = False
    sc.render.film_transparent = transparent
    sc.render.filepath = path
    bpy.ops.render.render(write_still=True)


def count_white(path):
    img = bpy.data.images.load(path)
    px = img.pixels[:]
    n = sum(1 for i in range(0, len(px), 4) if px[i + 3] > 0.5 and px[i] > 0.5)
    bpy.data.images.remove(img)
    return n


for job in jobs:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    doc = json.load(open(job["doc"]))
    out = job["out"]
    roots, meshes = {}, {}
    gltf = [o for o in doc["objects"] if o.get("type") == "gltf" and o["id"] in job["models"]]
    for i, o in enumerate(gltf):
        before = set(sc.objects)
        bpy.ops.import_scene.gltf(filepath=job["models"][o["id"]])
        new = [x for x in sc.objects if x not in before]
        e = bpy.data.objects.new("ROOT_" + o["id"], None)
        sc.collection.objects.link(e)
        e.location = g2b(o["position"])
        e.rotation_euler = (0, 0, math.radians(o["rotation"][1]))
        s = o["scale"]  # per axis (dims_m), glTF x, y, z
        e.scale = (s[0], s[2], s[1])
        for x in new:
            if x.type == "ARMATURE":
                x.data.pose_position = "REST"
            if x.parent is None:
                x.parent = e
        shapes = {pb.custom_shape for x in new if x.type == "ARMATURE" for pb in x.pose.bones}
        vis = []
        hue = (i * 0.618) % 1.0
        for x in new:
            if x in shapes or x.type != "MESH" or "_LOD" in x.name or x.name.startswith("UCX_"):
                x.hide_render = True
            else:
                vis.append(x)
                x.color = (*colorsys.hsv_to_rgb(hue, 0.55, 0.95), 1)
        meshes[o["id"]] = vis
    bpy.context.view_layer.update()
    pts = [x.matrix_world @ Vector(c) for v in meshes.values() for x in v for c in x.bound_box]
    if not pts:
        continue
    lo = [min(p[k] for p in pts) for k in range(3)]
    hi = [max(p[k] for p in pts) for k in range(3)]
    centre = Vector([(lo[k] + hi[k]) / 2 for k in range(3)])
    span = max(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2])
    world = bpy.data.worlds.new("w")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.75, 0.8, 0.86, 1)
    sc.world = world
    world.color = (0.75, 0.8, 0.86)
    g = next((o for o in doc["objects"] if o.get("id") == "ground"), None)
    ground = None
    if g:
        bpy.ops.mesh.primitive_plane_add(size=1, location=g2b(g["position"]))
        ground = bpy.context.active_object
        ground.scale = (g["scale"][0], g["scale"][1], 1)
        ground.color = (0.55, 0.5, 0.42, 1)
    cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
    sc.collection.objects.link(cam)
    sc.camera = cam
    c = doc["camera"]
    cam.data.sensor_fit = "VERTICAL"
    cam.data.angle = math.radians(c.get("fov", 50))
    cam.data.clip_end = 10000
    look(cam, g2b(c["position"]), g2b(c["target"]))
    sc.display.shading.color_type = "TEXTURE" if job.get("textured") else "OBJECT"
    render(sc, out + "/camera.png", (1280, 720))
    # Labels facing the camera.
    texts = []
    for o in gltf:
        vs = [x.matrix_world @ Vector(cc) for x in meshes[o["id"]] for cc in x.bound_box]
        if not vs:
            continue
        top = Vector((sum(v.x for v in vs) / len(vs), sum(v.y for v in vs) / len(vs), max(v.z for v in vs)))
        cu = bpy.data.curves.new("t_" + o["id"], "FONT")
        cu.body = job["labels"].get(o["id"], o["id"])
        cu.size = max(0.05, 0.022 * (Vector(g2b(c["position"])) - top).length)
        cu.align_x = "CENTER"
        t = bpy.data.objects.new("t_" + o["id"], cu)
        sc.collection.objects.link(t)
        t.location = top + Vector((0, 0, 0.02 * span))
        t.rotation_euler = cam.rotation_euler
        t.color = (0.05, 0.05, 0.05, 1)
        texts.append(t)
    sc.display.shading.color_type = "OBJECT"
    render(sc, out + "/camera_labels.png", (1280, 720))
    # From the top: ortho over the group and the ground.
    side = max(span, g["scale"][0] if g else 0) * 1.1
    cam.data.type = "ORTHO"
    cam.data.ortho_scale = side
    cam.location = (centre.x, centre.y, hi[2] + 50)
    cam.rotation_euler = (0, 0, 0)
    for t in texts:
        t.rotation_euler = (0, 0, 0)
        t.location.z = hi[2] + 1
    bpy.context.view_layer.update()
    render(sc, out + "/top.png", (900, 900))
    for t in texts:
        t.hide_render = True
    # Visibility from the scene camera: each piece white, the rest black (or hidden).
    cam.data.type = "PERSP"
    look(cam, g2b(c["position"]), g2b(c["target"]))
    if ground:
        ground.hide_render = True
    vis = {}
    for oid, mine in meshes.items():
        for other, xs in meshes.items():
            for x in xs:
                x.color = (1, 1, 1, 1) if other == oid else (0, 0, 0, 1)
                x.hide_render = False
        render(sc, out + "/_vis.png", (640, 360), flat=True, transparent=True)
        seen = count_white(out + "/_vis.png")
        for other, xs in meshes.items():
            for x in xs:
                x.hide_render = other != oid
        render(sc, out + "/_vis.png", (640, 360), flat=True, transparent=True)
        vis[oid] = {"visible": seen, "alone": count_white(out + "/_vis.png")}
    json.dump(vis, open(out + "/vis.json", "w"), indent=1)
    import os
    os.remove(out + "/_vis.png")
    print("RENDERED", out)
