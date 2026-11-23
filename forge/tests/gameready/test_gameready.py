"""Gameready stage tests. Run with one command (Blender runs them; they call bin/forge-gameready as Otto would):
  tools/heavy.sh tests/gameready/run.sh
Builds procedural inputs, runs the stage, then re-imports the real output files and checks them.
"""
import json
import os
import signal
import subprocess
import sys
import time
import unittest

import bmesh
import bpy
import numpy as np
from mathutils.bvhtree import BVHTree

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
STAGE = os.path.join(ROOT, "bin", "forge-gameready")
WORK = os.path.join(ROOT, "out", "gameready-tests")
INPUTS = os.path.join(WORK, "inputs")
BLENDER = bpy.app.binary_path
CASES = {  # name: (input, extra args); the output GLB is WORK/<name>/<name>.glb
    "sculpt": ("sculpt.glb", ["--tris", "15000", "--texture", "2048", "--lods", "3", "--collision", "--pack"]),
    "quads": ("sculpt.glb", ["--quads", "--tris", "8000", "--texture", "1024", "--lods", "1", "--pack"]),
    "hollow": ("hollow.glb", ["--tris", "5000", "--texture", "1024", "--lods", "2", "--collision"]),
    "multipart": ("multipart.glb", ["--tris", "8000", "--texture", "1024", "--lods", "2", "--collision"]),
    "rigged": ("rigged.glb", []),
}
# A real Pixal3D mesh (935k triangles, double-walled, 1,060 non-manifold edges after cleaning). Too big to keep
# in tests/, so it is referenced in place and its tests skip when it is absent.
KNIGHT = os.path.join(ROOT, "out", "p0", "character", "raw.glb")
if os.path.exists(KNIGHT):
    CASES["knight"] = (KNIGHT, ["--tris", "15000", "--texture", "2048", "--lods", "3", "--collision"])
    CASES["knight_quads"] = (KNIGHT, ["--quads", "--tris", "15000", "--texture", "1024", "--lods", "1"])
# The text-generated knight's Finish mesh (14,596 triangles, 2K texture): clean() used to turn 1,611 of its faces
# inside out, which baked into black marks on the face and boots.
TEXT_KNIGHT = os.path.join(ROOT, "out", "text_to_asset", "character", "finished", "finished.glb")
if os.path.isfile(TEXT_KNIGHT):
    CASES["text_knight"] = (TEXT_KNIGHT, ["--tris", "15000", "--texture", "2048", "--lods", "1"])
# The text knight's raw Pixal3D mesh (908k triangles, 31,553 non-manifold edges after cleaning): its voxel repair
# used to keep 21 non-manifold edges, so it was discarded and decimation stalled at 215k triangles (xatlas then hung).
TEXT_RAW = os.path.join(ROOT, "out", "closeup-gen", "baseline", "raw.glb")
if os.path.isfile(TEXT_RAW):
    CASES["text_raw"] = (TEXT_RAW, ["--tris", "15000", "--lods", "1", "--detail", "head,hands"])
CHEST = os.path.join(ROOT, "out", "p0", "prop", "finished.glb")
if os.path.isfile(CHEST):
    CASES["chest"] = (CHEST, ["--tris", "5000", "--lods", "3", "--collision"])
RUNS = {}


def out_glb(name):
    return os.path.join(WORK, name, name + ".glb")


def run_stage(name):
    src, extra = CASES[name]
    extra = [a if a != "--pack" else "--pack=" + os.path.join(WORK, name, "pack") for a in extra]
    t = time.perf_counter()
    p = subprocess.run([STAGE, os.path.join(INPUTS, src), "--output", out_glb(name), *extra],
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    lines = [ln for ln in p.stderr.splitlines() if ln.strip()]
    print(f"[test] {name}: exit {p.returncode} in {time.perf_counter() - t:.1f} s; last: {lines[-1] if lines else ''}")
    return p.returncode, lines


def setUpModule():
    subprocess.run(["rm", "-rf", WORK], check=True)
    subprocess.run([BLENDER, "-b", "--factory-startup", "--python-exit-code", "1", "--python",
                    os.path.join(os.path.dirname(__file__), "make_inputs.py"), "--", INPUTS],
                   check=True, stdout=subprocess.DEVNULL)
    for name in CASES:
        RUNS[name] = run_stage(name)


# ---------- helpers on re-imported files ----------

def load(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    if path.endswith(".fbx"):
        assert bpy.ops.import_scene.fbx(filepath=path) == {"FINISHED"}
    else:
        assert bpy.ops.import_scene.gltf(filepath=path) == {"FINISHED"}
    return {o.name: o for o in bpy.data.objects if o.type == "MESH"}


def tris(obj):
    return sum(len(p.vertices) - 2 for p in obj.data.polygons)


def mesh_arrays(obj):
    """World positions, loop-triangle vertex indices and per-corner UVs of the loop triangles."""
    me = obj.data
    me.calc_loop_triangles()
    co = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", co)
    m = np.array(obj.matrix_world)
    co = co.reshape(-1, 3) @ m[:3, :3].T + m[:3, 3]
    tv = np.empty(len(me.loop_triangles) * 3, np.int64)
    me.loop_triangles.foreach_get("vertices", tv)
    tl = np.empty(len(me.loop_triangles) * 3, np.int64)
    me.loop_triangles.foreach_get("loops", tl)
    uv = None
    if me.uv_layers:
        uv = np.empty(len(me.loops) * 2)
        me.uv_layers[0].data.foreach_get("uv", uv)
        uv = uv.reshape(-1, 2)[tl.reshape(-1, 3)]
    return co, tv.reshape(-1, 3), uv


def rasterize(tri_uv, res):
    """Per-pixel count of UV triangles covering the pixel centre (overlap -> count > 1)."""
    cover = np.zeros((res, res), np.int32)
    for a, b, c in tri_uv * res:
        x0, y0 = np.floor(np.minimum(np.minimum(a, b), c)).astype(int)
        x1, y1 = np.ceil(np.maximum(np.maximum(a, b), c)).astype(int)
        x0, y0, x1, y1 = max(x0, 0), max(y0, 0), min(x1, res), min(y1, res)
        if x1 <= x0 or y1 <= y0:
            continue
        py, px = np.mgrid[y0:y1, x0:x1] + 0.5
        d = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        if d == 0:
            continue
        w1 = ((px - a[0]) * (c[1] - a[1]) - (py - a[1]) * (c[0] - a[0])) / d
        w2 = ((b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])) / d
        inside = (w1 > 0) & (w2 > 0) & (w1 + w2 < 1)
        cover[y0:y1, x0:x1] += inside
    return cover


def material_images(obj):
    """{'basecolor'|'normal'|'orm': Image} from the LOD0 material's node links."""
    nt = obj.data.materials[0].node_tree
    bsdf = next(n for n in nt.nodes if n.type == "BSDF_PRINCIPLED")
    found = {}

    def upstream_image(sock):
        while sock.is_linked:
            node = sock.links[0].from_node
            if node.type == "TEX_IMAGE":
                return node.image
            sock = next((i for i in node.inputs if i.is_linked), None)
            if sock is None:
                return None
        return None

    for key, inp in (("basecolor", "Base Color"), ("normal", "Normal"), ("orm", "Roughness")):
        img = upstream_image(bsdf.inputs[inp])
        if img is not None:
            found[key] = img
    return found, bsdf


def pixels(img):
    w, h = img.size
    px = np.empty(w * h * 4, np.float32)
    img.pixels.foreach_get(px)
    return px.reshape(h, w, 4)


def sample(px, uv):
    h, w, _ = px.shape
    x = np.clip((uv[:, 0] * w).astype(int), 0, w - 1)
    y = np.clip((uv[:, 1] * h).astype(int), 0, h - 1)
    return px[y, x]


def surface_points(co, tv, uv, n, rng):
    """n random points on the surface: positions and interpolated UVs."""
    t = rng.integers(0, len(tv), n)
    r = rng.random((n, 2))
    flip = r.sum(1) > 1
    r[flip] = 1 - r[flip]
    w = np.stack([1 - r.sum(1), r[:, 0], r[:, 1]], 1)
    pos = np.einsum("ij,ijk->ik", w, co[tv[t]])
    return pos, np.einsum("ij,ijk->ik", w, uv[t])


def source_values(src_obj, src_px, points):
    """Texture values of the source high-poly at the surface point nearest to each given point."""
    co, tv, uv = mesh_arrays(src_obj)
    tree = BVHTree.FromPolygons(co.tolist(), tv.tolist())
    out_uv = []
    for p in points:
        loc, _, i, _ = tree.find_nearest(p)
        a, b, c = co[tv[i]]
        v0, v1, v2 = b - a, c - a, np.array(loc) - a
        d00, d01, d11, d20, d21 = v0 @ v0, v0 @ v1, v1 @ v1, v2 @ v0, v2 @ v1
        den = d00 * d11 - d01 * d01
        wb, wc = (d11 * d20 - d01 * d21) / den, (d00 * d21 - d01 * d20) / den
        out_uv.append((1 - wb - wc) * uv[i][0] + wb * uv[i][1] + wc * uv[i][2])
    return sample(src_px, np.array(out_uv))


def manifest(name):
    with open(out_glb(name) + ".manifest.json") as fh:
        return json.load(fh)


# ---------- tests ----------

class Contract(unittest.TestCase):
    def test_success_progress_lines_are_json_and_reach_100(self):
        for name in ("sculpt", "quads", "hollow", "multipart"):
            rc, lines = RUNS[name]
            self.assertEqual(rc, 0, f"{name}: {lines[-3:]}")
            events = [json.loads(ln) for ln in lines]  # every stderr line is one JSON object
            pcts = [e["pct"] for e in events]
            self.assertTrue(all(e["stage"] == "gameready" for e in events))
            self.assertEqual(pcts, sorted(pcts), name)
            self.assertEqual(pcts[-1], 100, name)

    def test_manifest_fields(self):
        m = manifest("sculpt")
        for key in ("tools", "licences", "timings", "triangles", "lods", "texture_maps"):
            self.assertIn(key, m)
        self.assertEqual(m["tools"]["coacd"], "1.0.14")
        self.assertEqual(m["licences"]["coacd"], "MIT")
        for k in ("import", "clean", "lowpoly", "uv", "bake", "lods", "collision", "export", "total"):
            self.assertGreater(m["timings"][k], 0, k)
        self.assertEqual(len(m["triangles"]), 4)

    def test_rigged_input_is_refused(self):
        rc, lines = RUNS["rigged"]
        self.assertNotEqual(rc, 0)
        self.assertIn("skinned", json.loads(lines[-1])["error"])
        self.assertFalse(os.path.exists(out_glb("rigged")))

    def test_sigterm_kills_blender_and_reports(self):
        out = os.path.join(WORK, "cancel", "cancel.glb")
        p = subprocess.Popen([STAGE, os.path.join(INPUTS, "sculpt.glb"), "--output", out],
                             stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
        p.stderr.readline()  # first progress line: Blender is running the script
        time.sleep(2)
        p.send_signal(signal.SIGTERM)
        rc = p.wait(timeout=15)
        rest = [ln for ln in p.stderr.read().splitlines() if ln.strip()]
        p.stderr.close()
        self.assertNotEqual(rc, 0)
        self.assertEqual(json.loads(rest[-1]), {"error": "cancelled"})
        time.sleep(0.5)
        left = subprocess.run(["pgrep", "-f", out], capture_output=True, text=True).stdout.strip()
        self.assertEqual(left, "", "a Blender child survived SIGTERM")
        self.assertFalse(os.path.exists(out + ".manifest.json"))


class Checks(unittest.TestCase):
    """Shared checks on re-imported outputs (no tests of its own)."""

    def check_levels(self, name, budget, lods, hulls):
        objs = load(out_glb(name))
        expected = [name] + [f"{name}_LOD{i}" for i in range(1, lods + 1)]
        for n in expected:
            self.assertIn(n, objs)
        ucx = sorted(n for n in objs if n.startswith(f"UCX_{name}_"))
        self.assertEqual(bool(ucx), hulls)
        self.assertEqual(set(objs), set(expected) | set(ucx))
        counts = [tris(objs[n]) for n in expected]
        self.assertLessEqual(abs(counts[0] - budget), 0.05 * budget, counts)
        for a, b in zip(counts, counts[1:]):
            self.assertLess(b, a)
            self.assertAlmostEqual(b / a, 0.5, delta=0.05)
        mats = {objs[n].data.materials[0].name for n in expected}
        self.assertEqual(len(mats), 1, "LODs must share LOD0's material and textures")
        return objs, counts

    def check_uvs(self, obj, res=2048, max_overlap=1e-5, min_cover=0.25):
        _, _, uv = mesh_arrays(obj)
        self.assertGreaterEqual(uv.min(), -1e-6)
        self.assertLessEqual(uv.max(), 1 + 1e-6)
        cover = rasterize(uv, res)
        covered = (cover > 0).sum()
        self.assertGreater(covered, min_cover * res * res, "UV islands use too little of the atlas")
        self.assertLessEqual((cover > 1).sum(), max_overlap * covered, "overlapping UV islands")
        return cover[::res // 1024, ::res // 1024] > 0 if res > 1024 else cover > 0

    def check_maps(self, obj, size, want_orm):
        images, bsdf = material_images(obj)
        self.assertEqual(set(images), {"basecolor", "normal", "orm"} if want_orm else {"basecolor", "normal"})
        for img in images.values():
            self.assertEqual(tuple(img.size), (size, size), img.name)
        self.assertEqual(len([i for i in bpy.data.images if i.size[0]]), len(images), "textures duplicated")
        return images, bsdf

    def normal_deviation(self, img, mask):
        px = pixels(img)
        h = px.shape[0]
        m = mask if mask.shape[0] == h else np.kron(mask, np.ones((h // mask.shape[0],) * 2, bool))
        n = px[..., :3][m] * 2 - 1
        n /= np.linalg.norm(n, axis=1, keepdims=True)
        self.assertGreater((n[:, 2] > 0).mean(), 0.99, "not a tangent-space normal map (blue must dominate)")
        return np.degrees(np.arccos(np.clip(n[:, 2], -1, 1)))

    def colour_errors(self, name, source, n=3000):
        """Max-channel error between LOD0's baked base colour / ORM and the source textures at n random
        surface points (the source value is read at the nearest point of the source mesh)."""
        objs = load(out_glb(name))
        images, _ = material_images(objs[name])
        co, tv, uv = mesh_arrays(objs[name])
        pos, luv = surface_points(co, tv, uv, n, np.random.default_rng(0))
        baked_c = sample(pixels(images["basecolor"]), luv)[:, :3]
        baked_orm = sample(pixels(images["orm"]), luv)[:, :3] if "orm" in images else None
        src_obj = max(load(source).values(), key=lambda o: len(o.data.polygons))
        src_imgs, _ = material_images(src_obj)
        src_c = source_values(src_obj, pixels(src_imgs["basecolor"]), pos)[:, :3]
        src_mr = source_values(src_obj, pixels(src_imgs["orm"]), pos)[:, :3] if "orm" in src_imgs else None
        return np.abs(baked_c - src_c).max(1), baked_orm, src_mr

    def open_edges(self, obj):
        """Non-manifold edges after welding the vertices the GLB split along UV seams."""
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-6)
        bad = sum(not e.is_manifold for e in bm.edges)
        bm.free()
        return bad


class SingleGlb(Checks):

    def test_sculpt(self):
        objs, _ = self.check_levels("sculpt", 15000, 3, True)
        mask = self.check_uvs(objs["sculpt"])
        for i in (1, 2, 3):  # decimation trims island borders, so LODs cover less of the shared atlas
            self.check_uvs(objs[f"sculpt_LOD{i}"], 1024, max_overlap=1e-3, min_cover=0.1)
        images, _ = self.check_maps(objs["sculpt"], 2048, True)
        dev = self.normal_deviation(images["normal"], mask)
        self.assertGreater((dev > 5).mean(), 0.25, "normal map is (nearly) flat: the bake missed the detail")

        # Baked maps vs the source textures at the same surface points (catches wrong UVs, misses, swaps).
        err, baked_orm, src_mr = self.colour_errors("sculpt", os.path.join(INPUTS, "sculpt.glb"))
        self.assertLess(np.median(err), 0.05, "base colour does not match the source texture")
        self.assertLess(np.percentile(err, 80), 0.15)
        self.assertLess(np.median(np.abs(baked_orm[:, 1] - src_mr[:, 1])), 0.05, "roughness bake")
        self.assertGreater((np.abs(baked_orm[:, 2] - src_mr[:, 2]) < 0.2).mean(), 0.85, "metallic bake")
        self.assertTrue(np.allclose(baked_orm[:, 0], 1.0), "ORM red (occlusion) should be 1")

    def test_sculpt_collision_hulls_convex_and_enclosing(self):
        objs = load(out_glb("sculpt"))
        hulls = [o for n, o in objs.items() if n.startswith("UCX_sculpt_")]
        self.assertTrue(1 <= len(hulls) <= 8, len(hulls))
        co, _, _ = mesh_arrays(objs["sculpt"])
        diag = np.linalg.norm(co.max(0) - co.min(0))
        inside_any = np.zeros(len(co), bool)
        for h in hulls:
            hv, ht, _ = mesh_arrays(h)
            a, b, c = hv[ht[:, 0]], hv[ht[:, 1]], hv[ht[:, 2]]
            nrm = np.cross(b - a, c - a)
            nrm /= np.linalg.norm(nrm, axis=1, keepdims=True) + 1e-30
            centre = hv.mean(0)
            nrm *= np.sign(np.einsum("ij,ij->i", nrm, a - centre))[:, None]  # outward
            dist = np.einsum("fk,fvk->fv", nrm, hv[None, :, :] - a[:, None, :])
            self.assertLess(dist.max(), 1e-4 * diag, f"{h.name} is not convex")
            d = np.einsum("fk,fvk->fv", nrm, co[None, :, :] - a[:, None, :])
            inside_any |= (d <= 0.01 * diag).all(0)
        self.assertGreater(inside_any.mean(), 0.99, "collision hulls do not enclose the mesh")

    def test_quads(self):
        objs, _ = self.check_levels("quads", 8000, 1, False)
        # A quad whose UVs are concave can fold a few pixels once the GLB triangulates it (measured ~1e-4).
        self.check_uvs(objs["quads"], 2048, max_overlap=3e-4)
        fbx = load(os.path.join(WORK, "quads", "pack", "quads_LOD0.fbx"))
        lod0 = fbx["quads"]
        share = sum(len(p.vertices) == 4 for p in lod0.data.polygons) / len(lod0.data.polygons)
        self.assertGreater(share, 0.95, "--quads output is not quad-dominant")
        self.assertLessEqual(abs(len(lod0.data.polygons) - 4000), 0.05 * 4000)

    def test_pack_files(self):
        pack = os.path.join(WORK, "sculpt", "pack")
        for i in range(4):
            for ext in ("glb", "fbx"):
                self.assertTrue(os.path.getsize(os.path.join(pack, f"sculpt_LOD{i}.{ext}")) > 0)
        for kind in ("basecolor", "normal", "orm"):
            self.assertTrue(os.path.exists(os.path.join(pack, "textures", f"sculpt_{kind}.png")))
        fbx = load(os.path.join(pack, "sculpt_LOD0.fbx"))
        self.assertTrue(any(n.startswith("UCX_sculpt_") for n in fbx), "LOD0 FBX lacks UCX_ hulls")

    def test_hollow_is_repaired(self):
        m = manifest("hollow")["clean"]
        self.assertGreater(m["merged_vertices"], 0)
        self.assertGreaterEqual(m["loose_parts_removed"], 1)
        self.assertGreaterEqual(m["holes_filled"], 3)
        self.assertLess(m["after"]["non_manifold_edges"], m["before"]["non_manifold_edges"])
        # A zero-thickness open tube voxelises into a slab that is still non-manifold: the repair is discarded.
        self.assertFalse(manifest("hollow")["repair"]["used"])
        objs, _ = self.check_levels("hollow", 5000, 2, True)
        # The stage keeps the input winding, so the tube's flipped patches stay flipped and xatlas cuts a chart
        # per patch (929 instead of 4): about 21% of the atlas instead of 70%.
        mask = self.check_uvs(objs["hollow"], 1024, min_cover=0.15)
        images, bsdf = self.check_maps(objs["hollow"], 1024, False)  # constant roughness/metallic: no ORM
        self.assertAlmostEqual(bsdf.inputs["Roughness"].default_value, 0.5, places=2)
        self.assertAlmostEqual(bsdf.inputs["Metallic"].default_value, 0.0, places=2)
        self.assertGreater(pixels(images["basecolor"])[..., :3][mask].std(), 0.1, "base colour bake is blank")

    def test_multipart_joined_with_all_colours(self):
        self.assertEqual(manifest("multipart")["input_parts"], 6)
        objs, _ = self.check_levels("multipart", 8000, 2, True)
        mask = self.check_uvs(objs["multipart"], 1024)
        images, _ = self.check_maps(objs["multipart"], 1024, True)  # roughness/metallic differ per part
        c = pixels(images["basecolor"])[..., :3][mask]
        red = (c[:, 0] > 0.5) & (c[:, 1] < 0.3) & (c[:, 2] < 0.3)
        blue = (c[:, 2] > 0.5) & (c[:, 0] < 0.4)
        dark = c.max(1) < 0.2
        for label, sel, share in (("red body", red, 0.2), ("blue dome", blue, 0.05), ("black wheels", dark, 0.05)):
            self.assertGreater(sel.mean(), share, label)
        metal = pixels(images["orm"])[..., 2][mask]
        self.assertGreater(((metal > 0.7) & (metal < 0.9)).mean(), 0.2, "body metallic 0.8 missing")
        self.assertGreater((metal < 0.1).mean(), 0.1, "non-metal parts missing")



@unittest.skipUnless(os.path.isfile(CHEST), f"real Finish chest absent ({CHEST})")
class RealChest(Checks):
    def test_manifold_lods_and_lid(self):
        rc, lines = RUNS["chest"]
        self.assertEqual(rc, 0, lines[-2:])
        objs, _ = self.check_levels("chest", 5000, 3, True)
        for name, ob in objs.items():
            if not name.startswith("UCX_"):
                self.assertEqual(self.open_edges(ob), 0, name)

        # Evaluate the same central span of wood between the two bands in source and output.
        # The old bake placed long, dark triangles across this smooth region.
        def lid(path, label):
            strip = os.path.join(WORK, label + ".png")
            subprocess.run([BLENDER, "-b", "--factory-startup", "--python-exit-code", "1",
                            "-P", os.path.join(ROOT, "tools/render_views.py"), "--", path, strip],
                           check=True, stdout=subprocess.DEVNULL)
            img = bpy.data.images.load(strip[:-4] + "_front.png", check_existing=False)
            try:
                rgba = np.empty(len(img.pixels), np.float32)
                img.pixels.foreach_get(rgba)
                rgb = rgba.reshape(img.size[1], img.size[0], 4)[::-1, :, :3]
                # Compare display-encoded values, on the same 0..255 scale as the PNGs.
                rgb = np.where(rgb < .0031308, 12.92*rgb, 1.055*rgb**(1/2.4)-.055)
                return rgb[235:300, 153:267] * 255
            finally:
                bpy.data.images.remove(img)

        difference = np.abs(lid(out_glb("chest"), "lod0") - lid(CHEST, "source")).mean(axis=2)
        self.assertLess(np.percentile(difference, 95), 15, "lid departs from the source finish")


@unittest.skipUnless(os.path.isfile(TEXT_KNIGHT), f"text knight's Finish mesh absent ({TEXT_KNIGHT})")
class TextKnightCloseups(Checks):
    def test_head_and_feet_keep_source_winding_and_colour(self):
        """On the head (top 13% of the height) and feet (bottom 10%): LOD0 faces face the same way as the source
        surface under them (an inside-out face renders black and is culled in engines), and no baked texel is
        near-black where the source colour is not."""
        rc, lines = RUNS["text_knight"]
        self.assertEqual(rc, 0, lines[-2:])
        src = max(load(TEXT_KNIGHT).values(), key=lambda o: len(o.data.polygons))
        sco, stv, _ = mesh_arrays(src)
        snrm = np.cross(sco[stv[:, 1]] - sco[stv[:, 0]], sco[stv[:, 2]] - sco[stv[:, 0]])
        tree = BVHTree.FromPolygons(sco.tolist(), stv.tolist())
        objs = load(out_glb("text_knight"))
        co, tv, uv = mesh_arrays(objs["text_knight"])
        z0, z1 = co[:, 2].min(), co[:, 2].max()
        centre = co[tv].mean(1)
        sel = (centre[:, 2] > z1 - .13 * (z1 - z0)) | (centre[:, 2] < z0 + .10 * (z1 - z0))
        nrm = np.cross(co[tv[:, 1]] - co[tv[:, 0]], co[tv[:, 2]] - co[tv[:, 0]])
        inverted = [snrm[tree.find_nearest(c)[2]] @ n < 0 for c, n in zip(centre[sel], nrm[sel])]
        self.assertLess(np.mean(inverted), 0.005, f"{sum(inverted)} of {len(inverted)} head/feet faces inside out")

        pos, luv = surface_points(co[:, :], tv[sel], uv[sel], 3000, np.random.default_rng(0))
        baked = sample(pixels(material_images(objs["text_knight"])[0]["basecolor"]), luv)[:, :3]
        src = max(load(TEXT_KNIGHT).values(), key=lambda o: len(o.data.polygons))
        source = source_values(src, pixels(material_images(src)[0]["basecolor"]), pos)[:, :3]
        black = (baked.max(1) < .08) & (source.max(1) > .2)
        self.assertLess(black.mean(), 0.005, f"{black.sum()} near-black texels where the source is not dark")


@unittest.skipUnless(os.path.isfile(TEXT_RAW), f"text knight's raw mesh absent ({TEXT_RAW})")
class TextRawDetail(Checks):
    def test_repair_closes_and_detail_regions_get_their_share(self):
        rc, lines = RUNS["text_raw"]
        self.assertEqual(rc, 0, lines[-2:])
        m = manifest("text_raw")
        self.assertTrue(m["repair"]["used"], "voxel repair discarded: decimation stalls on the open raw mesh")
        self.assertLessEqual(abs(m["triangles"][0] - 15000), 0.05 * 15000)
        d = m["lowpoly"]["detail"]
        self.assertAlmostEqual(d["region_triangles"], d["region_triangles_target"], delta=0.1 * d["region_triangles_target"])
        lod0 = m["detail"]["lod0"]
        for region in ("head", "left_hand", "right_hand"):
            self.assertGreater(lod0[region]["texels_per_cm2"], 3 * lod0["all"]["texels_per_cm2"], region)
            self.assertGreater(lod0[region]["triangles_per_cm2"], 3 * lod0["all"]["triangles_per_cm2"], region)
        objs = load(out_glb("text_raw"))
        self.check_uvs(objs["text_raw"], 2048, max_overlap=3e-4, min_cover=0.15)  # Blender's packer: ~1e-4


@unittest.skipUnless(os.path.exists(KNIGHT), f"real AI mesh absent ({KNIGHT}): run the generate stage first")
class RealKnight(Checks):
    """The Pixal3D knight stays open/non-manifold after cleaning, so the stage must repair it."""

    def test_triangles_closed_lods_halve_colour_kept(self):
        rc, lines = RUNS["knight"]
        self.assertEqual(rc, 0, lines[-2:])
        m = manifest("knight")
        self.assertGreater(m["clean"]["after"]["non_manifold_edges"], 0, "input no longer needs repair?")
        self.assertTrue(m["repair"]["used"])
        self.assertEqual(m["repair"]["non_manifold_edges"], 0)
        objs, _ = self.check_levels("knight", 15000, 3, True)  # LODs within 10% of half the previous level
        for n in ("knight", "knight_LOD1", "knight_LOD2", "knight_LOD3"):
            self.assertEqual(self.open_edges(objs[n]), 0, f"{n} is not closed manifold")
        self.check_uvs(objs["knight"])
        err, _, _ = self.colour_errors("knight", KNIGHT)
        self.assertLess(np.median(err), 0.06, "baked colour does not match the source")
        self.assertLess(np.percentile(err, 80), 0.2)

    def test_quads_succeed_on_repaired_mesh(self):
        rc, lines = RUNS["knight_quads"]
        self.assertEqual(rc, 0, lines[-2:])
        self.assertGreater(manifest("knight_quads")["lowpoly"]["quad_share"], 0.95)
        objs, _ = self.check_levels("knight_quads", 15000, 1, False)
        self.assertEqual(self.open_edges(objs["knight_quads"]), 0)
        self.check_uvs(objs["knight_quads"], 2048, max_overlap=3e-4)


if __name__ == "__main__":
    result = unittest.main(argv=["gameready"], exit=False, verbosity=2).result
    sys.exit(0 if result.wasSuccessful() else 1)
