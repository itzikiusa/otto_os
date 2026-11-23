"""Blender 5.2 worker. All lengths/rotations for FK are measured in armature space."""

import argparse
import json
import math
import re
import shutil
import sys
import time
from pathlib import Path

import bpy
from mathutils import Matrix, Quaternion, Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from grounding import SoleSampler, ground_action

FPS = 60
LIMBS = ("clavicle", "upperarm", "forearm", "hand", "thigh", "shin", "foot", "toe")
SPINE = ("hips", "spine", "chest", "upperchest", "neck", "head")


def check_space(path):
    if shutil.disk_usage(path).free < 55 * 1024**3:
        raise RuntimeError("Stopped: less than 55 GiB free")


def joint_map(rig):
    """Use semantic joints, excluding twist/deformation helpers with similar names."""
    names = {}
    for bone in rig.data.bones:
        name = bone.name.split(":")[-1].lower()
        name = re.sub(r"^(?:mixamorig\d*|bip0*1|def)[_. -]*", "", name)
        names[bone.name] = re.sub(r"[^a-z0-9]", "", name)
    mocap = "leftcollar" in names.values() and "leftelbow" in names.values()
    aliases = {
        "hips": ("hips", "pelvis", "hip", "rootpelvis"),
        "spine": ("chest",) if mocap else ("spine", "spine01", "spine001"),
        "chest": ("chest2",) if mocap else ("chest", "spine1", "spine02", "spine002"),
        "upperchest": ("chest4", "chest3")
        if mocap
        else ("upperchest", "spine2", "spine03", "spine003"),
        "neck": ("neck", "neck1", "neck01"),
        "head": ("head",),
    }
    limbs = {
        "clavicle": ("collar",) if mocap else ("shoulder", "clavicle", "collar"),
        "upperarm": ("shoulder",)
        if mocap
        else ("upperarm", "arm", "uparm", "armupper"),
        "forearm": ("lowerarm", "forearm", "elbow", "armlower"),
        "hand": ("hand", "wrist"),
        "thigh": ("upperleg", "upleg", "thigh", "hip", "legupper"),
        "shin": ("lowerleg", "leg", "calf", "shin", "knee", "leglower"),
        "foot": ("foot", "ankle"),
        "toe": ("toes", "toebase", "toe", "ball"),
    }
    for side, letter in (("left", "l"), ("right", "r")):
        for key, variants in limbs.items():
            aliases[letter + key] = [
                name
                for v in variants
                for name in (side + v, v + side, letter + v, v + letter)
            ]
        for digit, source_name in (
            ("Thumb", "thumb"),
            ("Index", "index"),
            ("Middle", "middle"),
            ("Ring", "ring"),
            ("Little", "pinky"),
        ):
            for number, segment in enumerate(("proximal", "intermediate", "distal"), 1):
                aliases[f"{letter}{digit}{number}"] = (
                    f"{source_name}0{number}{letter}",
                    f"{side}{digit.lower()}{segment}",
                    f"{side}hand{source_name}{number}",
                )
    lookup = {token: rig.data.bones[name] for name, token in names.items()}
    result = {}
    for joint, options in aliases.items():
        match = next((lookup[o] for o in options if o in lookup), None)
        if match:
            result[joint] = match
    required = {"hips", "head"} | {
        s + j
        for s in "lr"
        for j in ("upperarm", "forearm", "hand", "thigh", "shin", "foot")
    }
    missing = sorted(required - result.keys())
    if missing:
        raise ValueError(
            f"{rig.name}: unmapped humanoid joints {missing}; bones: {list(names)}"
        )
    return result


def skeleton_frame(joints, world=None):
    up = (joints["head"].head_local - joints["hips"].head_local).normalized()
    # Preserve an upright character's actual world vertical. A bind spine can
    # lean even when the armature object and ground plane are perfectly level.
    if world is not None:
        vertical = world.to_quaternion().inverted() @ Vector((0, 0, 1))
        if up.dot(vertical) > 0.7:
            up = vertical
    left = joints["lthigh"].head_local - joints["rthigh"].head_local
    if left.length < 1e-5:
        left = joints["lupperarm"].head_local - joints["rupperarm"].head_local
    forward = up.cross(left).normalized()
    if up.length < 0.9 or forward.length < 0.9:
        raise ValueError("Degenerate humanoid leg/torso rest directions")
    return Matrix((forward.cross(up), forward, up)).transposed().to_quaternion()


def leg_length(joints):
    return (
        sum(
            (joints[s + a].head_local - joints[s + b].head_local).length
            for s in "lr"
            for a, b in (("thigh", "shin"), ("shin", "foot"))
        )
        / 2
    )


def next_joint(joint, common):
    chains = [SPINE] + [
        tuple(side + key for key in chain)
        for side in "lr"
        for chain in (LIMBS[:4], LIMBS[4:])
    ]
    chains.extend(
        tuple(f"{side}{digit}{n}" for n in (1, 2, 3))
        for side in "lr"
        for digit in ("Thumb", "Index", "Middle", "Ring", "Little")
    )
    for chain in chains:
        if joint in chain:
            return next(
                (k for k in chain[chain.index(joint) + 1 :] if k in common), None
            )
    return None


def import_rig(path):
    before = set(bpy.data.objects)
    actions = set(bpy.data.actions)
    ext = Path(path).suffix.lower()
    if ext in {".gltf", ".glb"}:
        bpy.ops.import_scene.gltf(filepath=path)
    elif ext == ".fbx":
        bpy.ops.import_scene.fbx(filepath=path)
    elif ext == ".bvh":
        bpy.ops.import_anim.bvh(
            filepath=path,
            global_scale=0.01,
            use_fps_scale=True,
            rotate_mode="QUATERNION",
        )
    else:
        raise ValueError(f"Unsupported skeleton file: {path}")
    added = set(bpy.data.objects) - before
    rigs = [obj for obj in added if obj.type == "ARMATURE"]
    if len(rigs) != 1:
        raise ValueError(f"{path}: expected one armature, got {len(rigs)}")
    return rigs[0], added, set(bpy.data.actions) - actions


def set_source_action(rig, objects, actions, name):
    """Recover the glTF slot per object or the FBX/BVH skeletal action."""
    matches = []
    for obj in objects:
        if not (anim := obj.animation_data):
            continue
        for track in anim.nla_tracks:
            track.mute = True
        match = next(
            (
                track.strips[0]
                for track in anim.nla_tracks
                if track.name == name and len(track.strips)
            ),
            None,
        )
        if match:
            anim.action = match.action
            anim.action_slot = match.action_slot
            if obj == rig:
                matches.append(match.action)
        else:
            anim.action = None
    if matches:
        return matches[0]
    candidates = [
        act
        for act in actions
        if any(
            curve.data_path.startswith("pose.bones[")
            for layer in act.layers
            for strip in layer.strips
            for bag in strip.channelbags
            for curve in bag.fcurves
        )
    ]
    if name:
        selected = [
            act
            for act in candidates
            if re.sub(r"\.\d{3}$", "", act.name) == name
            or name in re.sub(r"\.\d{3}$", "", act.name).split("|")
        ]
        if selected:
            candidates = selected
        elif Path(rig.get("forge_source", "")).suffix.lower() != ".bvh":
            raise ValueError(
                f"Could not find action {name!r}; available: {[a.name for a in candidates]}"
            )
    if len(candidates) != 1:
        raise ValueError(
            f"Could not uniquely select action {name!r}: {[a.name for a in candidates]}"
        )
    action = candidates[0]
    anim = rig.animation_data_create()
    anim.action = action
    if len(action.slots) == 1:
        anim.action_slot = action.slots[0]
    return action


def add_curve(bag, path, column, times, samples):
    curve = bag.fcurves.new(data_path=path, index=column)
    keys = curve.keyframe_points
    keys.add(len(times))
    keys.foreach_set(
        "co", [v for t, point in zip(times, samples) for v in (t, point[column])]
    )
    linear = (
        bpy.types.Keyframe.bl_rna.properties["interpolation"].enum_items["LINEAR"].value
    )
    keys.foreach_set("interpolation", [linear] * len(times))
    curve.update()


def bake(source, target, action, job, static_world):
    scene = bpy.context.scene
    src, dst = joint_map(source), joint_map(target)
    common = dst.keys() & src.keys()
    body = (
        skeleton_frame(dst, target.matrix_world)
        @ skeleton_frame(src, static_world).inverted()
    )
    scale = leg_length(dst) / leg_length(src)
    # For each target segment, rotate its *anatomical* rest ray towards the
    # source rest ray. A bone's arbitrary local Y axis can be a poor proxy,
    # particularly in GLBs with independently chosen leaf directions.
    aligned = {}
    for key in common:
        child = next_joint(key, common)
        source_ray = (
            src[child].head_local - src[key].head_local
            if child
            else src[key].tail_local - src[key].head_local
        )
        target_ray = (
            dst[child].head_local - dst[key].head_local
            if child
            else dst[key].tail_local - dst[key].head_local
        )
        swing = Quaternion()
        if source_ray.length > 1e-6 and target_ray.length > 1e-6:
            swing = target_ray.rotation_difference(body @ source_ray)
        aligned[key] = (
            src[key].matrix_local.to_quaternion().inverted(),
            body.inverted() @ swing @ dst[key].matrix_local.to_quaternion(),
        )

    bones = sorted(target.data.bones, key=lambda bone: len(bone.parent_recursive))
    by_name = {dst[key].name: key for key in common}
    rest = {bone.name: bone.matrix_local.to_quaternion() for bone in bones}
    inverse_rest = {name: rot.inverted() for name, rot in rest.items()}
    keys = {name: [] for name in by_name}
    locations = {name: [] for name in by_name}
    scales = {name: [] for name in by_name}
    source_names = {src[key].name for key in common}
    # A mapped wrist can sit below an unmapped helper. Measure its animated
    # displacement relative to the entire chain back to the mapped ancestor.
    parent_offsets = {}
    for key in common - {"hips"}:
        ancestor = next(
            (b for b in src[key].parent_recursive if b.name in source_names), None
        )
        if ancestor:
            relative_head = ancestor.matrix_local.inverted() @ src[key].head_local
            parent_offsets[key] = (source.pose.bones[ancestor.name], relative_head)
    s0, s1 = action.frame_range
    duration = float(job["duration"])
    curves = [
        curve
        for layer in action.layers
        for strip in layer.strips
        for bag in strip.channelbags
        for curve in bag.fcurves
    ]
    is_bvh = Path(job["file"]).suffix.lower() == ".bvh"
    if is_bvh:
        # Blender's BVH importer uses one shared sample axis for every channel.
        curves = curves[:1]
        samples = curves[0].keyframe_points
        first = job.get("frame_start", 0)
        last = job.get("frame_end", len(samples) - 1)
        if not 0 <= first < last < len(samples):
            raise ValueError(f"{job['id']}: invalid BVH sample range {first}..{last}")
        s0, s1 = samples[first].co.x, samples[last].co.x
        sample_duration = (s1 - s0) / FPS
        if abs(duration - sample_duration) > max(0.002, sample_duration * 0.0001):
            raise ValueError(
                f"{job['id']}: catalog duration {duration}s differs from BVH samples ({sample_duration}s)"
            )
    if duration <= 0 or s1 <= s0:
        raise ValueError(f"{job['id']}: zero duration action")
    count = max(1, round(duration * 30))
    # Keep authored contact transitions, including 24 fps keys between bake samples.
    fractions = {i / count for i in range(count + 1)}
    for curve in curves:
        fractions.update(
            (point.co.x - s0) / (s1 - s0)
            for point in curve.keyframe_points
            if s0 < point.co.x < s1
        )
    fractions = sorted({round(f, 8) for f in fractions})
    times = [f * duration * FPS for f in fractions]
    hip0 = dst["hips"].head_local
    start_shift = Vector((0, 0, 0))
    for i, fraction in enumerate(fractions):
        frame = s0 + (s1 - s0) * fraction
        scene.frame_set(math.floor(frame), subframe=frame - math.floor(frame))
        # Includes animated armature-object/parent transforms in addition to joint motion.
        motion = static_world.inverted() @ source.matrix_world
        srot = motion.to_quaternion()
        src_hip = motion @ source.pose.bones[src["hips"].name].head
        wanted = {
            key: body
            @ srot
            @ source.pose.bones[src[key].name].matrix.to_quaternion()
            @ pre
            @ post
            for key, (pre, post) in aligned.items()
        }
        # Uniform stretch is authored on the ancestor root in KayKit's spawn
        # takes. Reproduce it through the mapped joints even on a rootless rig.
        stretch = {
            k: sum((motion @ source.pose.bones[src[k].name].matrix).to_scale()) / 3
            for k in common
        }
        shifts = {
            k: scale
            * (
                body
                @ (
                    motion.to_3x3()
                    @ (source.pose.bones[src[k].name].head - parent.matrix @ relative)
                )
            )
            for k, (parent, relative) in parent_offsets.items()
        }
        if i == 0 and is_bvh:
            # A mocap take can start arbitrarily far from its capture origin.
            start_shift = src_hip - src["hips"].head_local
            vert = skeleton_frame(src) @ Vector((0, 0, 1))
            height = vert.dot(start_shift)
            start_shift -= vert * height
            if abs(height) > leg_length(src) * 0.65:
                start_shift += vert * height
        wanted_hip = hip0 + scale * (
            body @ (src_hip - src["hips"].head_local - start_shift)
        )
        positions, rotations, world_scale = {}, {}, {}
        for bone in bones:
            name = bone.name
            key = by_name.get(name)
            if bone.parent:
                parent = bone.parent.name
                delta = rotations[parent] @ inverse_rest[parent]
                inherited = world_scale[parent]
                head = positions[parent] + inherited * (
                    delta @ (bone.head_local - bone.parent.head_local)
                )
                base = delta @ rest[name]
            else:
                head, base, inherited = bone.head_local.copy(), rest[name], 1.0
            rotations[name] = wanted.get(key, base)
            positions[name] = head
            world_scale[name] = stretch.get(key, inherited)
            if key:
                shift = (
                    wanted_hip - head
                    if key == "hips"
                    else shifts.get(key, Vector((0, 0, 0)))
                )
                positions[name] = head + shift
                local = (
                    base.inverted() @ shift / inherited
                    if inherited > 1e-7
                    else Vector((0, 0, 0))
                )
                locations[name].append(local)
                ratio = world_scale[name] / inherited if inherited > 1e-7 else 1.0
                scales[name].append((ratio, ratio, ratio))
                q = base.inverted() @ rotations[name]
                if keys[name]:
                    q.make_compatible(keys[name][-1])
                keys[name].append(q)
    result = bpy.data.actions.new(job["id"])
    slot = result.slots.new("OBJECT", target.name)
    bag = (
        result.layers.new("Retarget").strips.new(type="KEYFRAME").channelbags.new(slot)
    )
    # A source can lack clavicles or neck bones that another clip animates.
    # Explicit bind channels also make action switches deterministic in Blender.
    for bone in dst.values():
        name = bone.name
        pose = target.pose.bones[name]
        pose.rotation_mode = "QUATERNION"
        for prop, samples, neutral in [
            ("rotation_quaternion", keys.get(name), (1, 0, 0, 0)),
            ("location", locations.get(name), (0, 0, 0)),
            ("scale", scales.get(name), (1, 1, 1)),
        ]:
            ts = times
            if samples is None or all(tuple(sample) == neutral for sample in samples):
                samples, ts = [neutral, neutral], [times[0], times[-1]]
            for axis in range(len(neutral)):
                add_curve(bag, pose.path_from_id(prop), axis, ts, samples)
    return result, slot


def main():
    started = time.monotonic()
    parser = argparse.ArgumentParser()
    for option in ("mesh", "jobs", "output"):
        parser.add_argument("--" + option, required=True)
    parser.add_argument("--no-grounding", action="store_true")
    parser.add_argument("--native", action="store_true")
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    check_space(Path(args.output).parent)
    jobs = json.loads(Path(args.jobs).read_text())
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.fps, scene.render.fps_base = FPS, 1
    target, target_objects, existing_actions = import_rig(args.mesh)
    joint_map(target)
    if not any(
        obj.type == "MESH" and obj.find_armature() == target for obj in target_objects
    ):
        raise ValueError("Target has no mesh skinned to its humanoid armature")
    outputs = []
    if args.native:
        # These actions are already expressed in the target's bind frames, including the fingers.
        for job in jobs:
            action = set_source_action(
                target, target_objects, existing_actions, job["id"]
            )
            outputs.append((job, action, target.animation_data.action_slot))
    else:
        # The output contract embeds exactly the requested actions. Clearing
        # imported preview tracks leaves the skin, bind matrices and mesh intact.
        for obj in target_objects:
            obj.animation_data_clear()
            if obj.type == "MESH" and obj.data.shape_keys:
                obj.data.shape_keys.animation_data_clear()
        for action in existing_actions:
            bpy.data.actions.remove(action)
        for bone in target.pose.bones:
            bone.matrix_basis = Matrix.Identity(4)
        bpy.context.view_layer.update()
    soles = (
        None
        if args.no_grounding
        else SoleSampler(target, target_objects, joint_map(target))
    )
    source_files = () if args.native else dict.fromkeys(job["file"] for job in jobs)
    for file in source_files:
        check_space(Path(args.output).parent)
        source, objects, actions = import_rig(file)
        source["forge_source"] = file
        scene.frame_set(0)
        static_world = source.matrix_world.copy()
        for job in (j for j in jobs if j["file"] == file):
            for bone in source.pose.bones:
                bone.matrix_basis = Matrix.Identity(4)
            action = set_source_action(source, objects, actions, job.get("action"))
            result, slot = bake(source, target, action, job, static_world)
            outputs.append((job, result, slot))
        for obj in objects:
            bpy.data.objects.remove(obj, do_unlink=True)
        for act in actions:
            bpy.data.actions.remove(act)
        scene.render.fps, scene.render.fps_base = FPS, 1
    ad = target.animation_data_create()
    if soles is not None:
        for job, result, slot in outputs:
            ad.action = result
            ad.action_slot = slot
            ground_action(soles, result, slot, job, FPS, add_curve)
        ad.action = None
    for job, result, slot in outputs:
        name = job["id"]
        if ad.nla_tracks.get(name):
            ad.nla_tracks.remove(ad.nla_tracks[name])
        result.name = name
        track = ad.nla_tracks.new()
        track.name = name
        strip = track.strips.new(name, 0, result)
        strip.action_slot = slot
        track.mute = True
    scene.frame_set(0)
    bpy.ops.export_scene.gltf(
        filepath=args.output,
        export_format="GLB",
        export_animation_mode="ACTIONS",
        export_merge_animation="NLA_TRACK",
        export_anim_single_armature=False,
        export_frame_range=False,
        export_force_sampling=False,
        export_optimize_animation_size=False,
    )
    print("FORGE_BAKE_SECONDS", round(time.monotonic() - started, 3), flush=True)


if __name__ == "__main__":
    main()
