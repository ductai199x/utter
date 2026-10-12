"""Utter's offline tooling, never shipped. Four entry points (pyproject.toml):

- utter-data: grow the published datasets (passages, spoken versions, speech, transcripts, pairs)
  and publish them;
- utter-train: fine-tune a cleanup model on the text dataset and export the GGUF the app loads;
- utter-eval: grade cleanup models on the datasets' test splits, and speech models on public sets;
- utter-speech: rebuild the speech model Utter ships (ONNX export, quantization, checks).

The two datasets on the Hub (config.TEXT_REPO, config.SPEECH_REPO) are the standard every part
reads and writes: their schemas live in schema.py.
"""
