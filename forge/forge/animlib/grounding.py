"""Offline sole contact correction on the target's evaluated skin, in world space."""

import math

import bpy
import numpy as np
from mathutils import Matrix


class SoleSampler:
    def __init__(self, rig, objects, joints):
        self.rig = rig
        self.hip = rig.pose.bones[joints["hips"].name]
        self.feet = [rig.pose.bones[joints[s + "foot"].name] for s in "lr"]
        self.meshes = []
        self.indices = [[], []]
        rest = []
        total = 0
        rig.data.pose_position = "REST"
        bpy.context.view_layer.update()
        for obj in objects:
            if (
                obj.type != "MESH"
                or obj.find_armature() != rig
                or not obj.data.vertices
            ):
                continue
            buf = np.empty(len(obj.data.vertices) * 3, dtype=np.float32)
            points = self.points(obj, buf)
            rest.append(points)
            for i, side in enumerate("lr"):
                names = {
                    joints[k].name for k in (side + "foot", side + "toe") if k in joints
                }
                groups = {g.index for g in obj.vertex_groups if g.name in names}
                self.indices[i].extend(
                    total + v.index
                    for v in obj.data.vertices
                    if sum(g.weight for g in v.groups if g.group in groups) > 0.5
                )
            self.meshes.append((obj, buf))
            total += len(points)
        rest = np.concatenate(rest)
        self.floor = float(rest[:, 2].min())
        self.height = max(float(np.ptp(rest[:, 2])), 1e-6)
        self.markers = []
        for ids in self.indices:
            ids = np.array(ids, dtype=int)
            self.markers.append(
                ids[rest[ids, 2] <= np.percentile(rest[ids, 2], 10)]
                if len(ids)
                else ids
            )
        rig.data.pose_position = "POSE"
        bpy.context.view_layer.update()

    @staticmethod
    def points(obj, buf):
        ob = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
        ob.data.vertices.foreach_get("co", buf)
        mat = np.asarray(ob.matrix_world)
        return buf.reshape(-1, 3) @ mat[:3, :3].T + mat[:3, 3]

    def sample(self):
        coords = np.concatenate([self.points(obj, buf) for obj, buf in self.meshes])
        feet = []
        for i, ids in enumerate(self.indices):
            if ids:
                xy = coords[self.markers[i], :2].mean(axis=0)
                feet.append((*xy, coords[ids, 2].min()))
            else:
                feet.append(tuple(self.rig.matrix_world @ self.feet[i].head))
        # Local pelvis translations are in the animated parent's bind basis,
        # including any armature object scale. The pelvis' own rotation is irrelevant.
        hip = self.hip
        parent = (
            {}
            if hip.parent is None
            else {
                "parent_matrix": hip.parent.matrix,
                "parent_matrix_local": hip.parent.bone.matrix_local,
            }
        )
        base = hip.bone.convert_local_to_pose(
            Matrix.Identity(4), hip.bone.matrix_local, **parent
        )
        inverse = (self.rig.matrix_world @ base).to_3x3().inverted_safe()
        return feet, np.asarray(inverse)


def stance_correction(seconds, feet, floor, height, job):
    """Stabilize low soles about an anchor, allowing the belt speed of an in-place gait."""
    # Extend a closed in-place take around the seam. A stance is often split by frame zero.
    repeat = job["loopable"] and not job["root_motion"]
    n = len(seconds)
    if repeat:
        duration = seconds[-1] - seconds[0]
        seconds = np.r_[seconds[:-1] - duration, seconds, seconds[1:] + duration]
        feet = np.concatenate((feet[:-1], feet, feet[1:]))
    runs = []
    for side in range(2):
        contact = feet[:, side, 2] <= floor + 0.02 * height
        if job["root_motion"]:
            # A grounded shoe can still be intentionally travelling. Do not
            # anchor a whole take across a genuine stationary-to-gliding transition.
            speed = np.linalg.norm(
                np.gradient(feet[:, side, :2], seconds, axis=0), axis=1
            )
            contact &= speed < 0.15 * height
        low = np.flatnonzero(contact)
        for ix in np.split(low, np.where(np.diff(low) != 1)[0] + 1):
            if len(ix) >= 2 and seconds[ix[-1]] - seconds[ix[0]] >= 0.10:
                runs.append((ix, side))

    velocity = np.zeros(2)
    if runs and job["category"] == "locomotion" and not job["root_motion"]:
        variance = 0.0
        for ix, side in runs:
            t = seconds[ix] - seconds[ix].mean()
            velocity += t @ feet[ix, side, :2]
            variance += t @ t
        velocity /= variance

    desired = np.zeros((len(seconds), 2))
    weights = np.zeros(len(seconds))
    for ix, side in runs:
        t = seconds[ix] - seconds[ix].mean()
        xy = feet[ix, side, :2]
        desired[ix] += xy.mean(axis=0) + t[:, None] * velocity - xy
        weights[ix] += 1
    desired /= np.maximum(weights[:, None], 1)
    # Crossfade acquisition/release over 100 ms. Bound pelvis travel on noisy
    # or deliberately sliding captures without changing their net root motion.
    magnitude = np.linalg.norm(desired, axis=1)
    desired *= np.minimum(1, 0.10 * height / np.maximum(magnitude, 1e-12))[:, None]
    radius = min(len(seconds) - 1, max(1, round(0.05 / (seconds[1] - seconds[0]))))
    pad = np.pad(desired, ((radius, radius), (0, 0)), mode="edge")
    kernel = np.arange(1, radius + 2, dtype=float)
    kernel = np.r_[kernel, kernel[-2::-1]]
    smooth = np.stack(
        [np.convolve(ax, kernel / kernel.sum(), mode="valid") for ax in pad.T], axis=1
    )
    if repeat:
        smooth = smooth[n - 1 : 2 * n - 1]
        smooth[-1] = smooth[0]
    return smooth


def ground_action(soles, action, slot, job, fps, add_curve):
    scene = bpy.context.scene
    start, end = action.frame_range
    # Use exact half frames. A float32 end of 64.00002 must not shift every
    # sample off the authored contact keys by dividing the take into 65 bins.
    grid = np.r_[np.arange(start, end - 1e-4, 0.5), end]
    if len(grid) == 1:
        grid = np.array([start, end])
    feet, bases = [], []
    for t in grid:
        scene.frame_set(math.floor(t), subframe=t - math.floor(t))
        pair, basis = soles.sample()
        feet.append(pair)
        bases.append(basis)
    feet = np.asarray(feet)
    xy = stance_correction(grid / fps, feet, soles.floor, soles.height, job)
    # Small interpolation tolerance between samples on fast heel/toe rocks.
    lift = np.maximum(
        0.0, soles.floor + 0.0004 * soles.height - feet[:, :, 2].min(axis=1)
    )
    # Take a one-sample upper envelope during fast toe rolls: rotations
    # interpolate along arcs between the baked linear translation keys.
    extended = np.pad(lift, (1, 1), mode="edge")
    lift = np.maximum.reduce((extended[:-2], lift, extended[2:]))
    delta = np.einsum("nij,nj->ni", bases, np.column_stack((xy, lift)))
    bag = action.layers[0].strips[0].channelbag(slot)
    path = soles.hip.path_from_id("location")
    curves = [bag.fcurves.find(path, index=i) for i in range(3)]
    # Retain authored off-grid translation keys as well as the contact samples.
    times = sorted(
        set(grid).union(p.co.x for curve in curves for p in curve.keyframe_points)
    )
    values = np.stack(
        [
            [curve.evaluate(t) for t in times] + np.interp(times, grid, delta[:, i])
            for i, curve in enumerate(curves)
        ],
        axis=1,
    )
    for i, curve in enumerate(curves):
        bag.fcurves.remove(curve)
        add_curve(bag, path, i, times, values)
