//! The cleanup gate (DESIGN.md section 4, step 6): does a transcript need the language model?
//!
//! The speech model already punctuates and capitalizes. A transcript with none of the things
//! cleanup exists for (docs/cleanup-spec.md) goes straight to insertion, saving the model's
//! 100-200 ms. The rules lean towards sending text to the model: a needless cleanup costs
//! time, a skipped one leaves a filler or an uncorrected "2, no, 3" in the user's text.
//! Measured against the curated cleanup cases in this module's tests.

/// Why a transcript goes to the model, or `None` when it can skip it. `dictionary` is the
/// user's word list: a term the speech model may have heard as something close to it
/// ("super base" for "Supabase") needs the model to spell it.
pub fn reason(text: &str, dictionary: &[String]) -> Option<&'static str> {
    let lower = text.to_lowercase();
    let words: Vec<&str> =
        lower.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-')).filter(|w| !w.is_empty()).collect();

    const FILLERS: &[&str] = &["um", "uh", "er", "erm", "ah", "hmm", "mm", "uhm", "eh"];
    if words.iter().any(|w| FILLERS.contains(w)) {
        return Some("filler");
    }
    if words.windows(2).any(|p| p[0] == p[1] && !ALLOWED_DOUBLES.contains(&p[0])) {
        return Some("repeated word");
    }
    // A repeated phrase: "can you can you", "I think we I think we".
    for n in 2..=3 {
        if words.windows(2 * n).any(|w| w[..n] == w[n..]) {
            return Some("repeated word");
        }
    }
    if dictionary.iter().any(|term| sounds_present(term, &words)) {
        return Some("dictionary");
    }
    if words.iter().any(|w| w.ends_with('-') && w.len() > 1) {
        return Some("false start");
    }
    const PHRASES: &[(&str, &str)] = &[
        // Corrections.
        ("no wait", "correction"),
        ("no, wait", "correction"),
        ("i mean", "correction"),
        ("actually", "correction"),
        ("sorry,", "correction"),
        ("scratch that", "correction"),
        ("never mind", "correction"),
        ("make that", "correction"),
        (", no,", "correction"),
        ("or rather", "correction"),
        // Filler phrases.
        ("you know", "filler"),
        ("like,", "filler"),
        (", like", "filler"),
        ("kind of like", "filler"),
        ("sort of", "filler"),
        // Spoken formatting.
        ("new line", "spoken formatting"),
        ("new paragraph", "spoken formatting"),
        ("question mark", "spoken formatting"),
        ("exclamation point", "spoken formatting"),
        ("exclamation mark", "spoken formatting"),
        ("full stop", "spoken formatting"),
        // Lists.
        ("first,", "list"),
        ("second,", "list"),
        ("number one", "list"),
        // Addresses.
        (" dot ", "address"),
        (" at gmail", "address"),
        (" slash ", "address"),
    ];
    for (phrase, why) in PHRASES {
        if lower.contains(phrase) {
            return Some(why);
        }
    }
    // A spoken "comma", "period" or "colon" with no punctuation around it is a command.
    for cmd in ["comma", "period", "colon"] {
        if words.contains(&cmd) && !text.contains([',', '.', ':']) {
            return Some("spoken formatting");
        }
    }
    // Numbers spoken as words that cleanup writes as digits: units, money, percentages, times.
    if words.iter().any(|w| NUMBER_WORDS.contains(w)) {
        const TRIGGERS: &[&str] = &[
            "percent",
            "dollars",
            "dollar",
            "euros",
            "cents",
            "pm",
            "am",
            "o'clock",
            "thousand",
            "hundred",
            "million",
            "gigabytes",
            "megabytes",
            "minutes",
            "seconds",
            "hours",
            "point",
            "thirty",
            "fifteen",
            "forty-five",
        ];
        if words.iter().any(|w| TRIGGERS.contains(w)) {
            return Some("number");
        }
        // A list counted out loud: "one, ... two, ...".
        if lower.contains("one,") && lower.contains("two,") {
            return Some("list");
        }
    }
    // Dates spoken with ordinals: "March fourth", "the twelfth of May".
    if words.iter().any(|w| ORDINALS.contains(w)) && words.iter().any(|w| MONTHS.contains(w)) {
        return Some("number");
    }
    None
}

/// The dictionary terms cleanup should see for this transcript: those heard as spoken, or as
/// something close. A user's whole dictionary in every request would cost prefill time and
/// invite the model to write terms nobody said.
pub fn relevant_terms(text: &str, dictionary: &[String]) -> Vec<String> {
    let lower = text.to_lowercase();
    let words: Vec<&str> =
        lower.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-')).filter(|w| !w.is_empty()).collect();
    dictionary.iter().filter(|t| heard(t, &words, true)).cloned().collect()
}

/// Whether a dictionary term, or something within a small edit distance of it, appears in the
/// transcript as one to three consecutive words joined ("super base" -> "superbase" is close
/// to "supabase"). Already spelled exactly as in the dictionary, it needs no model.
fn sounds_present(term: &str, words: &[&str]) -> bool {
    heard(term, words, false)
}

/// `exact`: whether a term heard letter for letter (case aside) counts.
fn heard(term: &str, words: &[&str], exact: bool) -> bool {
    let letters =
        |s: &str| -> String { s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect() };
    let target = letters(term);
    if target.len() < 3 {
        return false;
    }
    for n in 1..=3 {
        for w in words.windows(n) {
            let joined: String = w.iter().map(|x| letters(x)).collect();
            if joined.is_empty() {
                continue;
            }
            let d = edit_distance(&joined, &target);
            // Exact (case-insensitive) is fine unless the dictionary's case differs; close but
            // not equal needs the model.
            if d == 0 {
                if exact {
                    return true;
                }
                continue;
            }
            if (d as f32) / (target.chars().count().max(joined.chars().count()) as f32) <= 0.34 {
                return true;
            }
        }
    }
    false
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Words that are fine twice in a row ("she had had enough", "very very").
const ALLOWED_DOUBLES: &[&str] = &["had", "very", "bye", "no", "so", "really", "ha", "go"];

const ORDINALS: &[&str] = &[
    "first",
    "second",
    "third",
    "fourth",
    "fifth",
    "sixth",
    "seventh",
    "eighth",
    "ninth",
    "tenth",
    "eleventh",
    "twelfth",
    "thirteenth",
    "fourteenth",
    "fifteenth",
    "sixteenth",
    "seventeenth",
    "eighteenth",
    "nineteenth",
    "twentieth",
    "thirtieth",
    "thirty-first",
];

const MONTHS: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const NUMBER_WORDS: &[&str] = &[
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
    "hundred",
    "thousand",
    "million",
    "twenty-five",
    "forty-five",
];

#[cfg(test)]
mod tests {
    use super::{reason, relevant_terms};

    #[test]
    fn cleanup_sees_only_the_terms_heard() {
        let dictionary: Vec<String> = ["Supabase", "Kubernetes", "Nguyen", "UMAP"].map(String::from).to_vec();
        assert_eq!(
            relevant_terms("we moved the auth to super base and kubernetes", &dictionary),
            ["Supabase", "Kubernetes"]
        );
        assert!(relevant_terms("lunch at noon?", &dictionary).is_empty());
    }

    #[test]
    fn clean_text_skips_the_model() {
        for t in [
            "The quarterly report is attached. Let me know if you have any questions.",
            "Could you review my pull request when you get a chance?",
            "Sounds good, see you then.",
            "She had had enough of the delays.",
            "I like the new design a lot.",
        ] {
            assert_eq!(reason(t, &[]), None, "{t}");
        }
    }

    #[test]
    fn each_cleanup_reason_is_caught() {
        for (t, why) in [
            ("Um, so I think we should push it.", "filler"),
            ("The the problem is the cache.", "repeated word"),
            ("Let's meet at two, no, wait, three PM.", "correction"),
            ("The deadline is Tuesday, I mean Wednesday.", "correction"),
            ("Dear team comma new line hello", "spoken formatting"),
            ("My email is john dot smith at gmail dot com.", "address"),
            ("It costs twenty five dollars per month.", "number"),
            ("I was, like, really surprised.", "filler"),
        ] {
            assert_eq!(reason(t, &[]), Some(why), "{t}");
        }
    }

    /// The curated cleanup cases (tools/eval/cleanup_cases.jsonl): the gate must not skip a
    /// case whose cleanup changes words, in the categories where a skip leaves a visible error.
    #[test]
    fn curated_cases_needing_cleanup_are_not_skipped() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/eval/cleanup_cases.jsonl");
        let text = std::fs::read_to_string(path).expect("cleanup cases");
        let words = |s: &str| -> Vec<String> {
            s.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .map(str::to_owned)
                .collect()
        };
        let (mut skipped_clean, mut clean, mut missed) = (0, 0, Vec::new());
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let case: serde_json::Value = serde_json::from_str(line).unwrap();
            let (id, input, expected) =
                (case["id"].as_str().unwrap(), case["input"].as_str().unwrap(), case["expected"].as_str().unwrap());
            let needs_words_changed = words(input) != words(expected);
            let dictionary: Vec<String> = case["dictionary"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            let skipped = reason(input, &dictionary).is_none();
            if !needs_words_changed {
                clean += 1;
                skipped_clean += skipped as usize;
            } else if skipped {
                missed.push(id.to_owned());
            }
        }
        println!("gate: skips {skipped_clean} of {clean} cases that need no word changes; missed: {missed:?}");
        let critical = ["filler", "backtrack", "repeat", "list", "spoken", "numbers", "dict"];
        let critical_missed: Vec<_> = missed.iter().filter(|id| critical.iter().any(|c| id.starts_with(c))).collect();
        assert!(critical_missed.is_empty(), "cases needing cleanup that the gate skips: {critical_missed:?}");
    }
}
