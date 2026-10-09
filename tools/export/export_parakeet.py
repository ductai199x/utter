"""Export a Parakeet TDT checkpoint (Transformers' ParakeetForTDT) to the ONNX graphs Utter's
Rust runtime runs, plus the constants its audio front end needs.

Output folder (default `<data_dir>/models/<name>/`):
- `encoder.onnx` (+ `encoder.onnx.data`): mel features [1, T, 128] -> encoder frames already
  projected for the joint, [1, T', 640]. One clip at a time, no mask: the features handed in
  are exactly the clip's valid frames (see `front_end_frames`). Its convolutions are
  rewritten without Conv nodes by default (`export/conv_free.py` says why).
- `decoder.onnx`: one prediction-network step. token [1, 1] int64, h and c [2, 1, 640] ->
  decoder output [1, 640], h', c'.
- `joint.onnx`: encoder frames [N, 640] and one decoder output [1, 640] -> logits
  [N, vocab + durations]. N is free, so the runtime can score one frame or many in one call.
- `step.onnx`: the decoder step and the joint fused (`Step`): token, h, c, all encoder frames
  [T, 640] and a start frame -> argmax token and duration index for frames start..T, h', c'.
- `mel_filters.f32`: the Slaney mel filterbank [128, 257], little-endian float32.
- `vocab.txt`: one token per line, line number = token id (SentencePiece pieces, "▁" kept).
- `config.json`: the numbers the runtime needs (blank id, durations, front-end parameters).

Run: uv run python -m export.export_parakeet [--weights moondream/parakeet-ultra] [--name parakeet-ultra]
"""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import torch
import typer
from rich.console import Console
from torch import nn

from eval.paths import data_dir

FRONT_END_REPO = "nvidia/parakeet-tdt-0.6b-v3"  # processor and generation config (same family)
OPSET = 20

console = Console()
app = typer.Typer(add_completion=False)


class Encoder(nn.Module):
    def __init__(self, model):
        super().__init__()
        self.encoder, self.projector = model.encoder, model.encoder_projector

    def forward(self, features: torch.Tensor) -> torch.Tensor:
        return self.projector(self.encoder(input_features=features).last_hidden_state)


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
        logits = self.joint.head(self.joint.activation(encoded[start[0]:] + dec))
        tokens = logits[:, : self.vocab].argmax(dim=-1)
        durations = logits[:, self.vocab:].argmax(dim=-1)
        return tokens, durations, h2, c2


def front_end_frames(num_samples: int, hop: int) -> int:
    """Valid feature frames for a clip, as Transformers' ParakeetFeatureExtractor counts them
    (n_fft // 2 * 2 - n_fft is 0 for an even n_fft)."""
    return num_samples // hop


def export(module: nn.Module, args: tuple, path: Path, inputs: list[str], outputs: list[str],
           dynamic: dict, large: bool = False) -> None:
    """Export with the TorchScript exporter. A model over protobuf's 2 GB limit comes out as
    one file per tensor; it is re-saved with every tensor in a single `<name>.data` beside it."""
    import onnx

    if not large:
        torch.onnx.export(module, args, str(path), input_names=inputs, output_names=outputs,
                          dynamic_axes=dynamic, opset_version=OPSET, dynamo=False)
        onnx.checker.check_model(str(path))
        return
    scratch = path.parent / f".{path.stem}-scratch"
    scratch.mkdir(exist_ok=True)
    torch.onnx.export(module, args, str(scratch / path.name), input_names=inputs, output_names=outputs,
                      dynamic_axes=dynamic, opset_version=OPSET, dynamo=False)
    proto = onnx.load(str(scratch / path.name), load_external_data=True)
    onnx.save_model(proto, str(path), save_as_external_data=True, all_tensors_to_one_file=True,
                    location=f"{path.name}.data", size_threshold=1024)
    for f in scratch.iterdir():
        f.unlink()
    scratch.rmdir()
    onnx.checker.check_model(str(path))


@app.command()
def main(
    weights: str = typer.Option("moondream/parakeet-ultra", help="ParakeetForTDT checkpoint."),
    name: str = typer.Option("parakeet-ultra", help="Output folder name under <data_dir>/models."),
    conv_free: bool = typer.Option(True, help="Rewrite the encoder's convolutions (export/conv_free.py)."),
    depthwise: str = typer.Option("fixed", help="Depthwise conv rewrite: fixed | taps."),
    max_frames: int = typer.Option(512, help="Longest clip, in encoder frames of 80 ms (depthwise=fixed)."),
) -> None:
    from transformers import AutoProcessor, GenerationConfig, ParakeetForTDT

    from export.conv_free import rewrite

    out = data_dir() / "models" / name
    out.mkdir(parents=True, exist_ok=True)
    proc = AutoProcessor.from_pretrained(FRONT_END_REPO)
    model = ParakeetForTDT.from_pretrained(weights, dtype=torch.float32).eval()
    model.generation_config = GenerationConfig.from_pretrained(FRONT_END_REPO)
    if conv_free:
        rewrite(model, depthwise=depthwise, max_frames=max_frames)
    cfg = model.config
    hidden, layers = cfg.decoder_hidden_size, cfg.num_decoder_layers

    with torch.inference_mode():
        console.print("encoder")
        export(Encoder(model), (torch.randn(1, 300, cfg.encoder_config.num_mel_bins),), out / "encoder.onnx",
               ["features"], ["encoded"], {"features": {1: "frames"}, "encoded": {1: "encoded_frames"}},
               large=True)
        console.print("decoder")
        zeros = torch.zeros(layers, 1, hidden)
        export(Decoder(model), (torch.tensor([[cfg.blank_token_id]]), zeros, zeros), out / "decoder.onnx",
               ["token", "h", "c"], ["decoded", "h_out", "c_out"], {})
        console.print("joint")
        export(Joint(model), (torch.randn(4, hidden), torch.randn(1, hidden)), out / "joint.onnx",
               ["encoded", "decoded"], ["logits"], {"encoded": {0: "n"}, "logits": {0: "n"}})
        console.print("step")
        export(Step(model), (torch.tensor([[cfg.blank_token_id]]), zeros, zeros, torch.randn(9, hidden),
                             torch.tensor([2])), out / "step.onnx",
               ["token", "h", "c", "encoded", "start"], ["tokens", "durations", "h_out", "c_out"],
               {"encoded": {0: "frames"}, "tokens": {0: "n"}, "durations": {0: "n"}})

    fe = proc.feature_extractor
    np.asarray(fe.mel_filters, dtype="<f4").tofile(out / "mel_filters.f32")
    tokens = proc.tokenizer.convert_ids_to_tokens(list(range(cfg.vocab_size)))
    (out / "vocab.txt").write_text("\n".join(t if t is not None else "" for t in tokens) + "\n",
                                   encoding="utf-8", newline="\n")
    (out / "config.json").write_text(json.dumps({
        "weights": weights,
        "front_end": FRONT_END_REPO,
        "encoder_convolutions": f"rewritten, depthwise {depthwise}" if conv_free else "conv",
        "max_encoder_frames": max_frames if conv_free and depthwise == "fixed" else None,
        "subsampling_factor": cfg.encoder_config.subsampling_factor,
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
    }, indent=2) + "\n", encoding="utf-8", newline="\n")
    console.print(f"Exported to {out}")


if __name__ == "__main__":
    app()
