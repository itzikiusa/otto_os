"""Exercise the downloaded Quaternius base, the extracted fingers and real garment skin."""
import json
from pathlib import Path
import struct
from collections import Counter
import tempfile
import unittest

from forge.library.characters import export_character, joint_names

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "models/library"


def read_glb(path):
    with path.open("rb") as f:
        magic, version, length, size, tag = struct.unpack("<4sIII4s", f.read(20))
        assert (magic, version, length, tag) == (b"glTF", 2, path.stat().st_size, b"JSON")
        doc = json.loads(f.read(size))
        n, t = struct.unpack("<I4s", f.read(8))
        assert t == b"BIN\0"
        return doc, f.read(n)


def skin_names(doc):
    assert len(doc["skins"]) == 1
    return {doc["nodes"][i]["name"] for i in doc["skins"][0]["joints"]}


def texture_bytes(doc, buf):
    return [buf[v.get("byteOffset", 0):v.get("byteOffset", 0) + v["byteLength"]]
            for im in doc["images"] for v in [doc["bufferViews"][im["bufferView"]]]]


def dominant_counts(doc, buf):
    def read(index):
        a = doc["accessors"][index]
        v = doc["bufferViews"][a["bufferView"]]
        fmt = struct.Struct("<4" + {5121: "B", 5123: "H", 5126: "f"}[a["componentType"]])
        start = v.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = v.get("byteStride", fmt.size)
        return (fmt.unpack_from(buf, start + stride*i) for i in range(a["count"]))
    names = [doc["nodes"][n]["name"] for n in doc["skins"][0]["joints"]]
    result = Counter()
    for mesh in doc["meshes"]:
        for primitive in mesh["primitives"]:
            attrs = primitive["attributes"]
            for joints, weights in zip(read(attrs["JOINTS_0"]), read(attrs["WEIGHTS_0"])):
                result[names[joints[max(range(4), key=weights.__getitem__)]]] += 1
    return result


class CharacterTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        def source(slug, name):
            return next((LIB / slug / "files").rglob(name), None)
        cls.base = source("universal-base-characters", "Superhero_Female_FullBody.gltf")
        cls.garment = source("modular-character-outfits-fantasy", "Female_Ranger_Arms.gltf")
        if not cls.base or not cls.garment:
            raise unittest.SkipTest("Install the Universal Base Characters and Fantasy modular outfit packs")
        cls.scratch = tempfile.TemporaryDirectory()
        cls.addClassCleanup(cls.scratch.cleanup)
        cls.out = Path(cls.scratch.name)
        cls.base_result = export_character(cls.base, cls.out/"base.glb", hand_dir=cls.out)
        cls.dressed = export_character(cls.base, cls.out/"dressed.glb", [cls.garment])

    def test_native_base_preserves_skin_mesh_and_material_bytes(self):
        before = json.loads(self.base.read_bytes())
        after, buf = read_glb(self.out/"base.glb")
        original = (self.base.parent / before["buffers"][0]["uri"]).read_bytes()
        self.assertEqual(buf[:len(original)], original)
        self.assertEqual(before["meshes"], after["meshes"])
        self.assertEqual(before["skins"], after["skins"])
        self.assertEqual(before["materials"], after["materials"])
        self.assertEqual(len(skin_names(after)), 65)
        self.assertTrue(set(joint_names().values()) <= skin_names(after))
        self.assertTrue(all(image.startswith(b"\x89PNG") for image in texture_bytes(after, buf)))
        self.assertEqual(len(self.base_result["sockets"]), 6)
        for key, parent in self.base_result["sockets"].items():
            bone = next(n for n in after["nodes"] if n["name"] == parent)
            self.assertIn(key, [after["nodes"][i]["name"] for i in bone["children"]])

    def test_wrist_extraction_keeps_articulated_skin(self):
        for side in ("left", "right"):
            doc, buf = read_glb(self.out/f"{side}.glb")
            names = skin_names(doc)
            details = self.base_result["templates"][side]
            chains = details["finger_joints"]
            self.assertEqual(len(chains), 5)
            self.assertEqual(set(map(len, chains.values())), {3})
            self.assertTrue(all(set(chain) <= names for chain in chains.values()))
            counts = dominant_counts(doc, buf)
            self.assertTrue(all(counts[j] >= 10 for chain in chains.values() for j in chain))
            self.assertEqual(len(names), 21)  # wrist, fifteen articulating bones, five end markers
            self.assertEqual(details["wrist_joint"], side.title() + "Hand")
            self.assertGreater(details["cut_ring_vertices"], 4)
            self.assertGreater(details["triangles"], 100)
            self.assertLess(details["triangles"], 4000)
            for mesh in doc["meshes"]:
                for p in mesh["primitives"]:
                    self.assertIn("JOINTS_0", p["attributes"])
                    self.assertIn("WEIGHTS_0", p["attributes"])

    def test_outfit_shares_the_skin_and_preserves_materials(self):
        doc, buf = read_glb(self.out/"dressed.glb")
        self.assertTrue(set(joint_names().values()) <= skin_names(doc))
        self.assertTrue(all(image.startswith(b"\x89PNG") for image in texture_bytes(doc, buf)))
        self.assertEqual(len(self.dressed["outfits"]), 1)
        self.assertGreater(self.dressed["hidden_base_faces_removed"], 100)
        meshes = [n for n in doc["nodes"] if "mesh" in n]
        self.assertEqual(len({n.get("skin") for n in meshes}), 1)
        self.assertGreaterEqual(len(meshes), 5)  # body, eyes, brows, sleeves, bracer
        self.assertLess(self.dressed["outfits"][0]["max_bind_joint_distance_m"], 1e-5)
        self.assertTrue(any("Ranger" in m["name"] for m in doc["materials"]))

    def test_incompatible_body_type_fails_before_export(self):
        other = next((LIB/"modular-character-outfits-fantasy/files").rglob("Male_Peasant_Arms.gltf"))
        with self.assertRaisesRegex(ValueError, "body type"):
            export_character(self.base, self.out/"mismatch.glb", [other])
        self.assertFalse((self.out/"mismatch.glb").exists())
