"""Record visual attachment landmarks in the exported glTF scene coordinate system."""

import json
from pathlib import Path
import sys
import tempfile
import bpy
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from forge.library.gltf import pack_glb, read
from forge.library.import_packs import ROOT, LIB
from forge.library.inspect_model import node_matrix

sys.path.insert(0, str(ROOT / "forge/fit"))
from fitlib import Prop
from props_lib import intake


def palm_width():
    source = next(
        (LIB / "universal-base-characters/files").rglob("*Female_FullBody.gltf")
    )
    doc, _ = read(source)
    positions = {}

    def visit(i, m):
        node = doc["nodes"][i]
        m = m @ node_matrix(node)
        positions[node.get("name")] = m[:3, 3]
        for child in node.get("children", []):
            visit(child, m)

    for i in doc["scenes"][0]["nodes"]:
        visit(i, np.eye(4))
    return float(np.linalg.norm(positions["index_01_r"] - positions["pinky_01_r"]))


def gltf_xyz(p):
    return [round(float(v), 6) for v in (p[0], p[2], -p[1])]


def landmarks(path, cls, width):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(path))
    objects = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    points = np.array(
        [list(o.matrix_world @ v.co) for o in objects for v in o.data.vertices]
    )
    if cls == "shield":
        # This publisher mesh assigns its single back strap to a distinct trim material.
        (obj,) = objects
        vertices = {
            i
            for face in obj.data.polygons
            if obj.material_slots[face.material_index].name == "MI_Trim_Props"
            for i in face.vertices
        }
        if not vertices:
            return {}, dict(passed=False, reason="No identifiable back strap")
        strap = np.array(
            [gltf_xyz(obj.matrix_world @ obj.data.vertices[i].co) for i in vertices]
        )
        y = strap[:, 1]
        span = np.ptp(y)
        ends = [
            strap[y <= y.min() + span * 0.08].mean(axis=0),
            strap[y >= y.max() - span * 0.08].mean(axis=0),
        ]
        return {"strap_line": [p.round(6).tolist() for p in ends]}, dict(
            passed=True, method="two ends of the modelled rear strap, MI_Trim_Props"
        )
    center = points.mean(axis=0)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    prop = Prop(path, "Preview", cls)
    result = intake(prop, width, 1.0)
    return {k: gltf_xyz(center + prop.anchors[k]) for k in ("grip", "tip")}, dict(
        passed=bool(result["pass"]),
        method="forge-fit intake against Universal Female right hand at source scale",
        checks=result["reasons"],
    )


def run():
    catalog = json.loads((ROOT / "forge/library/catalog.json").read_text())
    data = {}
    width = palm_width()
    with tempfile.TemporaryDirectory() as tmp:
        for row in catalog:
            words = set(row["name"].lower().split("_"))
            cls = (
                "shield"
                if row["name"] == "Shield_Wooden"
                else "sword"
                if row["kind"] == "prop" and words & {"sword", "cutlass", "dagger"}
                else None
            )
            if not cls:
                continue
            dest = Path(tmp) / "prop.glb"
            pack_glb(LIB / row["file"], LIB / row["pack"], dest)
            anchors, result = landmarks(dest, cls, width)
            data[row["id"]] = dict(
                prop_class=cls,
                anchor_space="glTF scene metres, Y up",
                anchors=anchors,
                intake=result,
            )
    target = ROOT / "forge/library/prop_anchors.json"
    target.write_text(json.dumps(data, indent=2) + "\n")


if __name__ == "__main__":
    run()
