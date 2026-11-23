"""Run the pinned SkinTokens predictor for one mesh with a reproducible seed."""
import argparse
from functools import partial
from pathlib import Path
import random
import sys

import numpy as np
import torch

# The interpreter is SkinTokens' bpy environment and cwd is its pinned checkout.
sys.path.insert(0, str(Path.cwd()))
import demo
from src.data.dataset import DatasetConfig, RigDatasetModule
from src.rig_package.parser.bpy import transfer_rigging


def rig(mesh: str, output: str, seed: int) -> None:
    random.seed(seed)
    np.random.seed(seed % 2**32)
    torch.manual_seed(seed % 2**64)
    demo.load_model(demo.MODEL_CKPTS[0], None)
    model = demo.model.eval()
    # This encoder uses default_rng(None) unless its separate seed is supplied.
    encoder = model.vae.model
    encoder._encode = partial(encoder._encode, seed=seed % 2**64)

    # Use the engine's in-process Blender loader. Point sampling and final weight
    # transfer now share the seeded process, including when two Forge rigs overlap.
    config = DatasetConfig.parse(
        datapath={"loader": "bpy", "data_name": None, "filepaths": {"articulation": [mesh]}},
        batch_size=1, num_workers=0, pin_memory=False, persistent_workers=False,
    )
    dataset = RigDatasetModule(predict_dataset_config=config.split_by_cls(),
                               predict_transform=demo.transform, tokenizer=demo.tokenizer,
                               process_fn=model._process_fn)
    batch = next(iter(dataset.predict_dataloader()["articulation"]))
    batch = {k: v.to(model.device) if isinstance(v, torch.Tensor) else v for k, v in batch.items()}
    # The sampling parameters are the defaults of the pinned demo.py CLI.
    batch["generate_kwargs"] = dict(max_length=2048, top_k=5, top_p=.95, temperature=1.0,
                                    repetition_penalty=2.0, num_return_sequences=1, num_beams=10,
                                    do_sample=True)
    with torch.inference_mode():
        prediction = model.predict_step(batch, make_asset=True)["results"][0]
    transfer_rigging(source_asset=prediction.asset, target_path=mesh, export_path=output, group_per_vertex=4)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--seed", required=True, type=int)
    args = parser.parse_args()
    rig(args.input, args.output, args.seed)
