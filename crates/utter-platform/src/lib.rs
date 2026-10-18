//! Utter's OS layer. Every call into the operating system lives in this crate (CLAUDE.md),
//! with one module per OS behind the same functions. Windows comes first; the other
//! platforms say what they lack rather than pretend.

pub mod autostart;
pub mod caret;
pub mod console;
pub mod disk;
pub mod focus;
pub mod gpu;
pub mod hotkey;
pub mod insert;
pub mod memory;
pub mod overlay;
pub mod process;
pub mod shell;
pub mod shortcut;
