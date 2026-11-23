"""Create calibrated 4x views with Real-ESRGAN RGB and bicubic alpha, for AssetFurnace Pixel Match."""
import json
import shutil
import sys
from pathlib import Path

import numpy as np
from PIL import Image
from image_to_3dlab import upscale


def upscale_views(source, destination):
    camera = source / "transforms.json"
    frames = json.loads(camera.read_text())["frames"]
    if not frames:
        raise ValueError(f"no calibrated frames in {camera}")
    destination.mkdir(parents=True, exist_ok=True)
    model = upscale.load()
    sizes = {}
    for frame in frames:
        name = frame["file_path"]
        if name in sizes:
            continue
        path = Path(name)
        if path.is_absolute() or ".." in path.parts:
            raise ValueError(f"expected relative camera image path: {name}")
        with Image.open(source / path) as opened:
            image = opened.convert("RGBA")
        rgb = np.asarray(image.convert("RGB"), dtype=np.float32) / 255
        sharp = upscale.upscale(model, rgb)
        result = Image.fromarray(np.rint(np.clip(sharp, 0, 1) * 255).astype(np.uint8))
        expected = (image.width * 4, image.height * 4)
        if result.size != expected:
            raise ValueError(f"unexpected 4x output size for {name}: {result.size}")
        result.putalpha(image.getchannel("A").resize(expected, Image.Resampling.BICUBIC))
        (destination / path).parent.mkdir(parents=True, exist_ok=True)
        result.save(destination / path)
        sizes[name] = {"input_px": image.size, "output_px": result.size}
    shutil.copyfile(camera, destination / "transforms.json")
    (destination / "upscale.json").write_text(json.dumps(sizes))


if __name__ == "__main__":
    upscale_views(Path(sys.argv[1]), Path(sys.argv[2]))
