"""Index the publisher's local UAL glTF files for the animation and asset browsers."""

import math
import re
import struct

from .gltf import read as read_glb

PACKS = {
    "universal-animation-library": ("UAL1", "universalanimationlibrary"),
    "universal-animation-library-2": ("UAL2", "universalanimationlibrary2"),
}
LICENCE_LINE = "CC0 1.0 Universal (CC0 1.0)"


def samples(document, binary, accessor_index):
    a = document["accessors"][accessor_index]
    view = document["bufferViews"][a["bufferView"]]
    if a["componentType"] != 5126 or view["buffer"] != 0 or "sparse" in a:
        raise ValueError("Expected local float animation samples")
    columns = {"SCALAR": 1, "VEC3": 3, "VEC4": 4}[a["type"]]
    fmt = struct.Struct("<" + "f" * columns)
    start = view.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = view.get("byteStride", fmt.size)
    return [fmt.unpack_from(binary, start + i * stride) for i in range(a["count"])]


def moves_root(doc, binary, action):
    """Actual root translation (>1 mm) or rotation (>0.1 deg), including return trips."""
    for channel in action["channels"]:
        target = channel["target"]
        node = doc["nodes"][target["node"]]
        if node["name"] != "root" or target["path"] not in {"translation", "rotation"}:
            continue
        values = samples(doc, binary, action["samplers"][channel["sampler"]]["output"])
        initial = values[0]
        if target["path"] == "translation":
            if any(math.dist(initial, value) > 0.001 for value in values[1:]):
                return True
        else:
            # q and -q represent identical orientations. Compare the unit dot product.
            length = math.hypot(*initial)
            for value in values[1:]:
                cosine = abs(sum(a * b for a, b in zip(initial, value))) / (
                    length * math.hypot(*value)
                )
                if cosine < math.cos(math.radians(0.1) / 2):
                    return True
    return False


def category(name):
    name = name.lower()
    if name == "a_tpose":
        return "pose"
    if name.startswith("death"):
        return "death"
    if name.startswith(("dance", "yes", "idle_no_", "idle_talking", "sitting_talking")):
        return "emote"
    if "idle" in name and not name.startswith(
        ("laytoidle", "jump", "ninjajump", "swim")
    ):
        return "idle"
    if name.startswith(
        (
            "walk",
            "climb",
            "swim",
            "slide",
            "roll",
            "jog",
            "jump",
            "ninjajump",
            "sprint",
            "crouch",
            "zombie_walk",
        )
    ):
        return "locomotion"
    if name.startswith(
        (
            "pistol",
            "sword",
            "punch",
            "melee",
            "shield",
            "hit",
            "spell",
            "zombie_scratch",
        )
    ):
        return "combat"
    return "interaction"


def library_clips(root):
    """Return the animlib schema for installed, licence-verified Standard editions."""
    result = {"clips": [], "packs": {}}
    for pack, (prefix, page) in PACKS.items():
        folder = root / pack / "files"
        if not folder.exists():
            continue
        notices = list(folder.glob("*/License.txt"))
        if len(notices) != 1 or not any(
            line.strip() == LICENCE_LINE for line in notices[0].read_text().splitlines()
        ):
            raise ValueError(f"Missing publisher CC0 notice for {pack}")
        result["packs"][pack] = {
            "licence": "CC0-1.0",
            "license_file": str(notices[0].relative_to(root)),
            "licence_url": "https://creativecommons.org/publicdomain/zero/1.0/",
            "source": f"https://quaternius.com/packs/{page}.html",
            "attribution": "Quaternius (optional credit)",
        }
        for variant in ("", "_RM"):
            path = (
                notices[0].parent / "Unreal-Godot" / f"{prefix}_Standard{variant}.glb"
            )
            doc, binary = read_glb(path)
            for action in doc["animations"]:
                name = action["name"]
                times = [doc["accessors"][s["input"]] for s in action["samplers"]]
                duration = max(a["max"][0] for a in times) - min(
                    a["min"][0] for a in times
                )
                slug = re.sub("[^a-z0-9]+", "-", name.lower()).strip("-")
                result["clips"].append(
                    {
                        "id": f"{prefix.lower()}-{slug}" + ("-rm" if variant else ""),
                        "pack": pack,
                        "licence": "CC0-1.0",
                        "category": category(name),
                        "action": name,
                        "duration": duration,
                        "loopable": name.endswith("_Loop"),
                        "root_motion": moves_root(doc, binary, action),
                        "file": str(path.relative_to(root)),
                    }
                )
    return result
