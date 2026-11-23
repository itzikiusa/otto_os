"""Download the Kimodo checkpoints into models/motion/hf (resumable; prints sizes first).

Run with the motion venv: forge/motion/.venv/bin/python -I forge/motion/fetch_models.py [--encoder-only|--no-encoder]
The Llama 3 base is gated: the owner accepts Meta's licence on the model page and logs in with
`hf auth login`; the token is read from ~/.cache/huggingface/token and never copied.
"""

import os
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
os.environ.setdefault("HF_HOME", str(ROOT / "models/motion/hf"))
os.environ.setdefault("HF_TOKEN_PATH", str(Path.home() / ".cache/huggingface/token"))
os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"

from huggingface_hub import HfApi, snapshot_download  # noqa: E402

# repo, pinned commit, allow patterns (None = everything)
REPOS = [
    ("nvidia/Kimodo-SOMA-RP-v1.2", "0e3c89024d896064b6851201373f7834784a386b", None),
    ("McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp", "31474e395ada192e8ed1586db6be79fb3b70c9c0", None),
    ("McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp-supervised", "baa8ebf04a1c2500e61288e7dad65e8ae42601a7", None),
    # The original/ folder duplicates the weights in Meta's own format (another 16 GB).
    ("meta-llama/Meta-Llama-3-8B-Instruct", "8afb486c1db24fe5011ec46dfbe5b5dccdb575c2", ["*.json", "*.safetensors", "LICENSE", "USE_POLICY.md"]),
]
CAP = 25 * 1024**3
FLOOR = 60 * 1024**3


def main():
    only = sys.argv[1:]
    repos = [r for r in REPOS if not (("--no-encoder" in only and r[0].startswith("meta-llama"))
                                       or ("--encoder-only" in only and not r[0].startswith("meta-llama")))]
    api = HfApi()
    total = 0
    for repo, rev, allow in repos:
        files = api.model_info(repo, revision=rev, files_metadata=True).siblings
        size = sum(f.size or 0 for f in files if allow is None or any(Path(f.rfilename).match(p) and "/" not in f.rfilename for p in allow))
        total += size
        print(f"{repo}@{rev[:8]}: {size / 1e9:.2f} GB", flush=True)
    print(f"total: {total / 1e9:.2f} GB (cache: {os.environ['HF_HOME']})", flush=True)
    if total > CAP:
        sys.exit(f"Refusing: {total / 1e9:.1f} GB exceeds the 25 GB cap")
    for repo, rev, allow in repos:
        if shutil.disk_usage(ROOT).free < FLOOR:
            sys.exit("Stopped: less than 60 GB free")
        path = snapshot_download(repo, revision="main", allow_patterns=allow)
        if Path(path).name != rev:
            sys.exit(f"{repo}: main is now {Path(path).name}, pinned {rev}; review before using it")
        print(f"ok {repo} -> {path}", flush=True)


if __name__ == "__main__":
    main()
