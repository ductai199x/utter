//! The user's settings, in `<data_dir>/settings.toml` (DESIGN.md section 11): a file the Home
//! window edits and a person can edit too (docs/ux/home.md). Missing keys take their defaults;
//! a missing file is written with every key commented, so there is something to edit. Saving
//! changes only the values, so a person's comments and layout survive.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use utter_llm::cleanup::Tier;
use utter_platform::insert::{self, Keep, PasteKeys, PasteRule};

use crate::PipelineConfig;

/// What a new settings file says: every key, at its default, with what it does.
const TEMPLATE: &str = "\
# Utter's settings. The Home window (tray > Open Utter) edits this file; changes made here by
# hand apply on the next dictation too.

# Names, product names and terms Utter should spell your way. They steer recognition toward
# these words and tell cleanup how to write them.
dictionary = []
# dictionary = [\"Supabase\", \"Kubernetes\", \"Nguyen\"]

# Spoken lists (\"first..., second...\", or items read one after another) become bullets.
lists = true

# Start Utter in the tray when you sign in.
start_at_login = true

# The key you hold to dictate: \"right-ctrl\", \"left-ctrl\", \"right-shift\", \"scroll-lock\", or
# \"f13\" to \"f24\" (extra keys on some keyboards); on a Mac also \"right-option\" and
# \"right-command\". Double-tap it for hands-free.
dictation_key = \"right-ctrl\"

# The microphone: part of its name, or \"\" for the system default.
microphone = \"\"

# The model: \"quality\" (the bigger one, about 4.8 GB of GPU memory, for graphics cards with
# 6 GB or more) or \"light\" (the smaller one, about 3.3 GB, for graphics cards with 4 GB).
model = \"quality\"

# How long History keeps your dictations, on this computer only: \"forever\", \"30-days\",
# \"24-hours\" or \"off\".
keep_history = \"forever\"

# Keep a detailed record of every dictation, with its audio, for troubleshooting
# (<data folder>/dictations). Development builds always keep it.
debug_log = false

# Keep Utter's logs (<data folder>/logs): timings, errors and what Utter did, never what you
# said. They help find out why something went wrong. false writes none and deletes them.
keep_logs = true

# How dictations are pasted, per app (docs/ux/insertion.md). Utter's defaults cover common
# terminals, Office and remote desktops; an entry here replaces the default for that program.
# with: \"ctrl-v\", \"ctrl-shift-v\", \"shift-insert\" or \"type\"
# keep: \"normal\" (0.5 s), \"3s\", \"5s\" or \"leave\" (the old clipboard is not restored)
# [[paste]]
# app = \"mintty.exe\"
# with = \"ctrl-shift-v\"
# keep = \"normal\"
";

/// The most dictionary entries (docs/ux/home.md).
pub const MAX_TERMS: usize = 2000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub dictionary: Vec<String>,
    pub lists: bool,
    pub start_at_login: bool,
    pub dictation_key: DictationKey,
    /// Part of the input device's name; empty for the system default.
    pub microphone: String,
    pub model: Model,
    pub keep_history: KeepHistory,
    /// Keep the detailed dictation log with audio (development builds always do).
    pub debug_log: bool,
    /// Keep Utter's own logs (utter_core::logs); off writes none and deletes them.
    pub keep_logs: bool,
    /// The person's paste rules: each replaces Utter's default for its program.
    pub paste: Vec<PasteOverride>,
}

/// The key held to dictate. Only keys that do nothing on their own when pressed alone: not Alt
/// on Windows and Linux (letting it go opens an app's menu), Caps Lock or Insert (they toggle),
/// or the mouse's side buttons (Back and Forward). Utter only listens: the key still reaches the
/// app. A Mac's Option and Command do nothing alone, and most Mac keyboards have no Right
/// Control: there Right Option is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DictationKey {
    RightCtrl,
    LeftCtrl,
    RightShift,
    ScrollLock,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    /// macOS only.
    RightOption,
    RightCommand,
}

impl Default for DictationKey {
    fn default() -> Self {
        if cfg!(target_os = "macos") { DictationKey::RightOption } else { DictationKey::RightCtrl }
    }
}

impl DictationKey {
    /// The keys Settings offers on this OS.
    pub fn offered() -> Vec<DictationKey> {
        let mac = cfg!(target_os = "macos");
        let first = mac.then_some([DictationKey::RightOption, DictationKey::RightCommand]).into_iter().flatten();
        first.chain(Self::ALL).collect()
    }

    pub const ALL: [DictationKey; 16] = [
        DictationKey::RightCtrl,
        DictationKey::LeftCtrl,
        DictationKey::RightShift,
        DictationKey::ScrollLock,
        DictationKey::F13,
        DictationKey::F14,
        DictationKey::F15,
        DictationKey::F16,
        DictationKey::F17,
        DictationKey::F18,
        DictationKey::F19,
        DictationKey::F20,
        DictationKey::F21,
        DictationKey::F22,
        DictationKey::F23,
        DictationKey::F24,
    ];

    /// Its virtual-key code, as the hotkey listener reports keys.
    pub fn vk(self) -> u32 {
        use utter_platform::hotkey::{VK_F13, VK_LCONTROL, VK_RCONTROL, VK_RSHIFT, VK_SCROLL};
        match self {
            DictationKey::RightCtrl => VK_RCONTROL,
            DictationKey::LeftCtrl => VK_LCONTROL,
            DictationKey::RightShift => VK_RSHIFT,
            DictationKey::ScrollLock => VK_SCROLL,
            // VK_RMENU and VK_RWIN, as macos.rs maps the Mac's keys.
            DictationKey::RightOption => 0xA5,
            DictationKey::RightCommand => 0x5C,
            f => VK_F13 + (f as u32 - DictationKey::F13 as u32),
        }
    }

    /// Its name as people read it ("Right Ctrl", "F13"; "Right Control" on a Mac).
    pub fn name(self) -> String {
        let mac = cfg!(target_os = "macos");
        match self {
            DictationKey::RightCtrl if mac => "Right Control".into(),
            DictationKey::LeftCtrl if mac => "Left Control".into(),
            DictationKey::RightCtrl => "Right Ctrl".into(),
            DictationKey::LeftCtrl => "Left Ctrl".into(),
            DictationKey::RightOption => "Right Option".into(),
            DictationKey::RightCommand => "Right Command".into(),
            DictationKey::RightShift => "Right Shift".into(),
            DictationKey::ScrollLock => "Scroll Lock".into(),
            f => format!("F{}", 13 + (f as u32 - DictationKey::F13 as u32)),
        }
    }
}

/// How long History keeps dictations (docs/ux/history.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum KeepHistory {
    #[default]
    #[serde(rename = "forever")]
    Forever,
    #[serde(rename = "30-days")]
    Days30,
    #[serde(rename = "24-hours")]
    Hours24,
    #[serde(rename = "off")]
    Off,
}

impl KeepHistory {
    /// The oldest an entry may be; `None`: no limit. Off keeps nothing at all.
    pub fn max_age(self) -> Option<std::time::Duration> {
        match self {
            KeepHistory::Forever => None,
            KeepHistory::Days30 => Some(std::time::Duration::from_secs(30 * 24 * 3600)),
            KeepHistory::Hours24 => Some(std::time::Duration::from_secs(24 * 3600)),
            KeepHistory::Off => Some(std::time::Duration::ZERO),
        }
    }
}

/// One app's paste rule, as the person set it (docs/ux/insertion.md, "Paste rules").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasteOverride {
    /// The program's file name, matched without regard to case.
    pub app: String,
    pub with: PasteWith,
    #[serde(default)]
    pub keep: PasteKeep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PasteWith {
    CtrlV,
    CtrlShiftV,
    ShiftInsert,
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PasteKeep {
    #[default]
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "3s")]
    Seconds3,
    #[serde(rename = "5s")]
    Seconds5,
    #[serde(rename = "leave")]
    Leave,
}

impl PasteOverride {
    pub fn rule(&self) -> PasteRule {
        let keys = match self.with {
            PasteWith::CtrlV => PasteKeys::Primary,
            PasteWith::CtrlShiftV => PasteKeys::CtrlShiftV,
            PasteWith::ShiftInsert => PasteKeys::ShiftInsert,
            PasteWith::Type => PasteKeys::Type,
        };
        let keep = match self.keep {
            PasteKeep::Normal => Keep::Normal,
            PasteKeep::Seconds3 => Keep::Seconds3,
            PasteKeep::Seconds5 => Keep::Seconds5,
            PasteKeep::Leave => Keep::Leave,
        };
        PasteRule { keys, keep }
    }

    pub fn from_rule(app: &str, rule: PasteRule) -> Self {
        let with = match rule.keys {
            PasteKeys::Primary => PasteWith::CtrlV,
            PasteKeys::CtrlShiftV => PasteWith::CtrlShiftV,
            PasteKeys::ShiftInsert => PasteWith::ShiftInsert,
            PasteKeys::Type => PasteWith::Type,
        };
        let keep = match rule.keep {
            Keep::Normal => PasteKeep::Normal,
            Keep::Seconds3 => PasteKeep::Seconds3,
            Keep::Seconds5 => PasteKeep::Seconds5,
            Keep::Leave => PasteKeep::Leave,
        };
        Self { app: app.to_owned(), with, keep }
    }
}

/// The paste rule for `exe`: the person's override, else this platform's default, else the
/// plain paste keys.
pub fn paste_rule(overrides: &[PasteOverride], exe: &str) -> PasteRule {
    overrides
        .iter()
        .find(|o| o.app.eq_ignore_ascii_case(exe))
        .map(PasteOverride::rule)
        .or_else(|| {
            insert::default_rules().iter().find(|(app, _, _)| app.eq_ignore_ascii_case(exe)).map(|&(_, _, r)| r)
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    /// The bigger model: about 6 GB of GPU memory in all.
    #[default]
    Quality,
    /// The smaller model: about 2.5 GB of GPU memory in all, speech on the CPU. "extra-light"
    /// was its name for an afternoon.
    #[serde(alias = "extra-light")]
    Light,
}

impl From<Model> for Tier {
    fn from(m: Model) -> Self {
        match m {
            Model::Quality => Tier::Quality,
            Model::Light => Tier::Light,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dictionary: Vec::new(),
            lists: true,
            start_at_login: true,
            dictation_key: DictationKey::RightCtrl,
            microphone: String::new(),
            model: Model::Quality,
            keep_history: KeepHistory::Forever,
            debug_log: false,
            keep_logs: true,
            paste: Vec::new(),
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        utter_core::data_dir().join("settings.toml")
    }

    /// The commented default file: TEMPLATE, with a Mac's default key on macOS.
    fn template() -> String {
        if cfg!(target_os = "macos") {
            TEMPLATE.replace("dictation_key = \"right-ctrl\"", "dictation_key = \"right-option\"")
        } else {
            TEMPLATE.to_owned()
        }
    }

    /// Reads the settings file, writing the commented default first if there is none.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(path, Self::template()).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut s: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        s.dictionary = clean_terms(&s.dictionary);
        Ok(s)
    }

    /// Writes these settings into the file, keeping its comments and the order of its keys.
    /// A file that does not parse is left alone: the person's edit comes first.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = if path.exists() {
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?
        } else {
            Self::template()
        };
        Self::parse(&text).map_err(|e| format!("settings.toml has an error, so it was not changed: {e}"))?;
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
        let mut terms = toml_edit::Array::new();
        for t in &self.dictionary {
            let mut v = toml_edit::Value::from(t.as_str());
            v.decor_mut().set_prefix("\n  ");
            terms.push_formatted(v);
        }
        terms.set_trailing(if self.dictionary.is_empty() { "" } else { "\n" });
        terms.set_trailing_comma(!self.dictionary.is_empty());
        set(&mut doc, "dictionary", toml_edit::Value::Array(terms));
        set(&mut doc, "lists", self.lists.into());
        set(&mut doc, "start_at_login", self.start_at_login.into());
        set(&mut doc, "dictation_key", word(&self.dictation_key).into());
        set(&mut doc, "microphone", self.microphone.as_str().into());
        let model = match self.model {
            Model::Quality => "quality",
            Model::Light => "light",
        };
        set(&mut doc, "model", model.into());
        set(&mut doc, "keep_history", word(&self.keep_history).into());
        set(&mut doc, "debug_log", self.debug_log.into());
        set(&mut doc, "keep_logs", self.keep_logs.into());
        // The paste rules are tables at the end of the file; with none, no entries (the
        // commented example stays).
        if self.paste.is_empty() {
            doc.remove("paste");
        } else {
            let mut rules = toml_edit::ArrayOfTables::new();
            for o in &self.paste {
                let mut t = toml_edit::Table::new();
                t["app"] = toml_edit::value(o.app.as_str());
                t["with"] = toml_edit::value(word(&o.with));
                t["keep"] = toml_edit::value(word(&o.keep));
                rules.push(t);
            }
            doc["paste"] = toml_edit::Item::ArrayOfTables(rules);
        }
        // Write then rename, so a crash never leaves half a file.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn apply(&self, cfg: &mut PipelineConfig) {
        cfg.dictionary = self.dictionary.clone();
        cfg.lists = self.lists;
        cfg.hotkey.key = self.dictation_key.vk();
        cfg.mic.device = (!self.microphone.is_empty()).then(|| self.microphone.clone());
        cfg.tier = self.model.into();
        cfg.paste = self.paste.clone();
        cfg.keep_history = self.keep_history;
        cfg.log.enabled = self.debug_log || utter_core::is_development_build();
        cfg.log.audio = cfg.log.enabled;
    }
}

/// A unit enum's name as serde writes it ("ctrl-shift-v", "3s", "30-days").
pub fn word<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Keeps a value's place (and the comments around it) when it exists; appends it otherwise.
fn set(doc: &mut toml_edit::DocumentMut, key: &str, value: toml_edit::Value) {
    match doc.get_mut(key).and_then(|item| item.as_value_mut()) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        None => doc[key] = toml_edit::Item::Value(value),
    }
}

/// Dictionary entries as stored: trimmed, at least two letters or digits, at most 64
/// characters, no repeats, at most [`MAX_TERMS`].
pub fn clean_terms(terms: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    terms
        .iter()
        .map(|t| t.trim().to_owned())
        .filter(|t| {
            t.chars().count() <= 64 && t.chars().filter(|c| c.is_alphanumeric()).count() >= 2 && seen.insert(t.clone())
        })
        .take(MAX_TERMS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("utter-settings-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.toml")
    }

    #[test]
    fn the_template_parses_to_the_defaults() {
        assert_eq!(Settings::parse(&Settings::template()).unwrap(), Settings::default());
    }

    #[test]
    fn terms_are_trimmed_and_deduplicated() {
        let s = Settings::parse("dictionary = [\" Supabase \", \"\", \"x\", \"Supabase\", \"Nguyen\"]\nlists = false")
            .unwrap();
        assert_eq!(s.dictionary, ["Supabase", "Nguyen"]);
        assert!(!s.lists);
    }

    #[test]
    fn a_misspelled_key_is_an_error_not_a_silent_default() {
        assert!(Settings::parse("dictonary = [\"x\"]").is_err());
        assert!(Settings::parse("model = \"huge\"").is_err());
    }

    #[test]
    fn a_missing_file_is_created_with_the_template() {
        let path = temp("create");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Settings::load_or_create(&path).unwrap(), Settings::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), Settings::template());
    }

    #[test]
    fn saving_keeps_comments_and_round_trips() {
        let path = temp("save");
        std::fs::write(&path, "# mine\nlists = true # keep this\n").unwrap();
        let s = Settings {
            dictionary: vec!["Supabase".into(), "Nguyen".into()],
            lists: false,
            start_at_login: false,
            dictation_key: DictationKey::F13,
            microphone: "USB".into(),
            model: Model::Light,
            keep_history: KeepHistory::Days30,
            debug_log: true,
            keep_logs: false,
            paste: vec![PasteOverride {
                app: "mintty.exe".into(),
                with: PasteWith::CtrlShiftV,
                keep: PasteKeep::Seconds3,
            }],
        };
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine\nlists = false # keep this\n"), "{text}");
        assert_eq!(Settings::parse(&text).unwrap(), s);
    }

    #[test]
    fn paste_rules_save_as_tables_and_clear_away() {
        let path = temp("paste");
        let _ = std::fs::remove_file(&path);
        let mut s = Settings {
            paste: vec![PasteOverride { app: "putty.exe".into(), with: PasteWith::CtrlV, keep: PasteKeep::Leave }],
            ..Settings::default()
        };
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[paste]]\napp = \"putty.exe\"\nwith = \"ctrl-v\"\nkeep = \"leave\""), "{text}");
        assert_eq!(Settings::parse(&text).unwrap(), s);
        s.paste.clear();
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("[[paste]]\napp"), "{text}");
        assert_eq!(Settings::parse(&text).unwrap(), s);
    }

    #[test]
    fn a_rule_overrides_the_default_and_other_apps_paste_plainly() {
        let mine = [PasteOverride { app: "MINTTY.exe".into(), with: PasteWith::CtrlShiftV, keep: PasteKeep::Normal }];
        assert_eq!(paste_rule(&mine, "mintty.exe").keys, PasteKeys::CtrlShiftV);
        assert_eq!(paste_rule(&[], "notepad.exe"), PasteRule::default());
        #[cfg(windows)]
        assert_eq!(paste_rule(&[], "mintty.exe").keys, PasteKeys::ShiftInsert);
    }

    #[test]
    fn every_dictation_key_has_its_own_code_and_name() {
        let every: Vec<DictationKey> =
            DictationKey::ALL.into_iter().chain([DictationKey::RightOption, DictationKey::RightCommand]).collect();
        let codes: std::collections::HashSet<u32> = every.iter().map(|k| k.vk()).collect();
        assert_eq!(codes.len(), every.len());
        assert_eq!(word(&DictationKey::RightOption), "right-option");
        assert_eq!(DictationKey::F24.vk(), 0x87);
        assert_eq!(DictationKey::F24.name(), "F24");
        assert_eq!(word(&DictationKey::RightShift), "right-shift");
        assert_eq!(Settings::parse("dictation_key = \"f13\"").unwrap().dictation_key, DictationKey::F13);
    }

    #[test]
    fn a_broken_file_is_not_overwritten() {
        let path = temp("broken");
        std::fs::write(&path, "lists = maybe\n").unwrap();
        assert!(Settings::default().save(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "lists = maybe\n");
    }
}
