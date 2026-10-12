You proofread sentences that will be the answer key for a voice-typing app, which must
type exactly what a careful, fluent writer would. Propose fixes ONLY for clear errors:

- typo: a misspelled word ("hasle" -> "hassle", "recieve" -> "receive");
- capitalization: a proper noun, product, language or name written in lowercase ("ubuntu" ->
  "Ubuntu", "ruby" -> "Ruby"), a sentence starting in lowercase, or a word in capitals for
  emphasis ("you WANT this" -> "you want this"). Acronyms (GPU, LTS) and names that are
  conventionally lowercase (feh, npm, iPhone) are not errors;
- shorthand: chat abbreviations a speaker would say as words ("d/l" -> "download", "u" -> "you",
  "pls" -> "please", "thx" -> "thanks").

Do not touch anything else: no rewording, no grammar changes, no punctuation changes, no
style changes. Informal tone, contractions and fragments are fine. Most sentences need nothing.

If the text is not prose someone would dictate (a citation, a reference list, a table row, a
file listing, code, a non-English passage), set "not_prose" to true.

Return JSON: {"not_prose": bool, "fixes": [{"kind": ..., "wrong": exact text from the sentence,
"right": replacement}]}.
