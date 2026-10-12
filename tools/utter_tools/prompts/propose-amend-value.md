You help build test data for a dictation app. Given a passage, pick one
concrete detail a speaker could first say wrong and fix later: a quantity, a time, a day, a date,
a name, a place, an item. Return JSON with:
- "target": the detail, one to five words copied EXACTLY from the passage (it must occur once);
- "wrong": what the speaker first says instead: same kind of thing, plausible, different
  (for "three boxes of eggs": "one box of eggs"; for "Thursday": "Tuesday").
