"""Register a parked road vehicle to its supporting surface and chassis axes before sizing."""
import re


_ROAD_VEHICLE = re.compile(r"\b(?:car|sedan|taxi|van|bus|truck|lorry|jeep|suv|hatchback|station wagon)$")


def requested(prompt):
    # The vehicle must be the subject, rather than a component or another
    # object next to one. Preserve an intentionally tipped or suspended pose.
    text = (prompt or "").lower()
    if re.search(r"\b(overturned|overturning|upside[ -]down|on (?:its|one) (?:side|roof)|wheelie|jacked|airborne|suspended)\b", text):
        return False
    subject = re.split(r"[,;.:]|\b(?:with|without|of|for|from|beside|under|on|in|next|at|that|whose|has|is|parked|stopped)\b",
                       text, maxsplit=1)[0].strip()
    return bool(_ROAD_VEHICLE.search(subject))


def support_frame(vertices):
    import numpy as np
    from scipy.spatial import ConvexHull, QhullError
    from trimesh.bounds import oriented_bounds_2D

    if not np.isfinite(vertices).all():
        return None
    hull = ConvexHull(vertices)
    points = vertices[hull.vertices]
    footprint = ConvexHull(points[:, [0, 2]]).volume
    centre = ((points.min(axis=0) + points.max(axis=0)) * .5)[[0, 2]]
    tolerance = .012 * np.ptp(points, axis=0).max()
    # A broad lower facet belongs to the imaginary floor that joins the tyres.
    # The physical centre must lie inside its contact polygon. Inspect a
    # bounded number of large facets even on a high-poly, rounded body.
    down = np.flatnonzero(-hull.equations[:, 1] >= np.cos(np.deg2rad(38)))
    faces = vertices[hull.simplices[down]]
    areas = np.linalg.norm(np.cross(faces[:, 1] - faces[:, 0], faces[:, 2] - faces[:, 0]), axis=1)
    normals = -hull.equations[down[np.argsort(areas)[-48:]], :3]
    best = None
    for up in normals:
        heights = points @ up
        contact = points[heights <= heights.min() + tolerance][:, [0, 2]]
        if len(contact) < 4:
            continue
        try:
            patch = ConvexHull(contact)
        except QhullError:  # Coplanar contacts can project to a single line.
            continue
        fraction = patch.volume / footprint
        if fraction < .40 or np.max(patch.equations[:, :2] @ centre + patch.equations[:, 2]) > 0:
            continue
        score = (fraction, up[1])
        if best is None or score > best[0]:
            best = score, up
    if best is None:
        return None
    _, up = best
    right = np.cross(up, [0., 0., 1.])
    right /= np.linalg.norm(right)
    level = np.array([right, up, np.cross(right, up)])
    flat = points @ level.T
    # The longest side of the minimum-area rectangle is the vehicle's length.
    outline = flat[:, [0, 2]]
    outline = outline[ConvexHull(outline).vertices]
    # The minimum rectangle routine compares every pair of outline vertices.
    # Bound its memory on smooth contours while retaining well-spaced extremes.
    if len(outline) > 256:
        angles = np.linspace(0., 2*np.pi, 256, endpoint=False)
        indices = [np.argmax(outline[:, 0]*np.cos(a) + outline[:, 1]*np.sin(a)) for a in angles]
        outline = outline[np.unique(indices)]
    box, (length, width) = oriented_bounds_2D(outline)
    if length < 1.4 * width:
        return None
    forward = box[0, 0] * level[0] + box[0, 1] * level[2]
    if forward[2] < 0:
        forward = -forward
    return np.array([np.cross(up, forward), up, forward])


def align(glb, doc):
    import numpy as np
    from scipy.spatial import QhullError
    from . import package

    extras = doc["asset"].get("extras", {})
    if doc.get("skins") or "vehicle_frame" in extras or "wheel_contacts" in extras:
        return None
    mesh = package.display_surface(glb)
    if mesh is None or len(mesh.vertices) < 4:
        return None
    try:
        rotation = support_frame(mesh.vertices)
    except QhullError:
        return None
    if rotation is None or np.trace(rotation) > 1 + 2 * np.cos(np.deg2rad(2)):
        return None
    matrix = np.eye(4)
    matrix[:3, :3] = rotation
    package.rotate_roots(doc, matrix, "Vehicle frame")
    report = {"rotation": rotation.tolist()}
    doc["asset"].setdefault("extras", {})["vehicle_frame"] = report
    return report
