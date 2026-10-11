# The Home window

Where a person changes how Utter works: their dictionary and their settings. Status: **spec,
not built**. History, snippets, styles and stats (DESIGN.md section 8.1) join it later as more
sidebar pages; this spec covers the first two pages and the frame they live in.

## What it is for

1. Teach Utter the words it gets wrong: names, products and terms, spelled the person's way.
2. Change the few settings that matter, and see at once that they took.
3. Stay out of the way: open it from the tray, change something, close it. Dictation keeps
   working the whole time.

## Opening and closing

- Tray menu: **Open Utter** (the first item, and a left click on the tray icon's menu).
- Launching Utter again while it runs opens the window (the single-instance handler).
- Closing the window hides it; Utter keeps running in the tray. Quit is in the tray menu.
- It opens on the page it was last on, centered on the monitor with the cursor.

## Frame

- A normal window with the OS title bar, 880 x 600 logical pixels, resizable to a minimum of
  640 x 440. It follows the OS light or dark theme and the OS text scale.
- A sidebar on the left, 200 px wide: the Utter mark and name at the top, then the pages
  (Dictionary, Settings), each a row with an icon and a label. The selected row has a filled
  background and an accent bar at its left edge.
- The page fills the rest: a 24 px title, a one-line description under it, then the content,
  in a column at most 640 px wide with 32 px padding.

## Dictionary page

Title "Dictionary". Description: "Words Utter should spell your way: names, products, terms."

- **Add field** at the top, with placeholder "Add a word or name". Enter adds what was typed.
  Several words separated by commas or new lines (pasted) are added as separate entries.
  Leading and trailing spaces are trimmed. An entry already in the list is not added again;
  the existing row flashes once (`--dur-flash`) so the person sees it is there.
- **The list** under it: one row per entry, newest first, 36 px high. Each row shows the entry
  as written, with a remove button (an x, 28 px target) at the right that appears on hover and
  on keyboard focus. Double-click or Enter on a row edits it in place; Enter saves, Esc
  cancels.
- **Count** at the list's top right: "12 words".
- **Import** (a text button beside the count): opens a file picker for a .txt or .csv file;
  each line is an entry, and for a CSV, the first column. The result is a one-line message
  under the add field: "Added 40 words (3 were already here)".
- **Empty state:** the list area says "No words yet. Add names and terms Utter mishears, like
  your company, your teammates or the tools you use."
- Removing shows a one-line "Removed Supabase. Undo" message under the add field for 6 s.
  Undo puts it back where it was.

Rules the code enforces (not the UI):
- An entry is 1 to 64 characters, with at least 2 letters or digits.
- At most 2000 entries. Past that, adding says "The dictionary is full (2000 words)".

## Settings page

Title "Settings". Rows of label, a description in a smaller muted line, and the control on the
right. Groups have a 13 px uppercase header.

**General**
| Setting | Control | Description |
|---|---|---|
| Start with Windows | switch, on | "Utter starts in the tray when you sign in." Only an installed copy registers itself; a development build never does. |

**Dictation**
| Setting | Control | Description |
|---|---|---|
| Format lists | switch, on | "Spoken lists become bullet points." |
| Microphone | dropdown: "System default", then each input device by name | "Utter listens to this one." |

**Cleanup model**
| Setting | Control | Description |
|---|---|---|
| Model | segmented: Light / Quality | Light: "Fastest. Runs on most GPUs (about 6 GB)." Quality: "Slower and larger (about 18 GB of GPU memory)." Quality is disabled, with "Not installed" under it, when its model is not installed. |

**Shortcut**
| Setting | Control | Description |
|---|---|---|
| Hold to talk | shows "Right Ctrl" (a key cap) | "Hold to dictate. Double-tap for hands-free; Esc cancels." Changing the key is a later feature. |

**About**
- Utter's version, the data folder (with an **Open** button) and the settings file (with an
  **Open** button), each on its own row.

## Saving and applying

- Every change saves at once; there is no Save button. Settings live in `settings.toml`, which
  a person can still edit by hand (DESIGN.md section 11).
- The window reads the file when it opens, and again when the file changes on disk, so a hand
  edit shows up.
- Changes apply to the next dictation, with no restart:
  - Dictionary and Format lists: immediately.
  - Microphone: the microphone reopens; if it cannot, the row shows the error in amber and the
    previous microphone stays in use.
  - Model: the new model loads (a few seconds) while the old one keeps serving. The row shows
    "Loading..." with a small spinner, then nothing. A dictation during the switch uses the
    old model.
- A change made during a dictation waits until it ends.
- If `settings.toml` has an error (a hand edit), the window opens on Settings with a banner:
  "settings.toml has an error on line 4: ...", with **Open file**. Nothing is overwritten
  until the file reads cleanly again.

## Motion

Uses the tokens in `app/src/lib/tokens.css`. Page switches are instant (no transition).
A new dictionary row fades and slides in from 4 px above over `--dur-enter`; a removed row
collapses over `--dur-exit`. `--dur-flash` (600 ms) is a background pulse to the accent color
at 20% and back.

## Keyboard

- Ctrl+1 and Ctrl+2 switch pages; Up/Down move between sidebar rows when the sidebar has focus.
- On the Dictionary page, Ctrl+F or typing into the add field filters the list as you type
  (the filter clears when an entry is added).
- Tab order: sidebar, then the page from top to bottom. Every control has a visible focus
  ring. Delete on a focused row removes it.
- Screen readers: the sidebar is a navigation landmark with a list of links; the dictionary
  list is a list with each row's remove button labelled "Remove <entry>".

## Acceptance checks

Checked on the real machine at 100% and 150% scaling, in light and dark themes.

1. Tray > Open Utter opens the window on the last page within 200 ms (the window is created
   once, hidden, like the Bar).
2. Adding "Supabase" and dictating "we moved auth to super base" types "Supabase", without a
   restart.
3. Pasting "Alice, Bob\nCarol" into the add field adds three entries.
4. Adding an existing entry adds nothing and flashes the existing row.
5. Remove, then Undo, restores the entry in its place.
6. Turning off Format lists and dictating "first eggs second milk" types a sentence, not
   bullets, without a restart.
7. Choosing another microphone takes effect on the next dictation; unplugging it shows the
   amber error and dictation keeps working on the previous one.
8. Switching to Quality shows Loading... and the next dictation after it finishes uses the
   Quality model (the dictation log names it).
9. A hand edit to settings.toml shows in the open window within a second; a broken edit shows
   the banner and overwrites nothing.
10. Everything is reachable and operable with the keyboard alone.
