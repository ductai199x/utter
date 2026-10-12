# Putting text in

How a finished dictation gets into the app the person is in, and what happens when it cannot.
Status: **spec**. The mechanism (DESIGN.md section 4, step 8) is built; this adds the rules
around it (Tai, 2026-10-11, after looking at how Wispr Flow does it).

## The one rule

**A dictation is never lost.** Either it goes in where the cursor was, or it is left on the
clipboard and the Bar says so. Either way it is in History, and the last one can always be
pasted or copied again.

## How text goes in

1. When the hotkey goes down, Utter notes the window with the keyboard focus (focus.rs).
2. When the dictation is done, and only if that window still has the focus: Utter saves the
   clipboard, puts the text on it, presses the app's paste keys, and puts the old clipboard
   back once the app has read it.
3. The old clipboard comes back only if nothing has changed it since: a copy the person makes
   in the meantime is never overwritten.
4. The dictated text is marked to stay out of Windows' clipboard history and cloud clipboard.

There is no OS call that pastes into any app: every app implements paste itself. The clipboard
and the app's own paste keys are what works in every app (Wispr Flow does the same). Text
Services Framework, the input-method layer, would avoid the clipboard; it is a later
investigation, not this.

## When it cannot go in

| Why | What Utter does | The Bar says |
|---|---|---|
| The focus moved to another window before the text was ready | leaves the text on the clipboard | "Copied: press Ctrl+V to paste" |
| No window had the focus | leaves it on the clipboard | "Copied: press Ctrl+V to paste" |
| The app runs as administrator (Windows drops keys Utter sends it) | leaves it on the clipboard, does not press keys | "Copied: this app runs as administrator. Press Ctrl+V" |
| The paste keys could not be sent | leaves it on the clipboard | "Copied: press Ctrl+V to paste" |
| Nothing was heard | nothing | "Didn't catch that" |

- "Left on the clipboard" means the old clipboard is not restored: the dictation stays until
  something else is copied. It is marked to stay out of clipboard history, as above.
- The message uses this platform's paste keys (Cmd+V on macOS).
- The Bar holds an error message for 2.5 s (`ERROR_HOLD`), as today.
- History marks the entry **Not inserted**, with the reason.

## Paste and copy the last dictation

Two shortcuts, the same as Wispr Flow's so they are familiar, and two tray menu items:

| Action | Shortcut (Windows, Linux) | macOS | Does |
|---|---|---|---|
| **Paste last dictation** | Alt+Shift+Z | Ctrl+Cmd+V | pastes the last dictation into the window that has the focus now, by its paste rule |
| **Copy last dictation** | Alt+Shift+X | Ctrl+Cmd+C | puts the last dictation on the clipboard, without pasting |

- "The last dictation" is the newest History entry with text, inserted or not. It survives a
  restart (it comes from History), unless History is off, in which case it lives only until
  Utter quits.
- Paste last into an administrator window falls back as above ("Copied: ...").
- With nothing to paste, the Bar says "Nothing to paste yet".
- The Bar confirms each: "Pasted" or "Copied", 1.2 s, no waveform.
- The shortcuts are registered with the OS (RegisterHotKey on Windows), so they reach Utter in
  every app and the app itself never sees them. If another program holds one, Utter logs it and
  the tray item still works.
- They become changeable with the dictation hotkey (later, DESIGN.md section 8.1).

## Paste rules

Most apps paste with Ctrl+V and read the clipboard at once. Some do not: terminals that paste
with Ctrl+Shift+V or Shift+Insert, apps that read the clipboard late (Word, Outlook), remote
desktops that sync it over the network. A paste rule says, for one app, how to paste and how
long to keep the dictation on the clipboard.

**An app** is its program file name, which is stable (`mintty.exe`, `WINWORD.EXE`), shown with
its friendly name from the program's version information ("Git Bash", "Microsoft Word"). A
Store app's window belongs to `ApplicationFrameHost.exe`; Utter looks inside it for the app's
own process.

**Paste with:**
- **Ctrl+V** (the default on Windows and Linux; Cmd+V on macOS)
- **Ctrl+Shift+V** (Linux terminals, some Windows terminals)
- **Shift+Insert** (old terminals, PuTTY, mintty)
- **Type it out**: the text as key strokes, for apps that block pasting. Slower, and a line
  break is an Enter key, which sends the message in chat apps: the rule's description says so.

**Keep on the clipboard:**
- **Normal**: the old clipboard comes back after 0.5 s (the default; Wispr Flow's too).
- **3 seconds**: for apps that read the clipboard late.
- **5 seconds**: for remote desktops.
- **Leave it there**: the old clipboard is not restored.

### Utter's defaults (Windows)

| App | Program | Paste with | Keep |
|---|---|---|---|
| Git Bash (mintty) | `mintty.exe` | Shift+Insert | Normal |
| PuTTY | `putty.exe` | Shift+Insert | Normal |
| Alacritty | `alacritty.exe` | Ctrl+Shift+V | Normal |
| WezTerm | `wezterm-gui.exe` | Ctrl+Shift+V | Normal |
| Microsoft Word | `WINWORD.EXE` | Ctrl+V | 3 seconds |
| Outlook (classic) | `OUTLOOK.EXE` | Ctrl+V | 3 seconds |
| Outlook (new) | `olk.exe` | Ctrl+V | 3 seconds |
| WhatsApp | `WhatsApp.exe` | Ctrl+V | 3 seconds |
| Remote Desktop | `mstsc.exe`, `msrdc.exe` | Ctrl+V | 5 seconds |
| Citrix Workspace | `wfica32.exe`, `CDViewer.exe` | Ctrl+V | 5 seconds |
| TeamViewer | `TeamViewer.exe` | Ctrl+V | 5 seconds |
| VNC Viewer | `vncviewer.exe` | Ctrl+V | 5 seconds |

Windows Terminal, the classic console and VS Code's terminal paste with Ctrl+V: no rule.
macOS and Linux get their own tables when those platforms are built (Linux terminals:
Ctrl+Shift+V). A person only ever sees their own platform's defaults and choices.

### Editing them (Settings → Typing)

- One row per app: its name and program(s), "Paste with" and "Keep" as two quiet dropdowns.
  Programs of one app (Remote Desktop's `mstsc.exe` and `msrdc.exe`) share a row, and a change
  applies to all of them.
- Utter's defaults that the person has not touched are folded away behind "Show Utter's
  defaults (12)"; the person's own rules and any default they edited always show.
- A default can be changed and **Reset** (back to Utter's value), not removed: set to Ctrl+V,
  Normal it is like any other app. The person's own rules have a remove button.
- **Add an app**: a menu of the apps open now, by friendly name, with **"<the app you last
  dictated into> (your last dictation)"** first, the quick path when a paste just did not work.
- Every other app: Ctrl+V, Normal.

**Stored** in settings.toml as the person's changes only, not the whole table, so a later
version's better defaults still arrive and the person's edits still win:

```toml
# How dictations are pasted, per app. Utter's defaults cover common terminals, Office and
# remote desktops; a line here replaces the default for that program.
# with: "ctrl-v", "ctrl-shift-v", "shift-insert" or "type"; keep: "normal", "3s", "5s" or "leave"
[[paste]]
app = "mintty.exe"
with = "ctrl-shift-v"
keep = "normal"
```

## Acceptance checks

1. Notepad, Chrome, Slack (Electron), Word, Windows Terminal: the text goes in, and the old
   clipboard (text and an image) comes back.
2. Click into another window before letting go: the text is on the clipboard, the Bar says
   "Copied: press Ctrl+V to paste", History marks it Not inserted.
3. An administrator Notepad: no keys sent, the text is on the clipboard, the Bar says so.
4. Alt+Shift+Z pastes the last dictation into the focused app; Alt+Shift+X copies it; both from
   the tray too; both after a restart.
5. Git Bash: pastes with Shift+Insert by default; a rule changed to Ctrl+Shift+V takes effect
   on the next dictation, and Reset brings the default back.
6. Copying something during the 0.5 s window: the new copy stays.
