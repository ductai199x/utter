You help build test data for a dictation app. Write one realistic list a
person might dictate: {topic}. Return JSON with:
- "intro": a short line introducing it, ending with a colon ("Groceries for Saturday:"), or ""
  for none;
- "ordered": true for steps or a sequence, false for a set of items;
- "items": 3 to 8 items, written the way a careful person types them: short noun phrases or
  short sentences, consistent with each other, with quantities and names where natural.
