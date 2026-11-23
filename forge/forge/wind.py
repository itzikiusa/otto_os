"""Landmarks for an axial wind instrument and the rigid mounting frame used for playing it."""
import numpy as np
import trimesh


def frame(axis, up):
    z = axis / np.linalg.norm(axis)
    y = up - np.dot(up, z) * z
    if np.linalg.norm(y) < 1e-6:
        y = np.array([1., 0., 0.]) - z[0] * z
    y /= np.linalg.norm(y)
    return np.column_stack([np.cross(y, z), y, z])


def fit(path, mouth, forward, left, height, scale):
    scene = trimesh.load(path, force="scene", process=False)
    vertices, colours = [], []
    for n in scene.graph.nodes_geometry:
        transform, name = scene.graph[n]
        if n.upper().startswith("UCX_") or "_LOD" in n.upper():
            continue
        part = scene.geometry[name]
        if not isinstance(part, trimesh.Trimesh) or not len(part.vertices):
            continue
        vertices.append(trimesh.transform_points(part.vertices, transform))
        visual = part.visual
        rgb = visual.to_color().vertex_colors if hasattr(visual, "to_color") else visual.vertex_colors
        colours.append(np.asarray(rgb)[:,:3] / 255)
    if not vertices:
        raise ValueError("wind instrument has no display mesh")
    p = np.concatenate(vertices)
    rgb = np.concatenate(colours)
    _, eig = np.linalg.eigh(np.cov(p.T))
    long = eig[:,-1]
    t = p @ long
    ends = [p[t <= np.quantile(t, .08)], p[t >= np.quantile(t, .92)]]
    ends.sort(key=lambda cloud: np.linalg.norm(np.std(cloud - np.outer(cloud@long, long), axis=0)))
    # A flared bell is at the broad end. Brass instruments often have a
    # separate silver mouthpiece; use its compact, neutral-metal surface.
    tip = ends[0].mean(axis=0)
    bell = ends[1].mean(axis=0)
    chroma = np.ptp(rgb, axis=1)
    silver = p[(chroma < min(0.12, np.quantile(chroma,0.005)+0.008)) & (rgb.mean(axis=1) > 0.30)]
    span = np.ptp(t)
    if span < 1e-6:
        raise ValueError("wind instrument has zero length")
    if len(silver) > 4 and np.linalg.norm(np.std(silver,axis=0)) < .16*span:
        tip = np.median(silver, axis=0)
        bell = max((e.mean(axis=0) for e in ends),key=lambda v:np.linalg.norm(v-tip))
    axis = bell - tip
    length = np.linalg.norm(axis)
    if length < 1e-6:
        raise ValueError("wind instrument has no opening axis")
    # Aim slightly across the body so the silhouette of the tube and bell
    # remains legible from the audience. Preserve the valves' upward side.
    rotation = frame(forward + .64 * left, np.array([0.,1.,0.])) @ frame(axis, np.array([0.,1.,0.])).T
    progress = (p-tip) @ (axis/length)
    grips = []
    for frac, sign in ((.52,1),(.35,-1)):
        band = p[np.abs(progress-frac*length)<.06*length]
        center = np.median(band,axis=0) if len(band) else tip+axis*frac
        grip = mouth + rotation @ ((center-tip)*scale)
        grips.append(grip + sign*left*.022*height)
    return {"point":mouth.tolist(), "tip":tip.tolist(), "rotation":rotation.tolist()}, grips
