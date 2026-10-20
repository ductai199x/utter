# Platforms

Windows, Linux and macOS, and the GPU backends Utter runs on (CUDA, Vulkan, Metal, the
processor): what is supported, how each OS is integrated, what was measured, and what is still
open. True for **Utter 0.1.10** (2026-10-20). How to build, test in VMs, rent a Mac and release
is in [development.md](development.md); the architecture and its principles are in
[DESIGN.md](../DESIGN.md).

## 1. Support matrix

| OS | Backend | Models offered | Status | Verified on |
|---|---|---|---|---|
| Windows 10/11 x64 | CUDA (NVIDIA) | Quality, Light | Shipping (0.1.0) | RTX 3090 Ti, Windows 11, every release |
| Windows 10/11 x64 | Vulkan (AMD, Intel) | Quality, Light | **Preview** (0.1.6) | Only the RTX 3090 Ti forced onto Vulkan and a 2-CU Radeon iGPU (2026-10-13); no real AMD or Intel card |
| Windows 10/11 x64 | Processor | Light, Extra light | Shipping (0.1.8) | Ryzen 9 9900X; a Windows 11 VM (2026-10-19) |
| Linux x86_64 | CUDA (NVIDIA) | Quality, Light | Shipping (0.1.8) | RTX 3090 Ti under WSL 2, Ubuntu 24.04 (2026-10-18); not on a native Linux desktop |
| Linux x86_64 | Vulkan (AMD, Intel) | Quality, Light | **Preview**, not tested on Linux | - |
| Linux x86_64 | Processor | Light, Extra light | Shipping (0.1.8) | VMs: Ubuntu 24.04 and 26.04, Fedora 44 (2026-10-18) |
| macOS, Apple Silicon | Metal | Quality, Light | Shipping (0.1.9) | Rented Mac mini M2, 16 GB, macOS 26.7 and 27.0.1 (2026-10-19/20) |
| macOS, Apple Silicon | Processor | Light, Extra light | Selectable in Settings | One figure on the M2 (section 3) |

Not supported: Intel Macs (onnxruntime 1.31 publishes no `osx-x64` build), Windows and Linux on
ARM. Extra light (`utter-mini-v1`, Gemma 3 1B) is offered only on the processor, and Quality
never there (`utter_pipeline::runtime_choice::runs_on`).

Linux desktops checked: GNOME 46 and 50 (Wayland; Xorg on GNOME 46) and KDE Plasma 6 (Wayland).
On GNOME the tray icon needs an AppIndicator extension; without one, Settings has Quit and a
second launch brings Home back.

### Minimum requirements

| | Requirement | Where it is enforced |
|---|---|---|
| Windows | 10 or 11, 64-bit | NSIS installer, per user, no administrator |
| Linux | glibc 2.39: Ubuntu 24.04+, Debian 13, Mint 22, Fedora 40+ | packages built on Ubuntu 24.04 (`packaging/linux/build.sh`) |
| macOS | 14 (Sonoma) or newer, arm64 | the cask (`depends_on macos: ">= :sonoma"`, `arch: :arm64`); the bundle itself says 13.0 (`tauri.macos.conf.json`) |
| NVIDIA driver | 580 or newer (CUDA 13.0: `MIN_DRIVER_CUDA = 13_000`) | first run's check |
| Vulkan driver | any that offers a non-software device | `utter_platform::gpu::vulkan_available`; no version check |
| Disk | about 10 GB | README |

Memory a model needs to be offered (`card_need` = `gpu_need` + 1 GB for the desktop and other
apps; on Metal and the processor it is compared with system memory):

| Backend | Quality | Light | Extra light | Source of `gpu_need` |
|---|---|---|---|---|
| CUDA | 5.8 GB | 4.25 GB | - | peaks measured end to end, 2026-10-11 |
| Vulkan | 7.2 GB | 5.5 GB | - | peaks measured on the 3090 Ti, 2026-10-13 |
| Metal | 7.2 GB | 5.5 GB | - | Vulkan's figures, **not measured** on a Mac |
| Processor | - | 5.5 GB | 4.0 GB | Vulkan's figures; Extra light estimated, **not measured** |

## 2. The platform layer per OS

Every OS call is in `crates/utter-platform`, one module per OS behind the same functions
(DESIGN.md section 7). The UI never calls the OS; the app shell only places windows.

| Job | Windows | Linux (X11 and Wayland alike) | macOS |
|---|---|---|---|
| Dictation key (`hotkey.rs`) | Raw Input (`RIDEV_INPUTSINK`) on a message-only window, own thread | evdev: every keyboard read from `/dev/input/event*`, keyboards plugged in later found within 2 s (`linux_input.rs`) | one listen-only `CGEventTap` (`macos.rs`) |
| Paste last / copy last (`shortcut.rs`) | `RegisterHotKey`: Alt+Shift+Z, Alt+Shift+X; the app never sees them | the same evdev stream; the keys also reach the app | the same event tap: Ctrl+Cmd+V, Ctrl+Cmd+C; the keys also reach the app |
| Insertion (`insert.rs`) | clipboard (every global-memory format saved), `SendInput` paste keys; text kept out of clipboard history | X11 CLIPBOARD through arboard (XWayland bridges it to Wayland apps), paste keys from Utter's own uinput keyboard | `NSPasteboard` (every item and type saved), Cmd+V as CoreGraphics events |
| Typing fallback | `KEYEVENTF_UNICODE`, any character | uinput, US layout, ASCII only | CoreGraphics Unicode events, any character |
| Focused app (`focus.rs`) | foreground window and its process | AT-SPI focus events, pid from the bus, name from `/proc/PID/exe` and desktop entries (`linux_a11y.rs`) | `NSWorkspace` frontmost app, its pid as the window |
| Text before the cursor (`caret.rs`) | UI Automation text pattern | AT-SPI Text (`CaretOffset`, `GetText`) | `AXSelectedTextRange`, `AXStringForRange` |
| Start at login (`autostart.rs`) | `HKCU\...\Run` value | `~/.config/autostart/utter.desktop`, with `TryExec`; the AppImage's own path (`$APPIMAGE`) | `~/Library/LaunchAgents/app.utter.plist` with `AssociatedBundleIdentifiers`, rewritten only when it changes |
| llama-server ends with Utter (`process.rs`) | job object, kill on close | `PR_SET_PDEATHSIG` from one long-lived spawner thread | a watcher shell polls every second; plus a sweep of recorded leftovers at the next server start (`utter-llm`) |
| GPU (`gpu.rs`) | DXGI adapters, high-performance order | Vulkan devices through `libvulkan.so.1`, discrete first, software skipped | the default Metal device |
| Keyboard access | nothing to grant | udev rule, `uaccess` (below) | Accessibility (below) |

The paste rules and their per-OS defaults are in [ux/insertion.md](ux/insertion.md): Linux
terminals paste with Ctrl+Shift+V (XTerm Shift+Insert); macOS terminals take Cmd+V like every app,
and only remote desktops, Word and Outlook have rules there.

### Keyboard access

- **Linux.** `packaging/linux/70-utter.rules` tags keyboards and `/dev/uinput` `uaccess`: the
  user at the seat, while logged in, and no remote login. The .deb installs it and applies it in
  `postinst.sh`. The AppImage (or a removed rule) gets it from first run's "Keyboard access" step:
  Allow runs a script named `utter-keyboard-access` through pkexec, which writes the rule, loads
  uinput and re-triggers udev, so it works without logging out.
- **macOS.** Accessibility alone lets Utter read keys and paste on macOS 26 and 27; asking for
  Input Monitoring at the same time added Utter to neither list. macOS applies the grant only to a
  new process, so Utter restarts itself when the switch turns on (`granted_on_restart`). The first
  Allow leaves the work to macOS's own prompt; later ones open the pane.

### Elevated windows and secure input

- **Windows:** a window of a program running as administrator, while Utter is not, drops
  injected keys (UIPI). `focus::refuses_input` checks the target's token; the dictation stays on
  the clipboard and the Bar says so.
- **Linux and macOS:** `refuses_input` is always false.
- **Secure input** (password fields, UAC's secure desktop, macOS Secure Event Input): **not
  built.** DESIGN.md section 7 plans detecting it; no OS module does.

### Wayland

Utter uses no portal. The GlobalShortcuts portal cannot bind a lone modifier such as Right Ctrl
nor report a held key; GNOME offers a Wayland client no way to set the clipboard without keyboard
focus, which the Bar never takes, and no way to keep a window above others or place it. So:

- keys in through evdev and out through uinput (kernel level, every app, no dialog);
- the clipboard owned through X11, bridged by XWayland;
- the windows under XWayland: the app sets `GDK_BACKEND=x11,wayland` and
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` unless the person set them;
- Qt apps join the accessibility bus only while the session's accessibility switch
  (`org.a11y.Status IsEnabled`) is on, which Plasma leaves off: Utter turns it on at start and
  back off at quit if it turned it on;
- Chromium and Electron apps expose AT-SPI only when asked; then the focus is an unknown window
  taken to be in front: the paste goes in, unchecked, without spacing or a paste rule.

### The Bar overlay

- **Windows:** `utter_platform::overlay` gives the window `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW`,
  places it topmost at the bottom centre of the work area of the monitor holding the focused
  window, scaled by that monitor's DPI, without activating it.
- **Linux and macOS:** Tauri places it (`bar_bottom_center` in `app/src-tauri/src/main.rs`) on
  the monitor under the mouse pointer, sets click-through and always-on-top at each show. On Linux
  it is a GTK notification-type window, which KWin never activates (GTK 3 offers
  `WM_TAKE_FOCUS`, and KWin made the Bar active; 2026-10-18). On macOS it is a plain transparent
  Tauri window (`macOSPrivateApi`), not an `NSPanel`.

Layout, states and motion: [ux/bar.md](ux/bar.md).

### Media

Pausing music while dictating: **not built** on any OS (DESIGN.md section 7 lists it).

## 3. GPU backends

### How the backend is chosen

`utter_pipeline::runtime_choice::available()` lists what the computer runs, best first: on
Windows and Linux, CUDA when NVIDIA's driver answers (`utter_runtime::gpu::probe`), Vulkan when a
graphics card offers it (NVIDIA's included), then the processor; on a Mac, Metal then the
processor. Settings' "Runs on" offers exactly that list; Auto takes the first. The app selects
the backend once at start, before anything asks. `UTTER_GPU_BACKEND` (cuda, vulkan, metal, cpu)
overrides it for testing.

### What each backend runs

| Backend | Speech (Parakeet) | Cleanup (llama-server b11535) | Runtime components |
|---|---|---|---|
| CUDA | 8-bit encoder (`parakeet-ultra-onnx-w8`) on onnxruntime's CUDA provider | `llama-cpp` (CUDA 13.4 build), `--flash-attn on` | `onnxruntime-gpu`, `cuda-runtime`, `cudnn` (+ `curand` on Linux), `silero-vad`, `llama-cpp` |
| Vulkan | float16 encoder on onnxruntime's WebGPU plugin 0.4.0 (Dawn over D3D12 on Windows, Vulkan on Linux) | `llama-cpp-vulkan`, `--flash-attn auto`, always `--device VulkanN` | `onnxruntime` (CPU build), `onnxruntime-ep-webgpu`, `silero-vad`, `llama-cpp-vulkan` |
| Metal | float16 encoder on the WebGPU provider built into onnxruntime's macOS build | `llama-cpp-metal`, `--flash-attn auto` | `onnxruntime`, `silero-vad`, `llama-cpp-metal` |
| Processor | int8 encoder (`parakeet-ultra-onnx-i8`) | `llama-cpp-cpu` (the Metal build on a Mac), no GPU layers, `--threads` | `onnxruntime`, `silero-vad`, `llama-cpp-cpu` or `llama-cpp-metal` |

All pinned with SHA-256 in [components.json](../crates/utter-runtime/components.json): onnxruntime
1.31.0, the WebGPU plugin 0.4.0, cuDNN 9.27.0.42, CUDA runtime 13.4, llama.cpp b11535. Linux
entries are `.tgz`/`.tar.gz` and manylinux wheels; the Mac's are `osx-arm64` and
`macos-arm64` (the Metal build needs `libggml-rpc` in its extract list). A macOS WebGPU plugin
entry is pinned but not used: registering it beside the built-in provider failed ("EP library is
already registered"). Libraries reach llama-server through `PATH`, `LD_LIBRARY_PATH` or
`DYLD_LIBRARY_PATH` set for that child only, never an AppImage's own folders.

Details that matter:

- **The Vulkan device.** The card is the first DXGI adapter in high-performance order (Windows)
  or the first discrete Vulkan device (Linux). llama-server gets it by name from
  `--list-devices` (asked once per process, about 300 ms): without `--device` the Vulkan build
  splits the model across every GPU, an iGPU too. The WebGPU plugin 0.4.0 ignores the device it is
  given and runs on Dawn's high-performance adapter, which is the same card (forcing low-power,
  the Radeon iGPU took 560 ms for the encoder against 34 ms on the 3090 Ti; 2026-10-13). The
  plugin lists every adapter with its LUID and DXGI index, which is how the card is tied to the
  probe and to llama.cpp's device.
- **Flash attention.** Vulkan and Metal pass `--flash-attn auto`, CUDA `on`: flash attention
  slows prompt processing on some Intel cards, and llama.cpp knows which.
- **WebGPU against CUDA.** About 2.5 times slower at steady state on the same card (33.9 against
  13.5 ms for 500 frames); a new input length costs nothing extra (pipelines are reused across
  lengths); validation mode and the buffer cache mode change nothing. About 900 nodes stay on
  the CPU, as with CUDA: shape arithmetic and the padding amounts of each convolution, small
  integer tensors; no audio crosses. The one clip whose transcript differs (59 of 60 identical)
  is the same in float16 and 8-bit, a borderline word. The one cleanup output that differs
  from CUDA's differs in case only. DirectML is not used: it is in sustained engineering
  (packages stopped at 1.24), a fallback only if WebGPU fails on a card.
- **Small iGPUs are not a target.** A 2-CU Radeon runs the cleanup correctly and far too slowly
  (table below); a real AMD or Intel card is needed to measure the target.
- **Memory is a per-process budget.** Windows has no global free-memory figure for a card; the
  probe reads the OS budget (`IDXGIAdapter3::QueryVideoMemoryInfo`), and `card_need` is compared
  with that.
- **Float16 on WebGPU.** WebGPU runs the 8-bit encoder slower than float16 (below), so Vulkan and
  Metal run float16; CUDA runs 8-bit, which transcribes the same as float16 with 0.66 GB less
  memory for about 5 ms a clip (2026-10-11).
- **The processor.** Threads per model are a share of the logical processors: half by default,
  75% with Settings' High (`utter_platform::cpu`, 2026-10-18). The int8 encoder exists because
  the GPU's 8-bit build runs on the processor dequantized to float32: 114.6 ms of encoder per
  second of audio, against 28.0 for float32 and 17.7 for int8; WER over 500 clips of five public
  sets 6.09%, 6.11%, 6.29% (2026-10-18). GPU keep-alive passes are off there: they took cores.
- **The clock hold** (a sleeping CUDA kernel while the key is down, so the GPU is not in a low
  power state at key up) is CUDA only. On Vulkan and Metal nothing holds the clocks.

### Measured

All on the development PC (RTX 3090 Ti, Ryzen 9 9900X) unless stated. Current in-app figures for
Windows CUDA are in [evals.md](evals.md).

**Cleanup alone**, Quality v2, `utter-cli llm-bench`, 100 cases (2026-10-13):

| | CUDA, 3090 Ti | Vulkan, 3090 Ti | Vulkan, Radeon iGPU (2 CUs) |
|---|---|---|---|
| Cleanup wall p50 / p95 | 250 / 387 ms | 274 / 417 ms | 6,464 / 9,481 ms (10 cases) |
| Prompt processing p50 | 21.3 ms | 45.3 ms | 1,619 ms |
| Per generated token p50 | 4.20 ms | 4.21 ms | 107.5 ms |
| Outputs identical to CUDA | - | 99 of 100 | 9 of 10 |

**Speech encoder alone**, CUDA vs the WebGPU plugin on the 3090 Ti, 60 clips (2026-10-13):
float16 20.0 / 24.5 ms against 47.8 / 76.3 ms; 8-bit 24.8 / 28.3 ms against 81.3 / 106.8 ms;
59 of 60 transcripts identical.

**In Utter, CUDA vs Vulkan** on the 3090 Ti (Utter 0.1.5 build, 2026-10-13):

| | CUDA | Vulkan |
|---|---|---|
| Speech bench, encoder p50 / p95 (120 clips) | 19.7 / 30.6 ms | 50.8 / 75.6 ms |
| Transcripts identical / WER | - / 6.76% | 118 of 120 / 6.79% |
| e2e Quality, key up to text p50 / p95 (80 dictations) | 180 / 361 ms | 234 / 429 ms |
| e2e Light, key up to text p50 / p95 | - | 201 / 381 ms |
| Peak GPU memory, Quality / Light | 4,550 / 3,085 MiB | 5,690 / 4,143 MiB |

The first Vulkan Quality run had two cleanups over 3 s; a repeat of its AMI half had none.
Taken as contention on the shared card, not shader compilation (which would repeat).

**Linux vs Windows, CUDA** (WSL 2, Ubuntu 24.04, `utter-cli e2e-bench`, Quality, 40 clips,
2026-10-18): key up to text p50 / p95 197 / 393 ms on Linux, 199 / 345 ms on Windows; speech
text identical 40 of 40; final-text WER 7.0% and 8.0%. 13 of 40 cleaned texts differ (the two
CUDA builds of llama.cpp round differently and greedy decoding follows).

**The processor** (`utter-cli e2e-bench`, Quality's 40 clips, 2026-10-18):

| | key up to text p50 / p95 | final-text WER |
|---|---|---|
| Extra light, int8 speech, before the threads setting | 0.37 / 0.73 s | 9.1% |
| Extra light, int8 speech from the published component | 0.39 / 0.81 s | - |
| Light, the same | 0.50 / 1.15 s | 7.2% |

How the processor path got there (Extra light, same bench, quiet machine, 2026-10-18):

| Step | key up to text p50 / p95 | speech after key up p50 |
|---|---|---|
| as it was (speech on 2 threads, GPU build's model, keep-alive passes) | 1.84 / 2.48 s | 1,608 ms |
| speech on the processor's cores | 1.20 / 2.04 s | 1,014 ms |
| + int8 speech model, no keep-alive passes | 0.38 / 0.82 s | 73 ms |
| + threads by physical cores (6 here; since replaced by the Settings share) | 0.37 / 0.73 s | 70 ms |

The int8 encoder is fastest with 4-6 threads (123 ms for a 9 s clip; 141 ms with 12, 146 with
24). `utter-speech quantize-cpu` makes it: dynamic quantization, int8 weights per channel,
`MatMulInteger` (AVX-512 VNNI here). MatMulNBits with int8 compute (`accuracy_level` 4) was 3x
faster and returned empty transcripts, so it is not used. llama.cpp already used everything this
processor has (AVX-512, VNNI, BF16, weights repacked to q4_0_8x8 at load, one thread per physical
core). The published int8 model is `ductai199x/utter-parakeet-ultra-onnx`, `i8/`, revision
`d1b9425d`.

Why Extra light is recommended there (superseded bench, 24 threads, 2 speech threads): cleanup p50 /
p95 308 / 786 ms for Extra light against 391 / 1,057 ms for Light, i.e. 21% and 26% faster, on a
fast desktop processor; a laptop's and the phone's widen the gap (1.5 s against 2.7 s on the
phone). Speech was most of the total then (1.6-2.0 s) and moved 0.4 s between runs, so only the
cleanup column ranks the models.

In a 6-core Ubuntu 24.04 VM with no GPU: 128-224 ms key up to text (the cleanup done during the
last pause), model loading 3.1 s, relaunch ready in 2.77 s (6.6 s before the int8 model). Fedora 44 VM, 6 cores:
150-191 ms, ready in 4.17 s (4.13 s with the page cache dropped); the first load after the download
took 8.1 s, once, while btrfs was still compressing the new files.

**Metal**, Mac mini M2 16 GB, macOS 27 (2026-10-19/20): ready 2.2-2.9 s after launch warm, 13.7 s
right after the first download. Key up to text: Quality 0.8-1.2 s, Light 0.15-0.5 s; the
processor (Light, int8 speech) about 1.5 s. Not profiled by stage.

### Known gaps

- Vulkan has never run on a real AMD or Intel card: no latency, memory, cold-start or
  shader-cache figure. Its `gpu_need` is the 3090 Ti's peak.
- No clock hold on Vulkan or Metal.
- Metal's and the processor's memory needs are Vulkan's figures; Extra light's is an estimate.
- Speech on CoreML (built into onnxruntime's macOS build) was never measured against WebGPU.
- Known llama.cpp risks on Intel: driver bugs at small batch sizes (llama.cpp #28648, Arc 140V
  iGPU, `-ub 128`, which Utter uses), flash-attention and cooperative-matrix problems per GPU;
  the escape hatches are the `GGML_VK_DISABLE_*` variables.
- Cold pipeline compilation on a first run (ggml-vulkan keeps no pipeline cache of its own) is
  unmeasured on a real card. Vulkan uses two GPU APIs on one card (Vulkan for llama.cpp, D3D12
  under Dawn on Windows); the WebGPU plugin is young (five releases in five months) and must match
  onnxruntime's version.

## 4. macOS

- **Signing.** Utter is signed with its own certificate ("Utter Homebrew Signing", SHA-1
  `E411284D187F5D405F62FCB184A7985F7DB275C2`, valid to 2036) by `packaging/macos/sign.sh`, with
  the hardened runtime. macOS ties Accessibility and Microphone grants to the app's designated
  requirement. With Tauri's ad-hoc signature that is the exact build, so every update lost the
  keyboard permission while System Settings still showed it on. With one certificate the
  requirement is "app.utter signed by this certificate", and grants survived updates, reinstalls
  and the in-app updater (measured on 26.7 and 27.0.1). Losing the key means every user grants
  the permissions again; it is kept outside the repository.
- **Entitlements** (`app/src-tauri/macos/Utter.entitlements`):
  `com.apple.security.device.audio-input` (without it the hardened runtime refuses the
  microphone silently: no prompt, no entry in the list) and
  `com.apple.security.cs.disable-library-validation` (onnxruntime is a downloaded library).
  `Info.plist` adds `NSMicrophoneUsageDescription` and `LSUIElement` (menu bar app, no Dock
  icon).
- **Gatekeeper.** The certificate is not an Apple Developer ID, so a quarantined copy would not
  open ("cannot be verified"), and macOS 15 and later no longer offer Open from the context menu.
  The cask's `postflight` removes the quarantine attribute after Homebrew has checked the zip's
  SHA-256.
- **Homebrew.** The repository is its own tap; `Casks/utter.rb` is generated by
  `packaging/macos/cask.sh` from `packaging/macos/utter.rb`. Homebrew 7 refuses a third-party
  tap's cask until `brew trust --cask ductai199x/utter/utter`. The cask sets `auto_updates true`,
  removes the LaunchAgent on uninstall, and `--zap` deletes the models and settings.
- **Updates.** The in-app updater (0.1.10) replaces Utter.app from a signed `.app.tar.gz`; the
  update is checked against the updater key, and the new build carries the same certificate, so
  permissions stay.
- **Facts that shaped the code** (macOS 26.7 and 27.0.1):
  - macOS 27 renamed Accessibility to "Device Control and Data Access"; Utter's text names
    neither pane.
  - Opening Utter while it runs is a `RunEvent::Reopen`, not a second process.
  - Session restore at sign-in starts a second copy beside the login item: a plain start within
    30 s of a sign-in start shows nothing.
  - Windows open behind the app in front unless focused at creation.
  - Sync Tauri commands run on the UI thread: opening the microphone there froze the window
    for 27 s while macOS asked for it; such commands are async.
  - A default input device that no longer exists is treated as no microphone.
  - Wording: no graphics card (shared memory), Cmd+V, a menu bar not a tray, Ctrl+Cmd+V/C,
    Right Option as the default key (most Mac keyboards have no Right Control), no Scroll Lock,
    F20-F24 or Insert.
- **Known only from a rented M2.** All macOS testing was on one Mac mini M2 with 16 GB, over
  SSH and screen sharing, with replayed speech through a virtual microphone. Not seen: an 8 GB
  Mac, M3/M4, a laptop on battery, a built-in microphone, full-screen apps and multiple displays
  with the Bar, Fn/Globe as the dictation key. How that testing works:
  [development.md](development.md).

## 5. Linux

### Packaging

`packaging/linux/build.sh` builds both on Ubuntu 24.04 (`tauri.linux.conf.json`):

- **.deb**: depends on `libasound2`; installs `70-utter.rules` to `/usr/lib/udev/rules.d` and
  applies it in `postinst.sh` (`postrm.sh` re-triggers udev on removal). Updates go through
  pkexec running `dpkg -i`.
- **AppImage**: `fix-appimage.sh` removes libwayland and libdrm (they must match the system's
  Mesa; older copies left WebKit's window empty under Fedora 44) and swaps Tauri's runtime for
  AppImage's static type2 runtime (pinned by SHA-256), so it runs without FUSE 2. The keyboard
  rule comes from first run (pkexec). Updates write the new image over the old one. Children get
  the desktop's environment, not the image's (`process::outside_appimage`): llama-server once
  loaded the image's libcrypto with Ubuntu 26.04's libssl and did not start. The AppImage writes
  no desktop entry of its own.
- Exit: on Linux the process ends before onnxruntime tears itself down
  (`skip_teardown_at_exit`, `crates/utter-asr/src/runtime.rs`): the ort crate released its
  environment after onnxruntime's own destructors and glibc aborted with heap corruption at every
  exit after a CUDA session (WSL 2, 2026-10-18). Linux's CUDA provider also does not load without
  cuRAND, hence the `curand` component.
- AT-SPI names the program from `/proc/PID/exe`, not `comm`, which the kernel cuts at 15
  characters (`gnome-terminal-`). `PR_SET_PDEATHSIG` fires when the *thread* that spawned the
  child exits, so llama-server is spawned from one long-lived thread (`process::spawn_tied`).
- Linux library errors are told apart: the driver's `libcuda.so` from Utter's own libraries.
- A second launch hands its startup token (`DESKTOP_STARTUP_ID` or `XDG_ACTIVATION_TOKEN`) to
  the running copy, so GNOME brings Home to the front instead of treating it as focus stealing.

### What the VM tests covered (2026-10-18/19)

| System | Package | Checked |
|---|---|---|
| Ubuntu 24.04, GNOME 46, Wayland | .deb | keyboard access from the rule alone (user not in `input`); first run end to end; the Bar with live words; dictations into the setup window and GNOME Terminal (Ctrl+Shift+V); the rule removed, then the problem screen, Allow, pkexec, working without logout |
| Ubuntu 24.04, GNOME 46, Xorg | AppImage, no FUSE 2 | starts, the Bar, dictation into GNOME Terminal |
| Ubuntu 26.04, GNOME 50, Wayland | built there | first run, the Bar, dictation into Ptyxis and Text Editor, tray menu, Quit leaving no process |
| Ubuntu 26.04, KDE Plasma 6, Wayland | built there | start at login, tray icon, dictation into Konsole (after the accessibility switch and notification-window fixes) |
| Fedora 44, GNOME 50, Wayland, btrfs | AppImage from 24.04 | first run with pkexec, the processor picked and Extra light recommended, dictations into Text Editor and Ptyxis, relaunch from a desktop entry |
| Ubuntu 24.04, no GPU, 6 cores | .deb | the processor path with Extra light, figures in section 3 |

Before 0.1.8 a full install, first run, dictation, restart, quit and uninstall ran in an Ubuntu
24.04 VM. AT-SPI was checked with GNOME Text Editor, GNOME Terminal, Ptyxis and Konsole.

### What VMs cannot test

VMware's virtual GPU has no CUDA and no usable Vulkan, so every VM ran on the processor: right
for keys, the Bar, insertion and packaging, not for speed. Not covered: a native Linux desktop
with an NVIDIA card (CUDA was measured under WSL 2 only), AMD or Intel graphics on Linux,
HiDPI and multi-monitor placement of the Bar, real microphones (a PipeWire virtual source fed eval
clips), and AT-SPI in Firefox, Chromium, Electron and Qt apps beyond Konsole.

## 6. Open items

**Every platform**
- Secure input detection (password fields, UAC, Secure Event Input): not built.
- Pausing media while dictating: not built.
- A Vulkan driver version check (the plan named Vulkan 1.2): not built; any non-software device
  passes.

**Windows**
- Vulkan on a real AMD card (RDNA 3) and Intel card (Arc B580 or similar): latency per stage,
  peak memory per tier (to replace `gpu_need`), cold start with an empty shader cache against the
  5 s bar, outputs against CUDA. The preview label comes off only when these meet the budgets.
- A clock hold on Vulkan (a napping compute dispatch), measured cold against warm.
- On the processor, dictations in a Windows 11 VM took about 1 s against 0.15-0.25 s in an
  Ubuntu VM (0.1.8 release checks, 2026-10-19); not investigated.
- The installer is unsigned: SmartScreen's "Windows protected your PC" on every install.

**Linux**
- Vulkan on AMD and Intel: never run.
- Native NVIDIA desktop: not run (WSL 2 only).
- Utter exited silently once mid-dictation on Plasma before the 2026-10-18 fixes; not seen in
  four fresh starts after them.
- Paste through a Wayland clipboard setter (`wl-copy`) was never checked for a focus-taking
  flash; Utter uses the X11 setter.
- AT-SPI in Firefox, Chromium, Electron and Qt apps; Xfce and wlroots compositors (Sway); KDE on
  X11.
- The Bar on HiDPI and with several monitors (it follows the mouse pointer, not the focused
  window).
- An AppImage desktop entry (today the person or a tool such as Gear Lever makes one).
- Typing fallback beyond ASCII on the US layout.

**macOS**
- Memory per tier on Metal and the processor (`gpu_need` is Vulkan's).
- An 8 GB Mac; M3/M4; laptops.
- Quality at 0.8-1.2 s key up to text on the M2: not profiled by stage.
- Speech on CoreML against WebGPU.
- The Bar as a non-activating `NSPanel` (it is a plain window; behaviour over full-screen apps
  and on several displays not checked).
- The bundle's `minimumSystemVersion` (13.0) and the cask's floor (14) disagree; only 26 and 27
  were run.

## 7. Decisions log

| Date | Decision | Why |
|---|---|---|
| 2026-10-12 | Windows hotkey through Raw Input, not `WH_KEYBOARD_LL` | Windows stops calling a low-level hook while a Chromium/WebView2 window has focus: Utter's own windows made it deaf |
| 2026-10-13 | NVIDIA keeps CUDA; AMD and Intel run Vulkan (llama.cpp) and WebGPU (speech) | Vulkan generates as fast as CUDA on NVIDIA but processes prompts twice as slowly (45.3 against 21.3 ms) |
| 2026-10-13 | Float16 encoder on WebGPU; always pass `--device` | 8-bit is slower on WebGPU; without `--device` the model is split across every GPU |
| 2026-10-13 | Ship Vulkan as a preview (0.1.6), and accept the WebGPU plugin ignoring its device (Tai: fine for now) | no AMD or Intel card to measure on; the plugin's high-performance adapter is the right card |
| 2026-10-16 | Linux first (GNOME, Wayland and X11 alike; Tai), then macOS; CI on GitHub Actions for all three | the order in DESIGN.md; no Mac on hand |
| 2026-10-16 | Linux keys through evdev and uinput with a udev `uaccess` rule; no portals | GlobalShortcuts cannot bind a lone modifier or report a held key; the spike pasted into a Wayland editor with no dialog |
| 2026-10-16 | macOS through Homebrew, no Apple Developer ID; test on a rented Mac rather than buy one (Tai) | cost; the first run must still explain each permission and never fail silently |
| 2026-10-18 | The processor path ships, with Light and Extra light, Quality off (Tai) | laptops and Linux without a supported card; Quality's cleanup takes seconds there |
| 2026-10-18 | int8 speech model for the processor; threads as a share of logical processors (Default 50%, High 75%) | on the CPU the GPU's 8-bit build took 114.6 ms of encoder per second of audio, int8 17.7 ms |
| 2026-10-18 | The Bar is a notification-type window on Linux; Utter turns on the session's accessibility switch | KWin activated the Bar and the paste went to it; Qt apps were invisible to AT-SPI |
| 2026-10-19 | macOS keyboard access through Accessibility only, with a self-restart | asking for Input Monitoring too added Utter to neither list; grants apply to new processes |
| 2026-10-19 | Sign macOS builds with Utter's own certificate (Tai), not ad hoc and not a paid Developer ID | ad-hoc builds lost the keyboard permission at every update |
| 2026-10-20 | In-app updates on every install type (0.1.10): offered once a day, installed only on Update | one path for Windows, AppImage, .deb and macOS, checked against the updater key |

Superseded: the plan of 2026-10-16 expected the GlobalShortcuts and RemoteDesktop portals,
`wl-clipboard`, an `NSPanel` Bar, `SMAppService` login items and a `.dmg`; none was used.
Threads on the processor were briefly the physical cores, at most 6, before the Settings share
replaced them (both 2026-10-18).
