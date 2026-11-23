"""Fit missing leg controls to separately measurable limbs of the generated mesh."""
from pathlib import Path
from tempfile import TemporaryDirectory

import numpy as np
from scipy.optimize import linear_sum_assignment
from scipy.spatial import cKDTree
from scipy.spatial.distance import cdist

from forge import bodyplan
from forge.library.gltf import append_accessor, write


def _surface(rig):
    import trimesh
    faces = []
    offset = 0
    parts = []
    for node in rig.doc["nodes"]:
        if node.get("skin") != 0 or "mesh" not in node:
            continue
        for prim in rig.doc["meshes"][node["mesh"]]["primitives"]:
            attrs = prim["attributes"]
            if "JOINTS_0" not in attrs or "WEIGHTS_0" not in attrs:
                continue
            count = rig.doc["accessors"][attrs["POSITION"]]["count"]
            if prim.get("mode", 4) != 4:
                return None, []
            indices = (bodyplan.accessor(rig.doc, rig.binary, prim["indices"]).astype(int).ravel()
                       if "indices" in prim else np.arange(count))
            faces.append(indices.reshape(-1, 3) + offset)
            parts.append((attrs, slice(offset, offset + count)))
            offset += count
    mesh = trimesh.Trimesh(rig.skin_rest, np.concatenate(faces), process=False)
    return mesh, parts


def _centres(mesh, fraction):
    y = mesh.bounds[0, 1] + fraction * mesh.extents[1]
    cut = mesh.section(plane_normal=[0, 1, 0], plane_origin=[0, y, 0])
    centres = []
    if cut is not None:
        for ring in cut.discrete:
            if np.ptp(ring[:, [0, 2]], axis=0).min() < .012 * mesh.extents.max():
                continue
            centre = (ring.min(0) + ring.max(0)) / 2
            # Concentric sections of a hollow metal limb share a control.
            if all(np.linalg.norm(centre - other) > .05 * mesh.extents[1] for other in centres):
                centres.append(centre)
    return np.asarray(centres)


def _tracks(mesh, count, heights):
    sections = []
    for height in heights:
        centres = _centres(mesh, height)
        if len(centres) != count:
            return None
        if sections:
            costs = cdist(sections[-1][:, [0, 2]], centres[:, [0, 2]])
            row, col = linear_sum_assignment(costs)
            if costs[row, col].max() > .25 * mesh.extents[1]:
                return None
            centres = centres[col]
        sections.append(centres)
    return np.stack(sections)


def _track_distances(tracks, pos):
    # Measure horizontally at each vertex's own height, so a sloped limb
    # stays together even where a neighbouring foot passes underneath it.
    paths = np.stack([np.column_stack([np.interp(pos[:, 1], t[:, 1], t[:, a]) for a in (0, 2)])
                      for t in tracks.transpose(1, 0, 2)], axis=1)
    return np.linalg.norm(paths - pos[:, None, [0, 2]], axis=2)


def _level(rig, mesh, count):
    rings = _tracks(mesh, count, (.12, .17, .22))
    if rings is None:
        return None
    height = mesh.extents[1]
    low = rig.skin_rest[rig.skin_rest[:, 1] < mesh.bounds[0, 1] + .26 * height]
    groups = cdist(low[:, [0, 2]], rings[1][:, [0, 2]]).argmin(1)
    soles = []
    for i in range(count):
        pts = low[groups == i]
        if not len(pts):
            return None
        soles.append(pts[pts[:, 1] < pts[:, 1].min() + .008 * height].mean(0))
    centred = np.asarray(soles) - np.mean(soles, axis=0)
    _, _, vt = np.linalg.svd(centred, full_matrices=False)
    normal = vt[-1] * np.sign(vt[-1, 1])
    if normal[1] < .93 or np.abs(centred @ normal).max() > .035 * height:
        return None
    matrix = np.eye(4)
    matrix[:3, :3] = bodyplan.rotation_between(normal, bodyplan.UP)
    return matrix


def _bind(rig, plan, mesh, parts, count, target):
    chains = [g["chain"] for g in plan["leg_chains"]]
    tracks = _tracks(mesh, count, (.12, .26, .39))
    templates = [i for i, chain in enumerate(chains) if len(chain) >= 3]
    if tracks is None or len(chains) != count - 1 or not templates:
        return False
    tips = rig.heads[[c[-1] for c in chains]]
    costs = cdist(tips[:, [0, 2]], tracks[0][:, [0, 2]])
    row, matched = linear_sum_assignment(costs)
    height = plan["height"]
    if costs[row, matched].max() > .18 * height:
        return False
    missing, = set(range(count)) - set(matched)
    foot = tracks[0, missing]
    centre = tracks[0].mean(0)
    side = np.cross(bodyplan.UP, plan["forward"])
    same_side = ((tips - centre) @ side) * ((foot - centre) @ side) > 0
    distance = np.linalg.norm(tips - foot, axis=1) + (~same_side) * height
    nearest = min(templates, key=lambda i: distance[i])
    chain = chains[nearest]
    offset = foot - tracks[0, matched[nearest]]
    limb_joints = {k for leg in chains for k in leg}
    parents = [k for k in rig.order if k not in limb_joints and rig.children[k]]
    hip = rig.heads[chain[0]] + offset
    torso = min(parents, key=lambda k: np.linalg.norm(rig.heads[k] - hip))
    parent, world = rig.nodes[torso], rig.rest_world[torso]

    doc = rig.doc
    skin = doc["skins"][0]
    binary = bytearray(rig.binary)
    binds = list(rig.ibm)
    added = []
    for j, src in enumerate(chain):
        joint = rig.rest_world[src].copy()
        joint[:3, 3] += offset
        local = np.linalg.solve(world, joint)
        t, q, s = bodyplan.node_trs({"matrix": local.T.flatten()})
        node = len(doc["nodes"])
        doc["nodes"].append(dict(name=f"recovered_leg_{j}", translation=t.tolist(),
                                 rotation=q.tolist(), scale=s.tolist()))
        doc["nodes"][parent].setdefault("children", []).append(node)
        parent, world = node, joint
        added.append(len(skin["joints"]))
        skin["joints"].append(node)
        binds.append(np.linalg.solve(joint, rig.rest_world[src] @ rig.ibm[src]))
    skin["inverseBindMatrices"] = append_accessor(doc, binary, np.asarray(binds, dtype="<f4").transpose(0, 2, 1),
                                          "MAT4", 5126)

    pos = rig.skin_rest
    y = pos[:, 1]
    dist = _track_distances(tracks, pos)
    nearest_track = dist.argmin(1)
    source_pts = (nearest_track == matched[nearest]) & (y < plan["floor"] + .69 * height)
    dest_pts = ((nearest_track == missing) & (y < plan["floor"] + .60 * height)
                & (dist[:, missing] < .135 * height))
    if not source_pts.any() or not dest_pts.any():
        return False
    ids = np.flatnonzero(source_pts)
    _, match = cKDTree(pos[source_pts]).query(pos[dest_pts] - offset)
    joints, weights = rig.vjoints.copy(), rig.vweights.copy()
    original = joints[ids[match]]
    mapping = np.arange(len(rig.nodes))
    mapping[chain] = added
    joints[dest_pts] = mapping[original]
    weights[dest_pts] = weights[ids[match]]
    return _save_weights(rig, parts, binary, joints, weights, target, count)


def _save_weights(rig, parts, binary, joints, weights, target, count):
    doc = rig.doc
    for attrs, region in parts:
        attrs["JOINTS_0"] = append_accessor(doc, binary, joints[region].astype("<u2"), "VEC4", 5123)
        attrs["WEIGHTS_0"] = append_accessor(doc, binary, weights[region].astype("<f4"), "VEC4", 5126)
    doc["buffers"][0]["byteLength"] = len(binary)
    write(doc, binary, target)
    rebound = bodyplan.Rig(target)
    return (bodyplan.detect(rebound)["legs"] == count
            and np.allclose(rebound.skin_rest, rig.skin_rest, atol=1e-5 * np.ptp(rig.skin_rest[:, 1])))


def _fit_uncontrolled(rig, plan, mesh, parts, count, target):
    # The image-to-rig model sometimes puts its root near a foot and assigns
    # each entire leg to a shoulder. Trace the existing skin up from the soles.
    tracks = _tracks(mesh, count, (.12, .19, .26, .33, .39))
    if tracks is None:
        return False
    pos = rig.skin_rest
    height = mesh.extents[1]
    y = pos[:, 1]
    distances = _track_distances(tracks, pos)
    nearest = distances.argmin(1)
    doc = rig.doc
    binary = bytearray(rig.binary)
    skin = doc["skins"][0]
    binds = list(rig.ibm)
    root = next(k for k in rig.order if rig.parent[k] < 0)
    bind_frame = rig.rest_world[root] @ rig.ibm[root]
    joints, weights = rig.vjoints.copy(), rig.vweights.copy()

    def attach(parent_node, parent_world, point):
        world = np.eye(4)
        world[:3, 3] = point
        local = np.linalg.solve(parent_world, world)
        t, q, scale = bodyplan.node_trs({"matrix": local.T.ravel()})
        index = len(skin["joints"])
        node = len(doc["nodes"])
        doc["nodes"].append(dict(name=f"leg_fit_{index}", translation=t.tolist(),
                                 rotation=q.tolist(), scale=scale.tolist()))
        doc["nodes"][parent_node].setdefault("children", []).append(node)
        skin["joints"].append(node)
        binds.append(np.linalg.solve(world, bind_frame))
        return index, node, world

    head = plan["snout"][-1] if plan["snout"] else plan["head"]
    if head != plan["hub"]:
        skin.setdefault("extras", {})["original_head_node"] = rig.nodes[head]
    # A fitted root can sit on the ground beside a foot. Anchor the hips
    # to an existing body joint above the visible legs.
    shoulder = tracks[-1].mean(0)
    torso_ids = np.flatnonzero(rig.heads[:, 1] >= shoulder[1])
    if not len(torso_ids):
        return False
    torso = torso_ids[np.linalg.norm(rig.heads[torso_ids] - shoulder, axis=1).argmin()]
    hub, hub_world = rig.nodes[torso], rig.rest_world[torso]
    for i, track in enumerate(tracks.transpose(1, 0, 2)):
        limb = (nearest == i) & (distances[:, i] <= .22 * height)
        low = pos[limb & (y <= track[0, 1])]
        if not len(low):
            return False
        sole = low[low[:, 1] <= low[:, 1].min() + .025 * height].mean(0)
        hip = track[-1] + (track[-1] - track[-2])
        knee = track[2]
        ankle = sole + .15 * (track[0] - sole)
        parent, world = hub, hub_world
        chain = []
        for point in (hip, knee, ankle):
            j, parent, world = attach(parent, world, point)
            chain.append(j)
        # A point halfway down a link follows the upstream joint. Blend only
        # around the hinge, preserving the original upper body attachment.
        ids = np.flatnonzero(limb & (y < hip[1]))
        upper = y[ids] > (knee[1] + ankle[1]) / 2
        split = np.where(upper, knee[1], ankle[1])
        blend = np.clip(.5 + (y[ids] - split) / (.10 * height), 0, 1)
        joints[ids] = 0
        joints[ids, 0] = np.where(upper, chain[0], chain[1])
        joints[ids, 1] = np.where(upper, chain[1], chain[2])
        weights[ids] = np.column_stack([blend, 1 - blend, np.zeros((len(ids), 2))])
    skin["inverseBindMatrices"] = append_accessor(doc, binary, np.asarray(binds, dtype="<f4").transpose(0, 2, 1),
                                                   "MAT4", 5126)
    return _save_weights(rig, parts, binary, joints, weights, target, count)


def recover(source, output, count):
    """Retain the original prediction unless the requested independent controls are verified."""
    rig = bodyplan.Rig(source)
    if len(rig.doc["skins"]) != 1 or bodyplan.detect(rig)["legs"] == count:
        return None
    mesh, _ = _surface(rig)
    rotation = _level(rig, mesh, count) if mesh is not None else None
    if rotation is None:
        return None
    for scene in rig.doc["scenes"]:
        parent = len(rig.doc["nodes"])
        rig.doc["nodes"].append(dict(name="FootPlane", matrix=rotation.T.flatten().tolist(), children=scene["nodes"]))
        scene["nodes"] = [parent]
    # Build in the job directory. Failed validation never replaces an accepted rig.
    with TemporaryDirectory(dir=Path(output).parent) as tmp:
        level = Path(tmp) / "level.glb"
        write(rig.doc, rig.binary, level)
        rig = bodyplan.Rig(level)
        plan = bodyplan.detect(rig)
        if plan["legs"] == count:
            level.replace(output)
            return {"legs": count, "controls_added": 0}
        fixed = Path(tmp) / "rebound.glb"
        mesh, parts = _surface(rig)
        if mesh is None:
            return None
        if plan["legs"] == 0 and _fit_uncontrolled(rig, plan, mesh, parts, count, fixed):
            fixed.replace(output)
            return {"legs": count, "controls_added": count}
        if plan["legs"] == count - 1 and _bind(rig, plan, mesh, parts, count, fixed):
            fixed.replace(output)
            return {"legs": count, "controls_added": 1}
    return None
