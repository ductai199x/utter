//! The cleanup request: the system prompt (`prompts/cleanup.md`, the rules of
//! docs/cleanup-spec.md) and the user message carrying the transcript and the dictionary.
//!
//! The system prompt never changes between requests and comes first, so llama-server keeps
//! it in its prompt cache: after the first request, only the transcript is new work.

use std::path::PathBuf;

use crate::{LlmError, Message, ServerOptions};

/// The cleanup system prompt.
pub const SYSTEM_PROMPT: &str = include_str!("../prompts/cleanup.md");

/// Which cleanup model runs. Both are Google's Gemma 4 QAT 4-bit builds, chosen by measurement
/// (docs/eval-results.md); both run with [`SERVER_ARGS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tier {
    /// Gemma 4 E4B: about 3.3 GB of GPU memory (its 2.7 GB of per-layer embeddings stay in
    /// system RAM), 97% of the cleanup checks, 124 ms p50 on real speech. The default: with
    /// the speech model, Utter takes about 5.6 GB of GPU memory in all (Tai, 2026-10-09).
    #[default]
    Light,
    /// Gemma 4 26B-A4B: about 15 GB of GPU memory, 99% of the checks, 137 ms p50. Optional, for
    /// GPUs with room.
    Quality,
}

impl Tier {
    /// The runtime component holding the model (crates/utter-runtime/components.json).
    pub fn component(self) -> &'static str {
        match self {
            Tier::Light => "gemma4-e4b-qat",
            Tier::Quality => "gemma4-26b-a4b-qat",
        }
    }

    pub fn model_file(self) -> &'static str {
        match self {
            Tier::Light => "gemma-4-E4B_q4_0-it.gguf",
            Tier::Quality => "gemma-4-26B_q4_0-it.gguf",
        }
    }
}

/// llama-server settings measured for these models (docs/eval-results.md, 2026-10-09):
/// - `--swa-full`: a full-size cache for the sliding-window layers. Without it every request
///   rebuilds that cache from a checkpoint; with it, prompt processing drops from about 95 ms
///   to 25 ms. Wall time per cleanup halves.
/// - n-gram drafting from the prompt, 3-token match, 6-token draft: cleanup copies most of its
///   input, so drafts are accepted about 60% of the time. Another 25% off on real speech;
///   outputs are the same as without it (all curated cases, 97% of real ones; the rest differ
///   by float rounding of batched verification).
pub const SERVER_ARGS: &[&str] = &[
    "--swa-full",
    "--spec-type",
    "ngram-simple",
    "--spec-ngram-simple-size-n",
    "3",
    "--spec-ngram-simple-size-m",
    "6",
];

/// The installed model file of `tier`, or an error naming how to install it.
pub fn installed_model(tier: Tier) -> Result<PathBuf, LlmError> {
    Ok(utter_runtime::component(tier.component())?.installed_dir()?.join(tier.model_file()))
}

/// Server options for `tier`'s installed model with its measured settings.
pub fn server_options(tier: Tier) -> Result<ServerOptions, LlmError> {
    let mut opts = ServerOptions::new(installed_model(tier)?);
    opts.extra_args = SERVER_ARGS.iter().map(|s| (*s).to_owned()).collect();
    Ok(opts)
}

/// The messages for cleaning up `transcript`, with the user's dictionary words.
pub fn messages(transcript: &str, dictionary: &[String]) -> [Message; 2] {
    let mut user = String::new();
    if !dictionary.is_empty() {
        user.push_str("Dictionary: ");
        user.push_str(&dictionary.join(", "));
        user.push('\n');
    }
    user.push_str("Dictated: ");
    user.push_str(transcript.trim());
    [Message::system(SYSTEM_PROMPT), Message::user(user)]
}

/// What a model returns, minus wrapping some models add despite the prompt: a leading
/// "Cleaned:" label, or quotes around the whole text.
pub fn tidy_output(raw: &str) -> String {
    let mut text = raw.trim();
    if let Some(rest) = text.strip_prefix("Cleaned:") {
        text = rest.trim_start();
    }
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') && !text[1..text.len() - 1].contains('"') {
        text = &text[1..text.len() - 1];
    }
    text.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictionary_comes_before_the_transcript() {
        let [system, user] = messages("  hello there ", &["Utter".into(), "Nguyen".into()]);
        assert_eq!(system.role, "system");
        assert_eq!(user.content, "Dictionary: Utter, Nguyen\nDictated: hello there");
    }

    #[test]
    fn no_dictionary_line_without_words() {
        let [_, user] = messages("hi", &[]);
        assert_eq!(user.content, "Dictated: hi");
    }

    #[test]
    fn labels_and_wrapping_quotes_are_removed() {
        assert_eq!(tidy_output("Cleaned: I think so."), "I think so.");
        assert_eq!(tidy_output("\"I think so.\""), "I think so.");
        assert_eq!(tidy_output("He said \"yes\" and \"no\""), "He said \"yes\" and \"no\"");
    }
}
