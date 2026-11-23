"""Synthetic skinned GLBs for the body-plan tests: any skeleton, a box around every bone, rigid weights.

    write(path, joints, tails=None, radius=0.03, twist=0.0)
joints: {name: (parent name or None, (x, y, z) head in metres)}, glTF Y-up, facing +Z.
tails: {leaf name: (x, y, z) end of the leaf's box} (default: 5 cm on along the parent bone).
twist: give every joint a different rest rotation (heads kept) to exercise local-space maths.
"""

import json
import struct
from pathlib import Path

import numpy as np


def _quat(axis, angle):
    axis = np.asarray(axis, float) / np.linalg.norm(axis)
    return np.r_[axis * np.sin(angle / 2), np.cos(angle / 2)]


def _qm(q):
    x, y, z, w = q
    return np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                     [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                     [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])


def _box(a, b, r):
    d = b - a
    length = np.linalg.norm(d)
    u = d / length if length > 1e-9 else np.array([0, 1.0, 0])
    p = np.cross(u, [1.0, 0, 0] if abs(u[0]) < 0.9 else [0, 1.0, 0])
    p /= np.linalg.norm(p)
    q = np.cross(u, p)
    corners = [a + s * r * p + t * r * q for s, t in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
    verts = np.array(corners + [c + d for c in corners])
    faces = [(0, 2, 1), (0, 3, 2), (4, 5, 6), (4, 6, 7)]
    for i in range(4):
        j = (i + 1) % 4
        faces += [(i, j, 4 + j), (i, 4 + j, 4 + i)]
    return verts, np.array(faces)


def write(path, joints, tails=None, radius=0.03, twist=0.0):
    tails = tails or {}
    names = list(joints)
    index = {n: i for i, n in enumerate(names)}
    heads = np.array([joints[n][1] for n in names], float)
    parent = [index[joints[n][0]] if joints[n][0] else -1 for n in names]
    children = [[c for c in range(len(names)) if parent[c] == k] for k in range(len(names))]
    rng = np.random.default_rng(3)
    rot = [_quat(rng.normal(size=3), twist * rng.uniform(-1, 1)) if twist else np.array([0, 0, 0, 1.0])
           for _ in names]
    world_r = [_qm(q) for q in rot]  # each joint's world rotation is its own twist (parents' undone)
    verts, faces, vj = [], [], []
    for k, n in enumerate(names):
        ends = [heads[c] for c in children[k]]
        if not ends:
            if n in tails:
                ends = [np.asarray(tails[n], float)]
            else:
                d = heads[k] - heads[parent[k]] if parent[k] >= 0 else np.array([0, 0.05, 0])
                ends = [heads[k] + 0.05 * d / max(np.linalg.norm(d), 1e-9)]
        for e in ends:
            v, f = _box(heads[k], e, radius)
            faces.append(f + sum(len(x) for x in verts))
            verts.append(v)
            vj += [k] * len(v)
    verts, faces = np.concatenate(verts), np.concatenate(faces)
    nodes = []
    for k, n in enumerate(names):
        p = parent[k]
        pw = world_r[p] if p >= 0 else np.eye(3)
        pos = heads[p] if p >= 0 else np.zeros(3)
        local_r = pw.T @ world_r[k]
        from forge.bodyplan import matrix_quat
        node = {"name": n, "translation": (pw.T @ (heads[k] - pos)).tolist(), "rotation": matrix_quat(local_r).tolist()}
        if children[k]:
            node["children"] = children[k]
        nodes.append(node)
    roots = [k for k in range(len(names)) if parent[k] < 0]
    mesh_node = len(nodes)
    nodes.append({"name": "body", "mesh": 0, "skin": 0})
    nodes.append({"name": "Armature", "children": roots + [mesh_node]})
    ibm = []
    for k in range(len(names)):
        m = np.eye(4)
        m[:3, :3] = world_r[k]
        m[:3, 3] = heads[k]
        ibm.append(np.linalg.inv(m).T.astype("<f4").ravel())
    blobs = [verts.astype("<f4").tobytes(), np.c_[np.array(vj), np.zeros((len(vj), 3), int)].astype("<u2").tobytes(),
             np.tile([1.0, 0, 0, 0], (len(vj), 1)).astype("<f4").tobytes(), faces.astype("<u4").ravel().tobytes(),
             np.concatenate(ibm).tobytes()]
    views, binary = [], b""
    for blob in blobs:
        views.append({"buffer": 0, "byteOffset": len(binary), "byteLength": len(blob)})
        binary += blob + b"\0" * (-len(blob) % 4)
    accessors = [
        {"bufferView": 0, "componentType": 5126, "count": len(verts), "type": "VEC3",
         "min": verts.min(axis=0).tolist(), "max": verts.max(axis=0).tolist()},
        {"bufferView": 1, "componentType": 5123, "count": len(verts), "type": "VEC4"},
        {"bufferView": 2, "componentType": 5126, "count": len(verts), "type": "VEC4"},
        {"bufferView": 3, "componentType": 5125, "count": faces.size, "type": "SCALAR"},
        {"bufferView": 4, "componentType": 5126, "count": len(names), "type": "MAT4"},
    ]
    doc = {"asset": {"version": "2.0", "generator": "forge tests/rig_any/synth.py"}, "scene": 0,
           "scenes": [{"nodes": [len(nodes) - 1]}], "nodes": nodes,
           "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}, "indices": 3}]}],
           "skins": [{"joints": list(range(len(names))), "inverseBindMatrices": 4}],
           "accessors": accessors, "bufferViews": views, "buffers": [{"byteLength": len(binary)}]}
    text = json.dumps(doc).encode()
    text += b" " * (-len(text) % 4)
    Path(path).write_bytes(struct.pack("<4sII", b"glTF", 2, 28 + len(text) + len(binary))
                           + struct.pack("<I4s", len(text), b"JSON") + text
                           + struct.pack("<I4s", len(binary), b"BIN\0") + binary)
    return Path(path)


def quadruped():
    j = {"pelvis": (None, (0, 0.50, -0.40)), "s1": ("pelvis", (0, 0.52, -0.15)), "s2": ("s1", (0, 0.55, 0.10)),
         "chest": ("s2", (0, 0.56, 0.30)), "neck": ("chest", (0, 0.66, 0.45)), "head": ("neck", (0, 0.76, 0.55)),
         "nose": ("head", (0, 0.72, 0.70)),
         "t1": ("pelvis", (0, 0.52, -0.55)), "t2": ("t1", (0, 0.56, -0.75)), "t3": ("t2", (0, 0.62, -0.92))}
    tails = {}
    for side, sx in (("L", 1), ("R", -1)):
        j[f"hip{side}"] = ("pelvis", (sx * 0.12, 0.45, -0.40))
        j[f"knee{side}"] = (f"hip{side}", (sx * 0.12, 0.28, -0.32))
        j[f"hock{side}"] = (f"knee{side}", (sx * 0.12, 0.12, -0.42))
        j[f"hpaw{side}"] = (f"hock{side}", (sx * 0.12, 0.04, -0.38))
        tails[f"hpaw{side}"] = (sx * 0.12, 0.04, -0.30)
        j[f"sh{side}"] = ("chest", (sx * 0.12, 0.48, 0.30))
        j[f"elb{side}"] = (f"sh{side}", (sx * 0.12, 0.30, 0.25))
        j[f"wri{side}"] = (f"elb{side}", (sx * 0.12, 0.10, 0.30))
        j[f"fpaw{side}"] = (f"wri{side}", (sx * 0.12, 0.04, 0.33))
        tails[f"fpaw{side}"] = (sx * 0.12, 0.04, 0.41)
        j[f"ear{side}"] = ("head", (sx * 0.05, 0.84, 0.55))
        j[f"eartip{side}"] = (f"ear{side}", (sx * 0.06, 0.94, 0.55))
    return j, tails


def humanoid():
    j = {"hips": (None, (0, 1.0, 0)), "spine": ("hips", (0, 1.15, 0)), "chest": ("spine", (0, 1.35, 0)),
         "neck": ("chest", (0, 1.52, 0)), "head": ("neck", (0, 1.62, 0)), "headend": ("head", (0, 1.78, 0))}
    tails = {}
    for side, sx in (("L", 1), ("R", -1)):
        j[f"sh{side}"] = ("chest", (sx * 0.06, 1.45, 0))
        j[f"ua{side}"] = (f"sh{side}", (sx * 0.18, 1.45, 0))
        j[f"fa{side}"] = (f"ua{side}", (sx * 0.45, 1.42, 0))
        j[f"hand{side}"] = (f"fa{side}", (sx * 0.70, 1.40, 0))
        j[f"th{side}"] = ("hips", (sx * 0.10, 0.95, 0))
        j[f"kn{side}"] = (f"th{side}", (sx * 0.10, 0.52, 0.03))
        j[f"an{side}"] = (f"kn{side}", (sx * 0.10, 0.09, 0))
        j[f"toe{side}"] = (f"an{side}", (sx * 0.10, 0.03, 0.12))
        tails[f"toe{side}"] = (sx * 0.10, 0.03, 0.18)
    return j, tails


def legs(n):
    """A body along +Z with n legs: pairs along the body, an odd one in front on the midline (or a stem for 1)."""
    if n == 1:
        j = {"root": (None, (0, 0.62, 0)), "cap": ("root", (0, 0.78, 0)), "top": ("cap", (0, 0.92, 0)),
             "l0": ("root", (0, 0.55, 0.0)), "l1": ("l0", (0, 0.32, 0.04)), "l2": ("l1", (0, 0.08, 0.0)),
             "l3": ("l2", (0, 0.03, 0.05))}
        return j, {"l3": (0, 0.03, 0.12)}
    if n == 2:  # a bird: legs under the pelvis, wings off the chest, a tail
        j = {"pelvis": (None, (0, 0.50, 0)), "chest": ("pelvis", (0, 0.56, 0.15)), "neck": ("chest", (0, 0.70, 0.20)),
             "head": ("neck", (0, 0.80, 0.22)), "beak": ("head", (0, 0.80, 0.32)),
             "tail1": ("pelvis", (0, 0.52, -0.15)), "tail2": ("tail1", (0, 0.50, -0.30))}
        tails = {}
        for side, sx in (("L", 1), ("R", -1)):
            j[f"w0{side}"] = ("chest", (sx * 0.08, 0.58, 0.12))
            j[f"w1{side}"] = (f"w0{side}", (sx * 0.20, 0.60, 0.05))
            j[f"w2{side}"] = (f"w1{side}", (sx * 0.30, 0.58, -0.05))
            j[f"th{side}"] = ("pelvis", (sx * 0.06, 0.45, 0))
            j[f"kn{side}"] = (f"th{side}", (sx * 0.06, 0.25, -0.04))
            j[f"an{side}"] = (f"kn{side}", (sx * 0.06, 0.05, 0.02))
            j[f"toe{side}"] = (f"an{side}", (sx * 0.06, 0.02, 0.08))
            tails[f"toe{side}"] = (sx * 0.06, 0.02, 0.14)
        return j, tails
    if n == 3:  # a stool: an upright body on three legs, one in front, two behind
        j = {"root": (None, (0, 0.60, 0)), "body": ("root", (0, 0.75, 0)), "head": ("body", (0, 0.90, 0))}
        tails = {}
        for i, th in enumerate((90, 210, 330)):
            c, s = np.cos(np.radians(th)), np.sin(np.radians(th))
            j[f"a{i}"] = ("root", (0.08 * c, 0.56, 0.08 * s))
            j[f"b{i}"] = (f"a{i}", (0.16 * c, 0.32, 0.16 * s))
            j[f"c{i}"] = (f"b{i}", (0.20 * c, 0.06, 0.20 * s))
            j[f"d{i}"] = (f"c{i}", (0.21 * c, 0.03, 0.21 * s))
            tails[f"d{i}"] = (0.24 * c, 0.03, 0.24 * s)
        return j, tails
    # 4, 6, 8: segments along the body; spider-like legs that arch up and out, then down
    pairs = n // 2
    j = {"thorax": (None, (0, 0.30, 0)), "head": ("thorax", (0, 0.31, 0.16)), "jaw": ("head", (0, 0.29, 0.26)),
         "abd1": ("thorax", (0, 0.32, -0.20)), "abd2": ("abd1", (0, 0.33, -0.40))}
    tails = {}
    prev = "thorax"
    for p in range(pairs):
        z = 0.08 - 0.16 * p / max(pairs - 1, 1)
        seg = f"seg{p}"
        j[seg] = (prev, (0, 0.30, z))
        prev = seg
        for side, sx in (("L", 1), ("R", -1)):
            zz = z * 2.5
            j[f"cx{p}{side}"] = (seg, (sx * 0.05, 0.30, z))
            j[f"fe{p}{side}"] = (f"cx{p}{side}", (sx * 0.22, 0.45, zz))
            j[f"ti{p}{side}"] = (f"fe{p}{side}", (sx * 0.42, 0.22, zz * 1.1))
            j[f"ta{p}{side}"] = (f"ti{p}{side}", (sx * 0.48, 0.03, zz * 1.15))
            tails[f"ta{p}{side}"] = (sx * 0.50, 0.03, zz * 1.15 + 0.05)
    return j, tails


def legless(kind="snake"):
    if kind == "snake":
        j, prev = {}, None
        for i in range(10):
            z = 0.5 - 0.11 * i
            name = f"s{i}"
            j[name] = (prev, (0.04 * np.sin(i * 0.8), 0.05, z))
            prev = name
        return j, {}
    if kind == "blob":
        return {"root": (None, (0, 0.20, 0)), "top": ("root", (0, 0.35, 0))}, {"top": (0, 0.45, 0)}
    raise ValueError(kind)
