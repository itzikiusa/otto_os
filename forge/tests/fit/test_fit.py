"""forge-fit tests on a procedural skinned humanoid with finger chains and procedural props, plus the
real knight when it exists. Run: tools/heavy.sh tests/fit/run.sh   (add --mutations to prove each test
fails when its feature is switched off with FORGE_FIT_DISABLE)."""

import json
import os
import signal
import subprocess
import sys
import time
import unittest
from pathlib import Path

TEST = Path(__file__).resolve().parent
ROOT = TEST.parents[1]
CLI = ROOT / "bin/forge-fit"
BLENDER = os.environ.get("BLENDER", "/Applications/Blender.app/Contents/MacOS/Blender")
KNIGHT = ROOT / "out/text_to_asset/character/rigged.glb"
WORK = TEST / "work"
FX = WORK / "fixtures"


def blender(script, *args):
    argv = [BLENDER, "-b", "--factory-startup", "--python-exit-code", "1", "-P", str(script), "--", *map(str, args)]
    if not os.environ.get("FORGE_HEAVY_HELD"):
        argv.insert(0, str(ROOT / "tools/heavy.sh"))
    p = subprocess.run(argv, capture_output=True, text=True)
    if p.returncode:
        raise AssertionError((p.stdout + p.stderr)[-3000:])


def fixtures():
    stamp = FX / "stamp"
    src = (TEST / "make_fixtures.py").stat().st_mtime
    if not stamp.exists() or float(stamp.read_text() or 0) != src:
        blender(TEST / "make_fixtures.py", FX)
        stamp.write_text(str(src))


def fit(*args, check=True, disable=None):
    """Run the CLI; return (exit code, parsed stderr lines, manifest or None)."""
    env = dict(os.environ)
    if disable:
        env["FORGE_FIT_DISABLE"] = ",".join(x for x in (env.get("FORGE_FIT_DISABLE"), disable) if x)
    out = Path(args[args.index("--output") + 1])
    for p in (out, Path(str(out) + ".manifest.json"), out.with_suffix(".png")):
        if p.exists():
            p.unlink()
    proc = subprocess.run([str(CLI), *map(str, args)], capture_output=True, text=True, env=env)
    lines = [json.loads(x) for x in proc.stderr.splitlines() if x.startswith("{")]
    if check and proc.returncode:
        raise AssertionError(proc.stderr[-3000:])
    manifest = Path(str(out) + ".manifest.json")
    return proc.returncode, lines, json.loads(manifest.read_text()) if manifest.exists() else None


def measures(report, clip):
    return report["clips"][clip]["measures"]


class Fit(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        WORK.mkdir(exist_ok=True)
        fixtures()
        cls.out = WORK / "out"
        cls.out.mkdir(exist_ok=True)

    def check(self, glb, name):
        _, _, man = fit("check", "--mesh", glb, "--output", self.out / f"{name}.json")
        return json.loads((self.out / f"{name}.json").read_text())

    def test_hands_intake(self):
        _, _, man = fit("hands", "--character", FX / "humanoid.glb", "--output", self.out / "hands.json")
        rep = man["measurements"]
        self.assertTrue(rep["pass"], rep)
        for side in ("left", "right"):
            for digit, d in rep["hands"][side]["digits"].items():
                self.assertEqual(d["drives"], digit)

    def test_hold_closes_on_the_handle(self):
        own = self.out / "hold_only"
        own.mkdir(exist_ok=True)
        glb = own / "hold.glb"
        _, lines, man = fit("hold", "--character", FX / "humanoid_anim.glb", "--prop", FX / "sword.glb",
                            "--hand", "right", "--output", glb)
        self.assertEqual(lines[-1]["pct"], 100)
        self.assertTrue({"tools", "licences", "timings", "measurements"} <= man.keys())
        self.assertFalse(list(own.glob(".fit-*")), "work directory left behind")
        rep = self.check(glb, "hold_check")
        g = measures(rep, "grip")
        self.assertGreaterEqual(g["prop_prop_right_wrap_deg"]["value"], 200, g["prop_prop_right_wrap_deg"])
        f = g["prop_prop_right_fingers_in_contact"]
        self.assertGreaterEqual(f["value"], 3, f)
        self.assertTrue(f["thumb"], f)
        self.assertLessEqual(g["prop_prop_right_penetration_m"]["value"], 0.003)
        w = measures(rep, "walk")
        self.assertGreaterEqual(w["prop_prop_right_contact_fraction"]["value"], 0.9, w["prop_prop_right_contact_fraction"])
        self.assertLessEqual(w["prop_prop_right_wobble_m"]["value"], 0.002)
        self.assertLessEqual(w["prop_prop_right_penetration_m"]["value"], 0.003)

    def test_check_flags_broken_and_passes_clean(self):
        rep = self.check(FX / "humanoid_anim.glb", "anim_check")
        clean, broken = measures(rep, "clean"), measures(rep, "broken")
        self.assertTrue(rep["clips"]["clean"]["pass"], {k: v for k, v in clean.items() if v.get("pass") is False})
        for key in ("foot_penetration_m", "knee_hyperextension_deg", "limb_body_penetration_m"):
            self.assertFalse(broken[key]["pass"], (key, broken[key]))
        self.assertTrue(Path(self.out / "anim_check.png").exists())

    def test_correct_helps_broken(self):
        glb = self.out / "corrected.glb"
        _, _, man = fit("correct", "--mesh", FX / "humanoid_anim.glb", "--output", glb)
        b = man["measurements"]["clips"]["broken"]
        self.assertLess(b["after"]["knee_hyperextension_deg"], b["before"]["knee_hyperextension_deg"])
        self.assertLess(b["after"]["foot_penetration_m"], b["before"]["foot_penetration_m"])
        self.assertTrue(all(s["kept"] or s["targets"] for s in b["steps"]))

    def test_reach_pull_sword(self):
        glb = self.out / "pull.glb"
        _, _, man = fit("reach", "--character", FX / "humanoid.glb", "--targets", "pull_sword",
                        "--prop", f"sword={FX / 'sword.glb'}", "--output", glb)
        self.assertLess(man["measurements"]["max_reach_shortfall_m"], 0.01)
        m = measures(self.check(glb, "pull_check"), "pull_sword")
        for side in ("right", "left"):
            f = m[f"prop_sword_{side}_fingers_in_contact"]
            self.assertGreaterEqual(f["value"], 3, (side, f))
            self.assertGreaterEqual(m[f"prop_sword_{side}_contact_fraction"]["value"], 0.9, (side, m[f"prop_sword_{side}_contact_fraction"]))
        self.assertLessEqual(m["foot_drift_m"]["value"], 0.01, m["foot_drift_m"])
        self.assertLessEqual(m["ownership_sword_jump_m"]["value"], 0.001)

    def test_reach_shoot_bow(self):
        glb = self.out / "shoot.glb"
        fit("reach", "--character", FX / "humanoid.glb", "--targets", "shoot_bow", "--prop", f"bow={FX / 'bow.glb'}",
            "--prop", f"quiver={FX / 'quiver.glb'}", "--prop", f"arrow={FX / 'arrow.glb'}", "--output", glb)
        m = measures(self.check(glb, "shoot_check"), "shoot_bow")
        for key in ("nock_to_string_m", "shaft_to_rest_m", "release_position_jump_m", "flight_tangent_deg",
                    "flight_residual_m", "draw_fingers_to_string_m", "foot_drift_m", "elbow_plane_flip_deg_per_frame60",
                    "prop_arrow_right_between_keys_m"):
            self.assertTrue(m[key]["pass"], (key, m[key]))
        self.assertAlmostEqual(m["flight_residual_m"]["fitted_gravity_ms2"], 9.81, delta=0.5)

    def test_cancel_leaves_no_blender(self):
        cdir = self.out / "cancel_only"
        cdir.mkdir(exist_ok=True)
        out = cdir / "cancel.glb"
        proc = subprocess.Popen([str(CLI), "hold", "--character", str(FX / "humanoid_anim.glb"), "--prop", str(FX / "sword.glb"),
                                 "--output", str(out)], stderr=subprocess.PIPE, text=True)
        deadline = time.time() + 60
        seen = False
        while time.time() < deadline and not seen:
            seen = subprocess.run(["pgrep", "-f", f"forge/fit/worker.py -- {self.out}"], capture_output=True).returncode == 0
            time.sleep(0.2)
        self.assertTrue(seen, "Blender never started")
        proc.send_signal(signal.SIGTERM)
        _, err = proc.communicate(timeout=30)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("error", json.loads(err.strip().splitlines()[-1]))
        time.sleep(1)
        left = subprocess.run(["pgrep", "-f", f"forge/fit/worker.py -- {self.out}"], capture_output=True)
        self.assertNotEqual(left.returncode, 0, "a Blender worker survived cancellation")
        self.assertFalse(out.exists())
        self.assertFalse(list(cdir.glob(".fit-*")), "cancellation left its work directory")

    def test_template_hands_replace_a_failing_hand(self):
        glb = self.out / "mitten_fixed.glb"
        _, _, man = fit("hands", "--replace", "--character", FX / "humanoid_mitten.glb", "--output", glb)
        rep = man["measurements"]
        self.assertFalse(rep["intake_before"]["right"]["pass"], "the mitten fixture should fail intake")
        self.assertIn("right", rep["replaced"])
        _, _, again = fit("hands", "--character", glb, "--output", self.out / "mitten_fixed_intake.json")
        self.assertTrue(again["measurements"]["hands"]["right"]["pass"], again["measurements"]["hands"]["right"]["failures"])

    def test_prop_intake_and_parametric_stand_in(self):
        _, _, bad = fit("prop-intake", "--class", "sword", "--prop", FX / "sword_two_ended.glb", "--output", self.out / "bad_sword.json")
        self.assertFalse(bad["measurements"]["pass"])
        self.assertTrue(any("middle" in r for r in bad["measurements"]["reasons"]), bad["measurements"]["reasons"])
        _, _, good = fit("prop-intake", "--class", "sword", "--prop", FX / "sword.glb", "--output", self.out / "good_sword.json")
        self.assertTrue(good["measurements"]["pass"], good["measurements"]["reasons"])
        _, _, built = fit("prop", "--class", "sword", "--like", FX / "sword_two_ended.glb", "--output", self.out / "param_sword.glb")
        self.assertTrue(built["measurements"]["parametric_intake"]["pass"], built["measurements"]["parametric_intake"])
        _, _, held = fit("hold", "--character", FX / "humanoid.glb", "--prop", FX / "sword_two_ended.glb", "--output", self.out / "held_param.glb")
        self.assertIn("replaced by a parametric", held["measurements"]["prop_intake"]["used"])

    def test_check_flags_prop_through_body(self):
        glb = self.out / "hold_body.glb"
        # without the carry-pose search, so the broken clip really drives the blade into the chest
        fit("hold", "--character", FX / "humanoid_anim.glb", "--prop", FX / "sword.glb", "--output", glb, disable="carry")
        rep = self.check(glb, "hold_body_check")
        self.assertFalse(measures(rep, "broken")["prop_prop_through_body_m"]["pass"], measures(rep, "broken")["prop_prop_through_body_m"])
        self.assertTrue(measures(rep, "clean")["prop_prop_through_body_m"]["pass"], measures(rep, "clean")["prop_prop_through_body_m"])

    def test_two_hand_hold_measures_both_hands(self):
        glb = self.out / "two_hand.glb"
        fit("hold", "--character", FX / "humanoid.glb", "--prop", FX / "sword.glb", "--hand", "both", "--output", glb)
        g = measures(self.check(glb, "two_hand_check"), "grip")
        for side in ("right", "left"):
            self.assertGreaterEqual(g[f"prop_prop_{side}_fingers_in_contact"]["value"], 3, (side, g.get(f"prop_prop_{side}_fingers_in_contact")))

    @unittest.skipUnless(KNIGHT.exists(), f"real knight not found at {KNIGHT}")
    def test_real_knight(self):
        _, _, man = fit("hands", "--character", KNIGHT, "--output", self.out / "knight_hands.json")
        self.assertFalse(man["measurements"]["pass"], "the knight's mitten hands should fail intake")
        code, lines, _ = fit("hold", "--character", KNIGHT, "--prop", FX / "sword.glb", "--keep-hands", "--output",
                             self.out / "knight_refused.glb", check=False)
        self.assertNotEqual(code, 0)
        self.assertIn("intake", lines[-1]["error"])
        glb = self.out / "knight_hold.glb"
        _, _, man = fit("hold", "--character", KNIGHT, "--prop", FX / "sword.glb", "--output", glb)
        self.assertTrue(man["measurements"]["hands_replaced"], "failing knight hands should be replaced")
        g = measures(self.check(glb, "knight_hold_check"), "grip")
        self.assertLessEqual(g["prop_prop_right_penetration_m"]["value"], 0.003)


MUTATIONS = [  # (FORGE_FIT_DISABLE value, test that must then fail)
    ("close", "test_hold_closes_on_the_handle"),
    ("parent", "test_hold_closes_on_the_handle"),
    ("hinge_sign", "test_check_flags_broken_and_passes_clean"),
    ("limb", "test_check_flags_broken_and_passes_clean"),
    ("feet", "test_check_flags_broken_and_passes_clean"),
    ("arm_ik", "test_reach_pull_sword"),
    ("plant", "test_reach_pull_sword"),
    ("follow", "test_reach_pull_sword"),
    ("nock", "test_reach_shoot_bow"),
    ("power", "test_reach_shoot_bow"),
    ("signal", "test_cancel_leaves_no_blender"),
    ("replace", "test_template_hands_replace_a_failing_hand"),
    ("prop_intake", "test_prop_intake_and_parametric_stand_in"),
    ("prop_body", "test_check_flags_prop_through_body"),
    ("pole_smooth", "test_reach_shoot_bow"),
    ("subframes", "test_reach_shoot_bow"),
]


def mutations():
    failures = []
    for flag, test in MUTATIONS:
        env = {**os.environ, "FORGE_FIT_DISABLE": flag}
        p = subprocess.run([sys.executable, "-B", __file__, f"Fit.{test}"], env=env, capture_output=True, text=True)
        verdict = "FAILED as required" if p.returncode else "STILL PASSES"
        print(f"mutation {flag:<11} -> {test}: {verdict}", flush=True)
        if not p.returncode:
            failures.append(flag)
        subprocess.run(["pkill", "-f", f"forge/fit/worker.py -- {WORK}"], capture_output=True)
    if failures:
        sys.exit(f"mutations not detected: {failures}")


if __name__ == "__main__":
    if "--mutations" in sys.argv:
        WORK.mkdir(exist_ok=True)
        fixtures()
        mutations()
    else:
        unittest.main(verbosity=2)
