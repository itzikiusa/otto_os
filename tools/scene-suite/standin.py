"""Stand-in models for the scene suite: tiny GLBs of boxes with a plausible
shape per kind (a person is a tall body with a head, arms and a skeleton the
way Otto Forge rigs it; a table is a top on four legs; a basket is an open
frame), at the item's planned size. Each GLB comes with a sidecar of its parts
(local boxes) and joints, so the checks can measure the placed scene exactly.

    python3 standin.py plan.json OUTDIR   # writes <item>.glb + <item>.parts.json
"""
import json
import struct
import sys
from pathlib import Path

HUMAN = set("""man woman boy girl child children kid person people lady gentleman robot keeper giant
wizard witch waiter waitress singer guitarist drummer player trumpeter musician princess prince
king queen knight pirate farmer teacher friend mr mrs miss grandma grandpa granny grandfather
grandmother student baby fighter warrior hero soldier rider cyclist ogre troll elf dwarf
snowman maya ben lee anna sailor captain chef guard monster""".split())
QUAD = set("frog toad turtle dog puppy cat kitten fox mouse horse pony cow pig sheep lion tiger bear wolf rabbit bunny dragon dragun deer goat".split())
BIRD = set("duck bird canary owl parrot chicken hen goose seagull gull penguin".split())
FISH = set("fish goldfish shark whale dolphin".split())
SNAKE = set("snake serpent worm".split())
SPIDER = set("octopus squid spider crab ant beetle bug insect".split())

# Object shapes: (aspect x, y, z with the longest side 1, parts kind). Earlier words win.
OBJECTS = [
    (["pier", "deck", "dock", "jetty", "boardwalk"], (0.5, 0.15, 1.0), "slab_posts"),
    (["corner", "room", "walls", "wall", "kitchen", "bedroom", "classroom", "hall", "tower"], (1.0, 0.7, 1.0), "corner"),
    (["beach", "sand", "ground", "field", "lawn", "grass", "road", "street", "island", "lake", "sea", "water",
      "snow", "yard", "meadow", "clearing", "pavement", "garden", "dust", "seabed", "floor", "path", "desert"], (1.0, 0.02, 1.0), "ground"),
    (["curtain", "backdrop"], (1.0, 0.8, 0.05), "solid"),
    (["earth", "planet", "sun", "moon"], (1.0, 1.0, 1.0), "solid"),
    (["steam", "smoke", "bubble", "trail", "cloud", "mist"], (0.5, 1.0, 0.5), "solid"),
    (["seaweed", "spotlight", "bass", "cello", "cactus"], (0.35, 1.0, 0.3), "solid"),
    (["stage", "platform", "stand"], (1.0, 0.15, 0.7), "solid"),
    (["table", "desk", "counter"], (1.0, 0.62, 0.6), "table"),
    (["armchair", "sofa", "couch"], (1.0, 0.9, 0.9), "armchair"),
    (["chair", "stool", "seat"], (0.5, 1.0, 0.5), "chair"),
    (["bench", "log"], (1.0, 0.35, 0.3), "solid"),
    (["bookshelf", "bookcase", "shelf", "shelves", "cupboard", "cabinet", "dresser"], (0.6, 1.0, 0.25), "shelf"),
    (["cage", "birdcage"], (0.7, 1.0, 0.7), "frame"),
    (["basket", "bowl", "pot", "bucket", "tank", "aquarium", "cup", "mug", "glass", "jar", "vase", "tub", "pan", "kettle", "box"], (1.0, 0.7, 1.0), "frame"),
    (["crate", "chest", "trunk", "radio", "drum", "kit"], (1.0, 0.65, 0.65), "solid"),
    (["bed", "mattress"], (0.55, 0.3, 1.0), "solid"),
    (["plate", "quilt", "blanket", "sheet", "towel", "scarf", "tray", "rug", "mat", "carpet", "tablecloth", "napkin", "puddle", "pond", "map", "paper", "letter"], (1.0, 0.04, 0.75), "solid"),
    (["book", "spellbook"], (0.75, 0.15, 1.0), "solid"),
    (["lighthouse", "tower", "lamp", "post", "pole", "tree", "candle", "bottle", "microphone", "mic", "telescope",
      "hourglass", "perch", "broom", "flower", "torch", "quill", "pen", "stick", "flag", "sign", "stop", "statue",
      "snowman", "oak", "pine", "fir", "birch", "willow", "palm", "maple", "spruce", "cedar"], (0.3, 1.0, 0.3), "solid"),
    (["hedge", "bush", "shrub", "fence"], (1.0, 0.25, 0.15), "solid"),
    (["umbrella", "parasol"], (1.0, 0.85, 1.0), "umbrella"),
    (["tent", "house", "castle", "castel", "hut", "barn", "shop"], (1.0, 0.75, 0.9), "solid"),
    (["boat", "sled", "sledge", "canoe", "car", "bus", "cart", "wagon", "train", "bicycle", "bike", "scooter", "skateboard"], (0.35, 0.45, 1.0), "solid"),
    (["horse", "rocking"], (0.3, 0.75, 1.0), "solid"),
    (["knife", "fork", "spoon", "oar", "paddle", "sword", "wand", "rope", "string", "ribbon"], (0.08, 0.04, 1.0), "solid"),
    (["guitar", "violin", "trumpet", "saxophone", "flute"], (0.35, 0.15, 1.0), "solid"),
    (["coil"], (1.0, 0.25, 1.0), "solid"),
    (["fire", "campfire", "bonfire"], (1.0, 0.6, 1.0), "solid"),
    (["rock", "stone", "boulder"], (1.0, 0.45, 0.8), "solid"),
    (["ball", "snowball", "balloon", "apple", "orange", "egg", "hat", "crystal", "globe", "lantern", "cake", "pie"], (1.0, 0.9, 1.0), "solid"),
]


def words(text):
    out = []
    for w in "".join(c.lower() if c.isalpha() else " " for c in text or "").split():
        out.append(w)
        if w.endswith("s"):
            out.append(w[:-1])
    return out


def item_words(it):
    # The name decides first, by its head word: the last one ("Brass
    # telescope"), the one before "of" ("Coil of rope"); then the description.
    name = words(it.get("name", ""))
    cut = next((k for k, w in enumerate(name) if w in ("of", "on", "in", "with", "for")), None)
    if cut:
        name = name[cut + 1:] + name[:cut]
    return list(reversed(name)) + words(it.get("description", ""))


def classify(it):
    ws = item_words(it)
    if it.get("kind") == "character":
        for w in ws:
            if w in HUMAN:
                return "human"
            for group, kind in ((QUAD, "quad"), (BIRD, "bird"), (FISH, "fish"), (SNAKE, "snake"), (SPIDER, "spider")):
                if w in group:
                    return kind
        return "human"
    for w in ws:
        for keys, aspect, parts in OBJECTS:
            if w in keys:
                return parts + ":" + ",".join(str(a) for a in aspect)
    return "solid:1.0,0.8,0.8"


def box(lo, hi, part="body"):
    return {"min": [round(v, 5) for v in lo], "max": [round(v, 5) for v in hi], "part": part}


# Joint layout (fractions of the height, feet at 0, facing +z, the right hand
# at -x), from the Otto Forge rig of the owner's robot (Hips .53 … Neck .92).
JOINTS = {
    "Hips": ("Armature", (0, .53, 0)), "Spine": ("Hips", (0, .58, 0)), "Chest": ("Spine", (0, .65, 0)),
    "UpperChest": ("Chest", (0, .72, 0)), "Neck": ("UpperChest", (0, .86, 0)), "Head": ("Neck", (0, .88, 0)),
    "LeftShoulder": ("UpperChest", (.06, .82, 0)), "LeftUpperArm": ("LeftShoulder", (.17, .80, 0)),
    "LeftLowerArm": ("LeftUpperArm", (.23, .64, 0)), "LeftHand": ("LeftLowerArm", (.27, .50, .03)),
    "RightShoulder": ("UpperChest", (-.06, .82, 0)), "RightUpperArm": ("RightShoulder", (-.17, .80, 0)),
    "RightLowerArm": ("RightUpperArm", (-.23, .64, 0)), "RightHand": ("RightLowerArm", (-.27, .50, .03)),
    "LeftUpperLeg": ("Hips", (.08, .50, 0)), "LeftLowerLeg": ("LeftUpperLeg", (.09, .27, 0)),
    "LeftFoot": ("LeftLowerLeg", (.09, .04, 0)), "RightUpperLeg": ("Hips", (-.08, .50, 0)),
    "RightLowerLeg": ("RightUpperLeg", (-.09, .27, 0)), "RightFoot": ("RightLowerLeg", (-.09, .04, 0)),
}


def human_parts(h):
    s = h
    return [
        box((-.13 * s, 0, -.06 * s), (-.03 * s, .5 * s, .06 * s), "leg"),
        box((.03 * s, 0, -.06 * s), (.13 * s, .5 * s, .06 * s), "leg"),
        box((-.16 * s, .5 * s, -.09 * s), (.16 * s, .84 * s, .09 * s), "torso"),
        box((-.08 * s, .84 * s, -.08 * s), (.08 * s, 1.0 * s, .08 * s), "head"),
        box((-.03 * s, .9 * s, .08 * s), (.03 * s, .94 * s, .12 * s), "nose"),
        box((-.30 * s, .46 * s, -.04 * s), (-.20 * s, .82 * s, .04 * s), "arm_right"),
        box((.20 * s, .46 * s, -.04 * s), (.30 * s, .82 * s, .04 * s), "arm_left"),
    ]


def creature_parts(kind, h):
    if kind == "quad":   # height h, length 2h
        L = 2.0 * h
        return [
            box((-.2 * h, .35 * h, -.5 * L), (.2 * h, .7 * h, .3 * L), "torso"),
            box((-.15 * h, .55 * h, .3 * L), (.15 * h, 1.0 * h, .5 * L), "head"),
            box((-.05 * h, .7 * h, .5 * L), (.05 * h, .8 * h, .5 * L + .1 * h), "nose"),
        ] + [box((x - .06 * h, 0, z - .06 * h), (x + .06 * h, .35 * h, z + .06 * h), "leg")
             for x in (-.13 * h, .13 * h) for z in (-.4 * L, .2 * L)]
    if kind == "bird":   # height h, length 1.2h
        return [
            box((-.25 * h, .15 * h, -.6 * h), (.25 * h, .65 * h, .35 * h), "torso"),
            box((-.15 * h, .6 * h, .1 * h), (.15 * h, 1.0 * h, .45 * h), "head"),
            box((-.05 * h, .75 * h, .45 * h), (.05 * h, .85 * h, .6 * h), "nose"),
            box((-.1 * h, 0, -.05 * h), (.1 * h, .15 * h, .05 * h), "leg"),
        ]
    if kind == "fish":   # height h, length 2h
        return [box((-.25 * h, 0, -1.0 * h), (.25 * h, 1.0 * h, .8 * h), "torso"),
                box((-.08 * h, .45 * h, .8 * h), (.08 * h, .6 * h, 1.0 * h), "nose")]
    if kind == "snake":  # a coil: height h, 4h across
        return [box((-2 * h, 0, -2 * h), (2 * h, .6 * h, 2 * h), "torso"),
                box((-.3 * h, .6 * h, 1.2 * h), (.3 * h, 1.0 * h, 2.0 * h), "head"),
                box((-.1 * h, .75 * h, 2.0 * h), (.1 * h, .85 * h, 2.3 * h), "nose")]
    # spider: height h, legs 3h across
    return [box((-.4 * h, .3 * h, -.5 * h), (.4 * h, 1.0 * h, .5 * h), "torso"),
            box((-1.5 * h, 0, -.1 * h), (1.5 * h, .3 * h, .1 * h), "leg"),
            box((-1.3 * h, 0, -.8 * h), (1.3 * h, .3 * h, -.6 * h), "leg"),
            box((-1.3 * h, 0, .6 * h), (1.3 * h, .3 * h, .8 * h), "leg"),
            box((-.1 * h, .5 * h, .5 * h), (.1 * h, .7 * h, .7 * h), "nose")]


def object_parts(kind, aspect, size):
    x, y, z = (a * size for a in aspect)
    hx, hz = x / 2, z / 2
    t = max(0.02 * size, 0.005)
    if kind == "table":
        legs = [box((sx * hx - (2 * t if sx > 0 else 0), 0, sz * hz - (2 * t if sz > 0 else 0)),
                    (sx * hx + (0 if sx > 0 else 2 * t), y - 2 * t, sz * hz + (0 if sz > 0 else 2 * t)), "leg")
                for sx in (-1, 1) for sz in (-1, 1)]
        return [box((-hx, y - 2 * t, -hz), (hx, y, hz), "top")] + legs
    if kind == "slab_posts":
        posts = [box((sx * hx * .8 - t * 2, 0, sz * hz * .8 - t * 2), (sx * hx * .8 + t * 2, y * .7, sz * hz * .8 + t * 2), "leg")
                 for sx in (-1, 1) for sz in (-1, 0, 1)]
        return [box((-hx, y * .7, -hz), (hx, y, hz), "top")] + posts
    if kind == "chair":
        return [box((-hx, .45 * y - 2 * t, -hz), (hx, .45 * y, hz), "seat"),
                box((-hx, .45 * y, -hz), (hx, y, -hz + 2 * t), "back")] + [
            box((sx * hx - t, 0, sz * hz - t), (sx * hx + t, .45 * y - 2 * t, sz * hz + t), "leg")
            for sx in (-.9, .9) for sz in (-.9, .9)]
    if kind == "armchair":
        return [box((-hx, 0, -hz), (hx, .45 * y, hz), "seat"),
                box((-hx, .45 * y, -hz), (hx, y, -hz + .2 * z), "back"),
                box((-hx, .45 * y, -hz), (-hx + .15 * x, .7 * y, hz), "arm"),
                box((hx - .15 * x, .45 * y, -hz), (hx, .7 * y, hz), "arm")]
    if kind == "shelf":
        boards = [box((-hx, f * y - t, -hz), (hx, f * y + (t if f < 1 else 0), hz), "shelf")
                  for f in (.02, .35, .68, 1.0)]
        return boards + [box((-hx, 0, -hz), (-hx + 2 * t, y, hz), "side"),
                         box((hx - 2 * t, 0, -hz), (hx, y, hz), "side"),
                         box((-hx, 0, -hz), (hx, y, -hz + t), "back")]
    if kind == "frame":  # an open container: floor, corner posts, rim
        p = max(t, 0.01 * size)
        posts = [box((sx * hx - (2 * p if sx > 0 else 0), 0, sz * hz - (2 * p if sz > 0 else 0)),
                     (sx * hx + (0 if sx > 0 else 2 * p), y, sz * hz + (0 if sz > 0 else 2 * p)), "post")
                 for sx in (-1, 1) for sz in (-1, 1)]
        rim = [box((-hx, y - p, -hz), (hx, y, -hz + p), "rim"), box((-hx, y - p, hz - p), (hx, y, hz), "rim"),
               box((-hx, y - p, -hz), (-hx + p, y, hz), "rim"), box((hx - p, y - p, -hz), (hx, y, hz), "rim")]
        return [box((-hx, 0, -hz), (hx, 2 * p, hz), "floor")] + posts + rim
    if kind == "corner":  # a floor with a back wall and a left wall
        return [box((-hx, 0, -hz), (hx, 2 * t, hz), "floor"), box((-hx, 0, -hz), (hx, y, -hz + 2 * t), "wall"),
                box((-hx, 0, -hz), (-hx + 2 * t, y, hz), "wall")]
    if kind == "umbrella":
        return [box((-hx, .8 * y, -hz), (hx, y, hz), "canopy"), box((-t, 0, -t), (t, .8 * y, t), "stick")]
    if kind == "ground":  # a big flat ground piece stays a few centimetres thick
        return [box((-hx, 0, -hz), (hx, min(y, 0.05), hz), "top")]
    return [box((-hx, 0, -hz), (hx, y, hz), "body")]


def size_of(it):
    s = (it.get("details") or {}).get("size_m")
    if s:
        return float(s)
    return 1.8 if it.get("kind") == "character" else 0.6


def standin(it):
    """(parts, joints, shape) for one character or object item."""
    shape = classify(it)
    size = size_of(it)
    joints = {}
    if shape == "human":
        parts = human_parts(size)
        joints = {k: [round(v * size, 5) for v in p] for k, (_, p) in JOINTS.items()}
    elif it.get("kind") == "character":
        parts = creature_parts(shape, size)
    else:
        kind, aspect = shape.split(":")
        parts = object_parts(kind, tuple(float(a) for a in aspect.split(",")), size)
    return parts, joints, shape


COLORS = {"nose": (0.9, 0.1, 0.1), "head": (0.95, 0.8, 0.6), "arm_right": (0.2, 0.4, 0.95), "arm_left": (0.95, 0.85, 0.2)}


def glb(parts, joints):
    """A GLB: one mesh (a primitive per part), a skeleton beside it when joints."""
    bin_ = bytearray()
    views, accessors, prims, materials = [], [], [], []
    for p in parts:
        lo, hi = p["min"], p["max"]
        faces = [((0, -1), [(0, 0, 0), (0, 0, 1), (0, 1, 1), (0, 1, 0)]), ((0, 1), [(1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)]),
                 ((1, -1), [(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)]), ((1, 1), [(0, 1, 0), (0, 1, 1), (1, 1, 1), (1, 1, 0)]),
                 ((2, -1), [(0, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 0)]), ((2, 1), [(0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)])]
        pos, nrm, idx = [], [], []
        for (axis, sign), quad in faces:
            base = len(pos)
            for c in quad:
                pos.append([hi[k] if c[k] else lo[k] for k in range(3)])
                n = [0.0, 0.0, 0.0]
                n[axis] = float(sign)
                nrm.append(n)
            idx += [base, base + 1, base + 2, base, base + 2, base + 3]
        for data, fmt, kind, target in ((pos, "<3f", "VEC3", 34962), (nrm, "<3f", "VEC3", 34962), (idx, "<H", "SCALAR", 34963)):
            off = len(bin_)
            for v in data:
                bin_ += struct.pack(fmt, *v) if kind == "VEC3" else struct.pack(fmt, v)
            while len(bin_) % 4:
                bin_ += b"\0"
            views.append({"buffer": 0, "byteOffset": off, "byteLength": len(bin_) - off, "target": target})
            acc = {"bufferView": len(views) - 1, "componentType": 5126 if kind == "VEC3" else 5123, "count": len(data), "type": kind}
            if data is pos:
                acc["min"] = [min(v[k] for v in pos) for k in range(3)]
                acc["max"] = [max(v[k] for v in pos) for k in range(3)]
            accessors.append(acc)
        c = COLORS.get(p["part"], (0.75, 0.75, 0.72))
        materials.append({"name": p["part"], "pbrMetallicRoughness": {"baseColorFactor": [*c, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.9}})
        a = len(accessors)
        prims.append({"attributes": {"POSITION": a - 3, "NORMAL": a - 2}, "indices": a - 1, "material": len(materials) - 1})
    nodes = [{"name": "standin", "mesh": 0}]
    doc = {"asset": {"version": "2.0", "generator": "otto scene-suite stand-in"}, "scene": 0,
           "meshes": [{"primitives": prims}], "materials": materials, "accessors": accessors, "bufferViews": views,
           "buffers": [{"byteLength": len(bin_)}]}
    if joints:
        names = list(joints)
        nodes.append({"name": "Armature", "children": []})
        index = {"Armature": 1}
        for n in names:
            index[n] = len(nodes)
            nodes.append({"name": n})
        for n in names:
            parent = JOINTS[n][0]
            ppos = joints.get(parent, [0.0, 0.0, 0.0])
            nodes[index[n]]["translation"] = [round(joints[n][k] - ppos[k], 5) for k in range(3)]
            nodes[index[parent]].setdefault("children", []).append(index[n])
        doc["skins"] = [{"joints": [index[n] for n in names]}]
        doc["scenes"] = [{"nodes": [0, 1]}]
    else:
        doc["scenes"] = [{"nodes": [0]}]
    doc["nodes"] = nodes
    js = json.dumps(doc, separators=(",", ":")).encode()
    while len(js) % 4:
        js += b" "
    total = 12 + 8 + len(js) + 8 + len(bin_)
    return (b"glTF" + struct.pack("<II", 2, total) + struct.pack("<I", len(js)) + b"JSON" + js
            + struct.pack("<I", len(bin_)) + b"BIN\0" + bytes(bin_))


def main(plan_path, out_dir):
    plan = json.loads(Path(plan_path).read_text())
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    for it in plan.get("items", []):
        if it.get("kind") not in ("character", "object"):
            continue
        parts, joints, shape = standin(it)
        (out / (it["id"] + ".glb")).write_bytes(glb(parts, joints))
        (out / (it["id"] + ".parts.json")).write_text(json.dumps({"shape": shape, "parts": parts, "joints": joints}))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
