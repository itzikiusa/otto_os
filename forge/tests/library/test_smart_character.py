"""Packaged animated characters through the same public CLI as Otto."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from forge.library.gltf import read
from forge.library.import_packs import LIB, ROOT


@unittest.skipUnless(
    (LIB / "inventory.json").exists(), "install the Quaternius character packs"
)
class SmartCharacter(unittest.TestCase):
    def test_four_dressed_characters_and_native_tracks(self):
        with tempfile.TemporaryDirectory() as temp:
            for sex, style in (
                ("female", "ranger"),
                ("male", "peasant"),
                ("female", "peasant"),
                ("male", "ranger"),
            ):
                with self.subTest(sex=sex, style=style):
                    dst = Path(temp) / f"{sex}-{style}.glb"
                    requested = (
                        ["ual2-consume", "walk", "ual2-laytoidle"]
                        if sex == "male"
                        else []
                    )
                    cmd = [
                        ROOT / "bin/forge",
                        "library",
                        "--prompt",
                        f"a {sex} {style}",
                        "--kind",
                        "character",
                        "--output",
                        dst,
                    ]
                    if requested:
                        cmd += ["--clips", ",".join(requested)]
                    r = subprocess.run(cmd, check=False, capture_output=True, text=True)
                    self.assertEqual(r.returncode, 0, r.stderr)
                    m = json.loads(Path(str(dst) + ".manifest.json").read_bytes())
                    doc, _ = read(dst)
                    self.assertEqual(len(doc["skins"]), 1)
                    self.assertEqual(len(doc["skins"][0]["joints"]), 65)
                    self.assertTrue(m["parts"])
                    self.assertGreater(m["size_m"][1], 1.7)
                    self.assertLess(m["size_m"][1], 2.1)
                    self.assertGreater(m["details"]["hidden_base_faces_removed"], 0)
                    self.assertEqual(
                        m["id"], f"modular-character-outfits-fantasy/{sex}-{style}"
                    )
                    self.assertEqual(
                        [a["name"] for a in doc["animations"]],
                        requested or ["idle", "walk", "run"],
                    )
                    self.assertTrue(
                        all(len(a["channels"]) == 195 for a in doc["animations"])
                    )
                    self.assertIn("universal-animation-library", m["licences"])
                    self.assertTrue(all("bufferView" in im for im in doc["images"]))
