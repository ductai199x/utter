"""utter-bench: speech models on public test sets, by word error rate, for choosing a recognizer
(English was settled on 2026-10-09; the candidates now are for Vietnamese and code-switching).

Each set is a fixed random sample of a public test set, as 16 kHz WAVs and a manifest the Rust
runtime reads too, so `utter-cli bench --name parakeet-rust` lands in the same table. Each model
runs in its official runtime, one clip at a time, the language left to it to detect.

    utter-bench fetch [SETS...]          the sets (300 clips each)
    utter-bench run [--model M ...]      the models on every set they cover, then the table
    utter-bench report                   the table of everything on disk
"""

from __future__ import annotations

import gc
import io
import math
import random
import re
import statistics
import tarfile
import time
import unicodedata
from abc import ABC, abstractmethod
from collections.abc import Iterator
from dataclasses import dataclass
from functools import cache
from pathlib import Path

import jiwer
import numpy as np
import pyarrow.parquet as pq
import soundfile as sf
import soxr
import typer
from huggingface_hub import HfFileSystem, hf_hub_download, snapshot_download

from utter_tools import common

RATE = 16_000
CLIP_SECONDS = (1.0, 30.0)  # dictation-sized: a sentence, one utterance
ROW_GROUPS = 8  # a sample spreads over this many row groups, so no one speaker dominates
WARMUP_CLIPS = 3  # first calls pay for kernel compilation and caches

app = typer.Typer(
    add_completion=False,
    no_args_is_help=True,
    help="Speech models on public test sets, by word error rate.",
)


# ---------------------------------------------------------------------------------------------
# The sets.


@dataclass(frozen=True)
class PublicSet:
    name: str
    lang: str
    repo: str
    files: str  # Parquet shards, or the tar.gz of a tar set
    text: str  # the transcript column (a tar set's is its TSV)
    id_column: str | None = None  # None: made from the shard and row
    tsv: str | None = None  # a tar set's transcripts


LEADERBOARD = "hf-audio/open-asr-leaderboard"
SETS = [
    # English: the Open ASR Leaderboard's test sets, weighted towards spontaneous speech.
    PublicSet("en-ami", "en", LEADERBOARD, "ami_cleaned/test-*.parquet", "text", "id"),
    PublicSet("en-earnings22", "en", LEADERBOARD, "earnings22/test-*.parquet", "text", "id"),
    PublicSet(
        "en-gigaspeech", "en", LEADERBOARD, "gigaspeech_cleaned/test-*.parquet", "text", "id"
    ),
    PublicSet(
        "en-voxpopuli", "en", LEADERBOARD, "voxpopuli_cleaned_aa/test-*.parquet", "text", "id"
    ),
    PublicSet("en-urgent24", "en", LEADERBOARD, "urgent2024/test-*.parquet", "text", "id"),
    PublicSet(
        "en-monsoon", "en", "VoiceArena/Monsoon_en_IN_test", "data/test-*.parquet", "text", "id"
    ),
    PublicSet(
        "en-libri-other", "en", LEADERBOARD, "librispeech/test.other-*.parquet", "text", "id"
    ),
    # Vietnamese: dialects, read speech, podcasts, in the wild (the last two gated).
    PublicSet(
        "vi-vimd", "vi", "nguyendv02/ViMD_Dataset", "data/test-*.parquet", "text", "filename"
    ),
    PublicSet(
        "vi-fleurs",
        "vi",
        "google/fleurs",
        "parquet-data/vi_vn/test-*.parquet",
        "raw_transcription",
        "id",
    ),
    PublicSet("vi-bud500", "vi", "linhtran92/viet_bud500", "data/test-*.parquet", "transcription"),
    PublicSet(
        "vi-gigaspeech2",
        "vi",
        "speechcolab/gigaspeech2",
        "data/vi/test.tar.gz",
        "",
        tsv="data/vi/test.tsv",
    ),
]


def _parquet_clips(s: PublicSet, want: int, rng: random.Random) -> Iterator[tuple[str, bytes, str]]:
    """Rows from a few random row groups, read over the Hub's file system."""
    fs = HfFileSystem()
    groups = []
    for shard in sorted(fs.glob(f"datasets/{s.repo}/{s.files}")):
        with fs.open(shard, "rb") as f:
            groups += [(shard, g) for g in range(pq.ParquetFile(f).metadata.num_row_groups)]
    rng.shuffle(groups)
    chosen = groups[:ROW_GROUPS]
    # A little more than each group's share: some rows fail the length filter.
    share = math.ceil(want * 1.5 / len(chosen))
    for shard, g in chosen:
        with fs.open(shard, "rb", block_size=16 << 20) as f:
            rows = pq.ParquetFile(f).read_row_group(g).to_pylist()
        rng.shuffle(rows)
        for i, row in enumerate(rows[:share]):
            text = (row.get(s.text) or "").strip()
            if row["audio"]["bytes"] and text:
                ident = str(row[s.id_column]) if s.id_column else f"{Path(shard).stem}-{g}-{i}"
                yield ident, row["audio"]["bytes"], text


def _tar_clips(s: PublicSet, want: int, rng: random.Random) -> Iterator[tuple[str, bytes, str]]:
    assert s.tsv
    lines = Path(hf_hub_download(s.repo, s.tsv, repo_type="dataset")).read_text(encoding="utf-8")
    texts = {
        i: t.strip()
        for i, _, t in (line.partition("\t") for line in lines.splitlines())
        if t.strip()
    }
    wanted = set(rng.sample(sorted(texts), min(len(texts), math.ceil(want * 1.5))))
    with tarfile.open(hf_hub_download(s.repo, s.files, repo_type="dataset"), "r:gz") as tar:
        for member in tar:
            ident = Path(member.name).stem
            f = tar.extractfile(member) if member.isfile() and ident in wanted else None
            if f is not None:
                yield ident, f.read(), texts[ident]


def fetch_set(s: PublicSet, want: int) -> None:
    out = common.eval_dir() / s.name
    if len(common.read_jsonl(out / "manifest.jsonl")) >= want:
        return
    out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(f"utter-eval-{s.name}")
    clips = _tar_clips(s, want, rng) if s.tsv else _parquet_clips(s, want, rng)
    rows = []
    for ident, encoded, text in clips:
        try:
            audio, rate = sf.read(io.BytesIO(encoded), dtype="float32", always_2d=True)
        except (sf.LibsndfileError, RuntimeError):  # one undecodable clip must not end the set
            continue
        audio = (
            soxr.resample(audio.mean(axis=1), rate, RATE, quality="VHQ")
            if rate != RATE
            else audio.mean(axis=1)
        )
        seconds = len(audio) / RATE
        if not CLIP_SECONDS[0] <= seconds <= CLIP_SECONDS[1]:
            continue
        wav = "".join(c if c.isalnum() or c in "-_." else "_" for c in ident) + ".wav"
        sf.write(out / wav, audio, RATE, subtype="PCM_16")
        rows.append(
            {
                "id": ident,
                "audio": wav,
                "text": text,
                "lang": s.lang,
                "duration_s": round(seconds, 3),
            }
        )
        if len(rows) >= want:
            break
    common.write_jsonl(out / "manifest.jsonl", rows)
    print(f"{s.name}: {len(rows)} clips, {sum(r['duration_s'] for r in rows) / 3600:.2f} h")


# ---------------------------------------------------------------------------------------------
# The models.


@dataclass
class Model(ABC):
    name: str
    langs: tuple[str, ...]
    repo: str

    @abstractmethod
    def load(self) -> None: ...

    @abstractmethod
    def transcribe(self, audio: np.ndarray, lang: str) -> tuple[str, str | None]:
        """The transcript and the language the model detected (None if it does not say)."""

    def sync(self) -> None:
        """Waits for queued GPU work, so a timing covers all of it."""
        import torch

        if torch.cuda.is_available():
            torch.cuda.synchronize()


class OnnxAsr(Model):
    """A NeMo model exported to ONNX, run by onnx-asr on onnxruntime-gpu."""

    def __init__(self, name: str, langs: tuple[str, ...], repo: str, kind: str) -> None:
        super().__init__(name, langs, repo)
        self._kind = kind  # onnx-asr's name for the architecture

    def load(self) -> None:
        import onnx_asr
        import onnxruntime as ort

        ort.preload_dlls()
        path = snapshot_download(
            self.repo, allow_patterns=["config.json", "vocab.txt", "model.onnx*"]
        )
        self._asr = onnx_asr.load_model(self._kind, path, providers=["CUDAExecutionProvider"])

    def transcribe(self, audio: np.ndarray, lang: str) -> tuple[str, str | None]:
        return self._asr.recognize(audio, sample_rate=RATE), None

    def sync(self) -> None:  # onnxruntime returns once the GPU is done
        pass


class Nemotron(Model):
    def load(self) -> None:
        import torch
        from transformers import AutoModelForRNNT, AutoProcessor

        self._proc = AutoProcessor.from_pretrained(self.repo)
        self._model = AutoModelForRNNT.from_pretrained(
            self.repo, device_map="cuda", dtype=torch.bfloat16
        ).eval()

    def transcribe(self, audio: np.ndarray, lang: str) -> tuple[str, str | None]:
        import torch

        inputs = self._proc(audio, sampling_rate=RATE, language="auto")
        inputs.to(self._model.device, dtype=self._model.dtype)
        with torch.inference_mode():
            out = self._model.generate(**inputs, return_dict_in_generate=True)
        tag = re.search(
            r"<([a-z]{2,3})-[A-Z]{2}>",
            self._proc.decode(out.sequences, skip_special_tokens=False)[0],
        )
        text = self._proc.decode(out.sequences, skip_special_tokens=True)[0]
        return text.strip(), tag.group(1) if tag else None


class Qwen3Asr(Model):
    def load(self) -> None:
        import torch
        from transformers import AutoModelForMultimodalLM, AutoProcessor

        self._proc = AutoProcessor.from_pretrained(self.repo)
        self._model = AutoModelForMultimodalLM.from_pretrained(
            self.repo, device_map="cuda", dtype=torch.bfloat16
        ).eval()

    def transcribe(self, audio: np.ndarray, lang: str) -> tuple[str, str | None]:
        import torch

        inputs = self._proc.apply_transcription_request(audio=audio).to(
            self._model.device, self._model.dtype
        )
        with torch.inference_mode():
            ids = self._model.generate(**inputs, max_new_tokens=448, do_sample=False)
        parsed = self._proc.decode(ids[:, inputs["input_ids"].shape[1] :], return_format="parsed")[
            0
        ]
        language = (parsed.get("language") or "").lower()
        code = {"english": "en", "vietnamese": "vi"}.get(language, language or None)
        return (parsed.get("transcription") or "").strip(), code


class WhisperTurbo(Model):
    def load(self) -> None:
        import torch
        from transformers import AutoModelForSpeechSeq2Seq, AutoProcessor

        self._proc = AutoProcessor.from_pretrained(self.repo)
        self._model = AutoModelForSpeechSeq2Seq.from_pretrained(
            self.repo, device_map="cuda", dtype=torch.bfloat16
        ).eval()

    def transcribe(self, audio: np.ndarray, lang: str) -> tuple[str, str | None]:
        import torch

        feats = self._proc(audio, sampling_rate=RATE, return_tensors="pt").input_features
        feats = feats.to(self._model.device, self._model.dtype)
        with torch.inference_mode():
            # Detected once and handed to generate, which would detect it again without saying.
            token = self._proc.tokenizer.convert_ids_to_tokens(
                int(self._model.detect_language(feats)[0])
            )
            code = token.strip("<|>")
            ids = self._model.generate(
                feats, task="transcribe", language=code, max_new_tokens=440, do_sample=False
            )
        return self._proc.batch_decode(ids, skip_special_tokens=True)[0].strip(), code


class CohereTranscribe(Model):
    """Detects no language: the bench gives it each clip's, which flatters it next to the
    models that detect their own."""

    def load(self) -> None:
        import torch
        from transformers import AutoProcessor, CohereAsrForConditionalGeneration

        self._proc = AutoProcessor.from_pretrained(self.repo)
        self._model = CohereAsrForConditionalGeneration.from_pretrained(
            self.repo, device_map="cuda", dtype=torch.bfloat16
        ).eval()

    def transcribe(self, audio: np.ndarray, lang: str) -> tuple[str, str | None]:
        import torch

        inputs = self._proc(
            audio, sampling_rate=RATE, return_tensors="pt", language=lang, punctuation=True
        )
        inputs.to(self._model.device, dtype=self._model.dtype)
        with torch.inference_mode():
            generate = self._model.generate  # pyright: ignore[reportAttributeAccessIssue]
            out = generate(**inputs, max_new_tokens=448)
        text = self._proc.decode(out, skip_special_tokens=True)
        return (text[0] if isinstance(text, list) else text).strip(), lang


def models() -> dict[str, Model]:
    both = ("en", "vi")
    candidates: list[Model] = [
        # Trained for Vietnamese-English code-switching, so it is measured on English too.
        OnnxAsr(
            "parakeet-ctc-0.6b-vi",
            both,
            "OpenVoiceOS/nvidia-parakeet-ctc-0.6b-vietnamese-onnx",
            "nemo-conformer-ctc",
        ),
        Nemotron("nemotron-3.5-streaming-0.6b", both, "nvidia/nemotron-3.5-asr-streaming-0.6b"),
        Qwen3Asr("qwen3-asr-0.6b", both, "Qwen/Qwen3-ASR-0.6B-hf"),
        Qwen3Asr("qwen3-asr-1.7b", both, "Qwen/Qwen3-ASR-1.7B-hf"),
        WhisperTurbo("whisper-large-v3-turbo", both, "openai/whisper-large-v3-turbo"),
        CohereTranscribe("cohere-transcribe", both, "CohereLabs/cohere-transcribe-03-2026"),
    ]
    return {m.name: m for m in candidates}


# ---------------------------------------------------------------------------------------------
# Running and scoring.


def results(model: str, set_name: str) -> Path:
    return common.eval_dir() / "results" / "asr" / model / f"{set_name}.jsonl"


def run_model(model: Model, set_name: str, limit: int) -> None:
    rows = common.read_jsonl(common.eval_dir() / set_name / "manifest.jsonl")
    rows = rows[:limit] if limit else rows
    out = results(model.name, set_name)
    if len(common.read_jsonl(out)) >= len(rows):
        return
    folder = common.eval_dir() / set_name
    clips = [(r, sf.read(folder / r["audio"], dtype="float32")[0]) for r in rows]
    for _, audio in clips[:WARMUP_CLIPS]:
        model.transcribe(audio, rows[0]["lang"])
    results_rows = []
    for row, audio in clips:
        model.sync()
        start = time.perf_counter()
        text, detected = model.transcribe(audio, row["lang"])
        model.sync()
        results_rows.append(
            {
                "id": row["id"],
                "lang": row["lang"],
                "ref": row["text"],
                "hyp": text,
                "detected": detected,
                "latency_s": round(time.perf_counter() - start, 4),
                "duration_s": row["duration_s"],
            }
        )
    common.write_jsonl(out, results_rows)


@cache
def _english_normalizer():
    from transformers import WhisperTokenizer

    return WhisperTokenizer.from_pretrained("openai/whisper-large-v3-turbo").normalize


def normalize(text: str, lang: str) -> str:
    """For word error rate: English by Whisper's normalizer (the leaderboard's); Vietnamese
    keeps its diacritics and drops case and punctuation, on which the references disagree."""
    if lang == "en":
        text = _english_normalizer()(text)
    else:
        text = re.sub(r"[^\w\s]", " ", unicodedata.normalize("NFC", text).lower())
    return " ".join(text.split())


def score(path: Path) -> dict:
    rows = common.read_jsonl(path)
    lang = rows[0]["lang"]
    pairs = [(normalize(r["ref"], lang), normalize(r["hyp"], lang)) for r in rows]
    pairs = [(ref, hyp) for ref, hyp in pairs if ref]
    latencies = [r["latency_s"] for r in rows]
    # A model that reports languages misses a clip it gave none for: Utter could not route it.
    reports = any(r["detected"] for r in rows)
    return {
        "wer": jiwer.wer([p[0] for p in pairs], [p[1] for p in pairs]),
        "lid_error": sum(r["detected"] != lang for r in rows) / len(rows) if reports else None,
        "p50_ms": 1000 * statistics.median(latencies),
    }


@app.command()
def fetch(names: list[str] = typer.Argument(None), per_set: int = 300) -> None:
    """Fetch the public sets (all, or those named)."""
    by_name = {s.name: s for s in SETS}
    for name in names or list(by_name):
        fetch_set(by_name[name], per_set)


@app.command()
def report() -> None:
    """Word error rate % of every model on every set, from what is on disk."""
    names = [s.name for s in SETS]
    scores: dict[str, dict[str, dict]] = {}
    for path in sorted((common.eval_dir() / "results" / "asr").glob("*/*.jsonl")):
        if path.stem in names:
            scores.setdefault(path.parent.name, {})[path.stem] = score(path)
    present = [n for n in names if any(n in s for s in scores.values())]
    print("| model | " + " | ".join(present) + " | language errors | p50 ms |")
    print("|---|" + "---|" * (len(present) + 2))
    for model, by_set in sorted(scores.items()):
        cells = [f"{100 * by_set[n]['wer']:.1f}" if n in by_set else "" for n in present]
        lid = [s["lid_error"] for s in by_set.values() if s["lid_error"] is not None]
        lid_cell = f"{100 * np.mean(lid):.1f}%" if lid else ""
        p50 = np.median([s["p50_ms"] for s in by_set.values()])
        print(f"| {model} | " + " | ".join(cells) + f" | {lid_cell} | {p50:.0f} |")


@app.command("run")
def run_cmd(
    model: list[str] = typer.Option([], help="Models to run (default: every candidate)."),
    limit: int = typer.Option(0, help="Clips per set (0: all)."),
) -> None:
    """Run models on every fetched set they cover, then report."""
    candidates = models()
    for name in model or list(candidates):
        m = candidates[name]
        sets = [
            s.name
            for s in SETS
            if s.lang in m.langs and (common.eval_dir() / s.name / "manifest.jsonl").exists()
        ]
        m.load()
        for set_name in sets:
            run_model(m, set_name, limit)
        del m
        gc.collect()
    report()


def cli() -> None:
    app()
