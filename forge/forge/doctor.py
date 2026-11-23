"""Read-only, offline dependency checks. No models are loaded or downloaded."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
AF = ROOT / "engines/assetfurnace"
MODELS = ROOT / "models"
TAG = "v0.6.0"
COMMIT = "5f9040bfa00e8269104930a0f918a68d828ccd87"
BLENDER = "/Applications/Blender.app/Contents/MacOS/Blender"


def result(status, detail, **fields):
    return dict(status=status, detail=detail, **fields)


def run(*args):
    """Capture subprocess diagnostics without writing bytecode or contacting HF."""
    p = subprocess.run(
        args,
        check=False,
        cwd=ROOT,
        text=True,
        capture_output=True,
        timeout=20,
        env={
            **os.environ,
            "HF_HUB_OFFLINE": "1",
            "HF_HUB_DISABLE_TELEMETRY": "1",
            "PYTHONDONTWRITEBYTECODE": "1",
        },
    )
    if p.returncode:
        lines = (p.stderr or p.stdout).strip().splitlines()
        raise ValueError(lines[-1] if lines else f"process exited {p.returncode}")
    return p.stdout.strip()


def probe(operation):
    try:
        return operation()
    except (
        OSError,
        ValueError,
        KeyError,
        TypeError,
        IndexError,
        subprocess.TimeoutExpired,
        zipfile.BadZipFile,
    ) as e:
        return result("broken", str(e))


def tool(path, version_flag=None):
    path = Path(path)
    if not os.path.lexists(path):
        return result("absent", "executable missing", path=str(path))
    if not path.is_file() or not os.access(path, os.X_OK):
        return result("broken", "file is not executable", path=str(path))
    version = run(path, version_flag).splitlines()[0] if version_flag else "executable installed"
    return result("ok", version, path=str(path))


def launcher(name, sources):
    status = tool(ROOT / "bin" / name)
    missing = [p for p in sources if not (ROOT / "forge" / p).is_file()]
    if status["status"] == "ok" and missing:
        status.update(status="broken", detail="missing source: " + ", ".join(missing))
    return status


def checkout():
    if not AF.exists():
        return result("absent", f"expected AssetFurnace {TAG}", path=str(AF))
    git = ["git", "-C", AF]
    if Path(run(*git, "rev-parse", "--show-toplevel")).resolve() != AF.resolve():
        return result("broken", "engine directory is not its own Git checkout", path=str(AF))
    commit = run(*git, "rev-parse", "HEAD")
    tags = run(*git, "tag", "--points-at", "HEAD").splitlines()
    ok = commit == COMMIT and TAG in tags
    return result(
        "ok" if ok else "broken",
        f"{', '.join(tags) or 'untagged'} @ {commit[:12]} (expected {TAG} @ {COMMIT[:12]})",
        path=str(AF),
        commit=commit,
        tags=tags,
    )


# Minimal subprocess probes run in the interpreter the stage will actually use. The
# larger torch/bpy packages are located via their specs to avoid allocating the runtime.
PYTHON_PROBE = """
import importlib, importlib.metadata as md, importlib.util as util, json, pathlib, sys
prefix, packages, native, pins = sys.argv[1], json.loads(sys.argv[2]), json.loads(sys.argv[3]), json.loads(sys.argv[4])
if sys.version_info[:2] != (3, 11):
    raise RuntimeError("stage requires Python 3.11, found " + sys.version.split()[0])
if pathlib.Path(sys.prefix).resolve() != pathlib.Path(prefix).resolve():
    raise RuntimeError("interpreter did not enter its venv")
for module in packages:
    if util.find_spec(module) is None:
        raise RuntimeError("missing module " + module)
for name, expected in pins.items():
    actual = md.version(name)
    if actual != expected:
        raise RuntimeError(f"{name} {actual}, expected {expected}")
for module in native:
    importlib.import_module(module)
print(sys.version.split()[0])
"""


def venv(path, packages, native=(), pins=None):
    if not path.exists():
        return result("absent", "environment missing", path=str(path))
    python = path / "bin/python"
    version = run(
        python,
        "-I",
        "-B",
        "-c",
        PYTHON_PROBE,
        str(path),
        json.dumps(packages),
        json.dumps(native),
        json.dumps(pins or {}),
    )
    return result(
        "ok",
        f"Python {version}, modules checked: {', '.join(packages)}",
        path=str(path),
    )


def blender_wheels(blender, target, version_output):
    if not target.exists():
        return result("absent", "CoACD and xatlas target directory missing", path=str(target))
    # The target directory contains extension modules compiled for Blender's bundled Python.
    match = re.search(r"Blender (\d+\.\d+)", version_output)
    if not match:
        raise ValueError("cannot identify Blender's Python ABI")
    folder = blender.resolve().parents[1] / "Resources" / match[1] / "python/bin"
    python = next(iter(sorted(folder.glob("python3.*"))), None)
    if not python:
        raise ValueError(f"missing Blender Python in {folder}")
    script = (
        "import sys; sys.path.insert(0,sys.argv[1]); import coacd,xatlas; "
        "from importlib.metadata import version; print(version('coacd'),version('xatlas'))"
    )
    versions = run(python, "-I", "-B", "-c", script, str(target)).split()
    ok = versions == ["1.0.14", "0.0.11"]
    return result(
        "ok" if ok else "broken",
        f"Blender Python loaded CoACD and xatlas {' / '.join(versions)}",
        path=str(target),
        python=str(python),
    )


def bundle(directory, spec, *, started=False):
    """Check the actual runtime paths, following snapshot symlinks and counting payload bytes."""
    files = {}
    bad = []
    for name, expected in spec.items():
        path = directory / name
        try:
            size = path.stat().st_size if path.is_file() else 0
        except OSError:
            # A download may have moved between the two metadata reads.
            size = 0
            started = True
        files[name] = size
        if size == 0 or (expected is not None and size != expected) or not os.access(path, os.R_OK):
            bad.append(name)
        started |= os.path.lexists(path)
    size = sum(files.values())
    if not started:
        return result("absent", "not downloaded", path=str(directory), bytes=0)
    return result(
        "broken" if bad else "ok",
        "missing or unexpected size: " + ", ".join(bad) if bad else f"{len(files)} files",
        path=str(directory),
        bytes=size,
        files=files,
    )


def has_contents(directory):
    try:
        return directory.is_dir() and any(directory.iterdir())
    except OSError:
        return True  # An inaccessible bundle needs a diagnostic, too.


def anim_assets(checks, models):
    catalog_path = Path(os.environ.get("FORGE_ANIMLIB_CATALOG", ROOT / "forge/animlib/catalog.json"))
    catalog = json.loads(catalog_path.read_text())
    packs = catalog["packs"]
    clips = catalog["clips"]
    if not packs or not clips or len({r["id"] for r in clips}) != len(clips):
        raise ValueError("empty catalog or duplicate clip IDs")
    counts = {}
    for pack, meta in packs.items():
        entries = [r for r in clips if r["pack"] == pack]
        counts[pack] = len(entries)
        if not entries:
            raise ValueError(f"{pack}: catalog has no clips")
        names = {r["file"]: None for r in entries}
        names[meta["license_file"]] = None
        if "credit_file" in meta:
            names[meta["credit_file"]] = None
        weights = {
            "100style/100STYLE.zip": 1468515354,
            "kaykit-adventurers/Knight.glb": 3659532,
            "kaykit-skeletons/Skeleton_Warrior.glb": 4863620,
        }
        for name in names:
            names[name] = weights.get(name)
        status = bundle(models, names, started=has_contents(models / pack))
        if status["status"] == "ok" and pack == "100style":
            with zipfile.ZipFile(models / entries[0]["file"]) as z:
                missing = {c["member"] for c in entries} - set(z.namelist())
                if missing:
                    status.update(
                        status="broken",
                        detail=f"{len(missing)} catalogued BVHs missing in ZIP",
                    )
        checks[f"pack/{pack}"] = status
    if sum(counts.values()) != len(clips):
        raise ValueError("clip references an unknown pack")
    return result("ok", f"{len(clips)} catalog entries", path=str(catalog_path), clips=counts)


def preset_assets():
    folder = AF / "image_to_3dlab/motions"
    if not folder.exists():
        return result("absent", "preset directory missing", path=str(folder))
    presets = json.loads((folder / "presets.json").read_text())["presets"]
    if not presets:
        raise ValueError("empty preset clip list")
    inventory = bundle(folder, {p["id"] + ".npz": None for p in presets}, started=True)
    if inventory["status"] == "ok":
        inventory["detail"] = f"{len(presets)} Kimodo motion presets (NVIDIA Open Model License)"
    return inventory


def model_inventory(checks):
    pixal = MODELS / "af/pixal3d-cpp/pixal3d-sv"
    # Sizes at the engine's pinned 46d399a GGUF revision, plus the flattened a57397b matte.
    pixal_files = {
        "pixal3d_naf.gguf": 1334656,
        "pixal3d_ss_flow_sv.gguf": 1426559744,
        "ss_dec.gguf": 147379392,
        "shape_dec.gguf": 881361568,
        "tex_dec.gguf": 881344576,
        "pixal3d_shape_flow_512_sv.gguf": 1476761760,
        "pixal3d_shape_flow_1024_sv.gguf": 1476761760,
        "pixal3d_tex_flow_1024_sv.gguf": 1476813984,
        "pixal3d-models.json": None,
        "LICENSE.md": None,
        "MIT_LICENSE.md": None,
        "DINOV3_LICENSE.md": None,
    }
    checks["realesrgan-x4plus"] = bundle(AF / "vendor/realesrgan", {"RealESRGAN_x4plus.pth": 67040989})
    checks["pixal3d"] = bundle(pixal, pixal_files, started=has_contents(pixal))
    checks["dinov3"] = bundle(pixal, {"dinov3.gguf": 323657920})
    if checks["dinov3"]["status"] == "ok":
        checks["dinov3"]["detail"] = "Pixal3D bundled encoder (Meta DINOv3 License)"
    checks["birefnet-gguf"] = bundle(pixal, {"birefnet.gguf": 882749024})
    checks["birefnet-onnx"] = bundle(MODELS / "af/u2net", {"birefnet-lite-lean.onnx": 180666557})
    hf = MODELS / "hf/hub"
    for label, repo, rev, name, size in (
        (
            "flux2",
            "leejet--FLUX.2-klein-4B-GGUF",
            "3b1f5a9dc3abb32238b053aeb3d823c30afdacbd",
            "flux-2-klein-4b-Q8_0.gguf",
            4300629440,
        ),
        (
            "qwen3-4b",
            "unsloth--Qwen3-4B-GGUF",
            "22c9fc8a8c7700b76a1789366280a6a5a1ad1120",
            "Qwen3-4B-Q8_0.gguf",
            4280405792,
        ),
        (
            "flux2-vae",
            "Comfy-Org--Flux-2-Klein-4B",
            "533c4cbc3d3b89af1743d9466c434e89810391c0",
            "split_files/vae/flux2-vae.safetensors",
            336211292,
        ),
    ):
        cache = hf / ("models--" + repo)
        checks[label] = bundle(cache / "snapshots" / rev, {name: size}, started=has_contents(cache))
    skin = AF / "vendor/SkinTokens"
    checks["skintokens-weights"] = bundle(
        skin / "experiments",
        {
            "articulation_xl_quantization_256_token_4/grpo_1400.ckpt": 1131603979,
            "skin_vae_2_10_32768/last.ckpt": 487311745,
        },
    )
    qwen = skin / "models/Qwen3-0.6B"
    checks["skintokens-qwen-tokenizer"] = bundle(
        qwen,
        {"config.json": None, "tokenizer_config.json": None, "tokenizer.json": None},
        started=has_contents(qwen),
    )
    voice = MODELS / "voice"
    kokoro = voice / "kokoro"
    voices = {p.name for p in (kokoro / "voices").glob("*.safetensors")}
    expected = {"config.json": None, "kokoro-v1_0.safetensors": 327115152}
    # All 28 English speakers supported by the pinned package, including the aliases.
    for group, ids in {
        "af": "alloy aoede bella heart jessica kore nicole nova river sarah sky",
        "am": "adam echo eric fenrir liam michael onyx puck santa",
        "bf": "alice emma isabella lily",
        "bm": "daniel fable george lewis",
    }.items():
        voices.update(f"{group}_{voice_id}.safetensors" for voice_id in ids.split())
    expected.update({"voices/" + v: 522320 for v in voices})
    checks["kokoro"] = bundle(kokoro, expected, started=has_contents(kokoro))
    qwen = voice / "qwen"
    checks["qwen-voicedesign"] = bundle(
        qwen,
        {
            "config.json": None,
            "model.safetensors": 1625226859,
            "model.safetensors.index.json": None,
            "merges.txt": None,
            "vocab.json": None,
            "tokenizer_config.json": None,
            "preprocessor_config.json": None,
            "generation_config.json": None,
            "speech_tokenizer/model.safetensors": 682293092,
            "speech_tokenizer/config.json": None,
            "speech_tokenizer/configuration.json": None,
            "speech_tokenizer/preprocessor_config.json": None,
        },
        started=has_contents(qwen),
    )
    site = ROOT / "forge/voice/.venv/lib/python3.11/site-packages"
    en = site / "en_core_web_sm/en_core_web_sm-3.8.0"
    checks["spacy-english"] = bundle(
        en,
        {
            name: None
            for name in (
                "config.cfg",
                "meta.json",
                "tokenizer",
                "tok2vec/model",
                "tagger/model",
                "parser/model",
                "ner/model",
                "lemmatizer/lookups/lookups.bin",
            )
        },
        started=has_contents(site / "en_core_web_sm"),
    )
    ps = site / "pocketsphinx/model/en-us"
    checks["pocketsphinx-english"] = bundle(
        ps,
        {
            name: None
            for name in (
                "cmudict-en-us.dict",
                "en-us/mdef",
                "en-us/means",
                "en-us/variances",
                "en-us/sendump",
                "en-us/transition_matrices",
                "en-us/feat.params",
            )
        },
        started=has_contents(site / "pocketsphinx"),
    )



# Sound effects, ambience and music (docs/audio.md).
AUDIO_SOURCES = ["audio/__main__.py", "audio/engine.py", "audio/analysis.py", "audio/fetch_models.py"]
ACE_COMMIT = "ca1e85fe9430179831e6bc6be790c332190a3866"


def ace_checkout():
    src = ROOT / "engines/ace-step"
    if not src.exists():
        return result("absent", "ACE-Step checkout missing", path=str(src))
    commit = run("git", "-C", src, "rev-parse", "HEAD")
    return result(
        "ok" if commit == ACE_COMMIT else "broken",
        f"ACE-Step @ {commit[:12]} (expected {ACE_COMMIT[:12]})",
        path=str(src),
    )


def audio_inventory(checks):
    moss = MODELS / "audio/moss-sfx"
    checks["moss-sfx"] = bundle(
        moss,
        {
            "mlx/dit.safetensors": 831262348,
            "mlx/vae.safetensors": 1486185128,
            "text_encoder/model-00001-of-00002.safetensors": 3441185608,
            "text_encoder/model-00002-of-00002.safetensors": 622329984,
            "text_encoder/config.json": None,
            "tokenizer/tokenizer.json": None,
            "model_index.json": None,
        },
        started=has_contents(moss),
    )
    ace = MODELS / "audio/ace-step"
    checks["ace-step"] = bundle(
        ace,
        {
            "acestep-v15-turbo/model.safetensors": 4787825604,
            "acestep-v15-turbo/config.json": None,
            "acestep-v15-turbo/silence_latent.pt": 3841215,
            "vae/diffusion_pytorch_model.safetensors": 337431388,
            "vae/config.json": None,
            "Qwen3-Embedding-0.6B/model.safetensors": 1191586416,
            "Qwen3-Embedding-0.6B/tokenizer.json": None,
        },
        started=has_contents(ace),
    )


def diagnose():
    checks = {}

    def add(key, fn, *args):
        checks[key] = probe(lambda: fn(*args))

    free = shutil.disk_usage(ROOT).free
    launchers = {
        "forge": ["cli.py", "glbstats.py", "versions.py", "skintokens_rig.py", "bodyplan.py", "gait.py", "fit/plan_check.py"],
        "forge-voice": [
            "voice/__main__.py",
            "voice/engine.py",
            "voice/lipsync.py",
            "voice/fetch_models.py",
            "voice/characters.json",
        ],
        "forge-animlib": ["animlib/cli.py", "animlib/retarget.py"],
        "forge-gameready": ["gameready/gameready.py"],
        "forge-skinfix": ["skinfix/cli.py", "skinfix/weights.py"],
        **{name: AUDIO_SOURCES for name in ("forge-sfx", "forge-music", "forge-audio")},
    }
    for name, sources in launchers.items():
        add("bin/" + name, launcher, name, sources)
    add("heavy.sh", tool, ROOT / "tools/heavy.sh")
    add("engine", checkout)
    blender = Path(shutil.which(os.environ.get("BLENDER", BLENDER)) or os.environ.get("BLENDER", BLENDER))
    add("blender", tool, blender, "--version")
    add("ffmpeg", tool, shutil.which("ffmpeg") or "ffmpeg", "-version")
    add(
        "engine-venv",
        venv,
        AF / ".venv",
        ["PIL", "rembg", "numpy", "torch", "onnxruntime", "scipy", "trimesh"],
        ["numpy", "onnxruntime"],
    )
    skin = AF / "vendor/SkinTokens"
    add(
        "skintokens-venv",
        venv,
        skin / ".venv",
        ["bpy", "torch", "diffusers", "transformers", "numpy"],
        ["numpy"],
    )
    add(
        "voice-venv",
        venv,
        ROOT / "forge/voice/.venv",
        ["mlx_audio", "mlx", "misaki", "spacy"],
        ["mlx.core"],
        {"mlx-audio": "0.6.7", "misaki": "0.9.4"},
    )
    add(
        "audio-venv",
        venv,
        ROOT / "forge/audio/.venv",
        ["moss_sfx_mlx", "acestep", "mlx", "mlx_lm", "torch", "pyloudnorm", "soundfile"],
        ["mlx.core"],
        {"moss-sfx-mlx": "0.1.0", "pyloudnorm": "0.2.0", "mlx": "0.30.6"},
    )
    add("ace-step-src", ace_checkout)
    add("trellis-cli", tool, AF / "vendor/pixal3d-cpp/build/trellis-cli")
    add("sd-cli", tool, AF / "vendor/sdcpp/sd-cli")
    target = ROOT / "forge/gameready/.venv"
    add(
        "gameready-python-target",
        blender_wheels,
        blender,
        target,
        checks["blender"]["detail"],
    )
    add("preset-clips", preset_assets)
    models = Path(os.environ.get("FORGE_ANIMLIB_MODELS", MODELS / "animlib"))
    add("catalog", anim_assets, checks, models)
    model_inventory(checks)
    audio_inventory(checks)

    stages = {}

    def stage(name, installed, tools, assets=(), any_pack=False, expected_after=()):
        required = list(tools)
        state = "ready"
        if not installed:
            state = "not_installed"
        elif any(checks[t]["status"] != "ok" for t in required) or any(
            checks[a]["status"] == "broken" for a in assets
        ):
            state = "broken"
        elif assets:
            ready = [checks[a]["status"] == "ok" for a in assets]
            if not (any(ready) if any_pack else all(ready)):
                state = (
                    "broken" if any(checks[k]["status"] == "ok" for k in expected_after) else "needs_models"
                )
        unavailable = [k for k in [*required, *assets] if checks[k]["status"] != "ok"]
        stages[name] = {
            "status": state,
            "runtime": required,
            "assets": list(assets),
            "unavailable": unavailable,
        }

    engine = AF.exists()
    runtime = ["engine", "engine-venv", "bin/forge", "heavy.sh"]
    stage("engine", engine, runtime)
    image_tools = runtime + ["blender", "trellis-cli"]
    image_models = ["pixal3d", "dinov3", "birefnet-gguf", "birefnet-onnx"]
    stage(
        "generate/image",
        (AF / "vendor/pixal3d-cpp").exists() or checks["pixal3d"]["status"] != "absent",
        image_tools,
        image_models,
        expected_after=("pixal3d",),
    )
    stage("generate/character-standard", engine, image_tools, [*image_models, "realesrgan-x4plus"])
    stage(
        "generate/prompt",
        (AF / "vendor/sdcpp").exists()
        or any(checks[k]["status"] != "absent" for k in ("flux2", "qwen3-4b", "flux2-vae")),
        image_tools + ["sd-cli"],
        image_models + ["flux2", "qwen3-4b", "flux2-vae"],
        expected_after=("flux2", "qwen3-4b", "flux2-vae"),
    )
    stage(
        "rig",
        skin.exists(),
        runtime + ["skintokens-venv"],
        ["skintokens-weights", "skintokens-qwen-tokenizer"],
        expected_after=("skintokens-weights",),
    )
    stage("animate/presets", skin.exists(), runtime + ["skintokens-venv", "preset-clips"])
    # any skeleton SkinTokens makes is kept: a humanoid gets Unity names and the presets, every other body
    # (any number of legs, or none) a body plan and procedural idle/walk/run (docs/engine.md#any-body)
    stages["rig"]["body_plans"] = stages["animate/presets"]["body_plans"] = "any"
    stage(
        "gameready",
        target.exists(),
        ["blender", "gameready-python-target", "bin/forge-gameready"],
    )
    stage("skinfix", (ROOT / "bin/forge-skinfix").exists(), ["blender", "heavy.sh", "bin/forge-skinfix"])
    packs = [k for k in checks if k.startswith("pack/")]
    stage(
        "animlib",
        (ROOT / "bin/forge-animlib").exists() and checks["blender"]["status"] == "ok" or has_contents(models),
        ["blender", "catalog", "bin/forge-animlib", "heavy.sh"],
        packs,
        any_pack=True,
    )
    voice = (ROOT / "forge/voice/.venv").exists()
    stage(
        "voice/kokoro",
        voice or checks["kokoro"]["status"] != "absent",
        ["voice-venv", "ffmpeg", "bin/forge-voice"],
        ["kokoro", "spacy-english"],
        expected_after=("kokoro",),
    )
    stage(
        "voice/qwen",
        voice or checks["qwen-voicedesign"]["status"] != "absent",
        ["voice-venv", "ffmpeg", "bin/forge-voice"],
        ["qwen-voicedesign"],
    )
    stage(
        "voice/visemes",
        voice,
        ["voice-venv", "bin/forge-voice"],
        ["pocketsphinx-english"],
        expected_after=("kokoro", "qwen-voicedesign"),
    )
    audio = (ROOT / "forge/audio/.venv").exists()
    stage(
        "audio/sfx",
        audio or checks["moss-sfx"]["status"] != "absent",
        ["audio-venv", "bin/forge-sfx", "bin/forge-audio", "heavy.sh"],
        ["moss-sfx"],
    )
    stage(
        "audio/music",
        audio or checks["ace-step"]["status"] != "absent",
        ["audio-venv", "ace-step-src", "bin/forge-music", "bin/forge-audio", "heavy.sh"],
        ["ace-step"],
    )
    disk_ok = free >= 55 * 1024**3
    ok = all(s["status"] != "broken" for s in stages.values()) and disk_ok
    return {
        "ok": ok,
        "verification": "Offline executable, import, package and asset checks. Byte counts cover the listed files. Inference and full hashes are outside this check.",
        "root": str(ROOT),
        "free_disk_bytes": free,
        "disk_floor_ok": disk_ok,
        "stages": stages,
        "checks": checks,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true")
    opts = parser.parse_args()
    info = diagnose()
    if opts.json:
        print(json.dumps(info, indent=2))
    else:
        print(
            f"Otto Forge  {info['root']}\nFree disk  {info['free_disk_bytes'] / 1024**3:.1f} GiB "
            f"({'ok' if info['disk_floor_ok'] else 'below 55 GiB'})"
        )
        for name, stage in info["stages"].items():
            reason = ", ".join(stage["unavailable"]) if stage["status"] in {"needs_models", "broken"} else ""
            plans = f"  body plans: {stage['body_plans']}" if "body_plans" in stage else ""
            print(f"  {name:<23} {stage['status']}" + (f" ({reason})" if reason else "") + plans)
        print("\nLocal checks")
        for name, check in info["checks"].items():
            size = f" ({check['bytes']:,} bytes)" if "bytes" in check else ""
            print(f"  {name:<27} {check['status']:<8} {check['detail']}{size}")
    return 0 if info["ok"] else 1


_diagnose_without_fit = diagnose


def diagnose():
    """The base report plus the fit stage (bin/forge-fit: Blender, the heavy lock and its sources)."""
    info = _diagnose_without_fit()
    checks, stages = info["checks"], info["stages"]
    sources = ["fit/cli.py", "fit/worker.py", "fit/fitlib.py", "fit/motion.py", "fit/measure.py", "fit/render.py",
               "fit/templates/pull_sword.json", "fit/templates/shoot_bow.json", "fit/templates/baton_pass.json",
               "animlib/retarget.py", "animlib/grounding.py"]
    checks["bin/forge-fit"] = probe(lambda: launcher("forge-fit", sources))
    runtime = ["blender", "heavy.sh", "bin/forge-fit"]
    bad = [k for k in runtime if checks.get(k, {}).get("status") != "ok"]
    stages["fit"] = {"status": "broken" if bad else "ready", "runtime": runtime, "assets": [], "unavailable": bad}
    info["ok"] = info["ok"] and not bad
    return info



_diagnose_without_motion = diagnose


def diagnose():
    """The base report plus the motion stage (bin/forge-motion: Kimodo venv and its pinned checkpoints)."""
    info = _diagnose_without_motion()
    checks, stages = info["checks"], info["stages"]
    import importlib.util  # forge/motion/cli.py: REPOS and HF_HOME, no heavy imports

    spec = importlib.util.spec_from_file_location("forge_motion_cli", ROOT / "forge/motion/cli.py")
    motion = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(motion)

    sources = ["motion/cli.py", "motion/generate.py", "animlib/retarget.py", "fit/cli.py"]
    checks["bin/forge-motion"] = probe(lambda: launcher("forge-motion", sources))
    py = ROOT / "forge/motion/.venv/bin/python"
    checks["motion-venv"] = result("ok", str(py)) if py.exists() else result("absent", "./install.sh --stage motion")
    kimodo = ROOT / "engines/kimodo"
    checks["kimodo-src"] = (result("ok", "pinned checkout") if (kimodo / ".git").exists()
                            and run("git", "-C", kimodo, "rev-parse", "HEAD") == motion.KIMODO_COMMIT
                            else result("absent", f"engines/kimodo @ {motion.KIMODO_COMMIT[:12]}"))
    assets = []
    for repo, rev in motion.REPOS.items():
        snap = motion.HF_HOME / "hub" / ("models--" + repo.replace("/", "--")) / "snapshots" / rev
        weights = list(snap.glob("*.safetensors"))
        key = "motion:" + repo.split("/")[1]
        checks[key] = (result("ok", repo, bytes=sum(w.stat().st_size for w in weights)) if weights
                       else result("absent", repo + (" (gated: accept Meta's licence on the model page, then hf auth login)"
                                                     if repo.startswith("meta-llama") else "")))
        assets.append(key)
    runtime = ["blender", "heavy.sh", "bin/forge-motion", "motion-venv", "kimodo-src"]
    unavailable = [k for k in runtime + assets if checks[k]["status"] != "ok"]
    state = ("not_installed" if checks["motion-venv"]["status"] != "ok"
             else "broken" if any(checks[k]["status"] != "ok" for k in runtime)
             else "needs_models" if unavailable else "ready")
    stages["motion"] = {"status": state, "runtime": runtime, "assets": assets, "unavailable": unavailable}
    info["ok"] = info["ok"] and state != "broken"
    return info

if __name__ == "__main__":
    sys.exit(main())
