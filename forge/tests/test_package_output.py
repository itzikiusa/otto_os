"""Public Forge regression cases for already prepared geometry, physical size and child diagnostics."""

import contextlib
import io
import json
import os
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from forge import cli, glbstats


class PackageOutput(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.mesh = self.tmp / "renamed.glb"
        self.out = self.tmp / "asset.glb"
        xyz = struct.pack("<9f", 0, 0, 0, 4, 0, 0, 0, 2, 1)
        nodes = [
            {"name": n, "mesh": 0}
            for n in (
                "gameready",
                "gameready_LOD1",
                "gameready_LOD2",
                "gameready_LOD3",
                "UCX_gameready_00",
            )
        ]
        data = {
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": list(range(5))}],
            "nodes": nodes,
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
            "accessors": [
                {
                    "bufferView": 0,
                    "componentType": 5126,
                    "type": "VEC3",
                    "count": 3,
                    "min": [0, 0, 0],
                    "max": [4, 2, 1],
                }
            ],
            "bufferViews": [{"buffer": 0, "byteLength": len(xyz)}],
            "buffers": [{"byteLength": len(xyz)}],
        }
        cli.write_glb(data, xyz, self.mesh)

    def test_low_forward_eyes_must_still_be_a_matched_pair(self):
        import numpy as np
        from forge.quality.eyes import paired_irises
        patches = np.repeat([[.29, .35, .94], [.70, .35, .94]], 8, axis=0)
        bright = np.full(len(patches), 158)
        self.assertIsNotNone(paired_irises(patches, bright))
        patches[8:, 1] = .53
        self.assertIsNone(paired_irises(patches, bright))
        patches[:, 1] = .06
        self.assertIsNone(paired_irises(patches, bright))

    def test_eye_reflections_require_two_similar_glossy_patches_on_the_visible_face(self):
        import numpy as np
        from forge.quality.eye_reflections import glossy_pair

        # One matte face with two glossy iris patches. A shiny nose alone,
        # a third glint, or two marks on the same cheek do not identify eyes.
        y, x = np.mgrid[0:1:61j, 0:1:61j]
        p = np.column_stack([x.ravel(), y.ravel(), np.full(x.size, .95)])
        idx = np.arange(x.size).reshape(x.shape)
        a = idx[:-1, :-1].ravel()
        faces = np.vstack([np.column_stack([a, a + 1, a + 61]),
                           np.column_stack([a + 1, a + 62, a + 61])])
        centers = p[faces].mean(axis=1)
        def patches(points):
            shiny = np.zeros(len(faces), dtype=bool)
            for xx, yy in points:
                shiny |= ((centers[:, 0] - xx) / .07) ** 2 + ((centers[:, 1] - yy) / .06) ** 2 < 1
            return glossy_pair(p, faces, shiny)
        pair = patches([(.32, .73), (.66, .75)])
        self.assertIsNotNone(pair)
        self.assertTrue(np.allclose([q[:2] for q in pair], [[.32, .73], [.66, .75]], atol=.025))
        for points in ([(.32, .73)], [(.25, .67), (.44, .69)],
                       [(.32, .73), (.50, .75), (.66, .75)], [(.32, .29), (.66, .29)]):
            self.assertIsNone(patches(points))

    def test_eye_finish_accepts_extension_only_textures_and_skips_unsupported_encodings(self):
        from PIL import Image
        from forge.quality import eyes
        for extension, encoding in (("EXT_texture_webp", "WEBP"), ("KHR_texture_basisu", None)):
            doc, binary = glbstats.read_glb(self.mesh)
            image = io.BytesIO()
            if encoding:
                Image.new("RGB", (8, 8), "black").save(image, format=encoding)
            else:
                image.write(b"\xabKTX 20\xbb\r\n\x1a\n")
            blob = image.getvalue()
            doc["bufferViews"].append({"buffer": 0, "byteOffset": len(binary), "byteLength": len(blob)})
            doc["images"] = [{"bufferView": 1, "mimeType": "image/webp" if encoding else "image/ktx2"}]
            doc["textures"] = [{"extensions": {extension: {"source": 0}}}]
            doc["materials"] = [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}]
            # This all-black map has no verified pair of eyes. It must survive unchanged.
            self.assertIsNone(eyes.apply(doc, binary + blob))
            self.assertNotIn("emissiveTexture", doc["materials"][0])

    def test_generated_patchwork_gets_a_stitched_colour_map_without_changing_its_geometry(self):
        run = cli.Run("generate", self.out, seed=7)
        self.addCleanup(shutil.rmtree, run.work, True)
        run.manifest["prompt"] = "Flat red and cream patchwork blanket with a stitched hem."
        run.finish(self.mesh)
        doc, binary = glbstats.read_glb(self.out)
        prim = doc["meshes"][0]["primitives"][0]
        self.assertEqual(prim["attributes"]["POSITION"], 0)
        mat = doc["materials"][prim["material"]]["pbrMetallicRoughness"]
        tex = mat["baseColorTexture"]
        self.assertIn("TEXCOORD_" + str(tex.get("texCoord", 0)), prim["attributes"])
        image = doc["textures"][tex["index"]]["source"]
        self.assertEqual(glbstats.image_size(doc, binary, image), [256, 256])
        self.assertEqual(run.manifest["surface_pattern"], "patchwork")

    def test_flat_covers_fit_along_their_hem_after_an_oblique_bake(self):
        import numpy as np
        import trimesh
        from forge.quality import package
        sheet = trimesh.creation.box(extents=[1.0, .05, 1.6])
        sheet.apply_transform(trimesh.transformations.euler_matrix(.41, -.49, .32))
        sheet.export(self.mesh)
        for prompt, should_lay_flat in [
            ("A flat cotton bedspread with a red border.", True),
            ("A table draped with a cotton bedspread.", False),
            ("A rolled bedspread with flat ends.", False),
        ]:
            run = cli.Run("gameready", self.out)
            self.addCleanup(shutil.rmtree, run.work)
            run.manifest["prompt"] = prompt
            run.finish(self.mesh)
            fitted = package.display_surface(self.out)
            expected = [1., .05, 1.6] if should_lay_flat else sheet.extents
            np.testing.assert_allclose(fitted.extents, expected, atol=.002)
            self.assertEqual(len(fitted.faces), len(sheet.faces))

    def test_game_finish_reapplies_appearance_to_baked_draft(self):
        # Smart makes a draft first; the final bake has new UVs and materials.
        draft = self.tmp / "draft.glb"
        doc, binary = glbstats.read_glb(self.mesh)
        doc["nodes"] = [{"name": "raw", "mesh": 0}]
        doc["scenes"] = [{"nodes": [0]}]
        cli.write_glb(doc, binary, draft)
        Path(f"{draft}.manifest.json").write_text(json.dumps(
            {"command": "generate", "prompt": "red and cream patchwork blanket", "seed": 7}))
        with patch.object(cli, "gameready", return_value=self.mesh):
            self.assertEqual(cli.main(["gameready", "--mesh", str(draft), "--output", str(self.out)]), 0)
        manifest = json.loads(Path(f"{self.out}.manifest.json").read_text())
        self.assertEqual(manifest["surface_pattern"], "patchwork")
        self.assertEqual(manifest["seed"], 7)

    def test_top_view_radial_body_is_measured_with_its_mantle_above_the_arms(self):
        import numpy as np
        import trimesh
        mantle = trimesh.creation.icosphere(subdivisions=2, radius=.27)
        mantle.apply_translation([0., 0., .22])
        arms = []
        for angle in np.arange(8) * np.pi / 4:
            ray = np.array([np.cos(angle), np.sin(angle), 0.])
            arms.append(trimesh.creation.cylinder(radius=.045, segment=[.10 * ray, ray], sections=16))
        trimesh.util.concatenate([mantle, *arms]).export(self.mesh)
        run = cli.Run("gameready", self.out)
        self.addCleanup(shutil.rmtree, run.work)
        run.manifest.update(prompt="An orange octopus with eight arms", size_request={"axis":"height", "metres":.6})
        run.finish(self.mesh)
        w, h, d = run.manifest["size_m"]
        self.assertAlmostEqual(h, .6, places=4)
        self.assertGreater(min(w, d), 2.8 * h)
        result = trimesh.load(self.out, force="mesh")
        core = result.vertices[np.linalg.norm(result.vertices[:, [0, 2]], axis=1) < .2]
        self.assertGreater(core[:, 1].max(), result.bounds[0, 1] + h * .9)

    def test_flag_cloth_is_mounted_on_the_pole_at_every_display_level(self):
        import numpy as np
        import trimesh
        pole = trimesh.creation.box([.04, 1.6, .04])
        pole.apply_translation([0., .8, 0.])
        cloth = trimesh.creation.box([.64, .38, .018])
        cloth.apply_translation([.80, 1.33, .20])
        source = trimesh.Scene()
        for name in ("flag", "flag_LOD1"):
            source.add_geometry(trimesh.util.concatenate([pole, cloth]), node_name=name)
        source.export(self.mesh)
        run = cli.Run("gameready", self.out)
        self.addCleanup(shutil.rmtree, run.work, True)
        run.manifest["prompt"] = "A blue cloth flag on a straight silver flagpole."
        run.finish(self.mesh)
        result = trimesh.load(self.out, force="scene")
        for node in result.graph.nodes_geometry:
            transform, key = result.graph[node]
            mesh = result.geometry[key].copy()
            mesh.apply_transform(transform)
            parts = sorted(mesh.split(only_watertight=False), key=lambda c: -c.extents[1])
            mast, panel = parts
            np.testing.assert_allclose(mast.bounds, pole.bounds, atol=1e-5)
            np.testing.assert_allclose(panel.extents, cloth.extents, atol=1e-5)
            self.assertLessEqual(np.min(np.linalg.norm(panel.vertices[:, [0, 2]], axis=1)), .025)
            self.assertAlmostEqual(panel.centroid[1], cloth.centroid[1], places=4)

    def test_tilted_vehicle_is_levelled_only_with_the_requested_pairs_of_tyres(self):
        import numpy as np
        import trimesh
        from forge.quality import wheels

        tilt = trimesh.transformations.rotation_matrix(np.deg2rad(14), [0, 0, 1])
        for count in (4, 6, 8):
            chassis = trimesh.creation.box([1.5, .47, 3.30])
            chassis.apply_translation([0, .70, 0])
            parts = [chassis]
            for z in np.linspace(-1.28, 1.28, count // 2):
                for x in (-.88, .88):
                    tyre = trimesh.creation.cylinder(radius=.39, height=.23, sections=64)
                    tyre.apply_transform(trimesh.transformations.rotation_matrix(np.pi/2, [0, 1, 0]))
                    tyre.apply_translation([x, .39, z])
                    parts.append(tyre)
            mesh = trimesh.util.concatenate(parts)
            mesh.apply_transform(tilt)
            rotation, _ = wheels.contacts(mesh, count)
            np.testing.assert_allclose(rotation @ tilt[:3, :3], np.eye(3), atol=1e-5)
            for wrong in {4, 6, 8} - {count}:
                self.assertIsNone(wheels.contacts(mesh, wrong))
            if count == 6:
                # Axles on the same vehicle can have different track widths.
                for axle, half_width in enumerate((.58, .88, 1.19)):
                    for tyre in parts[1+2*axle:3+2*axle]:
                        tyre.apply_translation([np.sign(tyre.centroid[0]) * (half_width-.88), 0, 0])
                flared = trimesh.util.concatenate(parts)
                flared.apply_transform(tilt)
                contacts = wheels.contacts(flared, 6)
                self.assertIsNotNone(contacts, "three complete axles sit on the same tilted plane")
                np.testing.assert_allclose(contacts[0] @ tilt[:3, :3], np.eye(3), atol=1e-5)
                # Shifting both rear contacts across the vehicle breaks the axle centreline.
                for tyre in parts[-2:]:
                    tyre.apply_translation([.8, 0, 0])
                self.assertIsNone(wheels.contacts(trimesh.util.concatenate(parts), 6))
            if count == 4:
                # An imperfect generated model still stands on its real wheels.
                # Preserve the discrepancy in the asset's public manifest.
                mesh.export(self.mesh)
                run = cli.Run("gameready", self.out)
                self.addCleanup(shutil.rmtree, run.work, True)
                run.manifest["prompt"] = "A lunar rover with six chunky dark wheels"
                run.finish(self.mesh)
                report = run.manifest["wheel_contacts"]
                self.assertEqual((report["requested_count"], report["count"]), (6, 4))
                result = trimesh.load(self.out, force="mesh")
                original_size = np.ptp(mesh.vertices @ rotation.T, axis=0)
                np.testing.assert_allclose(result.extents, original_size, atol=1e-4)

                # Camera landmarks stay at the four actual contacts after the
                # delivery scales the levelled GLB to the requested height.
                run.manifest["size_request"] = {"axis": "height", "metres": 2 * original_size[1]}
                run.finish(self.mesh)
                scene = trimesh.load(self.out, force="scene")
                marks = [scene.graph[n][0][:3, 3] for n in scene.graph.nodes
                         if n.startswith("WheelContact_")]
                self.assertEqual(len(marks), 4)
                expected = np.array([[x, 0., z] for x in (-.88,.88) for z in (-1.28,1.28)]) * 2
                for point in expected:
                    self.assertLess(min(np.linalg.norm(np.asarray(marks) - point, axis=1)), .07)
                del run.manifest["size_request"]

                # Some bakes leave the chassis tilted on otherwise coplanar tyres.
                for tyre in parts[1:]:
                    x, _, z = tyre.centroid
                    tyre.apply_translation([0, .026 * x + .022 * z, 0])
                trimesh.util.concatenate(parts).export(self.mesh)
                run.finish(self.mesh)
                result = trimesh.load(self.out, force="mesh")
                p = result.vertices
                bottoms = [p[(p[:, 0] * x > .3) & (p[:, 2] * z > .4), 1].min()
                           for x in (-1, 1) for z in (-1, 1)]
                self.assertLess(np.ptp(bottoms), .004)

    def test_a_parked_van_is_sized_across_its_chassis_and_stands_on_its_tyres(self):
        import numpy as np
        import trimesh

        body = trimesh.creation.box([1.65, 1.25, 4.25])
        body.apply_translation([0, 1.05, 0])
        pieces = [body]
        for x in (-.8, .8):
            for z in (-1.5, 1.5):
                tyre = trimesh.creation.cylinder(radius=.38, height=.25, sections=32)
                tyre.apply_transform(trimesh.transformations.rotation_matrix(np.pi/2, [0, 1, 0]))
                tyre.apply_translation([x, .38, z])
                pieces.append(tyre)
        van = trimesh.util.concatenate(pieces)
        expected = van.extents.copy()
        van.apply_transform(trimesh.transformations.euler_matrix(.22, .59, -.12))
        van.export(self.mesh)
        run = cli.Run("gameready", self.out)
        self.addCleanup(shutil.rmtree, run.work, True)
        run.manifest["prompt"] = "A blue delivery van with dusty windows"
        run.finish(self.mesh)
        aligned = trimesh.load(self.out, force="mesh")
        np.testing.assert_allclose(aligned.extents, expected, atol=.04)
        points = aligned.vertices
        soles = [points[(points[:,0] * x > .65) & (points[:,2] * z > 1),1].min()
                 for x in (-1, 1) for z in (-1, 1)]
        self.assertLess(np.ptp(soles), .025)

        for prompt in ("A van door with a dent", "A bench beside a parked van",
                       "A delivery van lying on its side"):
            run.manifest["prompt"] = prompt
            run.finish(self.mesh)
            self.assertEqual(self.out.read_bytes(), self.mesh.read_bytes(), prompt)

    def invoke(self, *extra):
        with contextlib.redirect_stderr(io.StringIO()) as log:
            code = cli.main(
                [
                    "gameready",
                    "--mesh",
                    str(self.mesh),
                    "--output",
                    str(self.out),
                    *extra,
                ]
            )
        return code, [json.loads(s) for s in log.getvalue().splitlines()]

    def test_accepts_renamed_legacy_package_without_receipt_or_blender(self):
        with patch.object(
            cli.Run, "run", side_effect=AssertionError("attempted a second bake")
        ):
            rc, events = self.invoke()
        self.assertEqual(rc, 0, events)
        self.assertEqual(self.mesh.read_bytes(), self.out.read_bytes())
        receipt = json.loads(Path(str(self.out) + ".manifest.json").read_text())
        self.assertTrue(receipt["reused_gameready"])
        self.assertEqual(receipt["size_m"], [4, 2, 1])

    def test_dimensions_include_display_transform_exclude_collision_and_scale_payload_once(
        self,
    ):
        doc, data = glbstats.read_glb(self.mesh)
        doc["nodes"][0]["scale"] = [1, 2, 1]
        doc["nodes"][-1]["translation"] = [100, 0, 0]
        cli.write_glb(doc, data, self.mesh)
        rc, events = self.invoke("--height-m", "1.8")
        self.assertEqual(rc, 0, events)
        out, buf = glbstats.read_glb(self.out)
        receipt = json.loads(Path(str(self.out) + ".manifest.json").read_text())
        self.assertEqual(receipt["size_m"], [1.8, 1.8, 0.45])
        self.assertAlmostEqual(struct.unpack_from("<f", buf, 12)[0], 1.8)
        self.assertAlmostEqual(out["nodes"][-1]["translation"][0], 45)
        self.assertEqual(out["nodes"][0]["scale"], [1, 2, 1])

    def test_longest_side_and_invalid_sizes(self):
        rc, events = self.invoke("--longest-side-m", ".15")
        self.assertEqual(rc, 0, events)
        self.assertEqual(
            json.loads(Path(str(self.out) + ".manifest.json").read_text())["size_m"],
            [0.15, 0.075, 0.0375],
        )
        for args in [
            ("--height-m", "0"),
            ("--height-m", "nan"),
            ("--longest-side-m", "-1"),
            ("--height-m", "1", "--longest-side-m", "2"),
        ]:
            for stage in ("generate", "gameready"):
                p = subprocess.run(
                    [cli.ROOT / "bin/forge", stage, "--output", self.out, *args],
                    check=False,
                    capture_output=True,
                )
                self.assertNotEqual(p.returncode, 0)
                self.assertIn("-m", json.loads(p.stderr.splitlines()[-1])["error"])

    def test_error_survives_trailing_blender_stdout(self):
        run = cli.Run("gameready", self.out)
        self.addCleanup(shutil.rmtree, run.work)
        child = "import sys; print('{\"error\":\"material is missing\"}',file=sys.stderr); print('Info: Baking map saved'); sys.exit(1)"
        with self.assertRaisesRegex(cli.Failed, "material is missing"):
            run.run("gameready", [sys.executable, "-c", child], heavy=False)

    def test_each_stage_failure_has_final_json_error(self):
        for stage in ("generate", "gameready", "rig", "animate", "library"):
            with self.subTest(stage=stage):
                result = subprocess.run(
                    [cli.ROOT / "bin/forge", stage, "--output", self.out],
                    check=False,
                    capture_output=True,
                    text=True,
                )
                self.assertNotEqual(result.returncode, 0)
                error = json.loads(result.stderr.splitlines()[-1])
                self.assertIsInstance(error["error"], str)
                self.assertTrue(error["error"])

    def test_sidecar_recognises_an_older_custom_package_only_when_geometry_matches(
        self,
    ):
        doc, binary = glbstats.read_glb(self.mesh)
        for i, node in enumerate(doc["nodes"]):
            node["name"] = f"part{i}"
        cli.write_glb(doc, binary, self.mesh)
        sidecar = Path(str(self.mesh) + ".manifest.json")

        def receipt(count):
            sidecar.write_text(
                json.dumps(
                    {
                        "stage": "gameready",
                        "lods": [{"node": "part0", "triangles": count}],
                    }
                )
            )

        with patch.object(cli.Run, "run", side_effect=ValueError("needs a bake")):
            receipt(1)
            rc, events = self.invoke()
            self.assertEqual(rc, 0, events)
            receipt(7)
            rc, events = self.invoke()
            self.assertEqual(rc, 1)
            self.assertIn("needs a bake", events[-1]["error"])

    def test_prepared_marker_survives_missing_sidecar_and_renamed_nodes(self):
        doc, binary = glbstats.read_glb(self.mesh)
        for i, node in enumerate(doc["nodes"]):
            node["name"] = f"piece{i}"
        doc["asset"]["extras"] = {"otto_forge_game_ready": True}
        cli.write_glb(doc, binary, self.mesh)
        rc, events = self.invoke()
        self.assertEqual(rc, 0, events)
        self.assertEqual(self.mesh.read_bytes(), self.out.read_bytes())

    def test_sigterm_during_blender_bake_ends_in_json(self):
        self.interrupt_bake(kill_worker=False)

    def test_killed_blender_during_bake_ends_in_json(self):
        self.interrupt_bake(kill_worker=True)

    def interrupt_bake(self, *, kill_worker):
        model = self.tmp / "fixture.obj"
        model.write_text(
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 3 2\nf 1 2 4\nf 2 3 4\nf 3 1 4\n"
        )
        env = dict(os.environ, FORGE_KEEP_WORK="1")
        with subprocess.Popen(
            [
                cli.ROOT / "bin/forge",
                "gameready",
                "--mesh",
                model,
                "--output",
                self.out,
            ],
            stderr=subprocess.PIPE,
            env=env,
        ) as proc:
            root = cli.ROOT / "out/work"
            deadline = time.monotonic() + 600
            try:
                while proc.poll() is None and time.monotonic() < deadline:
                    log = next(root.glob(f"asset-*-{proc.pid}/gameready.log"), None)
                    if log and b"Baking normal map" in log.read_bytes():
                        break
                    time.sleep(0.02)
                else:
                    self.fail("Blender never entered the normal bake")
                if kill_worker:
                    processes = subprocess.check_output(
                        ["ps", "-axo", "pid=,ppid=,comm="], text=True
                    )
                    descendants = {proc.pid}
                    rows = [
                        line.split(maxsplit=2)
                        for line in processes.splitlines()
                        if line.strip()
                    ]
                    while True:
                        children = {
                            int(pid)
                            for pid, parent, _ in rows
                            if int(parent) in descendants
                        }
                        if children <= descendants:
                            break
                        descendants |= children
                    (blender,) = [
                        int(pid)
                        for pid, _, command in rows
                        if int(pid) in descendants and Path(command).name == "Blender"
                    ]
                    os.kill(blender, signal.SIGKILL)
                else:
                    proc.send_signal(signal.SIGTERM)
                err = proc.communicate(timeout=15)[1]
                self.assertNotEqual(proc.returncode, 0)
                self.assertIn(
                    "blender exited" if kill_worker else "cancelled",
                    json.loads(err.splitlines()[-1])["error"],
                )
                self.assertFalse(self.out.exists())
            finally:
                if proc.poll() is None:
                    proc.terminate()
                    proc.wait(15)
                for folder in root.glob(f"asset-*-{proc.pid}"):
                    shutil.rmtree(folder)
