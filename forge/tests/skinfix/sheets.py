"""Build matched side-by-side comparisons from experiment.py PNGs."""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

work = Path(__file__).resolve().parent / "work"
font = ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial.ttf", 19)
for clip in ("walk", "chop"):
    for view in ("oblique", "side"):
        width, height = 315, 400
        sheet = Image.new("RGB", (3 * width, 3 * (height + 30) + 34), (28, 32, 39))
        draw = ImageDraw.Draw(sheet)
        draw.text(
            (8, 6),
            f"Knight   /   {clip}   /   {view}   /   30 fps",
            font=font,
            fill="white",
        )
        for row, frame in enumerate((3, 14, 26)):
            for col, strength in enumerate(("0", "0.5", "1")):
                x, y = col * width, 34 + row * (height + 30)
                draw.text(
                    (x + 8, y + 3),
                    f"Frame {frame}   |   stiffness {strength}",
                    font=font,
                    fill="white",
                )
                with Image.open(
                    work / f"{clip}_{frame:03}_{view}_{strength}.png"
                ) as im:
                    sheet.paste(
                        im.convert("RGB").resize(
                            (width, height), Image.Resampling.LANCZOS
                        ),
                        (x, y + 30),
                    )
        sheet.save(work / f"{clip}_{view}.jpg", quality=94)
