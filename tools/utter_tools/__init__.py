"""Utter's offline tooling, never shipped: five commands, one per job (pyproject.toml).

- utter-grade: how often each cleanup model types what the speaker meant (ready to send);
- utter-train: fine-tune a cleanup model on dictation-to-text and export its GGUF;
- utter-data: grow the two datasets, a batch at a time, and publish them;
- utter-bench: speech models on public test sets, by word error rate;
- utter-speech: rebuild the speech model Utter ships.

The two datasets on the Hub are the standard every command reads and writes (schema.py).
"""
