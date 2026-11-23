"""Retain a reflection on a peeking face whose two dark irises disappear in shade."""
import numpy as np

from forge import bodyplan
from forge.quality import eyes


def glossy_pair(p, faces, shiny):
    """Find two round, matte-separated patches on the front surface in the rig's facing frame."""
    import cv2
    triangles = p[faces]
    visible = (triangles[:, :, 1].max(1) > .48) & (triangles[:, :, 2].max(1) > .65)
    if not (shiny & visible).any():
        return None
    grid = 160
    depth = triangles[:, :, 2].mean(1)
    xy = np.rint(triangles[:, :, :2] * (grid - 1)).astype(np.int32)
    # Draw every front triangle in depth order. A glossy part on the far side
    # or a patch behind the eyelid must not show through the foreground.
    image = np.zeros((grid, grid), np.uint8)
    zbuf = np.zeros_like(image)
    for i in np.flatnonzero(visible)[np.argsort(depth[visible])]:
        cv2.fillConvexPoly(image, xy[i], 255 if shiny[i] else 0)
        cv2.fillConvexPoly(zbuf, xy[i], int(255 * depth[i]))
    image[:int(.48 * grid)] = 0
    image[zbuf < 184] = 0
    smooth = cv2.morphologyEx(image, cv2.MORPH_OPEN, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (3, 3)))
    count, labels, stats, centers = cv2.connectedComponentsWithStats(smooth)
    pair = []
    for i in range(1, count):
        _, _, width, height, area = stats[i]
        x, y = centers[i] / (grid - 1)
        if (6 <= width <= .28 * grid and 5 <= height <= .23 * grid
                and .44 < width / height < 2.4 and area >= .38 * width * height):
            pair.append((x, y, width / grid, height / grid, np.median(zbuf[labels == i]) / 255))
    if len(pair) != 2:
        return None
    a, b = sorted(pair)
    if (a[0] < .48 and b[0] > .52 and .12 < b[0] - a[0] < .58
            and abs(a[1] - b[1]) < .08 and abs(a[4] - b[4]) < .10
            and .5 < a[2] / b[2] < 2 and .5 < a[3] / b[3] < 2):
        return a, b
    return None


def _atlas(p, faces, uv, base, roughness):
    from PIL import Image, ImageDraw
    centers = p[faces].mean(axis=1)
    # Sample within each UV triangle. Border vertices can land in the empty
    # gutter of a packed texture atlas.
    uvfaces = uv[faces]
    sample = np.array([[1/3, 1/3, 1/3], [.6, .2, .2], [.2, .6, .2], [.2, .2, .6]]) @ uvfaces
    def values(tex):
        xy = ((sample % 1) * tex.shape[1::-1]).astype(int)
        return np.median(tex[xy[:, :, 1], xy[:, :, 0]], axis=1)

    luminance = values(base).mean(1)
    front = (centers[:, 1] > .48) & (centers[:, 2] > .65)
    if not front.any() or np.median(luminance[front]) > 82:
        return None
    shiny = (values(roughness) < 80) & (luminance < 90)
    pair = glossy_pair(p, faces, shiny)
    if pair is None:
        return None
    h, w = roughness.shape
    mask = Image.new("L", (w, h))
    draw = ImageDraw.Draw(mask)
    for x, y, width, height, depth in pair:
        rx = (centers[:, 0] - x) / (.5 * width)
        ry = (centers[:, 1] - y) / (.5 * height)
        near = (abs(rx) < 1.4) & (abs(ry) < 1.4) & (abs(centers[:, 2] - depth) < .075)
        # A broad, softly edged catchlight on the upper side keeps the dark pupil.
        reflection = np.exp(-.5 * (((rx + .20) / .80) ** 2 + ((ry - .19) / .67) ** 2))
        for i in np.flatnonzero(near):
            draw.polygon([tuple(pt) for pt in uvfaces[i] * [w, h]], fill=round(255 * reflection[i]))
    strength = np.asarray(mask) / 255 * np.clip((168. - roughness) / 110, 0, 1)
    return Image.fromarray((strength[:, :, None] * [254, 238, 206]).astype(np.uint8))


def apply(glb, doc, binary):
    if len(doc.get("skins", [])) != 1:
        return None
    rig = bodyplan.Rig(glb)
    forward = np.asarray(bodyplan.detect(rig)["forward"])
    up = bodyplan.UP
    p = rig.skin_rest @ np.column_stack([np.cross(up, forward), up, forward])
    span = np.ptp(p, axis=0)
    if min(span) < 1e-6:
        return None
    p = (p - p.min(0)) / span
    start = 0
    for node in doc["nodes"]:
        if node.get("skin") != 0 or "mesh" not in node:
            continue
        for prim in doc["meshes"][node["mesh"]]["primitives"]:
            attr = prim["attributes"]
            if "WEIGHTS_0" not in attr or "JOINTS_0" not in attr:
                continue
            count = doc["accessors"][attr["POSITION"]]["count"]
            region = p[start:start + count]
            start += count
            if prim.get("mode", 4) != 4 or "material" not in prim:
                continue
            material = doc["materials"][prim["material"]]
            pbr = material.get("pbrMetallicRoughness", {})
            base, orm = pbr.get("baseColorTexture"), pbr.get("metallicRoughnessTexture")
            if (not base or not orm or "emissiveTexture" in material
                    or any(material.get("emissiveFactor", ()))
                    or "KHR_materials_unlit" in material.get("extensions", {})):
                continue
            channel = orm.get("texCoord", 0)
            uv_key = f"TEXCOORD_{channel}"
            if base.get("texCoord", 0) != channel or uv_key not in attr or base.get("extensions") or orm.get("extensions"):
                continue
            color, packed = (eyes.texture_image(doc, binary, t) for t in (base, orm))
            if color is None or packed is None:
                continue
            indices = (bodyplan.accessor(doc, binary, prim["indices"]).astype(int).ravel()
                       if "indices" in prim else np.arange(count))
            uv = bodyplan.accessor(doc, binary, attr[uv_key])
            if np.any(uv < 0) or np.any(uv > 1):
                continue
            atlas = _atlas(region, indices.reshape(-1, 3), uv, color, packed[:, :, 1])
            if atlas is not None:
                data = bytearray(binary)
                eyes.attach_atlas(doc, data, material, atlas, channel, [1, 1, 1])
                ext = "KHR_materials_emissive_strength"
                material.setdefault("extensions", {})[ext] = {"emissiveStrength": 2.5}
                if ext not in doc.setdefault("extensionsUsed", []):
                    doc["extensionsUsed"].append(ext)
                return data
    return None
