"""Check the exported Parakeet graphs against Transformers, the reference.

For clips from the English eval sets it compares:
- encoder frames: the ONNX encoder on the clip's valid feature frames, against Transformers'
  encoder on the padded, masked features (max abs difference);
- tokens: a greedy TDT loop over the ONNX decoder and joint, against Transformers' generate
  (exact sequence match);
- text: tokens decoded with the rule the Rust runtime uses, against Transformers' decode.

It also writes the front-end fixture the Rust tests compare against (`--fixture`): a
synthetic clip and Transformers' features for it.

Run: uv run python -m export.check_parakeet [--clips 60] [--fixture ../crates/utter-asr/tests/fixtures]
"""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import onnxruntime as ort
import soundfile as sf
import torch
import typer
from rich.console import Console

from eval.paths import data_dir, eval_dir

console = Console()
app = typer.Typer(add_completion=False)


def decode_tokens(ids: list[int], vocab: list[str]) -> str:
    """The rule utter-asr uses: join the pieces, '▁' starts a word."""
    return "".join(vocab[i] for i in ids).replace("▁", " ").strip()


def greedy_tdt(enc: np.ndarray, dec_sess, joint_sess, cfg: dict) -> list[int]:
    """Greedy TDT decoding, one frame at a time: the loop utter-asr implements."""
    blank, vocab_size, durations = cfg["blank_id"], cfg["vocab_size"], cfg["durations"]
    layers, hidden, max_symbols = (
        cfg["decoder_layers"],
        cfg["decoder_hidden"],
        cfg["max_symbols_per_step"],
    )
    h = np.zeros((layers, 1, hidden), np.float32)
    c = np.zeros((layers, 1, hidden), np.float32)
    dec, h, c = dec_sess.run(None, {"token": np.array([[blank]], np.int64), "h": h, "c": c})
    tokens: list[int] = []
    t, emitted, frames = 0, 0, enc.shape[0]
    while t < frames:
        logits = joint_sess.run(None, {"encoded": enc[t : t + 1], "decoded": dec})[0][0]
        token = int(logits[:vocab_size].argmax())
        step = durations[int(logits[vocab_size:].argmax())]
        if token != blank:
            tokens.append(token)
            emitted += 1
            dec, h, c = dec_sess.run(None, {"token": np.array([[token]], np.int64), "h": h, "c": c})
        if step > 0:
            t += step
            emitted = 0
        elif token == blank or emitted >= max_symbols:
            t += 1
            emitted = 0
    return tokens


def write_fixture(proc, folder: Path) -> None:
    """A deterministic 1.37 s clip (a chirp, a tone and quiet noise; odd length on purpose) and
    Transformers' features for it."""
    folder.mkdir(parents=True, exist_ok=True)
    sr, n = 16_000, 21_937
    t = np.arange(n) / sr
    rng = np.random.default_rng(7)
    audio = (
        0.3 * np.sin(2 * np.pi * (200 + 900 * t) * t)
        + 0.1 * np.sin(2 * np.pi * 1234 * t)
        + 0.01 * rng.standard_normal(n)
    ).astype(np.float32)
    feats = proc.feature_extractor(audio, sampling_rate=sr, return_tensors="np")
    valid = int(feats["attention_mask"][0].sum())
    np.asarray(audio, "<f4").tofile(folder / "frontend_audio.f32")
    np.asarray(feats["input_features"][0, :valid], "<f4").tofile(folder / "frontend_features.f32")
    (folder / "frontend.json").write_text(
        json.dumps({"samples": n, "frames": valid, "mels": feats["input_features"].shape[2]})
        + "\n",
        encoding="utf-8",
        newline="\n",
    )
    console.print(f"fixture: {n} samples -> {valid} frames, in {folder}")


@app.command()
def main(
    name: str = typer.Option("parakeet-ultra"),
    weights: str = typer.Option("moondream/parakeet-ultra"),
    clips: int = typer.Option(60, help="Clips to check, spread over the English sets."),
    fixture: Path | None = typer.Option(None, help="Also write the front-end fixture here."),
) -> None:
    from transformers import AutoProcessor, GenerationConfig, ParakeetForTDT

    folder = data_dir() / "models" / name
    cfg = json.loads((folder / "config.json").read_text(encoding="utf-8"))
    vocab = (folder / "vocab.txt").read_text(encoding="utf-8").split("\n")[: cfg["vocab_size"]]
    proc = AutoProcessor.from_pretrained(cfg["front_end"])
    if fixture:
        write_fixture(proc, fixture)

    model = ParakeetForTDT.from_pretrained(weights, dtype=torch.float32).cuda().eval()
    model.generation_config = GenerationConfig.from_pretrained(cfg["front_end"])
    ort.preload_dlls()
    providers = ["CUDAExecutionProvider", "CPUExecutionProvider"]
    enc_sess = ort.InferenceSession(str(folder / "encoder.onnx"), providers=providers)
    dec_sess = ort.InferenceSession(str(folder / "decoder.onnx"), providers=providers)
    joint_sess = ort.InferenceSession(str(folder / "joint.onnx"), providers=providers)

    sets = sorted(p.parent.name for p in eval_dir().glob("en-*/manifest.jsonl"))
    per_set = max(1, clips // len(sets))
    worst_enc, token_match, text_match, total = 0.0, 0, 0, 0
    for s in sets:
        rows = [
            json.loads(line) for line in (eval_dir() / s / "manifest.jsonl").open(encoding="utf-8")
        ][:per_set]
        for r in rows:
            audio = sf.read(eval_dir() / s / r["audio"], dtype="float32")[0]
            x = proc(audio, sampling_rate=16_000, return_tensors="pt").to("cuda")
            valid = int(x["attention_mask"][0].sum())
            with torch.inference_mode():
                ref = model.get_audio_features(
                    input_features=x["input_features"], attention_mask=x["attention_mask"]
                )
                out = model.generate(**x)
            seq = out.sequences if hasattr(out, "sequences") else out
            ref_ids = [int(i) for i in seq[0].tolist() if int(i) not in (cfg["blank_id"], 2)]
            ref_text = proc.batch_decode(seq, skip_special_tokens=True)[0].strip()

            feats = x["input_features"][:, :valid].cpu().numpy()
            enc = enc_sess.run(None, {"features": feats})[0][0]
            ref_enc = ref.pooler_output[0].cpu().numpy()
            ref_valid = (
                int(ref.attention_mask[0].sum())
                if ref.attention_mask is not None
                else ref_enc.shape[0]
            )
            if enc.shape[0] != ref_valid:
                console.print(
                    f"[red]{s}/{r['id']}: {enc.shape[0]} encoder frames, reference {ref_valid}[/red]"
                )
            n = min(enc.shape[0], ref_valid)
            worst_enc = max(worst_enc, float(np.abs(enc[:n] - ref_enc[:n]).max()))

            ids = greedy_tdt(enc, dec_sess, joint_sess, cfg)
            text = decode_tokens(ids, vocab)
            total += 1
            token_match += ids == ref_ids
            text_match += text == ref_text
            if ids != ref_ids or text != ref_text:
                console.print(f"[yellow]{s}/{r['id']}[/yellow]\n  ref : {ref_text}\n  onnx: {text}")
    console.print(
        f"{total} clips: tokens identical {token_match}, text identical {text_match}, "
        f"worst encoder difference {worst_enc:.2e}"
    )


if __name__ == "__main__":
    app()
