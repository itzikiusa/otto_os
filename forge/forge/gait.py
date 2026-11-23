"""Procedural idle / walk / run for any body plan, baked into the GLB's own animation channels.

The humanoid keeps the library and preset retargets. Every other body (forge/bodyplan.py) gets clips made
from its own skeleton, in place like the preset loops (the manifest gives the forward speed a game
controller should move it at, which is the speed the planted feet travel backwards):

* legs: each leg has a phase in the cycle and a duty factor (the share of the cycle its foot is down).
  One leg hops; two alternate; three roll one after another; balanced sides walk a lateral-sequence wave
  (a dog's walk, an insect's metachronal wave) and run alternating diagonals (a trot, an alternating
  tripod or tetrapod). A planted foot moves straight back along the floor; a swinging foot lifts on an arc.
  Two-segment analytic IK (the knee keeps its rest side) reaches each foot target; the foot keeps its rest
  orientation so the sole stays flat. The body bobs with the stance, pitches and rolls a little; spine and
  tail swing with a lag, arms and ears sway, the head holds its world orientation (counter-motion).
* legless, long (a snake, a fish): a travelling wave of bending along the spine, growing to the tail.
* legless, upright or rigid (a blob, a slime, a robot on wheels): bounce with squash and stretch, sway.
* idle for every body: breathing, a slow weight shift with the feet planted, tail and ear sway, a look.

Every clip is periodic: its last key equals its first.
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

import numpy as np

from forge.bodyplan import UP, FOOT_LOW, Rig, axis_angle, detect, matrix_quat, quat_matrix, rotation_between

FPS = 30
G = 9.81
CLIPS = ("idle", "walk", "run")
# per gait: stride half-length and lift (share of hip height), Froude number (speed), crouch (share of hip height)
WALK = {"stride": 0.32, "lift": 0.10, "froude": 0.10, "crouch": 0.03, "spine_deg": 2.0, "tail_deg": 25.0,
        "arm_deg": 8.0, "pitch_deg": 1.0, "roll_deg": 3.0, "min_s": 1.0}
RUN = {"stride": 0.50, "lift": 0.16, "froude": 0.60, "crouch": 0.06, "spine_deg": 3.0, "tail_deg": 30.0,
       "arm_deg": 15.0, "pitch_deg": 3.0, "roll_deg": 4.0, "min_s": 0.8}


def _unit(v):
    n = np.linalg.norm(v)
    return v / n if n > 1e-12 else v


def _hermite(p0, m0, p1, m1, s):
    s2, s3 = s * s, s * s * s
    return (2 * s3 - 3 * s2 + 1) * p0 + (s3 - 2 * s2 + s) * m0 + (-2 * s3 + 3 * s2) * p1 + (s3 - s2) * m1


def phases(legs: list[dict], run: bool) -> list[float]:
    """Touch-down phase of each leg (fractions of a cycle), legs sorted front to back."""
    n = len(legs)
    by = {s: [i for i, g in enumerate(legs) if g["side"] == s] for s in ("left", "right", "center")}
    balanced = not by["center"] and len(by["left"]) == len(by["right"]) and n >= 2
    out = [0.0] * n
    if n == 1:
        return out
    if not balanced:  # one after another, front to back (a three-legged stool's tripod roll)
        return [i / n for i in range(n)]
    for side, shift in (("left", 0.0), ("right", 0.5)):
        legs_side = by[side]  # front to back
        m = len(legs_side)
        for k, i in enumerate(legs_side):
            if run:  # alternate along each side, sides opposite: trot / alternating tripod / tetrapod
                out[i] = ((k % 2) * 0.5 + shift) % 1.0
            else:    # a wave from the back to the front, sides half a cycle apart
                out[i] = ((m - 1 - k) / (2 * m) + shift) % 1.0
    return out


def duty(n: int, run: bool) -> float:
    if n == 1:
        return 0.4 if run else 0.5
    if n == 2:
        return 0.4 if run else 0.6
    if n == 4:
        return 0.45 if run else 0.65
    return 0.5 if run else 0.7


class Body:
    """Everything the clips need about one rig, computed once."""

    def __init__(self, rig: Rig, plan: dict):
        self.rig, self.plan = rig, plan
        r = rig
        self.n = len(r.names)
        self.root = next(k for k in r.order if r.parent[k] < 0)
        self.fwd, self.left = np.array(plan["forward"]), np.array(plan["left"])
        self.floor, self.H = plan["floor"], plan["height"]
        self.rest_rot = []
        for k in range(self.n):
            m = r.rest_world[k][:3, :3]
            self.rest_rot.append(m / np.linalg.norm(m, axis=0))
        self.R_loc = [quat_matrix(q) for _, q, _ in r.rest_local]
        skin, dom = r.skin_rest, r.dominant
        self.legs = []
        for g in plan["leg_chains"]:
            chain = g["chain"]
            hip_h = float(r.heads[chain[0]][1] - self.floor)
            low = self.floor + FOOT_LOW * hip_h
            a = len(chain)
            while a > 0 and r.heads[chain[a - 1]][1] <= low:
                a -= 1
            a = min(a, len(chain) - 1)
            feet = chain[a:] if a > 0 else chain
            own = np.isin(dom, feet)
            leg_skin = np.isin(dom, chain + g["twins"])  # the leg's lowest skin, which may not be the foot's
            sole = float(skin[leg_skin][:, 1].min()) if leg_skin.any() else float(r.heads[chain[-1]][1])
            drop = float(np.clip(sole - self.floor, 0.0, 0.25 * hip_h))
            if a == 0:  # the whole limb is a foot: swing it about its root, aim its lowest skin
                pts = skin[np.isin(dom, chain)]
                eff = pts[pts[:, 1] <= pts[:, 1].min() + 0.02 * self.H].mean(axis=0)
            else:
                eff = r.heads[chain[a]].copy()
            q = r.heads[chain[:a + 1]]
            knee = None
            if a >= 2:
                line = _unit(q[-1] - q[0])
                off = [np.linalg.norm((q[i] - q[0]) - ((q[i] - q[0]) @ line) * line) for i in range(1, a)]
                knee = 1 + int(np.argmax(off))
            fdir = np.zeros(3)
            if own.any():
                fdir = skin[own].mean(axis=0) - r.heads[chain[a]]
                fdir = fdir - (fdir @ UP) * UP
            flex_axis = _unit(np.cross(UP, fdir)) if np.linalg.norm(fdir) > 0.02 * hip_h else None
            foot_len = float(np.linalg.norm((skin[own] - r.heads[chain[a]]) * [1, 0, 1], axis=1).max()) if own.any() else 0.0
            limb_pts = skin[np.isin(dom, chain + g["twins"])]
            sole_pts = limb_pts[limb_pts[:, 1] <= limb_pts[:, 1].min() + 0.03 * self.H]
            self.legs.append({**g, "a": a, "knee": knee, "hip_h": hip_h, "drop": drop, "eff": eff,
                              "feet": feet, "flex_axis": flex_axis, "foot_len": foot_len, "sole": sole_pts})
        self.hip_h = float(np.mean([g["hip_h"] for g in self.legs])) if self.legs else 0.5 * self.H
        self.bottom = skin[skin[:, 1] <= self.floor + 0.05 * self.H]  # what a legless body stands on
        late = {x for g in self.legs for t in g["twins"] for x in r.subtree(t)}  # after the leg they follow
        self.order = [k for k in r.order if k not in late] + [k for k in r.order if k in late]

    def settle(self, want, crouch):
        """(how far to lower the body, half-stride, limiting leg): a leg standing straight cannot reach
        forward or back, so the body walks a little lower, as animals do (up to 15% of the hip height).
        Legs without a knee cannot take a lowered body: then it stays at its rest height."""
        best = (0.0, *self.max_stride(crouch))
        if best[1] >= want or any(g["a"] <= 1 for g in self.legs):
            return best
        for share in np.linspace(0.03, 0.15, 5):
            stride, limiting = self.max_stride(crouch + share * self.hip_h)
            if stride > best[1] + 1e-9:
                best = (share * self.hip_h, stride, limiting)
            if stride >= want:
                break
        return best

    def stance_inset(self, g, limit):
        if g["a"] < 2 or not limit:
            return 0.0
        # Bring a splayed foot toward its own hip without crossing under it.
        lateral = float((g["eff"] - self.rig.heads[g["chain"][0]]) @ self.left)
        return float(np.clip(0.5 * lateral, -limit, limit))

    def max_stride(self, crouch, inset=0.0):
        """(the largest half-stride every leg can reach with a planted foot, body lowered by `crouch`;
        the leg that limits it)."""
        best, limiting = np.inf, None
        for g in self.legs:
            chain, a, r = g["chain"], g["a"], self.rig
            q0 = r.heads[chain[0]]
            target = g["eff"] - g["drop"] * UP - self.left * self.stance_inset(g, inset)
            rel = target - q0
            ho, lo = float(rel @ self.fwd), float(rel @ self.left)
            v = float(rel @ UP) + crouch
            if a >= 2:
                k = g["knee"]
                reach = 0.98 * (np.linalg.norm(r.heads[chain[k]] - q0) + np.linalg.norm(r.heads[chain[a]] - r.heads[chain[k]]))
                room = reach ** 2 - v * v - lo * lo
                most = np.sqrt(room) - abs(ho) if room > 0 else 0.0
            elif a == 1:  # one bone to a flat foot: the ankle swings on an arc, rising at most 1.5% of the height
                length = float(np.linalg.norm(rel))
                most = length * np.sin(np.arccos(max(-1.0, 1 - 0.015 * self.H / max(length, 1e-9))))
            else:  # a leg without a knee swings rigidly: its sole rocks on the floor. The widest swing whose
                # sole stays within 4 mm under the floor and 1.5% of the height above it (forge-fit's planted band)
                pts = g["sole"] - q0
                lowest = float(pts[:, 1].min())
                reach = 0.0
                for deg in np.arange(1.0, 31.0):
                    ok = True
                    for sign in (1, -1):
                        turned = pts @ axis_angle(self.left, sign * np.radians(deg)).T
                        y = float(turned[:, 1].min())
                        ok &= (lowest - y) <= 0.004 and (y - lowest) <= 0.015 * self.H
                    if not ok:
                        break
                    reach = float(np.linalg.norm(rel)) * np.sin(np.radians(deg))
                most = reach
            if most < best:
                best, limiting = most, g["name"]
        return max(0.0, float(best)), limiting


def _foot_track(u, beta, stride, lift):
    """(forward offset, height) of a foot at cycle phase u since touch-down."""
    if u < beta:
        return stride - 2 * stride * u / beta, 0.0
    s = (u - beta) / (1 - beta)
    m = -2 * stride * (1 - beta) / beta  # keep the stance velocity through lift-off and touch-down
    return _hermite(-stride, m, stride, m, s), lift * np.sin(np.pi * s) ** 2


class Clip:
    """One clip's per-frame recipe: root offset/rotation/scale, per-joint world-axis turns, foot targets."""

    def __init__(self, body: Body, name: str, scale: float = 1.0):
        self.b, self.name, self.k = body, name, scale
        b = body
        self.speed = 0.0
        self.start_time = 0.0
        self.info = {}
        if name == "idle":
            self.T, self.cycles = 4.0, 1
            self.lower = b.settle(0.05 * b.hip_h, 0.0)[0] if b.legs else 0.0  # slack to plant every foot
        elif b.legs:
            p = RUN if name == "run" else WALK
            n = len(b.legs)
            hop = n == 1
            self.beta = duty(n, name == "run")
            self.phase = phases(b.legs, name == "run")
            if name == "run" and n >= 4 and set(self.phase) == {0.0, 0.5}:
                # The viewer holds frame zero until Play. Start during the
                # swing so the alternating stance is already visible.
                self.phase = [(phase - 0.25) % 1.0 for phase in self.phase]
            # a leg without a knee cannot take up a lowered or tipped body: then the body stays level
            self.level = any(leg["a"] <= 1 for leg in b.legs)
            crouch = 0.0 if self.level else (0.08 if hop else p["crouch"]) * b.hip_h * scale
            want = (0.35 if name == "run" else 0.25) * b.hip_h if hop else p["stride"] * b.hip_h
            self.lower, reach, limiting = b.settle(want * scale, crouch)
            self.stance_inset = 0.0
            if self.level and reach < 0.025 * b.hip_h and any(g["a"] >= 2 for g in b.legs):
                # Rigid legs keep the body level. If a fully extended knee
                # prevents a stride, shift its foot slightly inward instead.
                inset = 0.12 * b.hip_h
                alternative, leg = b.max_stride(crouch + self.lower, inset)
                if alternative > reach:
                    self.stance_inset, reach, limiting = inset, alternative, leg
            self.stride = min(want * scale, reach)
            self.lift = p["lift"] * b.hip_h * scale
            self.crouch = crouch
            self.hop = (0.18 if name == "run" else 0.10) * b.hip_h * scale if hop else 0.0
            v = np.sqrt((0.4 if (hop and name == "run") else p["froude"]) * G * b.hip_h)
            T = 2 * max(self.stride, 0.05 * b.hip_h) / (self.beta * v)
            # a short stride (stiff legs) slows the body down, not the cadence: no faster than the legs' pendulum
            T = max(T, (0.5 if name == "run" else 0.8) * 2 * np.pi * np.sqrt(b.hip_h / G))
            frames = max(8, round(T * FPS))
            self.T = frames / FPS
            self.cycles = int(np.ceil(p["min_s"] / self.T))
            self.speed = 2 * self.stride / (self.beta * self.T)
            self.p = p
            self.info = {"gait": "hop" if hop else ("alternating" if name == "run" else "wave"), "duty": self.beta,
                         "phases": dict(zip([g["name"] for g in b.legs], [round(x, 3) for x in self.phase])),
                         "stride_m": round(2 * self.stride, 4), "cycle_s": round(self.T, 4)}
            if reach < want * scale:  # say which leg holds the stride back (a stiff or folded leg)
                self.info["stride_limited_by"] = limiting
        else:
            self.T = 1.2 if name == "walk" else 0.7
            self.cycles = 1
            shape = b.plan["legless_shape"]
            length = sum(np.linalg.norm(b.rig.heads[j] - b.rig.heads[b.rig.parent[j]])
                         for j in b.plan["tail"] + b.plan["spine"][1:] + [b.plan["head"]] if b.rig.parent[j] >= 0)
            # a snake gains about a third of its length a wave; a hop covers about the body's height
            along = b.rig.skin_rest @ b.fwd
            length = min(length, float(along.max() - along.min()))  # a coiled chain is longer than the body
            self.speed = (0.35 * max(length, 0.5 * b.H) if shape == "long" else
                          (1.0 if shape == "upright" else 0.5) * b.H) / self.T
            self.info = {"gait": {"long": "travelling wave", "upright": "hop", "rigid": "glide"}[shape],
                         "cycle_s": self.T}
        self.duration = self.T * self.cycles

    def frame(self, t):
        """(root offset, root turn, root scale, {joint: world-axis turn}, {leg index: (target, flex)})."""
        t += self.start_time
        b, k = self.b, self.k
        w = 2 * np.pi / self.T
        turns, targets = {}, {}
        off, rot, scl = np.zeros(3), np.eye(3), np.ones(3)
        plan = b.plan
        deg = np.radians

        def add(j, axis, angle):
            turns[j] = axis_angle(axis, angle) @ turns.get(j, np.eye(3))

        if self.name == "idle":
            wi = 2 * np.pi / 4.0
            if b.legs:
                off = b.left * (0.012 * b.hip_h * np.sin(wi * t)) - UP * (0.006 * b.hip_h * (1 - np.cos(2 * wi * t)) / 2
                                                                           + self.lower)
                rot = axis_angle(b.fwd, deg(1.0) * np.sin(wi * t))
                for i, g in enumerate(b.legs):
                    targets[i] = (g["eff"] - g["drop"] * UP, 0.0)
            elif plan["legless_shape"] == "long":
                self._wave(t, 2 * np.pi / 4.0, 0.25, add)
            else:
                s = 0.015 * np.sin(2 * wi * t)
                scl = np.array([1 - s / 2, 1 + s, 1 - s / 2])
                rot = axis_angle(b.fwd, deg(1.5) * np.sin(wi * t))
                off = self._keep_up(UP * (s * self._rise()), rot, scl)
            for i, j in enumerate(plan["spine"][1:]):
                add(j, b.left, -deg(0.8) * np.sin(2 * wi * t - 0.4 * i))
            for i, j in enumerate(plan["tail"]):
                add(j, UP, deg(15.0) / len(plan["tail"]) * np.sin(wi * 2 * t - 0.8 * i))
            self._limbs(add, deg(2.0) * np.sin(wi * t), deg(4.0) * np.sin(wi * 4 * t))
            look = axis_angle(UP, deg(6.0) * np.sin(wi * t + 1.0))
            return off, rot, scl, turns, targets, look
        if b.legs:
            p = self.p
            u = [((t / self.T) - ph) % 1.0 for ph in self.phase]
            bump = [np.sin(np.pi * x / self.beta) if x < self.beta else 0.0 for x in u]
            off = -UP * (self.crouch * max(bump) + self.lower)
            if self.hop:
                s = (u[0] - self.beta) / (1 - self.beta)
                off = off + UP * (self.hop * np.sin(np.pi * s) if u[0] >= self.beta else 0.0)
            pitch = 0.0 if self.level else deg(p["pitch_deg"]) * k * np.sin(2 * w * t)
            sides = [1 if g["side"] == "left" else -1 if g["side"] == "right" else 0 for g in b.legs]
            sway = sum(sd * bp for sd, bp in zip(sides, bump)) / max(1, sum(abs(s) for s in sides))
            roll = 0.0 if self.level else -deg(p["roll_deg"]) * k * sway * (1.0 if len(b.legs) <= 2 else 0.3)  # lean over the stance feet
            if len(b.legs) <= 2:
                off = off + b.left * (0.03 * b.hip_h * k * sway)
            rot = axis_angle(b.fwd, roll) @ axis_angle(b.left, pitch)
            for i, g in enumerate(b.legs):
                f, y = _foot_track(u[i], self.beta, self.stride, self.lift)
                lift = y + (off @ UP if self.hop and u[i] >= self.beta else 0.0)
                # toes dip as the foot lifts, never further than half the lift (no scuffing through the floor)
                most = np.arcsin(min(1.0, 0.5 * self.lift / g["foot_len"])) if g["foot_len"] > 0 else 0.0
                flex = min(deg(20.0) * k, most) * (np.sin(np.pi * (u[i] - self.beta) / (1 - self.beta)) ** 2 if u[i] >= self.beta else 0.0)
                centre = g["eff"] - b.left * b.stance_inset(g, self.stance_inset)
                targets[i] = (centre - g["drop"] * UP + b.fwd * f + UP * lift, flex)
            for i, j in enumerate(plan["spine"][1:]):
                add(j, UP, deg(p["spine_deg"]) * k * np.sin(w * t - 0.5 * i))
            for i, j in enumerate(plan["tail"]):
                add(j, UP, deg(p["tail_deg"]) * k / len(plan["tail"]) * np.sin(w * t - 0.8 * (i + 1)))
            self._limbs(add, deg(p["arm_deg"]) * k * np.sin(w * t), deg(2.0) * k * np.sin(2 * w * t))
            look = axis_angle(b.left, deg(2.0) * k * np.sin(2 * w * t))
            return off, rot, scl, turns, targets, look
        shape = plan["legless_shape"]
        big = 1.0 if self.name == "walk" else 1.4
        if shape == "long":
            self._wave(t, w, big * k, add)
            look = np.eye(3)
        else:
            s = -0.06 * big * k * np.cos(2 * w * t)  # squash on the floor, stretch in the air
            scl = np.array([1 - s / 2, 1 + s, 1 - s / 2])
            # an upright body (a slime, a one-stem mushroom) hops; a rigid one (a robot on wheels) glides
            hop = (0.12 if shape == "upright" else 0.02) * big * k * b.H * (1 - np.cos(2 * w * t)) / 2
            off = UP * (hop + (scl[1] - 1) * self._rise())
            rot = axis_angle(b.fwd, np.radians(4.0) * big * k * np.sin(w * t)) @ axis_angle(UP, np.radians(3.0) * k * np.sin(w * t))
            off = self._keep_up(off, rot, scl)
            chain = [j for j in plan["spine"][1:] + [plan["head"]] if j != b.root]
            for i, j in enumerate(chain):
                add(j, b.fwd, np.radians(3.0) * big * k * np.sin(w * t - 0.7 * (i + 1)))
            look = np.eye(3)
        self._limbs(add, np.radians(6.0) * big * k * np.sin(w * t), np.radians(3.0) * k * np.sin(2 * w * t))
        return off, rot, scl, turns, targets, look

    def _keep_up(self, off, rot, scl):
        """Raise a legless body that sways or squashes so its lowest skin stays on the floor."""
        root = self.b.rig.heads[self.b.root]
        y = root[1] + off[1] + ((self.b.bottom - root) @ rot.T)[:, 1] * scl[1]
        return off + UP * max(0.0, float(self.b.floor - y.min()))

    def _rise(self):
        """How far the root stands above the floor (squash about the root must keep the bottom down)."""
        return float(self.b.rig.heads[self.b.root][1] - self.b.floor)

    def _wave(self, t, w, k, add):
        """A travelling wave of bending from the head to the tail, growing towards the tail."""
        b, plan, heads, parent = self.b, self.b.plan, self.b.rig.heads, self.b.rig.parent
        head_side = list(reversed(plan["spine"][1:])) + ([plan["head"]] if plan["head"] != b.plan["hub"] else [])
        seq = list(reversed(head_side)) + [plan["hub"]] + plan["tail"]  # head end .. tail end
        seq = [j for j in dict.fromkeys(seq)]
        if len(seq) < 3:
            return
        pos = np.cumsum([0.0] + [np.linalg.norm(heads[seq[i + 1]] - heads[seq[i]]) for i in range(len(seq) - 1)])
        s = pos / max(pos[-1], 1e-9)
        bend = 2 * np.pi * 0.08 * 2 * np.pi / len(seq)  # one wave along the body, sideways reach 8% of its length
        hub_at = seq.index(plan["hub"])
        for i, j in enumerate(seq):
            if j == plan["hub"] or parent[j] < 0:
                continue
            amp = bend * k * (0.4 + 0.6 * s[i])
            sign = 1.0 if i > hub_at else -1.0  # the hub's head side turns the other way (see the docstring)
            add(j, UP, sign * amp * np.sin(w * t - 2 * np.pi * s[i]))

    def _limbs(self, add, arm_angle, ear_angle):
        b = self.b
        for o in b.plan["limbs"]:
            if o["role"] == "arm":
                add(o["root"], b.left, arm_angle * (1 if o["side"] == "left" else -1))
            elif o["role"] == "ear":
                add(o["root"], b.fwd, ear_angle * (1 if o["side"] == "left" else -1))


def pose(body: Body, clip: Clip, t: float):
    """(joint world matrices (n, 4, 4), local rotations (n, 4), root local translation, root local scale)."""
    off, rot, scl, turns, targets, look = clip.frame(t)
    r = body.rig
    n = body.n
    W = np.zeros((n, 4, 4))
    quats = np.zeros((n, 4))
    absolute, twin_turn = {}, {}
    root_t, root_s = None, None
    for i in body.order:
        p = r.parent[i]
        B = W[p] @ r.offset[i] if p >= 0 else r.offset[i]
        sb = float(np.linalg.norm(B[:3, 0]))
        Rb = B[:3, :3] / sb
        T, _, S = r.rest_local[i]
        carried = Rb @ body.R_loc[i]
        if i == body.root:
            target = rot @ carried
            T = T + Rb.T @ off / sb
            S = S * scl
            root_t, root_s = T, S
        elif i in absolute:
            target = absolute[i]
        elif i == body.plan["head"]:
            target = look @ body.rest_rot[i]
        else:
            target = turns.get(i, np.eye(3)) @ carried
        for li, g in enumerate(body.legs):
            if i == g["chain"][0]:
                absolute.update(_leg(body, g, B, targets[li]))
                target = absolute.get(i, target)
                for t in g["twins"]:  # the other half of a leg split in two bones swings with it
                    twin_turn[t] = target @ carried.T
            elif i in twin_turn:
                target = twin_turn[i] @ carried
        R = Rb.T @ target
        q = matrix_quat(R)
        quats[i] = q
        M = np.eye(4)
        M[:3, :3] = quat_matrix(q) * S
        M[:3, 3] = T
        W[i] = B @ M
    return W, quats, root_t, root_s


def _leg(body: Body, g: dict, B0, target_flex):
    """World rotations for one leg's joints that put its foot on `target` (two-segment analytic IK)."""
    r = body.rig
    target, flex = target_flex
    chain, a = g["chain"], g["a"]
    # the leg carried rigidly by the posed body (rest local transforms)
    W, B = [], B0
    for idx, j in enumerate(chain[:a + 1]):
        if idx:
            B = W[-1] @ r.offset[j]
        T, Q, S = r.rest_local[j]
        M = np.eye(4)
        M[:3, :3] = quat_matrix(Q) * S
        M[:3, 3] = T
        W.append(B @ M)
    q = [w[:3, 3] for w in W]
    rots = [w[:3, :3] / np.linalg.norm(w[:3, :3], axis=0) for w in W]
    out = {}
    if a == 0:
        tip = q[0] + rots[0] @ body.rest_rot[chain[0]].T @ (g["eff"] - r.heads[chain[0]])
        out[chain[0]] = rotation_between(tip - q[0], target - q[0]) @ rots[0]
        return out
    if a == 1:
        swing = rotation_between(q[1] - q[0], target - q[0])
        out[chain[0]] = swing @ rots[0]
    else:
        k = g["knee"]
        A, Bv = q[k] - q[0], q[a] - q[k]
        la, lb = np.linalg.norm(A), np.linalg.norm(Bv)
        e0 = q[a] - q[0]
        e = _unit(e0)
        d = float(np.clip(np.linalg.norm(target - q[0]), abs(la - lb) + 1e-5, 0.999 * (la + lb)))
        u = A - (A @ e) * e
        if np.linalg.norm(u) < 1e-6 * la:
            u = np.cross(body.left, e)
        u = _unit(u)
        ca = np.clip((la * la + d * d - lb * lb) / (2 * la * d), -1.0, 1.0)
        A2 = la * (ca * e + np.sqrt(1 - ca * ca) * u)
        B2 = d * e - A2
        swing = rotation_between(e, target - q[0])
        RA, RB = swing @ rotation_between(A, A2), swing @ rotation_between(Bv, B2)
        for idx in range(a):
            out[chain[idx]] = (RA if idx < k else RB) @ rots[idx]
    flex_turn = axis_angle(g["flex_axis"], flex) if g["flex_axis"] is not None and flex else np.eye(3)
    for j in g["feet"]:
        out[j] = flex_turn @ body.rest_rot[j]
    return out


def start_in_stride(body: Body, clip: Clip, soles):
    """Open a many-legged run with the feet both spread fore/aft and lifted."""
    def stride_area(frame):
        world = pose(body, clip, frame / FPS)[0]
        feet = np.array([body.rig.skin(world, ids).mean(axis=0) for ids in soles])
        return np.ptp(feet @ body.fwd) * np.ptp(feet[:, 1])

    frames = round(clip.T * FPS)
    first = max(range(frames), key=stride_area)
    clip.start_time = first / FPS
    clip.info["phases"] = {leg["name"]: round((phase - first / frames) % 1.0, 3)
                           for leg, phase in zip(body.legs, clip.phase)}


def bake(body: Body, clip: Clip):
    frames = int(round(clip.duration * FPS))
    times = np.arange(frames + 1) / FPS
    quats, rt, rs = [], [], []
    for t in times[:-1]:
        _, q, T, S = pose(body, clip, t)
        quats.append(q)
        rt.append(T)
        rs.append(S)
    quats.append(quats[0])  # the loop closes exactly
    rt.append(rt[0])
    rs.append(rs[0])
    quats = np.array(quats)
    for f in range(1, len(quats)):  # keep each joint's quaternion on one hemisphere for interpolation
        flip = (quats[f] * quats[f - 1]).sum(axis=1) < 0
        quats[f][flip] *= -1
    if np.any((quats[-1] * quats[0]).sum(axis=1) < 0):  # hemisphere flips must not open the seam
        quats[-1] = quats[0]
    return times, quats, np.array(rt), np.array(rs)


SHORT = (("foot_penetration_m", "sink_m"), ("ground_penetration_m", "ground_sink_m"), ("foot_slide_m", "slide_m"),
         ("leg_swing_m", "foot_travel_m"), ("loop_seam_m", "loop_seam_m"))


def summary(report: dict, fit_report: str) -> dict:
    """The motion report as the stage manifest carries it, at most three levels deep under `motion`: per clip
    whether it passed and its worst numbers; why a clip was left out. Otto nests the manifest inside its own
    provenance, which an installed app caps at eight levels. The per-leg measurements stay in `fit_report`."""
    def shallow(entry):
        out = {k: v for k, v in entry.items() if k not in ("measures", "phases")}
        if "phases" in entry:
            out["phases"] = ", ".join(f"{leg} {ph}" for leg, ph in entry["phases"].items())
        for key, short in SHORT:
            if key in entry["measures"]:
                out[short] = round(entry["measures"][key]["value"], 4)
        failing = [k for k, m in entry["measures"].items() if not m["pass"]]
        if failing:
            out["failed_checks"] = ", ".join(failing)
        return out

    out = {k: v for k, v in report.items() if k not in ("clips", "failed")}
    out["clips"] = {name: shallow(e) for name, e in report["clips"].items()}
    if report["failed"]:
        out["left_out"] = {name: {**shallow(e), "reason": f"fails {shallow(e).get('failed_checks', '')} even at "
                                                         f"{round(e['motion_scale'] * 100)}% motion"}
                           for name, e in report["failed"].items()}
    out["fit_report"] = fit_report
    return out


def write_clips(rig: Rig, baked: dict, out: Path, landmarks: dict) -> None:
    """The input GLB with `baked` {name: (times, quats, root_t, root_s)} as its only animations."""
    doc = json.loads(json.dumps(rig.doc))
    binary = bytearray(rig.binary)
    root = rig.nodes[next(k for k in rig.order if rig.parent[k] < 0)]

    def add(array, kind, minmax=False):
        data = np.ascontiguousarray(array, dtype="<f4")
        binary.extend(b"\0" * (-len(binary) % 4))
        doc.setdefault("bufferViews", []).append({"buffer": 0, "byteOffset": len(binary), "byteLength": data.nbytes})
        binary.extend(data.tobytes())
        acc = {"bufferView": len(doc["bufferViews"]) - 1, "componentType": 5126, "count": len(data), "type": kind}
        if minmax:
            acc.update(min=[float(data.min())], max=[float(data.max())])
        doc.setdefault("accessors", []).append(acc)
        return len(doc["accessors"]) - 1

    doc["animations"] = []
    for name, (times, quats, rt, rs) in baked.items():
        anim = {"name": name, "samplers": [], "channels": []}
        if name in landmarks:
            anim["extras"] = landmarks[name]
        tin = add(times, "SCALAR", minmax=True)
        tracks = [(rig.nodes[j], "rotation", quats[:, j], "VEC4") for j in range(len(rig.nodes))]
        tracks += [(root, "translation", rt, "VEC3"), (root, "scale", rs, "VEC3")]
        for node, path, values, kind in tracks:
            anim["samplers"].append({"input": tin, "output": add(values, kind), "interpolation": "LINEAR"})
            anim["channels"].append({"sampler": len(anim["samplers"]) - 1, "target": {"node": node, "path": path}})
        doc["animations"].append(anim)
    binary.extend(b"\0" * (-len(binary) % 4))
    doc["buffers"] = [{"byteLength": len(binary)}]
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    Path(out).write_bytes(struct.pack("<4sII", b"glTF", 2, 28 + len(text) + len(binary))
                          + struct.pack("<I4s", len(text), b"JSON") + text
                          + struct.pack("<I4s", len(binary), b"BIN\0") + bytes(binary))


def animate(mesh: Path, out: Path, names=CLIPS, height_m=None, progress=lambda pct, msg: None) -> dict:
    """Make `names` (idle, walk, run) for a non-humanoid rig, check them, and write `out`.

    A locomotion clip that fails the foot/ground/loop checks is remade with smaller motion (70%, then 45%);
    if it still fails it is left out and the report says so. Other clip names are skipped and reported."""
    from forge.fit import plan_check
    rig = Rig(mesh)
    plan = detect(rig)
    body = Body(rig, plan)
    report = {"body_plan": plan["body_plan"], "legs": plan["legs"], "forward": [round(v, 4) for v in plan["forward"]],
              "clips": {}, "skipped": {}, "failed": {}}
    if plan["legless_shape"]:
        report["legless_shape"] = plan["legless_shape"]
    wanted = [n for n in names if n in CLIPS]
    for n in names:
        if n not in CLIPS:
            report["skipped"][n] = f"no procedural {n!r} for a {plan['body_plan']} body (made: idle, walk, run)"
    baked, landmarks = {}, {}
    for i, name in enumerate(wanted):
        progress(10 + 80 * i // max(len(wanted), 1), f"making {name} for a {plan['body_plan']} body")
        for scale in ((1.0,) if name == "idle" else (1.0, 0.7, 0.45)):
            clip = Clip(body, name, scale)
            if name == "run" and len(body.legs) >= 6:
                start_in_stride(body, clip, plan_check.soles(rig, plan))
            one = bake(body, clip)
            measures = plan_check.measure(rig, plan, name, clip.duration, lambda t: pose(body, clip, t)[0], height_m)
            entry = {**clip.info, "duration_s": round(clip.duration, 4), "speed_m_s": round(clip.speed, 4),
                     "motion_scale": scale, "pass": measures["pass"], "measures": measures["measures"]}
            if measures["pass"]:
                break
        if entry["pass"] or name == "idle":
            baked[name] = one
            if len(body.legs) >= 6:
                world = pose(body, clip, 0.)[0]
                skin = rig.skin(world)
                soles = [skin[ids].mean(axis=0).round(5).tolist() for ids in plan_check.soles(rig, plan)]
                # Two centres on each posed lower leg distinguish feet whose
                # soles are apart but whose shins hide one another in the shot.
                axes = []
                height = np.ptp(skin[:, 1])
                for leg in plan["leg_chains"]:
                    points = skin[np.isin(rig.dominant, leg["chain"] + leg.get("twins", []))]
                    lower = points[points[:, 1] <= points[:, 1].min() + .19 * height]
                    low, high = np.quantile(lower[:, 1], [.2, .8])
                    ends = [lower[lower[:, 1] <= low].mean(axis=0), lower[lower[:, 1] >= high].mean(axis=0)]
                    axes.append(np.round(ends, 5).tolist())
                landmarks[name] = {"foot_centres": soles, "lower_leg_axes": axes}
            report["clips"][name] = entry
        else:
            report["failed"][name] = entry
    if not baked:
        raise ValueError(f"no clip passed the checks for this {plan['body_plan']} body")
    if report["failed"]:
        why = sorted({k for e in report["failed"].values() for k, m in e["measures"].items() if not m["pass"]})
        held = sorted({e["stride_limited_by"] for e in report["failed"].values() if e.get("stride_limited_by")})
        report["note"] = (f"shipped {', '.join(baked)}; {', '.join(report['failed'])} left out: they fail "
                          f"{', '.join(why)} even at 45% motion"
                          + (f" (the stride is held back by {', '.join(held)}: a leg without a knee or one resting "
                             "on the floor, as in a sitting pose)" if held else ""))
    write_clips(rig, baked, out, landmarks)
    shipped = plan_check.check_glb(out, height_m=height_m)  # the numbers that count: read back from the file
    for name, result in shipped["clips"].items():
        report["clips"][name].update({"pass": result["pass"], "measures": result["measures"]})
    report["pass"] = all(c["pass"] for c in report["clips"].values())
    return report

