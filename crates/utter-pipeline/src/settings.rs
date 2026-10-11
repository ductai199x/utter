//! The user's settings, in `<data_dir>/settings.toml` (DESIGN.md section 11): a file the Home
//! window will edit and a person can edit too. Missing keys take their defaults; a missing file
//! is written with every key commented, so there is something to edit.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::PipelineConfig;

/// What a new settings file says: every key, at its default, with what it does.
const TEMPLATE: &str = "\
# Utter's settings. Changes apply when Utter starts.

# Names, product names and terms Utter should spell your way. They steer recognition toward
# these words and tell cleanup how to write them.
dictionary = []
# dictionary = [\"Supabase\", \"Kubernetes\", \"Nguyen\"]

# Spoken lists (\"first..., second...\", or items read one after another) become bullets.
lists = true
";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub dictionary: Vec<String>,
    pub lists: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { dictionary: Vec::new(), lists: true }
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
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut s: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        // Blank and repeated terms would only add noise to the prompt and the phrase tree.
        let mut seen = std::collections::HashSet::new();
        s.dictionary = s
            .dictionary
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty() && seen.insert(t.clone()))
            .collect();
        Ok(s)
    }

    pub fn apply(&self, cfg: &mut PipelineConfig) {
        cfg.dictionary = self.dictionary.clone();
        cfg.lists = self.lists;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_parses_to_the_defaults() {
        assert_eq!(Settings::parse(TEMPLATE).unwrap(), Settings::default());
    }

    #[test]
    fn terms_are_trimmed_and_deduplicated() {
        let s =
            Settings::parse("dictionary = [\" Supabase \", \"\", \"Supabase\", \"Nguyen\"]\nlists = false").unwrap();
        assert_eq!(s.dictionary, ["Supabase", "Nguyen"]);
        assert!(!s.lists);
    }

    #[test]
    fn a_misspelled_key_is_an_error_not_a_silent_default() {
        assert!(Settings::parse("dictonary = [\"x\"]").is_err());
    }

    #[test]
    fn a_missing_file_is_created_with_the_template() {
        let dir = std::env::temp_dir().join(format!("utter-settings-{}", std::process::id()));
        let path = dir.join("settings.toml");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Settings::load_or_create(&path).unwrap(), Settings::default());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
