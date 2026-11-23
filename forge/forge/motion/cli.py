"""forge-motion: a rigged humanoid performs a motion described in words (NVIDIA Kimodo, local).

Kimodo generates the motion on its own SOMA skeleton (forge/motion/generate.py, in forge/motion/.venv),
forge-animlib's retargeter bakes it onto the character with grounding (forge/animlib/retarget.py,
reused unchanged), and forge-fit check measures the result. Each model and Blender run takes a shared
heavy slot. Progress is JSON Lines on stderr; a failure ends with {"error": ...} and a non-zero exit.
"""

import argparse
import json
import os
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PYTHON = HERE / ".venv/bin/python"
HF_HOME = Path(os.environ.get("FORGE_MOTION_MODELS", ROOT / "models/motion/hf"))
BLENDER = os.environ.get("BLENDER", "/Applications/Blender.app/Contents/MacOS/Blender")
LIMIT = 55 * 1024**3
MODEL = "Kimodo-SOMA-RP-v1.2"
KIMODO_COMMIT = "e1c219c99b4b94ba48be1fb6a1cbc64ecee4c954"
# repo -> pinned commit (forge/motion/fetch_models.py downloads exactly these)
REPOS = {
    "nvidia/Kimodo-SOMA-RP-v1.2": "0e3c89024d896064b6851201373f7834784a386b",
    "McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp": "31474e395ada192e8ed1586db6be79fb3b70c9c0",
    "McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp-supervised": "baa8ebf04a1c2500e61288e7dad65e8ae42601a7",
    "meta-llama/Meta-Llama-3-8B-Instruct": "8afb486c1db24fe5011ec46dfbe5b5dccdb575c2",
}
LICENCES = {
    "kimodo_code": {"licence": "Apache-2.0", "source": "https://github.com/nv-tlabs/kimodo"},
    "kimodo_weights": {
        "licence": "NVIDIA Open Model License",
        "source": "https://huggingface.co/nvidia/Kimodo-SOMA-RP-v1.2",
        "url": "https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/",
        "note": "NVIDIA claims no ownership of outputs; keep the licence and attribution notice if the model is redistributed.",
    },
    "text_encoder": {
        "licence": "Meta Llama 3 Community License + Acceptable Use Policy (base); MIT (LLM2Vec adapters)",
        "source": "https://huggingface.co/meta-llama/Meta-Llama-3-8B-Instruct",
        "note": "Used only to embed the prompt; the encoder is not shipped. See NOTICE.md.",
    },
}


def progress(pct, message, **extra):
    print(json.dumps({"stage": "motion", "pct": pct, "message": message, **extra}), file=sys.stderr, flush=True)


def space(directory):
    if shutil.disk_usage(directory).free < LIMIT:
        raise OSError("Stopped: less than 55 GiB free on the working volume")


def missing_models():
    """Names of pinned checkpoints that are not in the local cache."""
    missing = []
    for repo, rev in REPOS.items():
        snap = HF_HOME / "hub" / ("models--" + repo.replace("/", "--")) / "snapshots" / rev
        if not snap.is_dir() or not any(snap.glob("*.safetensors")):
            missing.append(repo)
    return missing


def run(argv, log, env=None, heavy=True):
    """Run a child in its own process group (under the heavy lock); kill the group on any exit path."""
    argv = [str(a) for a in argv]
    tool = Path(argv[0]).name
    if heavy and not os.environ.get("FORGE_HEAVY_HELD"):
        argv.insert(0, str(ROOT / "tools/heavy.sh"))
    with log.open("w") as out:
        child = subprocess.Popen(argv, stdout=out, stderr=subprocess.STDOUT, env=env, start_new_session=True)
        try:
            while child.poll() is None:
                space(log.parent)
                try:
                    child.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass
        except BaseException:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
            raise
    text = log.read_text(errors="replace")
    if child.returncode:
        lines = text.splitlines()
        error = next((json.loads(x)["error"] for x in reversed(lines) if x.startswith('{"error"')), None)
        error = error or next((x.strip() for x in reversed(lines) if "Error:" in x or "Exception:" in x), None)
        raise RuntimeError(error or f"{tool} failed: " + " ".join(lines[-4:])[-700:])
    return text


def glb_document(path):
    with path.open("rb") as f:
        magic, version, total = struct.unpack("<4sII", f.read(12))
        if magic != b"glTF" or version != 2 or total != path.stat().st_size:
            raise ValueError(f"Invalid GLB: {path.name}")
        length, kind = struct.unpack("<I4s", f.read(8))
        if kind != b"JSON":
            raise ValueError("Missing glTF JSON chunk")
        return json.loads(f.read(length))


def fit_summary(report):
    """Per clip: pass and the failing measures with value and limit."""
    clips = {}
    for name, clip in report["clips"].items():
        failed = {k: {"value": round(m["value"], 4), "limit": m["limit"], "frame": m.get("frame")}
                  for k, m in clip["measures"].items() if m.get("pass") is False}
        clips[name] = {"pass": not failed, "failed": failed}
    return {"pass": report.get("pass"), "clips": clips}


def main():
    p = argparse.ArgumentParser(prog="forge-motion")
    p.add_argument("--prompt", required=True, help="the motion in words, e.g. 'jumps forward and lands'")
    p.add_argument("--character", type=Path, required=True, help="rigged humanoid GLB")
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--seconds", type=float, default=5.0, help="clip length, 1-10 s (default 5)")
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--name", help="clip name (default: from the prompt)")
    p.add_argument("--steps", type=int, default=100, help="denoising steps (default 100)")
    p.add_argument("--device", default="mps", choices=["cpu", "mps"], help="motion model device (default mps, ~4x faster)")
    p.add_argument("--height", type=float, default=1.8, help="character height in metres for the fit check")
    args = p.parse_args()
    if not args.prompt.strip():
        raise ValueError("--prompt is empty")
    if not 1 <= args.seconds <= 10:
        raise ValueError("--seconds must be between 1 and 10 (Kimodo's range)")
    mesh = args.character.resolve()
    if not mesh.is_file() or not glb_document(mesh).get("skins"):
        raise ValueError("--character must be a skinned humanoid GLB")
    if not PYTHON.exists():
        raise RuntimeError("Install the motion stage first: ./install.sh --stage motion --with-models")
    # FORGE_MOTION_STUB_ENCODER=1 ignores the prompt (zero text features: unconditioned motion) so the
    # pipeline can be tested without the gated Llama 3 weights; the manifest records it.
    stub = os.environ.get("FORGE_MOTION_STUB_ENCODER") == "1"
    if missing := [m for m in missing_models() if not (stub and m.startswith("meta-llama"))]:
        raise RuntimeError(f"Missing models: {', '.join(missing)}. Run ./install.sh --stage motion --with-models "
                           "(the Llama 3 base needs the licence accepted on its Hugging Face page and hf auth login).")
    dest = args.output.resolve()
    sidecar = Path(str(dest) + ".manifest.json")
    if dest.exists() or sidecar.exists():
        raise FileExistsError(f"Choose a fresh output path: {dest}")
    name = args.name or re.sub(r"[^a-z0-9]+", "_", args.prompt.lower()).strip("_")[:40]
    dest.parent.mkdir(parents=True, exist_ok=True)
    space(ROOT)
    space(dest.parent)
    start = time.monotonic()
    keep = os.environ.get("FORGE_KEEP_WORK")
    work = Path(tempfile.mkdtemp(prefix=".motion-", dir=dest.parent))
    try:
        progress(5, f"Generating {args.seconds:g} s with Kimodo (waits for a heavy slot)")
        env = {**os.environ, "HF_HOME": str(HF_HOME), "HF_HUB_OFFLINE": "1", "TRANSFORMERS_OFFLINE": "1",
               "LOCAL_CACHE": "True", "HF_HUB_DISABLE_TELEMETRY": "1", "TOKENIZERS_PARALLELISM": "false"}
        bvh = work / "motion.bvh"
        t = time.monotonic()
        run([PYTHON, "-I", HERE / "generate.py", "--prompt", args.prompt, "--seconds", args.seconds,
             "--seed", args.seed, "--steps", args.steps, "--device", args.device, "--bvh", bvh,
             *(["--stub-encoder"] if stub else [])], work / "kimodo.log", env)
        gen = json.loads(bvh.with_suffix(".json").read_text())
        gen["timings"]["wall_with_lock_sec"] = round(time.monotonic() - t, 2)
        # Travel of the hips decides how grounding treats the clip (animlib's root_motion rule: 20 cm).
        travel = gen["hips_travel_m"]
        duration = (gen["frames"] - 1) / gen["fps"]
        job = {"id": name, "file": str(bvh), "duration": duration, "loopable": False,
               "root_motion": travel > 0.20, "category": "kimodo"}
        (work / "jobs.json").write_text(json.dumps([job]))
        progress(55, "Retargeting onto the character with grounding (Blender)")
        result = work / "animated.glb"
        t = time.monotonic()
        blender = run([BLENDER, "-b", "--factory-startup", "-t", "3", "--python-exit-code", "1",
                       "-P", ROOT / "forge/animlib/retarget.py", "--", "--mesh", mesh, "--jobs", work / "jobs.json",
                       "--output", result], work / "blender.log")
        retarget_sec = round(time.monotonic() - t, 2)
        gltf = glb_document(result)
        anims = gltf.get("animations", [])
        if [a["name"] for a in anims] != [name]:
            raise ValueError(f"Exported clips {[a['name'] for a in anims]} != [{name!r}]")
        times = [gltf["accessors"][s["input"]] for s in anims[0]["samplers"]]
        exported = max(x["max"][0] for x in times) - min(x["min"][0] for x in times)
        if abs(exported - duration) > 0.055:
            raise ValueError(f"Exported {exported:.3f} s, expected {duration:.3f} s")
        progress(75, "Measuring the clip (forge-fit check)")
        t = time.monotonic()
        run([sys.executable, "-B", ROOT / "forge/fit/cli.py", "check", "--mesh", result, "--output", work / "fit.json",
             "--height", args.height], work / "fit.log", heavy=False)
        fit = fit_summary(json.loads((work / "fit.json").read_text()))
        fit_sec = round(time.monotonic() - t, 2)
        version = next((line.partition(" (")[0] for line in blender.splitlines()
                        if line.startswith("Blender ") and line[8:9].isdigit()), "Blender")
        manifest = {
            "stage": "motion",
            "input": str(mesh),
            "output": str(dest),
            "clip": {"name": name, "duration": exported, "fps_source": gen["fps"], "frames_source": gen["frames"],
                     "root_motion": job["root_motion"], "hips_travel_m_source": round(travel, 3), "grounding": True},
            "prompt": args.prompt,
            "seed": args.seed,
            "model": {"name": MODEL, "repos": REPOS, "kimodo_commit": KIMODO_COMMIT,
                      "steps": args.steps, "device": args.device, "text_device": gen["text_device"],
                      "stub_encoder": stub,
                      "postprocess": False, "torch": gen["torch"]},
            "tools": {"blender": version, "blender_gltf_exporter": gltf["asset"].get("generator")},
            "licences": LICENCES,
            "fit_check": fit,
            "modifications": "Generated on the SOMA skeleton, exported as BVH with a T-pose rest, retargeted to the "
                             "supplied skeleton with forge-animlib (rest-pose compensation, grounding), baked to glTF.",
            "timings": {"kimodo": gen["timings"], "peak_rss_gb": gen["peak_rss_gb"], "retarget_sec": retarget_sec,
                        "fit_check_sec": fit_sec, "total_sec": round(time.monotonic() - start, 2)},
        }
        if keep:
            manifest["work_dir"] = str(work)
        (work / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        os.link(work / "manifest.json", sidecar)
        try:
            os.link(result, dest)
        except BaseException:
            sidecar.unlink()
            raise
        progress(100, f"Wrote {dest.name} with clip {name!r}", fit_pass=fit["pass"])
    finally:
        if not keep:
            shutil.rmtree(work, ignore_errors=True)


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
