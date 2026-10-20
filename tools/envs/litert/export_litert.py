"""Exports Parakeet's encoder (with the joint's encoder projection) to one LiteRT model for the
phone's GPU (docs/android.md, "Speech encoder on the phone's GPU").

The GPU runs fixed shapes, so the model holds one signature per input length (`frames_<T>`, T
mel frames of 10 ms), all sharing the same weights. Each takes the clip's features padded to T,
`[1, T, 128]`, and one float mask per subsampling stage (1 for a valid frame, 0 for padding), and
returns the projected encoder frames `[1, T', 640]`; frames past the valid length are padding.
The stages and how each one's length follows from the last are written beside the model
(`<out>.json`), for the runtime to build the masks.

Three rewrites make the graph run on LiteRT's GPU, each exact in float32 (checked here against
Transformers on every signature):
- masks are float inputs, so the graph has no boolean or integer tensors, no length
  arithmetic and no masked fills (the stock graph put 48 of its 3,499 ops on the GPU);
- attention masking is an added bias, the convolution module's a multiply;
- every LayerNorm computes on x / 64 with eps / 64^2: its inputs reach |x| 7,320 and a mean
  square of 355,518 on real speech, past float16's 65,504, which broke the GPU's float16 math.

The weights are stored as float16 (half the file; the GPU computes in float16).

    utter-speech export-litert --out OUT.tflite [--frames 500,1000,2000,3000]
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

import numpy as np
import torch
import typer
from torch import nn

NEG = -1e4  # attention bias of a padded key: far below any score, still finite in float16
LAYER_NORM_SCALE = 64.0


def attention_forward(self, hidden_states, position_embeddings, attention_mask=None, **kwargs):
    """ParakeetEncoderAttention.forward with the mask as an additive bias [1, 1, 1, T']."""
    batch_size, seq_length = hidden_states.shape[:-1]
    hidden_shape = (batch_size, seq_length, -1, self.head_dim)
    q = self.q_proj(hidden_states).view(hidden_shape).transpose(1, 2)
    k = self.k_proj(hidden_states).view(hidden_shape).transpose(1, 2)
    v = self.v_proj(hidden_states).view(hidden_shape).transpose(1, 2)
    heads = self.config.num_attention_heads
    q_u = q + self.bias_u.view(1, heads, 1, self.head_dim)
    q_v = q + self.bias_v.view(1, heads, 1, self.head_dim)
    rel = self.relative_k_proj(position_embeddings).view(batch_size, -1, heads, self.head_dim)
    bd = self._rel_shift(q_v @ rel.permute(0, 2, 3, 1))[..., :seq_length] * self.scaling
    scores = (q_u @ k.transpose(2, 3)) * self.scaling + bd + attention_mask
    weights = torch.softmax(scores, dim=-1)
    out = (weights @ v).transpose(1, 2).reshape(batch_size, seq_length, -1)
    return self.o_proj(out), weights


def conv_forward(self, hidden_states, attention_mask=None):
    """ParakeetEncoderConvolutionModule.forward with padded frames zeroed by a multiply."""
    hidden_states = self.pointwise_conv1(hidden_states.transpose(1, 2))
    hidden_states = nn.functional.glu(hidden_states, dim=1)
    hidden_states = hidden_states * torch.exp(attention_mask[:, 0])  # bias 0 -> 1, NEG -> 0
    hidden_states = self.depthwise_conv(hidden_states)
    hidden_states = self.norm(hidden_states)
    hidden_states = self.activation(hidden_states)
    return self.pointwise_conv2(hidden_states).transpose(1, 2)


class ScaledLayerNorm(nn.Module):
    """The same LayerNorm computed on x / c with eps / c^2: identical in exact arithmetic."""

    def __init__(self, norm: nn.LayerNorm, c: float = LAYER_NORM_SCALE):
        super().__init__()
        self.norm, self.c = norm, c

    def forward(self, x):
        n = self.norm
        return nn.functional.layer_norm(
            x / self.c, n.normalized_shape, n.weight, n.bias, n.eps / self.c**2
        )


def scale_layer_norms(module: nn.Module) -> None:
    for name, child in module.named_children():
        if isinstance(child, nn.LayerNorm):
            setattr(module, name, ScaledLayerNorm(child))
        else:
            scale_layer_norms(child)


class Encoder(nn.Module):
    """Features [1, T, 128] and the stage masks -> projected encoder frames [1, T', 640]."""

    def __init__(self, model):
        super().__init__()
        self.encoder, self.projector = model.encoder, model.encoder_projector

    def forward(self, features, *masks):
        enc, sub = self.encoder, self.encoder.subsampling
        h = features.unsqueeze(1)
        stage = 0
        for layer in sub.layers:
            h = layer(h)
            if isinstance(layer, nn.Conv2d):
                h = h * masks[stage][:, None, :, None]
                stage += 1
        h = sub.linear(h.transpose(1, 2).reshape(1, h.shape[2], -1))
        h = h * enc.input_scale
        pos = enc.encode_positions(h)
        bias = ((1.0 - masks[-1]) * NEG)[:, None, None, :]
        for block in enc.layers:
            h = block(h, attention_mask=bias, position_embeddings=pos)
        return self.projector(h)


def stages(sub) -> list[dict]:
    """Each masked stage's convolution along time: its length is
    (length + 2 * padding - kernel) // stride + 1 of the stage before."""
    return [
        {"kernel": layer.kernel_size[0], "stride": layer.stride[0], "padding": layer.padding[0]}
        for layer in sub.layers
        if isinstance(layer, nn.Conv2d)
    ]


def stage_lengths(spec: list[dict], length: int) -> list[int]:
    out = []
    for s in spec:
        length = (length + 2 * s["padding"] - s["kernel"]) // s["stride"] + 1
        out.append(length)
    return out


def masks_for(spec: list[dict], frames: int, valid: int) -> list[torch.Tensor]:
    return [
        (torch.arange(n) < v).float()[None]
        for n, v in zip(stage_lengths(spec, frames), stage_lengths(spec, valid))
    ]


def to_float16_weights(src: Path, dst: Path) -> None:
    from ai_edge_quantizer import algorithm_manager, qtyping, quantizer

    qt = quantizer.Quantizer(str(src))
    qt.update_quantization_recipe(
        regex=".*",
        operation_name=qtyping.TFLOperationName.ALL_SUPPORTED,
        op_config=qtyping.OpQuantizationConfig(
            weight_tensor_config=qtyping.TensorQuantizationConfig(
                num_bits=16, dtype=qtyping.TensorDataType.FLOAT
            ),
            compute_precision=qtyping.ComputePrecision.FLOAT,
            explicit_dequantize=True,
        ),
        algorithm_key=algorithm_manager.AlgorithmName.FLOAT_CASTING,
    )
    qt.quantize().export_model(str(dst), overwrite=True)


def main(
    out: Path,
    frames: str = typer.Option("500,1000,2000,3000", help="Input lengths in mel frames of 10 ms."),
    weights: str = typer.Option("moondream/parakeet-ultra", help="ParakeetForTDT checkpoint."),
) -> None:
    import litert_torch
    from transformers import ParakeetForTDT
    from transformers.models.parakeet import modeling_parakeet as mp

    lengths = sorted(int(f) for f in frames.split(","))
    model = ParakeetForTDT.from_pretrained(weights, dtype=torch.float32).eval()
    spec = stages(model.encoder.subsampling)
    rng = np.random.default_rng(0)
    cases = {}
    with torch.no_grad():
        for t in lengths:
            valid = int(t * 0.6)
            feats = torch.from_numpy(rng.standard_normal((1, t, 128)).astype(np.float32))
            mask = (torch.arange(t) < valid).long()[None]
            ref = model.encoder_projector(
                model.encoder(input_features=feats, attention_mask=mask).last_hidden_state
            )
            cases[t] = (feats, masks_for(spec, t, valid), ref, stage_lengths(spec, valid)[-1])
    mp.ParakeetEncoderAttention.forward = attention_forward
    mp.ParakeetEncoderConvolutionModule.forward = conv_forward
    scale_layer_norms(model.encoder)
    encoder = Encoder(model).eval()

    converter = litert_torch
    for t, (feats, masks, _, _) in cases.items():
        converter = converter.signature(f"frames_{t}", encoder, (feats, *masks))
    edge = converter.convert()
    for t, (feats, masks, ref, keep) in cases.items():
        got = np.asarray(edge(feats, *masks, signature_name=f"frames_{t}"))
        err = float(np.abs(got[:, :keep] - ref[:, :keep].numpy()).max())
        print(f"frames_{t}: max difference from Transformers on the valid frames {err:.2e}")
        if err > 1e-2:
            raise SystemExit(f"frames_{t} does not match Transformers")

    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        full = Path(tmp) / "float32.tflite"
        edge.export(str(full))
        to_float16_weights(full, out)
    meta = {
        "weights": weights,
        "frames": lengths,
        "mels": 128,
        "hidden": int(cases[lengths[0]][2].shape[-1]),
        "stages": spec,
        "layer_norm_scale": LAYER_NORM_SCALE,
    }
    out.with_suffix(".json").write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
    print(f"written {out} ({out.stat().st_size / 1e9:.2f} GB) and {out.with_suffix('.json').name}")


if __name__ == "__main__":
    typer.run(main)
