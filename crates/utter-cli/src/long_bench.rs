//! `utter-cli long-bench`: long dictations through the cleanup window (utter_pipeline::window),
//! to choose its size. Long dictations are made from the cleanup eval set (eval/text-test), each
//! item a dictated text with its reference final text, chained; the dictated text arrives a
//! sentence at a time, as the app's segments do, and every arrival is cleaned as the app cleans
//! it: the window's part, appended to the settled start. The last sentence, said as the key goes
//! up, is cleaned once per key-up window (Window::split_final), the cleanup the person waits for.
//! Each live window size runs every dictation; the results (texts, request times and sizes, per
//! key-up window) go to results/long/<name>/<window>.jsonl for tools/ to score.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use utter_llm::LlamaServer;
use utter_llm::cleanup::{self, Tier};
use utter_pipeline::window::{Window, joined};

/// Speaking pace for the keep-up check: a request is late when it takes longer than the next
/// sentence takes to say.
const WORDS_PER_SECOND: f64 = 2.5;

pub struct Options {
    pub tier: Tier,
    pub items: usize,
    pub dictations: usize,
    pub windows: Vec<usize>,
    /// Key-up window sizes (a size wider than the dictation: the live window's own split).
    pub finals: Vec<usize>,
    pub name: String,
}

struct Item {
    dictated: String,
    final_text: String,
}

/// The dictated text cut where the speech model's segments would be: at sentence ends.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        current = joined(&current, word);
        if word.ends_with(['.', '?', '!']) {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn items(eval: &Path) -> Result<Vec<Item>> {
    let path = eval.join("text-test").join("manifest.jsonl");
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l)?;
            Ok(Item {
                dictated: v["dictated"].as_str().unwrap_or_default().to_owned(),
                final_text: v["final_text"].as_str().unwrap_or_default().to_owned(),
            })
        })
        .collect()
}

pub fn run(opts: &Options) -> Result<()> {
    let eval = utter_core::data_dir().join("eval");
    let all = items(&eval)?;
    // Spread over the set (it is grouped by category): every dictation mixes kinds of speech.
    let step = (all.len() / (opts.items * opts.dictations).max(1)).max(1);
    let picked: Vec<&Item> = all.iter().step_by(step).take(opts.items * opts.dictations).collect();
    let dictations: Vec<&[&Item]> = picked.chunks(opts.items).filter(|c| c.len() == opts.items).collect();
    let out_dir = eval.join("results").join("long").join(&opts.name);
    std::fs::create_dir_all(&out_dir)?;

    let server = LlamaServer::start(&cleanup::server_options(opts.tier)?).context("starting the cleanup model")?;
    let system = opts.tier.system_prompt();
    let ctx = cleanup::Context::default();
    // Warm, as the app does before its first dictation.
    let _ = utter_pipeline::clean_once(&server, system, "Um, this is a warm-up.", &[], &ctx);

    for &words in &opts.windows {
        let path = out_dir.join(format!("{words}.jsonl"));
        let mut out = std::fs::File::create(&path)?;
        let mut all_ms = Vec::new();
        let mut finals: Vec<Vec<f64>> = vec![Vec::new(); opts.finals.len()];
        let (mut late, mut failed, mut requests) = (0usize, 0usize, 0usize);
        for (d, dictation) in dictations.iter().enumerate() {
            let segments: Vec<String> = dictation.iter().flat_map(|i| sentences(&i.dictated)).collect();
            let reference = dictation.iter().map(|i| i.final_text.trim()).collect::<Vec<_>>().join(" ");
            let mut window = Window::new(words);
            let (mut whole, mut text) = (String::new(), String::new());
            let (mut ms, mut sent_words) = (Vec::new(), Vec::new());
            let Some((last, live)) = segments.split_last() else { continue };
            for (k, segment) in live.iter().enumerate() {
                whole = joined(&whole, segment);
                let (sent, start) = window.split(&whole);
                let t = Instant::now();
                let result = utter_pipeline::clean_once(&server, system, &sent, &[], &ctx);
                let took = t.elapsed().as_secs_f64() * 1e3;
                let ok = result.is_ok();
                failed += usize::from(!ok);
                text = joined(&start, &result.unwrap_or_else(|_| sent.clone()));
                window.accept(&whole, &text, ok);
                // The next sentence is being said meanwhile; the last one's cleanup is waited for.
                if let Some(next) = segments.get(k + 1) {
                    let say_ms = next.split_whitespace().count() as f64 / WORDS_PER_SECOND * 1e3;
                    late += usize::from(took > say_ms);
                }
                ms.push(took);
                sent_words.push(sent.split_whitespace().count());
                requests += 1;
            }
            // The key goes up with the last sentence: the cleanup the person waits for.
            whole = joined(&whole, last);
            let mut at_key_up = serde_json::Map::new();
            for (f, &final_words) in opts.finals.iter().enumerate() {
                let (sent, start) = window.split_final(&whole, final_words);
                let t = Instant::now();
                let result = utter_pipeline::clean_once(&server, system, &sent, &[], &ctx);
                let took = t.elapsed().as_secs_f64() * 1e3;
                failed += usize::from(result.is_err());
                let text = joined(&start, &result.unwrap_or_else(|_| sent.clone()));
                finals[f].push(took);
                at_key_up.insert(
                    final_words.to_string(),
                    serde_json::json!({ "text": text, "ms": took, "sent_words": sent.split_whitespace().count() }),
                );
            }
            all_ms.extend_from_slice(&ms);
            let line = serde_json::json!({
                "dictation": d, "window": words, "items": opts.items,
                "words": whole.split_whitespace().count(),
                "raw": whole, "live_text": text, "reference": reference,
                "ms": ms, "sent_words": sent_words, "finals": at_key_up,
            });
            writeln!(out, "{line}")?;
        }
        all_ms.sort_by(f64::total_cmp);
        let pct = |v: &[f64], q: f64| v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
        println!(
            "window {words:>5}: requests p50 {:.0} ms p95 {:.0} | late {late}/{requests} | failed {failed}",
            pct(&all_ms, 0.5),
            pct(&all_ms, 0.95),
        );
        for (f, final_words) in opts.finals.iter().enumerate() {
            let v = &mut finals[f];
            v.sort_by(f64::total_cmp);
            println!("    key up {final_words:>6}: p50 {:.0} ms max {:.0}", pct(v, 0.5), pct(v, 1.0));
        }
    }
    println!("results: {}", out_dir.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn dictated_text_is_cut_at_sentence_ends() {
        assert_eq!(super::sentences("One two. Three? Four"), vec!["One two.", "Three?", "Four"]);
    }
}
