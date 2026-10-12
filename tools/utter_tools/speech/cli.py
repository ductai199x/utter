"""utter-speech: rebuild the speech model Utter ships (parakeet-ultra in Utter's own ONNX runtime).

utter-speech export      the checkpoint to the ONNX graphs the Rust core runs
utter-speech quantize    the encoder's weights in 8 (or 4) bits, as the app runs it
utter-speech check       the export against Transformers, the reference
"""

from __future__ import annotations

from pathlib import Path

import typer

from utter_tools.speech import check, export, quantize

app = typer.Typer(
    add_completion=False, no_args_is_help=True, help="Rebuild the speech model Utter ships."
)


@app.command("export")
def export_cmd(
    weights: str = typer.Option("moondream/parakeet-ultra", help="ParakeetForTDT checkpoint."),
    name: str = typer.Option("parakeet-ultra", help="Output folder under <data dir>/models."),
    max_frames: int = typer.Option(512, help="Longest clip, in encoder frames of 80 ms."),
) -> None:
    """Export the checkpoint to ONNX."""
    export.main(weights, name, max_frames)


@app.command("quantize")
def quantize_cmd(
    src: str = typer.Option(..., help="Source model folder under <data dir>/models."),
    dst: str = typer.Option(..., help="Output model folder under <data dir>/models."),
    bits: int = typer.Option(8, help="4 or 8."),
    block_size: int = typer.Option(32, help="Weights per quantization scale."),
) -> None:
    """Quantize the encoder's weights."""
    quantize.main(src, dst, bits, block_size)


@app.command("check")
def check_cmd(
    name: str = typer.Option("parakeet-ultra", help="The export under <data dir>/models."),
    weights: str = typer.Option("moondream/parakeet-ultra", help="The reference checkpoint."),
    clips: int = typer.Option(60, help="Clips to check, spread over the public English sets."),
    fixture: Path = typer.Option(None, help="Also write the front-end fixture here."),
) -> None:
    """Check the export against Transformers."""
    check.main(name, weights, clips, fixture)
