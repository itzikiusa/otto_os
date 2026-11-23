"""forge-fit's foot, ground and loop checks for any body plan, on the deformed skin, without Blender.

The humanoid check (measure.py, in Blender) knows two feet by name. This one takes the legs the body plan
found (forge/bodyplan.py), any number of them, and measures each the same way with the same limits:

* foot_penetration_m: how far a sole goes below the rest floor;
* foot_slide_m: how far a planted sole moves, after the shared belt speed of an in-place gait is removed
  (planted: within 2% of the height of the floor and slower than 0.5 m/s relative to that belt, for at
  least 0.1 s; the belt is first fitted to every sole on the floor, so a fast in-place trot is still judged);
* loop_seam_m: how far any joint is from where it started, after the root's horizontal travel
  (judged on clips whose name says they loop: loop, walk, run, idle);
* hips_acceleration_ms2: the root's acceleration, from samples 1/15 s apart (a pop between keys);
* ground_penetration_m: the lowest skin of everything that is not a leg (a legless body: all of it) below the
  rest floor, so a tail, a belly or a leg the skeleton missed cannot sink through it either;
* leg_swing_m (walk and run only, a lower bound): the least any foot travels under the body. A walk whose
  legs stand still while the body moves is a slide, whatever the other measures say.

Distances are metres: glTF metres, or scaled so the body is --height tall when one is given.
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
from forge.bodyplan import Rig, accessor, detect, quat_matrix, trs  # noqa: E402

# the same numbers as measure.py's LIMITS (tests/rig_any checks they stay equal)
LIMITS = {"foot_penetration_m": 0.010, "foot_slide_m": 0.050, "loop_seam_m": 0.010, "hips_acceleration_ms2": 150.0}
LIMITS["ground_penetration_m"] = LIMITS["foot_penetration_m"]
LIMITS["leg_swing_m"] = 0.02  # walk/run only, a lower bound: every foot travels at least 2 cm under the body
STEP = 1 / 60


def soles(rig: Rig, plan: dict) -> list[np.ndarray]:
    """Per leg, the vertex ids of its sole: the leg's lowest skin (within 3% of the height of its bottom)."""
    out = []
    for g in plan["leg_chains"]:
        ids = np.flatnonzero(np.isin(rig.dominant, g["chain"] + g.get("twins", [])))
        y = rig.skin_rest[ids, 1]
        out.append(ids[y <= y.min() + 0.03 * plan["height"]])
    return out


def measure(rig: Rig, plan: dict, clip: str, duration: float, world_at, height_m=None) -> dict:
    """Measures for one clip; `world_at(t)` gives the joint world matrices (n, 4, 4) at time t."""
    s = plan["height"] / height_m if height_m else 1.0  # scene units per metre
    times = np.r_[np.arange(0.0, duration - 1e-9, STEP), duration]
    n = len(times)
    root = next(k for k in rig.order if rig.parent[k] < 0)
    sole_ids = soles(rig, plan)
    # the rest of the body (a legless body: all of it) must not sink through the floor either
    legs = [j for g in plan["leg_chains"] for j in g["chain"] + g.get("twins", [])]
    body = ~np.isin(rig.dominant, legs)
    bottom = np.flatnonzero(body & (rig.skin_rest[:, 1] <= plan["floor"] + 0.05 * plan["height"]))
    spread = np.flatnonzero(body)[::max(1, int(body.sum()) // 3000)]
    rest_ids = np.unique(np.r_[spread, bottom]).astype(int)
    feet = np.zeros((n, len(sole_ids), 3))  # (lowest height, x, z) per leg
    lowest = np.zeros(n)
    joints = np.zeros((n, len(rig.names), 3))
    for i, t in enumerate(times):
        W = world_at(float(t))
        joints[i] = W[:, :3, 3]
        for leg, ids in enumerate(sole_ids):
            p = rig.skin(W, ids)
            feet[i, leg] = (p[:, 1].min(), p[:, 0].mean(), p[:, 2].mean())
        lowest[i] = rig.skin(W, rest_ids)[:, 1].min() if len(rest_ids) else np.inf
    floor, height = plan["floor"], plan["height"]
    M = {}
    if sole_ids:
        pen = np.maximum(0.0, floor - feet[:, :, 0])
        i, leg = np.unravel_index(pen.argmax(), pen.shape)
        M["foot_penetration_m"] = {"value": float(pen.max() / s), "t": round(float(times[i]), 3),
                                   "leg": plan["leg_chains"][leg]["name"]}
        def planted(belt):
            """Stretches of at least 0.1 s with the sole on the floor and slower than 0.5 m/s on the belt
            (belt None: any speed)."""
            runs = []
            for leg in range(len(sole_ids)):
                xz = feet[:, leg, 1:]
                down = feet[:, leg, 0] <= floor + 0.02 * height
                if belt is not None and n > 1:
                    down &= np.linalg.norm(np.gradient(xz, STEP, axis=0) - belt, axis=1) / s < 0.5
                low = np.flatnonzero(down)
                for ix in np.split(low, np.where(np.diff(low) != 1)[0] + 1):
                    if len(ix) * STEP >= 0.1:
                        runs.append((leg, ix, xz[ix]))
            return runs

        def belt_of(runs):
            """The one velocity every planted sole shares (an in-place gait's treadmill), or zero."""
            num, den = np.zeros(2), 0.0
            for _, ix, p in runs:
                tt = ix * STEP - (ix * STEP).mean()
                num += tt @ (p - p.mean(axis=0))
                den += tt @ tt
            v = num / max(den, 1e-12)
            return v if np.linalg.norm(v) / s >= 0.05 else np.zeros(2)

        # the belt: the usual velocity of a sole on the floor (stance outlasts the low ends of a swing),
        # then the plants are measured on it
        on_floor = [np.gradient(feet[:, leg, 1:], STEP, axis=0)[feet[:, leg, 0] <= floor + 0.02 * height]
                    for leg in range(len(sole_ids))] if n > 1 else []
        on_floor = np.concatenate(on_floor) if on_floor else np.zeros((0, 2))
        vel = np.median(on_floor, axis=0) if len(on_floor) else np.zeros(2)
        runs = planted(vel)
        vel = belt_of(runs) if runs else vel
        per_leg = {g["name"]: {"slide_m": 0.0, "planted_intervals": 0, "swing_m": 0.0} for g in plan["leg_chains"]}
        worst = (0.0, 0, None)
        for leg, ix, p in runs:
            q = p - (ix * STEP)[:, None] * vel
            d = float(np.linalg.norm(q - q[0], axis=1).max())
            entry = per_leg[plan["leg_chains"][leg]["name"]]
            entry["slide_m"] = round(max(entry["slide_m"], d / s), 5)
            entry["planted_intervals"] += 1
            if d > worst[0]:
                worst = (d, int(ix[np.linalg.norm(q - q[0], axis=1).argmax()]), plan["leg_chains"][leg]["name"])
        root_xz = joints[:, root][:, [0, 2]]
        for leg, g in enumerate(plan["leg_chains"]):  # how far each foot travels under the body
            xz = feet[:, leg, 1:] - root_xz
            per_leg[g["name"]]["swing_m"] = round(float(np.linalg.norm(xz - xz.mean(axis=0), axis=1).max() * 2 / s), 4)
        M["foot_slide_m"] = {"value": worst[0] / s, "belt_speed_ms": float(np.linalg.norm(vel) / s),
                             "t": round(float(times[worst[1]]), 3), "leg": worst[2], "planted_intervals": len(runs),
                             "legs": per_leg}
        if any(w in clip.lower() for w in ("walk", "run")):  # a walk whose legs do not step is a slide
            still = min(per_leg, key=lambda k: per_leg[k]["swing_m"])
            M["leg_swing_m"] = {"value": per_leg[still]["swing_m"], "leg": still, "at_least": True}
    if len(rest_ids):
        i = int(lowest.argmin())
        M["ground_penetration_m"] = {"value": float(max(0.0, floor - lowest.min()) / s), "t": round(float(times[i]), 3)}
    travel = joints[-1, root] - joints[0, root]
    travel[1] = 0.0
    seam = float(np.linalg.norm(joints[-1] - joints[0] - travel, axis=1).max() / s)
    looped = any(w in clip.lower() for w in ("loop", "walk", "run", "idle"))
    M["loop_seam_m"] = {"value": seam, "applies": bool(looped and n > 4)}
    hips = joints[:, root]
    h4 = hips[::4]
    if len(h4) > 2:
        acc = np.linalg.norm(np.diff(h4, 2, axis=0), axis=1) / (4 * STEP) ** 2 / s
        M["hips_acceleration_ms2"] = {"value": float(acc.max()), "t": round(float(times[min(n - 1, 4 * (int(acc.argmax()) + 1))]), 3)}
    for key, m in M.items():
        m["limit"] = LIMITS[key]
        ok = m["value"] >= LIMITS[key] if m.get("at_least") else m["value"] <= LIMITS[key]
        m["pass"] = (not m.get("applies", True)) or ok
    return {"measures": M, "pass": all(m["pass"] for m in M.values())}


def _slerp(a, b, f):
    d = float(a @ b)
    if d < 0:
        b, d = -b, -d
    if d > 0.9995:
        q = a + f * (b - a)
    else:
        th = np.arccos(d)
        q = (np.sin((1 - f) * th) * a + np.sin(f * th) * b) / np.sin(th)
    return q / np.linalg.norm(q)


def clip_sampler(rig: Rig, animation: dict):
    """(duration, world_at) for one glTF animation of the rig (LINEAR or STEP channels)."""
    doc, binary = rig.doc, rig.binary
    index = {node: k for k, node in enumerate(rig.nodes)}
    tracks = {}
    duration = 0.0
    for ch in animation["channels"]:
        node = ch["target"].get("node")
        if node not in index or ch["target"]["path"] not in ("translation", "rotation", "scale"):
            continue
        smp = animation["samplers"][ch["sampler"]]
        times = accessor(doc, binary, smp["input"])[:, 0]
        values = accessor(doc, binary, smp["output"])
        if smp.get("interpolation") == "CUBICSPLINE":
            values = values[1::3]
        tracks[(index[node], ch["target"]["path"])] = (times, values, smp.get("interpolation", "LINEAR"))
        duration = max(duration, float(times[-1]))

    def value(j, path, t, rest):
        if (j, path) not in tracks:
            return rest
        times, values, mode = tracks[(j, path)]
        if t <= times[0]:
            return values[0]
        if t >= times[-1]:
            return values[-1]
        i = int(np.searchsorted(times, t, side="right") - 1)
        if mode == "STEP":
            return values[i]
        f = (t - times[i]) / (times[i + 1] - times[i])
        if path == "rotation":
            return _slerp(values[i], values[i + 1], f)
        return values[i] + f * (values[i + 1] - values[i])

    def world_at(t):
        W = np.zeros((len(rig.nodes), 4, 4))
        for j in rig.order:
            T, Q, S = rig.rest_local[j]
            local = trs(value(j, "translation", t, T), value(j, "rotation", t, Q), value(j, "scale", t, S))
            p = rig.parent[j]
            W[j] = (W[p] @ rig.offset[j] if p >= 0 else rig.offset[j]) @ local
        return W

    return duration, world_at


def check_glb(path, height_m=None, clips=None) -> dict:
    """Every clip (or `clips`) of a rigged GLB, measured per detected leg."""
    rig = Rig(path)
    plan = detect(rig)
    out = {}
    for animation in rig.doc.get("animations", []):
        name = animation.get("name", "")
        if clips and name not in clips:
            continue
        duration, world_at = clip_sampler(rig, animation)
        out[name] = measure(rig, plan, name, duration, world_at, height_m)
    return {"body_plan": plan["body_plan"], "legs": [g["name"] for g in plan["leg_chains"]], "clips": out,
            "limits": LIMITS, "pass": all(r["pass"] for r in out.values()),
            "scene_units_per_m": plan["height"] / height_m if height_m else 1.0}
