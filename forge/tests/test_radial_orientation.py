"""Regression coverage for shallow mantles in an upright fan of arms."""
import tempfile
import unittest
from pathlib import Path

import numpy as np
import trimesh

from forge import glbstats
from forge.quality import radial


def shallow_fan(bulge):
    y, x = np.mgrid[-1:1:121j, -1:1:121j]
    radius2 = x*x + y*y
    points = np.column_stack([x.ravel(), y.ravel(), np.full(x.size, -.015)])
    start = np.arange(x.size).reshape(x.shape)[:-1, :-1][radius2[:-1, :-1] < .97]
    faces = np.vstack([np.column_stack([start, start + 1, start + 121]),
                       np.column_stack([start + 1, start + 122, start + 121])])
    bottom = trimesh.Trimesh(points.copy(), faces[:, ::-1])
    points[:, 2] = .015 + bulge * np.exp(-radius2.ravel() / .065**2)
    return trimesh.util.concatenate([bottom, trimesh.Trimesh(points, faces)])


class RadialOrientation(unittest.TestCase):
    def test_shallow_mantle_and_symmetric_body_in_both_orientations(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "fan.glb"
            for bulge in (0, .19):
                for sign in (-1, 1):
                    with self.subTest(bulge=bulge, sign=sign):
                        mesh = shallow_fan(bulge)
                        mesh.apply_scale([1, 1, sign])
                        mesh.export(path)
                        doc, _ = glbstats.read_glb(path)
                        result = radial.align(path, doc)
                        if bulge:
                            self.assertIsNotNone(result)
                            self.assertGreater(sign * result["arm_plane_normal"][2], .99)
                        else:
                            self.assertIsNone(result)
