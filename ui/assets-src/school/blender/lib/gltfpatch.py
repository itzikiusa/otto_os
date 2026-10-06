"""Minimal glTF-binary editor (numpy): read/append accessors, forward
kinematics on node TRS, quaternion helpers, merge meshes from another .glb,
and compacting save.  Used for the headmaster, whose source animations key
only a few channels each — a Blender import/export round trip would re-base
the unkeyed joints on Blender's rest pose and break every clip, so the robot
is edited at the glTF level instead."""

import json
import struct

import numpy as np

from .glbinfo import Glb

COMP = {5126: (np.float32, 4), 5125: (np.uint32, 4), 5123: (np.uint16, 2), 5121: (np.uint8, 1),
        5122: (np.int16, 2), 5120: (np.int8, 1)}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


# ---------------------------------------------------------------- quaternions (x, y, z, w)

def q_mul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return np.array([aw * bx + ax * bw + ay * bz - az * by,
                     aw * by - ax * bz + ay * bw + az * bx,
                     aw * bz + ax * by - ay * bx + az * bw,
                     aw * bw - ax * bx - ay * by - az * bz])


def q_inv(q):
    return np.array([-q[0], -q[1], -q[2], q[3]]) / np.dot(q, q)


def q_rot(q, v):
    p = np.array([v[0], v[1], v[2], 0.0])
    return q_mul(q_mul(q, p), q_inv(q))[:3]


def q_axis(axis, deg):
    a = np.asarray(axis, dtype=float)
    a = a / np.linalg.norm(a)
    h = np.radians(deg) / 2
    return np.array([*(a * np.sin(h)), np.cos(h)])


def q_between(a, b):
    a = np.asarray(a, float) / np.linalg.norm(a)
    b = np.asarray(b, float) / np.linalg.norm(b)
    c = np.cross(a, b)
    d = np.dot(a, b)
    if d < -0.999999:
        ax = np.cross([1, 0, 0], a)
        if np.linalg.norm(ax) < 1e-6:
            ax = np.cross([0, 1, 0], a)
        return q_axis(ax, 180)
    q = np.array([*c, 1 + d])
    return q / np.linalg.norm(q)


def q_slerp(a, b, t):
    a = np.asarray(a, float)
    b = np.asarray(b, float)
    d = np.dot(a, b)
    if d < 0:
        b, d = -b, -d
    if d > 0.9995:
        r = a + (b - a) * t
        return r / np.linalg.norm(r)
    th = np.arccos(d)
    return (np.sin((1 - t) * th) * a + np.sin(t * th) * b) / np.sin(th)


def q_mat(q):
    x, y, z, w = q
    return np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                     [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                     [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])


def trs(t=(0, 0, 0), r=(0, 0, 0, 1), s=(1, 1, 1)):
    m = np.eye(4)
    m[:3, :3] = q_mat(r) @ np.diag(s)
    m[:3, 3] = t
    return m


# ---------------------------------------------------------------- document

class Doc:
    def __init__(self, path):
        g = Glb(path)
        self.j = g.json
        self.bin = bytearray(g.bin)
        self.parent = {}
        for i, n in enumerate(self.j["nodes"]):
            for c in n.get("children", []):
                self.parent[c] = i

    # accessors
    def acc(self, i):
        a = self.j["accessors"][i]
        dt, size = COMP[a["componentType"]]
        n = NCOMP[a["type"]]
        bv = self.j["bufferViews"][a["bufferView"]]
        off = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = bv.get("byteStride", size * n)
        if stride == size * n:
            arr = np.frombuffer(bytes(self.bin[off:off + a["count"] * stride]), dtype=dt).reshape(a["count"], n)
        else:
            arr = np.stack([np.frombuffer(bytes(self.bin[off + k * stride:off + k * stride + size * n]), dtype=dt)
                            for k in range(a["count"])])
        arr = arr.astype(np.float64)
        if a.get("normalized"):
            arr = arr / float(np.iinfo(dt).max)
        return arr

    def add_acc(self, arr, typ, comp=5126, target=None, minmax=False):
        arr = np.asarray(arr)
        dt, _ = COMP[comp]
        data = arr.astype(dt).tobytes()
        while len(self.bin) % 4:
            self.bin.append(0)
        bv = {"buffer": 0, "byteOffset": len(self.bin), "byteLength": len(data)}
        if target:
            bv["target"] = target
        self.bin += data
        self.j["bufferViews"].append(bv)
        a = {"bufferView": len(self.j["bufferViews"]) - 1, "componentType": comp,
             "count": int(arr.shape[0]), "type": typ}
        if minmax:
            flat = arr.reshape(arr.shape[0], -1)
            a["min"] = [float(x) for x in flat.min(axis=0)]
            a["max"] = [float(x) for x in flat.max(axis=0)]
        self.j["accessors"].append(a)
        return len(self.j["accessors"]) - 1

    # nodes
    def find(self, name, joint=None):
        joints = set(self.j["skins"][0]["joints"]) if self.j.get("skins") else set()
        for i, n in enumerate(self.j["nodes"]):
            if n.get("name") == name and (joint is None or (i in joints) == joint):
                return i
        raise KeyError(name)

    def local(self, i, rot=None):
        n = self.j["nodes"][i]
        if "matrix" in n:
            return np.array(n["matrix"]).reshape(4, 4).T
        return trs(n.get("translation", (0, 0, 0)), n.get("rotation", (0, 0, 0, 1)) if rot is None else rot,
                   n.get("scale", (1, 1, 1)))

    def world(self, i, rots=None):
        m = self.local(i, None if rots is None else rots.get(i))
        p = self.parent.get(i)
        while p is not None:
            m = self.local(p, None if rots is None else rots.get(p)) @ m
            p = self.parent.get(p)
        return m

    def world_rot(self, i, rots):
        """World rotation quaternion (assumes uniform positive scales)."""
        chain = []
        k = i
        while k is not None:
            chain.append(k)
            k = self.parent.get(k)
        q = np.array([0, 0, 0, 1.0])
        for k in reversed(chain):
            n = self.j["nodes"][k]
            q = q_mul(q, rots.get(k, np.array(n.get("rotation", (0, 0, 0, 1)), float)))
        return q

    def mesh_world_points(self, node_i):
        n = self.j["nodes"][node_i]
        pts = []
        for p in self.j["meshes"][n["mesh"]]["primitives"]:
            pts.append(self.acc(p["attributes"]["POSITION"]))
        pts = np.concatenate(pts)
        m = self.world(node_i)
        return (m @ np.c_[pts, np.ones(len(pts))].T).T[:, :3]

    # animations
    def anim(self, name):
        return next(a for a in self.j["animations"] if a["name"] == name)

    def sample(self, sampler, t):
        ts = self.acc(sampler["input"])[:, 0]
        vs = self.acc(sampler["output"])
        if t <= ts[0]:
            return vs[0]
        if t >= ts[-1]:
            return vs[-1]
        k = int(np.searchsorted(ts, t)) - 1
        u = (t - ts[k]) / (ts[k + 1] - ts[k])
        if vs.shape[1] == 4:
            return q_slerp(vs[k], vs[k + 1], u)
        return vs[k] + (vs[k + 1] - vs[k]) * u

    # images / skinned merges
    def add_image(self, data, mime="image/png", name=None):
        """Embed an image; returns a texture index."""
        while len(self.bin) % 4:
            self.bin.append(0)
        self.j["bufferViews"].append({"buffer": 0, "byteOffset": len(self.bin), "byteLength": len(data)})
        self.bin += data
        im = {"bufferView": len(self.j["bufferViews"]) - 1, "mimeType": mime}
        if name:
            im["name"] = name
        self.j.setdefault("images", []).append(im)
        self.j.setdefault("samplers", []).append({"magFilter": 9729, "minFilter": 9987, "wrapS": 33071,
                                                  "wrapT": 33071})
        self.j.setdefault("textures", []).append({"source": len(self.j["images"]) - 1,
                                                  "sampler": len(self.j["samplers"]) - 1})
        return len(self.j["textures"]) - 1

    def merge_skinned(self, src, mesh_name, joints, parent_node, name):
        """Copy skinned mesh `mesh_name` from Doc `src` (same joint names and
        order as `joints`), bound with the SOURCE inverse bind matrices so it
        sits on this skeleton exactly as it sat on its own."""
        sj = src.j
        node_i = next(i for i, n in enumerate(sj["nodes"]) if "mesh" in n and
                      sj["meshes"][n["mesh"]]["name"] == mesh_name)
        node = sj["nodes"][node_i]
        skin = sj["skins"][node["skin"]]
        src_names = [sj["nodes"][k]["name"] for k in skin["joints"]]
        dst_names = [self.j["nodes"][k]["name"] for k in joints]
        assert src_names == dst_names, "skeletons differ"
        mat_base = len(self.j["materials"])
        self.j["materials"] += [json.loads(json.dumps(m)) for m in sj["materials"]]
        prims = []
        for p in sj["meshes"][node["mesh"]]["primitives"]:
            attrs = {}
            for k, ai in p["attributes"].items():
                a = sj["accessors"][ai]
                arr = src.acc(ai)
                comp = a["componentType"]
                if k.startswith("JOINTS"):
                    arr = np.rint(arr).astype(np.uint16)
                    comp = 5123
                elif k.startswith("WEIGHTS") or comp == 5126:
                    comp = 5126
                attrs[k] = self.add_acc(arr, a["type"], comp, target=34962, minmax=(k == "POSITION"))
            idx = src.acc(p["indices"]).astype(np.uint32)
            prims.append({"attributes": attrs, "indices": self.add_acc(idx, "SCALAR", 5125, target=34963),
                          "material": mat_base + p.get("material", 0)})
        self.j["meshes"].append({"name": name, "primitives": prims})
        ibm = src.acc(skin["inverseBindMatrices"])
        self.j["skins"].append({"joints": list(joints), "inverseBindMatrices": self.add_acc(ibm, "MAT4")})
        self.j["nodes"].append({"name": name, "mesh": len(self.j["meshes"]) - 1,
                                "skin": len(self.j["skins"]) - 1})
        ni = len(self.j["nodes"]) - 1
        self.j["nodes"][parent_node].setdefault("children", []).append(ni)
        self.parent[ni] = parent_node
        return ni, mat_base

    def drop_node(self, i):
        n = self.j["nodes"][i]
        n.pop("mesh", None)
        n.pop("skin", None)
        p = self.parent.pop(i, None)
        if p is not None:
            self.j["nodes"][p]["children"].remove(i)
        else:
            sc = self.j["scenes"][self.j.get("scene", 0)]
            sc["nodes"].remove(i)

    # merge
    def merge_glb(self, path, parent_node, node_name):
        """Append every mesh of another .glb (materials included) as ONE node
        under `parent_node`, placed so it keeps its world position at rest."""
        src = Doc(path)
        mat_base = len(self.j.setdefault("materials", []))
        self.j["materials"] += src.j.get("materials", [])
        prims = []
        for node in src.j["nodes"]:
            if "mesh" not in node:
                continue
            m = src.world(src.j["nodes"].index(node))
            for p in src.j["meshes"][node["mesh"]]["primitives"]:
                pos = src.acc(p["attributes"]["POSITION"])
                nrm = src.acc(p["attributes"]["NORMAL"])
                pos = (m @ np.c_[pos, np.ones(len(pos))].T).T[:, :3]
                nrm = (m[:3, :3] @ nrm.T).T
                nrm /= np.linalg.norm(nrm, axis=1, keepdims=True)
                idx = src.acc(p["indices"])[:, 0].astype(np.uint32)
                attrs = {"POSITION": self.add_acc(pos, "VEC3", target=34962, minmax=True),
                         "NORMAL": self.add_acc(nrm, "VEC3", target=34962)}
                prims.append({"attributes": attrs,
                              "indices": self.add_acc(idx.reshape(-1, 1), "SCALAR", 5125, target=34963),
                              "material": mat_base + p.get("material", 0)})
        self.j["meshes"].append({"name": node_name, "primitives": prims})
        inv = np.linalg.inv(self.world(parent_node))
        self.j["nodes"].append({"name": node_name, "mesh": len(self.j["meshes"]) - 1,
                                "matrix": [float(x) for x in inv.T.ravel()]})
        ni = len(self.j["nodes"]) - 1
        self.j["nodes"][parent_node].setdefault("children", []).append(ni)
        self.parent[ni] = parent_node
        return ni

    def compact_attributes(self):
        """Core-glTF compaction (no extensions): JOINTS_0 → UNSIGNED_BYTE,
        WEIGHTS_0 → normalized UNSIGNED_BYTE (re-summed to 1), indices →
        UNSIGNED_SHORT when they fit."""
        mats = self.j.get("materials", [])
        for m in self.j.get("meshes", []):
            for p in m["primitives"]:
                at = p["attributes"]
                mt = mats[p["material"]] if "material" in p else {}
                if "TEXCOORD_0" in at and "Texture" not in json.dumps(mt):
                    del at["TEXCOORD_0"]  # untextured flat-colour material
                if "JOINTS_0" in at and "WEIGHTS_0" in at:
                    jj = np.rint(self.acc(at["JOINTS_0"])).astype(np.int64)
                    ww = self.acc(at["WEIGHTS_0"])
                    if jj.max() < 256:
                        at["JOINTS_0"] = self.add_acc(jj.astype(np.uint8), "VEC4", 5121, target=34962)
                    q = np.floor(ww * 255 + 0.5).astype(np.int64)
                    # fix rounding so each row sums to exactly 255
                    diff = 255 - q.sum(axis=1)
                    big = np.argmax(q, axis=1)
                    q[np.arange(len(q)), big] += diff
                    q = np.clip(q, 0, 255).astype(np.uint8)
                    at["WEIGHTS_0"] = self.add_acc(q, "VEC4", 5121, target=34962)
                    self.j["accessors"][at["WEIGHTS_0"]]["normalized"] = True
                if "indices" in p:
                    idx = np.rint(self.acc(p["indices"])).astype(np.int64)
                    n = self.j["accessors"][at["POSITION"]]["count"]
                    if n < 65535 and self.j["accessors"][p["indices"]]["componentType"] == 5125:
                        p["indices"] = self.add_acc(idx.astype(np.uint16), "SCALAR", 5123, target=34963)

    # save
    def save(self, path):
        """Drop unreferenced accessors/bufferViews, then write the .glb."""
        j = self.j
        # drop meshes/skins no node uses (e.g. a replaced head)
        mesh_used = sorted({n["mesh"] for n in j["nodes"] if "mesh" in n})
        mmap = {o: k for k, o in enumerate(mesh_used)}
        j["meshes"] = [j["meshes"][o] for o in mesh_used]
        skin_used = sorted({n["skin"] for n in j["nodes"] if "skin" in n})
        smap = {o: k for k, o in enumerate(skin_used)}
        j["skins"] = [j["skins"][o] for o in skin_used] if j.get("skins") else []
        for n in j["nodes"]:
            if "mesh" in n:
                n["mesh"] = mmap[n["mesh"]]
            if "skin" in n:
                n["skin"] = smap[n["skin"]]
        if not j["skins"]:
            j.pop("skins")
        used = set()
        for m in j.get("meshes", []):
            for p in m["primitives"]:
                used.update(p["attributes"].values())
                if "indices" in p:
                    used.add(p["indices"])
                for t in p.get("targets", []):
                    used.update(t.values())
        for s in j.get("skins", []):
            if "inverseBindMatrices" in s:
                used.add(s["inverseBindMatrices"])
        for a in j.get("animations", []):
            for s in a["samplers"]:
                used.update((s["input"], s["output"]))
        amap = {}
        new_acc = []
        for i, a in enumerate(j["accessors"]):
            if i in used:
                amap[i] = len(new_acc)
                new_acc.append(a)
        bv_used = sorted({a["bufferView"] for a in new_acc} |
                         {im["bufferView"] for im in j.get("images", []) if "bufferView" in im})
        bmap = {}
        out = bytearray()
        new_bv = []
        for b in bv_used:
            bv = dict(j["bufferViews"][b])
            while len(out) % 4:
                out.append(0)
            data = self.bin[bv.get("byteOffset", 0):bv.get("byteOffset", 0) + bv["byteLength"]]
            bv["byteOffset"] = len(out)
            out += data
            bmap[b] = len(new_bv)
            new_bv.append(bv)
        for a in new_acc:
            a["bufferView"] = bmap[a["bufferView"]]
        for im in j.get("images", []):
            if "bufferView" in im:
                im["bufferView"] = bmap[im["bufferView"]]
        j["accessors"] = new_acc
        j["bufferViews"] = new_bv
        for m in j.get("meshes", []):
            for p in m["primitives"]:
                p["attributes"] = {k: amap[v] for k, v in p["attributes"].items()}
                if "indices" in p:
                    p["indices"] = amap[p["indices"]]
                if "targets" in p:
                    p["targets"] = [{k: amap[v] for k, v in t.items()} for t in p["targets"]]
        for s in j.get("skins", []):
            if "inverseBindMatrices" in s:
                s["inverseBindMatrices"] = amap[s["inverseBindMatrices"]]
        for a in j.get("animations", []):
            for s in a["samplers"]:
                s["input"], s["output"] = amap[s["input"]], amap[s["output"]]
        while len(out) % 4:
            out.append(0)
        j["buffers"] = [{"byteLength": len(out)}]
        js = json.dumps(j, separators=(",", ":")).encode("utf-8")
        js += b" " * (-len(js) % 4)
        total = 12 + 8 + len(js) + 8 + len(out)
        blob = struct.pack("<III", 0x46546C67, 2, total) + struct.pack("<II", len(js), 0x4E4F534A) + js
        blob += struct.pack("<II", len(out), 0x004E4942) + bytes(out)
        with open(path, "wb") as f:
            f.write(blob)
