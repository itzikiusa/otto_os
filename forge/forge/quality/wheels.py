"""Level an explicitly counted, multi-axle vehicle using its measured wheel contacts."""
import re


_NUMBERS = {"two": 2, "three": 3, "four": 4, "six": 6, "eight": 8}
# Allow descriptive modifiers, including "deeply treaded". Keeping nouns and
# clause boundaries out avoids reading "four doors with wheels" as four wheels.
_PLAIN = r"(?:wide|large|small|dark|light|black|white|grey|gray|dusty|rubber|metal)"
_MODIFIER = rf"(?:{_PLAIN}|alloy|sprung|[a-z]+(?:ed|ly|y))"
_COUNT = re.compile(
    r"\b(two|three|four|six|eight|[23468])"
    # Commas and conjunctions can join adjectives, but cannot bridge a noun.
    # A hyphen immediately after the number can also describe spokes: six-spoked.
    rf"(?:(?:\s+{_MODIFIER}|-{_PLAIN})(?:(?:,\s*(?:and\s+)?|\s+and\s+|[ -]+){_MODIFIER}){{0,3}})?"
    r"[ -]+wheel(?:s|ed)\b"
    r"(?:\s+(on each side|per side)\b)?"
)


def requested(prompt):
    for match in _COUNT.finditer((prompt or "").lower()):
        word, per_side = match.groups()
        count = int(word) if word.isdigit() else _NUMBERS[word]
        total = 2 * count if per_side else count
        if total in (4, 6, 8):
            return total
    return None


def three_axle_vehicle(prompt):
    """Whether a two-sided chassis reference fits the described vehicle."""
    return requested(prompt) == 6 and bool(re.search(r"\b(rover|truck|lorry|buggy|carrier|bus|vehicle)\b", prompt, re.I))


def contacts(mesh, count):
    result = _wheelbase(mesh, count, allow_other_counts=False)
    return result[:2] if result else None


def _wheelbase(mesh, requested_count, *, allow_other_counts):
    import numpy as np
    from PIL import Image, ImageDraw
    from scipy.ndimage import binary_closing, label
    p = mesh.vertices
    normals = mesh.face_normals
    weights = mesh.area_faces
    if not len(weights) or min(np.ptp(p, axis=0)) < 1e-6:
        return None
    # The large chassis panels define an orthogonal reference even when
    # the image-to-shape step has baked the perspective into the GLB.
    axes = []
    candidates = normals[weights.argsort()[-180:]]
    for _ in range(2):
        if axes:
            candidates = candidates[np.abs(candidates @ axes[0]) < .24]
        if not len(candidates):
            return None
        best = max(candidates, key=lambda a: weights[np.abs(normals @ a) > .96].sum())
        group = np.abs(normals @ best) > .96
        axis = np.average(normals[group] * np.sign(normals[group] @ best)[:, None], weights=weights[group], axis=0)
        if axes:
            axis -= axes[0] * (axis @ axes[0])
        axes.append(axis / np.linalg.norm(axis))
    axes.append(np.cross(*axes))
    up = max(axes, key=lambda a: abs(a[1]))
    up = up * np.sign(up[1])
    if up[1] < .72:
        return None
    lateral = [a for a in axes if abs(a @ up) < .1]
    basis = np.column_stack([lateral[0], up, lateral[1]])
    v = p @ basis
    lo, span = v.min(axis=0), np.ptp(v, axis=0)
    candidates = []
    # Rasterise the actual tyre surfaces below each horizontal cut. Counting
    # vertices alone splits both sidewalls of a low-poly tyre into separate feet.
    triangles = v[mesh.faces]
    pixel = span.max() / 150
    for fraction in (.08, .11, .14, .17, .20):
        cut = lo[1] + fraction * span[1]
        mask = Image.new("1", (154, 154))
        draw = ImageDraw.Draw(mask)
        for triangle in triangles[triangles[:, :, 1].min(axis=1) <= cut]:
            poly = []
            for i, a in enumerate(triangle):
                b = triangle[(i + 1) % 3]
                if a[1] <= cut:
                    poly.append(a)
                if (a[1] <= cut) != (b[1] <= cut):
                    poly.append(a + (b - a) * ((cut - a[1]) / (b[1] - a[1])))
            if len(poly) >= 3:
                pixels = (np.asarray(poly)[:, [0, 2]] - lo[[0, 2]]) / pixel + 1
                draw.polygon([tuple(p) for p in pixels], fill=1)
        regions, n = label(binary_closing(np.asarray(mask)))
        centers = []
        for k in range(1, n + 1):
            z, x = np.nonzero(regions == k)
            if len(x) > max(5, .025 * np.count_nonzero(regions)):
                centers.append((np.array([x.mean(), z.mean()]) - 1) * pixel + lo[[0, 2]])
        count = len(centers)
        if count not in (4, 6, 8) or (count != requested_count and not allow_other_counts):
            continue
        centers = np.array(centers)
        for axis in [0, 1]:
            half = count // 2
            row = centers[np.argsort(centers[:, axis])].reshape((2, half, 2))
            # Match left and right contacts front to back. Different axle
            # widths are common, but their midpoints must follow one track.
            row = np.array([side[np.argsort(side[:, 1-axis])] for side in row])
            tracks = row[1, :, axis] - row[0, :, axis]
            width = tracks.mean()
            along = row[:, :, 1-axis]
            gaps = np.diff(along, axis=1)
            if (width < .30 * span.max() or gaps.min() < .11 * span.max()
                    or tracks.min() < .60 * width or np.ptp(row[:, :, axis].mean(axis=0)) > .24 * width
                    or np.max(np.abs(along[0]-along[1])) > .32 * gaps.mean()):
                continue
            error = np.mean(np.abs(along[0]-along[1])) / gaps.mean()
            candidates.append((count != requested_count, error, axis, fraction, count, centers))
    if not candidates:
        return None
    _, _, axis, fraction, count, centers = min(candidates, key=lambda item: item[:2])
    # The panels give the approximate chassis frame. Measure the soles of the
    # tyres separately: the suspension can put those on a slightly different plane.
    low_points = v[v[:, 1] <= lo[1] + fraction * span[1]]
    distances = low_points[:, None, [0, 2]] - centers
    nearest = np.argmin(np.sum(distances**2, axis=2), axis=1)
    patches = [low_points[nearest == i] for i in range(count)]
    contacts = []
    if all(len(points) for points in patches):
        local_up = np.array([0., 1., 0.])
        for _ in range(3):
            soles = []
            for points in patches:
                height = points @ local_up
                soles.append(points[height <= height.min() + .0005 * span.max()].mean(axis=0))
            soles = np.array(soles)
            if np.ptp(soles @ local_up) < .001 * span.max():
                break
            plane = np.linalg.svd(soles - soles.mean(axis=0), full_matrices=False)[2][-1]
            plane *= np.sign(plane[1])
            if plane[1] < .99 or np.ptp(soles @ plane) > .015 * span.max():
                break
            local_up = plane
        # Keep the centre of each measured contact patch on its sole plane.
        # These are world points before the delivery rotates and resizes the scene.
        for (x, z), points in zip(centers, patches):
            y = ((points @ local_up).min() - x * local_up[0] - z * local_up[2]) / local_up[1]
            contacts.append((basis @ [x, y, z]).tolist())
        up = basis @ local_up
    forward = lateral[1-axis]
    forward = forward - up * (forward @ up)
    forward /= np.linalg.norm(forward)
    if forward[2] < 0:
        forward = -forward
    right = np.cross(up, forward)
    rotation = np.array([right, up, forward])
    return rotation, fraction, count, contacts


def align(glb, doc, count):
    import numpy as np
    from forge.quality import package
    if doc.get("skins") or "wheel_contacts" in doc.get("asset", {}).get("extras", {}):
        return None
    mesh = package.display_surface(glb)
    if mesh is None:
        return None
    result = _wheelbase(mesh, count, allow_other_counts=True)
    if result is None:
        return None
    rotation, fraction, measured_count, contacts = result
    matrix = np.eye(4)
    matrix[:3,:3] = rotation
    # Empty landmark nodes follow the same transforms as the real tyres,
    # including a size request applied later by package.scale_metres.
    scene = doc["scenes"][doc.get("scene", 0)]
    for i, position in enumerate(contacts):
        scene["nodes"].append(len(doc["nodes"]))
        doc["nodes"].append({"name": f"WheelContact_{i}", "translation": position})
    package.rotate_roots(doc, matrix, "Wheelbase")
    report = {"count":measured_count, "rows":2, "lower_slice":round(fraction,3)}
    if measured_count != count:
        report["requested_count"] = count
    doc["asset"].setdefault("extras", {})["wheel_contacts"] = report
    return report
