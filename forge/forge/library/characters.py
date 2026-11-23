"""Package publisher characters and outfits for Forge, preserving their skins and materials."""

import json
import os
from pathlib import Path
import re
import shutil
import signal
import threading
import subprocess
import tempfile
from urllib.parse import unquote

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
BLENDER = "/Applications/Blender.app/Contents/MacOS/Blender"
DIGITS = {"index": "Index", "middle": "Middle", "ring": "Ring", "pinky": "Little", "thumb": "Thumb"}


def joint_names():
    names = dict(pelvis="Hips", spine_01="Spine", spine_02="Chest", spine_03="UpperChest",
                 neck_01="Neck", Head="Head")
    for suffix, side in (("l", "Left"), ("r", "Right")):
        for source, target in dict(clavicle="Shoulder", upperarm="UpperArm", lowerarm="LowerArm",
                                   hand="Hand", thigh="UpperLeg", calf="LowerLeg", foot="Foot",
                                   ball="Toes").items():
            names[source + "_" + suffix] = side + target
        for source, target in DIGITS.items():
            for j, segment in enumerate(("Proximal", "Intermediate", "Distal"), 1):
                names[f"{source}_0{j}_{suffix}"] = side + target + segment
    return names


def stage_source(source, directory):
    """The publisher uses _png.png in two normal-map URIs. Preserve the originals on disk."""
    directory.mkdir()
    doc = json.loads(source.read_bytes())
    names = joint_names()
    if len(doc.get("skins", [])) != 1:
        raise ValueError(f"Expected one skin in {source.name}")
    found = set()
    for index in doc["skins"][0]["joints"]:
        node = doc["nodes"][index]
        old = node["name"]
        if old in names:
            found.add(old)
            node.setdefault("extras", {})["publisher_name"] = old
            node["name"] = names[old]
    if found != names.keys():
        raise ValueError(f"{source.name} lacks expected joints: {sorted(names.keys() - found)}")
    repairs = []
    for resource in doc.get("buffers", []) + doc.get("images", []):
        uri = resource.get("uri", "")
        if not uri or uri.startswith("data:"):
            continue
        path = (source.parent / unquote(uri)).resolve()
        if not path.is_relative_to(source.parent.resolve()):
            raise ValueError(f"Expected a material or buffer inside {source.parent.name}: {uri}")
        if not path.is_file():
            fixed = path.with_name(re.sub(r"_png\.png$", ".png", path.name, flags=re.I))
            if not fixed.is_file():
                raise FileNotFoundError(f"{source.name} references missing {uri}")
            repairs.append([uri, fixed.name])
            path = fixed
        resource["uri"] = path.name
        link = directory / path.name
        if not link.exists():
            link.symlink_to(path.resolve())
    staged = directory / source.name
    staged.write_text(json.dumps(doc))
    return staged, repairs


def run_worker(cmd):
    def cancel(signum, frame):
        raise InterruptedError("Character export cancelled")
    install_handler = threading.current_thread() is threading.main_thread()
    old = signal.signal(signal.SIGTERM, cancel) if install_handler else None
    try:
        with subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                              start_new_session=True) as child:
            try:
                log, _ = child.communicate()
            except BaseException:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
                    try:
                        child.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait()
                raise
            if child.returncode:
                raise RuntimeError("Character export failed: " + log[-2400:])
    finally:
        if install_handler:
            signal.signal(signal.SIGTERM, old)


def export_character(base_path, output_path, parts=(), *, hand_dir=None):
    """Export one base, optionally dressed, to GLB and return manifest details.

    hand_dir requests two wrist-cut template GLBs, named left.glb and right.glb.
    """
    base = Path(base_path).resolve()
    output = Path(output_path).resolve()
    sources = [base] + [Path(p).resolve() for p in parts]
    if not base.stem.lower().endswith("_fullbody"):
        raise ValueError("Select a full-body base character")
    sex = base.stem.lower().split("_")[-2]
    if any(p.stem.lower().split("_")[0] != sex for p in sources[1:]):
        raise ValueError("Outfit and base must use the same publisher body type")
    if len(set(sources)) != len(sources):
        raise ValueError("The same asset was selected more than once")
    output.parent.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(output.parent).free < 55 * 2**30:
        raise OSError("Stopped below the 55 GiB free-space floor")
    with tempfile.TemporaryDirectory(dir=output.parent, prefix=".character-") as tmp:
        work = Path(tmp)
        staged, repairs = [], []
        for i, source in enumerate(sources):
            path, fixed = stage_source(source, work / str(i))
            staged.append(str(path))
            repairs += fixed
        job = work / "job.json"
        job.write_text(json.dumps({"sources": staged, "source_dir": str(base.parent), "templates": hand_dir is not None}))
        cmd = [os.getenv("BLENDER", BLENDER), "-b", "--factory-startup", "--disable-autoexec", "--python-exit-code", "1",
               "-P", str(HERE / "characters_blender.py"), "--", str(job)]
        if not os.environ.get("FORGE_HEAVY_HELD"):
            cmd.insert(0, str(ROOT / "tools/heavy.sh"))
        run_worker(cmd)
        result = json.loads((work / "result.json").read_text())
        result["texture_uri_repairs"] = repairs
        if hand_dir is not None:
            hand_dir = Path(hand_dir)
            hand_dir.mkdir(parents=True, exist_ok=True)
            for side in ("left", "right"):
                os.replace(work / f"{side}.glb", hand_dir / f"{side}.glb")
        os.replace(work / "character.glb", output)
    return result
