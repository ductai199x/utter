//! The model's vocabulary and how token ids become text.

use std::path::Path;

use crate::AsrError;

/// SentencePiece pieces by id. "▁" (U+2581) marks the start of a word.
pub struct Vocab {
    pieces: Vec<String>,
}

impl Vocab {
    pub fn load(dir: &Path, size: usize) -> Result<Self, AsrError> {
        let path = dir.join("vocab.txt");
        let text = std::fs::read_to_string(&path).map_err(|e| AsrError::io(&path, e))?;
        let pieces: Vec<String> = text.split('\n').take(size).map(str::to_owned).collect();
        if pieces.len() != size {
            return Err(AsrError::Model(format!("{}: {} tokens, expected {size}", path.display(), pieces.len())));
        }
        Ok(Self { pieces })
    }

    pub fn from_pieces(pieces: Vec<String>) -> Self {
        Self { pieces }
    }

    /// Piece text to id, for spelling phrases in the model's pieces.
    pub fn piece_index(&self) -> std::collections::HashMap<&str, u32> {
        self.pieces.iter().enumerate().map(|(i, p)| (p.as_str(), i as u32)).collect()
    }

    /// Joins the pieces and turns each "▁" into a space, trimming the ends. Checked against
    /// Transformers' decode on the eval clips (`tools/export/check_parakeet.py`).
    pub fn decode(&self, ids: &[u32]) -> String {
        let mut text = String::new();
        for &id in ids {
            if let Some(piece) = self.pieces.get(id as usize) {
                text.push_str(piece);
            }
        }
        text.replace('\u{2581}', " ").trim().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_starts_become_spaces_and_ends_are_trimmed() {
        let v = Vocab::from_pieces(["▁Hel", "lo", ",", "▁world", "."].map(String::from).to_vec());
        assert_eq!(v.decode(&[0, 1, 2, 3, 4]), "Hello, world.");
    }

    #[test]
    fn unknown_ids_are_skipped() {
        let v = Vocab::from_pieces(vec!["▁a".into()]);
        assert_eq!(v.decode(&[0, 7]), "a");
    }
}
