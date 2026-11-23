"""Small headless renders for evidence and contact sheets (Workbench: fast, honest silhouettes)."""

import bpy
import numpy as np
from mathutils import Vector


def setup(res=420, background=(0.08, 0.09, 0.11)):
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    shading = scene.display.shading
    shading.light = "STUDIO"
    shading.color_type = "MATERIAL"
    shading.show_cavity = True
    shading.cavity_type = "BOTH"
    shading.show_object_outline = True
    shading.background_type = "VIEWPORT"
    shading.background_color = background
    scene.render.resolution_x = scene.render.resolution_y = res
    scene.render.film_transparent = False
    scene.render.image_settings.file_format = "PNG"
    scene.world = scene.world or bpy.data.worlds.new("W")
    for o in scene.objects:
        if o.type == "EMPTY":
            o.hide_render = True
    cam = bpy.data.objects.get("FitCam")
    if cam is None:
        cam = bpy.data.objects.new("FitCam", bpy.data.cameras.new("FitCam"))
        scene.collection.objects.link(cam)
    scene.camera = cam
    label = bpy.data.objects.get("FitLabel")
    if label is None:
        curve = bpy.data.curves.new("FitLabel", "FONT")
        label = bpy.data.objects.new("FitLabel", curve)
        scene.collection.objects.link(label)
        mat = bpy.data.materials.new("FitLabelInk")
        mat.diffuse_color = (1, 0.85, 0.2, 1)
        curve.materials.append(mat)
    label.parent = cam
    return cam, label


def view(cam, label, centre, direction, span, text="", clip=False):
    centre, direction = Vector(centre), Vector(direction).normalized()
    cam.data.type = "ORTHO"
    cam.data.ortho_scale = span
    # clip=True cuts away whatever lies between the camera and the subject (e.g. a thigh in front of a palm)
    cam.data.clip_start, cam.data.clip_end = (span * 3.4 if clip else span * 0.01), span * 50
    cam.location = centre + direction * span * 4
    cam.rotation_euler = (-direction).to_track_quat("-Z", "Y" if abs(direction.z) < 0.99 else "Z").to_euler()
    label.data.body = text
    label.data.size = span * 0.06
    label.location = (-span * 0.48, span * 0.42, -span * 3.45 if clip else -span * 2)
    label.rotation_euler = (0, 0, 0)
    bpy.context.view_layer.update()


def shot(path):
    bpy.context.scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)
    return str(path)


def sheet(paths, cols, out):
    """Tile equally sized PNGs into one image with numpy (no PIL in Blender's Python)."""
    imgs = []
    for p in paths:
        im = bpy.data.images.load(p, check_existing=False)
        w, h = im.size
        a = np.empty(w * h * 4, dtype=np.float32)
        im.pixels.foreach_get(a)
        imgs.append(a.reshape(h, w, 4))
        bpy.data.images.remove(im)
    h, w = imgs[0].shape[:2]
    rows = (len(imgs) + cols - 1) // cols
    canvas = np.zeros((rows * h, cols * w, 4), dtype=np.float32)
    canvas[..., 3] = 1
    for i, a in enumerate(imgs):
        r, c = divmod(i, cols)
        y0 = (rows - 1 - r) * h  # Blender images start at the bottom row
        canvas[y0:y0 + h, c * w:(c + 1) * w] = a
    im = bpy.data.images.new("sheet", cols * w, rows * h, alpha=True)
    im.pixels.foreach_set(canvas.ravel())
    im.filepath_raw = str(out)
    im.file_format = "PNG"
    im.save()
    bpy.data.images.remove(im)
    return str(out)
