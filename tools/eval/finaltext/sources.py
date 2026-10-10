"""Passages of real written text: the gold results of the final-text eval.

Each passage is one paragraph someone actually typed, sized like a dictation (8-90 words, 1-5
sentences), drawn from openly licensed corpora in several registers:

| domain    | source                                                    | licence          |
|-----------|-----------------------------------------------------------|------------------|
| technical | Stack Exchange tech sites, GitHub issues (Common Pile)    | CC BY-SA / repos' own |
| everyday  | Stack Exchange general sites (Common Pile)                | CC BY-SA         |
| science   | arXiv abstracts (Common Pile)                             | CC0              |
| chat      | Ubuntu IRC (Common Pile)                                  | public domain    |
| prose     | English Wikipedia                                         | CC BY-SA         |

Filters keep only what a person would dictate: no code, LaTeX, URLs, markdown, quoted or
indented blocks; mostly letters; starts and ends like a sentence. Each source document is
assigned to the eval or the train split by a hash of its id, so the two never share a document.

Run: uv run python -m eval.finaltext.sources [--per-domain 200]
"""

from __future__ import annotations

import gzip
import hashlib
import json
import random
import re
from collections.abc import Iterator
from dataclasses import asdict, dataclass
from pathlib import Path

import typer

from eval.paths import eval_dir

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
    (
        "technical",
        "common-pile/github_archive",
        "gharchive/v0/documents/00010_github-archive-threads.jsonl.gz",
        "gh",
    ),
    (
        "technical",
        "common-pile/github_archive",
        "gharchive/v0/documents/00100_github-archive-threads.jsonl.gz",
        "gh",
    ),
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

EVAL_FRACTION = 0.2

_URL = re.compile(r"https?://|www\.|\.(com|org|net|io)/\S", re.IGNORECASE)
_CODE = re.compile(
    r"`|\{|\}|=>|->|::|==|!=|&&|\|\||</?\w+>|\$\w|\\[a-zA-Z]|;\s*$|\(\)|\w\(\w*\)|\[\d+\]|^\s*[#>*\-+] "
)
_LATEX = re.compile(r"\$|\\\w+")
# Pasted program output: shell prompts, bracketed version strings, installer progress, error codes.
_OUTPUT = re.compile(
    r"\w@[\w.-]+:|[~/][#$] |\[[^\]]*\d[^\]]*\]|\.\.\. ?Done|\bMsg \d+, Level\b|\bLine \d+\b"
)
# Enough candidates per shard to sample from; reading whole shards takes half an hour.
MAX_PER_SHARD = 40_000
_SENTENCE_END = re.compile(r"[.!?][\"')\]”’]?$")
_IRC_LINE = re.compile(r"^\[\d\d:\d\d\] <([^>]+)> (.+)$")


@dataclass
class Passage:
    id: str
    domain: str
    split: str
    text: str
    source: str
    url: str
    license: str


def split_of(doc_id: str) -> str:
    h = int(hashlib.sha256(doc_id.encode()).hexdigest()[:8], 16) / 0xFFFFFFFF
    return "eval" if h < EVAL_FRACTION else "train"


def dictatable(p: str) -> bool:
    words = p.split()
    if not 8 <= len(words) <= 90:
        return False
    if _URL.search(p) or _CODE.search(p) or _LATEX.search(p) or _OUTPUT.search(p):
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


def candidates(
    path: Path, kind: str, keep_titles: bool = False
) -> Iterator[tuple[str, str, str, str]]:
    for doc_id, text, url, lic in documents(path, kind):
        if kind == "irc":
            pieces = _irc_messages(text)
        elif kind == "wiki":
            # One paragraph per line; headings are their own lines (and do not end like a
            # sentence, so the filter drops them).
            pieces = (line.strip() for line in text.split("\n"))
        elif kind in ("se", "gh") and not keep_titles:
            # The first line is the question's or issue's title, followed by a single newline:
            # joined to the first paragraph it makes a run-on ("...RAID 5 array I have a...").
            pieces = paragraphs(text.split("\n", 1)[1] if "\n" in text else "")
        else:
            pieces = paragraphs(text)
        for p in pieces:
            p = " ".join(p.split())
            if dictatable(p):
                yield doc_id, p, url, lic


def main(
    per_domain: int = typer.Option(200, help="Passages per domain and split... per eval split."),
    train_per_domain: int = typer.Option(0, help="Train passages per domain (0: none yet)."),
    splits: list[str] = typer.Option(["eval", "train"], "--split", help="Which splits to write."),
    keep_titles: bool = typer.Option(
        False,
        help="Keep Stack Exchange and GitHub titles glued to the first paragraph, as eval v1 "
        "(2026-10-10) was built; reproduces that set exactly.",
    ),
    seed: int = 0,
) -> None:
    from huggingface_hub import hf_hub_download

    rng = random.Random(seed)
    pools: dict[tuple[str, str], list[Passage]] = {}
    seen: set[str] = set()
    for domain, repo, file, kind in SHARDS:
        path = Path(hf_hub_download(repo, file, repo_type="dataset"))
        n = 0
        for doc_id, text, url, lic in candidates(path, kind, keep_titles):
            if text in seen:
                continue
            seen.add(text)
            split = split_of(doc_id)
            pid = hashlib.sha256(text.encode()).hexdigest()[:12]
            pools.setdefault((domain, split), []).append(
                Passage(pid, domain, split, text, repo, url, lic)
            )
            n += 1
            if n >= MAX_PER_SHARD:
                break
        print(f"{domain:9} {file}: {n} passages", flush=True)

    out = eval_dir() / "finaltext"
    out.mkdir(parents=True, exist_ok=True)
    for split, want in (("eval", per_domain), ("train", train_per_domain)):
        if not want or split not in splits:
            continue
        chosen: list[Passage] = []
        for domain in sorted({d for d, _ in pools}):
            pool = pools.get((domain, split), [])
            rng.shuffle(pool)
            chosen += pool[:want]
            print(f"{split} {domain}: {min(want, len(pool))} of {len(pool)}")
        path = out / f"passages-{split}.jsonl"
        with path.open("w", encoding="utf-8") as fh:
            for p in chosen:
                fh.write(json.dumps(asdict(p), ensure_ascii=False) + "\n")
        print(f"wrote {len(chosen)} to {path}")


if __name__ == "__main__":
    typer.run(main)
