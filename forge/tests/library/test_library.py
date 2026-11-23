import json
from pathlib import Path
from PIL import Image
import struct
import subprocess
import tempfile
import unittest

from forge.library.gltf import buffers, payload, read
from forge.library.import_packs import ROOT, LIB
from forge.library.cli import CATALOG

CLI = ROOT / "bin/forge-library"


class LibraryTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not (LIB / "inventory.json").is_file():
            raise unittest.SkipTest(
                "Install and index the Quaternius packs to test their real exports"
            )
        cls.rows = json.loads(CATALOG.read_text())
        cls.by_id = {r["id"]: r for r in cls.rows}

    def search(self, *args):
        return json.loads(subprocess.check_output([CLI, "find", *args]))

    def test_search_uses_whole_words_and_kind(self):
        self.assertIn(
            "fantasy-props-megakit/sword-bronze",
            [r["id"] for r in self.search("blade", "--kind", "prop")],
        )
        self.assertTrue(self.search("treasure chest", "--kind", "prop"))
        self.assertNotIn(
            "fantasy-props-megakit/key-gold",
            [r["id"] for r in self.search("coins", "--kind", "prop")],
        )
        for term in ("bow", "quiver", "arrow", "elbow"):
            self.assertEqual(self.search(term, "--kind", "prop"), [])
        self.assertEqual(self.search("wall", "--kind", "animation"), [])

    def test_packaging_preserves_geometry_and_texture_bytes(self):
        key = "fantasy-props-megakit/chest-wood"
        row = self.by_id[key]
        source = LIB / row["file"]
        pack = LIB / row["pack"]
        original, embedded = read(source)
        parts = buffers(original, embedded, source, pack)
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "chest.glb"
            run = subprocess.run(
                [CLI, "get", key, "--output", str(dest)],
                capture_output=True,
                text=True,
                check=True,
            )
            self.assertEqual(json.loads(run.stderr.splitlines()[-1])["pct"], 100)
            doc, blob = read(dest)
            self.assertEqual(original["meshes"], doc["meshes"])
            self.assertEqual(original["materials"], doc["materials"])
            self.assertEqual(original["accessors"], doc["accessors"])
            for before, after in zip(original["bufferViews"], doc["bufferViews"]):
                size = before["byteLength"]
                offset = before.get("byteOffset", 0)
                offset2 = after.get("byteOffset", 0)
                self.assertEqual(
                    parts[before["buffer"]][offset : offset + size],
                    blob[offset2 : offset2 + size],
                )
            for before, after in zip(original["images"], doc["images"]):
                content, _ = payload(before["uri"], source, pack)
                v = doc["bufferViews"][after["bufferView"]]
                self.assertEqual(
                    content, blob[v["byteOffset"] : v["byteOffset"] + v["byteLength"]]
                )
            receipt = json.loads(Path(str(dest) + ".manifest.json").read_text())
            self.assertEqual(receipt["licences"][row["pack"]], "CC0-1.0")
            self.assertEqual(receipt["triangles"], row["triangles"])
            self.assertEqual([c["name"] for c in receipt["clips"]], row["animations"])
            self.assertTrue(receipt["bones"])

    def test_catalog_links_to_real_files_and_thumbnails(self):
        self.assertEqual(len(self.rows), len(self.by_id))
        self.assertEqual(len({r["pack"] for r in self.rows}), 12)
        for row in self.rows:
            self.assertTrue((LIB / row["file"]).is_file(), row["id"])
            thumb = LIB / row["thumbnail"]
            with thumb.open("rb") as f:
                header = f.read(24)
            self.assertEqual(header[:8], b"\x89PNG\r\n\x1a\n", row["id"])
            self.assertEqual(struct.unpack(">II", header[16:]), (192, 192))
            with Image.open(thumb) as im:
                extrema = im.convert("RGB").getextrema()
                self.assertGreater(max(hi - lo for lo, hi in extrema), 4, row["id"])
            self.assertGreater(row["triangles"], 0, row["id"])
            self.assertTrue(any(v > 0 for v in row["size_m"]), row["id"])
        self.assertEqual(
            self.by_id["pirate-kit/weapon-sword-1"]["licence"], "Unverified in archive"
        )

    def test_assemble_relative_output_from_another_directory(self):
        base = "universal-base-characters/superhero-female-fullbody"
        part = "modular-character-outfits-fantasy/female-ranger-arms"
        with tempfile.TemporaryDirectory() as tmp:
            subprocess.run(
                [CLI, "assemble", base, part, "--output", "dressed.glb"],
                cwd=tmp,
                capture_output=True,
                check=True,
            )
            output = Path(tmp) / "dressed.glb"
            doc, _ = read(output, include_binary=False)
            manifest = json.loads(Path(str(output) + ".manifest.json").read_text())
            self.assertEqual(len(doc["skins"]), 1)
            self.assertEqual(manifest["parts"], [part])
            self.assertIn("RightIndexProximal", manifest["bones"])
            self.assertEqual(set(manifest["licences"].values()), {"CC0-1.0"})
            self.assertGreater(manifest["details"]["hidden_base_faces_removed"], 0)

    def test_detected_landmarks_and_empty_class(self):
        sword = self.by_id["fantasy-props-megakit/sword-bronze"]
        self.assertEqual(set(sword["anchors"]), {"grip", "tip"})
        self.assertNotEqual(sword["anchors"]["grip"], sword["anchors"]["tip"])
        self.assertIsInstance(sword["intake"]["passed"], bool)
        shield = self.by_id["fantasy-props-megakit/shield-wooden"]
        self.assertTrue(shield["intake"]["passed"])
        line = shield["anchors"]["strap_line"]
        self.assertEqual(len(line), 2)
        self.assertLess(line[0][1], line[1][1])
        self.assertFalse(
            any("anchors" in r and r["kind"] == "animation" for r in self.rows)
        )

    def test_clip_export_selects_one_named_action(self):
        row = next(r for r in self.rows if r.get("action"))
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "clip.glb"
            subprocess.run(
                [CLI, "get", row["id"], "--output", dest],
                capture_output=True,
                check=True,
            )
            doc, _ = read(dest)
            self.assertEqual([a["name"] for a in doc["animations"]], [row["action"]])
            self.assertEqual(len(doc["skins"][0]["joints"]), 65)
