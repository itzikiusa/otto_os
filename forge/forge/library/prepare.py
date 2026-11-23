"""Prepare missing-format conversions in short Blender batches."""

import json
import shutil
import subprocess
from .sources import LIB, packs, sources, fallback_path
from .import_packs import ROOT

BLENDER = "/Applications/Blender.app/Contents/MacOS/Blender"


def run():
    jobs = []
    for pack in packs():
        for key, options in sources(pack):
            if options[0].suffix.lower() in (".gltf", ".glb"):
                continue
            dest = fallback_path(pack, key)
            receipt = dest.with_suffix(".json")
            if (
                not dest.is_file()
                or not receipt.is_file()
                or json.loads(receipt.read_text())["missing_source_images"]
            ):
                jobs.append([str(options[0]), str(dest), str(pack)])
    scratch = LIB / "verification" / "conversion"
    scratch.mkdir(parents=True, exist_ok=True)
    for i in range(0, len(jobs), 6):
        if shutil.disk_usage(LIB).free < 55 * 2**30:
            raise OSError("55 GiB free space required")
        target = scratch / f"batch-{i}.json"
        target.write_text(json.dumps(jobs[i : i + 6]))
        with target.with_suffix(".log").open("w") as log:
            subprocess.run(
                [
                    ROOT / "tools/heavy.sh",
                    BLENDER,
                    "-b",
                    "--factory-startup",
                    "--disable-autoexec",
                    "--python-exit-code",
                    "1",
                    "-P",
                    ROOT / "forge/library/convert.py",
                    "--",
                    target,
                ],
                stdout=log,
                stderr=subprocess.STDOUT,
                check=True,
            )


if __name__ == "__main__":
    run()
