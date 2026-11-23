"""Functional props: an intake test per class (in palm widths) and a small parametric library built in
Blender with correct proportions and anchors, styled from a generated prop (its colours, a few clamped shape
ratios) so the stand-in still looks like the requested object."""

import math

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

import fitlib as F


# -- intake ---------------------------------------------------------------------------------------
def centreline_deviation(points, axis, t0, t1, count=24):
    """Max distance of slab centroids from their best-fit line, between t0 and t1 along the axis."""
    t = points @ axis
    sel = (t >= t0) & (t <= t1)
    if sel.sum() < 20:
        return 0.0
    p = points[sel]
    tt = t[sel]
    edges = np.linspace(tt.min(), tt.max(), count + 1)
    cents = np.array([p[(tt >= a) & (tt < b)].mean(axis=0) for a, b in zip(edges[:-1], edges[1:]) if ((tt >= a) & (tt < b)).sum() > 3])
    if len(cents) < 3:
        return 0.0
    c = cents.mean(axis=0)
    _, _, vt = np.linalg.svd(cents - c)
    d = cents - c
    return float(np.linalg.norm(d - np.outer(d @ vt[0], vt[0]), axis=1).max())


def intake(prop, palm_width, s):
    """Can a hand use this as its class says? Returns {"pass", "reasons", "metrics"} (lengths in metres and
    palm widths)."""
    A, pts, cls = prop.anchors, prop.points, prop.cls
    axis = A["axis"]
    t = pts @ axis
    L = float(np.ptp(t))
    pw = palm_width
    m = {"length_m": L / s, "palm_width_m": pw / s}
    why = []
    if cls == "sword":
        gd = 2 * A["grip_radius"] / pw
        m["grip_diameter_palm_widths"] = gd
        m["grip_diameter_m"] = 2 * A["grip_radius"] / s
        m["grip_length_palm_widths"] = A["grip_length"] / pw
        g = (A["guard"] @ axis - t.min()) / L
        m["guard_position_fraction"] = float(g)
        rows = F.widths(pts, axis, np.zeros(3))
        wmax = max(r[3] for r in rows)
        m["guard_width_over_grip"] = float(wmax / max(2 * A["grip_radius"], 1e-9))
        if not 0.15 <= gd <= 0.6:
            why.append(f"grip is {gd:.2f} palm widths thick ({m['grip_diameter_m'] * 100:.1f} cm); a hand closes on 0.15-0.6")
        if 0.38 < g < 0.62:
            why.append(f"the guard sits in the middle ({g:.0%} of the length): a grip at both ends, no single handle")
        if m["guard_width_over_grip"] < 2.0:
            why.append("no guard wider than twice the grip")
        if m["grip_length_palm_widths"] < 0.9:
            why.append(f"grip only {m['grip_length_palm_widths']:.2f} palm widths long")
        tip, guard = A["tip"] @ axis, A["guard"] @ axis
        dev = centreline_deviation(pts, axis, min(tip, guard), max(tip, guard))
        m["blade_bend_fraction"] = dev / L
        if dev / L > 0.02:
            why.append(f"blade is bent ({dev / L:.1%} of the length off straight)")
    elif cls == "bow":
        gd = 2 * A["grip_radius"] / pw
        m["grip_diameter_palm_widths"] = gd
        m["grip_diameter_m"] = 2 * A["grip_radius"] / s
        if gd > 0.7:
            why.append(f"grip is {gd:.2f} palm widths thick ({m['grip_diameter_m'] * 100:.0f} cm); a hand closes on at most 0.7")
        if not 0.9 <= L / s <= 2.0:
            why.append(f"bow length {L / s:.2f} m is outside 0.9-2.0 m")
        m["brace_m"] = A.get("brace", 0) / s
    elif cls == "arrow":
        dev = centreline_deviation(pts, axis, t.min() + 0.15 * L, t.max() - 0.15 * L)
        m["shaft_bend_fraction"] = dev / L
        m["shaft_diameter_palm_widths"] = 2 * A["grip_radius"] / pw
        if dev / L > 0.01:
            why.append(f"shaft is bent ({dev / L:.1%})")
        if m["shaft_diameter_palm_widths"] > 0.2:
            why.append(f"shaft is {m['shaft_diameter_palm_widths']:.2f} palm widths thick")
        rows = F.widths(pts, axis, np.zeros(3))
        w = np.array([r[3] for r in rows])
        head = w[-4:].max() if A["tip"] @ axis > A["nock"] @ axis else w[:4].max()
        m["head_width_m"] = float(head / s)
        if head / s > 0.06:
            why.append(f"head is {head / s * 100:.0f} cm wide (a ball or a foam tip, not an arrowhead)")
    elif cls == "quiver":
        r = A["radius"] / s
        m["opening_radius_m"] = r
        if r < 0.025:
            why.append(f"opening radius {r * 100:.1f} cm is too narrow for a few arrows")
        if r > 0.09:
            why.append(f"opening radius {r * 100:.0f} cm: a barrel, not a quiver")
        m["strap"] = False
        why_soft = "no strap modelled (attached to the back socket instead)"
        m["note"] = why_soft
    if F.disabled("prop_intake"):
        why = []
    return {"class": cls, "pass": not why, "reasons": why, "metrics": m}


# -- styling --------------------------------------------------------------------------------------
def face_colours(obj):
    """Per face: centre (object space) and base colour (texture at the face UV centre, linear)."""
    import handfix
    me = obj.data
    uv = me.uv_layers.active
    cache, out = {}, []
    for poly in me.polygons:
        mat = obj.material_slots[poly.material_index].material if obj.material_slots else None
        img, col = handfix.base_colour_source(mat)
        if img is not None and uv is not None:
            if img.name not in cache:
                px = np.empty(img.size[0] * img.size[1] * 4, np.float32)
                img.pixels.foreach_get(px)
                cache[img.name] = (px.reshape(img.size[1], img.size[0], 4), img.colorspace_settings.name == "sRGB" and not img.is_float)
            px, enc = cache[img.name]
            u, v = np.mean([uv.data[i].uv[:] for i in poly.loop_indices], axis=0)
            c = px[int(np.clip(v % 1, 0, 0.9999) * px.shape[0]), int(np.clip(u % 1, 0, 0.9999) * px.shape[1]), :3]
            col = handfix.srgb_to_linear(c) if enc else c
        out.append((np.array(poly.center[:]), np.array(col[:3], float)))
    return out


def style_from(prop):
    """Region colours and clamped shape ratios of a (possibly failing) generated prop."""
    A = prop.anchors
    axis = A["axis"]
    fc = face_colours(prop.obj)
    t = np.array([c @ axis for c, _ in fc])
    cols = np.array([k for _, k in fc])
    L = float(np.ptp(prop.points @ axis))
    style = {"colours": {}, "ratios": {}}
    def region(lo, hi):
        sel = (t >= min(lo, hi)) & (t <= max(lo, hi))
        return tuple(np.median(cols[sel], axis=0).tolist()) if sel.sum() else None
    if prop.cls == "sword":
        g, tip, pom = A["guard"] @ axis, A["tip"] @ axis, A["pommel"] @ axis
        gr0 = A["grip"] @ axis - A["grip_length"] / 2 * np.sign(A["grip_axis"] @ axis)
        style["colours"] = {"blade": region(g + 0.1 * (tip - g), tip), "guard": region(g - 0.01 * L, g + 0.01 * L),
                            "grip": region(A["grip"] @ axis - 0.3 * A["grip_length"], A["grip"] @ axis + 0.3 * A["grip_length"]),
                            "pommel": region(pom, pom + 0.04 * (g - pom))}
        style["ratios"] = {"blade_over_length": float(np.clip(abs(tip - g) / L, 0.6, 0.82))}
    elif prop.cls == "bow":
        style["colours"] = {"limb": region((A["upper_tip"] @ axis + A["grip"] @ axis) / 2 - 0.1 * L, (A["upper_tip"] @ axis + A["grip"] @ axis) / 2 + 0.1 * L),
                            "grip": region(A["grip"] @ axis - 0.04 * L, A["grip"] @ axis + 0.04 * L),
                            "tips": region(A["upper_tip"] @ axis - 0.03 * L, A["upper_tip"] @ axis + 0.03 * L)}
    elif prop.cls == "arrow":
        n, tp = A["nock"] @ axis, A["tip"] @ axis
        style["colours"] = {"shaft": region(n + 0.4 * (tp - n), n + 0.6 * (tp - n)), "head": region(tp - 0.04 * (tp - n), tp),
                            "fletch": region(n, n + 0.12 * (tp - n))}
    elif prop.cls == "quiver":
        style["colours"] = {"body": region(t.min() + 0.3 * L, t.min() + 0.7 * L), "rim": region(A["opening"] @ axis - 0.04 * L, A["opening"] @ axis + 0.04 * L)}
    style["colours"] = {k: v for k, v in style["colours"].items() if v is not None}
    # a metal part must look like metal: a strongly coloured "blade"/"head" sample means the region guess hit
    # something else (a second grip, a foam ball), so the steel default is kept
    for part in ("blade", "head"):
        c = style["colours"].get(part)
        if c is not None and (max(c) - min(c)) > 0.25 * max(max(c), 1e-6):
            style["colours"].pop(part)
            style.setdefault("rejected", []).append(part)
    return style


# -- parametric builders (metres, z = principal axis) ---------------------------------------------
def _mat(name, rgb, metal=0.0, rough=0.5):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = (*rgb, 1)
    b.inputs["Metallic"].default_value = metal
    b.inputs["Roughness"].default_value = rough
    m.diffuse_color = (*rgb, 1)
    return m


def _cyl(r0, r1, z0, z1, mat, seg=16, x=0.0, y=0.0):
    bpy.ops.mesh.primitive_cone_add(vertices=seg, radius1=r0, radius2=r1, depth=z1 - z0, location=(x, y, (z0 + z1) / 2))
    o = bpy.context.object
    o.data.materials.append(mat)
    return o


def _box(cx, cy, cz, sx, sy, sz, mat):
    bpy.ops.mesh.primitive_cube_add(size=1, location=(cx, cy, cz))
    o = bpy.context.object
    o.scale = (sx, sy, sz)
    bpy.ops.object.transform_apply(scale=True)
    o.data.materials.append(mat)
    return o


def _join(objs, name):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    o = bpy.context.object
    o.name = name
    bpy.ops.object.shade_smooth()
    return o


def build(cls, height_m=1.8, palm_width_m=None, style=None):
    """One parametric prop in metres; returns the object. Sizes follow the character's height."""
    k = height_m / 1.8
    pw = palm_width_m or 0.085 * k
    c = (style or {}).get("colours", {})
    r = (style or {}).get("ratios", {})
    grey, brown, brass, wood = (0.62, 0.64, 0.68), (0.18, 0.09, 0.04), (0.6, 0.42, 0.15), (0.35, 0.2, 0.08)
    if cls == "sword":
        total = 1.0 * k
        blade = total * r.get("blade_over_length", 0.76)
        grip_len, grip_r = 0.19 * k, 0.19 * pw  # grip diameter ~0.38 palm widths
        mb, mg, mh, mp = (_mat("Blade", c.get("blade", grey), 1.0, 0.25), _mat("Guard", c.get("guard", brass), 0.9, 0.35),
                          _mat("Grip", c.get("grip", brown), 0.0, 0.7), _mat("Pommel", c.get("pommel", brass), 0.9, 0.35))
        z = 0.0
        parts = [_cyl(0.022 * k, 0.022 * k, z, z + 0.035 * k, mp, 12)]
        z += 0.03 * k
        parts.append(_cyl(grip_r, grip_r * 1.05, z, z + grip_len, mg if False else mh, 16))
        z += grip_len
        parts.append(_box(0, 0, z + 0.012 * k, 0.21 * k, 0.03 * k, 0.024 * k, mg))
        z += 0.024 * k
        # blade: diamond cross-section, parallel edges, the last 14% tapering to the point
        hw, ht = 0.025 * k, 0.0045 * k
        ring = lambda zz, sc: [(hw * sc, 0, zz), (0, ht * sc, zz), (-hw * sc, 0, zz), (0, -ht * sc, zz)]
        verts = ring(z, 1.0) + ring(z + blade * 0.86, 0.85) + [(0, 0, z + blade)]
        faces = [(i, (i + 1) % 4, 4 + (i + 1) % 4, 4 + i) for i in range(4)] + [(4 + i, 4 + (i + 1) % 4, 8) for i in range(4)] + [(3, 2, 1, 0)]
        me = bpy.data.meshes.new("blade")
        me.from_pydata(verts, [], faces)
        bl = bpy.data.objects.new("blade", me)
        bpy.context.collection.objects.link(bl)
        bl.data.materials.append(mb)
        parts.append(bl)
        return _join(parts, "sword")
    if cls == "bow":
        L = 1.5 * k
        grip_r = 0.18 * pw
        ml, mg, mt = (_mat("Limb", c.get("limb", wood), 0.0, 0.5), _mat("Grip", c.get("grip", brown), 0.0, 0.7),
                      _mat("Tips", c.get("tips", brass), 0.8, 0.4))
        parts = [_cyl(grip_r, grip_r, -0.06 * k, 0.06 * k, mg, 16)]
        for sg in (1, -1):
            prev = None
            for i in range(14):
                zz = sg * (0.06 * k + (L / 2 - 0.06 * k) * i / 13)
                yy = 0.17 * k * ((abs(zz) - 0.06 * k) / (L / 2 - 0.06 * k)) ** 2
                rr = grip_r * 0.75 * (1 - 0.6 * i / 13)
                p = Vector((0, yy, zz))
                if prev is not None:
                    d = p - prev
                    bpy.ops.mesh.primitive_cylinder_add(vertices=10, radius=rr, depth=d.length, location=(prev + p) / 2)
                    o = bpy.context.object
                    o.rotation_euler = d.to_track_quat("Z", "Y").to_euler()
                    bpy.ops.object.transform_apply(rotation=True)
                    o.data.materials.append(mt if i >= 12 else ml)
                    parts.append(o)
                prev = p
        return _join(parts, "bow")
    if cls == "arrow":
        L = 0.78 * k
        ms, mh, mf = (_mat("Shaft", c.get("shaft", wood), 0.0, 0.6), _mat("Head", c.get("head", grey), 1.0, 0.3),
                      _mat("Fletch", c.get("fletch", (0.7, 0.1, 0.08)), 0.0, 0.7))
        parts = [_cyl(0.004, 0.004, 0.0, L - 0.04, ms, 8), _cyl(0.011, 0.0, L - 0.045, L, mh, 8)]
        for i in range(3):
            a = 2 * math.pi * i / 3
            o = _box(0.009 * math.cos(a), 0.009 * math.sin(a), 0.08, 0.001, 0.012, 0.1, mf)
            o.rotation_euler = (0, 0, a)
            bpy.ops.object.transform_apply(rotation=True)
            parts.append(o)
        return _join(parts, "arrow")
    if cls == "quiver":
        L, rad = 0.55 * k, 0.055 * k
        mb, mr = _mat("Body", c.get("body", brown), 0.0, 0.7), _mat("Rim", c.get("rim", brass), 0.8, 0.4)
        bpy.ops.mesh.primitive_cylinder_add(vertices=24, radius=rad, depth=L, end_fill_type="NOTHING", location=(0, 0, L / 2))
        tube = bpy.context.object
        tube.data.materials.append(mb)
        bpy.ops.mesh.primitive_circle_add(vertices=24, radius=rad, fill_type="NGON")
        cap = bpy.context.object
        cap.data.materials.append(mb)
        rim = _cyl(rad * 1.06, rad * 1.06, L - 0.02 * k, L, mr, 24)
        strap = _box(0, -rad * 1.02, L * 0.5, 0.04 * k, 0.006 * k, L * 0.95, mb)
        o = _join([tube, cap, rim, strap], "quiver")
        sol = o.modifiers.new("Solidify", "SOLIDIFY")
        sol.thickness = 0.004
        bpy.ops.object.modifier_apply(modifier=sol.name)
        return o
    raise ValueError(f"No parametric builder for {cls}")


def export_prop(obj, path):
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.ops.export_scene.gltf(filepath=str(path), export_format="GLB", use_selection=True, export_animations=False)


def ensure_functional(path, name, cls, ch, length_m=None, anchors=None, work=None, keep=False):
    """Load a prop; if it fails intake, build a parametric one styled like it and load that instead."""
    prop = F.Prop(path, name, cls, length_m, ch.s, anchors)
    pw = ch.hand_frame("r" if "r" in ch.fingers else "l")["width"]
    report = intake(prop, pw, ch.s)
    report["source"] = str(path)
    if report["pass"] or keep or work is None:
        report["used"] = "as generated" if report["pass"] else "as generated (failed intake, kept on request)"
        return prop, report
    style = style_from(prop)
    bpy.data.objects.remove(prop.obj)
    obj = build(cls, ch.height_m, pw / ch.s, style)
    out = Path(work) / f"parametric_{name}.glb"
    export_prop(obj, out)
    bpy.data.objects.remove(obj)
    prop = F.Prop(out, name, cls, None, ch.s, None)
    report["used"] = (f"replaced by a parametric {cls}: the generated one failed intake (" + "; ".join(report["reasons"])
                      + "); colours and proportions were taken from it")
    report["style"] = style
    report["after"] = intake(prop, pw, ch.s)
    return prop, report


from pathlib import Path  # noqa: E402


def to_file_frame(prop, v, is_axis):
    """Prop-local scene-unit anchor -> the GLB's own frame in metres (inverse of the centring and scaling)."""
    v = np.asarray(v, float)
    return v if is_axis else v / prop.scale + prop.centre


def cmd_prop_intake(job):
    """Intake + anchors for one prop file (no character): what a library catalog stores."""
    H = job.get("height_m", 1.8)
    pw = job.get("palm_width_m") or 0.085 * H / 1.8
    prop = F.Prop(job["prop"], "prop", job["class"], job.get("length_m"), 1.0, None)
    rep = intake(prop, pw, 1.0)
    rep["anchors_file_frame_m"] = {k: to_file_frame(prop, v, k.endswith("axis")).tolist() for k, v in prop.anchors.items()
                                   if isinstance(v, np.ndarray)}
    rep["scalars_m"] = {k: float(v) / (prop.scale if prop.scale else 1) for k, v in prop.anchors.items() if not isinstance(v, np.ndarray)}
    return rep


def cmd_prop(job):
    """A parametric prop for a character height, styled like a generated one when given."""
    H = job.get("height_m", 1.8)
    pw = job.get("palm_width_m") or 0.085 * H / 1.8
    style, like_report = None, None
    if job.get("like"):
        like = F.Prop(job["like"], "like", job["class"], job.get("like_length_m"), 1.0, None)
        like_report = intake(like, pw, 1.0)
        style = style_from(like)
        bpy.data.objects.remove(like.obj)
    obj = build(job["class"], H, pw, style)
    export_prop(obj, job["output"])
    bpy.data.objects.remove(obj)
    built = F.Prop(job["output"], "built", job["class"], None, 1.0, None)
    after = intake(built, pw, 1.0)
    words = "parametric " + job["class"] + (" styled like " + str(job["like"]) if job.get("like") else "")
    if like_report is not None:
        words += ("; the generated prop " + ("passed" if like_report["pass"] else "FAILED") + " intake"
                  + ("" if like_report["pass"] else ": " + "; ".join(like_report["reasons"])))
    return {"summary": words, "generated_intake": like_report, "parametric_intake": after, "style": style,
            "height_m": H, "palm_width_m": pw}


COMMANDS = {"prop": cmd_prop, "prop_intake": cmd_prop_intake}
