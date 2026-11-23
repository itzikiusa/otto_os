"""The scene checks: a built scene3d document against its plan and what the
description asked for (corpus.json "expect"). Pure geometry on the placed
pieces (stand-in parts, or a real model's bounds box), no Otto code.

    python3 checks.py SCENE_DIR   # reads plan.json, scene.json, parts/, vis.json; writes result.json
"""
import json
import math
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent


def words(text):
    out = []
    for w in re.sub(r"[^a-z]+", " ", (text or "").lower()).split():
        out.append(w)
        if w.endswith("s") and len(w) > 3:
            out.append(w[:-1])
    return out


# ---------------------------------------------------------------- geometry
class Box:
    """A part in the world: centre, half sizes (its own axes) and yaw."""

    def __init__(self, c, h, yaw, owner, part):
        self.c, self.h, self.yaw, self.owner, self.part = c, h, yaw, owner, part

    def axes(self):
        t = math.radians(self.yaw)
        # glTF yaw about +y: local x -> (cos, -sin) in (x, z); local z -> (sin, cos)
        return [(math.cos(t), -math.sin(t)), (math.sin(t), math.cos(t))]

    def corners2d(self):
        (ax, az), (bx, bz) = self.axes()
        cx, cz = self.c[0], self.c[2]
        hx, hz = self.h[0], self.h[2]
        return [(cx + sx * hx * ax + sz * hz * bx, cz + sx * hx * az + sz * hz * bz) for sx in (-1, 1) for sz in (-1, 1)]

    @property
    def lo_y(self):
        return self.c[1] - self.h[1]

    @property
    def hi_y(self):
        return self.c[1] + self.h[1]

    def contains_xz(self, p, grow=0.0):
        dx, dz = p[0] - self.c[0], p[1] - self.c[2]
        (ax, az), (bx, bz) = self.axes()
        return abs(dx * ax + dz * az) <= self.h[0] + grow and abs(dx * bx + dz * bz) <= self.h[2] + grow


def overlap_depth(a, b):
    """How far two boxes interpenetrate (0 when apart): the smallest overlap
    over the y axis and the four separating axes in x, z."""
    dy = min(a.hi_y, b.hi_y) - max(a.lo_y, b.lo_y)
    if dy <= 0:
        return 0.0
    best = dy
    ca, cb = a.corners2d(), b.corners2d()
    for ax in a.axes() + b.axes():
        pa = [x * ax[0] + z * ax[1] for x, z in ca]
        pb = [x * ax[0] + z * ax[1] for x, z in cb]
        d = min(max(pa), max(pb)) - max(min(pa), min(pb))
        if d <= 0:
            return 0.0
        best = min(best, d)
    return best


def gap2d(a, b):
    """Horizontal distance between two boxes' footprints (0 when they overlap)."""
    ca, cb = a.corners2d(), b.corners2d()
    for ax in a.axes() + b.axes():
        pa = [x * ax[0] + z * ax[1] for x, z in ca]
        pb = [x * ax[0] + z * ax[1] for x, z in cb]
        if max(pa) < min(pb) or max(pb) < min(pa):
            break
    else:
        return 0.0

    def seg_pt(p, a, b):
        vx, vz = b[0] - a[0], b[1] - a[1]
        L = vx * vx + vz * vz
        t = 0 if L == 0 else max(0, min(1, ((p[0] - a[0]) * vx + (p[1] - a[1]) * vz) / L))
        return math.hypot(p[0] - a[0] - t * vx, p[1] - a[1] - t * vz)

    def edges(c):
        o = [c[0], c[1], c[3], c[2]]
        return [(o[i], o[(i + 1) % 4]) for i in range(4)]

    return min(min(seg_pt(p, *e) for p in ca for e in edges(cb)), min(seg_pt(p, *e) for p in cb for e in edges(ca)))


class Piece:
    def __init__(self, obj, model):
        self.id = obj["id"]
        # Per axis: the scene scales a piece toward the plan's dims_m.
        p, yaw = obj["position"], obj["rotation"][1]
        s = list(obj["scale"]) if isinstance(obj["scale"], list) else [obj["scale"]] * 3
        self.yaw, self.scale, self.pos = yaw, s, p
        self.boxes = []
        for part in model["parts"]:
            lo, hi = part["min"], part["max"]
            c = [(lo[k] + hi[k]) / 2 for k in range(3)]
            h = [s[k] * (hi[k] - lo[k]) / 2 for k in range(3)]
            self.boxes.append(Box(self.world(c), h, yaw, self.id, part.get("part", "body")))
        lo = [min(pt["min"][k] for pt in model["parts"]) for k in range(3)]
        hi = [max(pt["max"][k] for pt in model["parts"]) for k in range(3)]
        self.local = (lo, hi)
        c = [(lo[k] + hi[k]) / 2 for k in range(3)]
        self.whole = Box(self.world(c), [s[k] * (hi[k] - lo[k]) / 2 for k in range(3)], yaw, self.id, "whole")
        self.joints = {k: self.world(v) for k, v in (model.get("joints") or {}).items()}

    def world(self, v):
        t = math.radians(self.yaw)
        s, p = self.scale, self.pos
        x, y, z = (v[k] * s[k] for k in range(3))
        return [p[0] + x * math.cos(t) + z * math.sin(t), p[1] + y,
                p[2] - x * math.sin(t) + z * math.cos(t)]

    @property
    def lo_y(self):
        return self.whole.lo_y

    @property
    def hi_y(self):
        return self.whole.hi_y

    @property
    def height(self):
        return self.hi_y - self.lo_y

    def extent(self):
        return [2 * x for x in self.whole.h]

    def centre(self):
        return self.whole.c

    def facing(self):
        t = math.radians(self.yaw)
        return (math.sin(t), math.cos(t))  # glTF models face +z

    def corners(self):
        (lo, hi) = self.local
        return [self.world([hi[k] if (i >> k) & 1 else lo[k] for k in range(3)]) for i in range(8)]


def tol_for(p):
    return 0.02 + 0.02 * p.height


def rests_on(a, b):
    """The part of b that a rests on (its bottom on the part's top, its
    footprint centre over it), else None."""
    c = a.centre()
    for box in b.boxes:
        if abs(a.lo_y - box.hi_y) <= tol_for(a) and box.contains_xz((c[0], c[2]), grow=0.02):
            return box
    return None


# ------------------------------------------------------------------ camera
class Camera:
    def __init__(self, cam, aspect=16 / 9):
        self.pos, self.target = cam["position"], cam["target"]
        f = [self.target[k] - self.pos[k] for k in range(3)]
        n = math.sqrt(sum(x * x for x in f))
        self.fwd = [x / n for x in f]
        r = [-self.fwd[2], 0.0, self.fwd[0]]
        rn = math.hypot(r[0], r[2])
        self.right = [r[0] / rn, 0.0, r[2] / rn]
        self.up = [self.right[1] * self.fwd[2] - self.right[2] * self.fwd[1],
                   self.right[2] * self.fwd[0] - self.right[0] * self.fwd[2],
                   self.right[0] * self.fwd[1] - self.right[1] * self.fwd[0]]
        self.tan = math.tan(math.radians(cam.get("fov", 50)) / 2)
        self.aspect = aspect
        # On the ground: toward the camera is "front".
        tc = [-self.fwd[0], -self.fwd[2]]
        tn = math.hypot(*tc)
        self.toward = (tc[0] / tn, tc[1] / tn)

    def project(self, p):
        q = [p[k] - self.pos[k] for k in range(3)]
        d = sum(q[k] * self.fwd[k] for k in range(3))
        if d <= 1e-6:
            return None
        x = sum(q[k] * self.right[k] for k in range(3)) / (d * self.tan * self.aspect)
        y = sum(q[k] * self.up[k] for k in range(3)) / (d * self.tan)
        return (x, y)  # -1..1 is the frame

    def side(self, a, b):
        """(right, front) offset of point a from point b, metres, as the camera sees it."""
        dx, dz = a[0] - b[0], a[2] - b[2]
        return (dx * self.right[0] + dz * self.right[2], dx * self.toward[0] + dz * self.toward[1])


# ---------------------------------------------------------------- relations
REL_WORDS = {"in": "in", "inside": "in", "into": "in", "within": "in",
             "under": "under", "underneath": "under", "beneath": "under", "below": "under",
             "on": "on", "onto": "on", "atop": "on", "upon": "on", "top": "on",
             "beside": "beside", "next": "beside", "by": "beside", "near": "beside", "alongside": "beside",
             "against": "beside", "hand": "held", "hands": "held", "holding": "held", "held": "held"}


def relation_of(where):
    """The relation a plan's free-text `where` names: its first relation word
    ("in front", "in the middle" are no "in"); a socket name as such; else on."""
    w = where.strip().lower()
    if w in ("right_hand", "left_hand"):
        return "held"
    if w in ("neck", "back", "hip_left", "hip_right"):
        return "worn"
    w = re.sub(r"\bin the (middle|centre|center) of the (\w+ )?(top|tabletop|deck|surface|seat|shelf|floor|table|stage|desk|pier|roof)", "on", w)
    if re.search(r"\bon the (floor|ground)\b", w):
        return "under" if re.search(r"\b(legs|under|beneath|below)\b", w) else "beside"
    w = re.sub(r"\bin (front|the (middle|centre|center|corner|back))", " ", w)
    for t in re.sub(r"[^a-z]+", " ", w).split():
        if t in REL_WORDS:
            return REL_WORDS[t]
    return "on"


SUBPLACE = ("left", "right", "front", "back")


def check_relation(rel, a, b, cam, extra=None):
    """(ok, why) for 'a REL b'."""
    tol = tol_for(a)
    ca, cb = a.centre(), b.centre()
    if rel == "on":
        part = rests_on(a, b)
        if part:
            return True, f"rests on {b.id}'s {part.part}"
        return False, f"not on {b.id}: bottom {a.lo_y:.2f} m, {b.id} spans {b.lo_y:.2f}-{b.hi_y:.2f} m, centre {'inside' if b.whole.contains_xz((ca[0], ca[2])) else 'outside'} its footprint"
    if rel == "in":
        # Within its walls, or resting low in it and sticking out (a heap of apples).
        inside = (b.whole.contains_xz((ca[0], ca[2])) and b.lo_y - tol <= a.lo_y < b.hi_y
                  and (a.hi_y <= b.hi_y + tol or a.lo_y <= b.lo_y + 0.25 * b.height))
        return inside, ("inside " + b.id) if inside else f"not inside {b.id}: bottom {a.lo_y:.2f}, centre y {ca[1]:.2f}, {b.id} y {b.lo_y:.2f}-{b.hi_y:.2f}, xz {'inside' if b.whole.contains_xz((ca[0], ca[2])) else 'outside'}"
    if rel == "under":
        hit = max((overlap_depth(x, y) for x in a.boxes for y in b.boxes), default=0)
        ok = b.whole.contains_xz((ca[0], ca[2])) and a.hi_y <= b.hi_y and hit <= tol
        return ok, ("under " + b.id) if ok else f"not under {b.id}: top {a.hi_y:.2f} vs {b.id} top {b.hi_y:.2f}, footprint {'inside' if b.whole.contains_xz((ca[0], ca[2])) else 'outside'}, intersect {hit:.2f} m"
    if rel in ("held", "worn"):
        if a.lo_y >= b.hi_y - 0.1 * b.height:
            return False, f"on {b.id}'s head (bottom {a.lo_y:.2f} m, {b.id} top {b.hi_y:.2f} m)"
        hands = {k: v for k, v in b.joints.items() if k.endswith("Hand")}
        want = (extra or "").lower()
        if hands:
            def dist(p):
                return max(0.0, max(abs(p[k] - a.centre()[k]) - (a.whole.h[k] if k != 1 else a.whole.h[1]) for k in range(3)))
            near = {k: dist(v) for k, v in hands.items()}
            k = min(near, key=near.get)
            if near[k] <= 0.15 * b.height:
                side_ok = not want or ("right" in want) == (k == "RightHand") or not ("right" in want or "left" in want)
                return True, f"at {b.id}'s {k} ({near[k]:.2f} m)" + ("" if side_ok else f", but the plan says {want}")
        g = gap2d(a.whole, b.whole)
        if g <= 0.3 and a.lo_y < b.lo_y + 0.6 * b.height:
            return True, f"right beside {b.id} ({g:.2f} m), not in its hand"
        return False, f"not at {b.id}'s hand nor right beside it (gap {g:.2f} m, bottom {a.lo_y:.2f} m)"
    if rel == "beside":
        hit = max((overlap_depth(x, y) for x in a.boxes for y in b.boxes), default=0)
        g = gap2d(a.whole, b.whole)
        reach = max(0.6, 0.6 * max(max(a.extent()[0], a.extent()[2]), max(b.extent()[0], b.extent()[2])))
        same = abs(a.lo_y - b.lo_y) <= 0.1 + 0.1 * max(a.height, b.height)
        apart = not (a.whole.contains_xz((cb[0], cb[2])) or b.whole.contains_xz((ca[0], ca[2])))
        ok = hit <= tol and g <= reach and same and apart
        return ok, f"{'beside' if ok else 'not beside'} {b.id}: gap {g:.2f} m (≤ {reach:.2f}), floors {a.lo_y:.2f}/{b.lo_y:.2f}, intersect {hit:.2f}"
    if rel == "sky":
        ok = a.lo_y >= b.hi_y + 1.0
        return ok, f"bottom {a.lo_y:.2f} m vs {b.id} top {b.hi_y:.2f} m"
    if rel in SUBPLACE or rel == "around" or rel == "above":
        if rel == "above":
            ok = b.whole.contains_xz((ca[0], ca[2]), grow=0.1) and a.lo_y >= b.hi_y - tol
            return ok, f"bottom {a.lo_y:.2f} vs {b.id} top {b.hi_y:.2f}"
        r, f = cam.side(ca, cb)
        val = {"left": -r, "right": r, "front": f, "back": -f}.get(rel, 0)
        ok = val > 0.05
        return ok, f"{rel} of {b.id} by {val:+.2f} m (camera right {r:+.2f}, front {f:+.2f})"
    return None, "unknown relation " + rel


# ------------------------------------------------------------------- checks
def match_items(items, kw, kinds=("character", "object")):
    """Items whose name names kw (else whose description does)."""
    kws = kw if isinstance(kw, list) else [kw]
    pool = [i for i in items if i["kind"] in kinds]
    if "@character" in kws:  # people named by their own names
        return [i for i in pool if i["kind"] == "character"]

    def head(i):
        last = [x for x in re.sub(r"[^a-z]+", " ", i["name"].lower().split(" of ")[0]).split()][-1:]
        return set(words(" ".join(last)))
    for tier in (head, lambda i: set(words(i["name"])) | {i["id"]},
                 lambda i: set(words(i.get("description", "")))):
        hit = [i for i in pool if any(k in tier(i) for k in kws)]
        if hit:
            return hit
    return []


def jaccard(a, b):
    a, b = set(words(a)), set(words(b))
    return len(a & b) / max(1, len(a | b))


def run_checks(plan, doc, models, expect, vis=None, notes_extra=()):
    items = plan["items"]
    by_id = {i["id"]: i for i in items}
    objs = {o["id"]: o for o in doc.get("objects", []) if o.get("type") == "gltf"}
    pieces = {k: Piece(o, models[k]) for k, o in objs.items() if k in models}
    cam = Camera(doc["camera"])
    R = {}

    def put(name, ok, details):
        R[name] = {"pass": ok, "details": details}

    # The relations the plan asks for: layout attach_to / details.attach_to, holds.
    asked = []  # (a, rel, b, source, sub)
    for p in plan.get("layout", []):
        a = p["item"]
        at = p.get("attach_to") or (by_id.get(a, {}).get("details") or {}).get("attach_to")
        if at:
            where = at.get("where", "")
            asked.append((a, relation_of(where), at["item"], f"attach_to where={where!r}", where))
    for it in items:
        for h in (it.get("details") or {}).get("holds", []):
            if not any(x[0] == h["item"] and x[2] == it["id"] and x[1] == "held" for x in asked):
                asked.append((h["item"], "held", it["id"], f"holds socket={h['socket']}", h["socket"]))

    # 1. intersections (not 'in' pairs, not a held prop against its holder).
    exempt = {(a, b) for a, rel, b, _, _ in asked if rel in ("in", "held", "worn")}
    hits = []
    ids = sorted(pieces)
    for i, x in enumerate(ids):
        for y in ids[i + 1:]:
            if (x, y) in exempt or (y, x) in exempt:
                continue
            A, B = pieces[x], pieces[y]
            worst = max(((overlap_depth(p, q), p, q) for p in A.boxes for q in B.boxes), key=lambda t: t[0], default=(0, None, None))
            d, p, q = worst
            # Deep, or through most of the thinner piece (two plates in one spot).
            thin = min(2 * min(p.h), 2 * min(q.h)) if p else 1
            if d > 0.01 + 0.01 * min(A.height, B.height) or (d > 0.002 and d >= 0.5 * thin):
                hits.append(f"{x} ∩ {y}: {d:.2f} m deep")
    put("no_intersections", not hits, hits)

    # 2. the plan's relations, really there.
    rel_out = []
    rel_ok = True
    for a, rel, b, src, sub in asked:
        if a not in pieces or b not in pieces:
            rel_out.append(f"{a} {rel} {b}: not both in the scene ({src})")
            rel_ok = False
            continue
        ok, why = check_relation(rel, pieces[a], pieces[b], cam, sub)
        rel_out.append(f"{'ok' if ok else 'FAIL'} {a} {rel} {b} ({src}): {why}")
        rel_ok &= bool(ok)
        # Its place on the target ("back-right" of the deck), as the camera sees it.
        place = next((p.get("place", "") for p in plan.get("layout", []) if p["item"] == a), "")
        own = next((p.get("place", "") for p in plan.get("layout", []) if p["item"] == b), "")
        # (A place that only repeats the target's own slot says nothing about a spot on it.)
        for w in (set(re.split(r"[-\s_+]", place)) & set(SUBPLACE)) if rel in ("on", "in") and place != own else ():
            ok2, why2 = check_relation(w, pieces[a], pieces[b], cam)
            rel_out.append(f"{'ok' if ok2 else 'FAIL'} {a} place={place} on {b}: {why2}")
            rel_ok &= bool(ok2)
    put("plan_relations", rel_ok, rel_out)

    # 3. left/right/front/back of free pieces relative to the group's middle.
    attached = {a for a, _, _, _, _ in asked}
    free = [k for k in pieces if k not in attached]
    out, ok_all = [], True
    if len(free) > 1:
        mid = [sum(pieces[k].centre()[d] for k in free) / len(free) for d in range(3)]
        for p in plan.get("layout", []):
            if p["item"] not in free:
                continue
            for w in set(re.split(r"[-\s_+]", p.get("place", ""))) & set(SUBPLACE):
                ok, why = check_relation(w, pieces[p["item"]], type("M", (), {"centre": lambda s=None: mid, "id": "the group's middle"})(), cam)
                out.append(f"{'ok' if ok else 'FAIL'} {p['item']} place={p['place']}: {why}")
                ok_all &= ok
    put("plan_places", ok_all if out else None, out)

    # 4. sizes.
    out, ok_all = [], True
    for k, P in pieces.items():
        it = by_id.get(k)
        want = (it.get("details") or {}).get("size_m") if it else None
        if not want:
            continue
        lo, hi = P.local
        ext = [P.scale[d] * (hi[d] - lo[d]) for d in range(3)]
        got = ext[1] if it["kind"] == "character" else max(ext)
        ok = abs(got - want) <= 0.1 * want + 0.01
        ok_all &= ok
        out.append(f"{'ok' if ok else 'FAIL'} {k}: {got:.2f} m for size_m {want}")
    for big, small in expect.get("bigger", []):
        for bi in match_items(items, big):
            for si in match_items(items, small):
                if bi["id"] in pieces and si["id"] in pieces:
                    B, S = pieces[bi["id"]], pieces[si["id"]]
                    vb, vs = math.prod(B.extent()), math.prod(S.extent())
                    ok = vb > vs and B.height > S.height
                    ok_all &= ok
                    out.append(f"{'ok' if ok else 'FAIL'} {bi['id']} bigger than {si['id']}: heights {B.height:.2f} / {S.height:.2f} m")
    put("sizes", ok_all if out else None, out)

    # 5. supported: on the ground, on something, in something or held.
    out = []
    held_or_in = {a for a, rel, _, _, _ in asked if rel in ("in", "held", "worn")}
    for k, P in pieces.items():
        if k in held_or_in or abs(P.lo_y) <= tol_for(P):
            continue
        if any(rests_on(P, Q) for j, Q in pieces.items() if j != k):
            continue
        out.append(f"{k} floats: bottom at {P.lo_y:.2f} m with nothing under it")
    put("supported", not out, out)

    # 6. ground and framing.
    out = []
    ground = next((o for o in doc.get("objects", []) if o.get("id") == "ground"), None)
    allc = [c for P in pieces.values() for c in P.corners()]
    ok_ground = None
    if allc:
        ex = max(max(c[0] for c in allc) - min(c[0] for c in allc), max(c[2] for c in allc) - min(c[2] for c in allc))
        if ground:
            side = max(ground["scale"][0], ground["scale"][1])
            ok_ground = side <= max(2.5 * ex, ex + 3.0)
            out.append(f"ground {side:.1f} m across for a group {ex:.1f} m across")
    put("ground_size", ok_ground, out)
    out, ok_frame = [], None
    if allc:
        pts = [cam.project(c) for c in allc]
        if any(p is None for p in pts):
            ok_frame, out = False, ["a piece is behind the camera"]
        else:
            xs, ys = [p[0] for p in pts], [p[1] for p in pts]
            fill = max((min(1, max(xs)) - max(-1, min(xs))) / 2, (min(1, max(ys)) - max(-1, min(ys))) / 2)
            outside = [k for k, P in pieces.items() if all(abs(q[0]) > 1 or abs(q[1]) > 1 for q in (cam.project(c) for c in P.corners()))]
            ok_frame = fill >= 0.45 and not outside
            out.append(f"the group fills {fill:.0%} of the frame (16:9, across or up)")
            out += [f"{k} is out of the frame" for k in outside]
    put("framing", ok_frame, out)
    out, ok_vis = [], None
    if vis:
        ok_vis = True
        for k in pieces:
            v = vis.get(k, {})
            seen, alone = v.get("visible", 0), v.get("alone", 0)
            ok = seen >= 20 and (alone == 0 or seen >= 0.1 * alone)
            ok_vis &= ok
            out.append(f"{'ok' if ok else 'FAIL'} {k}: {seen} px seen of {alone} px when alone")
    put("visible", ok_vis, out)

    # 7. facing.
    out, ok_all = [], True
    for p in plan.get("layout", []):
        f = p.get("facing")
        if not f or p["item"] not in pieces or f not in pieces:
            continue
        A, B = pieces[p["item"]], pieces[f]
        fx, fz = A.facing()
        dx, dz = B.centre()[0] - A.centre()[0], B.centre()[2] - A.centre()[2]
        n = math.hypot(dx, dz)
        ang = 0.0 if n < 1e-6 else math.degrees(math.acos(max(-1, min(1, (fx * dx + fz * dz) / n))))
        ok = ang <= 70
        ok_all &= ok
        out.append(f"{'ok' if ok else 'FAIL'} {p['item']} faces {f}: {ang:.0f}° off")
    for a, b in expect.get("facing", []):
        for ai in match_items(items, a):
            for bi in match_items(items, b):
                if ai["id"] in pieces and bi["id"] in pieces and not any(p.get("facing") == bi["id"] and p["item"] == ai["id"] for p in plan.get("layout", [])):
                    A, B = pieces[ai["id"]], pieces[bi["id"]]
                    fx, fz = A.facing()
                    dx, dz = B.centre()[0] - A.centre()[0], B.centre()[2] - A.centre()[2]
                    n = math.hypot(dx, dz) or 1
                    ang = math.degrees(math.acos(max(-1, min(1, (fx * dx + fz * dz) / n))))
                    ok = ang <= 70
                    ok_all &= ok
                    out.append(f"{'ok' if ok else 'FAIL'} {ai['id']} faces {bi['id']} (asked by the description, not in the plan): {ang:.0f}° off")
    put("facing", ok_all if out else None, out)

    # 8. the builder's notes are true: no fallback "because I can't find".
    notes = list(plan.get("notes", [])) + list(notes_extra)
    bad = [n for n in notes if re.search(r"because I can.t find", n)]
    put("notes_true", not bad, bad)

    # 9. what the description asked for (corpus expect).
    out, ok_all = [], True
    note_text = " ".join(notes).lower()
    for group in expect.get("things", []):
        kinds = ("character", "object", "voice_line", "sound", "music")
        hit = match_items(items, group, kinds)
        if hit:
            out.append(f"ok {group[0]}: {', '.join(i['id'] for i in hit)}")
        elif any(k in note_text for k in group):
            out.append(f"ok {group[0]}: not an item, a note says why")
        else:
            out.append(f"FAIL {group[0]}: no item and no note")
            ok_all = False
    for want in expect.get("count", []):
        hit = match_items(items, want["of"])
        if len(hit) != want["n"]:
            ok_all = False
            out.append(f"FAIL {want['n']} × {want['of'][0]} asked, {len(hit)} in the plan")
    n3d = len([i for i in items if i["kind"] in ("character", "object")])
    if "max_3d" in expect and n3d > expect["max_3d"]:
        ok_all = False
        out.append(f"FAIL {n3d} 3D pieces for a description that names {expect['max_3d']}")
    for kind in expect.get("absent", []):
        extra_k = [i["id"] for i in items if i["kind"] == kind]
        if extra_k:
            ok_all = False
            out.append(f"FAIL the description asks for no {kind}, the plan has {', '.join(extra_k)}")
    for w in expect.get("absent_words", []):
        bad = [i["id"] for i in items if i["kind"] in ("sound", "music") and w in json.dumps(i).lower()]
        if bad:
            ok_all = False
            out.append(f"FAIL {w!r} cannot be there, but the plan has {', '.join(bad)}")
    for w in expect.get("note_mentions", []):
        if w not in note_text:
            ok_all = False
            out.append(f"FAIL no note says what was done about {w!r}")
    known = [k for g in expect.get("things", []) for k in g] + expect.get("allow", [])
    extra = [i["id"] for i in items if i["kind"] in ("character", "object")
             and not any(k in words(i["name"] + " " + i["id"]) for k in known)]
    if extra:
        out.append("WARN not named in the description (judge by hand): " + ", ".join(extra))
    put("plan_has_what_was_asked", ok_all, out)

    out, ok_all = [], True
    lines = [i for i in items if i["kind"] == "voice_line"]
    for want in expect.get("lines", []):
        best = max(lines, key=lambda l: jaccard(l["details"].get("text", ""), want["text"]), default=None)
        if not best or jaccard(best["details"].get("text", ""), want["text"]) < 0.5:
            ok_all = False
            out.append(f"FAIL no line for {want['text']!r}")
            continue
        sp = by_id.get(best["details"].get("speaker", ""), {})
        ok = any(k in words(sp.get("name", "") + " " + sp.get("id", "") + " " + sp.get("description", "")) for k in want["speaker"])
        # "@character": the description names no kind of speaker (a child by her own name).
        ok = ok or ("@character" in want["speaker"] and sp.get("kind") == "character")
        ok_all &= ok
        out.append(f"{'ok' if ok else 'FAIL'} {want['text']!r} → {best['id']} spoken by {sp.get('id', '?')}")
    spoken = [i for i in items if i["kind"] == "voice_line" and not (i["details"] or {}).get("nonverbal")]
    if len(spoken) > len(expect.get("lines", [])) and expect.get("lines") is not None and "lines" in expect:
        out.append(f"WARN {len(spoken)} spoken lines for {len(expect['lines'])} quoted sentences")
    put("plan_voices", ok_all if expect.get("lines") else None, out)

    # 10. the relations the description asks for (corpus), whether or not the plan says them.
    out, ok_all = [], True
    def group_of(kw):
        kws = kw if isinstance(kw, list) else [kw]
        return next((g for g in expect.get("things", []) if set(kws) & set(g)), kws)
    for a_kw, rel, b_kw in expect.get("relations", []):
        if b_kw is None:  # about a alone: "floating" off the floor
            for ai in match_items(items, group_of(a_kw)):
                if ai["id"] in pieces:
                    P = pieces[ai["id"]]
                    ok = P.lo_y >= 0.15
                    ok_all &= ok
                    out.append(f"{'ok' if ok else 'FAIL'} {ai['id']} {rel}: bottom {P.lo_y:.2f} m")
            continue
        A, B = match_items(items, group_of(a_kw)), match_items(items, group_of(b_kw))
        if not A or not B:
            out.append(f"FAIL {a_kw} {rel} {b_kw}: {'no ' + a_kw if not A else 'no ' + b_kw} in the plan")
            ok_all = False
            continue
        if {i["id"] for i in A} == {i["id"] for i in B}:
            out.append(f"FAIL {a_kw} {rel} {b_kw}: one item ({', '.join(i['id'] for i in A)}), so it cannot be placed")
            ok_all = False
            continue
        for ai in A:
            if any(bi["id"] == ai["id"] for bi in B):
                continue
            res = []
            for bi in B:
                if ai["id"] in pieces and bi["id"] in pieces:
                    res.append(check_relation(rel, pieces[ai["id"]], pieces[bi["id"]], cam) + (bi["id"],))
            if not res:
                out.append(f"FAIL {ai['id']} {rel} {b_kw}: not in the scene")
                ok_all = False
                continue
            good = [r for r in res if r[0]]
            pick = good[0] if good else res[0]
            ok_all &= bool(good)
            out.append(f"{'ok' if good else 'FAIL'} {ai['id']} {rel} {pick[2]}: {pick[1]}")
    # 11. the setting and details the description states or clearly implies
    # (written down before any run): in the scene ("scene": an object of the
    # scene or the document's own setting/environment fields, never a model's
    # prompt; "lights": a point or spot light), in the plan's items or setting
    # ("plan"), or a sound ("sound": a sound or voice item, or the scene).
    if expect.get("details"):
        put("setting_and_details", *details_check(plan, doc, expect))
    if expect.get("row_ascending"):
        row = [i for i in match_items(items, expect["row_ascending"]) if i["id"] in pieces]
        row.sort(key=lambda i: cam.side(pieces[i["id"]].centre(), [0, 0, 0])[0])
        hs = [round(pieces[i["id"]].height, 2) for i in row]
        ok = len(hs) > 1 and all(a < b for a, b in zip(hs, hs[1:]))
        ok_all &= ok
        out.append(f"{'ok' if ok else 'FAIL'} left to right as the camera sees them: " + ", ".join(f"{i['id']} {h} m" for i, h in zip(row, hs)))
    put("described_relations", ok_all if (expect.get("relations") or expect.get("row_ascending")) else None, out)
    return R


def has(text, ws):
    return any(re.search(r"\b" + re.escape(w), text) for w in ws)


def details_check(plan, doc, expect):
    def strip(v):
        if isinstance(v, dict):
            return {k: strip(x) for k, x in v.items() if k not in ("src", "camera", "position", "rotation", "scale")}
        if isinstance(v, list):
            return [strip(x) for x in v]
        return v
    scene_text = json.dumps(strip({k: v for k, v in doc.items() if k != "lights"}), ensure_ascii=False).lower()
    lights = [l for l in doc.get("lights", []) + [o for o in doc.get("objects", []) if o.get("type") in ("light", "pointLight", "spotLight")]]
    lights_text = " ".join(str(l.get("type", "")) for l in lights).lower()
    plan_text = json.dumps({k: v for k, v in plan.items() if k != "notes"}, ensure_ascii=False).lower()
    sound_text = json.dumps([i for i in plan["items"] if i["kind"] in ("sound", "voice_line", "music")], ensure_ascii=False).lower()
    notes = " ".join(plan.get("notes", [])).lower()
    found, missing = [], []
    for d in expect["details"]:
        ws = [w.lower() for w in d["words"]]
        where = d["where"]
        ok = {"scene": has(scene_text, ws), "lights": has(lights_text, ws), "plan": has(plan_text, ws),
              "sound": has(sound_text, ws) or has(scene_text, ws)}.get(where, has(plan_text, ws))
        if ok:
            found.append(d["what"])
        else:
            missing.append(f"missing: {d['what']} ({'named in a note' if has(notes, ws) else 'not even a note'}; looked in the {where})")
    head = f"found {len(found)}/{len(expect['details'])} (setting expected: {expect.get('setting')})"
    return not missing, [head] + missing + ["found: " + ", ".join(found)] if found else [head] + missing


def main(scene_dir):
    d = Path(scene_dir)
    plan = json.loads((d / "plan.json").read_text())
    doc = json.loads((d / "scene.json").read_text())
    models = {p.name[:-len(".parts.json")]: json.loads(p.read_text()) for p in (d / "parts").glob("*.parts.json")}
    expect = json.loads((d / "expect.json").read_text()) if (d / "expect.json").exists() else {}
    vis = json.loads((d / "vis.json").read_text()) if (d / "vis.json").exists() else None
    res = run_checks(plan, doc, models, expect, vis)
    (d / "checks.json").write_text(json.dumps(res, indent=1, ensure_ascii=False))
    return res


if __name__ == "__main__":
    r = main(sys.argv[1])
    for k, v in r.items():
        print(("PASS" if v["pass"] else "----" if v["pass"] is None else "FAIL"), k, *v["details"][:6], sep="\n  " if v["details"] else " ")
