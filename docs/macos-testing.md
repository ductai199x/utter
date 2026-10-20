# Developing and testing Utter on macOS

How Utter was built, tested and released on macOS without owning a Mac (2026-10-19/20): a rented
Apple Silicon Mac mini driven over SSH from the Windows PC, with Tai doing only the few steps
macOS reserves for a person. Everything here was done for real, on macOS 26.7 (Tahoe) and 27.0.1
(Golden Gate), on a Mac mini M2 with 16 GB. Read it before the next macOS session; the generic
remote-Mac tooling is the `vm-testing` skill (`scripts/mac.py`, `references/remote-mac.md`).

## 1. The machine

- **Scaleway Apple silicon** (Bare Metal > Apple silicon). Tai rented a Mac mini M2, 16 GB, 256 GB:
  slower than an M4, which makes it the better test machine. **24-hour minimum** (Apple's licence),
  so plan a session as a day. Tai creates the account, pays, and deletes the machine when done.
- The Overview page gives the SSH user (`m1`), the IP, and screen sharing: port (random, e.g.
  59010), user and password. Tai's own RSA key was registered at creation; Scaleway installs
  registered keys in `~/.ssh/scw_authorized_keys`, not `authorized_keys`.
- **A reinstall from the Scaleway console wipes everything** (including our key, the build
  tools, the signing keychain, sudoers) and takes about 2 hours. Upgrading macOS 26 to 27 that way
  was a fresh install: SSH then warned "REMOTE HOST IDENTIFICATION HAS CHANGED". Never accept a
  changed host key blindly: Tai read the fingerprint on the Mac (`ssh-keygen -lf
  /etc/ssh/ssh_host_ed25519_key.pub`) and it matched, then `ssh-keygen -R <ip>`.
- FileVault must stay off (it would lock SSH out at boot). Automatic login is off on Scaleway's
  image, so after every reboot Tai logs in once over screen sharing.

## 2. What only a person can do (and why)

Claude never types passwords, so these are Tai's, each a minute or two:

| When | What | Why it cannot be scripted |
|---|---|---|
| Once per machine | Log in to the desktop over screen sharing | the account password |
| Once per machine | `echo "m1 ALL=(ALL) NOPASSWD: ALL" \| sudo tee /etc/sudoers.d/m1-nopasswd` in Terminal | the password; afterwards Claude installs Homebrew, BlackHole etc. over SSH |
| Once per machine | Add `/usr/libexec/sshd-keygen-wrapper` to **Accessibility** (macOS 27: **Device Control and Data Access**) and **Screen & System Audio Recording** | macOS ignores synthetic clicks on privacy switches |
| Each first-run test | Turn on Utter's keyboard switch in System Settings and confirm with the password | the same; this *is* the first-run test |
| After each reboot | Log in at the login window | automatic login is off |
| macOS upgrade | Start it (Software Update, or the Scaleway console) | the password |

**Screen sharing client:** RealVNC Viewer failed against macOS ("Protocol error: key length too
large"; with its encryption off, "invalid security result": plain VNC auth, which macOS refuses).
**Devolutions Remote Desktop Manager** with the **Apple Remote Desktop (ARD)** type works (host,
port, user, password). The port answered only after a reboot once; if it does not connect, check
it with `Test-NetConnection <ip> -Port <port>` before blaming the client.

## 3. Setting up a fresh Mac

1. Tai: log in, run the sudoers line, add sshd-keygen-wrapper to the two privacy lists (section 2).
2. Claude, with the dedicated key `~/.ssh/scaleway-mac` (added through Tai's key if needed):
   `mac.py connect <ip> --user m1`, then `mac.py doctor` until screen recording and cliclick say ok.
3. `bash ~/utter/packaging/macos/dev-setup.sh` (or fetch it): Xcode tools if missing (Scaleway's
   image ships full Xcode), Homebrew, Node 24, uv, cliclick, SwitchAudioSource, BlackHole 2ch,
   Rust, the repository. Notes:
   - Fetch the script from the cloned repo, not raw.githubusercontent.com right after a push: its
     cache served the old version for minutes.
   - With passwordless sudo, `sudo -v` still asks for a password over SSH; the script uses
     `sudo -n true || sudo -v`.
   - BlackHole says "You must reboot"; `sudo killall coreaudiod` is enough.
4. The signing identity (section 6): copy `%USERPROFILE%\.utter-signing\` to `~/signing`, create a
   keychain `utter-signing`, import `utter.p12` (password `p12`), `set-key-partition-list`, add it
   to the search list. `security find-identity` shows it as `CSSMERR_TP_NOT_TRUSTED`: normal for a
   self-made certificate, and codesign uses it all the same.

## 4. Driving the Mac (mac.py)

- `exec` runs a zsh login shell (Homebrew and cargo on PATH); `bg LABEL -- cmd` keeps a build
  running with its log in `~/logs/LABEL.log`; poll for a marker line rather than sleeping.
- In Git Bash always `export MSYS_NO_PATHCONV=1`, or `/tmp/x` and `~/x` arrive as Windows paths;
  call mac.py by its `C:/...` path (with path conversion off, `$HOME/...` breaks).
- `shot` gives screen points (1920x1080 on that Mac); click coordinates are those pixels.
- **Clicks and keys:** `cliclick` for clicks. Typing into Utter's own windows (a web view) does
  not work with cliclick; System Events' `keystroke` does, but needs an Automation permission
  prompt for the calling process the first time. Keep it out of anything that must not block.
- **The dictation key:** `hold 61 SECONDS` (Right Option; 54 Right Command) posts real
  flagsChanged events with the device bit, which Utter's event tap needs; cliclick's `kd:alt`
  cannot tell left from right. `hold 53 0.05` is a real Esc; cliclick's `kp:esc` did not reach
  the tap. `hold KEYCODE S FLAGS` presses a shortcut (`hold 9 0.05 140000` is Ctrl+Cmd+V).
- **Scrolling:** `scroll X Y LINES` (a real wheel event); Page Down needs keyboard focus that a
  web view may not have.
- **Prompts that block a command:** asking Finder or System Events anything raises an Automation
  prompt and the command hangs until someone answers (the first `shot` hung on Finder; mac.py now
  reads the screen width from AppKit). Decline such prompts; they are about the test tool.
- **Menu bar extras:** clicking Utter's menu bar icon works, but after many synthetic clicks the
  menu bar stopped opening any menu (Spotlight too) for the rest of that session: a stuck input
  state, not Utter. Notification stacks also cover the top right; close them first.
- `/tmp` is cleared on reboot (the compiled hold tool goes with it; mac.py rebuilds it).

## 5. Testing Utter like a user

**Install and open.** `brew tap ductai199x/utter https://github.com/ductai199x/utter`, `brew trust
--cask ductai199x/utter/utter` (Homebrew 7 refuses a third-party tap's cask until trusted; found
only by testing the published release), `brew install --cask utter`. Open from Finder >
Applications (double-click) or Spotlight. Before a release, test the cask from a local tap
(`brew tap-new --no-git ductai199x/local`, copy the generated `Casks/utter.rb`) with a
`file://` URL (`packaging/macos/cask.sh ... file://$HOME/dist`). A Homebrew install from scratch
was verified by uninstalling Homebrew and running the official installer: it ends with **Next
steps**, three lines that put `brew` on PATH (the README repeats them).

**A clean first run.** `brew uninstall --cask --zap utter`, `tccutil reset All app.utter`, delete
`~/Library/Application Support/Utter` and `~/Library/LaunchAgents/app.utter.plist`. Caution:
after `tccutil reset Accessibility app.utter` macOS does not prompt again; Utter disappears from
the list and has to be added by hand (+, Cmd+Shift+G, `/Applications/Utter.app`). Only reset for
a true first run, never in the middle of one.

**The microphone.** `SwitchAudioSource -t output -s "BlackHole 2ch"` once; then `afplay clip.wav`
or `say -r 165 "sentence"` is what Utter hears (the sound stays inside the Mac). Dictation:
`(sleep 0.4; say -r 165 "...") & /tmp/mac-py-hold-<hash> 61 6.5`. Clips from the eval set
(`Utter_dev/eval/chatterbox/*.wav`) and `say` both worked. To test "no microphone", move
`/Library/Audio/Plug-Ins/HAL/BlackHole2ch.driver` away and restart coreaudiod.

**Checking results.** TextEdit as the target (`open -a TextEdit`, read it back with
`osascript -e 'tell application "TextEdit" to get text of document 1'`); Utter's log
(`~/Library/Application Support/Utter/logs/utter.log`: `dictation:` lines carry asr, cleanup,
insert and total ms; `ready at`); `history.jsonl`; process counts (`ps -axo command | grep -cE
"[U]tter.app/Contents/MacOS|[l]lama-server"`) around quit and `kill -9`.

**Lifecycle checks that found bugs** (each is worth repeating after macOS changes):
- first run: Allow, macOS's prompt, the switch, Utter restarting itself, microphone prompt, level
  bars, download, Try it, Finish;
- offline (`pfctl` blocking ports 80/443) and a connection cut mid-download, then Retry;
- update over the installed copy (permissions must survive: section 6);
- quit, `kill -9`, reopen while running (a "reopen" event, not a second process), a second
  process right after a sign-in start (macOS's session restore);
- reboot: start at login, no window, permissions kept;
- every Settings row, the paste rules window, Dictionary import (native file picker: Cmd+Shift+G),
  History (copy, favorite, search, delete, Undo), retention (age entries with a script while Utter
  is quit), Clear history, a broken `settings.toml`;
- no microphone at start, microphone back while running;
- uninstall and `--zap`.

## 6. Signing and permissions

macOS ties Accessibility and Microphone grants to the app's designated requirement. With the
ad-hoc signature Tauri makes, that is the exact build: **every update lost the keyboard
permission while System Settings still showed it on**. Utter is therefore signed with its own
certificate ("Utter Homebrew Signing", SHA-1 `E411284D...C2`, to 2036; `packaging/macos/sign.sh`):
the requirement becomes "app.utter signed by this certificate", and grants survived updates,
reinstalls and the in-app updater (measured on 26.7 and 27.0.1). The keys are backed up in
`a cloud drive\Utter signing keys.zip` (password in Tai's password manager) and in
`%USERPROFILE%\.utter-signing\`. Losing them means every user grants the permissions again.

Entitlements (`app/src-tauri/macos/Utter.entitlements`), with the hardened runtime:
`com.apple.security.device.audio-input` (without it macOS refuses the microphone silently: no
prompt, no entry in the list) and `com.apple.security.cs.disable-library-validation` (the speech
runtime is a downloaded library).

## 7. macOS facts that shaped the code

- **Keyboard access:** Accessibility alone lets Utter read keys and paste on macOS 26 and 27;
  asking for Input Monitoring at the same time added Utter to neither list. macOS applies the
  grant only to a new process, so Utter restarts itself when the switch turns on
  (`setup_restart_if_granted`). The first Allow shows only macOS's own prompt ("Open System
  Settings" goes to the switch); opening the pane as well left the prompt behind.
- **macOS 27** renamed Accessibility to **Device Control and Data Access**, and its prompt reads
  "Utter would like to control this Mac and access your data". Utter's text names neither pane and
  says beforehand that macOS words it strongly.
- **Speech:** onnxruntime's macOS build has WebGPU (Metal) built in; registering the WebGPU plugin
  as well failed ("EP library is already registered"). The Mac downloads no plugin.
- **llama.cpp for Metal** needs `libggml-rpc` in the component's extract list.
- **No parent-death signal on macOS:** a watcher shell ends llama-server when Utter ends.
- **Opening Utter while it runs** is a reopen event (`RunEvent::Reopen`), not a second process.
- **Session restore** at sign-in starts a second copy beside the login item: within 30 s of a
  sign-in start, it shows nothing.
- **Login item:** a LaunchAgent with `AssociatedBundleIdentifiers` (so Login Items shows "Utter"),
  written only when it changes (each write shows "Utter can run in the background").
- **Windows open behind** the app in front unless focused at creation (`.focused(true)`).
- **Sync Tauri commands run on the UI thread:** opening the microphone there froze the window for
  27 s while macOS asked for the microphone; such commands are async.
- **Wording:** a Mac has no graphics card (shared memory), pastes with Cmd+V, has a menu bar not a
  tray, Ctrl+Cmd+V/C shortcuts, no Scroll Lock or F20-F24, no Insert key; page shortcuts use Cmd.

## 8. Performance seen on the M2 (macOS 27, Metal)

Ready after launch 2.2-2.9 s warm (13.7 s right after the first download). Key up to text:
Quality 0.8-1.2 s, Light 0.15-0.5 s; the processor (Light, int8 speech) about 1.5 s.

## 9. Releasing for macOS

On the Mac, after the version bump: build (`npx tauri build --bundles app`), `sign.sh`,
`cask.sh ... VERSION dist` (makes the zip, the `.app.tar.gz` for the in-app updater and
`Casks/utter.rb`), copy them to the PC, `packaging/latest-json.sh VERSION DIR` (signs the update
files, writes `latest.json`), publish the GitHub release with all files, then commit
`Casks/utter.rb` and check the published zip's SHA-256 against it. Then run the README's macOS
steps against the published release on a clean Mac (`packaging/macos/README.md`).

## 10. Testing the in-app updater

Build the current code twice with versions overridden (`--config '{"version":"0.1.11"}'`), the
older one also with `"plugins":{"updater":{"dangerousInsecureTransportProtocol":true}}` (test
builds only: the updater refuses http). Sign the newer one's files with `latest-json.sh VERSION
DIR http://127.0.0.1:8765`, serve the folder (`python3 -m http.server 8765`), start the older
build with `UTTER_UPDATE_URL=http://127.0.0.1:8765/latest.json` (`open --env ... /Applications/
Utter.app`), wait 60 s for the first check, then Update from the menu bar menu or Settings. Check
the new version (`plutil -extract CFBundleShortVersionString raw .../Info.plist`), the signature
(`codesign -dr -`), and that dictation works without a prompt. Afterwards put the real release
back: a machine left on a test version higher than the next real one is never offered it.
