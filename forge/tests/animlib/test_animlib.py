"""Run with python3 tests/animlib/test_animlib.py. Each Blender invocation takes a heavy slot."""

import json
import os
import subprocess
import tempfile
import time
import unittest
import zipfile
from pathlib import Path

TEST = Path(__file__).resolve().parent
ROOT = TEST.parents[1]
CLI = ROOT / "bin/forge-animlib"
BLENDER = "/Applications/Blender.app/Contents/MacOS/Blender"


def blender(script, *args):
    cmd = [
        BLENDER,
        "-b",
        "--factory-startup",
        "-t",
        "3",
        "--python-exit-code",
        "1",
        "-P",
        TEST / script,
        "--",
        *args,
    ]
    if os.environ.get("FORGE_HEAVY_HELD") != "1":
        cmd = [ROOT / "tools/heavy.sh", *cmd]
    proc = subprocess.run(cmd, check=False, capture_output=True, text=True)
    if proc.returncode:
        raise AssertionError((proc.stdout + proc.stderr)[-3000:])


def generate_catalog(directory):
    (directory / "fixture-license.txt").write_text("Local synthetic test fixture.\n")
    clips = [
        {
            "id": k,
            "action": k,
            "file": "mixamo.glb",
            "pack": "fixture",
            "licence": "synthetic",
            "category": c,
            "duration": t,
            "root_motion": (k == "stride"),
            "loopable": False,
        }
        for k, c, t in [("stride", "locomotion", 1.0), ("gesture", "emote", 0.75)]
    ]
    cat = {
        "packs": {
            "fixture": {
                "license_file": "fixture-license.txt",
                "licence": "synthetic",
                "attribution": "Test fixture",
                "source": "fixture.py",
            }
        },
        "clips": clips,
    }
    (directory / "catalog.json").write_text(json.dumps(cat))
    return {
        **os.environ,
        "FORGE_ANIMLIB_CATALOG": str(directory / "catalog.json"),
        "FORGE_ANIMLIB_MODELS": str(directory),
    }


class AnimationStage(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        (TEST / "work").mkdir(exist_ok=True)
        cls.tmp = tempfile.TemporaryDirectory(dir=TEST / "work")
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.dir = Path(cls.tmp.name)
        blender("fixture.py", cls.dir)
        cls.env = generate_catalog(cls.dir)

    def test_rest_pose_and_contact(self):
        d = self.dir
        proc = subprocess.run(
            [
                CLI,
                "apply",
                "--mesh",
                d / "unity.glb",
                "--clips",
                "stride,gesture",
                "--output",
                d / "animated.glb",
            ],
            env=self.env,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        events = [json.loads(s) for s in proc.stderr.splitlines()]
        self.assertEqual(events[-1]["pct"], 100)
        receipt = json.loads((d / "animated.glb.manifest.json").read_text())
        self.assertEqual({c["id"] for c in receipt["clips"]}, {"stride", "gesture"})
        self.assertIn("fixture", receipt["licences"])
        blender("inspect_motion.py", d)
        self.assertEqual(len(list(d.glob("animated-*.png"))), 5)

    def test_real_knight_arms_and_shoes(self):
        rig = ROOT / "out/p0/character/rigged.glb"
        if not rig.is_file():
            self.skipTest("P0 SkinTokens knight absent: out/p0/character/rigged.glb")
        output = self.dir / "real-knight.glb"
        raw = self.dir / "ungrounded.glb"
        for dest, flags in ((output, []), (raw, ["--no-grounding"])):
            proc = subprocess.run(
                [
                    CLI,
                    "apply",
                    "--mesh",
                    rig,
                    "--clips",
                    "walk,1h-melee-attack-chop,100style-angry-fw",
                    "--output",
                    dest,
                    *flags,
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(proc.returncode, 0, proc.stderr)
            receipt = json.loads(Path(str(dest) + ".manifest.json").read_text())
            self.assertEqual(receipt["grounding"], dest == output)
        report = self.dir / "real-knight.json"
        blender(
            "real_rig.py",
            ROOT / "models/animlib/kaykit-adventurers/Knight.glb",
            output,
            report,
            raw,
        )
        metrics = json.loads(report.read_text())
        for clip, rays in metrics["rays"].items():
            with self.subTest(clip=clip, check="world limb directions"):
                self.assertLess(max(rays["max_world_angle_deg"].values()), 1.0, rays)
                for source, target in rays["median_elevations"].values():
                    self.assertAlmostEqual(source, target, delta=3.0)
            with self.subTest(clip=clip, check="switch from 100STYLE"):
                self.assertAlmostEqual(
                    rays["median_elevations"]["Right"][0],
                    metrics["soles"][clip]["switched_elevation"],
                    delta=3.0,
                )
            with self.subTest(clip=clip, check="lowest skinned shoe at 240 Hz"):
                self.assertGreaterEqual(metrics["soles"][clip]["minimum"], 0.0)
                self.assertLess(metrics["off"][clip]["minimum"], -0.005)
                self.assertLess(metrics["soles"][clip]["max_step_m_60hz"], 0.025)
                self.assertLess(
                    metrics["soles"][clip]["max_lateral_step_m_60hz"], 0.025
                )
        walk = metrics["soles"]["walk"]
        self.assertLess(walk["slide"], 0.065)
        self.assertLess(walk["slide"], 0.75 * metrics["off"]["walk"]["slide"])
        self.assertLess(walk["cycle_correction_seam_m"], 0.001)

    def test_publisher_assets(self):
        d = self.dir
        entries = json.loads(
            subprocess.check_output([CLI, "list", "--category", "locomotion"])
        )
        self.assertIn("walk", {c["id"] for c in entries})
        self.assertTrue(all(c["category"] == "locomotion" for c in entries))
        result = d / "published.glb"
        run = subprocess.run(
            [
                CLI,
                "apply",
                "--mesh",
                d / "unity.glb",
                "--clips",
                "walk,run,idle",
                "--output",
                result,
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        receipt = json.loads(Path(str(result) + ".manifest.json").read_text())
        self.assertEqual({c["id"] for c in receipt["clips"]}, {"walk", "run", "idle"})
        expected = {"walk": 32 / 30, "run": 0.8, "idle": 32 / 30}
        for clip in receipt["clips"]:
            self.assertAlmostEqual(clip["duration"], expected[clip["id"]], places=4)
            self.assertEqual(clip["licence"], "CC0-1.0")
        # A Mixamo target may already have actions. The exported set is precisely the requested IDs.
        mix_result = d / "published-mixamo.glb"
        run = subprocess.run(
            [
                CLI,
                "apply",
                "--mesh",
                d / "mixamo.glb",
                "--clips",
                "walk",
                "--output",
                mix_result,
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertEqual(
            [
                c["id"]
                for c in json.loads(
                    Path(str(mix_result) + ".manifest.json").read_text()
                )["clips"]
            ],
            ["walk"],
        )

    def test_catalog_sources_and_licences(self):
        catalog = json.loads((ROOT / "forge/animlib/catalog.json").read_text())
        models = ROOT / "models/animlib"
        rows = catalog["clips"]
        self.assertEqual(len({r["id"] for r in rows}), len(rows))
        with zipfile.ZipFile(models / "100style/100STYLE.zip") as archive:
            members = set(archive.namelist())
        for pack in catalog["packs"].values():
            self.assertGreater((models / pack["license_file"]).stat().st_size, 0)
        for row in rows:
            with self.subTest(clip=row["id"]):
                self.assertTrue((models / row["file"]).is_file())
                self.assertEqual(
                    row["licence"], catalog["packs"][row["pack"]]["licence"]
                )
                self.assertIsInstance(row["loopable"], bool)
                self.assertIsInstance(row["root_motion"], bool)
                self.assertGreater(row["duration"], 0)
                if row["pack"] == "100style":
                    self.assertIn(row["member"], members)
                    self.assertLess(row["frame_start"], row["frame_end"])
                    self.assertFalse(row["loopable"])

    def test_knight_world_landmarks(self):
        # Self-retarget checks exact authored FK. The real-rig test separately measures the ground pass.
        source = ROOT / "models/animlib/kaykit-adventurers/Knight.glb"
        output = self.dir / "knight.glb"
        proc = subprocess.run(
            [
                CLI,
                "apply",
                "--no-grounding",
                "--mesh",
                source,
                "--clips",
                "walk,run,idle",
                "--output",
                output,
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        blender("inspect_motion.py", self.dir, "--compare-knight", source, output)

    def test_published_capture(self):
        clip = "100style-twofootjump-id"
        output = self.dir / "capture.glb"
        run = subprocess.run(
            [
                CLI,
                "apply",
                "--mesh",
                self.dir / "unity.glb",
                "--clips",
                clip,
                "--output",
                output,
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(run.returncode, 0, run.stderr)
        receipt = json.loads(Path(str(output) + ".manifest.json").read_text())
        (motion,) = receipt["clips"]
        self.assertEqual(motion["id"], clip)
        self.assertAlmostEqual(motion["duration"], 6.33346, places=3)
        self.assertEqual(motion["licence"], "CC-BY-4.0")
        self.assertIn("Ian Mason", receipt["licences"]["100style"]["attribution"])
        self.assertIn("retargeted", receipt["modifications"])

    def test_error_and_cancellation(self):
        d = self.dir
        out = d / "cancelled.glb"
        base = [CLI, "apply", "--mesh", d / "unity.glb", "--output", out, "--clips"]
        bad = subprocess.run(
            base + ["missing"],
            env=self.env,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn("missing", json.loads(bad.stderr.splitlines()[-1])["error"])
        fake = d / "fake-blender"
        pidfile = d / "child-pid"
        fake.write_text('#!/bin/sh\necho $$ > "$PIDFILE"\nexec /bin/sleep 180\n')
        fake.chmod(0o755)
        proc = subprocess.Popen(
            base + ["stride"],
            env={**self.env, "BLENDER": str(fake), "PIDFILE": str(pidfile)},
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            for _ in range(3000):
                if pidfile.exists() or proc.poll() is not None:
                    break
                time.sleep(0.1)
            self.assertTrue(pidfile.exists(), "Blender child never started")
            child = int(pidfile.read_text())
            proc.terminate()
            _, err = proc.communicate(timeout=10)
            self.assertNotEqual(proc.returncode, 0)
            self.assertEqual(json.loads(err.splitlines()[-1]), {"error": "Cancelled"})
            for _ in range(20):
                try:
                    os.kill(child, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.05)
            with self.assertRaises(ProcessLookupError):
                os.kill(child, 0)
            self.assertFalse(out.exists())
            self.assertFalse(Path(str(out) + ".manifest.json").exists())
        finally:
            if proc.poll() is None:
                proc.terminate()
                proc.wait(timeout=10)


if __name__ == "__main__":
    unittest.main()
