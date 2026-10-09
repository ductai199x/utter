# Model survey

Checked live on 2026-10-09, from Hugging Face (model cards, the Hub API, the Open ASR
Leaderboard's raw results) and GitHub. Nothing here is from memory. Re-run the survey before a
model decision that is more than a few weeks old: this field moves monthly.

Utter's languages: **English and Vietnamese**, including switching between them mid-sentence
(Tai, 2026-10-09).

## Speech recognition

The Open ASR Leaderboard (English, version of 2026-09-10) ranks proprietary APIs first. The
best open models, with the languages Utter needs:

| Model | Released | Size | English avg WER | Speed (RTFx) | Vietnamese | Streaming | License |
|---|---|---|---|---|---|---|---|
| Qwen/Qwen3-ASR-1.7B | 2026-01 | 2.0B | 5.75 | 836 | yes (52 languages) | yes (vLLM only) | Apache-2.0 |
| Qwen/Qwen3-ASR-0.6B | 2026-01 | 0.9B | not listed | — | yes | yes (vLLM only) | Apache-2.0 |
| CohereLabs/cohere-transcribe-03-2026 | 2026-03 | 2.0B | 6.26 | 924 | yes (14 languages, no auto-detect, weak on code-switching) | no | Apache-2.0 |
| nvidia/nemotron-3.5-asr-streaming-0.6b | 2026-06 | 0.6B | not listed (FLEURS en 7.9–9.4) | — | yes (FLEURS vi 11.2–13.4) | **yes, cache-aware, 80 ms – 1.12 s chunks**; language auto-detect | OpenMDW-1.1 (commercial OK) |
| nvidia/parakeet-ctc-0.6b-Vietnamese | 2026-01 | 0.6B | — | — | **Vietnamese–English code-switching**, avg WER 9.3 on a blind set incl. GigaSpeech2 and ViMD | no | NVIDIA Open Model License |
| nvidia/parakeet-tdt-0.6b-v3 | 2025-08 | 0.6B | 6.38 | **13,305** | no (25 European languages) | no | CC-BY-4.0 |
| moondream/parakeet-ultra | 2026-09 | 0.6B | 5.80 (on its own card) | faster than v3 | no | no | CC-BY-4.0 |
| openai/whisper-large-v3-turbo | 2024-10 | 0.8B | about 6.1 | about 1,200 | yes | no | MIT |

What this means:
- **No single public number tells us Vietnamese accuracy on conversational speech.** The
  leaderboard has no Vietnamese track. Utter's own eval has to measure it.
- **Parakeet v3, the plan's earlier choice, cannot be the only model**: it has no Vietnamese.
  It stays a candidate for an English-only fast path.
- **Nemotron 3.5 streaming** matches the design best: it transcribes while you speak, detects
  the language, and covers both languages in one 0.6B model. Its English accuracy on the
  leaderboard sets is unknown.
- **Qwen3-ASR-1.7B** is the most accurate open model that covers both languages, but it is an
  autoregressive decoder, so it is likely slower per utterance. The bench decides.

Runtimes differ by model: ONNX (Parakeet, via onnx-asr), Transformers (Qwen3-ASR,
Cohere, Nemotron, Whisper), GGUF in C++ (NeMo-Speech.cpp for Nemotron, transcribe.cpp for
Cohere), vLLM (Linux only). The accuracy bench runs each model in its official runtime first.
Only the winners get a native runtime inside the Rust core.

Measured results and the choice made from them: `docs/eval-results.md`.

## Eval data (public, no recording needed)

English (`hf-audio/open-asr-leaderboard`, ungated, updated 2026-10-07; the leaderboard's own
test sets):
- AMI (meetings, spontaneous, disfluencies), Earnings-22 (accented, spontaneous), Voice Arena
  Monsoon (conversational, Indian accent), URGENT24 (noisy, reverberant), GigaSpeech
  (podcasts, YouTube), VoxPopuli, LibriSpeech (a clean baseline).

Vietnamese:
- `nguyendv02/ViMD_Dataset`: 63 provincial dialects, ungated.
- `google/fleurs` (vi_vn): read speech, ungated.
- `speechcolab/gigaspeech2` (Vietnamese): in-the-wild YouTube and podcasts. **Gated**: Tai must
  accept its terms on Hugging Face.
- `linhtran92/viet_bud500`: 500 h, podcasts and more. **Gated**, same.

Not covered by public data:
- **Vietnamese–English code-switching.** No public test set was found. It will need a small
  set made for it.
- **Dictation itself:** fillers, backtracks ("at 2, no, 3"), spoken punctuation. AMI has
  disfluencies, but nobody dictates in a meeting. The cleanup eval will use text pairs built
  for the purpose, checked by code.

## Cleanup and Command Mode LLM

Newest small and efficient models from the major labs (Hub API, 2026-10-09):

| Model | Released | Shape | Note |
|---|---|---|---|
| Qwen/Qwen3.5-4B, -9B, -2B | 2026-02 | dense | The newest small Qwen. Qwen3.8 starts at 27B. |
| google/gemma-4-E4B-it, -E2B-it | 2026-03 | dense, efficient | |
| google/gemma-4-12B-it | 2026-05 | dense | |
| google/gemma-4-26B-A4B-it | 2026-03 | MoE, 4B active | Large-model quality at small-model decode speed. About 16 GB at Q4. |
| Qwen/Qwen3.6-35B-A3B | 2026-04 | MoE, 3B active | About 21 GB at Q4: tight next to the ASR model on 24 GB. |
| openbmb/MiniCPM5-2B | 2026-09 | dense | |
| LiquidAI/LFM2.5-1.2B / 2.6B | 2026-08 | hybrid | |
| Qwen/Qwen3.8-27B | 2026-08 | dense | Quality reference. About 16 GB at Q4. |

All run in llama.cpp (latest build b11535, 2026-10-09, with Windows CUDA 13.4 binaries).
Vietnamese quality matters as much as English, which favours the Qwen and Gemma families.
