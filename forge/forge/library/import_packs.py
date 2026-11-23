"""Copy and unpack publisher archives under models/library without changing the source directory."""

import argparse
import re
import shutil
import stat
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "models/library"


def slug(name):
    name = re.sub(r"\[standard\]| - (?:may|nov) 20\d\d.*", "", name, flags=re.I)
    return re.sub(r"[^a-z0-9]+", "-", name.casefold()).strip("-")


def import_pack(archive):
    target = LIB / slug(archive.stem)
    if target.exists():
        if (target / archive.name).is_file() and (target / "files").is_dir():
            with (
                zipfile.ZipFile(target / archive.name) as saved,
                zipfile.ZipFile(archive) as original,
            ):
                if [(i.filename, i.CRC, i.file_size) for i in original.infolist()] == [
                    (i.filename, i.CRC, i.file_size) for i in saved.infolist()
                ]:
                    return
        raise ValueError(f"different or incomplete existing pack at {target}")
    with zipfile.ZipFile(archive) as z:
        members = z.infolist()
        needed = archive.stat().st_size + sum(m.file_size for m in members)
        if shutil.disk_usage(LIB).free - needed < 55 * 2**30:
            raise ValueError("pack would cross the 55 GiB free-space floor")
        with tempfile.TemporaryDirectory(prefix=".import-", dir=LIB) as tmp:
            stage = Path(tmp) / "pack"
            files = stage / "files"
            files.mkdir(parents=True)
            shutil.copy2(archive, stage / archive.name)
            with zipfile.ZipFile(stage / archive.name) as local:
                for m in local.infolist():
                    path = Path(m.filename)
                    if (
                        path.is_absolute()
                        or ".." in path.parts
                        or stat.S_ISLNK(m.external_attr >> 16)
                    ):
                        raise ValueError(f"unsupported archive member {m.filename!r}")
                    local.extract(m, files)
            stage.rename(target)
    print(target.name, flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    LIB.mkdir(exist_ok=True, parents=True)
    for archive in sorted(
        args.directory.glob("*.zip"),
        key=lambda p: (not p.name.startswith("Universal"), p.name),
    ):
        import_pack(archive)
