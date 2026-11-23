"""Body plans, procedural gaits and their checks, on synthetic skinned bodies (no Blender, no models).

    python3 tests/rig_any/test_rig_any.py

The real bodies (fox, spider, ...) and the humanoid byte-for-byte regression run in tests/rig_any/evidence.py.
"""

import ast
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np

TEST = Path(__file__).resolve().parent
ROOT = TEST.parents[1]
sys.path[:0] = [str(ROOT), str(TEST)]
import synth  # noqa: E402
from forge import bodyplan, gait  # noqa: E402
from forge.fit import plan_check  # noqa: E402

KNIGHT = ROOT / "out/text_to_asset/character/rigged.glb"
BODIES = {"humanoid": synth.humanoid(), "quadruped": synth.quadruped(), "snake": synth.legless("snake"),
          "blob": synth.legless("blob"), **{f"legs{n}": synth.legs(n) for n in (1, 2, 3, 4, 6, 8)}}
WORK = Path(tempfile.mkdtemp(prefix="rig-any-"))


def body(name, twist=0.0):
    path = WORK / f"{name}_{twist}.glb"
    if not path.exists():
        synth.write(path, *BODIES[name], twist=twist)
    return path


class BodyPlans(unittest.TestCase):
    EXPECT = {"quadruped": ("quadruped", 4), "snake": ("legless", 0), "blob": ("legless", 0),
              "legs1": ("legs:1", 1), "legs2": ("legs:2", 2), "legs3": ("legs:3", 3), "legs4": ("quadruped", 4),
              "legs6": ("legs:6", 6), "legs8": ("legs:8", 8)}

    def test_swimming_fish_faces_its_thick_head_across_a_rotated_mesh(self):
        angle = .68
        forward = np.array([np.sin(angle), 0., np.cos(angle)])
        across = np.cross(bodyplan.UP, forward)
        skin = [z * forward + width * np.cos(t) * across + .18 * np.sin(t) * bodyplan.UP
                for z in np.linspace(-1., 1., 61)
                for width in [.025 if z < -.12 else .12]
                for t in np.linspace(0., 2 * np.pi, 32, endpoint=False)]
        result = bodyplan.fish_forward(np.array(skin))
        np.testing.assert_allclose(result, forward, atol=.03)
        mirrored = np.asarray(skin) * [-1., 1., -1.]
        np.testing.assert_allclose(bodyplan.fish_forward(mirrored), -forward, atol=.03)
        self.assertIsNone(bodyplan.fish_forward(np.vstack([skin, -np.asarray(skin)])))

    def test_leg_counts_and_names(self):
        for name, (plan_name, legs) in self.EXPECT.items():
            for twist in (0.0, 0.6):  # rest rotations on every joint must not change the answer
                with self.subTest(body=name, twist=twist):
                    rig = bodyplan.Rig(body(name, twist))
                    self.assertFalse(bodyplan.humanoid(rig))
                    plan = bodyplan.detect(rig)
                    self.assertEqual((plan["body_plan"], plan["legs"]), (plan_name, legs))
                    names = bodyplan.readable_names(rig, plan)
                    self.assertEqual(len(set(names.values())), len(rig.names))
                    self.assertTrue(np.allclose(plan["forward"], [0, 0, 1], atol=0.1), plan["forward"])
                    self.assertFalse(any(v.startswith("bone_") for v in names.values()))

    def test_quadruped_parts(self):
        rig = bodyplan.Rig(body("quadruped"))
        plan = bodyplan.detect(rig)
        names = bodyplan.readable_names(rig, plan)
        by = {rig.names[k]: v for k, v in names.items()}
        self.assertEqual(by["hpawL"], "leg_hind_left_4")
        self.assertEqual(by["sh" + "R"], "leg_front_right_1")
        self.assertEqual(by["t3"], "tail_3")
        self.assertEqual(by["head"], "head")
        self.assertEqual(by["nose"], "head_tip_1")
        self.assertEqual(by["eartipL"], "ear_left_2")
        self.assertEqual(by["pelvis"], "root")
        self.assertEqual(by["neck"], "neck_1")

    def test_spider_and_bird_parts(self):
        rig = bodyplan.Rig(body("legs8"))
        by = {rig.names[k]: v for k, v in bodyplan.readable_names(rig, bodyplan.detect(rig)).items()}
        self.assertEqual(by["ta0L"], "leg_left_1_4")   # front left leg, its foot
        self.assertEqual(by["cx3R"], "leg_right_4_1")  # hind right leg, its first joint
        rig = bodyplan.Rig(body("legs2"))
        by = {rig.names[k]: v for k, v in bodyplan.readable_names(rig, bodyplan.detect(rig)).items()}
        self.assertEqual(by["w2L"], "arm_left_3")
        self.assertEqual(by["toeR"], "leg_right_4")
        self.assertEqual(by["tail2"], "tail_2")

    def test_any_skeleton_gets_a_plan_and_moves(self):  # one joint, or two separate roots: never an error
        for name, joints in (("one", {"root": (None, (0, 0.2, 0))}),
                             ("two_roots", {"a": (None, (0, 0.2, 0)), "b": ("a", (0, 0.4, 0)), "c": (None, (0.3, 0.2, 0))})):
            with self.subTest(body=name):
                path = synth.write(WORK / f"{name}.glb", joints)
                report = gait.animate(path, WORK / f"{name}_anim.glb")
                self.assertEqual(report["body_plan"], "legless")
                self.assertEqual(sorted(report["clips"]), ["idle", "run", "walk"])
                out = WORK / f"{name}_named.glb"
                out.write_bytes(path.read_bytes())
                self.assertEqual(bodyplan.name_bones(out)["renamed"], len(joints))

    def test_legless_shapes(self):
        for name, shape in (("snake", "long"), ("blob", "rigid")):
            self.assertEqual(bodyplan.detect(bodyplan.Rig(body(name)))["legless_shape"], shape)

    def test_humanoid_keeps_the_humanoid_path(self):
        self.assertTrue(bodyplan.humanoid(bodyplan.Rig(body("humanoid"))))
        self.assertTrue(bodyplan.humanoid(bodyplan.Rig(body("humanoid", 0.6))))
        if KNIGHT.exists():
            self.assertTrue(bodyplan.humanoid(bodyplan.Rig(KNIGHT)))

    def test_name_bones_writes_names_and_keeps_the_skin(self):
        src = body("quadruped", 0.6)
        out = WORK / "named.glb"
        out.write_bytes(src.read_bytes())
        report = bodyplan.name_bones(out)
        self.assertEqual(report["body_plan"], "quadruped")
        self.assertIn("leg_front_left", report["chains"])
        before, after = bodyplan.Rig(src), bodyplan.Rig(out)
        self.assertTrue(np.allclose(before.skin_rest, after.skin_rest))
        self.assertIn("leg_hind_right_1", after.names)


class HeadAndWings(unittest.TestCase):
    def test_wings_that_map_without_arms_are_not_a_human(self):
        """The acceptance run's owl: AssetFurnace's mapper matched its wings with no arm or shoulder, and the
        humanoid test then raised KeyError 'LeftShoulder', so forge rig failed and the owl stayed static."""
        from unittest import mock
        sys.path.insert(0, str(bodyplan.AF / "scripts"))
        import kimodo_retarget
        rig = bodyplan.Rig(body("humanoid"))
        parents = {rig.names[k]: rig.names[p] if p >= 0 else None for k, p in enumerate(rig.parent)}
        wings = {name: soma for name, soma in zip(parents, ("Hips", "Spine", "Chest", "Neck", "Head", "LeftHand",
                                                             "RightHand"))}
        with mock.patch.object(kimodo_retarget, "map_skintokens_to_soma", return_value=(wings, np.eye(3))):
            self.assertFalse(bodyplan.humanoid(rig))
        owl = ROOT / "out/speed-evidence/poses/fix/owl_skintokens.glb"
        if owl.is_file():   # the real owl's SkinTokens skeleton: a two-legged body with a head, posable
            real = bodyplan.Rig(owl)
            self.assertFalse(bodyplan.humanoid(real))
            plan = bodyplan.detect(real)
            self.assertEqual(plan["legs"], 2)
            self.assertIsNotNone(plan["head"])

    def test_the_eared_end_is_the_head_even_with_the_tail_held_high(self):
        """A fox whose bushy tail stands higher than its head and whose paws do not point forward: the ears
        decide which end is the head (the real standing fox sat backwards before)."""
        joints, tails = synth.quadruped()
        joints = dict(joints)
        joints["t2"] = ("t1", (0, 0.80, -0.70))
        joints["t3"] = ("t2", (0, 1.00, -0.80))            # the tail tip above the head (0.76)
        for side, sx in (("L", 1), ("R", -1)):
            tails[f"hpaw{side}"] = (sx * 0.12, 0.04, -0.46)   # paws reaching back, not forward
            tails[f"fpaw{side}"] = (sx * 0.12, 0.04, 0.25)
        path = WORK / "high_tail.glb"
        synth.write(path, joints, tails)
        rig = bodyplan.Rig(path)
        plan = bodyplan.detect(rig)
        self.assertEqual(rig.names[plan["head"]], "head")
        self.assertGreater(plan["forward"][2], 0.9)


class Gaits(unittest.TestCase):
    def setUp(self):
        self.rig = bodyplan.Rig(body("quadruped"))
        self.plan = bodyplan.detect(self.rig)
        self.body = gait.Body(self.rig, self.plan)

    def test_phase_offsets(self):
        legs = self.body.legs
        ph = dict(zip([g["name"] for g in legs], gait.phases(legs, run=False)))
        self.assertEqual(ph, {"leg_hind_left": 0.0, "leg_front_left": 0.25, "leg_hind_right": 0.5, "leg_front_right": 0.75})
        ph = dict(zip([g["name"] for g in legs], gait.phases(legs, run=True)))  # trot: diagonals together
        self.assertEqual(ph["leg_front_left"], ph["leg_hind_right"])
        self.assertEqual(ph["leg_front_right"], ph["leg_hind_left"])
        self.assertEqual(abs(ph["leg_front_left"] - ph["leg_front_right"]), 0.5)
        rig = bodyplan.Rig(body("legs6"))
        six = gait.Body(rig, bodyplan.detect(rig)).legs
        ph = dict(zip([g["name"] for g in six], gait.phases(six, run=True)))  # alternating tripods
        tripod = {ph[k] for k in ("leg_left_1", "leg_right_2", "leg_left_3")}
        self.assertEqual(len(tripod), 1)
        self.assertEqual({ph[k] for k in ("leg_right_1", "leg_left_2", "leg_right_3")}, {(tripod.pop() + 0.5) % 1})
        self.assertEqual(gait.phases([{"side": "center"}] * 3, run=False), [0.0, 1 / 3, 2 / 3])

    def test_loops_close_and_feet_plant_within_limits(self):
        for name in BODIES:
            if name == "humanoid":
                continue
            with self.subTest(body=name):
                out = WORK / f"{name}_anim.glb"
                report = gait.animate(body(name, 0.6), out)
                self.assertEqual(sorted(report["clips"]), ["idle", "run", "walk"], report.get("failed"))
                self.assertTrue(report["pass"], json.dumps(report["clips"], default=str)[:2000])
                rig = bodyplan.Rig(out)
                for anim in rig.doc["animations"]:
                    for s in anim["samplers"]:
                        v = bodyplan.accessor(rig.doc, rig.binary, s["output"])
                        self.assertTrue(np.allclose(v[0], v[-1], atol=1e-6), anim["name"])  # first key = last key
                if report["legs"]:
                    for clip in ("walk", "run"):
                        legs = report["clips"][clip]["measures"]["foot_slide_m"]["legs"]
                        for leg, m in legs.items():
                            self.assertGreaterEqual(m["planted_intervals"], 1, (clip, leg))  # every foot is measured
                            self.assertLessEqual(m["slide_m"], plan_check.LIMITS["foot_slide_m"])
                            self.assertGreater(m["swing_m"], 0.3 * report["clips"][clip]["stride_m"], (clip, leg))
                        self.assertGreater(report["clips"][clip]["speed_m_s"], 0)

    def test_a_six_legged_run_opens_with_its_feet_spread_and_lifted(self):
        output = WORK / "opening-stride.glb"
        report = gait.animate(body("legs6"), output, names=("run",))
        delivered = bodyplan.Rig(output)
        plan = bodyplan.detect(delivered)
        _, at = plan_check.clip_sampler(delivered, delivered.doc["animations"][0])
        soles = plan_check.soles(delivered, plan)

        def stride_area(t):
            world = at(t)
            feet = np.array([delivered.skin(world, ids).mean(axis=0) for ids in soles])
            return np.ptp(feet @ plan["forward"]) * np.ptp(feet[:, 1])

        cycle_frames = round(report["clips"]["run"]["cycle_s"] * gait.FPS)
        peak = max(stride_area(i / gait.FPS) for i in range(cycle_frames))
        self.assertGreater(peak, 0)
        self.assertGreaterEqual(stride_area(0), 0.999 * peak)

    def test_a_mixed_rig_takes_a_stride_with_its_outstretched_front_feet(self):
        # Four straight but articulated legs, plus two rigid hind legs which
        # prevent the usual trick of lowering the belly to gain reach.
        joints = {"hub": (None, (0,.32,0)), "neck": ("hub", (0,.32,.31)), "nose": ("neck", (0,.34,.42))}
        tails = {}
        for i, z in enumerate((.20, 0, -.20)):
            for side in (-1, 1):
                parent = "hub"
                shape = [(.06,.30),(.12,.17),(.18,.04)] if i < 2 else [(.14,.30),(.18,.04)]
                for k, (x, y) in enumerate(shape):
                    joint = f"leg_{side}_{i}_{k}"
                    joints[joint] = (parent, (side*x, y, z))
                    parent = joint
                tails[parent] = (side*.185, .035, z+.025)
        source = synth.write(WORK / "mixed.glb", joints, tails, radius=.012)
        report = gait.animate(source, WORK / "mixed-run.glb", names=("run",))
        clip = report["clips"]["run"]
        self.assertEqual(report["legs"], 6)
        self.assertTrue(clip["pass"])
        self.assertGreater(clip["stride_m"], .03)
        self.assertGreater(clip["speed_m_s"], .10)
        delivered = bodyplan.Rig(WORK / "mixed-run.glb")
        plan = bodyplan.detect(delivered)
        _, at = plan_check.clip_sampler(delivered, delivered.doc["animations"][0])
        heights = [delivered.skin(at(0), ids)[:, 1].min() - plan["floor"]
                   for ids in plan_check.soles(delivered, plan)]
        self.assertGreaterEqual(sum(y > .02 for y in heights), 2, heights)
        self.assertGreaterEqual(sum(abs(y) < .01 for y in heights), 2, heights)
        marks = delivered.doc["animations"][0]["extras"]["foot_centres"]
        for point, ids in zip(marks, plan_check.soles(delivered, plan), strict=True):
            np.testing.assert_allclose(point, delivered.skin(at(0), ids).mean(axis=0), atol=.0002)
        # The shot selector also needs the visible length of each lower leg:
        # separated soles alone can still hide an entire shin in projection.
        axes = delivered.doc["animations"][0]["extras"]["lower_leg_axes"]
        skin = delivered.skin(at(0))
        for (low, high), group in zip(axes, plan["leg_chains"], strict=True):
            pts = skin[np.isin(delivered.dominant, group["chain"] + group.get("twins", []))]
            pts = pts[pts[:, 1] <= pts[:, 1].min() + .19 * np.ptp(skin[:, 1])]
            self.assertGreater(high[1] - low[1], .01)
            for centroid, top in ((low, False), (high, True)):
                cut = np.quantile(pts[:, 1], .8 if top else .2)
                band = pts[pts[:, 1] >= cut] if top else pts[pts[:, 1] <= cut]
                np.testing.assert_allclose(centroid, band.mean(axis=0), atol=.0002)




    def test_planted_foot_tracks_the_belt(self):
        clip = gait.Clip(self.body, "walk")
        leg = 0
        sole = plan_check.soles(self.rig, self.plan)[leg]
        ph = clip.phase[leg]
        times = [(ph + f * clip.beta) * clip.T for f in (0.1, 0.5, 0.9)]
        xs = [self.rig.skin(gait.pose(self.body, clip, t)[0], sole).mean(axis=0) @ self.body.fwd for t in times]
        speed = (xs[0] - xs[2]) / (times[2] - times[0])
        self.assertAlmostEqual(speed, clip.speed, delta=0.05 * clip.speed)  # straight back at the stated speed

    def test_check_catches_a_slide_and_still_legs(self):
        out = WORK / "slide.glb"
        report = gait.animate(body("quadruped"), out, names=("walk",))
        self.assertTrue(report["pass"])
        rig = bodyplan.Rig(out)
        plan = bodyplan.detect(rig)
        duration, world_at = plan_check.clip_sampler(rig, rig.doc["animations"][0])

        def frozen_legs(t):  # the whole body glides forward, legs held in the rest pose: a sliding fox
            W = rig.rest_world.copy()
            W[:, :3, 3] += np.array(plan["forward"]) * 0.6 * t
            return W

        m = plan_check.measure(rig, plan, "walk", duration, frozen_legs)
        self.assertFalse(m["measures"]["leg_swing_m"]["pass"])
        self.assertFalse(m["pass"])

        def one_leg_skids(t):  # one planted foot creeps forward (0.23 m/s) while the others hold the belt
            W = world_at(t)
            for j in plan["leg_chains"][0]["chain"]:
                W[j, :3, 3] += np.array(plan["forward"]) * 0.25 * t / duration
            return W

        m = plan_check.measure(rig, plan, "walk", duration, one_leg_skids)
        self.assertGreater(m["measures"]["foot_slide_m"]["value"], plan_check.LIMITS["foot_slide_m"])


def depth(value):
    """Nesting depth: a manifest whose deepest value sits at /a/b/c has depth 3."""
    if isinstance(value, dict):
        return 1 + max((depth(v) for v in value.values()), default=0)
    if isinstance(value, list):
        return 1 + max((depth(v) for v in value), default=0)
    return 0


class Manifests(unittest.TestCase):
    """Otto nests a stage manifest inside its own provenance, and an installed app refuses anything deeper
    than eight levels: rig and animate manifests stay at most five deep, for every body."""

    def test_rig_and_animate_manifests_are_shallow(self):
        for name in BODIES:
            if name == "humanoid":
                continue  # the humanoid path runs the presets (Blender); its manifests did not change
            with self.subTest(body=name):
                named = WORK / f"{name}_depth_rigged.glb"
                named.write_bytes(body(name, 0.6).read_bytes())
                rig_body = bodyplan.name_bones(named)
                rig_manifest = {"body_plan": rig_body.pop("body_plan"), "legs": rig_body.pop("legs"), "body": rig_body,
                                "timings": {"body_plan": {"seconds": 0.01}}}
                self.assertLessEqual(depth(rig_manifest), 5)
                out = WORK / f"{name}_depth_animated.glb"
                p = subprocess.run([str(ROOT / "bin/forge"), "animate", "--mesh", str(named), "--output", str(out)],
                                   capture_output=True, text=True)
                self.assertEqual(p.returncode, 0, p.stderr[-2000:])
                manifest = json.loads(Path(f"{out}.manifest.json").read_text())
                self.assertLessEqual(depth(manifest), 5, json.dumps(manifest)[:1500])
                self.assertEqual(manifest["motion"]["fit_report"], out.name + ".fit.json")
                full = json.loads(Path(f"{out}.fit.json").read_text())  # the per-leg detail is kept beside it
                if full["legs"]:
                    self.assertIn("legs", full["clips"]["walk"]["measures"]["foot_slide_m"])
                    self.assertIn("slide_m", manifest["motion"]["clips"]["walk"])


class Checks(unittest.TestCase):
    def test_limits_match_the_humanoid_measures(self):
        tree = ast.parse((ROOT / "forge/fit/measure.py").read_text())
        limits = next(ast.literal_eval(n.value) for n in tree.body if isinstance(n, ast.Assign)
                      and getattr(n.targets[0], "id", "") == "LIMITS")
        for key in ("foot_penetration_m", "foot_slide_m", "loop_seam_m", "hips_acceleration_ms2"):
            self.assertEqual(plan_check.LIMITS[key], limits[key], key)

    def test_forge_fit_check_measures_any_body(self):
        anim = WORK / "spider_anim.glb"
        gait.animate(body("legs8"), anim)
        out = WORK / "spider_fit.json"
        p = subprocess.run([str(ROOT / "bin/forge-fit"), "check", "--mesh", str(anim), "--output", str(out)],
                           capture_output=True, text=True)
        self.assertEqual(p.returncode, 0, p.stderr[-2000:])
        result = json.loads(out.read_text())
        self.assertEqual(result["body_plan"], "legs:8")
        self.assertEqual(len(result["legs"]), 8)
        self.assertTrue(result["pass"])
        self.assertEqual(len(result["clips"]["walk"]["measures"]["foot_slide_m"]["legs"]), 8)


if __name__ == "__main__":
    unittest.main(verbosity=2)
