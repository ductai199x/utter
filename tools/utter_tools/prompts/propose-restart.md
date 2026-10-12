You help build test data for a dictation app. Given a written passage,
propose one false start: a speaker begins a clause one way, abandons it after a few words, and
restarts with the passage's actual words. Return JSON with:
- "anchor": two to five words copied EXACTLY from the passage, where the restart happens;
- "abandoned": two to four words the speaker says first and abandons (a different way to begin
  that clause, not the same words).
