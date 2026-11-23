"""Embed sockets and buffers in a publisher base without changing its animation frames."""
import json
import struct
from gltf import pack_glb

from mathutils import Matrix, Quaternion, Vector


def matrix_of(node):
    if "matrix" in node:
        return Matrix([node["matrix"][i:i+4] for i in range(0, 16, 4)]).transposed()
    x, y, z, w = node.get("rotation", (0, 0, 0, 1))
    return Matrix.LocRotScale(Vector(node.get("translation", (0, 0, 0))), Quaternion((w, x, y, z)),
                              Vector(node.get("scale", (1, 1, 1))))


def inverse_binds(source):
    doc = json.loads(source.read_bytes())
    skin = doc["skins"][0]
    a = doc["accessors"][skin["inverseBindMatrices"]]
    view = doc["bufferViews"][a["bufferView"]]
    raw = (source.parent / doc["buffers"][view["buffer"]]["uri"]).read_bytes()
    offset = a.get("byteOffset", 0) + view.get("byteOffset", 0)
    if a["componentType"] != 5126 or a["type"] != "MAT4" or a["count"] != len(skin["joints"]):
        raise ValueError("Expected one float inverse bind matrix per joint")
    result = {}
    for j, node in enumerate(skin["joints"]):
        entries = struct.unpack_from("<16f", raw, offset + view.get("byteStride", 64)*j)
        result[doc["nodes"][node]["name"]] = Matrix([entries[i:i+4] for i in range(0, 16, 4)]).transposed()
    return result


def pack_base(source, source_dir, dest, parents, frames):
    """Only append socket nodes and image views; original binary offsets and bind matrices stay intact."""
    doc = json.loads(source.read_bytes())
    nodes = doc["nodes"]
    worlds = {}
    def visit(i, parent):
        worlds[i] = parent @ matrix_of(nodes[i])
        for j in nodes[i].get("children", []):
            visit(j, worlds[i])
    for i in doc["scenes"][doc.get("scene", 0)]["nodes"]:
        visit(i, Matrix.Identity(4))
    joints = {nodes[i]["name"]: i for i in doc["skins"][0]["joints"]}
    blender_to_gltf = Matrix(((1,0,0,0), (0,0,1,0), (0,-1,0,0), (0,0,0,1)))
    for key, bone in parents.items():
        i = joints[bone]
        world = blender_to_gltf @ Matrix(frames[key]) @ blender_to_gltf.inverted()
        local = worlds[i].inverted() @ world
        nodes[i].setdefault("children", []).append(len(nodes))
        nodes.append({"name": key, "matrix": [v for col in local.col for v in col],
                      "extras": {"forge_socket": key}})
    socketed = source.with_name("socketed.gltf")
    socketed.write_text(json.dumps(doc))
    pack_glb(socketed, source_dir, dest)
