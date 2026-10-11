# Real test set: what to record (Tai)

The synthetic eval uses TTS voices; these recordings are the real test. A few are enough when
each covers several things. They are held out: never trained on.

**How:**
1. Dictate each one with Utter into Notepad, at your normal pace. Don't read it word for
   word: say it the way you would. Mistakes, restarts and "um"s are welcome; they are part of
   what is being tested.
2. After each, fix the text in Notepad until it is exactly what you meant. That is the gold.
3. Utter logs every dictation (audio and transcript) in `%LOCALAPPDATA%\Utter_dev\dictations\`.
   Save each fixed text as `<that dictation's id>.gold.txt` next to its log, or paste the texts
   to me and I'll match them.

| # | Situation | Say, in your own words | Covers |
|---|---|---|---|
| 1 | Grocery list for the weekend | eggs, milk, bread, spinach, coffee filters, a bag of rice. After a few items, decide you need three boxes of eggs, not one, and say so. Then drop one item you listed. | list, far-back edit, removal |
| 2 | Message to a coworker | the sync moves from Tuesday 3 PM to Thursday 4:30 PM in room 204. Get the day wrong once and correct yourself mid-sentence. Mention the budget is about $12,500. | numbers, times, money, self-correction |
| 3 | Slack update on the project | WaveRep with TF32 runs at about 240 videos a minute, four times faster (as an aside, in parentheses), version 2.1.3. | symbols, versions, parentheses, jargon |
| 4 | Telling a friend how to reset a router | four steps, in order. | numbered list from steps |
| 5 | Lunch plans | start a sentence, say "scratch that", say a different one. Later, start the whole thing over with "let's start over" and say it again. | retraction scope |
| 6 | Status update, about a minute | talk it through freely, thinking out loud; two or three topics. | fillers, paragraphs, long dictation, grammar |
| 7 | A prompt for ChatGPT | ask it a question and tell it to give three options. | typed, never answered |
| 8 | Contact details | your name spelled letter by letter, an email address, a phone number. | spelling, email, phone |
