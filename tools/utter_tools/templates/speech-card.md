---
pretty_name: Dictation Speech
license: cc-by-sa-4.0
language:
- en
task_categories:
- automatic-speech-recognition
- text-generation
size_categories:
- {size_category}
annotations_creators:
- machine-generated
language_creators:
- found
- machine-generated
multilinguality:
- monolingual
source_datasets:
{source_datasets}
tags:
- audio
- speech
- dictation
- synthetic-speech
- disfluency
- self-correction
configs:
{configs}
dataset_info:
  features:
{features}
  splits:
{splits}
---

# Dictation Speech

{total:,} clips ({hours:,.0f} hours) of **dictation**: people's kinds of text, spoken the way people dictate (fillers,
false starts, self-corrections, editing commands, spoken punctuation, lists, grammar slips), each with **what was said**
word for word and **the text the speaker meant to type**. 16 kHz mono, stored losslessly as FLAC. All speech is
synthetic (Kokoro-82M and Chatterbox).

It is the speech side of [Utter](https://github.com/ductai199x/utter)'s data:

- the `corpus*` collections are the audio behind
  [ductai199x/dictation-to-text](https://huggingface.co/datasets/ductai199x/dictation-to-text), one clip per text pair
  (join `rendition_id` to its `id`), each in its pair's split;
- the other collections are the eval sets Utter's models are measured on, all test, including voices (Chatterbox)
  no training clip uses.

## Dataset structure

| split | clips | hours |
|---|---|---|
{split_table}

Configs: `all` (the default: every collection, train and test) and one per collection:

{collections}

The same rendition can be in more than one test collection (spoken by both engines, or in a focused set and the main
one); `id` is unique, `rendition_id` is shared. No rendition and no passage is in both splits.

| column | type | meaning |
|---|---|---|
| `id` | string | `<collection>/<rendition id>`, unique |
| `rendition_id` | string | `<passage id>-<category>`; the `id` of the same row in dictation-to-text |
| `audio` | Audio (16 kHz) | the clip, FLAC |
| `duration_s` | float32 | its length |
| `language` | string | `en` |
| `transcript` | string | exactly what was said, as spoken (numbers and symbols in words) |
| `final_text` | string | the text the speaker meant to type |
| `category` | string | how it was spoken (below) |
| `domain` | string | what the passage is |
| `annotations` | list of strings | what the rendition contains, as `key=value` (e.g. `fillers=2`, `false_start=1`, `errors=VERB:SVA`, `op=value`) |
| `tts_engine` | string | {engines} |
| `tts_voice` | string | the voice (Kokoro's voice name, or a numbered donated reference voice for Chatterbox) |
| `tts_settings` | list of strings | `speed=` (Kokoro) or `exaggeration=` (Chatterbox); `audio=regenerated` on clips spoken again (below) |
| `collection` | string | the set it belongs to |
| `source` | string | where the passage came from |
| `source_url` | string | the passage's page (for passages Utter's tools wrote, the script that wrote them) |
| `license` | string | the passage's license |

Categories:

{categories}

## Sources and licensing

{sources}

{licenses}

The dataset is CC BY-SA 4.0, as most of its text requires; each row keeps its passage's license and source URL (the
page with the passage and its author).

The speech: Kokoro-82M is Apache-2.0, and its output is unrestricted; Chatterbox (MIT) cloned voices from
[kyutai/tts-voices](https://huggingface.co/kyutai/tts-voices) (`voice-donations`, CC0: recordings people donated
to the public domain).

## Dataset creation

1. Passages (Common Pile, Wikipedia, and everyday messages written for the dataset), rewritten as spoken renditions
   by a language model (Gemma 4 26B) in the categories above.
2. Spoken by Kokoro-82M (twelve English voices, rate 0.9-1.1, chosen per rendition by a hash of its id) or Chatterbox
   (voice-cloned from Kyutai's CC0 donated voices, `exaggeration` varied), resampled to 16 kHz, with 0.25 s of silence around.
3. {regenerated:,} corpus clips (`audio=regenerated`) are a second synthesis with the same text, voice and speed:
   the same content and length to within a few hundredths of a second, not the same samples as the audio
   dictation-to-text's transcripts were made from.
4. Final texts: the passage, with grammar corrected for the grammar category and mechanical punctuation slips
   fixed; the same as dictation-to-text's.

## Personal and sensitive information

All speech is synthetic; no recording of a real person is included. The Chatterbox voices imitate recordings people
donated to the public domain (kyutai/tts-voices); they are numbered here, not named, and must not be used to identify
or impersonate the donors. Passage text from public posts can contain the usernames their writers published.

## Bias, risks and limitations

- English only; synthetic voices (twelve Kokoro voices and a small set of donated ones), so real accents, rooms and
  microphones are not represented. Use it for dictation formatting and cleanup, and as an ASR stress test, not as a
  substitute for real speech.
- Technical forums are overrepresented next to everyday writing.

## Use

```python
from datasets import load_dataset

test = load_dataset("ductai199x/dictation-speech", split="test")             # every eval collection
amend = load_dataset("ductai199x/dictation-speech", "kokoro-amend", split="test")
row = test[0]; row["audio"]["array"], row["transcript"], row["final_text"]
```

## Citation

```bibtex
@misc{{nguyen2026dictationspeech,
  author       = {{Tai Duc Nguyen}},
  title        = {{Dictation Speech}},
  year         = {{2026}},
  publisher    = {{Hugging Face}},
  howpublished = {{\url{{https://huggingface.co/datasets/ductai199x/dictation-speech}}}}
}}
```
