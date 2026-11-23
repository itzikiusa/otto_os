"""forge accepts Otto's one argv shape on every stage, empty strings included.

Otto's "Otto Forge (local)" provider runs, for every stage:
    forge STAGE --mesh {mesh} --image {image} --prompt {prompt} --output {output}
                --poly-budget {poly_budget} --seed {seed}
with "" for whatever does not apply. Includes a small headless Blender game-ready fixture:
    engines/assetfurnace/.venv/bin/python -m unittest tests.test_forge_argv
Set FORGE_E2E_MESH=<rigged-or-not humanoid glb> to also run rig + animate for real (minutes).
"""

import json
import os
import shutil
import time
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FORGE = ROOT / "bin" / "forge"


def otto_argv(stage, output, mesh="", image="", prompt="", poly_budget="", seed=""):
    return [str(FORGE), stage, "--mesh", mesh, "--image", image, "--prompt", prompt,
            "--output", output, "--poly-budget", poly_budget, "--seed", seed]


def run(argv):
    done = subprocess.run(argv, capture_output=True, text=True, timeout=3600)
    lines = [json.loads(line) for line in done.stderr.splitlines() if line.startswith("{")]
    return done.returncode, lines


class OttoArgvShape(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.out = str(self.tmp / "out.glb")

    def assert_clean_failure(self, argv, message):
        code, lines = run(argv)
        self.assertEqual(code, 1)  # not argparse's 2: every flag was accepted
        self.assertIn(message, lines[-1]["error"])
        self.assertFalse(Path(self.out).exists())

    def test_generate_with_nothing_to_start_from(self):
        self.assert_clean_failure(otto_argv("generate", self.out), "give --prompt or --image")

    def test_generate_with_a_missing_image(self):
        self.assert_clean_failure(
            otto_argv("generate", self.out, image=str(self.tmp / "none.png"), prompt="a chest",
                      poly_budget="5000", seed="7"), "--image not found")

    def test_mesh_stages_need_a_mesh(self):
        for stage in ("gameready", "rig", "animate"):
            with self.subTest(stage=stage):
                self.assert_clean_failure(otto_argv(stage, self.out), "--mesh is required")

    def test_mesh_stages_with_a_missing_mesh(self):
        for stage in ("gameready", "rig", "animate"):
            with self.subTest(stage=stage):
                self.assert_clean_failure(
                    otto_argv(stage, self.out, mesh=str(self.tmp / "none.glb"), poly_budget="8000",
                              seed="1"), "--mesh not found")

    def test_dash_prefixed_otto_values_reach_each_stage(self):
        for prompt in ("--help-me", "-knight", "--help", "--output"):
            for stage in ("generate", "gameready", "rig", "animate"):
                with self.subTest(prompt=prompt, stage=stage):
                    result = subprocess.run(otto_argv(stage, self.out, prompt=prompt,
                                                      image="-absent.png", mesh="--missing.glb"),
                                            capture_output=True, text=True)
                    self.assertEqual(result.returncode, 1)
                    line = json.loads(result.stderr.splitlines()[-1])
                    expected = "--image not found" if stage == "generate" else "--mesh not found"
                    self.assertIn(expected, line["error"])
                    self.assertIn("-absent.png" if stage == "generate" else "--missing.glb", line["error"])

    def test_legacy_brace_placeholders_use_stage_defaults(self):
        from unittest.mock import patch
        from forge import cli
        from forge.cli import literal_option_values
        for stage in ("generate", "gameready", "rig", "animate"):
            with self.subTest(stage=stage):
                self.assert_clean_failure(otto_argv(stage, self.out, mesh="{mesh}", image="{image}",
                                                    prompt="{prompt}", seed="{seed}") + ["--clips={clips}"],
                                          "give --prompt or --image" if stage == "generate" else "--mesh is required")
        with patch.object(cli, "animate") as handler:
            cli.main(["animate", "--clips", "{clips}", "--seed={seed}", "--output", self.out])
            args = handler.call_args.args[0]
            self.assertEqual((args.clips, args.seed), ("", 42))
        self.assertEqual(literal_option_values(["--clips", "{clips}", "--mesh={mesh}",
                                               "--prompt", "a {knight}", "--seed="],
                                              {"--clips", "--mesh", "--prompt", "--seed"}),
                         ["--prompt=a {knight}"])

    def test_missing_option_value_ends_with_json_error(self):
        p = subprocess.run([FORGE, "generate", "--output", self.out, "--prompt"],
                           capture_output=True, text=True)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("--prompt", json.loads(p.stderr.splitlines()[-1])["error"])

    @unittest.skipUnless(os.environ.get("FORGE_E2E_MESH"), "set FORGE_E2E_MESH to run rig + animate")
    def test_rig_then_animate_for_real(self):
        rigged = str(self.tmp / "rigged.glb")
        code, lines = run(otto_argv("rig", rigged, mesh=os.environ["FORGE_E2E_MESH"], seed="7"))
        self.assertEqual(code, 0, lines[-1:])
        from forge.glbstats import read_glb
        first, first_bin = read_glb(Path(rigged))
        receipt = json.loads(Path(rigged + ".manifest.json").read_text())
        self.assertEqual(receipt["seed"], 7)
        from forge.quality.package import size_m
        before = size_m(*read_glb(Path(os.environ["FORGE_E2E_MESH"])))
        for actual, expected in zip(receipt["size_m"], before, strict=True):
            self.assertAlmostEqual(actual, expected, delta=max(0.001, expected * 0.01))
        code, lines = run(otto_argv("rig", rigged, mesh=os.environ["FORGE_E2E_MESH"], seed="7"))
        self.assertEqual(code, 0, lines[-1:])
        second, second_bin = read_glb(Path(rigged))
        self.assertEqual(first["skins"], second["skins"])
        self.assertEqual(first["nodes"], second["nodes"])
        self.assertTrue(first_bin == second_bin, "rig geometry, weights or bind matrices changed")
        code, lines = run(otto_argv("animate", self.out, mesh=rigged) + ["--clips", "{clips}"])
        self.assertEqual(code, 0, lines[-1:])
        manifest = json.loads(Path(self.out + ".manifest.json").read_text())
        self.assertEqual([c["name"] for c in manifest["clips"]], ["idle", "walk", "run"])
        self.assertTrue(manifest["bones"])
        for actual, expected in zip(manifest["size_m"], receipt["size_m"], strict=True):
            self.assertAlmostEqual(actual, expected, delta=max(0.001, expected * 0.01))
        self.assertEqual(sorted(p.name for p in self.tmp.iterdir()),
                         ["out.glb", "out.glb.manifest.json", "rigged.glb", "rigged.glb.manifest.json"])


class GenerationSource(unittest.TestCase):
    def test_game_ready_consumes_selected_source_before_cleanup(self):
        from contextlib import redirect_stderr
        from io import StringIO
        from unittest.mock import patch
        from forge import cli

        with tempfile.TemporaryDirectory() as temp:
            folder = Path(temp)
            image = folder / "input.png"
            image.touch()
            cases = [(None, False, "standard", None, None), ("raw", True, "draft", 6, "character"),
                     ("finish", False, "high", 12, "character"), (None, False, "standard", None, "character"),
                     ("raw", True, "high", 12, "character"), (None, False, "standard", None, "prop")]
            for source, keep, quality, steps, kind in cases:
                with self.subTest(source=source, keep=keep, quality=quality, kind=kind):
                    detailed = kind == "character" and source != "finish"
                    projected = detailed and quality != "draft"
                    stages = []
                    staging = []

                    def model_step(job, stage, args, **kwargs):
                        stages.append(stage)
                        if stage == "check_back_view":
                            self.assertTrue(keep)
                        elif stage == "matte":
                            Path(args[-1]).touch()
                            Path(args[-1]).with_suffix(".json").write_text('{"method":"input_alpha"}')
                        elif stage == "generate":
                            self.assertEqual(args[args.index("--steps") + 1] if "--steps" in args else None, steps)
                            self.assertEqual("--back" in args, keep)
                            raw = job.work / "raw.glb"
                            cli.write_glb({"asset": {"version": "2.0"}, "meshes": []}, b"", raw)
                            raw.with_suffix(".svviews").mkdir()
                            (raw.with_suffix(".svviews") / "transforms.json").write_text("{}")
                            raw.with_suffix(".provenance.json").write_text(json.dumps({
                                "parameters": {"steps": 8}, "input": {"sha256": "front"},
                                "output": {"sha256": "raw"}}))
                            (job.work / "generate.log").write_text("[trellis] generating model with seed 42")
                        elif stage == "finish":
                            shutil.copyfile(job.work / "raw.glb", job.work / "finished.glb")
                            (job.work / "finished.retopo-repaint.json").write_text('{"photo":{"count":1}}')
                        elif stage == "gameready":
                            consumed = Path(args[1])
                            self.assertEqual(consumed.name, "finished.glb" if source == "finish" else "raw.glb")
                            self.assertTrue((job.work / "raw.svviews/transforms.json").is_file())
                            self.assertTrue((job.work / "raw.glb").is_file())
                            staging.append(job.work)
                            shutil.copyfile(consumed, args[3])
                            self.assertEqual("--detail" in args, detailed)
                            self.assertEqual("--pack" in args, detailed)
                            if detailed:
                                self.assertEqual(args[args.index("--detail")+1], "head,hands")
                                pack = Path(args[args.index("--pack")+1])
                                pack.mkdir()
                                cli.write_glb({"nodes": [{"name": "UCX_gameready_00", "mesh": 0},
                                                         {"name": "gameready", "mesh": 1}],
                                               "meshes": [{"name": "hull"}, {"name": "display"}]},
                                              b"", pack / "gameready_LOD0.glb")
                        elif stage == "upscale_views":
                            self.assertTrue(projected)
                            self.assertEqual(args[-2], job.work / "raw.svviews")
                            args[-1].mkdir()
                            (args[-1] / "upscale.json").write_text('{"front.png":{"output_px":[4160,4160]}}')
                        elif stage == "photo_paint":
                            self.assertEqual(args[2], job.work / "display_LOD0.glb")
                            paint_doc, _ = cli.glbstats.read_glb(args[2])
                            self.assertEqual(paint_doc["meshes"], [{"name": "display"}])
                            self.assertEqual([node["name"] for node in paint_doc["nodes"] if "mesh" in node],
                                             ["gameready"])
                            self.assertEqual(dict(zip(args[5::2], args[6::2], strict=True)),
                                             {"--facing-high": 0.85, "--edge-px": 12,
                                              "--cutout-edge": 16 / 768, "--inner-edge": 16 / 768})
                            self.assertTrue((args[3] / "upscale.json").is_file())
                            shutil.copyfile(args[2], args[4])
                            (job.work / "photo_paint.log").write_text("[photo-paint] 1 view(s), 6% of the texture")
                        elif stage == "apply_colour":
                            self.assertEqual(args[-3], job.work / "gameready.glb")
                            self.assertEqual(args[-2], job.work / "painted_LOD0.glb")
                            shutil.copyfile(args[-3], args[-1])
                        else:
                            self.fail(stage)

                    output = folder / "game.glb"
                    argv = ["generate", "--image", str(image), "--output", str(output)]
                    argv.extend(["--quality", quality])
                    if source:
                        argv.extend(["--source", source])
                    if kind:
                        argv.extend(["--kind", kind])
                    if keep:
                        argv.extend(["--back-image", str(image)])
                    with patch.object(cli.Run, "run", model_step), patch.object(cli, "UPSCALER_WEIGHTS", image), patch.dict(os.environ, {
                        "FORGE_KEEP_WORK": "1" if keep else "0"
                    }), redirect_stderr(StringIO()) as log:
                        self.assertEqual(cli.main(argv), 0, log.getvalue())
                    self.assertEqual("finish" in stages, source == "finish")
                    self.assertTrue(output.is_file())
                    manifest = json.loads(Path(str(output) + ".manifest.json").read_text())
                    self.assertEqual(manifest["source"], source or "raw")
                    self.assertEqual(manifest["generation"]["quality"], quality)
                    self.assertEqual(manifest["generation"]["views"], "front + back" if keep else "front")
                    if keep:
                        self.assertTrue(any("pixal3d-q8_0-v1" in m["name"] for m in manifest["models"]))
                    self.assertEqual("photo_paint" in stages, projected)
                    if projected:
                        self.assertEqual(stages[-3:], ["upscale_views","photo_paint","apply_colour"])
                        self.assertEqual(manifest["generation"]["projection"]["image"], "shared base colour")
                        progress = [json.loads(line) for line in log.getvalue().splitlines()]
                        self.assertTrue(any("Sharpening the face" in event.get("message","") for event in progress))
                    elif detailed:
                        self.assertEqual(manifest["generation"]["projection"], {"applied":False,"reason":"draft"})
                    self.assertEqual(len(staging), 1)
                    self.assertEqual(staging[0].exists(), keep)
                    if keep:
                        self.addCleanup(shutil.rmtree, staging[0])
                        self.assertTrue((staging[0] / "raw.svviews/transforms.json").is_file())
                        self.assertTrue((staging[0] / "raw.glb").is_file())


class WorkDirectory(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.mesh = self.tmp / "tetra.obj"
        self.mesh.write_text("v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 3 2\nf 1 2 4\nf 2 3 4\nf 3 1 4\n")
        self.work_root = ROOT / "out/work"

    def run_stage(self, keep, fail=False, cancel=False):
        self.output = self.tmp / f"{self.tmp.name}-{int(keep)}-{int(fail)}-{int(cancel)}.glb"
        mesh = self.mesh
        if fail:
            mesh = self.tmp / "unsupported.xyz"
            mesh.touch()
        env = dict(os.environ)
        env.pop("FORGE_KEEP_WORK", None)
        if keep:
            env["FORGE_KEEP_WORK"] = "1"
        with subprocess.Popen(otto_argv("gameready", str(self.output), mesh=str(mesh)),
                              env=env, stderr=subprocess.PIPE, text=True) as proc:
            if cancel:
                deadline = time.monotonic() + 45
                while not list(self.work_root.glob(f"{self.output.stem}-*/gameready.log")):
                    if proc.poll() is not None or time.monotonic() > deadline:
                        proc.kill()
                        self.fail("game-ready child did not start")
                    time.sleep(.02)
                proc.terminate()
            _, stderr = proc.communicate(timeout=120)
        dirs = list(self.work_root.glob(f"{self.output.stem}-*"))
        for directory in dirs:
            self.addCleanup(shutil.rmtree, directory)
        self.assertEqual(proc.returncode, 143 if cancel else 1 if fail else 0, stderr)
        if cancel or fail:
            self.assertIn("error", json.loads(stderr.splitlines()[-1]))
            self.assertFalse(self.output.exists())
        else:
            receipt = json.loads(Path(str(self.output) + ".manifest.json").read_text())
            self.assertGreater(self.output.stat().st_size, 100)
            self.assertGreater(receipt["triangles"], 0)
            self.assertEqual(Path(receipt["work_dir"]).exists(), keep)
        self.assertEqual(len(dirs), int(keep))
        if keep:
            self.assertTrue((dirs[0] / "gameready.log").is_file())

    def test_success_cleans(self):
        self.run_stage(False)

    def test_success_keeps_requested_work(self):
        self.run_stage(True)

    def test_failure_and_cancel(self):
        for cancel in (False, True):
            for keep in (False, True):
                with self.subTest(cancel=cancel, keep=keep):
                    self.run_stage(keep, fail=not cancel, cancel=cancel)


class ProjectedPackage(unittest.TestCase):
    def test_only_the_shared_base_colour_changes(self):
        import io
        import struct
        from PIL import Image
        from forge import cli, glbstats

        def png(colour):
            stream = io.BytesIO()
            Image.new("RGB", (2, 2), colour).save(stream, format="PNG")
            return stream.getvalue()

        old, painted, normal = png("red"), png("green"), png((128, 128, 255))
        positions = struct.pack("<9f", 0, 0, 0, 1, 0, 0, 0, 1, 0)
        primitive = {"attributes": {"POSITION": 0}, "material": 0}

        def write(path, base, names):
            blob = bytearray()
            views = []
            for data in (positions, base, normal):
                blob.extend(bytes(-len(blob) % 4))
                views.append({"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)})
                blob.extend(data)
            doc = {
                "asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": list(range(len(names)))}],
                "nodes": [{"name": n, "mesh": i} for i, n in enumerate(names)],
                "meshes": [{"primitives": [primitive]} for _ in names],
                "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                               "min": [0, 0, 0], "max": [1, 1, 0]}],
                "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}},
                               "normalTexture": {"index": 1}}],
                "textures": [{"source": 0}, {"source": 1}],
                "images": [{"bufferView": i, "mimeType": "image/png"} for i in (1, 2)],
                "bufferViews": views, "buffers": [{"byteLength": len(blob)}],
            }
            cli.write_glb(doc, blob, path)
            return doc

        with tempfile.TemporaryDirectory() as tmp:
            original, lod, result = (Path(tmp) / n for n in ("main.glb", "paint.glb", "result.glb"))
            original_doc = write(original, old, ["Knight", "Knight_LOD1", "UCX_Knight_00"])
            write(lod, painted, ["Knight"])
            subprocess.run([cli.AF_PY, ROOT / "forge/quality/apply_colour.py", original, lod, result],
                           env={**os.environ, "PYTHONPATH": os.pathsep.join(map(str, (ROOT, cli.AF)))},
                           check=True)
            doc, binary = glbstats.read_glb(result)
            for key in ("nodes", "scenes", "meshes", "materials", "textures", "accessors", "images"):
                self.assertEqual(doc[key], original_doc[key], key)
            for view, expected in zip(doc["bufferViews"], (positions, painted, normal), strict=True):
                start = view.get("byteOffset", 0)
                self.assertEqual(binary[start:start + view["byteLength"]], expected)


class PreparedRigInput(unittest.TestCase):
    def test_joint_predictor_receives_the_world_frame_of_a_nested_display_mesh(self):
        from forge import cli, glbstats
        from forge.quality import package

        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            src = work / "nested.glb"
            doc = {
                "asset": {"version": "2.0"}, "scenes": [{"nodes": [0]}],
                "nodes": [
                    {"name": "Measured frame", "children": [1],
                     "matrix": [1,0,0,0, 0,0,1,0, 0,-1,0,0, 0,0,0,1]},
                    {"name": "Skin", "mesh": 0, "translation": [0,2,0]},
                ], "meshes": [{"primitives": []}]
            }
            cli.write_glb(doc, b"", src)
            path = cli.rig_input(src, work)
            intake, _ = glbstats.read_glb(path)
            # SkinTokens reads a mesh relative to its parent. A direct mesh root
            # supplies the same world coordinates for sampling and weight transfer.
            root, = intake["scenes"][0]["nodes"]
            mesh = intake["nodes"][root]
            self.assertEqual(mesh.get("mesh"), 0)
            self.assertEqual(package.local_matrix(mesh), [
                [1,0,0,0], [0,0,-1,0], [0,1,0,2], [0,0,0,1]
            ])
            self.assertEqual(cli.rig_input(path, work), path)

    def test_rig_uses_visible_meshes_and_keeps_node_transforms(self):
        from forge import cli, glbstats
        import struct

        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            src = work / "prepared.glb"
            positions = struct.pack("<9f", 0, 0, 0, 1, 0, 0, 0, 1, 0)
            primitive = {"attributes": {"POSITION": 0}}
            data = {
                "asset": {"version": "2.0"},
                "scene": 0, "scenes": [{"nodes": [0]}],
                "nodes": [
                    {"children": [1, 2, 3, 4]},
                    {"name": "Hero", "mesh": 0, "translation": [1, 0, 0]},
                    {"name": "Hero_LOD1", "mesh": 1}, {"name": "UCX_Hero_00", "mesh": 2},
                    {"name": "Cape", "mesh": 3},
                ],
                "meshes": [{"primitives": [primitive]} for _ in range(4)],
                "buffers": [{"byteLength": len(positions)}],
                "bufferViews": [{"buffer": 0, "byteLength": len(positions)}],
                "accessors": [{"bufferView": 0, "count": 3, "componentType": 5126,
                               "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}],
            }
            cli.write_glb(data, positions, src)
            output = cli.rig_input(src, work)
            doc, binary = glbstats.read_glb(output)
            self.assertEqual(binary, positions)
            self.assertEqual([n["name"] for n in doc["nodes"] if "mesh" in n], ["Hero", "Cape"])
            self.assertEqual(doc["nodes"][1], data["nodes"][1])
            self.assertEqual(cli.rig_input(output, work), output)


if __name__ == "__main__":
    unittest.main()
