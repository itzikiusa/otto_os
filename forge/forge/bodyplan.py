"""Body plan of any rigged GLB: legs, spine, head, tail and other limbs, found by shape.

SkinTokens fits a skeleton to any mesh and names its joints bone_0..N. A humanoid keeps today's path:
AssetFurnace's mapper (the one the preset moves use) decides what a humanoid is, and it gets Unity
Humanoid names. Every other body gets a body plan computed from the skeleton and its skinned mesh
(glTF is Y-up, so the floor is the lowest skinned vertex and the height is the skin's vertical extent):

* LEGS are branches whose skin reaches the floor (within 6% of the height). From each such foot the limb
  climbs towards the body while it contains no other foot and not the top of the body. A leg hangs from
  a joint at least 12% of the height above the floor, drops at least half its horizontal reach,
  and its lower half comes down steeply (at least 1 in 2) onto the foot, so a snake's head and tail, or a
  tail lying on the floor, are not legs.
* The SPINE is the longest chain of the other joints that lie on the body's mirror plane (wings, arms and
  ears come in mirrored pairs). On a long body the head is the end the paws point to (else the higher
  end); on an upright body it is the higher end. The other end is the TAIL.
* Everything else is an ARM (a side branch of the body that does not reach the floor), an EAR (a branch
  of the head or neck) or an EXTRA.

`detect` returns the plan; `name_bones` writes readable names (leg_front_left_1, spine_1, tail_1, ...)
over the skin's joints and caps each vertex at four weights, like the humanoid naming step.
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
AF = ROOT / "engines" / "assetfurnace"
UP = np.array([0.0, 1.0, 0.0])
GROUND = 0.06        # share of the height: skin this close to the floor touches it (generated feet hover a little)
LEG_ATTACH = 0.12    # a leg hangs from a joint at least this share of the height above the floor
LEG_DROP = 0.5       # ... and drops at least this share of its horizontal reach
FOOT_SPAN = 0.10     # leaves of one foot (toes) lie within this share of the height of each other
FOOT_LOW = 0.15      # joints this share of the hip height above the floor are the (flat) foot
SYMMETRIC = 0.06     # joints mirror within this share of the height: the body has a mirror plane
COMP = {5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
WIDTH = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


def accessor(doc, binary, index):
    """One accessor as a float64 (count, width) array (normalised integers scaled to 0..1)."""
    a = doc["accessors"][index]
    dtype = np.dtype(COMP[a["componentType"]]).newbyteorder("<")
    width = WIDTH[a["type"]]
    if "bufferView" not in a:
        return np.zeros((a["count"], width))
    view = doc["bufferViews"][a["bufferView"]]
    stride = view.get("byteStride") or dtype.itemsize * width
    raw = np.ndarray((a["count"], width), dtype=dtype, buffer=binary,
                     offset=view.get("byteOffset", 0) + a.get("byteOffset", 0), strides=(stride, dtype.itemsize))
    out = raw.astype(np.float64)
    if a.get("normalized") and dtype.kind in "iu":
        out /= np.iinfo(dtype).max
    return out


def quat_matrix(q):
    x, y, z, w = q
    return np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                     [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                     [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])


def matrix_quat(m):
    """Rotation matrix -> unit quaternion (x, y, z, w)."""
    t = np.trace(m)
    if t > 0:
        s = 2 * np.sqrt(t + 1)
        q = [(m[2, 1] - m[1, 2]) / s, (m[0, 2] - m[2, 0]) / s, (m[1, 0] - m[0, 1]) / s, s / 4]
    else:
        i = int(np.argmax(np.diag(m)))
        j, k = (i + 1) % 3, (i + 2) % 3
        s = 2 * np.sqrt(1 + m[i, i] - m[j, j] - m[k, k])
        q = [0.0] * 4
        q[i] = s / 4
        q[j] = (m[j, i] + m[i, j]) / s
        q[k] = (m[k, i] + m[i, k]) / s
        q[3] = (m[k, j] - m[j, k]) / s
    q = np.array(q)
    return q / np.linalg.norm(q)


def trs(t, r, s):
    m = np.eye(4)
    m[:3, :3] = quat_matrix(r) * np.asarray(s)
    m[:3, 3] = t
    return m


def node_trs(node):
    if "matrix" in node:
        m = np.array(node["matrix"], dtype=float).reshape(4, 4).T
        s = np.linalg.norm(m[:3, :3], axis=0)
        return m[:3, 3].copy(), matrix_quat(m[:3, :3] / s), s
    return (np.array(node.get("translation", [0, 0, 0]), float), np.array(node.get("rotation", [0, 0, 0, 1]), float),
            np.array(node.get("scale", [1, 1, 1]), float))


def axis_angle(axis, angle):
    """Rotation matrix about a unit axis (radians)."""
    x, y, z = np.asarray(axis, float) / np.linalg.norm(axis)
    c, s, C = np.cos(angle), np.sin(angle), 1 - np.cos(angle)
    return np.array([[c + x * x * C, x * y * C - z * s, x * z * C + y * s],
                     [y * x * C + z * s, c + y * y * C, y * z * C - x * s],
                     [z * x * C - y * s, z * y * C + x * s, c + z * z * C]])


def rotation_between(a, b):
    """Smallest rotation taking direction a to direction b."""
    a, b = a / np.linalg.norm(a), b / np.linalg.norm(b)
    v, c = np.cross(a, b), float(a @ b)
    if np.linalg.norm(v) < 1e-9:
        if c > 0:
            return np.eye(3)
        other = np.cross(a, [1.0, 0, 0] if abs(a[0]) < 0.9 else [0, 1.0, 0])
        return axis_angle(other, np.pi)
    return axis_angle(v, np.arctan2(np.linalg.norm(v), c))


class Rig:
    """The first skin of a GLB: joints, rest transforms and the skinned mesh at rest, in world space."""

    def __init__(self, path):
        from forge import glbstats
        self.path = Path(path)
        self.doc, self.binary = glbstats.read_glb(self.path)
        doc = self.doc
        skins = doc.get("skins") or []
        if not skins or not skins[0].get("joints"):
            raise LookupError(f"{self.path.name} has no skeleton")
        skin = skins[0]
        nodes = doc["nodes"]
        self.nodes = list(skin["joints"])
        index = {n: k for k, n in enumerate(self.nodes)}
        self.names = [nodes[n].get("name") or f"joint{k}" for k, n in enumerate(self.nodes)]
        parent_node = {c: i for i, n in enumerate(nodes) for c in n.get("children", [])}
        world = {}

        def world_of(i):
            if i not in world:
                p = parent_node.get(i)
                t, r, s = node_trs(nodes[i])
                world[i] = (world_of(p) if p is not None else np.eye(4)) @ trs(t, r, s)
            return world[i]

        n = len(self.nodes)
        self.parent = []
        self.offset = []  # parent joint's rest world (or identity) -> this joint's parent node, at rest
        for k, node in enumerate(self.nodes):
            p = parent_node.get(node)
            direct = world_of(p) if p is not None else np.eye(4)
            while p is not None and p not in index:
                p = parent_node.get(p)
            self.parent.append(index[p] if p is not None else -1)
            base = world_of(p) if p is not None else np.eye(4)
            self.offset.append(np.linalg.inv(base) @ direct)
        self.rest_local = [node_trs(nodes[j]) for j in self.nodes]
        self.rest_world = np.array([world_of(j) for j in self.nodes])
        self.heads = self.rest_world[:, :3, 3].copy()
        self.children = [[c for c in range(n) if self.parent[c] == k] for k in range(n)]
        self.order = []  # parents first
        stack = [k for k in range(n) if self.parent[k] < 0]
        while stack:
            k = stack.pop()
            self.order.append(k)
            stack += self.children[k]
        ibm = (accessor(doc, self.binary, skin["inverseBindMatrices"]).reshape(-1, 4, 4).transpose(0, 2, 1)
               if "inverseBindMatrices" in skin else np.tile(np.eye(4), (n, 1, 1)))
        self.ibm = ibm
        verts, joints, weights = [], [], []
        for node in nodes:
            if node.get("skin") != 0 or "mesh" not in node:
                continue
            for prim in doc["meshes"][node["mesh"]]["primitives"]:
                at = prim.get("attributes", {})
                if "JOINTS_0" not in at or "WEIGHTS_0" not in at:
                    continue
                verts.append(accessor(doc, self.binary, at["POSITION"]))
                joints.append(accessor(doc, self.binary, at["JOINTS_0"]).astype(int))
                weights.append(accessor(doc, self.binary, at["WEIGHTS_0"]))
        if not verts:
            raise LookupError(f"{self.path.name} has a skeleton but no skinned mesh")
        self.bind = np.concatenate(verts)
        self.vjoints = np.concatenate(joints)
        w = np.concatenate(weights)
        self.vweights = w / np.maximum(w.sum(axis=1, keepdims=True), 1e-12)
        self.skin_rest = self.skin(self.rest_world)
        self.dominant = self.vjoints[np.arange(len(w)), self.vweights.argmax(axis=1)]

    def skin(self, world, ids=None):
        """Skinned vertex positions (optionally only `ids`) for joint world matrices `world` (n, 4, 4)."""
        ids = slice(None) if ids is None else ids
        m = world @ self.ibm
        v = np.c_[self.bind[ids], np.ones(len(self.bind[ids]))]
        j, w = self.vjoints[ids], self.vweights[ids]
        out = np.zeros((len(v), 3))
        for k in range(4):
            out += w[:, k:k + 1] * np.einsum("nij,nj->ni", m[j[:, k]], v)[:, :3]
        return out

    def subtree(self, k):
        out, stack = [], [k]
        while stack:
            b = stack.pop()
            out.append(b)
            stack += self.children[b]
        return out

    def path_up(self, k):
        out = [k]
        while self.parent[out[-1]] >= 0:
            out.append(self.parent[out[-1]])
        return out


def humanoid(rig: Rig) -> bool:
    """True for an upright body with arms held out that AssetFurnace's humanoid mapper fits (the one Unity
    names and the preset moves use). A bird or a dinosaur can fit the mapper too (wings or arms off a chest,
    two legs), but its head is well forward of its hips, or its wings lie folded along its body: human moves
    would stand it up like a person or fling its wings out."""
    if rig.doc["skins"][0].get("extras", {}).get("swimming_fish") is True:
        return False
    if str(AF / "scripts") not in sys.path:
        sys.path.insert(0, str(AF / "scripts"))
    from kimodo_retarget import map_skintokens_to_soma
    parents = {rig.names[k]: (rig.names[p] if p >= 0 else None) for k, p in enumerate(rig.parent)}
    heads = {rig.names[k]: rig.heads[k] for k in range(len(rig.names))}
    try:
        mapping, frame = map_skintokens_to_soma(parents, heads, up=UP)
    except (ValueError, KeyError, IndexError, StopIteration):
        return False
    first = {}
    for bone, soma in mapping.items():
        first.setdefault(soma, bone)
    # a bird's wings can map without an arm or a shoulder: no arms to hold out, so not a human body
    if not all(k in first for k in ("Head", "Hips", "LeftHand", "RightHand")) or not all(
            f"{s}Arm" in first or f"{s}Shoulder" in first for s in ("Left", "Right")):
        return False
    rise = heads[first["Head"]] - heads[first["Hips"]]
    if float(rise @ UP) <= 1.5 * float(np.linalg.norm(_horizontal(rise))):
        return False
    # the presets assume arms held out from the body at rest (T or A pose: 0.36-0.69 of each arm's length
    # runs sideways on Forge's humans); folded wings lie along the body (a round bird: 0.06 and 0.2)
    for side in ("Left", "Right"):
        start, hand = first.get(f"{side}Arm") or first[f"{side}Shoulder"], first[f"{side}Hand"]
        arm = heads[hand] - heads[start]
        if abs(float(arm @ frame[:, 0])) < 0.25 * float(np.linalg.norm(arm)):
            return False
    return True


def _horizontal(v):
    v = np.asarray(v, float)
    return v - (v @ UP) * UP


def _unit(v):
    n = np.linalg.norm(v)
    return v / n if n > 1e-12 else v


def mirror_plane(points, height):
    """(normal, point on it, error share of height) of the vertical plane the joints are most mirror-symmetric
    about: a body's sagittal plane. Arms, wings, ears and legs come in mirrored pairs; the spine lies on it."""
    centre = points.mean(axis=0)
    planes = []
    for th in np.linspace(0, np.pi, 72, endpoint=False):
        normal = np.array([np.cos(th), 0.0, np.sin(th)])
        mirrored = points - 2 * ((points - centre) @ normal)[:, None] * normal
        planes.append((np.linalg.norm(mirrored[:, None] - points[None], axis=2).min(axis=1).mean(), normal))
    best = min(e for e, _ in planes)
    # equally good planes (a three-legged stool): the one facing glTF +Z, where Forge's concepts look
    err, normal = max((p for p in planes if p[0] <= best + 0.005 * height), key=lambda p: abs(p[1][0]))
    return normal, centre, err / height


def fish_forward(skin):
    """Point from a thin caudal fin to the thicker head of a swimming fish."""
    horizontal = skin[:, [0, 2]]
    if len(horizontal) < 8:
        return None
    _, axes = np.linalg.eigh(np.cov(horizontal.T))
    along, across = horizontal @ axes[:, 1], horizontal @ axes[:, 0]
    length = np.ptp(along)
    if length < 1e-6 or length < 1.4 * np.ptp(across):
        return None
    position = (along - along.min()) / length
    thickness = []
    for end in (position, 1. - position):
        widths = []
        for lower in (.08, .16, .24):
            section = across[(end >= lower) & (end < lower + .08)]
            if len(section) >= 4:
                widths.append(np.ptp(np.percentile(section, [5, 95])))
        if len(widths) < 2:
            return None
        thickness.append(np.median(widths))
    if max(thickness) <= 1.35 * min(thickness) + .005 * length:
        return None
    x, z = axes[:, 1] * np.sign(thickness[1] - thickness[0])
    return np.array([x, 0., z])


def detect(rig: Rig) -> dict:
    """The body plan: legs (any number), spine, head, tail, arms, ears, extras, forward and floor."""
    skin, heads, dom = rig.skin_rest, rig.heads, rig.dominant
    n = len(rig.names)
    floor = float(skin[:, 1].min())
    height = float(skin[:, 1].max() - floor) or 1e-6
    low = np.full(n, np.inf)  # lowest skin point of each joint's own vertices (or its head)
    np.minimum.at(low, dom, skin[:, 1])
    low = np.minimum(low, heads[:, 1])
    sub = [set(rig.subtree(k)) for k in range(n)]
    ground = floor + GROUND * height

    def own(leaf):
        return [leaf] + ([rig.parent[leaf]] if rig.parent[leaf] >= 0 else [])

    def tip(leaf):
        """The lowest skin of a foot (its own vertices and its parent's)."""
        mine = np.isin(dom, own(leaf))
        pts = skin[mine] if mine.any() else heads[[leaf]]
        return pts[pts[:, 1] <= pts[:, 1].min() + 0.02 * height].mean(axis=0)

    swimming = rig.doc["skins"][0].get("extras", {}).get("swimming_fish") is True
    feet = [] if swimming else [k for k in range(n) if not rig.children[k] and min(low[j] for j in own(k)) <= ground]
    tips = {f: tip(f) for f in feet}
    limbs = {}
    for f in feet:
        k = f
        # climb towards the body while the limb holds no other foot; a limb never takes in the root
        while rig.parent[k] >= 0 and rig.parent[rig.parent[k]] >= 0:
            p = rig.parent[k]
            if any(np.linalg.norm(_horizontal(tips[g] - tips[f])) > FOOT_SPAN * height for g in feet if g in sub[p]):
                break  # p holds another foot: it is the body
            if any(max(np.linalg.norm(heads[x] - heads[p]) for x in sub[c]) > 0.15 * height
                   for c in rig.children[p] if c != k):
                break  # p forks into something big that is not this foot (a head, a wing): it is the body
            k = p
        if rig.parent[k] < 0:
            continue
        attach = heads[rig.parent[k]]
        drop = float(attach[1] - tips[f][1])
        reach = float(np.linalg.norm(_horizontal(attach - tips[f])))
        # the lower half of a leg comes down steeply onto its foot; a tail lying on the floor runs along it
        up = rig.path_up(f)
        chain = list(reversed(up[:up.index(k) + 1]))
        mid = heads[chain[len(chain) // 2]] if len(chain) > 2 else attach
        steep = float(mid[1] - tips[f][1]) >= 0.5 * float(np.linalg.norm(_horizontal(mid - tips[f])))
        stands = min(low[j] for j in sub[k]) <= ground  # on its own skin, not the body it hangs from
        if drop >= LEG_ATTACH * height and drop >= LEG_DROP * reach and steep and stands:
            limbs.setdefault(k, f)  # toes of one foot climb to the same limb; the first found is the foot
    limbs = {k: f for k, f in limbs.items() if not any(k in sub[o] and o != k for o in limbs)}
    # two limbs from one joint whose feet stand together are one leg split in two bones: they move as one
    twins = {}
    for k in sorted(limbs, key=lambda x: -len(sub[x])):
        for o in list(limbs):
            if o == k or k not in limbs or rig.parent[o] != rig.parent[k]:
                continue
            base = heads[rig.parent[k]]
            u, v = _horizontal(tips[limbs[o]] - base), _horizontal(tips[limbs[k]] - base)
            same_way = float(_unit(u) @ _unit(v)) > np.cos(np.radians(30))  # mirrored legs point apart
            if same_way and np.linalg.norm(u - v) < 1.5 * FOOT_SPAN * height:
                twins.setdefault(k, []).append(o)
                del limbs[o]
    in_leg = set().union(*(sub[k] for k in limbs), *(sub[o] for t in twins.values() for o in t)) if limbs else set()

    # the sagittal plane; the spine is the longest chain of body joints that lie on it
    normal, centre, asym = mirror_plane(heads, height)
    body = [k for k in range(n) if k not in in_leg]
    midline = {k for k in body if abs(float((heads[k] - centre) @ normal)) < 0.04 * height}
    root = next(k for k in rig.order if rig.parent[k] < 0)
    allowed = set(body)
    if asym < SYMMETRIC and root in midline:
        keep, stack = set(), [root]  # the midline joints connected to the root
        while stack:
            k = stack.pop()
            keep.add(k)
            stack += [c for c in rig.children[k] if c in midline and c not in keep]
        allowed = keep if len(keep) >= 2 else allowed
    adj = {k: [] for k in allowed}
    for k in allowed:
        p = rig.parent[k]
        if p in adj:
            w = float(np.linalg.norm(heads[k] - heads[p]))
            adj[k].append((p, w))
            adj[p].append((k, w))

    def farthest(start):
        dist, prev, stack = {start: 0.0}, {start: None}, [start]
        while stack:
            a = stack.pop()
            for b, w in adj[a]:
                if b not in dist:
                    dist[b], prev[b] = dist[a] + w, a
                    stack.append(b)
        end = max(dist, key=dist.get)
        path = [end]
        while prev[path[-1]] is not None:
            path.append(prev[path[-1]])
        return end, path

    a, _ = farthest(root)
    _, path = farthest(a)
    ends = (path[0], path[-1])
    cue = sum((_horizontal(skin[dom == f].mean(axis=0) - heads[f]) for f in limbs.values() if (dom == f).any()),
              np.zeros(3))  # toes reach forward of the ankle

    def girth(end):  # how thick the body is near one end (a snake's or a fish's head is the thicker end)
        part = path[:max(2, len(path) // 5)] if end == ends[0] else path[-max(2, len(path) // 5):]
        near = np.isin(dom, part)
        return float(np.linalg.norm(skin[near] - heads[dom[near]], axis=1).mean()) if near.any() else 0.0

    def eared(end):
        """Ears, horns or a jaw fork off near this end of the chain (non-leg branches, two or more at a joint)."""
        part = path[:max(3, len(path) // 4)] if end == ends[0] else path[-max(3, len(path) // 4):]
        chain = set(path)
        return any(len([c for c in rig.children[k] if c not in chain and c not in limbs
                        and not any(x in sub[c] for x in limbs)]) >= 2 for k in part)

    def pick_head(direction):
        """The head end, given each end's horizontal direction from the body (ends[0], ends[1]). The end with
        ears comes first: a bushy tail held high is higher than the head, and paws can point either way."""
        # Refitting missing legs only changes the lower body. Round feet on
        # those controls supply no new evidence about which end is the head.
        kept = rig.doc["skins"][0].get("extras", {}).get("original_head_node")
        for end in ends:
            if rig.nodes[end] == kept:
                return end
        if eared(ends[0]) != eared(ends[1]):
            return ends[0] if eared(ends[0]) else ends[1]
        lean = [float(cue @ _unit(d)) for d in direction]
        if abs(lean[0] - lean[1]) > 0.02 * height * max(len(limbs), 1):
            return ends[0] if lean[0] > lean[1] else ends[1]
        if not limbs:
            return ends[0] if girth(ends[0]) >= girth(ends[1]) else ends[1]
        return ends[0] if heads[ends[0]][1] >= heads[ends[1]][1] else ends[1]

    rise = float(heads[ends[0]][1] - heads[ends[1]][1])
    swimmer = fish_forward(skin) if swimming else None
    if swimmer is not None:
        forward = swimmer
        head_end = max(ends, key=lambda k: heads[k] @ forward)
    elif asym < SYMMETRIC:  # mirror-symmetric: forward lies in the mirror plane
        axis = _unit(np.cross(normal, UP))
        along = float((heads[ends[0]] - heads[ends[1]]) @ axis)
        long_body = abs(along) > 0.6 * abs(rise) and abs(along) > 0.25 * height
        if long_body:
            head_end = pick_head([axis * np.sign(along), -axis * np.sign(along)])
            forward = axis * (np.sign(along) if head_end == ends[0] else -np.sign(along))
        else:
            head_end = ends[0] if rise >= 0 else ends[1]
            lean = float(cue @ axis)
            strong = abs(lean) > 0.02 * height * max(len(limbs), 1)
            forward = axis * ((1.0 if lean > 0 else -1.0) if strong else (1.0 if axis[2] >= 0 else -1.0))
    else:  # no clean mirror plane (a tail curled to one side): each end's direction from where they meet
        lca = next(k for k in rig.path_up(ends[0]) if k in set(rig.path_up(ends[1])))
        reach = [_horizontal(heads[e] - heads[lca]) for e in ends]
        long_body = max(np.linalg.norm(v) for v in reach) > 0.25 * height
        if long_body:
            head_end = pick_head(reach)
            h, t = (0, 1) if head_end == ends[0] else (1, 0)
            forward = _unit(reach[h] if np.linalg.norm(reach[h]) > 0.15 * height else -reach[t])
        else:
            head_end = ends[0] if rise >= 0 else ends[1]
            forward = _unit(cue) if np.linalg.norm(cue) > 0.02 * height else np.array([0.0, 0.0, 1.0])
    tail_end = ends[1] if head_end == ends[0] else ends[0]
    left = np.cross(UP, forward)

    up_head, up_tail = rig.path_up(head_end), rig.path_up(tail_end)
    hub = next(k for k in up_head if k in set(up_tail))
    head_path = list(reversed(up_head[:up_head.index(hub) + 1]))  # hub .. head
    tail_path = list(reversed(up_tail[:up_tail.index(hub)]))      # after the hub .. tail tip
    root_path = list(reversed(up_head[up_head.index(hub) + 1:]))   # root .. the hub's parent
    on_path = set(head_path) | set(tail_path) | set(root_path)
    # the head is where ears or a jaw fork off near the end of the chain; a snout beyond it is part of it
    def forks(k):  # ears, horns or a jaw: children that are not legs and not the chain itself
        return len([c for c in rig.children[k] if c not in limbs and not any(x in sub[c] for x in limbs)]) >= 2
    after = head_path[1:]
    head = next((k for k in reversed(after[-3:]) if forks(k)),
                after[-2] if len(after) >= 2 else head_path[-1])
    snout = head_path[head_path.index(head) + 1:]
    head_path = head_path[:head_path.index(head) + 1]

    # the line feet stand around (a generated skeleton's spine can sit off-centre); without legs, the spine
    mid = np.mean([tips[f] for f in limbs.values()], axis=0) if limbs else heads[head_path].mean(axis=0)

    def side_of(point, scale):
        s = float((point - mid) @ left)
        return "left" if s > scale else "right" if s < -scale else "center"

    # legs: front to back by where they hang, side by where the foot stands
    legs = []
    spread = max([abs(float((tips[f] - mid) @ left)) for f in limbs.values()] + [1e-9])
    for k, f in limbs.items():
        up = rig.path_up(f)
        legs.append({"root": k, "chain": list(reversed(up[:up.index(k) + 1])), "attach": rig.parent[k],
                     "twins": twins.get(k, []),
                     # an even count stands in left/right pairs: no leg is on the centre line
                     "x": float(heads[k] @ forward),
                     "side": side_of(tips[f], max(0.25 * spread, 0.03 * height) if len(limbs) % 2 else 0.0)})
    legs.sort(key=lambda g: -g["x"])
    sides = [g["side"] for g in legs]
    if len(legs) == 4 and sides.count("left") == 2 and sides.count("right") == 2 and \
            sorted(sides[:2]) == ["left", "right"]:
        for i, g in enumerate(legs):
            g["name"] = f"leg_{'front' if i < 2 else 'hind'}_{g['side']}"
    elif len(legs) == 2 and sorted(sides) == ["left", "right"]:
        for g in legs:
            g["name"] = f"leg_{g['side']}"
    else:
        count = {}
        for g in legs:
            count[g["side"]] = count.get(g["side"], 0) + 1
            g["name"] = f"leg_{g['side']}_{count[g['side']]}"

    # other limbs: branches off the body chain that are not legs
    near_head = {head} | ({head_path[-2]} if len(head_path) > 1 else set())
    leg_attach = {g["attach"] for g in legs}
    others = []
    for k in on_path:
        for c in rig.children[k]:
            if c in on_path or c in in_leg:
                continue
            mine = [x for x in sub[c] if x not in in_leg]
            end = max(mine, key=lambda x: float(np.linalg.norm(heads[x] - heads[c])))
            sd = side_of(heads[end], 0.05 * height)
            if any(x in sub[c] for x in limbs):  # body segments the legs hang from (a spider's thorax)
                role, sd = "body", "center"
            else:
                role = "ear" if k in near_head and k not in leg_attach else "arm" if sd != "center" else "extra"
            others.append({"root": c, "role": role, "side": sd, "attach": k, "x": float(heads[c] @ forward), "end": end})
    others.sort(key=lambda o: -o["x"])
    count = {}
    for o in others:
        key = (o["role"], o["side"])
        count[key] = count.get(key, 0) + 1
        base = o["role"] if o["role"] in ("extra", "body") else f"{o['role']}_{o['side']}"
        o["name"] = f"{base}_{count[key]}" if o["role"] in ("extra", "body") or count[key] > 1 else base
    attached = [head_path.index(k) for k in leg_attach | {o["attach"] for o in others if o["role"] == "arm"}
                if k in head_path]
    neck_from = max(attached) + 1 if attached and max(attached) > 0 else None

    shape = None
    if not legs:
        rise_dir = abs(float(_unit(heads[head_end] - heads[tail_end]) @ UP)) if head_end != tail_end else 1.0
        along_skin = skin @ forward
        elongated = float(along_skin.max() - along_skin.min()) >= 1.5 * height  # a snake, a fish; not a slime
        shape = "rigid" if len(path) < 3 else "long" if rise_dir < 0.7 and elongated else "upright"
    return {"body_plan": {0: "legless", 4: "quadruped"}.get(len(legs), f"legs:{len(legs)}"),
            "legs": len(legs), "floor": floor, "height": height, "forward": forward.tolist(), "left": left.tolist(),
            "hub": hub, "root_path": root_path, "spine": head_path[:-1], "head": head, "snout": snout, "tail": tail_path,
            "neck_from": neck_from, "leg_chains": legs, "limbs": others, "legless_shape": shape,
            "symmetry_error": round(float(asym), 4)}


def readable_names(rig: Rig, plan: dict) -> dict[int, str]:
    """{joint index: readable name} for every joint of the skin."""
    names = {}
    for i, k in enumerate(plan["root_path"]):
        names[k] = "root" if i == 0 else f"root_{i + 1}"
    spine, neck_from = plan["spine"], plan["neck_from"]
    s = 0
    for i, k in enumerate(spine):
        if i == 0 and not plan["root_path"]:
            names[k] = "root"
        elif neck_from and i >= neck_from:
            names[k] = f"neck_{i - neck_from + 1}"
        else:
            s += 1
            names[k] = f"spine_{s}"
    names[plan["head"]] = "head" if plan["head"] not in names else names[plan["head"]]
    for i, k in enumerate(plan["snout"]):
        names[k] = f"head_tip_{i + 1}"
    for i, k in enumerate(plan["tail"]):
        names[k] = f"tail_{i + 1}"

    def limb(start, end, base):
        up = rig.path_up(end)
        for i, k in enumerate(reversed(up[:up.index(start) + 1])):
            names.setdefault(k, f"{base}_{i + 1}")
        branch = 0
        for k in rig.subtree(start):
            if k in names:
                continue
            branch += 1
            run = [k]
            while len(rig.children[run[-1]]) == 1:
                run.append(rig.children[run[-1]][0])
            for i, j in enumerate(run):
                names[j] = f"{base}_b{branch}_{i + 1}"

    for g in plan["leg_chains"]:
        limb(g["root"], g["chain"][-1], g["name"])
        for i, t in enumerate(g["twins"]):
            far = max(rig.subtree(t), key=lambda x: len(rig.path_up(x)))
            limb(t, far, f"{g['name']}_twin{i + 1}")
    for o in plan["limbs"]:
        limb(o["root"], o["end"], o["name"])
    extra = 0
    for k in rig.order:  # branches off a branch of the tail, and the like
        if k not in names:
            extra += 1
            names[k] = f"extra_bone_{extra}"
    if len(set(names.values())) != len(names):
        raise ValueError("body-plan joint names collide")
    return names


def summary(rig: Rig, plan: dict, names: dict[int, str]) -> dict:
    """The plan as the manifest records it: joint names per chain, no indices."""
    nm = lambda ks: [names[k] for k in ks]
    out = {"body_plan": plan["body_plan"], "legs": plan["legs"],
           "forward": [round(v, 4) for v in plan["forward"]], "floor_m": round(plan["floor"], 5),
           "height_m": round(plan["height"], 5),
           "chains": {"spine": nm(plan["root_path"] + plan["spine"]), "head": nm([plan["head"]] + plan["snout"]),
                      "tail": nm(plan["tail"])}}
    for g in plan["leg_chains"]:
        out["chains"][g["name"]] = nm(g["chain"])
        for i, t in enumerate(g["twins"]):
            out["chains"][f"{g['name']}_twin{i + 1}"] = nm([t])
    for o in plan["limbs"]:
        out["chains"][o["name"]] = nm(list(reversed(rig.path_up(o["end"])[:rig.path_up(o["end"]).index(o["root"]) + 1])))
    if plan["legless_shape"]:
        out["legless_shape"] = plan["legless_shape"]
    return out


def name_bones(path: Path) -> dict:
    """Give a non-humanoid rig readable joint names and at most four weights a vertex, in place."""
    if str(AF) not in sys.path:
        sys.path.insert(0, str(AF))
    from image_to_3dlab.bone_names import limit_influences, rename
    path = Path(path)
    rig = Rig(path)
    plan = detect(rig)
    names = readable_names(rig, plan)
    data, capped = limit_influences(rename(path.read_bytes(), {rig.names[k]: v for k, v in names.items()}))
    path.write_bytes(data)
    return {**summary(rig, plan, names), "renamed": len(names), "capped_vertices": capped}
