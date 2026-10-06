"""Otto School asset build — rebuilds every file in ui/public/school/ from
ui/assets-src/school/ and writes manifest.json (see ../CONTRACT.md).

    Blender -b -P ui/assets-src/school/blender/build_school.py -- [--only kit,kids,headmaster]
                                                                  [--kids claude,codex] [--verify]

`--verify` builds nothing: it re-opens the outputs and exits non-zero on any
missing node/clip/material, a blown budget or a stale manifest.
"""

import json
import os
import sys
import traceback

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from lib import glbinfo  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.dirname(HERE)
REPO_UI = os.path.dirname(os.path.dirname(SRC))
OUT = os.path.join(REPO_UI, "public", "school")

PROVIDERS = ["claude", "codex", "grok", "agy", "shell", "custom"]
KID_CLIPS = ["Sit_Type", "Sit_Idle", "Sit_RaiseHand", "Sit_Slump", "Stand_Up", "Sit_Down", "Walk", "Idle_Stand",
             "Talk", "Look_Around", "Wave"]
HM_CLIPS = ["Idle", "Walk", "Nod", "Scold", "Wave", "ThumbsUp", "Inspect_Screen", "Point_Board"]
KIT_NODES = ["Workstation", "Chair", "TeacherDesk", "Board", "Bookshelf", "Plant", "Clock", "Bench",
             "Poster_1", "Poster_2", "Poster_3", "Poster_4", "Poster_5", "Poster_6",
             "Floor_Tile", "Wall", "Wall_Window", "Wall_Door", "Corridor_Floor", "Corridor_Wall",
             "Corridor_Door", "Ceiling_Light"]
# parent node → (child node, material name or None)
PAINTED = {"Workstation": ("Screen", "ScreenMat"), "Board": ("BoardTitle", None),
           "Corridor_Door": ("NamePlate", None)}
CHILDREN = {"Workstation": ["Screen"], "Board": ["BoardTitle"], "Wall_Door": ["DoorLeaf"],
            "Corridor_Door": ["DoorLeaf", "NamePlate", "DoorSign"]}
PAINTED_PLANES = ["Screen", "BoardTitle", "NamePlate", "DoorSign"]
BUDGET = {"kid": 8000, "headmaster": 12000, "kit": 40000}
MAX_TEX = 1024
MAX_TOTAL = 15 * 1024 * 1024  # raised from 10 MB by the lead for the polish pass


ENV_SRC = os.path.join(SRC, "env", "empty_play_room_1k.hdr")  # Poly Haven, CC0
EXTRA_NODES = ["Backpack", "Trashcan", "Plant_Desk", "Book", "Book_Open", "Water_Bottle", "Corkboard"]


def files():
    return [f"kid-{p}.glb" for p in PROVIDERS] + ["headmaster-otto.glb", "school-kit.glb"]


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    a = {"only": {"kit", "kids", "headmaster"}, "kids": PROVIDERS, "verify": False}
    i = 0
    while i < len(argv):
        if argv[i] == "--verify":
            a["verify"] = True
        elif argv[i] == "--only":
            a["only"] = set(argv[i + 1].split(","))
            i += 1
        elif argv[i] == "--kids":
            a["kids"] = argv[i + 1].split(",")
            i += 1
        i += 1
    return a


# ---------------------------------------------------------------- manifest

NOTES = {
    "units": "metres, Y up, fronts face +Z, origins on the floor unless noted",
    "paintedPlanes": "Screen, BoardTitle, NamePlate, DoorSign: TEXCOORD_0 (0,0) = bottom-left as seen, "
                     "(1,1) = top-right — i.e. use texture.flipY = true (three.js CanvasTexture default). "
                     "Each has its own material; the embedded default map is stored pre-flipped.",
    "Board": "origin x-centre, y = 0 (floor), z = wall surface; board spans y 0.9–2.3",
    "Ceiling_Light": "origin at the emitting face centre (y = 0), housing rises to y = 0.1",
    "Clock": "child nodes Clock_HourHand / Clock_MinuteHand pivot at the centre about Z (rest: 10:10)",
    "DoorLeaf": "origin on the hinge (−X edge); rotate about Y to open",
    "kidSeat": "Sit_* clips: kid origin on the Chair origin; Workstation at (0, 0, +0.55)",
    "Stand_Up": "in place: ends standing on the chair origin — slide the kid ±X into the aisle while it plays",
    "extras": "optional decor nodes in school-kit.glb: " + ", ".join(EXTRA_NODES) +
              " (floor-origin, front +Z; Corkboard origin at the wall-surface centre)",
}


def write_manifest():
    data = {"version": 1, "files": {}, "notes": NOTES}
    import hashlib
    raw = open(os.path.join(OUT, "env.hdr"), "rb").read()
    data["env"] = {"file": "env.hdr", "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest(),
                   "source": "Poly Haven 'Empty Play Room' 1k (CC0)", "loader": "RGBELoader"}
    for f in files():
        path = os.path.join(OUT, f)
        g = glbinfo.Glb(path)
        entry = {"bytes": g.bytes, "sha256": g.sha256, "tris": g.tris(), "clips": g.clips(),
                 "nodes": g.top_node_names() if f == "school-kit.glb" else []}
        if f != "school-kit.glb":
            entry["durations"] = {c: round(g.clip_duration(c), 4) for c in g.clips()}
        data["files"][f] = entry
    with open(os.path.join(OUT, "manifest.json"), "w") as fh:
        json.dump(data, fh, indent=2)
        fh.write("\n")
    return data


# ---------------------------------------------------------------- verify

def verify():
    errs = []
    total = 0
    man_path = os.path.join(OUT, "manifest.json")
    man = json.load(open(man_path)) if os.path.exists(man_path) else None
    if not man:
        errs.append("manifest.json missing")
    for f in files():
        path = os.path.join(OUT, f)
        if not os.path.exists(path):
            errs.append(f"{f}: missing")
            continue
        g = glbinfo.Glb(path)
        total += g.bytes
        clips = g.clips()
        if f.startswith("kid-"):
            need, budget = KID_CLIPS, BUDGET["kid"]
        elif f.startswith("headmaster"):
            need, budget = HM_CLIPS, BUDGET["headmaster"]
        else:
            need, budget = [], BUDGET["kit"]
        for c in need:
            if c not in clips:
                errs.append(f"{f}: clip {c} missing")
        tris = g.tris()
        if tris > budget:
            errs.append(f"{f}: {tris} tris > budget {budget}")
        for name, mime, (w, h), _n in g.image_sizes():
            if w > MAX_TEX or h > MAX_TEX:
                errs.append(f"{f}: texture {name} {w}x{h} > {MAX_TEX}")
        if f == "school-kit.glb":
            errs += verify_kit(g)
        if f.startswith("kid-"):
            errs += verify_seated(path, f)
        if man:
            e = man["files"].get(f)
            if not e or e["sha256"] != g.sha256 or e["bytes"] != g.bytes:
                errs.append(f"{f}: manifest entry stale")
            elif e["clips"] != clips or e["tris"] != tris:
                errs.append(f"{f}: manifest clips/tris stale")
    env = os.path.join(OUT, "env.hdr")
    if not os.path.exists(env):
        errs.append("env.hdr missing")
    else:
        total += os.path.getsize(env)
        if man and man.get("env", {}).get("bytes") != os.path.getsize(env):
            errs.append("env.hdr: manifest entry stale")
    if total > MAX_TOTAL:
        errs.append(f"shipped total {total} B > {MAX_TOTAL}")
    errs += verify_blender_import()
    return errs, total


def verify_kit(g):
    errs = []
    top = g.top_node_names()
    for n in KIT_NODES + EXTRA_NODES:
        if n not in top:
            errs.append(f"school-kit: top-level node {n} missing")
    mats = g.json.get("materials", [])
    users = g.material_users()
    for parent, kids in CHILDREN.items():
        _, pn = g.node_by_name(parent)
        if pn is None:
            continue
        names = [d.get("name") for d in g.descendants(pn)]
        for k in kids:
            if k not in names:
                errs.append(f"school-kit: {parent} lacks child {k}")
    for plane in PAINTED_PLANES:
        for parent in ("Workstation", "Board", "Corridor_Door"):
            _, pn = g.node_by_name(parent)
            node = next((d for d in g.descendants(pn) if d.get("name") == plane), None) if pn else None
            if node is None:
                continue
            ms = g.materials_of(node)
            if len(ms) != 1:
                errs.append(f"school-kit: {plane} must have exactly one material")
                continue
            if users.get(ms[0], 0) != 1:
                errs.append(f"school-kit: {plane} material is shared")
            if plane == "Screen" and mats[ms[0]].get("name") != "ScreenMat":
                errs.append("school-kit: Screen material must be named ScreenMat")
            errs += check_uv_orientation(g, node, plane, parent)
    return errs


def check_uv_orientation(g, node, plane, parent):
    """(0,0) bottom-left and (1,1) top-right as seen from the plane's front."""
    pos, uvs = g.positions_uvs(node)
    facing_minus_z = plane == "Screen"
    # viewer's right: −X for the screen (faces −Z), +X otherwise
    def seen(p):
        return (-p[0] if facing_minus_z else p[0], p[1])
    lo = min(range(len(uvs)), key=lambda i: uvs[i][0] + uvs[i][1])
    hi = max(range(len(uvs)), key=lambda i: uvs[i][0] + uvs[i][1])
    a, b = seen(pos[lo]), seen(pos[hi])
    if not (a[0] < b[0] and a[1] < b[1]):
        return [f"school-kit: {parent}/{plane} UV orientation wrong (uv0 at {a}, uv1 at {b})"]
    return []


def verify_seated(path, f):
    """Contract seated geometry, measured on the skinned mesh (glTF/three.js
    semantics): hips on the Chair seat (y ≈ 0.42) and both wrists at the
    Workstation keyboard (z ≈ 0.55 − 0.17) in every Sit_* clip."""
    import numpy as np
    from lib.gltfpatch import Doc
    from lib.posekit import Rig
    from lib.dims import KB_Y, KB_Z, SEAT_Y, WS_Z
    errs = []
    d = Doc(path)
    r = Rig(d)
    meshes = [i for i, n in enumerate(d.j["nodes"]) if "skin" in n]
    for clip in ("Sit_Type", "Sit_Idle", "Sit_RaiseHand", "Sit_Slump"):
        P = r.pose_at(clip, 0.0)
        pts = r.skin_points(P, meshes)
        hips = r.pos(P, "Hips")
        seat = pts[(np.abs(pts[:, 0]) < 0.12) & (np.abs(pts[:, 2] - hips[2]) < 0.10)][:, 1].min()
        if abs(seat - SEAT_Y) > 0.03:
            errs.append(f"{f}: {clip} seat contact at y={seat:.3f}, expected ≈{SEAT_Y}")
        if clip == "Sit_Type":
            for side in ("L", "R"):
                w = r.pos(P, f"Wrist.{side}")
                if abs(w[2] - (WS_Z + KB_Z)) > 0.08 or abs(w[1] - KB_Y) > 0.08:
                    errs.append(f"{f}: Sit_Type wrist {side} at {np.round(w, 3)} is off the keyboard")
    return errs


def verify_blender_import():
    """Re-open every .glb with Blender's importer (catches files three.js-ish
    loaders would also choke on: broken skins, accessors, images)."""
    errs = []
    try:
        import bpy
    except ImportError:
        return errs
    for f in files():
        path = os.path.join(OUT, f)
        if not os.path.exists(path):
            continue
        bpy.ops.wm.read_factory_settings(use_empty=True)
        try:
            bpy.ops.import_scene.gltf(filepath=path)
        except Exception as e:  # noqa: BLE001
            errs.append(f"{f}: Blender import failed: {e}")
            continue
        if f.startswith("kid-") or f.startswith("headmaster"):
            if not any(o.type == "ARMATURE" for o in bpy.data.objects):
                errs.append(f"{f}: no armature after import")
    return errs


# ---------------------------------------------------------------- main

def _pin_hash_seed():
    """Blender's glTF exporter iterates sets/dicts keyed by strings when it
    splits primitives, so output order depends on Python's per-process hash
    seed. Re-exec once with PYTHONHASHSEED=0 for byte-identical rebuilds."""
    if os.environ.get("PYTHONHASHSEED") == "0":
        return
    import bpy
    env = dict(os.environ, PYTHONHASHSEED="0")
    argv = sys.argv[sys.argv.index("--"):] if "--" in sys.argv else []
    os.execve(bpy.app.binary_path, [bpy.app.binary_path, "-b", "--factory-startup", "-P",
                                    os.path.abspath(__file__)] + argv, env)


def main():
    _pin_hash_seed()
    a = parse_args()
    if a["verify"]:
        errs, total = verify()
        print(f"[school] verify: {len(files())} files, {total} bytes")
        for e in errs:
            print("[school] FAIL", e)
        if errs:
            sys.exit(1)
        print("[school] verify OK")
        return
    os.makedirs(OUT, exist_ok=True)
    import shutil
    shutil.copyfile(ENV_SRC, os.path.join(OUT, "env.hdr"))
    from lib import kit, headmaster
    from lib import kids
    if "kit" in a["only"]:
        kit.build(os.path.join(OUT, "school-kit.glb"))
        print("[school] built school-kit.glb")
    if "kids" in a["only"]:
        for p in a["kids"]:
            info = kids.build(p, os.path.join(OUT, f"kid-{p}.glb"))
            print(f"[school] built kid-{p}.glb", info)
    if "headmaster" in a["only"]:
        info = headmaster.build(os.path.join(OUT, "headmaster-otto.glb"))
        print("[school] built headmaster-otto.glb", info)
    if all(os.path.exists(os.path.join(OUT, f)) for f in files()):
        write_manifest()
        print("[school] wrote manifest.json")


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception:  # noqa: BLE001
        traceback.print_exc()
        sys.exit(2)
