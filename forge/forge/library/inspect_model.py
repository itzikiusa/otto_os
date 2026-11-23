"""Measure the meshes instantiated in the default glTF scene in publisher metres."""

import re
from collections import Counter

import numpy as np

from forge.quality.package import local_matrix

from .gltf import buffers, read


def node_matrix(node):
    return np.asarray(local_matrix(node))


def inspect(source, pack, *, geometry=True):
    doc, embed = read(source, include_binary=geometry)
    data = buffers(doc, embed, source, pack) if geometry else []
    lo = np.full(3, np.inf)
    hi = -lo.copy()
    tris = 0
    nodes = doc.get("nodes", [])

    def visit(i, parent):
        nonlocal lo, hi, tris
        n = nodes[i]
        world = parent @ node_matrix(n)
        if "mesh" in n:
            for p in doc["meshes"][n["mesh"]]["primitives"]:
                a = doc["accessors"][p["attributes"]["POSITION"]]
                if geometry:
                    view = doc["bufferViews"][a["bufferView"]]
                    if a["componentType"] != 5126 or "sparse" in a:
                        raise ValueError(f"Unsupported POSITION encoding in {source}")
                    pos = np.ndarray(
                        (a["count"], 3),
                        dtype="<f4",
                        buffer=data[view["buffer"]],
                        offset=view.get("byteOffset", 0) + a.get("byteOffset", 0),
                        strides=(view.get("byteStride", 12), 4),
                    )
                    points = pos @ world[:3, :3].T + world[:3, 3]
                    lo = np.minimum(lo, points.min(axis=0))
                    hi = np.maximum(hi, points.max(axis=0))
                count = (
                    doc["accessors"][p["indices"]]["count"]
                    if "indices" in p
                    else a["count"]
                )
                mode = p.get("mode", 4)
                tris += (
                    count // 3
                    if mode == 4
                    else max(0, count - 2)
                    if mode in (5, 6)
                    else 0
                )
        for child in n.get("children", []):
            visit(child, world)

    for i in doc.get("scenes", [{}])[doc.get("scene", 0)].get("nodes", []):
        visit(i, np.eye(4))
    parents = {c: i for i, n in enumerate(nodes) for c in n.get("children", [])}
    skins = []
    for skin in doc.get("skins", []):
        joints = skin["joints"]
        roots = [j for j in joints if parents.get(j) not in joints]
        digits = Counter()
        for j in joints:
            name = nodes[j].get("name", "").lower()
            m = re.search("(thumb|index|middle|ring|pinky|little)", name)
            if m:
                side = (
                    "left"
                    if "left" in name or name.endswith(("_l", ".l"))
                    else "right"
                    if "right" in name or name.endswith(("_r", ".r"))
                    else "unsided"
                )
                digits[side + "_" + m.group()] += 1
        skins.append(
            dict(
                name=skin.get("name", ""),
                roots=[nodes[j].get("name", "") for j in roots],
                joints=len(joints),
                finger_joint_counts=dict(sorted(digits.items())),
            )
        )
    images = " ".join(
        im.get("name", "")
        + " "
        + (im.get("uri", "")[:120] if not im.get("uri", "").startswith("data:") else "")
        for im in doc.get("images", [])
    ).lower()
    materials = doc.get("materials", [])
    pbr = any(
        "normalTexture" in m
        or "metallicRoughnessTexture" in m.get("pbrMetallicRoughness", {})
        for m in materials
    )
    if "palette" in images:
        style = "palette"
    elif "atlas" in images or "trim" in images:
        style = "atlas / trim sheet"
    elif any(
        "baseColorTexture" in m.get("pbrMetallicRoughness", {}) for m in materials
    ):
        style = "base-colour texture"
    elif any(
        "COLOR_0" in p.get("attributes", {})
        for m in doc.get("meshes", [])
        for p in m["primitives"]
    ):
        style = "vertex colour"
    else:
        style = "material colours"
    if pbr:
        style += " + PBR maps"
    return dict(
        triangles=tris,
        size_m=[round(x, 6) for x in hi - lo] if np.isfinite(lo).all() else [0, 0, 0],
        rig=skins,
        animations=[a.get("name", "") for a in doc.get("animations", [])],
        texture=style,
    )
