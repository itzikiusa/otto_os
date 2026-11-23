"""Fetch public checkpoints at pinned revisions, resumably. No account or HF client needed."""

import argparse
import hashlib
import json
import shutil
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MODELS = ROOT / "models" / "audio"
# name: (repo, revision, files to keep; None keeps the whole repository)
SOURCES = {
    "moss-sfx": (
        "mlx-community/MOSS-SoundEffect-v2.0-4bit",
        "51d53e8b51bb5f7eff1ac28ca6faf8d14557dfcb",
        None,
    ),
    # The DiT-only path: the turbo DiT, its VAE and the Qwen3 text embedder. The
    # optional 1.7B planner LM (3.7 GB) is not used by Forge.
    "ace-step": (
        "ACE-Step/Ace-Step1.5",
        "19671f406d603126926c1b7e2adc169acbcade22",
        ("config.json", "README.md", "acestep-v15-turbo/", "vae/", "Qwen3-Embedding-0.6B/"),
    ),
}


def free_space():
    if shutil.disk_usage(ROOT).free < 60 * 1024**3:
        raise RuntimeError("Stopped: less than 60 GiB free")


def listing(name):
    repo, rev, keep = SOURCES[name]
    with urllib.request.urlopen(
        f"https://huggingface.co/api/models/{repo}/revision/{rev}?blobs=true", timeout=30
    ) as response:
        files = json.load(response)["siblings"]
    # Model code (*.py) is skipped: ACE-Step installs its own copy from the pinned checkout.
    return [f for f in files if (keep is None or f["rfilename"].startswith(keep)) and not f["rfilename"].endswith(".py")]


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(16 * 1024**2):
            digest.update(chunk)
    return digest.hexdigest()


def fetch(name):
    repo, rev, _ = SOURCES[name]
    for item in listing(name):
        rel, size = item["rfilename"], item["size"]
        path = MODELS / name / rel
        if path.is_file() and path.stat().st_size == size:
            continue
        free_space()
        path.parent.mkdir(parents=True, exist_ok=True)
        part = path.with_name(path.name + ".part")
        have = part.stat().st_size if part.exists() else 0
        if have < size:
            req = urllib.request.Request(
                f"https://huggingface.co/{repo}/resolve/{rev}/{rel}",
                headers={"Range": f"bytes={have}-"} if have else {},
            )
            with urllib.request.urlopen(req, timeout=60) as response:
                if have and response.status != 206:
                    have = 0  # The server ignored the range; start over.
                with open(part, "ab" if have else "wb") as out:
                    while chunk := response.read(4 * 1024**2):
                        free_space()
                        out.write(chunk)
        if part.stat().st_size != size or (
            "lfs" in item and sha256(part) != item["lfs"]["sha256"]
        ):
            part.unlink()
            raise RuntimeError(f"Invalid download, removed: {rel}")
        part.replace(path)
        print(name, rel, size, flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("models", nargs="+", choices=SOURCES)
    parser.add_argument("--plan", action="store_true", help="Print download sizes only.")
    args = parser.parse_args()
    for name in args.models:
        files = listing(name)
        todo = sum(
            f["size"] for f in files
            if not ((MODELS / name / f["rfilename"]).is_file()
                    and (MODELS / name / f["rfilename"]).stat().st_size == f["size"])
        )
        print(f"{name}: {SOURCES[name][0]}@{SOURCES[name][1][:7]} "
              f"{sum(f['size'] for f in files) / 1e9:.2f} GB, {todo / 1e9:.2f} GB to fetch", flush=True)
    if not args.plan:
        for name in args.models:
            fetch(name)
