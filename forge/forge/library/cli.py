"""Search the local Quaternius library and export selected assets."""

import argparse
import json
import os
import shutil
import signal
import sys
import tempfile
import time
from pathlib import Path

from forge.quality.package import size_m

from .gltf import pack_glb, read
from .import_packs import LIB
from .names import tokens

CATALOG = Path(__file__).with_name("catalog.json")
KINDS = ("character", "outfit part", "prop", "scenery", "animation")
STOP_WORDS = {"a", "an", "the", "of", "for", "with", "in", "and", "some", "find", "me"}


def eligible(row):
    return (
        row["pack"] != "pirate-kit"
        and row["licence"] == "CC0-1.0"
        and not row.get("flagged", False)
    )


def find(rows, query, kind=None):
    terms = set(tokens(query)) - STOP_WORDS
    if not terms:
        return []
    scored = []
    for row in rows:
        if not eligible(row) or (kind and row["kind"] != kind):
            continue
        name = set(tokens(row["name"]))
        words = set(row["tags"]) | name | set(tokens(row["kind"]))
        if not terms <= words:
            continue
        rank = (
            name == terms,
            len(terms & name),
            -len(name - terms),
        )
        scored.append((rank, row))
    scored.sort(key=lambda pair: pair[1]["id"])
    scored.sort(key=lambda pair: pair[0], reverse=True)
    return [r for _, r in scored]


def progress(pct, message):
    print(
        json.dumps(dict(stage="library", pct=pct, message=message)),
        file=sys.stderr,
        flush=True,
    )


def export(row, parts, dest, command, *, selection=None, animation_jobs=()):
    from .characters import export_character

    if not all(eligible(r) for r in [row, *parts]):
        raise ValueError("Pack is excluded from commercial export")
    dest = dest.resolve()
    if dest.suffix.lower() != ".glb":
        raise ValueError("Choose a .glb output path")
    manifest = Path(str(dest) + ".manifest.json")
    if dest.exists() or manifest.exists():
        raise FileExistsError("Choose a fresh output path: " + str(dest))
    dest.parent.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(dest.parent).free < 55 * 2**30:
        raise OSError("55 GiB free space required")
    source = LIB / row["file"]
    if not source.is_file():
        raise FileNotFoundError("Install the source pack for " + row["id"])
    started = time.monotonic()
    progress(5, "Reading " + row["name"])
    with tempfile.TemporaryDirectory(prefix=".library-", dir=dest.parent) as tmp:
        target = Path(tmp) / "asset.glb"
        base = row["pack"] == "universal-base-characters" and row["kind"] == "character"
        details = {}
        if command == "assemble" and (
            not base
            or any(p["pack"] != "modular-character-outfits-fantasy" for p in parts)
        ):
            raise ValueError(
                "assemble needs a full-body base and matching modular outfit parts"
            )
        if base:
            progress(
                20,
                "Adding character sockets" if not parts else "Fitting outfit to base",
            )
            details = export_character(source, target, [LIB / p["file"] for p in parts])
        else:
            pack_glb(source, LIB / row["pack"], target, row.get("action"))
        if animation_jobs:
            from .animation_native import attach_native

            progress(70, "Adding native character animations")
            animated = Path(tmp) / "animated.glb"
            if not attach_native(target, animation_jobs, animated):
                raise ValueError("Assembled outfit does not share the UAL skeleton")
            target = animated
        doc, binary = read(target)
        nodes = doc.get("nodes", [])
        accessors = doc["accessors"]
        triangles = 0
        for node in nodes:
            if "mesh" in node:
                for primitive in doc["meshes"][node["mesh"]]["primitives"]:
                    if primitive.get("mode", 4) == 4:
                        index = primitive.get(
                            "indices", primitive["attributes"]["POSITION"]
                        )
                        triangles += accessors[index]["count"] // 3
        joints = sorted(
            {
                nodes[j].get("name", str(j))
                for skin in doc.get("skins", [])
                for j in skin["joints"]
            }
        )
        clips = []
        for animation in doc.get("animations", []):
            times = [accessors[s["input"]] for s in animation["samplers"]]
            clips.append(
                {
                    "name": animation["name"],
                    "duration_s": max(t["max"][0] for t in times)
                    - min(t["min"][0] for t in times),
                }
            )
        data = dict(
            stage="library",
            command=command,
            output=str(dest),
            asset=row["id"],
            source=row["source"],
            licences={r["pack"]: r["licence"] for r in [row, *parts]},
            parts=[p["id"] for p in parts],
            tools={"library": "Forge", "glb_generator": doc["asset"].get("generator")},
            timings={"total_sec": round(time.monotonic() - started, 3)},
            triangles=triangles,
            size_m=[round(v, 6) for v in size_m(doc, binary)],
            bones=joints,
            clips=clips,
            details=details,
        )
        if animation_jobs:
            data["licences"].update({j["pack"]: j["licence"] for j in animation_jobs})
            data["animation_binding"] = "native"
            by_name = {job["id"]: job for job in animation_jobs}
            for clip in data["clips"]:
                source_clip = by_name[clip["name"]]
                clip.update(
                    catalog_id=source_clip["catalog_id"], pack=source_clip["pack"]
                )
        if selection:
            data.update(selection, source_file=row["source"])
        if "hand_templates" in row:
            data["hand_templates"] = row["hand_templates"]
        if "intake" in row:
            data.update(
                intake=row["intake"],
                anchors=row.get("anchors", {}),
                anchor_space=row.get("anchor_space"),
            )
        receipt = Path(tmp) / "manifest.json"
        receipt.write_text(json.dumps(data, indent=2) + "\n")
        os.link(target, dest)
        try:
            os.link(receipt, manifest)
        except BaseException:
            dest.unlink()
            raise
    progress(100, "Exported " + dest.name)


class Parser(argparse.ArgumentParser):
    def error(self, msg):
        raise ValueError(msg)


def cancel(signum, frame):
    raise InterruptedError("cancelled")


def main():
    signal.signal(signal.SIGTERM, cancel)
    signal.signal(signal.SIGINT, cancel)
    try:
        parser = Parser(prog="forge-library", description=__doc__)
        commands = parser.add_subparsers(dest="cmd", required=True)
        search = commands.add_parser("find")
        search.add_argument("text")
        search.add_argument("--kind", choices=KINDS)
        get = commands.add_parser("get")
        get.add_argument("id")
        get.add_argument("--output", type=Path, required=True)
        assemble = commands.add_parser("assemble")
        assemble.add_argument("base")
        assemble.add_argument("parts", nargs="+")
        assemble.add_argument("--output", type=Path, required=True)
        args = parser.parse_args()
        rows = json.loads(CATALOG.read_text())
        if args.cmd == "find":
            matches = find(rows, args.text, args.kind)
            print(
                json.dumps(
                    [
                        {
                            k: r[k]
                            for k in (
                                "id",
                                "name",
                                "plain_name",
                                "pack",
                                "kind",
                                "licence",
                                "thumbnail",
                            )
                        }
                        for r in matches
                    ],
                    indent=2,
                )
            )
        else:
            lookup = {r["id"]: r for r in rows}

            def entry(key):
                if key not in lookup:
                    raise ValueError("Unknown library id: " + key)
                return lookup[key]

            export(
                entry(args.id if args.cmd == "get" else args.base),
                [entry(k) for k in getattr(args, "parts", [])],
                args.output,
                args.cmd,
            )
    except Exception as err:
        print(json.dumps({"error": str(err)}), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
