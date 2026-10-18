"""utter-speech: rebuild the speech model Utter ships, parakeet-ultra in Utter's own ONNX runtime.

utter-speech export      the checkpoint to the ONNX graphs the Rust core runs
utter-speech quantize    the encoder's weights in 8 bits, as the app runs it
utter-speech quantize-cpu    the encoder in int8 for computers without a graphics card
utter-speech check       the export against Transformers, token for token
utter-speech export-litert   the encoder for Android's GPU (LiteRT)
"""

from __future__ import annotations

from pathlib import Path

import typer

app = typer.Typer(
    add_completion=False, no_args_is_help=True, help="Rebuild the speech model Utter ships."
)


@app.command("export")
def export_cmd(
    weights: str = typer.Option("moondream/parakeet-ultra", help="ParakeetForTDT checkpoint."),
    name: str = typer.Option("parakeet-ultra", help="Output folder under <data dir>/models."),
    max_frames: int = typer.Option(512, help="Longest clip, in encoder frames of 80 ms."),
) -> None:
    """Export the checkpoint to ONNX (fp16 encoder, convolutions rewritten)."""
    from utter_tools.speech.export import export_model

    export_model(weights, name, max_frames)


@app.command("quantize")
def quantize_cmd(
    src: str = typer.Option(..., help="The export under <data dir>/models."),
    dst: str = typer.Option(..., help="The quantized copy's folder under <data dir>/models."),
    bits: int = typer.Option(8, help="Bits per weight."),
    block_size: int = typer.Option(32, help="Weights per quantization scale."),
) -> None:
    """Quantize the encoder's weights."""
    from utter_tools.speech.export import quantize

    quantize(src, dst, bits, block_size)


@app.command("quantize-cpu")
def quantize_cpu_cmd(
    src: str = typer.Option("parakeet-ultra", help="A float32 export under <data dir>/models."),
    dst: str = typer.Option(
        "parakeet-ultra-i8", help="The int8 copy's folder under <data dir>/models."
    ),
    per_channel: bool = typer.Option(True, help="One weight scale per output channel."),
) -> None:
    """Quantize the encoder for the processor: int8 weights, int8 activations at run time."""
    from utter_tools.speech.export import quantize_cpu

    quantize_cpu(src, dst, per_channel)


@app.command("export-litert")
def export_litert_cmd(
    out: Path = typer.Option(..., help="The .tflite to write (its .json is written beside it)."),
    frames: str = typer.Option(
        "500,1000,2000,3000", help="Input lengths in mel frames of 10 ms, one signature each."
    ),
    weights: str = typer.Option("moondream/parakeet-ultra", help="ParakeetForTDT checkpoint."),
) -> None:
    """Export the encoder to LiteRT for Android's GPU (envs/litert; in WSL on Windows)."""
    from utter_tools.speech.litert import export_litert

    export_litert(out, frames, weights)


@app.command("check")
def check_cmd(
    name: str = typer.Option("parakeet-ultra", help="The export under <data dir>/models."),
    weights: str = typer.Option("moondream/parakeet-ultra", help="The reference checkpoint."),
    clips: int = typer.Option(
        60, help="Clips, spread over the public English sets (utter-bench fetch)."
    ),
    fixture: Path = typer.Option(
        None, help="Also write the front-end fixture the Rust tests use here."
    ),
) -> None:
    """Check the export against Transformers."""
    from utter_tools.speech.check import check

    check(name, weights, clips, fixture)


def cli() -> None:
    app()
