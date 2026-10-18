"""Export a Parakeet TDT checkpoint (Transformers' ParakeetForTDT) to the ONNX graphs Utter's
Rust runtime runs, plus the constants its audio front end needs.

Output folder (default `<data_dir>/models/<name>/`):
- `encoder.onnx`: mel features [1, T, 128] -> encoder frames already
  projected for the joint, [1, T', 640]. One clip at a time, no mask: the features handed in
  are exactly the clip's valid frames (samples // hop, as Transformers' ParakeetFeatureExtractor
  counts them for an even n_fft). Its convolutions are rewritten without Conv nodes by default
  (`conv_free.py` says why).
- `decoder.onnx`: one prediction-network step. token [1, 1] int64, h and c [2, 1, 640] ->
  decoder output [1, 640], h', c'.
- `joint.onnx`: encoder frames [N, 640] and one decoder output [1, 640] -> logits
  [N, vocab + durations]. N is free, so the runtime can score one frame or many in one call.
- `step.onnx`: the decoder step and the joint fused (`Step`): token, h, c, all encoder frames
  [T, 640] and a start frame -> argmax token and duration index for frames start..T, h', c'.
- `mel_filters.f32`: the Slaney mel filterbank [128, 257], little-endian float32.
- `vocab.txt`: one token per line, line number = token id (SentencePiece pieces, "▁" kept).
- `config.json`: the numbers the runtime needs (blank id, durations, front-end parameters).

quantize() then stores the encoder's MatMul weights in 8 bits (onnxruntime's MatMulNBits), as
the app runs it.
"""

from __future__ import annotations

import copy
import json
import shutil
from pathlib import Path

import numpy as np
import torch
from rich.console import Console
from torch import nn

from utter_tools.common import data_dir

FRONT_END_REPO = "nvidia/parakeet-tdt-0.6b-v3"  # processor and generation config (same family)
OPSET = 20

console = Console()


class Encoder(nn.Module):
    """The encoder and its projection. Inputs and outputs are always float32; with
    `dtype=float16` the weights and the computation are half precision, cast at both ends."""

    def __init__(self, model, dtype: torch.dtype = torch.float32):
        super().__init__()
        self.encoder, self.projector = model.encoder, model.encoder_projector
        self.dtype = dtype

    def forward(self, features: torch.Tensor) -> torch.Tensor:
        x = self.encoder(input_features=features.to(self.dtype)).last_hidden_state
        return self.projector(x).float()


class Decoder(nn.Module):
    def __init__(self, model):
        super().__init__()
        d = model.decoder
        self.embedding, self.lstm, self.projector = d.embedding, d.lstm, d.decoder_projector

    def forward(self, token: torch.Tensor, h: torch.Tensor, c: torch.Tensor):
        out, (h2, c2) = self.lstm(self.embedding(token), (h, c))
        return self.projector(out[:, 0, :]), h2, c2


class Joint(nn.Module):
    def __init__(self, model):
        super().__init__()
        self.joint = model.joint

    def forward(self, enc: torch.Tensor, dec: torch.Tensor) -> torch.Tensor:
        return self.joint.head(self.joint.activation(enc + dec))


class Step(nn.Module):
    """One decoder step fused with the joint over every frame from `start` on, reduced to
    the argmax token and duration per frame on the device. The greedy loop then walks the
    frames on the CPU and calls again only after it emits a token: one call per token
    instead of one per frame, and a few bytes back instead of the logits."""

    def __init__(self, model):
        super().__init__()
        self.decoder = Decoder(model)
        self.joint = model.joint
        self.vocab = model.config.vocab_size

    def forward(self, token, h, c, encoded, start):
        dec, h2, c2 = self.decoder(token, h, c)
        logits = self.joint.head(self.joint.activation(encoded[start[0] :] + dec))
        tokens = logits[:, : self.vocab].argmax(dim=-1)
        durations = logits[:, self.vocab :].argmax(dim=-1)
        return tokens, durations, h2, c2


def _to_onnx(
    module: nn.Module,
    args: tuple,
    path: Path,
    inputs: list[str],
    outputs: list[str],
    dynamic: dict,
) -> None:
    """Export with the TorchScript exporter, checked by onnx."""
    import onnx

    torch.onnx.export(
        module,
        args,
        str(path),
        input_names=inputs,
        output_names=outputs,
        dynamic_axes=dynamic,
        opset_version=OPSET,
        dynamo=False,
    )
    onnx.checker.check_model(str(path))


def export_model(
    weights: str = "moondream/parakeet-ultra",
    name: str = "parakeet-ultra",
    max_frames: int = 512,
) -> None:
    """Exports `weights` to <data_dir>/models/<name>/ as Utter ships it: the encoder's
    convolutions rewritten (conv_free.py; clips up to `max_frames` encoder frames of 80 ms) and
    its weights and math in fp16 (half the size; its inputs and outputs stay fp32)."""
    from transformers import (
        AutoProcessor,
        GenerationConfig,
        ParakeetEncoderConfig,
        ParakeetForTDT,
    )

    from utter_tools.speech.conv_free import rewrite

    out = data_dir() / "models" / name
    out.mkdir(parents=True, exist_ok=True)
    proc = AutoProcessor.from_pretrained(FRONT_END_REPO)
    model = ParakeetForTDT.from_pretrained(weights, dtype=torch.float32).eval()
    model.generation_config = GenerationConfig.from_pretrained(FRONT_END_REPO)
    rewrite(model, max_frames)
    cfg = model.config
    encoder_cfg = cfg.encoder_config
    assert isinstance(encoder_cfg, ParakeetEncoderConfig)
    hidden, layers = cfg.decoder_hidden_size, cfg.num_decoder_layers

    with torch.inference_mode():
        console.print("encoder (fp16)")
        # Traced on the GPU: PyTorch's CPU kernels do not all run in half precision. The rest of
        # the model is exported from the float32 model below.
        # Transformers' typing of the model's .half() and .cuda() is wrong (they run fine).
        half = copy.deepcopy(model).half().cuda()  # pyright: ignore[reportCallIssue]
        enc_module = Encoder(half, torch.float16)
        dummy = torch.randn(1, 300, encoder_cfg.num_mel_bins, device="cuda")
        _to_onnx(
            enc_module,
            (dummy,),
            out / "encoder.onnx",
            ["features"],
            ["encoded"],
            {"features": {1: "frames"}, "encoded": {1: "encoded_frames"}},
        )
        del half, enc_module
        torch.cuda.empty_cache()
        console.print("decoder")
        zeros = torch.zeros(layers, 1, hidden)
        _to_onnx(
            Decoder(model),
            (torch.tensor([[cfg.blank_token_id]]), zeros, zeros),
            out / "decoder.onnx",
            ["token", "h", "c"],
            ["decoded", "h_out", "c_out"],
            {},
        )
        console.print("joint")
        _to_onnx(
            Joint(model),
            (torch.randn(4, hidden), torch.randn(1, hidden)),
            out / "joint.onnx",
            ["encoded", "decoded"],
            ["logits"],
            {"encoded": {0: "n"}, "logits": {0: "n"}},
        )
        console.print("step")
        _to_onnx(
            Step(model),
            (
                torch.tensor([[cfg.blank_token_id]]),
                zeros,
                zeros,
                torch.randn(9, hidden),
                torch.tensor([2]),
            ),
            out / "step.onnx",
            ["token", "h", "c", "encoded", "start"],
            ["tokens", "durations", "h_out", "c_out"],
            {"encoded": {0: "frames"}, "tokens": {0: "n"}, "durations": {0: "n"}},
        )

    fe = proc.feature_extractor
    np.asarray(fe.mel_filters, dtype="<f4").tofile(out / "mel_filters.f32")
    tokens = proc.tokenizer.convert_ids_to_tokens(list(range(cfg.vocab_size)))
    (out / "vocab.txt").write_text(
        "\n".join(t if t is not None else "" for t in tokens) + "\n", encoding="utf-8", newline="\n"
    )
    (out / "config.json").write_text(
        json.dumps(
            {
                "weights": weights,
                "front_end": FRONT_END_REPO,
                "encoder_dtype": "fp16",
                "encoder_convolutions": "rewritten, depthwise fixed",
                "max_encoder_frames": max_frames,
                "subsampling_factor": encoder_cfg.subsampling_factor,
                "vocab_size": cfg.vocab_size,
                "blank_id": cfg.blank_token_id,
                "durations": list(cfg.durations),
                "max_symbols_per_step": cfg.max_symbols_per_step,
                "decoder_layers": layers,
                "decoder_hidden": hidden,
                "sample_rate": fe.sampling_rate,
                "n_fft": fe.n_fft,
                "win_length": fe.win_length,
                "hop_length": fe.hop_length,
                "n_mels": fe.feature_size,
                "preemphasis": fe.preemphasis,
                "log_guard": 2**-24,
                "norm_epsilon": 1e-5,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
        newline="\n",
    )
    console.print(f"Exported to {out}")


def quantize(src: str, dst: str, bits: int = 8, block_size: int = 32) -> None:
    """Quantizes the encoder of <data_dir>/models/<src> into <data_dir>/models/<dst>: `bits` per
    weight, one scale per `block_size` weights."""
    import onnx
    from onnxruntime.quantization.matmul_nbits_quantizer import (
        DefaultWeightOnlyQuantConfig,
        MatMulNBitsQuantizer,
    )
    from onnxruntime.quantization.onnx_model import ONNXModel

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
    assert isinstance(q.model, ONNXModel)
    out = q.model.model
    onnx.save_model(out, str(dst_dir / "encoder.onnx"))
    cfg_path = dst_dir / "config.json"
    cfg = json.loads(cfg_path.read_text(encoding="utf-8"))
    cfg["encoder_weights"] = (
        f"MatMul weights {bits}-bit (MatMulNBits, block {block_size}, symmetric)"
    )
    cfg_path.write_text(json.dumps(cfg, indent=2) + "\n", encoding="utf-8", newline="\n")
    size = (dst_dir / "encoder.onnx").stat().st_size / 1e6
    n = sum(1 for node in out.graph.node if node.op_type == "MatMulNBits")
    print(f"{dst}: {n} MatMulNBits nodes, encoder.onnx {size:.0f} MB")


def quantize_cpu(src: str, dst: str, per_channel: bool = True) -> None:
    """The encoder of <data_dir>/models/<src> (float32) for the processor, in <data_dir>/models/<dst>:
    int8 weights and int8 activations quantized at run time (onnxruntime's dynamic quantization:
    DynamicQuantizeLinear and MatMulInteger, which use VNNI where the processor has it).

    The GPU builds' 8-bit weights (quantize(): MatMulNBits over a float16 graph) run on the
    processor dequantized to float32, the float16 graph cast to and fro: 753 ms for a 9 s clip on
    a Ryzen 9 9900X (12 threads), against 237 ms for the float32 export and 142 ms for this one
    (2026-10-18). MatMulNBits with int8 compute (accuracy_level 4) returned nothing at all."""
    from onnxruntime.quantization import QuantType, quantize_dynamic

    src_dir, dst_dir = data_dir() / "models" / src, data_dir() / "models" / dst
    cfg = json.loads((src_dir / "config.json").read_text(encoding="utf-8"))
    if cfg.get("encoder_dtype", "fp32") != "fp32":
        raise SystemExit(f"{src} has a {cfg['encoder_dtype']} encoder: quantize a float32 export")
    if dst_dir.exists():
        shutil.rmtree(dst_dir)
    dst_dir.mkdir(parents=True)
    for f in src_dir.iterdir():
        if not f.name.startswith("encoder.onnx"):
            shutil.copy2(f, dst_dir / f.name)
    quantize_dynamic(
        str(src_dir / "encoder.onnx"),
        str(dst_dir / "encoder.onnx"),
        op_types_to_quantize=["MatMul"],
        per_channel=per_channel,
        weight_type=QuantType.QInt8,
    )
    cfg["encoder_weights"] = "int8 (dynamic: MatMulInteger, activations quantized at run time" + (
        ", per-channel weight scales)" if per_channel else ")"
    )
    (dst_dir / "config.json").write_text(
        json.dumps(cfg, indent=2) + "\n", encoding="utf-8", newline="\n"
    )
    size = (dst_dir / "encoder.onnx").stat().st_size / 1e6
    print(f"{dst}: encoder.onnx {size:.0f} MB")
