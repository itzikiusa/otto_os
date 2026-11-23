"""Builds the gameready test inputs procedurally (nothing downloaded). Run inside Blender:
  Blender -b --factory-startup --python make_inputs.py -- OUT_DIR
Writes sculpt.glb (~500k tris, colour + metal/rough textures, fine surface detail), hollow.glb (open,
non-manifold, flipped faces, duplicate verts, small holes, a loose fragment), multipart.glb (5 parts,
3 materials, parented) and rigged.glb (skinned: must be refused).
"""
import math
import os
import sys

import bmesh
import bpy
import numpy as np

OUT = sys.argv[sys.argv.index("--") + 1]
os.makedirs(OUT, exist_ok=True)


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def image(name, rgb):
    """rgb: (h, w, 3) float array in 0..1 -> packed Blender image."""
    h, w, _ = rgb.shape
    img = bpy.data.images.new(name, w, h, alpha=False)
    px = np.ones((h, w, 4), np.float32)
    px[..., :3] = rgb
    img.pixels.foreach_set(px.ravel())
    img.pack()
    return img


def material(name, color=None, rough=0.5, metal=0.0, tex=None, mr=None):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nt = mat.node_tree
    b = nt.nodes["Principled BSDF"]
    b.inputs["Roughness"].default_value, b.inputs["Metallic"].default_value = rough, metal
    if color:
        b.inputs["Base Color"].default_value = (*color, 1)
    if tex:
        n = nt.nodes.new("ShaderNodeTexImage")
        n.image = tex
        nt.links.new(n.outputs["Color"], b.inputs["Base Color"])
    if mr:
        n = nt.nodes.new("ShaderNodeTexImage")
        n.image = mr
        n.image.colorspace_settings.name = "Non-Color"
        s = nt.nodes.new("ShaderNodeSeparateColor")
        nt.links.new(n.outputs["Color"], s.inputs["Color"])
        nt.links.new(s.outputs["Green"], b.inputs["Roughness"])
        nt.links.new(s.outputs["Blue"], b.inputs["Metallic"])
    return mat


def export(path):
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB")


def colour_texture(n=1024):
    v, u = np.mgrid[0:n, 0:n] / n
    hue = (u * 3 + 0.3 * np.sin(v * 12)) % 1.0
    rgb = np.stack([0.5 + 0.5 * np.cos(2 * np.pi * (hue + k / 3)) for k in range(3)], -1)
    checker = ((u * 16).astype(int) + (v * 16).astype(int)) % 2
    rgb = rgb * (0.55 + 0.45 * checker[..., None])
    spots = np.sin(u * 90) * np.sin(v * 70) > 0.85
    rgb[spots] = (0.05, 0.05, 0.05)
    return rgb


def sculpt():
    reset()
    bpy.ops.mesh.primitive_uv_sphere_add(segments=640, ring_count=400, radius=1.0)
    ob = bpy.context.object
    me = ob.data
    co = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", co)
    d = co.reshape(-1, 3)
    d /= np.linalg.norm(d, axis=1, keepdims=True)
    x, y, z = d.T
    r = 1 + 0.18 * np.sin(3 * x) * np.cos(2 * z)                          # broad form
    for c in ((0.6, 0.0, 0.8), (-0.6, 0.0, 0.8), (0, -1, 0.1)):              # horns / snout
        r += 0.45 * np.exp(-np.sum((d - np.array(c) / np.linalg.norm(c)) ** 2, 1) / 0.03)
    r += 0.012 * (np.sin(45 * x) + np.sin(52 * y) + np.sin(61 * z))       # wrinkles: normal-map detail
    r -= 0.02 * (np.abs(np.sin(14 * np.arctan2(y, x))) > 0.97)           # carved grooves
    me.vertices.foreach_set("co", (d * r[:, None]).ravel())
    me.update()
    me.shade_smooth()
    n = 1024
    v, u = np.mgrid[0:n, 0:n] / n
    mr = np.zeros((n, n, 3))
    mr[..., 1] = 0.2 + 0.7 * v                                             # roughness ramp
    mr[..., 2] = (np.sin(u * 40) > 0.6).astype(float)                      # metal stripes
    me.materials.append(material("skin", tex=image("sculpt_color", colour_texture(n)), mr=image("sculpt_mr", mr)))
    export(os.path.join(OUT, "sculpt.glb"))


def hollow():
    reset()
    seg, rings, h = 256, 100, 2.0
    verts, faces = [], []
    for j in range(rings + 1):  # seam column seg duplicates column 0: duplicate vertices
        for i in range(seg + 1):
            a = 2 * math.pi * i / seg
            verts.append((math.cos(a), math.sin(a), h * j / rings))
    idx = lambda i, j: j * (seg + 1) + i
    holes = {(10, 20), (11, 20), (10, 21), (11, 21), (100, 50), (101, 50), (100, 51), (101, 51), (200, 80)}
    for j in range(rings):
        for i in range(seg):
            if (i, j) in holes:
                continue
            f = [idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1)]
            faces.append(f[::-1] if (i // 8 + j // 8) % 2 else f)              # patches of flipped faces
    k = len(verts)  # fin: an inner strip sharing 10 tube edges -> edges with 3 faces
    for j in range(40, 51):
        verts.append((0.6, 0.0, h * j / rings))
    for t, j in enumerate(range(40, 50)):
        faces.append([idx(0, j), idx(0, j + 1), k + t + 1, k + t])
    k = len(verts)  # tiny loose fragment (0.2% of the size)
    verts += [(3, 3, 3), (3.005, 3, 3), (3, 3.005, 3)]
    faces.append([k, k + 1, k + 2])
    me = bpy.data.meshes.new("tube")
    me.from_pydata(verts, [], faces)
    uv = me.uv_layers.new()
    for p in me.polygons:
        for li in p.loop_indices:
            vx, vy, vz = verts[me.loops[li].vertex_index]
            uv.data[li].uv = ((math.atan2(vy, vx) / (2 * math.pi)) % 1.0, min(vz / h, 1.0))
    me.update()
    ob = bpy.data.objects.new("tube", me)
    bpy.context.scene.collection.objects.link(ob)
    bm = bmesh.new()  # subdivide to ~200k tris so it looks like a raw AI mesh
    bm.from_mesh(me)
    bmesh.ops.subdivide_edges(bm, edges=bm.edges, cuts=1, use_grid_fill=True)
    bmesh.ops.triangulate(bm, faces=bm.faces)
    bm.to_mesh(me)
    me.materials.append(material("tube", tex=image("tube_color", colour_texture(512)), rough=0.5, metal=0.0))
    export(os.path.join(OUT, "hollow.glb"))


def multipart():
    reset()
    red = material("red_paint", color=(0.8, 0.05, 0.05), rough=0.3, metal=0.8)
    black = material("rubber", color=(0.02, 0.02, 0.02), rough=0.9, metal=0.0)
    blue = material("glass", color=(0.05, 0.15, 0.8), rough=0.1, metal=0.0)
    bpy.ops.mesh.primitive_cube_add(size=1)
    body = bpy.context.object
    body.name = "body"
    body.scale = (2.0, 1.0, 0.5)
    bevel = body.modifiers.new("b", "BEVEL")
    bevel.width, bevel.segments = 0.08, 4
    sub = body.modifiers.new("s", "SUBSURF")
    sub.levels = sub.render_levels = 2
    bpy.ops.object.convert(target="MESH")
    body.data.materials.append(red)
    for i, (x, y) in enumerate(((0.65, 0.55), (-0.65, 0.55), (0.65, -0.55), (-0.65, -0.55))):
        bpy.ops.mesh.primitive_cylinder_add(vertices=96, radius=0.3, depth=0.2, location=(x, y, -0.3),
                                            rotation=(math.pi / 2, 0, 0))
        w = bpy.context.object
        w.name = f"wheel{i}"
        m = w.modifiers.new("s", "SUBSURF")
        m.levels = m.render_levels = 2
        bpy.ops.object.convert(target="MESH")
        w.data.materials.append(black)
        w.parent = body
        w.matrix_parent_inverse = body.matrix_world.inverted()
    bpy.ops.mesh.primitive_uv_sphere_add(segments=128, ring_count=64, radius=0.45, location=(0.1, 0, 0.3))
    dome = bpy.context.object
    dome.name = "dome"
    dome.data.materials.append(blue)
    for o in bpy.context.scene.objects:
        o.data.shade_smooth()
    export(os.path.join(OUT, "multipart.glb"))


def rigged():
    reset()
    bpy.ops.mesh.primitive_cube_add()
    cube = bpy.context.object
    bpy.ops.object.armature_add()
    arm = bpy.context.object
    cube.select_set(True)
    arm.select_set(True)
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.parent_set(type="ARMATURE_AUTO")
    export(os.path.join(OUT, "rigged.glb"))


sculpt()
hollow()
multipart()
rigged()
