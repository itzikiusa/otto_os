"""Blender worker for publisher assets that have no glTF edition."""

import bpy
import json
import sys
from pathlib import Path, PureWindowsPath


def convert(source, dest, pack):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    if source.suffix.lower() == ".blend":
        bpy.ops.wm.open_mainfile(filepath=str(source), load_ui=False, use_scripts=False)
    elif source.suffix.lower() == ".fbx":
        bpy.ops.import_scene.fbx(filepath=str(source), use_image_search=True)
    elif source.suffix.lower() == ".obj":
        bpy.ops.wm.obj_import(filepath=str(source))
    else:
        raise ValueError(str(source))
    missing = []
    for im in bpy.data.images:
        if im.source != "FILE" or im.packed_file or not im.users:
            continue
        original = Path(bpy.path.abspath(im.filepath))
        if original.is_file():
            continue
        matches = sorted(pack.rglob(PureWindowsPath(original.name).name))
        if matches:
            im.filepath = str(matches[0])
            im.reload()
        else:
            missing.append(original.name)
    dest.parent.mkdir(exist_ok=True, parents=True)
    temp = dest.with_name(dest.stem + ".partial.glb")
    bpy.ops.export_scene.gltf(
        filepath=str(temp),
        export_format="GLB",
        export_cameras=False,
        export_lights=False,
    )
    temp.replace(dest)
    dest.with_suffix(".json").write_text(json.dumps({"missing_source_images": missing}))


if __name__ == "__main__":
    jobs = json.loads(Path(sys.argv[-1]).read_text())
    for row in jobs:
        convert(*map(Path, row))
