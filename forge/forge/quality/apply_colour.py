"""Move the painted LOD0 PNG into the shared base-colour slot of the full game-ready GLB."""
import sys
from pathlib import Path

from forge.glbstats import read_glb
from image_to_3dlab import photo_paint


def apply_colour(main, painted, output):
    doc, binary = read_glb(painted)
    image = doc["images"][photo_paint.base_colour_image_index(doc)]
    view = doc["bufferViews"][image["bufferView"]]
    offset = view.get("byteOffset", 0)
    png = binary[offset:offset + view["byteLength"]]
    if image.get("mimeType") != "image/png" or not png.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("Pixel Match did not produce an embedded base-colour PNG")
    output.write_bytes(photo_paint.replace_base_colour(main.read_bytes(), png))


if __name__ == "__main__":
    apply_colour(*map(Path, sys.argv[1:]))
