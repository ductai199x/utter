"""The speech models the accuracy bench compares, each in its official runtime.

Every model transcribes one clip at a time (batch 1), the way Utter will use it, with the
language left to the model to detect wherever it can: a dictation does not announce its
language. Candidates and why: docs/model-survey.md.
"""

from __future__ import annotations

import re
from abc import ABC, abstractmethod
from dataclasses import dataclass, field

import numpy as np

SAMPLE_RATE = 16_000


@dataclass
class Result:
    text: str
    language: str | None = None  # what the model detected, when it says


@dataclass
class AsrModel(ABC):
    name: str
    langs: tuple[str, ...]  # the languages of ours it covers
    auto_language: bool  # False: the bench tells it the language
    notes: dict[str, str] = field(default_factory=dict)

    @abstractmethod
    def load(self) -> None: ...

    @abstractmethod
    def transcribe(self, audio: np.ndarray, lang: str) -> Result: ...

    def sync(self) -> None:
        """Wait for queued GPU work, so a timing covers all of it."""
        import torch

        if torch.cuda.is_available():
            torch.cuda.synchronize()


def _cuda_providers() -> list[str]:
    return ["CUDAExecutionProvider", "CPUExecutionProvider"]


class OnnxAsr(AsrModel):
    """NeMo models exported to ONNX, run by onnx-asr on onnxruntime-gpu."""

    def __init__(self, name: str, langs: tuple[str, ...], model: str, repo: str | None = None):
        super().__init__(name, langs, auto_language=True)
        self._model_name, self._repo = model, repo

    def load(self) -> None:
        import onnx_asr
        import onnxruntime as ort

        ort.preload_dlls()
        path = None
        if self._repo:
            from huggingface_hub import snapshot_download

            path = snapshot_download(self._repo, allow_patterns=["config.json", "vocab.txt", "model.onnx*"])
        self._asr = onnx_asr.load_model(self._model_name, path, providers=_cuda_providers())
        self.notes["runtime"] = f"onnx-asr, onnxruntime {ort.__version__}, CUDA EP"

    def transcribe(self, audio: np.ndarray, lang: str) -> Result:
        return Result(self._asr.recognize(audio, sample_rate=SAMPLE_RATE))

    def sync(self) -> None:  # onnxruntime returns only after the GPU is done
        pass


class Nemotron(AsrModel):
    def __init__(self, lookahead_tokens: int | None = None):
        super().__init__("nemotron-3.5-streaming-0.6b", ("en", "vi"), auto_language=True)
        self._lookahead = lookahead_tokens

    def load(self) -> None:
        import torch
        from transformers import AutoModelForRNNT, AutoProcessor

        repo = "nvidia/nemotron-3.5-asr-streaming-0.6b"
        self._proc = AutoProcessor.from_pretrained(repo)
        if self._lookahead is not None:
            self._proc.set_num_lookahead_tokens(self._lookahead)
        self._model = AutoModelForRNNT.from_pretrained(repo, device_map="cuda", dtype=torch.bfloat16).eval()
        latency = getattr(self._proc, "streaming_latency_ms", None)
        self.notes["runtime"] = "transformers, bf16, offline decode of the whole clip"
        self.notes["streaming_latency_ms"] = str(latency)

    def transcribe(self, audio: np.ndarray, lang: str) -> Result:
        import torch

        inputs = self._proc(audio, sampling_rate=SAMPLE_RATE, language="auto")
        inputs.to(self._model.device, dtype=self._model.dtype)
        with torch.inference_mode():
            out = self._model.generate(**inputs, return_dict_in_generate=True)
        raw = self._proc.decode(out.sequences, skip_special_tokens=False)[0]
        tag = re.search(r"<([a-z]{2,3})-[A-Z]{2}>", raw)
        text = self._proc.decode(out.sequences, skip_special_tokens=True)[0]
        return Result(text.strip(), tag.group(1) if tag else None)


class Qwen3Asr(AsrModel):
    def __init__(self, size: str):
        super().__init__(f"qwen3-asr-{size}", ("en", "vi"), auto_language=True)
        self._repo = f"Qwen/Qwen3-ASR-{size.upper()}-hf"

    def load(self) -> None:
        import torch
        from transformers import AutoModelForMultimodalLM, AutoProcessor

        self._proc = AutoProcessor.from_pretrained(self._repo)
        self._model = AutoModelForMultimodalLM.from_pretrained(
            self._repo, device_map="cuda", dtype=torch.bfloat16
        ).eval()
        self.notes["runtime"] = "transformers, bf16, greedy"

    def transcribe(self, audio: np.ndarray, lang: str) -> Result:
        import torch

        inputs = self._proc.apply_transcription_request(audio=audio).to(self._model.device, self._model.dtype)
        with torch.inference_mode():
            ids = self._model.generate(**inputs, max_new_tokens=448, do_sample=False)
        parsed = self._proc.decode(ids[:, inputs["input_ids"].shape[1]:], return_format="parsed")[0]
        language = (parsed.get("language") or "").lower()
        code = {"english": "en", "vietnamese": "vi"}.get(language, language or None)
        return Result((parsed.get("transcription") or "").strip(), code)


class WhisperTurbo(AsrModel):
    def __init__(self) -> None:
        super().__init__("whisper-large-v3-turbo", ("en", "vi"), auto_language=True)

    def load(self) -> None:
        import torch
        from transformers import AutoModelForSpeechSeq2Seq, AutoProcessor

        repo = "openai/whisper-large-v3-turbo"
        self._proc = AutoProcessor.from_pretrained(repo)
        self._model = AutoModelForSpeechSeq2Seq.from_pretrained(
            repo, device_map="cuda", dtype=torch.bfloat16
        ).eval()
        self.notes["runtime"] = "transformers, bf16, greedy, task=transcribe, language detected"

    def transcribe(self, audio: np.ndarray, lang: str) -> Result:
        import torch

        feats = self._proc(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt").input_features
        feats = feats.to(self._model.device, self._model.dtype)
        with torch.inference_mode():
            # Detect the language once, then hand it to generate, which would otherwise detect it
            # again internally without returning it: the work timed is the same as a plain
            # generate, and the detected language is recorded.
            lang_id = int(self._model.detect_language(feats)[0])
            token = self._proc.tokenizer.convert_ids_to_tokens(lang_id)  # "<|en|>"
            code = token.strip("<|>")
            ids = self._model.generate(feats, task="transcribe", language=code, max_new_tokens=440, do_sample=False)
        text = self._proc.batch_decode(ids, skip_special_tokens=True)[0]
        return Result(text.strip(), code)


class CohereTranscribe(AsrModel):
    def __init__(self) -> None:
        # No language detection: the bench tells it the clip's language, which flatters it
        # next to the models that detect their own.
        super().__init__("cohere-transcribe", ("en", "vi"), auto_language=False)

    def load(self) -> None:
        import torch
        from transformers import AutoProcessor, CohereAsrForConditionalGeneration

        repo = "CohereLabs/cohere-transcribe-03-2026"
        self._proc = AutoProcessor.from_pretrained(repo)
        self._model = CohereAsrForConditionalGeneration.from_pretrained(
            repo, device_map="cuda", dtype=torch.bfloat16
        ).eval()
        self.notes["runtime"] = "transformers, bf16, language given"

    def transcribe(self, audio: np.ndarray, lang: str) -> Result:
        import torch

        inputs = self._proc(audio, sampling_rate=SAMPLE_RATE, return_tensors="pt", language=lang, punctuation=True)
        inputs.to(self._model.device, dtype=self._model.dtype)
        with torch.inference_mode():
            out = self._model.generate(**inputs, max_new_tokens=448)
        text = self._proc.decode(out, skip_special_tokens=True)
        return Result((text[0] if isinstance(text, list) else text).strip(), lang)


def registry() -> dict[str, AsrModel]:
    models: list[AsrModel] = [
        OnnxAsr("parakeet-tdt-0.6b-v3", ("en",), "nemo-parakeet-tdt-0.6b-v3"),
        # Trained for Vietnamese-English code-switching, so it is measured on English too.
        OnnxAsr("parakeet-ctc-0.6b-vi", ("vi", "en"), "nemo-conformer-ctc",
                repo="OpenVoiceOS/nvidia-parakeet-ctc-0.6b-vietnamese-onnx"),
        Nemotron(),
        Qwen3Asr("0.6b"),
        Qwen3Asr("1.7b"),
        WhisperTurbo(),
        CohereTranscribe(),
    ]
    return {m.name: m for m in models}
