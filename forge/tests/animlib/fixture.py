"""Build two independently skinned humanoids with equal lengths and different bind poses."""

import sys
from math import cos, pi, radians, sin
from pathlib import Path

import bpy
from mathutils import Quaternion, Vector

dest = Path(sys.argv[-1]).resolve()
dest.mkdir(parents=True, exist_ok=True)


def name(key, unity):
    if unity:
        return key
    mix = {
        "UpperChest": "Spine2",
        "Chest": "Spine1",
        "UpperArm": "Arm",
        "LowerArm": "ForeArm",
        "UpperLeg": "UpLeg",
        "LowerLeg": "Leg",
        "Toes": "ToeBase",
    }
    side = (
        "Left" if key.startswith("Left") else "Right" if key.startswith("Right") else ""
    )
    part = key[len(side) :]
    return "mixamorig:" + side + mix.get(part, part)


def build(unity):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.fps = 24
    rig = bpy.data.objects.new(
        "Player" if unity else "Performer", bpy.data.armatures.new("Skeleton")
    )
    bpy.context.collection.objects.link(rig)
    bpy.context.view_layer.objects.active = rig
    rig.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")
    pieces = []

    def bone(key, a, b, parent, thick, material):
        joint = rig.data.edit_bones.new(name(key, unity))
        joint.head = a
        joint.tail = b
        if parent:
            joint.parent = rig.data.edit_bones[name(parent, unity)]
        if unity:
            joint.roll = radians(13 if "Left" in key else -9)
        pieces.append((joint.name, Vector(a), Vector(b), thick, material))

    for key, z1, z2, parent, r, mat in [
        ("Hips", 0.98, 1.09, None, 0.115, 0),
        ("Spine", 1.09, 1.24, "Hips", 0.12, 0),
        ("Chest", 1.24, 1.39, "Spine", 0.15, 0),
        ("UpperChest", 1.39, 1.52, "Chest", 0.16, 0),
        ("Neck", 1.52, 1.61, "UpperChest", 0.06, 1),
        ("Head", 1.61, 1.86, "Neck", 0.112, 1),
    ]:
        bone(key, (0, 0, z1), (0, 0, z2), parent, r, mat)
    for side, k in [("Left", 1), ("Right", -1)]:
        angle = radians(37 if unity else 0)
        s = Vector((k * 0.175, 0, 1.495))
        axis = Vector((k * cos(angle), 0, -sin(angle)))
        e = s + 0.29 * axis
        w = e + 0.25 * axis
        hip = Vector((k * 0.105, 0, 0.98))
        knee = Vector((k * 0.105, -0.02, 0.53))
        ankle = Vector((k * 0.105, 0, 0.09))
        toe = ankle + Vector((0, -0.12, -0.045))
        bone(side + "Shoulder", (k * 0.028, 0, 1.495), s, "UpperChest", 0.065, 0)
        bone(side + "UpperArm", s, e, side + "Shoulder", 0.052, 0)
        bone(side + "LowerArm", e, w, side + "UpperArm", 0.046, 1)
        bone(side + "Hand", w, w + 0.106 * axis, side + "LowerArm", 0.05, 2)
        bone(side + "UpperLeg", hip, knee, "Hips", 0.066, 0)
        bone(side + "LowerLeg", knee, ankle, side + "UpperLeg", 0.055, 1)
        bone(side + "Foot", ankle, toe, side + "LowerLeg", 0.060, 2)
        bone(side + "Toes", toe, toe + Vector((0, -0.07, 0)), side + "Foot", 0.044, 2)
    bpy.ops.object.mode_set(mode="OBJECT")
    objects = []
    materials = []
    for title, color in [
        ("Slate", (0.11, 0.36, 0.43, 1)),
        ("Ivory", (0.82, 0.74, 0.56, 1)),
        ("Amber", (0.78, 0.22, 0.05, 1)),
    ]:
        mat = bpy.data.materials.new(title)
        mat.diffuse_color = color
        materials.append(mat)
    for key, a, b, r, m in pieces:
        bpy.ops.mesh.primitive_uv_sphere_add(segments=12, ring_count=8)
        ob = bpy.context.object
        length = (b - a).length
        q = (b - a).to_track_quat("Z", "Y")
        for vertex in ob.data.vertices:
            x, y, z = vertex.co
            vertex.co = (a + b) * 0.5 + q @ Vector(
                (x * r * 1.12, y * r, z * (length * 0.5 + 0.012))
            )
        ob.data.materials.append(materials[m])
        ob.vertex_groups.new(name=key).add(range(len(ob.data.vertices)), 1, "REPLACE")
        objects.append(ob)
    bpy.ops.object.select_all(action="DESELECT")
    for ob in objects:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    bpy.ops.object.join()
    skin = bpy.context.object
    skin.name = "Weighted body"
    skin.parent = rig
    skin.modifiers.new("Armature", "ARMATURE").object = rig
    if not unity:
        ad = rig.animation_data_create()
        for title, last in [("stride", 25), ("gesture", 19)]:
            action = bpy.data.actions.new(title)
            ad.action = action
            for frame in range(1, last + 1):
                u = (frame - 1) / (last - 1)
                for p in rig.pose.bones:
                    p.rotation_mode = "QUATERNION"
                    p.rotation_quaternion.identity()
                    p.location = (0, 0, 0)

                def rotate(key, axis, deg):
                    p = rig.pose.bones[name(key, False)]
                    # Rotation about an armature-space axis, represented in the joint's local frame.
                    local = p.bone.matrix_local.to_quaternion().inverted() @ Vector(
                        axis
                    )
                    p.rotation_quaternion = Quaternion(local, radians(deg))

                if title == "stride":
                    # Static supporting left foot for the entire first quarter of this clip.
                    hips = rig.pose.bones[name("Hips", False)]
                    travel = max(0, (u - 0.25) / 0.75)
                    hips.location = (
                        hips.bone.matrix_local.to_quaternion().inverted()
                        @ Vector((0, -0.47 * travel, 0))
                    )
                    rotate("RightUpperLeg", (1, 0, 0), 32 * sin(2 * pi * u))
                    rotate("RightLowerLeg", (1, 0, 0), -27 * max(0, sin(2 * pi * u)))
                    rotate("LeftUpperLeg", (1, 0, 0), -25 * sin(2 * pi * travel))
                    rotate("LeftUpperArm", (1, 0, 0), 23 * sin(2 * pi * u))
                else:
                    rotate("Chest", (0, 0, 1), 15 * sin(pi * u))
                    rotate("RightUpperArm", (0, 1, 0), 45 * sin(pi * u))
                    rotate("RightLowerArm", (0, 0, 1), 65 * sin(pi * u))
                    rotate("Head", (1, 0, 0), 10 * sin(pi * u))
                for pose in rig.pose.bones:
                    pose.keyframe_insert(
                        "rotation_quaternion", frame=frame, group=pose.name
                    )
                rig.pose.bones[name("Hips", False)].keyframe_insert(
                    "location", frame=frame
                )
            slot = ad.action_slot
            ad.action = None
            track = ad.nla_tracks.new()
            track.name = title
            strip = track.strips.new(title, 1, action)
            strip.action_slot = slot
    scene.frame_set(1)
    path = dest / ("unity.glb" if unity else "mixamo.glb")
    bpy.ops.export_scene.gltf(
        filepath=str(path),
        export_format="GLB",
        export_animations=not unity,
        export_animation_mode="NLA_TRACKS",
        export_frame_range=False,
        export_rest_position_armature=True,
    )


for unity in [False, True]:
    build(unity)
