"""Dependency-free .glb reader: JSON chunk, node tree, clips, triangle count,
embedded image sizes and TEXCOORD_0 reads (for the verify pass and the
manifest).  Pure Python so it also runs outside Blender."""

import hashlib
import json
import struct


class Glb:
    def __init__(self, path):
        with open(path, "rb") as f:
            self.raw = f.read()
        magic, version, length = struct.unpack_from("<III", self.raw, 0)
        if magic != 0x46546C67 or version != 2:
            raise ValueError(f"{path}: not a glTF 2.0 binary")
        off = 12
        self.json = None
        self.bin = b""
        while off < length:
            clen, ctype = struct.unpack_from("<II", self.raw, off)
            chunk = self.raw[off + 8:off + 8 + clen]
            if ctype == 0x4E4F534A:
                self.json = json.loads(chunk.decode("utf-8"))
            elif ctype == 0x004E4942:
                self.bin = chunk
            off += 8 + clen
        self.path = path

    # ------------------------------------------------------------ basics
    @property
    def bytes(self):
        return len(self.raw)

    @property
    def sha256(self):
        return hashlib.sha256(self.raw).hexdigest()

    def nodes(self):
        return self.json.get("nodes", [])

    def top_nodes(self):
        scene = self.json["scenes"][self.json.get("scene", 0)]
        return [self.json["nodes"][i] for i in scene["nodes"]]

    def top_node_names(self):
        return [n.get("name", "") for n in self.top_nodes()]

    def node_by_name(self, name):
        for i, n in enumerate(self.nodes()):
            if n.get("name") == name:
                return i, n
        return None, None

    def children(self, node):
        return [self.json["nodes"][i] for i in node.get("children", [])]

    def descendants(self, node):
        out = []
        for c in self.children(node):
            out.append(c)
            out += self.descendants(c)
        return out

    def clips(self):
        return [a.get("name", "") for a in self.json.get("animations", [])]

    def clip_duration(self, name):
        for a in self.json.get("animations", []):
            if a.get("name") == name:
                acc = self.json["accessors"]
                return max(acc[s["input"]]["max"][0] for s in a["samplers"])
        return None

    def mesh_tris(self, mi):
        n = 0
        for p in self.json["meshes"][mi]["primitives"]:
            mode = p.get("mode", 4)
            if "indices" in p:
                cnt = self.json["accessors"][p["indices"]]["count"]
            else:
                cnt = self.json["accessors"][p["attributes"]["POSITION"]]["count"]
            if mode == 4:
                n += cnt // 3
            elif mode in (5, 6):
                n += max(0, cnt - 2)
        return n

    def tris(self):
        """Triangles drawn by the default scene (each node's mesh counted per use)."""
        total = 0
        for node in self.top_nodes():
            for nd in [node] + self.descendants(node):
                if "mesh" in nd:
                    total += self.mesh_tris(nd["mesh"])
        return total

    def node_tris(self, node):
        return sum(self.mesh_tris(nd["mesh"]) for nd in [node] + self.descendants(node) if "mesh" in nd)

    def materials_of(self, node):
        out = []
        if "mesh" in node:
            for p in self.json["meshes"][node["mesh"]]["primitives"]:
                if "material" in p:
                    out.append(p["material"])
        return out

    def material_users(self):
        """material index → number of primitives referencing it."""
        users = {}
        for m in self.json.get("meshes", []):
            for p in m["primitives"]:
                if "material" in p:
                    users[p["material"]] = users.get(p["material"], 0) + 1
        return users

    # ------------------------------------------------------------ buffers
    def _view(self, acc_i):
        acc = self.json["accessors"][acc_i]
        bv = self.json["bufferViews"][acc["bufferView"]]
        comps = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[acc["type"]]
        fmt = {5126: "f", 5123: "H", 5125: "I", 5121: "B"}[acc["componentType"]]
        size = struct.calcsize("<" + fmt)
        stride = bv.get("byteStride", size * comps)
        base = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
        out = []
        for i in range(acc["count"]):
            out.append(struct.unpack_from("<" + fmt * comps, self.bin, base + i * stride))
        return out

    def positions_uvs(self, node):
        p = self.json["meshes"][node["mesh"]]["primitives"][0]
        return self._view(p["attributes"]["POSITION"]), self._view(p["attributes"]["TEXCOORD_0"])

    def image_sizes(self):
        out = []
        for im in self.json.get("images", []):
            bv = self.json["bufferViews"][im["bufferView"]]
            data = self.bin[bv.get("byteOffset", 0):bv.get("byteOffset", 0) + bv["byteLength"]]
            out.append((im.get("name", ""), im.get("mimeType", ""), image_size(data), len(data)))
        return out


def image_size(data):
    if data[:8] == b"\x89PNG\r\n\x1a\n":
        w, h = struct.unpack(">II", data[16:24])
        return w, h
    if data[:2] == b"\xff\xd8":
        i = 2
        while i < len(data):
            if data[i] != 0xFF:
                i += 1
                continue
            marker = data[i + 1]
            if marker in (0xC0, 0xC1, 0xC2):
                h, w = struct.unpack(">HH", data[i + 5:i + 9])
                return w, h
            seg = struct.unpack(">H", data[i + 2:i + 4])[0]
            i += 2 + seg
    return (0, 0)


def rename_nodes(path, fn):
    """Rewrite node names in place (`fn(name) -> name`); used to strip Blender's
    `.001` uniqueness suffixes from names the contract repeats (DoorLeaf)."""
    g = Glb(path)
    changed = False
    for n in g.json.get("nodes", []):
        new = fn(n.get("name", ""))
        if new != n.get("name"):
            n["name"] = new
            changed = True
    if not changed:
        return False
    js = json.dumps(g.json, separators=(",", ":")).encode("utf-8")
    js += b" " * (-len(js) % 4)
    bn = g.bin + b"\0" * (-len(g.bin) % 4)
    total = 12 + 8 + len(js) + (8 + len(bn) if bn else 0)
    out = struct.pack("<III", 0x46546C67, 2, total)
    out += struct.pack("<II", len(js), 0x4E4F534A) + js
    if bn:
        out += struct.pack("<II", len(bn), 0x004E4942) + bn
    with open(path, "wb") as f:
        f.write(out)
    return True


def canonicalize_indices(path):
    """Point every index at the FIRST vertex with identical attributes.
    Blender's exporter sometimes keeps duplicate (bit-identical) vertices and
    picks between them run-to-run; this makes the bytes reproducible without
    changing the geometry."""
    g = Glb(path)
    j = g.json
    buf = bytearray(g.bin)

    def view(acc_i):
        a = j["accessors"][acc_i]
        bv = j["bufferViews"][a["bufferView"]]
        size = {5126: 4, 5125: 4, 5123: 2, 5121: 1, 5122: 2, 5120: 1}[a["componentType"]]
        n = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[a["type"]]
        stride = bv.get("byteStride", size * n)
        off = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        return a, off, stride, size * n

    changed = False
    for m in j.get("meshes", []):
        for p in m["primitives"]:
            if "indices" not in p:
                continue
            attrs = [view(i) for _, i in sorted(p["attributes"].items())]
            count = attrs[0][0]["count"]
            first = {}
            canon = []
            for v in range(count):
                key = b"".join(bytes(buf[off + v * st:off + v * st + ln]) for _, off, st, ln in attrs)
                canon.append(first.setdefault(key, v))
            a, off, st, ln = view(p["indices"])
            fmt = {5125: "<I", 5123: "<H", 5121: "<B"}[a["componentType"]]
            idx = [canon[struct.unpack_from(fmt, buf, off + k * st)[0]] for k in range(a["count"])]
            if p.get("mode", 4) == 4:
                # same triangles, canonical order: rotate each so its smallest
                # index leads (winding kept), then sort the list.
                tris = []
                for t in range(0, len(idx) - len(idx) % 3, 3):
                    tri = idx[t:t + 3]
                    r = tri.index(min(tri))
                    tris.append(tuple(tri[r:] + tri[:r]))
                tris.sort()
                idx = [i for tri in tris for i in tri]
            for k, v in enumerate(idx):
                o = off + k * st
                if struct.unpack_from(fmt, buf, o)[0] != v:
                    struct.pack_into(fmt, buf, o, v)
                    changed = True
    if not changed:
        return False
    with open(path, "r+b") as f:
        raw = bytearray(f.read())
        # BIN chunk starts after header (12) + JSON chunk header (8) + JSON
        jlen = struct.unpack_from("<I", raw, 12)[0]
        start = 12 + 8 + jlen + 8
        raw[start:start + len(buf)] = buf
        f.seek(0)
        f.write(raw)
    return True
