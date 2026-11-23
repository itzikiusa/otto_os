"""Publisher source selection. Keep distinct origins and root-motion variants, group format replicas."""

from collections import defaultdict
from .import_packs import LIB, slug

FORMATS = (".glb", ".gltf", ".fbx", ".obj", ".blend")


def packs():
    return sorted(p for p in LIB.iterdir() if (p / "files").is_dir())


def sources(pack):
    groups = defaultdict(list)
    for path in (pack / "files").rglob("*"):
        if (
            path.suffix.lower() not in FORMATS
            or "__MACOSX" in path.parts
            or path.name.startswith("._")
        ):
            continue
        key = slug(path.stem)
        if "hairstyles" in str(path.relative_to(pack)).lower():
            key += "-origin" if "origin at 0" in str(path).lower() else "-rigged"
        groups[key].append(path)
    for key, options in sorted(groups.items()):
        yield (
            key,
            sorted(options, key=lambda p: (FORMATS.index(p.suffix.lower()), str(p))),
        )


def fallback_path(pack, key):
    return LIB / "derived" / "converted" / pack.name / (key + ".glb")
