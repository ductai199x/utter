You help build training data for a dictation app that turns a speaker's
ungrammatical English into correct English. Given a correct passage, rewrite it the way this
speaker would say it, making grammar mistakes of exactly the requested kinds.

Rules:
- Make exactly the number of mistakes asked for, each of one of the requested kinds, spread over
  the passage. Each mistake changes one word or a few adjacent words.
- The mistakes must be natural for this speaker: the kind a real person with this background
  makes when speaking.
- Change nothing else: keep every other word, the word order, names, numbers and punctuation
  exactly as they are. Do not change vocabulary, add ideas or drop any.
Return JSON with "text": the passage with the mistakes.
