"""Kimodo text-to-motion worker (runs in forge/motion/.venv). Writes a BVH and a JSON receipt.

Prints one JSON line per phase on stderr. The motion model runs on --device (cpu or mps); the
Llama 3 text encoder on --text-device. --stub-encoder replaces the text encoder with zeros
(unconditioned motion) so the rest of the pipeline can be tested without the gated weights.
"""

import argparse
import json
import os
import re
import resource
import sys
import time
import traceback
from pathlib import Path

import numpy as np
import torch

MODEL = "Kimodo-SOMA-RP-v1.2"
# SOMA77 names that forge/animlib/retarget.py would read as the wrong joint (LeftLeg is SOMA's
# thigh but a Mixamo shin; Chest is above Spine2). Renamed to Mixamo names, one token each.
RENAME = {"Spine1": "Spine", "Spine2": "Spine1", "Chest": "Spine2", "Neck1": "Neck",
          "LeftLeg": "LeftUpLeg", "LeftShin": "LeftLeg", "RightLeg": "RightUpLeg", "RightShin": "RightLeg"}


def log(**fields):
    print(json.dumps({"stage": "motion", **fields}), file=sys.stderr, flush=True)


class StubEncoder:
    def __call__(self, texts):
        texts = [texts] if isinstance(texts, str) else texts
        return torch.zeros(len(texts), 1, 4096), [1] * len(texts)

    def to(self, *args, **kwargs):
        return self

    def eval(self):
        return self


def peak_rss_gb():
    return round(resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024**3, 2)  # bytes on macOS


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--prompt", required=True)
    p.add_argument("--seconds", type=float, default=5.0)
    p.add_argument("--seed", type=int, default=0)
    p.add_argument("--steps", type=int, default=100)
    p.add_argument("--device", default="cpu")
    p.add_argument("--text-device", default="mps")
    p.add_argument("--stub-encoder", action="store_true")
    p.add_argument("--postprocess", action="store_true")
    p.add_argument("--bvh", type=Path, required=True)
    a = p.parse_args()
    if not 1 <= a.seconds <= 10:
        raise ValueError("--seconds must be between 1 and 10 (Kimodo's training range)")
    from kimodo import load_model
    from kimodo.exports.bvh import save_motion_bvh
    from kimodo.model import LLM2VecEncoder
    from kimodo.model.load_model import TEXT_ENCODER_PRESETS
    from kimodo.skeleton import SOMASkeleton30, global_rots_to_local_rots
    from kimodo.tools import seed_everything

    if a.device == "mps":
        # Some SOMA skeleton assets are float64 pickles, which MPS cannot hold (the change that
        # upstream PR #45 makes in kimodo/skeleton/base.py, applied here without patching Kimodo).
        def f32(fn):
            return lambda *x, **k: (lambda t: t.float() if torch.is_tensor(t) and t.dtype == torch.float64 else t)(fn(*x, **k))
        torch.load, torch.from_numpy = f32(torch.load), f32(torch.from_numpy)
    t0 = time.monotonic()
    log(pct=5, message="Loading the text encoder")
    if a.stub_encoder:
        encoder = StubEncoder()
    else:
        kw = dict(TEXT_ENCODER_PRESETS["llm2vec"]["kwargs"], device=a.text_device)
        encoder = LLM2VecEncoder(**kw)
    t1 = time.monotonic()
    log(pct=20, message="Loading the motion model")
    model = load_model(MODEL, device=a.device, text_encoder=encoder)
    t2 = time.monotonic()
    log(pct=30, message=f"Generating {a.seconds:g} s on {a.device}")
    seed_everything(a.seed)
    frames = int(round(a.seconds * model.fps))
    out = model(a.prompt, frames, num_denoising_steps=a.steps, post_processing=a.postprocess,
                return_numpy=True, progress_bar=lambda x: x)
    t3 = time.monotonic()
    skeleton = model.skeleton
    if isinstance(skeleton, SOMASkeleton30):
        skeleton = skeleton.somaskel77
    skeleton = skeleton.to("cpu")
    joints = torch.from_numpy(out["posed_joints"]).float()
    rots = torch.from_numpy(out["global_rot_mats"]).float()
    a.bvh.parent.mkdir(parents=True, exist_ok=True)
    save_motion_bvh(str(a.bvh), global_rots_to_local_rots(rots, skeleton), joints[:, skeleton.root_idx],
                    skeleton=skeleton, fps=model.fps, standard_tpose=True)
    text = a.bvh.read_text()
    text = re.sub(r"^(\s*(?:ROOT|JOINT) )(\w+)", lambda m: m[1] + RENAME.get(m[2], m[2]), text, flags=re.M)
    a.bvh.write_text(text)
    np.savez_compressed(a.bvh.with_suffix(".npz"), posed_joints=out["posed_joints"],
                        foot_contacts=out["foot_contacts"])
    receipt = {
        "model": MODEL, "prompt": a.prompt, "seed": a.seed, "steps": a.steps, "frames": frames,
        "fps": model.fps, "seconds": frames / model.fps, "device": a.device,
        "text_device": None if a.stub_encoder else a.text_device, "stub_encoder": a.stub_encoder,
        "postprocess": a.postprocess, "torch": torch.__version__,
        # largest horizontal excursion of the hips from the first frame (Kimodo is Y-up)
        "hips_travel_m": round(float(np.linalg.norm(out["posed_joints"][:, 0, [0, 2]] - out["posed_joints"][0, 0, [0, 2]], axis=1).max()), 3),
        "timings": {"encoder_load_sec": round(t1 - t0, 2), "model_load_sec": round(t2 - t1, 2),
                    "generate_sec": round(t3 - t2, 2), "total_sec": round(time.monotonic() - t0, 2)},
        "peak_rss_gb": peak_rss_gb(),
        "mps_driver_gb": round(torch.mps.driver_allocated_memory() / 1024**3, 2) if torch.backends.mps.is_available() else None,
    }
    a.bvh.with_suffix(".json").write_text(json.dumps(receipt, indent=2))
    log(pct=100, message="done", receipt=receipt)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print(json.dumps({"error": "Cancelled"}), file=sys.stderr)
        sys.exit(130)
    except Exception as exc:  # noqa: BLE001 - machine-readable failure boundary
        if os.environ.get("FORGE_MOTION_DEBUG"):
            traceback.print_exc()
        print(json.dumps({"error": f"{type(exc).__name__}: {exc}"}), file=sys.stderr)
        sys.exit(1)
