"""Blender worker for forge-fit. Run: Blender -b --factory-startup -P worker.py -- JOB.json

The job names the command and its arguments; the result is written next to it as result.json.
"""

import json
import math
import sys
import time
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fitlib as F  # noqa: E402
import render as RN  # noqa: E402


def edges_of(tris):
    e = np.concatenate([tris[:, [0, 1]], tris[:, [1, 2]], tris[:, [2, 0]]])
    return np.unique(np.sort(e, axis=1), axis=0)


def hand_views(ch, side, label_prefix, out_dir, states, background=(0.12, 0.42, 0.22)):
    """Palm and profile renders of one hand for each named state callback; returns file paths."""
    cam, label = RN.setup(380, background)
    hf = ch.hand_frame(side)
    centre = (hf["W"] + hf["K"]) / 2 + hf["d"] * hf["width"] * 0.6
    span = hf["width"] * 3.6
    paths = []
    for name, apply in states:
        apply()
        bpy.context.view_layer.update()
        for vname, direction in (("palm", hf["n"]), ("profile", hf["a"])):
            RN.view(cam, label, centre, direction, span, f"{label_prefix} {name} {vname}", clip=True)
            paths.append(RN.shot(Path(out_dir) / f"{label_prefix}_{name}_{vname}.png"))
    return paths


WEB_CURL = 0.6  # fraction of the anatomical limits at which webs and neighbour drag are judged (a grip, not a fist)
# Calibrated on the hand-made Quaternius hands at that curl: worst web stretch 1.75x (male little), worst
# neighbour drag 0.05; limits are those plus margin (x1.25 and x3). The generated knight (webs 2.4-4.3x, drag
# 0.24-0.41) and warrior (webs up to 2.7x, drag up to 0.21) still fail where they visibly fuse.
WEB_LIMIT = 2.2
DRAG_LIMIT = 0.15


def intake(ch, side, out_dir=None, curl=None):
    """Hand intake test: does each finger chain curl its own skin, and only its own?"""
    side = ch.side_names(side)
    ch.rest_pose()
    rest = ch.skin()
    hf = ch.hand_frame(side)
    width = hf["width"]
    digits = ch.fingers.get(side, {})
    hand_ids = ch.hand_vertices(side)
    edges = edges_of(ch.tris)
    report = {"palm_width": width, "palm_width_m": width / ch.s, "digits": {},
              "renamed_by_geometry": {k: ch.fingers[side][k][0] for k in ch.remapped.get(side, {})}}
    failures = []
    for digit in F.DIGITS:
        if digit not in digits:
            failures.append(f"{digit}: no bone chain")
            continue
        chain = digits[digit]
        own = np.flatnonzero(np.isin(ch.dom, chain))
        counts = {b: int((ch.dom == b).sum()) for b in chain}
        heads = [ch.head(b) for b in chain]
        tip = heads[2] + (heads[2] - heads[1])
        axis = F.unit(tip - heads[0])
        length = float(np.linalg.norm(tip - heads[0]))
        reasons = []
        empty = [b for b, c in counts.items() if c == 0]
        if empty:
            reasons.append("empty bone group (no skin vertex mainly follows " + ", ".join(empty) + ")")
        extent = float(np.ptp((rest[own] - heads[0]) @ axis)) if len(own) else 0.0
        if extent < 0.5 * length:
            reasons.append(f"stub or missing digit: its own skin spans {extent / max(length, 1e-9):.0%} of the chain")
        # Joint centres versus the visible finger: offset of each joint from its skin cross-section centre.
        offsets = []
        for h in heads:
            near = hand_ids[np.abs((rest[hand_ids] - h) @ axis) < 0.12 * length]
            if len(near):
                local = rest[near] - h
                local -= np.outer(local @ axis, axis)
                radius = np.percentile(np.linalg.norm(local, axis=1), 90)
                centroid = np.linalg.norm(local.mean(axis=0))
                offsets.append(float(centroid / max(radius, 1e-9)))
        F.curl_pose(ch, side, WEB_CURL if curl is None else curl, digits=(digit,))
        posed = ch.skin()
        disp = np.linalg.norm(posed - rest, axis=1)
        own_motion = float(disp[own].max()) if len(own) else 0.0
        moved = np.flatnonzero(disp > 0.05 * width)
        others = {d: np.flatnonzero(np.isin(ch.dom, c)) for d, c in digits.items() if d != digit}
        drag = {d: float(disp[ix].max() / max(own_motion, 1e-9)) for d, ix in others.items() if len(ix)}
        worst = max(drag, key=drag.get) if drag else None
        if worst and drag[worst] > DRAG_LIMIT:
            reasons.append(f"drags its neighbour {worst} by {drag[worst]:.0%} of its own motion")
        # Which digit's bones lie nearest the skin this chain actually moves (displacement weighted)?
        wts = disp[hand_ids]
        if wts.sum() > 0:
            centroid = (rest[hand_ids] * wts[:, None]).sum(axis=0) / wts.sum()
            def seg_dist(c):
                hs = [ch.head(b, posed=False) for b in c]
                hs.append(hs[2] + (hs[2] - hs[1]))
                return min(np.linalg.norm(centroid - (p + np.clip((centroid - p) @ (q - p) / max((q - p) @ (q - p), 1e-12), 0, 1) * (q - p)))
                           for p, q in zip(hs[:-1], hs[1:]))
            drives = min(digits, key=lambda d: seg_dist(digits[d]))
            if drives != digit:
                reasons.append(f"chain {chain[0]} curls the skin of the {drives} finger, not the {digit}")
        else:
            drives = None
        mask_own = np.zeros(len(rest), bool)
        mask_own[own] = True
        mask_other = np.zeros(len(rest), bool)
        for ix in others.values():
            mask_other[ix] = True
        web = edges[(mask_own[edges[:, 0]] & mask_other[edges[:, 1]]) | (mask_own[edges[:, 1]] & mask_other[edges[:, 0]])]
        stretch = 1.0
        if len(web):
            r0 = np.linalg.norm(rest[web[:, 0]] - rest[web[:, 1]], axis=1)
            r1 = np.linalg.norm(posed[web[:, 0]] - posed[web[:, 1]], axis=1)
            stretch = float((r1 / np.maximum(r0, 1e-9)).max())
        if stretch > WEB_LIMIT:
            reasons.append(f"fused web: {len(web)} skin edges join it to a neighbour and stretch to {stretch:.1f}x")
        ch.rest_pose()
        report["digits"][digit] = {
            "vertices_per_bone": counts, "skin_extent_of_chain": round(extent / max(length, 1e-9), 3),
            "joint_offset_over_radius": [round(o, 2) for o in offsets], "moved_vertices": int(len(moved)),
            "own_vertices": int(len(own)), "own_motion": own_motion, "neighbour_drag": drag, "drives": drives,
            "web_edges": int(len(web)), "web_stretch": round(stretch, 2), "pass": not reasons, "reasons": reasons,
        }
        failures.extend(f"{digit}: {r}" for r in reasons)
    report["pass"] = not failures
    report["failures"] = failures
    if out_dir:
        report["renders"] = hand_views(ch, side, ("left" if side == "l" else "right") + "_hand", out_dir, [
            ("open", ch.rest_pose), ("half", lambda: F.curl_pose(ch, side, 0.5)), ("closed", lambda: F.curl_pose(ch, side, 1.0))])
        ch.rest_pose()
    return report


class PalmTarget:
    """A prop-like closed proxy for closing a fist: the palm (and, for the thumb, the curled fingers)."""

    def __init__(self, ch, side, with_fingers=False):
        side = ch.side_names(side)
        pts, owner, _ = ch.samples(ch.skin(), ch.hand_vertices(side))
        groups = [ch.jname(side + "hand")]
        if with_fingers:
            groups += [b for d, c in ch.fingers[side].items() if d != "thumb" for b in c]
        hulls = [F.hull(pts[owner == g]) for g in groups if (owner == g).sum() >= 4]
        self.proxy = F.Proxy(hulls)
        hf = ch.hand_frame(side)
        self.anchors = {"grip": hf["K"] * 0.5 + hf["W"] * 0.5 + hf["n"] * hf["width"] * 0.3}

    def M(self):
        return np.eye(4)

    def world(self, key):
        return self.anchors[key]


def fist(ch, side):
    side = ch.side_names(side)
    fingers = [d for d in F.DIGITS[1:] if d in ch.fingers[side]]
    res = F.close_hand(ch, side, PalmTarget(ch, side), digits=fingers)
    if "thumb" in ch.fingers[side]:
        res.update(F.close_hand(ch, side, PalmTarget(ch, side, with_fingers=True), digits=("thumb",), ready=False))
    return res


def finger_bones(ch, sides):
    return [b for s in sides for c in ch.fingers.get(s, {}).values() for b in c]


def snapshot(ch, bones):
    return {b: ch.rig.pose.bones[b].rotation_quaternion.copy() for b in bones}


def add_static_clip(ch, name, pose, bones, frames=(0, 1)):
    act, slot = F.new_action(ch.rig, name)
    F.key_pose(act, slot, ch.rig, bones, [(f, pose) for f in frames])
    F.push_track(ch.rig, act, slot, name)


def add_blend_clip(ch, name, a, b, bones, seconds=0.25):
    n = max(2, round(seconds * F.FPS))
    frames = []
    for i in range(n + 1):
        u = i / n
        u = u * u * (3 - 2 * u)
        frames.append((i, {k: a[k].slerp(b[k], u) for k in bones}))
    act, slot = F.new_action(ch.rig, name)
    F.key_pose(act, slot, ch.rig, bones, frames)
    F.push_track(ch.rig, act, slot, name)


def overlay(ch, pose, bones, skip=()):
    """Layer a finger pose onto every imported body clip so the hand stays closed while it moves."""
    ad = ch.rig.animation_data
    if not ad:
        return []
    done = []
    for track in ad.nla_tracks:
        if track.name in skip or not track.strips:
            continue
        strip = track.strips[0]
        act, slot = strip.action, strip.action_slot
        bag = act.layers[0].strips[0].channelbag(slot)
        f0, f1 = act.frame_range
        for b in bones:
            pb = ch.rig.pose.bones[b]
            for prop in ("rotation_quaternion", "rotation_euler", "location", "scale"):
                for i in range(4):
                    c = bag.fcurves.find(pb.path_from_id(prop), index=i)
                    if c:
                        bag.fcurves.remove(c)
            path = pb.path_from_id("rotation_quaternion")
            for i in range(4):
                F.add_curve(bag, path, i, [f0, f1], [tuple(pose[b])] * 2)
        done.append(track.name)
    return done


def ensure_hands(ch, job, renders=None):
    """Intake; replace the hands that fail with template hands unless told to keep them."""
    import handfix
    sides = [s for s in ("l", "r") if s in ch.fingers]
    before = {s: intake(ch, s) for s in sides}
    failing = [s for s in sides if not before[s]["pass"]]
    if job.get("replace") == "all":
        failing = sides
    info = {"intake_before": {("left" if s == "l" else "right"): before[s] for s in sides}, "replaced": {}}
    if not failing or job.get("keep_hands") or F.disabled("replace"):
        return info
    if renders:
        info["renders_before"] = [p for s in failing for p in hand_views(
            ch, s, ("left" if s == "l" else "right") + "_original", renders, [
                ("open", ch.rest_pose), ("half", lambda s=s: F.curl_pose(ch, s, 0.5)), ("closed", lambda s=s: F.curl_pose(ch, s, 1.0))])]
        ch.rest_pose()
    info["replaced"] = handfix.replace_hands(ch, failing, job.get("hand_style", "simple"), job.get("hand_source"))
    info["intake_after"] = {("left" if s == "l" else "right"): intake(ch, s) for s in sides}
    return info


def cmd_hands(job):
    ch = F.Character(job["character"], job.get("height_m", 1.8))
    out = Path(job["renders"]) if job.get("renders") else None
    if out:
        out.mkdir(parents=True, exist_ok=True)
    if job.get("replace"):
        info = ensure_hands(ch, {**job, "replace": job["replace"]}, out)
        result = {"height_scene": ch.height, "scene_units_per_m": ch.s, **info}
        if out:
            after = []
            for s in ("l", "r"):
                if ("left" if s == "l" else "right") in info["replaced"]:
                    def closed(s=s):
                        ch.rest_pose()
                        fist(ch, s)  # the template's closed state is the fist clip (closed against the palm)
                    after += hand_views(ch, s, ("left" if s == "l" else "right") + "_template", out, [
                        ("open", ch.rest_pose), ("half", lambda s=s: F.curl_pose(ch, s, 0.5)), ("closed", closed)])
            ch.rest_pose()
            paths = []
            for k in range(0, len(after), 6):  # original row above template row, per side
                paths += info.get("renders_before", [])[k:k + 6] + after[k:k + 6]
            if paths:
                result["sheet"] = RN.sheet(paths, 6, out / "hands_replaced.png")
        after_intake = info.get("intake_after") or info["intake_before"]
        result["pass"] = all(h["pass"] for h in after_intake.values())
        ch.rest_pose()
        F.export(job["output"])
        return result
    result = {"height_scene": ch.height, "scene_units_per_m": ch.s, "hands": {}}
    for i, side in enumerate(("l", "r")):
        F.progress(10 + 40 * i, f"Intake test, {'left' if side == 'l' else 'right'} hand")
        result["hands"]["left" if side == "l" else "right"] = intake(ch, side, out)
    if out:
        paths = [p for h in result["hands"].values() for p in h.get("renders", [])]
        result["sheet"] = RN.sheet(paths, 6, out / "hands_intake.png")
    result["pass"] = all(h["pass"] for h in result["hands"].values())
    return result


def cmd_fist(job):
    ch = F.Character(job["character"], job.get("height_m", 1.8), keep_animation=True)
    sides = [s for s in ("l", "r") if s in ch.fingers]
    if not sides:
        raise ValueError("Character has no finger chains")
    bones = finger_bones(ch, sides)
    ch.rest_pose()
    open_pose = snapshot(ch, bones)
    angles = {}
    for s in sides:
        F.progress(20 if s == "l" else 50, "Closing the " + ("left" if s == "l" else "right") + " fist against the palm")
        angles["left" if s == "l" else "right"] = fist(ch, s)
    closed = snapshot(ch, bones)
    contact = 0.004 * ch.s
    report = {s: F.grip_report(ch, s, PalmTarget(ch, s), contact)[0] for s in sides}
    ch.rest_pose()
    add_static_clip(ch, "hand_open", open_pose, bones)
    add_static_clip(ch, "hand_closed", closed, bones)
    add_blend_clip(ch, "fist_close", open_pose, closed, bones)
    add_blend_clip(ch, "fist_open", closed, open_pose, bones)
    F.export(job["output"])
    return {"angles": angles, "palm_contact": report, "clips": ["hand_open", "hand_closed", "fist_close", "fist_open"]}


def fit_one_hand(ch, side, prop, kind="power", gap_to_guard_m=0.004, roll_to="d", reverse=False, slot=0):
    """Place the prop's grip in the open hand, press it onto the palm, then close the fingers.

    slot 0 puts the hand next to the guard (or centred on guardless grips), slot 1 one hand-width further.
    """
    side = ch.side_names(side)
    ch.rest_pose()
    hf = ch.hand_frame(side)
    F.set_world(prop.obj, F.place_for_hand(ch, side, prop, kind, roll_to=roll_to, reverse=reverse))
    bpy.context.view_layer.update()
    pts, owner, patches = ch.hand_patches(side)
    A = prop.anchors
    M = prop.M()
    g = prop.world("grip_axis")
    ix = np.r_[patches.get("index_0", []), patches.get("little_0", []), patches.get("thumb_0", [])].astype(int)
    proj = pts[ix] @ g if len(ix) else np.zeros(1)
    width = float(proj.max() - proj.min())
    gc = prop.world("grip") @ g
    half = A["grip_length"] / 2
    gap = gap_to_guard_m * ch.s
    if A.get("guard_side", 0) > 0:
        shift = (gc + half - gap - slot * (width + gap)) - proj.max()
    else:
        shift = gc + slot * (width + gap) - (proj.max() + proj.min()) / 2
    M[:3, 3] -= g * shift
    F.set_world(prop.obj, M)
    bpy.context.view_layer.update()
    # Press onto the palm (power) or onto the middle phalanges (hook).
    if kind == "power":
        def palm():
            p, o, pa = ch.hand_patches(side)
            return p[pa["palm"]]
    else:
        def palm():
            p, o, pa = ch.hand_patches(side)
            ix = np.r_[pa.get("index_1", []), pa.get("middle_1", []), pa.get("ring_1", [])].astype(int)
            return p[ix]
    travel, d0 = F.press(ch, prop, palm, hf["n"], 0.0005 * ch.s, hf["width"] * 1.5)
    digits = F.DIGITS if kind == "power" else ("index", "middle", "ring")
    joints = (0, 1, 2) if kind == "power" else (1, 2)
    angles = {} if F.disabled("close") else F.close_hand(ch, side, prop, digits=digits, joints=joints)
    rel = np.linalg.inv(ch.world(ch.jname(side + "hand"))) @ prop.M()
    return {"angles": angles, "press_travel": float(travel), "hand_to_prop": rel.tolist(), "axial_shift": float(shift)}


def two_hand_guard(ch, prop, fits, reports, contact, bones):
    """Both hands on one grip: the right hand next to the guard, the left one hand-width below it, in a
    guard pose in front of the body; the arms reach their grips with the same IK as reach."""
    import motion as MO
    fits["l"] = fit_one_hand(ch, "l", prop, slot=1)
    reports["l"] = F.grip_report(ch, "l", prop, contact)[0]
    grips = {}
    for s in ("r", "l"):
        grips["g_" + s] = {"hand": s, "prop": prop.name, "T": fits[s]["hand_to_prop"],
                           "fingers": None}
    # finger poses: re-fit each hand quickly from the stored angles is not needed; take them from the fits
    ch.rest_pose()
    for s in ("r", "l"):
        fit_one_hand(ch, s, prop, slot=0 if s == "r" else 1)
        grips["g_" + s]["fingers"] = {b: tuple(ch.rig.pose.bones[b].rotation_quaternion) for b in finger_bones(ch, [s])}
        ch.rest_pose()
    tpl = {"name": "grip", "duration": 1 / F.FPS, "tracks": {
        prop.name: [{"t": 0, "static": True}],
        "right_hand": [{"t": 0, "grip": "g_r"}], "left_hand": [{"t": 0, "grip": "g_l"}]},
        "order": ["right_hand", prop.name, "left_hand"]}
    m = MO.Motion(ch, tpl, {prop.name: prop}, grips, ch.height_m)
    m.now = 0.0
    start = m.place(prop.name, {"anchor": "grip", "z": [0, 0.6, 0.8], "x": "forward",
                                "at": {"base": "joint:hips", "add": [["forward", 0.32, "m"], ["up", 0.14, "m"]]}})
    F.set_world(prop.obj, start)
    m.cache[prop.name] = start
    m.run()
    for s in ("r", "l"):
        reports[s + "_posed_reach_shortfall_m"] = max(f["shortfall"][s] for f in m.frames) / ch.s
    # leave the rig in the solved pose to parent the prop at its solved place
    for b, M in m.frames[-1]["bones"].items():
        ch.rig.pose.bones[b].matrix_basis = Matrix(M.tolist())
    bpy.context.view_layer.update()
    for s in ("r", "l"):
        reports[s + "_posed"] = F.grip_report(ch, s, prop, contact)[0]
    return m, start, None


def carry_pose(ch, prop, side, clips, s):
    """Pick a small wrist offset (flex/deviation, 0 or +-25 degrees each) that keeps the held prop out of the
    body through the layered clips, and bake it into them; returns before/after depths in metres."""
    import measure as MSR
    from mathutils import Quaternion
    hand = ch.jname(side + "hand")
    pb = ch.rig.pose.bones[hand]
    proxy = MSR.prop_proxy(prop.obj)
    exclude = MSR.holding_ids(ch, [hand])
    ad = ch.rig.animation_data
    tracks = [t for t in ad.nla_tracks if t.name in clips] if ad else []
    def score(q):
        total = 0.0
        for tr in tracks:
            act = tr.strips[0].action
            ad.action, ad.action_slot = act, tr.strips[0].action_slot
            f0, f1 = act.frame_range
            worst = 0.0
            for fr in np.linspace(f0, f1, 9):
                bpy.context.scene.frame_set(int(fr), subframe=float(fr % 1))
                pb.rotation_quaternion = pb.rotation_quaternion @ q
                bpy.context.view_layer.update()
                d, _ = MSR.prop_body_depth(ch, proxy, F.matrix_np(prop.obj.matrix_world), ch.skin(), exclude)
                worst = max(worst, d)
            total += worst
        ad.action = None
        return total / s
    cands = [(math.radians(a), math.radians(b)) for a in (0, 25, -25) for b in (0, 25, -25)]
    results = []
    for a, b in cands:
        q = Quaternion((1, 0, 0), a) @ Quaternion((0, 0, 1), b)
        results.append((score(q) + 0.002 * (abs(a) + abs(b)), a, b, q))
    base = results[0][0]
    best = min(results, key=lambda r: r[0])
    if best[0] < base - 0.002 and not F.disabled("carry"):
        for tr in tracks:
            act, slot = tr.strips[0].action, tr.strips[0].action_slot
            bag = act.layers[0].strips[0].channelbag(slot)
            path = pb.path_from_id("rotation_quaternion")
            curves = [bag.fcurves.find(path, index=i) for i in range(4)]
            if any(c is None for c in curves):
                continue
            times = sorted({k.co.x for c in curves for k in c.keyframe_points})
            vals = []
            for t in times:
                q0 = Quaternion([c.evaluate(t) for c in curves])
                vals.append(tuple(q0 @ best[3]))
            for c in curves:
                bag.fcurves.remove(c)
            for i in range(4):
                F.add_curve(bag, path, i, times, vals)
    return {"before_m": float(base), "after_m": float(min(best[0], base)),
            "wrist_offset_deg": [math.degrees(best[1]), math.degrees(best[2])] if best[0] < base - 0.002 else [0.0, 0.0]}


def cmd_hold(job):
    ch = F.Character(job["character"], job.get("height_m", 1.8), keep_animation=True)
    sides = {"right": ["r"], "left": ["l"], "both": ["r", "l"]}[job["hand"]]
    F.progress(5, "Hand intake test")
    hand_info = ensure_hands(ch, job)
    hands = {s: intake(ch, s) for s in sides} if hand_info["replaced"] else {
        s: hand_info["intake_before"]["left" if s == "l" else "right"] for s in sides}
    if not all(h["pass"] for h in hands.values()) and not job.get("accept_hands"):
        raise ValueError("Hand intake failed: " + "; ".join(f for h in hands.values() for f in h["failures"])
                         + (" (template hands did not pass either)" if hand_info["replaced"] else
                            " (use --accept-hands to fit anyway, or drop --keep-hands to replace them)"))
    p = job["props"][0]
    import props_lib
    prop, prop_report = props_lib.ensure_functional(p["path"], p.get("name", "prop"), p.get("class", "sword"), ch,
                                                    p.get("length_m"), p.get("anchors"), job.get("work"), job.get("keep_prop"))
    if p.get("grip"):
        prop.anchors["grip"] = (np.array(p["grip"]) - prop.points.mean(axis=0)) * 0 + np.array(p["grip"]) * prop.scale
    contact = 0.004 * ch.s
    bones = finger_bones(ch, sides)
    open_pose = snapshot(ch, bones)
    fits, reports = {}, {}
    side = sides[0]
    F.progress(30, "Fitting the grip in the " + job["hand"] + " hand")
    fits[side] = fit_one_hand(ch, side, prop)
    reports[side] = F.grip_report(ch, side, prop, contact)[0]
    grip_pose = snapshot(ch, bones)
    hand_bone = ch.jname(side + "hand")
    M = prop.M()
    guard = None
    if len(sides) == 2:
        guard, M, arm_pose = two_hand_guard(ch, prop, fits, reports, contact, bones)
        grip_pose = arm_pose
    if not F.disabled("parent"):
        prop.obj.parent = ch.rig
        prop.obj.parent_type = "BONE"
        prop.obj.parent_bone = hand_bone
    F.set_world(prop.obj, M)
    bpy.context.view_layer.update()
    contact_clips = ["grip"] + [t for t in ch.tracks]  # grip_close starts open: no contact intended
    prop.obj["forge_fit"] = json.dumps({"class": prop.cls, "anchors": {k: np.asarray(v).tolist() for k, v in prop.anchors.items()},
                                        "hand": hand_bone, "hands": [ch.jname(s + "hand") for s in sides],
                                        "contact_clips": contact_clips})
    ch.rest_pose()
    fist_pose = {}
    for s in sides:
        fist(ch, s)
    fist_pose = snapshot(ch, bones)
    ch.rest_pose()
    carry = None
    if guard is None:
        layered = overlay(ch, grip_pose, bones)
        if layered:
            F.progress(80, "Choosing a carry pose that keeps the prop out of the body")
            carry = carry_pose(ch, prop, side, layered, ch.s)
        add_static_clip(ch, "hand_open", open_pose, bones)
        add_static_clip(ch, "hand_closed", fist_pose, bones)
        add_static_clip(ch, "grip", grip_pose, bones)
        add_blend_clip(ch, "grip_close", open_pose, grip_pose, bones)
    else:
        layered = []
        add_static_clip(ch, "hand_open", open_pose, bones)
        add_static_clip(ch, "hand_closed", fist_pose, bones)
        guard.bake("grip", props=False)
    F.export(job["output"])
    A = prop.anchors
    return {
        "intake": hands, "hands_replaced": hand_info["replaced"], "prop_intake": prop_report, "carry_pose": carry, "fit": fits, "contact": reports, "contact_threshold": contact,
        "prop": {"class": prop.cls, "scale": prop.scale, "length": prop.length, "grip_length": A.get("grip_length"),
                 "grip_radius": A.get("grip_radius"), "grip_diameter_palm_widths": 2 * A.get("grip_radius", 0) / ch.hand_frame(side)["width"],
                 "removed_string": getattr(prop, "removed_string", None)},
        "layered_clips": layered, "clips": ["hand_open", "hand_closed", "grip", "grip_close"] + layered,
        "scene_units_per_m": ch.s, "height_scene": ch.height,
    }


def activate(objects, clip):
    """Play one named clip on every imported object (rig and animated props)."""
    actions = set(bpy.data.actions)
    rig = next(o for o in objects if o.type == "ARMATURE")
    if clip:
        F.set_source_action(rig, objects, actions, clip)
    act = rig.animation_data.action if rig.animation_data else None
    return rig, (act.frame_range if act else (0, 0))


def cmd_render(job):
    """Evidence frames: views around a bone (or the whole body) at given clip fractions -> one sheet."""
    bpy.context.scene.render.fps = F.FPS
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=job["glb"])
    objects = set(bpy.data.objects) - before
    for o in list(objects):
        if o.type == "MESH" and o.name.startswith("Icosphere"):
            objects.discard(o)
            bpy.data.objects.remove(o)
    out = Path(job["out_dir"])
    out.mkdir(parents=True, exist_ok=True)
    cam, label = RN.setup(job.get("res", 420), tuple(job.get("background", (0.08, 0.09, 0.11))))
    paths = []
    for shot in job["shots"]:
        rig, (f0, f1) = activate(objects, shot.get("clip"))
        for t in shot.get("at", [0.0]):
            frame = f0 + t * (f1 - f0)
            bpy.context.scene.frame_set(int(math.floor(frame)), subframe=frame % 1)
            bpy.context.view_layer.update()
            mw = rig.matrix_world
            if shot.get("bone"):
                pb = rig.pose.bones[shot["bone"]]
                centre = np.array((mw @ pb.head)[:]) + np.array(shot.get("offset", (0, 0, 0)))
            else:
                pts = np.concatenate([F.SoleSampler.points(o, np.empty(len(o.data.vertices) * 3, np.float32))
                                      for o in objects if o.type == "MESH" and o.find_armature() == rig])
                centre = (pts.min(axis=0) + pts.max(axis=0)) / 2
            for name in shot.get("views", ["front"]):
                direction = {"front": (0, -1, 0.15), "back": (0, 1, 0.15), "left": (1, 0, 0.15), "right": (-1, 0, 0.15),
                             "top": (0, -0.2, 1), "below": (0, -0.3, -1), "three_quarter": (0.7, -0.7, 0.35),
                             "three_quarter_r": (-0.7, -0.7, 0.35)}.get(name, None) if isinstance(name, str) else name
                text = f"{shot.get('title', shot.get('clip') or 'rest')} t={t:.2f} {name if isinstance(name, str) else ''}"
                RN.view(cam, label, centre, direction, shot.get("span", 2.2), text, clip=shot.get("clip_near", False))
                paths.append(RN.shot(out / f"{shot.get('title', 'shot')}_{len(paths):03d}.png"))
    sheet = RN.sheet(paths, job.get("cols", 6), out / job.get("sheet", "sheet.png")) if job.get("sheet") else None
    return {"frames": paths, "sheet": sheet}


COMMANDS = {"hands": cmd_hands, "fist": cmd_fist, "hold": cmd_hold, "render": cmd_render}


def main():
    job_path = Path(sys.argv[sys.argv.index("--") + 1])
    job = json.loads(job_path.read_text())
    started = time.monotonic()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.render.fps = F.FPS
    import importlib
    for name in ("motion", "measure", "props_lib"):
        try:
            mod = importlib.import_module(name)
            COMMANDS.update(getattr(mod, "COMMANDS", {}))
        except ModuleNotFoundError as exc:
            if exc.name != name:
                raise
    if job.get("profile"):
        import cProfile
        import pstats
        prof = cProfile.Profile()
        result = prof.runcall(COMMANDS[job["command"]], job)
        pstats.Stats(prof).sort_stats("cumulative").print_stats(25)
    else:
        result = COMMANDS[job["command"]](job)
    result["blender_sec"] = round(time.monotonic() - started, 3)
    (job_path.parent / "result.json").write_text(json.dumps(result, indent=1, default=float))
    print("FIT_DONE", flush=True)


if __name__ == "__main__":
    main()
