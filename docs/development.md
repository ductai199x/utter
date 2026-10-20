# Developing, testing and releasing Utter

How to set up a development machine, run and test Utter, test it on other operating systems, and
ship a release. Checked against the code at version 0.1.10 (2026-10-20). Architecture and scope are
in [DESIGN.md](../DESIGN.md); what each platform does and why is in [platforms.md](platforms.md);
the eval method and numbers are in [evals.md](evals.md).

## 1. Repo and branches

`main` is the public face of Utter: code, packaging, the README. `dev` is where all work happens
and also carries `docs/`, `DESIGN.md`, `CLAUDE.md`, `.claude/` and `dev/`. Both branches are
readable on GitHub, so neither may hold a path on the maintainer's machine, a private-network
address, an email address or a credential.

| Rule | Detail |
|---|---|
| Daily work | Commit and push on `dev`. CI runs on both branches and on pull requests. |
| Publishing | Only when the maintainer says so: `uv run --no-project python dev/release.py publish --push -m "..."`. It needs a clean tree on `dev` and a clean scan, builds main's next tree from dev's last commit (private paths removed, links to them pointed at the dev branch), scans that tree and the message again, and adds one commit on top of main. It is a tree copy, not a merge. |
| Before publishing | `selftest` (the rules still catch what they should), `scan` (everything the next publish would reject), `fix` (the mechanical rewrites in `dev/patterns.toml`), `publish --dry-run`. |
| Scanner rules | `dev/patterns.toml` is committed and generic. Strings that would leak by being written down go in `dev/patterns.local.toml`, git-ignored: keep a copy in your backups, a fresh clone has only the generic rules. Scope `all` is banned everywhere; scope `main` is fine on dev only. |
| History | `release.py history --as main\|dev` scans every blob and message ever reachable. History was rewritten once, on 2026-10-20 (git-filter-repo). After adding a rule, run `history` first. |
| Releases | Build installers from `main` after a publish, so what ships is what is public. A change to `Casks/utter.rb` goes through dev and a publish like any other. |

The full workflow and the accepted exceptions (the attribution `Tai Duc Nguyen`, the GitHub handle
in URLs, benchmark hardware model names, the `Utter_dev` folder name) are in
[dev/README.md](../dev/README.md).

## 2. Setting up a development machine

### Toolchains

| Need | Version | Where it comes from |
|---|---|---|
| Rust | stable; the workspace sets `rust-version = "1.98"`, edition 2024 | rustup; CI installs `stable` with `clippy` and `rustfmt` |
| Node | 24 (CI); 22 or newer worked locally | for `app/` (Vite 8, Svelte 5, Tauri CLI 2) |
| uv | any recent | for `tools/` (Python 3.12; the project requires `>=3.12`) |
| GPU driver | NVIDIA driver for CUDA; any Vulkan driver otherwise | nothing else: no system CUDA, no Python at run time, no LM Studio |
| Windows | MSVC build tools, WebView2 | Windows 11 ships WebView2 |
| Linux | `build-essential curl libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev` | apt (Ubuntu 24.04 is the package build machine) |
| macOS | Xcode command line tools | `packaging/macos/dev-setup.sh` installs them and the rest (section 4) |

Utter itself needs only the GPU driver. Everything it loads is a pinned, checksummed component in
`crates/utter-runtime/components.json` (onnxruntime, CUDA and cuDNN runtimes, llama.cpp builds
per backend, Silero VAD, the speech and cleanup models), installed by `utter-cli setup`.

### First run

```
cargo run --release -p utter-cli -- setup      # download, verify, install this platform's components
cargo run --release -p utter-cli -- doctor     # data folder, GPU backend, what is installed
```

`setup` installs every component for this OS that is not `optional` and not for another GPU
backend; naming a component id installs that one (the two prompted reference models,
`gemma4-e4b-qat` and `gemma4-26b-a4b-qat`, are optional). A component folder that exists is
complete: downloads are verified by size and SHA-256 in a staging folder and renamed into
`<data dir>/runtime/<id>-<version>/` only when whole. The app runs the same install on its first
run, so a developer needs `setup` only for the CLI and the tools.

### Where data lives

`utter_core::data_dir()` ([crates/utter-core/src/lib.rs](../crates/utter-core/src/lib.rs)) picks the
folder: `UTTER_DATA_DIR` if set, else the platform folder below. A build running from a cargo
`target` folder is a development build and uses the `_dev` sibling, so development never touches an
installed copy's models or settings. The Python tools resolve the same development folder
(`tools/utter_tools/common.py`; on macOS they use the Linux rule, so set `UTTER_DATA_DIR` there).

| | Windows | Linux | macOS |
|---|---|---|---|
| Installed | `%LOCALAPPDATA%\Utter` | `~/.local/share/utter` (or `$XDG_DATA_HOME/utter`) | `~/Library/Application Support/Utter` |
| Development | `%LOCALAPPDATA%\Utter_dev` | `~/.local/share/utter_dev` | `~/Library/Application Support/Utter_dev` |

Inside it:

| What | Path | Notes |
|---|---|---|
| Runtime components, shipped models included | `runtime/<id>-<version>/` | Installed by `utter-cli setup` or the app. |
| Partial downloads | `downloads/` | Resumable. |
| Settings | `settings.toml` | The dictionary, list formatting, microphone, cleanup model. The Home window edits it; a hand edit applies on the next dictation. |
| History | `history.jsonl` | What Home shows. |
| Logs | `logs/utter.log`, `logs/llama-server.log` | Rotated at 5 MB, three rotated files kept (`utter.1.log` ...); the setting `keep_logs` turns them off and deletes them. |
| Dictation log | `dictations/` | One `<id>.json` per live dictation (recognizer passes, Bar state, cleanup input and output, timings) and its `<id>.wav`. Never leaves the machine. |
| Eval sets and results | `eval/<set>/`, `eval/results/` | 16 kHz mono WAVs plus `manifest.jsonl`; results per run in `asr/`, `llm/`, `score/`, `judge/`. See [evals.md](evals.md). |
| Data batches | `data/<batch>/` | `utter-data` working files until a batch is published. |
| Training runs | `train/<run>/` | The data trained on (with its dataset revision), the adapter and the merged model. |
| Local model exports | `models/<name>/` | Output of `utter-speech export`; `utter-cli --model-dir` runs one instead of the installed speech component. |

The Hugging Face cache (`~/.cache/huggingface/hub` on every OS) holds the models the Python tools
evaluate. It is shared with other projects and never committed. Datasets and models are public on
the Hub: `ductai199x/dictation-to-text`, `ductai199x/dictation-speech`,
`ductai199x/utter-parakeet-ultra-onnx`. The gated repos the tools read
(`speechcolab/gigaspeech2`, `linhtran92/viet_bud500`, `CohereLabs/cohere-transcribe-03-2026`) need
the account's terms accepted once.

**Run Utter from a normal terminal, not from inside a packaged app.** The Claude desktop app is an
MSIX package: a folder that a process started from it creates under `%LOCALAPPDATA%` lands in the
package's private store (`AppData\Local\Packages\<package>\LocalCache\Local\`), which only that
package sees. On 2026-10-12 the whole `Utter_dev` folder had been created that way, so
`npx tauri dev` in a normal terminal found it empty and downloaded everything.

### Environment variables

Developer-only knobs; none is read by an installed copy except `UTTER_DATA_DIR` and
`UTTER_GPU_BACKEND`.

| Variable | Effect | Read in |
|---|---|---|
| `UTTER_DATA_DIR` | The data folder. Also read by the Python tools. | `utter-core`, `tools/utter_tools/common.py` |
| `UTTER_GPU_BACKEND` | `cuda`, `vulkan`, `metal` or `cpu`: overrides the detected backend (CUDA if the NVIDIA driver answers, Vulkan otherwise, Metal on macOS). A machine without a usable GPU, such as a VM, runs on the CPU with `cpu`: slow, right for the desktop side only. | `utter-runtime` |
| `UTTER_GPU_INDEX` | Which Vulkan/WebGPU adapter to use, by position in the high-performance order. | `utter-platform`, `utter-asr` |
| `UTTER_NO_GPU_WARM` | Turns the GPU clock warmer off. | `utter-runtime` |
| `UTTER_ORT_DYLIB` | Load a different onnxruntime library, to test another build. | `utter-asr` |
| `UTTER_LLAMA_SERVER` | Run this llama-server instead of the pinned component. | `utter-llm`, tools |
| `UTTER_LLAMA_BUILD` | A llama.cpp source tree for `utter-train`'s GGUF export, on a platform with no llama.cpp release. | `tools/envs/train` |
| `UTTER_CLI` | The `utter-cli` the Python eval calls (default `target/release/utter-cli`): point it at a build made elsewhere. | tools |
| `UTTER_TEST_AUDIO` | Development builds only: a 16 kHz mono WAV every dictation hears instead of the microphone, in real time; an injected Right Ctrl may start one. Nothing is played. With `UTTER_DATA_DIR` set to a folder of its own it never touches the real History or settings. | `utter-pipeline` |
| `UTTER_UPDATE_URL` | A `latest.json` the updater asks instead of GitHub; a development build checks for updates only when this is set. | `app/src-tauri/src/update.rs` |
| `UTTER_TEST_RICHEDIT` | Windows insertion tests: use a RichEdit control. | `utter-platform` |
| `UTTER_LITERT_DYLIB` | Android: the LiteRT library. | `utter-asr` |
| `RUST_LOG` | Log filter (default: `info` for Utter's crates, `warn` for the rest). Logs go to the console in a debug build. | `utter-core` |

### The Python tools

`tools/` is a uv project. `uv sync` in `tools/` installs it. Set `PYTHONUTF8=1` on Windows:
Vietnamese text does not survive the console's default code page.

| Command | Does |
|---|---|
| `utter-data` | Grows the published datasets, a batch at a time: `new`, `passages`, `fix`, `render`, `speak`, `pairs`, `build`, `check`, `upload` (needs `--yes`), `status`. |
| `utter-train` | Fine-tunes a cleanup model on the text dataset at a pinned revision with the shipped recipe (`--name`, `--base e4b\|e2b\|g3-1b\|g3-270m`, `--gpus`), and exports a Q4_0 GGUF (`--gguf utter-<tier>-v<N>-q4_0.gguf`). |
| `utter-eval` | Evaluates cleanup models on the datasets' test splits (`--model NAME=PATH.gguf`, repeatable), with the 26B as judge. |
| `utter-bench` | Speech models on public test sets by word error rate: `fetch`, `run`, `report`. |
| `utter-speech` | Rebuilds the shipped speech model: `export`, `quantize`, `quantize-cpu`, `export-litert`, `check`. |

The two datasets are the standard every command reads and writes (`tools/utter_tools/schema.py`).
They grow only through `utter-data`: a batch is built against the Hub's current revisions, audited
row by row, and uploaded only after a clean audit and an explicit `--yes`.

The main environment (`tools/pyproject.toml`, groups `dev`, `runtime`, `models`, `grammar`, all on by
default) holds the tools, onnxruntime-gpu with the CUDA 13 and cuDNN 9 DLLs from the nvidia wheels
(for the Python side only; the Rust core never loads from it), PyTorch (CUDA 13.0 build) and
Transformers, and the grammar tools. It resolves only for Windows x86-64 and Linux x86-64. Speech
synthesis, training and the LiteRT export pin their own PyTorch and Transformers, so they have
their own uv projects: `tools/envs/tts-kokoro`, `tts-chatterbox`, `train`, `litert`
(`tools/envs/tts` holds the shared `speak.py`). The commands start them with
`uv run --project tools/envs/<name>`; to keep their environments off a small system drive, set
`UV_PROJECT_ENVIRONMENT` per project when running one by hand.

The README's pictures come from the app's own pages: with `npm run dev` serving them on port 1430,
`uv run --with playwright --with pillow python tools/media/record.py` drives the installed Edge,
records `app/promo.html` (the real Bar playing one dictation) into `media/demo.gif` and takes the
windows' screenshots on the pages' stand-in data, light and dark.

## 3. Running and testing

### Run the app

```
cd app
npm ci
npx tauri dev                 # the app, with the Bar hot-reloading (Vite on port 1430)
npx tauri build --no-bundle   # target/release/utter(.exe) without an installer
npm run check                 # svelte-check
npm run build                 # the pages, into app/dist
```

Development builds are optimized (`[profile.dev]` in `Cargo.toml`: `opt-level = 2`, dependencies 3,
no incremental), so timings in `tauri dev` are the product's. A full rebuild takes about 3 minutes,
a change to Utter's crates 20-40 s (measured when the profile was chosen). The app takes no
arguments but `--version` and `--help` (and two internal ones); anything else is refused.

To look at the Bar without the native app, run `npm run dev` in `app/` and open
`http://localhost:1430/bar.html?demo` (`?demo=notice` for the paste and copy notices). Dev builds
only. `.claude/launch.json` starts the same server for the preview pane.

### The CLI

`utter-cli` is the developer command line of the core. It loads the same components as the app, so
run `setup` first. Global option: `--model-dir` (a local speech export instead of the installed
component), plus several GPU-tuning flags for experiments (`--help`).

| Command | Does |
|---|---|
| `setup [ID...]`, `doctor` | Install, and show, the runtime components. |
| `transcribe FILE.wav...` | Transcript and per-stage timings of 16 kHz mono WAVs. |
| `clean "text"` | The cleanup model on its own (`--tier quality\|light\|mini\|reference`), cold and warm timings. |
| `chat --gguf M.gguf ...` | Start Utter's llama-server on a GGUF and send one message. |
| `record OUT.wav` | The microphone, 16 kHz, with pre-roll. |
| `vad FILE.wav` | Where voice detection cuts. |
| `dictate` | Live: hold Right Ctrl, speak, release. Double-tap Right Ctrl for hands-free, Esc cancels, Ctrl+C quits. `--tier`, `--dictionary`, `--no-cleanup`, `--device`. |
| `bench` | The Rust recognizer over the eval sets, results in the format the tools score. |
| `e2e-bench` | Eval clips through the whole pipeline in real time: key up to text ready. |
| `llm-bench` | Cleanup cases through llama-server on a GGUF, with pacing options that mimic a live dictation. |

Benches are sensitive to anything else using the GPU: record `nvidia-smi` utilization next to any
number.

### Checks

What CI runs, and what to run before pushing:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets      # CI sets RUSTFLAGS=-D warnings
cargo test --workspace
cd app && npm run check && npm run build
cd tools && uv run pytest                    # the tools' tests; not run by CI
cd tools && uv run ruff check . && uv run pyright   # dev group
```

The app's pages are built into the binary, so `app/dist` must exist before `app/src-tauri`
compiles: run `npm run build` first on a fresh clone.

**CI** (`.github/workflows/ci.yml`) is the only workflow. On every push to `main` and `dev` and on
every pull request, on `windows-latest`, `ubuntu-24.04` and `macos-latest` (not fail-fast): install
Rust stable, Node 24, the Linux libraries on Ubuntu; `npm ci`, `npm run check`, `npm run build`;
`cargo fmt --check` (Ubuntu only); `cargo clippy --workspace --all-targets`;
`cargo test --workspace`; and on macOS `npx tauri build --bundles app`, then check that the
Info.plist has `NSMicrophoneUsageDescription` and `LSUIElement` and that the bundle is signed
ad hoc. Nothing is uploaded. CI does not build installers, run the Python tests, run any model or
publish a release: releases are manual (section 5).

**What needs hardware.** `cargo test --workspace` needs no GPU, no model and no download: the
tests use fixtures (`crates/utter-asr/tests/fixtures`, `crates/utter-pipeline/tests/fixtures`).
Tests marked `#[ignore]` are run by hand with `cargo test -p <crate> -- --ignored <name>`:

| Test | Needs |
|---|---|
| `utter-runtime`: installed-components check, NVIDIA GPU check, GPU warmer | the components installed; an NVIDIA GPU |
| `utter-platform` `gpu.rs` | reads this machine's graphics cards |
| `utter-platform` `insert.rs`, `focus.rs` | a live Windows desktop: they open windows and use the clipboard and keys. Ask before running them on a machine someone is using. |
| `utter-pipeline` Raw Input test | registers for the keyboard's Raw Input (observes only) |

Everything that measures speed or quality needs the components, a GPU and eval sets: `utter-cli
bench`, `e2e-bench`, `llm-bench`, `utter-eval`, `utter-bench`. Their method, the sets and the
results are in [evals.md](evals.md). Claim nothing is faster or slower without a measurement
(CLAUDE.md).

### The app without speaking

`UTTER_TEST_AUDIO=<16 kHz mono WAV> UTTER_DATA_DIR=<a scratch folder> npx tauri dev` makes every
dictation hear that file instead of the microphone, in real time, and lets an injected Right Ctrl
start one, so a script can hold the key (SendInput on Windows) and check the paste, History and
the Bar. It is ignored by installed copies. Nothing is played.

### Linux builds

Ubuntu 24.04 is the package build machine: the packages' glibc is 2.39, so they run on Ubuntu
24.04 and later, Fedora 40 and later and Debian 13. The hotkey and the paste keys need
`packaging/linux/70-utter.rules` (the .deb installs it and its post-install script reloads udev; the
app's first run offers to install it through the desktop's password dialog).

```
sh packaging/linux/build.sh      # .deb and AppImage into target/release/bundle
```

## 4. Testing on VMs and a rented Mac

Windows and Linux are tested in throwaway VMware VMs; the `vm-testing` skill (Claude Code) covers
creating them unattended, installing, driving the GUI, snapshots and cleanup. A small VM has no
usable GPU: run with `UTTER_GPU_BACKEND=cpu`. C: space is limited on the maintainer's PC, so use
small VMs, one or two at a time, and delete each right after its run.

macOS has no VM option (Apple's licence), so Utter is built and tested on a rented Apple Silicon
Mac, driven over SSH from the Windows PC. This was done for real on 2026-10-19/20, on macOS 26.7
(Tahoe) and 27.0.1 (Golden Gate), on a Mac mini M2 with 16 GB. The generic remote-Mac tooling is the
`vm-testing` skill (`scripts/mac.py`, `references/remote-mac.md`). What macOS does and why the code
is the way it is: [platforms.md](platforms.md).

### The machine

- **Scaleway Apple silicon** (Bare Metal > Apple silicon): a Mac mini M2, 16 GB, 256 GB. Slower than
  an M4, which makes it the better test machine. **24-hour minimum** (Apple's licence), so plan a
  session as a day. The maintainer creates the account, pays, and deletes the machine when done.
- The Overview page gives the SSH user (`m1`), the IP, and screen sharing: port, user and password.
  The maintainer's key is registered at creation; Scaleway installs registered keys in
  `~/.ssh/scw_authorized_keys`, not `authorized_keys`.
- **A reinstall from the Scaleway console wipes everything** (the key, build tools, signing keychain,
  sudoers) and takes about 2 hours. Upgrading macOS 26 to 27 that way was a fresh install: SSH then
  warned "REMOTE HOST IDENTIFICATION HAS CHANGED". Never accept a changed host key blindly: read the
  fingerprint on the Mac (`ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`), compare, then
  `ssh-keygen -R <ip>`.
- FileVault must stay off (it would lock SSH out at boot). Automatic login is off on Scaleway's
  image, so after every reboot the maintainer logs in once over screen sharing.

### What only a person can do

Claude never types passwords, so these are the maintainer's, each a minute or two:

| When | What | Why it cannot be scripted |
|---|---|---|
| Once per machine | Log in to the desktop over screen sharing | the account password |
| Once per machine | `echo "m1 ALL=(ALL) NOPASSWD: ALL" \| sudo tee /etc/sudoers.d/m1-nopasswd` in Terminal | the password; afterwards Claude installs Homebrew, BlackHole etc. over SSH |
| Once per machine | Add `/usr/libexec/sshd-keygen-wrapper` to **Accessibility** (macOS 27: **Device Control and Data Access**) and **Screen & System Audio Recording** | macOS ignores synthetic clicks on privacy switches |
| Each first-run test | Turn on Utter's keyboard switch in System Settings and confirm with the password | the same; this is the first-run test |
| After each reboot | Log in at the login window | automatic login is off |
| macOS upgrade | Start it (Software Update, or the Scaleway console) | the password |

**Screen sharing client:** RealVNC Viewer failed against macOS ("Protocol error: key length too
large"; with its encryption off, "invalid security result": plain VNC auth, which macOS refuses).
**Devolutions Remote Desktop Manager** with the **Apple Remote Desktop (ARD)** type works (host,
port, user, password). The port answered only after a reboot once; if it does not connect, check it
with `Test-NetConnection <ip> -Port <port>` before blaming the client.

### Setting up a fresh Mac

1. The maintainer: log in, run the sudoers line, add sshd-keygen-wrapper to the two privacy lists.
2. Claude, with a dedicated SSH key (`~/.ssh/scaleway-mac`, added through the maintainer's key if
   needed): `mac.py connect <ip> --user m1`, then `mac.py doctor` until screen recording and cliclick
   say ok.
3. `bash ~/utter/packaging/macos/dev-setup.sh` (or fetch it): Xcode command line tools if missing
   (Scaleway's image ships full Xcode), Homebrew, Node 24, uv, cliclick, SwitchAudioSource,
   BlackHole 2ch, Rust, and the repository clone. It only fills in what is missing. Notes:
   - Fetch the script from the cloned repo, not raw.githubusercontent.com right after a push: its
     cache served the old version for minutes.
   - With passwordless sudo, `sudo -v` still asks for a password over SSH; the script uses
     `sudo -n true || sudo -v`.
   - BlackHole says "You must reboot"; `sudo killall coreaudiod` is enough.
4. The signing identity (section 5): copy `~/.utter-signing/` to `~/signing`, create a keychain
   `utter-signing`, import `utter.p12`, `set-key-partition-list`, add it to the search list.
   `security find-identity` shows it as `CSSMERR_TP_NOT_TRUSTED`: normal for a self-made
   certificate, and codesign uses it all the same.

### Driving the Mac (mac.py)

- `exec` runs a zsh login shell (Homebrew and cargo on PATH); `bg LABEL -- cmd` keeps a build running
  with its log in `~/logs/LABEL.log`; poll for a marker line rather than sleeping.
- In Git Bash always `export MSYS_NO_PATHCONV=1`, or `/tmp/x` and `~/x` arrive as Windows paths; call
  mac.py by its `C:/...` path (with path conversion off, `$HOME/...` breaks).
- `shot` gives screen points (1920x1080 on that Mac); click coordinates are those pixels.
- **Clicks and keys:** `cliclick` for clicks. Typing into Utter's own windows (a web view) does not
  work with cliclick; System Events' `keystroke` does, but needs an Automation permission prompt for
  the calling process the first time. Keep it out of anything that must not block.
- **The dictation key:** `hold 61 SECONDS` (Right Option; 54 Right Command) posts real flagsChanged
  events with the device bit, which Utter's event tap needs; cliclick's `kd:alt` cannot tell left from
  right. `hold 53 0.05` is a real Esc; cliclick's `kp:esc` did not reach the tap. `hold KEYCODE S
  FLAGS` presses a shortcut (`hold 9 0.05 140000` is Ctrl+Cmd+V).
- **Scrolling:** `scroll X Y LINES` (a real wheel event); Page Down needs keyboard focus that a web
  view may not have.
- **Prompts that block a command:** asking Finder or System Events anything raises an Automation
  prompt and the command hangs until someone answers (the first `shot` hung on Finder; mac.py now
  reads the screen width from AppKit). Decline such prompts; they are about the test tool.
- **Menu bar extras:** clicking Utter's menu bar icon works, but after many synthetic clicks the menu
  bar stopped opening any menu (Spotlight too) for the rest of that session: a stuck input state,
  not Utter. Notification stacks also cover the top right; close them first.
- `/tmp` is cleared on reboot (the compiled hold tool goes with it; mac.py rebuilds it).

### Testing Utter like a user

**Install and open.** `brew tap ductai199x/utter https://github.com/ductai199x/utter`, `brew trust
--cask ductai199x/utter/utter` (Homebrew 7 refuses a third-party tap's cask until trusted; found
only by testing the published release), `brew install --cask utter`. Open from Finder >
Applications (double-click) or Spotlight. Before a release, test the cask from a local tap
(`brew tap-new --no-git ductai199x/local`, copy the generated `Casks/utter.rb`) with a `file://` URL
(`packaging/macos/cask.sh ... file://$HOME/dist`; do not commit that `Casks/utter.rb`). A Homebrew
install from scratch was verified by uninstalling Homebrew and running the official installer: it
ends with **Next steps**, three lines that put `brew` on PATH (the README repeats them).

**A clean first run.** `brew uninstall --cask --zap utter`, `tccutil reset All app.utter`, delete
`~/Library/Application Support/Utter` and `~/Library/LaunchAgents/app.utter.plist`. Caution: after
`tccutil reset Accessibility app.utter` macOS does not prompt again; Utter disappears from the list
and has to be added by hand (+, Cmd+Shift+G, `/Applications/Utter.app`). Only reset for a true first
run, never in the middle of one.

**The microphone.** `SwitchAudioSource -t output -s "BlackHole 2ch"` once; then `afplay clip.wav` or
`say -r 165 "sentence"` is what Utter hears (the sound stays inside the Mac). Dictation:
`(sleep 0.4; say -r 165 "...") & /tmp/mac-py-hold-<hash> 61 6.5`. Clips from the eval set
(`Utter_dev/eval/chatterbox/*.wav`) and `say` both worked. To test "no microphone", move
`/Library/Audio/Plug-Ins/HAL/BlackHole2ch.driver` away and restart coreaudiod.

**Checking results.** TextEdit as the target (`open -a TextEdit`, read it back with `osascript -e
'tell application "TextEdit" to get text of document 1'`); Utter's log
(`~/Library/Application Support/Utter/logs/utter.log`: `dictation:` lines carry asr, cleanup, insert
and total ms; `ready at`); `history.jsonl`; process counts (`ps -axo command | grep -cE
"[U]tter.app/Contents/MacOS|[l]lama-server"`) around quit and `kill -9`.

**Lifecycle checks that found bugs** (each is worth repeating after macOS changes):

- first run: Allow, macOS's prompt, the switch, Utter restarting itself, microphone prompt, level
  bars, download, Try it, Finish;
- offline (`pfctl` blocking ports 80/443) and a connection cut mid-download, then Retry;
- update over the installed copy (permissions must survive: section 5, signing);
- quit, `kill -9`, reopen while running (a "reopen" event, not a second process), a second process
  right after a sign-in start (macOS's session restore);
- reboot: start at login, no window, permissions kept;
- every Settings row, the paste rules window, Dictionary import (native file picker: Cmd+Shift+G),
  History (copy, favorite, search, delete, Undo), retention (age entries with a script while Utter is
  quit), Clear history, a broken `settings.toml`;
- no microphone at start, microphone back while running;
- uninstall and `--zap`.

### Gotchas

The Mac-generic ones (SSH keys, screen sharing, cliclick, prompts, BlackHole) are in the
`vm-testing` skill's `references/gotchas.md`, "A remote Mac". Utter-specific:

| Symptom | Cause | Fix |
|---|---|---|
| "Could not start: EP library is already registered under WebGpuExecutionProvider" | onnxruntime's Mac build has WebGPU built in | the Mac downloads no WebGPU plugin (`runtime_for`, `utter-asr` runtime) |
| llama-server: `Library not loaded: @rpath/libggml-rpc.0.dylib`, then "Some of Utter's files are damaged" | the component's extract list left it out | `**/libggml-rpc*.dylib` in components.json |
| the microphone step stays at "Nothing heard yet", no prompt | the audio-input entitlement was missing | `Utter.entitlements` |
| setup froze for 27 s at the microphone step | a sync Tauri command opened the microphone on the UI thread while macOS asked | async commands for anything that can wait on the person |
| "Nothing heard yet" shows during macOS's microphone question | the timer started before the microphone opened | it starts once the microphone is open |
| Allow added Utter to neither Accessibility nor Input Monitoring | asking for both at once | ask for Accessibility only; it covers reading keys |
| keyboard access stays "Not allowed" after the switch is on | macOS applies it to a new process only | Utter restarts itself when the switch turns on |
| macOS's permission prompt stays on screen after the switch is on | Utter opened the pane on top of macOS's own prompt | the first Allow leaves it to the prompt |
| quitting Utter leaves llama-server (4 GB) running | no parent-death signal on macOS | a watcher shell ends it within seconds |
| switching models kills the new server, the old one comes back | the leftover sweep ended this Utter's own servers | sweep only servers whose parent is not this Utter |
| opening Utter while it runs shows nothing | macOS sends the running copy a reopen event | handle `RunEvent::Reopen` |
| a window opens at every sign-in | macOS's session restore starts a second copy beside the login item | a plain start within 30 s of a sign-in start shows nothing |
| "Utter can run in the background" at every start | the LaunchAgent was rewritten each start | write it only when it changes |
| Utter could not start with no microphone at all | macOS names a default input that no longer exists ("bad object") | treat it as no microphone |
| the download bar froze at 0 for 108 s offline | 60 s connect timeout, tried twice | 15 s to connect |
| Esc, paste-last and copy-last "did nothing" in a test | cliclick's events never reach Utter's event tap | test with `mac.py hold` (real CGEvents) before calling it a bug |
| tests show Utter 0.1.9 while the build is 0.1.11 | the version came from Cargo, not the app | the app's package version |

## 5. Releasing

There is no release workflow: a release is built by hand on each OS, signed by script, and
uploaded to a GitHub release. What the scripts do is below; the steps that are manual are marked.

### Version bump (manual)

The version appears in four files, which must agree:

| File | Field |
|---|---|
| `Cargo.toml` | `[workspace.package] version` (every crate inherits it; `Cargo.lock` follows) |
| `app/package.json` | `version` |
| `app/src-tauri/tauri.conf.json` | `version` (the installer and the updater compare this one) |
| `Casks/utter.rb` | rewritten by `cask.sh` (below), never by hand |

`app/package-lock.json` still says 0.1.7 and nothing reads it. `packaging/linux/build.sh` takes the
version from `Cargo.toml`. Commit on `dev`, run the checks (section 3), publish to `main`
(section 1), and build from `main`.

### Build the installers (manual, one per OS)

| OS | Build | Output |
|---|---|---|
| Windows | `cd app && npm ci && npx tauri build` | `target/release/bundle/nsis/Utter_VERSION_x64-setup.exe` (NSIS, per-user install; no Authenticode signing is configured) |
| Linux | `sh packaging/linux/build.sh` on Ubuntu 24.04 | `target/release/bundle/deb/Utter_VERSION_amd64.deb` and `appimage/Utter_VERSION_amd64.AppImage` |
| macOS | on an Apple Silicon Mac: `cd app && npm ci && npm run build && npx tauri build --bundles app`, then `sign.sh`, then `cask.sh` | `Utter-VERSION-macos-arm64.zip` and `.app.tar.gz` |

Linux details: `build.sh` strips debug info from the binary (the line tables are 70 MB on Linux),
then `fix-appimage.sh` repacks the AppImage without the build machine's libwayland and libdrm
(they must come from the system's Mesa; with older ones inside, WebKit could not make its EGL
display under Fedora 44) and swaps Tauri's runtime for AppImage's static type2 runtime (pinned by
URL and SHA-256), because Tauri's needs `libfuse.so.2`, which Ubuntu 24.04 and later do not
install. The .deb depends on `libasound2` and installs the udev rule with a post-install script.

macOS signing (`packaging/macos/sign.sh APP`): `codesign` with the hardened runtime, the entitlements
in `app/src-tauri/macos/Utter.entitlements` (audio input; disable library validation, because the
speech runtime is a downloaded library) and Utter's own certificate, "Utter Homebrew Signing"
(SHA-1 `E411284D187F5D405F62FCB184A7985F7DB275C2`, valid to 2036). Environment:
`UTTER_SIGNING_IDENTITY` (the SHA-1), `UTTER_SIGNING_KEYCHAIN`, `UTTER_SIGNING_KEYCHAIN_PASS`.
Why a self-made certificate and not Developer ID or ad hoc: macOS ties Accessibility and Microphone
grants to the app's designated requirement; ad hoc makes that the exact build, so every update lost
the keyboard permission while System Settings still showed it on. With one certificate the
requirement is "app.utter signed by this certificate" and grants survived updates, reinstalls and the
in-app updater (measured on 26.7 and 27.0.1). Every release must use this certificate. Its key
(`utter.p12` and the PEM files) lives outside the repository, in `~/.utter-signing/` and an encrypted
backup; losing it means every user grants the permissions again.

`packaging/macos/cask.sh APP VERSION OUT_DIR [URL_BASE]` writes the zip a release carries, the
`.app.tar.gz` the in-app updater installs, and `Casks/utter.rb` filled from the template
`packaging/macos/utter.rb` (version, the zip's SHA-256, the download URL). The cask requires macOS
Sonoma or later on arm64, removes the quarantine attribute after install (the app is not
notarized), and declares `auto_updates true`.

### Sign the update files and write latest.json

Collect `Utter_VERSION_x64-setup.exe`, `Utter_VERSION_amd64.AppImage`, `Utter_VERSION_amd64.deb` and
`Utter-VERSION-macos-arm64.app.tar.gz` in one folder (copy the Linux and Mac files to the PC). Add
`notes.md` if wanted: its first line becomes the update's description. Then, in Git Bash:

```
packaging/latest-json.sh VERSION DIR
```

It signs each file with the updater key (`npx tauri signer sign`, writing `<file>.sig`) and writes
`DIR/latest.json`. The key is `~/.utter-signing/updater.key` with its password in `updater.pass`
(override with `UTTER_UPDATER_KEY`, `UTTER_UPDATER_KEY_PASS`); the matching public key is in
`app/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). The updater asks for
`{os}-{arch}-{installer}` and then `{os}-{arch}`: each Linux install is offered only its own kind (an
AppImage never gets the .deb), Windows and macOS have one file each. An optional third argument
replaces the download base URL (section "Testing the in-app updater").

### Publish (manual)

1. Create the GitHub release `vVERSION` and attach: the NSIS installer, the AppImage, the .deb, the
   macOS zip, the `.app.tar.gz`, every `.sig`, and `latest.json`. The app checks
   `https://github.com/ductai199x/utter/releases/latest/download/latest.json`, so the release must be
   the latest, not a pre-release or draft.
2. Only after the release is public, commit `Casks/utter.rb` (its URL must work when it lands) on
   `dev` and publish to `main` like any change. Check the published zip's SHA-256 against the cask.
3. Run the README's install steps against the published release on a clean machine of each OS
   (section 4; `packaging/macos/README.md` for the Mac). Test the installed first run: v0.1.0's was
   broken.

### Testing the in-app updater

Build the current code twice with versions overridden (`--config '{"version":"0.1.11"}'`), the older
one also with `"plugins":{"updater":{"dangerousInsecureTransportProtocol":true}}` (test builds only:
the updater refuses http). Sign the newer one's files with `latest-json.sh VERSION DIR
http://127.0.0.1:8765`, serve the folder (`python3 -m http.server 8765`), and start the older build
with `UTTER_UPDATE_URL=http://127.0.0.1:8765/latest.json` (on macOS `open --env ...
/Applications/Utter.app`). The first check runs 60 s after Utter is ready; then Update from the menu
bar (tray) menu or Settings. Check the new version (`plutil -extract CFBundleShortVersionString raw
.../Info.plist` on macOS), the signature (`codesign -dr -`), and that dictation works without a
permission prompt. Afterwards put the real release back: a machine left on a test version higher than
the next real one is never offered it. The signature is checked even against a test server; nothing
is downloaded or installed until the person clicks Update.

## 6. Where things are

| Path | What |
|---|---|
| `crates/utter-core` | Data folder resolution (`data_dir`, development vs installed), log files and rotation, tracing setup |
| `crates/utter-platform` | Every OS call, behind functions with one module per OS: hotkey, text insertion, focus, caret, overlay, autostart, GPU/CPU/memory/disk probes, Linux input and accessibility, macOS |
| `crates/utter-runtime` | The pinned components (`components.json`), download, verify, install; GPU backend choice (CUDA, Vulkan, Metal, CPU); the GPU clock warmer |
| `crates/utter-audio` | Microphone capture with pre-roll history, resampling |
| `crates/utter-asr` | Speech recognition: the Parakeet TDT decoder on onnxruntime, features, Silero VAD, phrase boosting (the user's dictionary), LiteRT for Android |
| `crates/utter-llm` | The cleanup model: Utter's own llama-server child process, the cleanup prompts (`prompts/`), tiers |
| `crates/utter-pipeline` | The dictation session: segments at pauses, the cleanup gate, cleanup, insertion; settings, history, the dictation log, runtime choice |
| `crates/utter-mobile` | The core on Android behind a few JNI calls; `examples/replay.rs` runs it without a phone |
| `crates/utter-cli` | `utter-cli`, the developer command line (section 3) |
| `app/` | The UI: Svelte pages (`bar`, `home`, `setup`, `splash`, `rules`, `promo`, `brand`; sources in `app/src`, built to `app/dist`) |
| `app/src-tauri` | The Tauri 2 shell: windows, tray, commands, setup flow, updater (`update.rs`), `tauri*.conf.json`, macOS entitlements and Info.plist, Windows NSIS hooks |
| `tools/` | Python (uv), never shipped: `utter_tools/` (the five commands, `data/`, `speech/`, `prompts/`, `dictionaries/`, `templates/`), `envs/` (TTS, training, LiteRT projects), `media/record.py`, `tests/` |
| `packaging/` | `linux/` (build, AppImage fix, udev rule, deb scripts), `macos/` (sign, cask, dev-setup, cask template, README), `latest-json.sh` |
| `Casks/utter.rb` | The Homebrew cask; the repository is its own tap. Written by `cask.sh` |
| `android/` | The Android app (Kotlin, Gradle) and `native/build.sh`, which builds the Rust core and llama-server for it. See [android.md](android.md) |
| `dev/` | The dev/main workflow: `release.py`, `patterns.toml` (and the git-ignored `patterns.local.toml`) |
| `docs/`, `DESIGN.md` | Documentation, indexed in [docs/README.md](README.md) |
| `media/` | The README's pictures (made by `tools/media/record.py`) |
| `.github/workflows/ci.yml` | CI |
| `.claude/launch.json` | Preview-pane entry that serves the UI pages |
