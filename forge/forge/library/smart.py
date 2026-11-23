"""Resolve one planner item to a licensed library asset, or return a structured miss."""

from collections import Counter
import json
import math
from pathlib import Path
import re
import sys

from .cli import CATALOG, eligible, export, progress
from .names import SYNONYMS, tokens
from .looks import looks, clip_jobs

KINDS = ("prop", "character", "scenery")
WORDS = {word: group[0] for group in SYNONYMS for word in group}
WORDS.update(golden="gold", coin="coin", coins="coin", keys="key", wooden="wood")
# Planner words for things the packs name differently (an oak is one of the packs' trees).
WORDS.update(oak="tree", elm="tree", beech="tree", willow="tree", fir="pine", spruce="pine",
             hedge="bush", hedges="bush", shrub="bush", shrubs="bush",
             bookshelf="bookcase", bookshelves="bookcase")
FILLER = set(
    "a an the of some set to my for please make create model asset piece object game stylized small large little big tiny very old new realistic medieval fantasy weathered beautifully detailed forest garden kitchen".split()
)
MODIFIERS = set(
    "red green blue black white brown gold silver bronze wood wooden metal stone simple round long short iron plain light grey empty open closed twin common normal".split()
)
# Pack names that file tableware under "Table": a "Table Plate" is a plate, not a table.
PREFIXES = {"table"}
COLLECTIONS = {"coins": "pile"}
SPLIT_CONTEXT = re.compile(
    r"\b(?:full of|filled with|containing|holding|carrying|stuck in|lying on|standing on|sitting on|next to|with|on the|in the)\b",
    re.I,
)


def normalized(text):
    # Numbers only label copies ("Log seat 2", "Pine 3"): they never identify a thing.
    return [WORDS.get(w, w) for w in tokens(text) if w not in FILLER and not w.isdigit()]


def rank(rows, phrase, description=""):
    terms = set(normalized(phrase))
    if not terms:
        return []
    context = set(normalized(description))
    plural = {v for k, v in COLLECTIONS.items() if k in tokens(phrase)}
    ranked = []
    for row in rows:
        name = set(normalized(row["name"])) - {"prop", "environment", "superhero"}
        # Tags include folder names; a folder alone is insufficient evidence of identity.
        if row.get("base_id"):
            name |= {"character", "person", "human", "humanoid"}
        identity = name - MODIFIERS
        if not identity:
            identity = name
        if len(identity) > 1:
            identity -= PREFIXES
        if not terms & identity:
            continue
        style = set(tokens(row["pack"])) - {"kit", "megakit", "modular", "props"}
        words = name | style | {WORDS.get(w, w) for w in row["tags"]}
        matched = terms & words
        coverage = len(matched) / len(terms)
        precision = len(name & terms) / max(1, len(name))
        score = min(1.0, 0.80 * coverage + 0.20 * precision)
        if identity == terms:
            score = 1.0
        if plural and "coin" in name:
            score = min(1.0, score + 0.1) if plural & name else score * 0.83
        if context:
            score = 0.85 * score + 0.15 * len(context & words) / len(context)
        # The asset names a different thing that neither the item nor its description mentions
        # ("Book Stand" for a microphone stand, "Anvil Log" for a log to sit on): likely wrong.
        if identity - terms - context - plural:
            score *= 0.7
        ranked.append((round(score, 4), row))

    # An unqualified chest in a treasure scene uses the wooden fantasy prop;
    # explicit science-fiction descriptions select the corresponding themed kit.
    def order(pair):
        score, row = pair
        look_order = {
            "female-ranger": 0,
            "male-peasant": 1,
            "female-peasant": 2,
            "male-ranger": 3,
        }
        return (
            -score,
            look_order.get(row["id"].split("/")[-1], 4),
            row["pack"] != "fantasy-props-megakit",
            len(tokens(row["name"])),
            row["id"],
        )

    return sorted(ranked, key=order)


def identity(row, score):
    return dict(
        id=row["id"],
        pack=row["pack"],
        licence=row["licence"],
        plain_name=row["plain_name"],
        score=score,
    )


def phrases(query):
    # Otto sends either a sentence or an item name followed by its description.
    clauses = re.split(r"[.!?\n:]+", query)
    pieces = SPLIT_CONTEXT.split(clauses[0])
    primary = pieces[0].strip()
    secondary = SPLIT_CONTEXT.split(query)
    description = SPLIT_CONTEXT.split(" ".join(clauses[1:]))[0]
    return primary, description, secondary[1:]


def summary(rows):
    result = {}
    for kind in KINDS:
        items = [r for r in rows if r["kind"] == kind]
        examples = {
            "prop": (
                "chest",
                "coin",
                "key",
                "sword",
                "shield",
                "barrel",
                "potion",
                "axe",
                "crate",
                "chair",
            ),
            "scenery": (
                "tree",
                "rock",
                "wall",
                "door",
                "floor",
                "roof",
                "window",
                "column",
                "gate",
                "house",
                "bush",
            ),
            "character": ("ranger", "peasant", "female", "male", "alien", "drone"),
        }[kind]
        result[kind] = dict(
            count=len(items),
            categories=[
                name for name in examples if any(name in r["tags"] for r in items)
            ],
            packs=dict(sorted(Counter(r["pack"] for r in items).items())),
        )
    return {"kinds": result}


def stage(args):
    catalog = [r for r in json.loads(CATALOG.read_text()) if eligible(r)]
    rows = [r for r in catalog if r["kind"] in ("prop", "scenery")] + looks(catalog)
    if args.list_kinds:
        print(json.dumps(summary(rows), separators=(",", ":")))
        return 0
    if not args.prompt.strip() and args.kind == "character":
        args.prompt = "character"
    if not args.output or not args.prompt.strip():
        raise ValueError("library requires --prompt and --output")
    kind = args.kind or "any"
    if kind not in (*KINDS, "any"):
        raise ValueError("--kind must be prop, character, scenery or any")
    threshold = float(args.min_score)
    if not math.isfinite(threshold) or not 0 <= threshold <= 1:
        raise ValueError("--min-score must be between 0 and 1")
    candidates = [r for r in rows if kind == "any" or r["kind"] == kind]
    primary, description, other = phrases(args.prompt)
    matches = rank(candidates, primary, description)
    score, row = matches[0] if matches else (0, None)
    if not row or score < threshold:
        print(
            json.dumps(
                {
                    "error": "no library match",
                    "best": identity(row, score) if row else {},
                    "score": score,
                },
                separators=(",", ":"),
            ),
            file=sys.stderr,
        )
        return 1
    progress(1, "Found " + row["plain_name"])
    additional = {}
    for phrase in other:
        ranked = rank(candidates, phrase)
        if ranked and ranked[0][0] >= threshold:
            s, r = ranked[0]
            if r["id"] != row["id"]:
                additional[r["id"]] = identity(r, s)
    selection = dict(
        **identity(row, score),
        source="library",
        prompt=args.prompt,
        related_pieces=list(additional.values()),
    )
    if row.get("base_id"):
        base = next(r for r in catalog if r["id"] == row["base_id"])
        parts = [r for r in catalog if r["id"].startswith(row["id"] + "-")]
        if not parts:
            raise ValueError("Missing modular outfit parts for " + row["id"])
        selection["base_id"] = base["id"]
        export(
            base,
            parts,
            Path(args.output),
            "library",
            selection=selection,
            animation_jobs=clip_jobs(args.clips),
        )
    else:
        export(row, [], Path(args.output), "library", selection=selection)
    return 0
