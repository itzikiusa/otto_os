"""school-kit.glb — every classroom/corridor prop as a top-level node.

All nodes sit at the world origin (the runtime clones and places them); the
names, origins and facings follow CONTRACT.md.  Geometry is modelled here with
bevelled primitives for a soft, toy-like look; CC0 props from Quaternius
(plant, backpack, trash can, books, bottle) and CreativeTrio (corkboard) are
imported from base/props, and the wood is a Poly Haven texture.
"""

import math
import os
import random

import bpy
from mathutils import Vector

from . import common as C
from .common import G, box, cyl, plane, sphere, join, mat
from .dims import DESK_Y, KB_Z, SEAT_Y

T = lambda n: os.path.join(C.TEX, n)  # noqa: E731


# ---------------------------------------------------------------- palette

def palette():
    P = {}
    P["desk"] = mat("DeskTop", (0.30, 0.56, 0.92), rough=0.38)
    P["desk_edge"] = mat("DeskEdge", (0.93, 0.95, 0.98), rough=0.4)
    P["metal"] = mat("Metal", (0.72, 0.75, 0.80), rough=0.32, metal=0.75)
    P["metal_dark"] = mat("MetalDark", (0.22, 0.24, 0.28), rough=0.4, metal=0.5)
    P["panel"] = mat("DeskPanel", (0.80, 0.88, 0.97), rough=0.5)
    P["chair"] = mat("ChairShell", (0.12, 0.33, 0.78), rough=0.42)
    P["rubber"] = mat("Rubber", (0.10, 0.10, 0.12), rough=0.8)
    P["bezel"] = mat("MonitorBezel", (0.13, 0.14, 0.17), rough=0.35)
    P["case"] = mat("MonitorCase", (0.90, 0.91, 0.93), rough=0.4)
    P["key"] = mat("KeyCap", (0.96, 0.96, 0.97), rough=0.45)
    P["kb"] = mat("KeyboardBase", (0.55, 0.58, 0.64), rough=0.45)
    P["pad"] = mat("MousePad", (0.15, 0.22, 0.42), rough=0.85)
    P["wood"] = mat("WoodLight", (1, 1, 1), rough=0.5, tex=T("wood_oak.jpg"), tint=(1.0, 0.93, 0.85))
    P["wood_dark"] = mat("WoodDark", (1, 1, 1), rough=0.55, tex=T("wood_oak.jpg"), tint=(0.62, 0.45, 0.33))
    P["white"] = mat("TrimWhite", (0.95, 0.95, 0.94), rough=0.45)
    P["glass_dark"] = mat("GlassDark", (0.20, 0.32, 0.45), rough=0.1, metal=0.2)
    return P


# ---------------------------------------------------------------- pieces

def build_workstation(P):
    parts = []
    # Desk top: blue laminate over a thin white edge band.
    parts.append(box("ws_top", (1.2, 0.03, 0.6), (0, DESK_Y - 0.015, 0), P["desk"], bevel=0.012, seg=3))
    parts.append(box("ws_band", (1.18, 0.02, 0.58), (0, DESK_Y - 0.038, 0), P["desk_edge"], bevel=0.006, seg=1))
    # Frame: four tube legs + side runners + a front modesty panel (far side, +Z).
    for x in (-0.56, 0.56):
        for z in (-0.26, 0.26):
            parts.append(cyl("ws_leg", 0.02, DESK_Y - 0.05, (x, (DESK_Y - 0.05) / 2, z), P["metal"], verts=12))
            parts.append(cyl("ws_foot", 0.024, 0.02, (x, 0.01, z), P["rubber"], verts=12))
        parts.append(box("ws_runner", (0.03, 0.03, 0.52), (x, 0.12, 0), P["metal"], bevel=0.008, seg=1))
        parts.append(box("ws_apron", (0.025, 0.06, 0.52), (x, DESK_Y - 0.08, 0), P["metal"], bevel=0.006, seg=1))
    parts.append(box("ws_modesty", (1.08, 0.30, 0.015), (0, DESK_Y - 0.21, 0.255), P["panel"], bevel=0.006, seg=1))
    # Monitor (faces −Z toward the kid): base, neck, bezel shell, back hump.
    mz = 0.12
    parts.append(box("mon_base", (0.24, 0.014, 0.17), (0, DESK_Y + 0.007, mz + 0.03), P["case"], bevel=0.006, seg=2))
    parts.append(box("mon_neck", (0.06, 0.16, 0.03), (0, DESK_Y + 0.09, mz + 0.06), P["case"], bevel=0.01, seg=2))
    body_y = DESK_Y + 0.31
    parts.append(box("mon_body", (0.56, 0.37, 0.045), (0, body_y, mz), P["bezel"], bevel=0.016, seg=3))
    parts.append(box("mon_back", (0.42, 0.25, 0.05), (0, body_y - 0.01, mz + 0.035), P["case"], bevel=0.02, seg=3))
    # Keyboard: base + 4 rows of keycaps + spacebar.
    kz, ky = KB_Z, DESK_Y
    parts.append(box("kb_base", (0.44, 0.016, 0.15), (0, ky + 0.008, kz), P["kb"], bevel=0.006, seg=2))
    pitch = 0.031
    for r in range(4):
        z = kz + 0.045 - r * 0.028  # row 0 = far row
        n = 13 - (r % 2)
        x0 = -(n - 1) * pitch / 2
        for i in range(n):
            parts.append(box("key", (0.025, 0.008, 0.022), (x0 + i * pitch, ky + 0.019, z), P["key"]))
    parts.append(box("key_space", (0.19, 0.008, 0.022), (0, ky + 0.019, kz - 0.067), P["key"]))
    # Mouse on a pad, on the kid's right (−X when facing +Z).
    parts.append(box("pad", (0.20, 0.004, 0.17), (-0.36, ky + 0.002, kz + 0.01), P["pad"], bevel=0.002, seg=1))
    parts.append(sphere("mouse", 1.0, (-0.36, ky + 0.012, kz + 0.01), P["case"], seg=12, rings=7,
                        scale=(0.032, 0.017, 0.05)))
    # Pencil cup (far right corner).
    pc = mat("PencilCup", (0.98, 0.72, 0.25), rough=0.5)
    parts.append(cyl("cup", 0.035, 0.09, (0.46, DESK_Y + 0.045, 0.16), pc, verts=12, bevel=0.004))
    for i, (dx, dz, col) in enumerate([(-0.012, 0.0, (0.95, 0.35, 0.3)), (0.012, 0.008, (0.25, 0.65, 0.35)),
                                       (0.0, -0.012, (0.3, 0.5, 0.95))]):
        m = mat(f"Pencil{i}", col, rough=0.5)
        parts.append(cyl("pencil", 0.006, 0.15, (0.46 + dx, DESK_Y + 0.10, 0.16 + dz), m, verts=6))
    ws = join("Workstation", parts)

    # Screen: the live display plane (16:10), own material, UV as seen by the kid.
    sm = mat("ScreenMat", (0, 0, 0), rough=0.25, emit_tex=T("ph_screen.jpg"))
    _screen_base_from_emission(sm)
    sw, sh = 0.50, 0.3125
    sy = body_y - 0.185 + 0.04 + sh / 2
    screen = plane("Screen", sw, sh, (0, sy, mz - 0.0235), sm, facing="-z", painted=True)
    C.set_origin(screen, (0, sy, mz - 0.0235))
    C.parent(screen, ws)
    return ws


def _screen_base_from_emission(m):
    """Screens glow: export the map as both baseColor and emissive so three.js
    renders it readable under any lighting, and the runtime's `material.map`
    swap keeps working (it may also set `emissiveMap`)."""
    nt = m.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    te = [n for n in nt.nodes if n.type == "TEX_IMAGE"][0]
    nt.links.new(te.outputs["Color"], bsdf.inputs["Base Color"])
    bsdf.inputs["Emission Strength"].default_value = 0.85


def build_chair(P):
    parts = []
    parts.append(box("seat", (0.42, 0.04, 0.40), (0, SEAT_Y - 0.02, 0.01), P["chair"], bevel=0.016, seg=3))
    parts.append(box("back", (0.40, 0.24, 0.028), (0, SEAT_Y + 0.31, -0.19), P["chair"], bevel=0.012, seg=3,
                     rot=(-8, 0, 0)))
    for x in (-0.17, 0.17):
        parts.append(box("post", (0.022, 0.30, 0.022), (x, SEAT_Y + 0.13, -0.185), P["metal"], bevel=0.008,
                         seg=1, rot=(-6, 0, 0)))
        for z in (-0.16, 0.16):
            parts.append(cyl("leg", 0.014, SEAT_Y - 0.04, (x, (SEAT_Y - 0.04) / 2, z), P["metal"], verts=10))
            parts.append(cyl("cap", 0.018, 0.015, (x, 0.0075, z), P["rubber"], verts=10))
        parts.append(box("rail", (0.02, 0.02, 0.34), (x, SEAT_Y - 0.06, 0), P["metal"], bevel=0.006, seg=1))
    parts.append(box("brace", (0.36, 0.02, 0.02), (0, 0.14, -0.16), P["metal"], bevel=0.006, seg=1))
    return join("Chair", parts)


def build_teacher_desk(P):
    parts = []
    top_y = 0.78
    parts.append(box("td_top", (1.6, 0.045, 0.8), (0, top_y - 0.0225, 0), P["wood"], bevel=0.014, seg=3))
    # Right pedestal with three drawers (seen from the teacher side −Z and front +Z).
    parts.append(box("td_ped", (0.46, top_y - 0.07, 0.72), (0.53, (top_y - 0.07) / 2 + 0.02, 0), P["wood_dark"],
                     bevel=0.01, seg=2))
    for i in range(3):
        y = 0.16 + i * 0.2
        parts.append(box("drawer", (0.40, 0.17, 0.02), (0.53, y, -0.365), P["wood"], bevel=0.008, seg=2))
        parts.append(box("knob", (0.08, 0.018, 0.02), (0.53, y + 0.03, -0.38), P["metal"], bevel=0.006, seg=1))
    parts.append(box("td_side", (0.05, top_y - 0.07, 0.72), (-0.74, (top_y - 0.07) / 2 + 0.02, 0), P["wood_dark"],
                     bevel=0.01, seg=2))
    parts.append(box("td_front", (1.42, 0.52, 0.03), (0, top_y - 0.31, 0.36), P["wood_dark"], bevel=0.01, seg=2))
    parts.append(box("td_plinth", (1.5, 0.03, 0.7), (0, 0.015, 0), P["wood_dark"]))
    # Monitor facing the teacher (−Z).
    parts.append(box("tm_base", (0.22, 0.014, 0.16), (-0.25, top_y + 0.007, 0.12), P["case"], bevel=0.006))
    parts.append(box("tm_neck", (0.05, 0.15, 0.03), (-0.25, top_y + 0.08, 0.15), P["case"], bevel=0.01))
    parts.append(box("tm_body", (0.52, 0.34, 0.04), (-0.25, top_y + 0.31, 0.12), P["bezel"], bevel=0.014, seg=3))
    parts.append(plane("tm_glass", 0.47, 0.28, (-0.25, top_y + 0.32, 0.098),
                       mat("TeacherScreen", (0.05, 0.1, 0.2), rough=0.2, emit=(0.25, 0.55, 0.85),
                           emit_strength=0.6), facing="-z"))
    parts.append(box("tm_kb", (0.40, 0.016, 0.13), (-0.25, top_y + 0.008, -0.2), P["kb"], bevel=0.006))
    # Desk clutter: apple, mug, stacked books, papers.
    apple = mat("Apple", (0.85, 0.12, 0.12), rough=0.35)
    parts.append(sphere("apple", 0.045, (0.45, top_y + 0.045, 0.1), apple, seg=12, rings=8, scale=(1, 0.92, 1)))
    parts.append(cyl("stem", 0.005, 0.03, (0.45, top_y + 0.095, 0.1), P["wood_dark"], verts=6))
    leaf = mat("Leaf", (0.30, 0.68, 0.25), rough=0.5)
    parts.append(sphere("leaf", 1.0, (0.465, top_y + 0.1, 0.1), leaf, seg=8, rings=4, scale=(0.022, 0.004, 0.01)))
    mug = mat("Mug", (0.98, 0.98, 0.96), rough=0.3)
    parts.append(cyl("mug", 0.04, 0.09, (0.25, top_y + 0.045, -0.2), mug, verts=14, bevel=0.004))
    parts.append(cyl("coffee", 0.034, 0.002, (0.25, top_y + 0.085, -0.2),
                     mat("Coffee", (0.25, 0.13, 0.06), rough=0.15), verts=14))
    cols = [(0.85, 0.3, 0.3), (0.3, 0.55, 0.9), (0.95, 0.75, 0.3)]
    for i, c in enumerate(cols):
        parts.append(box("tbook", (0.24 - i * 0.02, 0.04, 0.17), (0.48, top_y + 0.02 + i * 0.04, -0.15),
                         mat(f"Book{i}", c, rough=0.6), bevel=0.004, seg=1, rot=(0, 8 * i - 6, 0)))
    parts.append(box("papers", (0.22, 0.01, 0.30), (0.05, top_y + 0.005, -0.15), mat("Paper", (0.98, 0.98, 0.96)),
                     rot=(0, 12, 0)))
    return join("TeacherDesk", parts)


def build_board(P):
    """Board origin: x-centre, floor level (y = 0), wall surface (z = 0);
    the board spans y 0.9 … 2.3 and faces +Z."""
    parts = []
    W, H, y0 = 4.0, 1.4, 0.9
    yc = y0 + H / 2
    parts.append(box("bd_back", (W - 0.04, H - 0.04, 0.025), (0, yc, 0.0125), mat("BoardBack", (0.16, 0.22, 0.18))))
    fr = P["wood"]
    t = 0.07
    parts.append(box("bd_ft", (W, t, 0.05), (0, y0 + H - t / 2, 0.025), fr, bevel=0.014, seg=2))
    parts.append(box("bd_fb", (W, t, 0.05), (0, y0 + t / 2, 0.025), fr, bevel=0.014, seg=2))
    parts.append(box("bd_fl", (t, H - 2 * t, 0.05), (-W / 2 + t / 2, yc, 0.025), fr, bevel=0.014, seg=2))
    parts.append(box("bd_fr", (t, H - 2 * t, 0.05), (W / 2 - t / 2, yc, 0.025), fr, bevel=0.014, seg=2))
    iw, ih = W - 2 * t, H - 2 * t
    chalk = mat("BoardChalkArt", (1, 1, 1), rough=0.85, tex=T("chalk_art.jpg"))
    blank = mat("BoardBlank", (1, 1, 1), rough=0.85, tex=T("board_blank.jpg"))
    parts.append(plane("bd_left", iw / 2, ih, (-iw / 4, yc, 0.026), chalk))
    parts.append(plane("bd_right", iw / 2, ih, (iw / 4, yc, 0.026), blank))
    # Chalk tray + chalk + eraser.
    parts.append(box("tray", (W - 0.5, 0.025, 0.08), (0, y0 - 0.005, 0.06), fr, bevel=0.008, seg=2))
    parts.append(box("tray_lip", (W - 0.5, 0.03, 0.012), (0, y0 + 0.01, 0.095), fr, bevel=0.005, seg=1))
    for i, (x, col) in enumerate([(-1.2, (0.98, 0.98, 0.96)), (-1.1, (0.98, 0.9, 0.45)), (0.6, (0.98, 0.6, 0.7)),
                                  (0.7, (0.98, 0.98, 0.96)), (1.4, (0.6, 0.8, 0.98))]):
        parts.append(cyl("chalk", 0.008, 0.08, (x, y0 + 0.018, 0.055), mat(f"Chalk{i}", col, rough=0.9),
                         axis="x", verts=8))
    parts.append(box("eraser_felt", (0.16, 0.025, 0.055), (-0.3, y0 + 0.02, 0.055), mat("Felt", (0.35, 0.35, 0.38),
                     rough=0.95), bevel=0.004, seg=1))
    parts.append(box("eraser_wood", (0.16, 0.03, 0.055), (-0.3, y0 + 0.047, 0.055), P["wood"], bevel=0.006, seg=2))
    board = join("Board", parts)
    bt_mat = mat("BoardTitleMat", (1, 1, 1), rough=0.85, tex=T("ph_board_title.png"))
    ty = y0 + H - t - 0.04 - 0.175
    title = plane("BoardTitle", 1.8, 0.35, (iw / 4, ty, 0.028), bt_mat, painted=True)
    C.set_origin(title, (iw / 4, ty, 0.028))
    C.parent(title, board)
    return board


def build_bookshelf(P):
    rng = random.Random(7)
    parts = []
    W, H, D = 1.2, 1.8, 0.4
    wd = P["wood"]
    parts.append(box("bs_l", (0.035, H, D), (-W / 2 + 0.0175, H / 2, 0), wd, bevel=0.008, seg=2))
    parts.append(box("bs_r", (0.035, H, D), (W / 2 - 0.0175, H / 2, 0), wd, bevel=0.008, seg=2))
    parts.append(box("bs_top", (W, 0.035, D), (0, H - 0.0175, 0), wd, bevel=0.008, seg=2))
    parts.append(box("bs_back", (W - 0.06, H - 0.06, 0.012), (0, H / 2, -D / 2 + 0.006),
                     mat("ShelfBack", (0.95, 0.88, 0.76), rough=0.7)))
    parts.append(box("bs_plinth", (W - 0.07, 0.07, D - 0.04), (0, 0.035, 0.0), P["wood_dark"]))
    shelf_ys = [0.07, 0.48, 0.89, 1.30]
    for y in shelf_ys[1:]:
        parts.append(box("shelf", (W - 0.07, 0.025, D - 0.03), (0, y - 0.0125, 0.0), wd, bevel=0.006, seg=1))
    cols = [(0.89, 0.33, 0.30), (0.30, 0.55, 0.90), (0.98, 0.76, 0.28), (0.36, 0.72, 0.45), (0.62, 0.42, 0.82),
            (0.97, 0.55, 0.30), (0.25, 0.70, 0.75), (0.95, 0.95, 0.92), (0.20, 0.30, 0.55), (0.90, 0.50, 0.65)]
    bmats = [mat(f"BookCover{i}", c, rough=0.55) for i, c in enumerate(cols)]
    page = mat("BookPages", (0.97, 0.95, 0.88), rough=0.8)
    for si, y in enumerate(shelf_ys):
        x = -W / 2 + 0.05
        end = W / 2 - 0.05
        stack_at = rng.choice([1, 2, 3]) if si != 3 else None
        k = 0
        while x < end - 0.03:
            k += 1
            if stack_at is not None and k == 6 + stack_at:  # a little lying stack
                w = 0.2
                if x + w > end:
                    break
                for j in range(3):
                    parts.append(box("bstk", (w - j * 0.02, 0.035, 0.22 - j * 0.01), (x + w / 2, y + 0.0175 + j * 0.035,
                                     0.02), rng.choice(bmats), bevel=0.004, seg=1))
                x += w + 0.02
                continue
            if rng.random() < 0.08:  # gap
                x += 0.06
                continue
            bw = rng.uniform(0.028, 0.055)
            bh = rng.uniform(0.24, 0.34)
            bd = rng.uniform(0.21, 0.27)
            if x + bw > end:
                break
            tilt = 0
            if rng.random() < 0.07 and x + bw + 0.06 < end:
                tilt = -14
                x += 0.035
            parts.append(box("book", (bw, bh, bd), (x + bw / 2, y + bh / 2 + 0.002, 0.03 - (0.25 - bd) / 2),
                             rng.choice(bmats), bevel=0.004, seg=1, rot=(0, 0, tilt)))
            parts.append(box("bpage", (bw * 0.8, bh * 0.92, 0.004), (x + bw / 2, y + bh / 2 + 0.002,
                             0.03 - (0.25 - bd) / 2 + bd / 2 + 0.001), page, rot=(0, 0, tilt)))
            x += bw + 0.004
    # Globe on top.
    parts.append(cyl("globe_base", 0.07, 0.02, (-0.3, H + 0.01, 0), P["wood_dark"], verts=14, bevel=0.005))
    parts.append(cyl("globe_post", 0.008, 0.07, (-0.3, H + 0.055, 0), P["metal"], verts=6))
    parts.append(sphere("globe", 0.11, (-0.3, H + 0.19, 0), mat("Globe", (0.30, 0.62, 0.92), rough=0.35), seg=16,
                        rings=10))
    land = mat("GlobeLand", (0.40, 0.75, 0.35), rough=0.5)
    for (a, b, s) in [(0.3, 0.2, 0.05), (2.0, -0.3, 0.06), (3.6, 0.5, 0.045), (4.9, -0.1, 0.04)]:
        p = Vector((math.cos(a) * math.cos(b), math.sin(b), math.sin(a) * math.cos(b))) * 0.1
        parts.append(sphere("land", 1.0, (-0.3 + p.x, H + 0.19 + p.y, p.z), land, seg=8, rings=5,
                            scale=(s, s * 0.8, s)))
    # Two upright books + a small cactus on top.
    parts.append(box("tb1", (0.05, 0.22, 0.17), (0.25, H + 0.11, 0), bmats[1], bevel=0.004, seg=1))
    parts.append(box("tb2", (0.04, 0.2, 0.16), (0.30, H + 0.10, 0), bmats[2], bevel=0.004, seg=1))
    return join("Bookshelf", parts)


def import_prop(file, name, height=None, width=None, depth=None, yaw=0.0, recolour=None, smooth=False):
    """Import a CC0 prop .glb from base/props, join it, scale it to a target
    height/width/depth (metres), turn it (glTF yaw, degrees) so its front
    faces +Z, put its origin at the footprint centre on the floor."""
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=os.path.join(C.BASE, "props", file))
    new = [o for o in bpy.data.objects if o not in before]
    meshes = [o for o in new if o.type == "MESH"]
    bpy.context.view_layer.update()
    for o in meshes:
        mw = o.matrix_world.copy()
        o.parent = None
        o.matrix_world = mw
    for o in new:
        if o.type != "MESH":
            bpy.data.objects.remove(o)
    ob = join(name, meshes)
    bpy.context.view_layer.objects.active = ob
    bpy.ops.object.select_all(action="DESELECT")
    ob.select_set(True)
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    me = ob.data
    if yaw:
        me.transform(C.grot(0, yaw, 0))
    co = [v.co for v in me.vertices]
    lo = Vector([min(c[i] for c in co) for i in range(3)])
    hi = Vector([max(c[i] for c in co) for i in range(3)])
    ext = hi - lo  # Blender axes: x = width, y = depth, z = height
    s = 1.0
    if height:
        s = height / ext.z
    elif width:
        s = width / ext.x
    elif depth:
        s = depth / ext.y
    cx, cy = (hi.x + lo.x) / 2, (hi.y + lo.y) / 2
    for v in me.vertices:
        v.co = Vector(((v.co.x - cx) * s, (v.co.y - cy) * s, (v.co.z - lo.z) * s))
    for sl in ob.material_slots:
        m = sl.material
        if m is None or not m.use_nodes:
            continue
        b = m.node_tree.nodes.get("Principled BSDF")
        if b and recolour and m.name.split(".")[0] in recolour:
            b.inputs["Base Color"].default_value = C.lin3(recolour[m.name.split(".")[0]]) + (1.0,)
    for p in me.polygons:
        p.use_smooth = smooth
    return ob


def build_plant(P):
    """Quaternius "Houseplant" (snake plant, CC0) at ≈1.0 m."""
    return import_prop("houseplant2.glb", "Plant", height=1.0,
                       recolour={"Plant_Green": (0.30, 0.66, 0.34), "Brown": (0.84, 0.47, 0.30),
                                 "Black": (0.92, 0.90, 0.86)})


def build_extras(P):
    """Optional decor (not in the contract table; the runtime may ignore them).
    All Quaternius / CreativeTrio CC0, see LICENSES.md."""
    out = [
        import_prop("backpack.glb", "Backpack", height=0.42,
                    recolour={"Green": (0.95, 0.45, 0.35), "LightGreen": (0.98, 0.72, 0.30),
                              "Brown": (0.28, 0.32, 0.45), "Gold": (0.92, 0.92, 0.94)}),
        import_prop("trash.glb", "Trashcan", height=0.5,
                    recolour={"LightMetal": (0.45, 0.70, 0.95), "Black": (0.20, 0.22, 0.27)}),
        import_prop("houseplant.glb", "Plant_Desk", height=0.32,
                    recolour={"Black": (0.96, 0.55, 0.35), "Brown": (0.40, 0.28, 0.20)}),
        import_prop("book.glb", "Book", height=0.24,
                    recolour={"DarkRed": (0.30, 0.52, 0.92), "Golden": (0.98, 0.84, 0.36)}),
        import_prop("openbook.glb", "Book_Open", width=0.34),
        import_prop("bottle.glb", "Water_Bottle", height=0.22,
                    recolour={"Red": (0.26, 0.70, 0.55)}),
        import_prop("corkboard.glb", "Corkboard", width=1.1),
    ]
    # Corkboard hangs on a wall: origin at the wall-surface centre, facing +Z.
    cb = out[-1]
    zs = [v.co.z for v in cb.data.vertices]
    ys = [v.co.y for v in cb.data.vertices]
    mid = (max(zs) + min(zs)) / 2
    back = max(ys)  # Blender +Y = glTF −Z (the wall side)
    for v in cb.data.vertices:
        v.co.z -= mid
        v.co.y -= back
    return out


def build_clock(P):
    """Wall clock; origin at the wall-surface centre, face +Z. Hands are child
    nodes pivoting at the centre (rotate about Z), set to 10:10."""
    parts = []
    rim = mat("ClockRim", (0.95, 0.42, 0.32), rough=0.4)
    parts.append(cyl("rim", 0.2, 0.05, (0, 0, 0.025), rim, axis="z", verts=32, bevel=0.012, seg=3))
    face = mat("ClockFace", (0.99, 0.98, 0.95), rough=0.5)
    parts.append(cyl("face", 0.172, 0.004, (0, 0, 0.051), face, axis="z", verts=32))
    tick = mat("ClockTick", (0.15, 0.17, 0.22), rough=0.5)
    for i in range(12):
        a = i * math.pi / 6
        big = i % 3 == 0
        r = 0.145
        parts.append(box("tick", (0.012 if big else 0.007, 0.03 if big else 0.018, 0.004),
                         (math.sin(a) * r, math.cos(a) * r, 0.055), tick, rot=(0, 0, -math.degrees(a))))
    parts.append(cyl("cap", 0.012, 0.012, (0, 0, 0.062), rim, axis="z", verts=12))
    clock = join("Clock", parts)
    hands = []
    # Blender +Y rotation = glTF −Z rotation: 10:10 → hour +55° (CCW), minute −60°.
    for name, ln, w, ang, z in [("Clock_HourHand", 0.085, 0.014, -55, 0.056),
                                ("Clock_MinuteHand", 0.13, 0.009, 60, 0.058)]:
        h = box(name, (w, ln, 0.003), (0, ln / 2 - 0.015, z), tick, bevel=0.003, seg=1)
        C.set_origin(h, (0, 0, z))
        h.rotation_mode = "XYZ"
        h.rotation_euler = (0, math.radians(ang), 0)
        C.parent(h, clock)
        hands.append(h)
    return clock


def build_bench(P):
    """Detention bench, front +Z. Seat top at SEAT_Y (same as the Chair) so
    the kids' Sit_* clips sit flush on it."""
    parts = []
    top = SEAT_Y
    for i, z in enumerate((-0.12, 0.0, 0.12)):
        parts.append(box("slat", (1.6, 0.035, 0.11), (0, top - 0.0175, z), P["wood"], bevel=0.01, seg=2))
    lh = top - 0.035
    for x in (-0.65, 0.65):
        parts.append(box("leg_f", (0.04, lh, 0.04), (x, lh / 2, 0.15), P["metal_dark"], bevel=0.008, seg=1))
        parts.append(box("leg_b", (0.04, lh, 0.04), (x, lh / 2, -0.15), P["metal_dark"], bevel=0.008, seg=1))
        parts.append(box("leg_t", (0.05, 0.03, 0.38), (x, lh - 0.01, 0), P["metal_dark"], bevel=0.008, seg=1))
        parts.append(box("leg_r", (0.035, 0.03, 0.34), (x, 0.08, 0), P["metal_dark"], bevel=0.006, seg=1))
    return join("Bench", parts)


def build_poster(P, i, frame_col):
    parts = []
    fm = mat(f"PosterFrame{i}", frame_col, rough=0.45)
    parts.append(box("pf", (0.7, 1.0, 0.02), (0, 0, 0.01), fm, bevel=0.008, seg=2))
    pm = mat(f"Poster{i}Mat", (1, 1, 1), rough=0.6, tex=T(f"poster_{i}.jpg"))
    parts.append(plane("pp", 0.64, 0.94, (0, 0, 0.0205), pm))
    return join(f"Poster_{i}", parts)


def _floor(name, tex_name, per_m):
    m = mat(f"{name}Mat", (1, 1, 1), rough=0.45, tex=T(tex_name))
    return box(name, (2.0, 0.02, 2.0), (0, -0.01, 0), m, uv_scale=per_m, uv_origin=(-1, 0, 1))


def _wall_paint():
    upper = mat("WallUpper", (1, 1, 1), rough=0.85, tex=T("wall_paint.jpg"))
    lower = mat("WallLower", (1, 1, 1), rough=0.8, tex=T("plaster.jpg"), tint=(0.47, 0.66, 0.90))
    return upper, lower


def _corridor_paint():
    upper = mat("CorridorUpper", (1, 1, 1), rough=0.85, tex=T("plaster.jpg"), tint=(1.0, 0.95, 0.84))
    lower = mat("CorridorLower", (1, 1, 1), rough=0.8, tex=T("plaster.jpg"), tint=(0.42, 0.70, 0.68))
    return upper, lower


def _wall_slab(name, x0, x1, y0, y1, m, D=0.15):
    return box(name, (x1 - x0, y1 - y0, D), ((x0 + x1) / 2, (y0 + y1) / 2, -D / 2), m, uv_scale=0.5,
               uv_origin=(-1, 0, 0))


def _wall_with_hole(P, upper, lower, hole=None, rail=0.95):
    """Wall 2×3×0.15 (inside face z = 0) split at the chair rail, minus an
    optional rectangular hole (x0, x1, y0, y1)."""
    parts = []
    W, H = 2.0, 3.0
    spans = [(-W / 2, W / 2, 0, H)]
    if hole:
        hx0, hx1, hy0, hy1 = hole
        spans = [(-W / 2, hx0, 0, H), (hx1, W / 2, 0, H), (hx0, hx1, 0, hy0), (hx0, hx1, hy1, H)]
    for (x0, x1, y0, y1) in spans:
        if x1 - x0 < 1e-4 or y1 - y0 < 1e-4:
            continue
        if y0 < rail < y1:
            parts.append(_wall_slab("wl", x0, x1, y0, rail, lower))
            parts.append(_wall_slab("wu", x0, x1, rail, y1, upper))
        else:
            parts.append(_wall_slab("w", x0, x1, y0, y1, lower if y1 <= rail + 1e-6 else upper))
    # Skirting + chair rail (interrupted by the hole).
    trims = [(-W / 2, W / 2)]
    if hole and hole[2] < rail:
        trims = [(-W / 2, hole[0]), (hole[1], W / 2)]
    for (x0, x1) in trims:
        if x1 - x0 > 0.01:
            parts.append(box("rail", (x1 - x0, 0.05, 0.025), ((x0 + x1) / 2, rail, 0.0125), P["white"], bevel=0.008,
                             seg=2))
    sk = [(-W / 2, W / 2)]
    if hole and hole[2] < 0.1:
        sk = [(-W / 2, hole[0]), (hole[1], W / 2)]
    for (x0, x1) in sk:
        if x1 - x0 > 0.01:
            parts.append(box("skirt", (x1 - x0, 0.1, 0.02), ((x0 + x1) / 2, 0.05, 0.01), P["white"], bevel=0.006,
                             seg=1))
    return parts


def build_wall(P):
    up, lo = _wall_paint()
    return join("Wall", _wall_with_hole(P, up, lo))


def build_wall_window(P):
    up, lo = _wall_paint()
    hx0, hx1, hy0, hy1 = -0.7, 0.7, 0.95, 2.45
    parts = _wall_with_hole(P, up, lo, (hx0, hx1, hy0, hy1))
    wf = P["white"]
    t = 0.07
    # Frame (sits inside the opening, proud of the inside face).
    parts.append(box("wf_t", (hx1 - hx0 + 0.08, t, 0.2), (0, hy1 - t / 2 + 0.04, -0.055), wf, bevel=0.012, seg=2))
    parts.append(box("wf_b", (hx1 - hx0 + 0.08, t, 0.2), (0, hy0 + t / 2 - 0.04, -0.055), wf, bevel=0.012, seg=2))
    parts.append(box("wf_l", (t, hy1 - hy0, 0.2), (hx0 + t / 2 - 0.04, (hy0 + hy1) / 2, -0.055), wf, bevel=0.012,
                     seg=2))
    parts.append(box("wf_r", (t, hy1 - hy0, 0.2), (hx1 - t / 2 + 0.04, (hy0 + hy1) / 2, -0.055), wf, bevel=0.012,
                     seg=2))
    parts.append(box("mull_v", (0.04, hy1 - hy0, 0.05), (0, (hy0 + hy1) / 2, -0.08), wf, bevel=0.008, seg=1))
    parts.append(box("mull_h", (hx1 - hx0, 0.04, 0.05), (0, hy0 + (hy1 - hy0) * 0.66, -0.08), wf, bevel=0.008, seg=1))
    parts.append(box("sill", (hx1 - hx0 + 0.2, 0.04, 0.16), (0, hy0 - 0.02, 0.03), wf, bevel=0.012, seg=2))
    # Outdoor view: an emissive backdrop behind the glass line.
    vm = mat("WindowView", (0, 0, 0), rough=1.0, emit_tex=T("window_view.jpg"), emit_strength=1.0)
    parts.append(plane("view", 2.6, 1.75, (0, (hy0 + hy1) / 2 + 0.1, -0.55), vm))
    # Thin glass sheen (very subtle, blended).
    gm = mat("WindowGlass", (0.75, 0.88, 1.0), rough=0.05, alpha=0.12)
    parts.append(plane("glass", hx1 - hx0, hy1 - hy0, (0, (hy0 + hy1) / 2, -0.1), gm))
    return join("Wall_Window", parts)


def _door(P, name, upper, lower, dx0, dx1, door_mat, glass=True):
    dh = 2.1
    parts = _wall_with_hole(P, upper, lower, (dx0, dx1, 0.0, dh))
    cas = P["white"]
    parts.append(box("cas_t", (dx1 - dx0 + 0.16, 0.08, 0.04), ((dx0 + dx1) / 2, dh + 0.04, 0.02), cas, bevel=0.01,
                     seg=2))
    parts.append(box("cas_l", (0.08, dh, 0.04), (dx0 - 0.04, dh / 2, 0.02), cas, bevel=0.01, seg=2))
    parts.append(box("cas_r", (0.08, dh, 0.04), (dx1 + 0.04, dh / 2, 0.02), cas, bevel=0.01, seg=2))
    parts.append(box("jamb_l", (0.02, dh, 0.15), (dx0 + 0.01, dh / 2, -0.075), cas))
    parts.append(box("jamb_r", (0.02, dh, 0.15), (dx1 - 0.01, dh / 2, -0.075), cas))
    parts.append(box("jamb_t", (dx1 - dx0, 0.02, 0.15), ((dx0 + dx1) / 2, dh - 0.01, -0.075), cas))
    wall = join(name, parts)
    # Door leaf: child, origin on its hinge (−X edge), swings about Y.
    lw = dx1 - dx0 - 0.04
    hz = -0.04
    lp = []
    lp.append(box("leaf", (lw, dh - 0.03, 0.045), (dx0 + 0.02 + lw / 2, (dh - 0.03) / 2 + 0.005, hz), door_mat,
                  bevel=0.01, seg=2))
    if glass:
        lp.append(box("leaf_win", (lw * 0.4, 0.5, 0.05), (dx0 + 0.02 + lw / 2, 1.55, hz), P["glass_dark"],
                      bevel=0.008, seg=1))
    for fz in (hz + 0.04, hz - 0.04):
        lp.append(box("handle", (0.12, 0.022, 0.022), (dx1 - 0.13, 1.0, fz), P["metal"], bevel=0.008, seg=2))
    lp.append(box("kick", (lw - 0.04, 0.18, 0.05), (dx0 + 0.02 + lw / 2, 0.11, hz), P["metal"], bevel=0.006, seg=1))
    leaf = join("DoorLeaf", lp)
    C.set_origin(leaf, (dx0 + 0.02, 0, hz))
    C.parent(leaf, wall)
    return wall


def build_wall_door(P):
    up, lo = _wall_paint()
    dm = mat("DoorWood", (0.93, 0.68, 0.38), rough=0.5)
    return _door(P, "Wall_Door", up, lo, -0.5, 0.5, dm)


def build_corridor_floor(P):
    return _floor("Corridor_Floor", "corridor_floor.jpg", 0.5)


def build_corridor_wall(P):
    up, lo = _corridor_paint()
    parts = [_wall_slab("cw_upper", -1.0, 1.0, 1.95, 3.0, up)]
    parts.append(_wall_slab("cw_lower", -1.0, 1.0, 0.0, 1.95, lo))
    # Locker bank: 4 lockers across 2 m, shallow (0.14) so it keeps the wall line.
    lk = mat("LockerFront", (1, 1, 1), rough=0.4, metal=0.15, tex=T("lockers.jpg"))
    lb = mat("LockerBody", (0.30, 0.66, 0.94), rough=0.4, metal=0.15)
    parts.append(box("lk_body", (1.98, 1.8, 0.14), (0, 0.1 + 0.9, 0.07), lb, bevel=0.01, seg=1))
    parts.append(plane("lk_front", 1.96, 1.78, (0, 1.0, 0.1405), lk))
    parts.append(box("lk_plinth", (1.98, 0.1, 0.12), (0, 0.05, 0.06), P["metal_dark"]))
    parts.append(box("lk_cap", (2.0, 0.04, 0.16), (0, 1.92, 0.08), lb, bevel=0.008, seg=1))
    parts.append(box("cw_rail", (2.0, 0.05, 0.025), (0, 2.1, 0.0125), P["white"], bevel=0.008, seg=2))
    return join("Corridor_Wall", parts)


def build_corridor_door(P):
    up, lo = _corridor_paint()
    dm = mat("CorridorDoor", (0.32, 0.55, 0.88), rough=0.45)
    wall = _door(P, "Corridor_Door", up, lo, -0.85, 0.15, dm)
    # NamePlate above the door, DoorSign beside it — own materials each.
    backing = mat("PlateBacking", (0.25, 0.30, 0.40), rough=0.5)
    back = box("np_back", (1.26, 0.36, 0.02), (-0.35, 2.48, 0.01), backing, bevel=0.008, seg=2)
    back2 = box("ds_back", (0.56, 0.41, 0.02), (0.6, 1.5, 0.01), backing, bevel=0.008, seg=2)
    w2 = join(wall.name, [wall, back, back2])
    np_m = mat("NamePlateMat", (1, 1, 1), rough=0.5, tex=T("ph_name_plate.png"))
    ds_m = mat("DoorSignMat", (1, 1, 1), rough=0.5, tex=T("ph_door_sign.png"))
    for nm, w, h, c, m in [("NamePlate", 1.2, 0.3, (-0.35, 2.48, 0.0215), np_m),
                           ("DoorSign", 0.5, 0.35, (0.6, 1.5, 0.0215), ds_m)]:
        p = plane(nm, w, h, c, m, painted=True)
        C.set_origin(p, c)
        C.parent(p, w2)
    return w2


def build_ceiling_light(P):
    """Panel light, origin at its footprint centre with the emitting face at
    y = 0 (facing down); the housing rises to y = 0.1."""
    parts = []
    parts.append(box("cl_frame", (1.2, 0.08, 0.3), (0, 0.06, 0), mat("LightFrame", (0.9, 0.91, 0.93), rough=0.35,
                     metal=0.3), bevel=0.01, seg=2))
    parts.append(box("cl_mount", (0.1, 0.02, 0.1), (0, 0.11, 0), P["metal"]))
    em = mat("LightPanel", (1, 1, 1), rough=0.5, emit=(1.0, 0.98, 0.92), emit_strength=3.0)
    parts.append(box("cl_diffuser", (1.14, 0.02, 0.25), (0, 0.012, 0), em, bevel=0.004, seg=1))
    return join("Ceiling_Light", parts)


# ---------------------------------------------------------------- build

NODES = ["Workstation", "Chair", "TeacherDesk", "Board", "Bookshelf", "Plant", "Clock", "Bench",
         "Poster_1", "Poster_2", "Poster_3", "Poster_4", "Poster_5", "Poster_6",
         "Floor_Tile", "Wall", "Wall_Window", "Wall_Door", "Corridor_Floor", "Corridor_Wall",
         "Corridor_Door", "Ceiling_Light"]
EXTRAS = ["Backpack", "Trashcan", "Plant_Desk", "Book", "Book_Open", "Water_Bottle", "Corkboard"]


def build_all():
    """Builds every kit node into the current (empty) scene; returns them."""
    P = palette()
    out = [
        build_workstation(P), build_chair(P), build_teacher_desk(P), build_board(P), build_bookshelf(P),
        build_plant(P), build_clock(P), build_bench(P),
    ]
    frames = [(0.95, 0.95, 0.94), (0.98, 0.70, 0.30), (0.95, 0.95, 0.94), (0.40, 0.62, 0.95), (0.95, 0.95, 0.94),
              (0.55, 0.78, 0.50)]
    for i in range(1, 7):
        out.append(build_poster(P, i, frames[i - 1]))
    out.append(_floor("Floor_Tile", "floor_tiles.jpg", 0.5))
    out += [build_wall(P), build_wall_window(P), build_wall_door(P), build_corridor_floor(P),
            build_corridor_wall(P), build_corridor_door(P), build_ceiling_light(P)]
    out += build_extras(P)
    for o in out:
        assert o.parent is None, o.name
    names = [o.name for o in out]
    assert names == NODES + EXTRAS, names
    return out


def build(out_path):
    import re
    from . import glbinfo
    C.reset()
    nodes = build_all()
    allobjs = [o for o in bpy.data.objects]
    C.export_glb(out_path, objects=allobjs, animations=False)
    # Blender forces unique object names; the contract repeats `DoorLeaf`.
    glbinfo.rename_nodes(out_path, lambda n: re.sub(r"\.\d{3}$", "", n) if n.startswith("DoorLeaf") else n)
    return nodes
