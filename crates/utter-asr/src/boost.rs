//! Phrase boosting: the user's dictionary steers greedy decoding toward its words, without
//! changing the model (docs/plan-final-text.md, "Vocabulary"; NVIDIA's GPU phrase boosting,
//! arXiv 2508.07014, is the same idea for NeMo).
//!
//! The phrases become a tree over token sequences. At every decoding step, a token that
//! continues a phrase matched so far gets a bonus added to its log-probability, growing with the
//! depth of the match so a phrase half-spoken is pulled through to its end; starting a phrase
//! gets a smaller one (`start_score`). Whether a token is emitted at all stays the model's
//! decision (the decoder compares blank with the unboosted scores): a bonus that beat blank
//! would write words nobody said. Measured in docs/eval-results.md, "Phrase boosting".
//!
//! The model's vocabulary ships without piece scores, so the exact split SentencePiece would
//! make is unknown. Every split of a phrase into vocabulary pieces goes into the tree instead
//! (capped per phrase): whichever the model takes is boosted.

use std::collections::{HashMap, HashSet};

use crate::vocab::Vocab;

/// How strongly phrases are boosted.
#[derive(Debug, Clone, Copy)]
pub struct BoostConfig {
    /// Bonus, in log-probability, for a token that starts a phrase. Kept small: greedy decoding
    /// cannot take a start back when the rest does not follow, and a phrase's first pieces are
    /// often a common word's ("▁U" of "UMAP" turns "um" into "UM").
    pub start_score: f32,
    /// Bonus for a token that continues a phrase already started.
    pub score: f32,
    /// Added per level of depth past the second token, so the rest of a phrase is followed
    /// through.
    pub depth_bonus: f32,
    /// The most splits of one phrase into pieces kept in the tree.
    pub max_splits: usize,
}

impl Default for BoostConfig {
    fn default() -> Self {
        Self { start_score: 0.0, score: 1.5, depth_bonus: 0.5, max_splits: 64 }
    }
}

#[derive(Debug, Default)]
struct Node {
    children: HashMap<u32, usize>,
    depth: u32,
}

/// Where decoding stands in the phrase tree: a node id (0 is the root, no match under way).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State(usize);

#[derive(Debug)]
pub struct PhraseBoost {
    nodes: Vec<Node>,
    cfg: BoostConfig,
}

impl PhraseBoost {
    /// Builds the tree for `phrases` over `vocab`'s pieces; phrases that cannot be written in
    /// the vocabulary's pieces are skipped. Returns None when no phrase is usable.
    pub fn new(vocab: &Vocab, phrases: &[String], cfg: BoostConfig) -> Option<Self> {
        Self::build(vocab, phrases, cfg).0
    }

    /// [`Self::new`], also counting the phrases that could be spelled in the pieces.
    pub fn build(vocab: &Vocab, phrases: &[String], cfg: BoostConfig) -> (Option<Self>, usize) {
        let index = vocab.piece_index();
        let mut boost = Self { nodes: vec![Node::default()], cfg };
        let mut usable = 0;
        for phrase in phrases {
            let mut spelled = false;
            for variant in variants(phrase) {
                for split in splits(&variant, &index, cfg.max_splits) {
                    boost.insert(&split);
                    spelled = true;
                }
            }
            usable += spelled as usize;
        }
        ((boost.nodes.len() > 1).then_some(boost), usable)
    }

    fn insert(&mut self, tokens: &[u32]) {
        let mut at = 0;
        for &t in tokens {
            at = match self.nodes[at].children.get(&t) {
                Some(&next) => next,
                None => {
                    let depth = self.nodes[at].depth + 1;
                    self.nodes.push(Node { children: HashMap::new(), depth });
                    let next = self.nodes.len() - 1;
                    self.nodes[at].children.insert(t, next);
                    next
                }
            };
        }
    }

    /// The bonus for emitting `token` in `state`: for continuing the match under way, or else
    /// for starting a phrase.
    pub fn bonus(&self, state: State, token: u32) -> f32 {
        let next = self.nodes[state.0].children.get(&token).or_else(|| self.nodes[0].children.get(&token));
        match next.map(|&n| self.nodes[n].depth) {
            Some(1) => self.cfg.start_score,
            Some(depth) => self.cfg.score + self.cfg.depth_bonus * (depth - 2) as f32,
            None => 0.0,
        }
    }

    /// The state after emitting `token`.
    pub fn advance(&self, state: State, token: u32) -> State {
        match self.nodes[state.0].children.get(&token).or_else(|| self.nodes[0].children.get(&token)) {
            Some(&n) => State(n),
            None => State(0),
        }
    }

    /// Adds the bonus to each boosted token's log-probability in `logp`, once (blank excluded).
    pub fn apply(&self, state: State, logp: &mut [f32], blank: usize) {
        let mut seen = HashSet::new();
        let tokens = self.nodes[state.0].children.keys().chain(self.nodes[0].children.keys());
        for &token in tokens {
            let t = token as usize;
            // A token reachable both from the match under way and from the root gets the bonus
            // once: `bonus` prefers continuing the match.
            if t != blank && t < logp.len() && seen.insert(token) {
                logp[t] += self.bonus(state, token);
            }
        }
    }
}

/// The written forms a phrase may come out as: as given, and with a capital first letter (at
/// the start of a sentence). Each is one word-initial string in SentencePiece's spelling.
fn variants(phrase: &str) -> Vec<String> {
    let words: Vec<&str> = phrase.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let spelled = |ws: &[String]| ws.iter().map(|w| format!("\u{2581}{w}")).collect::<String>();
    let given: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
    let mut capital = given.clone();
    let mut chars = capital[0].chars();
    if let Some(first) = chars.next() {
        capital[0] = first.to_uppercase().chain(chars).collect();
    }
    let mut out = vec![spelled(&given)];
    if capital != given {
        out.push(spelled(&capital));
    }
    out
}

/// Every split of `text` into vocabulary pieces, longest pieces first, at most `max`.
fn splits(text: &str, index: &HashMap<&str, u32>, max: usize) -> Vec<Vec<u32>> {
    let longest = index.keys().map(|p| p.len()).max().unwrap_or(0);
    let mut out = Vec::new();
    let mut path = Vec::new();
    fn walk(
        rest: &str,
        index: &HashMap<&str, u32>,
        longest: usize,
        max: usize,
        path: &mut Vec<u32>,
        out: &mut Vec<Vec<u32>>,
    ) {
        if out.len() >= max {
            return;
        }
        if rest.is_empty() {
            out.push(path.clone());
            return;
        }
        let ends: Vec<usize> = rest.char_indices().map(|(i, c)| i + c.len_utf8()).filter(|&e| e <= longest).collect();
        for &end in ends.iter().rev() {
            if let Some(&id) = index.get(&rest[..end]) {
                path.push(id);
                walk(&rest[end..], index, longest, max, path, out);
                path.pop();
            }
        }
    }
    walk(text, index, longest, max, &mut path, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocab {
        // ids:          0       1      2       3     4      5        6       7
        let pieces =
            ["\u{2581}S", "QL", "\u{2581}SQL", "\u{2581}Sent", "ry", "\u{2581}the", "\u{2581}s", "\u{2581}Sentry"];
        Vocab::from_pieces(pieces.iter().map(|s| (*s).to_owned()).collect())
    }

    #[test]
    fn every_split_of_a_phrase_is_in_the_tree() {
        let b = PhraseBoost::new(&vocab(), &["SQL Sentry".to_owned()], BoostConfig::default()).unwrap();
        // "▁SQL" then "▁Sentry" or "▁Sent" "ry"; and "▁S" "QL" ...
        for start in [2, 0] {
            assert_ne!(b.advance(State(0), start), State(0), "a phrase can start with token {start}");
        }
        let after_sql = b.advance(State(0), 2);
        assert!(b.bonus(after_sql, 7) > 0.0);
        assert!(b.bonus(after_sql, 3) > 0.0);
    }

    #[test]
    fn starting_gets_the_start_score_and_continuing_more_with_depth() {
        let cfg = BoostConfig { start_score: 0.25, ..BoostConfig::default() };
        let b = PhraseBoost::new(&vocab(), &["SQL Sentry SQL".to_owned()], cfg).unwrap();
        assert_eq!(b.bonus(State(0), 2), 0.25);
        let s = b.advance(State(0), 2);
        assert_eq!(b.bonus(s, 7), cfg.score);
        let s = b.advance(s, 7);
        assert_eq!(b.bonus(s, 2), cfg.score + cfg.depth_bonus);
    }

    #[test]
    fn deeper_tokens_get_more_and_unrelated_tokens_nothing() {
        let b = PhraseBoost::new(&vocab(), &["SQL Sentry".to_owned()], BoostConfig::default()).unwrap();
        let first = b.bonus(State(0), 2);
        let s = b.advance(State(0), 2);
        assert!(b.bonus(s, 7) > first);
        assert_eq!(b.bonus(State(0), 5), 0.0); // "▁the"
        assert_eq!(b.advance(s, 5), State(0)); // a match broken returns to the root
    }

    #[test]
    fn the_blank_is_never_boosted() {
        let cfg = BoostConfig { start_score: 1.0, ..BoostConfig::default() };
        let b = PhraseBoost::new(&vocab(), &["SQL Sentry".to_owned()], cfg).unwrap();
        let mut logp = vec![-5.0f32; 9];
        b.apply(State(0), &mut logp, 8);
        assert_eq!(logp[8], -5.0);
        assert!(logp[2] > -5.0);
        assert_eq!(logp[5], -5.0);
    }

    #[test]
    fn a_token_reachable_two_ways_is_boosted_once() {
        // "▁SQL" continues a match after "▁SQL" (phrase "SQL SQL") and also starts one.
        let b = PhraseBoost::new(&vocab(), &["SQL SQL".to_owned()], BoostConfig::default()).unwrap();
        let s = b.advance(State(0), 2);
        let mut logp = vec![0.0f32; 9];
        b.apply(s, &mut logp, 8);
        assert_eq!(logp[2], b.bonus(s, 2));
    }

    #[test]
    fn phrases_not_in_the_vocabulary_give_nothing() {
        assert!(PhraseBoost::new(&vocab(), &["xyzzy".to_owned()], BoostConfig::default()).is_none());
    }
}
