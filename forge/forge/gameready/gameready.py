"""Otto Forge, gameready stage: raw AI high-poly mesh -> game-ready asset (headless Blender).

Run through bin/forge-gameready, which calls:
  Blender -b --factory-startup --python gameready.py -- INPUT --output OUT.glb [options]

Steps: import + join + clean -> low-poly (collapse decimate, or QuadriFlow with --quads) -> Smart UV
+ pack -> Cycles bake high -> low (base colour, tangent normal, roughness/metallic when the source has
them) -> LODs -> CoACD collision hulls -> ONE GLB holding <name> (LOD0), <name>_LOD1.. and UCX_<name>_NN,
plus <output>.manifest.json. --pack DIR also writes per-level GLB + FBX files and the PNG maps.
Progress: one JSON object per line on the fd in FORGE_PROGRESS_FD (stderr when unset).
"""
import argparse
from collections import Counter
import json
import math
import os
import sys
import time
import traceback

import bmesh
import bpy
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.append(os.path.join(HERE, ".venv"))  # pip --target dir with coacd + xatlas (see bin/forge-gameready)

COACD_VERSION = "1.0.14"
XATLAS_VERSION = "0.0.11"
LOOSE_PART_FRACTION = 0.01  # drop disconnected parts smaller than 1% of the asset's bounding-box diagonal
HOLE_MAX_SIDES = 32         # fill boundary loops with at most this many edges
REPAIR_VOXEL_DIVS = 1000    # watertight repair voxel = 1/1000 of the bounding-box diagonal
TIMINGS = {}
_fd = os.environ.get("FORGE_PROGRESS_FD")
PROGRESS = os.fdopen(int(_fd), "w", buffering=1) if _fd else sys.stderr


def progress(pct, message):
    PROGRESS.write(json.dumps({"stage": "gameready", "pct": pct, "message": message}) + "\n")
    PROGRESS.flush()


class Fail(Exception):
    pass


class step:
    def __init__(self, name, pct, message):
        self.name, self.pct, self.message = name, pct, message

    def __enter__(self):
        self.t = time.perf_counter()
        progress(self.pct, self.message)

    def __exit__(self, *exc):
        TIMINGS[self.name] = round(time.perf_counter() - self.t, 2)


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    p = argparse.ArgumentParser(prog="forge-gameready")
    p.add_argument("input", help="GLB, glTF or OBJ")
    p.add_argument("--output", required=True, help="the single output GLB")
    p.add_argument("--tris", type=int, default=15000, help="LOD0 triangle budget")
    p.add_argument("--quads", action="store_true", help="quad-dominant LOD0 (QuadriFlow) at tris/2 quads")
    p.add_argument("--texture", type=int, default=2048, help="baked texture size in pixels")
    p.add_argument("--lods", type=int, default=3, help="extra LOD levels at 50%%, 25%%, 12.5%% ...")
    p.add_argument("--collision", action="store_true", help="CoACD convex hulls named UCX_<name>_NN")
    p.add_argument("--pack", metavar="DIR", help="also write per-level GLB + FBX files and PNG maps here")
    p.add_argument("--detail", metavar="REGIONS", help="upright humanoid: 'head', 'hands' or 'head,hands' keep more "
                   "triangles and texels (off for props)")
    p.add_argument("--detail-strength", type=float, default=6.0, help="detail regions get this many times the "
                   "triangle and texel density of the rest (1 = off)")
    return p.parse_args(argv)


# ---------- mesh helpers ----------

def tri_count(obj):
    return sum(len(p.vertices) - 2 for p in obj.data.polygons)


def manifold_stats(me):
    corner_edges = np.empty(len(me.loops), np.int64)
    me.loops.foreach_get("edge_index", corner_edges)
    c = np.bincount(corner_edges, minlength=len(me.edges))
    return {"non_manifold_edges": int((c != 2).sum()), "boundary_edges": int((c == 1).sum())}


def bbox(obj):
    co = np.empty(len(obj.data.vertices) * 3)
    obj.data.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    lo, hi = co.min(0), co.max(0)
    return lo, hi, float(np.linalg.norm(hi - lo))


def select_only(objs, active=None):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = active or objs[0]


def copy_object(obj, name):
    new = obj.copy()
    new.data = obj.data.copy()
    new.name = new.data.name = name
    bpy.context.scene.collection.objects.link(new)
    return new


def decimate_to(obj, target, frozen=None, tolerance=None):
    """Collapse-decimate obj in place to about target triangles (re-aims up to 3 times from the same start).
    frozen (per-vertex bool) keeps those vertices out of the collapse while others remain: Blender adds edge
    length x factor x (2 - the two inverted weights) to the cost, which at any factor dwarfs the quadric error."""
    if frozen is not None:  # vertex groups live on the mesh data: set before the start copy
        obj.vertex_groups.new(name="forge_detail").add(np.flatnonzero(frozen).tolist(), 1.0, "REPLACE")
    src = obj.data.copy()
    ratio = target / tri_count(obj)
    for _ in range(3):
        obj.data = src.copy()
        mod = obj.modifiers.new("decimate", "DECIMATE")
        mod.decimate_type, mod.ratio, mod.use_collapse_triangulate = "COLLAPSE", min(ratio, 1.0), True
        if frozen is not None:
            mod.vertex_group, mod.vertex_group_factor, mod.invert_vertex_group = "forge_detail", 1000.0, True
        select_only([obj])
        bpy.ops.object.modifier_apply(modifier=mod.name)
        got = tri_count(obj)
        if abs(got - target) <= (tolerance or 0.02 * target):
            break
        ratio *= target / got
    bpy.data.meshes.remove(src)
    if frozen is not None:
        obj.vertex_groups.remove(obj.vertex_groups["forge_detail"])


def components(me):
    """Connected-component label per vertex."""
    e = np.empty(len(me.edges) * 2, np.int64)
    me.edges.foreach_get("vertices", e)
    return union_labels(len(me.vertices), e[0::2], e[1::2])


def union_labels(n, e0, e1):
    """Connected-component label per node of the graph with edges (e0[i], e1[i]) (union-find by root hooking +
    pointer jumping, all numpy)."""
    lab = np.arange(n)
    while True:
        la, lb = lab[e0], lab[e1]
        m = la != lb
        if not m.any():
            return lab
        np.minimum.at(lab, np.maximum(la[m], lb[m]), np.minimum(la[m], lb[m]))
        while True:
            nxt = lab[lab]
            if (nxt == lab).all():
                break
            lab = nxt


# ---------- steps ----------

def import_and_join(path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    ext = os.path.splitext(path)[1].lower()
    if ext in (".glb", ".gltf"):
        bpy.ops.import_scene.gltf(filepath=path)
    elif ext == ".obj":
        bpy.ops.wm.obj_import(filepath=path)
    else:
        raise Fail(f"unsupported input '{ext}': use GLB, glTF or OBJ")
    objs = list(bpy.context.scene.objects)
    meshes = [o for o in objs if o.type == "MESH"]
    if any(o.type == "ARMATURE" for o in objs) or any(m.type == "ARMATURE" for o in meshes for m in o.modifiers):
        raise Fail("skinned (rigged) input is out of scope for v0: export the mesh without its armature and skin")
    if not meshes:
        raise Fail("input contains no mesh")
    select_only(meshes)
    bpy.ops.object.parent_clear(type="CLEAR_KEEP_TRANSFORM")
    for o in objs:
        if o.type != "MESH":
            bpy.data.objects.remove(o)
    bpy.ops.object.make_single_user(object=True, obdata=True)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    if len(meshes) > 1:
        bpy.ops.object.join()
    high = bpy.context.view_layer.objects.active
    high.name = "highpoly"
    if not high.data.materials:  # e.g. OBJ without .mtl: bake a neutral grey instead of black
        mat = bpy.data.materials.new("default")
        mat.use_nodes = True
        high.data.materials.append(mat)
    return high, len(meshes)


def clean(high):
    me = high.data
    before = manifold_stats(me) | {"triangles": tri_count(high)}
    _, _, diag = bbox(high)

    bm = bmesh.new()
    bm.from_mesh(me)
    merged = len(bm.verts)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=diag * 1e-5)
    merged -= len(bm.verts)
    bmesh.ops.delete(bm, geom=[e for e in bm.edges if not e.link_faces], context="EDGES")
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_edges], context="VERTS")
    bm.to_mesh(me)

    lab = components(me)
    co = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    lo = np.full((len(lab), 3), np.inf)
    hi = np.full((len(lab), 3), -np.inf)
    np.minimum.at(lo, lab, co)
    np.maximum.at(hi, lab, co)
    roots = np.unique(lab)
    small = np.zeros(len(lab), bool)
    small[roots] = np.linalg.norm(hi[roots] - lo[roots], axis=1) < LOOSE_PART_FRACTION * diag
    bm.clear()
    bm.from_mesh(me)
    bm.verts.ensure_lookup_table()
    bmesh.ops.delete(bm, geom=[bm.verts[i] for i in np.flatnonzero(small[lab])], context="VERTS")

    holes = bmesh.ops.holes_fill(bm, edges=[e for e in bm.edges if e.is_boundary], sides=HOLE_MAX_SIDES)["faces"]
    bmesh.ops.triangulate(bm, faces=holes)
    # No recalc_face_normals: the input's winding is what its renders show. recalc makes each region consistent
    # and takes its outside from the face farthest from the region's centre; on the text knight's Finish mesh it
    # turned 1,611 faces inside out (folds and patches cut off by non-manifold edges), the black marks on the face
    # and boots after the bake.
    bm.to_mesh(me)
    bm.free()

    if "custom_normal" in me.attributes:  # imported split normals no longer match merged/flipped faces
        me.attributes.remove(me.attributes["custom_normal"])
    # Smooth, with no auto-sharp edges: the low-poly shades like the source and the normal map carries the rest.
    # Sharp edges at 30 degrees put hard creases across the 14k text knight's nose and chin and on the chest's
    # rounded bands (shading 2 to 8x further from the source's, measured on matched close-ups).
    me.shade_smooth()
    return {"before": before, "after": manifold_stats(me) | {"triangles": tri_count(high)},
            "merged_vertices": merged, "loose_parts_removed": int(small[roots].sum()), "holes_filled": len(holes)}


def patch_pinched_edges(high, stats):
    """Replace one small over-connected patch on a closed shell; keep the textured bake source intact."""
    if stats["boundary_edges"] or not 0 < stats["non_manifold_edges"] <= 8:
        return None
    bm = bmesh.new()
    try:
        bm.from_mesh(high.data)
        vertices = {v for e in bm.edges if not e.is_manifold for v in e.verts}
        # Absorb a branching rim vertex once. Only cap a single small, simple boundary loop.
        for _ in range(2):
            faces = {f for v in vertices for f in v.link_faces}
            rim = {e for f in faces for e in f.edges if any(n not in faces for n in e.link_faces)}
            degree = Counter(v for e in rim for v in e.verts)
            branches = {v for v, n in degree.items() if n != 2}
            if not branches:
                break
            vertices.update(branches)
        if branches or not 3 <= len(rim) <= HOLE_MAX_SIDES or len(faces) > len(bm.faces) * .02:
            return None
        first = next(iter(degree))
        seen, pending = {first}, [first]
        while pending:
            v = pending.pop()
            for e in v.link_edges:
                other = e.other_vert(v)
                if e in rim and other not in seen:
                    seen.add(other)
                    pending.append(other)
        if seen != set(degree) or max((a.co-b.co).length for a in seen for b in seen) > .05*bbox(high)[2]:
            return None

        bmesh.ops.delete(bm, geom=list(faces), context="FACES")
        center = bm.verts.new(sum((v.co for v in seen), Vector()) / len(seen))
        for edge in rim:
            loop = edge.link_loops[0]
            bm.faces.new((loop.link_loop_next.vert, loop.vert, center))
        if any(not e.is_manifold for e in bm.edges) or any(not v.is_manifold for v in bm.verts):
            return None
        geo = copy_object(high, "repaired")
        bm.to_mesh(geo.data)
        return geo, {"method": "local cap on pinched edges", "removed_faces": len(faces),
                     "cap_triangles": len(rim), "triangles": tri_count(geo), **manifold_stats(geo.data)}
    finally:
        bm.free()


def repair(high):
    """Closed manifold copy of a mesh that cleaning left open: voxel remesh (OpenVDB, in Blender). It is the
    geometry source for the low-poly only; every map is still baked from the original textured mesh. It also
    drops interior walls (AI meshes are often double-walled), which would otherwise eat the budget and stall
    the LODs."""
    geo = copy_object(high, "repaired")
    geo.data.remesh_voxel_size = bbox(high)[2] / REPAIR_VOXEL_DIVS
    select_only([geo])
    bpy.ops.object.voxel_remesh()
    bm = bmesh.new()
    bm.from_mesh(geo.data)
    bmesh.ops.dissolve_degenerate(bm, dist=1e-4, edges=bm.edges)  # QuadriFlow refuses edges under 1e-4
    # Marching cubes leaves a few non-manifold spots where two surfaces touch inside one voxel (21 edges of 5.4M
    # triangles on the text knight's raw mesh, which made the whole repair fail and decimation stall at 215k
    # triangles). Cut the faces around them out and cap the holes, up to three rounds.
    for patched in range(4):
        spots = {v for e in bm.edges if not e.is_manifold for v in e.verts} | {v for v in bm.verts if not v.is_manifold}
        if not spots or patched == 3:
            break
        bmesh.ops.delete(bm, geom=list({f for v in spots for f in v.link_faces}), context="FACES")
        bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context="VERTS")
        holes = bmesh.ops.holes_fill(bm, edges=[e for e in bm.edges if e.is_boundary], sides=HOLE_MAX_SIDES)["faces"]
        bmesh.ops.triangulate(bm, faces=holes)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)  # reliable here: the voxel shell is closed
    bm.to_mesh(geo.data)
    bm.free()
    def area(me):
        a = np.empty(len(me.polygons))
        me.polygons.foreach_get("area", a)
        return a.sum()
    return geo, {"method": "voxel remesh (Blender/OpenVDB)", "voxel_m": round(geo.data.remesh_voxel_size, 6),
                 "triangles": tri_count(geo), "local_patch_rounds": patched,
                 "area_ratio": round(float(area(geo.data) / area(high.data)), 3), **manifold_stats(geo.data)}


def vertices(obj):
    co = np.empty(len(obj.data.vertices) * 3)
    obj.data.vertices.foreach_get("co", co)
    return co.reshape(-1, 3)


def find_regions(co, names):
    """Detail regions of an upright humanoid (Z up, A- or T-pose), without a rig. Head: everything above the neck,
    the narrowest horizontal slice (X extent) between 70% and 92% of the height. Hands: within 10% of the height
    of the outermost vertex on either side, among vertices between 20% and 75% of the height."""
    z0, z1 = co[:, 2].min(), co[:, 2].max()
    h, regions = z1 - z0, {}
    if "head" in names:
        edges = np.linspace(z0 + .70 * h, z0 + .92 * h, 45)
        widths = [np.ptp(co[(co[:, 2] >= a) & (co[:, 2] < b), 0]) if ((co[:, 2] >= a) & (co[:, 2] < b)).any() else
                  np.inf for a, b in zip(edges, edges[1:])]
        regions["head"] = {"above_z": float(edges[int(np.argmin(widths))])}
    if "hands" in names:
        band = co[(co[:, 2] > z0 + .2 * h) & (co[:, 2] < z0 + .75 * h)]
        for side, i in (("left_hand", band[:, 0].argmax()), ("right_hand", band[:, 0].argmin())):
            regions[side] = {"centre": band[i].round(5).tolist(), "radius": float(.1 * h)}
    return regions


def region_masks(regions, pts):
    return {name: pts[:, 2] > r["above_z"] if "above_z" in r else
            np.linalg.norm(pts - np.array(r["centre"]), axis=1) < r["radius"] for name, r in regions.items()}


def region_stats(lod0, regions, size):
    """Triangles and texels per square centimetre on LOD0, per detail region and for the whole asset."""
    me = lod0.data
    me.calc_loop_triangles()
    co = vertices(lod0)
    tv = np.empty(len(me.loop_triangles) * 3, np.int64)
    me.loop_triangles.foreach_get("vertices", tv)
    tl = np.empty(len(me.loop_triangles) * 3, np.int64)
    me.loop_triangles.foreach_get("loops", tl)
    uv = np.empty(len(me.loops) * 2)
    me.uv_layers[0].data.foreach_get("uv", uv)
    p, t = co[tv.reshape(-1, 3)], uv.reshape(-1, 2)[tl.reshape(-1, 3)]
    area = np.linalg.norm(np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0]), axis=1) / 2 * 1e4  # cm2
    d1, d2 = t[:, 1] - t[:, 0], t[:, 2] - t[:, 0]
    texels = np.abs(d1[:, 0] * d2[:, 1] - d1[:, 1] * d2[:, 0]) / 2 * size * size
    masks = {"all": np.ones(len(p), bool)} | region_masks(regions, p.mean(1))
    return {n: {"triangles": int(m.sum()), "area_cm2": round(float(area[m].sum()), 1),
                "triangles_per_cm2": round(float(m.sum() / area[m].sum()), 3),
                "texels_per_cm2": round(float(texels[m].sum() / area[m].sum()), 1)} for n, m in masks.items()}


def decimate_detail(obj, tris, regions, strength):
    """Decimate to tris giving the detail regions strength x the triangle density of the rest: freeze the regions
    while the rest goes down to its share, then freeze the rest while the regions go down to theirs."""
    def inside():
        me = obj.data
        centres = np.empty(len(me.polygons) * 3)
        me.polygons.foreach_get("center", centres)
        area = np.empty(len(me.polygons))
        me.polygons.foreach_get("area", area)
        return np.any(list(region_masks(regions, centres.reshape(-1, 3)).values()), axis=0), area
    if tri_count(obj) > 40 * tris:  # uniform first: at 40x the budget no region is near its share yet
        decimate_to(obj, 40 * tris)
    faces, area = inside()
    share = area[faces].sum() / area.sum()
    region_tris = round(tris * strength * share / (strength * share + 1 - share))
    frozen = np.any(list(region_masks(regions, vertices(obj)).values()), axis=0)
    decimate_to(obj, int(faces.sum()) + tris - region_tris, frozen, tolerance=0.02 * tris)
    frozen = ~np.any(list(region_masks(regions, vertices(obj)).values()), axis=0)
    decimate_to(obj, tris, frozen)
    return {"area_share": round(float(share), 4), "region_triangles_target": region_tris,
            "region_triangles": int(inside()[0].sum())}


def make_lowpoly(src, tris, quads, regions=None, strength=1.0):
    """LOD0 from src (the cleaned high-poly, or its repaired copy)."""
    low = copy_object(src, "lod0")
    me = low.data
    while me.uv_layers:
        me.uv_layers.remove(me.uv_layers[0])
    me.materials.clear()
    detail = regions and strength != 1
    if not quads:
        if detail:
            return low, {"method": "collapse decimate (Blender), detail regions first frozen, then decimated last",
                         "detail": decimate_detail(low, tris, regions, strength)}
        decimate_to(low, tris)
        return low, {"method": "collapse decimate (Blender)"}
    # On a bumpy decimated mesh QuadriFlow collapses whole regions (asked for 4000 quads it made 1300, with
    # folds), so it gets a decimated, Laplacian-smoothed copy and the result is shrink-wrapped back onto the
    # surface afterwards.
    if detail:
        decimate_detail(low, min(tri_count(low), tris * 4), regions, strength)
    else:
        decimate_to(low, min(tri_count(low), tris * 4))
    smooth = low.modifiers.new("smooth", "LAPLACIANSMOOTH")
    smooth.iterations, smooth.lambda_factor, smooth.use_volume_preserve = 5, 0.5, True
    select_only([low])
    bpy.ops.object.modifier_apply(modifier=smooth.name)
    # QuadriFlow's face count responds non-linearly to target_faces (4000 asked -> ~2700 made), so aim by
    # secant steps in log space and keep the closest of up to 6 tries.
    start, target, tries = low.data, tris // 2, []
    for _ in range(6):
        low.data = start.copy()
        r = bpy.ops.object.quadriflow_remesh(mode="FACES", target_faces=target, seed=1)
        if r != {"FINISHED"} or len(low.data.polygons) == len(start.polygons):
            raise Fail("QuadriFlow remesh failed on this mesh; rerun without --quads")
        tries.append((target, tri_count(low), low.data))
        if abs(tries[-1][1] - tris) <= 0.03 * tris:
            break
        slope = 1.0
        if len(tries) > 1 and tries[-1][0] != tries[-2][0] and tries[-1][1] != tries[-2][1]:
            (t1, g1, _), (t2, g2, _) = tries[-2:]
            slope = min(max(math.log(g2 / g1) / math.log(t2 / t1), 0.2), 2.0)
        target = max(4, round(target * (tris / tries[-1][1]) ** (1 / slope)))
    low.data = min(tries, key=lambda t: abs(t[1] - tris))[2]
    wrap = low.modifiers.new("wrap", "SHRINKWRAP")
    wrap.target, wrap.wrap_method = src, "NEAREST_SURFACEPOINT"
    bpy.ops.object.modifier_apply(modifier=wrap.name)
    quad_share = sum(len(p.vertices) == 4 for p in low.data.polygons) / len(low.data.polygons)
    return low, {"method": "QuadriFlow (Blender) on a smoothed copy, shrink-wrapped to the high-poly",
                 "quad_faces": len(low.data.polygons), "quad_share": round(quad_share, 4),
                 "tries": [[t, g // 2] for t, g, _ in tries]}


def unwrap(low, size, regions=None, strength=1.0):
    """xatlas charts + packing (2x the texel use of Blender's Smart UV Project on decimated meshes, which
    splits them into thousands of islands). Quads: xatlas only sees triangles, so its chart borders become
    seams and Blender unwraps and packs."""
    import xatlas
    me = low.data
    me.shade_smooth()
    me.calc_loop_triangles()
    v = np.empty(len(me.vertices) * 3, np.float32)
    me.vertices.foreach_get("co", v)
    f = np.empty(len(me.loop_triangles) * 3, np.uint32)
    me.loop_triangles.foreach_get("vertices", f)
    f = f.reshape(-1, 3)
    atlas = xatlas.Atlas()
    atlas.add_mesh(v.reshape(-1, 3), f)
    po = xatlas.PackOptions()
    po.resolution, po.padding, po.bilinear = size, max(2, size // 256), True
    co = xatlas.ChartOptions()
    quads = any(len(p.vertices) != 3 for p in me.polygons)
    if quads:  # small charts stay disk-like, so Blender's unwrap cannot wrap one over itself
        co.max_chart_area = sum(p.area for p in me.polygons) / 150
    atlas.generate(co, po)
    _, idx, uvs = atlas[0]
    layer = me.uv_layers.new(name="UVMap")
    info = {"charts": atlas.chart_count, "xatlas_utilization": round(atlas.utilization, 3), "padding_px": po.padding}
    if not quads:
        method = "xatlas charts + pack"
        loops = np.empty(f.size, np.int64)
        me.loop_triangles.foreach_get("loops", loops)
        uv = np.zeros((len(me.loops), 2))
        uv[loops] = uvs[idx.ravel()]
        if regions and strength != 1:  # detail charts grow by sqrt(strength), then Blender packs all islands again
            chart = union_labels(len(uvs), np.r_[idx[:, 0], idx[:, 1]], np.r_[idx[:, 1], idx[:, 2]])[idx[:, 0]]
            inside = np.any(list(region_masks(regions, v.reshape(-1, 3)[f].mean(1)).values()), axis=0)
            share = np.bincount(chart, inside, len(uvs)) / np.maximum(np.bincount(chart, minlength=len(uvs)), 1)
            grow = share[chart] > .5
            for c in np.unique(chart[grow]):
                lp = loops.reshape(-1, 3)[chart == c].ravel()
                mid = uv[lp].mean(0)
                uv[lp] = mid + (uv[lp] - mid) * math.sqrt(strength)
            layer.data.foreach_set("uv", uv.ravel())
            select_only([low])
            bpy.ops.object.mode_set(mode="EDIT")
            bpy.ops.mesh.select_all(action="SELECT")
            bpy.ops.uv.select_all(action="SELECT")
            bpy.ops.uv.pack_islands(rotate=True, margin_method="FRACTION", margin=po.padding / size)
            bpy.ops.object.mode_set(mode="OBJECT")
            info["detail_charts"] = int(np.unique(chart[grow]).size)
            del info["xatlas_utilization"]  # Blender packed the final layout
            return info | {"method": method + ", detail charts scaled by sqrt(strength), Blender Pack Islands"}
        layer.data.foreach_set("uv", uv.ravel())
        return info | {"method": method}
    # seam = a triangle edge whose two triangles map its ends to different xatlas vertices. A cut along a
    # quad's diagonal is rerouted over the quad's other two edges of one triangle (same end points).
    nv = len(me.vertices)
    a, b, c = f.astype(np.int64), np.roll(f, -1, axis=1).astype(np.int64), np.roll(f, -2, axis=1).astype(np.int64)
    na, nb = idx, np.roll(idx, -1, axis=1)
    swap = a > b
    key = (np.minimum(a, b) * nv + np.maximum(a, b)).ravel()
    pair = np.stack([np.where(swap, nb, na).ravel(), np.where(swap, na, nb).ravel()], 1)
    rows = np.unique(np.column_stack([key, pair]), axis=0)
    keys, counts = np.unique(rows[:, 0], return_counts=True)
    cut = np.isin(key, keys[counts > 1])
    ev = np.empty(len(me.edges) * 2, np.int64)
    me.edges.foreach_get("vertices", ev)
    ev = np.sort(ev.reshape(-1, 2), axis=1)
    edge_keys = ev[:, 0] * nv + ev[:, 1]
    diag = cut & ~np.isin(key, edge_keys)
    _, first = np.unique(key[diag], return_index=True)
    rows_d = np.flatnonzero(diag)[first]
    a, b, c = a.ravel()[rows_d], b.ravel()[rows_d], c.ravel()[rows_d]
    reroute = np.concatenate([np.minimum(a, c) * nv + np.maximum(a, c), np.minimum(b, c) * nv + np.maximum(b, c)])
    me.edges.foreach_set("use_seam", np.isin(edge_keys, np.concatenate([key[cut], reroute])))
    select_only([low])
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.unwrap(method="MINIMUM_STRETCH", margin=0.0)  # SLIM: locally injective, so no flipped faces
    bpy.ops.uv.pack_islands(rotate=True, margin_method="FRACTION", margin=po.padding / size)
    bpy.ops.object.mode_set(mode="OBJECT")
    return info | {"method": "xatlas charts as seams + Blender Minimum Stretch (SLIM) unwrap + Pack Islands"}


def principled(mat):
    return next((n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED"), None) if mat.use_nodes else None


def source_channel(high, name):
    """'bake' if any source material drives input 'name' with a texture or the values differ; else the constant."""
    vals = []
    for mat in high.data.materials:
        b = principled(mat) if mat else None
        if b is None:
            continue
        sock = b.inputs[name]
        if sock.is_linked:
            return "bake"
        vals.append(round(float(sock.default_value), 4))
    if len(set(vals)) > 1:
        return "bake"
    return vals[0] if vals else (0.5 if name == "Roughness" else 0.0)


def emit_from(high, name):
    """Route each source material's input 'name' into an Emission shader so an EMIT bake reads it exactly."""
    for mat in high.data.materials:
        nt = mat.node_tree
        out = next(n for n in nt.nodes if n.type == "OUTPUT_MATERIAL" and n.is_active_output)
        em = nt.nodes.get("forge_emit") or nt.nodes.new("ShaderNodeEmission")
        em.name = "forge_emit"
        for link in list(em.inputs["Color"].links):
            nt.links.remove(link)
        b = principled(mat)
        sock = b.inputs[name] if b else None
        if sock is not None and sock.is_linked:
            nt.links.new(sock.links[0].from_socket, em.inputs["Color"])
        elif name == "Base Color":
            em.inputs["Color"].default_value = tuple(sock.default_value) if sock is not None else (0.8, 0.8, 0.8, 1)
        else:
            v = sock.default_value if sock is not None else 0.5
            em.inputs["Color"].default_value = (v, v, v, 1.0)
        nt.links.new(em.outputs["Emission"], out.inputs["Surface"])


def cage_distance(high, low, diag):
    """Extrusion = 2x the 99th-percentile gap between the low-poly surface and the high-poly (min 0.2% diag)."""
    tree = BVHTree.FromObject(high, bpy.context.evaluated_depsgraph_get())
    pts = [v.co for v in low.data.vertices] + [p.center for p in low.data.polygons]
    gaps = np.array([tree.find_nearest(p)[3] for p in pts])
    return max(2.0 * float(np.percentile(gaps, 99)), 0.002 * diag)


def bake(high, low, name, size):
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    # CPU, on purpose: these bakes take ~0.5 s per map on the CPU and ~1 s on Metal, and the Metal path
    # compiles kernels first (190 s cold) and crashed once inside Apple's shader compiler (heap corruption).
    scene.cycles.device = "CPU"
    scene.cycles.samples = 4
    scene.render.bake.margin = max(4, size // 128)
    scene.render.bake.margin_type = "EXTEND"
    extrusion = cage_distance(high, low, bbox(high)[2])

    rough, metal = source_channel(high, "Roughness"), source_channel(high, "Metallic")
    has_orm = "bake" in (rough, metal)

    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    low.data.materials.append(mat)
    nt = mat.node_tree
    bsdf = principled(mat)

    def image_node(kind, srgb):
        node = nt.nodes.new("ShaderNodeTexImage")
        node.image = bpy.data.images.new(f"{name}_{kind}", size, size, alpha=False)
        node.image.colorspace_settings.name = "sRGB" if srgb else "Non-Color"
        return node

    base = image_node("basecolor", True)
    nt.links.new(base.outputs["Color"], bsdf.inputs["Base Color"])
    normal = image_node("normal", False)
    nmap = nt.nodes.new("ShaderNodeNormalMap")
    nt.links.new(normal.outputs["Color"], nmap.inputs["Color"])
    nt.links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])
    passes = [("normal", normal, "NORMAL", None, 45, "Baking normal map"),
              ("basecolor", base, "EMIT", "Base Color", 55, "Baking base colour")]
    if has_orm:
        orm = image_node("orm", False)
        sep = nt.nodes.new("ShaderNodeSeparateColor")
        nt.links.new(orm.outputs["Color"], sep.inputs["Color"])
        nt.links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
        nt.links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
        r_tmp, m_tmp = image_node("rough_tmp", False), image_node("metal_tmp", False)
        passes += [("roughness", r_tmp, "EMIT", "Roughness", 62, "Baking ORM maps (roughness)"),
                   ("metallic", m_tmp, "EMIT", "Metallic", 68, "Baking ORM maps (metallic)")]
    if rough != "bake":
        bsdf.inputs["Roughness"].default_value = rough
    if metal != "bake":
        bsdf.inputs["Metallic"].default_value = metal

    select_only([high, low], active=low)
    for kind, node, bake_type, channel, pct, message in passes:
        progress(pct, message)
        if channel:
            emit_from(high, channel)
        nt.nodes.active = node
        t = time.perf_counter()
        bpy.ops.object.bake(type=bake_type, use_selected_to_active=True, cage_extrusion=extrusion,
                            max_ray_distance=2 * extrusion, normal_space="TANGENT", use_clear=True)
        TIMINGS[f"bake_{kind}"] = round(time.perf_counter() - t, 2)

    missed = missed_texels(high, low, nt, size, extrusion)
    images = [base.image, normal.image]
    if has_orm:  # pack ORM: R occlusion (1, not baked), G roughness, B metallic (glTF reads G/B, Unreal all three)
        px = np.ones(size * size * 4, np.float32)
        tmp = np.empty_like(px)
        r_tmp.image.pixels.foreach_get(tmp)
        px[1::4] = tmp[0::4]
        m_tmp.image.pixels.foreach_get(tmp)
        px[2::4] = tmp[0::4]
        orm.image.pixels.foreach_set(px)
        images.append(orm.image)
        for node in (r_tmp, m_tmp):
            bpy.data.images.remove(node.image)
            nt.nodes.remove(node)
    for img in images:
        img.pack()  # PNG-encoded in memory so the glTF exporter embeds it
    maps = {img.name.removeprefix(name + "_"): {"image": img.name, "size": list(img.size)} for img in images}
    return images, maps, {"device": "CPU", "samples": scene.cycles.samples, "cage_extrusion": round(extrusion, 6),
                          "max_ray_distance": round(2 * extrusion, 6), "margin_px": scene.render.bake.margin,
                          "missed_texels": missed,
                          "roughness": rough, "metallic": metal}


def missed_texels(high, low, nt, size, extrusion):
    """Texels inside LOD0's UV islands whose cage ray found no source surface (Blender's margin fills them from
    their neighbours afterwards, so a large count means smeared colour, not black). Two extra EMIT bakes at
    margin 0: constant white from the source, and LOD0's own white for the island mask."""
    scene = bpy.context.scene
    margin, scene.render.bake.margin = scene.render.bake.margin, 0
    for mat in high.data.materials:
        em = mat.node_tree.nodes["forge_emit"]
        for link in list(em.inputs["Color"].links):
            mat.node_tree.links.remove(link)
        em.inputs["Color"].default_value = (1, 1, 1, 1)
    node = nt.nodes.new("ShaderNodeTexImage")
    out = next(n for n in nt.nodes if n.type == "OUTPUT_MATERIAL")
    masks = []
    for selected in (True, False):
        node.image = bpy.data.images.new("forge_mask", size, size, alpha=False)
        nt.nodes.active = node
        if not selected:  # LOD0 itself emits white
            surface = out.inputs["Surface"].links[0].from_socket
            em = nt.nodes.new("ShaderNodeEmission")
            nt.links.new(em.outputs["Emission"], out.inputs["Surface"])
            select_only([low])
        bpy.ops.object.bake(type="EMIT", use_selected_to_active=selected, cage_extrusion=extrusion,
                            max_ray_distance=2 * extrusion, use_clear=True)
        px = np.empty(size * size * 4, np.float32)
        node.image.pixels.foreach_get(px)
        masks.append(px[0::4] > .5)
        bpy.data.images.remove(node.image)
    nt.links.new(surface, out.inputs["Surface"])
    nt.nodes.remove(em)
    nt.nodes.remove(node)
    scene.render.bake.margin = margin
    return int((masks[1] & ~masks[0]).sum())


def make_lods(lod0, n, name):
    levels = [lod0]
    for i in range(1, n + 1):
        lod = copy_object(levels[-1], f"{name}_LOD{i}")
        decimate_to(lod, max(4, round(tri_count(lod0) / 2 ** i)))
        levels.append(lod)
    return levels


def make_collision(lod0):
    import coacd
    me = lod0.data
    me.calc_loop_triangles()
    v = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", v)
    f = np.empty(len(me.loop_triangles) * 3, np.int32)
    me.loop_triangles.foreach_get("vertices", f)
    coacd.set_log_level("error")
    parts = coacd.run_coacd(coacd.Mesh(v.reshape(-1, 3), f.reshape(-1, 3)), threshold=0.08, max_convex_hull=8,
                             decimate=True, max_ch_vertex=64, seed=1)  # engines cap hulls (PhysX: 255 verts)
    hulls = []
    for i, (hv, hf) in enumerate(parts):
        hm = bpy.data.meshes.new(f"UCX_{lod0.name}_{i:02d}")
        hm.from_pydata(hv.tolist(), [], hf.tolist())
        hm.update()
        ob = bpy.data.objects.new(hm.name, hm)
        bpy.context.scene.collection.objects.link(ob)
        hulls.append(ob)
    return hulls


def export_glb(objs, path):
    select_only(objs)
    bpy.ops.export_scene.gltf(filepath=path, use_selection=True, export_format="GLB", export_yup=True,
                              export_apply=True)


def export_pack(pack, levels, hulls, images):
    """Per-level GLB + FBX files (LOD0's carry the UCX_ hulls, Unreal's convention) and the PNG maps."""
    tex = os.path.join(pack, "textures")
    os.makedirs(tex, exist_ok=True)
    for img in images:  # saved first so the FBX files reference textures/<map>.png relatively
        img.filepath_raw = os.path.join(tex, img.name + ".png")
        img.file_format = "PNG"
        img.save()
    files = []
    for i, lod in enumerate(levels):
        objs = [lod] + (hulls if i == 0 else [])
        stem = os.path.join(pack, lod.name if i else lod.name + "_LOD0")
        export_glb(objs, stem + ".glb")
        bpy.ops.export_scene.fbx(filepath=stem + ".fbx", use_selection=True, mesh_smooth_type="FACE",
                                 use_tspace=True, path_mode="RELATIVE", object_types={"MESH"})
        files += [stem + ".glb", stem + ".fbx"]
    return [os.path.relpath(f, pack) for f in files] + [os.path.join("textures", img.name + ".png") for img in images]


def main():
    args = parse_args()
    t0 = time.perf_counter()
    src = os.path.abspath(args.input)
    output = os.path.abspath(args.output)
    os.makedirs(os.path.dirname(output), exist_ok=True)
    name = os.path.splitext(os.path.basename(output))[0]

    with step("import", 2, "Importing and joining parts"):
        high, parts = import_and_join(src)
    with step("clean", 8, "Cleaning: merge, loose parts, holes, normals"):
        cleaned = clean(high)
    geo, repaired = high, None
    if cleaned["after"]["non_manifold_edges"]:
        with step("repair", 12, "Repairing non-manifold geometry"):
            # Within the budget the cleaned mesh is LOD0 as it is (a Finish mesh): a voxel copy would only
            # re-approximate it. A voxel shell with more area than the input (1.33x on the open test tube) is a
            # zero-thickness sheet turned into a slab, whose LODs plateau: keep the cleaned mesh. A double-walled
            # generator mesh shrinks instead (0.79x on the text knight's raw mesh).
            local = patch_pinched_edges(high, cleaned["after"])
            geo, repaired = local or (repair(high) if tri_count(high) > args.tris else
                                      (high, {"method": "none: already within the triangle budget"}))
            repaired["used"] = bool(geo is not high and not repaired["non_manifold_edges"] and
                                    repaired.get("area_ratio", 1) < 1.2)
            if not repaired["used"] and geo is not high:
                bpy.data.objects.remove(geo)
                geo = high
    regions = find_regions(vertices(geo), args.detail.split(",")) if args.detail else None
    with step("lowpoly", 15, "Building the quad low-poly" if args.quads else "Decimating to the triangle budget"):
        lod0, lowpoly = make_lowpoly(geo, args.tris, args.quads, regions, args.detail_strength)
        if geo is not high:
            bpy.data.objects.remove(geo)
        lod0.name = lod0.data.name = name
    with step("uv", 30, "UV unwrapping"):
        uv = unwrap(lod0, args.texture, regions, args.detail_strength)
    with step("bake", 40, "Baking"):
        images, maps, bake_info = bake(high, lod0, name, args.texture)
    with step("lods", 75, f"Building {args.lods} LODs"):
        levels = make_lods(lod0, args.lods, name)
    hulls = []
    if args.collision:
        with step("collision", 82, "Computing CoACD collision hulls"):
            hulls = make_collision(lod0)
    with step("export", 92, "Exporting GLB"):
        export_glb(levels + hulls, output)
        packed = export_pack(os.path.abspath(args.pack), levels, hulls, images) if args.pack else None

    lo, hi, _ = bbox(lod0)
    tools = {"blender": bpy.app.version_string, "cycles": bpy.app.version_string, "xatlas": XATLAS_VERSION}
    licences = {"blender": "GPL-3.0-or-later (run as a tool; outputs are not covered)", "cycles": "Apache-2.0",
                "xatlas": "MIT"}
    if args.quads:
        tools["quadriflow"], licences["quadriflow"] = "bundled with Blender", "MIT"
    if hulls:
        tools["coacd"], licences["coacd"] = COACD_VERSION, "MIT"
    manifest = {
        "stage": "gameready", "input": src, "output": output,
        "options": {k: v for k, v in vars(args).items() if k not in ("input", "output")},
        "tools": tools, "licences": licences,
        "triangles": [tri_count(o) for o in levels],
        "lods": [{"node": o.name, "triangles": tri_count(o), "faces": len(o.data.polygons), **manifold_stats(o.data)}
                 for o in levels],
        "texture_maps": maps,
        "collision": [{"node": h.name, "vertices": len(h.data.vertices)} for h in hulls],
        "collision_method": "CoACD threshold 0.08, at most 8 hulls of at most 64 vertices" if hulls else None,
        "detail": {"regions": regions, "strength": args.detail_strength,
                   "lod0": region_stats(lod0, regions, args.texture)} if regions else None,
        "input_parts": parts, "clean": cleaned, "repair": repaired, "lowpoly": lowpoly, "uv": uv, "bake": bake_info,
        "bounding_box_m": {"min": lo.round(5).tolist(), "max": hi.round(5).tolist(),
                           "size": (hi - lo).round(5).tolist(), "axes": "Blender Z-up (the GLB is Y-up)"},
        "timings": TIMINGS | {"total": round(time.perf_counter() - t0, 2)},
    }
    if packed:
        manifest["pack"] = {"dir": os.path.abspath(args.pack), "files": packed}
    with open(output + ".manifest.json", "w") as fh:
        json.dump(manifest, fh, indent=2)
    progress(100, f"Done: {manifest['triangles'][0]} triangles, {len(levels) - 1} LODs, {len(hulls)} hulls")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # Fail and anything unexpected: one {"error"} line, exit 3 (the launcher's "already reported")
        if not isinstance(e, Fail):
            traceback.print_exc()
        PROGRESS.write(json.dumps({"error": str(e)}) + "\n")
        PROGRESS.flush()
        sys.exit(3)
