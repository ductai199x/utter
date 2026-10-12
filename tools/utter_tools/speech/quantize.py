"""Weight-only quantization of an exported Parakeet encoder: its MatMul weights stored in 4 or
8 bits (onnxruntime's MatMulNBits, dequantized on the GPU in the kernel), activations left in
the encoder's own precision. Everything else in the model folder is copied unchanged.

Why: the speech model's GPU footprint (DESIGN.md: about 5 GB for Utter in all, 2026-10-09).
Accuracy and latency are checked by the speech bench, never assumed.

Run: utter-speech quantize --src parakeet-ultra-fp16 --dst parakeet-ultra-fp16-w8 --bits 8
"""

from __future__ import annotations

import json
import shutil

import onnx
from onnxruntime.quantization.matmul_nbits_quantizer import (
    DefaultWeightOnlyQuantConfig,
    MatMulNBitsQuantizer,
)
from rich.console import Console

from utter_tools.config import data_dir

console = Console()


def main(src: str, dst: str, bits: int = 8, block_size: int = 32) -> None:
    """Quantizes the encoder of <data_dir>/models/<src> into <data_dir>/models/<dst>: `bits` (4 or
    8) per weight, one scale per `block_size` weights."""
    src_dir, dst_dir = data_dir() / "models" / src, data_dir() / "models" / dst
    if dst_dir.exists():
        shutil.rmtree(dst_dir)
    dst_dir.mkdir(parents=True)
    for f in src_dir.iterdir():
        if not f.name.startswith("encoder.onnx"):
            shutil.copy2(f, dst_dir / f.name)
    model = onnx.load(str(src_dir / "encoder.onnx"), load_external_data=True)
    config = DefaultWeightOnlyQuantConfig(
        block_size=block_size, is_symmetric=True, bits=bits, op_types_to_quantize=("MatMul",)
    )
    q = MatMulNBitsQuantizer(
        model, bits=bits, block_size=block_size, is_symmetric=True, algo_config=config
    )
    q.process()
    out = q.model.model if hasattr(q.model, "model") else q.model
    onnx.save_model(out, str(dst_dir / "encoder.onnx"))
    cfg_path = dst_dir / "config.json"
    cfg = json.loads(cfg_path.read_text(encoding="utf-8"))
    cfg["encoder_weights"] = (
        f"MatMul weights {bits}-bit (MatMulNBits, block {block_size}, symmetric)"
    )
    cfg_path.write_text(json.dumps(cfg, indent=2) + "\n", encoding="utf-8", newline="\n")
    size = (dst_dir / "encoder.onnx").stat().st_size / 1e6
    n = sum(1 for node in out.graph.node if node.op_type == "MatMulNBits")
    console.print(f"{dst}: {n} MatMulNBits nodes, encoder.onnx {size:.0f} MB")
