"""Blender-side core of forge-fit: rig, fingers, skin samples, closed prop proxies, anchors, grips.

Everything is measured on the evaluated (deformed) skin in world space. Lengths are scene units;
"s" converts metres to scene units from the character's declared height.
"""

import json
import math
import os
import re
import sys
from pathlib import Path

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Quaternion, Vector
from mathutils.bvhtree import BVHTree

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "animlib"))
from grounding import SoleSampler, ground_action  # noqa: E402,F401  (reused, not copied)
from retarget import add_curve, import_rig, joint_map, set_source_action  # noqa: E402,F401

FPS = 60
DIGITS = ("thumb", "index", "middle", "ring", "little")
# Flexion limits in degrees per joint (proximal, intermediate, distal): finger MCP/PIP/DIP and thumb CMC/MCP/IP.
LIMITS = {"thumb": (45, 55, 75), "finger": (90, 105, 80)}


def disabled(feature):
    """Test hook: FORGE_FIT_DISABLE=a,b switches features off so the tests can prove they detect it."""
    return feature in os.environ.get("FORGE_FIT_DISABLE", "").split(",")


def progress(pct, message):
    print("FIT_PROGRESS " + json.dumps({"pct": round(pct), "message": message}), flush=True)


def np3(v):
    return np.array(v[:3], dtype=float)


def unit(v):
    n = np.linalg.norm(v)
    return v / n if n > 1e-12 else v * 0


def finger_map(rig):
    """side ('l'/'r') -> digit -> [proximal, intermediate, distal] bone names (Unity or Mixamo names)."""
    out = {}
    for b in rig.data.bones:
        t = b.name.split(":")[-1].lower()
        t = re.sub(r"[^a-z0-9]", "", re.sub(r"^(?:mixamorig\d*|def)[_. -]*", "", t))
        m = re.fullmatch(
            r"(left|right)(?:hand)?(thumb|index|middle|ring|little|pinky)(proximal|intermediate|distal|1|2|3)", t
        )
        if m:
            digit = "little" if m[2] == "pinky" else m[2]
            j = {"proximal": 0, "1": 0, "intermediate": 1, "2": 1, "distal": 2, "3": 2}[m[3]]
            out.setdefault(m[1][0], {}).setdefault(digit, [None] * 3)[j] = b.name
    for side in out.values():
        for digit, chain in list(side.items()):
            if None in chain:
                del side[digit]
    return out


def remap_digits(rig, chains, hand):
    """Never trust finger names: label chains by geometry. The thumb root is nearest the wrist; the other
    roots are ordered along the knuckle line starting next to the thumb (index, middle, ring, little)."""
    if len(chains) != 5:
        return chains, {}
    root = {d: np.array(rig.data.bones[c[0]].head_local) for d, c in chains.items()}
    wrist = np.array(rig.data.bones[hand].head_local)
    thumb = min(root, key=lambda d: np.linalg.norm(root[d] - wrist))
    rest = [d for d in chains if d != thumb]
    a, b = max(((p, q) for p in rest for q in rest), key=lambda pq: np.linalg.norm(root[pq[0]] - root[pq[1]]))
    axis = unit(root[b] - root[a])
    if np.linalg.norm(root[a] - root[thumb]) > np.linalg.norm(root[b] - root[thumb]):
        axis = -axis
    order = sorted(rest, key=lambda d: (root[d] - root[thumb]) @ axis)
    new = {"thumb": chains[thumb]}
    new.update({name: chains[d] for name, d in zip(DIGITS[1:], order)})
    changed = {name: d for name, d in zip(DIGITS, [thumb] + order) if name != d}
    return new, changed


def matrix_np(m):
    return np.array([list(r) for r in m], dtype=float)


class Character:
    """One humanoid: its armature, skinned meshes, joints, fingers, scale and rest-pose skin."""

    def __init__(self, path, height_m=1.8, keep_animation=False):
        rig, objects, actions = import_rig(str(path))
        self.rig, self.objects, self.actions = rig, objects, actions
        for o in list(objects):
            if o.type == "MESH" and any(p.custom_shape == o for p in rig.pose.bones):
                objects.discard(o)
                bpy.data.objects.remove(o)
        self.meshes = [o for o in objects if o.type == "MESH" and o.find_armature() == rig and o.data.vertices]
        if not self.meshes:
            raise ValueError("Character has no mesh skinned to its armature")
        self.joints = joint_map(rig)
        self.fingers = finger_map(rig)
        self.remapped = {}
        for side in list(self.fingers):
            self.fingers[side], self.remapped[side] = remap_digits(rig, self.fingers[side], self.joints[side + "hand"].name)
        self.tracks = [t.name for t in rig.animation_data.nla_tracks] if rig.animation_data else []
        self.height_m = float(height_m)
        self.index()

    def index(self, keep_scale=False):
        """(Re)read the skinned meshes: rest skin, scale, dominant bones, triangles (after hands are replaced)."""
        rig = self.rig
        self.meshes = [o for o in self.objects if o.type == "MESH" and o.find_armature() == rig and o.data.vertices]
        self.__dict__.pop("_bary_key", None)
        self.rest_pose()
        self.bufs = [(o, np.empty(len(o.data.vertices) * 3, dtype=np.float32)) for o in self.meshes]
        self.rest = self.skin()
        if not keep_scale:
            self.floor = float(self.rest[:, 2].min())
            self.height = float(np.ptp(self.rest[:, 2]))
            self.s = self.height / self.height_m  # scene units per metre
        names, dom, groups, offset = [], [], [], 0
        self.tris = []
        for o in self.meshes:
            gnames = {g.index: g.name for g in o.vertex_groups}
            for v in o.data.vertices:
                best = max(v.groups, key=lambda g: g.weight, default=None)
                dom.append(gnames.get(best.group, "") if best else "")
                groups.append({gnames.get(g.group, ""): g.weight for g in v.groups if g.weight > 0.01})
            o.data.calc_loop_triangles()
            t = np.empty(len(o.data.loop_triangles) * 3, dtype=np.int64)
            o.data.loop_triangles.foreach_get("vertices", t)
            self.tris.append(t.reshape(-1, 3) + offset)
            offset += len(o.data.vertices)
        self.tris = np.concatenate(self.tris)
        self.dom = np.array(dom)
        self.weights = groups
        self.mw = matrix_np(rig.matrix_world)
        up = np.array([0.0, 0.0, 1.0])
        right = self.head("rthigh") - self.head("lthigh")
        right[2] = 0
        self.right = unit(right)
        self.up = up
        self.forward = unit(np.cross(up, self.right))

    # -- rig state -------------------------------------------------------------------------------
    def rest_pose(self):
        ad = self.rig.animation_data
        if ad:
            ad.action = None
            for t in ad.nla_tracks:
                t.mute = True
        for o in self.objects:
            if o.animation_data and o != self.rig:
                o.animation_data.action = None
                for t in o.animation_data.nla_tracks:
                    t.mute = True
        for p in self.rig.pose.bones:
            p.matrix_basis = Matrix.Identity(4)
            p.rotation_mode = "QUATERNION"
        bpy.context.view_layer.update()

    def deform(self, on):
        for o in self.meshes:
            for m in o.modifiers:
                if m.type == "ARMATURE":
                    m.show_viewport = on
        bpy.context.view_layer.update()

    def skin(self):
        return np.concatenate([SoleSampler.points(o, buf) for o, buf in self.bufs]).astype(float)

    def bone(self, name):
        return self.rig.pose.bones[name]

    def jname(self, key):
        return self.joints[key].name

    def head(self, key, posed=True):
        """World head of a joint key (retarget joint map) or bone name."""
        name = self.joints[key].name if key in self.joints else key
        b = self.rig.pose.bones[name]
        local = b.head if posed else b.bone.head_local
        return np3(self.rig.matrix_world @ local)

    def world(self, name):
        return matrix_np(self.rig.matrix_world @ self.rig.pose.bones[name].matrix)

    def side_names(self, side):
        k = {"l": "l", "r": "r", "left": "l", "right": "r"}[side]
        return k

    # -- hands ----------------------------------------------------------------------------------
    def hand_frame(self, side):
        """Wrist W, knuckle centre K, across a (little -> index), finger direction d, palm normal n."""
        side = self.side_names(side)
        f = self.fingers.get(side, {})
        if not all(d in f for d in DIGITS[1:]):
            raise ValueError(f"{'left' if side == 'l' else 'right'} hand: needs index..little finger chains")
        W = self.head(side + "hand")
        knuckles = {d: self.head(f[d][0]) for d in DIGITS[1:]}
        K = np.mean(list(knuckles.values()), axis=0)
        a = unit(knuckles["index"] - knuckles["little"])
        d = unit(K - W)
        d = unit(d - a * (d @ a))
        n = unit(np.cross(d, a)) * self.palm_sign(side)
        return {"W": W, "K": K, "a": a, "d": d, "n": n, "knuckles": knuckles,
                "width": float(np.linalg.norm(knuckles["index"] - knuckles["little"]))}

    def palm_sign(self, side):
        """Which side of the hand is the palm: the side the thumb tip is on, decided once in the rest pose."""
        cache = self.__dict__.setdefault("_palm_sign", {})
        if side not in cache:
            f = self.fingers[side]
            h = lambda b: np3(self.rig.matrix_world @ self.rig.data.bones[b].head_local)
            W = h(self.joints[side + "hand"].name)
            kn = {d: h(f[d][0]) for d in DIGITS[1:]}
            K = np.mean(list(kn.values()), axis=0)
            a = unit(kn["index"] - kn["little"])
            d = unit(K - W)
            n = unit(np.cross(unit(d - a * (d @ a)), a))
            if "thumb" in f:
                t = h(f["thumb"][2])
                tip = t + (t - h(f["thumb"][1]))
                ref = (tip - W) @ n
            else:
                ref = (np.r_[0, 0, self.rest[:, 2].mean()] - W) @ n
            cache[side] = 1.0 if ref >= 0 else -1.0
        return cache[side]

    def hand_vertices(self, side):
        side = self.side_names(side)
        names = {self.jname(side + "hand")} | {b for c in self.fingers.get(side, {}).values() for b in c}
        return np.flatnonzero(np.isin(self.dom, list(names)))

    def samples(self, coords, vids, spacing=None):
        """Surface samples: vertices plus a barycentric grid on every triangle whose corners are all in
        vids, dense enough that no sample is farther than ~spacing from another (rest-pose edge lengths)."""
        mask = np.zeros(len(coords), bool)
        mask[vids] = True
        tri = self.tris[mask[self.tris].all(axis=1)]
        key = (len(coords), len(tri))
        if getattr(self, "_bary_key", None) != (key, tuple(vids[:3]), len(vids)):
            spacing = spacing or 0.006 * self.height
            edge = np.linalg.norm(self.rest[tri] - self.rest[np.roll(tri, 1, axis=1)], axis=2).max(axis=1)
            k = np.clip(np.ceil(edge / spacing), 1, 8).astype(int)
            rows, owners = [], []
            for n in np.unique(k):
                ix = np.flatnonzero(k == n)
                bary = np.array([(i / n, j / n, 1 - (i + j) / n) for i in range(n + 1) for j in range(n + 1 - i)
                                 if 0 < i / n < 1 or 0 < j / n < 1 or n == 1])
                if n == 1:
                    bary = np.array([[1 / 3, 1 / 3, 1 / 3]])
                rows.append((ix, bary))
            self._bary = rows
            self._bary_key = (key, tuple(vids[:3]), len(vids))
        pts, owner = [coords[vids]], [self.dom[vids]]
        for ix, bary in self._bary:
            t = coords[tri[ix]]  # (m, 3, 3)
            pts.append(np.einsum("bk,mkc->mbc", bary, t).reshape(-1, 3))
            dom = self.dom[tri[ix]]
            nearest = np.argmax(bary, axis=1)
            owner.append(dom[:, nearest].reshape(-1))
        return np.concatenate(pts), np.concatenate(owner), tri

    def hand_patches(self, side, coords=None):
        """Independent skin patches: each digit's pad (distal segment), each finger segment, the palm."""
        side = self.side_names(side)
        coords = self.skin() if coords is None else coords
        hf = self.hand_frame(side)
        pts, owner, _ = self.samples(coords, self.hand_vertices(side))
        patches = {}
        for digit, chain in self.fingers.get(side, {}).items():
            for j, b in enumerate(chain):
                ix = np.flatnonzero(owner == b)
                if j == 2 and len(ix):
                    # pad: the palm-facing half of the distal segment
                    rel = (pts[ix] - hf["W"]) @ hf["n"]
                    pad = ix[rel >= np.median(rel)]
                    patches[digit + "_pad"] = pad
                patches[f"{digit}_{j}"] = ix
        hand = np.flatnonzero(owner == self.jname(side + "hand"))
        rel = (pts[hand] - hf["W"]) @ hf["n"]
        patches["palm"] = hand[rel >= np.percentile(rel, 60)] if len(hand) else hand
        return pts, owner, patches

    def local_axis(self, bone, axis_world):
        """A world axis, as seen now, in the bone's parent-carried rest frame (so it rotates with parents)."""
        pb = self.rig.pose.bones[bone]
        arm = self.rig.matrix_world.inverted().to_3x3() @ Vector(axis_world)
        frame = pb.bone.matrix_local if pb.parent is None else (
            pb.parent.matrix @ pb.parent.bone.matrix_local.inverted() @ pb.bone.matrix_local)
        return (frame.to_3x3().inverted() @ arm).normalized()

    def set_curl(self, bone, local_axis, degrees, base=None):
        """Rotation about a carried hinge axis (from local_axis), applied on top of a base rotation."""
        pb = self.rig.pose.bones[bone]
        pb.rotation_mode = "QUATERNION"
        q = Quaternion(Vector(local_axis), math.radians(degrees))
        pb.rotation_quaternion = q @ base if base is not None else q


def finger_axes(ch, side, digit, toward=None):
    """Hinge axes (world, at rest) so that a positive angle flexes towards the palm (or towards 'toward')."""
    side = ch.side_names(side)
    hf = ch.hand_frame(side)
    chain = ch.fingers[side][digit]
    heads = [ch.head(b) for b in chain]
    tip = heads[2] + (heads[2] - heads[1])
    pts = heads + [tip]
    axes = []
    for j in range(3):
        seg = unit(pts[j + 1] - pts[j])
        if digit == "thumb" and toward is not None:
            axis = unit(np.cross(seg, unit(toward - pts[j])))
        else:
            axis = unit(np.cross(seg, hf["n"]))
        axes.append(axis)
    return axes


# -- closed proxies -------------------------------------------------------------------------------
def hull(points):
    """Convex hull planes (normals, offsets) and triangles of a point set (Blender's own hull)."""
    bm = bmesh.new()
    for p in points:
        bm.verts.new(p)
    if len(bm.verts) < 4:
        bm.free()
        return None
    try:
        res = bmesh.ops.convex_hull(bm, input=bm.verts, use_existing_faces=False)
    except Exception:  # noqa: BLE001 - coplanar sets
        bm.free()
        return None
    dead = {g for g in res["geom_interior"] + res["geom_unused"] if isinstance(g, bmesh.types.BMVert)}
    bmesh.ops.delete(bm, geom=list(dead), context="VERTS")
    bmesh.ops.triangulate(bm, faces=bm.faces)
    bm.normal_update()
    if not bm.faces:
        bm.free()
        return None
    verts = np.array([v.co[:] for v in bm.verts])
    index = {v: i for i, v in enumerate(bm.verts)}
    faces = np.array([[index[v] for v in f.verts] for f in bm.faces])
    centre = verts.mean(axis=0)
    normals = np.cross(verts[faces[:, 1]] - verts[faces[:, 0]], verts[faces[:, 2]] - verts[faces[:, 0]])
    lengths = np.linalg.norm(normals, axis=1)
    keep = lengths > 1e-14
    normals, faces = normals[keep] / lengths[keep, None], faces[keep]
    offsets = np.einsum("ij,ij->i", normals, verts[faces[:, 0]])
    flip = normals @ centre - offsets > 0
    normals[flip] *= -1
    offsets[flip] *= -1
    bm.free()
    return {"n": normals, "d": offsets, "verts": verts, "faces": faces}


class Proxy:
    """Union of convex hulls in a local frame: a closed volume, so penetration is signed and meaningful."""

    def __init__(self, hulls):
        self.hulls = [h for h in hulls if h is not None]
        verts, faces, base = [], [], 0
        for h in self.hulls:
            verts.extend(h["verts"].tolist())
            faces.extend((h["faces"] + base).tolist())
            base += len(h["verts"])
        self.bvh = BVHTree.FromPolygons(verts, faces) if faces else None

    def signed(self, pts_world, M_world, exact_below=np.inf):
        """Signed distance: positive outside (true distance to the union), negative inside (depth).

        Outside points get the exact nearest-surface distance when their plane bound is below
        exact_below (local units); others keep the plane bound, a lower bound of the true distance.
        """
        if not len(pts_world) or self.bvh is None:
            return np.full(len(pts_world), np.inf)
        inv = np.linalg.inv(M_world)
        local = pts_world @ inv[:3, :3].T + inv[:3, 3]
        scale = np.cbrt(abs(np.linalg.det(M_world[:3, :3])))
        s = np.full(len(local), np.inf)
        for h in self.hulls:
            s = np.minimum(s, (local @ h["n"].T - h["d"]).max(axis=1))
        out = np.flatnonzero((s > 0) & (s < exact_below))
        for i in out:
            hit = self.bvh.find_nearest(Vector(local[i]))
            if hit[0] is not None:
                s[i] = hit[3]
        return s * scale


def slices(points, axis, origin, count=48, min_points=6):
    """Split points into slabs along an axis; returns (t0, t1, member indices) per non-empty slab."""
    t = (points - origin) @ axis
    edges = np.linspace(t.min(), t.max() + 1e-9, count + 1)
    out = []
    for i in range(count):
        ix = np.flatnonzero((t >= edges[i]) & (t < edges[i + 1]))
        if len(ix) >= min_points:
            out.append((edges[i], edges[i + 1], ix))
        elif out and len(ix):
            out[-1] = (out[-1][0], edges[i + 1], np.r_[out[-1][2], ix])
    return out, t


def proxy_from_points(points, axis, origin, count=48):
    pieces, _ = slices(points, axis, origin, count)
    # Overlap neighbouring slabs by sharing their boundary points so the union has no cracks.
    t = (points - origin) @ axis
    hulls = []
    for t0, t1, ix in pieces:
        pad = 0.15 * (t1 - t0)
        sel = np.flatnonzero((t >= t0 - pad) & (t <= t1 + pad))
        hulls.append(hull(points[sel]))
    return Proxy(hulls)


# -- props ----------------------------------------------------------------------------------------
class Prop:
    """A rigid prop: one joined mesh object, its principal frame, anchors and a closed proxy."""

    def __init__(self, path, name, cls="auto", length_m=None, s=1.0, anchors=None, keep_string=False):
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=str(path))
        added = [o for o in bpy.data.objects if o not in before]
        meshes = [o for o in added if o.type == "MESH" and o.data.vertices and not o.name.startswith("UCX_")
                  and not re.search(r"_LOD[1-9]$", o.name)]
        if not meshes:
            raise ValueError(f"{path}: no mesh in prop")
        if any(o.find_armature() for o in meshes):
            raise ValueError(f"{path}: prop must be a rigid (unskinned) mesh")
        for o in added:
            if o not in meshes:
                bpy.data.objects.remove(o)
        strings = [o for o in meshes if "string" in o.name.lower()]
        if cls == "bow" and not keep_string:
            for o in strings:  # never trust a generated string: the fit builds its own
                meshes.remove(o)
                bpy.data.objects.remove(o)
        for o in meshes:
            o.data = o.data.copy()
            o.data.transform(o.matrix_world)
            o.parent = None
            o.matrix_world = Matrix.Identity(4)
        bpy.ops.object.select_all(action="DESELECT")
        for o in meshes:
            o.select_set(True)
        bpy.context.view_layer.objects.active = meshes[0]
        if len(meshes) > 1:
            bpy.ops.object.join()
        obj = bpy.context.view_layer.objects.active
        obj.name = name
        obj.data.name = name
        pts = np.array([v.co[:] for v in obj.data.vertices])
        # Units: the file's metres unless a real length is given; then into the character's scene units.
        centre = pts.mean(axis=0)
        u, sv, vt = np.linalg.svd(pts - centre, full_matrices=False)
        span = float(np.ptp((pts - centre) @ vt[0]))
        k = (length_m / span if length_m else 1.0) * s
        pts = (pts - centre) * k
        self.name, self.cls, self.obj = name, cls, obj
        self.scale = k
        self.centre = centre
        self.removed_string = "separate-object" if strings and cls == "bow" and not keep_string else None
        dead = strip_fused_string(pts, vt) if cls == "bow" and not strings and not keep_string else np.zeros(len(pts), bool)
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bm.verts.ensure_lookup_table()
        for v, p in zip(bm.verts, pts):
            v.co = p
        if dead.any():
            self.removed_string = f"fused: {int(dead.sum())} vertices on the tip-to-tip line"
            bmesh.ops.delete(bm, geom=[bm.verts[i] for i in np.flatnonzero(dead)], context="VERTS")
        bm.to_mesh(obj.data)
        bm.free()
        obj.data.calc_loop_triangles()
        tris = np.array([t.vertices[:] for t in obj.data.loop_triangles])
        verts = np.array([v.co[:] for v in obj.data.vertices])
        self.points = surface_points(verts, tris, np.ptp(verts @ vt[0]) / 160)
        self.axes = vt  # rows: principal, second, third (local frame)
        self.anchors = find_anchors(self.points, vt, cls)
        for key, value in (anchors or {}).items():  # explicit override, prop-local metres
            if key.endswith("_axis"):
                self.anchors[key] = unit(np3(value))
            else:
                self.anchors[key] = (np3(value) - centre) * k
        self.proxy = proxy_from_points(self.points, self.anchors["axis"], np.zeros(3))
        self.length = float(np.ptp(self.points @ self.anchors["axis"]))

    def M(self):
        return matrix_np(self.obj.matrix_world)

    def world(self, key, M=None):
        M = self.M() if M is None else M
        v = self.anchors[key]
        if key.endswith("axis") or key.endswith("_dir"):
            return unit(M[:3, :3] @ v)
        return M[:3, :3] @ v + M[:3, 3]


def surface_points(verts, tris, spacing):
    """Vertices plus barycentric grid samples on every triangle (so long, low-poly faces are covered)."""
    edge = np.linalg.norm(verts[tris] - verts[np.roll(tris, 1, axis=1)], axis=2).max(axis=1)
    k = np.clip(np.ceil(edge / spacing), 1, 40).astype(int)
    out = [verts]
    for n in np.unique(k):
        ix = np.flatnonzero(k == n)
        bary = np.array([(i / n, j / n, 1 - (i + j) / n) for i in range(n + 1) for j in range(n + 1 - i)])
        out.append(np.einsum("bk,mkc->mbc", bary, verts[tris[ix]]).reshape(-1, 3))
    return np.concatenate(out)


def widths(points, axis, origin, count=64):
    """Cross-section width (2 x 90th percentile radius) per slab along the axis."""
    pieces, t = slices(points, axis, origin, count, min_points=3)
    rows = []
    for t0, t1, ix in pieces:
        p = points[ix]
        c = p.mean(axis=0)
        r = p - c
        r -= np.outer(r @ axis, axis)
        rows.append(((t0 + t1) / 2, t0, t1, 2 * float(np.percentile(np.linalg.norm(r, axis=1), 90)), c))
    return rows


def strip_fused_string(pts, vt):
    """Drop vertices on the straight tip-to-tip line of a bow (a modelled-in string), away from the tips."""
    axis = vt[0]
    t = pts @ axis
    lo, hi = pts[t.argmin()], pts[t.argmax()]
    line = unit(hi - lo)
    rel = pts - lo
    along = rel @ line
    dist = np.linalg.norm(rel - np.outer(along, line), axis=1)
    length = np.linalg.norm(hi - lo)
    thin = dist < 0.006 * length
    inner = (along > 0.08 * length) & (along < 0.92 * length)
    return thin & inner


def find_anchors(points, vt, cls):
    """Prop anchors from geometry. Positions are prop-local; *_axis are unit vectors."""
    axis = vt[0].copy()
    rows = widths(points, axis, np.zeros(3))
    t = np.array([r[0] for r in rows])
    w = np.array([r[3] for r in rows])
    A = {"axis": axis, "second_axis": vt[1].copy(), "third_axis": vt[2].copy()}
    if cls in ("sword", "auto"):
        guard = int(w.argmax())
        # The handle is the round side, the blade the flat one (ratio of the two cross-section extents);
        # only when both look alike does the shorter side count as the handle.
        tp = points @ axis
        def roundness(mask):
            p = points[mask]
            if len(p) < 10:
                return 0.0
            a, b = np.ptp(p @ vt[1]), np.ptp(p @ vt[2])
            return float(min(a, b) / max(a, b, 1e-12))
        gt = t[guard]
        span = np.ptp(t)
        low = roundness((tp < gt - 0.05 * span) & (tp > t[0] + 0.05 * span))
        high = roundness((tp > gt + 0.05 * span) & (tp < t[-1] - 0.05 * span))
        if abs(low - high) > 0.2:
            handle_low = low > high
        else:
            handle_low = (t[guard] - t[0]) < (t[-1] - t[guard])  # the short side is the handle
        ix = np.arange(guard) if handle_low else np.arange(guard + 1, len(t))
        if not len(ix):
            ix = np.arange(len(t))
        thin = w[ix] <= 1.35 * w[ix].min()
        runs = np.split(ix[thin], np.where(np.diff(ix[thin]) != 1)[0] + 1)
        run = max(runs, key=len)
        if len(run) < 0.4 * len(ix):
            # no distinct thin section (a generated grip as thick as its blade): use the handle side
            # without its last 12% (the pommel) as the grip
            run = ix[: max(1, int(len(ix) * 0.88))] if not handle_low else ix[int(len(ix) * 0.12):]
        g0, g1 = rows[run[0]][1], rows[run[-1]][2]
        toward_blade = axis if handle_low else -axis
        A.update({
            "grip": axis * (g0 + g1) / 2 + np.mean([rows[i][4] - axis * (rows[i][4] @ axis) for i in run], axis=0),
            "grip_axis": toward_blade, "grip_length": float(g1 - g0),
            "grip_radius": float(np.median(w[run]) / 2),
            "guard": axis * t[guard],
            "tip": axis * (t[-1] if handle_low else t[0]),
            "pommel": axis * (t[0] if handle_low else t[-1]),
            "roll_axis": vt[1].copy(),
        })
        A["guard_side"] = float((t[guard] - (g0 + g1) / 2) * (1 if handle_low else -1))
    elif cls == "baton":
        A.update({"grip": np.zeros(3), "grip_axis": axis, "grip_length": float(np.ptp(t)),
                  "grip_radius": float(np.median(w) / 2), "roll_axis": vt[1].copy(),
                  "end_a": axis * t.min(), "end_b": axis * t.max(), "guard_side": 0.0})
    elif cls == "bow":
        mid = np.abs(t) <= 0.09 * np.ptp(t)
        riser = points[np.abs(points @ axis) <= 0.09 * np.ptp(t)]
        depth = vt[1]
        tips = [points[(points @ axis).argmax()], points[(points @ axis).argmin()]]
        chord_mid = (tips[0] + tips[1]) / 2
        # depth axis points from the riser towards the string line (the archer's side)
        rc = riser.mean(axis=0)
        if (chord_mid - rc) @ depth < 0:
            depth = -depth
        belly = riser[(riser @ depth).argmax()]  # deepest point of the grip on the string side
        A.update({
            "grip": rc, "grip_axis": axis, "grip_length": float(2 * 0.09 * np.ptp(t)),
            "grip_radius": float(np.median(w[mid]) / 2) if mid.any() else float(w.min() / 2),
            "roll_axis": depth, "depth_axis": depth, "side_axis": unit(np.cross(axis, depth)),
            "upper_tip": tips[0], "lower_tip": tips[1], "nock_rest": chord_mid,
            "arrow_rest": rc + axis * (0.09 * np.ptp(t)) + depth * ((belly - rc) @ depth),
            "riser_half_thickness": float(np.ptp(riser @ unit(np.cross(axis, depth))) / 2),
            "brace": float((chord_mid - belly) @ depth), "guard_side": 0.0,
        })
    elif cls == "quiver":
        ends = [(points @ axis).argmax(), (points @ axis).argmin()]
        open_end = None
        for sign, e in ((1, ends[0]), (-1, ends[1])):
            cap = points[np.abs((points - points[e]) @ axis) < 0.03 * np.ptp(t)]
            c = cap.mean(axis=0)
            r = cap - c
            r -= np.outer(r @ axis, axis)
            rr = np.linalg.norm(r, axis=1)
            hollow = np.mean(rr < 0.4 * rr.max())
            if open_end is None or hollow < open_end[0]:
                open_end = (hollow, sign, c)
        sign = open_end[1]
        A.update({"opening": open_end[2], "opening_axis": axis * sign, "radius": float(np.median(w) / 2),
                  "bottom": -axis * sign * (np.ptp(t) / 2), "roll_axis": vt[1].copy()})
    elif cls == "arrow":
        # the head end has a short wide part; the fletched end a long one
        wide = w > 2.2 * np.median(w)
        lo, hi = int(wide[: len(w) // 2].sum()), int(wide[len(w) // 2:].sum())
        long_run = 0.06 * len(w)
        if lo and hi:
            sign = 1 if hi < lo else -1
        else:
            sign = 1 if (hi and hi <= long_run) or (lo > long_run) else -1
        A.update({"tip": axis * (t[-1] if sign > 0 else t[0]), "nock": axis * (t[0] if sign > 0 else t[-1]),
                  "shaft_axis": axis * sign, "grip_axis": axis * sign, "grip_radius": float(np.median(w) / 2),
                  "roll_axis": vt[1].copy()})
        A["grip"] = A["nock"] + A["shaft_axis"] * 0.06 * np.ptp(t)
        A["grip_length"] = float(0.08 * np.ptp(t))
        A["guard_side"] = 0.0
    else:
        raise ValueError(f"Unknown prop class {cls}")
    return A


def frame_from(z, x, origin):
    z = unit(np3(z))
    x = unit(np3(x) - z * (np3(x) @ z))
    y = np.cross(z, x)
    M = np.eye(4)
    M[:3, 0], M[:3, 1], M[:3, 2], M[:3, 3] = x, y, z, origin
    return M


def set_world(obj, M):
    obj.matrix_world = Matrix(M.tolist())


# -- grips ----------------------------------------------------------------------------------------
def grip_frame(prop, at="grip"):
    """Prop-local frame of a grip: z along the grip axis, x along the prop's roll axis."""
    A = prop.anchors
    z = A["grip_axis"] if at == "grip" else A.get(at + "_axis", A["grip_axis"])
    return frame_from(z, A["roll_axis"], A[at] if at in A else A["grip"])


def place_for_hand(ch, side, prop, kind="power", along=0.0, roll_to="d", reverse=False):
    """World matrix for the prop so its grip lies across the hand (before the fingers close).

    power: the grip axis along the knuckle line, in the curl hollow at the proximal phalanges, pressed
    onto the palm skin. hook: the grip axis across the middle phalanges (bowstring, arrow shaft).
    """
    hf = ch.hand_frame(side)
    f = ch.fingers[ch.side_names(side)]
    r = prop.anchors["grip_radius"]
    if kind == "power":
        base = hf["K"] + 0.22 * (hf["W"] - hf["K"])  # distal palm, just behind the knuckles
    else:
        base = np.mean([ch.head(f[d][1]) for d in ("index", "middle", "ring")], axis=0)
        base = base + 0.5 * (np.mean([ch.head(f[d][2]) for d in ("index", "middle", "ring")], axis=0) - base)
    x = {"d": hf["d"], "-d": -hf["d"], "-n": -hf["n"], "n": hf["n"]}[roll_to]
    G = grip_frame(prop)
    origin = base + hf["n"] * (r + 0.25 * hf["width"]) + hf["a"] * along
    target = frame_from(-hf["a"] if reverse else hf["a"], x, origin)
    return target @ np.linalg.inv(G)


def press(ch, prop, patch_pts_fn, direction, tol, max_travel):
    """Slide the prop along -direction until the patch skin touches it (stop at the first contact)."""
    M0 = prop.M()
    lo, hi = 0.0, max_travel
    def depth(t):
        M = M0.copy()
        M[:3, 3] -= direction * t
        return prop.proxy.signed(patch_pts_fn(), M, exact_below=tol + 1e-9).min()
    d0 = depth(0)
    if d0 < -tol:  # starts inside: back out to first clearance
        lo, hi = -max_travel, 0.0
        for _ in range(24):
            mid = (lo + hi) / 2
            if depth(mid) < -tol:
                hi = mid
            else:
                lo = mid
        M = M0.copy()
        M[:3, 3] -= direction * lo
        set_world(prop.obj, M)
        bpy.context.view_layer.update()
        return lo, depth(lo)
    if d0 <= tol:
        return 0.0, d0
    # march (the prop can pass right through a thin hand, so the gap is not monotonic), then bisect
    steps = np.linspace(0, max_travel, 61)
    hit = next((i for i in range(1, len(steps)) if depth(steps[i]) <= tol), None)
    if hit is None:
        lo = 0.0
    else:
        lo, hi = steps[hit - 1], steps[hit]
        for _ in range(20):
            mid = (lo + hi) / 2
            if depth(mid) > tol:
                lo = mid
            else:
                hi = mid
    M = M0.copy()
    M[:3, 3] -= direction * lo
    set_world(prop.obj, M)
    bpy.context.view_layer.update()
    return lo, depth(lo)


def close_hand(ch, side, prop, digits=DIGITS, tol=None, contact=None, step=3.0, joints=(0, 1, 2), limits=None, ready=True):
    """Curl each joint until the skin from that joint outwards touches the prop's closed proxy.

    Joints flex proximal to distal within anatomical limits; a joint stops at contact (no penetration
    beyond tol, bisected to 0.1 degree), its children keep wrapping. Samples that already start inside
    the proxy only count if they go deeper.
    """
    side = ch.side_names(side)
    tol = 0.0004 * ch.s if tol is None else tol
    M = prop.M()
    toward = prop.world("grip") if "grip" in prop.anchors else M[:3, 3]
    result = {}
    vids = ch.hand_vertices(side)
    for digit in sorted(digits, key=lambda d: d == "thumb"):  # fingers first, the thumb wraps over them
        if digit not in ch.fingers[side]:
            continue
        chain = ch.fingers[side][digit]
        if digit == "thumb" and ready:
            ready_thumb(ch, side, prop, M)
        bpy.context.view_layer.update()
        base_q = [ch.rig.pose.bones[b].rotation_quaternion.copy() for b in chain]
        axes = [ch.local_axis(b, ax) for b, ax in zip(chain, finger_axes(ch, side, digit, toward=toward))]
        lim = (limits or {}).get(digit) or LIMITS["thumb" if digit == "thumb" else "finger"]
        angles = [0.0, 0.0, 0.0]
        def worst(j):
            ch.deform(True)
            pts, owner, _ = ch.samples(ch.skin(), vids)
            ix = np.isin(owner, chain[j:])
            return prop.proxy.signed(pts[ix], M, exact_below=0)
        for j in joints:
            a = 0.0
            # A segment that starts inside the prop (e.g. a thumb lying across the palm) opens first.
            while (worst(j) < -tol).any() and a > -40:
                a -= step
                ch.set_curl(chain[j], axes[j], a, base_q[j])
                bpy.context.view_layer.update()
            base = np.minimum(worst(j), 0)
            def bad(a):
                ch.set_curl(chain[j], axes[j], a, base_q[j])
                bpy.context.view_layer.update()
                return ((worst(j) - base) < -tol).any()
            while a < lim[j]:
                b = min(lim[j], a + step)
                if bad(b):
                    lo, hi = a, b
                    while hi - lo > 0.1:
                        mid = (lo + hi) / 2
                        lo, hi = (lo, mid) if bad(mid) else (mid, hi)
                    a = lo
                    break
                a = b
            ch.set_curl(chain[j], axes[j], a, base_q[j])
            bpy.context.view_layer.update()
            angles[j] = round(a, 2)
        result[digit] = {"angles_deg": angles, "limits_deg": list(lim)}
    return result


def ready_thumb(ch, side, prop, M):
    """Before it closes, swing the thumb out of the palm (towards the palm normal, slightly to the index
    side), so it closes onto the far side of the grip instead of starting inside it or in the guard."""
    hf = ch.hand_frame(side)
    b = ch.fingers[side]["thumb"][0]
    pb = ch.rig.pose.bones[b]
    h0 = ch.head(b)
    tip = ch.head(ch.fingers[side]["thumb"][2])
    cur = unit(tip - h0)
    want = unit(-0.25 * hf["a"] + hf["n"] + 0.2 * hf["d"])  # out of the palm, over the far side, away from a guard
    q = Quaternion(Vector(np.cross(cur, want)).normalized() if np.linalg.norm(np.cross(cur, want)) > 1e-9 else Vector((1, 0, 0)),
                   math.acos(float(np.clip(cur @ want, -1, 1))))
    axis = ch.local_axis(b, np.array(q.axis))
    ch.set_curl(b, axis, math.degrees(q.angle), pb.rotation_quaternion.copy())
    bpy.context.view_layer.update()


def grip_report(ch, side, prop, contact):
    """Per patch: gap (+) or penetration (-) of the deformed skin against the closed prop proxy."""
    pts, owner, patches = ch.hand_patches(side)
    sd = prop.proxy.signed(pts, prop.M())
    out = {}
    for key, ix in patches.items():
        if len(ix):
            out[key] = {"gap": float(sd[ix].min()), "samples": int(len(ix)), "contact": bool(sd[ix].min() <= contact)}
    out["_max_penetration"] = float(max(0.0, -sd.min()))
    return out, sd


def curl_pose(ch, side, fraction, digits=DIGITS):
    """Free (no prop) flexion at a fraction of each joint's limit; thumb opposes towards the palm centre."""
    side = ch.side_names(side)
    hf = ch.hand_frame(side)
    centre = hf["K"] * 0.6 + hf["W"] * 0.4 + hf["n"] * hf["width"] * 0.5
    for digit in digits:
        if digit not in ch.fingers[side]:
            continue
        chain = ch.fingers[side][digit]
        axes = [ch.local_axis(b, ax) for b, ax in zip(chain, finger_axes(ch, side, digit, toward=centre))]
        lim = LIMITS["thumb" if digit == "thumb" else "finger"]
        for j, b in enumerate(chain):
            ch.set_curl(b, axes[j], fraction * lim[j])
    bpy.context.view_layer.update()


def key_pose(action, slot, rig, bones, frame_values):
    """Write rotation keys: frame_values = [(frame, {bone: Quaternion})]."""
    bag = action.layers[0].strips[0].channelbag(slot)
    for b in bones:
        pb = rig.pose.bones[b]
        path = pb.path_from_id("rotation_quaternion")
        times = [f for f, _ in frame_values]
        vals = [tuple(v[b]) for _, v in frame_values]
        for i in range(4):
            c = bag.fcurves.find(path, index=i)
            if c:
                bag.fcurves.remove(c)
            add_curve(bag, path, i, times, vals)


def new_action(rig, name):
    act = bpy.data.actions.new(name)
    slot = act.slots.new("OBJECT", rig.name)
    act.layers.new("Fit").strips.new(type="KEYFRAME").channelbags.new(slot)
    return act, slot


def push_track(obj, action, slot, name):
    ad = obj.animation_data_create()
    if ad.nla_tracks.get(name):
        ad.nla_tracks.remove(ad.nla_tracks[name])
    action.name = name
    track = ad.nla_tracks.new()
    track.name = name
    strip = track.strips.new(name, int(action.frame_range[0]), action)
    strip.action_slot = slot
    track.mute = True


def export(path):
    bpy.context.scene.frame_set(0)
    bpy.ops.export_scene.gltf(
        filepath=str(path), export_format="GLB", export_animation_mode="ACTIONS",
        export_merge_animation="NLA_TRACK", export_anim_single_armature=False, export_frame_range=False,
        export_force_sampling=False, export_optimize_animation_size=False, export_extras=True,
    )
