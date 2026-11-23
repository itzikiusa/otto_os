"""List the local, licensed motion catalogue or bake selected clips onto one rig."""

import argparse
import json
import os
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
CATALOG = Path(os.environ.get("FORGE_ANIMLIB_CATALOG", HERE / "catalog.json"))
MODELS = Path(os.environ.get("FORGE_ANIMLIB_MODELS", ROOT / "models" / "animlib"))
BLENDER = os.environ.get("BLENDER", "/Applications/Blender.app/Contents/MacOS/Blender")
sys.path.insert(0, str(ROOT))
from forge.library.animation import PACKS as LIBRARY_PACKS
from forge.library.animation import library_clips
from forge.library.animation_native import attach_native

LIBRARY = Path(os.environ.get("FORGE_LIBRARY_MODELS", ROOT / "models" / "library"))
LIMIT = 55 * 1024**3


def progress(pct, message):
    print(
        json.dumps({"stage": "animlib", "pct": pct, "message": message}),
        file=sys.stderr,
        flush=True,
    )


def space(directory):
    if shutil.disk_usage(directory).free < LIMIT:
        raise OSError("Stopped: less than 55 GiB free on the working volume")


def child(argv, directory):
    log = directory / "blender.log"
    if os.environ.get("FORGE_HEAVY_HELD") != "1":
        argv = [ROOT / "tools/heavy.sh", *argv]
    with log.open("w") as output:
        process = subprocess.Popen(
            argv,
            stdout=output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        try:
            while process.poll() is None:
                space(directory)
                try:
                    process.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass
        except BaseException:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            raise
    if process.returncode:
        lines = log.read_text(errors="replace").splitlines()
        detail = next(
            (x.strip() for x in reversed(lines) if "Error:" in x or "Exception:" in x),
            "",
        )
        raise RuntimeError(detail or "Blender failed: " + " ".join(lines[-4:])[-700:])
    version = "Blender"
    timings = {}
    with log.open(errors="replace") as stream:
        for line in stream:
            if (
                line.startswith("Blender ")
                and line.removeprefix("Blender ")[:1].isdigit()
            ):
                version = line.partition(" (")[0].strip()
            elif line.startswith("FORGE_BAKE_SECONDS "):
                timings["blender_sec"] = float(line.split()[1])
    return version, timings


def glb_document(path):
    with path.open("rb") as f:
        magic, version, total = struct.unpack("<4sII", f.read(12))
        if magic != b"glTF" or version != 2 or total != path.stat().st_size:
            raise ValueError(f"Invalid GLB header: {path.name}")
        length, kind = struct.unpack("<I4s", f.read(8))
        if kind != b"JSON":
            raise ValueError("Missing glTF JSON chunk")
        return json.loads(f.read(length))


def prepare_jobs(catalog, ids, stage):
    lookup = {c["id"]: c for c in catalog["clips"]}
    if len(lookup) != len(catalog["clips"]):
        raise ValueError("Duplicate clip ID in catalog")
    jobs = []
    archive_files = {}
    for index, key in enumerate(ids):
        if key not in lookup:
            raise ValueError(
                f"Unknown clip '{key}'. Use forge-animlib list for available IDs."
            )
        clip = lookup[key]
        library_pack = (
            "FORGE_ANIMLIB_CATALOG" not in os.environ and clip["pack"] in LIBRARY_PACKS
        )
        source_root = LIBRARY if library_pack else MODELS
        path = (source_root / clip["file"]).resolve()
        if not path.is_relative_to(source_root.resolve()) or not path.is_file():
            raise ValueError(f"Missing local source for {key}: {clip['file']}")
        pack = catalog["packs"][clip["pack"]]
        license_path = (source_root / pack["license_file"]).resolve()
        if (
            not license_path.is_relative_to(source_root.resolve())
            or not license_path.is_file()
        ):
            raise ValueError(f"Missing local licence for {clip['pack']}")
        if clip.get("member"):
            member = clip["member"]
            pair = (path, member)
            if pair not in archive_files:
                destination = stage / (f"source-{index}" + Path(member).suffix)
                with (
                    zipfile.ZipFile(path) as archive,
                    archive.open(member) as src,
                    destination.open("wb") as dst,
                ):
                    while buf := src.read(1024**2):
                        space(stage)
                        dst.write(buf)
                archive_files[pair] = destination
            path = archive_files[pair]
        jobs.append({**pack, **clip, "file": str(path)})
    return jobs


def apply(args, catalog):
    mesh = args.mesh.resolve()
    if not mesh.is_file() or not glb_document(mesh).get("skins"):
        raise ValueError("Input must be a skinned humanoid GLB")
    ids = [name.strip() for name in args.clips.split(",")]
    if not all(ids) or len(set(ids)) != len(ids):
        raise ValueError("Provide a comma-separated list of distinct clip IDs")
    dest = args.output.resolve()
    sidecar = Path(str(dest) + ".manifest.json")
    if dest.exists() or sidecar.exists():
        raise FileExistsError(f"Choose a fresh output path: {dest}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    space(ROOT)
    space(dest.parent)
    start = time.monotonic()
    progress(0, "Preparing animation sources")
    with tempfile.TemporaryDirectory(prefix=".animlib-", dir=dest.parent) as tmp:
        stage = Path(tmp)
        jobs = prepare_jobs(catalog, ids, stage)
        jobs_file = stage / "jobs.json"
        jobs_file.write_text(json.dumps(jobs))
        result = stage / "animated.glb"
        progress(15, f"Binding {len(jobs)} clip(s) to the target skeleton")
        native_path = stage / "native.glb" if args.grounding else result
        native = attach_native(mesh, jobs, native_path)
        version, timings = None, {}
        if not native or args.grounding:
            version, timings = child(
                [
                    BLENDER,
                    "-b",
                    "--factory-startup",
                    "-t",
                    "3",
                    "--python-exit-code",
                    "1",
                    "-P",
                    HERE / "retarget.py",
                    "--",
                    *(["--native"] if native else []),
                    *(["--no-grounding"] if not args.grounding else []),
                    "--mesh",
                    native_path if native else mesh,
                    "--jobs",
                    jobs_file,
                    "--output",
                    result,
                ],
                stage,
            )
        progress(90, "Checking embedded clips and writing provenance")
        gltf = glb_document(result)
        animations = gltf.get("animations", [])
        names = [a["name"] for a in animations]
        if sorted(names) != sorted(ids) or not gltf.get("skins"):
            raise ValueError(
                f"Exported skin or animations did not match request. Found {names}"
            )
        clips = []
        for animation in animations:
            samples = [gltf["accessors"][s["input"]] for s in animation["samplers"]]
            duration = max(s["max"][0] for s in samples) - min(
                s["min"][0] for s in samples
            )
            catalog_clip = next(x for x in jobs if x["id"] == animation["name"])
            if abs(duration - catalog_clip["duration"]) > 0.055:
                raise ValueError(
                    f"{animation['name']}: expected {catalog_clip['duration']:.3f}s, exported {duration:.3f}s"
                )
            clips.append(
                {
                    **{
                        k: catalog_clip[k]
                        for k in (
                            "id",
                            "pack",
                            "licence",
                            "category",
                            "loopable",
                            "root_motion",
                        )
                    },
                    "duration": duration,
                }
            )
        # Pack notices remain available to downstream game distribution, including required attribution.
        packs = {c["pack"]: catalog["packs"][c["pack"]] for c in clips}
        used_tools = {}
        if native:
            used_tools["native_binding"] = "Forge glTF joint-local bind transfer"
        if version:
            used_tools.update(
                blender=version, blender_gltf_exporter=gltf["asset"].get("generator")
            )
        manifest = {
            "stage": "animlib",
            "grounding": args.grounding,
            "input": str(mesh),
            "output": str(dest),
            "tools": used_tools,
            "clips": clips,
            "modifications": (
                "Native joint-local motion applied relative to target bind transforms."
                if native
                else "Motion retargeted to the supplied skeleton and baked to glTF animation channels."
            ),
            "licences": packs,
            "timings": {**timings, "total_sec": round(time.monotonic() - start, 3)},
        }
        receipt = stage / "manifest.json"
        receipt.write_text(json.dumps(manifest, indent=2) + "\n")
        os.link(receipt, sidecar)
        try:
            os.link(result, dest)
        except BaseException:
            sidecar.unlink()
            raise
    progress(100, f"Wrote {dest.name} with {len(clips)} embedded clips")


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError(message)


def main():
    p = Parser(prog="forge-animlib")
    sub = p.add_subparsers(dest="command", required=True)
    ls = sub.add_parser("list")
    ls.add_argument("--category")
    command = sub.add_parser("apply")
    command.add_argument("--mesh", type=Path, required=True)
    command.add_argument("--clips", required=True)
    command.add_argument("--output", type=Path, required=True)
    command.add_argument(
        "--grounding",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Correct skinned sole height and stabilize stance (default: on)",
    )
    args = p.parse_args()
    catalog = json.loads(CATALOG.read_text())
    if "FORGE_ANIMLIB_CATALOG" not in os.environ:
        extra = library_clips(LIBRARY)
        catalog["clips"].extend(extra["clips"])
        catalog["packs"].update(extra["packs"])
    if args.command == "list":
        print(
            json.dumps(
                [
                    c
                    for c in catalog["clips"]
                    if args.category is None or c["category"] == args.category
                ],
                indent=2,
            )
        )
    else:
        apply(args, catalog)


def cancel(signum, frame):
    raise KeyboardInterrupt


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, cancel)
    try:
        main()
    except KeyboardInterrupt:
        print(json.dumps({"error": "Cancelled"}), file=sys.stderr)
        sys.exit(130)
    except Exception as exc:  # noqa: BLE001 - machine-readable failure boundary for the CLI
        print(json.dumps({"error": str(exc) or type(exc).__name__}), file=sys.stderr)
        sys.exit(1)
