"""Index the local publisher assets. Run with tools/heavy.sh python3 forge/animlib/catalog_tools/build.py."""
import csv
import io
import json
import math
import re
import shutil
import struct
import tempfile
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ASSETS = HERE.parents[1] / "models/animlib"
KAYKIT = (
    ("kaykit-adventurers", "Adventures", "Knight.glb"),
    ("kaykit-skeletons", "Skeletons", "Skeleton_Warrior.glb"),
)
PRIMARY = {"Walking_A": "walk", "Running_A": "run", "Idle": "idle"}


def slug(value):
    return re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")


def category(name):
    if name.startswith("Death"):
        return "death"
    if name.startswith(("Running", "Walking", "Jump", "Dodge", "Spawn")):
        return "locomotion"
    if name.startswith(("Cheer", "Taunt")):
        return "emote"
    if "Idle" in name or "Inactive" in name:
        return "idle"
    if name.startswith(("Sit", "Lie", "Interact", "PickUp", "Use_Item", "Skeletons_Awaken")):
        return "interaction"
    return "combat"


def yaw_radians(quaternion):
    x, y, z, w = quaternion
    return math.atan2(2 * (w * y + x * z), 1 - 2 * (y * y + x * x))


def glb_rows(path, pack, seen):
    blob = path.read_bytes()
    length = struct.unpack_from("<I", blob, 12)[0]
    gltf = json.loads(blob[20:20 + length])
    buffer_start = 28 + length

    def samples(index):
        accessor = gltf["accessors"][index]
        view = gltf["bufferViews"][accessor["bufferView"]]
        item = struct.Struct("<" + "f" * {"VEC3": 3, "VEC4": 4}[accessor["type"]])
        start = buffer_start + view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
        stride = view.get("byteStride", item.size)
        return [item.unpack_from(blob, start + i * stride) for i in range(accessor["count"])]

    for animation in gltf["animations"]:
        name = animation["name"]
        if name in seen:
            continue
        time_arrays = [gltf["accessors"][s["input"]] for s in animation["samplers"]]
        duration = max(a["max"][0] for a in time_arrays) - min(a["min"][0] for a in time_arrays)
        if duration <= 0:
            continue  # Single-frame reference poses aren't playable animations.
        seen.add(name)
        root_motion = False
        cycle = name.startswith(("Running", "Walking")) or "Idle" in name or name in {
            "Blocking", "Spellcasting", "1H_Ranged_Shooting", "2H_Ranged_Shooting",
            "2H_Melee_Attack_Spinning",
        }
        for channel in animation["channels"]:
            target = channel["target"]
            bone = gltf["nodes"][target["node"]]["name"]
            field = target["path"]
            if any(marker in bone.lower() for marker in ("ik", "control", "slot")):
                continue
            values = samples(animation["samplers"][channel["sampler"]]["output"])
            if bone == "root" and field == "translation":
                axes = list(zip(*values))
                root_motion |= any(max(axes[i]) - min(axes[i]) > 0.01 for i in (0, 2))
            if bone == "root" and field == "rotation":
                yaw = yaw_radians(values[0])
                root_motion |= any(abs(math.remainder(yaw_radians(v) - yaw, math.tau)) >
                                   math.radians(20) for v in values)
            if field == "rotation":
                dot = abs(sum(a*b for a, b in zip(values[0], values[-1])))
                cycle &= 2 * math.acos(min(1, dot)) <= math.radians(5)
            else:
                cycle &= math.dist(values[0], values[-1]) <= 0.015
        yield dict(id=PRIMARY.get(name, slug(name)), pack=pack, licence="CC0-1.0",
                   category=category(name), duration=round(duration, 6), loopable=cycle,
                   root_motion=root_motion, file=str(path.relative_to(ASSETS)), action=name)


def bvh_rows(archive):
    with zipfile.ZipFile(archive) as bundle:
        cuts = {}
        with bundle.open("100STYLE/Frame_Cuts.csv") as data:
            for row in csv.DictReader(io.TextIOWrapper(data, encoding="utf-8-sig")):
                cuts[row["STYLE_NAME"].strip().lower()] = row
        members = [n for n in bundle.namelist() if n.startswith("100STYLE/") and n.endswith(".bvh")]
        for member in sorted(members):
            style, kind = Path(member).stem.rsplit("_", 1)
            cut = cuts[style.lower()]
            with bundle.open(member) as data:
                root_channels = None
                for line in data:
                    if root_channels is None and line.lstrip().startswith(b"CHANNELS"):
                        root_channels = line.split()[2:]
                    if line.startswith(b"Frames:"):
                        frames = int(line.split(b":", 1)[1])
                    if line.startswith(b"Frame Time:"):
                        interval = float(line.split(b":", 1)[1])
                        break
                first = int(cut[kind + "_START"])
                last = int(cut[kind + "_STOP"])
                if not 0 <= first < last < frames:
                    raise ValueError(f"Invalid frame cut {member}: {first}..{last} of {frames}")
                # Hips carry both capture-space movement and local body sway. Use larger
                # thresholds here than for the dedicated KayKit root.
                if root_channels != b"Xposition Yposition Zposition Yrotation Xrotation Zrotation".split():
                    raise ValueError(f"Unexpected root channels in {member}: {root_channels}")
                origin = None
                root_motion = False
                for i, line in enumerate(data):
                    if i < first:
                        continue
                    if i > last:
                        break
                    x, _, z, yrot, _ = line.split(None, 4)
                    point = (float(x), float(z))
                    yaw = float(yrot)
                    if origin is None:
                        origin = point
                        initial_yaw = yaw
                    if math.dist(point, origin) > 20 or abs(math.remainder(yaw - initial_yaw, 360)) > 45:
                        root_motion = True
                        break
            yield dict(id="100style-" + slug(style + "-" + kind), pack="100style",
                       licence="CC-BY-4.0", category="idle" if kind == "ID" else "locomotion",
                       duration=round((last - first) * interval, 6), loopable=False,
                       root_motion=root_motion, file=str(archive.relative_to(ASSETS)), member=member,
                       frame_start=first, frame_end=last, import_scale=0.01, fps=round(1 / interval))


def main():
    if shutil.disk_usage(HERE).free < 55 * 1024**3:
        raise RuntimeError("Reached 55 GiB disk floor")
    packs, rows, seen = {}, [], set()
    for pack, repo, asset in KAYKIT:
        folder = ASSETS / pack
        if not folder.exists():
            continue
        if not (folder / "LICENSE.txt").is_file():
            raise FileNotFoundError(f"Missing publisher licence for {pack}")
        rev = (folder / "revision.txt").read_text().split()[0]
        url = f"https://github.com/KayKit-Game-Assets/KayKit-Character-Pack-{repo}-1.0"
        packs[pack] = dict(licence="CC0-1.0", attribution="Kay Lousberg (optional credit)",
                           source=url + "/tree/" + rev, license_file=pack + "/LICENSE.txt",
                           licence_url="https://creativecommons.org/publicdomain/zero/1.0/")
        rows.extend(glb_rows(folder / asset, pack, seen))
    archive = ASSETS / "100style/100STYLE.zip"
    if archive.exists():
        for notice in ("LICENSE.txt", "ATTRIBUTION.txt"):
            if not (archive.parent / notice).is_file():
                raise FileNotFoundError(f"Missing 100STYLE {notice}")
        packs["100style"] = dict(licence="CC-BY-4.0", attribution="The 100STYLE Dataset - Ian Mason",
                                 source="https://zenodo.org/records/8127870",
                                 license_file="100style/LICENSE.txt",
                                 licence_url="https://creativecommons.org/licenses/by/4.0/",
                                 credit_file="100style/ATTRIBUTION.txt",
                                 publisher="https://www.ianxmason.com/100style/")
        rows.extend(bvh_rows(archive))
    if not rows:
        raise ValueError("Install at least one licensed pack before building a catalog")
    if len({r["id"] for r in rows}) != len(rows):
        raise ValueError("Duplicate clip IDs in catalog")
    catalog = HERE / "catalog.json"
    with tempfile.NamedTemporaryFile(mode="w", dir=HERE, delete=False) as temp:
        json.dump({"packs": packs, "clips": rows}, temp, indent=2)
        temp.write("\n")
    Path(temp.name).replace(catalog)
    print(f"{len(rows)} clips: " + ", ".join(f"{p}={sum(r['pack']==p for r in rows)}" for p in packs))


if __name__ == "__main__":
    main()
