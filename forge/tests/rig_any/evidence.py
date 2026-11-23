"""Evidence for one test body: generate (if needed), rig, animate, forge-fit check, and a strip per clip.

    python3 tests/rig_any/evidence.py NAME [--prompt TEXT | --mesh MODEL.glb] [--height-m H | --longest-side-m L]

Everything lands in out/rig-any-evidence/NAME/: model.glb, rigged.glb, animated.glb (+ manifests), fit.json,
<clip>_strip.png and summary.json. Every model run goes through bin/forge (it takes the shared heavy slot);
the strips run Blender under tools/heavy.sh, one at a time.
"""

import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "out/rig-any-evidence"
BLENDER = "/Applications/Blender.app/Contents/MacOS/Blender"


def run(argv, log):
    started = time.time()
    with open(log, "w") as f:
        p = subprocess.run([str(a) for a in argv], stdout=f, stderr=subprocess.STDOUT)
    lines = Path(log).read_text(errors="replace").strip().splitlines()
    return p.returncode, round(time.time() - started, 1), (lines[-1] if lines else "")


def main():
    a = argparse.ArgumentParser()
    a.add_argument("name")
    a.add_argument("--prompt")
    a.add_argument("--mesh")
    a.add_argument("--height-m")
    a.add_argument("--longest-side-m")
    a.add_argument("--poly-budget", default="8000")
    a.add_argument("--reuse-rig", action="store_true", help="keep an existing rigged.glb (re-animate only)")
    args = a.parse_args()
    d = OUT / args.name
    d.mkdir(parents=True, exist_ok=True)
    old = d / "summary.json"
    summary = {"name": args.name, "steps": {k: v for k, v in json.loads(old.read_text())["steps"].items()
                                             if k in ("generate", "rig")} if args.reuse_rig and old.exists() else {}}
    for p in d.glob("*_strip.png"):
        p.unlink()
    model = d / "model.glb"
    if args.mesh:
        if Path(args.mesh).resolve() != model.resolve():
            shutil.copyfile(args.mesh, model)
    elif not model.exists():
        argv = [ROOT / "bin/forge", "generate", "--prompt", args.prompt, "--poly-budget", args.poly_budget, "--output", model]
        if args.height_m:
            argv += ["--height-m", args.height_m]
        if args.longest_side_m:
            argv += ["--longest-side-m", args.longest_side_m]
        summary["steps"]["generate"] = run(argv, d / "generate.log")
    for stage, src, dst in (("rig", model, d / "rigged.glb"), ("animate", d / "rigged.glb", d / "animated.glb")):
        if stage == "rig" and args.reuse_rig and dst.exists():
            continue
        for p in (dst, Path(f"{dst}.manifest.json")):
            p.unlink(missing_ok=True)
        summary["steps"][stage] = run([ROOT / "bin/forge", stage, "--mesh", src, "--output", dst], d / f"{stage}.log")
        if summary["steps"][stage][0]:
            break
    animated = d / "animated.glb"
    if animated.exists():
        for p in (d / "fit.json", d / "fit.json.manifest.json"):
            p.unlink(missing_ok=True)
        summary["steps"]["fit"] = run([ROOT / "bin/forge-fit", "check", "--mesh", animated, "--output", d / "fit.json"],
                                      d / "fit.log")
        manifest = json.loads(Path(f"{animated}.manifest.json").read_text())
        motion = manifest.get("motion", {})
        forward = ",".join(str(v) for v in motion.get("forward", [0, 0, 1]))
        for clip in [c["name"] for c in manifest.get("clips", [])]:
            speed = motion.get("clips", {}).get(clip, {}).get("speed_m_s", 0.0)
            out = d / f"{clip}_strip.png"
            summary["steps"][f"strip_{clip}"] = run(
                [ROOT / "tools/heavy.sh", BLENDER, "-b", "--factory-startup", "--python-exit-code", "1", "-P",
                 ROOT / "tests/rig_any/strip.py", "--", "--glb", animated, "--clip", clip, "--out", out,
                 f"--forward={forward}", f"--speed={speed}"], d / f"strip_{clip}.log")
    (d / "summary.json").write_text(json.dumps(summary, indent=1) + "\n")
    print(json.dumps(summary))
    return 0 if all(v[0] == 0 for v in summary["steps"].values()) else 1


if __name__ == "__main__":
    sys.exit(main())
