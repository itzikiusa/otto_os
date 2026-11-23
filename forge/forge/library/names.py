"""Asset search words and their common equivalents."""

import re


def tokens(label):
    return re.findall(
        "[a-z]+|[0-9]+", re.sub(r"([a-z])([A-Z])", r"\1 \2", label).lower()
    )


SYNONYMS = [
    ("woman", "female"),
    ("man", "male"),
    ("wood", "wooden"),
    ("sword", "blade", "cutlass"),
    ("chest", "treasure"),
    ("coins", "coin"),
    ("tree", "trees"),
    ("rock", "stone", "rocks"),
    ("house", "building", "houses"),
    ("wall", "walls"),
    ("gate", "gateway"),
    ("barrel", "cask"),
    ("axe", "hatchet"),
    ("potion", "elixir"),
    ("key", "keys"),
    ("bow", "bows"),
    ("arrow", "arrows"),
    ("quiver", "quivers"),
    ("shield", "shields"),
    ("run", "running"),
    ("walk", "walking"),
]


def tags(name, source):
    result = set(tokens(name))
    for group in SYNONYMS:
        if result.intersection(group):
            result.update(group)
    if "coin" in result:
        result.add("gold")
    path = source.parts
    if "files" in path:
        for folder in path[path.index("files") + 1 : -1]:
            if not any(
                s in folder.lower()
                for s in [
                    "standard",
                    "quaternius",
                    "megakit",
                    "unreal",
                    "gltf",
                    "godot",
                    "unity",
                ]
            ):
                result.update(tokens(folder))
    return sorted(result)


def plain_name(label):
    return " ".join(tokens(label)).title()
