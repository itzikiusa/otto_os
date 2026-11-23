"""Install the two small CC0 GLBs. Run via tools/heavy.sh from any directory."""
import hashlib
import shutil
import tempfile
from pathlib import Path
from urllib.request import urlopen

ASSETS = Path(__file__).resolve().parents[3] / "models/animlib"
PACKS = (
    ("kaykit-adventurers", "Adventures", "672074b73ba276876a19e8816ecdc5241817ab47",
     "Knight.glb", "60428e3abc09ba83e595d256e3af8c5c976b46cdae599f0802fc82b4a3445168",
     "ae322141814056dda0deea7540d74c41d87aee1da319977cd1bd84ee5a923629"),
    ("kaykit-skeletons", "Skeletons", "15b62b9bad122f72926c10fb14d622c73819fa54",
     "Skeleton_Warrior.glb", "178b6fda810b814c250d8a2010c24dfd9b458b9006dd323353e620b7ff118bbe",
     "5d822abca4e08c5a91d329e5372b3dc605cba8d994f752cb0f7dfdb7a0a79954"),
)


def install(url, path, sha256):
    if path.exists():
        if hashlib.sha256(path.read_bytes()).hexdigest() != sha256:
            raise ValueError(f"Existing local file has unexpected contents: {path}")
        return
    if shutil.disk_usage(path.parent).free < 55 * 1024**3 + 8 * 1024**2:
        raise RuntimeError("Reached 55 GiB disk floor")
    with urlopen(url, timeout=60) as response:
        data = response.read(8 * 1024**2)
    if hashlib.sha256(data).hexdigest() != sha256:
        raise ValueError(f"Publisher SHA-256 mismatch for {path.name}")
    with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as pending:
        pending.write(data)
    Path(pending.name).replace(path)


def main():
    for pack, repo, revision, asset, asset_sha, license_sha in PACKS:
        folder = ASSETS / pack
        folder.mkdir(parents=True, exist_ok=True)
        base = (f"https://raw.githubusercontent.com/KayKit-Game-Assets/"
                f"KayKit-Character-Pack-{repo}-1.0/{revision}/")
        install(base + "LICENSE.txt", folder / "LICENSE.txt", license_sha)
        asset_path = f"addons/kaykit_character_pack_{repo.lower()}/Characters/gltf/{asset}"
        install(base + asset_path, folder / asset, asset_sha)
        (folder / "revision.txt").write_text(revision + "\n")
        print(f"Verified {pack}/{asset} and publisher licence")


if __name__ == "__main__":
    main()
