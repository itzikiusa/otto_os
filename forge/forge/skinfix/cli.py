"""Optional bind-weight sharpening for embedded rigged GLBs."""

import argparse
import json
import math
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError(message)


def progress(pct, message):
    print(
        json.dumps({"stage": "skinfix", "pct": pct, "message": message}),
        file=sys.stderr,
        flush=True,
    )


def cancel(sig, frame):
    raise InterruptedError(f"Cancelled by {signal.Signals(sig).name}")


def main():
    parser = Parser(description=__doc__)
    parser.add_argument("--mesh", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--stiffness",
        type=float,
        default=0,
        help="0 preserves input bytes (default), 1 is strongest",
    )
    args = parser.parse_args()
    if not math.isfinite(args.stiffness) or not 0 <= args.stiffness <= 1:
        raise ValueError("--stiffness must be between 0 and 1")
    output = args.output.absolute()
    manifest = Path(str(output) + ".manifest.json")
    if output.suffix.lower() != ".glb" or not args.mesh.is_file():
        raise ValueError("Supply an existing --mesh and a .glb --output")
    if output.exists() or manifest.exists():
        raise ValueError("Output or manifest already exists")
    blender = (
        os.environ.get("BLENDER")
        or shutil.which("blender")
        or "/Applications/Blender.app/Contents/MacOS/Blender"
    )
    start = time.monotonic()
    output.parent.mkdir(parents=True, exist_ok=True)
    progress(0, "Reading bind weights")
    with tempfile.TemporaryDirectory(dir=output.parent) as temp:
        tmp = Path(temp)
        glb = tmp / "mesh.glb"
        command = [
            str(ROOT / "tools/heavy.sh"),
            blender,
            "-b",
            "--factory-startup",
            "--disable-autoexec",
            "--python-exit-code",
            "1",
            "-P",
            str(Path(__file__).with_name("weights.py")),
            "--",
            str(args.mesh.absolute()),
            str(glb),
            str(args.stiffness),
        ]
        child = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            start_new_session=True,
        )
        try:
            log = child.communicate()[0]
        except BaseException:
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            child.wait()
            raise
        stats = next(
            (json.loads(x[8:]) for x in log.splitlines() if x.startswith("SKINFIX ")),
            {},
        )
        if child.returncode or "blender" not in stats:
            raise ValueError(
                stats.get("error") or "Blender failed: " + log.strip()[-400:]
            )
        progress(90, "Writing mesh and manifest")
        source = Path(str(args.mesh) + ".manifest.json")
        licences = {"tools": {"blender": "GPL-3.0-or-later", "numpy": "BSD-3-Clause"}}
        if source.is_file():
            licences["input"] = json.loads(source.read_text()).get("licences", [])
        meta = dict(
            stage="skinfix",
            input=str(args.mesh),
            output=str(args.output),
            tools={"blender": stats.pop("blender")},
            licences=licences,
            stiffness=args.stiffness,
            timings={"total_sec": round(time.monotonic() - start, 3)},
            **stats,
        )
        side = tmp / "manifest.json"
        side.write_text(json.dumps(meta, indent=2) + "\n")
        os.link(side, manifest)
        try:
            os.link(glb, output)
        except BaseException:
            manifest.unlink()
            raise
    progress(100, "Complete")


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    try:
        main()
    except Exception as exc:  # noqa: BLE001 - the CLI contract requires a final JSON error
        print(json.dumps({"error": str(exc)}), file=sys.stderr)
        sys.exit(1)
