"""Apply UAL tracks to the same joint hierarchy while keeping the target's bind proportions."""

import math
import struct
from pathlib import Path

from .animation import PACKS, samples
from .gltf import read, write

DEFAULT = {"translation": (0, 0, 0), "rotation": (0, 0, 0, 1), "scale": (1, 1, 1)}


def quaternion(a, b):
    x, y, z, w = a
    i, j, k, r = b
    return (
        w * i + x * r + y * k - z * j,
        w * j + y * r + z * i - x * k,
        w * k + z * r + x * j - y * i,
        w * r - x * i - y * j - z * k,
    )


def bind_relative(values, prop, source, target):
    before = source.get(prop, DEFAULT[prop])
    after = target.get(prop, DEFAULT[prop])
    if tuple(before) == tuple(after):
        return values
    if prop == "translation":
        return [
            tuple(dst + (v - src) for dst, src, v in zip(after, before, row))
            for row in values
        ]
    if prop == "scale":
        return [
            tuple(dst * (v / src) for dst, src, v in zip(after, before, row))
            for row in values
        ]
    # glTF rotations are unit quaternions in xyzw order. new = targetBind * inv(sourceBind) * old
    basis = quaternion(after, (-before[0], -before[1], -before[2], before[3]))
    magnitude = math.hypot(*basis)
    basis = tuple(v / magnitude for v in basis)
    return [quaternion(basis, row) for row in values]


def indices(doc):
    skins = doc.get("skins", [])
    if not skins:
        return {}, {}
    joints = skins[0]["joints"]
    if any(set(skin["joints"]) != set(joints) for skin in skins[1:]):
        return {}, {}
    nodes = doc["nodes"]

    def name(i):
        node = nodes[i]
        return node.get("extras", {}).get("publisher_name", node.get("name"))

    ids = {name(i): i for i in joints}
    if len(ids) != len(joints) or not ids or any("matrix" in nodes[i] for i in joints):
        return {}, {}
    parents = {
        child: i for i, node in enumerate(nodes) for child in node.get("children", [])
    }
    hierarchy = {
        name(i): name(parents[i]) if parents.get(i) in joints else None for i in joints
    }
    return ids, hierarchy


def append_values(target, buffer, original, values, is_time):
    row = struct.Struct("<" + "f" * len(values[0]))
    packed = b"".join(row.pack(*v) for v in values)
    buffer.extend(bytes(-len(buffer) % 4))
    view = len(target["bufferViews"])
    target["bufferViews"].append(
        {"buffer": 0, "byteOffset": len(buffer), "byteLength": len(packed)}
    )
    buffer.extend(packed)
    accessor = {
        "bufferView": view,
        "componentType": 5126,
        "count": len(values),
        "type": original["type"],
    }
    if is_time:
        accessor.update(min=original["min"], max=original["max"])
    target["accessors"].append(accessor)
    return len(target["accessors"]) - 1


def transfer_action(source, binary, target, buffer, joints, original, name, cache):
    result = {"name": name, "channels": [], "samplers": []}
    for channel in original["channels"]:
        src = source["nodes"][channel["target"]["node"]]
        node = joints[src["name"]]
        prop = channel["target"]["path"]
        sampler = original["samplers"][channel["sampler"]]
        if (
            sampler.get("interpolation", "LINEAR") not in ("LINEAR", "STEP")
            or prop not in DEFAULT
        ):
            raise ValueError("Unexpected UAL interpolation or channel type")
        new_sampler = dict(sampler)
        for which in ("input", "output"):
            idx = sampler[which]
            is_time = which == "input"
            key = (idx,) if is_time else (idx, node, prop)
            if key not in cache:
                values = samples(source, binary, idx)
                if not is_time:
                    values = bind_relative(values, prop, src, target["nodes"][node])
                cache[key] = append_values(
                    target, buffer, source["accessors"][idx], values, is_time
                )
            new_sampler[which] = cache[key]
        result["channels"].append(
            {"sampler": len(result["samplers"]), "target": {"node": node, "path": prop}}
        )
        result["samplers"].append(new_sampler)
    return result


def attach_native(mesh, jobs, destination):
    """Write full native tracks when all sources and the target share the publisher hierarchy."""
    if not all(job["pack"] in PACKS for job in jobs):
        return False
    target, target_binary = read(mesh)
    joints, target_hierarchy = indices(target)
    if not {"root", "pelvis", "pinky_03_r", "index_03_l"}.issubset(joints):
        return False
    buffer = bytearray(target_binary)
    target["animations"] = []
    for file in dict.fromkeys(job["file"] for job in jobs):
        source, binary = read(Path(file))
        _, hierarchy = indices(source)
        if not hierarchy or any(
            name not in joints or target_hierarchy[name] != parent
            for name, parent in hierarchy.items()
        ):
            return False
        actions = {a["name"]: a for a in source["animations"]}
        cache = {}
        for job in (j for j in jobs if j["file"] == file):
            animation = transfer_action(
                source,
                binary,
                target,
                buffer,
                joints,
                actions[job["action"]],
                job["id"],
                cache,
            )
            target["animations"].append(animation)
    order = {job["id"]: i for i, job in enumerate(jobs)}
    target["animations"].sort(key=lambda action: order[action["name"]])
    write(target, buffer, destination)
    return True
