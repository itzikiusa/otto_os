"""Integration checks for the publisher's UAL packs and Forge's public animation CLI."""

import json
import math
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from forge.library.animation import library_clips, samples
from forge.library.gltf import read

LIB = ROOT / "models/library"
CLI = ROOT / "bin/forge-animlib"


@unittest.skipUnless(
    (LIB / "universal-animation-library-2/files").exists()
    and (LIB / "universal-animation-library/files").exists(),
    "Both Quaternius Universal Animation Library Standard packs must be installed",
)
class UniversalAnimationTests(unittest.TestCase):
    def test_editions_and_observed_root_motion(self):
        clips = library_clips(LIB)["clips"]
        ids = {c["id"]: c for c in clips}
        self.assertEqual(len(ids), 172)
        self.assertEqual(len(clips), len(ids))
        self.assertTrue(ids["ual1-walk-loop-rm"]["root_motion"])
        self.assertFalse(ids["ual1-walk-loop"]["root_motion"])
        self.assertFalse(ids["ual1-a-tpose-rm"]["root_motion"])
        self.assertTrue(ids["ual1-walk-loop"]["loopable"])
        self.assertFalse(ids["ual2-laytoidle"]["loopable"])
        self.assertEqual(ids["ual2-climbup-1m"]["category"], "locomotion")
        public = json.loads(subprocess.check_output([CLI, "list"]))
        self.assertTrue(ids.keys() <= {c["id"] for c in public})

    def test_native_channel_keys_survive_self_binding(self):
        rows = library_clips(LIB)["clips"]
        clip = next(c for c in rows if c["id"] == "ual1-walk-loop-rm")
        source = LIB / clip["file"]
        d, b = read(source)
        authored = next(a for a in d["animations"] if a["name"] == clip["action"])
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "bound.glb"
            subprocess.run(
                [
                    CLI,
                    "apply",
                    "--mesh",
                    source,
                    "--clips",
                    clip["id"],
                    "--no-grounding",
                    "--output",
                    out,
                ],
                check=True,
                capture_output=True,
            )
            e, f = read(out)
            self.assertEqual(d["skins"], e["skins"])
            self.assertEqual(d["meshes"], e["meshes"])
            self.assertEqual(d["materials"], e["materials"])
            self.assertEqual(d["nodes"], e["nodes"])
            self.assertEqual([a["name"] for a in e["animations"]], [clip["id"]])
            bound = e["animations"][0]
            self.assertEqual(len(bound["channels"]), len(d["skins"][0]["joints"]) * 3)
            for a, c in zip(authored["channels"], bound["channels"], strict=True):
                self.assertEqual(a["target"], c["target"])
                before, after = (
                    authored["samplers"][a["sampler"]],
                    bound["samplers"][c["sampler"]],
                )
                for field in ("input", "output"):
                    self.assertEqual(
                        samples(d, b, before[field]), samples(e, f, after[field])
                    )
            meta = json.loads(Path(str(out) + ".manifest.json").read_text())
            self.assertFalse(meta["grounding"])
            self.assertIn("Native", meta["modifications"])

    def test_canonical_female_base_keeps_bind_proportions_and_authored_finger_motion(
        self,
    ):
        mesh = LIB / "derived/characters/female/base.glb"
        if not mesh.is_file():
            self.skipTest("Canonical female base has not been exported")
        clip = next(c for c in library_clips(LIB)["clips"] if c["id"] == "ual2-consume")
        source, source_bin = read(LIB / clip["file"])
        target, _ = read(mesh)
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "canonical.glb"
            proc = subprocess.run(
                [
                    CLI,
                    "apply",
                    "--mesh",
                    mesh,
                    "--clips",
                    clip["id"],
                    "--no-grounding",
                    "--output",
                    out,
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            bound, bound_bin = read(out)
        self.assertEqual(target["nodes"], bound["nodes"])
        src_action = next(
            a for a in source["animations"] if a["name"] == clip["action"]
        )

        def track(doc, blob, action, name, prop):
            for c in action["channels"]:
                bone = doc["nodes"][c["target"]["node"]]
                if bone["name"] == name and c["target"]["path"] == prop:
                    return bone[prop], samples(
                        doc, blob, action["samplers"][c["sampler"]]["output"]
                    )
            self.fail(f"Missing {name}.{prop}")

        # Segment offsets are retained. An absolute paste of source translations shortens or stretches the base.
        a, av = track(source, source_bin, src_action, "upperarm_l", "translation")
        b, bv = track(
            bound, bound_bin, bound["animations"][0], "LeftUpperArm", "translation"
        )
        self.assertGreater(math.dist(a, b), 0.05)
        for x, y in zip(av, bv, strict=True):
            self.assertLess(
                math.dist([k - i for k, i in zip(x, a)], [k - i for k, i in zip(y, b)]),
                1e-6,
            )

        # Rotation angles are invariant under a bind-frame change, including a strongly animated index joint.
        a, av = track(source, source_bin, src_action, "index_02_l", "rotation")
        b, bv = track(
            bound,
            bound_bin,
            bound["animations"][0],
            "LeftIndexIntermediate",
            "rotation",
        )

        def angle(bind, q):
            dot = abs(sum(i * j for i, j in zip(bind, q))) / (
                math.hypot(*bind) * math.hypot(*q)
            )
            return 2 * math.acos(min(1.0, dot))

        self.assertGreater(max(angle(a, x) for x in av), 0.5)
        self.assertLess(
            max(abs(angle(a, x) - angle(b, y)) for x, y in zip(av, bv, strict=True)),
            1e-5,
        )

    def test_grounded_native_clip_keeps_the_authored_finger_tracks(self):
        mesh = LIB / "derived/characters/male/base.glb"
        if not mesh.exists():
            self.skipTest("Canonical male base has not been exported")
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "base-grounded.glb"
            proc = subprocess.run(
                [
                    CLI,
                    "apply",
                    "--mesh",
                    mesh,
                    "--clips",
                    "ual2-consume,ual1-walk-loop-rm",
                    "--output",
                    out,
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            doc, binary = read(out)
            self.assertEqual(
                {a["name"] for a in doc["animations"]},
                {"ual2-consume", "ual1-walk-loop-rm"},
            )
            consume = next(a for a in doc["animations"] if a["name"] == "ual2-consume")
            joint = next(
                c
                for c in consume["channels"]
                if doc["nodes"][c["target"]["node"]]["name"] == "LeftIndexIntermediate"
                and c["target"]["path"] == "rotation"
            )
            rotations = samples(
                doc, binary, consume["samplers"][joint["sampler"]]["output"]
            )
            self.assertGreater(max(math.dist(r, rotations[0]) for r in rotations), 0.2)
            receipt = json.loads(Path(str(out) + ".manifest.json").read_text())
            self.assertTrue(receipt["grounding"])
            self.assertIn("native_binding", receipt["tools"])

    def test_real_generated_rig_gets_grounded_body_and_digits(self):
        mesh = ROOT / "out/archer/warrior/rigged-handfit.glb"
        if not mesh.exists():
            self.skipTest("Corrected 52-joint generated warrior is unavailable")
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "warrior.glb"
            subprocess.run(
                [
                    CLI,
                    "apply",
                    "--mesh",
                    mesh,
                    "--clips",
                    "ual2-consume",
                    "--output",
                    out,
                ],
                check=True,
                capture_output=True,
            )
            doc, binary = read(out)
            self.assertEqual([a["name"] for a in doc["animations"]], ["ual2-consume"])
            action = doc["animations"][0]
            moving_digits = set()
            for c in action["channels"]:
                node = doc["nodes"][c["target"]["node"]]["name"]
                if "Index" in node and c["target"]["path"] == "rotation":
                    rotations = samples(
                        doc, binary, action["samplers"][c["sampler"]]["output"]
                    )
                    if any(
                        max(abs(v - w) for v, w in zip(r, rotations[0])) > 0.005
                        for r in rotations
                    ):
                        moving_digits.add(node)
            self.assertGreaterEqual(len(moving_digits), 2)
            receipt = json.loads(Path(str(out) + ".manifest.json").read_text())
            self.assertTrue(receipt["grounding"])
            self.assertEqual(receipt["clips"][0]["licence"], "CC0-1.0")


if __name__ == "__main__":
    unittest.main()
