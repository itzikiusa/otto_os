#!/usr/bin/env python3
"""Turn raw generated images + decal SVGs into the build's texture sources.

The committed files in `textures/` ARE the sources the Blender build reads; this
script documents (and can repeat) how they were made.  Needs Pillow and
`rsvg-convert` (brew install librsvg).

    python3 ui/assets-src/school/prep_textures.py <raw-dir>

`<raw-dir>` holds the Codex image-generation outputs, one `<name>.png` per
prompt in `TEXTURE_PROMPTS.md`.  Without it only the decals and placeholders
are regenerated.
"""

import os
import subprocess
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageOps, ImageStat

HERE = os.path.dirname(os.path.abspath(__file__))
TEX = os.path.join(HERE, "textures")
DEC = os.path.join(HERE, "decals")

# name → (max edge px, format)
RAW = {
    "poster_1": (1024, "jpg"), "poster_2": (1024, "jpg"), "poster_3": (1024, "jpg"),
    "poster_4": (1024, "jpg"), "poster_5": (1024, "jpg"), "poster_6": (1024, "jpg"),
    "chalk_art": (1024, "jpg"), "board_blank": (1024, "jpg"),
    "floor_tiles": (1024, "jpg"), "corridor_floor": (1024, "jpg"),
    "wall_paint": (512, "jpg"), "lockers": (1024, "jpg"),
    "window_view": (1024, "jpg"), "pc_sleep": (512, "jpg"),
}


def fit(im, edge):
    w, h = im.size
    s = min(1.0, edge / max(w, h))
    return im.resize((max(1, round(w * s)), max(1, round(h * s))), Image.LANCZOS)


def save(im, name, fmt):
    path = os.path.join(TEX, f"{name}.{fmt}")
    if fmt == "jpg":
        im.convert("RGB").save(path, quality=88, optimize=True, progressive=False)
    else:
        im.save(path, optimize=True)
    print("wrote", os.path.relpath(path, HERE), im.size)


def from_raw(raw):
    for name, (edge, fmt) in RAW.items():
        src = os.path.join(raw, f"{name}.png")
        if not os.path.exists(src):
            print("skip (missing raw)", name)
            continue
        im = Image.open(src).convert("RGB")
        if name == "floor_tiles":
            # The generator returned 1254² for a 4×4 checker: crop to an exact
            # multiple of the tile pitch so the repeat is seamless.
            im = im.crop((0, 0, min(im.size), min(im.size)))
        save(fit(im, edge), name, fmt)
        if name == "wall_paint":
            # Neutral plaster: same grain, no hue, mean ≈ 0.94 → tinted per use.
            g = ImageOps.grayscale(im)
            mean = ImageStat.Stat(g).mean[0]
            g = g.point(lambda v: max(0, min(255, round(240 + (v - mean) * 1.6))))
            save(fit(g.convert("RGB"), edge), "plaster", fmt)


def decals():
    for name in ("claude", "codex", "grok", "agy", "shell"):
        svg = os.path.join(DEC, f"{name}.svg")
        out = os.path.join(TEX, f"decal_{name}.png")
        subprocess.run(["rsvg-convert", "-w", "256", "-h", "256", svg, "-o", out],
                       check=True)
        print("wrote", os.path.relpath(out, HERE))


def placeholders():
    """Default maps for the planes the runtime repaints, so each material is
    created with a `map` slot (three.js) and looks sane before the first paint.

    Those planes carry glTF TEXCOORD_0 with (0,0) at the bottom-left as seen
    (the contract), i.e. the orientation three.js expects for a texture with
    `flipY = true` (its CanvasTexture default).  GLTFLoader loads embedded
    images with `flipY = false`, so the embedded defaults are stored upside
    down (`ph_*` files) to look upright until the runtime repaints them."""
    im = Image.open(os.path.join(TEX, "pc_sleep.jpg")).convert("RGB")
    save(ImageOps.flip(im), "ph_screen", "jpg")
    # BoardTitle: blank chalkboard green with a faint chalk underline.
    im = Image.new("RGB", (512, 100), (46, 92, 64))
    d = ImageDraw.Draw(im)
    d.line((40, 82, 472, 82), fill=(196, 214, 200), width=3)
    save(ImageOps.flip(im.filter(ImageFilter.GaussianBlur(0.6))), "ph_board_title", "png")
    # NamePlate: brushed light plate with a darker border.
    im = Image.new("RGB", (512, 128), (238, 240, 244))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((4, 4, 507, 123), radius=14, outline=(70, 96, 140), width=6)
    save(ImageOps.flip(im), "ph_name_plate", "png")
    # DoorSign: white card with a coloured header band.
    im = Image.new("RGB", (256, 180), (250, 250, 247))
    d = ImageDraw.Draw(im)
    d.rectangle((0, 0, 255, 40), fill=(80, 140, 220))
    d.rounded_rectangle((2, 2, 253, 177), radius=10, outline=(60, 70, 90), width=4)
    save(ImageOps.flip(im), "ph_door_sign", "png")


if __name__ == "__main__":
    os.makedirs(TEX, exist_ok=True)
    if len(sys.argv) > 1:
        from_raw(sys.argv[1])
    decals()
    placeholders()
