"""Writes each dataset's README.md (the dataset card: YAML metadata, then the documentation) from
the build statistics, per huggingface.co/docs/hub/datasets-cards and the card template.

Run (from tools): uv run python -m publish.card text OUT_DIR   (or: voice OUT_DIR)
"""

from __future__ import annotations

import collections
import glob
import json
from pathlib import Path

import pyarrow.parquet as pq
import typer

SOURCE_DATASETS = [
    "common-pile/stackexchange",
    "common-pile/arxiv_abstracts",
    "common-pile/ubuntu_irc",
    "wikimedia/wikipedia",
]
CATEGORIES = {
    "natural": "the passage said plainly",
    "disfluent": "with fillers, repeats and false starts",
    "correction": 'with a spoken self-correction ("three, no, four")',
    "command": 'with a spoken editing command ("scratch that")',
    "explicit": 'with spoken punctuation ("comma", "new paragraph")',
    "amend": "with a later edit to something said earlier",
    "list": "a list, spoken item by item",
    "grammar": "with the grammar slips of real speakers, to be corrected",
    "restate": "said again in other words partway through",
}


def _yaml_features(features: list[tuple[str, str]]) -> str:
    out = []
    for name, kind in features:
        if kind == "string-list":
            out.append(f"  - name: {name}\n    list: string")
        elif kind == "messages":
            out.append(
                f"  - name: {name}\n    list:\n    - name: role\n      dtype: string\n    - name: content\n      dtype: string"
            )
        elif kind == "audio":
            out.append(f"  - name: {name}\n    dtype:\n      audio:\n        sampling_rate: 16000")
        else:
            out.append(f"  - name: {name}\n    dtype: {kind}")
    return "\n".join(out)


def _table(rows: list[list], header: list[str]) -> str:
    lines = ["| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
    lines += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return "\n".join(lines)


def _counts(files: list[str], column: str) -> collections.Counter:
    c = collections.Counter()
    for f in files:
        c.update(pq.read_table(f, columns=[column]).column(column).to_pylist())
    return c


def summarize(out: Path) -> dict:
    """Per shard of a speech dataset folder: split, collection, rows, bytes, hours, and counts of
    license, source, category and engine. Written to summary-<name>.json where shards live, so a
    card can be made where only some of them are."""
    out_files = {}
    for f in sorted(glob.glob(str(out / "data" / "*" / "*.parquet"))):
        t = pq.read_table(f, columns=["duration_s", "license", "source", "category", "tts_engine"])
        out_files[f"{Path(f).parent.name}/{Path(f).name}"] = {
            "split": Path(f).name.split("-")[0],
            "collection": Path(f).parent.name,
            "rows": t.num_rows,
            "bytes": Path(f).stat().st_size,
            "hours": sum(t.column("duration_s").to_pylist()) / 3600,
            **{
                c: dict(collections.Counter(t.column(c).to_pylist()))
                for c in ("license", "source", "category", "tts_engine")
            },
        }
    return out_files


def summary(out: Path, name: str) -> None:
    (out / f"summary-{name}.json").write_text(
        json.dumps(summarize(out), indent=1), encoding="utf-8"
    )
    print(f"wrote {out / f'summary-{name}.json'}")


def text(out: Path) -> None:
    stats = json.loads((out / "stats.json").read_text(encoding="utf-8"))
    files = sorted(glob.glob(str(out / "data" / "*.parquet")))
    licenses = _counts(files, "license")
    sources = _counts(files, "source")
    categories = _counts(files, "category")
    shares = [
        v
        for aspect in stats["balance"].values()
        for k, v in aspect.items()
        if k != "_rows" and aspect["_rows"][k] >= 500
    ]
    balance_lo, balance_hi = 100 * min(shares), 100 * max(shares)
    nbytes = {
        s: sum(Path(f).stat().st_size for f in files if Path(f).name.startswith(s))
        for s in ("train", "test")
    }
    features = [
        ("id", "string"),
        ("category", "string"),
        ("domain", "string"),
        ("dictated", "string"),
        ("final_text", "string"),
        ("dictionary", "string-list"),
        ("lists", "bool"),
        ("messages", "messages"),
        ("source", "string"),
        ("source_url", "string"),
        ("license", "string"),
    ]
    total = stats["train"]["rows"] + stats["test"]["rows"]
    card = f"""---
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
{chr(10).join("- " + s for s in SOURCE_DATASETS)}
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
{_yaml_features(features)}
  splits:
  - name: train
    num_bytes: {nbytes["train"]}
    num_examples: {stats["train"]["rows"]}
  - name: test
    num_bytes: {nbytes["test"]}
    num_examples: {stats["test"]["rows"]}
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
| train | {stats["train"]["rows"]:,} |
| test | {stats["test"]["rows"]:,} |

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

{_table([[k, CATEGORIES.get(k, ""), f"{v:,}"] for k, v in categories.most_common()], ["category", "spoken", "rows"])}

## Sources and licensing

{_table([[k, f"{v:,}"] for k, v in sources.most_common()], ["source", "rows"])}

Passages from [Common Pile](https://huggingface.co/common-pile) (Stack Exchange, arXiv abstracts, Ubuntu IRC)
and [Wikipedia](https://huggingface.co/datasets/wikimedia/wikipedia), plus everyday messages, emails and notes written
for this dataset (`synthetic`). Every row carries its passage's license:

{_table([[k, f"{v:,}"] for k, v in licenses.most_common()], ["license", "rows"])}

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
  howpublished = {{\\url{{https://huggingface.co/datasets/ductai199x/dictation-to-text}}}}
}}
```
"""
    (out / "README.md").write_text(card, encoding="utf-8")
    print(f"wrote {out / 'README.md'}")


def voice(out: Path) -> None:
    stats: dict[str, dict] = {}
    for f in sorted(out.glob("stats-*.json")):
        stats.update(json.loads(f.read_text(encoding="utf-8")))
    shards: dict[str, dict] = {}
    for f in sorted(out.glob("summary-*.json")):
        shards.update(json.loads(f.read_text(encoding="utf-8")))
    shards.update(summarize(out))

    def total(column: str) -> collections.Counter:
        c = collections.Counter()
        for sh in shards.values():
            c.update(sh[column])
        return c

    licenses, sources, categories, engines = (
        total(c) for c in ("license", "source", "category", "tts_engine")
    )
    features = [
        ("id", "string"),
        ("rendition_id", "string"),
        ("audio", "audio"),
        ("duration_s", "float32"),
        ("language", "string"),
        ("transcript", "string"),
        ("final_text", "string"),
        ("category", "string"),
        ("domain", "string"),
        ("annotations", "string-list"),
        ("tts_engine", "string"),
        ("tts_voice", "string"),
        ("tts_settings", "string-list"),
        ("collection", "string"),
        ("source", "string"),
        ("source_url", "string"),
        ("license", "string"),
    ]
    # Per split and collection, from the shards themselves (the card cannot drift from the data).
    split_rows: collections.Counter = collections.Counter()
    split_bytes: collections.Counter = collections.Counter()
    split_hours: collections.Counter = collections.Counter()
    by_collection: dict[str, collections.Counter] = collections.defaultdict(collections.Counter)
    for sh in shards.values():
        split, c = sh["split"], sh["collection"]
        split_rows[split] += sh["rows"]
        split_bytes[split] += sh["bytes"]
        split_hours[split] += sh["hours"]
        by_collection[c][split] += sh["rows"]
        by_collection[c]["hours"] += sh["hours"]
    configs = ["- config_name: all\n  default: true\n  data_files:"]
    for split in ("train", "test"):
        if split_rows[split]:
            configs.append(f"  - split: {split}\n    path: data/*/{split}-*.parquet")
    for c in stats:
        configs.append(f"- config_name: {c}\n  data_files:")
        for split in ("train", "test"):
            if by_collection[c][split]:
                configs.append(f"  - split: {split}\n    path: data/{c}/{split}-*.parquet")
    splits_yaml = "\n".join(
        f"  - name: {s}\n    num_bytes: {split_bytes[s]}\n    num_examples: {split_rows[s]}"
        for s in ("train", "test")
        if split_rows[s]
    )
    total = sum(split_rows.values())
    hours = sum(split_hours.values())
    regenerated = sum(s.get("regenerated", 0) for s in stats.values())
    collection_rows = [
        [
            c,
            f"{by_collection[c]['train']:,}" if by_collection[c]["train"] else "",
            f"{by_collection[c]['test']:,}" if by_collection[c]["test"] else "",
            f"{by_collection[c]['hours']:.1f}",
            ", ".join(s["categories"]),
        ]
        for c, s in stats.items()
    ]
    card = f"""---
pretty_name: Dictation Speech
license: cc-by-sa-4.0
language:
- en
task_categories:
- automatic-speech-recognition
- text-generation
size_categories:
- {"100K<n<1M" if total >= 100_000 else "10K<n<100K"}
annotations_creators:
- machine-generated
language_creators:
- found
- machine-generated
multilinguality:
- monolingual
source_datasets:
{chr(10).join("- " + s for s in SOURCE_DATASETS)}
tags:
- audio
- speech
- dictation
- synthetic-speech
- disfluency
- self-correction
configs:
{chr(10).join(configs)}
dataset_info:
  features:
{_yaml_features(features)}
  splits:
{splits_yaml}
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
{chr(10).join(f"| {s} | {split_rows[s]:,} | {split_hours[s]:.0f} |" for s in ("train", "test") if split_rows[s])}

Configs: `all` (the default: every collection, train and test) and one per collection:

{_table(collection_rows, ["collection", "train", "test", "hours", "categories"])}

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
| `tts_engine` | string | {", ".join(f"`{e}` ({n:,})" for e, n in engines.most_common())} |
| `tts_voice` | string | the voice (Kokoro's voice name, or a numbered donated reference voice for Chatterbox) |
| `tts_settings` | list of strings | `speed=` (Kokoro) or `exaggeration=` (Chatterbox); `audio=regenerated` on clips spoken again (below) |
| `collection` | string | the set it belongs to |
| `source` | string | where the passage came from |
| `source_url` | string | the passage's page (for passages Utter's tools wrote, the script that wrote them) |
| `license` | string | the passage's license |

Categories:

{_table([[k, CATEGORIES.get(k, ""), f"{v:,}"] for k, v in categories.most_common()], ["category", "spoken", "clips"])}

## Sources and licensing

{_table([[k, f"{v:,}"] for k, v in sources.most_common()], ["source", "clips"])}

{_table([[k, f"{v:,}"] for k, v in licenses.most_common()], ["license", "clips"])}

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
  howpublished = {{\\url{{https://huggingface.co/datasets/ductai199x/dictation-speech}}}}
}}
```
"""
    (out / "README.md").write_text(card, encoding="utf-8")
    print(f"wrote {out / 'README.md'}")


app = typer.Typer()
app.command("text")(text)
app.command("voice")(voice)
app.command("summary")(summary)

if __name__ == "__main__":
    app()
