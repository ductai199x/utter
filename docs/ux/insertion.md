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

### How Utter knows the app

At key down Utter asks the OS for the window with the keyboard focus, then the process that owns
it, then that process's program file: its file name is what rules match on, and its version
information gives the friendly name. A Store app's window belongs to its frame host, so Utter
looks inside it for the app's own process. It knows the app, not the text field: every site in
a browser is the browser, and a terminal inside VS Code is VS Code.

### Editing them

**Settings → Typing** says how many apps Utter's defaults cover, lists the person's own rules
and any default they changed (editable in place), and has **All paste rules…**.

**All paste rules…** opens the **Paste rules window** (880 x 620, resizable to 640 x 420):
- A table, one row per app: its name and program(s), **Paste with** and **Keep on clipboard**
  as quiet dropdowns edited in place, **Dictations** into it (from History), and **Source**:
  Default, Changed (an edited default) or Yours. Programs of one app (Remote Desktop's
  `mstsc.exe` and `msrdc.exe`) share a row, and a change applies to all of them.
- **Search** by app or program name (Ctrl+F), with the count ("3 of 13"); **sort** by any
  column (Dictations, most first, to start); **filter chips** for Source, Paste with and Keep.
- A changed default has **Reset** (back to Utter's value); a default cannot be removed (set to
  Ctrl+V, Normal it is like any other app). The person's own rules have a remove button.
- **Add an app** opens a picker: a search field, then **the apps open right now** (asked for
  each time it opens), then the apps dictated into (most used first), each with its program
  name and dictation count; apps that already have a rule say so. **Choose a program…** at the
  bottom picks any `.exe`: only its file name is kept. The new row flashes once in the table.
- Both the Settings section and the window edit settings.toml and follow each other live.
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

## How a paste works, on every OS

One portable sequence (insert.rs, `paste_by_clipboard`); each OS supplies six small primitives:
`save_all` and `restore` the clipboard, `put_text`, `sequence` (a change counter), `is_free`
(no one holds the clipboard), and `press` (the paste keys).

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

A paste cannot be confirmed. Offering the text lazily (delayed rendering on Windows, a data
provider on macOS, selection requests on X11) tells Utter when something read it, but managers
read every clipboard at once and the OS then answers the app from that copy, so the app's own
read never comes back. Tried on Windows: it also missed more (5 of 240 into a RichEdit box,
against none), so it is not used. A paste that misses is in History, and Alt+Shift+Z pastes it.

## Measured (2026-10-12, insert::live, run by hand)

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

## Other platforms

The primitives exist on macOS and Linux (checked 2026-10-12 against Apple's, freedesktop's and
the compositors' documentation and code); each has a catch to design for:

| Primitive | macOS | Linux X11 | Linux Wayland |
|---|---|---|---|
| Save and put text | NSPasteboard; mark with the nspasteboard.org transient and concealed types | own the CLIPBOARD selection; mark with `x-kde-passwordManagerHint` | `ext-data-control-v1` / `wlr-data-control` (wlroots, KDE); **not GNOME**, which needs XWayland's clipboard |
| Change counter | `changeCount` | XFixes selection events (Utter keeps a count) | the data-control offer events |
| Is free | always (no lock) | always | always |
| Paste keys | `CGEventPost` Cmd+V: the **Accessibility** permission | XTest, no permission | the **RemoteDesktop portal** (a consent dialog once, then a restore token), or uinput (root-like access) |
| Focused app | `NSWorkspace.frontmostApplication` | `_NET_ACTIVE_WINDOW`, `_NET_WM_PID` | per desktop: wlr-foreign-toplevel, a KWin script, a GNOME Shell extension, or AT-SPI |

- **macOS reading the clipboard prompts:** since the macOS 15.4 SDK Apple has been rolling out
  an alert when an app reads the pasteboard without a paste gesture ("Paste from Other Apps" in
  Privacy & Security). Saving the person's clipboard is such a read, so first run asks for it
  once, beside the Accessibility permission.
- **Clipboard managers everywhere** read at once (GNOME's own reads every change); the quiet
  wait covers them the same way.
- **Linux terminals** paste with Ctrl+Shift+V (Shift+Insert pastes the primary selection there).

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
