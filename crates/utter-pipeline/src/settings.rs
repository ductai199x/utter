//! The user's settings, in `<data_dir>/settings.toml` (DESIGN.md section 11): a file the Home
//! window edits and a person can edit too (docs/ux/home.md). Missing keys take their defaults;
//! a missing file is written with every key commented, so there is something to edit. Saving
//! changes only the values, so a person's comments and layout survive.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use utter_llm::cleanup::Tier;

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

# The microphone: part of its name, or \"\" for the system default.
microphone = \"\"

# The cleanup model: \"light\" (fast, about 6 GB of GPU memory) or \"quality\" (about 18 GB).
model = \"light\"
";

/// The most dictionary entries (docs/ux/home.md).
pub const MAX_TERMS: usize = 2000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub dictionary: Vec<String>,
    pub lists: bool,
    /// Part of the input device's name; empty for the system default.
    pub microphone: String,
    pub model: Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    #[default]
    Light,
    Quality,
}

impl From<Model> for Tier {
    fn from(m: Model) -> Self {
        match m {
            Model::Light => Tier::Light,
            Model::Quality => Tier::Quality,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self { dictionary: Vec::new(), lists: true, microphone: String::new(), model: Model::Light }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        utter_core::data_dir().join("settings.toml")
    }

    /// Reads the settings file, writing the commented default first if there is none.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(path, TEMPLATE).map_err(|e| format!("{}: {e}", path.display()))?;
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
            TEMPLATE.to_owned()
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
        set(&mut doc, "microphone", self.microphone.as_str().into());
        set(&mut doc, "model", (if self.model == Model::Quality { "quality" } else { "light" }).into());
        // Write then rename, so a crash never leaves half a file.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn apply(&self, cfg: &mut PipelineConfig) {
        cfg.dictionary = self.dictionary.clone();
        cfg.lists = self.lists;
        cfg.mic.device = (!self.microphone.is_empty()).then(|| self.microphone.clone());
        cfg.tier = self.model.into();
    }
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
        assert_eq!(Settings::parse(TEMPLATE).unwrap(), Settings::default());
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
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
    }

    #[test]
    fn saving_keeps_comments_and_round_trips() {
        let path = temp("save");
        std::fs::write(&path, "# mine\nlists = true # keep this\n").unwrap();
        let s = Settings {
            dictionary: vec!["Supabase".into(), "Nguyen".into()],
            lists: false,
            microphone: "USB".into(),
            model: Model::Quality,
        };
        s.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine\nlists = false # keep this\n"), "{text}");
        assert_eq!(Settings::parse(&text).unwrap(), s);
    }

    #[test]
    fn a_broken_file_is_not_overwritten() {
        let path = temp("broken");
        std::fs::write(&path, "lists = maybe\n").unwrap();
        assert!(Settings::default().save(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "lists = maybe\n");
    }
}
