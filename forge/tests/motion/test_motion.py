"""forge-motion on a real character: python3 tests/motion/test_motion.py

Runs the whole stage (Kimodo, retarget, forge-fit check) on the text-to-asset knight. Skipped when the
motion venv or the pinned checkpoints are absent. FORGE_MOTION_STUB_ENCODER=1 runs it without the gated
Llama 3 encoder (unconditioned motion: pipeline checks only, not prompt adherence).
"""

import json
import os
import signal
import struct
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "forge/motion"))
import cli  # noqa: E402

CHARACTER = Path(os.environ.get("FORGE_MOTION_TEST_CHARACTER", ROOT / "out/text_to_asset/character/rigged.glb"))
STUB = os.environ.get("FORGE_MOTION_STUB_ENCODER") == "1"
MISSING = [m for m in cli.missing_models() if not (STUB and m.startswith("meta-llama"))]
REASON = ("motion models absent: " + ", ".join(MISSING) + " (./install.sh --stage motion --with-models)") if MISSING \
    else "motion venv absent" if not cli.PYTHON.exists() else "test character absent" if not CHARACTER.exists() else ""
WORK = Path(tempfile.mkdtemp(prefix="motion-tests-", dir=ROOT / "out"))


def forge_motion(out, *extra, seed=7, seconds=3, prompt="jumps forward and lands"):
    return subprocess.run([ROOT / "bin/forge-motion", "--prompt", prompt, "--character", CHARACTER, "--seconds",
                           str(seconds), "--seed", str(seed), "--output", out, *extra], capture_output=True, text=True)


def glb(path):
    data = path.read_bytes()
    n = struct.unpack_from("<I", data, 12)[0]
    doc = json.loads(data[20:20 + n])
    return doc, data[20 + n + 8:]


def floats(doc, binary, index):
    acc = doc["accessors"][index]
    view = doc["bufferViews"][acc["bufferView"]]
    width = {"SCALAR": 1, "VEC3": 3, "VEC4": 4}[acc["type"]]
    start = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
    flat = struct.unpack_from(f"<{acc['count'] * width}f", binary, start)
    return [flat[i:i + width] for i in range(0, len(flat), width)]


def channels(path):
    doc, binary = glb(path)
    anim = doc["animations"][0]
    out = {}
    for ch in anim["channels"]:
        name = doc["nodes"][ch["target"]["node"]]["name"]
        out[(name, ch["target"]["path"])] = floats(doc, binary, anim["samplers"][ch["sampler"]]["output"])
    return doc, out


@unittest.skipIf(REASON, REASON)
class MotionStage(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.out = WORK / "jump.glb"
        cls.made = forge_motion(cls.out)

    def test_clip_exists_with_requested_length(self):
        self.assertEqual(self.made.returncode, 0, self.made.stderr[-2000:])
        doc, _ = glb(self.out)
        self.assertEqual([a["name"] for a in doc["animations"]], ["jumps_forward_and_lands"])
        m = json.loads(Path(str(self.out) + ".manifest.json").read_text())
        self.assertAlmostEqual(m["clip"]["duration"], 3 - 1 / 30, delta=0.02)
        self.assertEqual(m["seed"], 7)
        self.assertIn("NVIDIA Open Model License", m["licences"]["kimodo_weights"]["licence"])
        self.assertFalse(list(self.out.parent.glob(".motion-*")), "work directory left behind")

    def test_character_moves(self):
        _, ch = channels(self.out)
        hips = next(v for (n, p), v in ch.items() if p == "translation" and "hip" in n.lower())
        span = max(max(c) - min(c) for c in zip(*hips))
        rotating = sum(1 for (n, p), v in ch.items() if p == "rotation"
                       and max(max(c) - min(c) for c in zip(*v)) > 0.2)
        self.assertGreater(span, 0.05, "hips do not move")
        self.assertGreaterEqual(rotating, 10, "fewer than 10 joints rotate noticeably")

    def test_feet_do_not_sink_after_grounding(self):
        m = json.loads(Path(str(self.out) + ".manifest.json").read_text())
        clip = m["fit_check"]["clips"]["jumps_forward_and_lands"]
        self.assertNotIn("foot_penetration_m", clip["failed"], clip)

    def test_same_seed_reproduces(self):
        again = WORK / "jump-again.glb"
        run = forge_motion(again)
        self.assertEqual(run.returncode, 0, run.stderr[-2000:])
        a, b = channels(self.out)[1], channels(again)[1]
        self.assertEqual(a.keys(), b.keys())
        worst = max(abs(x - y) for k in a for ra, rb in zip(a[k], b[k]) for x, y in zip(ra, rb))
        self.assertLess(worst, 1e-4)

    def test_cancellation_leaves_no_process(self):
        out = WORK / "cancel.glb"
        proc = subprocess.Popen([ROOT / "bin/forge-motion", "--prompt", "walks tired and slow", "--character",
                                 CHARACTER, "--seconds", "4", "--output", out], stderr=subprocess.PIPE, text=True)
        time.sleep(8)
        proc.send_signal(signal.SIGTERM)
        err = proc.communicate(timeout=60)[1].strip().splitlines()
        self.assertEqual(proc.returncode, 130)
        self.assertEqual(json.loads(err[-1]), {"error": "Cancelled"})
        time.sleep(1)
        left = subprocess.run(["pgrep", "-f", "forge/motion/generate.py"], capture_output=True, text=True).stdout
        self.assertEqual(left.strip(), "", "a generate.py process survived the cancel")
        self.assertFalse(out.exists())
        self.assertFalse(list(out.parent.glob(".motion-*")))


if __name__ == "__main__":
    try:
        unittest.main(verbosity=2)
    finally:
        import shutil

        shutil.rmtree(WORK, ignore_errors=True)
