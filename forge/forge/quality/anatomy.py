"""Fast, offline concept checks on macOS before committing a drawing to 3D."""
import json
from pathlib import Path
import re
import subprocess
import sys


def compact_legless_face(prompt, body):
    """A belly-supported character whose face is meant to show both eyes at once."""
    if body != "legless":
        return False
    words = set(re.findall(r"[a-z]+", prompt.lower()))
    if "eyes" not in words or words & {"tail", "fins", "snake", "worm", "eel", "fish", "coiled",
                                       "tentacles", "floating", "hovering", "serpent"}:
        return False
    return bool(words & {"shadow", "shadows", "darkness", "blob", "slime"} or
                ("legless" in words and words & {"creature", "monster", "compact", "round", "rounded"}))


def description(prompt, body):
    words = set(re.findall(r"[a-z]+", prompt.lower()))
    if body == "legless":
        if words & {"fish", "goldfish", "clownfish", "trout", "carp", "tuna", "mackerel"}:
            return "One natural fish in side profile, swimming horizontally from nose to tail, with distinct fins and tail forming its outline."
        if needs_base_check(prompt, body):
            return ("One upright snow figure in one vertical stack. Each smaller snowball rests at the centre "
                    "of the one below it. Only the broad bottom snowball rests on the ground.")
    if body == "other" and radial_count(prompt) == 8:
        return ("View the animal from directly above. The eight tapering arms of the octopus form "
                "a loose radial pose. Each arm occupies one compass direction and wide angular "
                "gaps extend inward almost to the mantle.")
    if body == "other" and words & {"octopus", "octopod"}:
        return ("One natural octopus with a bulbous mantle. The tapering arms radiate from under "
                "the mantle, with each individual tip visible from a high three-quarter angle.")
    if compact_legless_face(prompt, body):
        return ("Full front view. The compact body rests directly on its broad belly, in a single "
                "unbroken contact with the ground. Both eyes face the viewer, clearly separated "
                "against the face, each with a visible pupil.")
    if body == "other" and paired_legs(prompt) == 6:
        return "A low horizontal six-legged body. Three separate pairs of articulated legs spring from either side; alternate the feet towards and away from the viewer so all six show."
    if body == "bird":
        return "full body, a bird in profile with wings folded, both legs and feet visible. Its beak projects from a small head; feathered wings cover its flanks and thin legs end in gripping claws."
    return None


def redraw_description(prompt, body, check):
    """Change the projection when countable appendages merged in the first silhouette."""
    if "visible_tips" in check and radial_count(prompt) == 8:
        return ("View the octopus directly overhead. Its eight long tapering arms fan out in a single "
                "layer around the mantle, one tip in each of eight different directions, with a wedge "
                "of background between neighboring tips.")
    if "visible_feet" in check and compact_legless_face(prompt, body):
        return ("Show the full front of the face just above the ground. The entire underside is "
                "one smooth belly resting flat, with a continuous outline from cheek to cheek. "
                "Keep both eyes in view.")
    if "visible_feet" in check and paired_legs(prompt) == 6:
        return ("Show the animal walking in a staggered stance. Three pairs of legs attach at the "
                "front, middle and rear. Seen from a low three-quarter side view, the far feet land "
                "in the gaps between the near feet. Show six individual contacts with the ground.")
    return description(prompt, body)


def needs_base_check(prompt, body):
    words = set(re.findall(r"[a-z]+", prompt.lower()))
    return body == "legless" and bool(words & {"snowman", "snowwoman"} or
           (words & {"snow", "snowballs"} and {"carrot", "nose"} <= words and
            words & {"balls", "snowballs", "spheres"}))


def radial_count(prompt):
    words = set(re.findall(r"[a-z0-9]+", prompt.lower()))
    return 8 if words & {"octopus", "octopod"} and words & {"eight", "8"} else None


def paired_legs(prompt):
    """Read an explicit limb count, including a count given in pairs or per side."""
    text = prompt.lower()
    numbers = {"two": 2, "three": 3, "four": 4, "six": 6, "eight": 8}
    number = r"(two|three|four|six|eight|[23468])"
    per_side = r"(?:on (?:each|either) side|per side)"
    # Keep counts within the leg phrase. Participles cover hinged, jointed,
    # spring-loaded, etc., without crossing a relation such as “hinges on”.
    modifiers = r"(?:[ -]+(?:[a-z]+(?:-[a-z]+)*ed|walking|metal|mechanical|robotic|slender|short|long|thin|thick|silver|steel)){0,4}"
    count = re.search(r"\b" + number + r"(?: (pairs?) of)?" + modifiers +
                      r"[ -]+leg(?:s|ged)\b(?:\s+(" + per_side + r"))?", text)
    if count:
        value = numbers.get(count[1]) or int(count[1])
        value *= (2 if count[2] else 1) * (2 if count[3] else 1)
        return value if value in (4, 6, 8) else None
    count = re.search(r"\blegs\s*[,:(–—-]\s*" + number + r"\s+" + per_side + r"\b", text)
    if count:
        value = 2 * (numbers.get(count[1]) or int(count[1]))
        return value if value in (4, 6, 8) else None
    return 6 if re.search(r"\bhexapod\b", text) else None


def structure_reference(prompt, body):
    """Use a proportion guide only when the prompt describes its anatomy."""
    if needs_base_check(prompt, body) and re.search(
        r"(?:^|\b(?:of|from|has|with)\s+)(?:three|3)\s+"
        r"(?:(?:round|stacked|packed|white|large)\s+)*(?:snowballs|snow balls|balls of (?:packed )?(?:white )?snow)\b",
        prompt.strip(), re.I,
    ):
        return Path(__file__).parent / "guides/three-stacked-snowballs.png"
    if body == "other" and paired_legs(prompt) == 6:
        words = set(re.findall(r"[a-z]+", prompt.lower()))
        if words & {"dog", "hound", "canine", "puppy"} and words & {"robot", "robotic", "mechanical", "mech"}:
            return Path(__file__).parent / "guides/mechanical-canine-six.png"
    return None


def needs_matte(prompt, body):
    return needs_base_check(prompt, body) or compact_legless_face(prompt, body) or (body == "other" and
           (radial_count(prompt) is not None or paired_legs(prompt) is not None))


def visible_feet(matte):
    """Count distinct lower-limb silhouettes across several levels above the lowest foot."""
    from collections import Counter
    import numpy as np
    from PIL import Image
    with Image.open(matte) as image:
        alpha = image.convert("RGBA").getchannel("A")
        alpha.thumbnail((384, 384))
        mask = np.asarray(alpha) > 160
    ys, xs = np.nonzero(mask)
    if len(ys) < 128:
        return None
    # Specular streaks in metal sometimes become one- or two-pixel alpha gaps
    # inside a leg. Join those before counting the much wider gaps between feet.
    counts = []
    for fraction in (.10, .15, .20, .25, .30, .35):
        y = round(ys.max() - fraction * np.ptp(ys))
        kernel = (1, 1, 1)
        expanded = np.convolve(mask[y], kernel, mode="same") > 0
        filled = np.convolve(expanded, kernel, mode="same") == 3
        spans = np.flatnonzero(np.diff(np.r_[False, filled, False])).reshape(-1, 2)
        counts.append(sum(1 for a, b in spans if b - a >= max(2, np.ptp(xs) * .012)))
    viable = [n for n, votes in Counter(counts).items() if votes >= 2 and n > 0]
    return max(viable, default=None)


def tentacle_tips(matte):
    """Count long tapering branches of the foreground silhouette, excluding the thick mantle."""
    import numpy as np
    from PIL import Image
    with Image.open(matte) as im:
        alpha = im.convert("RGBA").getchannel("A")
        alpha.thumbnail((320, 320))
        mask = np.asarray(alpha) > 160
    return silhouette_tips(mask)


def silhouette_tips(mask):
    """Ends of distinct tapering branches in a two-dimensional foreground mask."""
    import numpy as np
    from scipy.ndimage import binary_closing, binary_fill_holes, convolve, distance_transform_edt
    from skimage.morphology import skeletonize
    mask = binary_fill_holes(binary_closing(mask))
    if mask.sum() < 128:
        return None
    sk = skeletonize(mask)
    ys, xs = np.nonzero(mask)
    span = max(np.ptp(ys), np.ptp(xs))
    radii = distance_transform_edt(mask)

    def branches(skeleton):
        degree = convolve(skeleton.astype(int), np.ones((3, 3), dtype=int), mode="constant") - skeleton
        for y, x in np.argwhere(skeleton & (degree == 1)):
            previous = None
            path = [(y, x)]
            while True:
                neighbors = [(a, b) for a in range(y-1, y+2) for b in range(x-1, x+2)
                             if (a, b) != (y, x) and (a, b) != previous and skeleton[a, b]]
                if len(neighbors) != 1:
                    break
                previous = y, x
                y, x = neighbors[0]
                path.append((y, x))
            yield path, degree[y, x]

    # A curled tip or a sucker can fork the end of a single long arm into
    # several short branches. Strip those spurs, keeping their shared stem.
    spurs = [pixel for path, degree in branches(sk) if degree >= 3 and len(path) < .035*span
             for pixel in path[:-1]]
    if spurs:
        rows, cols = np.asarray(spurs).T
        sk[rows, cols] = False
        sk = skeletonize(sk)
    tips = []
    for path, _ in branches(sk):
        y, x = path[0]
        ey, ex = path[-1]
        if radii[y, x] > .065*span or len(path) < .095*span or np.hypot(ey-y, ex-x) < .065*span:
            continue
        if np.median([radii[a, b] for a, b in path]) <= .085*span:
            tips.append([int(y), int(x)])
    return tips


def split_base(matte):
    """Two wide contact patches with daylight between them indicate separate feet."""
    import numpy as np
    from PIL import Image
    with Image.open(matte) as image:
        mask = np.asarray(image.convert("RGBA"))[:, :, 3] > 196
    rows, cols = np.where(mask)
    if len(rows) < 128:
        return False
    height = rows.max() - rows.min()
    width = cols.max() - cols.min()
    votes = 0
    for level in (0.012, 0.019, 0.026, 0.033):
        row = mask[int(rows.max() - level * height)]
        ends = np.flatnonzero(np.diff(np.r_[False, row, False])).reshape(-1, 2)
        spans = [(a, b) for a, b in ends if b - a > 0.09 * width]
        if len(spans) == 2 and spans[1][0] - spans[0][1] > 0.08 * width:
            votes += 1
    return votes >= 2


def inspect(image, body, prompt, matte=None):
    expected = description(prompt, body)
    if body not in {"legless", "bird", "quadruped"} and expected is None:
        return {"status": "not_applicable"}
    if matte is not None and needs_base_check(prompt, body) and split_base(matte):
        return {"status": "rejected", "reason": "Two separate feet support the legless figure in the drawing."}
    details = {}
    if matte is not None and compact_legless_face(prompt, body):
        contacts = visible_feet(matte)
        if contacts is not None:
            details["lower_contacts"] = contacts
            if contacts >= 3 or (contacts == 2 and split_base(matte)):
                return {"status": "rejected", "visible_feet": contacts,
                        "reason": f"The legless body stands on {contacts} separate feet in the drawing."}
    if matte is not None and body == "other" and radial_count(prompt):
        tips = tentacle_tips(matte)
        if tips is not None:
            if len(tips) != radial_count(prompt):
                return {"status":"rejected", "visible_tips":len(tips),
                        "reason":f"The silhouette resolves {len(tips)} separate tapering tips; eight were requested."}
            details["visible_tips"] = len(tips)
    legs = paired_legs(prompt) if body == "other" else None
    if matte is not None and legs and not radial_count(prompt):
        seen = visible_feet(matte)
        if seen is not None:
            details["visible_feet"] = seen
            if seen != legs:
                return {"status": "rejected", **details,
                        "reason": f"The lower silhouette shows {seen} separate legs; {legs} were requested."}
    if sys.platform != "darwin":
        return {"status": "checked" if details else "unavailable", **details}
    # Vision handles the real source pixels locally. A lack of a human skeleton
    # is not positive species identification; this only rejects a clear mismatch.
    try:
        probe = subprocess.run(["/usr/bin/osascript", "-l", "JavaScript",
                                str(Path(__file__).with_name("human_joints.js")), str(image)],
                               capture_output=True, check=True, timeout=12)
        people = json.loads(probe.stdout)
    except (OSError, subprocess.SubprocessError, ValueError):
        return {"status": "checked" if details else "unavailable", **details}
    for joints in people:
        ends = []
        for side in ("left", "right"):
            shoulder = joints.get(side + "_shoulder_1_joint")
            elbow = joints.get(side + "_forearm_joint")
            hand = joints.get(side + "_hand_joint")
            if shoulder and elbow and hand and min(shoulder[2], elbow[2], hand[2]) >= 0.16:
                if shoulder[1] - hand[1] > 0.10:
                    ends.append(hand[0])
        if len(ends) == 2 and abs(ends[0] - ends[1]) > 0.24 and "head_joint" in joints:
            return {"status": "rejected", "reason": "The drawing has two human arms ending in hands in a standing A-pose."}
    return {"status": "checked", **details}
