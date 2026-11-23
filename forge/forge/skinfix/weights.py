"""Change only glTF weight accessors; keep animation, embedded materials and bind data."""

import json
import struct
import sys
from pathlib import Path

import numpy as np


def unpack_glb(raw):
    magic, version, size = struct.unpack_from("<4sII", raw)
    if (magic, version, size) != (b"glTF", 2, len(raw)):
        raise ValueError("Expected a GLB 2.0 file")
    length, kind = struct.unpack_from("<I4s", raw, 12)
    if kind != b"JSON":
        raise ValueError("Missing GLB JSON")
    doc = json.loads(raw[20 : 20 + length])
    offset = 20 + length
    size, kind = struct.unpack_from("<I4s", raw, offset)
    if kind != b"BIN\0" or offset + 8 + size != len(raw):
        raise ValueError("Expected one embedded GLB binary chunk")
    if (
        len(doc["buffers"]) != 1
        or "uri" in doc["buffers"][0]
        or any("uri" in i for i in doc.get("images", []))
    ):
        raise ValueError("Embed the buffers and images in the input GLB first")
    return doc, bytearray(raw[offset + 8 :])


def read(doc, binary, index, shape, allowed):
    a = doc["accessors"][index]
    formats = {5121: "u1", 5123: "<u2", 5126: "<f4"}
    if a["type"] != shape or a["componentType"] not in allowed or "sparse" in a:
        raise ValueError(f"Unsupported format for skin accessor {index}")
    v = doc["bufferViews"][a["bufferView"]]
    if v.get("extensions") or v.get("buffer") != 0:
        raise ValueError("Decompress skin attributes before running skinfix")
    dtype = np.dtype(formats[a["componentType"]])
    cols = {"VEC3": 3, "VEC4": 4, "MAT4": 16}[shape]
    width = cols * dtype.itemsize
    stride = v.get("byteStride", width)
    offset = a.get("byteOffset", 0)
    if (
        stride < width
        or offset < 0
        or a["count"] < 1
        or offset + (a["count"] - 1) * stride + width > v["byteLength"]
    ):
        raise ValueError(f"Invalid buffer range for skin accessor {index}")
    values = np.ndarray(
        (a["count"], cols),
        dtype=dtype,
        buffer=binary,
        offset=v.get("byteOffset", 0) + offset,
        strides=(stride, dtype.itemsize),
    ).copy()
    if a.get("normalized"):
        if dtype.kind != "u":
            raise ValueError("Invalid normalized component type")
        values = values.astype(float) / np.iinfo(dtype).max
    return values


def store_weights(doc, binary, values):
    block = np.asarray(values, dtype="<f4").tobytes()
    binary.extend(b"\0" * (-len(binary) % 4))
    doc["bufferViews"].append(
        {
            "buffer": 0,
            "byteOffset": len(binary),
            "byteLength": len(block),
            "target": 34962,
        }
    )
    binary.extend(block)
    doc["accessors"].append(
        {
            "bufferView": len(doc["bufferViews"]) - 1,
            "componentType": 5126,
            "count": len(values),
            "type": "VEC4",
        }
    )
    return len(doc["accessors"]) - 1


def process(src, dest, strength):
    raw = src.read_bytes()
    doc, binary = unpack_glb(raw)
    users = {}
    for node in doc["nodes"]:
        if "mesh" in node:
            users.setdefault(node["mesh"], set()).add(node.get("skin"))
    if not any(skins - {None} for skins in users.values()):
        raise ValueError("Input has no skinned mesh")
    count = changed = 0
    for mesh, skins in users.items():
        if skins == {None}:
            continue
        if len(skins) != 1:
            raise ValueError(
                "A mesh shared across different skins needs separate mesh instances"
            )
        skin = doc["skins"][next(iter(skins))]
        if "inverseBindMatrices" in skin:
            matrices = read(doc, binary, skin["inverseBindMatrices"], "MAT4", {5126})
            bind = np.linalg.inv(
                matrices.astype(float).reshape(-1, 4, 4).transpose(0, 2, 1)
            )
            centers = bind[:, :3, 3]
        else:
            centers = np.zeros((len(skin["joints"]), 3))
        primitives = doc["meshes"][mesh]["primitives"]
        xyz = [
            read(doc, binary, p["attributes"]["POSITION"], "VEC3", {5126})
            for p in primitives
        ]
        extent = max(
            np.max([p.max(axis=0) for p in xyz], axis=0)
            - np.min([p.min(axis=0) for p in xyz], axis=0)
        )
        if extent <= 0 or len(centers) != len(skin["joints"]):
            raise ValueError("Degenerate mesh bounds or invalid bind matrices")
        for prim, positions in zip(primitives, xyz):
            if "KHR_draco_mesh_compression" in prim.get("extensions", {}):
                raise ValueError("Decompress the Draco mesh before running skinfix")
            attrs = prim["attributes"]
            sets = sorted(int(k[8:]) for k in attrs if k.startswith("WEIGHTS_"))
            if not sets or sets != list(range(len(sets))):
                raise ValueError(
                    "Skinned primitive needs consecutive JOINTS_n and WEIGHTS_n attributes"
                )
            weights = np.hstack(
                [
                    read(doc, binary, attrs[f"WEIGHTS_{s}"], "VEC4", {5121, 5123, 5126})
                    for s in sets
                ]
            ).astype(float)
            joints = np.hstack(
                [
                    read(doc, binary, attrs[f"JOINTS_{s}"], "VEC4", {5121, 5123})
                    for s in sets
                ]
            )
            if (
                len(weights) != len(positions)
                or joints.shape != weights.shape
                or np.max(joints) >= len(centers)
            ):
                raise ValueError("Weight and joint data do not match the skin")
            if (
                not np.isfinite(weights).all()
                or weights.min() < 0
                or np.any(weights.sum(axis=1) <= 0)
            ):
                raise ValueError("Invalid or unweighted skin vertex")
            count += len(weights)
            if not strength:
                continue
            # A sphere around each pivot fades smoothly into the stiffer segment.
            # Distance uses mesh bind coordinates, independent of the current pose.
            distance = np.full(len(positions), np.inf)
            for center in centers:
                np.minimum(
                    distance, np.linalg.norm(positions - center, axis=1), out=distance
                )
            t = np.clip((distance / extent - 0.025) / 0.04, 0, 1)
            exponent = 1 + 5 * strength * t * t * (3 - 2 * t)
            weights /= weights.sum(axis=1, keepdims=True)
            sharpened = np.power(weights, exponent[:, None])
            sharpened /= sharpened.sum(axis=1, keepdims=True)
            changed += np.count_nonzero(np.max(abs(sharpened - weights), axis=1) > 1e-6)
            for s in sets:
                attrs[f"WEIGHTS_{s}"] = store_weights(
                    doc, binary, sharpened[:, s * 4 : s * 4 + 4]
                )
    if strength:
        doc["buffers"][0]["byteLength"] = len(binary)
        js = json.dumps(doc, separators=(",", ":"), allow_nan=False).encode()
        js += b" " * (-len(js) % 4)
        raw = (
            struct.pack(
                "<4sIIII", b"glTF", 2, 28 + len(js) + len(binary), len(js), 0x4E4F534A
            )
            + js
            + struct.pack("<I4s", len(binary), b"BIN\0")
            + binary
        )
    dest.write_bytes(raw)
    return {"vertices": count, "changed_vertices": int(changed)}


if __name__ == "__main__":
    import bpy

    args = sys.argv[sys.argv.index("--") + 1 :]
    try:
        stats = process(Path(args[0]), Path(args[1]), float(args[2]))
        print(
            "SKINFIX " + json.dumps(dict(stats, blender=bpy.app.version_string)),
            flush=True,
        )
    except Exception as exc:
        print("SKINFIX " + json.dumps({"error": str(exc)}), flush=True)
        raise
