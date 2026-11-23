"""A real model's parts for the checks: its world-space bounds box (the way
Otto's scene builder reads them: every primitive's POSITION min/max through
its node, a skinned mesh through its skeleton root's parent) and its rest
joints (hands, neck, head).

    python3 glbparts.py model.glb out.parts.json
"""
import json
import struct
import sys

I4 = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]


def glb_json(b):
    n = struct.unpack("<I", b[12:16])[0]
    return json.loads(b[20:20 + n])


def local(n):
    if "matrix" in n:
        return n["matrix"]
    tx, ty, tz = n.get("translation", [0, 0, 0])
    x, y, z, w = n.get("rotation", [0, 0, 0, 1])
    sx, sy, sz = n.get("scale", [1, 1, 1])
    return [(1 - 2 * (y * y + z * z)) * sx, (2 * (x * y + z * w)) * sx, (2 * (x * z - y * w)) * sx, 0,
            (2 * (x * y - z * w)) * sy, (1 - 2 * (x * x + z * z)) * sy, (2 * (y * z + x * w)) * sy, 0,
            (2 * (x * z + y * w)) * sz, (2 * (y * z - x * w)) * sz, (1 - 2 * (x * x + y * y)) * sz, 0, tx, ty, tz, 1]


def mul(a, b):
    m = [0.0] * 16
    for c in range(4):
        for r in range(4):
            m[c * 4 + r] = sum(a[k * 4 + r] * b[c * 4 + k] for k in range(4))
    return m


def apply(m, p):
    return [m[r] * p[0] + m[4 + r] * p[1] + m[8 + r] * p[2] + m[12 + r] for r in range(3)]


def parts(b):
    d = glb_json(b)
    nodes = d["nodes"]
    parent = {c: i for i, n in enumerate(nodes) for c in n.get("children", [])}
    roots = d["scenes"][d.get("scene", 0)]["nodes"]
    world, st = {}, [(r, I4) for r in roots]
    while st:
        i, u = st.pop()
        if i in world:
            continue
        world[i] = mul(u, local(nodes[i]))
        st += [(c, world[i]) for c in nodes[i].get("children", [])]
    lo, hi = [1e9] * 3, [-1e9] * 3
    for i, n in enumerate(nodes):
        if "mesh" not in n or i not in world:
            continue
        m = world[i]
        if "skin" in n:
            joints = d["skins"][n["skin"]]["joints"]
            root = d["skins"][n["skin"]].get("skeleton")
            if root is None:
                root = next(j for j in joints if parent.get(j) not in joints)
            m = world.get(parent[root], I4) if root in parent else I4
        for p in d["meshes"][n["mesh"]]["primitives"]:
            a = d["accessors"][p["attributes"]["POSITION"]]
            if "min" not in a:
                continue
            for k in range(8):
                c = apply(m, [a["max"][j] if (k >> j) & 1 else a["min"][j] for j in range(3)])
                lo = [min(lo[j], c[j]) for j in range(3)]
                hi = [max(hi[j], c[j]) for j in range(3)]
    joints = {}
    for i, n in enumerate(nodes):
        name = n.get("name", "")
        if i in world and name in ("LeftHand", "RightHand", "Neck", "Head"):
            joints[name] = [round(v, 5) for v in world[i][12:15]]
    return {"shape": "real", "parts": [{"min": lo, "max": hi, "part": "whole"}], "joints": joints}


if __name__ == "__main__":
    out = parts(open(sys.argv[1], "rb").read())
    open(sys.argv[2], "w").write(json.dumps(out))
