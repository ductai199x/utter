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
