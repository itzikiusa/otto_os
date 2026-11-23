"""Metres in the glTF default scene, excluding collision and lower display LODs."""

import math
import re
import struct

IDENTITY = ((1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1))


def floats(doc, data, accessor):
    import numpy as np
    a = doc["accessors"][accessor]
    width = {"VEC2": 2, "VEC3": 3}.get(a["type"])
    if width is None or a["componentType"] != 5126 or "bufferView" not in a or "sparse" in a:
        return None
    v = doc["bufferViews"][a["bufferView"]]
    return np.ndarray((a["count"], width), "<f4", data,
                      offset=v.get("byteOffset", 0) + a.get("byteOffset", 0),
                      strides=(v.get("byteStride", width * 4), 4))


def display_surface(glb):
    """World-space triangles of the highest-detail display meshes."""
    import trimesh
    scene = trimesh.load(glb, force="scene", process=False)
    parts = []
    for node in scene.graph.nodes_geometry:
        if node.upper().startswith("UCX_") or "_LOD" in node.upper():
            continue
        transform, name = scene.graph[node]
        part = scene.geometry[name].copy()
        part.apply_transform(transform)
        parts.append(part)
    return trimesh.util.concatenate(parts) if parts else None


def rotate_roots(doc, matrix, name):
    """Apply the same measured frame to the display mesh, its LODs and collision."""
    for scene in doc["scenes"]:
        if scene.get("nodes"):
            index = len(doc["nodes"])
            doc["nodes"].append({"name": name, "matrix": matrix.T.flatten().tolist(), "children": scene["nodes"]})
            scene["nodes"] = [index]


def local_matrix(node):
    if "matrix" in node:
        return [node["matrix"][i::4] for i in range(4)]
    x, y, z, w = node.get("rotation", (0, 0, 0, 1))
    basis = [
        (1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)),
        (2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)),
        (2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)),
    ]
    scale = node.get("scale", (1, 1, 1))
    offset = node.get("translation", (0, 0, 0))
    rows = [
        tuple(v * s for v, s in zip(row, scale)) + (t,) for row, t in zip(basis, offset)
    ]
    return rows + [(0, 0, 0, 1)]


def root_static_meshes(doc):
    """Give the joint predictor world-space mesh roots for a static GLB.

    SkinTokens uses the transform relative to each mesh's parent. A measured
    frame on an empty parent would disappear from both joint fitting and export.
    """
    import numpy as np
    if doc.get("skins") or doc.get("animations"):
        return False
    nodes = doc.get("nodes", [])
    flat = []
    scene_roots = []
    inherited = False

    def visit(index, parent, roots):
        nonlocal inherited
        node = nodes[index]
        world = parent @ local_matrix(node)
        if "mesh" in node:
            inherited |= not np.array_equal(parent, IDENTITY)
            roots.append(len(flat))
            mesh = {k: v for k, v in node.items()
                    if k not in ("children", "matrix", "translation", "rotation", "scale")}
            mesh["matrix"] = world.T.ravel().tolist()
            flat.append(mesh)
        for child in node.get("children", []):
            visit(child, world, roots)

    for scene in doc.get("scenes", []):
        roots = []
        for root in scene.get("nodes", []):
            visit(root, np.eye(4), roots)
        scene_roots.append(roots)
    if inherited:
        doc["nodes"] = flat
        for scene, roots in zip(doc["scenes"], scene_roots):
            scene["nodes"] = roots
    return inherited


def axis_bounds(accessor, views, binary, axis):
    x, y, z, offset = axis
    # A local accessor bound is exact for an axis-aligned world direction.
    nonzero = [i for i, value in enumerate((x, y, z)) if value]
    if len(nonzero) == 1 and "min" in accessor and "max" in accessor:
        i = nonzero[0]
        return sorted(accessor[key][i] * axis[i] + offset for key in ("min", "max"))
    if accessor["componentType"] != 5126 or "sparse" in accessor:
        raise ValueError("Unsupported display POSITION encoding for world dimensions")
    view = views[accessor["bufferView"]]
    start = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
    stride = view.get("byteStride", 12)
    low, high = math.inf, -math.inf
    for i in range(accessor["count"]):
        px, py, pz = struct.unpack_from("<3f", binary, start + i * stride)
        value = px * x + py * y + pz * z + offset
        low, high = min(low, value), max(high, value)
    return low, high


def size_m(doc, binary):
    low, high = [math.inf] * 3, [-math.inf] * 3
    nodes = doc.get("nodes", [])

    def visit(index, parent):
        node = nodes[index]
        local = local_matrix(node)
        world = [
            [sum(a * b for a, b in zip(row, col)) for col in zip(*local)]
            for row in parent
        ]
        name = node.get("name", "")
        if "mesh" in node and not (
            name.startswith("UCX_") or re.search(r"_LOD[1-9]\d*$", name)
        ):
            for primitive in doc["meshes"][node["mesh"]]["primitives"]:
                accessor = doc["accessors"][primitive["attributes"]["POSITION"]]
                for i, axis in enumerate(world[:3]):
                    a, b = axis_bounds(accessor, doc["bufferViews"], binary, axis)
                    low[i], high[i] = min(low[i], a), max(high[i], b)
        for child in node.get("children", []):
            visit(child, world)

    for root in doc.get("scenes", [{}])[doc.get("scene", 0)].get("nodes", []):
        visit(root, IDENTITY)
    return (
        [b - a for a, b in zip(low, high)]
        if all(map(math.isfinite, low))
        else [0, 0, 0]
    )


def prepared(doc, receipt):
    extras = doc["asset"].get("extras")
    if isinstance(extras, dict) and extras.get("otto_forge_game_ready") is True:
        return True
    # Pre-marker packages are often renamed and copied out of Otto without their sidecar.
    names = {n.get("name", "") for n in doc.get("nodes", []) if "mesh" in n}
    legacy = {
        "gameready",
        "gameready_LOD1",
        "gameready_LOD2",
        "gameready_LOD3",
        "UCX_gameready_00",
    }
    if legacy <= names:
        return True
    stage = receipt.get("gameready", receipt)
    if (
        not isinstance(stage, dict)
        or stage.get("stage") != "gameready"
        or not stage.get("lods")
    ):
        return False
    meshes = {
        n.get("name"): doc["meshes"][n["mesh"]] for n in doc["nodes"] if "mesh" in n
    }
    for level in stage["lods"]:
        mesh = meshes.get(level.get("node"))
        if not mesh:
            return False
        triangles = sum(
            doc["accessors"][p.get("indices", p["attributes"]["POSITION"])]["count"]
            // 3
            for p in mesh["primitives"]
            if p.get("mode", 4) == 4
        )
        if triangles != level.get("triangles"):
            return False
    return True


def scale_metres(doc, binary, factor):
    """Convert all lengths, including shared geometry, morph deltas and native animation, once."""
    import numpy as np

    fields = {}
    for mesh in doc.get("meshes", []):
        for p in mesh["primitives"]:
            for attributes in [p["attributes"], *p.get("targets", [])]:
                if "POSITION" in attributes:
                    fields[attributes["POSITION"]] = (3, slice(None))
    for node in doc.get("nodes", []):
        if "matrix" in node:
            node["matrix"][12:15] = [v * factor for v in node["matrix"][12:15]]
        if "translation" in node:
            node["translation"] = [v * factor for v in node["translation"]]
    for skin in doc.get("skins", []):
        if "inverseBindMatrices" in skin:
            fields[skin["inverseBindMatrices"]] = (16, slice(12, 15))
    for anim in doc.get("animations", []):
        for ch in anim["channels"]:
            if ch["target"]["path"] == "translation":
                fields[anim["samplers"][ch["sampler"]]["output"]] = (3, slice(None))
    result = bytearray(binary)
    for index, (width, cols) in fields.items():
        acc = doc["accessors"][index]
        if acc["componentType"] != 5126 or "sparse" in acc:
            raise ValueError("Unsupported length accessor in prepared Forge package")
        view = doc["bufferViews"][acc["bufferView"]]
        array = np.ndarray(
            (acc["count"], width),
            dtype="<f4",
            buffer=result,
            offset=view.get("byteOffset", 0) + acc.get("byteOffset", 0),
            strides=(view.get("byteStride", width * 4), 4),
        )
        with np.errstate(over="raise", invalid="raise"):
            array[:, cols] *= factor
        for key in ("min", "max"):
            if key in acc:
                acc[key][cols] = [v * factor for v in acc[key][cols]]
    return result
