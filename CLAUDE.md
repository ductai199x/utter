# Utter

Local voice dictation for Windows, Linux and macOS: hold a key, speak, and clean text appears
in the focused app. A local answer to Wispr Flow. Read `DESIGN.md` (and `docs/README.md`, the index of the rest) before changing
architecture or scope; it is the source of truth.

## Priorities

- **No slop.** Tai's rule: attention to detail over speed of prototyping. A feature is done
  when it meets its latency budget, eval numbers and UX checks (DESIGN.md section 12), not
  when it first works.
- **Latency first.** Every stage of a dictation is traced. Claim nothing is faster or slower
  without a measurement.
- **Local only.** Nothing leaves the machine except model downloads and updates.
- **Standalone.** Utter depends on nothing installed on the machine but the GPU driver: no
  LM Studio, no Python, no system CUDA. Everything it runs on is a pinned, checksummed
  component in `crates/utter-runtime/components.json`, installed by `utter-cli setup`.

## Conventions

- One codebase for Windows, Linux and macOS. OS calls live only in `crates/utter-platform`,
  behind traits. Windows is implemented first. Android (later) is separate Kotlin.
- The UI renders state and sends intent. Logic and timing live in Rust.
- Python is only for `tools/` (eval, datasets, benchmarks), managed with uv. It is never
  shipped.
- LF line endings everywhere.

## Branches

- Work on `dev`; it carries the docs, `DESIGN.md`, this file and `dev/`. `main` is public and gets only what `dev/release.py publish` builds, **only when Tai says so**. Never commit to `main` or push it otherwise.
- Keep paths on his machine, private addresses, emails and other setup details out of every file, even on dev (both branches are public). Use `%LOCALAPPDATA%`, `<repo>` and the like. Run `release.py scan` before committing anything with paths or hostnames. Details: `dev/README.md`.
