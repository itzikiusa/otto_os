"""Blender worker for publisher character bases, shared-rig outfits and wrist-cut hands."""
import json
import math
from pathlib import Path
import sys

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "fit"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from characters_native import inverse_binds, pack_base
import fitlib as F
from motion import Motion


def clear():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def save(path):
    bpy.ops.export_scene.gltf(filepath=str(path), export_format="GLB", export_animations=False,
                              export_extras=True, export_all_influences=True)


def dress(ch, path, target_binds):
    """Convert the authored bind of every garment to this body, then share its rig."""
    garment, objects, _ = F.import_rig(path)
    target = ch.rig
    body_bones = target.data.bones
    part_bones = garment.data.bones
    if set(body_bones.keys()) != set(part_bones.keys()):
        raise ValueError(f"{Path(path).name}: skeleton joint names differ")
    # Use the publisher inverse binds directly. Blender's FBX/glTF importer
    # chooses its own local bone axes, so them matching by name isn't enough.
    source_binds = inverse_binds(Path(path))
    to_blender = Matrix(((1,0,0,0), (0,0,-1,0), (0,1,0,0), (0,0,0,1)))
    deltas = {}
    translation = rotation = 0.0
    for name, inv_source in source_binds.items():
        src, dst = inv_source.inverted(), target_binds[name].inverted()
        translation = max(translation, (src.translation - dst.translation).length)
        angle = math.degrees(src.to_quaternion().rotation_difference(dst.to_quaternion()).angle)
        rotation = max(rotation, min(angle, 360.0 - angle))
        deltas[name] = to_blender @ dst @ inv_source @ to_blender.inverted()
    meshes = sorted((o for o in objects if o.type == "MESH" and o.find_armature() == garment),
                    key=lambda o: o.name)
    if not meshes:
        raise ValueError(f"{Path(path).name}: no outfit meshes")
    for obj in meshes:
        modifier = next(m for m in obj.modifiers if m.type == "ARMATURE" and m.object == garment)
        bpy.context.view_layer.objects.active = obj
        if translation > 1e-5 or rotation > 0.001:
            # Bake the rest-space correction but retain every authored weight.
            inverse_world = obj.matrix_world.inverted()
            xforms = {g.index: inverse_world @ deltas[g.name] @ obj.matrix_world
                      for g in obj.vertex_groups if g.name in deltas}
            normals = [corner.vector.copy() for corner in obj.data.corner_normals]
            rotations = []
            for vertex in obj.data.vertices:
                co = vertex.co.copy()
                groups = [(g.weight, xforms[g.group]) for g in vertex.groups
                          if g.group in xforms and g.weight > 0]
                total = sum(w for w, _ in groups)
                if total:
                    blend = sum((m * (w / total) for w, m in groups), (Matrix.Identity(4) * 0.0))
                    vertex.co = blend @ co
                    rotations.append(blend.to_3x3().inverted_safe().transposed())
                else:
                    rotations.append(Matrix.Identity(3))
            corrected = [(rotations[loop.vertex_index] @ n).normalized()
                         for loop, n in zip(obj.data.loops, normals)]
            obj.data.normals_split_custom_set(corrected)
        world = obj.matrix_world.copy()
        obj.parent = target
        obj.matrix_world = world
        modifier.object = target
    for obj in objects - set(meshes):
        bpy.data.objects.remove(obj, do_unlink=True)
    return {"source": Path(path).name, "meshes": [o.name for o in meshes],
            "max_bind_joint_distance_m": translation, "max_bind_orientation_deg": rotation}


def hide_covered_skin(ch, outfits):
    """Outfit author supplies replacement limbs and asks for only the head under a full outfit."""
    if not outfits:
        return 0
    slots = {}
    for report in outfits:
        for name in report["meshes"]:
            key = name.split("_")[2].casefold()
            if key not in ("body", "arms", "legs", "feet"):
                continue
            obj = bpy.data.objects[name]
            co = np.array([obj.matrix_world @ v.co for v in obj.data.vertices])
            slots.setdefault(key, []).append(co)
    if not slots:
        return 0
    spans = {key: np.concatenate(coords) for key, coords in slots.items()}
    full = len(slots) == 4
    chest_x = ch.head("neck")[0]
    body = max(ch.meshes, key=lambda o: len(o.data.vertices))
    bm = bmesh.new()
    bm.from_mesh(body.data)
    deform = bm.verts.layers.deform.active
    names = {g.index: g.name for g in body.vertex_groups}
    # The two publisher bases are both in T pose. Keep the weighted neck above the collar;
    # cut only the regions actually being dressed when individual parts are selected.
    limits = {}
    for key, points in spans.items():
        x = np.abs(points[:, 0] - chest_x)
        limits[key] = (x.min(), x.max(), points[:, 2].min(), points[:, 2].max())
    remove = []
    for f in bm.faces:
        neck_weight = sum(w for v in f.verts for group, w in v[deform].items()
                          if names[group] in ("Neck", "Head")) / len(f.verts)
        if neck_weight > 0.22:
            continue
        x, _, z = body.matrix_world @ f.calc_center_median()
        x = abs(x - chest_x)
        if full:
            remove.append(f)
            continue
        for key, (xmin, xmax, zmin, zmax) in limits.items():
            if key == "arms":
                covered = x > xmin + .008 and zmin - .05 < z < zmax + .05
            elif key == "feet":
                covered = z < zmax - .012
            elif key == "legs":
                covered = zmin + .012 < z < zmax - .025 and x < xmax + .015
            else:
                covered = zmin + .015 < z < zmax - .012 and x < xmax - .008
            if covered:
                remove.append(f)
                break
    removed = len(remove)
    bmesh.ops.delete(bm, geom=remove, context="FACES")
    bm.to_mesh(body.data)
    bm.free()
    return removed


def sockets(ch):
    motion = Motion(ch, {}, {}, {}, ch.height)
    frames = motion.build_sockets()
    parents = {key: value[0] for key, value in motion.sockets.items()}
    for side, name in (("l", "left_hand"), ("r", "right_hand")):
        h = ch.hand_frame(side)
        frames[name] = F.frame_from(h["a"], h["d"], h["K"] + 0.22 * (h["W"] - h["K"])).tolist()
        parents[name] = ch.jname(side + "hand")
    for name, m in frames.items():
        empty = bpy.data.objects.new(name, None)
        bpy.context.scene.collection.objects.link(empty)
        empty.parent = ch.rig
        empty.parent_type = "BONE"
        empty.parent_bone = parents[name]
        bpy.context.view_layer.update()
        empty.matrix_world = Matrix(m)
        empty["forge_socket"] = name
        empty.empty_display_size = 0.02
    bpy.context.view_layer.update()
    return parents, frames


def make_hand(base, side, path):
    clear()
    ch = F.Character(path=base)
    frame = ch.hand_frame(side)
    hand = ch.joints[side + "hand"]
    rig = ch.rig
    wrist_name = hand.name
    wrist = rig.matrix_world @ hand.head_local
    elbow = rig.matrix_world @ ch.joints[side + "forearm"].head_local
    normal = (wrist - elbow).normalized()
    keep = {hand.name} | {b.name for b in hand.children_recursive}
    eligible = keep | {ch.jname(side + "forearm")}
    def has_finger_weights(obj):
        group = obj.vertex_groups.get(ch.fingers[side]["index"][0])
        return group is not None and any(g.group == group.index and g.weight > 0.01
                                         for v in obj.data.vertices for g in v.groups)
    bodies = [o for o in ch.meshes if has_finger_weights(o)]
    if len(bodies) != 1:
        raise ValueError(f"Expected one hand-bearing mesh, found {len(bodies)}")
    body = bodies[0]
    for o in list(bpy.data.objects):
        if o not in (rig, body):
            bpy.data.objects.remove(o, do_unlink=True)
    bpy.context.view_layer.objects.active = body
    bm = bmesh.new()
    bm.from_mesh(body.data)
    deform = bm.verts.layers.deform.active
    names = {g.index: g.name for g in body.vertex_groups}
    def dominant(v):
        w = v[deform]
        weights = list(w.items())
        return names[max(weights, key=lambda pair: pair[1])[0]] if weights else None
    # Restrict the cut to the hand and forearm first. In an A-pose the same
    # infinite plane can also intersect the leg on the other side of the body.
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if dominant(v) not in eligible], context="VERTS")
    world = body.matrix_world
    plane_co = world.inverted() @ wrist
    plane_no = (world.to_3x3().transposed() @ normal).normalized()
    bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
                          dist=1e-7, plane_co=plane_co, plane_no=plane_no, clear_inner=True)
    loose = [v for v in bm.verts if not v.link_faces]
    bmesh.ops.delete(bm, geom=loose, context="VERTS")
    border = {v for f in bm.edges if f.is_boundary for v in f.verts
              if abs((world @ v.co - wrist).dot(normal)) < 2e-5}
    if len(border) < 3:
        raise ValueError("Wrist plane produced no attachment edge")
    # Blender can split one geometric border vertex at a UV seam. Store the
    # weldable ring by position, independent of the exporter's vertex indices.
    tang = normal.orthogonal().normalized()
    bitang = normal.cross(tang).normalized()
    rim = {tuple(round(float(x), 6) for x in world @ v.co) for v in border}
    def polar(xyz):
        offset = Vector(xyz) - wrist
        return math.atan2(offset.dot(bitang), offset.dot(tang))
    ring_gltf = [[x, z, -y] for x, y, z in sorted(rim, key=polar)]
    wrist_group = body.vertex_groups[hand.name].index
    for v in bm.verts:
        w = v[deform]
        collapsed = sum(val for idx, val in w.items() if names[idx] not in keep)
        for idx in list(w.keys()):
            if names[idx] not in keep:
                del w[idx]
        if collapsed:
            w[wrist_group] = w.get(wrist_group, 0.) + collapsed
        norm = sum(w.values())
        if norm <= 0:
            raise ValueError("Unweighted hand vertex")
        for idx in list(w.keys()):
            w[idx] /= norm
    report = {"wrist_joint": hand.name, "finger_joints": ch.fingers[side],
              "wrist_world_blender_m": list(wrist), "outward_axis_blender": list(normal),
              "palm_width_m": frame["width"], "cut_ring_vertices": len(border), "wrist_ring_gltf_m": ring_gltf,
              "vertices": len(bm.verts), "triangles": sum(len(f.verts)-2 for f in bm.faces)}
    bm.to_mesh(body.data)
    bm.free()
    for group in list(body.vertex_groups):
        if group.name not in keep:
            body.vertex_groups.remove(group)
    for obj in bpy.context.selected_objects:
        obj.select_set(False)
    rig.select_set(True)
    bpy.context.view_layer.objects.active = rig
    bpy.ops.object.mode_set(mode="EDIT")
    for b in list(rig.data.edit_bones):
        if b.name not in keep:
            rig.data.edit_bones.remove(b)
    bpy.ops.object.mode_set(mode="OBJECT")
    body.name = "Template_" + wrist_name
    body["wrist_joint"] = wrist_name
    body["wrist_ring_gltf_m"] = [v for pos in ring_gltf for v in pos]
    body["wrist_center_gltf_m"] = [wrist.x, wrist.z, -wrist.y]
    body["wrist_outward_gltf"] = [normal.x, normal.z, -normal.y]
    save(path)
    return report


def main():
    jobfile = Path(sys.argv[sys.argv.index("--")+1])
    job = json.loads(jobfile.read_text())
    work = jobfile.parent
    clear()
    ch = F.Character(job["sources"][0])
    target_binds = inverse_binds(Path(job["sources"][0]))
    reports = [dress(ch, path, target_binds) for path in job["sources"][1:]]
    parents, frames = sockets(ch)
    result = {"canonical_joints": 52, "skin_joints": len(ch.rig.data.bones), "outfits": reports,
              "sockets": parents, "height_m": ch.height}
    result["hidden_base_faces_removed"] = hide_covered_skin(ch, reports)
    base = work / "character.glb"
    if reports:
        save(base)
    else:
        pack_base(Path(job["sources"][0]), Path(job["source_dir"]), base, parents, frames)
    if job["templates"]:
        if reports:
            raise ValueError("Extract wrist templates from a bare base")
        result["templates"] = {name: make_hand(base, k, work / f"{name}.glb")
                               for k, name in (("l", "left"), ("r", "right"))}
    (work / "result.json").write_text(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
