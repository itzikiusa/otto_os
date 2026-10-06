"""headmaster-otto.glb — RobotExpressive (CC0, Tomás Laulhé, mod. Don
McCurdy) as Headmaster Otto: recoloured, scaled to 1.6 m, a mortarboard + bow
tie, clips renamed per the contract, plus Inspect_Screen and Point_Board
authored here by layering world-space joint rotations over the robot's Idle.

Edited at the glTF level (lib/gltfpatch.py): the source clips key only a few
channels each, so a Blender import → export round trip re-bases every
unkeyed joint on Blender's rest pose and wrecks the clips.  The accessory
meshes are modelled in Blender and merged in.
"""

import math
import os
import tempfile

import bpy
import numpy as np

from . import common as C
from .dims import ROBOT_HEIGHT
from .gltfpatch import Doc, q_axis, q_between, q_inv, q_mul, q_rot

SRC = os.path.join(C.BASE, "robot-expressive", "RobotExpressive.glb")
RENAME = {"Idle": "Idle", "Walking": "Walk", "Yes": "Nod", "No": "Scold", "Wave": "Wave", "ThumbsUp": "ThumbsUp"}
ORDER = ["Idle", "Walk", "Nod", "Scold", "Wave", "ThumbsUp", "Inspect_Screen", "Point_Board"]
COLOURS = {  # sRGB, roughness, metallic
    "Main": ((0.94, 0.95, 0.97), 0.38, 0.05),   # pearl-white shell
    "Grey": ((0.22, 0.45, 0.82), 0.42, 0.15),   # Otto-blue joints
    "Black": ((0.06, 0.07, 0.09), 0.3, 0.0),
}


def _scale_root(doc):
    """RootNode scale so the standing robot is ROBOT_HEIGHT tall, feet at 0."""
    ys = []
    for i, n in enumerate(doc.j["nodes"]):
        if "mesh" in n and "skin" not in n:
            ys.append(doc.mesh_world_points(i)[:, 1])
    ys = np.concatenate(ys)
    root = doc.find("RootNode")
    s = ROBOT_HEIGHT / (ys.max() - ys.min())
    doc.j["nodes"][root]["scale"] = [s, s, s]
    return s


def _facing(doc):
    """Unit vector the face looks along (from the eye/visor material)."""
    head_mesh = doc.find("Head", joint=False)
    n = doc.j["nodes"][head_mesh]
    mats = [m.get("name") for m in doc.j["materials"]]
    pts_all, pts_face = [], []
    m = doc.world(head_mesh)
    for p in doc.j["meshes"][n["mesh"]]["primitives"]:
        pos = doc.acc(p["attributes"]["POSITION"])
        w = (m @ np.c_[pos, np.ones(len(pos))].T).T[:, :3]
        pts_all.append(w)
        if mats[p["material"]] == "Black":
            pts_face.append(w)
    c_all = np.concatenate(pts_all).mean(axis=0)
    c_face = np.concatenate(pts_face).mean(axis=0)
    d = c_face - c_all
    d[1] = 0
    return d / np.linalg.norm(d)


def _recolour(doc):
    for m in doc.j["materials"]:
        if m.get("name") in COLOURS:
            c, r, mt = COLOURS[m["name"]]
            pbr = m.setdefault("pbrMetallicRoughness", {})
            pbr["baseColorFactor"] = [C.srgb(x) for x in c] + [1.0]
            pbr["roughnessFactor"] = r
            pbr["metallicFactor"] = mt
        m.pop("extras", None)


def _accessories(doc, fwd):
    """Model a mortarboard and a bow tie in Blender at their rest world
    positions, export, and merge them under the Head / Torso joints."""
    head = doc.mesh_world_points(doc.find("Head", joint=False))
    torso = doc.mesh_world_points(doc.find("Torso", joint=False))
    top = float(head[:, 1].max())
    hc = [float(x) for x in (head.max(axis=0) + head.min(axis=0)) / 2]
    hw = float(head[:, 0].max() - head[:, 0].min())
    t_top = float(torso[:, 1].max())
    band = torso[torso[:, 1] > t_top - 0.06]
    front = float((band @ fwd).max())
    C.reset()
    s = hw / 0.62
    black = C.mat("Mortarboard", (0.10, 0.11, 0.14), rough=0.5)
    gold = C.mat("Tassel", (0.98, 0.76, 0.22), rough=0.35, metal=0.3)
    red = C.mat("BowTie", (0.88, 0.18, 0.22), rough=0.45)
    yaw = math.degrees(math.atan2(fwd[0], fwd[2]))
    cap = [
        C.cyl("cap_band", 0.17 * s, 0.10 * s, (hc[0], top - 0.025 * s, hc[2]), black, verts=24, bevel=0.01 * s),
        C.box("cap_board", (0.52 * s, 0.022 * s, 0.52 * s), (hc[0], top + 0.035 * s, hc[2]), black,
              bevel=0.006 * s, seg=2, rot=(0, 45 + yaw, 0)),
        C.sphere("cap_button", 0.028 * s, (hc[0], top + 0.05 * s, hc[2]), gold, seg=10, rings=6),
    ]
    side = np.cross([0, 1, 0], fwd)  # robot's left
    tx = np.array(hc) + side * 0.33 * s
    cap.append(C.cyl("tassel", 0.012 * s, 0.22 * s, (float(tx[0]), top - 0.07 * s, float(tx[2])), gold, verts=8))
    cap.append(C.sphere("tassel_knot", 0.022 * s, (float(tx[0]), top + 0.03 * s, float(tx[2])), gold, seg=8,
                        rings=5))
    C.join("Mortarboard", cap)
    tie_c = np.array([hc[0], t_top - 0.035 * s, 0.0]) + fwd * (front + 0.005 * s)
    tie_c[1] = t_top - 0.035 * s
    tie = []
    for sx in (-1, 1):
        p = tie_c + side * sx * 0.055 * s
        tie.append(C.box("wing", (0.09 * s, 0.075 * s, 0.03 * s), tuple(float(x) for x in p), red, bevel=0.012 * s, seg=2,
                         rot=(0, yaw, sx * 12)))
    tie.append(C.sphere("knot", 0.03 * s, tuple(float(x) for x in tie_c + fwd * 0.008 * s), red, seg=10, rings=6))
    C.join("BowTie", tie)
    tmp = tempfile.mkdtemp()
    out = {}
    for name in ("Mortarboard", "BowTie"):
        path = os.path.join(tmp, name + ".glb")
        C.export_glb(path, objects=[bpy.data.objects[name]])
        out[name] = path
    doc.merge_glb(out["Mortarboard"], doc.find("Head", joint=True), "Mortarboard")
    doc.merge_glb(out["BowTie"], doc.find("Torso", joint=True), "BowTie")


def _derive(doc, name, base, edits, fwd):
    """New clip `name` = `base` clip + world-space joint rotations.
    `edits(t)` → list of (joint name, ("rot", axis, deg) | ("aim", direction))
    applied in order (parents first)."""
    src = doc.anim(base)
    chans = src["channels"]
    samp = src["samplers"]
    inp = samp[0]["input"]
    times = doc.acc(inp)[:, 0]
    dur = times[-1]
    joint = {n: doc.find(n, joint=True) for n in
             ("Abdomen", "Torso", "Neck", "Head", "UpperArm.L", "UpperArm.R", "LowerArm.L", "LowerArm.R")}
    animated = {c["target"]["node"]: samp[c["sampler"]] for c in chans if c["target"]["path"] == "rotation"}
    touched = []
    for _t, ed in [(0, edits(0.0))]:
        for jn, _ in ed:
            if joint[jn] not in touched:
                touched.append(joint[jn])
    outs = {j: [] for j in touched}
    for t in times:
        rots = {k: doc.sample(s, t) for k, s in animated.items()}
        for jn, op in edits(t / dur if dur else 0.0):
            j = joint[jn]
            par = doc.parent[j]
            wp = doc.world_rot(par, rots)
            local = rots.get(j, np.array(doc.j["nodes"][j].get("rotation", (0, 0, 0, 1)), float))
            wj = q_mul(wp, local)
            if op[0] == "rot":
                R = q_axis(op[1], op[2])
            else:  # aim: point the joint's +Y (toward its child) along a world direction
                R = q_between(q_rot(wj, (0, 1, 0)), op[1])
            new_local = q_mul(q_inv(wp), q_mul(R, wj))
            rots[j] = new_local / np.linalg.norm(new_local)
        for j in touched:
            outs[j].append(rots[j])
    a = {"name": name, "channels": [], "samplers": []}
    for c in chans:
        if c["target"]["path"] == "rotation" and c["target"]["node"] in touched:
            continue
        a["samplers"].append(dict(samp[c["sampler"]]))
        a["channels"].append({"sampler": len(a["samplers"]) - 1, "target": dict(c["target"])})
    for j in touched:
        acc = doc.add_acc(np.array(outs[j]), "VEC4")
        a["samplers"].append({"input": inp, "output": acc, "interpolation": "LINEAR"})
        a["channels"].append({"sampler": len(a["samplers"]) - 1, "target": {"node": j, "path": "rotation"}})
    doc.j["animations"].append(a)


def build(out_path):
    doc = Doc(SRC)
    _scale_root(doc)
    measured = _facing(doc)
    # The contract says +Z; the visor measurement (≈ +Z, skewed by the rest
    # head turn) only guards against a source file that faces elsewhere.
    assert measured[2] > 0.9, measured
    fwd = np.array([0.0, 0.0, 1.0])
    up = np.array([0.0, 1.0, 0.0])
    lean = np.cross(up, fwd)        # +deg about this tips the head toward fwd
    right = np.cross(fwd, up)       # the robot's right
    _recolour(doc)
    _accessories(doc, fwd)
    TAU = 2 * math.pi

    def inspect(t):
        bob = math.sin(t * TAU)
        return [("Abdomen", ("rot", lean, 10)), ("Torso", ("rot", lean, 15 + 1.5 * bob)),
                ("Head", ("rot", lean, 12 + 3 * math.sin(t * 2 * TAU))), ("Head", ("rot", up, 8 * bob)),
                ("UpperArm.L", ("rot", lean, -20)), ("UpperArm.R", ("rot", lean, -20))]

    def point(t):
        w = math.sin(t * 2 * TAU)
        d = fwd * 0.62 + up * (0.66 + 0.04 * w) + right * 0.30
        return [("Torso", ("rot", up, -10)), ("UpperArm.R", ("aim", d)), ("LowerArm.R", ("aim", d + up * 0.08)),
                ("Head", ("rot", up, -16)), ("Head", ("rot", lean, -9))]

    _derive(doc, "Inspect_Screen", "Idle", inspect, fwd)
    _derive(doc, "Point_Board", "Idle", point, fwd)
    anims = {}
    for a in doc.j["animations"]:
        new = RENAME.get(a["name"], a["name"])
        if new in ORDER:
            a["name"] = new
            anims[new] = a
    doc.j["animations"] = [anims[n] for n in ORDER]
    doc.j["asset"] = {"version": "2.0", "generator": "Otto School build (from RobotExpressive, CC0)"}
    doc.save(out_path)
    return {"facing_measured": [round(float(x), 3) for x in measured]}
