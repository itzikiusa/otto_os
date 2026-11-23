"""Procedural, fast fixtures for the fit tests: a skinned humanoid with finger chains (Unity names) and
clean props (sword, baton, bow with a modelled string to be discarded, quiver, arrow), all in metres.

Run: Blender -b --factory-startup -P tests/fit/make_fixtures.py -- OUTDIR
"""

import sys
from math import cos, radians, sin
from pathlib import Path

import bpy
from mathutils import Matrix, Quaternion, Vector

dest = Path(sys.argv[-1]).resolve()
dest.mkdir(parents=True, exist_ok=True)


def fresh():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.render.fps = 30


def material(name, rgb, metal=0.0):
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*rgb, 1)
    m.use_nodes = True
    bsdf = m.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*rgb, 1)
    bsdf.inputs["Metallic"].default_value = metal
    return m


def capsule(a, b, r, mat, segments=14, rings=8):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=segments, ring_count=rings)
    ob = bpy.context.object
    a, b = Vector(a), Vector(b)
    q = (b - a).to_track_quat("Z", "Y")
    half = (b - a).length * 0.5
    for v in ob.data.vertices:
        x, y, z = v.co
        z = z * (half + r) if abs(z) < 1e-6 else (z / abs(z)) * half + z * r
        v.co = (a + b) * 0.5 + q @ Vector((x * r, y * r, z))
    ob.data.materials.append(mat)
    return ob


def box(centre, axes, size, mat):
    bpy.ops.mesh.primitive_cube_add(size=1)
    ob = bpy.context.object
    M = Matrix((axes[0], axes[1], axes[2])).transposed()
    for v in ob.data.vertices:
        v.co = Vector(centre) + M @ Vector((v.co.x * size[0], v.co.y * size[1], v.co.z * size[2]))
    ob.data.materials.append(mat)
    return ob


def join(objs, name):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    ob = bpy.context.object
    ob.name = name
    bpy.ops.object.shade_smooth()
    return ob


def humanoid():
    fresh()
    rig = bpy.data.objects.new("Fixture", bpy.data.armatures.new("Skeleton"))
    bpy.context.collection.objects.link(rig)
    bpy.context.view_layer.objects.active = rig
    bpy.ops.object.mode_set(mode="EDIT")
    pieces = []
    skin, cloth, glove = material("Skin", (0.8, 0.62, 0.5)), material("Cloth", (0.15, 0.3, 0.45)), material("Glove", (0.55, 0.35, 0.2))

    def bone(name, a, b, parent, r, mat, shape="capsule"):
        e = rig.data.edit_bones.new(name)
        e.head, e.tail = a, b
        if parent:
            e.parent = rig.data.edit_bones[parent]
        pieces.append((name, Vector(a), Vector(b), r, mat, shape))

    for key, z1, z2, parent, r in [("Hips", 0.98, 1.09, None, 0.13), ("Spine", 1.09, 1.24, "Hips", 0.125),
                                   ("Chest", 1.24, 1.39, "Spine", 0.15), ("UpperChest", 1.39, 1.52, "Chest", 0.155),
                                   ("Neck", 1.52, 1.61, "UpperChest", 0.055), ("Head", 1.61, 1.86, "Neck", 0.105)]:
        bone(key, (0, 0, z1), (0, 0, z2), parent, r, skin if key in ("Neck", "Head") else cloth)
    for side, k in (("Left", 1), ("Right", -1)):
        ang = radians(37)
        s = Vector((k * 0.175, 0, 1.495))
        d = Vector((k * cos(ang), 0, -sin(ang)))
        n = Vector((-k * sin(ang), 0, -cos(ang)))  # palm faces the thigh and the floor
        a = Vector((0, -1, 0))  # little -> index points forward
        e = s + 0.29 * d
        w = e + 0.25 * d
        bone(side + "Shoulder", (k * 0.03, 0, 1.495), s, "UpperChest", 0.06, cloth)
        bone(side + "UpperArm", s, e, side + "Shoulder", 0.05, cloth)
        bone(side + "LowerArm", e, w, side + "UpperArm", 0.042, skin)
        bone(side + "Hand", w, w + 0.09 * d, side + "LowerArm", 0.0, glove, "palm")
        for digit, off, lens in (("Index", 0.03, (0.045, 0.027, 0.022)), ("Middle", 0.01, (0.05, 0.03, 0.024)),
                                 ("Ring", -0.01, (0.046, 0.028, 0.022)), ("Little", -0.03, (0.036, 0.022, 0.019))):
            p = w + 0.09 * d + off * a
            parent = side + "Hand"
            for seg, ln in zip(("Proximal", "Intermediate", "Distal"), lens):
                q = p + ln * d
                bone(side + digit + seg, p, q, parent, 0.0085, glove)
                parent, p = side + digit + seg, q
        t = (0.55 * d + 0.55 * a + 0.6 * n).normalized()
        p = w + 0.025 * d + 0.035 * a + 0.012 * n
        parent = side + "Hand"
        for seg, ln in zip(("Proximal", "Intermediate", "Distal"), (0.04, 0.032, 0.026)):
            q = p + ln * t
            bone(side + "Thumb" + seg, p, q, parent, 0.0105, glove)
            parent, p = side + "Thumb" + seg, q
        pieces.append(("__palm__" + side, w, d, a, n, glove))
        hip, knee, ankle = Vector((k * 0.105, 0, 0.98)), Vector((k * 0.105, -0.01, 0.53)), Vector((k * 0.105, 0, 0.09))
        toe = ankle + Vector((0, -0.12, -0.045))
        bone(side + "UpperLeg", hip, knee, "Hips", 0.075, cloth)
        bone(side + "LowerLeg", knee, ankle, side + "UpperLeg", 0.055, cloth)
        bone(side + "Foot", ankle, toe, side + "LowerLeg", 0.055, glove)
        bone(side + "Toes", toe, toe + Vector((0, -0.07, 0)), side + "Foot", 0.044, glove)
    bpy.ops.object.mode_set(mode="OBJECT")
    objs = []
    for piece in pieces:
        if piece[0].startswith("__palm__"):
            _, w, d, a, n, mat = piece
            ob = box(w + 0.05 * d, (d, a, n), (0.1, 0.085, 0.03), mat)
            ob.vertex_groups.new(name=piece[0][8:] + "Hand").add(range(len(ob.data.vertices)), 1, "REPLACE")
            objs.append(ob)
            continue
        name, a, b, r, mat, shape = piece
        if shape != "capsule" or r == 0:
            continue
        ob = capsule(a, b, r, mat, 10 if r < 0.02 else 16, 6 if r < 0.02 else 10)
        ob.vertex_groups.new(name=name).add(range(len(ob.data.vertices)), 1, "REPLACE")
        objs.append(ob)
    body = join(objs, "Body")
    body.parent = rig
    body.modifiers.new("Armature", "ARMATURE").object = rig
    return rig


def rotate(rig, name, axis, deg):
    p = rig.pose.bones[name]
    p.rotation_mode = "QUATERNION"
    local = p.bone.matrix_local.to_3x3().inverted() @ Vector(axis)
    p.rotation_quaternion = Quaternion(local.normalized(), radians(deg))


def clip(rig, title, frames, pose_fn):
    ad = rig.animation_data_create()
    action = bpy.data.actions.new(title)
    ad.action = action
    for f in range(frames + 1):
        for p in rig.pose.bones:
            p.rotation_mode = "QUATERNION"
            p.matrix_basis = Matrix.Identity(4)
        pose_fn(f / frames)
        bpy.context.view_layer.update()
        for p in rig.pose.bones:
            p.keyframe_insert("rotation_quaternion", frame=f + 1)
            p.keyframe_insert("location", frame=f + 1)
    slot = ad.action_slot
    ad.action = None
    track = ad.nla_tracks.new()
    track.name = title
    track.strips.new(title, 1, action).action_slot = slot
    track.mute = True


def chest_ik(rig, u):
    """Right hand driven into the chest by Blender IK, then frozen into FK channels."""
    target = bpy.data.objects.get("ChestTarget") or bpy.data.objects.new("ChestTarget", None)
    if target.name not in bpy.context.collection.objects:
        bpy.context.collection.objects.link(target)
    start = Vector(rig.pose.bones["RightHand"].bone.head_local)
    inside = Vector((0.0, -0.02, 1.33))
    target.location = start.lerp(inside, min(1.0, 2 * u) if u < 0.5 else max(0.0, 2 - 2 * u))
    pb = rig.pose.bones["RightLowerArm"]
    c = pb.constraints.get("IK") or pb.constraints.new("IK")
    c.target, c.chain_count = target, 2
    bpy.context.view_layer.update()
    mats = {n: rig.pose.bones[n].matrix.copy() for n in ("RightUpperArm", "RightLowerArm")}
    pb.constraints.remove(c)
    bpy.context.view_layer.update()
    for n in ("RightUpperArm", "RightLowerArm"):
        rig.pose.bones[n].matrix = mats[n]
        bpy.context.view_layer.update()


def animated(rig):
    def idle(u):
        rotate(rig, "Chest", (0, 1, 0), 3 * sin(6.2832 * u))
        rotate(rig, "Head", (1, 0, 0), 4 * sin(6.2832 * u))
        rotate(rig, "LeftUpperArm", (0, 1, 0), 4 * sin(6.2832 * u))

    def walk(u):
        ph = 6.2832 * u
        for side, sg in (("Left", 1), ("Right", -1)):
            rotate(rig, side + "UpperLeg", (1, 0, 0), -24 * sg * sin(ph))
            rotate(rig, side + "LowerLeg", (1, 0, 0), 30 * max(0.0, sg * sin(ph + 1.2)))
            rotate(rig, side + "UpperArm", (1, 0, 0), 18 * sg * sin(ph))
        rig.pose.bones["Hips"].location = (0, 0.012 * cos(2 * ph), 0)

    def broken(u):
        w = sin(3.1416 * u)
        rig.pose.bones["Hips"].location = (0, -0.06 * w, 0)  # hips' local Y is world up: feet under ground
        rotate(rig, "RightLowerLeg", (1, 0, 0), -32 * w)  # knee bent backwards
        chest_ik(rig, u)  # hand through the chest

    clip(rig, "clean", 30, idle)
    clip(rig, "walk", 30, walk)
    clip(rig, "broken", 30, broken)


def export(path, animations=True):
    bpy.context.scene.frame_set(1)
    bpy.ops.export_scene.gltf(filepath=str(path), export_format="GLB", export_animations=animations,
                              export_animation_mode="NLA_TRACKS", export_frame_range=False,
                              export_rest_position_armature=True)


def props():
    def fresh_props():
        fresh()
        return (material("Steel", (0.75, 0.77, 0.8), 1.0), material("Leather", (0.3, 0.17, 0.08)),
                material("Wood", (0.45, 0.28, 0.12)), material("Brass", (0.7, 0.5, 0.2), 1.0))

    steel, leather, wood, brass = fresh_props()
    parts = [capsule((0, 0, 0), (0, 0, 0.001), 0.028, brass),
             capsule((0, 0, 0.03), (0, 0, 0.19), 0.016, leather, 16, 8),
             box((0, 0, 0.205), ((1, 0, 0), (0, 1, 0), (0, 0, 1)), (0.2, 0.035, 0.025), brass)]
    bpy.ops.mesh.primitive_cone_add(vertices=4, radius1=1, radius2=0.08, depth=1)
    blade = bpy.context.object
    for v in blade.data.vertices:
        v.co = Vector((v.co.x * 0.026 * 1.4142 / 1.0, v.co.y * 0.005, (v.co.z + 0.5) * 0.765 + 0.218))
    blade.data.materials.append(steel)
    parts.append(blade)
    join(parts, "Sword")
    export(dest / "sword.glb", False)

    steel, leather, wood, brass = fresh_props()
    parts = [capsule((0, 0, -0.45), (0, 0, -0.28), 0.03, leather, 16, 8),
             box((0, 0, 0), ((1, 0, 0), (0, 1, 0), (0, 0, 1)), (0.2, 0.035, 0.025), brass),
             box((0, 0, -0.14), ((1, 0, 0), (0, 1, 0), (0, 0, 1)), (0.05, 0.006, 0.26), steel),
             box((0, 0, 0.14), ((1, 0, 0), (0, 1, 0), (0, 0, 1)), (0.05, 0.006, 0.26), steel),
             capsule((0, 0, 0.28), (0, 0, 0.45), 0.03, leather, 16, 8)]
    join(parts, "BrokenSword")
    export(dest / "sword_two_ended.glb", False)

    steel, leather, wood, brass = fresh_props()
    capsule((0, 0, -0.16), (0, 0, 0.16), 0.018, wood, 20, 8).name = "Baton"
    export(dest / "baton.glb", False)

    steel, leather, wood, brass = fresh_props()
    pieces = [capsule((0, 0, -0.065), (0, 0, 0.065), 0.021, leather, 16, 8),
              box((0, -0.004, 0.12), ((1, 0, 0), (0, 1, 0), (0, 0, 1)), (0.03, 0.045, 0.11), wood),
              box((0, -0.004, -0.12), ((1, 0, 0), (0, 1, 0), (0, 0, 1)), (0.03, 0.045, 0.11), wood)]
    for sg in (1, -1):
        prev = None
        for i in range(13):
            z = sg * (0.17 + 0.58 * i / 12)
            y = 0.18 * ((abs(z) - 0.17) / 0.58) ** 2
            p = Vector((0, y, z))
            if prev is not None:
                pieces.append(capsule(prev, p, 0.013 - 0.007 * i / 12, wood, 10, 6))
            prev = p
    join(pieces, "Bow")
    tip = 0.75
    capsule((0, 0.18, tip), (0, 0.18, -tip), 0.0015, material("String", (0.9, 0.9, 0.85)), 6, 4).name = "BowString"
    export(dest / "bow.glb", False)

    steel, leather, wood, brass = fresh_props()
    bpy.ops.mesh.primitive_cylinder_add(vertices=24, radius=0.05, depth=0.5, end_fill_type="NOTHING")
    tube = bpy.context.object
    tube.location = (0, 0, 0.25)
    bpy.ops.object.transform_apply(location=True)
    bpy.ops.mesh.primitive_circle_add(vertices=24, radius=0.05, fill_type="NGON")
    bottom = bpy.context.object
    tube.data.materials.append(leather)
    bottom.data.materials.append(leather)
    join([tube, bottom], "Quiver")
    bpy.ops.object.modifier_add(type="SOLIDIFY")
    bpy.context.object.modifiers[0].thickness = 0.004
    bpy.ops.object.modifier_apply(modifier=bpy.context.object.modifiers[0].name)
    export(dest / "quiver.glb", False)

    steel, leather, wood, brass = fresh_props()
    parts = [capsule((0, 0, 0.0), (0, 0, 0.75), 0.004, wood, 8, 4)]
    bpy.ops.mesh.primitive_cone_add(vertices=8, radius1=0.011, radius2=0, depth=0.045, location=(0, 0, 0.77))
    parts.append(bpy.context.object)
    parts[-1].data.materials.append(steel)
    for k in range(3):
        a = radians(120 * k)
        parts.append(box((0.009 * cos(a), 0.009 * sin(a), 0.07), ((cos(a), sin(a), 0), (-sin(a), cos(a), 0), (0, 0, 1)),
                         (0.012, 0.001, 0.1), material("Fletch%d" % k, (0.8, 0.1, 0.1))))
    join(parts, "Arrow")
    export(dest / "arrow.glb", False)


def mitten(rig):
    """Replace the right hand's separate fingers with one fused mitten weighted to the nearest finger bones
    (smoothly), the kind of hand that fails intake."""
    body = bpy.data.objects["Body"]
    names = [b.name for b in rig.data.bones if b.name.startswith("Right") and any(
        d in b.name for d in ("Thumb", "Index", "Middle", "Ring", "Little"))]
    gi = {body.vertex_groups[n].index for n in names}
    bm_dead = [v.index for v in body.data.vertices if any(g.group in gi for g in v.groups)]
    bpy.context.view_layer.objects.active = body
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="DESELECT")
    bpy.ops.object.mode_set(mode="OBJECT")
    for i in bm_dead:
        body.data.vertices[i].select = True
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.delete(type="VERT")
    bpy.ops.object.mode_set(mode="OBJECT")
    heads = {n: rig.data.bones[n].head_local for n in names}
    tails = {n: rig.data.bones[n].tail_local for n in names}
    lo = Vector([min(min(heads[n][i], tails[n][i]) for n in names) - 0.012 for i in range(3)])
    hi = Vector([max(max(heads[n][i], tails[n][i]) for n in names) + 0.012 for i in range(3)])
    bpy.ops.mesh.primitive_cube_add(size=1, location=(lo + hi) / 2)
    m = bpy.context.object
    m.scale = hi - lo
    bpy.ops.object.transform_apply(scale=True)
    mod = m.modifiers.new("Sub", "SUBSURF")
    mod.levels = 3
    bpy.ops.object.modifier_apply(modifier="Sub")
    for v in m.data.vertices:
        p = Vector(v.co)
        d = {n: ((p - heads[n]).length + (p - tails[n]).length) for n in names}
        best = sorted(d, key=d.get)[:3]
        tot = sum(1 / max(d[n], 1e-4) ** 2 for n in best)
        for n in best:
            g = m.vertex_groups.get(n) or m.vertex_groups.new(name=n)
            g.add([v.index], (1 / max(d[n], 1e-4) ** 2) / tot, "REPLACE")
    m.data.materials.append(body.data.materials[-1])
    m.parent = rig
    m.modifiers.new("Armature", "ARMATURE").object = rig
    m.name = "Mitten"


rig = humanoid()
export(dest / "humanoid.glb", False)
animated(rig)
export(dest / "humanoid_anim.glb", True)
mitten(rig)
export(dest / "humanoid_mitten.glb", True)
props()
print("FIXTURES", dest)
