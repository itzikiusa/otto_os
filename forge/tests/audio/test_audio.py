"""Real Apple Silicon tests for the sound-effect, music and scene stages.

Run once, under the machine-wide lock, after ./install.sh --stage audio --with-models:
    tools/heavy.sh forge/audio/.venv/bin/python tests/audio/test_audio.py
Generations go to out/audio-tests/ and the listening page to out/audio-evidence/.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

import numpy as np
import soundfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).parent))
from forge.audio import analysis  # noqa: E402
import evidence  # noqa: E402

OUT = ROOT / "out" / "audio-tests"
BIN = ROOT / "bin"
SFX = {
    "footsteps": ["--prompt", "footsteps on gravel, slow walking pace", "--seconds", "3", "--seed", "11",
                  "--variations", "2"],
    "sword": ["--prompt", "two steel swords clashing, sharp metallic ring", "--seconds", "2", "--seed", "7"],
    "sword-repeat": ["--prompt", "two steel swords clashing, sharp metallic ring", "--seconds", "2", "--seed", "7"],
    "forest-loop": ["--prompt", "forest at night, crickets chirping, an owl hooting, light wind in leaves",
                    "--seconds", "12", "--loop", "--seed", "3"],
}
MUSIC = {
    "tavern-loop": ["--prompt", "cheerful medieval tavern folk, lute, fiddle and hand drum", "--seconds", "16",
                    "--loop", "--bpm", "110", "--seed", "5"],
    "dungeon": ["--prompt", "dark ambient dungeon music, low strings, slow and ominous", "--seconds", "20",
                "--bpm", "70", "--seed", "9"],
    "dungeon-repeat": ["--prompt", "dark ambient dungeon music, low strings, slow and ominous", "--seconds", "20",
                       "--bpm", "70", "--seed", "9"],
}
SCENE = {
    "seed": 21,
    "ambience": [{"id": "cave", "prompt": "cave interior, slow water drips echoing, distant low rumble",
                  "seconds": 8, "loop": True}],
    "spots": [{"id": "door", "prompt": "heavy wooden door creaking open", "seconds": 1.5, "variations": 1}],
    "music": [{"id": "theme", "prompt": "calm fantasy exploration theme, harp and soft flute", "seconds": 12,
               "loop": True, "bpm": 90}],
}


def wait_for_memory():
    """Hold back while macOS reports critical memory pressure (level 4)."""
    while subprocess.run(["sysctl", "-n", "kern.memorystatus_vm_pressure_level"],
                         capture_output=True, text=True).stdout.strip() == "4":
        time.sleep(30)


def run(tool, args):
    wait_for_memory()
    if shutil.disk_usage(ROOT).free < 60 * 1024**3:
        raise RuntimeError("Stopped: less than 60 GiB free")
    started = time.monotonic()
    proc = subprocess.run([BIN / tool, *map(str, args)], capture_output=True, text=True)
    events = [json.loads(line) for line in proc.stderr.splitlines()]
    if proc.returncode:
        raise AssertionError(f"{tool} failed: {events[-1] if events else proc.stderr}")
    return events, time.monotonic() - started


def read(path):
    y, sr = soundfile.read(str(path), dtype="float32", always_2d=True)
    return y, sr, json.loads(Path(str(path) + ".manifest.json").read_text())


class AudioStage(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        shutil.rmtree(OUT, ignore_errors=True)
        OUT.mkdir(parents=True)
        cls.events, cls.wall = {}, {}
        for name, args in SFX.items():
            cls.events[name], cls.wall[name] = run("forge-sfx", [*args, "--output", OUT / f"{name}.wav"])
        for name, args in MUSIC.items():
            cls.events[name], cls.wall[name] = run("forge-music", [*args, "--output", OUT / f"{name}.wav"])
        cue_sheet = OUT / "scene-cues.json"
        cue_sheet.write_text(json.dumps(SCENE))
        cls.events["scene"], cls.wall["scene"] = run(
            "forge-audio", ["scene", "--cue-sheet", cue_sheet, "--out", OUT / "scene"])

    @classmethod
    def tearDownClass(cls):
        report = evidence.build([OUT, OUT / "scene"])
        (OUT / "wall_clock.json").write_text(json.dumps(cls.wall, indent=2) + "\n")
        print(f"\n{len(report)} files on {evidence.OUT / 'index.html'}", file=sys.stderr)

    def files(self):
        return sorted(OUT.glob("*.wav")) + sorted((OUT / "scene").glob("*.wav"))

    def test_progress_protocol(self):
        for name, events in self.events.items():
            stage = {"scene": "scene"}.get(name, "music" if name in MUSIC else "sfx")
            self.assertTrue(all(e["stage"] == stage and 0 <= e["pct"] <= 100 for e in events), name)
            self.assertEqual(events[-1]["pct"], 100, name)

    def test_duration_level_and_clipping(self):
        for path in self.files():
            y, sr, manifest = read(path)
            m = analysis.describe(y, sr)
            with self.subTest(file=path.name):
                self.assertEqual(sr, manifest["sample_rate"])
                tolerance = 0.05
                if manifest["stage"] == "music" and manifest["loop"]["enabled"]:
                    tolerance = 30 / manifest["bpm"] + 0.01  # beat-aligned loop length, half a beat
                self.assertLess(abs(m["duration_sec"] - manifest["requested_sec"]), tolerance)
                self.assertGreater(m["rms_dbfs"], -45, "silent")
                self.assertEqual(m["clipped_samples"], 0)
                self.assertLessEqual(m["true_peak_dbtp"], -0.85)
                target = manifest["loudness"]["target_lufs"]
                if target is None:  # one-shot: peak-normalised
                    self.assertAlmostEqual(m["true_peak_dbtp"], -1.0, delta=0.3)
                elif abs(m["loudness_lufs"] - target) > 1.0:  # only the ceiling may hold it lower
                    self.assertLess(m["loudness_lufs"], target)
                    self.assertAlmostEqual(m["true_peak_dbtp"], -1.0, delta=0.3)

    def test_loop_seams(self):
        loops = [p for p in self.files() if read(p)[2]["loop"]["enabled"]]
        self.assertGreaterEqual(len(loops), 4)
        for path in loops:
            y, sr, _ = read(path)
            seam = analysis.seam(y, sr)
            with self.subTest(file=path.name, seam=seam):
                for key in ("jump_ratio", "slope_ratio", "spectral_ratio"):
                    self.assertLessEqual(seam[key], analysis.SEAM_LIMIT, key)
        # Positive control: a piece that was not built as a loop must fail when wrapped.
        y, sr, _ = read(OUT / "dungeon.wav")
        self.assertGreater(max(v for k, v in analysis.seam(y, sr).items() if k.endswith("_ratio")), analysis.SEAM_LIMIT)

    def test_prompts_differ_and_seeds_reproduce(self):
        def load(name):
            return read(OUT / f"{name}.wav")[0]

        def distance(a, b):
            return float(np.mean(np.abs(analysis.profile(a, 48000) - analysis.profile(b, 48000))))

        for a, b in (("sword", "footsteps"), ("dungeon", "tavern-loop"), ("forest-loop", "sword")):
            self.assertGreater(distance(load(a), load(b)), 3.0, f"{a} vs {b}")
        for a, b in (("sword", "sword-repeat"), ("dungeon", "dungeon-repeat")):
            x, y = load(a), load(b)
            self.assertEqual(x.shape, y.shape)
            self.assertLessEqual(np.abs(x - y).max(), 4 / 32768, f"{a} is not reproducible")
        x, y = load("footsteps"), load("footsteps-2")
        self.assertLess(np.corrcoef(x[:, 0], y[:, 0])[0, 1], 0.9, "variations are copies")

    def test_music_tempo(self):
        for name in ("tavern-loop", "dungeon"):
            y, sr, manifest = read(OUT / f"{name}.wav")
            estimate = analysis.describe(y, sr)["tempo_bpm"]
            ratios = [estimate / (manifest["bpm"] * k) for k in (0.5, 1, 2)]
            self.assertTrue(any(abs(r - 1) <= 0.04 for r in ratios), f"{name}: {estimate:.1f} vs {manifest['bpm']}")

    def test_manifests(self):
        for path in self.files():
            manifest = read(path)[2]
            self.assertTrue({"tool", "model", "runtime", "prompt", "seed", "duration_sec", "sample_rate",
                             "loudness", "loop", "timings", "memory"} <= set(manifest), path.name)
            self.assertIn(manifest["model"]["licence"], ("Apache-2.0", "MIT"))
            self.assertNotIn("/Users/", json.dumps(manifest["runtime"]))
        scene = json.loads((OUT / "scene" / "manifest.json").read_text())
        self.assertEqual({e["file"] for e in scene["files"]}, {"cave.wav", "door.wav", "theme.wav"})
        self.assertEqual(list((ROOT / "out" / "work").glob("audio-*")), [])

    def test_failure_protocol(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "long.wav"
            proc = subprocess.run([BIN / "forge-sfx", "--prompt", "rain", "--seconds", "45", "--output", out],
                                  capture_output=True, text=True)
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("error", json.loads(proc.stderr.splitlines()[-1]))
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_cancellation(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "cancel.wav"
            wait_for_memory()
            proc = subprocess.Popen([BIN / "forge-sfx", "--prompt", "thunderstorm", "--seconds", "20",
                                     "--output", out], stderr=subprocess.PIPE, text=True)
            try:
                for line in proc.stderr:
                    if "Generating" in json.loads(line).get("message", ""):
                        break
                time.sleep(3)
                proc.terminate()
                rest = proc.stderr.read()
                proc.wait(timeout=60)
                self.assertNotEqual(proc.returncode, 0)
                self.assertEqual(json.loads(rest.splitlines()[-1]), {"error": "Cancelled"})
                with self.assertRaises(ProcessLookupError):
                    os.kill(proc.pid, 0)
                self.assertEqual(subprocess.run(["pgrep", "-f", str(out)], capture_output=True).stdout, b"")
                self.assertEqual(list(Path(tmp).iterdir()), [])
                self.assertEqual(list((ROOT / "out" / "work").glob("audio-cancel-*")), [])
            finally:
                if proc.poll() is None:
                    proc.kill()
                    proc.wait()
                proc.stderr.close()


if __name__ == "__main__":
    unittest.main(verbosity=2)
