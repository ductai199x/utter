# History

Every dictation, kept on this computer, findable and copyable: the first section of the Home
window ([home.md](home.md)). Status: **built** (0.1.10).

Design reference (Tai, 2026-10-11): UniClipboard was studied for its UX, not copied:
keyboard-first browsing, search that answers as you type with a live count, a list beside the
selected item in full, one-key actions with their keys shown, filters that combine. The look is
Utter's own: its accent and type, and the mark's idea of a voice laying itself down into lines
of text.

Code: `app/src/home/History.svelte` (the section), `app/src-tauri/src/history.rs` (commands),
`crates/utter-pipeline/src/history.rs` (the store, search, "what Utter heard"),
`crates/utter-pipeline/src/lib.rs` (`history_entry`, retention).

## What it is for

1. Get any dictation back: one that did not go in, or one to use again elsewhere.
2. See what Utter heard beside what it typed, and what cleanup changed: the thing people are
   curious about in a dictation app, and the way to learn what to add to the Dictionary.
3. Nothing leaves the machine, and the person decides how long anything is kept.

## Where it lives

History is the first section on the Home window's rail and the one the window opens on the first
time; after that it opens on the section it was last on. The window, rail, sizes and shortcuts
that switch sections are in [home.md](home.md).

## The section

Two columns: the **list** (380 px at most, 300 px at least: a narrow window shrinks it first)
and the **entry** (the rest).

### Search and filters

At the top of the list:
- **Search** across the list's width, placeholder "Search dictations", with the count at its
  right ("1,208", or "12 of 1,208" while filtering). It searches the typed text and what was
  heard, ignoring case and accents, as the person types (the query is sent 90 ms after the last
  key; the core searches, a page at a time). Ctrl+F (Cmd+F on a Mac) or `/` focuses it; Esc
  clears it. Matches are highlighted in the list.
- Under it, one row of compact **filter chips**: a star (favorites only), **Status** (All,
  Inserted, Not inserted; a dot on the chip when any dictation did not go in, and the count in
  the menu), **App** (All apps, then the apps dictated into, most used first, with counts),
  **Time** (All time, Today, Yesterday, Last 7 days, Last 30 days). A chip with a choice shows
  it ("Slack", "Today") in the accent with a small x to clear it. Filters combine with each
  other and with the search. "Today" and "Yesterday" follow the local calendar; the other two
  are rolling 7 and 30 days.

### The list

Newest first, under day headers ("Today", "Yesterday", a weekday name within the last week,
else "Oct 3", with the year when it is not this one):
- Each dictation is a row that reads like a line of writing: the text, up to 2 lines at 14/21,
  then an ellipsis; under it, muted, the app's name ("Unknown app" when none), a dot and the
  time ("3:14 PM").
- A short accent rule at the left edge, which glides to the selected row and glints as it lands,
  marks the selection (motion off under reduced-motion). An amber rule and an amber dot before
  the app name mark a dictation that did not go in. Favorites show a small star at the end of
  the meta line.
- Up and Down move the selection, scrolling to keep it in view; the first row is selected when
  the section opens. The list loads 60 entries at a time and the next page as the end nears, so
  a long history stays light.

### The entry

The selected dictation in full:
- A muted header: the app, the full date and time ("Today at 3:14 PM", else "Monday, October 5
  at 3:14 PM"), and "42 words · 4.2 s of speech · ready in 190 ms" ("ready in" is key up to the
  paste, `total_ms`).
- **Not inserted**, when it was not, as an amber line: "Not inserted. <reason> It was left on
  the clipboard." The reasons, by the code stored in the entry:

  | Code | Reason shown |
  |---|---|
  | `focus-moved` | The focus moved to another window before it was ready. |
  | `no-window` | No window had the focus. |
  | `refused` | The app runs as administrator, so Windows would not let Utter type into it. |
  | `failed` | The paste did not go through. |
  | `unconfirmed` | The app did not take the paste. A paste rule can fix that app: Settings, Paste rules. |

  When the entry has no code, `failed` is shown. When copying also failed, the Bar says so
  ([insertion.md](insertion.md)) and the dictation is still here.
- **The text**, 19/31, selectable, scrolling if long.
- **What Utter heard**, under it, 14/24: the speech model's words with cleanup's changes marked
  in place. Words cleanup removed (fillers, a retracted phrase) are struck through and muted;
  words it changed or added are underlined in the accent. The compare is by word, ignoring case,
  accents and punctuation; a text over 1500 words shows as one replacement. A **Changes /
  Plain** toggle switches to the plain transcript. Shown only when it differs from the text.
- Actions along the bottom, each a text button with its key in a small kbd: **Copy** `C`,
  **Favorite** `F` (Unfavorite when it is one), **Delete** `D` (the Delete key too), and, when
  what was heard differs, **Copy what was heard**.
- Copy puts the text on the clipboard and leaves it there (this is the person copying, so it
  is an ordinary copy that clipboard managers see); the button says "Copied" for 1.2 s. Delete
  removes it at once and shows "Deleted. Undo" for 6 s (Ctrl+Z / Cmd+Z too); the next row is
  selected. The single-letter keys work when no text field has the focus and no text is
  selected.

### Empty states

- No dictations yet: the animated mark, drawn large, and "Hold Right Ctrl and speak in any app.
  Your dictations show up here." (the chosen dictation key), with "Nothing here yet." in the
  list column.
- History off: "History is off. Turn it on in Settings to keep your dictations here."
- No matches: "No dictations match." with **Clear filters**.
- The file cannot be read: a banner "History could not be read, so nothing was changed: ..."
  with **Open folder**. Nothing is written over it.

## Settings

Under Settings > History ([home.md](home.md)):
- **Keep history**: Forever (the default) / 30 days / 24 hours / Off. Keeping less deletes older
  entries after a confirm. Off deletes everything after a confirm and keeps no new ones; paste
  last then works from memory only, until Utter quits.
- **Clear history**: deletes everything, favorites too, after a confirm.

## Storage

- `<data dir>/history.jsonl`: one JSON object per line, appended as each dictation finishes (a
  line and its newline in one write). Edits (favorite, delete, undo, retention, clear) rewrite
  the file to `history.jsonl.tmp` and rename it over, so a crash never leaves it half written.
- Each entry (camelCase keys): `id` (the time in microseconds, nudged to stay unique and in
  order), `at` (ms since 1970), `appExe` and `appName` (program file name and friendly name),
  `text`, `heard`, `inserted`, `notInserted` (the code above, absent when inserted), `words`,
  `speechS`, `readyMs`, `favorite`. No audio. `text` is what Utter decided to type, without the
  space put in front for joining ([insertion.md](insertion.md), Spacing).
- What is kept: a dictation with text that was not a replay, while Keep history is not Off. A
  dictation where nothing was heard is not kept. Retry last dictation adds a new entry.
- A line that does not parse (a crash mid-append, a newer version's entry) is moved to
  `history.unreadable.jsonl` beside it, never deleted, and the file is rewritten without it.
  Bytes that are not UTF-8 spoil only their own line. A file that cannot be read at all is an
  error and is read again next time; it is never written over.
- Retention is applied when Utter starts, every hour while it runs, and when the setting
  changes. Off keeps nothing.
- It stays on this computer (DESIGN.md: local only).
- The developer's dictation log (`<data dir>/dictations/`, every pass with the audio) is a
  separate thing, written only by development builds or when settings.toml has
  `debug_log = true`, so an installed Utter keeps no audio.

## Live

A dictation that finishes while the section is open appears at the top, sliding in 8 px over
260 ms (the core sends a `history-changed` event). The selection stays where it was unless it
was on the first row.

## Acceptance checks

Legend as in [home.md](home.md): met (test), met (code), unverified, not built.

| # | Check | Status |
|---|---|---|
| 1 | A new dictation appears at the top within a moment of being typed, with its app and time | Met (code): the pipeline adds the entry and the Bar emits `history-changed`. Not timed. |
| 2 | A dictation that did not go in is marked Not inserted with the reason, and Copy works | Met (code) for the mark and Copy. The stored code comes from `NotInserted::code`; no UI test. |
| 3 | Search finds by the typed text and by what was heard; filters combine; the count follows | Met (test): `search_ignores_case_and_accents_and_reads_what_was_heard`, `filters_combine_and_apps_are_counted`. |
| 4 | Favorite, Delete and Undo work from the buttons and from C, F and D | Met (test) for the store (`favorite_delete_and_undo_are_saved`); the keys are met (code). |
| 5 | What Utter heard marks a removed filler and a corrected word correctly | Met (test): `changes_mark_fillers_corrections_and_rewritten_numbers`. |
| 6 | Keep history at 24 hours removes older entries after the confirm; Off stops keeping new ones; Clear history empties it | Met (test) for prune and clear (`pruning_and_clearing_remove_entries`); the confirms and Off are met (code). |
| 7 | 5,000 entries: the section opens and scrolls without a stutter | Unverified. The core pages 60 at a time; no 5,000-entry run is recorded. |
| 8 | An installed Utter writes nothing to `dictations/` | Met (code): logging is on only for `debug_log` or a development build (`Settings::apply`). |
| 9 | A torn or non-UTF-8 line never loses the other entries; an unreadable file is never overwritten | Met (test): `a_torn_last_line_is_kept_aside_and_the_next_entry_survives`, `bytes_that_are_not_utf8_spoil_only_their_line`, `a_history_that_cannot_be_read_is_an_error_and_never_written_over`. |
| 10 | The list is operable with the keyboard alone | Partly: Up/Down, `/`, Ctrl+F, C, F, D, Esc work; the filter menus are click-driven. Unverified. |
