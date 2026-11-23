"""Check and correct Unity digit controls on a front-facing SkinTokens GLB."""
import json

import numpy as np
from scipy.optimize import linear_sum_assignment
from scipy.sparse import coo_matrix
from scipy.sparse.csgraph import connected_components
from scipy.spatial.transform import Rotation

from forge.glbstats import read_glb

DIGITS = ("Thumb", "Index", "Middle", "Ring", "Little")
SEGMENTS = ("Proximal", "Intermediate", "Distal")


class HandSkin:
    """A writable view of one SkinTokens mesh, in glTF Y-up rest space."""
    def __init__(self, path):
        self.doc, binary = read_glb(path)
        self.binary = bytearray(binary)
        nodes = self.doc["nodes"]
        skinned = [node for node in nodes if "skin" in node and "mesh" in node]
        if len(skinned) != 1:
            raise ValueError("hand intake requires one skinned display mesh")
        mesh = skinned[0]
        primitives = self.doc["meshes"][mesh["mesh"]]["primitives"]
        if len(primitives) != 1 or primitives[0].get("mode", 4) != 4:
            raise ValueError("hand intake requires one triangle primitive")
        primitive = primitives[0]
        attrs = primitive["attributes"]
        if "JOINTS_1" in attrs or self.doc["accessors"][attrs["WEIGHTS_0"]]["componentType"] != 5126:
            raise ValueError("hand intake requires the four float weights exported by SkinTokens")
        self.ids = self.accessor(attrs["JOINTS_0"])
        self.weights = self.accessor(attrs["WEIGHTS_0"])
        xyz = self.accessor(attrs["POSITION"])
        self.homogeneous = np.column_stack((xyz, np.ones(len(xyz))))
        self.faces = self.accessor(primitive["indices"]).reshape(-1, 3)
        skin = self.doc["skins"][mesh["skin"]]
        self.joints = np.array(skin["joints"])
        self.inverse_bind = self.accessor(skin["inverseBindMatrices"]).reshape(-1, 4, 4).transpose(0, 2, 1)
        self.named = {nodes[n].get("name"): i for i, n in enumerate(self.joints)}
        parents = {child: i for i, n in enumerate(nodes) for child in n.get("children", [])}
        self.parents = parents
        local = []
        for n in nodes:
            if "matrix" in n:
                local.append(np.asarray(n["matrix"]).reshape(4, 4).T)
                continue
            m = np.eye(4)
            m[:3, 3] = n.get("translation", (0, 0, 0))
            m[:3, :3] = Rotation.from_quat(n.get("rotation", (0, 0, 0, 1))).as_matrix() @ np.diag(n.get("scale", (1, 1, 1)))
            local.append(m)
        solved = {}
        def world(i):
            if i not in solved:
                solved[i] = world(parents[i]) @ local[i] if i in parents else local[i]
            return solved[i]
        self.world = np.stack([world(n) for n in self.joints])
        self.rest = self.points()
        self.height = float(np.ptp(self.rest[:, 1]))
        if self.height <= 0:
            raise ValueError("mesh has zero height")
        mid = (self.rest[:, 0].min() + self.rest[:, 0].max()) / 2
        self.front = np.column_stack((550 + (self.rest[:, 0]-mid)*1000/self.height,
                                      1020 - (self.rest[:, 1]-self.rest[:, 1].min())*1000/self.height))

    def accessor(self, index):
        a = self.doc["accessors"][index]
        if a.get("normalized") or "sparse" in a:
            raise ValueError("hand intake requires dense, unnormalized accessors")
        v = self.doc["bufferViews"][a["bufferView"]]
        kind = np.dtype({5121:"u1", 5123:"<u2", 5125:"<u4", 5126:"<f4"}[a["componentType"]])
        width = {"SCALAR":1, "VEC3":3, "VEC4":4, "MAT4":16}[a["type"]]
        return np.ndarray((a["count"], width), dtype=kind, buffer=self.binary,
                          offset=a.get("byteOffset", 0) + v.get("byteOffset", 0),
                          strides=(v.get("byteStride", width*kind.itemsize), kind.itemsize))

    def points(self, world=None):
        matrices = (self.world if world is None else world) @ self.inverse_bind
        points = np.zeros((len(self.ids), 3))
        for j, weight in zip(self.ids.T, self.weights.T):
            points += weight[:, None] * np.einsum("vij,vj->vi", matrices[j, :3], self.homogeneous)
        totals = self.weights.sum(axis=1, keepdims=True)
        if np.any(totals <= 0):
            raise ValueError("skinned display mesh contains an unweighted vertex")
        return points / totals

    def chains(self, side):
        chains = []
        for digit in DIGITS:
            chain = [self.named.get(side+digit+seg) for seg in SEGMENTS]
            if None in chain:
                raise ValueError(f"{side}{digit} is missing one of its three named joints")
            if any(self.parents.get(self.joints[b]) != self.joints[a] for a,b in zip(chain, chain[1:])):
                raise ValueError(f"{side}{digit} does not form a finger chain")
            chains.append(chain)
        return chains

    def measure(self, chains, patches):
        rotation = np.eye(4)
        rotation[:3, :3] = Rotation.from_euler("x", 20, degrees=True).as_matrix()
        result = []
        for chain in chains:
            pivot = self.world[chain[0]]
            posed = self.world.copy()
            posed[chain] = pivot @ rotation @ np.linalg.inv(pivot) @ self.world[chain]
            movement = np.linalg.norm(self.points(posed) - self.rest, axis=1)*1000/self.height
            result.append([np.median(movement[patch]) for patch in patches])
        return np.array(result)

    def distal_regions(self, marks, tips):
        palm, wrist = np.asarray(marks["palm"]), np.asarray(marks["wrist"])
        axis = palm - wrist
        cut, reach = marks["cut"], marks["reach"]
        if not np.linalg.norm(axis) or not 0 < cut < reach:
            raise ValueError("invalid palm cut, reach or wrist direction")
        _, first, inverse = np.unique(np.rint(self.rest/(self.height*1e-5)).astype(int), axis=0,
                                      return_index=True, return_inverse=True)
        inverse = inverse.ravel()
        delta = self.front[first] - palm
        radius = np.linalg.norm(delta, axis=1)
        keep = np.flatnonzero((radius > cut) & (radius < reach) & (delta @ (axis/np.linalg.norm(axis)) > -.18*cut))
        if not len(keep):
            raise ValueError("no surface beyond the annotated palm")
        edges = inverse[self.faces[:, [0,1,1,2,2,0]].reshape(-1, 2)]
        graph = coo_matrix((np.ones(len(edges), bool), edges.T), shape=(len(first), len(first))).tocsr()
        _, labels = connected_components(graph[keep][:, keep], directed=False)
        matched = [labels[np.argmin(np.linalg.norm(self.front[first[keep]]-tip, axis=1))] for tip in tips]
        if len(set(matched)) != 5:
            raise ValueError("finger tips must identify five distinct distal mesh branches")
        return [np.flatnonzero(np.isin(inverse, keep[labels == label])) for label in matched]

    def remove_leak(self, region, intended, foreign):
        changed = 0
        joints = self.world[intended, :3, 3]
        for vertex in region:
            ids, weights = self.ids[vertex], self.weights[vertex]
            wrong = np.isin(ids, foreign) & (weights > 0)
            if not wrong.any():
                continue
            nearest = intended[np.argmin(np.linalg.norm(joints-self.rest[vertex], axis=1))]
            totals = {}
            for bone, w, replace in zip(ids, weights, wrong):
                if w > 0:
                    k = nearest if replace else int(bone)
                    totals[k] = totals.get(k, 0.) + float(w)
            pairs = sorted(totals.items(), key=lambda item: -item[1])
            ids[:] = 0
            weights[:] = 0
            for i,(k,w) in enumerate(pairs):
                ids[i], weights[i] = k,w
            changed += 1
        return changed


def correct(source, landmarks):
    skin = HandSkin(source)
    marks = json.loads(landmarks.read_text())
    report = {}
    for side in ("Left", "Right"):
        hand = marks[side]
        tips = np.array([hand["tips"][digit] for digit in DIGITS], dtype=float)
        if tips.shape != (5,2) or not np.isfinite(tips).all():
            raise ValueError(f"{side} requires five finite 2D tip coordinates")
        regions = skin.distal_regions(hand, tips)
        patches = [np.argsort(np.linalg.norm(skin.front-t, axis=1))[:8] for t in tips]
        if max(np.median(np.linalg.norm(skin.front[p]-t, axis=1)) for p,t in zip(patches,tips)) > 6:
            raise ValueError(f"{side} tip landmarks miss the skin")
        chains = skin.chains(side)
        before = skin.measure(chains, patches)
        # A chain must independently prefer its assigned digit; arbitrary tie-breaks are unsafe.
        _, assignment = linear_sum_assignment(-before)
        for row,target in zip(before, assignment):
            if row[target] < 1 or row[target] < 1.1*np.max(np.delete(row,target)):
                raise ValueError(f"{side} has ambiguous digit motion")
        rename = {}
        anatomical = [None]*5
        for i,target in enumerate(assignment):
            anatomical[target] = chains[i]
            if i != target:
                rename[DIGITS[i]] = DIGITS[target]
                for j,seg in zip(chains[i], SEGMENTS):
                    skin.doc["nodes"][skin.joints[j]]["name"] = side+DIGITS[target]+seg
        changed = 0
        ordered = before[np.argsort(assignment)]
        for i,region in enumerate(regions):
            if np.max(np.delete(ordered[:, i],i)) > .2*ordered[i,i]:
                foreign = [j for k,chain in enumerate(anatomical) if k != i for j in chain]
                changed += skin.remove_leak(region, anatomical[i], foreign)
        after = skin.measure(anatomical, patches)
        if any(row[i] < 1 or np.max(np.delete(after[:,i],i)) > .2*row[i] or row.argmax() != i
               for i,row in enumerate(after)):
            raise ValueError(f"{side} still fails the independent control check after remapping")
        report[side] = dict(renamed=rename, weight_rows_changed=changed, before=before.round(3).tolist(),
                            after=after.round(3).tolist())
    error = float(np.max(np.linalg.norm(skin.points()-skin.rest, axis=1)) * 1000/skin.height)
    if not np.isfinite(error) or error > .001:
        raise ValueError("weight correction moved the bind-pose surface")
    report.update(max_rest_error_per_1000_height=error, digit_order=DIGITS, landmarks=str(landmarks))
    return skin.doc, skin.binary, report
