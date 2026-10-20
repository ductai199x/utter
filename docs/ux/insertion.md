# Putting text in

How a finished dictation gets into the app the person is in, and what happens when it cannot.
Status: **built** (0.1.10) on Windows, Linux and macOS; the measurements below are Windows
only. The mechanism is DESIGN.md section 4, step 8; the rules around it (never lost, spacing,
confirming, paste last, paste rules) were set by Tai on 2026-10-11, after looking at how Wispr
Flow does it.

Code: `crates/utter-platform/src/insert.rs` (paste, clipboard, defaults; macOS in `macos.rs`,
Linux keys in `linux_input.rs`), `focus.rs` (the focused app), `caret.rs` (text before the
cursor), `shortcut.rs` (paste last, copy last), `crates/utter-pipeline/src/lib.rs` (spacing,
confirming, the commands, `NotInserted`), `crates/utter-pipeline/src/settings.rs` (rules in
`settings.toml`), `app/src/rules/Rules.svelte` (the Paste rules window).

## The one rule

**A dictation is never lost.** Either it goes in where the cursor was, or it is left on the
clipboard and the Bar says so. Either way it is in History ([history.md](history.md)), and the
last one can always be pasted or copied again.

## How text goes in

1. When the hotkey goes down, Utter notes the window with the keyboard focus (`focus.rs`).
2. When the dictation is done, and only if that window still has the focus: Utter saves the
   clipboard, puts the text on it, presses the app's paste keys, and puts the old clipboard
   back once the app has read it.
3. The old clipboard comes back only if nothing has changed it since: a copy the person makes
   in the meantime is never overwritten. One restore is pending at a time: a paste within the
   previous one's keep time keeps the clipboard that paste saved (the person's, not Utter's
   text) and restores it once, later.
4. The dictated text is marked to stay out of clipboard history and monitors (Windows: the
   `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` and
   `CanUploadToCloudClipboard` formats; macOS: the nspasteboard.org transient types; Linux: the
   exclude-from-history hint).

There is no OS call that pastes into any app: every app implements paste itself. The clipboard
and the app's own paste keys are what works in every app (Wispr Flow does the same). Text
Services Framework, the input-method layer, would avoid the clipboard; it is a later
investigation, not built.

## Spacing

A dictation that continues text gets a space in front, the way one would type it: two in a
row read "levels. There is", not "levels.There is". At key down Utter asks the focused
control for the two characters before the cursor (`caret.rs`), on a thread of its own while
the person speaks, so the paste does not wait for it. The source per OS: UI Automation's text
pattern on Windows, AT-SPI's Text interface on Linux, `AXSelectedTextRange` on macOS.

| Before the cursor | Space? |
|---|---|
| a letter, digit, or closing punctuation (`.` `?` `)` and so on) | yes |
| nothing (the start of the field), a space or a line break | no |
| an opening bracket or quote, `/`, `-`, a dash, `@`, `#` | no |
| the app does not say (a terminal), or not within 20 ms of the text being ready | no, as before |

Text that starts with punctuation (`, and more`, also `;` `:` `!` `?` `)` `%`) never gets one.
The space goes into the paste only, not into History. Paste last (below) follows the same rule
and waits up to 150 ms for the answer, because it asks only when its keys are pressed.
Checked in Notepad (2026-10-12): the start of the field, right after a dictation, and after a
line break; the rule itself has unit tests (`a_dictation_after_text_gets_a_space`,
`no_space_where_typing_would_not_put_one`).

## When it cannot go in

| Why | What Utter does | The Bar says |
|---|---|---|
| The focus moved to another window before the text was ready | leaves the text on the clipboard | "Copied: press Ctrl+V to paste" |
| No window had the focus, or the taskbar, the desktop, Windows' own input overlay or the lock screen did (a paste there vanishes) | leaves it on the clipboard | "Copied: press Ctrl+V to paste" |
| The app runs as administrator (Windows drops keys Utter sends it) | leaves it on the clipboard, does not press keys | "Copied: this app runs as administrator. Press Ctrl+V" |
| The paste keys could not be sent | leaves it on the clipboard | "Copied: press Ctrl+V to paste" |
| The keys went, but the app's text does not show the dictation (Confirming the paste) | leaves it on the clipboard | "It didn't paste here. Copied: press Ctrl+V" |
| Copying failed as well | nothing | "Couldn't paste or copy it: it is in History" |
| Nothing was heard | nothing, and no History entry | "Didn't catch that" |

- "Left on the clipboard" means the old clipboard is not restored: the dictation stays until
  something else is copied. It is an ordinary copy, in clipboard history and seen by clipboard
  managers, since the person pastes it themselves; so are History's Copy and copy last. Only
  a paste's own text, put back over within a second, is marked to stay out.
- The message uses this platform's paste keys (Cmd+V on macOS).
- The Bar holds an error message for 2.5 s (`ERROR_HOLD`, `app/src-tauri/src/main.rs`) and a
  confirmation for 1.2 s (`NOTICE_HOLD`).
- History marks the entry **Not inserted**, with the reason code (`focus-moved`, `no-window`,
  `refused`, `failed`, `unconfirmed`).
- The administrator check exists on Windows only; Linux and macOS never refuse.

## Confirming the paste

An app that says what is before the cursor (the same reading as Spacing) shows whether a
paste went in: for up to 400 ms after the keys, Utter reads it again every 40 ms, and the last
16 letters and digits of the dictation must end it. Only letters and digits count, so an app
that turns quotes curly, adds a space or changes case still counts as pasted. When they do not
show, the dictation is copied as above, History marks it Not inserted, and the Paste rules
window shows a line at its top: "Your last dictation, in Slack, did not paste." with one button
that adds a rule for that app typing the text out ("Type it out in Slack"), or, when the app
already has a rule, "Show its rule". An app that does not say what is before the cursor is not
second-guessed (it is treated as pasted).

## Paste and copy the last dictation

Two shortcuts, the same as Wispr Flow's so they are familiar, and two tray menu items (the
shortcut is shown beside each):

| Action | Shortcut (Windows, Linux) | macOS | Does |
|---|---|---|---|
| **Paste last dictation** | Alt+Shift+Z | Ctrl+Cmd+V | pastes the last dictation into the window that has the focus now, by its paste rule |
| **Copy last dictation** | Alt+Shift+X | Ctrl+Cmd+C | puts the last dictation on the clipboard, without pasting |

- "The last dictation" is the newest one with text, inserted or not. It is held in memory and,
  after a restart, taken from History (the newest entry with text). With History off it lives
  only until Utter quits.
- The shortcut waits (up to 1 s) for its own modifier keys to be let go before sending paste
  keys, so the app does not see Ctrl+Alt+Shift+V.
- Paste last into an administrator window, or with no window in front, falls back as above
  ("Copied: ...").
- With nothing to paste, the Bar says "Nothing to paste yet".
- The Bar confirms each: "Pasted" or "Copied", 1.2 s, no waveform.
- The shortcuts are registered with the OS (`RegisterHotKey` on Windows; the evdev listener on
  Linux; the event tap on macOS), so they reach Utter in every app. If another program holds
  one, Utter logs it and the tray item still works.
- They are not changeable yet; the keys are shown, read-only, in Settings > Shortcuts. Making
  them changeable alongside the dictation key is not built.

A third tray item, **Retry last dictation** (no shortcut), runs the last dictation's audio
through recognition and cleanup again, as fast as they go (not at speaking pace), and pastes the
result where paste last would. It is for a dictation something went wrong with: a GPU error, a
cleanup that timed out (the raw text went in), a microphone that dropped out. The audio is kept
in memory only, never written, and only the newest one; with none yet, the Bar says "Nothing to
retry yet". The retry makes a new History entry.

## Paste rules

Most apps paste with Ctrl+V and read the clipboard at once. Some do not: terminals that paste
with Ctrl+Shift+V or Shift+Insert, apps that read the clipboard late (Word, Outlook), remote
desktops that sync it over the network. A paste rule says, for one app, how to paste and how
long to keep the dictation on the clipboard.

**An app** is its program file name, which is stable (`mintty.exe`, `WINWORD.EXE`; on Linux the
process name, `konsole`), matched without regard to case, shown with its friendly name (from
the program's version information on Windows, the desktop entry's Name on Linux). A Store app's
window belongs to `ApplicationFrameHost.exe`; Utter looks inside it for the app's own process.

**Paste with:**
- **Ctrl+V** (the default on Windows and Linux; Cmd+V on macOS)
- **Ctrl+Shift+V** (Linux terminals, some Windows terminals; Cmd+Shift+V on macOS)
- **Shift+Insert** (old terminals, PuTTY, mintty; not offered on a Mac, which has no Insert key)
- **Type it out**: the text as key strokes, for apps that block pasting. Slower, and a line
  break is an Enter key, which sends the message in chat apps. On Linux only characters on the
  US layout can be typed; any other makes the paste fail (and fall back to the clipboard).

**Keep on the clipboard:**
- **Normal**: the old clipboard comes back after 0.5 s (the default; Wispr Flow's too).
- **3 seconds**: for apps that read the clipboard late.
- **5 seconds**: for remote desktops.
- **Leave it there**: the old clipboard is not restored.

### Utter's defaults

Windows (`insert.rs`, `DEFAULT_RULES`):

| App | Program | Paste with | Keep |
|---|---|---|---|
| Git Bash (mintty) | `mintty.exe` | Shift+Insert | Normal |
| PuTTY | `putty.exe` | Shift+Insert | Normal |
| Alacritty | `alacritty.exe` | Ctrl+Shift+V | Normal |
| WezTerm | `wezterm-gui.exe` | Ctrl+Shift+V | Normal |
| Microsoft Word | `WINWORD.EXE` | Ctrl+V | 3 seconds |
| Outlook (classic) | `OUTLOOK.EXE` | Ctrl+V | 3 seconds |
| Outlook (new, listed as "Outlook") | `olk.exe` | Ctrl+V | 3 seconds |
| WhatsApp | `WhatsApp.exe` | Ctrl+V | 3 seconds |
| Remote Desktop | `mstsc.exe`, `msrdc.exe` | Ctrl+V | 5 seconds |
| Citrix Workspace | `wfica32.exe`, `CDViewer.exe` | Ctrl+V | 5 seconds |
| TeamViewer | `TeamViewer.exe` | Ctrl+V | 5 seconds |
| VNC Viewer | `vncviewer.exe` | Ctrl+V | 5 seconds |

Windows Terminal, the classic console and VS Code's terminal paste with Ctrl+V: no rule.

Linux: every common terminal pastes with Ctrl+Shift+V, Normal: GNOME Terminal
(`gnome-terminal-server`), Console (`kgx`), Ptyxis (`ptyxis`, `ptyxis-agent`), Konsole, Xfce
Terminal, Tilix, Terminator, Alacritty, kitty, WezTerm (`wezterm-gui`), foot. XTerm pastes with
Shift+Insert.

macOS: terminals paste with Cmd+V like every app, so no terminal rules. Late readers: Microsoft
Word and Outlook (3 seconds); Microsoft Remote Desktop, Windows App, Citrix Viewer, TeamViewer
and VNC Viewer (5 seconds).

A person only ever sees their own platform's defaults and choices. Every other app: Ctrl+V,
Normal.

### How Utter knows the app

At key down Utter asks the OS for the window with the keyboard focus, then the process that
owns it, then that process's program file: its file name is what rules match on. It knows the
app, not the text field: every site in a browser is the browser, and a terminal inside VS Code
is VS Code. On Linux the focused app comes from the accessibility bus; when that knows nothing
(an app with accessibility off, such as Chromium by default), the window is unknown: the
dictation is still pasted where the person is typing, but it cannot be matched to a rule or
checked for focus changes.

### Editing them

**Settings > Typing** says how many apps Utter's defaults cover, lists the person's own rules
and any default they changed (Paste with and Keep as dropdowns, edited in place), and has
**All paste rules...** ([home.md](home.md)).

**All paste rules...** opens the **Paste rules window** (title "Paste rules", 880 x 620,
resizable to 640 x 420). Heading line: "How each app pastes a dictation. Every other app:
Ctrl+V, and the clipboard back after 0.5 s."
- A table, one row per app: its name and program(s), **Paste with** and **Keep on clipboard**
  as dropdowns edited in place, **Dictations** into it (from History), and **Source**: Default,
  Changed (an edited default) or Yours. Programs of one app (Remote Desktop's `mstsc.exe` and
  `msrdc.exe`) share a row, and a change applies to all of them.
- **Search apps** by app or program name (Ctrl+F), with the count ("3 of 12": Remote Desktop and Citrix's two programs each share a row); **sort** by any
  column (Dictations, most first, to start; ties A to Z); **filter chips** for Source (All,
  Yours, Changed, Defaults, with counts), Paste with and Keep.
- A changed default has **Reset** (back to Utter's value); a default cannot be removed (set to
  Ctrl+V, Normal it is like any other app, and the row stays as a Changed default). The
  person's own rules have a remove button. Setting a rule back to Utter's default values removes
  it from the file.
- **Add an app** opens a picker: a search field ("Find an app"), then **Open now** (the apps
  open right now, asked for each time it opens), then **Dictated into** (most used first), each
  with its program name and dictation count; apps that already have a rule say "Has a rule" and
  cannot be picked, so a new rule is never made over a default. Enter picks the first free
  match. **Choose a program...** at the bottom picks a program file: only its file name is
  kept. The new row flashes once in the table. The picker's file filter is `.exe`, which fits
  Windows only; on Linux and macOS only the lists above it are useful.
- Both the Settings section and the window edit settings.toml and follow each other live.
- Esc closes the picker, then a filter menu, then clears the search.

**Stored** in settings.toml as the person's changes only, not the whole table, so a later
version's better defaults still arrive and the person's edits still win. The file's template
carries this example, commented:

```toml
# How dictations are pasted, per app. Utter's defaults cover common terminals, Office and
# remote desktops; an entry here replaces the default for that program.
# with: "ctrl-v", "ctrl-shift-v", "shift-insert" or "type"
# keep: "normal" (0.5 s), "3s", "5s" or "leave" (the old clipboard is not restored)
[[paste]]
app = "mintty.exe"
with = "ctrl-shift-v"
keep = "normal"
```

`keep` may be left out (it then reads "normal"); an unknown key or value is an error that the
Home window shows as a banner ([home.md](home.md)).

## How a paste works, on every OS

One portable sequence (`insert.rs`, `paste_by_clipboard`); each OS supplies small primitives:
`save_all` and `restore` the clipboard, `put_text`, `sequence` (a change counter), `is_free`
(no one holds the clipboard), and `press` (the paste keys), plus `type_text` for Type it out.

1. Save the clipboard. Contents can show up a moment after a change, so a format listed but not
   readable yet is waited for, and an empty-looking clipboard is looked at again (150 ms) before
   it is believed.
2. Put the text on it, plainly, marked to stay out of clipboard history and monitors.
3. **Wait until the clipboard is quiet:** no one holding it and nothing changing it for 25 ms,
   at most 150 ms. Clipboard managers (Windows' clipboard history, GNOME's and KDE's, macOS
   history apps, UniClipboard) read every new clipboard at once, and an app gets one try at the
   clipboard when the paste keys arrive: if a manager holds it that instant, the app pastes
   nothing.
4. Press the app's paste keys.
5. After the keep time, put the old clipboard back, unless something new was copied meanwhile
   (the sequence number, read after the clipboard is closed).

A paste cannot be confirmed through the clipboard. Offering the text lazily (delayed rendering
on Windows, a data provider on macOS, selection requests on X11) tells Utter when something
read it, but managers read every clipboard at once and the OS then answers the app from that
copy, so the app's own read never comes back. Tried on Windows: it also missed more (5 of 240
into a RichEdit box, against none), so it is not used. Confirming reads the text instead
(Confirming the paste). A paste that misses is in History, and Alt+Shift+Z pastes it.

What each OS does:

| | Windows | Linux | macOS |
|---|---|---|---|
| Clipboard | Win32 clipboard; every format held in global memory is saved and restored (GDI handle formats such as CF_BITMAP are skipped; their memory forms, CF_DIB, are kept) | the `arboard` crate: X11 clipboard (Wayland through XWayland); only text, else an image, is saved | NSPasteboard; every item and type saved whole |
| Marked transient | the three formats above | exclude-from-history hint | nspasteboard.org transient and auto-generated types |
| Change counter | `GetClipboardSequenceNumber` | Utter's own write count | NSPasteboard `changeCount` |
| Is free | `OpenClipboard` succeeds | always | always |
| Paste keys | `SendInput` | Utter's own virtual keyboard (uinput), the same on X11 and Wayland; needs the udev rule the package installs | CoreGraphics key events; needs the Accessibility permission |
| Focused app | `GetForegroundWindow` and its process | the accessibility bus (AT-SPI) | the frontmost application |
| Text before the cursor | UI Automation | AT-SPI Text | Accessibility API |

Platform status and how each was checked: [../platforms.md](../platforms.md). Not built:
the RemoteDesktop portal and Wayland data-control clipboard protocols (GNOME has no
data-control; the design notes of 2026-10-12 considered them, and uinput plus XWayland's
clipboard replaced them, which worked on GNOME and KDE in VMs). Not verified here: the
"Paste from Other Apps" prompt that macOS 15.4 and later may show when an app reads the
pasteboard; Utter's first-run text names Accessibility only.

## Measured (2026-10-12, Windows, `insert::live`, run by hand)

A window of Utter's own (the RichEdit control Notepad is built on), never another app's, with
the person's clipboard saved and put back around every run:
- **Like dictations** (the clipboard settled for a second, then a paste, 60 times): with no
  clipboard manager, 60 of 60 went in and 60 of 60 clipboards came back. With UniClipboard
  running, 1 of 120 missed (from 1 of 60 before the quiet wait); every clipboard came back.
  A paste takes 52 ms p50, 69 ms at most.
- **Each case:** Ctrl+V, Shift+Insert, typing it out, Leave it there, a newer copy kept, focus
  moved refused: all pass.
- **The restore was broken before this:** the sequence number was read while the clipboard was
  still open, one behind, so the restore took every paste for the person's own copy and never
  put anything back. Fixed; 120 of 120 save-and-restore rounds come back.
- **The basic EDIT box** misses more (about 1 in 20 back to back) however the text is put on
  the clipboard: it opens the clipboard once and gives up. Real apps measured here are RichEdit.

These were run once, on Windows. They are not repeated on each build (the tests are `#[ignore]`d
and need a desktop: `cargo test -p utter-platform pastes_into_a_real_window -- --ignored
--nocapture`, and the others in the `live` module). The paste has not been timed or counted
on Linux or macOS; what was checked there by hand (VMs, an M2 Mac) is in
[../platforms.md](../platforms.md).

## Acceptance checks

Legend as in [home.md](home.md): met (test), met (code), unverified, not built.

| # | Check | Status |
|---|---|---|
| 1 | Notepad, Chrome, Slack (Electron), Word, Windows Terminal: the text goes in, and the old clipboard (text and an image) comes back | Partly met (test): the RichEdit window above, 2026-10-12. The five real apps are unverified on 0.1.10. |
| 2 | Click into another window before letting go: the text is on the clipboard, the Bar says "Copied: press Ctrl+V to paste", History marks it Not inserted | Met (code): `FocusChanged` becomes `FocusMoved`, copy-instead, History code `focus-moved`. The "focus moved refused" case passed in the live test. |
| 3 | An administrator Notepad: no keys sent, the text is on the clipboard, the Bar says so | Met (code): `refuses_input` is checked before any key. Not run on 0.1.10. |
| 4 | Alt+Shift+Z pastes the last dictation into the focused app; Alt+Shift+X copies it; both from the tray too; both after a restart | Met (code) for the shortcuts and tray; the restart part is met (test): `entries_survive_a_reopen_newest_first` checks `last_text`. |
| 5 | Git Bash: pastes with Shift+Insert by default; a rule changed to Ctrl+Shift+V takes effect on the next dictation, and Reset brings the default back | Met (test) for the rules: `a_rule_overrides_the_default_and_other_apps_paste_plainly`. Not run in Git Bash. |
| 6 | Copying something during the 0.5 s window: the new copy stays | Met (test): "a newer copy kept", live test 2026-10-12. |
| 7 | A dictation after text gets a space; none at the start of a field, after a line break, an opening bracket or in a terminal | Met (test) for the rule; Notepad checked by hand 2026-10-12. |
| 8 | A paste the app ignores is detected: copied, History "unconfirmed", the Paste rules window offers the fix | Met (test) for the detection (`a_paste_is_recognized_through_what_an_app_changes`); the window banner is met (code). |
| 9 | Retry last dictation re-runs the newest audio and pastes it; "Nothing to retry yet" with none | Met (code). Not run. |
| 10 | Linux terminals paste with Ctrl+Shift+V by default | Met (code) (`DEFAULT_RULES`); checked by hand in GNOME Terminal, Ptyxis and Konsole in VMs (see [../platforms.md](../platforms.md)). |
