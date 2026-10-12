---
pretty_name: Dictation to Text
license: cc-by-sa-4.0
language:
- en
task_categories:
- text-generation
task_ids:
- text2text-generation
- conversational
size_categories:
- 10K<n<100K
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
- dictation
- speech-to-text
- text-cleanup
- disfluency-removal
- self-correction
- grammar-correction
configs:
- config_name: default
  data_files:
  - split: train
    path: data/train-*.parquet
  - split: test
    path: data/test-*.parquet
dataset_info:
  features:
{features}
  splits:
  - name: train
    num_bytes: {train_bytes}
    num_examples: {train_rows}
  - name: test
    num_bytes: {test_bytes}
    num_examples: {test_rows}
---

# Dictation to Text

{total:,} pairs of **what a speech recognizer heard someone dictate** and **the text they meant to type**: punctuated,
formatted and corrected the way a careful writer would type it, in the speaker's own words. Fillers and false starts
go, spoken self-corrections and editing commands are applied, spoken lists become lists, grammar slips are fixed.

It is the training data of [Utter](https://github.com/ductai199x/utter)'s cleanup models
([ductai199x/utter](https://huggingface.co/ductai199x/utter)). Its speech counterpart, the audio these transcripts came
from, is [ductai199x/dictation-speech](https://huggingface.co/datasets/ductai199x/dictation-speech) (joined on `id` =
`rendition_id`).

## Dataset structure

| split | rows |
|---|---|
| train | {train_rows:,} |
| test | {test_rows:,} |

The split is by passage (a passage's renditions are all in one split) and balanced: test holds {balance_lo:.1f}-{balance_hi:.1f}%
of every category, domain, source, license, length and of the rows with a dictionary or with lists off. No pair appears
twice.

| column | type | meaning |
|---|---|---|
| `id` | string | `<passage id>-<category>` |
| `category` | string | how the passage was spoken (below) |
| `domain` | string | what the passage is: `chat`, `prose`, `qa`, `abstract`, `list`, `everyday-text` and so on |
| `dictated` | string | what the speech recognizer (Parakeet) heard: the model's input |
| `final_text` | string | the text the speaker meant to type: the model's target |
| `dictionary` | list of strings | the person's dictionary words given with the request (empty for most rows); some are the right spelling of a misheard word, some are near-misses that must be left alone |
| `lists` | bool | whether spoken lists are to be formatted as lists (`false`: the request said "Lists: off", keep prose) |
| `messages` | list of {{role, content}} | the exact chat the model was trained on: system prompt, user message (`Dictionary:` / `Lists: off` lines, then `Dictated: ...`), assistant reply |
| `source` | string | where the passage came from |
| `source_url` | string | the passage's page (for passages Utter's tools wrote, the script that wrote them) |
| `license` | string | the passage's license |

Categories:

{categories}

## Sources and licensing

{sources}

Passages from [Common Pile](https://huggingface.co/common-pile) (Stack Exchange, arXiv abstracts, Ubuntu IRC)
and [Wikipedia](https://huggingface.co/datasets/wikimedia/wikipedia), plus everyday messages, emails and notes written
for this dataset (`synthetic`). Every row carries its passage's license:

{licenses}

The dataset as a whole is CC BY-SA 4.0, as most of its text requires (share-alike; CC BY-SA 2.5 and 3.0 allow
adaptations under later versions). Rows under CC0 or in the public domain keep that status; text written for the
dataset is CC BY-SA 4.0. The passages were adapted (spoken renditions, corrections; see Dataset creation).
Attribution: each row's `source_url`, the page with the passage and its author.

## Dataset creation

1. **Passages**: real text of the kinds people dictate (questions and answers, chat, notes, prose), filtered to
   English and to lengths people say in one go; everyday messages written by a language model to cover what the
   public sources lack.
2. **Spoken renditions**: each passage rewritten the way someone would say it, in one of the categories above,
   by a language model (Gemma 4 26B).
3. **Speech**: each rendition spoken by Kokoro-82M (twelve English voices, speaking rate 0.9-1.1).
4. **Recognition**: Parakeet (the speech model Utter ships) transcribed the audio; its output, errors and habits
   included, is `dictated`.
5. **Dictionary lines** added to about a third of the rows.

### Final texts

- The passage itself, as its writer wrote it, with mechanical punctuation slips fixed (`?.`, `,.`, a space before a
  comma); grammar-category targets are their passage with its grammar corrected.
- With `lists` false, a spoken list's target is written as prose.

## Personal and sensitive information

Public posts (Stack Exchange, IRC) can contain usernames and the occasional personal detail their writers
published. Nothing was added; nothing about the people who use Utter is in this dataset.

## Bias, risks and limitations

- English only, written-style text; the spoken side is synthetic speech recognized by one model, so real speakers'
  accents and recognition errors are only partly represented.
- Technical forums are overrepresented next to everyday writing.
- Targets were produced with a language model's help and checked automatically, not by people, row by row.

## Use

```python
from datasets import load_dataset

ds = load_dataset("ductai199x/dictation-to-text")
ds["train"][0]["messages"]   # ready for chat fine-tuning
```

## Citation

```bibtex
@misc{{nguyen2026dictation,
  author       = {{Tai Duc Nguyen}},
  title        = {{Dictation to Text}},
  year         = {{2026}},
  publisher    = {{Hugging Face}},
  howpublished = {{\url{{https://huggingface.co/datasets/ductai199x/dictation-to-text}}}}
}}
```
