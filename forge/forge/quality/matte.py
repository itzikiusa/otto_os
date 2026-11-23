"""Keep an artist's alpha matte through Pixal3D conditioning, or cut out an opaque input."""
import json
import sys
from pathlib import Path

from PIL import Image
from image_to_3dlab.matte import cut_out, is_matted


def prepare(source, output):
    with Image.open(source) as image:
        rgba = image.convert("RGBA")
        if is_matted(rgba):
            cut, method = rgba, "input_alpha"
        else:
            cut, method = cut_out(image)
        cut.save(output)
    output.with_suffix(".json").write_text(json.dumps({"method": method}))


if __name__ == "__main__":
    prepare(Path(sys.argv[1]), Path(sys.argv[2]))
