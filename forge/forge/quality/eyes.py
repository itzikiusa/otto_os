"""Finish visibly pale eyes on a dark, shadow-dwelling character from its existing iris texture."""
import io
import re

from forge.glbstats import texture_source
from forge.quality.package import floats


def requested(prompt, *, peeking=False):
    text = (prompt or "").lower()
    if re.search(r"\b(?:no|without)(?: any)? (?:visible|bright|pale|ivory|luminous|glowing|glinting) (?:eyes|irises)\b", text):
        return False
    # The model prompt describes the face. The peek pose supplies the
    # context when the planner leaves the lighting in the scene.
    if re.search(r"\b(?:eyes|irises) (?:are |remain )?(?:hidden|concealed|unlit)\b", text):
        return False
    if peeking and re.search(r"\b(?:pale|ivory|bright) irises\b|"
                             r"\beyes (?:that |which )?(?:stand out|catch the light|glisten|glint)\b", text):
        return True
    words = set(re.findall(r"[a-z]+", text))
    return ("eyes" in words and bool(words & {"shadow", "shadows", "darkness", "nocturnal"})
            and bool(words & {"bright", "pale", "ivory", "luminous", "glowing", "glinting"}))


def paired_irises(p, lum):
    import numpy as np
    # On a compact legless face the eyes can sit low over the floor. In that
    # case only search the foremost part of the head, clear of paws or a belly.
    for region in ((p[:, 1] > .60) & (p[:, 2] > .53),
                   (p[:, 1] > .20) & (p[:, 2] > .85)):
        bright = region & (lum > 105)
        halves = [p[bright & (p[:, 0] < .49)], p[bright & (p[:, 0] > .51)]]
        if any(len(side) < 5 for side in halves):
            continue
        a, b = (np.median(side, axis=0) for side in halves)
        if .07 < b[0] - a[0] < .55 and abs(a[1] - b[1]) < .08 and abs(a[2] - b[2]) < .10:
            return a, b
    return None


def texture_image(doc, binary, info):
    import numpy as np
    from PIL import Image
    index = texture_source(doc["textures"][info["index"]])
    if index is None or "bufferView" not in doc["images"][index]:
        return None
    view = doc["bufferViews"][doc["images"][index]["bufferView"]]
    start = view.get("byteOffset", 0)
    try:
        with Image.open(io.BytesIO(binary[start:start + view["byteLength"]])) as image:
            return np.asarray(image.convert("RGB"))
    except OSError:
        # Basis-compressed images still render without this optional finish.
        return None


def attach_atlas(doc, data, material, atlas, texcoord, factor):
    buf = io.BytesIO()
    atlas.save(buf, format="PNG")
    blob = buf.getvalue()
    data.extend(bytes(-len(data) % 4))
    view = len(doc["bufferViews"])
    doc["bufferViews"].append({"buffer": 0, "byteOffset": len(data), "byteLength": len(blob)})
    data.extend(blob)
    image = len(doc["images"])
    doc["images"].append({"bufferView": view, "mimeType": "image/png"})
    texture = len(doc["textures"])
    doc["textures"].append({"source": image})
    material["emissiveTexture"] = {"index": texture, "texCoord": texcoord}
    material["emissiveFactor"] = factor
    data.extend(bytes(-len(data) % 4))
    doc["buffers"][0]["byteLength"] = len(data)


def apply(doc, binary):
    """Keep the pupils from the albedo and limit the reflected light to two verified eye patches."""
    import numpy as np
    from PIL import Image, ImageDraw
    data = bytearray(binary)
    changed = False
    for index, material in enumerate(doc.get("materials", [])):
        base = material.get("pbrMetallicRoughness", {}).get("baseColorTexture")
        if not base or "emissiveTexture" in material:
            continue
        pixels = texture_image(doc, binary, base)
        if pixels is None:
            continue
        h, w = pixels.shape[:2]
        mask = Image.new("L", (w, h))
        pen = ImageDraw.Draw(mask)
        eyes = False
        for mesh in doc["meshes"]:
            for prim in mesh["primitives"]:
                attr = prim["attributes"]
                uv_attr = "TEXCOORD_" + str(base.get("texCoord", 0))
                if prim.get("material") != index or uv_attr not in attr:
                    continue
                pos, uv = floats(doc, binary, attr["POSITION"]), floats(doc, binary, attr[uv_attr])
                if pos is None or uv is None or len(pos) < 40:
                    continue
                span = np.ptp(pos, axis=0)
                if min(span) < 1e-6:
                    continue
                p = (pos - pos.min(axis=0)) / span
                xy = ((uv % 1) * [w, h]).astype(int)
                colors = pixels[xy[:, 1], xy[:, 0]]
                lum = colors.mean(axis=1)
                if np.median(lum) > 68:
                    continue
                # Two pale regions on the front of a dark head, aligned in height and depth.
                centers = paired_irises(p, lum)
                if centers is None:
                    continue
                radius = max(2, round(min(w, h) / 150))
                for center in centers:
                    patch = np.all(np.abs(p - center) < [.11, .08, .10], axis=1)
                    for x, y in xy[patch]:
                        pen.ellipse((int(x)-radius, int(y)-radius, int(x)+radius, int(y)+radius),fill=255)
                eyes = True
        if not eyes:
            continue
        intensity = np.clip((pixels.mean(axis=2) - 52) / 100, 0, 1) * np.asarray(mask) / 255.
        atlas = Image.fromarray((pixels * intensity[..., None]).astype("uint8"))
        attach_atlas(doc, data, material, atlas, base.get("texCoord", 0), [.68, .66, .60])
        changed = True
    return data if changed else None
