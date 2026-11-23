"""forge-fit: hold props, close hands, author reaches, and measure whether motion fits.

Every Blender run takes a shared heavy slot (tools/heavy.sh). Progress is JSON Lines on stderr; a failure
ends with {"error": ...} and a non-zero exit; SIGTERM/SIGINT kill the Blender process group.
"""

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT))
BLENDER = os.environ.get("BLENDER", "/Applications/Blender.app/Contents/MacOS/Blender")
FLOOR = 60 * 10**9
TEMPLATES = HERE / "templates"


def progress(pct, message):
    print(json.dumps({"stage": "fit", "pct": round(pct), "message": message}), file=sys.stderr, flush=True)


def space(directory):
    if shutil.disk_usage(directory).free < FLOOR:
        raise OSError("Stopped: less than 60 GB free on the working volume")


def blender(job, work):
    """Run the worker under the shared heavy lock; relay its progress; return (result, blender version)."""
    job_file = work / "job.json"
    job_file.write_text(json.dumps(job))
    log_path = work / "blender.log"
    argv = [BLENDER, "-b", "--factory-startup", "--python-exit-code", "1", "-P", str(HERE / "worker.py"), "--", str(job_file)]
    if not os.environ.get("FORGE_HEAVY_HELD"):  # a caller that already holds a heavy slot (the test suite) sets it
        argv.insert(0, str(ROOT / "tools/heavy.sh"))
    with log_path.open("w") as log:
        proc = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                start_new_session=True)
        try:
            for line in proc.stdout:
                log.write(line)
                if line.startswith("FIT_PROGRESS "):
                    p = json.loads(line[len("FIT_PROGRESS "):])
                    progress(5 + 0.9 * p["pct"], p["message"])
            proc.wait()
        except BaseException:
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except OSError:
                pass
            proc.wait()
            raise
    text = log_path.read_text(errors="replace")
    if proc.returncode:
        lines = text.splitlines()
        detail = next((x.strip() for x in reversed(lines) if x.startswith(("ValueError", "RuntimeError", "OSError", "KeyError", "FileNotFoundError")) or "Error:" in x and "script failed" not in x), "")
        raise RuntimeError(detail or "Blender failed: " + " ".join(lines[-4:])[-600:])
    version = next((ln.partition(" (")[0].strip() for ln in text.splitlines()
                    if ln.startswith("Blender ") and ln[8:9].isdigit()), "Blender")
    return json.loads((work / "result.json").read_text()), version


def prop_licences(paths):
    out = {}
    for p in paths:
        m = Path(str(p) + ".manifest.json")
        if m.is_file():
            try:
                data = json.loads(m.read_text())
                out[Path(p).name] = {k: data[k] for k in ("tools", "licences", "command", "prompt") if k in data}
            except ValueError:
                pass
    return out


def humanoid(mesh):
    """Unity/AssetFurnace humanoids go to the Blender measures; every other body plan to plan_check."""
    from forge import bodyplan
    return bodyplan.humanoid(bodyplan.Rig(mesh))


def run(args):
    started = time.monotonic()
    cmd = args.command
    output = Path(args.output).resolve()
    sidecar = Path(str(output) + ".manifest.json")
    if output.exists() or sidecar.exists():
        raise FileExistsError(f"Choose a fresh output path: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    space(output.parent)
    inputs = {}
    job = {"command": cmd, "height_m": args.height or 1.8, "keep_prop": getattr(args, "keep_prop", False),
           "keep_hands": getattr(args, "keep_hands", False)}
    keep = bool(os.environ.get("FORGE_KEEP_WORK"))
    work = Path(tempfile.mkdtemp(prefix=".fit-", dir=output.parent))
    try:
        result_file = work / ("out" + (output.suffix or ".json"))
        job["work"] = str(work)
        job["command"] = {"prop-intake": "prop_intake"}.get(cmd, cmd)
        if cmd in ("hold", "fist", "hands", "reach"):
            job["character"] = str(Path(args.character).resolve())
            inputs["character"] = job["character"]
        if cmd in ("check", "correct"):
            job["mesh"] = str(Path(args.mesh).resolve())
            inputs["mesh"] = job["mesh"]
        for key in ("character", "mesh"):
            if key in job and not Path(job[key]).is_file():
                raise FileNotFoundError(f"No such file: {job[key]}")
        if cmd == "hold":
            if args.grip != "auto" and args.grip_axis is None:
                raise ValueError("--grip x,y,z needs --grip-axis x,y,z (prop-local metres)")
            anchors = {}
            if args.grip != "auto":
                anchors = {"grip": [float(v) for v in args.grip.split(",")],
                           "grip_axis": [float(v) for v in args.grip_axis.split(",")]}
            job.update({"hand": args.hand, "accept_hands": args.accept_hands, "keep_hands": args.keep_hands,
                        "hand_style": args.hand_style, "output": str(result_file),
                        "hand_source": (args.hand_source if args.hand_source in ("male", "female", "procedural") else str(Path(args.hand_source).resolve())) if args.hand_source else None,
                        "props": [{"path": str(Path(args.prop).resolve()), "name": args.prop_name, "class": args.prop_class,
                                   "length_m": args.prop_length, "anchors": anchors or None}]})
            inputs["prop"] = job["props"][0]["path"]
        elif cmd == "fist":
            job["output"] = str(result_file)
        elif cmd == "hands":
            job["renders"] = str(work / "renders") if args.renders else None
            if args.replace:
                job.update({"replace": "all" if args.replace == "all" else "failing", "hand_style": args.hand_style,
                            "hand_source": (args.hand_source if args.hand_source in ("male", "female", "procedural") else str(Path(args.hand_source).resolve())) if args.hand_source else None,
                            "output": str(result_file)})
        elif cmd == "check":
            job.update({"prop_node": args.prop_node, "sheet": str(work / "sheet.png"),
                        "clips": args.clips.split(",") if args.clips else None})
        elif cmd == "correct":
            job.update({"output": str(result_file), "sheet": str(work / "sheet.png")})
        elif cmd in ("prop", "prop-intake"):
            job.update({"class": args.prop_class, "height_m": args.height_m or args.height, "palm_width_m": args.palm_width_m})
            if cmd == "prop":
                job.update({"output": str(result_file), "like": str(Path(args.like).resolve()) if args.like else None,
                            "like_length_m": args.like_length})
            else:
                job.update({"prop": str(Path(args.prop).resolve()), "length_m": args.prop_length})
        elif cmd == "reach":
            targets = Path(args.targets)
            if not targets.is_file() and (TEMPLATES / (args.targets + ".json")).is_file():
                targets = TEMPLATES / (args.targets + ".json")
            job.update({"hand_style": args.hand_style, "hand_source": (args.hand_source if args.hand_source in ("male", "female", "procedural") else str(Path(args.hand_source).resolve())) if args.hand_source else None})
            job.update({"targets": str(targets.resolve()), "output": str(result_file),
                        "props": dict(p.split("=", 1) for p in (args.prop or []))})
            job["props"] = {k: str(Path(v).resolve()) for k, v in job["props"].items()}
            inputs.update({"targets": job["targets"], "props": job["props"]})
        progress(2, f"Starting forge-fit {cmd}")
        if cmd == "check" and not humanoid(job["mesh"]):
            # any other body: the same foot/ground/loop measures per detected leg, on the skin, no Blender
            import plan_check
            progress(10, "Measuring every clip per detected leg (body plan)")
            result = plan_check.check_glb(job["mesh"], height_m=args.height, clips=job["clips"])
            version = None
        else:
            result, version = blender(job, work)
        space(output.parent)
        files = {}
        if cmd == "prop-intake":
            result_file.write_text(json.dumps(result, indent=1) + chr(10))
        if cmd in ("hands", "check"):
            if cmd == "hands" and args.renders:
                dest = Path(args.renders).resolve()
                dest.mkdir(parents=True, exist_ok=True)
                for p in (work / "renders").glob("*.png"):
                    shutil.copy2(p, dest / p.name)
                files["renders"] = str(dest)
            if cmd == "check" and Path(job["sheet"]).is_file():
                sheet = output.with_suffix(".png")
                shutil.copy2(job["sheet"], sheet)
                files["contact_sheet"] = str(sheet)
            if not (cmd == "hands" and args.replace):  # with --replace the output is the GLB the worker wrote
                result_file.write_text(json.dumps(result, indent=1) + "\n")
        elif cmd == "correct" and Path(job["sheet"]).is_file():
            sheet = output.with_suffix(".png")
            shutil.copy2(job["sheet"], sheet)
            files["contact_sheet"] = str(sheet)
        manifest = {
            "stage": "fit", "command": cmd, "inputs": inputs, "output": str(output), "files": files,
            "tools": ({"blender": version, "blender_gltf_exporter": "Khronos glTF Blender I/O (bundled)"} if version
                      else {"plan_check": "forge/fit/plan_check.py (numpy, no Blender)"}),
            "licences": {"forge-fit": "this repository's licence", "blender": "GPL-3.0 (tool only; outputs unaffected)",
                         "inputs": prop_licences([*(inputs.get("props") or {}).values(), inputs.get("prop"),
                                                  inputs.get("character"), inputs.get("mesh")])},
            "timings": {"blender_sec": result.pop("blender_sec", None), "total_sec": round(time.monotonic() - started, 3)},
            "measurements": result,
        }
        (work / "manifest.json").write_text(json.dumps(manifest, indent=1, default=float) + "\n")
        os.replace(work / "manifest.json", sidecar)
        os.replace(result_file, output)
        progress(100, f"Wrote {output.name}")
        return manifest
    finally:
        if not keep:
            shutil.rmtree(work, ignore_errors=True)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError(message)


def main():
    p = Parser(prog="forge-fit", description=__doc__)
    sub = p.add_subparsers(dest="command", required=True)
    def common(sp, needs):
        sp.add_argument(needs, required=True)
        sp.add_argument("--output", required=True)
        sp.add_argument("--height", type=float, help="character height in metres (default 1.8; a non-humanoid "
                        "body: its glTF metres)")
    s = sub.add_parser("hands", help="hand intake test: does each finger chain curl its own finger")
    common(s, "--character")
    s.add_argument("--renders", help="directory for palm/profile renders (open, half, closed)")
    s.add_argument("--replace", nargs="?", const="failing", choices=["failing", "all"],
                   help="replace the hands that fail intake (or all) with template hands; --output is then a GLB")
    s.add_argument("--hand-style", default="simple", choices=["simple", "gloved", "chunky"])
    s.add_argument("--hand-source", help="male|female (library wrist-cut hands, default male), procedural, or a hand GLB / directory with left.glb and right.glb")
    s = sub.add_parser("hold", help="make the character hold a prop")
    common(s, "--character")
    s.add_argument("--prop", required=True)
    s.add_argument("--prop-name", default="prop")
    s.add_argument("--class", dest="prop_class", default="sword", choices=["sword", "baton", "bow", "quiver", "arrow"])
    s.add_argument("--prop-length", type=float, help="real length of the prop in metres (default: file units are metres)")
    s.add_argument("--hand", default="right", choices=["right", "left", "both"])
    s.add_argument("--grip", default="auto", help="auto, or x,y,z in prop-local metres (with --grip-axis)")
    s.add_argument("--grip-axis")
    s.add_argument("--accept-hands", action="store_true", help="fit even when the hand intake test fails")
    s.add_argument("--keep-hands", action="store_true", help="do not replace failing hands with template hands")
    s.add_argument("--keep-prop", action="store_true", help="use a prop that fails intake instead of a parametric stand-in")
    s.add_argument("--hand-style", default="simple", choices=["simple", "gloved", "chunky"])
    s.add_argument("--hand-source", help="male|female (library wrist-cut hands, default male), procedural, or a hand GLB / directory with left.glb and right.glb")
    s = sub.add_parser("fist", help="hand_open / hand_closed / fist_close / fist_open clips")
    common(s, "--character")
    s = sub.add_parser("check", help="measured physics report over every clip")
    common(s, "--mesh")
    s.add_argument("--prop-node")
    s.add_argument("--clips", help="comma-separated clip names (default: every clip)")
    s = sub.add_parser("correct", help="feet planted, limbs out of the torso, hinge limits; kept only if they help")
    common(s, "--mesh")
    for name, helptext in (("prop", "a parametric functional prop, styled like a generated one (--like)"),
                           ("prop-intake", "can a hand use this prop as its class says? also prints its anchors")):
        s = sub.add_parser(name, help=helptext)
        s.add_argument("--output", required=True)
        s.add_argument("--class", dest="prop_class", required=True, choices=["sword", "bow", "arrow", "quiver"])
        s.add_argument("--height", type=float, default=1.8)
        s.add_argument("--height-m", type=float, help="character height the prop is sized for (default --height)")
        s.add_argument("--palm-width-m", type=float, help="the character's palm width (default 0.085 m at 1.8 m)")
        if name == "prop":
            s.add_argument("--like", help="generated prop to take colours and proportions from")
            s.add_argument("--like-length", type=float, help="real length of the --like prop in metres")
        else:
            s.add_argument("--prop", required=True)
            s.add_argument("--prop-length", type=float)
    s = sub.add_parser("reach", help="author an interaction clip from a target template with IK")
    common(s, "--character")
    s.add_argument("--targets", required=True, help="template JSON (or a bundled name: pull_sword, shoot_bow, baton_pass)")
    s.add_argument("--prop", action="append", help="name=path overrides for the template's props")
    s.add_argument("--keep-prop", action="store_true", help="use props that fail intake instead of parametric stand-ins")
    s.add_argument("--keep-hands", action="store_true")
    s.add_argument("--hand-style", default="simple", choices=["simple", "gloved", "chunky"])
    s.add_argument("--hand-source", help="male|female (library hands, default male), procedural, or a GLB/directory")
    args = p.parse_args()
    run(args)


def cancel(signum, frame):
    raise KeyboardInterrupt


if __name__ == "__main__":
    if "signal" not in os.environ.get("FORGE_FIT_DISABLE", "").split(","):
        signal.signal(signal.SIGTERM, cancel)
    try:
        main()
    except KeyboardInterrupt:
        print(json.dumps({"error": "Cancelled"}), file=sys.stderr)
        sys.exit(130)
    except Exception as exc:  # noqa: BLE001 - machine-readable failure boundary
        print(json.dumps({"error": str(exc) or type(exc).__name__}), file=sys.stderr)
        sys.exit(1)
