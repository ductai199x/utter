# Plan: Utter on Linux and macOS

Status: **plan, nothing built** (2026-10-16). DESIGN.md sections 2 and 7 set the order (Linux,
GNOME first, then macOS) and the platform traits; this plan is what the code needs, from an
inventory of the tree at a267822, and what is still unknown and must be measured first.

## Where the code is

- **The core already compiles on Linux.** `cargo check --workspace --all-targets` on Ubuntu
  24.04 (2026-10-16): one error, in the app shell (`app/src-tauri/src/main.rs:96`,
  `window.hwnd()`, Windows-only in Tauri). Every Windows call in `utter-platform` is behind
  `cfg(windows)` with a non-Windows stub, and the pure logic (the hotkey's press-and-tap machine,
  the session, segmenting, cleanup, the gate) is shared, as Android proved.
- **The stubs** decide what a Linux or macOS build does today:

  | Module | Windows | Elsewhere today | Effect |
  |---|---|---|---|
  | `hotkey` | Raw Input | error | `Pipeline::run` fails: no dictation |
  | `gpu` | DXGI | no adapter | speech, cleanup and setup refuse: "no graphics card" |
  | `insert` | clipboard + `SendInput` | error | nothing goes in, not even a copy |
  | `focus` | foreground window, elevation | none | every dictation is "no window" |
  | `caret` | UI Automation | none | no spacing, no "did it land" check |
  | `overlay` | `WS_EX_NOACTIVATE`, topmost, DPI | no-op | the Bar is not placed |
  | `shortcut` | `RegisterHotKey` | error (logged) | no paste last / copy last keys |
  | `autostart` | Run key | error (logged) | no start at login |
  | `disk` | `GetDiskFreeSpaceExW` | none | the disk check always passes |
  | `process` | job object | error (logged) | llama-server can outlive a crash |
  | `shell`, `console` | real | real | fine |

- **Runtime components are Windows x86_64 only** (`crates/utter-runtime/components.json`): on
  Linux and macOS `component()` fails for every runtime library; the models (`os: any`) resolve.
  Extraction reads zip only and sets no executable bit. Library loading prepends `PATH`, which
  works for DLLs only: on Linux `LD_LIBRARY_PATH` is read at process start (`runtime.rs:108`), and
  nothing sets it for llama-server's `libggml*.so`. onnxruntime's name has no `.dylib` case.
- **Packaging:** NSIS only; no CI at all (no `.github/`), although DESIGN.md says macOS is
  compile-checked in CI; no updater, no signing.
- **The UI says Windows** in places: "Start with Windows", Ctrl+V and Alt+Shift+Z wording,
  `e.ctrlKey` shortcuts (no Cmd), paste rules keyed by `.exe`, NVIDIA driver 580 text, Win32
  error strings matched in `problem.rs`.

## The runtimes exist for both, at our pinned versions

Checked 2026-10-16 against the releases Utter already pins:

| | Linux x86_64 | macOS arm64 |
|---|---|---|
| llama.cpp b11535 | `ubuntu-cuda-13.4-x64` (+ cudart), `ubuntu-vulkan-x64`, `ubuntu-x64` (CPU) | `macos-arm64` (Metal) |
| onnxruntime 1.31.0 | `linux-x64-gpu_cuda13`, `linux-x64` | `osx-arm64` (CoreML provider built in) |
| WebGPU plugin 0.4.0 | `manylinux_2_28_x86_64` wheel (Dawn on Vulkan) | `macosx_14_0_universal2` wheel (Dawn on Metal) |
| cuDNN 9 | `nvidia-cudnn-cu13` manylinux wheel | - |

No source builds are needed. Two consequences: the backends map cleanly (NVIDIA on Linux:
CUDA, as on Windows; AMD and Intel on Linux: Vulkan and WebGPU, as on Windows; Apple Silicon:
Metal for cleanup, and for speech either WebGPU over Metal, which Utter already runs, or CoreML,
to be measured); and **Intel Macs are out** unless onnxruntime is built from source (no
`osx-x64` 1.31 build is published).

## What is known from the chess project (same machine, same Tauri, Linux done there)

docs/app.md in the sibling project, checked in VMware VMs (Ubuntu 22.04, 24.04, 26.04,
Fedora 44), 2026-10-13/14:
- **A held key on Wayland: evdev**, with a udev rule (logind `uaccess`, the user at the desktop)
  that the .deb installs and the AppImage offers once. Works on Wayland and X11 alike, tells
  Left and Right apart, finds keyboards plugged in later. The GlobalShortcuts portal cannot bind
  a lone modifier such as Right Ctrl.
- **Overlay windows on GNOME Wayland: XWayland** (`GDK_BACKEND=x11,wayland`, WebKitGTK's DMA-BUF
  renderer off): Mutter gives Wayland clients no way to stay on top or place themselves.
- **Packaging:** build on Ubuntu 24.04 (glibc 2.39: Ubuntu 24.04+, Fedora 40+, Debian 13, Mint
  22); AppImage with the static type2 runtime (no FUSE 2), libwayland and libdrm taken from the
  system (22.04's broke WebKit on Fedora's Mesa); a .deb; the desktop entry written by the
  AppImage; no tray on plain GNOME (no StatusNotifierWatcher), so close-to-tray falls back;
  raise windows with the X server's time or GNOME treats it as focus stealing.
- **The VM rig:** an Ubuntu cloud image configured by cloud-init through VMware guestinfo, key
  SSH, `vmrun captureScreen` for screenshots, input through a uinput device, commands run in the
  logged-in GNOME session through `systemd-run --user`.

## Unknowns to measure first (Linux spikes, in a GNOME VM, Wayland and Xorg)

1. **Insertion on GNOME Wayland.** Wayland lets a client set the clipboard only with keyboard
   focus, and the Bar never takes it. Candidates, to try in this order:
   a. the clipboard set from an XWayland window (Mutter bridges X selections to Wayland) and
      Ctrl+V sent through uinput (kernel level: reaches Wayland and X apps alike; the same udev
      rule grants it);
   b. the RemoteDesktop portal (keys) with its Clipboard interface (GNOME 45+), one permission
      dialog, remembered with a restore token;
   c. typing through uinput (layout-dependent: ASCII only, a fallback at most).
   Then the clipboard restore, and terminals' Ctrl+Shift+V (the paste rules).
2. **The Bar under XWayland**: keep-above, never focused, click-through, placed bottom centre of
   the monitor with the focused window, at the right scale on HiDPI.
3. **Focus and caret through AT-SPI**: the focused application (name, pid) for the paste rules
   and "never types into the wrong window", and the text before the cursor, in GTK, Qt, Firefox,
   Chromium and Electron apps (Chromium exposes AT-SPI only when an assistive tool asks for it).
4. **llama-server tied to Utter**: `PR_SET_PDEATHSIG` fires when the *thread* that spawned the
   child exits, and Utter starts the server from short-lived threads; so spawn from one
   long-lived thread, and sweep stale servers from a pid file at start as a backstop (macOS has
   no PDEATHSIG at all).

macOS has its own unknowns (speech on CoreML vs WebGPU over Metal; the Bar as a non-activating
NSPanel; which key to hold: Mac keyboards have Right Command and Right Option, and Fn/Globe is
what Wispr Flow uses), but none can be measured without a Mac (below).

## Spike results (2026-10-16, Ubuntu 24.04, GNOME 46, Wayland, a VMware VM)

1. **Insertion works, with no permission dialog.** The clipboard set by a background process,
   then a virtual Ctrl+V from a kernel uinput keyboard, put text into a native Wayland editor
   (GNOME Text Editor), accents and symbols intact, the editor keeping the focus throughout.
   Both setters worked: an X11 client through XWayland (`xclip`) and a Wayland client
   (`wl-copy`); Mutter bridges the two clipboards in both directions. Typing through uinput also
   reaches a Wayland client. So insertion is the Windows recipe (clipboard, Ctrl+V, restore)
   with a different keyboard: no RemoteDesktop portal needed. It needs `/dev/uinput`, which the
   same udev `uaccess` rule that grants the hotkey's `/dev/input` grants.
   Still to check: that `wl-copy` never flashes a focus-taking surface (the XWayland setter is
   the safe default), the clipboard restore, terminals' Ctrl+Shift+V, Xorg and KDE.
3. **AT-SPI names the focused app and reads the caret.** One walk of the accessibility tree at
   start (the active window, then the object with the focused state) and the
   `object:state-changed:focused` events after it give the focused object; the bus's
   `GetConnectionUnixProcessID` gives its pid; the Text interface's `CaretOffset` property and
   `GetText` give the text before the cursor, exactly. The program name comes from
   `/proc/PID/exe`, not `comm`, which the kernel cuts at 15 characters (`gnome-terminal-`).
   GNOME Text Editor checked; Firefox, Chromium, Electron and Qt still to check.
4. **PDEATHSIG from one long-lived spawner thread works:** a child started through
   `process::spawn_tied` died with its parent.

The platform layer (`crates/utter-platform`, `examples/probe.rs`) passes all of these in the
VM: the hold-to-talk key (Right Ctrl held starts and finishes, tapped cancels) read from evdev
under Wayland, the paste through Utter's own uinput keyboard into the focused editor, the
clipboard restored after it.

## Progress (2026-10-18)

**Linux, done:** the platform layer (above); the Bar placed by Tauri under XWayland (the app
sets `GDK_BACKEND=x11,wayland` and turns WebKit's DMA-BUF renderer off unless the person set
them); the paste keys made ready at start (`insert::warm`); first run's "Keyboard access" check
with Allow, which installs the udev rule through pkexec and works without logging out (a
refused permission is no longer cached for the session); the same as a problem screen when
dictation cannot start; "Start at login" (the AppImage's own path, not its mount); Quit in
Settings where the desktop shows no tray icons (GNOME without an AppIndicator extension);
programs named by their desktop entries; Linux library errors told apart (the driver's
`libcuda.so` from Utter's own); the .deb (the udev rule, applied at install) and the AppImage
(static runtime, the system's libwayland and libdrm), built by `packaging/linux/build.sh`.

**Checked in the VM** (Ubuntu 24.04, GNOME 46, Wayland, the .deb, the CPU backend, the test
user out of the `input` group): keyboard access from the udev rule alone; first run through
check, model, microphone (a PipeWire virtual source fed an eval clip), download and Try it;
the Bar bottom centre with live words; the dictation pasted into the setup window and into
GNOME Terminal (Ctrl+Shift+V), 1.9-2.4 s from key up on the VM's CPU; with the rule removed,
the problem screen, Allow, the desktop's dialog naming `utter-keyboard-access`, Cancel quiet,
and after authorizing, dictation working without a logout. Found and fixed there: the app
aborted at start (click-through set on a never-shown window), and the dialog showed the raw
shell commands. Then on Xorg (GDM with Wayland off), the AppImage with no FUSE 2 installed:
it starts (the static runtime), the Bar shows its words, the dictation lands in GNOME Terminal
(2.7 s on the CPU).

Ubuntu 26.04 (packages built there): GNOME 50, Wayland: first run, the Bar, the dictation into
Ptyxis (its paste rule) and Text Editor, the tray menu, Quit leaving no process. KDE Plasma 6,
Wayland: "Start at login" started Utter at sign-in, the tray icon, and the dictation into
Konsole after two fixes found there: Qt apps join the accessibility bus only while the
session's accessibility switch is on, which Plasma leaves off, so Konsole was unknown and got
Ctrl+V (Utter now turns the switch on at start and back off at quit if it turned it on); and
KWin made the Bar the active window (GTK 3 offers WM_TAKE_FOCUS whatever the window's focus
setting), so the paste keys went to the Bar (the Bar is now a notification-type window, which
KWin never activates, and unfocusable in its config). Once, before those fixes, Utter exited
silently mid-dictation on Plasma; not seen again in four fresh starts after them.
Not yet: Fedora, AMD or Intel graphics (Vulkan) on Linux.

**Linux on a real GPU** (WSL 2, Ubuntu 24.04, this PC's RTX 3090 Ti, `utter-cli e2e-bench`,
Quality, 20 clips each of en-ami and en-voxpopuli, the same command on Windows):

| | key up -> text ready p50 | p95 | speech text | final text WER vs reference |
|---|---|---|---|---|
| Windows | 199 ms | 345 ms | (baseline) | 8.0% |
| Linux | 197 ms | 393 ms | identical, 40 of 40 | 7.0% |

Each OS repeats itself exactly; 13 of the 40 cleaned texts differ between them (the two CUDA
builds of llama.cpp round differently, and greedy decoding follows), with no loss of quality.
Found and fixed there: Linux's onnxruntime CUDA provider would not load without cuRAND (a
component now), and every exit after a CUDA session aborted with heap corruption, from the ort
crate releasing its environment after onnxruntime's own teardown (`skip_teardown_at_exit`).

**Without a graphics card** (Tai, 2026-10-18): Utter runs on the processor, picked by itself,
with Light or **Extra light**, the phone's Gemma 3 1B (`utter-mini-v1-q4_0.gguf`, published in
ductai199x/utter under the Gemma terms), recommended there; Quality is grayed. Settings'
"Runs on" lists CUDA, Vulkan and the processor as the computer has them. Measured on this PC's
processor (`utter-cli e2e-bench`, 40 clips): Extra light p50 1.85 s / p95 2.67 s, Light
2.38 / 3.71 s; speech recognition is most of it (1.55-1.64 s). In an Ubuntu VM: first run with
Extra light recommended, dictations at 2.0-2.1 s (cleanup 224-526 ms), Quit, relaunch ready in
6.6 s, nothing left running.

**macOS, written and compile-checked only** (`crates/utter-platform/src/macos.rs`): one
listen-only event tap for the hotkey and the shortcuts (Ctrl+Cmd+V, Ctrl+Cmd+C), the
pasteboard saved whole and restored, Cmd+V and typing as CoreGraphics key events, the
frontmost app, the caret through Accessibility, Metal's device; first run's check asks for
Input Monitoring and Accessibility and opens the right pane of System Settings; Right Option
is the default key (most Mac keyboards have no Right Control). Still to do on a Mac: run it,
the Bar as a non-activating panel, the Homebrew formula, speech on CoreML vs WebGPU.

**CI:** `.github/workflows/ci.yml`, Windows, Ubuntu 24.04 and macOS: the pages, format, clippy
with warnings as errors, the tests.

## The work, in order

**0. Foundations, every OS (testable now):**
- CI on GitHub Actions: Windows, Ubuntu 24.04 and macOS (arm64): build, clippy, unit tests, the
  app's compile. Free for a public repository.
- The app shell compiles everywhere (`hwnd()` behind the platform layer, which takes the window
  rather than an HWND).
- Components: tar.gz extraction, the executable bit, Linux and macOS entries (the table above);
  libraries found without the environment (preloaded in order on Linux, `@rpath` beside the
  binary on macOS), and llama-server's own `LD_LIBRARY_PATH` / its folder.
- GPU discovery off Windows: Vulkan's device list (name, memory) on Linux, Metal's
  (recommended working set) on macOS; the backend gains Metal.
- **Parity, measured:** `utter-cli e2e-bench` on Linux with CUDA (FLAME's A6000s or this PC's
  3090 Ti) gives the Windows transcripts and final texts, within the latency budget; on macOS CI
  it builds and runs on the CPU.

**1. Linux desktop (GNOME first, Wayland and X11 alike, as Tai asked of the chess app):**
the four spikes above, then the platform traits (`hotkey` and the paste-last keys from one
evdev listener, with an X11 fallback; `insert`; `focus` and `caret` through AT-SPI and X11;
`overlay`; `autostart` as `~/.config/autostart/utter.desktop`; `disk` with `statvfs`;
`process`), the first run saying what it needs (the udev rule), the UI's words per OS, the .deb
and AppImage. Checked in VMs as the chess app was: Ubuntu 24.04 (Wayland, Xorg), 26.04, Fedora.

**2. macOS (Apple Silicon):** the traits through the system frameworks (`CGEventTap`; the
pasteboard and Cmd+V by `CGEvent`; AX for focus and caret; an NSPanel for the Bar;
`SMAppService` for login; `statfs`); the first run asking for the Microphone, Accessibility and
Input Monitoring permissions; Metal for cleanup, speech on whichever of CoreML and WebGPU
measures faster; a .dmg.

## What the VMs cannot test

VMware's virtual GPU has no CUDA and no usable Vulkan, so a VM runs Utter on the CPU: right for
the desktop integration (keys, the Bar, insertion, packaging), not for speed. Speed is measured
where the GPU is: CUDA on Linux (FLAME, or this PC), Metal on a Mac. AMD and Intel on Linux need
such a machine too.

## Decided (Tai, 2026-10-16)

- **macOS ships through Homebrew, unsigned** (no Apple Developer ID): the core and its UI
  installed by a formula, macOS's own permission prompts (Microphone, Accessibility, Input
  Monitoring). The experience must still be intuitive and smooth: the app explains each
  permission before macOS asks, sees when one is missing (an unsigned build's grants are tied to
  its ad-hoc signature, which changes at every update) and takes the person straight to the
  right pane of System Settings; it never fails silently.
- **No Mac here.** macOS is compile-checked in CI from the start; development and measurement on
  a rented Apple Silicon Mac (by the hour) when the macOS phase starts; buying one only if that
  proves too limiting.

## Decisions for Tai

1. **A Mac to test on.** None here, and VMware on Windows cannot run macOS. Options: buy or
   borrow an Apple Silicon Mac; rent one by the hour (Scaleway, AWS); or ship macOS
   compile-checked only, as DESIGN.md planned, until hardware exists.
2. **Apple Developer ID** (USD 99 a year): without it Gatekeeper blocks the app, and macOS ties
   the Accessibility and Input Monitoring grants to the signature, so an unsigned app loses them
   at every update.
3. **Running without a GPU.** The VMs need a CPU path anyway. Should it ship (laptops, Linux
   without a supported card), and with what latency target? The phone ran the 1B cleanup at
   1.5 s on its CPU.
4. **Which Linux:** Ubuntu 24.04+ and Fedora 40+ (the chess app's floor), GNOME first; KDE when?
5. **CI on GitHub Actions** for the repository.
