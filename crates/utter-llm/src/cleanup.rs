//! The cleanup request: the system prompt (`prompts/cleanup.md`, the rules of
//! docs/cleanup-spec.md) and the user message carrying the transcript and the dictionary.
//!
//! The system prompt never changes between requests and comes first, so llama-server keeps
//! it in its prompt cache: after the first request, only the transcript is new work.

use std::path::PathBuf;

use crate::{LlmError, Message, ServerOptions};

/// The cleanup system prompt.
pub const SYSTEM_PROMPT: &str = include_str!("../prompts/cleanup.md");

/// The system prompt the fine-tuned cleanup model is trained with (tools/train,
/// eval.finaltext.trainset --system): short, since the rules are in its weights. That model
/// must be sent exactly this.
pub const FINE_TUNED_PROMPT: &str = include_str!("../prompts/cleanup-ft.md");

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
    messages_with(SYSTEM_PROMPT, transcript, dictionary, &Context::default())
}

/// Where the dictation goes, as far as the platform can tell (docs/plan-use-cases.md, "The
/// request grows a context block"). Everything is optional; the default writes no lines, so
/// the request is then exactly the plain one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// The kind of app: "email", "chat", "document", "code", "terminal", "ai-prompt", "other".
    pub app: Option<String>,
    /// The field takes one line (a search box, a chat input): no line breaks or lists.
    pub single_line: bool,
    /// Text before the cursor (the end of it), so a dictation continues it.
    pub before: Option<String>,
    /// Text after the cursor (the start of it).
    pub after: Option<String>,
    /// Spoken lists stay prose (a setting; lists are formatted by default).
    pub lists_off: bool,
}

/// How much of the text around the cursor goes with a request.
pub const BEFORE_CHARS: usize = 200;
pub const AFTER_CHARS: usize = 50;

/// [`messages`] with another system prompt and a context: the one place the request format is
/// defined (tools/eval/finaltext/trainset.py mirrors it for training).
pub fn messages_with(system: &str, transcript: &str, dictionary: &[String], ctx: &Context) -> [Message; 2] {
    let mut user = String::new();
    if let Some(app) = &ctx.app {
        user.push_str(&format!("App: {app}\n"));
    }
    if ctx.single_line {
        user.push_str("Field: single-line\n");
    }
    // Quoted as JSON strings: the text may hold line breaks and quotes of its own.
    if let Some(before) = ctx.before.as_deref().filter(|s| !s.is_empty()) {
        let tail: String = before.chars().rev().take(BEFORE_CHARS).collect::<Vec<_>>().into_iter().rev().collect();
        user.push_str(&format!("Before: {}\n", serde_json::to_string(&tail).expect("a string serializes")));
    }
    if let Some(after) = ctx.after.as_deref().filter(|s| !s.is_empty()) {
        let head: String = after.chars().take(AFTER_CHARS).collect();
        user.push_str(&format!("After: {}\n", serde_json::to_string(&head).expect("a string serializes")));
    }
    if ctx.lists_off {
        user.push_str("Lists: off\n");
    }
    if !dictionary.is_empty() {
        user.push_str("Dictionary: ");
        user.push_str(&dictionary.join(", "));
        user.push('\n');
    }
    user.push_str("Dictated: ");
    user.push_str(transcript.trim());
    [Message::system(system), Message::user(user)]
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
    fn context_lines_come_first_in_a_fixed_order() {
        let ctx = Context {
            app: Some("email".into()),
            single_line: true,
            before: Some("Hi Sam,\nI think we \"should\"".into()),
            after: Some(" today.".into()),
            lists_off: true,
        };
        let [_, user] = messages_with(SYSTEM_PROMPT, "move it", &["Utter".into()], &ctx);
        assert_eq!(
            user.content,
            "App: email\nField: single-line\nBefore: \"Hi Sam,\\nI think we \\\"should\\\"\"\n\
             After: \" today.\"\nLists: off\nDictionary: Utter\nDictated: move it"
        );
    }

    #[test]
    fn only_the_end_of_the_text_before_the_cursor_is_sent() {
        let ctx = Context { before: Some("x".repeat(BEFORE_CHARS + 10) + "end"), ..Context::default() };
        let [_, user] = messages_with(SYSTEM_PROMPT, "hi", &[], &ctx);
        let before = user.content.lines().next().unwrap();
        assert!(before.ends_with("end\""));
        assert_eq!(before.len(), "Before: \"\"".len() + BEFORE_CHARS);
    }

    #[test]
    fn labels_and_wrapping_quotes_are_removed() {
        assert_eq!(tidy_output("Cleaned: I think so."), "I think so.");
        assert_eq!(tidy_output("\"I think so.\""), "I think so.");
        assert_eq!(tidy_output("He said \"yes\" and \"no\""), "He said \"yes\" and \"no\"");
    }
}
