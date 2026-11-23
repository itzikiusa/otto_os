"""generate --concept-only and --skip-gameready: the two opt-in shortcuts for a fast first look.

    engines/assetfurnace/.venv/bin/python -m unittest tests.test_speed_flags
"""

import base64
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import textwrap
import time
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
import tempfile
import unittest
from contextlib import redirect_stderr
from io import StringIO
from pathlib import Path
from unittest.mock import patch

from forge import cli

POINTS = [(0.0, 0.0, 0.0), (0.3, 0.0, 0.1), (0.0, 0.9, 0.0)]


def write_triangle(path):
    """A one-triangle mesh 0.9 m tall, enough for sizing and scaling."""
    binary = b"".join(struct.pack("<3f", *p) for p in POINTS)
    cli.write_glb({"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}],
                   "nodes": [{"name": "raw", "mesh": 0}],
                   "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
                   "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                                  "min": [0.0, 0.0, 0.0], "max": [0.3, 0.9, 0.1]}],
                   "bufferViews": [{"buffer": 0, "byteLength": len(binary)}],
                   "buffers": [{"byteLength": len(binary)}]}, binary, path)


FAKE_SD_SERVER = textwrap.dedent('''\
    import base64, json, os, sys
    from http.server import BaseHTTPRequestHandler, HTTPServer
    out = sys.argv[sys.argv.index("--record") + 1]
    open(out, "w").write(json.dumps({"pid": os.getpid(), "argv": sys.argv[1:]}))
    class H(BaseHTTPRequestHandler):
        def reply(self, doc):
            body = json.dumps(doc).encode()
            self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers()
            self.wfile.write(body)
        def do_GET(self):
            self.reply({"data": []})
        def do_POST(self):
            self.rfile.read(int(self.headers["Content-Length"]))
            self.reply({"images": [base64.b64encode(b"warm picture").decode()]})
        def log_message(self, *a):
            pass
    HTTPServer(("127.0.0.1", int(sys.argv[sys.argv.index("--listen-port") + 1])), H).serve_forever()
''')


class SpeedFlags(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.stages = []
        self.argvs = {}

    def model_step(self, job, stage, args, **kwargs):
        self.stages.append(stage)
        self.argvs[stage] = [str(a) for a in args]
        if stage == "concept":
            Path(args[args.index("-o") + 1]).write_bytes(b"concept picture")
        elif stage == "check_base":
            shutil.copyfile(args[2], args[3])
        elif stage == "matte":
            Path(args[-1]).touch()
            Path(args[-1]).with_suffix(".json").write_text('{"method":"input_alpha"}')
        elif stage == "generate":
            raw = job.work / "raw.glb"
            write_triangle(raw)
            raw.with_suffix(".svviews").mkdir()
            (raw.with_suffix(".svviews") / "transforms.json").write_text('{"frames": []}')
            (raw.with_suffix(".svviews") / "input.png").write_bytes(b"front view")
            raw.with_suffix(".provenance.json").write_text(json.dumps({
                "parameters": {"steps": 8}, "input": {"sha256": "front"}, "output": {"sha256": "raw"}}))
            (job.work / "generate.log").write_text("[trellis] generating model with seed 7")
        elif stage == "gameready":
            self.consumed = cli.glbstats.read_glb(Path(args[1]))
            shutil.copyfile(args[1], args[3])
            self.assertEqual(args[args.index("--detail") + 1], "head,hands")
            pack = Path(args[args.index("--pack") + 1])
            pack.mkdir()
            cli.write_glb({"nodes": [{"name": "UCX_gameready_00", "mesh": 0}, {"name": "gameready", "mesh": 1}],
                           "meshes": [{"name": "hull"}, {"name": "display"}]}, b"", pack / "gameready_LOD0.glb")
        elif stage == "upscale_views":
            self.assertEqual((Path(args[-2]) / "input.png").read_bytes(), b"front view")
            args[-1].mkdir()
            (args[-1] / "upscale.json").write_text("{}")
        elif stage == "photo_paint":
            shutil.copyfile(args[2], args[4])
            (job.work / "photo_paint.log").write_text("[photo-paint] 1 view(s)")
        elif stage == "apply_colour":
            shutil.copyfile(args[-3], args[-1])
        else:
            self.fail(f"unexpected stage {stage}")

    def forge(self, *argv):
        with patch.object(cli.Run, "run", lambda job, *a, **k: self.model_step(job, *a, **k)), \
                patch.object(cli, "hf_file", lambda repo, name: self.tmp / name), \
                patch.dict(os.environ, {"FORGE_KEEP_WORK": "0"}), redirect_stderr(StringIO()) as log:
            code = cli.main(list(argv))
        return code, [json.loads(line) for line in log.getvalue().splitlines()]

    def test_concept_only_writes_the_picture_and_stops(self):
        out = self.tmp / "fox.png"
        code, events = self.forge("generate", "--prompt", "a small red fox sitting", "--seed", "7",
                                  "--output", str(out), "--concept-only")
        self.assertEqual(code, 0, events)
        self.assertEqual(self.stages, ["concept"])
        sd = self.argvs["concept"]
        self.assertEqual(sd[sd.index("--seed") + 1], "7")
        self.assertEqual(out.read_bytes(), b"concept picture")
        manifest = json.loads(Path(f"{out}.manifest.json").read_text())
        self.assertTrue(manifest["concept_only"])
        self.assertEqual((manifest["seed"], manifest["prompt"]), (7, "a small red fox sitting"))
        self.assertEqual(events[-1]["pct"], 100)

    def test_concept_only_refuses_an_input_picture(self):
        image = self.tmp / "in.png"
        image.touch()
        code, events = self.forge("generate", "--prompt", "a fox", "--image", str(image),
                                  "--output", str(self.tmp / "o.png"), "--concept-only")
        self.assertEqual(code, 1)
        self.assertIn("--concept-only", events[-1]["error"])
        self.assertEqual(self.stages, [])

    def test_skip_gameready_writes_the_generator_mesh(self):
        out = self.tmp / "fox.glb"
        with patch.object(cli, "GAMEREADY", self.tmp / "absent"):
            code, events = self.forge("generate", "--prompt", "a small red fox sitting", "--seed", "7",
                                      "--output", str(out), "--skip-gameready")
        self.assertEqual(code, 0, events)
        self.assertEqual(self.stages, ["concept", "matte", "generate"])
        self.assertTrue(out.is_file())
        manifest = json.loads(Path(f"{out}.manifest.json").read_text())
        self.assertTrue(manifest["gameready"].startswith("skipped"))
        self.assertNotIn("lods", manifest)

    def test_concept_server_replaces_sd_cli(self):
        seen = []

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                seen.append((self.path, json.loads(self.rfile.read(int(self.headers["Content-Length"])))))
                reply = json.dumps({"images": [base64.b64encode(b"served picture").decode()]}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(reply)))
                self.end_headers()
                self.wfile.write(reply)

            def log_message(self, *args):
                pass

        server = HTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        out = self.tmp / "fox.png"
        code, events = self.forge("generate", "--prompt", "a small red fox sitting", "--seed", "7", "--output", str(out),
                                  "--concept-only", "--concept-server", f"http://127.0.0.1:{server.server_port}/")
        self.assertEqual(code, 0, events)
        self.assertEqual(self.stages, [])  # no sd-cli child
        self.assertEqual(out.read_bytes(), b"served picture")
        path, body = seen[0]
        self.assertEqual(path, "/sdapi/v1/txt2img")
        self.assertEqual({k: body[k] for k in ("width", "height", "steps", "cfg_scale", "seed", "sampler_name")},
                         {"width": 1024, "height": 1024, "steps": 4, "cfg_scale": 1.0, "seed": 7, "sampler_name": "euler"})
        self.assertTrue(body["prompt"].startswith("a small red fox sitting, "))
        manifest = json.loads(Path(f"{out}.manifest.json").read_text())
        self.assertIn("server", manifest["timings"]["concept"])

        with patch.object(cli.anatomy, "inspect", return_value={"status": "checked"}):
            code, events = self.forge("generate", "--prompt", "A robot dog on six legs", "--body", "other",
                                      "--seed", "7", "--output", str(out), "--concept-only",
                                      "--concept-server", f"http://127.0.0.1:{server.server_port}")
        self.assertEqual(code, 0, events)
        body = seen[-1][1]
        self.assertEqual(body["steps"], 8)
        self.assertEqual(base64.b64decode(body["extra_images"][0]),
                         cli.anatomy.structure_reference("A robot dog on six legs", "other").read_bytes())


    def test_unreachable_concept_server_is_a_clean_failure(self):
        code, events = self.forge("generate", "--prompt", "a fox", "--output", str(self.tmp / "o.png"),
                                  "--concept-only", "--concept-server", "http://127.0.0.1:9/")
        self.assertEqual(code, 1)
        self.assertIn("concept server http://127.0.0.1:9/", events[-1]["error"])

    def test_character_draft_finishes_like_one_go(self):
        draft = self.tmp / "fox.glb"
        with patch.object(cli, "UPSCALER_WEIGHTS", self.tmp / "absent"):
            code, events = self.forge("generate", "--prompt", "a small red fox standing", "--kind", "character",
                                      "--height-m", "0.45", "--output", str(draft), "--skip-gameready")
        self.assertEqual(code, 0, events)
        self.assertNotIn("gameready", self.stages)
        receipt = json.loads(Path(f"{draft}.manifest.json").read_text())
        self.assertEqual(receipt["finish"], {"kind": "character", "quality": "standard", "inputs": "fox.glb.finish",
                                             "size_request": {"axis": "height", "metres": 0.45}})
        self.assertAlmostEqual(receipt["size_m"][1], 0.45, places=5)
        kept = self.tmp / "fox.glb.finish" / "raw.svviews"
        self.assertEqual(sorted(p.name for p in kept.iterdir()), ["input.png", "transforms.json"])

        self.stages.clear()
        final = self.tmp / "final.glb"
        with patch.object(cli, "UPSCALER_WEIGHTS", self.tmp / "absent"):
            code, events = self.forge("gameready", "--mesh", str(draft), "--poly-budget", "8000",
                                      "--output", str(final))
        self.assertEqual(code, 0, events)
        self.assertEqual(self.stages, ["gameready", "upscale_views", "photo_paint", "apply_colour"])
        doc, binary = self.consumed  # game-ready saw the generator's exact mesh, as in one go
        self.assertEqual(bytes(binary), b"".join(struct.pack("<3f", *p) for p in POINTS))
        self.assertEqual([n["name"] for n in doc["nodes"]], ["raw"])
        manifest = json.loads(Path(f"{final}.manifest.json").read_text())
        self.assertTrue(manifest["generation"]["projection"]["applied"])
        self.assertEqual(manifest["size_request"], {"axis": "height", "metres": 0.45})
        self.assertAlmostEqual(manifest["size_m"][1], 0.45, places=5)
        self.assertFalse((self.tmp / "fox.glb.finish").exists())

    def test_character_draft_without_its_inputs_is_refused(self):
        draft = self.tmp / "fox.glb"
        code, _ = self.forge("generate", "--prompt", "a fox", "--kind", "character", "--quality", "draft",
                             "--output", str(draft), "--skip-gameready")
        self.assertEqual(code, 0)
        shutil.rmtree(self.tmp / "fox.glb.finish")
        code, events = self.forge("gameready", "--mesh", str(draft), "--output", str(self.tmp / "f.glb"))
        self.assertEqual(code, 1)
        self.assertIn("finishing inputs are missing", events[-1]["error"])

    def test_concept_server_command_serves_until_sigterm(self):
        fake = self.tmp / "sd-server.py"
        fake.write_text(FAKE_SD_SERVER)
        launcher = self.tmp / "fake-sd-server"
        record = self.tmp / "record.json"
        launcher.write_text(f"#!/bin/sh\nexec {sys.executable} {fake} \"$@\" --record {record}\n")
        launcher.chmod(0o755)
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        code = (f"import sys; from pathlib import Path; from forge import cli; cli.SD_SERVER = Path({str(launcher)!r}); "
                f"cli.HEAVY = Path('/usr/bin/env'); cli.hf_file = lambda repo, name: Path(name); "
                f"sys.exit(cli.main(['concept-server', '--port', '{port}']))")
        server = subprocess.Popen([sys.executable, "-c", code], cwd=cli.ROOT, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True, env={**os.environ, "PYTHONPATH": str(cli.ROOT)})
        self.addCleanup(server.stderr.close)
        self.addCleanup(server.stdout.close)
        self.addCleanup(lambda: server.poll() is None and server.kill())
        ready = json.loads(server.stdout.readline())
        self.assertEqual(ready, {"ready": True, "url": f"http://127.0.0.1:{port}"})
        child = json.loads(record.read_text())
        self.assertIn("flux-2-klein-4b-Q8_0.gguf", child["argv"])
        self.assertEqual(child["argv"][child["argv"].index("--steps") + 1], "4")

        out = self.tmp / "fox.png"
        code, events = self.forge("generate", "--prompt", "a fox", "--output", str(out), "--concept-only",
                                  "--concept-server", ready["url"])
        self.assertEqual(code, 0, events)
        self.assertEqual(out.read_bytes(), b"warm picture")

        server.send_signal(signal.SIGTERM)
        self.assertEqual(server.wait(timeout=15), 0, server.stderr.read())
        for _ in range(50):  # the server's whole process group is stopped
            try:
                os.kill(child["pid"], 0)
            except ProcessLookupError:
                break
            time.sleep(0.1)
        else:
            self.fail("sd-server still running after SIGTERM")

    def test_print_prompt_prints_the_concept_wording_and_draws_nothing(self):
        for kind, extra in (("character", ["--kind", "character"]), ("prop", ["--kind", "prop"]), (None, []),
                            ("character", ["--hero-face"])):
            with self.subTest(kind=kind, extra=extra), redirect_stderr(StringIO()) as log, \
                    patch("sys.stdout", new_callable=StringIO) as out, \
                    patch.object(cli.Run, "run", lambda *a, **k: self.fail("nothing may run")):
                code = cli.main(["generate", "--prompt", "a red fox", *extra, "--print-prompt"])
            self.assertEqual((code, log.getvalue()), (0, ""))
            lines = out.getvalue().splitlines()
            self.assertEqual(len(lines), 1)
            doc = json.loads(lines[0])
            self.assertEqual(doc, {"prompt": cli.prompt_for("a red fox", kind), "negative_prompt": "",
                                   "width": 1024, "height": 1024, "kind": kind, "body": None,
                                   "cloud_suffix": cli.CLOUD_SUFFIX})
            self.assertTrue(doc["prompt"].startswith("a red fox, "))
        self.assertIn("A-pose", cli.prompt_for("a red fox", "character"))
        image = self.tmp / "in.png"
        image.touch()
        for argv in (["--image", str(image), "--prompt", "a fox"], []):
            code, events = self.forge("generate", *argv, "--print-prompt")
            self.assertEqual(code, 1)
            self.assertIn("--print-prompt", events[-1]["error"])
        code, events = self.forge("generate", "--prompt", "a fox")
        self.assertEqual((code, events[-1]["error"]), (1, "--output is required"))
        self.assertEqual(self.stages, [])

    def test_seashell_drums_keep_the_shell_material_in_the_concept(self):
        for material in ("sea shells", "sea-shells", "seashells", "scallops"):
            with self.subTest(material=material):
                text = cli.prompt_for(f"A low drum kit built from pink {material}.", "prop")
                self.assertIn("fluted shell body", text)
        self.assertNotIn("fluted shell body", cli.prompt_for("A maple drum shell.", "prop"))
        self.assertNotIn("fluted shell body", cli.prompt_for("A maple drum kit with seashell cymbals.", "prop"))
        self.assertNotIn("fluted shell body", cli.prompt_for("A drum kit made from maple with seashell cymbals.", "prop"))

    def test_a_snow_figure_described_by_its_parts_gets_a_vertical_stack(self):
        prompt = ("Three round balls of packed white snow with a carrot nose, coal eyes and buttons, "
                  "twig arms, and a red and cream striped knit scarf.")
        self.assertTrue(cli.anatomy.needs_base_check(prompt, "legless"))
        with patch.object(cli.anatomy, "inspect", return_value={"status": "checked"}):
            code, events = self.forge("generate", "--prompt", prompt, "--body", "legless",
                                      "--concept-only", "--output", str(self.tmp / "snowman.png"))
        self.assertEqual(code, 0, events)
        argv = self.argvs["concept"]
        guide = Path(argv[argv.index("-r") + 1])
        self.assertTrue(guide.is_file())
        self.assertEqual(guide.name, "three-stacked-snowballs.png")
        text = argv[argv.index("-p") + 1]
        self.assertIn("one vertical stack", text)
        self.assertNotIn("six separately visible feet", text)
        for other in ("Three snowballs in a pile", "A snowman made of two snowballs",
                      "A snowman holding three snowballs", "A legless fish with a carrot-orange nose"):
            self.assertIsNone(cli.anatomy.structure_reference(other, "legless"), other)

    def test_seashell_kit_reference_uses_the_shells_as_resonators(self):
        prompt = "A compact drum kit built from ivory seashells, with drumheads over large spiral shells and scallop cymbals."
        code, events = self.forge("generate", "--prompt", prompt, "--concept-only",
                                  "--output", str(self.tmp / "shells.png"))
        self.assertEqual(code, 0, events)
        args = self.argvs["concept"]
        guide = Path(args[args.index("-r") + 1])
        self.assertEqual(guide.name, "coiled-shell-percussion.png")
        self.assertTrue(guide.is_file())
        self.assertIn("spiral", args[args.index("-p") + 1])
        self.assertNotIn("fan-shaped scallop valves", args[args.index("-p") + 1])

    def test_scallop_drum_kit_draws_separate_playing_surfaces(self):
        prompt = ("A low drum kit made of hollow cream and coral seashells, with a large scallop "
                  "shell in front and smaller shells on short stands.")
        code, events = self.forge("generate", "--prompt", prompt, "--concept-only",
                                  "--output", str(self.tmp / "scallops.png"))
        self.assertEqual(code, 0, events)
        args = self.argvs["concept"]
        self.assertIn("-r", args)
        guide = Path(args[args.index("-r") + 1])
        self.assertEqual(guide.name, "scallop-shell-percussion.png")
        self.assertTrue(guide.is_file())
        self.assertIn("separate drumhead", args[args.index("-p") + 1])

    def test_counted_wheelbase_survives_modifiers_in_the_planners_prompt(self):
        from forge.quality import wheels
        for description, expected in [
                ("Exactly six chunky wheels, three on each side.", 6),
                ("A boxy Moon rover with exactly six chunky dark wheels arranged three per side.", 6),
                ("A chassis on six deeply treaded wheels.", 6),
                ("A rover with six dark, deeply treaded wheels, three on each side.", 6),
                ("A carrier with three wide, black and dusty wheels per side.", 6),
                ("Six doors, dark rubber wheels and two fog lights.", None),
                ("An eight-wheeled cart.", 8),
                ("A cart with three wheels on each side.", 6),
                ("A four-seat cab with polished wheels.", None),
                ("A cart with six-spoked wheels.", None),
                ("Six storage boxes and wheels.", None),
                ("Four doors and six independently sprung wheels.", 6)]:
            with self.subTest(description=description):
                self.assertEqual(wheels.requested(description), expected)
        prompt = cli.prompt_for("A silver rover with exactly six chunky wheels.", "prop")
        self.assertIn("3 distinct axles", prompt)

    def test_body_picks_the_pose_words_and_keeps_faces_off_robots_and_animals(self):
        def printed(*extra):
            with redirect_stderr(StringIO()), patch("sys.stdout", new_callable=StringIO) as out:
                code = cli.main(["generate", "--prompt", "a rusty robot", *extra, "--print-prompt"])
            self.assertEqual(code, 0)
            return json.loads(out.getvalue())
        human = ("A-pose", "palms")
        face = ("eyelids", "expression", "hair")
        for body, has, lacks in (("humanoid", human + ("fingers spread",), face),
                                 ("quadruped", ("all four legs", "tail held out behind and clear of the legs"),
                                  human + face + ("fingers",)),
                                 ("bird", ("wings folded", "both legs and feet visible"), human + face),
                                 ("legless", ("whole length", "does not cross itself"), human + face),
                                 ("other", ("limbs held apart",), human + face)):
            with self.subTest(body=body):
                doc = printed("--body", body)
                self.assertEqual((doc["kind"], doc["body"]), ("character", body))
                self.assertTrue(doc["prompt"].startswith("a rusty robot, full body"))
                for word in has:
                    self.assertIn(word, doc["prompt"])
                for word in lacks:
                    self.assertNotIn(word, doc["prompt"])
        hero = printed("--body", "humanoid", "--hero-face")["prompt"]
        self.assertTrue(all(word in hero for word in face + human))
        self.assertIsNone(printed("--kind", "character")["body"])   # no --body: today's wording, unchanged
        self.assertEqual(printed("--kind", "character")["prompt"], cli.prompt_for("a rusty robot", "character"))
        for argv, message in ((["--body", "fish"], "--body is one of"), (["--body", "bird", "--kind", "prop"], "prop"),
                              (["--body", "bird", "--hero-face"], "humanoid")):
            code, events = self.forge("generate", "--prompt", "x", *argv, "--print-prompt")
            self.assertEqual(code, 1)
            self.assertIn(message, events[-1]["error"])
        out = self.tmp / "fox.png"
        code, events = self.forge("generate", "--prompt", "a small red fox", "--body", "quadruped",
                                  "--output", str(out), "--concept-only")
        self.assertEqual(code, 0, events)
        self.assertIn(cli.prompt_for("a small red fox", "character", "quadruped"), self.argvs["concept"])

    def test_human_shaped_drawing_for_a_fish_is_redrawn_before_shape(self):
        out = self.tmp / "fish.png"
        rejected = {"status": "rejected", "reason": "human arms"}
        with patch.object(cli.anatomy, "inspect", side_effect=[rejected, {"status": "checked"}]):
            code, events = self.forge("generate", "--prompt", "striped fish", "--body", "legless",
                                     "--seed", "42", "--concept-only", "--output", str(out))
        self.assertEqual(code, 0, events)
        self.assertEqual(self.stages.count("concept"), 2)
        self.assertIn("43", self.argvs["concept"])
        self.assertNotIn("shape", self.stages)

    def test_count_rejection_changes_the_redraw_view(self):
        cases = (
            ("An orange octopus with eight arms.", {"visible_tips": 6}, "directly overhead"),
            ("A dog with exactly six jointed legs.", {"visible_feet": 4}, "front, middle and rear"),
        )
        for prompt, measurement, expected in cases:
            with self.subTest(prompt=prompt):
                rejected = {"status": "rejected", "reason": "silhouette count mismatch", **measurement}
                with patch.object(cli.anatomy, "inspect", side_effect=[rejected, {"status": "checked"}]):
                    code, events = self.forge("generate", "--prompt", prompt, "--body", "other",
                                              "--seed", "42", "--concept-only", "--output", str(self.tmp / "view.png"))
                self.assertEqual(code, 0, events)
                text = self.argvs["concept"][self.argvs["concept"].index("-p") + 1]
                self.assertIn(expected, text)
                self.assertIn(prompt, text)
                self.assertNotIn(cli.anatomy.description(prompt, "other"), text)
                self.assertIn("43", self.argvs["concept"])

    def test_six_wheeled_vehicle_uses_a_three_axle_drawing_reference(self):
        prompt = "A pale metal rover with exactly six dark, deeply treaded wheels, three on each side."
        code, events = self.forge("generate", "--prompt", prompt, "--concept-only",
                                  "--output", str(self.tmp / "rover.png"))
        self.assertEqual(code, 0, events)
        args = self.argvs["concept"]
        self.assertIn("-r", args)
        guide = Path(args[args.index("-r") + 1])
        self.assertTrue(guide.is_file())
        self.assertFalse(cli.wheels.three_axle_vehicle("A watch movement with six wheels"))
        self.assertIn("three axles", args[args.index("-p") + 1])
        self.assertNotIn("feet", args[args.index("-p") + 1])
        self.assertEqual(args[args.index("--steps") + 1], "8")

    def test_mechanical_canine_structure_reaches_the_concept_engine(self):
        prompt = "A red mechanical hound with six articulated legs."
        with patch.object(cli.anatomy, "inspect", return_value={"status": "checked"}):
            code, events = self.forge("generate", "--prompt", prompt, "--body", "other",
                                      "--concept-only", "--output", str(self.tmp / "hound.png"))
        self.assertEqual(code, 0, events)
        args = self.argvs["concept"]
        self.assertIn("-r", args)
        self.assertIn(prompt, args[args.index("-p") + 1])
        self.assertEqual(args[args.index("--steps") + 1], "8")
        self.assertIsNone(cli.anatomy.structure_reference("A live hound with six legs", "other"))
        self.assertIsNone(cli.anatomy.structure_reference("A red robot dog with four legs", "quadruped"))

    def test_a_curled_arm_counts_once_and_a_missing_arm_still_fails(self):
        from PIL import Image, ImageDraw
        from forge.quality import anatomy

        # The octopus's lower-left arm ends in two tiny silhouette spurs.
        outline = Image.open(Path(__file__).parent / "fixtures/curled-octopus-outline.png").convert("L")
        for count in (8, 7):
            if count == 7:
                ImageDraw.Draw(outline).polygon([(156, 162), (0, 179), (0, 319)], fill=0)
            matte = self.tmp / f"curled-{count}.png"
            image = Image.new("RGBA", outline.size)
            image.putalpha(outline)
            image.save(matte)
            self.assertEqual(len(anatomy.tentacle_tips(matte)), count)

    def test_planned_leg_modifiers_still_select_the_guide_and_silhouette_check(self):
        prompt = ("A small dog-shaped robot with exactly six hinged metal legs, three on each side, "
                  "a low silver body, and a head with short pointed metal ears.")
        with patch.object(cli.anatomy, "inspect", return_value={"status": "checked"}):
            code, events = self.forge("generate", "--prompt", prompt, "--body", "other",
                                      "--concept-only", "--output", str(self.tmp / "dog.png"))
        self.assertEqual(code, 0, events)
        self.assertIn("check_base", self.stages)
        args = self.argvs["concept"]
        self.assertEqual(Path(args[args.index("-r") + 1]).name, "mechanical-canine-six.png")
        for words in ("six hinged metal legs", "three pairs of articulated legs",
                      "three slender legs on each side", "legs, three on each side"):
            self.assertEqual(cli.anatomy.paired_legs(words), 6, words)
        for words in ("six hinges attached to its legs", "six spots on each of its legs",
                      "four legs, three toes on each side"):
            self.assertNotEqual(cli.anatomy.paired_legs(words), 6, words)

    def test_compact_legless_face_is_drawn_from_the_front_and_rejects_feet(self):
        from PIL import Image, ImageDraw
        from forge.quality import anatomy
        prompt = "A round legless garden slime with two large round eyes and a speckled green body."
        text = cli.prompt_for(prompt, "character", "legless")
        self.assertIn("front", text)
        self.assertNotIn(cli.BODIES["legless"], text)
        self.assertTrue(anatomy.needs_matte(prompt, "legless"))
        matte = self.tmp / "slime.png"
        for feet in (0, 4):
            image = Image.new("RGBA", (180, 120))
            draw = ImageDraw.Draw(image)
            draw.ellipse((15, 15, 165, 90), fill="green")
            for x in range(feet):
                draw.rectangle((32 + x * 30, 65, 42 + x * 30, 110), fill="green")
            image.save(matte)
            with patch("sys.platform", "linux"):
                check = anatomy.inspect(matte, "legless", prompt, matte)
            if feet:
                self.assertEqual(check["status"], "rejected")
                self.assertEqual(check["visible_feet"], feet)
                self.assertIn("belly", anatomy.redraw_description(prompt, "legless", check))
            else:
                self.assertEqual(check["status"], "checked")
        for other in ("A long coiled snake with two round eyes.",
                      "A tiny pond fish with two large eyes.",
                      "A floating jellyfish with two big eyes and trailing tentacles."):
            self.assertFalse(anatomy.needs_matte(other, "legless"), other)
        self.assertFalse(anatomy.needs_matte(prompt, "quadruped"))

    def test_visible_feet_of_explicit_multi_legged_creature(self):
        from PIL import Image, ImageDraw
        from forge.quality import anatomy
        requested = "A walking brass surveyor with exactly six jointed legs."
        self.assertTrue(anatomy.needs_matte(requested, "other"))
        with patch("sys.platform", "linux"):
            for count in (4, 6, 7):
                img = Image.new("RGBA", (240, 180))
                draw = ImageDraw.Draw(img)
                draw.ellipse((14, 16, 226, 103), fill="black")
                for i in range(count):
                    x = 34 + round(i * 165 / (count-1))
                    draw.rectangle((x, 80, x+9, 158), fill="black")
                    if i == 0:
                        draw.line((x+4, 101, x+4, 155), fill=(0, 0, 0, 0), width=1)
                matte = self.tmp / f"walker-{count}.png"
                img.save(matte)
                check = anatomy.inspect(matte, "other", requested, matte)
                self.assertEqual(json.loads(json.dumps(check))["visible_feet"], count)
                self.assertEqual(check["status"], "checked" if count == 6 else "rejected")

    def test_flags_are_exclusive(self):
        code, events = self.forge("generate", "--prompt", "a fox", "--output", str(self.tmp / "o.glb"),
                                  "--concept-only", "--skip-gameready")
        self.assertEqual(code, 1)
        self.assertIn("choose one", events[-1]["error"])


if __name__ == "__main__":
    unittest.main()
