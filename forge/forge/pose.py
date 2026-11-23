"""forge pose: hold a rigged character in a pose fitted to its scene, as a clip played by default.

The pose is computed on the character's own skeleton (forge/bodyplan.py reads it) by aiming a few joints along
world directions, never by adding angles to the rest pose: a SkinTokens rest pose may be a T, an A or a crouch,
and its glTF bone tails are guesses, so every bone is aimed along its joint-to-child direction.

Humanoid (Unity names, as forge rig writes them): stand, sit (on a seat of a given height: thighs along the seat,
feet on the floor or hanging when the seat is above the knee), kneel (left knee down), crouch, lie (on the back),
curl (on the side, knees drawn up), hold-out (an arm held forward, palm up). Four legs: stand, sit (on the
haunches, front legs straight), lie (legs folded), curl (lying, spine and tail curled round). Any body with a head:
look-at. Other poses on other bodies are refused.

The posed model is checked before it is written: skin under the floor or the seat, and skin triangles that
collapse (lose most of their area) fail the pose rather than ship a broken mesh.
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

import numpy as np

from forge.bodyplan import UP, Rig, accessor, axis_angle, detect, matrix_quat, quat_matrix, rotation_between

POSES = ("stand", "sit", "kneel", "crouch", "lie", "curl", "hold-out", "hold", "lean", "cross-arms", "peek")
FPS = 30
BREATH_S = 3.0
BREATH_DEG = 1.2
# fail the pose when these are exceeded (shares of the character's height, or of its skin triangles)
LIMITS = {"floor_penetration": 0.02, "seat_penetration": 0.03, "collapsed_share": 0.02, "torn_share": 0.015}
STRETCH, TEAR = 3.0, 0.05   # a skin edge grown past 3x its rest length and by over 5% of the height is torn
HUMAN = ("Hips", "Head", "LeftUpperLeg", "RightUpperLeg", "LeftLowerLeg", "RightLowerLeg", "LeftFoot",
         "RightFoot", "LeftUpperArm", "RightUpperArm", "LeftLowerArm", "RightLowerArm", "LeftHand", "RightHand")


class Unsupported(ValueError):
    pass


def _unit(v):
    n = np.linalg.norm(v)
    return v / n if n > 1e-12 else v


def _rot(m):
    return m[:3, :3] / np.linalg.norm(m[:3, :3], axis=0)


def _slerp_matrix(R, amount):
    q = matrix_quat(R)
    angle = 2 * np.arccos(np.clip(q[3], -1, 1))
    if angle < 1e-9:
        return np.eye(3)
    return axis_angle(q[:3] / np.linalg.norm(q[:3]), angle * amount)


class Skeleton:
    """Local joint rotations and the root translation of one rig, with forward kinematics."""

    def __init__(self, rig: Rig):
        self.rig = rig
        self.n = len(rig.names)
        self.idx = {name: k for k, name in enumerate(rig.names)}
        self.q = np.array([q for _, q, _ in rig.rest_local], float)
        self.T = [np.array(t, float) for t, _, _ in rig.rest_local]
        self.S = [np.array(s, float) for _, _, s in rig.rest_local]
        self.roots = [k for k in rig.order if rig.parent[k] < 0]
        self.root = self.roots[0]

    def j(self, name):
        return self.idx.get(name)

    def world(self):
        r = self.rig
        W = np.zeros((self.n, 4, 4))
        for i in r.order:
            p = r.parent[i]
            B = W[p] @ r.offset[i] if p >= 0 else r.offset[i]
            M = np.eye(4)
            M[:3, :3] = quat_matrix(self.q[i]) * self.S[i]
            M[:3, 3] = self.T[i]
            W[i] = B @ M
        return W

    def turn(self, k, R, W=None):
        """Rotate joint k (and everything it carries) by the world rotation R about its own head."""
        W = self.world() if W is None else W
        p = self.rig.parent[k]
        B = W[p] @ self.rig.offset[k] if p >= 0 else self.rig.offset[k]
        self.q[k] = matrix_quat(_rot(B).T @ (R @ _rot(W[k])))

    def aim(self, k, child, direction, amount=1.0):
        """Point the joint-to-child direction of joint k along a world direction."""
        if k is None or child is None:
            return
        W = self.world()
        cur = W[child][:3, 3] - W[k][:3, 3]
        if np.linalg.norm(cur) < 1e-9:
            return
        R = rotation_between(cur, np.asarray(direction, float))
        self.turn(k, R if amount == 1.0 else _slerp_matrix(R, amount), W)

    def move(self, delta):
        """Translate the whole body in world space."""
        for k in self.roots:
            B = self.rig.offset[k]
            self.T[k] = self.T[k] + _rot(B).T @ np.asarray(delta, float) / float(np.linalg.norm(B[:3, 0]))


def _child(sk, k, preferred=()):
    for name in preferred:
        if sk.j(name) is not None:
            return sk.j(name)
    kids = sk.rig.children[k]
    return kids[0] if len(kids) == 1 else None


class Body:
    """What the poses need: facing, floor, height and the named parts of a humanoid or a body plan."""

    def __init__(self, rig: Rig):
        self.rig = rig
        skin = rig.skin_rest
        self.floor = float(skin[:, 1].min())
        self.height = float(skin[:, 1].max() - self.floor)
        swimming = rig.doc["skins"][0].get("extras", {}).get("swimming_fish") is True
        self.humanoid = not swimming and all(n in set(rig.names) for n in HUMAN)
        self.arm_joints = {}
        if self.humanoid:
            self.plan = None
            left = rig.heads[rig.names.index("LeftUpperLeg")] - rig.heads[rig.names.index("RightUpperLeg")]
            self.left = _unit(left - (left @ UP) * UP)
            self.fwd = _unit(np.cross(self.left, UP))
            self.kind = "humanoid"
        else:
            self.plan = detect(rig)
            self.fwd, self.left = np.array(self.plan["forward"]), np.array(self.plan["left"])
            self.kind = "quadruped" if self.plan["body_plan"] == "quadruped" else self.plan["body_plan"]

            if self.kind == "legs:2":
                self.arm_joints = self._paired_arms()

    def _paired_arms(self):
        """Find a shoulder, elbow and palm on each arm ending in a fork of fingers."""
        arms = [o for o in self.plan["limbs"] if o["role"] == "arm"]
        if len(arms) != 2 or {a["side"] for a in arms} != {"left", "right"}:
            return {}
        aliases = {}
        for arm in arms:
            chain = [arm["root"]]
            while len(self.rig.children[chain[-1]]) == 1:
                chain.append(self.rig.children[chain[-1]][0])
            if len(chain) < 3 or len(self.rig.children[chain[-1]]) < 2:
                return {}
            for name, index in zip(("UpperArm", "LowerArm", "Hand"), chain[-3:]):
                aliases[arm["side"].title() + name] = index
        return aliases

    def paired_paws(self):
        """Recover two free, opposing arm chains from their measured joints."""
        if self.kind not in ("legs:2", "legless"):
            return {}
        rig = self.rig
        root = rig.heads[rig.order[0]]
        points = (rig.heads - root) / self.height
        lateral = points @ self.left
        sides = {"Left": [], "Right": []}
        for wrist, children in enumerate(rig.children):
            # Branched digits distinguish a palm on a misclassified body.
            # A two-legged body can also have a paw with a single end joint.
            fork = len(children) >= 3
            if not fork and (children or self.kind != "legs:2"):
                continue
            elbow = rig.parent[wrist]
            shoulder = rig.parent[elbow] if elbow >= 0 else -1
            if shoulder < 0 or len(rig.children[elbow]) != 1 or len(rig.children[shoulder]) != 1:
                continue
            height = (rig.heads[wrist, 1] - self.floor) / self.height
            reach = abs(lateral[wrist]) - abs(lateral[shoulder])
            lengths = np.linalg.norm(np.diff(points[[shoulder, elbow, wrist]], axis=0), axis=1)
            if not (0.22 < height < 0.78 and reach > 0.07 and lengths.min() > 0.045
                    and points[shoulder, 1] - points[wrist, 1] > 0.07):
                continue
            side = "Left" if lateral[wrist] > 0 else "Right"
            sides[side].append((fork, [shoulder, elbow, wrist]))
        pairs = [(a, b) for fork, a in sides["Left"] for other, b in sides["Right"]
                 if fork == other and abs(lateral[a[-1]] + lateral[b[-1]]) < 0.12
                 and np.max(np.abs(points[a, 1] - points[b, 1])) < 0.12]
        if len(pairs) != 1:
            return {}
        return {side + part: joint for side, chain in zip(("Left", "Right"), pairs[0])
                for part, joint in zip(("UpperArm", "LowerArm", "Hand"), chain)}

    def head(self, sk):
        if self.humanoid:
            return sk.j("Head"), sk.j("Neck")
        h = self.plan.get("head")
        if h is None:
            return None, None
        p = self.rig.parent[h]
        return h, (p if p >= 0 else None)

    def supported(self):
        if self.humanoid:
            return set(POSES)
        if self.arm_joints:
            return {"stand", "hold", "hold-out", "peek"}
        if self.kind == "quadruped":
            return {"stand", "sit", "lie", "curl", "peek"}
        return {"stand", "peek"}


# ---------------------------------------------------------------- humanoid poses

def _knee(hip, ankle, l1, l2, pole):
    """Two-segment IK: the knee for a hip and an ankle target, bending towards the pole direction."""
    e = ankle - hip
    d = float(np.clip(np.linalg.norm(e), abs(l1 - l2) + 1e-5, 0.999 * (l1 + l2)))
    e = _unit(e)
    a = (l1 * l1 - l2 * l2 + d * d) / (2 * d)
    u = _unit(pole - (pole @ e) * e)
    return hip + e * a + u * np.sqrt(max(l1 * l1 - a * a, 0.0))


def reach_hand(sk, side, position, bend):
    """Place a wrist using its two measured arm lengths and an elbow bend direction."""
    shoulder, elbow, wrist = [sk.j(side + part) for part in ("UpperArm", "LowerArm", "Hand")]
    w = sk.world()[:, :3, 3]
    a, b, c = w[[shoulder, elbow, wrist]]
    joint = _knee(a, position, np.linalg.norm(b - a), np.linalg.norm(c - b), bend)
    sk.aim(shoulder, elbow, joint - a)
    sk.aim(elbow, wrist, position - sk.world()[elbow][:3, 3])


def cross_arms(sk, body):
    """Fold the forearms across the chest, one above the other."""
    world = sk.world()
    w = world[:, :3, 3]
    shoulders = [w[sk.j(side + "UpperArm")] for side in ("Left", "Right")]
    middle = (shoulders[0] + shoulders[1]) / 2
    width = np.linalg.norm(shoulders[0] - shoulders[1])
    # Measure the shirt between the shoulders and waist. A wrist can
    # cross the sternum inside a thick torso and be invisible from the front.
    torso = [sk.idx[n] for n in ("Hips", "Spine", "Chest", "UpperChest") if n in sk.idx]
    ids = np.flatnonzero(np.isin(body.rig.dominant, torso))
    chest = body.rig.skin(world, ids) - middle
    chest = chest[(abs(chest @ body.left) < .45 * width)
                  & (chest[:, 1] < -.06 * body.height) & (chest[:, 1] > -.20 * body.height)]
    depth = .10 * body.height
    bend_forward = 0.
    if len(chest) >= 12:
        front = float(np.percentile(chest @ body.fwd, 95)) + .035 * body.height
        bend_forward = np.clip((front - depth) / max(.20 * width, 1e-6), 0., 1.)
        depth = max(depth, front)
    for side, sign in (("Left", 1), ("Right", -1)):
        target = (middle - sign * 0.27 * width * body.left
                  + depth * body.fwd
                  - (0.12 + sign * 0.018) * body.height * UP)
        reach_hand(sk, side, target, sign * body.left - 1.5 * UP + bend_forward * body.fwd)
        hand = sk.j(side + "Hand")
        sk.aim(hand, _child(sk, hand, (side + "MiddleProximal",)), -sign * body.left + 0.22 * UP)


def lean(sk, body):
    """Shift the shoulders behind the planted feet; the soles stay level."""
    turn = axis_angle(body.left, np.radians(-17))
    sk.turn(sk.j("Hips"), turn)
    for side in ("Left", "Right"):
        sk.turn(sk.j(side + "Foot"), turn.T)


def _leg(sk, side):
    return sk.j(side + "UpperLeg"), sk.j(side + "LowerLeg"), sk.j(side + "Foot"), sk.j(side + "Toes")


def _lengths(rig, sk, side):
    u, l, f, _ = _leg(sk, side)
    return float(np.linalg.norm(rig.heads[l] - rig.heads[u])), float(np.linalg.norm(rig.heads[f] - rig.heads[l]))


def _flat(sk, k):
    """Give joint k its rest world orientation again (a foot keeps its sole flat on the floor)."""
    if k is not None:
        W = sk.world()
        sk.turn(k, _rot(sk.rig.rest_world[k]) @ _rot(W[k]).T, W)


def _lean(sk, body, deg, joint="Spine"):
    k = sk.j(joint)
    if k is not None:
        sk.turn(k, axis_angle(body.left, np.radians(deg)))


def sit(sk, body, seat):
    """Thighs along the seat, buttocks on its top; each foot on the floor, or hanging when it cannot reach."""
    rig, fwd = body.rig, body.fwd
    ankle = {s: float(rig.heads[sk.j(s + "Foot")][1] - body.floor) for s in ("Left", "Right")}
    contact = np.isin(rig.dominant, [sk.j("Hips"), sk.j("LeftUpperLeg"), sk.j("RightUpperLeg")])
    for side in ("Left", "Right"):
        u, l, _, _ = _leg(sk, side)
        sk.aim(u, l, fwd - 0.05 * UP)
    _lean(sk, body, -6)
    top = body.floor + seat
    for _ in range(3):
        pts = rig.skin(sk.world())[contact]
        sk.move(UP * (top - float(pts[:, 1].min())))
        W = sk.world()
        for side in ("Left", "Right"):
            u, l, f, toes = _leg(sk, side)
            l2 = _lengths(rig, sk, side)[1]
            dz = W[l][1, 3] - (body.floor + ankle[side])
            ahead = np.sqrt(max(l2 * l2 - dz * dz, 0.0))
            sk.aim(l, f, fwd * ahead - UP * dz if dz < l2 else -UP)
            _flat(sk, f)
    return contact


def kneel(sk, body):
    fwd = body.fwd
    u, l, f, toes = _leg(sk, "Left")
    sk.aim(u, l, -UP + 0.1 * fwd)
    sk.aim(l, f, -fwd - 0.05 * UP)
    if toes is not None:
        sk.aim(f, toes, -fwd - 0.6 * UP)
    u, l, f, toes = _leg(sk, "Right")
    sk.aim(u, l, fwd - 0.1 * UP)
    sk.aim(l, f, -UP + 0.05 * fwd)
    _flat(sk, f)


def crouch(sk, body):
    rig, fwd = body.rig, body.fwd
    l1, l2 = _lengths(rig, sk, "Left")
    _lean(sk, body, 22)
    for side in ("Left", "Right"):
        u, l, f, toes = _leg(sk, side)
        W = sk.world()
        hip = W[u][:3, 3]
        ankle = hip - UP * 0.55 * (l1 + l2) + fwd * 0.15 * (l1 + l2)
        knee = _knee(hip, ankle, l1, l2, fwd)
        sk.aim(u, l, knee - hip)
        sk.aim(l, f, ankle - sk.world()[l][:3, 3])
        _flat(sk, f)


def lie(sk, body):
    sk.turn(sk.root, axis_angle(body.left, np.radians(-90)))    # head back, face up


def peek(sk, body):
    """Stay low with the body extending back into cover and the face lifted out of it."""
    sk.turn(sk.root, axis_angle(body.left, np.radians(77)))
    head, neck = body.head(sk)
    if neck is not None:
        sk.turn(neck, axis_angle(body.left, np.radians(-38)))
    if head is not None:
        sk.turn(head, axis_angle(body.left, np.radians(-34)))
    for side, sign in (("Left", 1), ("Right", -1)):
        shoulder, elbow, wrist = [sk.j(side + name) for name in ("UpperArm", "LowerArm", "Hand")]
        sk.aim(shoulder, elbow, -body.fwd + sign * .28 * body.left - .20*UP)
        sk.aim(elbow, wrist, -.3*body.fwd + sign*.20*body.left + .10*UP)


def curl(sk, body):
    fwd = body.fwd
    for side in ("Left", "Right"):
        u, l, f, _ = _leg(sk, side)
        sk.aim(u, l, fwd + 0.15 * UP)
        sk.aim(l, f, -fwd - 0.6 * UP)
        sk.aim(sk.j(side + "UpperArm"), sk.j(side + "LowerArm"), fwd - 0.6 * UP)
        sk.aim(sk.j(side + "LowerArm"), sk.j(side + "Hand"), fwd)
    _lean(sk, body, 25)
    if sk.j("Head") is not None:
        sk.turn(sk.j("Head"), axis_angle(body.left, np.radians(20)))
    hips = sk.j("Hips")
    for c in sk.rig.children[hips] if hips is not None else []:   # a tail (an animal on a two-legged rig)
        chain = [c]
        while len(sk.rig.children[chain[-1]]) == 1:
            chain.append(sk.rig.children[chain[-1]][0])
        if sk.rig.names[c] in ("Spine", "LeftUpperLeg", "RightUpperLeg") or len(chain) < 3:
            continue
        for i, (a, b) in enumerate(zip(chain, chain[1:])):   # round the feet, in what becomes the floor's plane
            t = np.pi * i / max(len(chain) - 2, 1)
            sk.aim(a, b, -np.cos(t) * fwd - np.sin(t) * UP)
    sk.turn(sk.root, axis_angle(fwd, np.radians(90)))           # onto its right side


def _palm_frame(sk, side, W):
    """(direction along the hand, palm normal) from the finger roots, or None without fingers."""
    hand, index = sk.j(side + "Hand"), sk.j(side + "IndexProximal")
    little, middle = sk.j(side + "LittleProximal"), sk.j(side + "MiddleProximal")
    if None in (hand, index, little, middle):
        return None
    along = _unit(W[middle][:3, 3] - W[hand][:3, 3])
    across = _unit(W[little][:3, 3] - W[index][:3, 3])
    return along, _unit(np.cross(along, across) if side == "Right" else np.cross(across, along))


def hold_out(sk, body, hand):
    side = "Right" if hand == "right" else "Left"
    out = -body.left if side == "Right" else body.left
    fwd = body.fwd
    sk.aim(sk.j(side + "UpperArm"), sk.j(side + "LowerArm"), fwd - 0.45 * UP + 0.12 * out)
    sk.aim(sk.j(side + "LowerArm"), sk.j(side + "Hand"), fwd + 0.15 * UP)
    k = sk.j(side + "Hand")
    sk.aim(k, _child(sk, k, (side + "MiddleProximal",)), fwd + 0.05 * UP)
    W = sk.world()
    frame = _palm_frame(sk, side, W)
    if frame is not None:   # roll the hand about its length until the palm faces up
        along, n = frame
        want = _unit(UP - (UP @ along) * along)
        have = _unit(n - (n @ along) * along)
        sk.turn(k, axis_angle(along, np.arctan2(np.cross(have, want) @ along, have @ want)), W)



FINGERS = ("Index", "Middle", "Ring", "Little")
CURL_DEG = {"Proximal": 70, "Intermediate": 80, "Distal": 55}   # a loose fist
WRIST_DEG = 65   # how far the wrist turns the fist towards a target


def _close_fist(sk, side):
    """Fold every finger towards the palm and lay the thumb across it; nothing to do without finger joints."""
    W = sk.world()
    frame = _palm_frame(sk, side, W)
    if frame is None:
        return
    along, n = frame
    bend = _unit(np.cross(along, n))   # turning a finger about this axis folds it towards the palm side
    for finger in FINGERS:
        for seg in ("Proximal", "Intermediate", "Distal"):
            k = sk.j(side + finger + seg)
            if k is not None:
                sk.turn(k, axis_angle(bend, np.radians(CURL_DEG[seg])))
    for seg, deg in (("Proximal", 25), ("Intermediate", 35), ("Distal", 30)):
        k = sk.j(side + "Thumb" + seg)
        if k is not None:
            sk.turn(k, axis_angle(bend, np.radians(deg)))


def _fist_frame(sk, side, W):
    """(centre of the closed fist where a handle passes, the handle's line towards the thumb side, palm normal)."""
    hand = sk.j(side + "Hand")
    index, little, middle = (sk.j(side + f + "Proximal") for f in ("Index", "Little", "Middle"))
    if None in (index, little, middle):   # no finger joints: the middle of the hand's own bone
        c = _child(sk, hand)
        end = W[c][:3, 3] if c is not None else W[hand][:3, 3] + _rot(W[hand])[:, 1] * 0.05
        along = _unit(end - W[hand][:3, 3])
        return W[hand][:3, 3] + 0.5 * (end - W[hand][:3, 3]), None, along
    root = (W[index][:3, 3] + W[little][:3, 3]) / 2
    length = float(np.linalg.norm(W[middle][:3, 3] - W[hand][:3, 3]))
    along = _unit(W[middle][:3, 3] - W[hand][:3, 3])
    across = _unit(W[index][:3, 3] - W[little][:3, 3])   # little-finger side -> thumb side
    n = _unit(np.cross(along, -across) if side == "Right" else np.cross(-across, along))
    return root + n * 0.3 * length, across, along


def hold(sk, body, hand, style="raise", target=None):
    """An arm brought forward and bent with a closed fist: raise holds a stick up and forward (towards a
    target when given, within a wrist's reach); carry holds a hanging load away from the body."""
    side = "Right" if hand == "right" else "Left"
    out = -body.left if side == "Right" else body.left
    fwd = body.fwd
    upper, lower, k = sk.j(side + "UpperArm"), sk.j(side + "LowerArm"), sk.j(side + "Hand")
    if style == "carry":
        sk.aim(upper, lower, -UP + 0.35 * fwd + 0.3 * out)
        sk.aim(lower, k, fwd + 0.25 * out - 0.05 * UP)
    else:
        sk.aim(upper, lower, fwd - 0.8 * UP + 0.2 * out)
        forearm = fwd + 0.35 * UP + 0.1 * out
        if target is not None:   # the forearm turns half way towards the target, never back past the side
            elbow = sk.world()[lower][:3, 3]
            want = _unit(target - elbow)
            if want @ fwd > -0.2:
                forearm = _unit(_unit(forearm) + want)
        sk.aim(lower, k, forearm)
    sk.aim(k, _child(sk, k, (side + "MiddleProximal",)), sk.world()[k][:3, 3] - sk.world()[lower][:3, 3])
    _close_fist(sk, side)
    W = sk.world()
    point, across, along = _fist_frame(sk, side, W)
    if across is None:
        return
    if style == "carry":
        want = UP   # thumb on top, the bail across the fist, the load hanging below
    elif target is not None:
        want = _unit(target - point)
    else:
        want = _unit(fwd + UP)
    # roll about the forearm first, then bend the wrist the rest of the way (within WRIST_DEG)
    flat_want, flat_have = want - (want @ along) * along, across - (across @ along) * along
    if np.linalg.norm(flat_want) > 1e-6 and np.linalg.norm(flat_have) > 1e-6:
        a, b = _unit(flat_have), _unit(flat_want)
        sk.turn(k, axis_angle(along, np.arctan2(np.cross(a, b) @ along, a @ b)), W)
        W = sk.world()
        point, across, along = _fist_frame(sk, side, W)
    R = rotation_between(across, want)
    angle = np.degrees(np.arccos(np.clip(across @ want, -1, 1)))
    if angle > 1e-3:
        sk.turn(k, _slerp_matrix(R, min(1.0, WRIST_DEG / angle)), W)


def grip(sk, body, hand, style, target):
    """The manifest's grip: where a handle passes through the fist, which way it runs, and where a 1 m stick
    held that way would reach."""
    side = "Right" if hand == "right" else "Left"
    W = sk.world()
    point, across, along = _fist_frame(sk, side, W)
    top = across if across is not None else (_unit(target - point) if target is not None else _unit(body.fwd + UP))
    axis = -UP if style == "carry" else top
    tip = point + axis * 1.0
    toward = None
    if target is not None:
        s = float(np.clip((target - point) @ axis, 0.0, 1.0))
        toward = round(float(np.linalg.norm(target - (point + axis * s))), 4)
    return {"hand": side.lower(), "style": style, "point": _r(point), "axis": _r(axis), "up": _r(top),
            "reach": {"tip_1m": _r(tip), "toward_m": toward}}


# ---------------------------------------------------------------- four-legged poses

def _legs(body):
    chains = body.plan["leg_chains"]
    centre = np.mean([body.rig.heads[g["chain"][0]] for g in chains], axis=0)
    front = [g for g in chains if (body.rig.heads[g["chain"][0]] - centre) @ body.fwd > 0]
    return front, [g for g in chains if g not in front]


def _ground(sk, body, level):
    sk.move(UP * (level - float(body.rig.skin(sk.world())[:, 1].min())))


def quad_sit(sk, body, level):
    """On the haunches, like a sitting dog: the body pitches nose-up (15-50 degrees) about the hind hips, the
    rump rests on the surface with the hind legs folded flat beside it, and the straight front legs stand on
    it too. Front legs longer than the pitch allows plant their paws a little ahead of the chest instead of
    tipping the body upright. The tail slopes back down to the surface."""
    rig, fwd = body.rig, body.fwd
    front, hind = _legs(body)
    paws = np.isin(rig.dominant, [j for g in front for j in g["chain"][-2:]])
    carriers = {k for g in hind for k in rig.path_up(g["chain"][0])}
    tail = [k for k in body.plan.get("tail") or [] if k not in carriers]
    while tail and len(rig.children[tail[-1]]) == 1:   # a tail tip the body plan named as a limb
        tail.append(rig.children[tail[-1]][0])
    pelvis = {rig.parent[g["chain"][0]] for g in hind} - {-1}
    rump = np.isin(rig.dominant, [g["chain"][0] for g in hind] + sorted(pelvis) + tail[:1])
    feet = np.isin(rig.dominant, [j for g in hind for j in g["chain"][-2:]])
    start = sk.q.copy(), [t.copy() for t in sk.T]

    def hips(W):
        return np.mean([W[g["chain"][0]][:3, 3] for g in hind], axis=0)

    def fold(shin):   # thigh forward, shin back and down by `shin`, the hind foot flat and forward
        for g in hind:
            c = g["chain"]
            sk.aim(c[0], c[1] if len(c) > 1 else _child(sk, c[0]), fwd - 0.25 * UP)
            if len(c) > 2:
                sk.aim(c[1], c[2], -fwd - shin * UP)
                for a, b in zip(c[2:], c[3:]):
                    sk.aim(a, b, fwd - 0.05 * UP)

    best = None
    for deg in range(15, 51, 5):
        sk.q, sk.T = start[0].copy(), [t.copy() for t in start[1]]
        before = hips(sk.world())
        sk.turn(sk.root, axis_angle(body.left, np.radians(-deg)))   # nose up about the hind hips,
        sk.move(before - hips(sk.world()))                           # wherever the rig's root joint is
        for g in front:
            for a, b in zip(g["chain"], g["chain"][1:]):
                sk.aim(a, b, -UP + 0.08 * fwd)
        miss = []
        for shin in (0.2, 0.5, 0.9, 1.5, 3.0):   # the hind feet reach as low as the rump
            fold(shin)
            pts = rig.skin(sk.world())
            miss.append(abs(float(pts[feet][:, 1].min() - pts[rump][:, 1].min())) if feet.any() else 0.0)
        fold((0.2, 0.5, 0.9, 1.5, 3.0)[int(np.argmin(miss))])
        pts = rig.skin(sk.world())
        seat = float(min(pts[rump][:, 1].min(), pts[feet][:, 1].min() if feet.any() else np.inf))
        sk.move(UP * (level - seat))                                 # the rump on the surface
        gap = float(pts[paws][:, 1].min()) - seat if paws.any() else 0.0
        if best is None or abs(gap) < abs(best[0]):
            best = (gap, sk.q.copy(), [t.copy() for t in sk.T])
    gap, sk.q, sk.T = best
    if gap < 0:   # the front paws would push through the surface: lean the legs forward by the difference
        W = sk.world()
        reach = float(np.mean([W[g["chain"][0]][1, 3] for g in front])) - float(rig.skin(W)[paws][:, 1].min())
        lean = np.arccos(np.clip(1 + gap / reach, np.cos(np.radians(40)), 1.0))
        for g in front:
            for a, b in zip(g["chain"], g["chain"][1:]):
                sk.aim(a, b, -np.cos(lean) * UP + np.sin(lean) * fwd)
    tail_skin = np.isin(rig.dominant, tail[1:])
    for deg in (60, 45, 30, 15, 0):   # the tail slopes back down as steeply as it can without going under
        for a, b in zip(tail[1:], tail[2:]):
            sk.aim(a, b, -np.cos(np.radians(deg)) * fwd - np.sin(np.radians(deg)) * UP)
        if not tail_skin.any() or float(rig.skin(sk.world())[tail_skin][:, 1].min()) >= level:
            break
    _ground(sk, body, level)


def quad_lie(sk, body, level, curl_it=False):
    """lie: on the belly, front legs forward (a sphinx), hind legs folded, head a little low, tail along the floor.
    curl: a sleeping ball, legs tucked under, the spine bent round so the head meets the tail, which wraps it.
    A rig made sitting up is first tipped forward until its back is level."""
    rig, fwd = body.rig, body.fwd
    front, hind = _legs(body)
    carriers = {k for g in hind for k in rig.path_up(g["chain"][0])}
    tail = [k for k in body.plan.get("tail") or [] if k not in carriers]
    head = body.plan.get("head")
    if head is not None and hind:
        W = sk.world()
        hips = np.mean([W[g["chain"][0]][:3, 3] for g in hind], axis=0)
        back = W[head][:3, 3] - hips
        rise = np.degrees(np.arctan2(back @ UP, np.linalg.norm(back - (back @ UP) * UP)))
        if rise > 15:   # level the back, about the hind hips
            sk.turn(sk.root, axis_angle(body.left, np.radians(rise - 10)))
            W2 = sk.world()
            sk.move(hips - np.mean([W2[g["chain"][0]][:3, 3] for g in hind], axis=0))
    for g in front:
        c, heads = g["chain"], rig.heads[g["chain"]]
        seg = np.linalg.norm(np.diff(heads, axis=0), axis=1)
        if len(c) > 3 and seg[0] < 0.25 * seg.sum():   # a short shoulder link before the upper arm
            c = c[1:]
        if curl_it:   # tucked: elbow back under the chest, forearm and paw folded back
            dirs = [-0.6 * UP - fwd] + [-fwd - 0.1 * UP] * len(c)
        else:         # a sphinx: elbow down behind the shoulder, forearm and paw forward on the floor
            dirs = [-0.6 * UP - 0.8 * fwd] + [fwd - 0.08 * UP] * len(c)
        for (a, b), d in zip(zip(c, c[1:]), dirs):
            sk.aim(a, b, d)
    for g in hind:   # thigh forward along the side, shin back, foot forward
        c = g["chain"]
        dirs = [fwd - 0.3 * UP, -fwd - 0.25 * UP] + [fwd - 0.08 * UP] * len(c)
        if len(c) == 1:
            sk.aim(c[0], _child(sk, c[0]), fwd - 0.3 * UP)
        for (a, b), d in zip(zip(c, c[1:]), dirs):
            sk.aim(a, b, d)
    for a, b in zip(tail, tail[1:]):
        sk.aim(a, b, -fwd - 0.15 * UP)
    if curl_it:   # bend the body round to its left, head down and round towards the tail; the tail follows
        spine = [k for k in body.plan.get("spine") or [] if k != sk.root and k not in carriers and k != head]
        spine += [head] if head is not None else []
        for k in spine:
            sk.turn(k, axis_angle(UP, np.radians(150.0 / max(len(spine), 1))))
        for k in tail:
            sk.turn(k, axis_angle(UP, np.radians(35)))
    if head is not None:
        sk.turn(head, axis_angle(_unit(np.cross(UP, _gaze(sk, body, sk.world()))), np.radians(-20 if curl_it else -10)))
    _ground(sk, body, level)


# ---------------------------------------------------------------- look-at, breathing, checks

def _gaze(sk, body, W, k=None):
    h = body.head(sk)[0] if k is None else k
    return _unit(_rot(W[h]) @ _rot(body.rig.rest_world[h]).T @ body.fwd)


def look_at(sk, body, target):
    """Turn the neck (40%) and the head towards a point, within a neck's reach of the way the chest faces."""
    h, neck = body.head(sk)
    W = sk.world()
    want = _unit(target - W[h][:3, 3])
    chest = _gaze(sk, body, W, neck if neck is not None else h)
    flat = _unit(chest - (chest @ UP) * UP)
    wflat = _unit(want - (want @ UP) * UP)
    yaw = float(np.clip(np.arctan2(np.cross(flat, wflat) @ UP, flat @ wflat), -np.radians(75), np.radians(75)))
    pitch = float(np.clip(np.arcsin(np.clip(want @ UP, -1, 1)), -np.radians(45), np.radians(60)))
    want = axis_angle(UP, yaw) @ (np.cos(pitch) * flat + np.sin(pitch) * UP)
    for k, share in ((neck, 0.4), (h, 1.0)):
        if k is not None:
            W = sk.world()
            sk.turn(k, _slerp_matrix(rotation_between(_gaze(sk, body, W), want), share), W)


def breathe(sk, body):
    """A periodic clip: the chest rises and falls by about a degree; the hips, the feet and the root stay still."""
    if body.humanoid:
        joints = [k for k in (sk.j("Chest"), sk.j("UpperChest")) if k is not None]
    else:
        spine = (body.plan or {}).get("spine") or []
        joints = spine[len(spine) // 2: len(spine) // 2 + 1]
    frames = int(BREATH_S * FPS)
    times = np.arange(frames + 1) / FPS
    base, quats = sk.q.copy(), []
    for t in times[:-1]:
        sk.q = base.copy()
        a = np.radians(BREATH_DEG) * np.sin(2 * np.pi * t / BREATH_S) / max(len(joints), 1)
        for k in joints:
            sk.turn(k, axis_angle(body.left, -a))
        quats.append(sk.q.copy())
    sk.q = base
    quats = np.array(quats + [quats[0]])
    for f in range(1, len(quats)):
        quats[f][(quats[f] * quats[f - 1]).sum(axis=1) < 0] *= -1
    quats[-1] = quats[0]
    return times, quats, np.tile(sk.T[sk.root], (len(times), 1)), np.tile(sk.S[sk.root], (len(times), 1))


def _skinned(doc):
    """(primitive, its vertex offset into rig.bind) for every skinned primitive, in Rig's order."""
    base = 0
    for node in doc["nodes"]:
        if node.get("skin") != 0 or "mesh" not in node:
            continue
        for prim in doc["meshes"][node["mesh"]]["primitives"]:
            at = prim.get("attributes", {})
            if "JOINTS_0" in at and "WEIGHTS_0" in at:
                yield prim, base
                base += doc["accessors"][at["POSITION"]]["count"]


def triangles(rig):
    """Index triples into rig.bind, collected in the same order as Rig collects the skinned primitives."""
    doc, out = rig.doc, []
    for prim, base in _skinned(doc):
        if prim.get("mode", 4) == 4:
            count = doc["accessors"][prim["attributes"]["POSITION"]]["count"]
            idx = (accessor(doc, rig.binary, prim["indices"]).astype(int).ravel() if "indices" in prim
                   else np.arange(count))
            out.append(idx.reshape(-1, 3) + base)
    return np.concatenate(out) if out else np.zeros((0, 3), int)


def torn(body, posed, tris):
    """Skin triangles the pose tears into spikes: an edge stretched past STRETCH times its rest length and
    longer by more than TEAR of the body's height."""
    def edges(p):
        return np.linalg.norm(p[tris] - p[np.roll(tris, -1, axis=1)], axis=2)
    rest, now = edges(body.rig.skin_rest), edges(posed)
    return ((now > STRETCH * rest) & (now - rest > TEAR * body.height)).any(axis=1)


def seams(rig, tris):
    """Triangles fusing body parts four or more bones apart (a forearm generated touching the thigh): the
    skin between them is not skin, it is where two parts that move apart were modelled touching."""
    n = len(rig.names)
    line = np.zeros((n, n), bool)   # line[k]: k and its ancestors
    for k in rig.order:
        if rig.parent[k] >= 0:
            line[k] = line[rig.parent[k]]
        line[k, k] = True
    apart = (line[:, None, :] != line[None, :, :]).sum(axis=2)   # bones between two joints
    d = rig.dominant[tris]
    return np.maximum.reduce([apart[d[:, 0], d[:, 1]], apart[d[:, 1], d[:, 2]], apart[d[:, 0], d[:, 2]]]) >= 4


def _areas(p, tris):
    return 0.5 * np.linalg.norm(np.cross(p[tris[:, 1]] - p[tris[:, 0]], p[tris[:, 2]] - p[tris[:, 0]]), axis=1)


def check(body, posed, tris, level, contact=None, seat=None):
    """Skin under the surface the body rests on, skin inside the seat (under the part of the body resting on
    it, from the back of the seat to three quarters of the way to the knees), collapsed skin triangles and
    torn ones (stretched into spikes; any torn triangle fusing two body parts four or more bones apart fails)."""
    H = body.height
    rest = _areas(body.rig.skin_rest, tris)
    real = rest > 1e-12
    ratio = _areas(posed, tris)[real] / rest[real]
    collapsed = float((ratio < 0.2).mean()) if len(ratio) else 0.0
    tears = torn(body, posed, tris)
    tear = float(tears.mean()) if len(tris) else 0.0
    fused = int((tears & seams(body.rig, tris)).sum())
    under = max(0.0, level - float(posed[:, 1].min()))
    seat_pen = None
    if seat is not None and contact is not None:
        top = body.floor + seat
        pts = posed[contact]
        rest_on = pts[pts[:, 1] <= pts[:, 1].min() + 0.01 * H]
        across, ahead = posed @ body.left, posed @ body.fwd
        a, f = rest_on @ body.left, rest_on @ body.fwd
        front = f.min() + 0.75 * (f.max() - f.min())
        over = (across > a.min()) & (across < a.max()) & (ahead > f.min()) & (ahead < front) & ~contact
        seat_pen = max(0.0, top - float(pts[:, 1].min()))
        if over.any():
            seat_pen = max(seat_pen, float(np.clip(top - posed[over][:, 1], 0, None).max()))
    reasons = []
    if collapsed > LIMITS["collapsed_share"]:
        reasons.append(f"{collapsed:.1%} of the skin triangles collapse (limit {LIMITS['collapsed_share']:.0%})")
    if fused:
        reasons.append(f"{fused} skin triangles fuse body parts the pose pulls apart and tear into spikes")
    if tear > LIMITS["torn_share"]:
        reasons.append(f"{tear:.2%} of the skin triangles tear into spikes (limit {LIMITS['torn_share']:.2%})")
    if under > LIMITS["floor_penetration"] * H:
        reasons.append(f"skin {under:.3f} m under the floor (limit {LIMITS['floor_penetration'] * H:.3f} m)")
    if seat_pen is not None and seat_pen > LIMITS["seat_penetration"] * H:
        reasons.append(f"skin {seat_pen:.3f} m inside the seat (limit {LIMITS['seat_penetration'] * H:.3f} m)")
    return {"pass": not reasons, "floor_penetration_m": round(under, 4),
            "seat_penetration_m": None if seat_pen is None else round(seat_pen, 4),
            "collapsed_share": round(collapsed, 5), "torn_share": round(tear, 5), "reasons": reasons}


def _ray_hits(origin, direction, p, tris):
    """Distances along a ray to every skin triangle it crosses, with each triangle's unit normal."""
    a, b, c = p[tris[:, 0]], p[tris[:, 1]], p[tris[:, 2]]
    e1, e2 = b - a, c - a
    h = np.cross(direction, e2)
    det = (e1 * h).sum(axis=1)
    ok = np.abs(det) > 1e-12
    inv = np.where(ok, 1.0 / np.where(ok, det, 1.0), 0.0)
    s = origin - a
    u = (s * h).sum(axis=1) * inv
    q = np.cross(s, e1)
    v = (q @ direction) * inv
    t = (e2 * q).sum(axis=1) * inv
    hit = ok & (u >= 0) & (v >= 0) & (u + v <= 1)
    normals = np.cross(e1, e2)[hit]
    return t[hit], normals / np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-12)


def palm_surface(sk, rig, W, posed, tris, side):
    """The open palm's skin at its hollow (62% of the way from the wrist to the middle finger's root) in the posed
    mesh, a millimetre out, and the skin's normal there pointing away from the back of the hand.

    Seen from the palm side: a ray coming down the palm normal from a hand's length above the palm meets the
    skin a viewer sees there first (generated meshes can be double-walled, so crossings from the back of the
    hand are not counted)."""
    k = sk.j(side + "Hand")
    frame = _palm_frame(sk, side, W)
    middle = sk.j(side + "MiddleProximal")
    if frame is not None and middle is not None:
        along, n = frame
        centre = W[k][:3, 3] + 0.62 * (W[middle][:3, 3] - W[k][:3, 3])   # the palm's hollow
        size = float(np.linalg.norm(W[middle][:3, 3] - W[k][:3, 3]))
    else:   # no finger joints: the hand's own skin, facing up
        pts = posed[rig.dominant == k]
        centre = pts.mean(axis=0) if len(pts) else W[k][:3, 3]
        size = float(np.ptp(pts, axis=0).max()) if len(pts) else 0.1
        n = UP
    side_a = _unit(np.cross(n, [1.0, 0, 0] if abs(n[0]) < 0.9 else [0, 0, 1.0]))
    side_b = np.cross(n, side_a)
    hits = []   # the middle ray, then a ring of eight a sixth of the palm out: their normals are averaged
    for angle in [None] + list(np.linspace(0, 2 * np.pi, 8, endpoint=False)):
        offset = np.zeros(3) if angle is None else size / 6 * (np.cos(angle) * side_a + np.sin(angle) * side_b)
        origin = centre + offset + n * size
        t, normals = _ray_hits(origin, -n, posed, tris)
        keep = (t > 0) & (t < 2 * size)
        if keep.any():
            first = np.flatnonzero(keep)[np.argmin(t[keep])]
            hits.append((origin - n * t[first], normals[first] if normals[first] @ n > 0 else -normals[first]))
        elif angle is None:
            return centre, n
    normal = _unit(np.mean([h[1] for h in hits], axis=0))
    return hits[0][0] + normal * 0.001, normal


def _frame_point(body, xyz):
    """A point given in the character's own frame (x its left, y up from its floor, z the way it faces)."""
    if xyz is None:
        return None
    return xyz[0] * body.left + (body.floor + xyz[1]) * UP + xyz[2] * body.fwd


def _r(v):
    return [round(float(x), 4) for x in v]


def write(rig, clip_name, baked, out, cut=None):
    """The input GLB with the pose clip inserted first; every other existing clip is kept. cut: a mask over
    triangles(rig) of skin triangles to leave out of the mesh."""
    doc = json.loads(json.dumps(rig.doc))
    binary = bytearray(rig.binary)
    root = rig.nodes[next(k for k in rig.order if rig.parent[k] < 0)]

    def add(array, kind, minmax=False, dtype="<f4", component=5126):
        data = np.ascontiguousarray(array, dtype=dtype)
        binary.extend(b"\0" * (-len(binary) % 4))
        doc.setdefault("bufferViews", []).append({"buffer": 0, "byteOffset": len(binary), "byteLength": data.nbytes})
        binary.extend(data.tobytes())
        acc = {"bufferView": len(doc["bufferViews"]) - 1, "componentType": component, "count": len(data), "type": kind}
        if minmax:
            acc.update(min=[float(data.min())], max=[float(data.max())])
        doc.setdefault("accessors", []).append(acc)
        return len(doc["accessors"]) - 1

    if cut is not None and cut.any():
        tris, first = triangles(rig), 0
        for prim, base in _skinned(doc):
            if prim.get("mode", 4) != 4:
                continue
            count = (doc["accessors"][prim["indices"]]["count"] if "indices" in prim
                     else doc["accessors"][prim["attributes"]["POSITION"]]["count"]) // 3
            keep = tris[first:first + count][~cut[first:first + count]] - base
            prim["indices"] = add(keep.ravel(), "SCALAR", dtype="<u4", component=5125)
            first += count

    times, quats, rt, rs = baked
    anim = {"name": clip_name, "samplers": [], "channels": []}
    tin = add(times, "SCALAR", minmax=True)
    tracks = [(rig.nodes[j], "rotation", quats[:, j], "VEC4") for j in range(len(rig.nodes))]
    tracks += [(root, "translation", rt, "VEC3"), (root, "scale", rs, "VEC3")]
    for node, path, values, kind in tracks:
        anim["samplers"].append({"input": tin, "output": add(values, kind), "interpolation": "LINEAR"})
        anim["channels"].append({"sampler": len(anim["samplers"]) - 1, "target": {"node": node, "path": path}})
    doc["animations"] = [anim] + [a for a in doc.get("animations", []) if a.get("name") != clip_name]
    binary.extend(b"\0" * (-len(binary) % 4))
    doc["buffers"] = [{"byteLength": len(binary)}]
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    Path(out).write_bytes(struct.pack("<4sII", b"glTF", 2, 28 + len(text) + len(binary))
                          + struct.pack("<I4s", len(text), b"JSON") + text
                          + struct.pack("<I4s", len(binary), b"BIN\0") + bytes(binary))


def make(mesh, out, names, seat=None, look=None, hand=None, hold_style=None, hold_toward=None,
         progress=lambda pct, msg: None, wind_mesh=None, wind_scale=1.0, other_style=None, other_toward=None, fingerboard=None, strings=None, plant=None, strike=None, swimming_fish=False):
    """Pose the rigged GLB at mesh and write it to out. Returns the manifest additions (check["pass"] False:
    nothing written). Raises Unsupported for a pose this body cannot take, ValueError for bad input."""
    rig = Rig(mesh)
    if swimming_fish:
        rig.doc["skins"][0].setdefault("extras", {})["swimming_fish"] = True
    body = Body(rig)
    sk = Skeleton(rig)
    names = list(dict.fromkeys(names))
    needs_hands = any(n in names for n in ("hold", "hold-out")) or any(
        target is not None for target in (wind_mesh, fingerboard, plant))
    if needs_hands and not body.humanoid and not body.arm_joints:
        body.arm_joints = body.paired_paws()
    sk.idx.update(body.arm_joints)
    for n in names:
        if n not in POSES:
            raise ValueError(f"unknown pose {n!r}; one of: {', '.join(POSES)}")
    held = [n for n in names if n not in ("hold-out", "hold", "cross-arms", "stand")]
    if len(held) > 1:
        raise ValueError("give one body pose (stand, sit, kneel, crouch, lie or curl), plus hold-out or hold")
    if "cross-arms" in names and ("hold" in names or "hold-out" in names):
        raise ValueError("crossed arms cannot also hold an object")
    if "hold" in names and "hold-out" in names:
        raise ValueError("choose hold or hold-out")
    if hold_style not in (None, "raise", "carry"):
        raise ValueError("--hold-style is raise or carry")
    if "hold" not in names and (hold_style or hold_toward is not None):
        raise ValueError("--hold-style and --hold-toward need the hold pose")
    if hold_style == "carry" and hold_toward is not None:
        raise ValueError("--hold-toward aims a raised stick; a carried load hangs straight down")
    if other_style is not None and ("hold" not in names or other_style not in ("raise", "carry")):
        raise ValueError("a second hand requires hold and a raise or carry style")
    if other_toward is not None and other_style != "raise":
        raise ValueError("only a raised second hand can aim at a target")
    opposite = "left" if (hand or "right") == "right" else "right"
    style = hold_style or "raise"
    main = held[0] if held else "stand"
    if ("hold" in names or "hold-out" in names) and main in ("lie", "curl"):
        raise ValueError("hold and hold-out go with stand, sit, kneel or crouch")
    head, _ = body.head(sk)
    allowed = body.supported()
    for n in names:
        if n not in allowed:
            have = sorted(allowed) + (["look-at"] if head is not None else [])
            raise Unsupported(f"pose {n!r} is not supported for a {body.kind} body (supported here: {', '.join(have)})")
    if look is not None and head is None:
        raise Unsupported(f"look-at needs a head; none was found on this {body.kind} body")
    if main == "sit" and body.humanoid and seat is None:
        raise ValueError("sit needs --seat-height (the seat top above the character's floor, in metres)")
    if seat is not None and not 0 <= seat <= 2 * body.height:
        raise ValueError("--seat-height must be between 0 and twice the character's height")
    progress(20, f"posing a {body.kind} body: {', '.join(names)}")
    contact, level, feet = None, body.floor, "touch"
    if body.humanoid or (body.arm_joints and ("hold" in names or "hold-out" in names)):
        if main == "sit":
            contact = sit(sk, body, seat)
        elif main != "stand":
            {"kneel": kneel, "crouch": crouch, "lie": lie, "curl": curl, "lean": lean, "peek": peek}[main](sk, body)
        if "cross-arms" in names:
            cross_arms(sk, body)
        if "hold-out" in names:
            hold_out(sk, body, hand or "right")
        if "hold" in names:
            hold(sk, body, hand or "right", style, _frame_point(body, hold_toward))
            if other_style:
                hold(sk, body, opposite, other_style, _frame_point(body, other_toward))
        if main != "sit":
            _ground(sk, body, body.floor)
    elif body.kind == "quadruped":
        level = body.floor + (seat or 0.0)
        if main == "sit":
            quad_sit(sk, body, level)
        elif main in ("lie", "curl", "peek"):
            quad_lie(sk, body, level, curl_it=main == "curl")
        else:
            _ground(sk, body, level)
    else:
        feet = "touch" if (body.plan or {}).get("legs") else "none"
    if look is not None:
        look_at(sk, body, _frame_point(body, look))
    if plant is not None:
        if not (body.humanoid or body.arm_joints):
            raise Unsupported("planting a shaft needs an articulated hand")
        side = "Left" if hand == "left" else "Right"
        if body.humanoid:
            _lean(sk, body, 20)
        target = _frame_point(body, plant)
        hold(sk, body, side.lower(), "raise")
        wrist = sk.j(side+"Hand")
        for _ in range(4):
            W = sk.world()
            point, _, _ = _fist_frame(sk, side, W)
            reach_hand(sk, side, W[wrist][:3,3] + target - point, -UP + body.left * (0.4 if side=="Left" else -0.4))
    if (fingerboard is None) != (strings is None):
        raise ValueError("both fingerboard and string contact points are needed")
    if fingerboard is not None:
        if not (body.humanoid or body.arm_joints):
            raise Unsupported("playing upright strings needs two articulated arms")
        for side, target, sign in (("Left", fingerboard, 1), ("Right", strings, -1)):
            point = _frame_point(body, target)
            wrist = sk.j(side + "Hand")
            digit = _child(sk, wrist, (side + "MiddleProximal",))
            toward = _unit(point-sk.world()[sk.j(side+"UpperArm")][:3,3])
            if digit is None and sk.rig.children[wrist]:
                # Use the knuckle nearest the centre of an unnamed finger fan.
                w = sk.world()[:, :3, 3]
                tips = sk.rig.children[wrist]
                centre = w[tips].mean(axis=0)
                digit = min(tips, key=lambda k: np.linalg.norm(w[k] - centre))
            span = np.linalg.norm(sk.world()[digit][:3,3]-sk.world()[wrist][:3,3]) if digit is not None else 0.0
            reach_hand(sk, side, point-toward*span, -UP + sign*body.left*0.55)
            sk.aim(wrist, digit, point-sk.world()[wrist][:3,3])
    tapping = None
    if strike is not None:
        from forge import percussion
        tapping = percussion.strike(sk,body,[_frame_point(body,t) for t in strike])
    mouth = None
    if head is not None and (body.humanoid or body.arm_joints):
        W = sk.world()
        gaze = _unit(_gaze(sk, body, W))
        lip = W[head][:3,3] + UP * (0.018 * body.height)
        origin = lip + gaze * body.height
        distances, _ = _ray_hits(origin, -gaze, body.rig.skin(W), triangles(body.rig))
        mouth = origin - gaze * np.min(distances) if len(distances) else lip + gaze * 0.07 * body.height
    mount = None
    if wind_mesh is not None:
        if mouth is None:
            raise Unsupported("playing a wind instrument needs a mouth and two articulated arms")
        if not np.isfinite(wind_scale) or wind_scale <= 0:
            raise ValueError("wind instrument scale must be positive")
        from forge import wind
        mount, grips = wind.fit(wind_mesh, mouth, gaze, body.left, body.height, wind_scale)
        for side, target, lateral in zip(("Left", "Right"), grips, (1, -1)):
            reach_hand(sk, side, target, -UP + lateral * body.left * 0.5)
            hand_joint = sk.j(side + "Hand")
            sk.aim(hand_joint, _child(sk, hand_joint, (side + "MiddleProximal",)), -lateral * body.left + .2 * UP)
            _close_fist(sk, side)
    progress(60, "checking the posed skin")
    W = sk.world()
    posed = rig.skin(W)
    tris = triangles(rig)
    # skin fusing parts that move apart (a forearm modelled touching the thigh) tears into long spikes when they
    # do: those triangles are cut from the written mesh rather than stretched
    cut = seams(rig, tris) & torn(body, posed, tris)
    result = check(body, posed, tris[~cut], level, contact, seat if contact is not None else None)
    result["cut_triangles"] = int(cut.sum())
    seat_point = None
    if contact is not None:
        pts = posed[contact]
        low = pts[pts[:, 1] <= pts[:, 1].min() + 0.01 * body.height]
        seat_point = [float(low[:, 0].mean()), body.floor + seat, float(low[:, 2].mean())]
        foot = [sk.j(s) for s in ("LeftFoot", "RightFoot", "LeftToes", "RightToes") if sk.j(s) is not None]
        soles = posed[np.isin(rig.dominant, foot)]
        feet = "touch" if float(soles[:, 1].min()) - body.floor <= 0.02 * body.height else "dangle"
    elif body.kind == "quadruped" and main != "stand":
        hind = _legs(body)[1]
        hips = np.mean([W[g["chain"][0]][:3, 3] for g in hind], axis=0) if hind else W[sk.root][:3, 3]
        seat_point = [float(hips[0]), level, float(hips[2])]
    elif main in ("lie", "curl"):
        feet = "none"
    palm = None
    if "hold-out" in names:
        side = "Right" if (hand or "right") == "right" else "Left"
        point, normal = palm_surface(sk, rig, W, posed, tris, side)
        palm = {"hand": side.lower(), "point": _r(point), "normal": _r(normal)}
    manifest = {
        "pose": {"names": names, "clip": main, "body": body.kind, "seat_height_m": seat,
                 "look_at": None if look is None else _r(look),
                 "hand": (hand or "right") if palm or "hold" in names else None,
                 "hold_style": style if "hold" in names else None,
                 "hold_toward": None if hold_toward is None else _r(hold_toward)},
        "default_clip": main,
        "rest_mesh_unchanged": not cut.any() and bool(np.allclose(posed, rig.skin_rest, rtol=0, atol=1e-5)),
        "contact": {"floor_y": round(level, 4), "seat": None if seat_point is None else _r(seat_point), "feet": feet},
        "palm": palm,
        "grip": grip(sk, body, hand or "right", style, _frame_point(body, hold_toward)) if "hold" in names else None,
        "head": {"point": _r(W[head][:3, 3]), "gaze": _r(_gaze(sk, body, W))} if head is not None else None,
        "bounds": {"min": _r(posed.min(axis=0)), "max": _r(posed.max(axis=0))},
        "check": result,
    }
    if swimming_fish:
        # The saved rig may have recorded its tail as forward. Layout needs
        # the same rest direction that this pose used for its gaze target.
        manifest["forward"] = _r(body.fwd)
    if tapping is not None:
        manifest["contact"]["sticks"] = [_r(W[k][:3,3]) for k in tapping[2]]
        manifest["limb_tips"] = [_r(W[k][:3,3]) for k in tapping[3]]
        sk.q = tapping[1].copy()
        alternate = check(body,rig.skin(sk.world()),tris[~cut],level,None,None)
        sk.q = tapping[0].copy()
        if not alternate["pass"]:
            result["pass"] = False
            result["reasons"].extend(alternate["reasons"])
    if plant is not None:
        manifest["contact"]["shaft"] = _r(_fist_frame(sk, side, W)[0])
    if head is not None:
        # The head joint pivots behind the face. Measure the posed skin so a
        # closed helmet can enclose the jaw, crown and face after a lean.
        weights = np.where(np.isin(rig.vjoints, rig.subtree(head)), rig.vweights, 0).sum(axis=1)
        surface = posed[weights >= 0.65]
        if len(surface) and np.all(np.ptp(surface, axis=0) > 0):
            manifest["head"]["bounds"] = {"min": _r(surface.min(axis=0)), "max": _r(surface.max(axis=0))}
    if mouth is not None:
        manifest["head"]["mouth"] = _r(mouth)
    if other_style:
        manifest["other_grip"] = grip(sk, body, opposite, other_style, _frame_point(body, other_toward))
    if mount is not None:
        manifest["wind_mount"] = mount
    if main == "lean":
        # Actual back-of-body contact points at hip, waist and shoulder height.
        samples = []
        for fraction in (0.46, 0.60, 0.75):
            band = posed[np.abs(posed[:, 1] - (level + fraction * body.height)) < 0.05 * body.height]
            if not len(band):
                break
            depths = band @ body.fwd
            samples.append(_r(band[depths <= np.percentile(depths, 5)].mean(axis=0)))
        if len(samples) == 3:
            manifest["contact"]["back"] = samples
    if result["pass"]:
        progress(80, f"writing the {main} clip")
        write(rig, main, percussion.cycle(sk,tapping) if tapping is not None else breathe(sk, body), out, cut)
    return manifest
