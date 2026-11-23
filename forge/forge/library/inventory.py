"""Build measurements and the searchable catalog from the installed publisher packs."""

import json
import statistics
import shutil
from collections import Counter, defaultdict
from .sources import LIB, packs, sources, fallback_path
from .import_packs import ROOT
from .inspect_model import inspect
from .animation import library_clips

from .names import tags, tokens, plain_name


def prop_group(name):
    t = set(tokens(name))
    if t & {"sword", "axe", "shield", "arrow", "bow", "quiver", "staff"}:
        return "Weapons and shields"
    if t & {
        "crate",
        "barrel",
        "chest",
        "cabinet",
        "bucket",
        "bag",
        "pouch",
        "cage",
        "vase",
        "pot",
        "cauldron",
    }:
        return "Containers"
    if t & {
        "key",
        "coin",
        "potion",
        "book",
        "bottle",
        "scroll",
        "carrot",
        "chalice",
        "mug",
        "cutlery",
        "fork",
        "spoon",
        "knife",
        "plate",
        "bowl",
        "candle",
    }:
        return "Small items"
    if t & {
        "bench",
        "chair",
        "bed",
        "shelf",
        "shelves",
        "table",
        "stool",
        "workbench",
        "bookcase",
        "nightstand",
    }:
        return "Furniture"
    return "Workshop and set dressing"


def kind(pack, name, rig):
    if pack.startswith("universal-animation") and name.startswith("UAL"):
        return "animation"
    if pack == "universal-base-characters":
        return "character" if "fullbody" in name.lower() else "outfit part"
    if pack == "modular-character-outfits-fantasy":
        return "outfit part"
    if "mannequin" in name.lower():
        return "character"
    if pack == "fantasy-props-megakit":
        return "prop"
    if name.startswith(("Enemy_", "Alien_", "Characters_")):
        return "character"
    if name.startswith(("Environment_", "Scene_")):
        return "scenery"
    if name.startswith(("Prop_", "Weapon_", "Furniture_", "Gun_")):
        return "prop"
    if rig and any(s["finger_joint_counts"] for s in rig):
        return "character"
    if pack in ("pirate-kit", "sci-fi-essentials-kit") and not any(
        t in name.lower() for t in ("floor", "wall", "scene", "terrain", "building")
    ):
        return "prop"
    return "scenery"


def build(*, metadata_only=False):
    catalog = []
    inventory = []
    groupnames = defaultdict(list)
    for pack in packs():
        if shutil.disk_usage(LIB).free < 55 * 2**30:
            raise OSError("55 GiB free space required")
        notices = [
            p
            for p in (pack / "files").rglob("*.txt")
            if p.name.lower().startswith(("license", "licence"))
            and "__MACOSX" not in p.parts
        ]
        line = next(
            (
                line.strip()
                for p in notices
                for line in p.read_text(errors="replace").splitlines()
                if line.strip() == "CC0 1.0 Universal (CC0 1.0)"
            ),
            None,
        )
        licence = "CC0-1.0" if line else "Unverified in archive"
        models = []
        formats = Counter()
        for key, options in sources(pack):
            original = options[0]
            formats.update(x.suffix.lower()[1:] for x in options)
            source = (
                original
                if original.suffix.lower() in (".glb", ".gltf")
                else fallback_path(pack, key)
            )
            info = inspect(source, pack, geometry=not metadata_only)
            if metadata_only:
                info.pop("size_m")
            id = pack.name + "/" + key
            asset = dict(
                id=id,
                pack=pack.name,
                licence=licence,
                kind=kind(pack.name, original.stem, info["rig"]),
                name=original.stem,
                plain_name=plain_name(original.stem),
                tags=tags(original.stem, original),
                source=str(original.relative_to(LIB)),
                file=str(source.relative_to(LIB)),
                thumbnail="thumbnails/" + id + ".png",
                **info,
            )
            if (
                pack.name == "universal-base-characters"
                and asset["kind"] == "character"
            ):
                body = original.stem.lower().split("_")[-2]
                folder = LIB / "derived" / "characters" / body
                evidence = (
                    LIB / "verification" / "characters" / f"{body}-template-intake.json"
                )
                if evidence.is_file():
                    hands = json.loads(evidence.read_text())["hands"]
                    asset["socketed_base"] = str((folder / "base.glb").relative_to(LIB))
                    asset["hand_templates"] = {
                        side: {
                            "file": str((folder / (side + ".glb")).relative_to(LIB)),
                            "fit_pass": hands[side[0]]["pass"],
                            "web_stretch": {
                                key: digit["web_stretch"]
                                for key, digit in hands[side[0]]["digits"].items()
                            },
                            "evidence": str(evidence.relative_to(LIB)),
                        }
                        for side in ("left", "right")
                    }
            catalog.append(asset)
            models.append(asset)
            if pack.name == "fantasy-props-megakit":
                groupnames[prop_group(original.stem)].append(original.stem)
        counts = [m["triangles"] for m in models]
        names = list(dict.fromkeys(n for m in models for n in m["animations"]))
        inventory.append(
            dict(
                pack=pack.name,
                licence=licence,
                licence_line=line,
                licence_files=[str(p.relative_to(LIB)) for p in notices],
                formats=dict(formats),
                model_count=len(models),
                triangles_min=min(counts),
                triangles_median=statistics.median(counts),
                triangles_max=max(counts),
                texture_styles=dict(Counter(m["texture"] for m in models)),
                rigged=[m["id"] for m in models if m["rig"]],
                animated=[m["id"] for m in models if m["animations"]],
                distinct_actions=len(names),
                first_30=names[:30],
                models=models,
            )
        )
    (LIB / "inventory.json").write_text(
        json.dumps(dict(packs=inventory, fantasy_props=groupnames), indent=2) + "\n"
    )
    if metadata_only:
        return inventory, groupnames
    native = {m["file"]: m for m in catalog}
    for clip in library_clips(LIB)["clips"]:
        asset = native[clip["file"]]
        catalog.append(
            dict(
                asset,
                id=clip["id"],
                name=clip["action"],
                plain_name=plain_name(clip["action"]),
                action=clip["action"],
                animations=[clip["action"]],
                category=clip["category"],
                duration=clip["duration"],
                root_motion=clip["root_motion"],
                loopable=clip["loopable"],
                tags=sorted(
                    set(tags(clip["action"], LIB / clip["file"])) | {clip["category"]}
                ),
                thumbnail="thumbnails/clips/" + clip["id"] + ".png",
            )
        )
    dest = ROOT / "forge/library/catalog.json"
    anchor_file = dest.with_name("prop_anchors.json")
    if anchor_file.is_file():
        anchors = json.loads(anchor_file.read_text())
        for asset in catalog:
            asset.update(anchors.get(asset["id"], {}))
    lines = [
        json.dumps(asset, ensure_ascii=False, separators=(",", ":"))
        for asset in catalog
    ]
    dest.write_text("[\n" + ",\n".join(lines) + "\n]\n")
    return inventory, groupnames


def report(rows, groups):
    intro = [
        "# Quaternius Standard pack inventory",
        "",
        "Counts group duplicate format exports by model name. Hair origins, animation editions and combined showcase scenes are distinct entries. Dimensions use glTF scene metres and triangle counts include mesh instances. Median triangle counts describe source scenes.",
        "",
        "| Pack | Models | Source files by format | Skinned | Animated | Distinct clips | Triangles min / median / max | Archive licence |",
        "|---|---:|---|---:|---:|---:|---|---|",
    ]
    for p in rows:
        intro.append(
            "| "
            + " | ".join(
                map(
                    str,
                    [
                        p["pack"],
                        p["model_count"],
                        ", ".join(f"{k} {v}" for k, v in sorted(p["formats"].items())),
                        len(p["rigged"]),
                        len(p["animated"]),
                        p["distinct_actions"],
                        f"{p['triangles_min']:,} / {p['triangles_median']:,.0f} / {p['triangles_max']:,}",
                        p["licence"],
                    ],
                )
            )
            + " |"
        )
    details = []
    for p in rows:
        details += [
            "",
            "## " + p["pack"],
            "",
            (
                "“"
                + p["licence_line"]
                + "” — "
                + ", ".join(
                    "[" + n.split("/")[-1] + "](<" + n + ">)"
                    for n in p["licence_files"]
                )
            )
            if p["licence_line"]
            else "No licence text in the supplied archive. Commercial rights remain unverified from these files.",
            "",
            "Material evidence: "
            + ", ".join(f"{k} ({v})" for k, v in p["texture_styles"].items())
            + ".",
        ]
        signatures = defaultdict(list)
        for m in p["models"]:
            if m["rig"]:
                desc = []
                for r in m["rig"]:
                    digits = (
                        ", ".join(
                            f"{k} {v}" for k, v in r["finger_joint_counts"].items()
                        )
                        or "no named finger joints"
                    )
                    desc.append(
                        f"{r['name']} (root {'/'.join(r['roots'])}, {r['joints']} joints, {digits})"
                    )
                signatures[" / ".join(desc)].append(m["name"])
        for desc, models in signatures.items():
            details += ["", "- " + desc + " — " + ", ".join(models)]
        if p["first_30"]:
            details += [
                "",
                "First 30 distinct action names (or all if fewer): "
                + ", ".join(p["first_30"]),
            ]
    props = ["", "## Fantasy Props Standard", ""]
    for group, names in sorted(groups.items()):
        props += ["- **" + group + "** — " + ", ".join(names)]
    caveats = [
        "",
        "The Fantasy Props archive has no bow, arrow, quiver or staff. Both full-body bases contain five named digits per hand, each with three phalanges and a terminal joint. Animation rig names match the base rigs, but their bind proportions differ. Naming alone does not prove zero positional error.",
        "",
    ]
    (LIB / "INVENTORY.md").write_text(
        "\n".join(intro + props + caveats + details) + "\n"
    )
    return "\n".join(intro + props + caveats)


if __name__ == "__main__":
    rows, groups = build()
    print(report(rows, groups))
