"""Drive Otto's uniform argv through the installed stage and read its exported GLBs."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from forge.library.cli import CATALOG
from forge.library.gltf import read
from forge.library.import_packs import LIB, ROOT

FORGE = ROOT / "bin/forge"


@unittest.skipUnless(
    (LIB / "inventory.json").exists(), "install the Quaternius library"
)
class SmartLibrary(unittest.TestCase):
    def call(self, text, target, *extra):
        return subprocess.run(
            [
                FORGE,
                "library",
                "--prompt",
                text,
                "--mesh",
                "{mesh}",
                "--image",
                "",
                "--output",
                target,
                "--clips",
                "{clips}",
                "--poly-budget",
                "unused",
                "--seed",
                "{seed}",
                *extra,
            ],
            check=False,
            capture_output=True,
            text=True,
        )

    def test_scene_pieces(self):
        with tempfile.TemporaryDirectory() as temp:
            for text, item in (
                ("Treasure chest. A chest full of gold coins", "chest-wood"),
                ("a small golden key", "key-gold"),
                ("gold coins", "coin-pile"),
                ("a sword stuck in the ground", "sword-bronze"),
            ):
                with self.subTest(text=text):
                    path = Path(temp) / (item + ".glb")
                    result = self.call(text, path, "--kind", "prop")
                    self.assertEqual(result.returncode, 0, result.stderr)
                    doc, _ = read(path)
                    receipt = json.loads(
                        Path(str(path) + ".manifest.json").read_bytes()
                    )
                    self.assertEqual(receipt["source"], "library")
                    self.assertEqual(receipt["id"], "fantasy-props-megakit/" + item)
                    self.assertEqual(receipt["licence"], "CC0-1.0")
                    self.assertGreater(receipt["score"], 0)
                    self.assertTrue(doc["meshes"])
                    sizes = {
                        "chest-wood": 1.276047,
                        "key-gold": 0.122281,
                        "coin-pile": 0.197456,
                        "sword-bronze": 1.132172,
                    }
                    self.assertAlmostEqual(
                        max(receipt["size_m"]), sizes[item], places=5
                    )
                    if item == "chest-wood":
                        self.assertIn(
                            "Chest_Open", [a["name"] for a in doc["animations"]]
                        )
                        self.assertEqual(
                            receipt["related_pieces"][0]["id"],
                            "fantasy-props-megakit/coin-pile",
                        )
                    if item == "sword-bronze":
                        self.assertIn("grip", receipt["anchors"])

    def test_misses_and_errors_are_distinct(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "unused.glb"
            for prompt in (
                "monster",
                "a monster holding a sword",
                "Captain Barbarossa",
            ):
                result = self.call(prompt, path, "--min-score", "0")
                self.assertNotEqual(result.returncode, 0)
                msg = json.loads(result.stderr.splitlines()[-1])
                self.assertEqual(set(msg), {"error", "best", "score"})
                self.assertEqual(msg["error"], "no library match")
                self.assertEqual(msg["best"], {})
                self.assertEqual(msg["score"], 0)
            result = self.call("a tarnished golden key", path, "--min-score", "1")
            self.assertNotEqual(result.returncode, 0)
            miss = json.loads(result.stderr.splitlines()[-1])
            self.assertEqual(miss["best"]["id"], "fantasy-props-megakit/key-gold")
            self.assertGreater(miss["score"], 0)
            self.assertLess(miss["score"], 1)
            result = self.call("wooden chest", path, "--min-score", "NaN")
            self.assertNotEqual(result.returncode, 0)
            self.assertNotEqual(
                json.loads(result.stderr.splitlines()[-1])["error"], "no library match"
            )
            self.assertFalse(path.exists())

    def test_flagged_packs_excluded(self):
        from forge.library.cli import find

        rows = json.loads(CATALOG.read_bytes())
        self.assertFalse(any(r["pack"] == "pirate-kit" for r in find(rows, "sword")))
        result = subprocess.run(
            [FORGE, "library", "--list-kinds"], check=False, capture_output=True, text=True
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        summary = json.loads(result.stdout)
        self.assertEqual(set(summary["kinds"]), {"character", "prop", "scenery"})
        self.assertNotIn("pirate-kit", result.stdout)
