"""Preview renders of the BUILT assets (re-imported from ui/public/school/*.glb,
so what you see is what the runtime loads).

    Blender -b -P ui/assets-src/school/blender/render_previews.py -- [classroom,corridor,lineup,seat]

Writes ui/assets-src/school/previews/<name>.jpg (1600×900, EEVEE).
"""

import math
import os
import random
import sys

import bpy
import numpy as np
from mathutils import Vector

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from lib.common import G  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)
PUB = os.path.join(os.path.dirname(os.path.dirname(SRC)), "public", "school")
OUT = os.path.join(SRC, "previews")

KIT = {}
_TMP = {}


# ---------------------------------------------------------------- scene setup

def reset(res=(1600, 900)):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    KIT.clear()
    _TMP.clear()
    sc = bpy.context.scene
    sc.render.fps = 30
    sc.render.engine = "BLENDER_EEVEE"
    sc.render.resolution_x, sc.render.resolution_y = res
    sc.render.resolution_percentage = 100
    sc.render.film_transparent = False
    ee = sc.eevee
    ee.taa_render_samples = 64
    if hasattr(ee, "shadow_pool_size"):
        ee.shadow_pool_size = "1024"
    for attr, val in (("use_raytracing", True), ("use_shadows", True), ("shadow_ray_count", 2),
                      ("shadow_step_count", 8), ("use_gtao", True), ("fast_gi_method", "GLOBAL_ILLUMINATION")):
        if hasattr(ee, attr):
            try:
                setattr(ee, attr, val)
            except Exception:  # noqa: BLE001
                pass
    if hasattr(ee, "ray_tracing_options"):
        ee.ray_tracing_options.resolution_scale = "2"
    sc.view_settings.view_transform = "AgX"
    try:
        sc.view_settings.look = "AgX - Punchy"
    except TypeError:
        pass
    sc.view_settings.exposure = 0.1
    # Same environment the runtime loads (ui/public/school/env.hdr).
    w = bpy.data.worlds.new("World")
    sc.world = w
    w.use_nodes = True
    nt = w.node_tree
    bg = nt.nodes["Background"]
    env = nt.nodes.new("ShaderNodeTexEnvironment")
    env.image = bpy.data.images.load(os.path.join(PUB, "env.hdr"))
    nt.links.new(env.outputs["Color"], bg.inputs["Color"])
    bg.inputs["Strength"].default_value = 0.9
    # camera rays see a plain backdrop; everything else is lit by the HDRI
    plain = nt.nodes.new("ShaderNodeBackground")
    plain.inputs["Color"].default_value = (0.62, 0.70, 0.80, 1)
    lp = nt.nodes.new("ShaderNodeLightPath")
    mix = nt.nodes.new("ShaderNodeMixShader")
    nt.links.new(lp.outputs["Is Camera Ray"], mix.inputs[0])
    nt.links.new(bg.outputs[0], mix.inputs[1])
    nt.links.new(plain.outputs[0], mix.inputs[2])
    nt.links.new(mix.outputs[0], nt.nodes["World Output"].inputs["Surface"])
    return sc


def load_kit():
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=os.path.join(PUB, "school-kit.glb"))
    for o in bpy.data.objects:
        if o not in before and o.parent is None:
            KIT[o.name] = o
            o.hide_render = True
            o.hide_set(True)
            for c in o.children_recursive:
                c.hide_render = True
                c.hide_set(True)


def inst(name, pos, rot_y=0.0, mats=None):
    """Clone a kit node (with children) at a glTF position, rotated about Y."""
    src = KIT[name]

    def dup(o, parent):
        n = o.copy()
        bpy.context.scene.collection.objects.link(n)
        n.hide_render = False
        n.hide_set(False)
        if parent:
            n.parent = parent
        if mats and o.name.split(".")[0] in mats:
            n.data = o.data.copy()
            n.material_slots[0].link = "OBJECT"
            n.material_slots[0].material = mats[o.name.split(".")[0]]
        for c in o.children:
            dup(c, n)
        return n
    root = dup(src, None)
    root.location = G(*pos)
    root.rotation_mode = "XYZ"
    root.rotation_euler = (0, 0, math.radians(rot_y))
    return root


def _gl_material(doc, k, tag):
    """glTF material → Blender material (baseColor factor/texture, emissive, mask)."""
    key = (tag, k)
    if key in _TMP:
        return _TMP[key]
    g = doc.j["materials"][k]
    m = bpy.data.materials.new(g.get("name", "m"))
    m.use_nodes = True
    nt = m.node_tree
    b = nt.nodes["Principled BSDF"]
    pbr = g.get("pbrMetallicRoughness", {})
    b.inputs["Base Color"].default_value = tuple(pbr.get("baseColorFactor", [1, 1, 1, 1]))
    b.inputs["Roughness"].default_value = pbr.get("roughnessFactor", 1.0)
    b.inputs["Metallic"].default_value = pbr.get("metallicFactor", 1.0)
    if "emissiveFactor" in g:
        b.inputs["Emission Color"].default_value = tuple(g["emissiveFactor"]) + (1,)
        b.inputs["Emission Strength"].default_value = 1.0
    if "baseColorTexture" in pbr:
        im = doc.j["images"][doc.j["textures"][pbr["baseColorTexture"]["index"]]["source"]]
        bv = doc.j["bufferViews"][im["bufferView"]]
        data = bytes(doc.bin[bv.get("byteOffset", 0):bv.get("byteOffset", 0) + bv["byteLength"]])
        import tempfile
        path = os.path.join(tempfile.gettempdir(), f"otto_school_{tag}_{k}.png")
        with open(path, "wb") as f:
            f.write(data)
        t = nt.nodes.new("ShaderNodeTexImage")
        t.image = bpy.data.images.load(path)
        nt.links.new(t.outputs["Color"], b.inputs["Base Color"])
        if g.get("alphaMode") in ("MASK", "BLEND"):
            nt.links.new(t.outputs["Alpha"], b.inputs["Alpha"])
            m.surface_render_method = "DITHERED"
    _TMP[key] = m
    return m


def char_import(file, pos, rot_y, clip, frame):
    """Blender's glTF importer (fine for the headmaster, whose clips key the
    joints they use and whose rest scale is uniform)."""
    from mathutils import Matrix
    before = set(bpy.data.objects)
    acts_before = set(bpy.data.actions)
    bpy.ops.import_scene.gltf(filepath=os.path.join(PUB, file))
    new = [o for o in bpy.data.objects if o not in before]
    new_acts = [a for a in bpy.data.actions if a not in acts_before]
    for o in new:
        if o.type == "MESH" and o.name.startswith("Icosphere"):
            bpy.data.objects.remove(o)
    new = [o for o in bpy.data.objects if o not in before]
    arm = next(o for o in new if o.type == "ARMATURE")
    top = arm
    while top.parent is not None:
        top = top.parent
    bpy.context.view_layer.update()
    top.matrix_world = Matrix.Translation(G(*pos)) @ Matrix.Rotation(math.radians(rot_y), 4, "Z") @ top.matrix_world
    act = next(a for a in new_acts if a.name.split(".")[0] == clip or a.name.startswith(clip + "_"))
    arm.animation_data_create()
    arm.animation_data.action = act
    if act.slots:
        arm.animation_data.action_slot = act.slots[0]
    arm["_frame"] = frame
    return top, arm


def char(file, pos, rot_y, clip, frame):
    if file.startswith("headmaster"):
        return char_import(file, pos, rot_y, clip, frame)
    return char_baked(file, pos, rot_y, clip, frame)


def char_baked(file, pos, rot_y, clip, frame):
    """Pose a character with lib/posekit (glTF semantics, exactly what
    three.js does: unkeyed joints keep their node TRS) and bake it into
    static Blender meshes — Blender's own importer re-bases unkeyed joints
    and drops joint rest scale, so it can't preview these files faithfully."""
    from lib.gltfpatch import Doc
    from lib.posekit import Rig
    doc = Doc(os.path.join(PUB, file))
    rig = Rig(doc)
    dur = rig.duration(clip)
    t = (frame / 30.0) % dur if dur else 0.0
    P = rig.pose_at(clip, t)
    W = rig.world(P)
    j = doc.j
    reach = []
    stack = list(j["scenes"][j.get("scene", 0)]["nodes"])
    while stack:
        i = stack.pop()
        reach.append(i)
        stack += j["nodes"][i].get("children", [])
    root = bpy.data.objects.new(file, None)
    bpy.context.scene.collection.objects.link(root)
    tag = f"{file}_{len(bpy.data.objects)}"
    for i in sorted(reach):
        node = j["nodes"][i]
        if "mesh" not in node:
            continue
        if "skin" in node:
            skin = j["skins"][node["skin"]]
            ibm = doc.acc(skin["inverseBindMatrices"]).reshape(-1, 4, 4).transpose(0, 2, 1)
            mats = np.stack([W[jn] @ ibm[k] for k, jn in enumerate(skin["joints"])])
        else:
            M = rig.node_world(P, i, W)
        for pi, p in enumerate(j["meshes"][node["mesh"]]["primitives"]):
            pos_ = doc.acc(p["attributes"]["POSITION"])
            nrm = doc.acc(p["attributes"]["NORMAL"]) if "NORMAL" in p["attributes"] else None
            if "skin" in node:
                jj = doc.acc(p["attributes"]["JOINTS_0"]).astype(int)
                ww = doc.acc(p["attributes"]["WEIGHTS_0"])
                ph = np.c_[pos_, np.ones(len(pos_))]
                v = np.zeros((len(pos_), 4))
                n = np.zeros_like(nrm) if nrm is not None else None
                for k in range(4):
                    mk = mats[jj[:, k]]
                    v += ww[:, k:k + 1] * np.einsum("nij,nj->ni", mk, ph)
                    if n is not None:
                        n += ww[:, k:k + 1] * np.einsum("nij,nj->ni", mk[:, :3, :3], nrm)
                v = v[:, :3]
            else:
                v = (M @ np.c_[pos_, np.ones(len(pos_))].T).T[:, :3]
                n = (M[:3, :3] @ nrm.T).T if nrm is not None else None
            if n is not None:
                n /= np.linalg.norm(n, axis=1, keepdims=True) + 1e-12
            idx = doc.acc(p["indices"])[:, 0].astype(int).reshape(-1, 3)
            bv = [(float(a), float(-c), float(b_)) for a, b_, c in v]
            me = bpy.data.meshes.new(f"{node.get('name', 'm')}_{pi}")
            me.from_pydata(bv, [], [tuple(int(x) for x in f) for f in idx])
            if "TEXCOORD_0" in p["attributes"]:
                uv = doc.acc(p["attributes"]["TEXCOORD_0"])
                lay = me.uv_layers.new(name="UVMap")
                for poly in me.polygons:
                    for li in poly.loop_indices:
                        vi = me.loops[li].vertex_index
                        lay.data[li].uv = (uv[vi, 0], 1 - uv[vi, 1])
            if n is not None:
                me.normals_split_custom_set_from_vertices([(float(a), float(-c), float(b_)) for a, b_, c in n])
            if "material" in p:
                me.materials.append(_gl_material(doc, p["material"], tag))
            ob = bpy.data.objects.new(me.name, me)
            bpy.context.scene.collection.objects.link(ob)
            ob.parent = root
    from mathutils import Matrix
    root.matrix_world = Matrix.Translation(G(*pos)) @ Matrix.Rotation(math.radians(rot_y), 4, "Z")
    return root, None


def apply_frames():
    """Each character holds its own frame: bake the evaluated pose into the
    pose bones so a single scene frame can show different clip phases."""
    sc = bpy.context.scene
    arms = [o for o in bpy.data.objects if o.type == "ARMATURE" and "_frame" in o]
    if not arms:
        return
    for a in arms:
        sc.frame_set(int(a["_frame"]))
        bpy.context.view_layer.update()
        basis = {pb.name: (pb.location.copy(), pb.rotation_quaternion.copy(), pb.scale.copy())
                 for pb in a.pose.bones}
        a.animation_data.action = None
        for pb in a.pose.bones:
            pb.location, pb.rotation_quaternion, pb.scale = basis[pb.name]
    bpy.context.view_layer.update()


def camera(pos, target, lens=24):
    cam = bpy.data.objects.new("Cam", bpy.data.cameras.new("Cam"))
    bpy.context.scene.collection.objects.link(cam)
    cam.location = G(*pos)
    d = G(*target) - cam.location
    cam.rotation_euler = d.to_track_quat("-Z", "Y").to_euler()
    cam.data.lens = lens
    cam.data.clip_end = 200
    bpy.context.scene.camera = cam
    return cam


def area(pos, size, power, color=(1, 0.97, 0.92), target=None, shape="RECTANGLE", size_y=None):
    L = bpy.data.lights.new("Area", "AREA")
    L.energy = power
    L.color = color
    L.shape = shape
    L.size = size[0] if isinstance(size, tuple) else size
    if isinstance(size, tuple):
        L.size_y = size[1]
    o = bpy.data.objects.new("Area", L)
    bpy.context.scene.collection.objects.link(o)
    o.location = G(*pos)
    if target is not None:
        o.rotation_euler = (G(*target) - o.location).to_track_quat("-Z", "Y").to_euler()
    return o


def sun(direction, strength, color=(1, 0.95, 0.86), angle=2.0):
    L = bpy.data.lights.new("Sun", "SUN")
    L.energy = strength
    L.color = color
    L.angle = math.radians(angle)
    o = bpy.data.objects.new("Sun", L)
    bpy.context.scene.collection.objects.link(o)
    o.rotation_euler = Vector(G(*direction)).to_track_quat("-Z", "Y").to_euler()
    return o


def code_image(name, seed, w=320, h=200):
    """Fake code-editor screenshot for the preview screens (pre-flipped: the
    Screen UVs follow the painted-plane convention)."""
    rng = random.Random(seed)
    px = np.zeros((h, w, 4), dtype=np.float32)
    px[..., :3] = (0.11, 0.12, 0.17)
    px[..., 3] = 1
    px[:, :18, :3] = (0.15, 0.16, 0.22)
    cols = [(0.55, 0.75, 1.0), (0.95, 0.6, 0.85), (0.6, 0.9, 0.6), (1.0, 0.8, 0.45), (0.85, 0.85, 0.9)]
    y = 12
    indent = 0
    while y < h - 10:
        x = 26 + indent * 12
        for _ in range(rng.randint(1, 4)):
            ln = rng.randint(10, 60)
            if x + ln > w - 8:
                break
            px[y:y + 5, x:x + ln, :3] = rng.choice(cols)
            x += ln + 6
        indent = max(0, min(3, indent + rng.choice([-1, 0, 0, 1])))
        y += 11
    # rows are already bottom-up in Blender's buffer; flip for the painted UVs
    img = bpy.data.images.new(name, w, h)
    img.pixels = px.ravel()
    return img


def screen_mat(img):
    m = bpy.data.materials.new("ScreenPreview")
    m.use_nodes = True
    nt = m.node_tree
    b = nt.nodes["Principled BSDF"]
    t = nt.nodes.new("ShaderNodeTexImage")
    t.image = img
    nt.links.new(t.outputs["Color"], b.inputs["Base Color"])
    nt.links.new(t.outputs["Color"], b.inputs["Emission Color"])
    b.inputs["Emission Strength"].default_value = 1.0
    b.inputs["Roughness"].default_value = 0.25
    return m


def plane_obj(name, w, d, pos, color, emit=0.0):
    bpy.ops.mesh.primitive_plane_add(size=1, location=G(*pos))
    o = bpy.context.active_object
    o.name = name
    o.scale = (w, d, 1)
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = color + (1,)
    b.inputs["Roughness"].default_value = 0.9
    if emit:
        b.inputs["Emission Color"].default_value = color + (1,)
        b.inputs["Emission Strength"].default_value = emit
    o.data.materials.append(m)
    return o


def chalk_text(text, pos, size, rot_y=0.0, color=(0.92, 0.94, 0.9)):
    cu = bpy.data.curves.new("txt", "FONT")
    cu.body = text
    cu.size = size
    cu.align_x = "CENTER"
    cu.align_y = "CENTER"
    o = bpy.data.objects.new("txt", cu)
    bpy.context.scene.collection.objects.link(o)
    o.location = G(*pos)
    o.rotation_euler = (math.radians(90), 0, math.radians(rot_y))
    m = bpy.data.materials.new("Chalk")
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = tuple(color) + (1,)
    b.inputs["Roughness"].default_value = 1.0
    cu.materials.append(m)
    return o


def render(name):
    os.makedirs(OUT, exist_ok=True)
    apply_frames()
    sc = bpy.context.scene
    sc.render.image_settings.file_format = "JPEG"
    sc.render.image_settings.quality = 90
    sc.render.filepath = os.path.join(OUT, name + ".jpg")
    bpy.ops.render.render(write_still=True)
    print("[preview] wrote", sc.render.filepath)


# ---------------------------------------------------------------- scenes

PROVS = ["claude", "codex", "grok", "agy", "shell", "custom"]


def classroom():
    reset()
    load_kit()
    X0, X1, Z0, Z1 = -5.0, 5.0, -6.0, 6.0
    for x in np.arange(X0 + 1, X1, 2.0):
        for z in np.arange(Z0 + 1, Z1, 2.0):
            inst("Floor_Tile", (x, 0, z))
    # Front wall (board) at z = +6 facing −Z; back wall z = −6 facing +Z.
    for x in (-4, -2, 0, 2, 4):
        inst("Wall", (x, 0, Z1), 180)
        inst("Wall", (x, 0, Z0), 0)
    for i, z in enumerate((-5, -3, -1, 1, 3, 5)):
        inst("Wall_Window" if i in (1, 2, 3, 4) else "Wall", (X0, 0, z), 90)
        inst("Wall_Door" if i == 4 else "Wall", (X1, 0, z), -90)
    inst("Board", (0.4, 0, Z1), 180)
    inst("Clock", (-3.3, 2.55, Z1), 180)
    inst("Poster_1", (3.4, 1.65, Z1), 180)
    inst("Poster_2", (-4.2, 1.65, Z1), 180)
    inst("Poster_3", (X1, 1.7, -3.0), -90)
    inst("Poster_4", (X1, 1.7, -0.6), -90)
    inst("Poster_5", (X1, 1.7, 1.2), -90)
    inst("Poster_6", (2.5, 1.7, Z0), 0)
    inst("Bookshelf", (-1.2, 0, Z0 + 0.22), 0)
    inst("Bookshelf", (0.1, 0, Z0 + 0.22), 0)
    inst("Plant", (-4.4, 0, Z1 - 0.5))
    inst("Plant", (4.4, 0, Z0 + 0.5))
    inst("Bench", (-3.6, 0, Z0 + 0.3))
    inst("TeacherDesk", (-2.6, 0, 4.1), 180)
    for x in (-2.5, 0, 2.5):
        for z in (-3.5, 0.0, 3.5):
            inst("Ceiling_Light", (x, 2.98, z))
    ceil = plane_obj("Ceiling", 10, 12, (0, 3.0, 0), (0.95, 0.96, 0.98))
    ceil.rotation_euler = (math.pi, 0, 0)
    # Desks: 4 columns × 5 rows, kids face +Z (the board).
    rng = random.Random(3)
    cols = (-3.3, -1.1, 1.1, 3.3)
    rows = (-4.4, -2.9, -1.4, 0.1, 1.6)
    poses = ["Sit_Type"] * 12 + ["Sit_RaiseHand"] * 3 + ["Sit_Idle"] * 3 + ["Sit_Slump"]
    rng.shuffle(poses)
    k = 0
    for zi, z in enumerate(rows):
        for xi, x in enumerate(cols):
            ws_mats = None
            if rng.random() < 0.8:
                ws_mats = {"Screen": screen_mat(code_image(f"code{k}", k))}
            inst("Workstation", (x, 0, z + 0.55), 0, mats=ws_mats)
            inst("Chair", (x, 0, z), 0)
            if (xi, zi) == (2, 2):
                continue  # this kid is out walking the aisle
            p = PROVS[(xi + zi * 3) % 6]
            clip = poses[k % len(poses)]
            char(f"kid-{p}.glb", (x, 0, z), 0, clip, rng.randint(0, 40))
            k += 1
    # decor from the kit's optional extras
    for k, (x, z) in enumerate([(-3.3, -4.4), (1.1, -2.9), (3.3, 0.1), (-1.1, 1.6), (1.1, -1.4)]):
        inst("Backpack", (x + 0.32, 0, z + 0.1), 90 + 25 * k)
    for (x, z) in [(-1.1, -4.4), (3.3, -2.9), (-3.3, 0.1)]:
        inst("Book", (x + 0.42, 0.6, z + 0.55 + 0.12), 15)
        inst("Water_Bottle", (x - 0.48, 0.6, z + 0.55 + 0.1))
    inst("Plant_Desk", (-3.2, 0.78, 4.0))
    inst("Book_Open", (-2.4, 0.78, 3.85), 170)
    inst("Trashcan", (-4.4, 0, 3.6))
    inst("Corkboard", (-3.4, 1.6, Z0), 0)
    char("kid-codex.glb", (-2.2, 0, -2.3), 160, "Walk", 6)
    char("headmaster-otto.glb", (3.0, 0, 4.85), -105, "Point_Board", 20)
    chalk_text("Agents 101", (0.4 - 0.97, 2.03, Z1 - 0.035), 0.24, 180)
    # Light: sun through the left windows + ceiling panels + window bounce.
    sun((0.75, -0.55, 0.35), 4.0)
    for x in (-2.5, 0, 2.5):
        for z in (-3.5, 0.0, 3.5):
            area((x, 2.9, z), (1.2, 0.3), 160, target=(x, 0, z))
    area((X0 + 0.3, 1.7, 0), (0.3, 8.0), 400, color=(0.85, 0.92, 1.0), target=(0, 1.0, 0))
    camera((-4.3, 2.55, -5.5), (0.6, 0.55, 2.6), lens=20)
    render("classroom")


def corridor():
    reset()
    load_kit()
    for x in (-5, -3, -1, 1, 3, 5):
        inst("Corridor_Floor", (x, 0, 0))
        inst("Corridor_Floor", (x, 0, 2))
    for i, x in enumerate((-5, -3, -1, 1, 3, 5)):
        inst("Corridor_Door" if i in (0, 2, 4) else "Corridor_Wall", (x, 0, -1), 0)
        inst("Corridor_Wall", (x, 0, 3), 180)
    inst("Bench", (-2.0, 0, 2.6), 180)
    for x, name in ((-5, "otto_os"), (-1, "agents-lab"), (3, "web-ui")):
        chalk_text(name, (x - 0.35, 2.48, -1 + 0.03), 0.16, 0, color=(0.08, 0.12, 0.25))
    inst("Plant", (5.4, 0, 2.5))
    for x in (-4, 0, 4):
        inst("Ceiling_Light", (x, 2.98, 1))
    ceil = plane_obj("Ceiling", 12, 4, (0, 3.0, 1), (0.95, 0.96, 0.98))
    ceil.rotation_euler = (math.pi, 0, 0)
    char("kid-claude.glb", (-1.6, 0, 0.6), 90, "Walk", 4)
    char("kid-agy.glb", (1.9, 0, 0.3), -30, "Talk", 10)
    char("kid-shell.glb", (2.6, 0, 0.5), -120, "Look_Around", 30)
    char("kid-grok.glb", (-2.0, 0, 2.55), 180, "Sit_Slump", 10)
    char("headmaster-otto.glb", (4.2, 0, 1.3), -100, "Walk", 8)
    for x in (-4, 0, 4):
        area((x, 2.9, 1), (1.2, 0.3), 160, target=(x, 0, 1))
    area((0, 2.0, 2.5), (10.0, 0.5), 300, color=(1.0, 0.95, 0.88), target=(0, 1.0, -1))
    camera((-5.6, 1.75, 2.6), (1.5, 0.95, -0.9), lens=20)
    render("corridor")


def lineup():
    reset()
    load_kit()
    for x in (-3, -1, 1, 3):
        for z in (-1, 1):
            inst("Floor_Tile", (x, 0, z))
    for x in (-3, -1, 1, 3):
        inst("Wall", (x, 0, -1.6))
    inst("Poster_1", (-2.5, 1.7, -1.6))
    inst("Poster_3", (2.5, 1.7, -1.6))
    clips = ["Wave", "Idle_Stand", "Talk", "Idle_Stand", "Look_Around", "Wave"]
    for i, p in enumerate(PROVS):
        char(f"kid-{p}.glb", (-2.9 + i * 0.95, 0, 0.4), 0, clips[i], 8 + i * 5)
    char("headmaster-otto.glb", (3.3, 0, -0.1), -20, "Wave", 10)
    area((0, 3.5, 3.5), (6, 2), 900, target=(0, 0.6, 0))
    area((-4, 2.5, 1.5), 2, 200, color=(0.85, 0.92, 1.0), target=(0, 0.6, 0))
    sun((0.3, -0.8, -0.5), 2.0)
    camera((0.45, 1.25, 6.6), (0.45, 0.78, 0), lens=28)
    render("lineup")


def seat():
    """Debug: side + 3/4 views of each seated clip on a Chair + Workstation."""
    reset((1600, 900))
    load_kit()
    for x in (-3, -1, 1, 3):
        inst("Floor_Tile", (x, 0, 0))
    clips = ["Sit_Type", "Sit_Idle", "Sit_RaiseHand", "Sit_Slump"]
    for i, c in enumerate(clips):
        x = -2.7 + i * 1.8
        inst("Chair", (x, 0, 0))
        inst("Workstation", (x, 0, 0.55))
        char(f"kid-{PROVS[i]}.glb", (x, 0, 0), 0, c, 10)
    area((0, 4, 3), (6, 3), 1200, target=(0, 0.5, 0))
    sun((0.4, -0.7, -0.4), 2.5)
    side = "--side" in sys.argv
    if side:
        camera((9.5, 0.75, 0.2), (0, 0.6, 0.2), lens=40)
    else:
        camera((-3.6, 1.9, -2.6), (0.4, 0.55, 0.4), lens=28)
    render("seat_side" if side else "seat")


def robot():
    """Debug: the headmaster in every clip, plus a 1.6 m reference pole."""
    reset()
    load_kit()
    for x in (-5, -3, -1, 1, 3, 5):
        inst("Floor_Tile", (x, 0, 0))
    clips = ["Idle", "Walk", "Nod", "Scold", "Wave", "ThumbsUp", "Inspect_Screen", "Point_Board"]
    for i, c in enumerate(clips):
        char("headmaster-otto.glb", (-5.25 + i * 1.5, 0, 0), 0, c, 12)
    bpy.ops.mesh.primitive_cylinder_add(radius=0.02, depth=1.6, location=G(5.9, 0.8, 0))
    area((0, 5, 6), (12, 3), 2500, target=(0, 0.8, 0))
    camera((0, 1.2, 11), (0, 0.8, 0), lens=35)
    render("dbg/robot")


def window():
    """Debug: wall pieces seen straight on."""
    reset()
    load_kit()
    for x in (-3, -1, 1, 3):
        inst("Floor_Tile", (x, 0, 1))
    inst("Wall", (-3, 0, 0))
    inst("Wall_Window", (-1, 0, 0))
    inst("Wall_Door", (1, 0, 0))
    inst("Corridor_Door", (3.2, 0, 0))
    inst("Corridor_Wall", (5.4, 0, 0))
    inst("Clock", (-3, 2.4, 0))
    inst("Bench", (5.4, 0, 0.8))
    inst("Bookshelf", (-3, 0, 0.25))
    area((0, 3, 5), (10, 3), 1500, target=(0, 1.3, 0))
    camera((1.2, 1.6, 7.5), (1.2, 1.4, 0), lens=26)
    render("dbg/walls")


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    which = [a for a in argv if not a.startswith("--")]
    which = which[0].split(",") if which else ["classroom", "corridor", "lineup"]
    for w in which:
        globals()[w]()


def seatdbg():
    """Debug: one image per (clip, view) → previews/dbg/."""
    views = {"side": ((3.0, 0.75, 0.3), (0, 0.6, 0.3)), "front": ((1.4, 1.5, 2.6), (0, 0.6, 0.2)),
             "back": ((-1.2, 1.6, -1.8), (0, 0.55, 0.3))}
    argv = sys.argv[sys.argv.index("--") + 1:]
    clips = [a for a in argv if a.startswith("clip=")]
    clips = clips[0][5:].split(",") if clips else ["Sit_Type", "Sit_Idle", "Sit_RaiseHand", "Sit_Slump"]
    prov = [a for a in argv if a.startswith("prov=")]
    prov = prov[0][5:] if prov else "codex"
    frame = [a for a in argv if a.startswith("frame=")]
    frame = int(frame[0][6:]) if frame else 10
    for c in clips:
        for vn, (cp, ct) in views.items():
            reset((800, 600))
            load_kit()
            inst("Floor_Tile", (0, 0, 0))
            if c.startswith("Sit"):
                inst("Chair", (0, 0, 0))
                inst("Workstation", (0, 0, 0.55))
            char(f"kid-{prov}.glb", (0, 0, 0), 0, c, frame)
            area((2, 4, 3), (4, 3), 900, target=(0, 0.5, 0))
            sun((0.4, -0.7, -0.4), 2.5)
            camera(cp, ct, lens=35)
            apply_frames()
            sc = bpy.context.scene
            sc.render.filepath = os.path.join(OUT, "dbg", f"{c}_{vn}.png")
            bpy.ops.render.render(write_still=True)


if __name__ == "__main__":
    main()
