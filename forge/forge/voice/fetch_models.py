"""Fetch public checkpoints at pinned revisions. No account or HF client needed."""

import argparse
import hashlib
import json
import shutil
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MODELS = ROOT / "models" / "voice"
SOURCES = {
    "kokoro": (
        "mlx-community/Kokoro-82M-bf16",
        "a71e4d38b236d968966a2002c4c895dbd12b1c3c",
    ),
    "qwen": (
        "mlx-community/Qwen3-TTS-12Hz-1.7B-VoiceDesign-4bit",
        "5c390979e4b93af5f2932f90742ca99c7dd04687",
    ),
}


def free_space():
    if shutil.disk_usage(ROOT).free < 55 * 1024**3:
        raise RuntimeError("Stopped: less than 55 GiB free")


def fetch(engine):
    repo, rev = SOURCES[engine]
    with urllib.request.urlopen(
        f"https://huggingface.co/api/models/{repo}/revision/{rev}?blobs=true",
        timeout=30,
    ) as response:
        files = json.load(response)["siblings"]
    for item in files:
        name = item["rfilename"]
        if engine == "kokoro" and not (
            name in ("config.json", "kokoro-v1_0.safetensors")
            or name.startswith("voices/")
            and name.endswith(".safetensors")
        ):
            continue
        path = MODELS / engine / name
        if path.is_file() and path.stat().st_size == item["size"]:
            continue
        free_space()
        path.parent.mkdir(parents=True, exist_ok=True)
        # Concurrent installers must never share an incomplete checkpoint file.
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as out:
            tmp = Path(out.name)
            digest = hashlib.sha256()
            try:
                with urllib.request.urlopen(
                    f"https://huggingface.co/{repo}/resolve/{rev}/{name}", timeout=60
                ) as response:
                    while chunk := response.read(4 * 1024**2):
                        free_space()
                        out.write(chunk)
                        digest.update(chunk)
                out.flush()
                if out.tell() != item["size"] or (
                    "lfs" in item and digest.hexdigest() != item["lfs"]["sha256"]
                ):
                    raise RuntimeError(f"Invalid download: {name}")
                tmp.replace(path)
            finally:
                tmp.unlink(missing_ok=True)
        print(engine, name, item["size"], flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("engines", nargs="+", choices=SOURCES)
    for engine in parser.parse_args().engines:
        fetch(engine)
