"""Template hands: when a generated hand fails intake, cut it off at the wrist and grow a clean procedural hand
along the character's own finger chains (Blender Skin + Subdivision modifiers: one connected, smooth, quad
mesh with five separate fingers), coloured from the original hand and weighted rigidly per segment.
"""

import math
from pathlib import Path

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

import fitlib as F

# radii in palm widths (index-to-little knuckle distance); cuff = how far the wrist reaches into the sleeve
STYLES = {
    "simple": {"finger": 0.13, "tip": 0.85, "thumb": 0.16, "palm": 0.17, "mid": 0.2, "wrist": 0.3, "cuff": 0.3, "metal": 0.0, "rough": 0.6},
    "gloved": {"finger": 0.15, "tip": 0.9, "thumb": 0.18, "palm": 0.19, "mid": 0.22, "wrist": 0.34, "cuff": 0.45, "metal": 0.5, "rough": 0.45},
    "chunky": {"finger": 0.18, "tip": 0.9, "thumb": 0.21, "palm": 0.21, "mid": 0.25, "wrist": 0.36, "cuff": 0.35, "metal": 0.0, "rough": 0.6},
}


def srgb_to_linear(c):
    c = np.asarray(c, float)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def base_colour_source(mat):
    if mat is None or not mat.use_nodes:
        return None, (0.7, 0.7, 0.7)
    bsdf = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
    if bsdf is None:
        return None, tuple(mat.diffuse_color[:3])
    inp = bsdf.inputs["Base Color"]
    if inp.is_linked and inp.links[0].from_node.type == "TEX_IMAGE":
        return inp.links[0].from_node.image, None
    return None, tuple(inp.default_value[:3])


def sample_colour(ch, names):
    """Median base colour (linear) of the faces about to be removed: texture at their UVs or material colour."""
    cols, metallic = [], []
    cache = {}
    for o in ch.meshes:
        gi = {g.index for g in o.vertex_groups if g.name in names}
        me = o.data
        w = np.array([sum(g.weight for g in v.groups if g.group in gi) for v in me.vertices])
        uv = me.uv_layers.active
        for poly in me.polygons:
            if w[list(poly.vertices)].mean() < 0.5:
                continue
            mat = o.material_slots[poly.material_index].material if o.material_slots else None
            img, col = base_colour_source(mat)
            if mat is not None and mat.use_nodes:
                b = next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None)
                if b is not None:
                    metallic.append(1.0 if b.inputs["Metallic"].is_linked else b.inputs["Metallic"].default_value)
            if img is None or uv is None:
                cols.append(col)
                continue
            if img.name not in cache:
                px = np.empty(img.size[0] * img.size[1] * 4, np.float32)
                img.pixels.foreach_get(px)
                # byte images in sRGB come back encoded; the BSDF colour wants linear
                cache[img.name] = (px.reshape(img.size[1], img.size[0], 4),
                                   img.colorspace_settings.name == "sRGB" and not img.is_float)
            px, encoded = cache[img.name]
            u, v = np.mean([uv.data[i].uv[:] for i in poly.loop_indices], axis=0)
            x = int(np.clip(u % 1.0, 0, 0.9999) * px.shape[1])
            y = int(np.clip(v % 1.0, 0, 0.9999) * px.shape[0])
            c = px[y, x, :3]
            cols.append(srgb_to_linear(c) if encoded else c)
    if not cols:
        return (0.6, 0.6, 0.6), 0.0
    return tuple(float(x) for x in np.median(np.array(cols), axis=0)), float(np.mean(metallic)) if metallic else 0.0


def cut_hand(ch, names):
    """Delete the original skin that mainly follows the hand and finger bones (the cut sits at the wrist)."""
    removed = 0
    for o in ch.meshes:
        gi = {g.index for g in o.vertex_groups if g.name in names}
        bm = bmesh.new()
        bm.from_mesh(o.data)
        deform = bm.verts.layers.deform.active
        dead = [v for v in bm.verts if deform and sum(w for g, w in v[deform].items() if g in gi) > 0.5]
        removed += len(dead)
        bmesh.ops.delete(bm, geom=dead, context="VERTS")
        bm.to_mesh(o.data)
        bm.free()
        o.data.update()
    return removed


def build_hand(ch, side, style, colour, metal, finger_colour=None):
    """Skin-modifier hand on the rest-pose joints of one side; returns the new skinned object."""
    st = STYLES[style]
    hf = ch.hand_frame(side)
    pw = hf["width"]
    j = ch.jname
    W = hf["W"]
    fore = F.unit(W - ch.head(side + "forearm"))
    f = ch.fingers[side]
    verts, radii, edges, segs, part = [], [], [], [], []
    current = [0]

    def add(p, r):
        verts.append(np.asarray(p, float))
        radii.append(r * pw)
        part.append(current[0])
        return len(verts) - 1

    back = add(W - fore * st["cuff"] * pw, st["wrist"] * (1.15 if st["cuff"] > 0.4 else 1.0))
    w = add(W, st["wrist"])
    edges.append((back, w))
    segs.append((j(side + "forearm"), verts[back], verts[w], None, 0))
    # palm: one broad branch per finger from the wrist (tubes that merge into a slab), not a single stem
    palm = {}
    for digit in F.DIGITS[1:]:
        K = ch.head(f[digit][0])
        p = add(W + 0.5 * (K - W), st["mid"])
        k = add(K, st["palm"])
        edges += [(w, p), (p, k)]
        segs.append((j(side + "hand"), verts[w], verts[p], j(side + "forearm"), 0))
        segs.append((j(side + "hand"), verts[p], verts[k], j(side + "forearm"), 0))
        palm[digit] = p
    for n, digit in enumerate(F.DIGITS):
        current[0] = n + 1  # each digit is its own shell: no skin web can join two fingers
        chain = f[digit]
        heads = [ch.head(b) for b in chain]
        tip = heads[2] + (heads[2] - heads[1]) * 0.85
        pts = heads + [tip]
        r0 = st["thumb"] if digit == "thumb" else st["finger"]
        root = w if digit == "thumb" else palm[digit]
        # the shell starts inside the palm, a little before the knuckle
        start = add(verts[root] + 0.6 * (heads[0] - verts[root]), r0 * 0.95)
        prev = start
        ids = []
        for k, p in enumerate(pts):
            r = r0 * (1 - (1 - st["tip"]) * k / 3)
            if k == 0 and digit != "thumb":
                r = max(r, st["palm"])  # knuckle
            i = add(p, r)
            edges.append((prev, i))
            ids.append(i)
            prev = i
        segs.append((j(side + "hand"), verts[start], verts[ids[0]], j(side + "forearm"), n + 1))
        for k in range(3):
            parent = chain[k - 1] if k else j(side + "hand")
            segs.append((chain[k], verts[ids[k]], verts[ids[k + 1]], parent, n + 1))
    shells = []
    for k in sorted(set(part)):
        ids = [i for i in range(len(verts)) if part[i] == k]
        remap = {i: n for n, i in enumerate(ids)}
        mesh = bpy.data.meshes.new(f"fit_hand_{side}_{k}")
        mesh.from_pydata([tuple(verts[i]) for i in ids], [(remap[a], remap[b]) for a, b in edges if a in remap and b in remap], [])
        sh = bpy.data.objects.new(mesh.name, mesh)
        bpy.context.collection.objects.link(sh)
        skin = sh.modifiers.new("Skin", "SKIN")
        skin.use_smooth_shade = True
        skin.branch_smoothing = 0.6
        for i in ids:
            sh.data.skin_vertices[0].data[remap[i]].radius = (radii[i], radii[i])
        sh.data.skin_vertices[0].data[0].use_root = True
        sub = sh.modifiers.new("Subdivision", "SUBSURF")
        sub.levels = sub.render_levels = 1
        bpy.ops.object.select_all(action="DESELECT")
        bpy.context.view_layer.objects.active = sh
        sh.select_set(True)
        for m in list(sh.modifiers):
            bpy.ops.object.modifier_apply(modifier=m.name)
        shells.append(sh)
    mats = []
    for k, c in ((0, colour), (1, finger_colour or colour)):
        mat = bpy.data.materials.new(f"fit_hand_{style}_{'palm' if k == 0 else 'fingers'}")
        mat.use_nodes = True
        b = mat.node_tree.nodes["Principled BSDF"]
        b.inputs["Base Color"].default_value = (*c, 1)
        b.inputs["Metallic"].default_value = max(st["metal"], min(metal, 0.8)) if k == 0 else st["metal"] * 0.5
        b.inputs["Roughness"].default_value = st["rough"]
        mat.diffuse_color = (*c, 1)
        mats.append(mat)
    for k, sh in zip(sorted(set(part)), shells):
        sh.data.materials.append(mats[0])
        sh.data.materials.append(mats[1])
        for p in sh.data.polygons:
            p.material_index = 0 if k == 0 else 1
    shell_sizes = [(k, len(sh.data.vertices)) for k, sh in zip(sorted(set(part)), shells)]
    bpy.ops.object.select_all(action="DESELECT")
    for sh in shells:
        sh.select_set(True)
    bpy.context.view_layer.objects.active = shells[0]
    bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    obj.name = f"fit_hand_{'L' if side == 'l' else 'R'}"
    for p in obj.data.polygons:
        p.use_smooth = True
    # mostly rigid weights: nearest bone segment OF THE SAME SHELL, blended with its parent only near the joint
    pts = np.array([v.co[:] for v in obj.data.vertices])
    shell_of = np.concatenate([np.full(n, k) for k, n in shell_sizes])
    best = np.full(len(pts), np.inf)
    owner = [None] * len(pts)
    for bone, a, bb, parent, k in segs:
        ab = bb - a
        u = np.clip(((pts - a) @ ab) / max(ab @ ab, 1e-12), 0, 1)
        d = np.linalg.norm(pts - (a + u[:, None] * ab), axis=1)
        d[shell_of != k] = np.inf
        closer = d < best - 1e-12
        best[closer] = d[closer]
        for i in np.flatnonzero(closer):
            owner[i] = (bone, parent, float(u[i]))
    groups = {}
    for i, (bone, parent, u) in enumerate(owner):
        # a joint belongs to the parent side until 8% into the child segment (keeps neighbours apart at the knuckles)
        wself = 1.0 if parent is None or u >= 0.2 else 0.3 + 3.5 * u
        groups.setdefault(bone, []).append((i, wself))
        if wself < 1.0:
            groups.setdefault(parent, []).append((i, 1.0 - wself))
    for bone, items in groups.items():
        vg = obj.vertex_groups.get(bone) or obj.vertex_groups.new(name=bone)
        for i, wv in items:
            vg.add([i], wv, "ADD")
    obj.parent = ch.rig
    obj.matrix_parent_inverse = ch.rig.matrix_world.inverted()
    obj.modifiers.new("Armature", "ARMATURE").object = ch.rig
    ch.objects.add(obj)
    return obj


def bone_frames(rig, fingers, hand, side_frame):
    """Rest frame (4x4, world) and length of the hand bone and every finger segment: origin at the joint,
    z along the segment, x along the palm normal; keyed (digit, index) with ("hand", 0) for the palm."""
    mw = F.matrix_np(rig.matrix_world)
    def head(b):
        return mw[:3, :3] @ np.array(rig.data.bones[b].head_local) + mw[:3, 3]
    n = side_frame["n"]
    out = {}
    W = head(hand)
    K = np.mean([head(fingers[d][0]) for d in F.DIGITS[1:]], axis=0)
    out[("hand", 0)] = (F.frame_from(K - W, n, W), (side_frame["width"], float(np.linalg.norm(K - W))))
    for d, chain in fingers.items():
        hs = [head(b) for b in chain]
        hs.append(hs[2] + (hs[2] - hs[1]))
        for k in range(3):
            seg = hs[k + 1] - hs[k]
            out[(d, k)] = (F.frame_from(seg, n, hs[k]), float(np.linalg.norm(seg)))
    return out


def attach_hand_glb(ch, side, path):
    """Pluggable template hand: a skinned hand GLB whose finger joints use Unity/Mixamo names. Each vertex is
    carried from the source hand's bone frames to the character's (per-bone similarity, blended by the source
    weights), so knuckles and finger joints land on the character's own pivots; groups are renamed."""
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(path))
    added = [o for o in bpy.data.objects if o not in before]
    rigs = [o for o in added if o.type == "ARMATURE"]
    if not rigs:
        raise ValueError(f"{path}: template hand has no armature")
    src = rigs[0]
    sf = F.finger_map(src)
    letter = side
    if letter not in sf:
        letter = next(iter(sf))  # a one-handed template: mirror by mapping onto this side
    sjoints = F.joint_map(src) if False else None
    hand_name = src.data.bones[sf[letter]["index"][0]].parent.name
    meshes = [o for o in added if o.type == "MESH" and o.find_armature() == src]
    names = {hand_name} | {b for c in sf[letter].values() for b in c}
    obj = max(meshes, key=lambda o: sum(1 for v in o.data.vertices
                                        if any(o.vertex_groups[g.group].name in names and g.weight > 0.5 for g in v.groups)))
    # source frame (palm normal from its thumb side), target frame from the character
    class Shim:
        pass
    sh = Shim()
    mw = F.matrix_np(src.matrix_world)
    h = lambda b: mw[:3, :3] @ np.array(src.data.bones[b].head_local) + mw[:3, 3]
    kn = {d: h(sf[letter][d][0]) for d in F.DIGITS[1:]}
    a = F.unit(kn["index"] - kn["little"])
    W = h(hand_name)
    dd = F.unit(np.mean(list(kn.values()), axis=0) - W)
    dd = F.unit(dd - a * (dd @ a))
    nn = F.unit(np.cross(dd, a))
    t = h(sf[letter]["thumb"][2])
    if (t - W) @ nn < 0:
        nn = -nn
    src_side = {"n": nn, "width": float(np.linalg.norm(kn["index"] - kn["little"]))}
    S = bone_frames(src, sf[letter], hand_name, src_side)
    T = bone_frames(ch.rig, ch.fingers[side], ch.jname(side + "hand"), ch.hand_frame(side))
    mats = {}
    kh = T[("hand", 0)][1][0] / max(S[("hand", 0)][1][0], 1e-12)  # palm-width ratio
    seg_k = {}
    for key, (Ms, ls) in S.items():
        if key[0] != "hand":
            # finger thickness follows the hand; length may differ, but not wildly (and the distal segment,
            # whose end is only an estimate on both rigs, takes its intermediate's scale)
            seg_k[key] = float(np.clip(T[key][1] / max(ls, 1e-12), 0.7 * kh, 1.4 * kh))
    for key, (Ms, ls) in S.items():
        Mt, lt = T[key]
        if key[0] == "hand":  # palm: width across, length along the palm
            kw, kl = lt[0] / max(ls[0], 1e-12), lt[1] / max(ls[1], 1e-12)
            sc = np.diag([kw, kw, kl, 1.0])
        else:
            k = seg_k[(key[0], 1)] if key[1] == 2 else seg_k[key]
            sc = np.diag([kh, kh, k, 1.0])
        mats[key] = Mt @ sc @ np.linalg.inv(Ms)
    name_of = {hand_name: ("hand", 0)}
    for d, chain in sf[letter].items():
        for k, b in enumerate(chain):
            name_of[b] = (d, k)
    # terminal/extra joints (finger tips, metacarpals): the distal segment of their digit, else the palm
    for g in {g.name for o in meshes for g in o.vertex_groups}:
        if g in name_of:
            continue
        low = g.lower()
        digit = next((d for d in F.DIGITS if d in low or (d == "little" and "pinky" in low)), None)
        name_of[g] = (digit, 2) if digit in sf[letter] else ("hand", 0)
    target_name = {("hand", 0): ch.jname(side + "hand")}
    for d, chain in ch.fingers[side].items():
        for k, b in enumerate(chain):
            target_name[(d, k)] = b
    me = obj.data
    Mo = F.matrix_np(obj.matrix_world)
    gname = {g.index: g.name for g in obj.vertex_groups}
    new = []
    for v in me.vertices:
        p = Mo[:3, :3] @ np.array(v.co[:]) + Mo[:3, 3]
        acc, wsum = np.zeros(3), 0.0
        for g in v.groups:
            key = name_of.get(gname[g.group], ("hand", 0))
            acc += g.weight * (mats[key][:3, :3] @ p + mats[key][:3, 3])
            wsum += g.weight
        new.append(acc / wsum if wsum > 0 else mats[("hand", 0)][:3, :3] @ p + mats[("hand", 0)][:3, 3])
    # a template that has to tear to fit (splayed target knuckles far from the source's) is refused
    old = np.array([Mo[:3, :3] @ np.array(v.co[:]) + Mo[:3, 3] for v in me.vertices]) * kh
    newa = np.array(new)
    eds = np.array([e.vertices[:] for e in me.edges])
    if len(eds):
        r = np.linalg.norm(newa[eds[:, 0]] - newa[eds[:, 1]], axis=1) / np.maximum(
            np.linalg.norm(old[eds[:, 0]] - old[eds[:, 1]], axis=1), 1e-9)
        tear = float(np.percentile(r, 99.5))
        if tear > 2.5 and not F.disabled("tear_check"):
            for o in added:
                bpy.data.objects.remove(o)
            raise ValueError(f"template would tear: 0.5% of its edges stretch over {tear:.1f}x on this skeleton")
    for v, p in zip(me.vertices, new):
        v.co = p
    obj.parent = None
    obj.matrix_world = Matrix.Identity(4)
    for m in list(obj.modifiers):
        obj.modifiers.remove(m)
    merged = {}
    for g in list(obj.vertex_groups):
        key = name_of.get(g.name, ("hand", 0))
        merged.setdefault(target_name[key], []).append(g)
    for tname, groups in merged.items():  # several source groups can land on one target bone: merge them
        if len(groups) == 1:
            groups[0].name = tname
            continue
        acc = {}
        for g in groups:
            for v in me.vertices:
                for e in v.groups:
                    if e.group == g.index:
                        acc[v.index] = acc.get(v.index, 0.0) + e.weight
        for g in groups:
            obj.vertex_groups.remove(g)
        ng = obj.vertex_groups.new(name=tname)
        for i, wv in acc.items():
            ng.add([i], min(1.0, wv), "REPLACE")
    for o in added:
        if o is not obj:
            bpy.data.objects.remove(o)
    obj.name = f"fit_hand_{'L' if side == 'l' else 'R'}"
    obj.parent = ch.rig
    obj.matrix_parent_inverse = ch.rig.matrix_world.inverted()
    obj.modifiers.new("Armature", "ARMATURE").object = ch.rig
    ch.objects.add(obj)
    return obj


LIBRARY = Path(__file__).resolve().parents[2] / "models" / "library" / "derived" / "characters"


def resolve_source(source, side):
    """None/"male"/"female" -> the library's wrist-cut hand for this side; a directory -> its left/right.glb;
    a file -> itself; "procedural" -> None."""
    if source == "procedural":
        return None
    if source in (None, "male", "female"):
        d = LIBRARY / (source or "male")
        return d / ("left.glb" if side == "l" else "right.glb") if d.is_dir() else None
    p = Path(source)
    if p.is_dir():
        return p / ("left.glb" if side == "l" else "right.glb")
    return p


def replace_hands(ch, sides=("l", "r"), style="simple", source=None):
    """source: template-hand GLB/dir/library name (library hands by default); procedural hand as fallback."""
    out = {}
    for side in sides:
        src = resolve_source(source, side)
        fingers = {b for c in ch.fingers[side].values() for b in c}
        names = {ch.jname(side + "hand")} | fingers
        colour, metal = sample_colour(ch, {ch.jname(side + "hand")})  # back of the hand / glove
        finger_colour, _ = sample_colour(ch, fingers)
        removed = cut_hand(ch, names)
        used = "procedural"
        obj = None
        if src is not None:
            try:
                obj = attach_hand_glb(ch, side, src)
                used = f"template GLB {src}"
            except Exception as exc:  # noqa: BLE001 - fall back to the procedural hand, and say why
                used = f"procedural (template GLB failed: {exc})"
        if obj is None:
            obj = build_hand(ch, side, style, colour, metal, finger_colour)
        out["left" if side == "l" else "right"] = {"source": used, "removed_vertices": removed, "template_vertices": len(obj.data.vertices),
                                                    "palm_colour_linear": colour, "finger_colour_linear": finger_colour,
                                                    "metallic_hint": metal, "style": style}
    ch.index(keep_scale=True)
    return out
