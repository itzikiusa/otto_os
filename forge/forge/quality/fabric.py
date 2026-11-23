"""Finish a requested patchwork textile with a continuous stitched pattern across its folds."""
import io
import re

from .package import local_matrix

COLOURS = {
    "red": "#ae5e54", "rust": "#ac674e", "orange": "#c68051", "gold": "#ceaa6a",
    "yellow": "#d5b96d", "green": "#719582", "blue": "#638aa5", "navy": "#4b6480",
    "purple": "#897597", "pink": "#c58b96", "cream": "#e3d3ae", "white": "#e9e3d5",
    "black": "#35343a", "grey": "#979397", "gray": "#979397", "brown": "#8e7059",
}


def requested(prompt):
    words = set(re.findall(r"[a-z]+", (prompt or "").lower()))
    return ("patchwork" in words and bool(words & {"quilt", "bedspread", "blanket", "throw"})
            and not words & {"wearing", "clothed", "dressed", "jacket", "coat"})


def laid_flat(prompt):
    text = (prompt or "").lower()
    subject = re.split(r"\b(?:with|holding|wearing|beside|beneath|under)\b", text, maxsplit=1)[0]
    if not re.search(r"\b(quilt|bedspread|blanket|rug|carpet|tablecloth)\b", subject):
        return False
    words = set(re.findall(r"[a-z]+", text))
    return (bool(words & {"flat", "outspread", "unfolded", "spread", "draped"})
            and not words & {"rolled", "hanging", "wrapped", "wearing", "worn", "bundled"})


def align(glb, doc):
    """Measure a spread textile across its broad face before sizing it to its support."""
    import numpy as np
    from scipy.spatial import QhullError
    from trimesh.bounds import oriented_bounds
    from . import package

    if doc.get("skins") or "fabric_frame" in doc["asset"].get("extras", {}):
        return None
    mesh = package.display_surface(glb)
    if mesh is None or len(mesh.vertices) < 4:
        return None
    try:
        frame, spans = oriented_bounds(mesh, angle_digits=1)
    except (QhullError, ValueError):
        return None
    if spans[0] > .6 * spans[1] or spans[1] <= 1.e-8:
        return None
    # The shortest box axis is thickness, the longest runs along the hem.
    up, forward = frame[:3, :3][[0, 2]]
    if up[1] < 0:
        up = -up
    if forward[2] < 0:
        forward = -forward
    rotation = np.array([np.cross(up, forward), up, forward])
    if np.trace(rotation) > 1 + 2 * np.cos(np.deg2rad(2)):
        return None
    matrix = np.eye(4)
    matrix[:3, :3] = rotation
    package.rotate_roots(doc, matrix, "Textile frame")
    report = {"rotation": rotation.tolist()}
    doc["asset"].setdefault("extras", {})["fabric_frame"] = report
    return report


def tile(prompt, seed):
    import numpy as np
    from PIL import Image, ImageColor
    rng = np.random.default_rng(seed)
    words = set(re.findall(r"[a-z]+", prompt.lower()))
    colours = [v for k, v in COLOURS.items() if k in words]
    colours = colours or [COLOURS[x] for x in ("blue", "cream", "rust", "green", "gold")]
    colours = np.asarray([ImageColor.getrgb(c) for c in colours], dtype=float)
    y, x = np.indices((256, 256))
    row, col = y // 64, x // 64
    swatches = (row + col + row // 2 + col // 2) % len(colours)
    cloth = colours[swatches].copy()
    cloth *= (0.91 + 0.10 * ((row + col) % 3))[..., None]
    cloth += (1.8 * np.sin(x * 2.1) + 2.1 * np.cos(y * 2.5) + rng.normal(0, 2, x.shape))[..., None]
    stripe = ((row + col) % 3 == 1) & ((x + y) % 12 < 2)
    cloth[stripe] = cloth[stripe] * 0.86 + 19
    edge = (x % 64 < 2) | (y % 64 < 2)
    cloth[edge] *= 0.62
    stitch = ((x % 64 == 3) & (y % 8 < 4)) | ((y % 64 == 3) & (x % 8 < 4))
    cloth[stitch] = cloth[stitch] * 0.42 + 127
    image = io.BytesIO()
    Image.fromarray(np.uint8(np.clip(cloth, 0, 255))).save(image, format="PNG")
    return image.getvalue()


def apply(doc, binary, prompt, seed):
    """Project in the cloth's own principal plane. Keep vertices, rig data and other UV sets."""
    import numpy as np
    if doc.get("skins"):
        return None
    visible = {}
    def visit(index, parent):
        node = doc["nodes"][index]
        frame = parent @ np.asarray(local_matrix(node))
        if "mesh" in node and not node.get("name", "").startswith("UCX_"):
            visible.setdefault(node["mesh"], frame)
        for child in node.get("children", []):
            visit(child, frame)
    for i in doc["scenes"][doc.get("scene", 0)]["nodes"]:
        visit(i, np.eye(4))
    pieces = []
    for mesh, frame in visible.items():
        for prim in doc["meshes"][mesh]["primitives"]:
            attr = prim["attributes"]
            a = doc["accessors"][attr["POSITION"]]
            if a.get("componentType") != 5126 or "bufferView" not in a or "sparse" in a:
                return None
            view = doc["bufferViews"][a["bufferView"]]
            pos = np.ndarray((a["count"], 3), "<f4", binary, offset=view.get("byteOffset", 0) + a.get("byteOffset", 0),
                             strides=(view.get("byteStride", 12), 4))
            pieces.append((prim, pos @ frame[:3,:3].T + frame[:3,3]))
    if not pieces:
        return None
    points = np.concatenate([pos for _, pos in pieces])
    if len(points) < 3:
        return None
    centre = points.mean(axis=0)
    _, axes = np.linalg.eigh(np.cov((points-centre).T))
    uv = (points-centre) @ axes[:, 1:]
    size = np.ptp(uv, axis=0)
    if min(size) <= 1e-8:
        return None
    channel = next((i for i in range(4) if all("TEXCOORD_" + str(i) not in prim["attributes"] for prim, _ in pieces)), None)
    if channel is None:
        return None
    data = bytearray(binary)
    def append(field, value):
        items = doc.setdefault(field, [])
        items.append(value)
        return len(items) - 1
    def blob(payload):
        data.extend(bytes(-len(data) % 4))
        offset = len(data)
        data.extend(payload)
        return append("bufferViews", {"buffer": 0, "byteOffset": offset, "byteLength": len(payload)})
    image = append("images", {"bufferView": blob(tile(prompt, seed)), "mimeType": "image/png"})
    texture = append("textures", {"source": image})
    material = append("materials", {"name": "Stitched cotton", "doubleSided": True, "pbrMetallicRoughness": {
        "baseColorTexture": {"index": texture, "texCoord": channel}, "metallicFactor": 0, "roughnessFactor": 0.96}})
    lo = uv.min(axis=0)
    for prim, pos in pieces:
        coords = (((pos-centre) @ axes[:, 1:] - lo) / size).astype("<f4")
        accessor = append("accessors", {"bufferView": blob(coords.tobytes()), "componentType": 5126,
                                         "count": len(coords), "type": "VEC2"})
        prim["attributes"]["TEXCOORD_" + str(channel)] = accessor
        prim["attributes"].pop("COLOR_0", None)
        prim["material"] = material
    data.extend(bytes(-len(data) % 4))
    doc["buffers"][0]["byteLength"] = len(data)
    return data
