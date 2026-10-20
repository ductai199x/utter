# The Home window

Where a person changes how Utter works and looks back at what it did: History, Dictionary and
Settings, in one window. Status: **built** (0.1.10). History has its own spec
([history.md](history.md)); this one covers the frame and the Dictionary and Settings sections.
Paste rules, which Settings opens in a second window, are in [insertion.md](insertion.md).

Code: `app/home.html`, `app/src/home/` (`Home.svelte` frame, `Dictionary.svelte`,
`Settings.svelte`, `History.svelte`, `core.ts` the link to the core), `app/src-tauri/src/home.rs`
(commands and the settings watcher), `crates/utter-pipeline/src/settings.rs` (the file).

## What it is for

1. Teach Utter the words it gets wrong: names, products and terms, spelled the person's way.
2. Change the few settings that matter, and see at once that they took.
3. Stay out of the way: open it from the tray, change something, close it. Dictation keeps
   working the whole time.

## Opening and closing

- Tray menu: **Open Utter** (the first item). A left click on the tray icon opens the menu.
- Starting Utter again while it runs opens the window (the single-instance handler). While the
  setup window is open (first run, a model download, the problem screen) Open Utter brings
  setup forward instead: Settings with nothing installed yet makes no sense.
- Closing the window hides it; Utter keeps running. Quit is in the tray menu (macOS: the menu
  bar). On a desktop that shows no tray icons (GNOME without an AppIndicator extension),
  Settings > General has a **Quit** row.
- The window is created once, hidden, at start. It opens on the section it was last on
  (remembered per computer); the first time, on History. A `settings.toml` that does not read
  opens it on Settings.
- It re-reads its state when it gains focus, so what changed while it was hidden (a microphone
  plugged in, a download done) shows.

## Frame

- A normal window with the OS title bar, 1040 x 680 logical pixels by default, at least
  760 x 480, centered. On a screen smaller than that it shrinks to the work area (taskbar or
  dock left out, 16 px margin) each time it opens. It follows the OS light or dark theme.
- A **rail** on the left, 60 px wide: the animated mark at the top (it draws itself in when the
  window opens, then rests), then **History**, **Dictionary**, and **Settings** at the bottom.
  Each is a 40 px icon button with a tooltip ("Dictionary (Ctrl+2)"; Cmd on a Mac). The selected
  one has a soft accent fill and a short accent rule at the rail's edge.
- A section fills the rest. Dictionary and Settings are a column at most 680 px wide (36 px top,
  44 px side padding): a 26 px title, a one-line description, then the content. Groups have an
  11.5 px uppercase header. A section change fades in over 160 ms.

## Dictionary

Title "Dictionary". Description: "Words Utter should spell your way: names, products, terms."

- **Add field** at the top, placeholder "Add a word or name", with an **Add** button. Enter or
  Add adds what was typed. Several words separated by commas or line breaks (typed, or pasted:
  a paste with a line break is added at once) become separate entries. Spaces at both ends are
  trimmed.
- An entry already in the list is not added again; its row flashes once (`--dur-flash`, 600 ms).
- **The list**: one row per entry, newest first (several added together keep the order typed),
  38 px high. The text is as written. A remove button (an x, 28 px) shows at the right on hover and on keyboard focus. Double-click or
  Enter on a row edits it in place: Enter saves (or on losing focus), Esc cancels. Delete on a
  focused row removes it. Typing into the add field filters the list as you type (not while the
  text has a comma or line break); the filter clears when an entry is added.
- **Count** at the list's top left: "12 words" ("1 word").
- **Import** (a text button at the right of the count): a file picker for a .txt or .csv file.
  Each line is an entry; for a .csv the first column, quotes removed. The message under the
  field: "Added 40 words (3 already here)".
- **Empty state:** "No words yet. Add names and terms Utter mishears, like your company, your
  teammates or the tools you use."
- **Remove** shows "Removed Supabase." with **Undo** under the add field for 6 s. Undo puts the
  entry back at its place. Messages (including "Added ...") stay 6 s.
- Ctrl+F (Cmd+F on a Mac) on this section focuses the add field.

Rules the code enforces (`clean_terms`, `settings.rs`; the page checks the same to say why):
- An entry is 1 to 64 characters with at least 2 letters or digits. Otherwise the message is
  "A word needs at least two letters or digits, and at most 64 characters".
- At most 2000 entries. Past that, "The dictionary is full (2000 words)".
- Entries are trimmed and de-duplicated when `settings.toml` is read, so a hand edit is cleaned
  the same way.

## Settings

Title "Settings". Description: "Changes apply to your next dictation." Rows are a label, a muted
description line under it, and the control at the right. Groups, in order:

**General**

| Setting | Control | Description |
|---|---|---|
| Start with Windows ("Start at login" elsewhere) | switch, on | "Utter starts in the tray when you sign in." ("... starts in the background ..." off Windows). Only an installed copy registers itself; a development build never does. |
| Quit Utter | button, only when the desktop shows no tray icons | "This desktop shows no tray icons. Opening Utter again brings back this window." |

**Dictation**

| Setting | Control | Description |
|---|---|---|
| Format lists | switch, on | "Spoken lists become bullet points." |
| Microphone | dropdown: "System default", each input device by name, and a saved one that is unplugged as "<name> (not connected)" | "Utter listens to this one." With no device: "No microphone connected. Plug one in; Utter uses it right away." A microphone that cannot open shows its error in amber and the previous one stays. |

**Cleanup model**

| Setting | Control | Description |
|---|---|---|
| Runs on | dropdown, only when the computer can run more than one: NVIDIA CUDA, Vulkan, Apple GPU, Processor; the best is marked "(recommended)" | One line per choice (for example "Fastest on NVIDIA graphics cards."). Choosing another saves it and shows a row "Utter restarts to switch to <name>" with **Restart now**; it says what it downloads first (dictation is off until that is done) or "It takes a few seconds." |
| Processor threads | dropdown, only while running on the processor: Default (half the threads) / High (three quarters), with the thread counts | Applies at the next start, with the same restart row. |
| Model | segmented: Quality / Light / Extra light, only those that run on the chosen runtime | The description follows the choice and the hardware (figures from `gpu_need`, `crates/utter-pipeline/src/lib.rs`). On CUDA: Quality "The bigger model, most accurate: about 4.8 GB of GPU memory."; Light "The smaller model, for 4 GB graphics cards: about 3.3 GB."; Extra light (processor only) "The smallest model, fastest on the processor: about N GB of memory." Quality is disabled, with "Quality needs a graphics card with N GB of memory." underneath, when the card is too small or there is none. A model not installed yet says "Downloads N GB" in its tooltip; choosing it opens setup to fetch it, and dictation switches when that is done. |

While a model loads the row shows "Loading..." with a spinner, then nothing; a failure shows the
error in amber and the old model keeps serving. A saved model that does not run on the runtime
shows the one that runs instead (Quality on the processor: Extra light; Extra light on a
graphics card: Light).

**Typing**

| Setting | Control | Description |
|---|---|---|
| Paste rules | button **All paste rules...** | "Most apps paste with Ctrl+V and read the clipboard at once. Utter knows N apps that don't, and you have M of your own." Under it, the person's own rules and any default they changed, each editable in place (Paste with, Keep on the clipboard) with **Reset** (a changed default) or a remove button (their own). See [insertion.md](insertion.md). |

**History**

| Setting | Control | Description |
|---|---|---|
| Keep history | dropdown: Forever (default) / 30 days / 24 hours / Off | "Your dictations, kept on this computer only." Keeping less asks first: "Delete 840 dictations older than 24 hours?" (title "Keep less history?"); Off asks "This deletes every dictation in History and keeps no new ones." (title "Turn History off?"). Keeping longer deletes nothing. |
| Clear history | button **Clear...** | "Deletes every dictation, favorites too." Asks "Delete all 1208 dictations? This cannot be undone."; does nothing when History is empty. |

The confirm dialog opens with the focus on Cancel, so a stray Enter deletes nothing; Esc or a
click outside cancels. If History cannot be read, the section says "History could not be read:
..." and nothing is written over the file.

**Shortcuts**

| Setting | Control | Description |
|---|---|---|
| Hold to talk | dropdown of keys | "Hold to dictate. Double-tap for hands-free; Esc cancels." |
| Paste last dictation | the keys, read-only | "Into the app you are in. Also in the tray menu." Alt+Shift+Z (Ctrl+Cmd+V on a Mac). |
| Copy last dictation | the keys, read-only | "To the clipboard, without pasting." Alt+Shift+X (Ctrl+Cmd+C on a Mac). |

The dictation keys are those that do nothing when pressed alone: Right Ctrl (default), Left
Ctrl, Right Shift, Scroll Lock, F13 to F24. Not Alt (letting it go opens an app's menu), Caps
Lock or Insert (they toggle), or the mouse's side buttons. A Mac gets Right Option (default)
and Right Command first, and has no Scroll Lock or F20 to F24. A change applies at once; the
tray tooltip, the splash and Try it name the chosen key. A key that cannot be listened to shows
the error on the row.

**About**

- **Private by design**: five plain lines on what Utter does with data (everything on this
  computer; the internet only for models and updates; never reads files or the screen; history
  stays here; open source).
- **Version**.
- **Updates**: a status line and a switch. On, Utter asks GitHub once a day (the first check a
  minute after it is ready) and only offers: "Utter 0.1.11 is ready to install. Your models and
  settings stay." with **Update**; nothing installs until it is clicked. **Check now** asks at
  once ("Checking...", then "You have the latest version (0.1.10)."). Off: "Off: Utter never
  checks." The tray also shows "Update to Utter X..." while one is offered; it opens Settings
  at this row. A .deb install adds "Your password is asked once."
- **Data folder** and **Settings file** with the path (monospace) and an **Open** button each.
- **Keep logs**: "Timings and errors, never what you say. They help find out what went wrong;
  off deletes them." On shows an **Open** link to the logs folder.

`debug_log` (the developer's dictation log with audio) has no control: it is a line in
`settings.toml` ([history.md](history.md), Storage).

## Saving and applying

- Every change saves at once; there is no Save button. Settings live in `settings.toml`
  (`<data dir>`; DESIGN.md section 11), which a person can edit by hand. Saving rewrites only the
  values, so comments and key order survive; the file is written to a temporary file and renamed
  over.
- The core watches the file's modification time every 500 ms and applies a change from either
  source, so a hand edit shows in an open window and reaches the pipeline.
- Changes apply to the next dictation, with no restart, except the runtime and processor
  threads (Restart now):
  - Dictionary, Format lists, paste rules, Keep history: immediately.
  - Microphone: the microphone reopens.
  - Model: the new model loads while the old one keeps serving; a dictation during the switch
    uses the old model. If it is not downloaded, setup opens for it first.
  - Dictation key: the listener restarts on the new key.
- A change made during a dictation waits until it ends (the pipeline reads settings between
  dictations).
- If `settings.toml` has an error (a hand edit), the window opens on Settings with a banner
  showing the parser's message and **Open file**. Nothing is overwritten until the file reads
  cleanly: a save is refused ("settings.toml has an error, so it was not changed: ...").
  Unknown keys count as errors, so a misspelled key is not a silent default.

## Motion

Uses the tokens in `app/src/lib/tokens.css`. A section change fades in over 160 ms. A new
dictionary row slides in from 4 px above over 120 ms and a removed row collapses over 160 ms
(`--dur-enter`, `--dur-exit`). `--dur-flash` (600 ms) is a background pulse to the accent and
back. History's motion is in [history.md](history.md).

## Keyboard

- Ctrl+1, Ctrl+2, Ctrl+3 (Cmd on a Mac) switch to History, Dictionary and Settings. Up and Down
  move between rail buttons when the rail has focus.
- On Dictionary, Ctrl+F focuses the add field.
- Tab goes through the rail, then the page from top to bottom; controls show a focus ring.
- The rail is a navigation landmark ("Sections") with the current section marked
  `aria-current`; the remove buttons are labelled "Remove <entry>"; switches are
  `role="switch"` and the model choice a radio group.

## Acceptance checks

Legend: **met (test)** an automated test covers it; **met (code)** the code does it and I read
it, no automated test; **unverified** needs the real window, a timer or another computer;
**not built**. No UI test exists for this window (the app has no browser test suite); the file
and History logic have Rust tests.

| # | Check | Status |
|---|---|---|
| 1 | Tray > Open Utter opens the window on the last section within 200 ms | Unverified. The window is created once, hidden, and the section is remembered (code); the time was never measured. |
| 2 | Adding "Supabase" and dictating "we moved auth to super base" types "Supabase", without a restart | Unverified end to end. The dictionary reaches the pipeline through the watcher and `PipelineConfig.dictionary` (code); the cleanup eval covers dictionary use ([cleanup.md](../cleanup.md)). |
| 3 | Pasting "Alice, Bob\nCarol" into the add field adds three entries | Met (code): `submitDraft` splits on commas and line breaks. |
| 4 | Adding an existing entry adds nothing and flashes the existing row | Met (code). |
| 5 | Remove, then Undo, restores the entry in its place | Met (code): `restore` inserts at the saved index. |
| 6 | Turning off Format lists and dictating "first eggs second milk" types a sentence, not bullets, without a restart | Unverified end to end. `lists` reaches cleanup as `lists_off` (code); the lists eval is in [cleanup.md](../cleanup.md). |
| 7 | Choosing another microphone applies on the next dictation; unplugging it shows the amber error and dictation keeps working on the previous one | Unverified. The `Applied` event carries the error to the row (code). |
| 8 | Switching to Light shows Loading... and the next dictation after it uses the Light model | Unverified live. `ModelLoading` and `Applied` drive the row; the dictation log names the model (code). |
| 9 | A hand edit to settings.toml shows within a second; a broken edit shows the banner and overwrites nothing | Met (test) for "overwrites nothing" (`a_broken_file_is_not_overwritten`, `a_misspelled_key_is_an_error_not_a_silent_default`). The 500 ms poll (code) makes "within a second" plausible; not timed. |
| 10 | Everything is reachable and operable with the keyboard alone | Unverified. Buttons, selects and switches are native elements; the History list and filter menus are partly mouse-first (see [history.md](history.md)). |
| 11 | At 100% and 150% scaling, light and dark themes | Unverified. |
| 12 | Settings round trip: every key survives a save, comments and order are kept | Met (test): `saving_keeps_comments_and_round_trips`, `the_template_parses_to_the_defaults`. |
