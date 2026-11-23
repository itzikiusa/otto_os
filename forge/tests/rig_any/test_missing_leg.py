"""A generated six-legged body can have just five independent controls."""
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
sys.path[:0] = [str(HERE.parents[1]), str(HERE)]
import synth
from forge import bodyplan, glbstats
from forge.library.gltf import write


class MissingLeg(unittest.TestCase):
    def test_recover_sixth_leg_from_its_skin(self):
        self.check_recovery(short_leg=False)

    def test_a_short_control_chain_on_another_leg_does_not_block_recovery(self):
        self.check_recovery(short_leg=True)

    def test_fits_measured_legs_when_the_skeleton_only_drives_the_body(self):
        from forge.quality import leg_controls
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "rig.glb"
            output = Path(temp) / "recovered.glb"
            skeleton, tails = synth.legs(6)
            synth.write(source, skeleton, tails, radius=.012, twist=.2)
            doc, data = glbstats.read_glb(source)
            # Strip only leg controls. The thorax takes over their skin weights.
            data = bytearray(data)
            skin = doc["skins"][0]
            torso_count = list(skeleton).index("seg0")
            skin["joints"] = skin["joints"][:torso_count]
            doc["accessors"][skin["inverseBindMatrices"]]["count"] = torso_count
            attr = doc["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"]
            view = doc["bufferViews"][doc["accessors"][attr]["bufferView"]]
            attached = np.frombuffer(data, dtype="<u2", offset=view["byteOffset"], count=view["byteLength"] // 2)
            attached[attached >= torso_count] = 0
            write(doc, data, source)
            original = bodyplan.detect(bodyplan.Rig(source))
            self.assertEqual(original["legs"], 0)
            self.assertIsNone(leg_controls.recover(source, output, 8))
            report = leg_controls.recover(source, output, 6)
            self.assertIsNotNone(report)
            self.assertEqual(report["legs"], 6)
            rig = bodyplan.Rig(output)
            fitted = bodyplan.detect(rig)
            self.assertEqual(original["head"], fitted["head"])
            self.assertGreater(np.dot(original["forward"], fitted["forward"]), .95)
            legs = fitted["leg_chains"]
            self.assertEqual(len(legs), 6)
            for leg in legs:
                self.assertGreaterEqual(len(leg["chain"]), 3)
                self.assertTrue(np.isin(rig.dominant, leg["chain"]).any())
            self.assertIsNone(leg_controls.recover(output, source, 6))

    def check_recovery(self, short_leg):
        with tempfile.TemporaryDirectory() as temp:
            source = Path(temp) / "five-controls.glb"
            delivered = Path(temp) / "six-controls.glb"
            joints, tails = synth.legs(6)
            if short_leg:
                # Two controls drive this intact leg. A different, fully
                # articulated neighbour can supply the missing leg's controls.
                del joints["fe0R"], joints["ti0R"]
                joints["ta0R"] = ("cx0R", joints["ta0R"][1])
            synth.write(source, joints, tails, radius=.012, twist=.2)
            doc, binary = glbstats.read_glb(source)
            binary = bytearray(binary)
            skin = doc["skins"][0]
            lost = skin["joints"][-4:]
            del skin["joints"][-4:]
            doc["accessors"][skin["inverseBindMatrices"]]["count"] -= 4
            parent = next(n for n in doc["nodes"] if lost[0] in n.get("children", []))
            parent["children"].remove(lost[0])
            attrib = doc["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"]
            acc = doc["accessors"][attrib]
            view = doc["bufferViews"][acc["bufferView"]]
            weights = np.frombuffer(binary, dtype="<u2", count=4 * acc["count"],
                                    offset=view.get("byteOffset", 0))
            weights[weights >= len(skin["joints"])] = 0
            write(doc, binary, source)
            before = bodyplan.Rig(source)
            self.assertEqual(bodyplan.detect(before)["legs"], 5)
            from forge.quality import leg_controls
            self.assertIsNotNone(leg_controls.recover(source, delivered, 6))
            after = bodyplan.Rig(delivered)
            self.assertEqual(bodyplan.detect(after)["legs"], 6)
            frame = next(n for n in after.doc["nodes"] if n.get("name") == "FootPlane")
            rotation = np.array(frame["matrix"]).reshape(4, 4).T[:3, :3]
            self.assertTrue(np.allclose(before.skin_rest @ rotation.T, after.skin_rest, atol=1e-5))
            self.assertIsNone(leg_controls.recover(source, delivered, 8))


if __name__ == "__main__":
    unittest.main()
