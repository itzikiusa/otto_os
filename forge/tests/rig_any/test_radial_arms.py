"""A fan whose branches are distinct in the mesh can drive eight independently skinned arms."""
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np
import trimesh
from scipy.spatial import cKDTree

from forge import bodyplan, cli, glbstats
from forge.library.gltf import write
from forge.quality import radial


class RadialRig(unittest.TestCase):
    def setUp(self):
        self.work = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.source = self.work / "fan.glb"
        self.output = self.work / "rig.glb"

    def fan(self, count):
        mantle = trimesh.creation.icosphere(subdivisions=2, radius=.24)
        mantle.apply_translation([0, .16, 0])
        parts = [mantle]
        for angle in np.arange(count) * 2 * np.pi / count:
            ray = np.array([np.cos(angle), 0., np.sin(angle)])
            arm = trimesh.creation.cone(radius=.10, height=.9, sections=16)
            arm.apply_transform(trimesh.geometry.align_vectors([0, 0, 1], ray))
            arm.apply_translation(.1 * ray)
            parts.append(arm)
        mesh = trimesh.util.concatenate(parts)
        mesh.export(self.source)
        return mesh

    def test_ordinary_rig_binds_eight_separate_arms_to_the_original_skin(self):
        mesh = self.fan(8)
        # A generated fan can be stored upright locally and rotated flat by the bake.
        doc, data = glbstats.read_glb(self.source)
        mesh_node = next(n for n in doc["nodes"] if "mesh" in n)
        matrix = trimesh.transformations.rotation_matrix(np.pi / 3, [0, 0, 1])
        for prim in doc["meshes"][mesh_node["mesh"]]["primitives"]:
            pos = doc["accessors"][prim["attributes"]["POSITION"]]
            view = doc["bufferViews"][pos["bufferView"]]
            data = bytearray(data)
            vertices = np.ndarray((pos["count"], 3), "<f4", data,
                                  offset=view.get("byteOffset", 0) + pos.get("byteOffset", 0))
            vertices[:] = vertices @ matrix[:3, :3]
            pos.update(min=vertices.min(axis=0).tolist(), max=vertices.max(axis=0).tolist())
        mesh_node["matrix"] = matrix.T.flatten().tolist()
        write(doc, data, self.source)
        # The same command as Smart uses. Contact count and skin weights must both agree.
        with patch.object(cli.Run, "run", side_effect=AssertionError("used the generic predictor")):
            status = cli.main(["rig", "--mesh", str(self.source), "--prompt",
                               "Orange octopus with eight arms", "--output", str(self.output)])
        self.assertEqual(status, 0)
        rig = bodyplan.Rig(self.output)
        root = rig.parent.index(-1)
        self.assertEqual(len(rig.children[root]), 8)
        self.assertLess(cKDTree(rig.skin_rest).query(mesh.vertices)[0].max(), 1e-5)
        self.assertLess(cKDTree(mesh.vertices).query(rig.skin_rest)[0].max(), 1e-5)
        # Smart uses the skeleton frame to size the rest mesh before applying the pose.
        frame = rig.offset[root]
        display = rig.bind @ frame[:3, :3].T + frame[:3, 3]
        np.testing.assert_allclose(np.ptp(display, axis=0), mesh.extents, atol=1e-5)
        leaves = [i for i, children in enumerate(rig.children) if not children]
        for tip in leaves:
            nearest = np.linalg.norm(rig.skin_rest - rig.heads[tip], axis=1).argmin()
            self.assertEqual(rig.dominant[nearest], tip)

    def test_seven_observed_branches_defer_to_normal_prediction(self):
        self.fan(7)
        self.assertFalse(radial.fit_arms(self.source, self.output, 8))
        self.assertFalse(self.output.exists())
