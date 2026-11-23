"""Exercise the public CLI with an embedded, interleaved, quantized two-joint skin."""

import json
import struct
import subprocess
import tempfile
import unittest
from pathlib import Path

CLI = Path(__file__).resolve().parents[2] / "bin/forge-skinfix"


def document(data):
    length = struct.unpack_from("<I", data, 12)[0]
    return json.loads(data[20 : 20 + length]), data[28 + length :]


def fixture(path):
    binary = bytearray()
    views, accessors = [], []

    def attr(rows, kind, fmt, typ, stride=None, **kwargs):
        offset = len(binary)
        for row in rows:
            record = struct.pack("<" + fmt, *row)
            binary.extend(record.ljust(stride or len(record), b"\0"))
        views.append(
            {"buffer": 0, "byteOffset": offset, "byteLength": len(binary) - offset}
        )
        if stride:
            views[-1]["byteStride"] = stride
        accessors.append(
            dict(
                bufferView=len(views) - 1,
                count=len(rows),
                componentType=kind,
                type=typ,
                **kwargs,
            )
        )
        return len(accessors) - 1

    identity = (1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)
    inverse = list(identity)
    inverse[13] = -1
    attrs = {
        "POSITION": attr(
            [(0, 0, 0), (0, 0.5, 0), (0, 1, 0)],
            5126,
            "3f",
            "VEC3",
            min=[0, 0, 0],
            max=[0, 1, 0],
        ),
        "JOINTS_0": attr([(0, 1, 0, 0)] * 3, 5121, "4B", "VEC4"),
        "WEIGHTS_0": attr(
            [(170, 85, 0, 0)] * 3, 5121, "4B", "VEC4", 8, normalized=True
        ),
    }
    ibm = attr([identity, inverse], 5126, "16f", "MAT4")
    data = {
        "asset": {"version": "2.0"},
        "buffers": [{"byteLength": len(binary)}],
        "bufferViews": views,
        "accessors": accessors,
        "meshes": [{"primitives": [{"attributes": attrs}]}],
        "skins": [{"joints": [1, 2], "inverseBindMatrices": ibm}],
        "nodes": [
            {"mesh": 0, "skin": 0},
            {"children": [2]},
            {"translation": [0, 1, 0]},
        ],
        "scene": 0,
        "scenes": [{"nodes": [0, 1]}],
    }
    js = json.dumps(data).encode()
    js += b" " * (-len(js) % 4)
    path.write_bytes(
        struct.pack(
            "<4sIIII", b"glTF", 2, 28 + len(js) + len(binary), len(js), 0x4E4F534A
        )
        + js
        + struct.pack("<I4s", len(binary), b"BIN\0")
        + binary
    )


class Stage(unittest.TestCase):
    def run_stage(self, root, *args):
        out = root / "output.glb"
        result = subprocess.run(
            [str(CLI), "--mesh", str(root / "rig.glb"), "--output", str(out), *args],
            check=False,
            capture_output=True,
            text=True,
        )
        return result, [json.loads(line) for line in result.stderr.splitlines()], out

    def test_default_preserves_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture(root / "rig.glb")
            proc, lines, out = self.run_stage(root)
            self.assertEqual(proc.returncode, 0, lines)
            self.assertEqual(out.read_bytes(), (root / "rig.glb").read_bytes())
            self.assertEqual(lines[-1]["pct"], 100)
            self.assertEqual(
                json.loads(Path(str(out) + ".manifest.json").read_text())["stiffness"],
                0,
            )

    def test_segments_sharpen_and_joint_centres_stay_soft(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture(root / "rig.glb")
            proc, lines, out = self.run_stage(root, "--stiffness", "1")
            self.assertEqual(proc.returncode, 0, lines)
            doc, data = document(out.read_bytes())
            original, original_data = document((root / "rig.glb").read_bytes())
            self.assertEqual(data[: len(original_data)], original_data)
            for field in ("nodes", "skins", "scenes"):
                self.assertEqual(doc[field], original[field])
            attrs = doc["meshes"][0]["primitives"][0]["attributes"]
            acc = doc["accessors"][attrs["WEIGHTS_0"]]
            view = doc["bufferViews"][acc["bufferView"]]
            rows = list(
                struct.iter_unpack(
                    "<4f",
                    data[view["byteOffset"] : view["byteOffset"] + view["byteLength"]],
                )
            )
            self.assertAlmostEqual(rows[0][0], 2 / 3, places=6)
            self.assertAlmostEqual(rows[2][0], 2 / 3, places=6)
            self.assertGreater(rows[1][0], 0.95)
            self.assertTrue(all(abs(sum(r) - 1) < 1e-6 for r in rows))

    def test_invalid_settings(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture(root / "rig.glb")
            for value in ("nan", "1.1", "-1"):
                proc, lines, out = self.run_stage(root, "--stiffness", value)
                self.assertNotEqual(proc.returncode, 0)
                self.assertIn("error", lines[-1])
                self.assertFalse(out.exists())

    def test_failure_preserves_existing_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture(root / "rig.glb")
            (root / "output.glb").write_bytes(b"existing")
            proc, lines, out = self.run_stage(root, "--stiffness", "0.5")
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("already exists", lines[-1]["error"])
            self.assertEqual(out.read_bytes(), b"existing")
            self.assertFalse(Path(str(out) + ".manifest.json").exists())

    def test_invalid_glb_never_publishes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "rig.glb").write_bytes(b"this is not a glTF document")
            proc, lines, out = self.run_stage(root)
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn("GLB", lines[-1]["error"])
            self.assertFalse(out.exists())
            self.assertFalse(Path(str(out) + ".manifest.json").exists())
