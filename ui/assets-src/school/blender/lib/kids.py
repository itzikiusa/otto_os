"""kid-<provider>.glb — Quaternius "Ultimate Animated Character" hoodie teen
(CC0) turned into a kid: bigger head, scaled to ≈1.05 m, hoodie recoloured
per provider, chest decal, a different head per provider (heads from the same
pack, bound with their own inverse bind matrices), and all 11 contract clips
authored on the 62-joint rig with lib/posekit.py (IK for seated hands/feet).

Edited at the glTF level: the source clips key a subset of joints, so a
Blender round trip would rebase the rest and break them (see headmaster.py).
"""

import copy
import math
import os

import numpy as np

from . import common as C
from .dims import KB_Y, KB_Z, KID_HEIGHT, SEAT_Y, WS_Z
from .gltfpatch import Doc
from .posekit import Rig, ik2, lerp_pose, sample_frames, write_clip

QDIR = os.path.join(C.BASE, "quaternius")
BASE = "hoodie.glb"
A = "CharacterArmature|"

# provider: (head donor file|None, head mesh, hoodie sRGB, pants sRGB, skin sRGB, decal)
KIDS = {
    "claude": ("woman1.glb", "Casual_Head", (0.80, 0.40, 0.26), (0.20, 0.24, 0.36), (0.98, 0.80, 0.68), "claude"),
    "codex": ("casual.glb", "Casual2_Head", (0.22, 0.64, 0.40), (0.30, 0.33, 0.40), (0.62, 0.42, 0.30), "codex"),
    "grok": (None, "Casual_Head", (0.15, 0.15, 0.17), (0.36, 0.44, 0.62), (0.93, 0.74, 0.60), "grok"),
    "agy": ("woman2.glb", "Formad_Head", (0.23, 0.46, 0.88), (0.85, 0.85, 0.82), (0.96, 0.78, 0.66), "agy"),
    "shell": ("adventurer.glb", "Adventurer_Head", (0.52, 0.54, 0.58), (0.22, 0.26, 0.34), (0.80, 0.58, 0.42), "shell"),
    "custom": ("punk.glb", "Punk_Head", (0.20, 0.60, 0.60), (0.24, 0.26, 0.30), (0.96, 0.82, 0.70), None),
}
HEAD_SCALE = 1.6
CLIPS = ["Sit_Type", "Sit_Idle", "Sit_RaiseHand", "Sit_Slump", "Stand_Up", "Sit_Down", "Walk", "Idle_Stand",
         "Talk", "Look_Around", "Wave"]

X = np.array([1.0, 0, 0])   # the kid's left
Y = np.array([0, 1.0, 0])
Z = np.array([0, 0, 1.0])   # the kid's front


def _mat_by_name(doc, name, start=0):
    for k, m in enumerate(doc.j["materials"]):
        if k >= start and m.get("name") == name:
            return k
    return None


def _set_col(doc, k, srgb, rough=0.7, metal=0.0):
    m = doc.j["materials"][k]
    pbr = m.setdefault("pbrMetallicRoughness", {})
    pbr["baseColorFactor"] = [C.srgb(x) for x in srgb] + [1.0]
    pbr["roughnessFactor"] = rough
    pbr["metallicFactor"] = metal
    m.pop("extras", None)


def _split_material(doc, mesh_name, mat_name, new_name):
    """Give `mesh_name`'s primitives using `mat_name` their own copy."""
    k = _mat_by_name(doc, mat_name)
    doc.j["materials"].append(dict(copy.deepcopy(doc.j["materials"][k]), name=new_name))
    nk = len(doc.j["materials"]) - 1
    for m in doc.j["meshes"]:
        if m["name"] == mesh_name:
            for p in m["primitives"]:
                if p.get("material") == k:
                    p["material"] = nk
    return nk


def _restyle(doc, provider):
    donor, head_mesh, hoodie, pants, skin, _ = KIDS[provider]
    j = doc.j
    # Shoes keep a neutral accent instead of following the hoodie colour.
    shoe = _split_material(doc, "Casual_Feet", "Purple", "ShoeAccent")
    _set_col(doc, shoe, (0.92, 0.92, 0.94), 0.6)
    _set_col(doc, _mat_by_name(doc, "Purple"), hoodie, 0.8)
    j["materials"][_mat_by_name(doc, "Purple")]["name"] = "Hoodie"
    _set_col(doc, _mat_by_name(doc, "LightBlue"), pants, 0.75)
    j["materials"][_mat_by_name(doc, "LightBlue")]["name"] = "Pants"
    first_head_mat = len(j["materials"])
    if donor:
        src = Doc(os.path.join(QDIR, donor))
        old = next(i for i, n in enumerate(j["nodes"]) if "mesh" in n and
                   j["meshes"][n["mesh"]]["name"] == "Casual_Head")
        parent = doc.parent[old]
        joints = j["skins"][j["nodes"][old]["skin"]]["joints"]
        doc.drop_node(old)
        doc.merge_skinned(src, head_mesh, joints, parent, "KidHead")
    else:
        hn = next(i for i, n in enumerate(j["nodes"]) if "mesh" in n and
                  j["meshes"][n["mesh"]]["name"] == "Casual_Head")
        j["nodes"][hn]["name"] = "KidHead"
        first_head_mat = 0
    for k, m in enumerate(j["materials"]):
        if m.get("name", "").startswith("Skin") and (k >= first_head_mat or m["name"] == "Skin"):
            shade = 0.86 if m["name"] == "Skin_Darker" else 1.0
            _set_col(doc, k, tuple(c * shade for c in skin), 0.65)
    for n in j["nodes"]:
        if "mesh" in n and n.get("name", "").startswith("Casual_"):
            n["name"] = "Kid" + n["name"][len("Casual_"):]


def _decal(doc, rig, provider, P_rest):
    """Chest decal: a quad rigidly parented to the Chest joint."""
    name = KIDS[provider][5]
    if not name:
        return
    j = doc.j
    body = [i for i, n in enumerate(j["nodes"]) if n.get("name") == "KidBody"]
    pts = rig.skin_points(P_rest, body)
    W = rig.world(P_rest)
    chest = W[rig.j("Chest")][:3, 3]
    band = pts[(np.abs(pts[:, 0] - chest[0]) < 0.06) & (np.abs(pts[:, 1] - (chest[1] - 0.02)) < 0.05)]
    front = band[:, 2].max()
    width = np.ptp(pts[np.abs(pts[:, 1] - chest[1]) < 0.04][:, 0]) * 0.42
    cy = chest[1] - 0.03
    hw = width / 2
    z = front + 0.004
    pos = np.array([[-hw, cy - hw, z], [hw, cy - hw, z], [hw, cy + hw, z], [-hw, cy + hw, z]])
    # express in Chest-joint local space (node matrix = inverse chest world at rest)
    inv = np.linalg.inv(W[rig.j("Chest")])
    nrm = np.tile([0.0, 0.0, 1.0], (4, 1))
    uv = np.array([[0, 1], [1, 1], [1, 0], [0, 0]], float)  # glTF v down; upright image
    with open(os.path.join(C.TEX, f"decal_{name}.png"), "rb") as f:
        tex = doc.add_image(f.read(), name=f"decal_{name}")
    j["materials"].append({"name": "ChestDecal", "alphaMode": "MASK", "alphaCutoff": 0.5, "doubleSided": False,
                           "pbrMetallicRoughness": {"baseColorTexture": {"index": tex}, "metallicFactor": 0.0,
                                                    "roughnessFactor": 0.7}})
    prim = {"attributes": {"POSITION": doc.add_acc(pos, "VEC3", target=34962, minmax=True),
                           "NORMAL": doc.add_acc(nrm, "VEC3", target=34962),
                           "TEXCOORD_0": doc.add_acc(uv, "VEC2", target=34962)},
            "indices": doc.add_acc(np.array([[0], [1], [2], [0], [2], [3]], np.uint32), "SCALAR", 5125,
                                   target=34963),
            "material": len(j["materials"]) - 1}
    j["meshes"].append({"name": "ChestDecal", "primitives": [prim]})
    j["nodes"].append({"name": "ChestDecal", "mesh": len(j["meshes"]) - 1,
                       "matrix": [float(x) for x in inv.T.ravel()]})
    ni = len(j["nodes"]) - 1
    j["nodes"][rig.j("Chest")].setdefault("children", []).append(ni)
    doc.parent[ni] = rig.j("Chest")


# ---------------------------------------------------------------- choreography

class Kid:
    def __init__(self, rig, mesh_nodes):
        self.r = rig
        self.mesh = mesh_nodes
        self.dur = {c: rig.duration(A + c) for c in ("Idle", "Walk", "Wave", "Interact")}

    def base(self, clip, t):
        return self.r.pose_at(A + clip, (t % 1.0) * self.dur[clip])

    def seat(self, P, lean=6.0, arms="type", t=0.0, hand_dy=(0.0, 0.0), raise_wave=0.0, slump=0.0,
             head=(0.0, 0.0), lift=None):
        r = self.r
        # 1. hips over the seat (lift computed once from the skinned mesh)
        hips = r.pos(P, "Hips")
        target = np.array([0.0, self.hip_y if lift is None else lift, 0.03])
        r.move(P, "Body", target - hips)
        # 2. legs: thighs forward, shins down, feet dangling a little forward
        for side, sx in (("L", 1), ("R", -1)):
            hp = r.pos(P, f"UpperLeg.{side}")
            foot = hp + np.array([sx * 0.03, -0.0, 0.0]) + Z * (self.thigh * 0.97) + \
                -Y * (self.shin * 0.95) + Z * 0.06
            ik2(r, P, f"UpperLeg.{side}", f"LowerLeg.{side}", f"Foot.{side}", foot, Z * 1.0 + Y * 0.3)
            # Foot.* are IK-target joints parented to Root (not to the shin):
            # carry them along explicitly.
            r.move(P, f"Foot.{side}", foot - r.pos(P, f"Foot.{side}"))
        # 3. spine lean
        r.rot(P, "Abdomen", X, lean * 0.5 + slump * 0.4)
        r.rot(P, "Chest", X, lean * 0.5 + slump * 0.6)
        # 4. arms
        if arms in ("type", "raise"):
            for side, sx, dy in (("L", 1, hand_dy[0]), ("R", -1, hand_dy[1])):
                if arms == "raise" and side == "R":
                    continue
                # wrist just short of the keyboard centre: fingers on the keys
                tgt = np.array([sx * 0.10, KB_Y + 0.03 + dy, WS_Z + KB_Z - 0.04])
                ik2(r, P, f"UpperArm.{side}", f"LowerArm.{side}", f"Wrist.{side}", tgt,
                    -Y * 0.6 + X * sx * 0.5 - Z * 0.4)
                self._palm_down(P, side)
        if arms == "raise":
            sh = r.pos(P, "UpperArm.R")
            up = sh + Y * (self.arm * 0.98) - X * (0.10 + 0.05 * raise_wave) + Z * 0.04
            ik2(r, P, "UpperArm.R", "LowerArm.R", "Wrist.R", up, -X * 1.0 - Z * 0.3)
            r.rot(P, "Wrist.R", Z, -15 * raise_wave)
        if arms == "lap":
            for side, sx in (("L", 1), ("R", -1)):
                hp = r.pos(P, f"UpperLeg.{side}")
                tgt = hp + Z * 0.16 + Y * 0.05 + X * sx * 0.02
                ik2(r, P, f"UpperArm.{side}", f"LowerArm.{side}", f"Wrist.{side}", tgt,
                    -Y * 0.5 + X * sx * 0.7 - Z * 0.3)
        if arms == "slump":
            for side, sx in (("L", 1), ("R", -1)):
                hp = r.pos(P, f"UpperLeg.{side}")
                tgt = hp + Z * 0.06 - Y * 0.02 + X * sx * 0.12
                ik2(r, P, f"UpperArm.{side}", f"LowerArm.{side}", f"Wrist.{side}", tgt,
                    -Y * 0.2 + X * sx * 1.0 - Z * 0.2)
        # 5. head
        r.rot(P, "Neck", X, head[0] * 0.5 + slump * 0.6)
        r.rot(P, "Head", X, head[0] * 0.5 + slump * 0.6)
        r.rot(P, "Head", Y, head[1])
        return P

    def _palm_down(self, P, side):
        """Roll the wrist so the back of the hand faces up."""
        r = self.r
        W = r.world(P)
        wr = W[r.j(f"Wrist.{side}")]
        # the hand's palm normal ≈ local −Z or +Z depending on rig; pick the
        # local axis most aligned with world −Y after a test roll.
        fwd = W[r.j(f"Middle1.{side}")][:3, 3] - wr[:3, 3]
        fwd = fwd / np.linalg.norm(fwd)
        thumb = W[r.j(f"Thumb1.{side}")][:3, 3] - wr[:3, 3]
        side_v = thumb - fwd * np.dot(thumb, fwd)
        side_v /= np.linalg.norm(side_v)
        want = np.array([-1.0 if side == "L" else 1.0, 0, 0])  # thumbs point inward
        want = want - fwd * np.dot(want, fwd)
        want /= np.linalg.norm(want)
        ang = math.degrees(math.atan2(np.dot(np.cross(side_v, want), fwd), np.dot(side_v, want)))
        r.rot(P, f"Wrist.{side}", fwd, ang)

    def calibrate(self):
        r = self.r
        P = self.base("Idle", 0.0)
        W = r.world(P)
        L = lambda a, b: np.linalg.norm(W[r.j(a)][:3, 3] - W[r.j(b)][:3, 3])  # noqa: E731
        self.thigh = L("UpperLeg.L", "LowerLeg.L")
        self.shin = L("LowerLeg.L", "Foot.L")
        self.arm = L("UpperArm.L", "LowerArm.L") + L("LowerArm.L", "Wrist.L")
        # seat contact: put the hips at y, skin, and shift so the lowest
        # buttock vertex rests 1 cm into the seat top.
        # (the thigh mesh deforms with hip height, so iterate to a fixed point)
        self.hip_y = 0.5
        for _ in range(6):
            P = self.seat(self.base("Idle", 0.0), arms="lap")
            pts = r.skin_points(P, self.mesh)
            hips = r.pos(P, "Hips")
            under = pts[(np.abs(pts[:, 0]) < 0.12) & (np.abs(pts[:, 2] - hips[2]) < 0.10)]
            err = (SEAT_Y - 0.01) - under[:, 1].min()
            self.hip_y += err
            if abs(err) < 0.002:
                break

    def build(self):
        r = self.r
        self.calibrate()
        TAU = 2 * math.pi
        out = {}

        def clip(name, dur, fn):
            frames, d = sample_frames(fn, dur)
            out[name] = write_clip(r, name, frames, d)

        def sit_type(t):
            a = math.sin(t * TAU * 6)
            b = math.sin(t * TAU * 6 + 1.9)
            P = self.base("Idle", t)
            return self.seat(P, lean=12 + 1.0 * math.sin(t * TAU * 2), arms="type",
                             hand_dy=(0.012 * max(a, 0), 0.012 * max(b, 0)),
                             head=(8 + 2 * math.sin(t * TAU * 4), 6 * math.sin(t * TAU)))
        clip("Sit_Type", 2.0, sit_type)

        def sit_idle(t):
            lean = -5 * max(0.0, math.sin(t * TAU)) if t < 0.5 else 0.0
            P = self.base("Idle", t)
            return self.seat(P, lean=lean + 1.5 * math.sin(t * TAU * 2), arms="lap",
                             head=(2 * math.sin(t * TAU * 2 + 1), 10 * math.sin(t * TAU)))
        clip("Sit_Idle", 4.0, sit_idle)

        def sit_raise(t):
            w = math.sin(t * TAU * 2)
            P = self.base("Idle", t)
            return self.seat(P, lean=3, arms="raise", raise_wave=w, head=(-6, -6))
        clip("Sit_RaiseHand", 1.6, sit_raise)

        def sit_slump(t):
            br = math.sin(t * TAU)
            P = self.base("Idle", t)
            return self.seat(P, lean=10, arms="slump", slump=22 + 2 * br, head=(10, 0))
        clip("Sit_Slump", 4.0, sit_slump)

        sit0 = sit_idle(0.0)
        stand0 = self.base("Idle", 0.0)

        def stand_up(t):
            a = t * t * (3 - 2 * t)
            P = lerp_pose(sit0, stand0, a)
            lean = 25 * math.sin(math.pi * min(1.0, t * 1.2))
            r.rot(P, "Abdomen", X, lean * 0.6)
            r.rot(P, "Chest", X, lean * 0.4)
            return P
        clip("Stand_Up", 0.8, stand_up)
        clip("Sit_Down", 0.8, lambda t: stand_up(1.0 - t))

        # Walk: the pack's walk, in place (Root/Body horizontal drift removed).
        wdur = self.dur["Walk"]

        def walk(t):
            P = self.base("Walk", t)
            return P
        clip("Walk", wdur, walk)
        clip("Idle_Stand", self.dur["Idle"], lambda t: self.base("Idle", t))
        clip("Wave", self.dur["Wave"], lambda t: self.base("Wave", t))

        def talk(t):
            P = self.base("Idle", t)
            g1 = 0.5 + 0.5 * math.sin(t * TAU * 2)
            g2 = 0.5 + 0.5 * math.sin(t * TAU * 2 + 2.2)
            for side, sx, g in (("R", -1, g1), ("L", 1, g2)):
                sh = r.pos(P, f"UpperArm.{side}")
                tgt = sh + Z * (0.22 + 0.08 * g) - Y * (0.22 - 0.12 * g) + X * sx * 0.12
                ik2(r, P, f"UpperArm.{side}", f"LowerArm.{side}", f"Wrist.{side}", tgt, -Y + X * sx * 0.6 - Z * 0.3)
            r.rot(P, "Chest", Y, 6 * math.sin(t * TAU))
            r.rot(P, "Head", X, 5 * math.sin(t * TAU * 4))
            r.rot(P, "Head", Y, -5 * math.sin(t * TAU))
            return P
        clip("Talk", 2.4, talk)

        def look(t):
            x = math.sin(t * TAU)
            y = max(-1.0, min(1.0, x * 1.6))
            P = self.base("Idle", t)
            r.rot(P, "Chest", Y, 10 * y)
            r.rot(P, "Neck", Y, 18 * y)
            r.rot(P, "Head", Y, 22 * y)
            return P
        clip("Look_Around", 4.0, look)
        return out


def _scale_to_height(doc, rig, mesh_nodes):
    root = doc.find("RootNode")
    P = rig.pose_at(A + "Idle", 0.0)
    pts = rig.skin_points(P, mesh_nodes)
    s = KID_HEIGHT / (pts[:, 1].max() - pts[:, 1].min())
    doc.j["nodes"][root]["scale"] = [s * x for x in doc.j["nodes"][root].get("scale", [1, 1, 1])]
    rig.refresh_rest()


def build(provider, out_path):
    doc = Doc(os.path.join(QDIR, BASE))
    _restyle(doc, provider)
    rig = Rig(doc)
    # kid proportions: bigger head (rest scale of the Head joint)
    hj = rig.j("Head")
    doc.j["nodes"][hj]["scale"] = [HEAD_SCALE] * 3
    rig.refresh_rest()
    mesh_nodes = [i for i, n in enumerate(doc.j["nodes"]) if "skin" in n and "mesh" in n]
    _scale_to_height(doc, rig, mesh_nodes)
    _decal(doc, rig, provider, rig.pose_at(A + "Idle", 0.0))
    kid = Kid(rig, mesh_nodes)
    keep = kid.build()
    doc.j["animations"] = [keep[c] for c in CLIPS]
    doc.j["asset"] = {"version": "2.0", "generator": "Otto School build (Quaternius UACP, CC0)"}
    for n in doc.j["nodes"]:
        n.pop("extras", None)
    doc.compact_attributes()
    doc.save(out_path)
    from .glbinfo import canonicalize_indices
    canonicalize_indices(out_path)
    return {"hip_y": round(float(kid.hip_y), 3), "thigh": round(float(kid.thigh), 3),
            "arm": round(float(kid.arm), 3)}
