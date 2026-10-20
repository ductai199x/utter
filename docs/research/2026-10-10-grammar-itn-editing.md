# Grammar, written forms, voice edits: literature and data (2026-10-10)

> Reference research from 2026-10-10; what Utter does today is in [docs/cleanup.md](../cleanup.md).
> Not maintained.

What research and public data offer for the cleanup model's next skills. Condensed from a
research agent's report. Licenses were checked on primary pages where reachable. Anything marked
unverified was not checked.

## Grammar correction (GEC)

**Choosing the target.**
- *Minimal* edits fix only what is wrong (CoNLL-2014, BEA-2019). *Fluency* edits rewrite until
  the text reads native (JFLEG; Sakaguchi et al., TACL 2016, https://aclanthology.org/Q16-1013/).
- Fluency edits are where meaning drift appears.
- Utter's rule (docs/cleanup.md): fix everything ungrammatical, keep correct wording and
  register. That is minimal edits plus the fluency a native writer would need.

**Metrics.**
- ERRANT (https://github.com/chrisjbryant/errant) extracts and types edits: about 25 types ×
  missing/replacement/unnecessary, scored by F0.5.
- Its per-type precision is the over-correction alarm.
- GLEU is the fluency metric used with JFLEG.

**LLM behavior.**
- LLMs score well on fluency and poorly on minimal-edit sets: they over-edit and sometimes change
  meaning (https://arxiv.org/abs/2304.01746, https://arxiv.org/abs/2303.14342).
- Prompting for minimal edits helps (https://arxiv.org/abs/2401.07702).
- For training labels, sampling N outputs and keeping only edits that recur helps
  (https://arxiv.org/abs/2605.13624).

**Benchmarks and corpora.** None of these are for training.

| Corpus | What it is | License | How to use |
|---|---|---|---|
| BEA-2019 (W&I+LOCNESS), FCE, NUCLE/CoNLL-2014, Lang-8 | Learner writing | Non-commercial only | Eval only. The native LOCNESS half measures over-correction. |
| JFLEG (https://github.com/keisks/jfleg) | Fluency references | CC BY-NC-SA 4.0 | Eval only |
| cLang-8 | 2.4M pairs | CC BY-NC-SA, plus Lang-8 by request | Non-commercial |
| Speak & Improve 2025 (https://arxiv.org/abs/2412.11986) | Real non-native speech: about 315-340 h, of which about 55-60 h is transcribed with disfluency and grammar references | Non-commercial; signed agreement; no sharing; publishing models **or statistics** derived from it needs approval | Closest match to Utter's task. Local held-out eval only, after Tai signs. |

The S&I challenge protocol to copy for that eval
(https://arxiv.org/html/2412.11985v2):
- spoken GEC is scored by WER against fluent corrected references;
- feedback is scored by ERRANT F0.5, with a baseline of 0.22;
- verb and noun errors are the hardest; prepositions are the easiest.

**Data usable for training a commercial model:**
- C4_200M edits (CC BY 4.0);
- CoEdIT (Apache-2.0 per its card; its models are non-commercial);
- Disfl-QA (CC BY 4.0);
- NeMo text-processing grammars (Apache-2.0).

## Making realistic grammar errors synthetically

- **Tagged corruption** (Stahlberg & Kumar 2021, https://arxiv.org/abs/2105.13318; C4_200M):
  - a model takes a clean sentence and an ERRANT tag and writes a corrupted version;
  - tags are sampled to match a real error distribution.
- PIE (https://github.com/awasthiabhijeet/PIE, MIT) has rule ideas: verb forms, articles,
  prepositions.
- **LLM-generated errors.** They help (https://arxiv.org/abs/2403.05493), but ChatLang-8
  (https://arxiv.org/abs/2406.03202) warns they come out "simplistic and repetitive" unless the
  error types are controlled explicitly. Prompting in a learner's style works
  (https://aclanthology.org/2025.bea-1.72.pdf).
- **Realism recipe:**
  1. condition on the error type, the speaker's first language and proficiency level;
  2. match the type mix to real data;
  3. verify with ERRANT that the realized edits are the requested ones, and drop the rest.

**Taxonomy for spoken errors.** These are the ERRANT types a speaker can produce:
- determiners;
- prepositions;
- verb tense, agreement and form;
- the wrong verb;
- noun number;
- the wrong noun;
- pronouns;
- adjective forms;
- adverbs;
- word order, including in questions;
- morphology;
- particles;
- conjunctions;
- a missing verb or auxiliary.

Drop spelling, orthography and contraction errors (a recognizer cannot misspell), and most
punctuation errors (Parakeet supplies punctuation).

## Written forms of numbers and entities (inverse text normalization)

- **NeMo text processing** (https://github.com/NVIDIA/NeMo-text-processing, Apache-2.0;
  https://arxiv.org/abs/2104.05055):
  - What it is: grammars per class (cardinal, ordinal, decimal, money, measure, date, time,
    telephone, electronic, whitelist).
  - Its written-to-spoken direction can make the spoken forms of entities in training passages
    deterministically. That is safer than an LLM inventing them.
  - Its test cases are per-class unit evals.
- **Neural models hallucinate in this task** (Thutmose tagger, https://arxiv.org/abs/2208.00064),
  so digit exactness must be a hard check.
- **Google TN data** (https://github.com/rwsproat/text-normalization-data): license not stated,
  and the pairs are not reversible.
- **Earnings-21/22:** real speech dense with numbers (CC BY-SA 4.0 per the HF listing; verify).
  An entity eval on real speech.

## Lists, paragraphs, voice edits

- **No public paper or dataset turns dictated lists into bullets.** Build it synthetically from
  real markdown lists in permissively licensed text.
- **Paragraph segmentation** as insertion-only edits under constrained decoding
  (https://arxiv.org/abs/2512.24517).
- **TERTiUS** (Microsoft, ACL 2023, https://arxiv.org/abs/2307.04008):
  - 1320 real dictation sessions with free-form commands; more than 15 action types (replace,
    delete, insert, capitalize...);
  - telling commands from dictation reaches 90.9 F1; interpreting a command reaches 30-55% exact
    end state;
  - its online design never re-reads text before the last commit, so far-back edits are not
    measured;
  - the repository returns 404 today, so the authors would have to be asked.
- **No public dataset of long-reach spoken corrections** ("change the eggs to three boxes", said
  minutes later).

## Disfluency

- **DRES** (https://arxiv.org/abs/2509.20321):
  - segmenting the input helps;
  - reasoning models over-delete;
  - fine-tuning reaches near state of the art but generalizes worse;
  - Switchboard is paid LDC data.
- **Disfl-QA** (https://github.com/google-research-datasets/disfl-qa, CC BY 4.0): about 12k
  human-written disfluent questions ("Norse found no wait Normandy"), more than 90% corrections
  and restarts. Speak the test split with TTS for an eval; the train split may train.

## Real dictation data

- There is no public English dictation corpus with written targets.
- Medical sets (REFLACX) are credentialed and narrow.
- **Recording our own held-out set** is the only real test of lists, far-back edits, emails and
  non-native speech: Tai plus several other speakers.

## Measuring over-correction and faithfulness

- **No-op rate:** clean input must come back unchanged (the native LOCNESS half, or our own
  plain renditions).
- **ERRANT per-type precision.**
- **Reference-free metrics** (SOME, https://aclanthology.org/2020.coling-main.573/; IMPARA,
  https://aclanthology.org/2022.coling-1.316/). They can be gamed
  (https://arxiv.org/abs/2509.25961), so never use one as the only gate.
- **LLM judges** agree with humans on GEC (https://arxiv.org/abs/2403.17540) and on
  spoken-to-written faithfulness (CoS2W, https://arxiv.org/abs/2408.09688).
- **Hard checks:**
  - every number, name and entity of the gold appears in the output;
  - entailment in both directions with the intended passage;
  - a ceiling on the edit ratio.
