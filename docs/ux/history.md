# History

Every dictation, kept on this computer, findable and copyable: the page the Home window opens
on. Status: **spec**.

UniClipboard was studied for its UX, not copied (Tai, 2026-10-11): keyboard-first browsing,
search that answers as you type with a live count, a list beside the selected item in full,
one-key actions with their keys shown, filters that combine. The look is Utter's own: its
accent and type, and the mark's idea of a voice laying itself down into lines of text.

## What it is for

1. Get any dictation back: one that did not go in, or one to use again elsewhere.
2. See what Utter heard beside what it typed, and what cleanup changed: the thing people are
   curious about in a dictation app, and the way to learn what to add to the Dictionary.
3. Nothing leaves the machine, and the person decides how long anything is kept.

## The Home window, reframed

Three sections on a narrow **rail** at the left edge, 56 px wide: History (first, and where
the window opens), Dictionary, and Settings at the bottom. Each is a 40 px icon button with a
tooltip; the selected one has a soft accent fill. The animated mark sits at the top of the rail
(it draws itself in when the window opens, then rests).

- Window: 1040 x 680 by default, at least 760 x 480. It opens on the section it was last on;
  the first time, on History.
- Dictionary and Settings keep their pages (docs/ux/home.md) right of the rail.

## The History section

Two columns right of the rail: the **list** (380 px; narrower windows shrink the entry first)
and the **entry** (the rest).

### Search and filters

At the top of the list:
- **Search** across the list's width, "Search dictations", with the count at its right
  ("1,208", or "12 of 1,208" while filtering). It searches the typed text and what was heard,
  ignoring case and accents, as the person types. Ctrl+F or / focuses it; Esc clears it.
  Matches are highlighted in the list.
- Under it, one row of compact **filter chips**: a star (favorites only), **Status** (All,
  Inserted, Not inserted, with a count dot when any did not go in), **App** (the apps dictated
  into, most used first, with counts), **Time** (All time, Today, Yesterday, Last 7 days,
  Last 30 days). A chip with a choice shows it ("Slack", "Today") in the accent and has a
  small x to clear it. Filters combine.

### The list

Newest first, under sticky day headers ("Today", "Yesterday", "Monday", "Oct 3"):
- Each dictation is a row that reads like a line of writing: the text, up to 2 lines, then an
  ellipsis; under it, 12 px muted, the app's name and the time ("3:14 PM"), and "· 42 words".
- A short accent rule at the row's left edge, like the mark's lines, marks the selected row;
  an amber one marks a dictation that did not go in. Favorites show a small star at the end of
  the meta line.
- ↑ and ↓ move the selection, the list scrolling to keep it in view; the first row is selected
  when the section opens. Long histories load as they scroll.

### The entry

The selected dictation in full:
- A header, muted: the app, the full date and time, "42 words · 4.2 s of speech · ready in
  190 ms".
- **Not inserted**, when it was not, as an amber line with the reason: "The focus moved to
  another window." / "No window had the focus." / "The app runs as administrator."
- **The text**, 17/28, selectable, scrolling if long.
- **What Utter heard**, under it, 14/22: the speech model's words with cleanup's changes marked
  in place. Removed words (fillers, a retracted phrase) are struck through and muted; words
  cleanup changed or added are underlined in the accent. A toggle shows the plain transcript
  instead. Shown only when it differs from the text.
- Actions along the bottom, each a text button with its key in a small kbd: **Copy** `C`,
  **Favorite** `F` (Unfavorite when it is one), **Delete** `D`; and **Copy what was heard**.
- Copy puts the text on the clipboard and leaves it there (this is the person copying); the
  button says "Copied" for 1.2 s. Delete removes it at once and shows "Deleted. Undo" for 6 s;
  the next row is selected.

### Empty states

- No dictations yet: the mark, drawn large and faint, and "Nothing here yet. Hold Right Ctrl
  and speak in any app: your dictations show up here."
- History off: "History is off. Turn it on in Settings to keep your dictations here."
- No matches: "No dictations match." with **Clear filters**.

## Settings → History

- **Keep history**: Forever (the default) / 30 days / 24 hours / Off. Shortening it deletes
  older entries at once, after a confirm: "Delete 840 dictations older than 30 days?". Off
  deletes everything after a confirm, and stops keeping new ones.
- **Clear history**: deletes everything, after a confirm ("Delete all 1,208 dictations? This
  cannot be undone."). Favorites go too.

## Storage

- `<data dir>/history.jsonl`: one JSON object per dictation, appended as each finishes.
  Edits (favorite, delete, retention) rewrite the file to a temporary one and rename it over,
  so a crash never leaves it half written.
- Each entry: an id, the time, the app (program file name and friendly name), the text, what
  was heard, inserted or not and why, the word count, seconds of speech, and the time from key
  up to text. No audio.
- Retention is applied when Utter starts, every hour, and when the setting changes.
- It stays on this computer, like everything else Utter has (DESIGN.md: local only).
- The developer's dictation log (`<data dir>/dictations/`, every pass with the audio) is a
  separate thing: written only by development builds, or when settings.toml has
  `debug_log = true`, so an installed Utter keeps no audio.

## Live

A dictation that finishes while History is open appears at the top with a soft fade in; the
selection stays where it was unless it was on the first row.

## Acceptance checks

1. A new dictation appears at the top within a moment of being typed, with its app and time.
2. A dictation that did not go in is marked Not inserted with the reason, and Copy works.
3. Search finds by the typed text and by what was heard; filters combine; the count follows.
4. Favorite, Delete and Undo work from the buttons and from C, F and D.
5. What Utter heard marks a removed filler and a corrected word correctly.
6. Keep history at 24 hours removes older entries after the confirm; Off stops keeping new
   ones; Clear history empties it.
7. 5,000 entries: the section opens and scrolls without a stutter.
8. An installed Utter writes nothing to `dictations/`.
