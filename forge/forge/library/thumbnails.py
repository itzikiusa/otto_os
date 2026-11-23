"""Render installed catalog entries in short locked batches. Existing valid images are reusable."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
from .import_packs import LIB, ROOT
from .prepare import BLENDER


def run():
    catalog = ROOT / "forge/library/catalog.json"
    if catalog.is_file():
        rows = json.loads(catalog.read_text())
    else:
        from .animation import library_clips

        rows = [
            r
            for p in json.loads((LIB / "inventory.json").read_text())["packs"]
            for r in p["models"]
        ]
        original = {r["file"]: r for r in rows}
        for clip in library_clips(LIB)["clips"]:
            rows.append(
                dict(
                    original[clip["file"]],
                    id=clip["id"],
                    action=clip["action"],
                    thumbnail="thumbnails/clips/" + clip["id"] + ".png",
                )
            )
    scratch = LIB / "verification/thumbnails"
    scratch.mkdir(parents=True, exist_ok=True)
    jobs = [r for r in rows if not (LIB / r["thumbnail"]).is_file()]
    jobs.sort(
        key=lambda r: (
            0 if r["kind"] in ("character", "prop") else 1,
            r["file"],
            r["id"],
        )
    )
    for i in range(0, len(jobs), 16):
        if shutil.disk_usage(LIB).free < 55 * 2**30:
            raise OSError("55 GiB free space required")
        with tempfile.TemporaryDirectory(dir=scratch) as tmp:
            job = Path(tmp) / "jobs.json"
            job.write_text(json.dumps(jobs[i : i + 16]))
            command = [
                str(ROOT / "tools/heavy.sh"),
                os.environ.get("BLENDER", BLENDER),
                "-b",
                "--factory-startup",
                "--disable-autoexec",
                "-t",
                "2",
                "--python-exit-code",
                "1",
                "-P",
                str(ROOT / "forge/library/thumbnail_blender.py"),
                "--",
                str(job),
            ]
            with (scratch / f"batch-{i:04d}.log").open("w") as log:
                subprocess.run(
                    command, stdout=log, stderr=subprocess.STDOUT, check=True
                )
        print(f"{min(i + 16, len(jobs))}/{len(jobs)}", flush=True)
        if i + 16 < len(jobs):
            time.sleep(2.2)  # Give other heavy.sh waiters a full polling interval.


if __name__ == "__main__":
    run()
