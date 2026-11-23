import argparse
import gc
import json
import os
import re
import shutil
import signal
import sys
import tempfile
import time
from pathlib import Path

from . import engine
from .fetch_models import ROOT

events = sys.stderr
STAGE = "audio"


def emit(**event):
    print(json.dumps(event), file=events, flush=True)


last = None


def progress(pct, message):
    global last
    if (int(pct), message) != last:
        last = (int(pct), message)
        emit(stage=STAGE, pct=int(pct), message=message)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError(message)


def sfx(args, work):
    check_wav(args.output)
    engine.check(1 <= args.variations <= 16, "variations must be 1-16")
    out = args.output
    names = [out.name] + [f"{out.stem}-{k}{out.suffix}" for k in range(2, args.variations + 1)]
    files = []
    for k, name in enumerate(names):
        cue = dict(prompt=args.prompt, seconds=args.seconds, loop=args.loop, seed=args.seed + k,
                   kind="ambience" if args.loop else "effect")
        span = 90 / len(names)
        y, manifest = engine.render("sfx", cue, work, lambda f: progress(5 + span * (k + f), f"Generating {name}"))
        manifest.update(output=str(out.parent / name), variations=[str(out.parent / n) for n in names])
        files += engine.write(work, name, y, manifest)
    progress(97, "Publishing")
    engine.publish(files, out.parent)
    progress(100, f"Wrote {', '.join(names)}")


def music(args, work):
    check_wav(args.output)
    engine.check(not (args.instrumental and args.lyrics), "--instrumental and --lyrics are exclusive")
    cue = dict(prompt=args.prompt, seconds=args.seconds, loop=args.loop, seed=args.seed, bpm=args.bpm,
               lyrics=args.lyrics, kind="music")
    y, manifest = engine.render("music", cue, work, lambda f: progress(5 + 90 * f, "Generating music"))
    manifest["output"] = str(args.output)
    files = engine.write(work, args.output.name, y, manifest)
    progress(97, "Publishing")
    engine.publish(files, args.output.parent)
    progress(100, f"Wrote {args.output} ({manifest['duration_sec']:.2f} s)")


SAFE_ID = re.compile(r"^[A-Za-z0-9_-]{1,64}$")


def load_cues(path):
    sheet = json.loads(Path(path).read_text())
    engine.check(isinstance(sheet, dict), "cue sheet must be a JSON object")
    unknown = set(sheet) - {"scene", "seed", "ambience", "spots", "music"}
    engine.check(not unknown, f"unknown cue sheet keys: {', '.join(sorted(unknown))}")
    base = sheet.get("seed", 42)
    engine.check(isinstance(base, int), "seed must be an integer")
    cues, seen = [], set()
    for group, stage, kind in (("ambience", "sfx", "ambience"), ("spots", "sfx", "effect"), ("music", "music", "music")):
        entries = sheet.get(group, [])
        engine.check(isinstance(entries, list), f"{group} must be a list")
        for item in entries:
            engine.check(isinstance(item, dict) and SAFE_ID.match(str(item.get("id", ""))),
                         f"{group}: every cue needs an id of letters, digits, _ or -")
            engine.check(item["id"] not in seen, f"duplicate cue id {item['id']}")
            engine.check(isinstance(item.get("seconds"), (int, float)), f"{item['id']}: seconds must be a number")
            seen.add(item["id"])
            variations = item.get("variations", 1) if group == "spots" else 1
            engine.check(isinstance(variations, int) and 1 <= variations <= 16, f"{item['id']}: variations must be 1-16")
            seed = item.get("seed", base + len(cues))
            engine.check(isinstance(seed, int), f"{item['id']}: seed must be an integer")
            cues.append(dict(
                id=item["id"], stage=stage, kind=kind, prompt=item.get("prompt"), seconds=item["seconds"],
                loop=item.get("loop", group == "ambience"), seed=seed, bpm=item.get("bpm"),
                lyrics=item.get("lyrics"), variations=variations,
            ))
    engine.check(cues, "the cue sheet has no cues")
    return sheet, cues


def scene(args, work):
    sheet, cues = load_cues(args.cue_sheet)
    out = args.out
    out.mkdir(parents=True, exist_ok=True)
    engine.check(not (out / "manifest.json").exists(), f"{out / 'manifest.json'} exists: choose a fresh folder")
    started = time.monotonic()
    total = sum(c["variations"] for c in cues)
    done, files, entries, peaks = 0, [], [], {}
    # Effects first, then release that model before loading the music model.
    for c in sorted(cues, key=lambda c: c["stage"] != "sfx"):
        if c["stage"] == "music" and engine.moss.cache_info().currsize:
            engine.moss.cache_clear()
            gc.collect()
        for k in range(c["variations"]):
            name = f"{c['id']}.wav" if k == 0 else f"{c['id']}-{k + 1}.wav"
            cue = {**c, "seed": c["seed"] + k}
            y, manifest = engine.render(
                c["stage"], cue, work,
                lambda f, n=done: progress(2 + 93 * (n + f) / total, f"Generating {name}"))
            manifest.update(output=str(out / name), cue_id=c["id"])
            files += engine.write(work, name, y, manifest)
            entries.append({
                "id": c["id"], "file": name, "kind": c["kind"], "prompt": c["prompt"], "seed": cue["seed"],
                "duration_sec": manifest["duration_sec"], "loop": c["loop"],
                "loudness_lufs": manifest["loudness"]["loudness_lufs"],
                "true_peak_dbtp": manifest["loudness"]["true_peak_dbtp"],
                "seam_ok": manifest["loop"].get("seam_ok"), "model": manifest["model"]["id"],
                "licence": manifest["model"]["licence"],
            })
            for key, value in manifest["memory"].items():
                peaks[key] = max(peaks.get(key, 0), value)
            done += 1
    scene_manifest = work / "manifest.json"
    scene_manifest.write_text(json.dumps({
        "tool": "forge-audio scene", "cue_sheet": sheet, "files": entries,
        "timings": {"total_sec": time.monotonic() - started},
        "memory": peaks,
    }, indent=2) + "\n")
    progress(97, "Publishing")
    engine.publish(files + [scene_manifest], out)
    progress(100, f"Wrote {len(entries)} files and manifest.json to {out}")


def check_wav(path):
    engine.check(path.suffix.lower() == ".wav", "--output must end in .wav")


def main(argv):
    global STAGE
    parser = Parser(prog="forge-audio")
    commands = parser.add_subparsers(dest="command", required=True)
    s = commands.add_parser("sfx")
    s.add_argument("--prompt", required=True)
    s.add_argument("--seconds", type=float, required=True)
    s.add_argument("--loop", action="store_true")
    s.add_argument("--seed", type=int, default=42)
    s.add_argument("--variations", type=int, default=1)
    s.add_argument("--output", type=Path, required=True)
    m = commands.add_parser("music")
    m.add_argument("--prompt", required=True)
    m.add_argument("--seconds", type=float, required=True)
    m.add_argument("--loop", action="store_true")
    m.add_argument("--bpm", type=int)
    m.add_argument("--instrumental", action="store_true", help="the default; kept for explicit briefs")
    m.add_argument("--lyrics", help="sing these lyrics instead of an instrumental")
    m.add_argument("--seed", type=int, default=42)
    m.add_argument("--output", type=Path, required=True)
    c = commands.add_parser("scene")
    c.add_argument("--cue-sheet", type=Path, required=True)
    c.add_argument("--out", type=Path, required=True)
    args = parser.parse_args(argv)
    STAGE = args.command
    target = args.out if args.command == "scene" else args.output
    target = target.resolve()
    if args.command == "scene":
        args.out = target
    else:
        args.output = target
        target.parent.mkdir(parents=True, exist_ok=True)
    work_root = ROOT / "out" / "work"
    work_root.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(dir=work_root, prefix=f"audio-{target.stem}-", suffix=f"-{os.getpid()}"))
    keep = os.environ.get("FORGE_KEEP_WORK") == "1"
    try:
        # MLX, torch, tokenizers and loguru write diagnostics to fd 2; keep them in the work log.
        with open(work / "stage.log", "w") as log:
            os.dup2(log.fileno(), 2)
        progress(0, "Preparing")
        {"sfx": sfx, "music": music, "scene": scene}[args.command](args, work)
    except BaseException as exc:
        if keep:
            exc.add_note(f"work retained at {work}")
        raise
    finally:
        signal.pthread_sigmask(signal.SIG_BLOCK, (signal.SIGTERM, signal.SIGINT))
        if not keep:
            shutil.rmtree(work, ignore_errors=True)


def cancelled(signum, frame):
    raise KeyboardInterrupt


if __name__ == "__main__":
    events = os.fdopen(os.dup(2), "w")
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, cancelled)
    try:
        main(sys.argv[1:])
    except KeyboardInterrupt as exc:
        emit(error="Cancelled" + "".join("; " + n for n in getattr(exc, "__notes__", [])))
        sys.exit(130)
    except Exception as exc:  # noqa: BLE001 — public CLI failures are JSONL
        message = (str(exc).splitlines() or [type(exc).__name__])[0]
        emit(error=message + "".join("; " + n for n in getattr(exc, "__notes__", [])))
        sys.exit(1)
