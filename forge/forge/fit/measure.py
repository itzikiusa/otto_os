"""forge-fit check and correct: measured physics of every clip on the exported, deformed skin.

All thresholds are in metres/degrees at the character's declared height (docs/fit.md justifies them).
"""

import json
import math
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix, Vector

import fitlib as F
import render as RN
import worker as W

LIMITS = {
    "foot_penetration_m": 0.010, "foot_slide_m": 0.050,
    "knee_hyperextension_deg": 5.0, "knee_side_deg": 30.0, "knee_flexion_deg": 160.0,
    "elbow_hyperextension_deg": 10.0, "elbow_side_deg": 45.0, "elbow_flexion_deg": 155.0,
    "limb_body_penetration_m": 0.03,
    "prop_contact_fraction": 0.90, "prop_penetration_m": 0.003, "prop_gap_m": 0.005,
    "loop_seam_m": 0.010, "hips_acceleration_ms2": 150.0, "wrist_deg_per_frame60": 90.0,
    "elbow_plane_flip_deg_per_frame60": 60.0, "quiver_penetration_m": 0.010,
    "nock_to_string_m": 0.002, "shaft_to_rest_m": 0.005, "bow_wobble_m": 0.002, "bow_wobble_deg": 1.0,
    "draw_fingers_to_string_m": 0.006, "release_position_jump_m": 0.001, "flight_residual_m": 0.01,
    "flight_tangent_deg": 2.0, "string_forward_overshoot_brace": 0.5, "ownership_jump_m": 0.001,
    "grip_wrap_deg": 200.0, "fingers_in_contact": 3, "foot_drift_m": 0.010, "prop_through_body_m": 0.015, "prop_between_keys_m": 0.003,
}


def angle(u, v):
    return math.degrees(math.acos(float(np.clip(F.unit(u) @ F.unit(v), -1, 1))))


class Hinge:
    """Knee or elbow: signed flexion (negative = bent the wrong way) and sideways deviation of the bend."""

    def __init__(self, ch, upper, lower, end, front_world):
        self.ch, self.bones = ch, (upper, lower, end)
        R0 = np.array(ch.rig.pose.bones[upper].bone.matrix_local.to_3x3())
        mw = np.array(ch.rig.matrix_world.to_3x3())
        self.front_local = np.linalg.inv(mw @ R0) @ front_world

    def measure(self):
        ch = self.ch
        a, b, c = (ch.head(x) for x in self.bones)
        Rw = np.array((ch.rig.matrix_world @ ch.rig.pose.bones[self.bones[0]].matrix).to_3x3())
        front = F.unit(Rw @ self.front_local)
        u, v = b - a, c - b
        flex = angle(u, v)
        line = F.unit(c - a)
        o = (b - a) - line * ((b - a) @ line)
        side_axis = F.unit(np.cross(line, front))
        f, s = o @ front, o @ side_axis
        side = math.degrees(math.atan2(abs(s), abs(f))) if flex > 20 else 0.0
        # backwards only when the bend lies within 45 degrees of the anatomical plane; a mostly sideways bend
        # of a nearly straight limb has an ill-defined sign
        backwards = f < -1e-9 and math.degrees(math.atan2(abs(s), abs(f))) < 45
        sign = -1.0 if backwards and not F.disabled("hinge_sign") else 1.0
        return sign * flex, side, F.unit(np.cross(u, v)) if flex > 1 else None


def body_proxies(ch):
    """Closed convex proxies of torso and thighs in each bone's rest frame (two slabs per bone)."""
    out = {}
    keys = [k for k in ("hips", "spine", "chest", "upperchest", "lthigh", "rthigh") if k in ch.joints]
    for k in keys:
        bone = ch.jname(k)
        ids = np.flatnonzero(ch.dom == bone)
        if len(ids) < 8:
            continue
        rest_bone = ch.world(bone)
        local = (ch.rest[ids] - rest_bone[:3, 3]) @ rest_bone[:3, :3]
        out[bone] = F.proxy_from_points(local, np.array([0, 1.0, 0]), np.zeros(3), 2)
    return out


def prop_body_depth(ch, proxy, M, coords, exclude):
    """Deepest skin vertex of the body (minus the excluded holding hands/forearms) inside a prop's closed proxy."""
    if F.disabled("prop_body"):
        return 0.0, None
    inv = np.linalg.inv(M)
    keep = np.ones(len(coords), bool)
    keep[exclude] = False
    pts = coords[keep]
    local = pts @ inv[:3, :3].T + inv[:3, 3]
    lo = np.min([h["verts"].min(axis=0) for h in proxy.hulls], axis=0)
    hi = np.max([h["verts"].max(axis=0) for h in proxy.hulls], axis=0)
    near = np.all((local >= lo - 1e-6) & (local <= hi + 1e-6), axis=1)
    if not near.any():
        return 0.0, None
    d = proxy.signed(pts[near], M, exact_below=0)
    i = int(d.argmin())
    return float(max(0.0, -d[i])), str(ch.dom[np.flatnonzero(keep)[np.flatnonzero(near)[i]]])


def holding_ids(ch, hands):
    names = set()
    for hb in hands:
        sd = "l" if hb == ch.jname("lhand") else "r"
        names |= {hb, ch.jname(sd + "forearm")} | {b for c in ch.fingers.get(sd, {}).values() for b in c}
    return np.flatnonzero(np.isin(ch.dom, list(names)))


def limb_points(ch, side, coords):
    names = {ch.jname(side + "forearm"), ch.jname(side + "hand")} | {b for c in ch.fingers.get(side, {}).values() for b in c}
    ids = np.flatnonzero(np.isin(ch.dom, list(names)))
    return coords[ids]


def prop_proxy(obj):
    """Closed proxy of an imported prop mesh, in its object frame."""
    me = obj.data
    me.calc_loop_triangles()
    verts = np.array([v.co[:] for v in me.vertices])
    tris = np.array([t.vertices[:] for t in me.loop_triangles])
    c = verts.mean(axis=0)
    _, _, vt = np.linalg.svd(verts - c, full_matrices=False)
    pts = F.surface_points(verts, tris, np.ptp(verts @ vt[0]) / 160)
    return F.Proxy(F.proxy_from_points(pts, vt[0], np.zeros(3)).hulls)


def clip_samples(f0, f1, step=0.5):
    grid = np.r_[np.arange(f0, f1 - 1e-4, step), f1]
    return grid


def set_frame(t):
    bpy.context.scene.frame_set(int(math.floor(t)), subframe=float(t - math.floor(t)))


def check_clip(ch, clip, props, extras, soles, hinges, proxies):
    s, fps = ch.s, F.FPS
    F.set_source_action(ch.rig, ch.objects, set(bpy.data.actions), clip)
    act = ch.rig.animation_data.action
    f0, f1 = act.frame_range
    info = extras.get(clip, {})
    tpl = info.get("template", {})
    times = sorted(set(np.round(np.r_[np.arange(f0, f1 - 1e-4, 0.5), f1], 6)))
    grid = np.array(times)
    n = len(grid)
    dt = 0.5 / fps
    hold = {}  # (prop, hand bone) -> grip name
    for name, p in props.items():
        for hb in p.get("hands") or ([p["hand"]] if p["hand"] else []):
            hold[(name, hb)] = name
    for gname, g in info.get("grips", {}).items():
        if g["prop"] in props:
            hold.setdefault((g["prop"], ch.jname(g["hand"] + "hand")), gname)
    events = {e["attach"]: e["t"] for e in tpl.get("events", []) if e.get("attach")}
    rows = []
    for t in grid:
        set_frame(t)
        coords = ch.skin()
        feet, _ = soles.sample()
        row = {"feet": feet, "hinge": {k: h.measure() for k, h in hinges.items()},
               "hips": ch.head("hips"), "joints": np.array([ch.head(k) for k in ("lhand", "rhand", "lfoot", "rfoot", "head")]),
               "wrist": {sd: F.matrix_np((ch.rig.matrix_world @ ch.rig.pose.bones[ch.jname(sd + "hand")].matrix).to_3x3()) for sd in "lr"}}
        worst, where = 0.0, None
        for sd in ("" if F.disabled("limb") else "lr"):
            pts = limb_points(ch, sd, coords)
            for bone, px in proxies.items():
                d = float(-px.signed(pts, ch.world(bone), exact_below=0).min())
                if d > worst:
                    worst, where = d, f"{'left' if sd == 'l' else 'right'} arm in {bone}"
        row["limb"] = (worst, where)
        row["props"] = {}
        row["holds"] = {}
        for name, p in props.items():
            Mp = F.matrix_np(p["obj"].matrix_world)
            r = {"M": Mp}
            hands_of = [hb for (pn, hb) in hold if pn == name]
            if p["cls"] != "quiver":
                r["body_depth"], r["body_where"] = prop_body_depth(ch, p["proxy"], Mp, coords, holding_ids(ch, hands_of))
                # and the other way round: prop surface inside the torso/thigh proxies (a thin blade through the
                # chest can only hide skin up to half its own thickness, but its points go deep into the body)
                if not F.disabled("prop_body"):
                    pts = p.setdefault("_pts", np.concatenate([h["verts"] for h in p["proxy"].hulls]))
                    wpts = pts @ Mp[:3, :3].T + Mp[:3, 3]
                    for bone, px in proxies.items():
                        dd = float(-px.signed(wpts, ch.world(bone), exact_below=0).min())
                        if dd > r["body_depth"]:
                            r["body_depth"], r["body_where"] = dd, bone
            for (pn, hb), gname in hold.items():
                if pn != name:
                    continue
                sd = "l" if hb == ch.jname("lhand") else "r"
                pts, owner, patches = ch.hand_patches(sd, coords)
                dist = p["proxy"].signed(pts, Mp, exact_below=0.02 * s)
                h = {"patch": {k: float(dist[ix].min()) for k, ix in patches.items() if len(ix)},
                     "pen": float(max(0.0, -dist.min())), "hand_M": ch.world(hb), "M": Mp}
                A = p["anchors"]
                if "grip" in A and "grip_axis" in A:
                    g0 = Mp[:3, :3] @ np.asarray(A["grip"]) + Mp[:3, 3]
                    ga = F.unit(Mp[:3, :3] @ np.asarray(A["grip_axis"]))
                    near = pts[dist <= LIMITS["prop_gap_m"] * s] - g0
                    near = near[np.abs(near @ ga) <= 0.6 * float(A.get("grip_length", 1.0)) * np.cbrt(abs(np.linalg.det(Mp[:3, :3])))]
                    if len(near) >= 3:
                        e1 = F.unit(np.cross(ga, [1.0, 0, 0]) if abs(ga[0]) < 0.9 else np.cross(ga, [0, 1.0, 0]))
                        e2 = np.cross(ga, e1)
                        ang = np.sort(np.degrees(np.arctan2(near @ e2, near @ e1)))
                        gaps = np.diff(np.r_[ang, ang[0] + 360])
                        h["wrap"] = float(360 - gaps.max())
                    else:
                        h["wrap"] = 0.0
                row["holds"][(name, hb)] = h
            if p["cls"] == "quiver":
                ids = np.flatnonzero(np.isin(ch.dom, [ch.jname(k) for k in ch.joints if k in (
                    "spine", "chest", "upperchest", "lupperarm", "rupperarm", "lforearm", "rforearm", "lclavicle", "rclavicle")]))
                r["body_pen"] = float(max(0.0, -p["proxy"].signed(coords[ids], Mp, exact_below=0).min()))
            row["props"][name] = r
        row["empties"] = {o.name: np.array(o.matrix_world.translation[:]) for o in ch.objects if o.type == "EMPTY" or o.name.startswith("bow_string")}
        rows.append(row)
    M = {}

    def frame_of(i):
        return round(float(grid[i]), 3)

    feet = np.array([[f[2] for f in r["feet"]] + [f[0] for f in r["feet"]] + [f[1] for f in r["feet"]] for r in rows])
    pen = np.maximum(0, soles.floor - feet[:, :2]) * (0.0 if F.disabled("feet") else 1.0)
    i = int(np.unravel_index(pen.argmax(), pen.shape)[0])
    M["foot_penetration_m"] = {"value": float(pen.max() / s), "frame": frame_of(i), "focus": "foot"}
    runs = []
    for side in range(2):
        xyv = feet[:, [2 + side, 4 + side]]
        speed = np.linalg.norm(np.gradient(xyv, dt, axis=0), axis=1) / s if n > 1 else np.zeros(n)
        # planted: sole within 2% of the height above the floor AND moving under 0.5 m/s (a gliding step is not
        # a plant; the hand-made baseline's lunges would otherwise read as half-metre slides)
        low = np.flatnonzero((feet[:, side] <= soles.floor + 0.02 * ch.height) & (speed < 0.5))
        for ix in np.split(low, np.where(np.diff(low) != 1)[0] + 1):
            if len(ix) * dt >= 0.1:
                runs.append((ix, feet[ix][:, [2 + side, 4 + side]]))
    vel = np.zeros(2)
    if runs:
        num, den = np.zeros(2), 0.0
        for ix, p in runs:
            tt = ix * dt - (ix * dt).mean()
            num += tt @ (p - p.mean(axis=0))
            den += tt @ tt
        vel = num / max(den, 1e-12)
        if np.linalg.norm(vel) / s < 0.05:  # under 5 cm/s: treat as standing, no belt
            vel = np.zeros(2)
    worst_slide, wi, raw = 0.0, 0, 0.0
    for ix, p in runs:
        q = p - (ix * dt)[:, None] * vel
        d = np.linalg.norm(q - q[0], axis=1)
        raw = max(raw, float(np.linalg.norm(p - p[0], axis=1).max()))
        if d.max() > worst_slide:
            worst_slide, wi = float(d.max()), int(ix[d.argmax()])
    M["foot_slide_m"] = {"value": worst_slide / s, "raw_m": raw / s, "belt_speed_ms": float(np.linalg.norm(vel) / s),
                         "frame": frame_of(wi), "planted_intervals": len(runs), "focus": "foot"}
    xy = feet[:, 2:]
    drift = np.linalg.norm(np.stack([xy[:, [0, 2]] - xy[0, [0, 2]], xy[:, [1, 3]] - xy[0, [1, 3]]]), axis=2).max(axis=0)
    M["foot_drift_m"] = {"value": float(drift.max() / s), "frame": frame_of(int(drift.argmax())), "applies": bool(tpl),
                         "focus": "foot", "note": "sole travel from the first frame; judged on authored interactions only"}
    for key in ("knee", "elbow"):
        vals = [(i, k, r["hinge"][k][0], r["hinge"][k][1]) for i, r in enumerate(rows) for k in r["hinge"] if k.startswith(key)]
        hyper = min(vals, key=lambda v: v[2])
        side_w = max(vals, key=lambda v: v[3])
        flex = max(vals, key=lambda v: v[2])
        M[key + "_hyperextension_deg"] = {"value": float(max(0.0, -hyper[2])), "frame": frame_of(hyper[0]), "joint": hyper[1], "focus": hyper[1]}
        if key == "elbow":
            M[key + "_hyperextension_deg"].update({"applies": False, "note": "reported only: the hand-made baseline's sword "
                                                   "attack reads 30 deg here on a visibly normal, fast-swinging, nearly straight arm"})
        M[key + "_side_deg"] = {"value": float(side_w[3]), "frame": frame_of(side_w[0]), "joint": side_w[1], "focus": side_w[1],
                                "applies": False, "note": "reported only: the hand-made baseline (Quaternius UAL on its own base) reaches 70-90 deg here "
                                                          "with arms overhead and wide lunges, so this measure does not separate good from bad"}
        M[key + "_flexion_deg"] = {"value": float(flex[2]), "frame": frame_of(flex[0]), "joint": flex[1], "focus": flex[1]}
    flips = []
    for k in hinges:
        if not k.startswith("elbow"):
            continue
        for i in range(1, n):
            a, b = rows[i - 1]["hinge"][k][2], rows[i]["hinge"][k][2]
            if a is not None and b is not None and rows[i]["hinge"][k][0] > 20:
                flips.append((angle(a, b) * (dt * 60) ** -1, i, k))
    if flips:
        w = max(flips)
        M["elbow_plane_flip_deg_per_frame60"] = {"value": float(w[0]), "frame": frame_of(w[1]), "joint": w[2], "focus": w[2]}
    limb = [r["limb"] for r in rows]
    li = int(np.argmax([l[0] for l in limb]))
    M["limb_body_penetration_m"] = {"value": float(limb[li][0] / s), "frame": frame_of(li), "where": limb[li][1],
                                    "rest_m": float(getattr(ch, "_rest_limb", 0.0) / s), "focus": "hand"}
    hips = np.array([r["hips"] for r in rows])
    if n > 2:
        # at 30 Hz: clips are keyed at 30 Hz or finer with linear interpolation, whose kinks would read as
        # spikes at 120 Hz sampling; a real pop is a jump between keys
        h4 = hips[::4]
        acc = np.linalg.norm(np.diff(h4, 2, axis=0), axis=1) / (4 * dt) ** 2 / s if len(h4) > 2 else np.zeros(1)
        M["hips_acceleration_ms2"] = {"value": float(acc.max()), "frame": frame_of(min(n - 1, 4 * (int(acc.argmax()) + 1))), "focus": "body"}
        wr = []
        for i in range(1, n):
            for sd in "lr":
                R = rows[i]["wrist"][sd] @ rows[i - 1]["wrist"][sd].T
                wr.append((math.degrees(math.acos(float(np.clip((np.trace(R) - 1) / 2, -1, 1)))) / (dt * 60), i, sd))
        w = max(wr)
        M["wrist_deg_per_frame60"] = {"value": float(w[0]), "frame": frame_of(w[1]), "hand": w[2], "focus": "hand"}
    joints = np.array([r["joints"] for r in rows])
    travel = hips[-1] - hips[0]
    travel[2] = 0
    seam = np.linalg.norm(joints[-1] - joints[0] - travel, axis=1).max() / s
    looped = any(w in clip.lower() for w in ("loop", "walk", "run", "idle"))
    M["loop_seam_m"] = {"value": float(seam), "applies": bool(looped and n > 4), "focus": "body",
                        "note": "judged on clips whose name says they loop (loop, walk, run, idle)"}
    # props held in a hand: contact on independent skin patches, penetration, over the hold interval
    for (name, hand), gname in hold.items():
        only = props[name].get("contact_clips")
        if only is not None and clip not in only:
            continue  # e.g. hand_open / hand_closed exported beside the grip: no contact intended
        t_att = events.get(name)
        tr = tpl.get("tracks", {}).get("left_hand" if hand == ch.jname("lhand") else "right_hand", [])
        t_grip = next((k["t"] for k in tr if gname in (k.get("grip"), (k.get("carry") or {}).get("grip"))), None)
        t_end = next((k["t"] for k in tr if t_grip is not None and k["t"] > t_grip and not (gname in (k.get("grip"), (k.get("carry") or {}).get("grip")))), None)
        ends = [e["t"] for e in tpl.get("events", []) if t_grip is not None and e["t"] > t_grip + 1e-6
                and name in (e.get("nock"), e.get("release")) or (e.get("attach") == name and e.get("to") == "world" and t_grip is not None and e["t"] > t_grip)]
        if ends:
            t_end = min([t_end] + ends) if t_end is not None else min(ends)
        t0 = t_grip if t_grip is not None else t_att
        if t_att is not None and t0 is not None and t_att > t0:
            t0 = t_att  # judged from the moment the hand owns the prop
        start = 0 if t0 is None else int(np.searchsorted(grid, f0 + t0 * fps - 1e-6))
        stop = len(rows) if t_end is None else int(np.searchsorted(grid, f0 + t_end * fps - 1e-6))
        rel = [r["holds"][(name, hand)] for r in rows[start:stop] if (name, hand) in r["holds"]]
        if not rel:
            continue
        tag = name + ("_left" if hand == ch.jname("lhand") else "_right")
        intended = [k for k, v in rel[0]["patch"].items() if v <= LIMITS["prop_gap_m"] * s]
        frac = {k: float(np.mean([r["patch"][k] <= LIMITS["prop_gap_m"] * s for r in rel])) for k in intended}
        pens = np.array([r["pen"] for r in rel])
        kind = info.get("grips", {}).get(gname, {}).get("kind", "power")
        M[f"prop_{tag}_wrap_deg"] = {"value": float(rel[0].get("wrap", 0.0)), "higher_is_better": True, "applies": kind == "power",
                                     "limit_key": "grip_wrap_deg", "focus": "l_hand" if hand == ch.jname("lhand") else "r_hand"}
        digits = {d: min((v for k, v in rel[0]["patch"].items() if k.startswith(d)), default=np.inf) / s for d in F.DIGITS}
        M[f"prop_{tag}_fingers_in_contact"] = {"value": int(sum(digits[d] <= 0.004 for d in F.DIGITS[1:])),
                                               "thumb": bool(digits["thumb"] <= 0.004), "higher_is_better": True,
                                               "limit_key": "fingers_in_contact", "digit_gap_m": {d: round(float(v), 4) for d, v in digits.items()},
                                               "focus": "l_hand" if hand == ch.jname("lhand") else "r_hand"}
        M[f"prop_{tag}_contact_fraction"] = {
            "value": float(min(frac.values())) if frac else 0.0, "per_patch": frac, "intended_patches": intended,
            "worst_gap_m": {k: float(max(r["patch"][k] for r in rel) / s) for k in intended},
            "first_frame_gaps_m": {k: round(v / s, 4) for k, v in rel[0]["patch"].items()},
            "higher_is_better": True, "limit_key": "prop_contact_fraction", "focus": "l_hand" if hand == ch.jname("lhand") else "r_hand"}
        M[f"prop_{tag}_penetration_m"] = {"value": float(pens.max() / s), "frame": frame_of(start + int(pens.argmax())),
                                          "limit_key": "prop_penetration_m", "focus": "l_hand" if hand == ch.jname("lhand") else "r_hand"}
        Ts = [np.linalg.inv(r["hand_M"]) @ r["M"] for r in rel]
        keyed = [T for T, t in zip(Ts, grid[start:stop]) if abs(t - round(t)) < 1e-6] or Ts
        dT = max(float(np.linalg.norm(T[:3, 3] - keyed[0][:3, 3])) for T in keyed) / s
        dR = max(angle(T[:3, 2], keyed[0][:3, 2]) for T in keyed)
        between = max(float(np.linalg.norm(T[:3, 3] - keyed[0][:3, 3])) for T in Ts) / s
        M[f"prop_{tag}_between_keys_m"] = {"value": between, "limit_key": "prop_between_keys_m", "focus": "body",
                                           "note": "drift relative to the hand at half frames, as an engine interpolating the prop node and the bones sees it"}
        M[f"prop_{tag}_wobble_m"] = {"value": dT, "rotation_deg": dR, "between_keys_m": between, "focus": "body",
                                     "limit_key": "bow_wobble_m",
                                     "note": "drift of the prop relative to the hand bone at the 60 Hz keys; between_keys_m is the drift at "
                                             "half frames, where an engine interpolates a separate prop node and the bone chain differently"}
    for name, p in props.items():
        if p["cls"] != "quiver":
            dep = [(r["props"][name].get("body_depth", 0.0), i, r["props"][name].get("body_where")) for i, r in enumerate(rows)]
            w = max(dep, key=lambda x: x[0])
            M[f"prop_{name}_through_body_m"] = {"value": float(w[0] / s), "frame": frame_of(w[1]), "where": w[2],
                                               "limit_key": "prop_through_body_m", "focus": "body",
                                               "note": "the held prop against every body vertex except the holding hands and forearms"}
        if p["cls"] == "quiver":
            q = np.array([r["props"][name].get("body_pen", 0) for r in rows])
            M["quiver_penetration_m"] = {"value": float(q.max() / s), "frame": frame_of(int(q.argmax())), "focus": "body"}
    # ownership changes: the prop's world transform on both sides of the event, sub-frame
    for name, t_att in events.items():
        if name not in props:
            continue
        obj = props[name]["obj"]
        fr = f0 + t_att * fps
        pos = []
        for eps in (-1.0, -1e-3, 1e-3, 1.0):
            set_frame(fr + eps * (1 if abs(eps) < 1 else 1))
            pos.append(np.array(obj.matrix_world.translation[:]))
        jump = float(np.linalg.norm(pos[2] - pos[1]) / s)
        v_before = (pos[1] - pos[0]) * fps / s
        v_after = (pos[3] - pos[2]) * fps / s
        M[f"ownership_{name}_jump_m"] = {"value": jump, "limit_key": "ownership_jump_m", "frame": round(fr, 3), "velocity_before_ms": float(np.linalg.norm(v_before)),
                                         "velocity_after_ms": float(np.linalg.norm(v_after)), "focus": "r_hand"}
    archer(ch, clip, M, rows, grid, f0, info, props, s, fps)
    for k, v in M.items():
        lim = LIMITS.get(v.get("limit_key", k))
        if lim is None or "value" not in v:
            continue
        v["limit"] = lim
        if v.get("applies", True):
            v["pass"] = bool(v["value"] >= lim) if v.get("higher_is_better") else bool(v["value"] <= lim + 1e-12)
    return {"frames": n, "duration_s": float((f1 - f0) / fps), "measures": M,
            "pass": all(v.get("pass", True) for v in M.values()), "f0": float(f0)}


def archer(ch, clip, M, rows, grid, f0, info, props, s, fps):
    """Bow, string and arrow: nock on string, shaft on rest, hook fingers on string, release and flight."""
    tpl = info.get("template")
    if not tpl or "string" not in tpl.get("tracks", {}):
        return
    arrow = next((n for n, p in props.items() if p["cls"] == "arrow"), None)
    bow = next((n for n, p in props.items() if p["cls"] == "bow"), None)
    if not arrow or not bow:
        return
    tr = tpl["tracks"]
    t_nock = next((k["t"] for k in tr.get(arrow, []) if k.get("nocked")), None)
    t_rel = next((k["t"] for k in tr.get(arrow, []) if k.get("release")), None)
    t_draw = next((k["t"] for k in tr.get("string", []) if k.get("follow")), None)
    A_arrow = props[arrow]["anchors"]
    A_bow = props[bow]["anchors"]
    rest_off = A_bow.get("riser_half_thickness", 0) + A_arrow.get("grip_radius", 0)

    def in_window(i, a, b):
        tt = (grid[i] - f0) / fps
        return a is not None and b is not None and a - 1e-6 <= tt <= b - 1e-6

    nock_d, rest_d, hook_d = [], [], []
    for i, r in enumerate(rows):
        e = r["empties"]
        if in_window(i, t_nock, t_rel):
            nk = e.get(f"{arrow}.nock")
            sn = e.get("bow_string_nock")
            if nk is not None and sn is not None:
                nock_d.append((float(np.linalg.norm(nk - sn) / s), i))
            Ma = r["props"][arrow]["M"]
            axis = F.unit(Ma[:3, :3] @ np.asarray(A_arrow["shaft_axis"]))
            p0 = Ma[:3, :3] @ np.asarray(A_arrow["nock"]) + Ma[:3, 3]
            rp = e.get(f"{bow}.arrow_rest")
            if rp is not None:
                v = rp - p0
                dist = np.linalg.norm(v - axis * (v @ axis))
                rest_d.append((float(abs(dist - rest_off) / s), i))
    if nock_d:
        w = max(nock_d)
        M["nock_to_string_m"] = {"value": w[0], "frame": round(float(grid[w[1]]), 3), "frames": len(nock_d), "focus": "r_hand"}
    if rest_d:
        w = max(rest_d)
        M["shaft_to_rest_m"] = {"value": w[0], "frame": round(float(grid[w[1]]), 3), "frames": len(rest_d), "focus": "l_hand"}
    # draw fingers to string: nearest hook-finger skin to the two string segments while drawing
    if t_draw is not None and t_rel is not None:
        worst, wi = 0.0, 0
        ups = [o for o in ch.objects if o.name.startswith("bow_string_upper")]
        for i in range(len(rows)):
            if not in_window(i, t_draw, t_rel):
                continue
            set_frame(grid[i])
            coords = ch.skin()
            pts, owner, patches = ch.hand_patches("r", coords)
            ix = np.r_[tuple(patches.get(k, np.zeros(0, int)) for k in ("index_1", "index_2", "middle_1", "middle_2", "ring_1", "ring_2"))].astype(int)
            e = {o.name: np.array(o.matrix_world.translation[:]) for o in ch.objects if o.type == "EMPTY" or o.name.startswith("bow_")}
            sn = e.get("bow_string_nock")
            tips = [e.get(f"{bow}.upper_tip"), e.get(f"{bow}.lower_tip")]
            if sn is None or any(tp is None for tp in tips) or not len(ix):
                continue
            best = np.inf
            for tp in tips:
                seg = sn - tp
                u = np.clip(((pts[ix] - tp) @ seg) / (seg @ seg), 0, 1)
                best = min(best, float(np.linalg.norm(pts[ix] - (tp + u[:, None] * seg), axis=1).min()))
            if best > worst:
                worst, wi = best, i
        M["draw_fingers_to_string_m"] = {"value": worst / s, "frame": round(float(grid[wi]), 3), "focus": "r_hand"}
    rel = info.get("release") or {}
    if t_rel is not None and rel:
        obj = props[arrow]["obj"]
        fr = f0 + t_rel * fps
        def pos(f):
            set_frame(f)
            return np.array(obj.matrix_world.translation[:]), F.matrix_np(obj.matrix_world)
        pa, _ = pos(fr - 1e-3)
        pb, _ = pos(fr + 1e-3)
        M["release_position_jump_m"] = {"value": float(np.linalg.norm(pb - pa) / s), "frame": round(fr, 3), "focus": "r_hand"}
        Tp = float(rel.get("power_s", 0.0))
        v0 = float(rel.get("v0", 0.0))
        # flight: positions at 60 Hz after separation; fit a parabola, compare g and the tangent
        ts = np.arange(1, 25) / fps
        P, tang = [], []
        for tt in ts:
            p, Mo = pos(fr + (Tp + tt) * fps)
            P.append(p)
            tang.append(F.unit(Mo[:3, :3] @ np.asarray(A_arrow["shaft_axis"])))
        P = np.array(P) / s
        X = np.column_stack((np.ones_like(ts), ts, 0.5 * ts * ts))
        coef, *_ = np.linalg.lstsq(X, P, rcond=None)
        resid = float(np.linalg.norm(X @ coef - P, axis=1).max())
        vel = np.gradient(P, ts, axis=0)
        tdev = max(angle(tg, v) for tg, v in zip(tang, vel))
        M["flight_residual_m"] = {"value": resid, "fitted_gravity_ms2": float(-coef[2][2]), "launch_speed_ms": float(np.linalg.norm(coef[1])),
                                  "model_speed_ms": v0, "draw_length_m": rel.get("draw_m"), "power_stroke_s": Tp, "focus": "body"}
        M["flight_tangent_deg"] = {"value": float(tdev), "focus": "body"}
        # string return: forward overshoot past the brace line (towards the target), relative to brace height
        sn_obj = next((o for o in ch.objects if o.name == "bow_string_nock"), None)
        rp_obj = next((o for o in ch.objects if o.name == f"{bow}.nock_rest"), None)
        if sn_obj and rp_obj:
            worst = 0.0
            for k in range(1, 40):
                set_frame(fr + Tp * fps + k * 0.5)
                d = np.array(sn_obj.matrix_world.translation[:]) - np.array(rp_obj.matrix_world.translation[:])
                aim = F.unit(np.array(rel.get("axis", [0, 0, 0])))
                worst = max(worst, float(d @ aim))
            brace = A_bow.get("brace", 1.0)
            M["string_forward_overshoot_brace"] = {"value": worst / max(brace, 1e-9), "focus": "l_hand"}


def rest_limb_penetration(ch, proxies):
    ch.rest_pose()
    worst = 0.0
    for side in "lr":
        pts = limb_points(ch, side, ch.rest)
        for bone, px in proxies.items():
            worst = max(worst, float(-px.signed(pts, ch.world(bone), exact_below=0).min()))
    return worst


def load(job):
    ch = F.Character(job["mesh"], job.get("height_m", 1.8), keep_animation=True)
    extras = {}
    if "forge_fit" in ch.rig.keys():
        try:
            extras = json.loads(ch.rig["forge_fit"])
        except (TypeError, ValueError):
            extras = {}
    props = {}
    for o in ch.objects:
        if o.type != "MESH" or o in ch.meshes or o.name.startswith(("bow_string", "UCX_", "Icosphere")):
            continue
        if job.get("prop_node") and o.name != job["prop_node"] and not extras:
            continue
        cls = "prop"
        for info in extras.values():
            cls = info.get("props", {}).get(o.name, {}).get("class", cls)
        meta = json.loads(o["forge_fit"]) if "forge_fit" in o.keys() else {}
        cls = meta.get("class", cls)
        anchors = meta.get("anchors") or next((i["props"][o.name]["anchors"] for i in extras.values() if o.name in i.get("props", {})), {})
        hand = o.parent_bone if o.parent_type == "BONE" else None
        props[o.name] = {"obj": o, "proxy": prop_proxy(o), "cls": cls, "anchors": anchors, "hand": hand,
                         "hands": meta.get("hands"), "contact_clips": meta.get("contact_clips")}
    return ch, props, extras


def hinges_of(ch):
    j = ch.jname
    out = {}
    for sd in "lr":
        out["knee_" + sd] = Hinge(ch, j(sd + "thigh"), j(sd + "shin"), j(sd + "foot"), ch.forward)
        out["elbow_" + sd] = Hinge(ch, j(sd + "upperarm"), j(sd + "forearm"), j(sd + "hand"), -ch.forward)
    return out


def check_all(ch, props, extras, clips=None):
    ch.rest_pose()
    soles = F.SoleSampler(ch.rig, ch.objects, ch.joints)
    hinges = hinges_of(ch)
    proxies = body_proxies(ch)
    ch._rest_limb = rest_limb_penetration(ch, proxies)
    clips = clips or ch.tracks
    out = {}
    for k, clip in enumerate(clips):
        F.progress(10 + 70 * k / max(len(clips), 1), f"Checking {clip}")
        out[clip] = check_clip(ch, clip, props, extras, soles, hinges, proxies)
    ch.rest_pose()
    return out


def contact_sheet(ch, results, out_png, max_rows=8):
    """Worst frames: full body beside a close-up, for failing measures first."""
    items = []
    for clip, r in results.items():
        for key, m in r["measures"].items():
            if "frame" in m:
                items.append((not m.get("pass", True), clip, key, m))
    items.sort(key=lambda x: (not x[0],))
    items = [i for i in items if i[0]][:max_rows] or items[:4]
    if not items:
        return None
    work = Path(out_png).parent / (Path(out_png).stem + "_frames")
    work.mkdir(parents=True, exist_ok=True)
    cam, label = RN.setup(360)
    paths = []
    for failing, clip, key, m in items:
        F.set_source_action(ch.rig, ch.objects, set(bpy.data.actions), clip)
        set_frame(m["frame"])
        bpy.context.view_layer.update()
        coords = ch.skin()
        lo, hi = coords.min(axis=0), coords.max(axis=0)
        centre = (lo + hi) / 2
        span = float(max(hi - lo)) * 1.15
        focus = m.get("focus", "body")
        bone = {"foot": None, "hand": None, "l_hand": ch.jname("lhand"), "r_hand": ch.jname("rhand")}.get(focus)
        if focus.startswith("knee"):
            bone = ch.jname(focus[-1] + "shin")
        elif focus.startswith("elbow"):
            bone = ch.jname(focus[-1] + "forearm")
        elif focus == "foot":
            bone = ch.jname("lfoot") if "l" in str(m.get("joint", "l")) else ch.jname("rfoot")
        elif focus == "hand":
            bone = ch.jname("rhand")
        verdict = "FAIL" if failing else "ok"
        text = f"{clip} f{m['frame']:.1f} {key} {m['value']:.3g} {verdict}"
        RN.view(cam, label, centre, ch.forward * -1 + ch.right * 0.6 + np.array([0, 0, 0.25]), span, text)
        paths.append(RN.shot(work / f"{len(paths):03d}_body.png"))
        if bone:
            c = ch.head(bone)
            RN.view(cam, label, c, ch.right * 1.0 - ch.forward * 0.4 + np.array([0, 0, 0.2]), ch.height * 0.3, key)
        else:
            RN.view(cam, label, centre, ch.right, span, key + " side")
        paths.append(RN.shot(work / f"{len(paths):03d}_close.png"))
    return RN.sheet(paths, 4, out_png)


def cmd_check(job):
    ch, props, extras = load(job)
    results = check_all(ch, props, extras, job.get("clips"))
    sheet = None
    if job.get("sheet"):
        F.progress(85, "Rendering the worst frames")
        sheet = contact_sheet(ch, results, job["sheet"])
    return {"clips": results, "limits": LIMITS, "pass": all(r["pass"] for r in results.values()),
            "scene_units_per_m": ch.s, "height_scene": ch.height, "props": {k: v["cls"] for k, v in props.items()}, "sheet": sheet}


def rewrite_rot(ch, act, bone, times, quats):
    """Replace one bone's rotation curves in a clip with keys at the given frames."""
    slot = act.slots[0]
    bag = act.layers[0].strips[0].channelbag(slot)
    path = ch.rig.pose.bones[bone].path_from_id("rotation_quaternion")
    for i in range(4):
        c = bag.fcurves.find(path, index=i)
        if c:
            bag.fcurves.remove(c)
    prev = None
    vals = []
    for q in quats:
        q = q.copy()
        if prev is not None:
            q.make_compatible(prev)
        prev = q
        vals.append(tuple(q))
    for i in range(4):
        F.add_curve(bag, path, i, times, vals)


def key_times(act, f0, f1):
    ts = set(np.round(np.r_[np.arange(f0, f1 - 1e-4, 1.0), f1], 6))
    return sorted(ts)


def clamp_hinges(ch, clip, hinges):
    """Turn a knee or elbow bent the wrong way back to 2 degrees of correct flexion, frame by frame."""
    import motion as MO
    rg = MO.Rig(ch)
    act = ch.rig.animation_data.action
    f0, f1 = act.frame_range
    times = key_times(act, f0, f1)
    changed = {}
    per_bone = {}
    for t in times:
        set_frame(t)
        for key, h in hinges.items():
            upper, lower, end = h.bones
            per_bone.setdefault(lower, [])
            flex = h.measure()[0]
            q = ch.rig.pose.bones[lower].rotation_quaternion.copy()
            if flex < 0:
                a, b, c = (ch.head(x) for x in h.bones)
                Rw = np.array((ch.rig.matrix_world @ ch.rig.pose.bones[upper].matrix).to_3x3())
                front = F.unit(Rw @ h.front_local)
                u, v = F.unit(b - a), F.unit(c - b)
                fp = F.unit(front - u * (front @ u))
                want = F.unit(u * math.cos(math.radians(2)) - fp * math.sin(math.radians(2)))
                rg.rotate_world(lower, MO.rot_between(v, want))
                bpy.context.view_layer.update()
                q = ch.rig.pose.bones[lower].rotation_quaternion.copy()
                changed[key] = changed.get(key, 0) + 1
            per_bone[lower].append(q)
    for bone, qs in per_bone.items():
        if any(k for k, h in hinges.items() if h.bones[1] == bone and k in changed):
            rewrite_rot(ch, act, bone, times, qs)
    return {"frames_clamped": changed}


def ground_feet(ch, clip, soles, before):
    """animlib's grounding (sole height and stance stabilisation), reused unchanged."""
    act = ch.rig.animation_data.action
    slot = ch.rig.animation_data.action_slot
    bag = act.layers[0].strips[0].channelbag(slot)
    path = soles.hip.path_from_id("location")
    f0, f1 = act.frame_range
    for i in range(3):
        if bag.fcurves.find(path, index=i) is None:
            F.add_curve(bag, path, i, [f0, f1], [(0, 0, 0), (0, 0, 0)])
    M = before["measures"]
    job = {"loopable": bool(M.get("loop_seam_m", {}).get("applies")), "root_motion": False,
           "category": "locomotion" if M.get("foot_slide_m", {}).get("belt_speed_ms", 0) > 0 else "other"}
    F.ground_action(soles, act, slot, job, F.FPS, F.add_curve)
    return {"job": job}


def push_arms(ch, clip, proxies, s):
    """Move a hand that sinks into the torso or a thigh outwards (shoulder and elbow re-solved by two-bone
    IK, wrist orientation kept), offsets smoothed over 0.1 s so nothing pops."""
    import types

    import motion as MO
    act = ch.rig.animation_data.action
    f0, f1 = act.frame_range
    times = key_times(act, f0, f1)
    offsets = {sd: [] for sd in "lr"}
    for t in times:
        set_frame(t)
        coords = ch.skin()
        for sd in "lr":
            pts = limb_points(ch, sd, coords)
            best = (0.0, None)
            for bone, px in proxies.items():
                Mb = ch.world(bone)
                d = px.signed(pts, Mb, exact_below=0)
                i = int(d.argmin())
                if -d[i] > best[0]:
                    axis_pt = Mb[:3, 3]
                    axis = F.unit(Mb[:3, 1])
                    v = pts[i] - axis_pt
                    radial = F.unit(v - axis * (v @ axis))
                    best = (-d[i], radial)
            depth, radial = best
            offsets[sd].append(radial * (depth + 0.01 * s) if radial is not None else np.zeros(3))
    sigma = 0.1 * F.FPS
    shim = types.SimpleNamespace(ch=ch, rg=MO.Rig(ch))
    out = {}
    for sd in "lr":
        off = np.array(offsets[sd])
        if not np.abs(off).sum():
            continue
        tt = np.array(times)
        w = np.exp(-0.5 * ((tt[:, None] - tt[None, :]) / sigma) ** 2)
        smooth = (w @ off) / w.sum(axis=1)[:, None]
        # never less than the raw push where the raw push is needed
        mag = np.linalg.norm(off, axis=1)
        sm = np.linalg.norm(smooth, axis=1)
        smooth *= np.where(sm > 1e-12, np.maximum(1.0, mag / np.maximum(sm, 1e-12)), 1.0)[:, None]
        j = ch.jname
        bones = (j(sd + "upperarm"), j(sd + "forearm"), j(sd + "hand"))
        qs = {b: [] for b in bones}
        for k, t in enumerate(times):
            set_frame(t)
            if np.linalg.norm(smooth[k]) > 1e-9:
                S, E, Wp = (ch.head(b) for b in bones)
                mid = (S + Wp) / 2
                pole = F.unit(E - mid) if np.linalg.norm(E - mid) > 1e-6 else -ch.forward
                limb = MO.Limb(shim, *bones, lambda pole=pole: pole)
                target = ch.world(bones[2])
                target[:3, 3] += smooth[k]
                limb.solve(target)
                bpy.context.view_layer.update()
            for b in bones:
                qs[b].append(ch.rig.pose.bones[b].rotation_quaternion.copy())
        for b in bones:
            rewrite_rot(ch, act, b, times, qs[b])
        out[sd] = {"frames_pushed": int((np.linalg.norm(off, axis=1) > 0).sum()), "max_push_m": float(mag.max() / s)}
    return out


def smooth_root(ch, clip, soles):
    """Blend the hips' acceleration: a Gaussian (sigma 50 ms) over the hip location keys around take-off and
    landing spikes; the rest of the clip keeps its keys."""
    act = ch.rig.animation_data.action
    slot = ch.rig.animation_data.action_slot
    bag = act.layers[0].strips[0].channelbag(slot)
    path = soles.hip.path_from_id("location")
    curves = [bag.fcurves.find(path, index=i) for i in range(3)]
    if any(c is None for c in curves):
        return {"skipped": "no hip location keys"}
    f0, f1 = act.frame_range
    times = np.arange(f0, f1 + 1e-6, 1.0)
    vals = np.array([[c.evaluate(t) for c in curves] for t in times])
    sig = 0.05 * F.FPS
    k = np.exp(-0.5 * (np.arange(-3 * sig, 3 * sig + 1) / sig) ** 2)
    k /= k.sum()
    pad = np.pad(vals, ((len(k) // 2, len(k) // 2), (0, 0)), mode="edge")
    sm = np.stack([np.convolve(pad[:, i], k, mode="valid") for i in range(3)], axis=1)
    acc = np.linalg.norm(np.diff(vals, 2, axis=0), axis=1)
    hot = np.zeros(len(times))
    if len(acc):
        thr = np.percentile(acc, 90)
        for i in np.flatnonzero(acc > thr):
            hot[max(0, i - int(2 * sig)):i + int(2 * sig) + 2] = 1
    w = np.convolve(hot, k, mode="same")
    w = np.clip(w / max(w.max(), 1e-9), 0, 1)
    out = vals * (1 - w[:, None]) + sm * w[:, None]
    for c in curves:
        bag.fcurves.remove(c)
    for i in range(3):
        F.add_curve(bag, path, i, list(times), [tuple(v) for v in out])
    return {"blended_frames": int((w > 0.05).sum())}


def cmd_correct(job):
    ch, props, extras = load(job)
    ch.rest_pose()
    soles = F.SoleSampler(ch.rig, ch.objects, ch.joints)
    hinges = hinges_of(ch)
    proxies = body_proxies(ch)
    ch._rest_limb = rest_limb_penetration(ch, proxies)
    report = {}
    for k, clip in enumerate(ch.tracks):
        F.progress(5 + 85 * k / max(len(ch.tracks), 1), f"Correcting {clip}")
        before = check_clip(ch, clip, props, extras, soles, hinges, proxies)
        cur = before
        steps = []
        plan = [("hinge_clamp", ("knee_hyperextension_deg", "elbow_hyperextension_deg"), lambda: clamp_hinges(ch, clip, hinges)),
                ("ground_feet", ("foot_penetration_m", "foot_slide_m"), lambda: ground_feet(ch, clip, soles, cur)),
                ("arms_out_of_torso", ("limb_body_penetration_m",), lambda: push_arms(ch, clip, proxies, ch.s)),
                ("smooth_root", ("hips_acceleration_ms2",), lambda: smooth_root(ch, clip, soles))]
        for name, targets, fn in plan:
            if all(cur["measures"].get(t, {}).get("pass", True) for t in targets):
                continue
            F.set_source_action(ch.rig, ch.objects, set(bpy.data.actions), clip)
            ad = ch.rig.animation_data
            act = ad.action
            backup = act.copy()
            detail = fn()
            after = check_clip(ch, clip, props, extras, soles, hinges, proxies)
            gain = sum(cur["measures"][t]["value"] - after["measures"][t]["value"] for t in targets if t in cur["measures"])
            worse = [m for m, v in after["measures"].items() if "value" in v and m in cur["measures"] and m not in targets
                     and not v.get("higher_is_better") and v["value"] > cur["measures"][m]["value"] * 1.05 + 1e-4
                     and v.get("pass", True) is False]
            keep = gain > 0 and not worse
            steps.append({"step": name, "kept": keep, "detail": detail, "worse": worse,
                          "targets": {t: [cur["measures"][t]["value"], after["measures"][t]["value"]] for t in targets if t in cur["measures"]}})
            track = next(t for t in ad.nla_tracks if t.name == clip)
            if keep:
                cur = after
                bpy.data.actions.remove(backup)
            else:
                track.strips[0].action = backup
                track.strips[0].action_slot = backup.slots[0]
                ad.action = None
                bpy.data.actions.remove(act)
                backup.name = clip
        report[clip] = {"before": {m: v.get("value") for m, v in before["measures"].items()},
                        "after": {m: v.get("value") for m, v in cur["measures"].items()},
                        "pass_before": before["pass"], "pass_after": cur["pass"], "steps": steps,
                        "failing_after": [m for m, v in cur["measures"].items() if v.get("pass") is False]}
    ch.rest_pose()
    F.export(job["output"])
    return {"clips": report, "limits": LIMITS, "scene_units_per_m": ch.s}


COMMANDS = {"check": cmd_check, "correct": cmd_correct}
