"""Integration test against the baseline SkinTokens export, when installed."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from forge.glbstats import read_glb

ROOT = Path(__file__).resolve().parents[1]
ASSET = ROOT / "out/archer/warrior"
DIGITS = ("Thumb", "Index", "Middle", "Ring", "Little")


@unittest.skipUnless((ASSET / "rigged.glb").is_file() and (ASSET / "hand-landmarks.json").is_file(),
                     "requires out/archer/warrior/rigged.glb and hand-landmarks.json")
class FingerIntake(unittest.TestCase):
    def run_stage(self, mesh, marks, out):
        return subprocess.run([ROOT / "bin/forge", "rig", "--mesh", mesh, "--hand-intake", marks,
                               "--output", out], text=True, capture_output=True)

    def test_existing_rig_corrected_and_repeat_is_idempotent(self):
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "corrected.glb"
            p = self.run_stage(ASSET / "rigged.glb", ASSET / "hand-landmarks.json", out)
            self.assertEqual(p.returncode, 0, p.stderr)
            manifest = json.loads(Path(str(out) + ".manifest.json").read_text())
            self.assertIsNone(manifest["seed"])
            report = manifest["hand_intake"]
            self.assertEqual(report["Left"]["renamed"], {"Little":"Middle","Middle":"Ring","Ring":"Little"})
            self.assertEqual(report["Right"]["renamed"], {})
            self.assertGreater(report["Left"]["weight_rows_changed"], 0)
            for side in ("Left", "Right"):
                for i,row in enumerate(report[side]["after"]):
                    self.assertEqual(row.index(max(row)), i)
            self.assertLess(report["max_rest_error_per_1000_height"], .001)
            second = Path(folder) / "second.glb"
            p = self.run_stage(out, ASSET / "hand-landmarks.json", second)
            self.assertEqual(p.returncode, 0, p.stderr)
            self.assertEqual(read_glb(out), read_glb(second))

    def test_ambiguous_annotation_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            out, marks = Path(folder) / "out.glb", Path(folder) / "marks.json"
            data = json.loads((ASSET / "hand-landmarks.json").read_text())
            data["Left"]["tips"]["Ring"] = data["Left"]["tips"]["Middle"]
            marks.write_text(json.dumps(data))
            p = self.run_stage(ASSET / "rigged.glb", marks, out)
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("distinct", json.loads(p.stderr.splitlines()[-1])["error"])
            self.assertFalse(out.exists())
