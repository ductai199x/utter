# Utter

Local voice dictation for Windows, Linux and macOS: hold a key, speak, and clean text appears
in the focused app. A local answer to Wispr Flow. Read `DESIGN.md` before changing
architecture or scope; it is the source of truth.

## Priorities

- **No slop.** Tai's rule: attention to detail over speed of prototyping. A feature is done
  when it meets its latency budget, eval numbers and UX checks (DESIGN.md section 12), not
  when it first works.
- **Latency first.** Every stage of a dictation is traced. Claim nothing is faster or slower
  without a measurement.
- **Local only.** Nothing leaves the machine except model downloads and updates.

## Conventions

- One codebase for Windows, Linux and macOS. OS calls live only in `crates/utter-platform`,
  behind traits. Windows is implemented first. Android (later) is separate Kotlin.
- The UI renders state and sends intent. Logic and timing live in Rust.
- Python is only for `tools/` (eval, datasets, benchmarks), managed with uv. It is never
  shipped.
- LF line endings everywhere.
