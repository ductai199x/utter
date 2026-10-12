"""Passages of real written text: what people actually typed, the gold of dictation.

Each passage is one paragraph sized like a dictation (8-90 words, 1-5 sentences), from openly
licensed corpora in several registers:

| domain    | source                                     | license       |
|-----------|--------------------------------------------|---------------|
| technical | Stack Exchange tech sites (Common Pile)    | CC BY-SA      |
| everyday  | Stack Exchange general sites (Common Pile) | CC BY-SA      |
| science   | arXiv abstracts (Common Pile)              | CC0           |
| chat      | Ubuntu IRC (Common Pile)                   | public domain |
| prose     | English Wikipedia                          | CC BY-SA      |

Filters keep only what a person would dictate: no code, LaTeX, URLs, markdown, quoted or indented
blocks; mostly letters; English; starts and ends like a sentence; well formed (the gold is what a
careful typist writes). Only licenses the datasets publish are kept (schema.PUBLISHED_LICENSES).
"""

from __future__ import annotations

import gzip
import json
import random
import re
from collections.abc import Iterator
from pathlib import Path

from utter_tools import schema
from utter_tools.data.batch import passage

# (domain, repo, file, kind of document)
SHARDS = [
    *[
        ("technical", "common-pile/stackexchange", f"{site}/documents/00000_se.jsonl.gz", "se")
        for site in (
            "superuser.com",
            "askubuntu.com",
            "unix.stackexchange.com",
            "serverfault.com",
            "softwareengineering.stackexchange.com",
            "datascience.stackexchange.com",
            "dba.stackexchange.com",
            "electronics.stackexchange.com",
        )
    ],
    *[
        ("everyday", "common-pile/stackexchange", f"{site}/documents/00000_se.jsonl.gz", "se")
        for site in (
            "workplace.stackexchange.com",
            "travel.stackexchange.com",
            "cooking.stackexchange.com",
            "money.stackexchange.com",
            "academia.stackexchange.com",
            "parenting.stackexchange.com",
        )
    ],
    ("science", "common-pile/arxiv_abstracts", "00000_arxiv-abstracts.jsonl.gz", "arxiv"),
    ("chat", "common-pile/ubuntu_irc", "v0/documents/00003_ubuntu.jsonl.gz", "irc"),
    ("chat", "common-pile/ubuntu_irc", "v0/documents/00006_ubuntu.jsonl.gz", "irc"),
    ("prose", "wikimedia/wikipedia", "20231101.en/train-00000-of-00041.parquet", "wiki"),
]

URL = re.compile(r"https?://|www\.|\.(com|org|net|io)/\S", re.IGNORECASE)
CODE = re.compile(
    r"`|\{|\}|=>|->|::|==|!=|&&|\|\||</?\w+>|\$\w|\\[a-zA-Z]|;\s*$|\(\)|\w\(\w*\)|\[\d+\]|^\s*[#>*\-+] "
)
LATEX = re.compile(r"\$|\\\w+")
# Pasted program output: shell prompts, bracketed version strings, installer progress, error codes.
_OUTPUT = re.compile(
    r"\w@[\w.-]+:|[~/][#$] |\[[^\]]*\d[^\]]*\]|\.\.\. ?Done|\bMsg \d+, Level\b|\bLine \d+\b"
)
# Enough candidates per shard to sample from; reading whole shards takes half an hour.
MAX_PER_SHARD = 40_000
_SENTENCE_END = re.compile(r"[.!?][\"')\]”’]?$")
_IRC_LINE = re.compile(r"^\[\d\d:\d\d\] <([^>]+)> (.+)$")


def dictatable(p: str) -> bool:
    words = p.split()
    if not 8 <= len(words) <= 90:
        return False
    if URL.search(p) or CODE.search(p) or LATEX.search(p) or _OUTPUT.search(p):
        return False
    if not (p[0].isupper() or p[0].isdigit() or p[0] in "\"“'"):
        return False
    if not _SENTENCE_END.search(p):
        return False
    letters = sum(c.isalpha() for c in p)
    visible = sum(not c.isspace() for c in p)
    if letters / max(1, visible) < 0.75:
        return False
    if sum(ord(c) > 0x2FF and c not in "“”‘’—–…×~°€£" for c in p) > 0:
        return False  # other scripts, emoji, odd symbols: not this eval
    sentences = len(re.findall(r"[.!?](?:\s|$)", p))
    return 1 <= sentences <= 5 and well_formed(p) and english(p)


# Common words of the other languages in the sources (chiefly Ubuntu IRC's German, Italian,
# Dutch, French and Finnish channels), minus words English shares ("die", "on", "was").
_ENGLISH_WORDS = frozenset({
    "the", "a", "an", "to", "of", "and", "is", "in", "it", "you", "that", "i", "for", "on",
    "with", "this", "be", "not", "are", "have", "but", "can", "my", "do", "was", "if", "or",
    "as", "at", "so", "what", "how", "your", "me", "we", "they", "he", "she", "there", "from",
    "by", "just", "no", "yes", "all", "will", "would", "one", "about",
})  # fmt: skip
_FOREIGN_WORDS = frozenset({
    # German
    "der", "das", "und", "ist", "nicht", "ich", "ein", "eine", "mit", "auf", "zu", "von",
    "sich", "auch", "noch", "wenn", "aber", "dann", "habe", "kann",
    # Italian
    "il", "che", "di", "la", "sono", "del", "della", "una", "questo", "ho", "ma", "cosa",
    "anche",
    # French
    "le", "les", "et", "est", "pas", "un", "une", "des", "du", "de", "sont", "pour", "avec",
    "dans", "sur",
    # Dutch and Afrikaans
    "het", "een", "van", "ik", "niet", "zijn", "ook", "maar", "wat", "ons", "nie",
    # Finnish
    "se", "ja", "ei", "ole", "mutta",
    # Slovenian and Croatian
    "je", "da", "ki", "pa", "sem",
})  # fmt: skip


def english(p: str) -> bool:
    """Not a passage in another language: its common words are English ones."""
    words = re.findall(r"[a-z']+", p.lower())
    en = sum(w in _ENGLISH_WORDS for w in words)
    foreign = sum(w in _FOREIGN_WORDS for w in words)
    return foreign < 2 or en > foreign


# The gold must be what a careful typist writes: Utter aims at well-formed text, so sloppy
# typing in the source ("Yes , I", "it.. ok", "done. then", "i think") would score a correct
# output as wrong.
_SLOPPY = re.compile(
    r" [,.?!;:](?!\.\.)"  # a space before punctuation (" ..." aside)
    r"|(?<!\.)\.\.(?!\.)|\.{4,}|[!?]{2,}"  # ".." or "....", "!!", "?!"
    r"|(?<!\.\.)[.!?] +[a-z]"  # a sentence starting in lowercase (not after an ellipsis)
    r"|(?:^|\s)i(?:\s|'|$)"  # lowercase "i"
    r"|[_^=]|\+-|\+/-"  # formulas
    r"|  "  # double spaces
)


_ABBREVIATIONS = re.compile(r"\b(?:e\.g|i\.e|etc|vs|approx|cf|al|incl|esp)\.", re.IGNORECASE)


def well_formed(p: str) -> bool:
    # Abbreviations end in a period that does not end a sentence ("e.g. the").
    return not _SLOPPY.search(_ABBREVIATIONS.sub("ABBR", p))


def paragraphs(text: str) -> Iterator[str]:
    """Blank-line separated paragraphs, with their own line breaks joined; paragraphs that
    contain indented or code-like lines are dropped whole."""
    for block in re.split(r"\n\s*\n", text):
        lines = block.split("\n")
        if any(line.startswith(("    ", "\t", ">", "|")) for line in lines):
            continue
        p = " ".join(line.strip() for line in lines).strip()
        p = re.sub(r"\s+", " ", p)
        if p:
            yield p


def _irc_messages(text: str) -> Iterator[str]:
    for line in text.split("\n"):
        m = _IRC_LINE.match(line.strip())
        if m and not m.group(1).startswith(("ubottu", "lubotu", "ubot")):
            msg = m.group(2)
            # Drop "nick: " addressing, which is a chat-client convention, not dictation.
            msg = re.sub(r"^[\w\-`|\[\]]+[:,] ", "", msg)
            yield msg


def documents(path: Path, kind: str) -> Iterator[tuple[str, str, str, str]]:
    """(doc id, text, url, licence) for each document in a shard."""
    if kind == "wiki":
        import pyarrow.parquet as pq

        table = pq.read_table(path, columns=["id", "url", "text"])
        for doc_id, url, text in zip(
            *(table[c].to_pylist() for c in ("id", "url", "text")), strict=True
        ):
            yield f"wiki:{doc_id}", text, url, "CC BY-SA 4.0"
        return
    with gzip.open(path, "rt", encoding="utf-8") as fh:
        for line in fh:
            d = json.loads(line)
            meta = d.get("metadata") or {}
            yield (
                d["id"],
                d["text"],
                meta.get("url", ""),
                meta.get("license", "") or "see repository",
            )


def candidates(path: Path, kind: str) -> Iterator[tuple[str, str, str, str]]:
    for doc_id, text, url, lic in documents(path, kind):
        if kind == "irc":
            pieces = _irc_messages(text)
        elif kind == "wiki":
            # One paragraph per line; headings are their own lines (and do not end like a
            # sentence, so the filter drops them).
            pieces = (line.strip() for line in text.split("\n"))
        elif kind == "se":
            # The first line is the question's title, followed by a single newline: joined to
            # the first paragraph it makes a run-on ("...RAID 5 array I have a...").
            pieces = paragraphs(text.split("\n", 1)[1] if "\n" in text else "")
        else:
            pieces = paragraphs(text)
        for p in pieces:
            p = " ".join(p.split())
            if dictatable(p):
                yield doc_id, p, url, lic


def collect(per_domain: int, seed: int) -> list[dict]:
    """`per_domain` passages from each domain, sampled with `seed` from up to MAX_PER_SHARD
    candidates per shard; documents under a license the datasets do not publish (license_id
    raises) or without a URL to credit are skipped."""
    from huggingface_hub import hf_hub_download

    rng = random.Random(seed)
    pools: dict[str, list[dict]] = {}
    seen: set[str] = set()
    for domain, repo, file, kind in SHARDS:
        path = Path(hf_hub_download(repo, file, repo_type="dataset"))
        n = 0
        for _doc_id, text, url, lic in candidates(path, kind):
            if text in seen:
                continue
            seen.add(text)
            try:
                row = passage(schema.new_passage_id(text), domain, text, repo, url, lic)
            except ValueError:
                continue
            if not url:
                continue
            pools.setdefault(domain, []).append(row)
            n += 1
            if n >= MAX_PER_SHARD:
                break
        print(f"{domain:9} {file}: {n} passages", flush=True)
    chosen: list[dict] = []
    for domain in sorted(pools):
        pool = pools[domain]
        rng.shuffle(pool)
        chosen += pool[:per_domain]
        print(f"{domain}: {min(per_domain, len(pool))} of {len(pool)}")
    return chosen
