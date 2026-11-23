"""Generate the owner's example motions on the real characters and judge them (writes out/motion-evidence/).

python3 tests/motion/evidence.py [--cpu] [ID ...]   (each step takes the shared heavy lock itself)
Per clip: <id>.glb + manifest (forge-motion, with the forge-fit check summary), <id>_strip.png (front
and side), <id>_strip.json (hand-to-prop-anchor distances where a forge-fit reach reference exists).
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "out/motion-evidence"
KNIGHT = ROOT / "out/text_to_asset/character/rigged.glb"
WARRIOR = ROOT / "out/archer/warrior/rigged-handfit.glb"
FIT = ROOT / "out/fit-evidence"
BLENDER = "/Applications/Blender.app/Contents/MacOS/Blender"
SWORD = dict(ref=FIT / "knight/pull_sword.glb", ref_clip="pull_sword", anchor="sword.grip")
QUIVER = dict(ref=FIT / "archer/shoot_bow_procedural.glb", ref_clip="shoot_bow", anchor="quiver.opening",
              anchor_bone="UpperChest")
CLIPS = {
    "pull_sword": ("a man grabs the hilt of a sword stuck upright in the ground in front of him with both hands "
                   "and pulls it up and out", KNIGHT, 1.8, 4, SWORD),
    "draw_bow": ("a woman draws an arrow from a quiver on her back, nocks it, draws a bow and shoots",
                 WARRIOR, 1.7, 6, QUIVER),
    "sword_arc": ("swings a sword in a wide arc", KNIGHT, 1.8, 3, None),
    "stumble_fall": ("stumbles backward and falls", KNIGHT, 1.8, 4, None),
    "jump_forward": ("jumps forward and lands", KNIGHT, 1.8, 3, None),
    "walk_tired": ("walks tired and slow", WARRIOR, 1.7, 5, None),
    "idle": ("stands idle, breathing and shifting weight", WARRIOR, 1.7, 4, None),
}


def main(ids, device="mps", seed=1):
    OUT.mkdir(parents=True, exist_ok=True)
    for key in ids:
        prompt, character, height, seconds, ref = CLIPS[key]
        stem = key + ("_cpu" if device == "cpu" else "")  # a CPU run sits next to the MPS one
        glb = OUT / f"{stem}.glb"
        if not glb.exists():
            r = subprocess.run([ROOT / "bin/forge-motion", "--prompt", prompt, "--character", character, "--seconds",
                                str(seconds), "--seed", str(seed), "--name", key, "--device", device, "--height",
                                str(height), "--output", glb], capture_output=True, text=True)
            print(r.stderr.strip().splitlines()[-1], flush=True)
            if r.returncode:
                continue
        extra = []
        if ref:
            extra = ["--ref", ref["ref"], "--ref-clip", ref["ref_clip"], "--anchor", ref["anchor"]]
            extra += ["--anchor-bone", ref["anchor_bone"]] if "anchor_bone" in ref else []
        r = subprocess.run([ROOT / "tools/heavy.sh", BLENDER, "-b", "--factory-startup", "--python-exit-code", "1",
                            "-P", ROOT / "tests/motion/strip.py", "--", "--glb", glb, "--clip", key, "--height",
                            str(height), "--out", OUT / f"{stem}_strip.png", "--report", OUT / f"{stem}_strip.json",
                            *map(str, extra)], capture_output=True, text=True)
        line = next((x for x in r.stdout.splitlines() if x.startswith("FORGE_STRIP")), r.stdout[-800:])
        print(key, line, flush=True)


if __name__ == "__main__":
    args = sys.argv[1:]
    device = "cpu" if "--cpu" in args else "mps"
    main([a for a in args if not a.startswith("--")] or list(CLIPS), device)
