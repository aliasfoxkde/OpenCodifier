#!/usr/bin/env python3
"""Download SigLIP2 split-tower ONNX files to the models directory.

Arm of record: onnx-community/siglip2-base-patch16-256-ONNX
Needed files:
  - onnx/vision_model.onnx   (vision tower)
  - onnx/text_model.onnx      (text tower)
  - tokenizer.json             (text tokenizer)
  - preprocessor_config.json  (ImageNet mean/std)

Target: /nas/Temp/work/oc-model-eval/models/siglip2-base-patch16-256-ONNX/
"""
from __future__ import annotations

import os
import sys
from huggingface_hub import hf_hub_download, list_repo_files, snapshot_download

REPO_ID = "onnx-community/siglip2-base-patch16-256-ONNX"
TARGET_DIR = "/nas/Temp/work/oc-model-eval/models/siglip2-base-patch16-256-ONNX"

NEEDED = {
    "onnx/vision_model.onnx",
    "onnx/text_model.onnx",
    "tokenizer.json",
    "preprocessor_config.json",
}


def main() -> int:
    target = TARGET_DIR.rstrip("/")
    os.makedirs(target, exist_ok=True)

    all_files = set(list_repo_files(REPO_ID))
    missing = [f for f in NEEDED if f not in all_files]
    if missing:
        print(f"ERROR: files not in {REPO_ID}: {missing}", file=sys.stderr)
        return 1

    for rel_path in sorted(NEEDED):
        local_path = hf_hub_download(
            repo_id=REPO_ID,
            filename=rel_path,
            local_dir=target,
            local_dir_use_symlinks=False,
        )
        size_mb = os.path.getsize(local_path) / 1e6
        print(f"  {rel_path}: {size_mb:.1f} MB -> {local_path}")

    print(f"\nAll files downloaded to {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
