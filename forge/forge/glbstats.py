"""Numbers for one GLB: triangles, vertices, materials and their maps, bones, clips, size.

Pure glTF parse (JSON chunk + embedded image headers), no Blender.
    python -m forge.glbstats model.glb [stats.json]
"""

from __future__ import annotations

import io
import json
import struct
import sys
from pathlib import Path

MAPS = {"baseColorTexture": "base_color", "metallicRoughnessTexture": "metallic_roughness"}


def read_glb(path: Path) -> tuple[dict, bytes]:
    data = path.read_bytes()
    magic, _version, _length = struct.unpack_from("<4sII", data, 0)
    if magic != b"glTF":
        raise ValueError(f"{path} is not a binary glTF")
    offset, gltf, binary = 12, None, b""
    while offset < len(data):
        size, kind = struct.unpack_from("<I4s", data, offset)
        chunk = data[offset + 8: offset + 8 + size]
        if kind == b"JSON":
            gltf = json.loads(chunk)
        elif kind == b"BIN\0":
            binary = chunk
        offset += 8 + size
    return gltf, binary


def texture_source(texture: dict) -> int | None:
    if "source" in texture:
        return texture["source"]
    return next((ext["source"] for ext in texture.get("extensions", {}).values() if "source" in ext), None)


def image_size(gltf: dict, binary: bytes, index: int) -> list[int] | None:
    from PIL import Image

    image = gltf["images"][index]
    if "bufferView" not in image:
        return None
    view = gltf["bufferViews"][image["bufferView"]]
    start = view.get("byteOffset", 0)
    try:
        return list(Image.open(io.BytesIO(binary[start: start + view["byteLength"]])).size)
    except Exception:  # KTX2/WebP the local Pillow cannot open: report the mime type only
        return None


def stats(path: Path) -> dict:
    gltf, binary = read_glb(path)
    accessors = gltf.get("accessors", [])
    def mesh_counts(mesh: dict) -> tuple[int, int]:
        tris = verts = 0
        for prim in mesh["primitives"]:
            if prim.get("mode", 4) == 4:
                count = accessors[prim["attributes"]["POSITION"]]["count"]
                verts += count
                tris += (accessors[prim["indices"]]["count"] if "indices" in prim else count) // 3
        return tris, verts

    # LOD0 only: LOD1.. and UCX_ collision nodes (the game-ready stage's extras) are listed apart
    triangles = vertices = 0
    for node in gltf.get("nodes", []):
        name = node.get("name", "")
        if "mesh" in node and "_LOD" not in name and not name.startswith("UCX_"):
            tris, verts = mesh_counts(gltf["meshes"][node["mesh"]])
            triangles, vertices = triangles + tris, vertices + verts

    def texture(info: dict | None) -> dict | None:
        if not info:
            return None
        tex = gltf["textures"][info["index"]]
        source = texture_source(tex)
        image = gltf["images"][source] if source is not None else {}
        return {"size": image_size(gltf, binary, source) if source is not None else None,
                "mime": image.get("mimeType")}

    materials = []
    for mat in gltf.get("materials", []):
        pbr = mat.get("pbrMetallicRoughness", {})
        maps = {name: texture(pbr.get(key)) for key, name in MAPS.items() if pbr.get(key)}
        for key, name in (("normalTexture", "normal"), ("occlusionTexture", "occlusion"),
                          ("emissiveTexture", "emissive")):
            if mat.get(key):
                maps[name] = texture(mat[key])
        materials.append({"name": mat.get("name"), "maps": maps,
                          "metallic_factor": pbr.get("metallicFactor", 1.0),
                          "roughness_factor": pbr.get("roughnessFactor", 1.0),
                          "alpha_mode": mat.get("alphaMode", "OPAQUE")})

    nodes = gltf.get("nodes", [])
    bones = sorted({nodes[j].get("name", f"node_{j}") for skin in gltf.get("skins", [])
                    for j in skin["joints"]}, key=str)
    clips = []
    for anim in gltf.get("animations", []):
        end = max((accessors[s["input"]].get("max", [0])[0] for s in anim["samplers"]), default=0)
        clips.append({"name": anim.get("name"), "duration_s": round(end, 3),
                      "channels": len(anim["channels"])})
    names = [n.get("name", "") for n in nodes]
    return {"file": str(path), "bytes": path.stat().st_size, "triangles": triangles,
            "vertices": vertices, "meshes": len(gltf.get("meshes", [])),
            "materials": materials, "bone_count": len(bones), "bones": bones, "clips": clips,
            "lods": [{"node": n["name"], "triangles": mesh_counts(gltf["meshes"][n["mesh"]])[0]}
                     for n in nodes if "_LOD" in n.get("name", "") and "mesh" in n],
            "collision_hulls": [n for n in names if n.startswith("UCX_")],
            "generator": gltf.get("asset", {}).get("generator"),
            "extensions_used": gltf.get("extensionsUsed", [])}


if __name__ == "__main__":
    result = stats(Path(sys.argv[1]))
    text = json.dumps(result, indent=2)
    if len(sys.argv) > 2:
        Path(sys.argv[2]).write_text(text + "\n")
    print(text)
