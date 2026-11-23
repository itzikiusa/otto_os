"""Mount a detached flag panel by its hoist edge on a measured vertical pole."""
import re

import numpy as np

from forge.quality import package


def requested(prompt):
    words = set(re.findall(r"[a-z]+", (prompt or "").lower()))
    return bool(words & {"flag", "banner", "pennant"} and words & {"pole", "staff", "flagpole", "flagstaff"})


def mounting(mesh):
    mesh.merge_vertices(merge_tex=True, merge_norm=True)
    parts = sorted(mesh.split(only_watertight=False), key=lambda part: -part.area)
    if len(parts) < 2 or (len(parts) > 2 and parts[2].area > .03 * parts[1].area):
        return None
    pole, cloth = sorted(parts[:2], key=lambda part: -part.extents[1])
    height = pole.extents[1]
    if (height <= 0 or max(pole.extents[[0, 2]]) > .09 * height
            or not .10 * height < cloth.extents[1] < .75 * height
            or cloth.bounds[0, 1] < pole.bounds[0, 1] + .32 * height):
        return None
    axis = pole.bounds.mean(axis=0)[[0, 2]]
    xz = cloth.vertices[:, [0, 2]]
    radius = np.linalg.norm(pole.vertices[:, [0, 2]] - axis, axis=1).max()
    gap = np.linalg.norm(xz - axis, axis=1).min()
    if gap < radius + .04 * height:
        return None
    _, axes = np.linalg.eigh(np.cov(xz.T))
    along, across = xz @ axes[:, 1], xz @ axes[:, 0]
    width = np.ptp(along)
    if width < .15 * height or np.ptp(across) > .30 * width:
        return None
    edges = [cloth.vertices[mask] for mask in
             (along < along.min() + .055 * width, along > along.max() - .055 * width)]
    spans = [np.ptp(edge[:, 1]) for edge in edges]
    ends = [np.median(edge[:, [0, 2]], axis=0) for edge in edges]
    if max(spans) < .55 * cloth.extents[1]:
        return None
    # A waving free end can curl up; the hoist still spans the panel height.
    hoist = np.argmax(spans) if max(spans) > 1.3 * min(spans) else np.argmin(
        [np.linalg.norm(end - axis) for end in ends])
    return axis, (radius + gap) / 2, axis - ends[hoist]


def mount(glb, doc, binary):
    if doc.get("skins") or doc.get("animations"):
        return None
    mesh = package.display_surface(glb)
    fit = mounting(mesh) if mesh is not None else None
    if fit is None:
        return None
    axis, cutoff, delta = fit
    # Move the same component in the display meshes, LODs and collision hulls.
    # Positions can share an accessor and be under a transformed parent.
    frames = {}
    hulls = set()
    nodes = doc["nodes"]
    todo = [(i, np.eye(4)) for i in doc["scenes"][doc.get("scene", 0)]["nodes"]]
    while todo:
        i, parent = todo.pop()
        node = nodes[i]
        frame = parent @ np.asarray(package.local_matrix(node))
        todo.extend((child, frame) for child in node.get("children", []))
        if "mesh" not in node:
            continue
        for prim in doc["meshes"][node["mesh"]]["primitives"]:
            idx = prim["attributes"]["POSITION"]
            if prim.get("targets") or (idx in frames and not np.allclose(frames[idx], frame)):
                return None
            frames[idx] = frame
            if node.get("name", "").upper().startswith("UCX_"):
                hulls.add(idx)
    data = bytearray(binary)
    changes = {}
    for idx, frame in frames.items():
        original = package.floats(doc, binary, idx)
        if original is None or abs(np.linalg.det(frame[:3, :3])) < 1e-12:
            return None
        world = original @ frame[:3, :3].T + frame[:3, 3]
        panel = np.linalg.norm(world[:, [0, 2]] - axis, axis=1) > cutoff
        if panel.any():
            if idx in hulls and not panel.all():
                return None  # a single hull for both parts would need rebuilding
            v = package.floats(doc, data, idx)
            v[panel] = original[panel] + np.linalg.solve(frame[:3, :3], [delta[0], 0., delta[1]])
            changes[idx] = {"min": v.min(axis=0).tolist(), "max": v.max(axis=0).tolist()}
    for idx, bounds in changes.items():
        doc["accessors"][idx].update(bounds)
    return data, {"hoist_translation_m": [round(float(v), 5) for v in delta]}
