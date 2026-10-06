"""Pose authoring on a glTF skeleton (numpy, no Blender round trip).

Clips are evaluated to per-node local TRS, edited with world-space operations
(rotate about a joint, aim a bone at a direction/point, move a joint), then
written back as glTF animations that key every joint whose value differs from
its rest TRS — so three.js never falls back to the (FBX bind) rest pose for a
joint the clip cares about.  Includes CPU skinning so builds can measure the
posed mesh (height, seat contact, hand reach) instead of guessing.
"""

import numpy as np

from .gltfpatch import q_between, q_inv, q_mul, q_slerp, q_axis, q_mat

FPS = 30


def mat_to_q(m):
    m = np.asarray(m, float)
    t = np.trace(m)
    if t > 0:
        s = np.sqrt(t + 1.0) * 2
        q = [(m[2, 1] - m[1, 2]) / s, (m[0, 2] - m[2, 0]) / s, (m[1, 0] - m[0, 1]) / s, 0.25 * s]
    elif m[0, 0] > m[1, 1] and m[0, 0] > m[2, 2]:
        s = np.sqrt(1.0 + m[0, 0] - m[1, 1] - m[2, 2]) * 2
        q = [0.25 * s, (m[0, 1] + m[1, 0]) / s, (m[0, 2] + m[2, 0]) / s, (m[2, 1] - m[1, 2]) / s]
    elif m[1, 1] > m[2, 2]:
        s = np.sqrt(1.0 + m[1, 1] - m[0, 0] - m[2, 2]) * 2
        q = [(m[0, 1] + m[1, 0]) / s, 0.25 * s, (m[1, 2] + m[2, 1]) / s, (m[0, 2] - m[2, 0]) / s]
    else:
        s = np.sqrt(1.0 + m[2, 2] - m[0, 0] - m[1, 1]) * 2
        q = [(m[0, 2] + m[2, 0]) / s, (m[1, 2] + m[2, 1]) / s, 0.25 * s, (m[1, 0] - m[0, 1]) / s]
    q = np.array(q)
    return q / np.linalg.norm(q)


def compose(t, r, s):
    m = np.eye(4)
    m[:3, :3] = q_mat(r) @ np.diag(s)
    m[:3, 3] = t
    return m


class Rig:
    def __init__(self, doc, skin=0):
        self.doc = doc
        j = doc.j
        self.joints = list(j["skins"][skin]["joints"])
        self.name = {j["nodes"][i]["name"]: i for i in self.joints}
        # skeleton = joints + their ancestors (RootNode, armature) — in order
        nodes = set()
        for i in self.joints:
            k = i
            while k is not None:
                nodes.add(k)
                k = doc.parent.get(k)
        self.nodes = sorted(nodes, key=self._depth)
        self.rest = {}
        for i in self.nodes:
            n = j["nodes"][i]
            self.rest[i] = (np.array(n.get("translation", (0, 0, 0)), float),
                            np.array(n.get("rotation", (0, 0, 0, 1)), float),
                            np.array(n.get("scale", (1, 1, 1)), float))

    def _depth(self, i):
        d = 0
        k = self.doc.parent.get(i)
        while k is not None:
            d += 1
            k = self.doc.parent.get(k)
        return d

    def refresh_rest(self):
        for i in self.nodes:
            n = self.doc.j["nodes"][i]
            self.rest[i] = (np.array(n.get("translation", (0, 0, 0)), float),
                            np.array(n.get("rotation", (0, 0, 0, 1)), float),
                            np.array(n.get("scale", (1, 1, 1)), float))

    def j(self, name):
        return self.name[name]

    # ------------------------------------------------------------ evaluation
    def pose_at(self, clip, t):
        """Local TRS dict for every skeleton node at time t of `clip`
        (None → rest)."""
        P = {i: [v.copy() for v in self.rest[i]] for i in self.nodes}
        if clip is None:
            return P
        a = self.doc.anim(clip)
        for c in a["channels"]:
            node = c["target"]["node"]
            if node not in P:
                continue
            path = c["target"]["path"]
            if path == "weights":
                continue
            v = self.doc.sample(a["samplers"][c["sampler"]], t)
            P[node][{"translation": 0, "rotation": 1, "scale": 2}[path]] = np.array(v, float)
        return P

    def duration(self, clip):
        a = self.doc.anim(clip)
        return max(self.doc.acc(s["input"])[-1, 0] for s in a["samplers"])

    def world(self, P):
        W = {}
        for i in self.nodes:
            m = compose(*P[i])
            p = self.doc.parent.get(i)
            W[i] = W[p] @ m if p is not None else m
        return W

    def pos(self, P, name, W=None):
        W = W or self.world(P)
        return W[self.j(name)][:3, 3].copy()

    # ------------------------------------------------------------ edits
    def _rot_world(self, P, i, R, W=None):
        W = W or self.world(P)
        p = self.doc.parent[i]
        wp = W[p][:3, :3]
        wp_r = wp / np.linalg.norm(wp, axis=0)
        qp = mat_to_q(wp_r)
        P[i][1] = q_mul(q_inv(qp), q_mul(R, q_mul(qp, P[i][1])))
        P[i][1] /= np.linalg.norm(P[i][1])

    def rot(self, P, name, axis, deg):
        """Rotate a joint about its own pivot around a world axis."""
        self._rot_world(P, self.j(name), q_axis(axis, deg))

    def aim(self, P, name, child, direction):
        """Turn `name` so the bone towards `child` points along `direction`."""
        W = self.world(P)
        a = W[self.j(name)][:3, 3]
        b = W[self.j(child)][:3, 3]
        self._rot_world(P, self.j(name), q_between(b - a, direction), W)

    def aim_at(self, P, name, child, target):
        W = self.world(P)
        a = W[self.j(name)][:3, 3]
        self.aim(P, name, child, np.asarray(target, float) - a)

    def move(self, P, name, delta):
        """Translate a joint by a world-space delta."""
        W = self.world(P)
        i = self.j(name)
        wp = W[self.doc.parent[i]][:3, :3]
        P[i][0] = P[i][0] + np.linalg.solve(wp, np.asarray(delta, float))

    def node_world(self, P, i, W=None):
        """World matrix of ANY node (rigid meshes under joints included)."""
        W = W or self.world(P)
        if i in W:
            return W[i]
        p = self.doc.parent.get(i)
        m = self.doc.local(i)
        return (self.node_world(P, p, W) @ m) if p is not None else m

    def skin_normals(self, P, node_i):
        W = self.world(P)
        j = self.doc.j
        node = j["nodes"][node_i]
        skin = j["skins"][node["skin"]]
        ibm = self.doc.acc(skin["inverseBindMatrices"]).reshape(-1, 4, 4).transpose(0, 2, 1)
        mats = np.stack([(W[jn] @ ibm[k])[:3, :3] for k, jn in enumerate(skin["joints"])])
        out = []
        for p in j["meshes"][node["mesh"]]["primitives"]:
            nrm = self.doc.acc(p["attributes"]["NORMAL"])
            jj = self.doc.acc(p["attributes"]["JOINTS_0"]).astype(int)
            ww = self.doc.acc(p["attributes"]["WEIGHTS_0"])
            acc = np.zeros_like(nrm)
            for k in range(4):
                acc += ww[:, k:k + 1] * np.einsum("nij,nj->ni", mats[jj[:, k]], nrm)
            acc /= np.linalg.norm(acc, axis=1, keepdims=True) + 1e-12
            out.append(acc)
        return out

    # ------------------------------------------------------------ skinning
    def skin_points(self, P, mesh_nodes):
        """World positions of the skinned vertices of `mesh_nodes` in pose P."""
        W = self.world(P)
        out = []
        j = self.doc.j
        for mn in mesh_nodes:
            node = j["nodes"][mn]
            skin = j["skins"][node["skin"]]
            joints = skin["joints"]
            ibm = self.doc.acc(skin["inverseBindMatrices"]).reshape(-1, 4, 4).transpose(0, 2, 1)
            # joints may include nodes outside self.nodes only if another skin
            mats = np.stack([W[jn] @ ibm[k] for k, jn in enumerate(joints)])
            for p in j["meshes"][node["mesh"]]["primitives"]:
                pos = self.doc.acc(p["attributes"]["POSITION"])
                jj = self.doc.acc(p["attributes"]["JOINTS_0"]).astype(int)
                ww = self.doc.acc(p["attributes"]["WEIGHTS_0"])
                ph = np.c_[pos, np.ones(len(pos))]
                acc = np.zeros((len(pos), 4))
                for k in range(4):
                    acc += ww[:, k:k + 1] * np.einsum("nij,nj->ni", mats[jj[:, k]], ph)
                out.append(acc[:, :3])
        return np.concatenate(out)


def lerp_pose(A, B, t):
    out = {}
    for i in A:
        ta, ra, sa = A[i]
        tb, rb, sb = B[i]
        out[i] = [ta + (tb - ta) * t, q_slerp(ra, rb, t), sa + (sb - sa) * t]
    return out


def write_clip(rig, name, frames, dur):
    """frames: list of pose dicts sampled uniformly over [0, dur]."""
    doc = rig.doc
    n = len(frames)
    times = np.linspace(0, dur, n).reshape(-1, 1)
    t_full = doc.add_acc(times, "SCALAR", minmax=True)
    t_two = doc.add_acc(np.array([[0.0], [dur]]), "SCALAR", minmax=True)
    a = {"name": name, "channels": [], "samplers": []}
    t_cache = {}
    for i in rig.nodes:
        if i not in rig.joints:
            continue
        for k, path, typ in ((0, "translation", "VEC3"), (1, "rotation", "VEC4"), (2, "scale", "VEC3")):
            vals = np.array([f[i][k] for f in frames])
            if k == 1:  # keep quaternion hemisphere continuous
                for r in range(1, n):
                    if np.dot(vals[r], vals[r - 1]) < 0:
                        vals[r] = -vals[r]
            rest = rig.rest[i][k]
            tol = 1e-5 if k != 1 else 1e-5
            if k == 1:
                diff = np.max(1 - np.abs(vals @ rest))
            else:
                diff = np.max(np.abs(vals - rest)) / max(1e-9, np.max(np.abs(rest)) + 1e-3)
            # joints with a non-unit rest scale (the kid's head) always get a
            # scale track: some importers (Blender) drop rest scale on bones.
            force = k == 2 and np.max(np.abs(rest - 1.0)) > 1e-4
            if diff < tol and not force:
                continue
            const = np.max(np.abs(vals - vals[0])) < (2e-4 if k == 1 else 1e-6)
            if const:
                out = doc.add_acc(vals[:1].repeat(2, axis=0), typ)
                inp = t_two
            else:
                keep = _reduce(vals, 2e-4 if k == 1 else 2e-4 * (np.max(np.abs(rest)) + 1e-3))
                if len(keep) == n:
                    out, inp = doc.add_acc(vals, typ), t_full
                else:
                    key = tuple(keep)
                    if key not in t_cache:
                        t_cache[key] = doc.add_acc(times[keep], "SCALAR", minmax=True)
                    out, inp = doc.add_acc(vals[keep], typ), t_cache[key]
            a["samplers"].append({"input": inp, "output": out, "interpolation": "LINEAR"})
            a["channels"].append({"sampler": len(a["samplers"]) - 1, "target": {"node": i, "path": path}})
    doc.j["animations"].append(a)
    return a


def _reduce(vals, tol):
    """Indices of keys to keep so linear interpolation stays within `tol`
    (greedy; endpoints always kept)."""
    n = len(vals)
    keep = [0]
    i = 0
    while i < n - 1:
        j = i + 2
        while j < n:
            seg = vals[i:j + 1]
            t = np.linspace(0, 1, j - i + 1)[:, None]
            lin = vals[i] + (vals[j] - vals[i]) * t
            if np.max(np.abs(seg - lin)) > tol:
                break
            j += 1
        keep.append(j - 1)
        i = j - 1
    if keep[-1] != n - 1:
        keep.append(n - 1)
    return keep


def sample_frames(fn, dur, fps=FPS):
    n = max(2, int(round(dur * fps)) + 1)
    return [fn(k / (n - 1)) for k in range(n)], dur


def ik2(rig, P, upper, lower, end, target, pole):
    """Two-bone IK: place `end` at `target`, bending toward `pole` (world dir)."""
    W = rig.world(P)
    S, E, H = (W[rig.j(n)][:3, 3] for n in (upper, lower, end))
    a = np.linalg.norm(E - S)
    b = np.linalg.norm(H - E)
    d = np.asarray(target, float) - S
    L = np.clip(np.linalg.norm(d), abs(a - b) + 1e-4, a + b - 1e-4)
    dn = d / np.linalg.norm(d)
    x = (a * a - b * b + L * L) / (2 * L)
    h = np.sqrt(max(a * a - x * x, 0.0))
    p = np.asarray(pole, float)
    p = p - dn * np.dot(p, dn)
    p /= np.linalg.norm(p)
    elbow = S + dn * x + p * h
    rig.aim(P, upper, lower, elbow - S)
    rig.aim_at(P, lower, end, S + dn * L)
