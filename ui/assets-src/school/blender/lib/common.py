"""Shared Blender helpers for the Otto School asset build.

Coordinates: every builder thinks in **glTF space** (metres, Y up, fronts face
+Z) and converts with `G()` / `gsize()` when it touches Blender, whose space is
Z up with glTF +Z mapped to Blender -Y (the glTF exporter's `export_yup`
conversion).  Keeping the authoring in glTF terms makes the contract's numbers
(`Workstation` at z = +0.55, seat top at y = 0.42, …) appear verbatim in code.
"""

import math
import os

import bmesh
import bpy
from mathutils import Matrix, Vector

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.dirname(HERE)  # ui/assets-src/school
TEX = os.path.join(SRC, "textures")
BASE = os.path.join(SRC, "base")


def G(x, y, z):
    """glTF point → Blender point."""
    return Vector((x, -z, y))


def grot(rx=0, ry=0, rz=0):
    """glTF Euler rotation (degrees, applied X then Y then Z) as a Blender
    4×4 matrix (glTF X→Blender X, Y→Z, Z→−Y)."""
    return (Matrix.Rotation(math.radians(rz), 4, Vector((0, -1, 0))) @
            Matrix.Rotation(math.radians(ry), 4, "Z") @
            Matrix.Rotation(math.radians(rx), 4, "X"))


def gsize(w, h, d):
    """glTF extent (w along X, h along Y, d along Z) → Blender extent."""
    return Vector((w, d, h))


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    sc = bpy.context.scene
    sc.render.fps = 30
    sc.unit_settings.system = "METRIC"


# ---------------------------------------------------------------- materials

def srgb(c):
    """sRGB component → linear (Blender colour sockets and glTF factors are linear)."""
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def lin3(c):
    return tuple(srgb(x) for x in c[:3])


_IMG_CACHE = {}


def image(path, colorspace="sRGB"):
    key = (path, colorspace)
    if key in _IMG_CACHE and _IMG_CACHE[key].name in bpy.data.images:
        return _IMG_CACHE[key]
    img = bpy.data.images.load(path, check_existing=True)
    img.colorspace_settings.name = colorspace
    _IMG_CACHE[key] = img
    return img


def mat(name, color=(0.8, 0.8, 0.8), rough=0.6, metal=0.0, tex=None, emit=None,
        emit_tex=None, emit_strength=1.0, alpha=None, tint=None):
    """Principled material the glTF exporter maps 1:1 (baseColor/roughness/
    metallic/emissive). Colours are given in sRGB. `tex` is an image path for
    baseColor; `tint` multiplies it (exported as baseColorFactor)."""
    m = bpy.data.materials.get(name)
    if m:
        return m
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    out = nt.nodes["Material Output"]
    rgba = lin3(color) + (1.0,)
    bsdf.inputs["Base Color"].default_value = rgba
    bsdf.inputs["Roughness"].default_value = rough
    bsdf.inputs["Metallic"].default_value = metal
    if tex:
        ti = nt.nodes.new("ShaderNodeTexImage")
        ti.image = image(tex)
        ti.location = (-500, 300)
        if tint:
            mix = nt.nodes.new("ShaderNodeMix")
            mix.data_type = "RGBA"
            mix.blend_type = "MULTIPLY"
            mix.inputs["Factor"].default_value = 1.0
            nt.links.new(ti.outputs["Color"], mix.inputs[6])
            mix.inputs[7].default_value = lin3(tint) + (1.0,)
            nt.links.new(mix.outputs[2], bsdf.inputs["Base Color"])
        else:
            nt.links.new(ti.outputs["Color"], bsdf.inputs["Base Color"])
    if emit_tex:
        te = nt.nodes.new("ShaderNodeTexImage")
        te.image = image(emit_tex)
        te.location = (-500, -200)
        nt.links.new(te.outputs["Color"], bsdf.inputs["Emission Color"])
        bsdf.inputs["Emission Strength"].default_value = emit_strength
    elif emit:
        bsdf.inputs["Emission Color"].default_value = lin3(emit) + (1.0,)
        bsdf.inputs["Emission Strength"].default_value = emit_strength
    if alpha is not None:
        bsdf.inputs["Alpha"].default_value = alpha
        m.surface_render_method = "BLENDED"
    m.diffuse_color = rgba
    _ = out
    return m


# ---------------------------------------------------------------- geometry

def _new_obj(name, bm, materials=()):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    for m in materials:
        me.materials.append(m)
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    return ob


def _bevel_all(bm, width, segments):
    if width <= 0:
        return
    bmesh.ops.bevel(bm, geom=list(bm.edges) + list(bm.verts), offset=width,
                    segments=segments, profile=0.5, affect="EDGES",
                    clamp_overlap=True)


def box(name, size, center, mat_=None, bevel=0.0, seg=2, uv_scale=None,
        rot=(0, 0, 0), uv_origin=None):
    """Axis-aligned bevelled box. `size`/`center` are glTF (w, h, d)/(x, y, z).
    `rot` = glTF Euler degrees (X, Y, Z) about the box centre.
    `uv_scale` → cube-project UVs in metres (u per metre) for tiling textures."""
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    s = gsize(*size)
    for v in bm.verts:
        v.co = Vector((v.co.x * s.x, v.co.y * s.y, v.co.z * s.z))
    _bevel_all(bm, min(bevel, min(s) * 0.49), seg)
    if uv_scale is None and mat_ is not None and mat_.use_nodes and \
            any(n.type == "TEX_IMAGE" for n in mat_.node_tree.nodes):
        uv_scale, uv_origin = 1.0, center  # textured (wood): 1 repeat per metre
    if uv_scale and uv_origin is not None:
        o = G(*uv_origin) - G(*center)
        box_uv(bm, uv_scale, o)
        uv_scale = None
    if any(rot):
        bmesh.ops.transform(bm, matrix=grot(*rot), verts=bm.verts)
    c = G(*center)
    for v in bm.verts:
        v.co += c
    if uv_scale:
        box_uv(bm, uv_scale)
    ob = _new_obj(name, bm, [mat_] if mat_ else [])
    for p in ob.data.polygons:
        p.use_smooth = bool(bevel > 0)
    return ob


def box_uv(bm, per_m, origin=Vector((0, 0, 0))):
    """Cube projection in metres (Blender axes), good for walls/floors."""
    uv = bm.loops.layers.uv.verify()
    for f in bm.faces:
        n = f.normal
        ax = max(range(3), key=lambda i: abs(n[i]))
        for l in f.loops:
            co = l.vert.co - origin
            if ax == 0:
                u, v = (co.y if n.x < 0 else -co.y), co.z
            elif ax == 1:
                u, v = (co.x if n.y < 0 else -co.x), co.z
            else:
                u, v = co.x, co.y
            l[uv].uv = (u * per_m, v * per_m)


def cyl(name, radius, height, center, mat_=None, axis="y", verts=16, bevel=0.0,
        seg=2, r2=None):
    """Cylinder (or cone when r2 given) along a glTF axis, centred at `center`."""
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=verts,
                          radius1=radius, radius2=radius if r2 is None else r2,
                          depth=height)
    if bevel > 0:
        rim = [e for e in bm.edges if len(e.link_faces) == 2 and
               abs(e.link_faces[0].normal.dot(e.link_faces[1].normal)) < 0.5]
        bmesh.ops.bevel(bm, geom=rim, offset=bevel, segments=seg, profile=0.5,
                        affect="EDGES", clamp_overlap=True)
    # Blender cone is along Blender Z (= glTF Y).
    rot = {"y": Matrix(), "x": Matrix.Rotation(math.pi / 2, 4, "Y"),
           "z": Matrix.Rotation(math.pi / 2, 4, "X")}[axis]
    bmesh.ops.transform(bm, matrix=rot, verts=bm.verts)
    c = G(*center)
    for v in bm.verts:
        v.co += c
    ob = _new_obj(name, bm, [mat_] if mat_ else [])
    for p in ob.data.polygons:
        p.use_smooth = True
    _flat_caps(ob)
    return ob


def _flat_caps(ob):
    me = ob.data
    for p in me.polygons:
        if len(p.vertices) > 4:
            p.use_smooth = False


def sphere(name, radius, center, mat_=None, seg=12, rings=8, scale=(1, 1, 1)):
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=seg, v_segments=rings, radius=radius)
    s = gsize(*scale)
    c = G(*center)
    for v in bm.verts:
        v.co = Vector((v.co.x * s.x, v.co.y * s.y, v.co.z * s.z)) + c
    ob = _new_obj(name, bm, [mat_] if mat_ else [])
    for p in ob.data.polygons:
        p.use_smooth = True
    # Flat-shaded on purpose: with smooth shading Blender's normals at the
    # poles differ in the last bits between runs, which changes the
    # exporter's vertex welding and so the .glb bytes (non-reproducible).
    for p in ob.data.polygons:
        p.use_smooth = False
    return ob


def plane(name, w, h, center, mat_=None, facing="+z", uv_flip_u=False,
          uv_rect=(0, 0, 1, 1), painted=False):
    """A w×h quad facing a glTF direction. UV (0,0) is bottom-left **as seen
    by a viewer looking at the front**, in Blender UV space — or, with
    `painted`, in exported glTF TEXCOORD_0 space (the exporter stores
    v' = 1 − v, so the Blender UV is flipped here to land (0,0) bottom-left in
    the .glb, as the contract specifies for runtime-painted planes)."""
    bm = bmesh.new()
    hw, hh = w / 2, h / 2
    # corners in a local frame: right, up; built facing +z then rotated.
    local = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
    u0, v0, u1, v1 = uv_rect
    uvs = [(u0, v0), (u1, v0), (u1, v1), (u0, v1)]
    if facing == "+z":
        pts = [(x, y, 0) for x, y in local]          # viewer right = +x
    elif facing == "-z":
        pts = [(-x, y, 0) for x, y in local]         # viewer right = -x
    elif facing == "+y":
        pts = [(x, 0, -y) for x, y in local]         # seen from above, top = -z
    elif facing == "-y":
        pts = [(x, 0, y) for x, y in local]
    else:
        raise ValueError(facing)
    c = center
    vs = [bm.verts.new(G(c[0] + p[0], c[1] + p[1], c[2] + p[2])) for p in pts]
    f = bm.faces.new(vs)
    uv = bm.loops.layers.uv.verify()
    for l, t in zip(f.loops, uvs):
        u, v = (1 - t[0], t[1]) if uv_flip_u else t
        l[uv].uv = (u, 1 - v) if painted else (u, v)
    bm.normal_update()
    ob = _new_obj(name, bm, [mat_] if mat_ else [])
    return ob


def join(name, objs):
    """Join meshes into the first one (materials preserved) and rename."""
    objs = [o for o in objs if o is not None]
    # Join keeps only the active object's UV layers: give every part one
    # named "UVMap" first or textured parts lose their UVs.
    for o in objs:
        if o.type == "MESH":
            if not o.data.uv_layers:
                o.data.uv_layers.new(name="UVMap")
            else:
                o.data.uv_layers[0].name = "UVMap"
    if len(objs) == 1:
        objs[0].name = name
        objs[0].data.name = name
        return objs[0]
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    ob = bpy.context.view_layer.objects.active
    ob.name = name
    ob.data.name = name
    return ob


def set_origin(ob, gpoint):
    """Move the object origin to a glTF point without moving geometry."""
    p = G(*gpoint)
    ob.data.transform(Matrix.Translation(-p + ob.location))
    ob.location = p


def parent(child, par):
    bpy.context.view_layer.update()  # matrix_world is stale right after edits
    mw = child.matrix_world.copy()
    child.parent = par
    child.matrix_parent_inverse = par.matrix_world.inverted()
    child.matrix_world = mw


def tris(ob):
    me = ob.evaluated_get(bpy.context.evaluated_depsgraph_get()).to_mesh()
    n = sum(len(p.vertices) - 2 for p in me.polygons)
    ob.evaluated_get(bpy.context.evaluated_depsgraph_get()).to_mesh_clear()
    return n


def triangulate(ob):
    """Deterministic triangulation (the exporter's own n-gon split can vary
    between runs for flat caps, which breaks byte-identical rebuilds)."""
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bmesh.ops.triangulate(bm, faces=bm.faces[:], quad_method="FIXED", ngon_method="EAR_CLIP")
    bm.to_mesh(ob.data)
    bm.free()


def export_glb(path, objects=None, animations=False, jpeg=False):
    for o in (objects if objects is not None else bpy.data.objects):
        if o.type == "MESH" and o.data.users == 1:
            triangulate(o)
    bpy.ops.object.select_all(action="DESELECT")
    if objects is not None:
        for o in objects:
            o.select_set(True)
    kw = dict(
        filepath=path,
        export_format="GLB",
        use_selection=objects is not None,
        export_apply=True,
        export_yup=True,
        export_texcoords=True,
        export_normals=True,
        export_materials="EXPORT",
        export_image_format="JPEG" if jpeg else "AUTO",
        export_animations=animations,
        export_cameras=False,
        export_lights=False,
        export_extras=False,
    )
    if jpeg:
        kw["export_jpeg_quality"] = 86
    if animations:
        kw.update(export_animation_mode="ACTIONS", export_force_sampling=True,
                  export_frame_step=1, export_optimize_animation_size=True,
                  export_anim_single_armature=True, export_reset_pose_bones=True)
    bpy.ops.export_scene.gltf(**kw)
    from .glbinfo import canonicalize_indices
    canonicalize_indices(path)
