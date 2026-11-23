"""Fetch one batch of the publicly linked 100STYLE archive. Invoke using tools/heavy.sh."""
import concurrent.futures
import fcntl
import hashlib
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from http.client import IncompleteRead
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[3]
TARGET = ROOT / "models/animlib/100style/100STYLE.zip"
PART = TARGET.with_suffix(".part")
URL = "https://zenodo.org/records/8127870/files/100STYLE.zip?download=1"
SIZE = 1468515354
MD5 = "3cf627852fd8192024c04a8d0ef49583"
STEP = 16 * 1024**2
RANGES = 8

ATTRIBUTION = """The 100STYLE Dataset - Ian Mason
Ian Mason, Sebastian Starke and Taku Komura.
Real-Time Style Modelling of Human Locomotion via Feature-Wise Transformations and
Local Motion Phases (2022). https://doi.org/10.1145/3522618
Dataset: https://zenodo.org/records/8127870
Publisher's licence grant and requested credit: https://www.ianxmason.com/100style/
Creative Commons Attribution 4.0 International
https://creativecommons.org/licenses/by/4.0/
This directory contains the authors' original motion recordings. When redistributing
a retargeted animation, identify your modifications and retain the credit and licence.
"""


def get_range(start):
    end = min(SIZE, start + STEP) - 1
    data = bytearray()
    for attempt in range(4):
        offset = start + len(data)
        request = Request(URL, headers={"Range": f"bytes={offset}-{end}"})
        try:
            with urlopen(request, timeout=90) as response:
                expected = f"bytes {offset}-{end}/{SIZE}"
                if response.status != 206 or response.headers.get("Content-Range") != expected:
                    raise RuntimeError(f"Expected {expected}, got {response.headers.get('Content-Range')}")
                while len(data) < end - start + 1:
                    try:
                        block = response.read(min(1024**2, end - start + 1 - len(data)))
                    except IncompleteRead as exc:
                        data.extend(exc.partial)
                        break
                    if not block:
                        break
                    data.extend(block)
        except HTTPError as exc:
            if exc.code not in {429, 500, 502, 503, 504} or attempt == 3:
                raise
            time.sleep(float(exc.headers.get("Retry-After", 2 ** attempt)))
        except (TimeoutError, URLError, ConnectionError):
            if attempt == 3:
                raise
            time.sleep(2 ** attempt)
        if len(data) == end - start + 1:
            return data
    raise RuntimeError(f"Incomplete publisher byte range {start}-{end}, received {len(data)}")


def fetch_batch():
    clock = time.monotonic()
    if TARGET.exists():
        return
    if shutil.disk_usage(TARGET.parent).free < 55 * 1024**3 + RANGES * STEP:
        raise RuntimeError("Reached 55 GiB disk floor")
    with PART.open("ab") as output:
        if output.tell() > SIZE:
            raise ValueError("Partial archive exceeds the published byte count")
        batch_end = min(SIZE, output.tell() + STEP * RANGES)
        # Avoid a separate lock wait for the short tail of the archive.
        if SIZE - batch_end < STEP * 2:
            batch_end = SIZE
        starts = range(output.tell(), batch_end, STEP)
        with concurrent.futures.ThreadPoolExecutor(max_workers=RANGES) as pool:
            for data in pool.map(get_range, starts):
                if shutil.disk_usage(TARGET.parent).free < 55 * 1024**3 + len(data):
                    raise RuntimeError("Reached 55 GiB disk floor")
                output.write(data)
        print(f"{output.tell()}/{SIZE} ({time.monotonic() - clock:.1f}s)", flush=True)
    if PART.stat().st_size == SIZE:
        verify(PART)
        PART.replace(TARGET)


def verify(path):
    if path.stat().st_size != SIZE:
        raise ValueError(f"Wrong byte count for {path}")
    checksum = hashlib.md5()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024**2), b""):
            checksum.update(block)
    digest = checksum.hexdigest()
    if digest != MD5:
        raise RuntimeError(f"100STYLE MD5 mismatch: {digest}")


def main():
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(TARGET.parent, os.O_RDONLY)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        fetch_batch()
    finally:
        os.close(descriptor)


def install_notices():
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(TARGET.parent).free < 55 * 1024**3:
        raise RuntimeError("Reached 55 GiB disk floor")
    licence = TARGET.parent / "LICENSE.txt"
    if licence.exists():
        data = licence.read_bytes()
    else:
        with urlopen("https://creativecommons.org/licenses/by/4.0/legalcode.txt", timeout=30) as response:
            data = response.read(65536)
    if hashlib.sha256(data).hexdigest() != "9ba9550ad48438d0836ddab3da480b3b69ffa0aac7b7878b5a0039e7ab429411":
        raise ValueError("CC BY 4.0 legal code did not match pinned copy")
    if not licence.exists():
        licence.write_bytes(data)
    credit = TARGET.parent / "ATTRIBUTION.txt"
    if not credit.exists():
        credit.write_text(ATTRIBUTION)


if __name__ == "__main__":
    if sys.argv[1:] == ["--all"]:
        # Acquire the shared lock separately per batch so pending Blender jobs can run.
        run = [str(ROOT / "tools/heavy.sh"), sys.executable, str(Path(__file__).resolve())]
        subprocess.run([*run, "--notices"], check=True)
        if TARGET.exists():
            subprocess.run([*run, "--verify"], check=True)
        while not TARGET.exists():
            subprocess.run(run, check=True)
            time.sleep(0.25)
    elif sys.argv[1:] == ["--notices"]:
        install_notices()
    elif sys.argv[1:] == ["--verify"]:
        verify(TARGET)
    elif not sys.argv[1:]:
        main()
    else:
        raise SystemExit("Usage: python3 fetch_100style.py --all")
