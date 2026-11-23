"""Set a flat, radially spread octopus down on its arms before measuring its height."""
import numpy as np

from forge import bodyplan
from forge.quality import package


def align(glb, doc):
    if doc.get("skins") or "radial_orientation" in doc["asset"].get("extras", {}):
        return None
    mesh = package.display_surface(glb)
    if mesh is None or mesh.area <= 1e-12:
        return None
    weights = mesh.area_faces
    centres = mesh.triangles_center
    delta = centres - np.average(centres, axis=0, weights=weights)
    values, axes = np.linalg.eigh((delta.T * weights) @ delta / weights.sum())
    # Only interpret a broad fan. A standing or curled body can already have
    # the correct up axis and does not provide an arm plane to measure.
    if values[1] < 1e-12 or values[0] > .20 * values[1] or values[1] < .45 * values[2]:
        return None
    normal = axes[:, 0]
    radius = np.linalg.norm(delta @ axes[:, 1:], axis=1)
    reach = np.percentile(radius, 98)
    offset = delta @ normal
    centre = offset[radius < .30 * reach]
    arms = offset[radius > .70 * reach]
    if not len(centre) or not len(arms):
        return None
    arm_level = np.median(arms)
    bottom, top = np.percentile(centre, [3, 97]) - arm_level
    # The mantle rises above a thinner bed of arms. A symmetric disk has
    # no evidence for which way up it belongs.
    side = top + bottom
    if abs(side) < .08 * reach:
        # A compact mantle can be diluted by nearby arm roots in the outer
        # sample. Require the same side in both smaller central samples.
        for fraction in (.18, .24):
            inner = offset[radius < fraction * reach]
            if len(inner) < 32:
                return None
            protrusion = np.sum(np.percentile(inner, [3, 97]) - arm_level)
            if np.sign(side) * protrusion < .08 * reach:
                return None
    normal *= np.sign(side)
    if normal[1] > .995:
        return None
    frame = np.eye(4)
    frame[:3, :3] = bodyplan.rotation_between(normal, bodyplan.UP)
    package.rotate_roots(doc, frame, "Mantle")
    result = {"arm_plane_normal": np.round(normal, 4).tolist()}
    doc["asset"].setdefault("extras", {})["radial_orientation"] = result
    return result


def _arm_paths(mesh, count):
    """Trace each tip back to the mantle along the thick middle of the actual arm."""
    import cv2
    from scipy.ndimage import distance_transform_edt
    from scipy.spatial import cKDTree
    from skimage.graph import MCP_Geometric
    from forge.quality.anatomy import silhouette_tips

    vertices = mesh.vertices
    xz = vertices[:, [0, 2]]
    span = np.ptp(xz, axis=0)
    if min(span) < 1e-6 or np.ptp(vertices[:, 1]) > .65 * min(span):
        return None
    scale = 300 / max(span)
    low = xz.min(axis=0)
    pixels = np.rint((xz - low) * scale + 10).astype("int32")
    mask = np.zeros((320, 320), np.uint8)
    for face in mesh.faces:
        cv2.fillConvexPoly(mask, pixels[face], 1)
    tips = silhouette_tips(mask.astype(bool))
    if not tips or len(tips) != count:
        return None
    radius = distance_transform_edt(mask)
    hub = np.unravel_index(radius.argmax(), radius.shape)
    trace = MCP_Geometric(np.where(mask, 1 / (.5 + radius), np.inf))
    costs, _ = trace.find_costs([hub])
    tree = cKDTree(xz)

    def world(pixel):
        x, z = (np.asarray(pixel)[::-1] - 10) / scale + low
        _, nearby = tree.query([x, z], k=min(16, len(vertices)))
        return [x, np.median(vertices[nearby, 1]), z]

    arms = []
    for tip in tips:
        if not np.isfinite(costs[tuple(tip)]):
            return None
        path = np.asarray(trace.traceback(tip))
        lengths = np.r_[0, np.linalg.norm(np.diff(path, axis=0), axis=1).cumsum()]
        first = max(.85 * radius[hub], .22 * lengths[-1])
        if lengths[-1] - first < 7:
            return None
        sample = np.linspace(first, lengths[-1], 7)
        arms.append([world(path[np.abs(lengths - d).argmin()]) for d in sample])
    return world(hub), radius[hub] / scale, arms


def fit_arms(source, target, count):
    """Bind a single static display mesh only when its horizontal fan provides every requested arm."""
    from forge import glbstats
    from forge.library.gltf import append_accessor, write

    if source.suffix.lower() != ".glb":
        return False
    doc, binary = glbstats.read_glb(source)
    nodes = doc.get("nodes", [])
    display = [node for node in nodes if "mesh" in node]
    if len(display) != 1 or len(doc.get("scenes", [])) != 1 or doc.get("skins") or doc.get("animations"):
        return False
    node = display[0]
    # rig_input already roots a transformed mesh. A parent left in this form
    # is an identity container. Preserve the materials and original vertex order.
    prims = doc["meshes"][node["mesh"]]["primitives"]
    if any(p.get("mode", 4) != 4 or p.get("targets") for p in prims):
        return False
    mesh = package.display_surface(source)
    traces = _arm_paths(mesh, count) if mesh is not None and len(mesh.faces) else None
    if traces is None:
        return False
    hub, radius, arms = traces
    heads = np.asarray([hub, *(p for arm in arms for p in arm)])
    # Put both the skin and the joint hierarchy in the display mesh's frame.
    # This keeps sizing, collision and skinning in agreement on a rotated bake.
    frame = np.asarray(package.local_matrix(node))
    if abs(np.linalg.det(frame[:3, :3])) < 1e-12:
        return False
    local_heads = np.linalg.solve(frame[:3, :3], (heads - frame[:3, 3]).T).T
    frame_index = len(nodes)
    start = frame_index + 1
    nodes.append({"name": "ArmFrame", "matrix": frame.T.flatten().tolist(), "children": [start]})
    doc["scenes"][0]["nodes"].append(frame_index)
    nodes.append({"name": "mantle", "translation": local_heads[0].tolist(), "children": []})
    for index, arm in enumerate(arms):
        parent = start
        for level in range(len(arm)):
            child = len(nodes)
            nodes[parent].setdefault("children", []).append(child)
            offset = local_heads[child - start] - local_heads[parent - start]
            nodes.append({"name": f"arm_{index + 1}_{level + 1}", "translation": offset.tolist()})
            parent = child

    binary = bytearray(binary)
    inverse = np.repeat(np.eye(4)[None], len(heads), axis=0)
    inverse[:, :3, 3] = -local_heads
    bind = np.asarray(inverse, dtype="<f4").transpose(0, 2, 1)
    doc["skins"] = [{"joints": list(range(start, len(nodes))), "skeleton": start,
                     "inverseBindMatrices": append_accessor(doc, binary, bind, "MAT4", 5126)}]
    node["skin"] = 0
    for prim in prims:
        attrs = prim["attributes"]
        local = bodyplan.accessor(doc, binary, attrs["POSITION"])
        # Imported meshes can have many more vertices than generated assets.
        # Keep the vertex-by-joint distance matrix bounded.
        all_joints = np.empty((len(local), 4), dtype="<u2")
        all_weights = np.empty((len(local), 4), dtype="<f4")
        for first in range(0, len(local), 4096):
            region = slice(first, first + 4096)
            world = local[region] @ frame[:3, :3].T + frame[:3, 3]
            distances = np.linalg.norm(world[:, None, :] - heads[None, :, :], axis=2)
            # The elevated central mantle is carried by the hub as the arms curl.
            core = np.linalg.norm(world[:, [0, 2]] - heads[0, [0, 2]], axis=1)
            distances[:, 0] = np.minimum(distances[:, 0], np.maximum(core - .65 * radius, 0))
            joints = np.argsort(distances, axis=1)[:, :4]
            nearest = np.take_along_axis(distances, joints, axis=1)
            weights = 1 / np.maximum(nearest, .005 * radius) ** 4
            all_joints[region] = joints
            all_weights[region] = weights / weights.sum(axis=1, keepdims=True)
        attrs["JOINTS_0"] = append_accessor(doc, binary, all_joints, "VEC4", 5123)
        attrs["WEIGHTS_0"] = append_accessor(doc, binary, all_weights, "VEC4", 5126)
    write(doc, binary, target)
    return True
