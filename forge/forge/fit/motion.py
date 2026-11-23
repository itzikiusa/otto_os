"""Interaction templates: small data descriptions (targets relative to anchors and sockets over time),
executed by one routine. Arms and legs use an analytic two-bone IK on joint heads (AI rigs have arbitrary
bone tails, which Blender's IK constraint would treat as the end effector); the spine, hips and head are FK.
"""

import json
import math
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Quaternion, Vector

import fitlib as F
import worker as W

G = 9.81


def mat(m):
    return Matrix(np.asarray(m).tolist())


def npm(m):
    return F.matrix_np(m)


def rot_between(a, b):
    a, b = F.unit(a), F.unit(b)
    return np.array(Vector(a).rotation_difference(Vector(b)).to_matrix())


def align(a0, b0, a1, b1):
    """Rotation taking a0 to a1 and the plane (a0, b0) to the plane (a1, b1)."""
    def frame(a, b):
        a = F.unit(a)
        b = F.unit(b - a * (b @ a))
        return np.column_stack((a, b, np.cross(a, b)))
    return frame(a1, b1) @ frame(a0, b0).T


def slerp_m(A, B, u):
    """Blend two 4x4 rigid transforms (lerp position, slerp rotation)."""
    qa = mat(A).to_quaternion()
    qb = mat(B).to_quaternion()
    q = qa.slerp(qb, u)
    out = np.eye(4)
    out[:3, :3] = np.array(q.to_matrix())
    out[:3, 3] = A[:3, 3] * (1 - u) + B[:3, 3] * u
    return out


def ease(u):
    u = min(1.0, max(0.0, u))
    return u * u * (3 - 2 * u)


class Rig:
    """Armature-space pose writing: set world matrices for bones, parents first, without depsgraph lag."""

    def __init__(self, ch):
        self.ch, self.rig = ch, ch.rig
        self.mw = npm(ch.rig.matrix_world)
        self.imw = np.linalg.inv(self.mw)

    def pose_world(self, name):
        return self.mw @ npm(self.rig.pose.bones[name].matrix)

    def set_pose_world(self, name, M_world, parent_world=None):
        pb = self.rig.pose.bones[name]
        pose = self.imw @ M_world
        rest = npm(pb.bone.matrix_local)
        if pb.parent is None:
            basis = np.linalg.inv(rest) @ pose
        else:
            pp = self.imw @ (parent_world if parent_world is not None else self.pose_world(pb.parent.name))
            basis = np.linalg.inv(rest) @ npm(pb.parent.bone.matrix_local) @ np.linalg.inv(pp) @ pose
        pb.matrix_basis = mat(basis)

    def rotate_world(self, name, R, pivot=None):
        """Apply a world rotation to a bone about its head (children follow)."""
        Mw = self.pose_world(name)
        pivot = Mw[:3, 3] if pivot is None else pivot
        T = np.eye(4)
        T[:3, :3] = R
        T[:3, 3] = pivot - R @ pivot
        self.set_pose_world(name, T @ Mw)


def two_bone(S, E0, W0, T, pole, L1=None, L2=None):
    """Elbow/knee position for a wrist/ankle target T; returns (E, W, reach_shortfall)."""
    L1 = np.linalg.norm(E0 - S) if L1 is None else L1
    L2 = np.linalg.norm(W0 - E0) if L2 is None else L2
    v = T - S
    dist = np.linalg.norm(v)
    u = F.unit(v)
    d = min(max(dist, abs(L1 - L2) + 1e-6), (L1 + L2) * 0.9999)
    p = pole - u * (pole @ u)
    p = F.unit(p) if np.linalg.norm(p) > 1e-9 else F.unit(np.cross(u, [1, 0, 0]))
    cos_a = (L1 * L1 + d * d - L2 * L2) / (2 * L1 * d)
    a = math.acos(min(1, max(-1, cos_a)))
    E = S + u * L1 * math.cos(a) + p * L1 * math.sin(a)
    Wp = S + u * d
    return E, Wp, max(0.0, dist - d)


class Limb:
    def __init__(self, motion, upper, lower, end, pole_dir, rest_pole=None):
        self.m, self.upper, self.lower, self.end = motion, upper, lower, end
        self.rest_pole = rest_pole  # function -> the anatomical bend direction in the current FK pose
        ch = motion.ch
        self.L1 = np.linalg.norm(ch.head(lower, False) - ch.head(upper, False))
        self.L2 = np.linalg.norm(ch.head(end, False) - ch.head(lower, False))
        self.pole_dir = pole_dir  # function(frame) -> world direction of the elbow/knee

    def solve(self, target, smooth=True):
        """Place upper/lower/end so the end bone takes the target world matrix (as far as reachable).

        The elbow/knee bend plane is kept continuous in time: its offset direction may turn at most 12 degrees
        per 60 Hz frame (a pole that swings across a straight arm would otherwise flip the elbow in one frame)."""
        rg = self.m.rg
        Mu, Ml = rg.pose_world(self.upper), rg.pose_world(self.lower)
        S, E0, W0 = Mu[:3, 3], Ml[:3, 3], rg.pose_world(self.end)[:3, 3]
        pole = self.pole_dir()
        E, Wp, short = two_bone(S, E0, W0, target[:3, 3], pole, self.L1, self.L2)
        now = getattr(self.m, "now", None)
        last = getattr(self, "prev", None)
        if last is not None and now is not None and last[0] < now - 1e-9:
            self.base = last  # the last solve of an earlier frame (several solves can happen within one frame)
        elif last is None:
            self.base = None
        prev = getattr(self, "base", None)
        if prev is not None and now is not None and now > prev[0] and not F.disabled("pole_smooth"):
            uw = F.unit(Wp - S)
            off = F.unit((E - S) - uw * ((E - S) @ uw))
            po = prev[1] - uw * (prev[1] @ uw)
            if np.linalg.norm(po) > 1e-9:
                po = F.unit(po)
                limit = math.radians(12.0) * max(1.0, (now - prev[0]) * 60)
                ang = math.acos(float(np.clip(off @ po, -1, 1)))
                if ang > limit:
                    axis = np.cross(po, off)
                    axis = F.unit(axis) if np.linalg.norm(axis) > 1e-9 else uw
                    newpole = np.array(Matrix.Rotation(limit, 3, Vector(axis)) @ Vector(po))
                    E, Wp, short = two_bone(S, E0, W0, target[:3, 3], newpole, self.L1, self.L2)
        if now is not None:
            uw = F.unit(Wp - S)
            self.prev = (now, F.unit((E - S) - uw * ((E - S) @ uw)))
        # bend-plane normals: current (from the FK pose and the same pole rule) and solved
        # The FK bend plane is defined by the anatomical bend direction (elbow back, knee forward), not by
        # this frame's pole, so the upper bone's twist stays anatomical (no sideways elbows, no flips).
        ref = self.rest_pole() if self.rest_pole else pole
        n0 = np.cross(E0 - S, ref)
        n1 = np.cross(E - S, Wp - E)
        if np.linalg.norm(n1) < 1e-9:
            n1 = np.cross(E - S, pole)
        # map the anatomical bend direction onto the solved elbow/knee offset
        uw = F.unit(Wp - S)
        off = (E - S) - uw * ((E - S) @ uw)
        if np.linalg.norm(off) < 1e-9:
            off = pole
        R1 = align(E0 - S, ref, E - S, off)
        Mu2 = np.eye(4)
        Mu2[:3, :3] = R1 @ Mu[:3, :3]
        Mu2[:3, 3] = S
        Ml_rel = R1 @ Ml[:3, :3]
        E_fk = S + R1 @ (E0 - S)
        W_fk = E_fk + R1 @ (W0 - E0)
        R2 = rot_between(W_fk - E_fk, Wp - E)
        Ml2 = np.eye(4)
        Ml2[:3, :3] = R2 @ Ml_rel
        Ml2[:3, 3] = E
        Me = target.copy()
        Me[:3, 3] = Wp
        rg.set_pose_world(self.upper, Mu2)
        rg.set_pose_world(self.lower, Ml2, Mu2)
        rg.set_pose_world(self.end, Me, Ml2)
        return short


class Motion:
    """Executes one template on one character with its props."""

    def __init__(self, ch, template, props, grips, height_m):
        self.ch, self.t, self.props = ch, template, props
        self.rg = Rig(ch)
        self.grips = grips  # name -> {"hand": side, "prop": name, "T": hand->prop, "fingers": {bone: quat}}
        self.fps = template.get("fps", F.FPS)
        self.aim_yaw = math.radians(template.get("aim_yaw_deg", 0))
        up, fwd, right = ch.up, ch.forward, ch.right
        c, s = math.cos(self.aim_yaw), math.sin(self.aim_yaw)
        # aim rotates from forward towards the character's left by aim_yaw
        self.aim = F.unit(fwd * c - right * s)
        self.aim_left = F.unit(np.cross(up, self.aim))
        self.rest_world = {b.name: self.rg.pose_world(b.name) for b in ch.rig.pose.bones}
        self.owner = {}  # prop -> (owner key, relative matrix)
        self.cache = {}
        self.state = {}
        j = ch.jname
        back = -fwd
        self.arms = {s: Limb(self, j(s + "upperarm"), j(s + "forearm"), j(s + "hand"),
                             (lambda s=s: self.elbow_dir(s)), (lambda: -self.body_axes()[1])) for s in "lr"}
        self.legs = {s: Limb(self, j(s + "thigh"), j(s + "shin"), j(s + "foot"), (lambda: self.knee_dir()),
                             (lambda: self.body_axes()[1])) for s in "lr"}
        self.feet = {s: self.rest_world[j(s + "foot")] for s in "lr"}
        self.H = ch.height
        self.arm_len = {s: self.arms[s].L1 + self.arms[s].L2 for s in "lr"}
        self.log = {"reach_shortfall": [], "frames": []}
        self.clav_state = {}

    # -- frames and expressions -------------------------------------------------------------------
    def body_axes(self):
        hips = self.rg.pose_world(self.ch.jname("hips"))
        R = hips[:3, :3] @ self.rest_world[self.ch.jname("hips")][:3, :3].T
        return R @ self.ch.right, R @ self.ch.forward, self.ch.up

    def elbow_dir(self, side):
        right, fwd, up = self.body_axes()
        out = right if side == "r" else -right
        e = self.state.get("elbow_" + side, (0.35, -0.75, -0.55))  # outward, backward, downward
        return F.unit(out * e[0] + fwd * e[1] + up * e[2])

    def knee_dir(self):
        right, fwd, up = self.body_axes()
        return F.unit(fwd + 0.15 * up)

    def direction(self, ref):
        if isinstance(ref, (list, tuple)):
            right, fwd, up = self.body_axes()
            return F.unit(right * ref[0] + fwd * ref[1] + up * ref[2])
        sign = -1.0 if ref.startswith("-") else 1.0
        ref = ref.lstrip("-")
        right, fwd, up = self.body_axes()
        named = {"up": up, "down": -up, "forward": fwd, "back": -fwd, "right": right, "left": -right,
                 "aim": self.aim, "aim_left": self.aim_left, "aim_right": -self.aim_left}
        if ref in named:
            return sign * named[ref]
        kind, _, rest = ref.partition(":")
        if kind == "prop":
            name, _, key = rest.partition(".")
            return sign * self.props[name].world(key, self.pose_of(name))
        if kind == "joint":
            a, _, b = rest.partition(">")
            return sign * F.unit(self.joint(b) - self.joint(a))
        raise ValueError(f"Unknown direction {ref!r}")

    def joint(self, key):
        name = self.ch.jname(key) if key in self.ch.joints else key
        return self.rg.pose_world(name)[:3, 3]

    def unit_len(self, unit):
        if unit == "m":
            return self.ch.s
        if unit == "H":
            return self.H
        if unit.startswith("arm"):
            return self.arm_len[unit[-1]] if unit[-1] in "lr" else self.arm_len["r"]
        if unit.startswith("len:"):
            return self.props[unit[4:]].length
        if unit.startswith("r:"):
            p = self.props[unit[2:]]
            return p.anchors.get("radius", p.anchors.get("grip_radius"))
        raise ValueError(f"Unknown unit {unit!r}")

    def point(self, spec):
        if isinstance(spec, str):
            spec = {"base": spec}
        base = spec.get("base", "floor")
        kind, _, rest = base.partition(":")
        if base == "floor":
            p = self.joint("hips").copy()
            p[2] = self.ch.floor
        elif kind == "prop":
            name, _, key = rest.partition(".")
            p = self.props[name].world(key, self.pose_of(name))
        elif kind == "joint":
            p = self.joint(rest)
        elif kind == "socket":
            p = self.socket(rest)[:3, 3]
        elif kind == "hook":
            p = self.hook_point(rest)
        elif kind == "string":
            p = self.string_nock()
        else:
            raise ValueError(f"Unknown point {base!r}")
        for d, amount, unit in spec.get("add", []):
            p = p + self.direction(d) * amount * self.unit_len(unit)
        return p

    # -- sockets ----------------------------------------------------------------------------------
    def socket(self, name):
        bone, rel = self.sockets[name]
        return self.rg.pose_world(bone) @ rel

    def build_sockets(self, quiver_tilt_deg=25, handed="right"):
        ch, j = self.ch, self.ch.jname
        rest = ch.rest
        up, fwd, right = ch.up, ch.forward, ch.right
        socks = {}
        def surface(bone, direction, origin):
            ids = np.flatnonzero(ch.dom == bone)
            if not len(ids):
                return origin
            depth = ((rest[ids] - origin) @ direction).max()
            return origin + direction * depth
        uc = ch.head("upperchest" if "upperchest" in ch.joints else "chest")
        back = surface(j("upperchest" if "upperchest" in ch.joints else "chest"), -fwd, uc)
        tilt = math.radians(quiver_tilt_deg)
        side = right if handed == "right" else -right
        socks["back"] = (j("upperchest" if "upperchest" in ch.joints else "chest"),
                         F.frame_from(up * math.cos(tilt) + side * math.sin(tilt), -fwd, back))
        hips = ch.head("hips")
        for s, sg in (("hip_left", -1), ("hip_right", 1)):
            socks[s] = (j("hips"), F.frame_from(-up, -fwd, surface(j("hips"), right * sg, hips)))
        socks["neck"] = (j("neck") if "neck" in ch.joints else j("head"), F.frame_from(up, fwd, ch.head("neck") if "neck" in ch.joints else ch.head("head")))
        self.sockets = {k: (b, np.linalg.inv(self.rest_world[b]) @ M) for k, (b, M) in socks.items()}
        return {k: M.tolist() for k, (b, M) in socks.items()}

    # -- props ------------------------------------------------------------------------------------
    def pose_of(self, name):
        return self.cache.get(name, self.props[name].M())

    def place(self, name, spec):
        """World matrix putting a prop anchor at a point, its named axis along z, its roll axis towards x."""
        prop = self.props[name]
        if "nock_on" in spec:  # an arrow on a bow: nock on the string's nock point, shaft through the rest
            bow = spec["nock_on"]
            nock = self.string_rest() if F.disabled("nock") else self.string_nock()
            rest = self.props[bow].world("arrow_rest", self.pose_of(bow)) + self.rest_offset(bow)
            z = F.unit(rest - nock)
            x = self.props[bow].world("grip_axis", self.pose_of(bow))
            target = F.frame_from(z, x, nock)
            local = F.frame_from(prop.anchors["shaft_axis"], prop.anchors["roll_axis"], prop.anchors["nock"])
            return target @ np.linalg.inv(local)
        anchor = spec.get("anchor", "grip")
        zkey = spec.get("axis", "grip_axis" if "grip_axis" in prop.anchors else "axis")
        z = self.direction(spec["z"]) if "z" in spec else prop.world(zkey, self.pose_of(name))
        x = self.direction(spec["x"]) if "x" in spec else prop.world(spec.get("roll", "roll_axis"), self.pose_of(name))
        target = F.frame_from(z, x, self.point(spec["at"]))
        local = F.frame_from(prop.anchors[zkey], prop.anchors[spec.get("roll", "roll_axis")], prop.anchors[anchor])
        return target @ np.linalg.inv(local)

    def rest_offset(self, bow):
        """The arrow lies beside the riser on the archer's view-left side (a right-handed archer)."""
        p = self.props[bow]
        side = p.world("side_axis", self.pose_of(bow))
        if side @ self.aim_left < 0:
            side = -side
        r_arrow = next((q.anchors["grip_radius"] for q in self.props.values() if q.cls == "arrow"), 0.004 * self.ch.s)
        return side * (p.anchors.get("riser_half_thickness", 0) + r_arrow)

    # -- string (two straight segments from the tips to a nock point) -----------------------------
    def string_rest(self):
        bow = self.string_bow
        return self.props[bow].world("nock_rest", self.pose_of(bow))

    def string_nock(self):
        ks = [k for k in self.keys("string") if k["t"] <= self.now + 1e-9]
        if not ks or ks[-1].get("rest"):
            return self.string_rest()
        return self.state.get("string_nock", self.string_rest())

    def hook_point(self, hand):
        g = next(g for g in self.grips.values() if g["prop"] == "string" and g["hand"] == hand[0])
        M = self.rg.pose_world(self.ch.jname(g["hand"] + "hand")) @ np.asarray(g["T"])
        return M[:3, 3]

    # -- evaluation -------------------------------------------------------------------------------
    def keys(self, track):
        return self.t["tracks"].get(track, [])

    def bracket(self, track, t):
        ks = self.keys(track)
        if not ks:
            return None, None, 0.0
        if t <= ks[0]["t"]:
            return ks[0], ks[0], 0.0
        for a, b in zip(ks, ks[1:]):
            if a["t"] <= t <= b["t"]:
                return a, b, ease((t - a["t"]) / max(b["t"] - a["t"], 1e-9))
        return ks[-1], ks[-1], 0.0

    def scalar(self, track, key, t, default=0.0):
        a, b, u = self.bracket(track, t)
        if a is None:
            return np.asarray(default, float)
        va, vb = np.asarray(a.get(key, default), float), np.asarray(b.get(key, default), float)
        return va * (1 - u) + vb * u

    def fk_body(self, t):
        ch, rg = self.ch, self.rg
        for pb in ch.rig.pose.bones:
            pb.matrix_basis = Matrix.Identity(4)
        bpy.context.view_layer.update()
        right, fwd, up = ch.right, ch.forward, ch.up
        off = self.scalar("hips", "offset", t, [0, 0, 0]) * self.H
        comp = getattr(self, "comp", (0.0, 0.0))
        off = off + np.array([0, 0, -comp[1]])
        turn = math.radians(float(self.scalar("hips", "turn", t)))
        hips = ch.jname("hips")
        M = self.rest_world[hips].copy()
        R = np.array(Matrix.Rotation(turn, 3, Vector(up)))
        M[:3, :3] = R @ M[:3, :3]
        M[:3, 3] = M[:3, 3] + right * off[0] + fwd * off[1] + up * off[2]
        rg.set_pose_world(hips, M)
        bpy.context.view_layer.update()
        spine = [k for k in ("spine", "chest", "upperchest") if k in ch.joints]
        sturn = math.radians(float(self.scalar("spine", "turn", t))) / len(spine)
        bend = math.radians(float(self.scalar("spine", "bend", t)) + comp[0]) / len(spine)
        lean = math.radians(float(self.scalar("spine", "lean", t))) / len(spine)
        for k in spine:
            r_axis, _, _ = self.body_axes()
            Rb = np.array(Matrix.Rotation(sturn, 3, Vector(up)) @ Matrix.Rotation(-bend, 3, Vector(r_axis))
                          @ Matrix.Rotation(lean, 3, Vector(self.body_axes()[1])))
            rg.rotate_world(ch.jname(k), Rb)
            bpy.context.view_layer.update()
        for s in "lr":
            e = self.scalar("elbow_" + s, "dir", t, [0.35, -0.75, -0.55])
            self.state["elbow_" + s] = tuple(e)

    def hand_value(self, side, key, t):
        """World matrix of a hand for one key value at time t."""
        hand = self.ch.jname(side + "hand")
        if key.get("rest", False):
            return self.fk_hand[side]
        if "grip" in key:
            g = self.grips[key["grip"]]
            owned = self.owner.get(g["prop"], ((None,),))[0][0] == side + "_hand"
            frozen = self.frozen.get(key["grip"]) if owned else None
            P = frozen if frozen is not None else self.pose_of(g["prop"])
            return P @ np.linalg.inv(np.asarray(g["T"]))
        if "carry" in key:
            c = key["carry"]
            g = self.grips[c["grip"]]
            return self.place(g["prop"], c["place"]) @ np.linalg.inv(np.asarray(g["T"]))
        if "draw" in key:
            d = key["draw"]
            g = self.grips[d["grip"]]
            bow = self.string_bow
            at = self.string_rest() if d.get("at") == "string_rest" else self.point(d["at"])
            z = self.props[bow].world("grip_axis", self.pose_of(bow))
            x = F.unit(self.props[bow].world("arrow_rest", self.pose_of(bow)) - at)
            # the string grip's object frame is the hook frame: z along the string, x along the fingers
            return F.frame_from(z, x, at) @ np.linalg.inv(np.asarray(g["T"]))
        if "point" in key:
            M = self.fk_hand[side].copy()
            M[:3, 3] = self.point(key["point"])
            return M
        raise ValueError(f"Unknown hand key {key}")

    def finger_value(self, side, key):
        if key.get("rest", False) or "point" in key:
            return key.get("fingers_grip") and self.grips[key["fingers_grip"]]["fingers"] or {}
        name = key.get("grip") or (key.get("carry") or {}).get("grip") or (key.get("draw") or {}).get("grip")
        return self.grips[name]["fingers"] if name else {}

    def solve_hand(self, side, t):
        a, b, u = self.bracket({"l": "left_hand", "r": "right_hand"}[side], t)
        if a is None:
            return
        A = self.hand_value(side, a, t)
        B = A if b is a else self.hand_value(side, b, t)
        target = slerp_m(A, B, u)
        short = 0.0 if F.disabled("arm_ik") else self.arms[side].solve(target)
        S = self.rg.pose_world(self.arms[side].upper)[:3, 3]
        self.reach_down[side] = bool(target[2, 3] < S[2] - 0.1 * self.H)
        clav = self.ch.joints.get(side + "clavicle")
        if clav is not None and not F.disabled("clavicle"):
            # reach with the shoulder girdle: swing the clavicle towards a target above the shoulder that the
            # arm cannot reach (at most 35 degrees), rate-limited to 4 degrees per 60 Hz frame so it never pops
            C = self.rg.pose_world(clav.name)[:3, 3]
            v0, v1 = S - C, target[:3, 3] - C
            want = 0.0
            if short > 0.005 * self.H and not self.reach_down[side]:
                want = min(math.radians(35), math.acos(float(np.clip(F.unit(v0) @ F.unit(v1), -1, 1))))
            st = self.clav_state.get(side)
            if st is not None and self.now > st[0]:
                step = math.radians(4) * max(1.0, (self.now - st[0]) * 60)
                ang = st[1] + float(np.clip(want - st[1], -step, step))
            elif st is not None:
                ang = st[2]  # same frame: keep this frame's value
            else:
                ang = 0.0
            self.clav_state[side] = (self.now, st[1] if st is not None and self.now <= st[0] else ang, ang) if False else (self.now, ang, ang)
            axis = np.cross(v0, v1)
            if ang > math.radians(0.3) and np.linalg.norm(axis) > 1e-9:
                self.rg.rotate_world(clav.name, np.array(Matrix.Rotation(ang, 3, Vector(F.unit(axis)))))
                bpy.context.view_layer.update()
                short = self.arms[side].solve(target)
        self.shortfall[side] = short
        fa, fb = self.finger_value(side, a), self.finger_value(side, b)
        for bone in set(fa) | set(fb):
            qa = Quaternion(fa.get(bone, (1, 0, 0, 0)))
            qb = Quaternion(fb.get(bone, (1, 0, 0, 0)))
            pb = self.ch.rig.pose.bones[bone]
            pb.rotation_mode = "QUATERNION"
            pb.rotation_quaternion = qa.slerp(qb, u)
        bpy.context.view_layer.update()

    def solve_head(self, t):
        a, b, u = self.bracket("head", t)
        if a is None:
            return
        ch, rg = self.ch, self.rg
        head = ch.jname("head")
        Mh = rg.pose_world(head)
        fwd_now = Mh[:3, :3] @ self.rest_world[head][:3, :3].T @ ch.forward
        def want(k):
            if "look" not in k:
                return fwd_now
            v = self.point(k["look"]) - Mh[:3, 3] if not k["look"].startswith(("aim", "-", "up", "down", "forward", "back", "left", "right")) else self.direction(k["look"])
            return F.unit(v)
        wa, wb = want(a), want(b)
        target = F.unit(wa * (1 - u) + wb * u)
        R = rot_between(fwd_now, target)
        q = Quaternion(mat(np.pad(R, ((0, 1), (0, 1)))).to_quaternion())
        parts = [("neck", 0.4), ("head", 0.6)] if "neck" in ch.joints else [("head", 1.0)]
        for key, share in parts:
            qq = Quaternion(q.axis, q.angle * share)
            rg.rotate_world(ch.jname(key), np.array(qq.to_matrix()))
            bpy.context.view_layer.update()

    def owner_matrix(self, owner):
        kind, _, rest = owner.partition(":")
        if kind == "socket":
            return self.socket(rest)
        if owner in ("left_hand", "right_hand"):
            return self.rg.pose_world(self.ch.jname(owner[0] + "hand"))
        return self.pose_of(owner)

    def prop_value(self, name, key, t):
        if "place" in key:
            return self.place(name, key["place"])
        if "follow" in key:
            o = key["follow"]
            if self.owner.get(name, (None,))[0] != (o, key["t"]):
                rel = np.eye(4) if F.disabled("follow") else np.linalg.inv(self.owner_matrix(o)) @ self.pose_of(name)
                self.owner[name] = ((o, key["t"]), rel)
            return self.owner_matrix(o) @ self.owner[name][1]
        if "nocked" in key:
            return self.place(name, {"nock_on": self.string_bow})
        if "release" in key:
            return self.flight(name, t - key["t"])
        return self.pose_of(name)

    def solve_prop(self, name, t):
        ks = self.keys(name)
        if not ks:
            return
        cur = [k for k in ks if k["t"] <= t + 1e-9]
        key = cur[-1] if cur else ks[0]
        if "follow" not in key:
            self.owner.pop(name, None)  # nocked, released, placed or static: no hand owns it now
        self.cache[name] = self.prop_value(name, key, t)
        F.set_world(self.props[name].obj, self.cache[name])

    def solve_string(self, t):
        if not getattr(self, "string_bow", None):
            return
        ks = self.keys("string")
        cur = [k for k in ks if k["t"] <= t + 1e-9]
        key = cur[-1] if cur else {"rest": True}
        rest = self.string_rest()
        if "follow" in key:
            self.state["string_nock"] = self.hook_point(key["follow"])
        elif "release" in key:
            self.state["string_nock"] = self.string_after_release(t - key["t"])
        else:
            self.state["string_nock"] = rest

    # -- release and flight -----------------------------------------------------------------------
    def release_setup(self, arrow):
        """Snapshot at release: nock position, axis, draw length -> launch speed (simple energy model)."""
        p = self.t.get("params", {})
        nock = self.string_nock()
        rest = self.string_rest()
        draw_m = float(np.linalg.norm(nock - rest) / self.ch.s)
        F_draw = p.get("draw_weight_n", 178.0)
        m = p.get("arrow_mass_kg", 0.025)
        eta = p.get("efficiency", 0.75)
        # Linear draw-force curve: stored energy 1/2 F x; the arrow gets eta of it: 1/2 m v^2 = eta 1/2 F x.
        v0 = math.sqrt(max(eta * F_draw * draw_m / m, 0.0))
        axis = F.unit(self.props[arrow].world("shaft_axis", self.pose_of(arrow)))
        self.release = {"nock": nock, "rest_point": rest, "axis": axis, "draw_m": draw_m, "v0": v0,
                        "power_s": 2 * draw_m / v0 if v0 > 0 else 0.0, "M": self.pose_of(arrow).copy(),
                        "bow_M": self.pose_of(self.string_bow).copy()}
        return self.release

    def flight(self, arrow, dt):
        r = self.release
        s = self.ch.s
        Tp = 0.0 if F.disabled("power") else r["power_s"]
        if dt <= Tp:  # power stroke: constant acceleration along the shaft while on the string
            dist = 0.5 * (r["v0"] / Tp) * dt * dt * s if Tp > 0 else 0.0
            M = r["M"].copy()
            M[:3, 3] += r["axis"] * dist
            return M
        tau = dt - Tp
        p0 = r["M"][:3, 3] + r["axis"] * r["draw_m"] * s
        v = r["axis"] * r["v0"] * s
        g = np.array([0, 0, -G * s])
        pos = p0 + v * tau + 0.5 * g * tau * tau
        vel = v + g * tau
        R = rot_between(r["axis"], vel) @ r["M"][:3, :3]
        M = np.eye(4)
        M[:3, :3] = R
        M[:3, 3] = pos + (r["M"][:3, 3] - r["M"][:3, 3])
        return M

    def string_after_release(self, dt):
        r = self.release
        Tp = r["power_s"]
        rest = r["rest_point"] if np.allclose(r["bow_M"], self.pose_of(self.string_bow)) else self.string_rest()
        if dt <= Tp:
            M = self.flight(self.release_arrow, dt)
            nock_local = self.props[self.release_arrow].anchors["nock"]
            return M[:3, :3] @ nock_local + M[:3, 3]
        # damped overshoot past the brace line towards the bow, then settle (12 Hz, 40 ms decay)
        tau = dt - Tp
        brace = self.props[self.string_bow].anchors.get("brace", 0.18 * self.ch.s)
        amp = 0.2 * brace
        return rest + r["axis"] * amp * math.exp(-tau / 0.04) * math.sin(2 * math.pi * 12 * tau)

    # -- run --------------------------------------------------------------------------------------
    def times(self):
        dur = self.t["duration"]
        ts = set(np.round(np.arange(0, dur + 1e-9, 1 / self.fps), 6))
        for tr in self.t["tracks"].values():
            ts.update(k["t"] for k in tr)
        rel = [k["t"] for k in self.keys(self.release_arrow or "") if "release" in k] if getattr(self, "release_arrow", None) else []
        for t0 in rel:  # sub-frame keys through the power stroke so the export carries it exactly
            ts.update(np.round(t0 + np.arange(1, 13) * 0.002, 6))
        return sorted(t for t in ts if t <= dur + 1e-9)

    def run(self):
        """Two passes: find how much extra spine bend and hip drop each frame needs for the hands to reach
        (feet stay planted), smooth that over time so nothing pops, then solve and record."""
        ch = self.ch
        ch.deform(False)
        times = self.times()
        raw = []
        if self.t.get("compensate", True):
            snap = (dict(self.owner), dict(self.cache), dict(self.state))
            for limb in list(self.arms.values()) + list(self.legs.values()):
                limb.prev = None
            self.clav_state = {}
            self.frozen = {}
            for t in times:
                comp = [0.0, 0.0]
                for _ in range(12):
                    self.comp = tuple(comp)
                    self.solve_frame(t, record=False)
                    short = max(v for k, v in self.shortfall.items() if self.reach_down.get(k, False)) if any(self.reach_down.values()) else 0.0
                    if short < 0.003 * self.H or (comp[0] >= 50 and comp[1] >= 0.10 * self.H):
                        break
                    # reach with the spine first; the knees bend only after 35 degrees of extra bend, and the
                    # hip drop is capped at 10% of the height
                    if comp[0] < 35.0:
                        comp[0] = min(35.0, comp[0] + math.degrees(short / (0.35 * self.H)))
                    else:
                        comp[1] = min(0.10 * self.H, comp[1] + 0.6 * short)
                        comp[0] = min(50.0, comp[0] + math.degrees(short / (0.7 * self.H)))
                raw.append(comp)
            self.owner, self.cache, self.state = dict(snap[0]), dict(snap[1]), dict(snap[2])
            self.release = None
            self.released = False
            raw = np.array(raw)
            tt = np.array(times)
            env = np.array([raw[np.abs(tt - x) <= 0.12].max(axis=0) for x in tt])
            w = np.exp(-0.5 * ((tt[:, None] - tt[None, :]) / 0.06) ** 2)
            comps = (w @ env) / w.sum(axis=1)[:, None]
        else:
            comps = np.zeros((len(times), 2))
        self.frozen = {}
        frames = []
        self.released = False
        for limb in list(self.arms.values()) + list(self.legs.values()):
            limb.prev = None
        self.clav_state = {}
        for t, c in zip(times, comps):
            self.comp = (float(c[0]), float(c[1]))
            frames.append(self.solve_frame(t, record=True))
        ch.deform(True)
        self.frames = frames
        self.compensation = {"max_extra_bend_deg": float(comps[:, 0].max()), "max_extra_hip_drop_m": float(comps[:, 1].max() / ch.s)}
        return frames

    def solve_frame(self, t, record=True):
        ch = self.ch
        bones = [b.name for b in ch.rig.pose.bones]
        prop_order = self.t.get("order", list(self.props))
        arrow = getattr(self, "release_arrow", None)
        if arrow and not self.__dict__.get("released"):
            rk = [k for k in self.keys(arrow) if "release" in k]
            if rk and t >= rk[0]["t"] - 1e-9:
                self.now = rk[0]["t"] - 1e-6  # state as held, just before letting go
                self.release_setup(arrow)
                self.released = True
        if True:
            self.now = t
            self.shortfall = {"l": 0.0, "r": 0.0}
            self.reach_down = {}
            self.fk_body(t)
            self.fk_hand = {s: self.rg.pose_world(ch.jname(s + "hand")) for s in "lr"}
            for s in ("" if F.disabled("plant") else "lr"):
                self.legs[s].solve(self.feet[s])
            bpy.context.view_layer.update()
            for item in prop_order:
                if item in ("left_hand", "right_hand"):
                    self.solve_hand(item[0], t)
                elif item == "string":
                    self.solve_string(t)
                elif item == "head":
                    self.solve_head(t)
                else:
                    self.solve_prop(item, t)
            for e in self.t.get("events", []):
                if e.get("attach") and abs(e["t"] - t) < 1e-6:
                    for g, info in self.grips.items():
                        if info["prop"] == e["attach"]:
                            self.frozen[g] = self.pose_of(info["prop"]).copy()
            if not record:
                return None
            return {"t": t, "bones": {b: npm(ch.rig.pose.bones[b].matrix_basis) for b in bones},
                    "props": {n: self.pose_of(n).copy() for n in self.props},
                    "string": self.string_nock().copy() if getattr(self, "string_bow", None) else None,
                    "shortfall": dict(self.shortfall), "comp": list(self.comp),
                    "owners": {n: o[0][0] for n, o in self.owner.items()}}

    def bake(self, name, props=True):
        """Write the solved frames as one clip on the rig and on every prop (same NLA track name)."""
        ch = self.ch
        act, slot = F.new_action(ch.rig, name)
        bag = act.layers[0].strips[0].channelbag(slot)
        times = [f["t"] * F.FPS for f in self.frames]
        for b in [p.name for p in ch.rig.pose.bones]:
            pb = ch.rig.pose.bones[b]
            locs, rots = [], []
            prev = None
            for f in self.frames:
                M = mat(f["bones"][b])
                loc, q, _ = M.decompose()
                if prev is not None:
                    q.make_compatible(prev)
                prev = q
                locs.append(tuple(loc))
                rots.append(tuple(q))
            for prop, vals, n in (("location", locs, 3), ("rotation_quaternion", rots, 4)):
                for i in range(n):
                    F.add_curve(bag, pb.path_from_id(prop), i, times, vals)
        held = self.held_subframes(act, slot, times) if props else {}
        F.push_track(ch.rig, act, slot, name)
        for pname, prop in (self.props.items() if props else ()):
            pa, ps = bpy.data.actions.new(name), None
            ps = pa.slots.new("OBJECT", prop.obj.name)
            pbag = pa.layers.new("Fit").strips.new(type="KEYFRAME").channelbags.new(ps)
            locs, rots, scl, prev = [], [], [], None
            seq = held.get(pname) or [(tt, f["props"][pname]) for tt, f in zip(times, self.frames)]
            ptimes = [tt for tt, _ in seq]
            for _, Mx in seq:
                loc, q, sc = mat(Mx).decompose()
                if prev is not None:
                    q.make_compatible(prev)
                prev = q
                locs.append(tuple(loc)); rots.append(tuple(q)); scl.append(tuple(sc))
            prop.obj.rotation_mode = "QUATERNION"
            for path, vals, n in (("location", locs, 3), ("rotation_quaternion", rots, 4), ("scale", scl, 3)):
                for i in range(n):
                    F.add_curve(pbag, path, i, ptimes, vals)
            F.push_track(prop.obj, pa, ps, name)
        if props and getattr(self, "string_bow", None):
            self.bake_string(name, times)

    def held_subframes(self, act, slot, times, steps=4):
        """A prop held by a hand follows the BAKED bone chain, sampled 4x per key, so an engine interpolating
        the prop node and the bones sees no drift between frames (the solver frames alone would leave some)."""
        if F.disabled("subframes"):
            return {}
        ad = self.ch.rig.animation_data_create()
        ad.action, ad.action_slot = act, slot
        scene = bpy.context.scene
        hands = {"left_hand": self.ch.jname("lhand"), "right_hand": self.ch.jname("rhand")}
        def hand_at(fr, bone):
            scene.frame_set(int(math.floor(fr)), subframe=float(fr - math.floor(fr)))
            return self.rg.pose_world(bone)
        out = {}
        for pname in self.props:
            seq, any_held = [], False
            for i, (tt, f) in enumerate(zip(times, self.frames)):
                seq.append((tt, f["props"][pname]))
                o = f.get("owners", {}).get(pname)
                if i + 1 < len(times) and o in hands and self.frames[i + 1].get("owners", {}).get(pname) == o:
                    any_held = True
                    rel = np.linalg.inv(hand_at(tt, hands[o])) @ f["props"][pname]
                    for k in range(1, steps):
                        ts = tt + (times[i + 1] - tt) * k / steps
                        seq.append((ts, hand_at(ts, hands[o]) @ rel))
            if any_held:
                out[pname] = seq
        ad.action = None
        return out

    def bake_string(self, name, times):
        """The bowstring as nodes under the bow: a nock empty and two segments aimed tip -> nock."""
        bow = self.props[self.string_bow]
        parts = self.string_parts
        for key, obj in parts.items():
            act = bpy.data.actions.new(name)
            sl = act.slots.new("OBJECT", obj.name)
            bag = act.layers.new("Fit").strips.new(type="KEYFRAME").channelbags.new(sl)
            vals = {"location": [], "rotation_quaternion": [], "scale": []}
            prev = None
            for f in self.frames:
                Mb = f["props"][self.string_bow]
                inv = np.linalg.inv(Mb)
                nock = inv[:3, :3] @ f["string"] + inv[:3, 3]
                if key == "nock":
                    loc, q, sc = Vector(nock), Quaternion(), Vector((1, 1, 1))
                else:
                    tip = bow.anchors["upper_tip" if key == "upper" else "lower_tip"]
                    v = nock - tip
                    q = Vector((0, 0, 1)).rotation_difference(Vector(v))
                    loc, sc = Vector(tip), Vector((1, 1, float(np.linalg.norm(v))))
                if prev is not None:
                    q.make_compatible(prev)
                prev = q
                vals["location"].append(tuple(loc)); vals["rotation_quaternion"].append(tuple(q)); vals["scale"].append(tuple(sc))
            obj.rotation_mode = "QUATERNION"
            for path, v in vals.items():
                for i in range(len(v[0])):
                    F.add_curve(bag, path, i, times, v)
            F.push_track(obj, act, sl, name)


def string_prop(ch, bow):
    """A thin cylinder standing in for the bowstring while fitting the draw hand's hook (object frame =
    hook frame: z along the string, x across it)."""
    r = 0.0015 * ch.s
    length = 0.12 * ch.s
    bpy.ops.mesh.primitive_cylinder_add(vertices=12, radius=r, depth=length)
    obj = bpy.context.object
    obj.name = "fit_string_proxy"
    pts = np.array([v.co[:] for v in obj.data.vertices])
    prop = F.Prop.__new__(F.Prop)
    prop.name, prop.cls, prop.obj, prop.scale = "string", "string", obj, 1.0
    prop.points = F.surface_points(pts, np.array([t.vertices[:] for t in (obj.data.calc_loop_triangles() or obj.data.loop_triangles)]), length / 40)
    prop.anchors = {"axis": np.array([0, 0, 1.0]), "grip": np.zeros(3), "grip_axis": np.array([0, 0, 1.0]),
                    "roll_axis": np.array([1.0, 0, 0]), "grip_radius": r, "grip_length": length, "guard_side": 0.0}
    prop.proxy = F.proxy_from_points(prop.points, prop.anchors["axis"], np.zeros(3), 8)
    prop.length = length
    return prop


def build_string_nodes(bow):
    """Nock empty and two unit-length segment meshes, children of the bow object, aimed each frame."""
    r = 0.0012 * max(bow.length, 1e-6) / 1.0
    parts = {}
    nock = bpy.data.objects.new("bow_string_nock", None)
    bpy.context.collection.objects.link(nock)
    parts["nock"] = nock
    for key in ("upper", "lower"):
        mesh = bpy.data.meshes.new("bow_string_" + key)
        verts, faces = [], []
        for k in range(6):
            a = 2 * math.pi * k / 6
            verts += [(r * math.cos(a), r * math.sin(a), 0), (r * math.cos(a), r * math.sin(a), 1)]
        for k in range(6):
            i, j = 2 * k, 2 * ((k + 1) % 6)
            faces.append((i, j, j + 1, i + 1))
        mesh.from_pydata(verts, [], faces)
        mat = bpy.data.materials.new("BowString")
        mat.diffuse_color = (0.92, 0.9, 0.82, 1)
        mesh.materials.append(mat)
        obj = bpy.data.objects.new("bow_string_" + key, mesh)
        bpy.context.collection.objects.link(obj)
        parts[key] = obj
    for obj in parts.values():
        obj.parent = bow.obj
    return parts


def anchor_empties(prop):
    """Anchors as child empties ('<prop>.<anchor>') so engines and the checker can read them per frame."""
    for key, v in prop.anchors.items():
        if key.endswith("axis") or not isinstance(v, np.ndarray) or v.shape != (3,):
            continue
        e = bpy.data.objects.new(f"{prop.name}.{key}", None)
        bpy.context.collection.objects.link(e)
        e.parent = prop.obj
        e.matrix_parent_inverse = Matrix.Identity(4)
        e.location = Vector(v)
        e.empty_display_size = 0.01


def cmd_reach(job):
    tpl = json.loads(Path(job["targets"]).read_text())
    ch = F.Character(job["character"], job.get("height_m", 1.8))
    hand_info = W.ensure_hands(ch, job)
    F.progress(5, "Loading props")
    import props_lib
    props, prop_reports = {}, {}
    for name, spec in tpl.get("props", {}).items():
        path = (job.get("props") or {}).get(name) or spec["file"]
        if not Path(path).is_absolute():
            path = str(Path(job["targets"]).parent / path)
        props[name], prop_reports[name] = props_lib.ensure_functional(
            path, name, spec["class"], ch, spec.get("length_m"), spec.get("anchors"), job.get("work"), job.get("keep_prop"))
    m = Motion(ch, tpl, props, {}, job.get("height_m", 1.8))
    sockets = m.build_sockets(tpl.get("quiver_tilt_deg", 25))
    bows = [n for n, p in props.items() if p.cls == "bow"]
    if bows:
        m.string_bow = bows[0]
    arrows = [n for n in props if any("release" in k for k in tpl["tracks"].get(n, []))]
    m.release_arrow = arrows[0] if arrows else None
    # Fit every grip at rest: hand -> prop transform and the closed finger pose.
    starts = {}
    F.progress(15, "Fitting grips")
    for gname, g in tpl.get("grips", {}).items():
        side = g["hand"][0]
        prop = string_prop(ch, props[m.string_bow]) if g["prop"] == "string" else props[g["prop"]]
        res = W.fit_one_hand(ch, side, prop, g.get("kind", "power"), roll_to=g.get("roll", "d"),
                             reverse=g.get("reverse", False), slot=g.get("slot", 0))
        bones = W.finger_bones(ch, [side])
        m.grips[gname] = {"hand": side, "prop": g["prop"], "T": res["hand_to_prop"],
                          "fingers": {b: tuple(ch.rig.pose.bones[b].rotation_quaternion) for b in bones},
                          "fit": res["angles"], "contact": F.grip_report(ch, side, prop, 0.004 * ch.s)[0]}
        if g["prop"] == "string":
            bpy.data.objects.remove(prop.obj)
        ch.rest_pose()
    # Start placements in the rest pose, in template order.
    for name in tpl.get("order", list(props)):
        spec = tpl.get("props", {}).get(name, {}).get("start")
        if not spec or name not in props:
            continue
        if "socket" in spec:
            S = m.socket(spec["socket"])
            prop = props[name]
            zkey = spec.get("axis", "axis")
            local = F.frame_from(prop.anchors[zkey], prop.anchors["roll_axis"], np.zeros(3))
            M = S @ np.linalg.inv(local)
            back = S[:3, 0]
            M[:3, 3] += back * (prop.anchors.get("radius", 0.05 * ch.s) + 0.03 * ch.s)
            F.set_world(prop.obj, M)
            ids = np.flatnonzero(np.isin(ch.dom, [ch.jname(k) for k in ("spine", "chest", "upperchest", "lshoulder", "rshoulder") if k in ch.joints] + [ch.jname(k) for k in ("lclavicle", "rclavicle") if k in ch.joints]))
            def torso():
                return ch.samples(ch.skin(), ids)[0]
            F.press(ch, prop, torso, back, 0.0005 * ch.s, 0.3 * ch.s)
        elif "grip" in spec:
            g = m.grips[spec["grip"]]
            F.set_world(props[name].obj, m.rest_world[ch.jname(g["hand"] + "hand")] @ np.asarray(g["T"]))
        else:
            m.now = 0.0
            F.set_world(props[name].obj, m.place(name, spec["place"]))
        bpy.context.view_layer.update()
        m.cache[name] = props[name].M()
        starts[name] = m.cache[name].tolist()
    debug = {n: {k: np.round(p.world(k), 3).tolist() for k in p.anchors if isinstance(p.anchors[k], np.ndarray) and p.anchors[k].shape == (3,)}
             for n, p in props.items()}
    debug["joints"] = {k: np.round(ch.head(k), 3).tolist() for k in ("rupperarm", "lupperarm", "head", "hips")}
    debug["frame"] = {"forward": ch.forward.tolist(), "right": ch.right.tolist(), "aim": m.aim.tolist()}
    F.progress(35, f"Solving {tpl['name']} ({tpl['duration']} s)")
    m.run()
    for p in props.values():
        anchor_empties(p)
    if bows:
        m.string_parts = build_string_nodes(props[m.string_bow])
    F.progress(80, "Baking the clip")
    m.bake(tpl["name"])
    ch.rest_pose()
    for name, p in props.items():
        F.set_world(p.obj, np.asarray(starts.get(name, p.M())))
    ch.rig["forge_fit"] = json.dumps({tpl["name"]: {
        "template": tpl, "props": {n: {"class": p.cls, "anchors": {k: np.asarray(v).tolist() for k, v in p.anchors.items()}}
                                   for n, p in props.items()},
        "grips": {k: {"hand": v["hand"], "prop": v["prop"], "T": v["T"], "kind": tpl["grips"][k].get("kind", "power")}
                  for k, v in m.grips.items()},
        "release": {k: (v.tolist() if isinstance(v, np.ndarray) else v) for k, v in (getattr(m, "release", None) or {}).items()},
        "height_m": ch.height_m, "scene_units_per_m": ch.s}}, default=float)
    F.export(job["output"])
    short = max((max(f["shortfall"].values()) for f in m.frames), default=0.0)
    trace = [[round(f["t"], 3), round(f["shortfall"]["l"] / ch.s, 3), round(f["shortfall"]["r"] / ch.s, 3), [round(c, 3) for c in f["comp"]]]
             for f in m.frames[:: max(1, len(m.frames) // 24)]]
    return {"clip": tpl["name"], "frames": len(m.frames), "sockets": sockets, "hands_replaced": hand_info["replaced"],
            "prop_intake": prop_reports, "trace_t_short_l_r_comp": trace, "debug": debug,
            "grips": {k: {"fit": v["fit"], "contact": v["contact"]} for k, v in m.grips.items()},
            "max_reach_shortfall": short, "max_reach_shortfall_m": short / ch.s, "compensation": m.compensation,
            "release": {k: (v.tolist() if isinstance(v, np.ndarray) else v) for k, v in (getattr(m, "release", None) or {}).items() if k in ("v0", "draw_m", "power_s")},
            "scene_units_per_m": ch.s}


COMMANDS = {"reach": cmd_reach}
