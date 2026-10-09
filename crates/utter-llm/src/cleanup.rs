//! The cleanup request: the system prompt (`prompts/cleanup.md`, the rules of
//! docs/cleanup-spec.md) and the user message carrying the transcript and the dictionary.
//!
//! The system prompt never changes between requests and comes first, so llama-server keeps
//! it in its prompt cache: after the first request, only the transcript is new work.

use crate::Message;

/// The cleanup system prompt.
pub const SYSTEM_PROMPT: &str = include_str!("../prompts/cleanup.md");

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
