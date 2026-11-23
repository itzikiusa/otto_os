"""forge pose on synthetic skinned rigs (no packs, no Blender, no GPU).

    engines/assetfurnace/.venv/bin/python -m unittest tests.pose.test_pose
"""

import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
from unittest.mock import patch

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(ROOT), str(ROOT / "tests" / "rig_any")]
import synth  # noqa: E402
from forge import cli, glbstats, pose  # noqa: E402
from forge.bodyplan import UP, Rig  # noqa: E402

WORK = Path(tempfile.mkdtemp(prefix="pose-"))


def humanoid(hanging=False):
    """A 1.8 m Unity-named humanoid in an A pose, with finger roots (index, middle, little) on each hand; hanging:
    the arms hang straight down with the hands touching the thighs."""
    j = {"Hips": (None, (0, 1.0, 0)), "Spine": ("Hips", (0, 1.15, 0)), "Chest": ("Spine", (0, 1.30, 0)),
         "UpperChest": ("Chest", (0, 1.40, 0)), "Neck": ("UpperChest", (0, 1.52, 0)), "Head": ("Neck", (0, 1.62, 0))}
    tails = {"Head": (0, 1.80, 0.02)}
    for side, sx in (("Left", 1), ("Right", -1)):
        j[f"{side}Shoulder"] = ("UpperChest", (sx * 0.06, 1.45, 0))
        j[f"{side}UpperArm"] = (f"{side}Shoulder", (sx * 0.18, 1.45, 0))
        j[f"{side}LowerArm"] = (f"{side}UpperArm", (sx * (0.17 if hanging else 0.40), 1.18 if hanging else 1.30, 0))
        j[f"{side}Hand"] = (f"{side}LowerArm", (sx * (0.16 if hanging else 0.60), 0.92 if hanging else 1.15, 0))
        for finger, dz in (("Index", 0.03), ("Middle", 0.0), ("Little", -0.03)):
            j[f"{side}{finger}Proximal"] = (f"{side}Hand", (sx * 0.16, 0.86, dz) if hanging else (sx * 0.68, 1.09, dz))
            tails[f"{side}{finger}Proximal"] = (sx * 0.16, 0.80, dz) if hanging else (sx * 0.74, 1.05, dz)
        j[f"{side}UpperLeg"] = ("Hips", (sx * 0.10, 0.95, 0))
        j[f"{side}LowerLeg"] = (f"{side}UpperLeg", (sx * 0.10, 0.52, 0.02))
        j[f"{side}Foot"] = (f"{side}LowerLeg", (sx * 0.10, 0.09, 0))
        j[f"{side}Toes"] = (f"{side}Foot", (sx * 0.10, 0.03, 0.12))
        tails[f"{side}Toes"] = (sx * 0.10, 0.03, 0.18)
    return j, tails


def rig(name, twist=0.0):
    path = WORK / f"{name}_{twist}.glb"
    if not path.exists():
        spec = {"humanoid": humanoid(), "hanging": humanoid(hanging=True), "quadruped": synth.quadruped(), "snake": synth.legless("snake"),
                "six": synth.legs(6), "eight": synth.legs(8)}[name]
        synth.write(path, *spec, twist=twist)
    return path


def make(name, names, twist=0.0, **kw):
    out = WORK / f"{name}_{'-'.join(names)}_{twist}_{len(list(WORK.iterdir()))}.glb"
    report = pose.make(rig(name, twist), out, names, **kw)
    return report, out


def posed_skin(out):
    """The output's first clip at time 0, evaluated independently of forge/pose.py's own state."""
    r = Rig(out)
    sk = pose.Skeleton(r)
    doc = r.doc
    anim = doc["animations"][0]
    joint = {n: k for k, n in enumerate(r.nodes)}
    from forge.bodyplan import accessor
    for ch in anim["channels"]:
        k = joint.get(ch["target"]["node"])
        value = accessor(doc, r.binary, anim["samplers"][ch["sampler"]]["output"])[0]
        if ch["target"]["path"] == "rotation":
            sk.q[k] = value
        elif ch["target"]["path"] == "translation":
            sk.T[k] = value
    return r, sk, r.skin(sk.world())


class Humanoid(unittest.TestCase):
    def test_planting_leans_the_body_over_a_chest_high_grip(self):
        report, out = make("humanoid", ["stand"], hand="left", plant=[0.25, 1.26, 0.36])
        self.assertTrue(report["check"]["pass"], report["check"])
        rig, sk, _ = posed_skin(out)
        spine, head = sk.j("Spine"), sk.j("Head")
        rest = rig.heads[head] - rig.heads[spine]
        points = sk.world()[:, :3, 3]
        leaned = points[head] - points[spine]
        cosine = (rest @ leaned) / (np.linalg.norm(rest) * np.linalg.norm(leaned))
        self.assertGreater(np.degrees(np.arccos(np.clip(cosine, -1, 1))), 15)
        np.testing.assert_allclose(np.asarray(report["contact"]["shaft"])[[0, 2]], [0.25, 0.36], atol=0.035)

    def test_head_bounds_follow_the_posed_head_surface(self):
        report, out = make("humanoid", ["lean", "cross-arms"])
        rig, sk, skin = posed_skin(out)
        head = rig.names.index("Head")
        vertices = skin[rig.dominant == head]
        self.assertGreater(len(vertices), 0)
        bounds = report["head"]["bounds"]
        np.testing.assert_allclose(bounds["min"], vertices.min(axis=0), atol=0.0002)
        np.testing.assert_allclose(bounds["max"], vertices.max(axis=0), atol=0.0002)

    def test_crossed_wrists_clear_a_thick_shirt_front(self):
        r = Rig(rig("humanoid"))
        sk = pose.Skeleton(r)
        garment = np.isin(r.dominant, [sk.j(name) for name in ("Spine", "Chest", "UpperChest")])
        # Bring the garment surface forward while keeping the shoulder and arm bones fixed.
        r.bind[garment, 2] += 0.23
        r.skin_rest = r.skin(r.rest_world)
        body = pose.Body(r)
        pose.cross_arms(sk, body)
        points = sk.world()[:, :3, 3]
        left, right = [points[sk.j(side + "Hand")] for side in ("Left", "Right")]
        self.assertLess(left[0], 0)
        self.assertGreater(right[0], 0)
        self.assertGreater(min(left[2], right[2]), 0.25)

    def test_lean_with_crossed_arms_reaches_across_chest_and_keeps_feet_down(self):
        report, out = make("humanoid", ["lean", "cross-arms"])
        self.assertTrue(report["check"]["pass"], report["check"])
        r, sk, skin = posed_skin(out)
        points = sk.world()[:, :3, 3]
        left, right = [points[sk.j(side + "Hand")] for side in ("Left", "Right")]
        self.assertLess(left[0], right[0])
        self.assertGreater(left[1], points[sk.j("Hips")][1])
        torso = points[sk.j("Head")] - (points[sk.j("LeftFoot")] + points[sk.j("RightFoot")]) / 2
        self.assertLess(torso[2], -0.13)
        self.assertAlmostEqual(skin[:,1].min(), r.skin_rest[:,1].min(), delta=0.01)

    def test_two_armed_biped_can_hold_without_unity_joint_names(self):
        joints, tails = humanoid()
        for side, x in (("Left", 0.08), ("Right", -0.08)):
            joints[side + "Ear"] = ("Head", (x, 1.82, 0.03))
            tails[side + "Ear"] = (x, 2.03, 0.04)
        rename = {name: f"joint_{i}" for i, name in enumerate(joints)}
        mesh, out = WORK / "biped.glb", WORK / "biped-holding.glb"
        synth.write(mesh, {rename[n]: (rename.get(parent), xyz) for n, (parent, xyz) in joints.items()},
                    {rename[n]: xyz for n, xyz in tails.items()})
        report = pose.make(mesh, out, ["hold"], hand="left")
        self.assertTrue(report["check"]["pass"])
        _, sk, _ = posed_skin(out)
        hand = sk.world()[sk.j(rename["LeftHand"])][:3, 3]
        shoulder = sk.world()[sk.j(rename["LeftUpperArm"])][:3, 3]
        self.assertGreater(hand[2] - shoulder[2], 0.2)
        self.assertLess(np.linalg.norm(np.array(report["grip"]["point"]) - hand), 0.15)


    def test_unnamed_two_legged_paws_can_reach_without_named_fingers(self):
        joints, tails = humanoid()
        # This importer connects the arms at the same hub as the neck and
        # ends each paw at its wrist. The ear branches point upwards.
        for side, sign in (("Left", 1), ("Right", -1)):
            joints[side + "Shoulder"] = ("Head", (sign * 0.06, 1.45, 0))
            joints[side + "Ear"] = ("Head", (sign * 0.07, 1.7, 0.02))
            tails[side + "Ear"] = (sign * 0.1, 2.05, 0.02)
        joints = {n: v for n, v in joints.items() if "Proximal" not in n}
        renamed = {n: f"joint_{i}" for i, n in enumerate(joints)}
        src, dst = WORK / "paws.glb", WORK / "playing.glb"
        synth.write(src, {renamed[n]: (renamed.get(parent), pos) for n, (parent, pos) in joints.items()},
                    {renamed[n]: pos for n, pos in tails.items() if n in joints})
        report = pose.make(src, dst, ["stand"], fingerboard=[0.1, 1.2, 0.25], strings=[0.0, 0.91, 0.25])
        self.assertTrue(report["check"]["pass"])
        rig, sk, _ = posed_skin(dst)
        for side in ("Left", "Right"):
            wrist = sk.j(renamed[side + "Hand"])
            self.assertGreater(sk.world()[wrist, 2, 3] - rig.heads[wrist, 2], 0.14)

    def test_sit_rests_on_the_seat_and_the_feet_reach_the_floor(self):
        for twist in (0.0, 0.7):   # local joint frames must not matter
            with self.subTest(twist=twist):
                report, out = make("humanoid", ["sit"], twist, seat=0.40)
                self.assertTrue(report["check"]["pass"], report["check"])
                r, sk, p = posed_skin(out)
                floor = float(r.skin_rest[:, 1].min())
                self.assertAlmostEqual(report["contact"]["seat"][1], floor + 0.40, places=4)
                buttocks = p[np.isin(r.dominant, [sk.j("Hips"), sk.j("LeftUpperLeg"), sk.j("RightUpperLeg")])]
                self.assertAlmostEqual(float(buttocks[:, 1].min()), floor + 0.40, delta=0.01)
                self.assertEqual(report["contact"]["feet"], "touch")
                soles = p[np.isin(r.dominant, [sk.j(n) for n in ("LeftFoot", "RightFoot", "LeftToes", "RightToes")])]
                self.assertLess(abs(float(soles[:, 1].min()) - floor), 0.02)
                self.assertGreaterEqual(float(p[:, 1].min()), floor - 0.005)
                thigh = sk.world()[sk.j("LeftLowerLeg")][:3, 3] - sk.world()[sk.j("LeftUpperLeg")][:3, 3]
                self.assertGreater(float(thigh @ np.array([0, 0, 1.0])), 0.9 * np.linalg.norm(thigh))   # along the seat

    def test_a_high_seat_leaves_the_feet_hanging(self):
        report, _ = make("humanoid", ["sit"], seat=0.80)
        self.assertTrue(report["check"]["pass"], report["check"])
        self.assertEqual(report["contact"]["feet"], "dangle")

    def test_look_at_turns_the_head_to_the_point(self):
        target = [0.6, 1.3, 1.2]   # x left, y up from the floor, z ahead
        report, out = make("humanoid", ["stand"], look=target)
        r, sk, _ = posed_skin(out)
        floor = float(r.skin_rest[:, 1].min())
        head = np.array(report["head"]["point"])
        want = np.array([target[0], floor + target[1], target[2]]) - head
        gaze = np.array(report["head"]["gaze"])
        angle = np.degrees(np.arccos(np.clip(gaze @ want / np.linalg.norm(want), -1, 1)))
        self.assertLess(angle, 10)

    def test_hold_out_puts_the_palm_up_and_reports_where(self):
        for hand in ("right", "left"):
            with self.subTest(hand=hand):
                report, out = make("humanoid", ["stand", "hold-out"], hand=hand)
                palm = report["palm"]
                self.assertEqual(palm["hand"], hand)
                self.assertGreater(float(np.array(palm["normal"]) @ UP), 0.85)   # the skin's normal, facing up
                r, sk, _ = posed_skin(out)
                side = hand.capitalize()
                hand_joint = sk.world()[sk.j(side + "Hand")][:3, 3]
                self.assertLess(float(np.linalg.norm(np.array(palm["point"]) - hand_joint)), 0.15)
                ahead = sk.world()[sk.j(side + "Hand")][:3, 3] - sk.world()[sk.j(side + "UpperArm")][:3, 3]
                self.assertGreater(float(ahead[2]), 0.25)   # held forward

    def test_every_pose_passes_its_checks_and_stays_above_the_floor(self):
        for names in (["kneel"], ["crouch"], ["lie"], ["curl"], ["sit", "hold-out"]):
            with self.subTest(names=names):
                report, out = make("humanoid", names, seat=0.45 if "sit" in names else None)
                self.assertTrue(report["check"]["pass"], report["check"])
                r, _, p = posed_skin(out)
                self.assertGreaterEqual(float(p[:, 1].min()), float(r.skin_rest[:, 1].min()) - 0.005)
        lying, _ = make("humanoid", ["lie"])
        size = np.array(lying["bounds"]["max"]) - np.array(lying["bounds"]["min"])
        self.assertLess(size[1], 0.5)

    def test_sit_needs_a_seat_height(self):
        with self.assertRaises(ValueError):
            make("humanoid", ["sit"])


def _directions(n=26):
    k = np.arange(n) + 0.5
    phi, theta = np.arccos(1 - 2 * k / n), np.pi * (1 + 5 ** 0.5) * k
    return np.c_[np.cos(theta) * np.sin(phi), np.sin(theta) * np.sin(phi), np.cos(phi)]


def assert_on_the_palm(test, out, report, gap=0.02, radius=0.015):
    """A small sphere at palm.point + gap * normal is clear of the skin, the point lies over the palm (between
    the wrist and the finger roots), and it is where the clip's first frame (what a viewer plays) has it."""
    r, sk, p = posed_skin(out)
    tris = pose.triangles(r)
    side = report["palm"]["hand"].capitalize()
    point, normal = np.array(report["palm"]["point"]), np.array(report["palm"]["normal"])
    W = sk.world()
    again, _ = pose.palm_surface(sk, r, W, p, tris, side)
    test.assertLess(float(np.linalg.norm(again - point)), 0.001)   # the manifest is the first frame's palm
    centre = point + gap * normal
    for d in _directions():
        t, _ = pose._ray_hits(centre, d, p, tris)
        nearest = float(t[t > 0].min()) if (t > 0).any() else np.inf
        test.assertGreater(nearest, radius, "skin inside the sphere")
    down, _ = pose._ray_hits(centre, -normal, p, tris)
    test.assertAlmostEqual(float(down[down > 0].min()), gap + 0.001, delta=0.002)   # right above the skin
    along, n = pose._palm_frame(sk, side, W)
    wrist, root = W[sk.j(side + "Hand")][:3, 3], W[sk.j(side + "MiddleProximal")][:3, 3]
    a = float((point - wrist) @ along) / float(np.linalg.norm(root - wrist))
    test.assertTrue(0.0 < a < 1.0, a)
    test.assertGreater(float(normal @ n), 0.7)   # out of the palm, not the back of the hand


class Palm(unittest.TestCase):
    def test_synthetic_palm_point_is_on_the_skin(self):
        for hand in ("right", "left"):
            with self.subTest(hand=hand):
                report, out = make("humanoid", ["stand", "hold-out"], hand=hand)
                assert_on_the_palm(self, out, report)

    def test_real_rigs(self):
        found = 0
        for name, height in (("giant.glb", 8.0), ("human_b.glb", 1.2)):
            path = ROOT / "out/speed-evidence/poses/inputs" / name
            if not path.is_file():
                continue
            found += 1
            with self.subTest(rig=name):
                out = WORK / f"real_{name}"
                report = pose.make(path, out, ["stand", "hold-out"])
                assert_on_the_palm(self, out, report, gap=0.02 * height / 1.2, radius=0.015 * height / 1.2)
        if not found:
            self.skipTest("the real evidence rigs are not on this machine (out/speed-evidence/poses/inputs)")


class Percussion(unittest.TestCase):
    def test_radial_swimmer_strikes_with_arms_that_do_not_stand_on_the_floor(self):
        joints = {"hub": (None, (0., .4, 0.))}
        for i, a in enumerate(np.arange(8) * np.pi / 4):
            parent = "hub"
            for k in range(1, 5):
                name = f"ray_{i}_{k}"
                joints[name] = (parent, (.10 * k * np.cos(a), .4 + .10 * k * np.sin(a), .03 * k))
                parent = name
        source = synth.write(WORK / "radial.glb", joints, radius=.012)
        out = WORK / "radial-strike.glb"
        report = pose.make(source, out, ["stand"], strike=[[.25, .32, .14], [-.25, .32, .14]])
        self.assertTrue(report["check"]["pass"], report["check"])
        self.assertTrue(out.exists())
        self.assertGreater(np.linalg.norm(np.diff(report["contact"]["sticks"], axis=0)), .20)
        # Each tip is in the opening frame of the exported clip.
        _, skeleton, _ = posed_skin(out)
        world = skeleton.world()
        expected = [world[skeleton.j(f"ray_{i}_4"), :3, 3] for i in range(8)]
        tips = np.asarray(report["limb_tips"])
        self.assertEqual(tips.shape, (8, 3))
        for tip in expected:
            self.assertLess(np.linalg.norm(tips - tip, axis=1).min(), .0002)


    def test_swimming_drummer_lifts_its_rear_arms_clear_of_the_body(self):
        from forge import percussion
        joints = {"hub": (None, (0., .5, 0.))}
        for arm, angle in enumerate(np.arange(8) * np.pi / 4):
            parent = "hub"
            for step in range(1, 5):
                name = f"arm_{arm}_{step}"
                joints[name] = (parent, (.1 * step * np.sin(angle), .5, .1 * step * np.cos(angle)))
                parent = name
        rig = Rig(synth.write(WORK / "swimming-drummer.glb", joints, radius=.012))
        body, sk = pose.Body(rig), pose.Skeleton(rig)
        body.plan = {"leg_chains": []}  # A swimmer with no weight-bearing legs.
        before = sk.world()[:, :3, 3].copy()
        poses = percussion.strike(sk, body, [np.array([x, .4, .4]) for x in [-.2, .2]])
        rear, side = sk.j("arm_4_4"), sk.j("arm_2_4")
        for key in poses[:2]:
            sk.q = key
            after = sk.world()[:, :3, 3]
            self.assertGreater(after[rear, 1], before[rear, 1] + .08)
            np.testing.assert_allclose(after[side], before[side], atol=.001)

    def test_two_tips_strike_while_the_other_six_stay_supported(self):
        r = pose.Rig(rig("eight"))
        b = pose.Body(r)
        report, out = make("eight", ["stand"],
                           strike=[[.2*b.height,.28*b.height,.55*b.height],
                                   [-.2*b.height,.28*b.height,.55*b.height]])
        self.assertTrue(report["check"]["pass"], report["check"])
        self.assertEqual(len(report["contact"]["sticks"]), 2)
        self.assertTrue(out.is_file())
        self.assertGreater(np.linalg.norm(np.diff(report["contact"]["sticks"],axis=0)),.1*b.height)


class Hold(unittest.TestCase):
    def test_both_sticks_have_distinct_grips_on_the_recorded_wrists(self):
        report, out = make("humanoid", ["stand", "hold"], hand="left",
                           hold_toward=[0.15, 0.7, 1.0], other_style="raise",
                           other_toward=[-0.15, 0.7, 1.0])
        self.assertTrue(report["check"]["pass"])
        _, sk, _ = posed_skin(out)
        for key, side in (("grip", "Left"), ("other_grip", "Right")):
            g = report[key]
            self.assertEqual(g["hand"], side.lower())
            wrist = sk.world()[sk.j(side + "Hand")][:3, 3]
            self.assertLess(np.linalg.norm(np.array(g["point"]) - wrist), 0.15)
        self.assertGreater(np.linalg.norm(np.array(report["grip"]["point"]) - report["other_grip"]["point"]), 0.2)

    def test_raise_closes_the_fist_and_points_the_handle_at_the_target(self):
        target = [0.0, 0.0, 1.0]   # on the floor a metre ahead (the fire)
        report, out = make("humanoid", ["stand", "hold"], hand="right", hold_toward=target)
        self.assertTrue(report["check"]["pass"], report["check"])
        g = report["grip"]
        self.assertEqual((g["hand"], g["style"]), ("right", "raise"))
        r, sk, p = posed_skin(out)
        floor = float(r.skin_rest[:, 1].min())
        point, axis = np.array(g["point"]), np.array(g["axis"])
        self.assertAlmostEqual(float(np.linalg.norm(axis)), 1.0, places=3)
        hand = sk.world()[sk.j("RightHand")][:3, 3]
        self.assertLess(float(np.linalg.norm(point - hand)), 0.15)
        want = np.array([0.0, floor, 1.0]) - point
        self.assertGreater(float(axis @ want / np.linalg.norm(want)), 0.8)   # aimed at the fire
        self.assertIsNotNone(g["reach"]["toward_m"])
        self.assertLess(g["reach"]["toward_m"], 0.5)
        self.assertAlmostEqual(float(np.linalg.norm(np.array(g["reach"]["tip_1m"]) - point)), 1.0, places=3)
        # the fingers folded towards the palm: their skin is nearer the grip than at rest
        fingers = np.isin(r.dominant, [sk.j("Right" + f + "Proximal") for f in ("Index", "Middle", "Little")])
        rest_hand = r.heads[sk.j("RightHand")]
        rest_gap = np.linalg.norm(r.skin_rest[fingers] - rest_hand, axis=1).mean()
        self.assertLess(float(np.linalg.norm(p[fingers] - hand, axis=1).mean()), 0.97 * rest_gap)

    def test_carry_holds_the_load_down_and_away_from_the_body(self):
        report, out = make("humanoid", ["stand", "hold"], hand="left", hold_style="carry")
        g = report["grip"]
        self.assertEqual((g["hand"], g["style"]), ("left", "carry"))
        self.assertEqual(g["axis"], [0.0, -1.0, 0.0])
        self.assertGreater(float(np.array(g["up"]) @ UP), 0.7)   # thumb on top, the bail across the fist
        r, sk, _ = posed_skin(out)
        hips = sk.world()[sk.j("Hips")][:3, 3]
        away = np.array(g["point"]) - hips
        self.assertGreater(float(np.linalg.norm(away[[0, 2]])), 0.25)   # out where it can be seen
        self.assertLess(g["point"][1], sk.world()[sk.j("LeftUpperArm")][1, 3])
        self.assertTrue(report["check"]["pass"])

    def test_with_a_seat_and_bad_combinations(self):
        report, _ = make("humanoid", ["sit", "hold"], seat=0.45, hold_toward=[0.0, 0.1, 1.0])
        self.assertTrue(report["check"]["pass"], report["check"])
        for names, kw, message in ((["stand", "hold", "hold-out"], {}, "hold or hold-out"),
                                   (["stand", "hold"], {"hold_style": "carry", "hold_toward": [0, 0, 1]}, "hangs"),
                                   (["lie", "hold"], {}, "stand, sit, kneel or crouch"),
                                   (["stand"], {"hold_style": "raise"}, "need the hold pose")):
            with self.subTest(names=names), self.assertRaises(ValueError) as caught:
                make("humanoid", names, **kw)
            self.assertIn(message, str(caught.exception))

    def test_without_finger_joints_there_is_still_a_grip(self):
        joints, tails = humanoid()
        joints = {k: v for k, v in joints.items() if "Proximal" not in k}
        tails = {k: v for k, v in tails.items() if "Proximal" not in k}
        tails.update({"LeftHand": (0.70, 1.10, 0), "RightHand": (-0.70, 1.10, 0)})
        path = WORK / "no_fingers.glb"
        synth.write(path, joints, tails)
        report = pose.make(path, WORK / "no_fingers_hold.glb", ["stand", "hold"])
        self.assertTrue(report["check"]["pass"])
        self.assertIsNotNone(report["grip"])
        r = Rig(WORK / "no_fingers_hold.glb")
        self.assertEqual(len(report["grip"]["point"]), 3)


class FourLegs(unittest.TestCase):
    def test_sit_on_the_haunches_raises_the_head(self):
        report, out = make("quadruped", ["sit"], look=[0.0, 1.6, 1.0])
        self.assertTrue(report["check"]["pass"], report["check"])
        r, sk, p = posed_skin(out)
        head = sk.idx["head"]
        self.assertGreater(sk.world()[head][1, 3], r.heads[head][1] + 0.1)
        self.assertAlmostEqual(float(p[:, 1].min()), float(r.skin_rest[:, 1].min()), places=3)
        self.assertGreater(report["head"]["gaze"][1], 0.3)   # looking up

    def test_sit_keeps_the_front_paws_and_the_rump_on_the_floor_like_a_dog(self):
        report, out = make("quadruped", ["sit"])
        self.assertTrue(report["check"]["pass"], report["check"])
        r, sk, p = posed_skin(out)
        body = pose.Body(r)
        front, hind = pose._legs(body)
        H, floor = body.height, float(r.skin_rest[:, 1].min())
        def lowest(joints):
            return float(p[np.isin(r.dominant, joints)][:, 1].min()) - floor
        self.assertLess(lowest([g["chain"][-1] for g in front]), 0.02 * H)   # front paws planted
        self.assertLess(lowest([g["chain"][0] for g in hind]), 0.08 * H)    # haunches down
        W = sk.world()
        hips = np.mean([W[g["chain"][0]][:3, 3] for g in hind], axis=0)
        chest = np.mean([W[g["chain"][0]][:3, 3] for g in front], axis=0)
        back = chest - hips
        pitch = np.degrees(np.arctan2(back @ UP, np.linalg.norm(back - (back @ UP) * UP)))
        self.assertTrue(10 < pitch < 60, pitch)   # chest up, never tipped upright

    def test_on_a_crate_the_body_rests_on_the_crate_top(self):
        report, out = make("quadruped", ["sit"], seat=0.30)
        r, _, p = posed_skin(out)
        self.assertAlmostEqual(float(p[:, 1].min()), float(r.skin_rest[:, 1].min()) + 0.30, places=3)
        self.assertAlmostEqual(report["contact"]["seat"][1], report["contact"]["floor_y"], places=4)

    def test_lie_and_curl_lower_the_body(self):
        for names in (["lie"], ["curl"]):
            with self.subTest(names=names):
                report, out = make("quadruped", names)
                self.assertTrue(report["check"]["pass"], report["check"])
                r, _, p = posed_skin(out)
                self.assertLess(float(np.ptp(p[:, 1])), 0.75 * float(np.ptp(r.skin_rest[:, 1])))
                size = np.array(report["bounds"]["max"]) - np.array(report["bounds"]["min"])
                self.assertAlmostEqual(float(size[1]), float(np.ptp(p[:, 1])), places=3)   # bounds are the posed size

    def test_curl_is_a_rounder_ball_than_lie(self):
        lying, _ = make("quadruped", ["lie"])
        curled, _ = make("quadruped", ["curl"])

        def footprint(report):
            size = np.array(report["bounds"]["max"]) - np.array(report["bounds"]["min"])
            return max(size[0], size[2]) / min(size[0], size[2])
        self.assertLess(footprint(curled), footprint(lying))

    def test_human_poses_are_refused(self):
        for names in (["kneel"], ["stand", "hold-out"]):
            with self.subTest(names=names), self.assertRaises(pose.Unsupported):
                make("quadruped", names)


class OtherBodies(unittest.TestCase):
    def test_pose_reports_whether_the_original_mesh_surface_can_be_reused(self):
        for kind, moves in [("snake", ["stand"]), ("humanoid", ["lean", "cross-arms"])]:
            with self.subTest(kind=kind):
                report, out = make(kind, moves)
                rig, _, skin = posed_skin(out)
                same = np.allclose(skin, rig.skin_rest, atol=1e-5, rtol=0)
                self.assertEqual(bool(same), kind == "snake")
                self.assertEqual(report["rest_mesh_unchanged"], bool(same))

    def test_other_plans_only_stand_and_look(self):
        with self.assertRaises(pose.Unsupported) as caught:
            make("six", ["sit"], seat=0.2)
        self.assertIn("not supported", str(caught.exception))
        report, _ = make("six", ["stand"])
        self.assertTrue(report["check"]["pass"])
        self.assertEqual(report["pose"]["clip"], "stand")


class Checks(unittest.TestCase):
    def test_a_collapsed_skin_fails(self):
        r = Rig(rig("humanoid"))
        body = pose.Body(r)
        squashed = r.skin_rest * [1, 0.05, 1]
        result = pose.check(body, squashed, pose.triangles(r), body.floor)
        self.assertFalse(result["pass"])
        self.assertTrue(any("collapse" in reason for reason in result["reasons"]))


class Tears(unittest.TestCase):
    """A right forearm generated touching the thigh: carry pulls them apart."""

    def carry(self, r):
        body, sk = pose.Body(r), pose.Skeleton(r)
        pose.hold(sk, body, "right", "carry")
        return body, r.skin(sk.world())

    def test_skin_fusing_the_forearm_to_the_thigh_is_cut_not_torn(self):
        r = Rig(rig("hanging"))
        tris = pose.triangles(r)
        arm, thigh = (np.flatnonzero(r.dominant == r.names.index(n)) for n in ("RightLowerArm", "RightUpperLeg"))
        a = arm[np.argmin(r.skin_rest[arm, 1])]   # the forearm's lowest point and the two thigh points nearest it
        b, c = thigh[np.argsort(np.linalg.norm(r.skin_rest[thigh] - r.skin_rest[a], axis=1))[:2]]
        fused = np.vstack([tris, [[a, b, c], [a, c, b]]])
        body, posed = self.carry(r)
        result = pose.check(body, posed, fused, body.floor)
        self.assertFalse(result["pass"])
        self.assertTrue(any("fuse body parts" in reason for reason in result["reasons"]), result)
        cut = pose.seams(r, fused) & pose.torn(body, posed, fused)
        self.assertEqual(np.flatnonzero(cut).tolist(), [len(tris), len(tris) + 1])   # only the fused pair
        self.assertTrue(pose.check(body, posed, fused[~cut], body.floor)["pass"])

    def test_thigh_skin_weighted_to_the_forearm_is_refused(self):
        r = Rig(rig("hanging"))
        thigh = np.flatnonzero(r.dominant == r.names.index("RightUpperLeg"))
        inner = thigh[r.skin_rest[thigh, 0] < r.skin_rest[thigh, 0].mean()]   # the side the hand touched
        r.vjoints[inner, 1] = r.names.index("RightLowerArm")
        r.vweights[inner] = [0.5, 0.5, 0, 0]
        body, posed = self.carry(r)
        result = pose.check(body, posed, pose.triangles(r), body.floor)
        self.assertFalse(result["pass"])
        self.assertGreater(result["torn_share"], pose.LIMITS["torn_share"])
        self.assertTrue(any("tear into spikes" in reason for reason in result["reasons"]), result)

    def test_a_clean_rig_tears_nothing_in_any_pose(self):
        for names, kw in ((["stand", "hold"], {"hold_style": "carry"}), (["stand", "hold"], {}),
                          (["stand", "hold-out"], {}), (["crouch"], {}), (["curl"], {})):
            with self.subTest(names=names, **kw):
                report, _ = make("hanging", names, **kw)
                self.assertEqual((report["check"]["torn_share"], report["check"]["cut_triangles"]), (0.0, 0))

    def test_the_cut_triangles_leave_the_written_mesh(self):
        r = Rig(rig("humanoid"))
        tris = pose.triangles(r)
        cut = np.zeros(len(tris), bool)
        cut[[0, 5, len(tris) - 1]] = True
        out = WORK / "cut.glb"
        pose.write(r, "stand", pose.breathe(pose.Skeleton(r), pose.Body(r)), out, cut)
        self.assertEqual(pose.triangles(Rig(out)).tolist(), tris[~cut].tolist())


class Stage(unittest.TestCase):
    """forge pose through Otto's uniform argv."""

    def forge(self, *argv):
        with redirect_stderr(StringIO()) as log:
            code = cli.main(list(argv))
        return code, [json.loads(line) for line in log.getvalue().splitlines() if line.startswith("{")]

    def argv(self, out, *extra, mesh=None):
        return ["pose", "--mesh", str(mesh or rig("humanoid")), "--image", "", "--prompt", "", "--output", str(out),
                "--poly-budget", "", "--seed", "", *extra]

    def test_peek_checks_existing_irises_only_when_the_scene_requests_them(self):
        for label, name, prompt, expected in (
            ("shadow", "peek", "Two pale eyes glint in the shadow.", True),
            ("reflection", "peek", "Dark face with two large round eyes, pale irises and dark pupils.", True),
            ("contrasting", "peek", "Charcoal fur and two round eyes that stand out against its dark body.", True),
            ("hidden", "peek", "Two bright eyes hidden behind opaque glasses.", False),
            ("coat", "peek", "A pale coat, dark eyes and dark pupils.", False),
            ("unlit", "peek", "A face in shadow with no luminous eyes or bright markings.", False),
            ("dark", "peek", "Dark eyes without pale irises in shadow.", False),
            ("plain", "peek", "A green animal looks out.", False),
            ("standing", "stand", "Two pale eyes glint in the shadow.", False),
        ):
            out = WORK / f"stage_{label}.glb"
            argv = self.argv(out, "--pose", name, mesh=rig("snake"))
            argv[argv.index("--prompt") + 1] = prompt
            with patch.object(cli.eyes, "apply", wraps=cli.eyes.apply) as finish:
                code, events = self.forge(*argv)
            self.assertEqual(code, 0, events)
            self.assertEqual(finish.called, expected, label)

    def test_writes_the_pose_first_and_keeps_the_other_clips(self):
        first = WORK / "stage_stand.glb"
        code, events = self.forge(*self.argv(first, "--pose", "stand", "--look-at", "0,1.7,2"))
        self.assertEqual(code, 0, events)
        out = WORK / "stage_sit.glb"
        code, events = self.forge(*self.argv(out, "--pose", "sit,hold-out", "--seat-height", "0.42",
                                             "--hand", "left", mesh=first))
        self.assertEqual(code, 0, events)
        doc, _ = glbstats.read_glb(out)
        self.assertEqual([a["name"] for a in doc["animations"]], ["sit", "stand"])
        m = json.loads(Path(f"{out}.manifest.json").read_text())
        self.assertEqual(m["default_clip"], "sit")
        self.assertEqual(m["pose"]["names"], ["sit", "hold-out"])
        self.assertEqual(m["palm"]["hand"], "left")
        self.assertEqual(set(m["contact"]), {"floor_y", "seat", "feet"})
        self.assertEqual(set(m["check"]), {"pass", "floor_penetration_m", "seat_penetration_m", "collapsed_share",
                                           "torn_share", "cut_triangles", "reasons"})
        self.assertIn("pose", m["timings"])

        def depth(v):
            return 1 + max(map(depth, v.values()), default=0) if isinstance(v, dict) else (
                1 + max(map(depth, v), default=0) if isinstance(v, list) else 0)
        self.assertLessEqual(depth({k: m[k] for k in ("pose", "contact", "palm", "head", "bounds", "check")}), 5)

    def test_a_failed_check_writes_no_model(self):
        out = WORK / "stage_fail.glb"
        with patch.dict(pose.LIMITS, {"collapsed_share": -1.0}):
            code, events = self.forge(*self.argv(out, "--pose", "kneel"))
        self.assertEqual(code, 1)
        self.assertIn("pose not written", events[-1]["error"])
        self.assertFalse(out.exists())
        self.assertFalse(json.loads(Path(f"{out}.manifest.json").read_text())["check"]["pass"])

    def test_bad_input_is_a_clean_error(self):
        out = WORK / "stage_bad.glb"
        for extra, message in ((["--pose", "fly"], "unknown pose"), (["--pose", "sit"], "seat-height"),
                               (["--pose", "stand", "--look-at", "1,2"], "--look-at"),
                               (["--pose", "sit,kneel", "--seat-height", "0.4"], "one body pose")):
            with self.subTest(extra=extra):
                code, events = self.forge(*self.argv(out, *extra))
                self.assertEqual(code, 1)
                self.assertIn(message, events[-1]["error"])
        code, events = self.forge(*self.argv(out, "--pose", "sit", "--seat-height", "0.3", mesh=rig("snake")))
        self.assertEqual(code, 1)
        self.assertIn("not supported", events[-1]["error"])

    def test_the_real_cli_binary(self):
        out = WORK / "stage_bin.glb"
        done = subprocess.run([str(ROOT / "bin" / "forge"), *self.argv(out, "--pose", "crouch")], capture_output=True,
                              text=True, timeout=120)
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual(json.loads(done.stderr.splitlines()[-1])["pct"], 100)


if __name__ == "__main__":
    unittest.main()
