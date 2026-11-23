"""forge: one CLI command per game-asset stage, over the AssetFurnace engine (pinned v0.6.0).

    forge generate  --prompt TEXT [--image PATH] [--kind character|prop] [--quality draft|standard|high]
                    [--back-image PATH] [--source raw|finish] [--hero-face] [--poly-budget N] [--height-m H | --longest-side-m L]
                    [--concept-only | --skip-gameready] [--concept-server URL] --output PATH
    forge gameready --mesh PATH [--poly-budget N] [--height-m H | --longest-side-m L] --output PATH
    forge rig       --mesh PATH [--hand-intake LANDMARKS.json] --output PATH
    forge animate   --mesh PATH [--clips idle,walk,run] --output PATH
    forge library   --prompt TEXT [--kind prop|character|scenery|any] [--min-score S] --output PATH
    forge library   --list-kinds
    forge concept-server [--port 7831]   (warm FLUX for generate --concept-server URL; stops on SIGTERM)
    forge pose      --mesh RIGGED --pose stand|sit|kneel|crouch|lie|curl[,hold-out] [--seat-height H]
                    [--look-at x,y,z] [--hand left|right] --output PATH   (docs/pose.md)

Otto runs one argv shape for every stage:
    forge STAGE --mesh M --image I --prompt P --output O --poly-budget N --seed S
so every stage accepts all of them, and an empty string means "not given".

Each command writes exactly --output plus <output>.manifest.json (animate on a non-humanoid body also writes
<output>.fit.json, its per-leg measurements; the manifest keeps shallow per-clip summaries). Progress goes to stderr as one
JSON object per line; on failure the last stderr line is {"error": "..."} and the exit code is 1.
Set FORGE_KEEP_WORK=1 to retain intermediates and child logs under out/work/. Model runs take the shared
2-slot lock (tools/heavy.sh). SIGTERM/SIGINT kill the running child's whole process group.
"""

from __future__ import annotations

import argparse
import base64
import json
import math
import os
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

from forge import bodyplan, glbstats, versions
from forge.quality import anatomy, eyes, fabric, flag, package, radial, vehicle, wheels

ROOT = Path(__file__).resolve().parents[1]
AF = ROOT / "engines" / "assetfurnace"
AF_PY = AF / ".venv" / "bin" / "python"
SKINTOKENS = AF / "vendor" / "SkinTokens"
ST_PY = SKINTOKENS / ".venv" / "bin" / "python"
SD_CLI = AF / "vendor" / "sdcpp" / "sd-cli"
SD_SERVER = AF / "vendor" / "sdcpp" / "sd-server"
UPSCALER_WEIGHTS = AF / "vendor/realesrgan/RealESRGAN_x4plus.pth"
HEAVY = ROOT / "tools" / "heavy.sh"
MODELS = ROOT / "models"
ENV = {
    "HF_HOME": str(MODELS / "hf"),
    "ASSETFURNACE_MODELS": str(MODELS / "af"),
    "U2NET_HOME": str(MODELS / "af" / "u2net"),
    # AssetFurnace's anonymous usage counts and news fetch: off.
    "I3D_NO_USAGE": "1", "DO_NOT_TRACK": "1", "I3D_NO_NOTICES": "1",
    "HF_HUB_DISABLE_TELEMETRY": "1", "HF_HUB_OFFLINE": "1",
}
FLUX_FILES = {  # sd-cli weights, as AssetFurnace's FLUX engine names them (Q8_0)
    "diffusion_model": ("models--leejet--FLUX.2-klein-4B-GGUF", "flux-2-klein-4b-Q8_0.gguf"),
    "llm": ("models--unsloth--Qwen3-4B-GGUF", "Qwen3-4B-Q8_0.gguf"),
    "vae": ("models--Comfy-Org--Flux-2-Klein-4B", "flux2-vae.safetensors"),
}
SUFFIX = "plain light grey background, soft even studio lighting, whole subject in view"
# Added for a cloud picture model (Codex, Gemini) after the same wording: they follow it literally, and
# without this they sometimes add a caption, a frame or a second view, or crop the feet.
CLOUD_SUFFIX = ("Make exactly one square picture of this one subject alone, all of it in view with a small "
                "margin, with no text, labels, watermark, border or second view.")


class Failed(Exception):
    pass


class Cancelled(Exception):
    pass


class Run:
    """One forge command: its work folder, the child it is running, and what goes in the manifest."""

    def __init__(self, command: str, output: Path, seed: int | None = None):
        self.command = command
        self.output = output.resolve()
        work_root = ROOT / "out" / "work"
        work_root.mkdir(parents=True, exist_ok=True)
        self.work = Path(tempfile.mkdtemp(dir=work_root, prefix=self.output.stem + "-", suffix=f"-{os.getpid()}"))
        self.keep_work = os.environ.get("FORGE_KEEP_WORK") == "1"
        self.spawning = False
        self.cancelled = False
        self.child: subprocess.Popen | None = None
        self.manifest: dict = {"command": command, "output": str(self.output), "seed": seed,
                               "work_dir": str(self.work), "work_retained": self.keep_work, "tools": [], "models": [],
                               "licences": [], "timings": {}}

    def progress(self, stage: str, pct: int, message: str) -> None:
        print(json.dumps({"stage": stage, "pct": pct, "message": message}), file=sys.stderr, flush=True)

    def run(self, stage: str, argv: list, cwd: Path = AF, env: dict | None = None,
            heavy: bool = True) -> None:
        """Run one child in its own process group, log to work/<stage>.log, record time + peak RSS."""
        argv = [str(a) for a in argv]
        if heavy:
            argv = [str(HEAVY), *argv]
        log = self.work / f"{stage}.log"
        started = time.time()
        with log.open("w") as out:
            out.write("$ " + " ".join(argv) + "\n")
            out.flush()
            # Defer cancellation until the new process group has an owner that can stop it.
            self.spawning = True
            try:
                self.child = subprocess.Popen(argv, cwd=cwd, stdout=out, stderr=subprocess.STDOUT,
                                              env={**os.environ, **ENV, **(env or {})},
                                              start_new_session=True)
            finally:
                self.spawning = False
            if self.cancelled:
                raise Cancelled
            _, status, usage = os.wait4(self.child.pid, 0)
            self.child.returncode = os.waitstatus_to_exitcode(status)
        code, self.child = self.child.returncode, None
        self.manifest["timings"][stage] = {
            "seconds": round(time.time() - started, 1),
            "peak_rss_mb": round(usage.ru_maxrss / 1048576),  # bytes on macOS, as /usr/bin/time -l
            "command": " ".join(argv)}
        if code != 0:
            lines = [ln.strip() for ln in log.read_text(errors="replace").splitlines() if ln.strip()]
            detail = lines[-1] if lines else "no output"
            for line in reversed(lines):
                if not line.startswith("{"):
                    continue
                try:
                    event = json.loads(line)
                except ValueError:
                    continue
                if isinstance(event, dict) and event.get("error"):
                    detail = event["error"]
                    break
            raise Failed(f"{stage} failed (exit {code}): {detail}")

    def cancel(self, *_):
        if not self.cancelled:
            self.cancelled = True
            if not self.spawning:
                raise Cancelled

    def stop_child(self):
        if self.child is None:
            return
        try:
            os.killpg(self.child.pid, signal.SIGTERM)
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                self.child.poll()
                os.killpg(self.child.pid, 0)
                time.sleep(.05)
            os.killpg(self.child.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):  # macOS may report EPERM for a zombie group
            pass
        finally:
            self.child.wait()

    def uses(self, *names: str) -> None:
        for name in names:
            entry = versions.PARTS[name]
            target = "models" if entry.get("kind") == "model" else "tools"
            if entry not in self.manifest[target]:
                self.manifest[target].append(entry)
            if entry["licence"] not in self.manifest["licences"]:
                self.manifest["licences"].append(entry["licence"])

    def finish(self, glb: Path, *, draft=False, **extra) -> None:
        """Move the result to --output (the only file written there) and write the manifest."""
        doc, binary = glbstats.read_glb(glb)
        # The ordinary Smart path sends its draft through a separate bake. That
        # bake can change UVs, textures and frame; finish the delivered mesh.
        finish_appearance = (self.command in {"generate", "gameready"} and not draft
                             and not self.manifest.get("reused_gameready"))
        prompt = self.manifest.get("prompt")
        wheelbase = None
        count = wheels.requested(self.manifest.get("prompt"))
        if finish_appearance and count:
            wheelbase = wheels.align(glb, doc, count)
            if wheelbase:
                self.manifest["wheel_contacts"] = wheelbase
        vehicle_frame = None
        if finish_appearance and not count and vehicle.requested(self.manifest.get("prompt")):
            vehicle_frame = vehicle.align(glb, doc)
            if vehicle_frame:
                self.manifest["vehicle_frame"] = vehicle_frame
        mantle = None
        if finish_appearance and anatomy.radial_count(self.manifest.get("prompt") or ""):
            mantle = radial.align(glb, doc)
            if mantle:
                self.manifest["radial_orientation"] = mantle
        mounted = None
        if finish_appearance and flag.requested(self.manifest.get("prompt")):
            mounted = flag.mount(glb, doc, binary)
            if mounted:
                binary, self.manifest["flag_mount"] = mounted
        cloth_frame = fabric.align(glb, doc) if finish_appearance and fabric.laid_flat(prompt) else None
        if cloth_frame:
            self.manifest["fabric_frame"] = cloth_frame
        dimensions = package.size_m(doc, binary)
        target = self.manifest.get("size_request")
        stamp = isinstance(self.manifest.get("gameready"), dict) and not self.manifest.get("reused_gameready")
        if target:
            span = dimensions[1] if target["axis"] == "height" else max(dimensions)
            if span <= 0:
                raise Failed("Cannot resize an empty or zero-span display mesh")
            factor = target["metres"] / span
            binary = package.scale_metres(doc, binary, factor)
            dimensions = package.size_m(doc, binary)
        if stamp:
            doc["asset"].setdefault("extras", {})["otto_forge_game_ready"] = True
        patterned = False
        if finish_appearance and fabric.requested(prompt):
            finished = fabric.apply(doc, binary, prompt, self.manifest.get("seed") or 0)
            if finished is not None:
                binary, patterned = finished, True
                self.manifest["surface_pattern"] = "patchwork"
        peeking = self.command == "pose" and "peek" in self.manifest.get("pose", {}).get("names", [])
        if (finish_appearance or peeking) and eyes.requested(prompt, peeking=peeking):
            finished = eyes.apply(doc, binary)
            if finished is not None:
                binary, patterned = finished, True
                self.manifest["eye_finish"] = "existing pale irises in shadow"
            elif peeking:
                from forge.quality import eye_reflections
                finished = eye_reflections.apply(glb, doc, binary)
                if finished is not None:
                    binary, patterned = finished, True
                    self.manifest["eye_finish"] = "catchlights on two existing glossy eyes"
        if stamp or target or patterned or wheelbase or vehicle_frame or mantle or mounted or cloth_frame:
            glb = self.work / "delivery.glb"
            write_glb(doc, binary, glb)
        self.manifest["size_m"] = [round(v, 6) for v in dimensions]
        stats = glbstats.stats(glb)
        self.output.parent.mkdir(parents=True, exist_ok=True)
        if glb != self.output:
            shutil.copyfile(glb, self.output)
        self.manifest.update(triangles=stats["triangles"], vertices=stats["vertices"],
                             bytes=stats["bytes"], **extra)
        if stats["lods"]:
            self.manifest["lods"] = stats["lods"]
        if stats["bones"]:
            self.manifest["bones"] = stats["bones"]
        if stats["clips"]:
            self.manifest["clips"] = [{"name": c["name"], "duration_s": c["duration_s"]}
                                      for c in stats["clips"]]
        Path(f"{self.output}.manifest.json").write_text(json.dumps(self.manifest, indent=2) + "\n")
        self.progress(self.command, 100, f"wrote {self.output}")


def hf_file(repo_dir: str, name: str) -> Path:
    found = sorted((MODELS / "hf" / "hub" / repo_dir / "snapshots").glob(f"*/**/{name}"))
    if not found:
        raise Failed(f"missing weight {repo_dir}/{name}; run AssetFurnace's bootstrap_flux_image.py")
    return found[0]


BODIES = {   # --body: a picture the 3D step and the rigger read as that body
    "humanoid": "full body, front view, standing in an A-pose, arms straight and angled down about 45 degrees "
                "with a clear gap from the body, hands open, palms facing down, empty hands, feet slightly apart, "
                "fingers spread with visible background between the tips",
    "quadruped": "full body, standing on all four legs, seen side-on turned three-quarters toward the camera, "
                 "all four legs apart and clearly visible with gaps between them, tail held out behind and clear "
                 "of the legs",
    "bird": "full body, standing, seen side-on turned three-quarters toward the camera, wings folded against "
            "the body but distinct, both legs and feet visible, tail clear of the legs",
    "legless": "full body, its whole length in view, laid out in a loose gentle curve that does not cross "
               "itself, seen side-on turned three-quarters toward the camera",
    "other": "full body, the whole body in view, limbs held apart from the body and from each other, seen "
             "turned three-quarters toward the camera",
}
FACE = "eyes open, slightly larger, clearly defined eyes with visible eyelids, relaxed neutral expression, solid hair masses"


def shell_resonators(prompt):
    """A shell is the drum body. A decorative shell or a shell cymbal is a separate part."""
    shell = r"(?:sea[ -]?shells?|scallops?|conch(?:es)?|nautilus)"
    for clause in re.split(r"[,.;:]|\bwith\b", (prompt or "").lower()):
        if re.search(rf"\b{shell}[ -]drums?\b", clause):
            return True
        material = re.search(r"\bdrums?(?:\s+(?:kit|set|bodies))?\s+"
                             r"(?:(?:made|built|carved|formed|fashioned|assembled|crafted)\s+)?"
                             r"(?:out of|of|from)\s+|\bdrumheads?\s+(?:stretched\s+)?over\s+", clause)
        if material and re.search(rf"\b(?:spiral shells?|{shell})\b", clause[material.end():]):
            return True
    return False


def prompt_for(prompt: str, kind: str | None, body: str | None = None, face: bool = False,
               *, body_view: str | None = None) -> str:
    if body is not None:   # a character's body words, with a human face only for a Hero Face run
        words = (body_view or anatomy.description(prompt, body) or BODIES[body]) + (f", {FACE}" if face else "")
        return (f"{prompt}, {words}, plain light grey background, soft even studio lighting, softly lit 3D "
                f"character render, the whole {'figure' if body in ('humanoid', 'other') else 'animal'} "
                "filling the frame with a small margin around it")
    if kind != "character":
        terms = set(re.findall(r"[a-z]+", prompt.lower()))
        if shell_resonators(prompt):
            form = ("A cluster of large spiral sea shells, the coiled sides in full view."
                    if terms & {"spiral", "coiled", "conch", "nautilus"} else
                    "A cluster of natural ribbed sea shells in different sizes, each opening upward.")
            prompt = (f"{form} Each fluted shell body is the resonator. A pale drumhead laces directly "
                      f"across its open lip. {prompt}")
        count = wheels.requested(prompt)
        if count:
            prompt += (f". The wheelbase has {count//2} distinct axles crossing under the chassis, each ending "
                       "in one visible tyre per side. Keep the ground level and show the open space between tyres.")
    if kind is None:
        return f"{prompt}, {SUFFIX}"
    recipes = json.loads((AF / "image_to_3dlab" / "prompt_recipes.json").read_text())
    wording = recipes["goals"][kind]["wording"]
    if kind == "character":
        wording += (", softly lit 3D character render, relaxed neutral expression, solid hair masses, "
                    "fingers spread with visible background between the tips, one figure filling "
                    "the frame with a small margin above the hair and below the soles")
    return f"{prompt}, {wording}"


class Stop(Exception):
    pass


def concept_server(port: int) -> int:
    """Keep FLUX warm for generate --concept-server: sd-server with the weights and sampler the
    concept step uses, under the heavy lock. One JSON line on stdout once it answers, then serve
    until SIGTERM/SIGINT, which stop the whole server process group."""
    weights = {key: hf_file(*spec) for key, spec in FLUX_FILES.items()}
    argv = [HEAVY, SD_SERVER, "--diffusion-model", weights["diffusion_model"], "--vae", weights["vae"],
            "--llm", weights["llm"], "--cfg-scale", "1.0", "--sampling-method", "euler", "--steps", "4",
            "--diffusion-fa", "--listen-ip", "127.0.0.1", "--listen-port", port]
    url = f"http://127.0.0.1:{port}"
    log = tempfile.NamedTemporaryFile(prefix=f"concept-server-{port}-", suffix=".log", delete=False)

    def stop(*_):
        raise Stop

    previous = {sig: signal.signal(sig, stop) for sig in (signal.SIGTERM, signal.SIGINT)}
    child = None
    try:
        child = subprocess.Popen([str(a) for a in argv], cwd=AF, stdout=log, stderr=subprocess.STDOUT,
                                 env={**os.environ, **ENV}, start_new_session=True)
        ready = False
        while child.poll() is None:
            if not ready:
                try:
                    with urllib.request.urlopen(url + "/v1/models", timeout=2):
                        ready = True
                    print(json.dumps({"ready": True, "url": url}), flush=True)
                except OSError:
                    pass
            time.sleep(0.5)
        lines = [ln for ln in Path(log.name).read_text(errors="replace").splitlines() if ln.strip()]
        raise Failed(f"sd-server exited with code {child.returncode}: {lines[-1] if lines else 'no output'}")
    except Stop:
        return 0
    finally:
        signal.pthread_sigmask(signal.SIG_BLOCK, previous.keys())
        if child is not None and child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGTERM)
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
            except ProcessLookupError:
                child.wait()
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        signal.pthread_sigmask(signal.SIG_UNBLOCK, previous.keys())
        log.close()
        os.unlink(log.name)


def need_mesh(args) -> Path:
    if not args.mesh:
        raise Failed("--mesh is required for this stage")
    mesh = Path(args.mesh).resolve()
    if not mesh.is_file():
        raise Failed(f"--mesh not found: {mesh}")
    return mesh


def draw_concept(args, run, image, text, seed):
    guide = anatomy.structure_reference(args.prompt or "", args.body)
    if guide and args.body == "legless":
        text = ("One snow figure in one vertical stack, with one head above one torso above one base. "
                "Match the arrangement in the reference. " + text)
    elif guide:
        text += (". Use the reference for the leg placement: preserve all six separately visible "
                 "feet, with three pairs of articulated legs. Render the subject as described.")
    elif not args.body and args.kind != "character" and wheels.three_axle_vehicle(args.prompt):
        guide = Path(wheels.__file__).parent / "guides/three-axle-vehicle.png"
        text += (". Match the wheel placement in the reference. Each of the three axles carries two "
                 "separate tires, visible beneath the chassis. Render the body and materials as described.")
    elif (not args.body and args.kind != "character" and shell_resonators(args.prompt)
          and re.search(r"\b(?:kit|set)\b", args.prompt, re.I)):
        # A scallop cymbal on a spiral-shell drum does not change the resonator.
        coiled = re.search(r"\b(?:spiral|coiled|conch|nautilus)\b", args.prompt, re.I)
        bivalve = re.search(r"\b(?:scallops?|clams?|bivalves?)\b", args.prompt, re.I)
        if coiled or not bivalve:
            guide = Path(__file__).parent / "quality/guides/coiled-shell-percussion.png"
        elif not re.match(r"(?:[ -]+shell)?[ -]+cymbals?\b", args.prompt[bivalve.end():], re.I):
            guide = Path(__file__).parent / "quality/guides/scallop-shell-percussion.png"
        if guide:
            text += (". Match the separate shell bodies of the reference. Stretch a separate drumhead "
                     "across the opening of each shell. Follow the description for colours, "
                     "surface texture and fittings.")
    steps = 8 if guide else 4
    if guide:
        run.manifest["concept_reference"] = guide.name
        run.manifest["concept_prompt"] = text
    if args.concept_server:
        # A warm sd-server started with the same weights and flags (docs/engine.md): the same
        # pixels as sd-cli for the same seed, without reloading FLUX and Qwen3 for each picture.
        started = time.time()
        body = {"prompt": text, "negative_prompt": "", "width": 1024, "height": 1024, "steps": steps,
                "cfg_scale": 1.0, "seed": seed, "sampler_name": "euler", "batch_size": 1}
        if guide:
            body["extra_images"] = [base64.b64encode(guide.read_bytes()).decode("ascii")]
        request = urllib.request.Request(args.concept_server.rstrip("/") + "/sdapi/v1/txt2img",
                                         json.dumps(body).encode(), {"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=900) as reply:
                image.write_bytes(base64.b64decode(json.loads(reply.read())["images"][0]))
        except (OSError, ValueError, KeyError, IndexError) as exc:
            raise Failed(f"concept server {args.concept_server}: {exc}")
        run.manifest["timings"]["concept"] = {"seconds": round(time.time() - started, 1),
                                              "server": args.concept_server}
    else:
        weights = {key: hf_file(*spec) for key, spec in FLUX_FILES.items()}
        # The arguments AssetFurnace's studio passes for its FLUX engine (viewer/image_api.py)
        run.run("concept", [SD_CLI, "--diffusion-model", weights["diffusion_model"],
                            "--vae", weights["vae"], "--llm", weights["llm"], "-p", text,
                            *(["-r", guide] if guide else []),
                            "--cfg-scale", "1.0", "--sampling-method", "euler", "--steps", str(steps),
                            "-W", "1024", "-H", "1024", "--seed", seed, "--diffusion-fa",
                            "-o", image])


def generate(args, run: Run) -> None:
    if not (args.prompt or args.image):
        raise Failed("give --prompt or --image")
    if args.format != "glb":
        raise Failed(f"--format {args.format}: only glb is supported")
    image = Path(args.image).resolve() if args.image else None
    if image is not None and not image.is_file():
        raise Failed(f"--image not found: {image}")
    back = Path(args.back_image).resolve() if args.back_image else None
    if back and not back.is_file():
        raise Failed(f"--back-image not found: {back}")
    if args.quality not in ("draft", "standard", "high"):
        raise Failed("--quality must be draft, standard or high")
    # Let the engine select eight steps on patched builds and twelve on older installs.
    steps = {"draft": 6, "standard": None, "high": 12}[args.quality]
    if args.pixal_steps is not None:
        steps = int(args.pixal_steps)
        if not 1 <= steps <= 50:
            raise Failed("--pixal-steps must be 1 to 50")
    guidance = float(args.structure_guidance)
    if not math.isfinite(guidance) or guidance < 0:
        raise Failed("--structure-guidance must be a finite nonnegative number")
    if args.source not in (None, "raw", "finish"):
        raise Failed("--source must be raw or finish")
    if args.concept_only and (image or not args.prompt):
        raise Failed("--concept-only draws a picture from --prompt; give --prompt and no --image")
    if args.concept_only and args.skip_gameready:
        raise Failed("choose one of --concept-only and --skip-gameready")
    if (args.source or "raw") == "raw" and not args.hero_face and not args.concept_only \
            and not args.skip_gameready and not GAMEREADY.is_file():
        raise Failed("raw generation requires bin/forge-gameready to reduce the Pixal3D mesh")
    if args.hero_face and args.source == "raw":
        raise Failed("--hero-face produces a finished head graft; omit --source raw to use it")
    character_raw = args.kind == "character" and not args.hero_face and args.source != "finish"
    sharpen = character_raw and args.quality != "draft" and not args.skip_gameready and not args.concept_only
    if sharpen and not UPSCALER_WEIGHTS.is_file():
        raise Failed("standard and high characters need Real-ESRGAN x4plus (67 MB); "
                     "run scripts/bootstrap_upscaler.py in AssetFurnace or use --quality draft")
    if args.hero_face:
        if args.kind == "prop":
            raise Failed("--hero-face requires a character")
        run.run("check_hero", [AF_PY, "-c",
                "import sys; sys.path.insert(0,'scripts'); import hero_route as h; "
                "m=h.missing_parts(h.needed_parts()); "
                "sys.exit('Hero Face needs: '+', '.join(m) if m else 0)"], heavy=False)
    if back:
        # Let the engine check its own checkout and weight inventory before running FLUX.
        run.run("check_back_view", [AF_PY, "-c",
                "import sys; sys.path.insert(0,'scripts'); import pixal3d_generate as p; "
                "s=p.multiview_readiness(p.CLI,p.MV_MODELS); "
                "sys.exit(0 if s['ready'] else 'install front/back weights using "
                "scripts/bootstrap_pixal3d.py --multiview (5.9 GB)')"], heavy=False)
    run.manifest["prompt"] = args.prompt
    if image is None:
        run.progress("concept", 2, "drawing the concept picture (FLUX.2 klein 4B)")
        image = run.work / "concept.png"
        text = prompt_for(args.prompt, "character" if args.hero_face else args.kind, args.body, args.hero_face)
        run.manifest["concept_prompt"] = text
        for attempt in range(2):
            seed = (args.seed + attempt) % (2**32)
            started = time.monotonic()
            draw_concept(args, run, image, text, seed)
            base_matte = None
            if anatomy.needs_matte(args.prompt or "", args.body):
                base_matte = run.work / "shape-outline.png"
                run.run("check_base", [AF_PY, ROOT / "forge/quality/matte.py", image, base_matte],
                        env={"PYTHONPATH": os.pathsep.join(filter(None, (str(AF), os.environ.get("PYTHONPATH"))))})
            check = anatomy.inspect(image, args.body, args.prompt or "", base_matte)
            if check["status"] != "not_applicable":
                run.manifest.setdefault("concept_checks", []).append({
                    **check, "seed": seed, "seconds": round(time.monotonic() - started, 2)})
            if check["status"] != "rejected":
                break
            reason = check["reason"]
            if attempt:
                raise Failed(f"Concept anatomy still mismatches the requested body after two drawings: {reason}")
            run.progress("concept", 3, reason + " Redrawing before making the shape.")
            correction = anatomy.redraw_description(args.prompt, args.body, check) or BODIES[args.body]
            text = prompt_for(args.prompt, args.kind, args.body, args.hero_face,
                              body_view=f"{correction} Make the outline and its points of contact clearly visible.")
            run.manifest["concept_prompt"] = text
        run.uses("sd.cpp", "flux2-klein-4b", "qwen3-4b", "flux2-vae")
    run.manifest["input_image"] = str(image)
    if args.concept_only:
        # The picture alone, so a person can look at it before any 3D is built. A later
        # generate --image OUTPUT --seed S builds the same model a one-go run would.
        run.output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(image, run.output)
        run.manifest["concept_only"] = True
        Path(f"{run.output}.manifest.json").write_text(json.dumps(run.manifest, indent=2) + "\n")
        run.progress("concept", 100, f"wrote {run.output}")
        return

    run.progress("matte", 15, "preparing the subject matte")
    matted = run.work / "matted.png"
    run.run("matte", [AF_PY, ROOT / "forge/quality/matte.py", image, matted],
            env={"PYTHONPATH": os.pathsep.join(filter(None, (str(AF), os.environ.get("PYTHONPATH"))))})
    method = json.loads(matted.with_suffix(".json").read_text())["method"]
    run.manifest["matte_method"] = method
    run.uses("assetfurnace")
    if method == "birefnet-lite-lean":
        run.uses("birefnet-lite")

    run.progress("generate", 20, "building the 3D model (Pixal3D)")
    raw = run.work / "raw.glb"
    build = [AF_PY, "scripts/pixal3d_generate.py", matted, raw, "--seed", args.seed,
             "--res", 1024, "--gss", guidance, "--gsh", 7.5]
    if steps is not None:
        build += ["--steps", steps]
    if back:
        build += ["--back", back]
    run.run("generate", build)
    run.uses("pixal3d.cpp", "dinov3")
    if back:
        run.manifest["models"].append({
            **versions.PARTS["pixal3d-sv-q8"],
            "name": "raven38/pixal3d-q8_0-v1 (Pixal3D multi-view flows + decoders)",
            "version": "1c4f4bd",
        })
        licence = versions.PARTS["pixal3d-sv-q8"]["licence"]
        if licence not in run.manifest["licences"]:
            run.manifest["licences"].append(licence)
    else:
        run.uses("pixal3d-sv-q8")
    record = json.loads(raw.with_suffix(".provenance.json").read_text())
    seed_line = re.search(r"generating model with seed (\d+)", (run.work / "generate.log").read_text(errors="replace"))
    run.manifest["generation"] = {
        "sampling_seed": int(seed_line[1]) if seed_line else None,
        "quality": args.quality, **record["parameters"], "quantization": "Q8_0",
        "texture_guidance": 1.0, "raw_atlas_px": 4096,
        "views": "front + back" if back else "front", "back_image": str(back) if back else None,
        "matted_front_sha256": record["input"]["sha256"],
        "back_sha256": record.get("back_input", {}).get("sha256"),
        "raw_sha256": record["output"]["sha256"],
    }

    source = "hero" if args.hero_face else args.source or "raw"
    run.manifest["source"] = source
    stage_input = raw if source == "raw" else run.work / "finished.glb"
    run.manifest["generation"]["pixel_match"] = None
    run.manifest["generation"]["hero_face"] = args.hero_face
    if args.hero_face:
        hero_finish(args, run, image, back, raw, stage_input)
    elif source == "finish":
        run.progress("finish", 80, f"AssetFurnace Finish to {args.poly_budget} faces")
        finish = [AF_PY, "scripts/retopo_repaint.py", raw, matted, stage_input, "--skip-paint",
                  "--faces", args.poly_budget, "--steps-dir", run.work / "finish"]
        if raw.with_suffix(".svviews").is_dir():
            finish += ["--views", raw.with_suffix(".svviews")]  # Pixel Match
        if raw.with_suffix(".post.bin").is_file():
            finish += ["--native-dump", raw.with_suffix(".post.bin")]
        if args.kind == "prop":
            finish += ["--upright"]  # Pixal3D keeps the picture's camera tilt; stand props on the floor
        run.run("finish", finish)
        finish_record = json.loads(stage_input.with_suffix(".retopo-repaint.json").read_text())
        run.manifest["generation"]["pixel_match"] = finish_record.get("photo")
    run.uses("blender")
    run.manifest["poly_budget"] = args.poly_budget
    if args.skip_gameready:
        # A draft to look at: the generator's own mesh. forge gameready on it later gives the
        # same LODs and collision a one-go run makes (props; characters lose detail + projection).
        run.manifest["gameready"] = "skipped (--skip-gameready); run forge gameready on this file"
        keep = None
        if character_raw:
            # The head/hand detail and the face projection need the generator's calibrated views
            # (about 1 MB). Keep them beside the draft so forge gameready finishes it as one go would.
            keep = Path(f"{run.output}.finish")
            shutil.copytree(raw.with_suffix(".svviews"), run.work / "finish" / "raw.svviews")
            run.manifest["finish"] = {"kind": "character", "quality": args.quality, "inputs": keep.name,
                                      "size_request": run.manifest.pop("size_request", None)}
            target = run.manifest["finish"]["size_request"]
            if target:
                # Size the draft on a new root node, not in its vertices: the finish removes the node and
                # gets the generator's exact mesh back (game-ready reacts to one-ulp vertex changes).
                doc, binary = glbstats.read_glb(raw)
                dims = package.size_m(doc, binary)
                factor = target["metres"] / (dims[1] if target["axis"] == "height" else max(dims))
                scene = doc["scenes"][doc.get("scene", 0)]
                doc["nodes"].append({"name": "draft_size", "scale": [factor] * 3, "children": scene["nodes"]})
                scene["nodes"] = [len(doc["nodes"]) - 1]
                stage_input = run.work / "draft.glb"
                write_glb(doc, binary, stage_input)
        run.finish(stage_input, draft=True)
        if keep:
            shutil.rmtree(keep, ignore_errors=True)
            shutil.move(run.work / "finish", keep)
        return

    pack = run.work / "pack" if character_raw else None
    game = gameready(run, stage_input, args.poly_budget, character_pack=pack)
    if sharpen:
        game = sharpen_character(run, game, pack, raw.with_suffix(".svviews"))
    elif character_raw:
        run.manifest["generation"]["projection"] = {"applied": False, "reason": "draft"}
    run.finish(game)


def uses_upscaler(run):
    run.manifest["models"].append({"kind": "model", "name": "Real-ESRGAN x4plus",
                                   "version": "v0.1.0/RealESRGAN_x4plus.pth", "licence": "BSD-3-Clause"})
    run.manifest["licences"].append("BSD-3-Clause")


def hero_finish(args, run, image, back, raw, finished):
    run.progress("hero", 65, "finishing the body and reconstructing the head from its own crop")
    folder = run.work / "hero"
    command = [AF_PY, "scripts/hero_route.py", image, "--body", raw,
               "--body-views", raw.with_suffix(".svviews"), "--body-dump", raw.with_suffix(".post.bin"),
               "--body-record", raw.with_suffix(".provenance.json"), "--seed", args.seed,
               "--out", folder, "--result", finished]
    if back:
        command += ["--back", back]
    run.run("hero", command)
    run.uses("pixal3d-sv-q8", "skintokens", "skintokens-weights", "blender-bpy")
    uses_upscaler(run)
    hero = json.loads(finished.with_suffix(".provenance.json").read_text())
    head = json.loads((folder / "head_raw.provenance.json").read_text())
    generation = run.manifest["generation"]
    generation["hero"] = {**hero["parameters"], "head_sampling": head["parameters"], "notes": hero["unsure"]}
    generation["pixel_match"] = {
        part: json.loads((folder / f"{part}.retopo-repaint.json").read_text())["photo"]
        for part in ("head", "body")
    }


GAMEREADY = ROOT / "bin" / "forge-gameready"


def gameready(run: Run, mesh: Path, poly_budget: int, *, character_pack: Path | None = None) -> Path:
    """CALL SITE for the game-ready stage (bin/forge-gameready, forge/gameready, owned by another
    agent): decimate/UV/bake, LODs, collision hulls, as one GLB."""
    if not GAMEREADY.is_file():
        run.manifest["gameready"] = "not run: bin/forge-gameready is not installed"
        return mesh
    out = run.work / "gameready.glb"
    argv = [GAMEREADY, mesh, "--output", out, "--tris", poly_budget, "--lods", "3", "--collision"]
    detail = ""
    if character_pack is not None:
        argv.extend(["--detail", "head,hands", "--pack", character_pack])
        detail = ", preserving detail on the head and hands"
    run.progress("gameready", 90, f"Preparing {poly_budget} triangles, three LODs and collision{detail}")
    run.run("gameready", argv, cwd=ROOT)
    theirs = Path(f"{out}.manifest.json")
    if theirs.is_file():
        run.manifest["gameready"] = json.loads(theirs.read_text())
    return out


def paint_display_mesh(lod0: Path, work: Path) -> Path:
    """Pixel Match counts every primitive, including unreferenced meshes and UCX hulls."""
    doc, binary = glbstats.read_glb(lod0)
    nodes = [node for node in doc["nodes"] if "mesh" in node]
    display = [node for node in nodes if not node.get("name", "").startswith("UCX_")]
    if len(display) != 1:
        raise Failed(f"expected one display mesh in {lod0.name}, found {len(display)}")
    if len(display) == len(nodes) and len(doc["meshes"]) == 1:
        return lod0
    mesh = doc["meshes"][display[0]["mesh"]]
    for node in nodes:
        if node is display[0]:
            node["mesh"] = 0
        else:
            del node["mesh"]
    doc["meshes"] = [mesh]
    output = work / "display_LOD0.glb"
    write_glb(doc, binary, output)
    return output


def sharpen_character(run: Run, game: Path, pack: Path, views: Path) -> Path:
    lod0 = pack / f"{game.stem}_LOD0.glb"
    if not lod0.is_file() or not (views / "transforms.json").is_file():
        raise Failed("cannot sharpen the face: game-ready LOD0 or calibrated Pixal3D views are missing")
    display = paint_display_mesh(lod0, run.work)
    scaled = run.work / "views_4x"
    painted = run.work / "painted_LOD0.glb"
    final = run.work / "painted_game.glb"
    helper_env = {"PYTHONPATH": os.pathsep.join((str(AF), str(ROOT)))}
    run.progress("upscale_views", 94, "Sharpening the face with 4x concept images")
    run.run("upscale_views", [AF_PY, ROOT / "forge/quality/upscale_view.py", views, scaled], env=helper_env)
    run.progress("photo_paint", 96, "Painting the concept colours onto the display mesh")
    # The native painter scales these two feather widths by the calibrated image width.
    feather = {"facing_high": 0.85, "edge_px": 12, "cutout_edge": 16 / 768, "inner_edge": 16 / 768}
    paint = [AF_PY, "scripts/photo_paint.py", display, scaled, painted]
    for name, value in feather.items():
        paint.extend(("--" + name.replace("_", "-"), value))
    run.run("photo_paint", paint)
    run.progress("apply_colour", 99, "Applying the painted colour to every level of detail")
    run.run("apply_colour", [AF_PY, ROOT / "forge/quality/apply_colour.py", game, painted, final],
            env=helper_env, heavy=False)
    uses_upscaler(run)
    lines = (run.work / "photo_paint.log").read_text(errors="replace").splitlines()
    record = {"applied": True, "detail": "head,hands", "upscale": "Real-ESRGAN x4plus",
              "views": json.loads((scaled / "upscale.json").read_text()), "pack_lod0": lod0.name, "painted_mesh": display.name,
              "image": "shared base colour", "feather": feather, "reports": [line for line in lines if line.startswith("[photo-paint]")]}
    run.manifest["generation"]["projection"] = record
    run.manifest["generation"]["pixel_match"] = record["reports"]
    return final


def finish_character(args, run: Run, mesh: Path, prior: dict) -> None:
    """Finish a generate --kind character --skip-gameready draft the way the one-go run would have:
    game-ready with head/hand detail from the generator's own mesh, then the face projection."""
    plan = prior["finish"]
    inputs = mesh.parent / plan["inputs"]
    views = inputs / "raw.svviews"
    if not (views / "transforms.json").is_file():
        raise Failed(f"this character draft's finishing inputs are missing ({inputs}); generate it again")
    doc, binary = glbstats.read_glb(mesh)
    scene = doc["scenes"][doc.get("scene", 0)]
    if doc["nodes"][-1].get("name") == "draft_size":  # back to the generator's own frame and vertices
        scene["nodes"] = doc["nodes"].pop()["children"]
        mesh = run.work / "raw.glb"
        write_glb(doc, binary, mesh)
    for key in ("prompt", "concept_prompt", "input_image", "matte_method", "source", "tools", "models",
                "licences", "generation", "poly_budget"):
        if key in prior:
            run.manifest[key] = prior[key]
    if "size_request" not in run.manifest and plan.get("size_request"):
        run.manifest["size_request"] = plan["size_request"]
    pack = run.work / "pack"
    game = gameready(run, mesh, args.poly_budget, character_pack=pack)
    if plan["quality"] != "draft":
        game = sharpen_character(run, game, pack, views)
    else:
        run.manifest["generation"]["projection"] = {"applied": False, "reason": "draft"}
    run.finish(game)
    shutil.rmtree(inputs)


def pose_cmd(args, run: Run) -> None:
    from forge import pose as posing
    mesh = need_mesh(args)
    names = [n.strip() for n in (args.pose or "stand").split(",") if n.strip()]
    if "peek" in names and args.prompt:
        run.manifest["prompt"] = args.prompt
    def contact(text):
        if text is None:
            return None
        point = [float(v) for v in text.split(",")]
        if len(point) != 3 or not all(map(math.isfinite, point)):
            raise ValueError("contact must be x,y,z")
        return point
    seat = look = toward = other_toward = None
    try:
        if args.other_hold_toward:
            other_toward = [float(v) for v in args.other_hold_toward.split(",")]
            if len(other_toward) != 3 or not all(map(math.isfinite, other_toward)):
                raise ValueError
        if args.hold_toward:
            toward = [float(v) for v in args.hold_toward.split(",")]
            if len(toward) != 3 or not all(map(math.isfinite, toward)):
                raise ValueError
        if args.seat_height:
            seat = float(args.seat_height)
            if not math.isfinite(seat):
                raise ValueError
        if args.look_at:
            look = [float(v) for v in args.look_at.split(",")]
            if len(look) != 3 or not all(map(math.isfinite, look)):
                raise ValueError
    except ValueError:
        raise Failed("--seat-height is metres; --look-at and --hold-toward are x,y,z in metres (x left, y up, z ahead)")
    if args.hand not in (None, "left", "right"):
        raise Failed("--hand is left or right")
    out = run.work / "posed.glb"
    started = time.time()
    try:
        if bool(args.strike_left) != bool(args.strike_right):
            raise ValueError("striking requires two contact points")
        targets = [contact(args.strike_left),contact(args.strike_right)] if args.strike_left else None
        report = posing.make(mesh, out, names, seat, look, args.hand, args.hold_style, toward,
                             wind_mesh=args.wind_instrument, wind_scale=float(args.wind_scale),
                             other_style=args.other_hold_style, other_toward=other_toward,
                             fingerboard=contact(args.fingerboard), strings=contact(args.strings), plant=contact(args.plant_at), strike=targets,
                             swimming_fish=args.swimming_fish,
                             progress=lambda pct, message: run.progress("pose", pct, message))
    except (LookupError, ValueError) as exc:   # no skeleton, a pose this body cannot take, or bad input
        raise Failed(str(exc))
    run.manifest["timings"]["pose"] = {"seconds": round(time.time() - started, 2)}
    run.manifest.update(report)
    if not report["check"]["pass"]:
        run.manifest["error"] = "pose not written: " + "; ".join(report["check"]["reasons"])
        run.output.parent.mkdir(parents=True, exist_ok=True)
        Path(f"{run.output}.manifest.json").write_text(json.dumps(run.manifest, indent=2) + "\n")
        raise Failed(run.manifest["error"])
    run.finish(out)


def gameready_cmd(args, run: Run) -> None:
    mesh = need_mesh(args)
    if args.prompt:
        run.manifest["prompt"] = args.prompt
    if mesh.suffix.lower() == ".glb":
        doc, _ = glbstats.read_glb(mesh)
        sidecar = Path(str(mesh) + ".manifest.json")
        prior = json.loads(sidecar.read_text()) if sidecar.is_file() else {}
        if prior.get("command") == "generate":
            for key in ("prompt", "seed"):
                if prior.get(key) is not None:
                    run.manifest[key] = prior[key]
        if package.prepared(doc, prior):
            run.progress("gameready", 90, "Using the existing game-ready mesh, LODs and collision")
            run.manifest["reused_gameready"] = True
            if prior.get("stage") == "gameready":
                run.manifest["gameready"] = prior
            else:
                for key in ("source", "tools", "models", "licences", "generation", "gameready", "poly_budget"):
                    if key in prior:
                        run.manifest[key] = prior[key]
            run.finish(mesh)
            return
        if isinstance(prior.get("finish"), dict) and prior["finish"].get("kind") == "character":
            finish_character(args, run, mesh, prior)
            return
    run.finish(gameready(run, mesh, args.poly_budget))


def rig_input(mesh: Path, work: Path) -> Path:
    """SkinTokens collects every mesh in Blender. Feed it the display geometry of a Forge package."""
    if mesh.suffix.lower() != ".glb":
        return mesh
    doc, binary = glbstats.read_glb(mesh)
    nodes = [n for n in doc.get("nodes", []) if "mesh" in n]
    names = {n.get("name", "") for n in nodes}
    removed = False
    for node in nodes:
        name = node.get("name", "")
        level = re.fullmatch(r"(.+)_LOD[1-9][0-9]*", name)
        hull = re.fullmatch(r"UCX_(.+)_[0-9]+", name)
        if (level and level[1] in names) or (hull and hull[1] in names):
            del node["mesh"]
            removed = True
    rooted = package.root_static_meshes(doc)
    if not removed and not rooted:
        return mesh
    output = work / "rig-display-mesh.glb"
    write_glb(doc, binary, output)
    return output


def rig(args, run: Run) -> None:
    mesh = need_mesh(args)
    landmarks = Path(args.hand_intake).resolve() if args.hand_intake else None
    if landmarks and not landmarks.is_file():
        raise Failed(f"missing hand landmarks: {landmarks}")
    if landmarks and glbstats.stats(mesh)["bones"]:
        run.manifest.update(seed=None, input_rig=str(mesh))
        finish_hand_intake(run, mesh, landmarks)
        return
    mesh = rig_input(mesh, run.work)
    rigged = run.work / "rigged.glb"
    arm_count = anatomy.radial_count(args.prompt or "")
    measured = arm_count and not landmarks and radial.fit_arms(mesh, rigged, arm_count)
    if measured:
        run.progress("rig", 80, f"fitted independent joints to {arm_count} measured arm tips")
        run.manifest["radial_arms"] = arm_count
    else:
        run.progress("rig", 5, "fitting a skeleton and skin weights (SkinTokens)")
        # Same three steps as AssetFurnace's Rig (viewer/autorig_api.py), Unity Humanoid names.
        run.run("rig", [ST_PY, ROOT / "forge/skintokens_rig.py", "--input", mesh, "--output", rigged,
                        "--seed", args.seed], cwd=SKINTOKENS, env={"PYTHONHASHSEED": str(args.seed % 2**32)})
        run.run("clean_weights", [ST_PY, "scripts/clean_skin_weights.py", "--in", rigged, "--out", rigged])
        run.uses("assetfurnace", "skintokens", "skintokens-weights", "blender-bpy")
    if args.swimming_fish:
        doc, binary = glbstats.read_glb(rigged)
        doc["skins"][0].setdefault("extras", {})["swimming_fish"] = True
        write_glb(doc, binary, rigged)
    if not measured and bodyplan.humanoid(bodyplan.Rig(rigged)):
        run.run("bone_names", [AF_PY, "scripts/name_bones.py", rigged, rigged, "--preset", "unity"],
                heavy=False)
        run.manifest.update(body_plan="biped_humanoid", legs=2)
    else:  # any other body: keep SkinTokens' skeleton, find its legs/spine/tail and name them
        run.progress("rig", 85, "finding the body plan (legs, spine, head, tail) and naming the bones")
        started = time.time()
        requested = anatomy.paired_legs(args.prompt or "")
        if requested in (6, 8):
            from forge.quality import leg_controls
            corrected = run.work / "fitted-legs.glb"
            recovery = leg_controls.recover(rigged, corrected, requested)
            if recovery:
                rigged = corrected
                run.manifest["leg_controls"] = recovery
        body = bodyplan.name_bones(rigged)
        run.manifest["timings"]["body_plan"] = {"seconds": round(time.time() - started, 2)}
        run.manifest.update(body_plan=body.pop("body_plan"), legs=body.pop("legs"), body=body)
    if landmarks:
        finish_hand_intake(run, rigged, landmarks)
    else:
        run.finish(rigged)


def finish_hand_intake(run, mesh, landmarks):
    from forge.quality.fingers import correct
    run.progress("hand_intake", 90, "testing named finger controls against fingertip landmarks")
    fixed = run.work / "handfit.glb"
    doc, binary, report = correct(mesh, landmarks)
    write_glb(doc, binary, fixed)
    run.finish(fixed, hand_intake=report)


def animate(args, run: Run) -> None:
    mesh = need_mesh(args)
    if not glbstats.stats(mesh)["bones"]:
        raise Failed(f"{mesh} has no skeleton; run forge rig first")
    if not bodyplan.humanoid(bodyplan.Rig(mesh)):
        animate_body_plan(args, run, mesh)
        return
    motions = AF / "image_to_3dlab" / "motions"
    index = json.loads((motions / "presets.json").read_text())
    presets = {p["id"]: p for p in index["presets"]}
    clips = []
    names = [c.strip() for c in (args.clips or "idle,walk,run").split(",") if c.strip()]
    for n, name in enumerate(names):
        preset = presets.get(index["aliases"].get(name, name))
        if preset is None:
            raise Failed(f"unknown clip {name!r}; one of: {', '.join(presets)}")
        run.progress("animate", 5 + 80 * n // len(names), f"retargeting {name}")
        clip = run.work / f"{name}.glb"
        # The flags AssetFurnace's Animate step passes (viewer/autorig_api.py animate_command)
        looping = preset.get("loop", preset["id"].startswith(("idle", "walk", "run", "sneak", "strafe")))
        argv = [ST_PY, AF / "scripts" / "kimodo_retarget.py", "--rig", mesh,
                "--motion", motions / f"{preset['id']}.npz", "--out", clip,
                "--arm-spread", "0", "--speed", preset.get("speed", 1.0)]
        argv += ["--loop"] if looping or preset.get("loop") is False else []
        argv += ["--keep-whole"] if preset.get("loop") is False else []
        argv += ["--in-place"] if preset.get("in_place", looping) else []
        run.run(f"animate_{name}", argv)
        clips.append((name, clip))
    run.uses("assetfurnace", "kimodo-clips", "blender-bpy")
    merged = run.work / "animated.glb"
    merge_clips(clips, merged)
    run.finish(merged)


def animate_body_plan(args, run: Run, mesh: Path) -> None:
    """idle / walk / run made from a non-humanoid body's own skeleton (forge/gait.py), checked per leg."""
    from forge import gait
    names = [c.strip() for c in (args.clips or "idle,walk,run").split(",") if c.strip()]
    out = run.work / "animated.glb"
    started = time.time()
    report = gait.animate(mesh, out, names, progress=lambda pct, msg: run.progress("animate", pct, msg))
    run.manifest["timings"]["procedural_clips"] = {"seconds": round(time.time() - started, 2)}
    run.uses("forge-gait")
    # every per-leg measurement beside the output; the manifest keeps shallow per-clip summaries
    full = Path(f"{run.output}.fit.json")
    full.parent.mkdir(parents=True, exist_ok=True)
    full.write_text(json.dumps(report, indent=1) + "\n")
    run.finish(out, body_plan=report.pop("body_plan"), legs=report.pop("legs"),
               motion=gait.summary(report, full.name))


def merge_clips(clips: list[tuple[str, Path]], out: Path) -> None:
    """One GLB with every clip: the first clip's file, plus the other files' animations.

    All clips come from one rig, so node indices line up; checked by node names."""
    base, binary = glbstats.read_glb(clips[0][1])
    binary = bytearray(binary)
    base["animations"] = []
    for name, path in clips:
        gltf, data = glbstats.read_glb(path)
        if [n.get("name") for n in gltf["nodes"]] != [n.get("name") for n in base["nodes"]]:
            raise Failed(f"clip {name} was retargeted onto a different node tree")
        for anim in gltf["animations"]:
            remap = {}
            for sampler in anim["samplers"]:
                for key in ("input", "output"):
                    old = sampler[key]
                    if old not in remap:
                        accessor = dict(gltf["accessors"][old])
                        view = dict(gltf["bufferViews"][accessor["bufferView"]])
                        chunk = data[view.get("byteOffset", 0): view.get("byteOffset", 0) + view["byteLength"]]
                        binary += b"\0" * (-len(binary) % 4)
                        view.update(buffer=0, byteOffset=len(binary))
                        binary += chunk
                        base["bufferViews"].append(view)
                        accessor["bufferView"] = len(base["bufferViews"]) - 1
                        base["accessors"].append(accessor)
                        remap[old] = len(base["accessors"]) - 1
                    sampler[key] = remap[old]
            anim["name"] = name
            base["animations"].append(anim)
    binary += b"\0" * (-len(binary) % 4)
    base["buffers"] = [{"byteLength": len(binary)}]
    write_glb(base, binary, out)


def write_glb(doc: dict, binary: bytes | bytearray, out: Path) -> None:
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    out.write_bytes(struct.pack("<4sII", b"glTF", 2, 12 + 8 + len(text) + 8 + len(binary))
                    + struct.pack("<I4s", len(text), b"JSON") + text
                    + struct.pack("<I4s", len(binary), b"BIN\0") + bytes(binary))


class ForgeParser(argparse.ArgumentParser):
    def error(self, message):
        self.exit(1, json.dumps({"error": message}) + "\n")


def literal_option_values(argv, value_options):
    """Otto supplies a value for every option, including option-looking prompt text."""
    result = []
    tokens = iter(argv)
    for token in tokens:
        option, attached, value = token.partition("=")
        if option not in value_options:
            result.append(token)
            continue
        if not attached:
            value = next(tokens, None)
        if value is None:
            result.append(token)  # argparse reports a genuinely missing operand
        elif value and not re.fullmatch(r"\{[A-Za-z_][A-Za-z0-9_]*\}", value):
            result.append(f"{option}={value}")
    return result


def main(argv: list[str] | None = None) -> int:
    parser = ForgeParser(prog="forge", description=__doc__.splitlines()[0])
    value_options = set()

    def option(target, flag, **kwargs):
        target.add_argument(flag, **kwargs)
        value_options.add(flag)

    common = argparse.ArgumentParser(add_help=False)
    for flag in ("--mesh", "--image", "--prompt", "--clips", "--kind"):
        option(common, flag, default="")
    option(common, "--poly-budget", default="")
    option(common, "--seed", default="")
    option(common, "--format", default="glb")
    option(common, "--height-m", help="output display height in metres for generate or gameready")
    option(common, "--longest-side-m", help="longest display dimension in metres for generate or gameready")
    sub = parser.add_subparsers(dest="cmd", required=True)
    server = sub.add_parser("concept-server", help="keep FLUX warm for generate --concept-server URL")
    server.add_argument("--port", type=int, default=7831)
    for name in ("generate", "gameready", "rig", "animate", "library", "pose"):
        stage = sub.add_parser(name, parents=[common])
        option(stage, "--output", required=name not in ("library", "generate"))
        if name == "library":
            stage.add_argument("--list-kinds", action="store_true")
            option(stage, "--min-score", default=0.65)
        if name == "pose":
            option(stage, "--strike-left", help="first percussion contact, x,y,z in the player frame")
            option(stage, "--strike-right", help="second percussion contact, x,y,z in the player frame")
            option(stage, "--plant-at", help="hand contact on a grounded shaft, x,y,z in the player frame")
            option(stage, "--fingerboard", help="left-hand contact on a stopped string, x,y,z in the player frame")
            option(stage, "--strings", help="right-hand plucking contact, x,y,z in the player frame")
            option(stage, "--other-hold-style", choices=["carry","raise"], help="close the opposite hand around a second object")
            option(stage, "--other-hold-toward", help="target for the opposite hand, in the same frame as --hold-toward")
            option(stage, "--wind-instrument", help="a wind instrument GLB to bring to the mouth with both hands")
            option(stage, "--wind-scale", default="1", help="instrument scale relative to the player")
            option(stage, "--pose", default="", help="stand, sit, lean, kneel, crouch, lie or curl, plus ,cross-arms, ,hold-out or ,hold")
            option(stage, "--seat-height", help="seat top above the character's floor, metres (sit)")
            option(stage, "--look-at", help="x,y,z metres from the character's ground point: x left, y up, z ahead")
            option(stage, "--hand", help="left or right (hold-out or hold; default right)")
            option(stage, "--hold-style", help="raise (a stick held up and forward, default) or carry (a load hanging)")
            option(stage, "--hold-toward", help="x,y,z the raised thing points towards (same frame as --look-at)")
        if name in ("rig", "pose"):
            stage.add_argument("--swimming-fish", action="store_true",
                               help="use the swimming body and locate its head from the mesh")
        if name == "rig":
            option(stage, "--hand-intake", metavar="LANDMARKS.json",
                   help="verify and correct Unity digit controls; reuse an existing skinned input")
        if name == "generate":
            stage.add_argument("--hero-face", action="store_true",
                               help="rebuild a character head from its upscaled crop (requires Real-ESRGAN and SkinTokens)")
            option(stage, "--source", metavar="{raw,finish}",
                   help="game-ready input: raw Pixal3D mesh and adjacent cameras (default), or engine Finish")
            option(stage, "--quality", default="standard", metavar="{draft,standard,high}",
                               help="Pixal3D 1024 steps: draft 6, standard engine default (8 patched, otherwise 12), high 12")
            option(stage, "--pixal-steps", metavar="1..50", help="override the preset steps per flow")
            option(stage, "--structure-guidance", default=10.0, help="sparse structure guidance (default: 10)")
            option(stage, "--back-image", help="matching rear view in the same pose (requires MV weights)")
            option(stage, "--concept-server", metavar="URL",
                   help="draw the concept picture on a running sd-server (same weights) instead of sd-cli")
            option(stage, "--body", metavar="{humanoid,quadruped,bird,legless,other}",
                   help="the character's body, for the concept picture's pose words (implies --kind character)")
            stage.add_argument("--print-prompt", action="store_true",
                               help="print the concept picture's prompt as one JSON line and draw nothing")
            stage.add_argument("--concept-only", action="store_true",
                               help="draw only the concept picture and write it (PNG) to --output")
            stage.add_argument("--skip-gameready", action="store_true",
                               help="write the generator's mesh without LODs, collision or projection (a draft)")
    args = parser.parse_args(literal_option_values(sys.argv[1:] if argv is None else argv, value_options))
    run = None
    handlers = {}
    status = 0
    try:
        if args.cmd == "concept-server":
            return concept_server(args.port)
        if args.cmd == "library":
            from forge.library import smart
            from forge.library.cli import cancel
            for sig in (signal.SIGTERM, signal.SIGINT):
                handlers[sig] = signal.signal(sig, cancel)
            return smart.stage(args)
        # "" means not given (Otto passes every flag to every stage)
        args.poly_budget = int(args.poly_budget) if args.poly_budget else 20000
        args.seed = int(args.seed) if args.seed else 42
        args.format = args.format or "glb"
        args.kind = args.kind or None
        if args.kind not in (None, "character", "prop"):
            raise Failed("--kind is character or prop")
        args.body = getattr(args, "body", None) or None
        if args.body is not None:
            if args.body not in BODIES:
                raise Failed(f"--body is one of {', '.join(BODIES)}")
            if args.kind == "prop":
                raise Failed("--body describes a character: it does not go with --kind prop")
            if args.hero_face and args.body != "humanoid":
                raise Failed("--hero-face needs a humanoid --body")
            args.kind = "character"
        if args.cmd == "generate" and args.print_prompt:
            # The wording the concept step would draw, for a picture made elsewhere and passed back as --image.
            if not args.prompt or args.image:
                raise Failed("--print-prompt needs --prompt and no --image")
            kind = "character" if args.hero_face else args.kind
            print(json.dumps({"prompt": prompt_for(args.prompt, kind, args.body, args.hero_face),
                              "negative_prompt": "", "width": 1024, "height": 1024, "kind": kind, "body": args.body,
                              "cloud_suffix": CLOUD_SUFFIX}), flush=True)
            return 0
        if not args.output:
            raise Failed("--output is required")
        size_request = None
        if args.cmd in ("generate", "gameready"):
            if args.height_m and args.longest_side_m:
                raise Failed("Choose one of --height-m and --longest-side-m")
            for flag, value, axis in [("--height-m", args.height_m, "height"),
                                      ("--longest-side-m", args.longest_side_m, "longest")]:
                if value is not None:
                    try:
                        metres = float(value)
                    except ValueError:
                        metres = math.nan
                    if not math.isfinite(metres) or metres <= 0:
                        raise Failed(f"{flag} requires a finite positive number in metres")
                    size_request = {"axis": axis, "metres": metres}
        # A signal arriving between mkdtemp and handler installation must also clean up.
        signals = (signal.SIGTERM, signal.SIGINT)
        previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, signals)
        try:
            run = Run(args.cmd, Path(args.output), args.seed if args.cmd in ("generate", "rig") else None)
            if size_request:
                run.manifest["size_request"] = size_request
            for sig in signals:
                handlers[sig] = signal.signal(sig, run.cancel)
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)
        {"generate": generate, "gameready": gameready_cmd, "rig": rig, "animate": animate,
         "pose": pose_cmd}[args.cmd](args, run)
    except Exception as exc:
        if isinstance(exc, Cancelled):
            run.stop_child()
            status, message = 143, "cancelled"
        else:
            status, message = 1, str(exc).splitlines()[0] if str(exc) else type(exc).__name__
        if run and run.keep_work:
            message += f"; work retained at {run.work}"
        print(json.dumps({"error": message}), file=sys.stderr, flush=True)
    finally:
        # A second cancellation must not interrupt deletion or leave the child unwaited.
        if run:
            run.cancelled = True
            if not run.keep_work:
                try:
                    shutil.rmtree(run.work)
                except OSError as exc:
                    print(json.dumps({"error": f"could not remove {run.work}: {exc}"}), file=sys.stderr, flush=True)
                    status = status or 1
        for sig, previous in handlers.items():
            signal.signal(sig, previous)
    return status


if __name__ == "__main__":
    raise SystemExit(main())
