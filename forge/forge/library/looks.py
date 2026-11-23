"""Assemble the publisher's four full outfits and bind selected UAL takes natively."""

from .animation import library_clips
from .import_packs import LIB

ALIASES = {
    "idle": "ual1-idle-loop",
    "walk": "ual1-walk-loop",
    "run": "ual1-jog-fwd-loop",
}


def looks(rows):
    by_id = {r["id"]: r for r in rows}
    result = []
    for style, body in (
        ("ranger", "female"),
        ("peasant", "male"),
        ("peasant", "female"),
        ("ranger", "male"),
    ):
        base_id = f"universal-base-characters/superhero-{body}-fullbody"
        outfit_id = f"modular-character-outfits-fantasy/{body}-{style}"
        if outfit_id in by_id and base_id in by_id:
            outfit = by_id[outfit_id]
            result.append(
                dict(
                    outfit,
                    kind="character",
                    base_id=base_id,
                    tags=[*outfit["tags"], "character", "person", "human", "humanoid"],
                )
            )
    return result


def clip_jobs(clips):
    sources = library_clips(LIB)["clips"]
    by_id = {r["id"]: r for r in sources}
    requested = [s.strip() for s in clips.split(",")] if clips else list(ALIASES)
    if len(set(requested)) != len(requested) or not all(requested):
        raise ValueError(
            "Give distinct comma-separated UAL clip IDs or idle, walk, run"
        )
    jobs = []
    for name in requested:
        source = by_id.get(ALIASES.get(name, name))
        if source is None:
            # Publisher action names refer to the ordinary edition. Use a catalog ID for RM.
            hits = [
                r
                for r in sources
                if r["action"].casefold() == name.casefold()
                and not r["id"].endswith("-rm")
            ]
            if len(hits) != 1:
                raise ValueError(
                    f"Unknown or ambiguous UAL clip '{name}'. Use a full UAL catalog ID."
                )
            source = hits[0]
        jobs.append(
            dict(
                source, file=str(LIB / source["file"]), id=name, catalog_id=source["id"]
            )
        )
    return jobs
