"""Read the publisher's glTF, and package its buffers and material images without re-importing geometry."""

import base64
import json
import mimetypes
import struct
from urllib.parse import unquote, unquote_to_bytes, urlsplit


def read(path, *, include_binary=True):
    if path.suffix.lower() == ".gltf":
        return json.loads(path.read_bytes()), b""
    with path.open("rb") as f:
        magic, version, length = struct.unpack("<4sII", f.read(12))
        if (magic, version) != (b"glTF", 2) or length != path.stat().st_size:
            raise ValueError(f"invalid GLB header in {path}")
        size, kind = struct.unpack("<II", f.read(8))
        if kind != 0x4E4F534A:
            raise ValueError("missing glTF JSON chunk")
        doc = json.loads(f.read(size))
        if not include_binary:
            return doc, b""
        header = f.read(8)
        if not header:
            return doc, b""
        size, kind = struct.unpack("<II", header)
        if kind != 0x004E4942:
            raise ValueError("expected a glTF BIN chunk")
        return doc, f.read(size)


def payload(uri, source, pack):
    if uri.startswith("data:"):
        head, data = uri.split(",", 1)
        return (
            base64.b64decode(data) if ";base64" in head else unquote_to_bytes(data),
            head[5:].split(";")[0],
        )
    if urlsplit(uri).scheme or urlsplit(uri).netloc:
        raise ValueError(f"expected a local glTF resource in {source.name}: {uri}")
    path = (source.parent / unquote(uri)).resolve()
    if not path.is_relative_to(pack.resolve()):
        raise ValueError(f"resource outside its pack: {uri}")
    # The Standard sci-fi decal refers to Blender's duplicate suffix, but ships the
    # matching unsuffixed binary alongside its glTF (accessor ranges verified).
    if not path.is_file() and path.name == source.stem + "_001.bin":
        path = source.with_suffix(".bin")
    if not path.is_file() and path.name.endswith("_png.png"):
        corrected = path.with_name(path.name[:-8] + ".png")
        if corrected.is_file():
            path = corrected
    if not path.is_file():
        # Some publisher glTFs reference Blender's image filename, although the archive puts
        # the texture in a separate directory. Prefer its non-Unreal copy for glTF normals.
        candidates = sorted(p for p in pack.rglob(path.name) if p.is_file())
        if not candidates and path.name.endswith("_png.png"):
            candidates = sorted(
                p for p in pack.rglob(path.name[:-8] + ".png") if p.is_file()
            )
        if not candidates:
            raise FileNotFoundError(f"{source.name} references missing resource {uri}")
        preferred = [
            p for p in candidates if "unreal" not in str(p.relative_to(pack)).lower()
        ]
        options = preferred or candidates
        data = options[0].read_bytes()
        if any(x.read_bytes() != data for x in options[1:]):
            raise ValueError(f"ambiguous texture {uri} in {pack.name}")
    else:
        data = path.read_bytes()
    return data, mimetypes.guess_type(path.name)[0]


def buffers(doc, embedded, source, pack):
    result = []
    for b in doc.get("buffers", []):
        data = payload(b["uri"], source, pack)[0] if "uri" in b else embedded
        size = b["byteLength"]
        if len(data) < size:
            raise ValueError(f"truncated buffer in {source.name}")
        result.append(data[:size])
    return result


def append_accessor(doc, binary, values, shape, component):
    """Append aligned, tightly packed glTF accessor data to an embedded buffer."""
    import numpy as np
    raw = np.ascontiguousarray(values).tobytes()
    binary.extend(b"\0" * (-len(binary) % 4))
    doc["bufferViews"].append(dict(buffer=0, byteOffset=len(binary), byteLength=len(raw)))
    binary.extend(raw)
    doc["accessors"].append(dict(bufferView=len(doc["bufferViews"]) - 1, componentType=component,
                                 type=shape, count=len(values)))
    return len(doc["accessors"]) - 1


def write(doc, data, output):
    doc["buffers"] = [{"byteLength": len(data)}]
    meta = json.dumps(doc, separators=(",", ":"), ensure_ascii=False).encode()
    meta += b" " * (-len(meta) % 4)
    pad = b"\0" * (-len(data) % 4)
    with output.open("wb") as f:
        f.write(
            struct.pack(
                "<4sIIII",
                b"glTF",
                2,
                28 + len(meta) + len(data) + len(pad),
                len(meta),
                0x4E4F534A,
            )
        )
        f.write(meta)
        f.write(struct.pack("<II", len(data) + len(pad), 0x004E4942))
        f.write(data)
        f.write(pad)


def pack_glb(source, pack, output, action=None):
    doc, embedded = read(source)
    if action is not None:
        doc["animations"] = [
            a for a in doc.get("animations", []) if a.get("name") == action
        ]
        if len(doc["animations"]) != 1:
            raise ValueError(f"expected exactly one animation named {action}")
    data = bytearray()
    offsets = []
    for part in buffers(doc, embedded, source, pack):
        data.extend(bytes(-len(data) % 4))
        offsets.append(len(data))
        data.extend(part)
    views = doc.setdefault("bufferViews", [])
    for v in views:
        v["byteOffset"] = offsets[v["buffer"]] + v.get("byteOffset", 0)
        v["buffer"] = 0
    for im in doc.get("images", []):
        if "uri" not in im:
            continue
        content, mime = payload(im.pop("uri"), source, pack)
        if mime not in ("image/png", "image/jpeg", "image/webp", "image/ktx2"):
            raise ValueError(f"unsupported embedded image {mime}")
        data.extend(bytes(-len(data) % 4))
        im.update(mimeType=mime, bufferView=len(views))
        views.append({"buffer": 0, "byteOffset": len(data), "byteLength": len(content)})
        data.extend(content)
    write(doc, data, output)
