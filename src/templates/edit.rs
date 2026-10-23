//! Descript-style editing through a text file. `flits cut x.mp4 --edit` writes `x.edit.txt`,
//! one sentence per line with its start time:
//!
//! ```text
//! [00:12.40] So this is the first sentence.
//! [00:15.02] {SPEAKER_01} And this is the answer.
//! ```
//!
//! The user deletes lines or words (or wraps parts in `[[...]]`) and `--apply` turns what is left
//! into word-level cuts. Lines are matched to the transcript by their timestamp; within a line the
//! remaining text is aligned to the original words (case and punctuation ignored, typo fixes keep
//! the word), so every original word that has no counterpart any more is removed.

use super::fillers::norm_word;
use crate::compose::captions::{Transcript, Word};
use std::ops::Range;
use std::path::Path;

/// One editable line: a sentence (or a segment piece) of the transcript.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub start: f64,
    pub end: f64,
    pub speaker: Option<String>,
    /// indices into `EditDoc::words`
    pub words: Range<usize>,
}

/// The transcript as editable lines. `words` is in time order (the same order `cut` uses).
#[derive(Clone, Debug, Default)]
pub struct EditDoc {
    pub words: Vec<Word>,
    pub lines: Vec<Line>,
}

fn ends_sentence(w: &str) -> bool {
    w.trim_end_matches(['"', '\'', ')', '»', '”']).ends_with(['.', '?', '!', '…'])
}

impl EditDoc {
    /// Split into lines at segment boundaries, sentence ends, speaker changes and long pauses;
    /// very long sentences are split at a comma (or hard at 45 words).
    pub fn new(tr: &Transcript) -> EditDoc {
        let mut tagged: Vec<(Word, usize)> =
            tr.segments.iter().enumerate().flat_map(|(si, s)| s.words.iter().map(move |w| (w.clone(), si))).collect();
        tagged.sort_by(|a, b| a.0.start.total_cmp(&b.0.start));
        let mut lines: Vec<Line> = vec![];
        for (i, (w, seg)) in tagged.iter().enumerate() {
            let new_line = match (i.checked_sub(1).map(|p| &tagged[p]), lines.last()) {
                (Some((prev, prev_seg)), Some(line)) => {
                    let n = i - line.words.start;
                    prev_seg != seg
                        || ends_sentence(&prev.text)
                        || prev.speaker != w.speaker
                        || w.start - prev.end > 1.5
                        || (n >= 25 && prev.text.ends_with([',', ';', ':']))
                        || n >= 45
                }
                _ => true,
            };
            if new_line {
                lines.push(Line { start: w.start, end: w.end, speaker: w.speaker.clone(), words: i..i + 1 });
            } else {
                let l = lines.last_mut().unwrap();
                l.words.end = i + 1;
                l.end = l.end.max(w.end);
            }
        }
        EditDoc { words: tagged.into_iter().map(|(w, _)| w).collect(), lines }
    }

    /// The edit file: a short header (as `#` comments) and one `[mm:ss.cc] text` line per sentence.
    pub fn render(&self, input: &Path, transcript: &Path, edit_file: &Path) -> String {
        let mut s = format!(
            "# flits edit file for {}\n# transcript: {}\n#\n\
             # Delete a line to cut that sentence, delete words to cut only those words, or wrap\n\
             # parts in [[ ]] to cut them. Keep the [mm:ss.cc] time at the start of every line you\n\
             # keep (it ties the line to the recording). Case, punctuation and typo fixes don't\n\
             # matter, reordering lines is not supported. Lines starting with # are ignored.\n\
             # Then run:  flits cut \"{}\" --apply \"{}\"\n\
             # (add --export fcpxml|premiere|resolve|edl|json for an editor timeline)\n\n",
            input.display(),
            transcript.display(),
            input.display(),
            edit_file.display()
        );
        for l in &self.lines {
            s += &stamp(l.start);
            if let Some(sp) = &l.speaker {
                s += &format!(" {{{sp}}}");
            }
            for w in &self.words[l.words.clone()] {
                s.push(' ');
                s += &w.text;
            }
            s.push('\n');
        }
        s
    }
}

/// The transcript an edit file was made from (its `# transcript:` header line), if it still exists.
pub fn transcript_from_header(text: &str) -> Option<std::path::PathBuf> {
    let p = text.lines().take_while(|l| l.starts_with('#')).find_map(|l| l.strip_prefix("# transcript:"))?.trim();
    let p = std::path::PathBuf::from(p);
    p.is_file().then_some(p)
}

/// `[mm:ss.cc]`, or `[h:mm:ss.cc]` from an hour on.
pub fn stamp(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    let (h, m, sec, c) = (cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100);
    if h > 0 {
        format!("[{h}:{m:02}:{sec:02}.{c:02}]")
    } else {
        format!("[{m:02}:{sec:02}.{c:02}]")
    }
}

/// Parse a leading `[mm:ss.cc]` / `[h:mm:ss]` / `[ss.cc]`; returns the time and the rest of the line.
pub fn parse_stamp(line: &str) -> Option<(f64, &str)> {
    let l = line.trim_start();
    let rest = l.strip_prefix('[')?;
    if rest.starts_with('[') {
        return None;
    }
    let (inner, after) = rest.split_once(']')?;
    let mut t = 0.0;
    let parts: Vec<&str> = inner.trim().split(':').collect();
    if parts.len() > 3 {
        return None;
    }
    for p in parts {
        t = t * 60.0 + p.trim().parse::<f64>().ok().filter(|v| *v >= 0.0)?;
    }
    Some((t, after))
}

/// Result of applying an edit file.
#[derive(Debug, Default)]
pub struct Applied {
    /// one flag per `EditDoc::words` entry
    pub removed: Vec<bool>,
    pub kept_lines: usize,
    pub removed_lines: usize,
    pub warnings: Vec<String>,
}

/// Remove `[[ ... ]]` parts (they may span lines; `in_cut` carries the state).
fn strip_cuts(line: &str, in_cut: &mut bool) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let marker = if *in_cut { "]]" } else { "[[" };
        match rest.find(marker) {
            Some(i) => {
                if !*in_cut {
                    out += &rest[..i];
                }
                // a cut glues the words around it together otherwise
                out.push(' ');
                *in_cut = !*in_cut;
                rest = &rest[i + 2..];
            }
            None => {
                if !*in_cut {
                    out += rest;
                }
                return out;
            }
        }
    }
}

/// Character similarity in 0..=1 (optimal string alignment distance: "teh" -> "the" is one edit).
fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let n = a.len().max(b.len());
    if n == 0 {
        return 1.0;
    }
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, v) in d[0].iter_mut().enumerate() {
        *v = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let mut v = (d[i - 1][j - 1] + (a[i - 1] != b[j - 1]) as usize).min(d[i - 1][j] + 1).min(d[i][j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    1.0 - d[a.len()][b.len()] as f64 / n as f64
}

/// Align the edited tokens to the original words of a line (weighted edit distance). Returns
/// for every original word whether it is kept, plus the edited tokens that matched nothing.
/// A changed word counts as the same word (a typo fix) only when it is at least 60 % similar.
fn align(orig: &[String], edited: &[String]) -> (Vec<bool>, Vec<String>) {
    let (m, n) = (orig.len(), edited.len());
    let sub = |i: usize, j: usize| {
        if orig[i] == edited[j] {
            return 0.0;
        }
        let s = similarity(&orig[i], &edited[j]);
        // 2.0 = as expensive as delete + insert; the traceback then prefers deleting
        if s >= 0.6 {
            2.0 * (1.0 - s)
        } else {
            2.0
        }
    };
    let mut d = vec![vec![0.0f64; n + 1]; m + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i as f64;
    }
    for (j, v) in d[0].iter_mut().enumerate() {
        *v = j as f64;
    }
    for i in 1..=m {
        for j in 1..=n {
            d[i][j] = (d[i - 1][j - 1] + sub(i - 1, j - 1)).min(d[i - 1][j] + 1.0).min(d[i][j - 1] + 1.0);
        }
    }
    let (mut keep, mut extra) = (vec![false; m], vec![]);
    let (mut i, mut j) = (m, n);
    const EPS: f64 = 1e-9;
    while i > 0 || j > 0 {
        let here = d[i][j];
        if i > 0 && j > 0 && orig[i - 1] == edited[j - 1] && (here - d[i - 1][j - 1]).abs() < EPS {
            keep[i - 1] = true;
            i -= 1;
            j -= 1;
        } else if i > 0 && (here - (d[i - 1][j] + 1.0)).abs() < EPS {
            // prefer deleting the original word over a weak substitution
            i -= 1;
        } else if j > 0 && (here - (d[i][j - 1] + 1.0)).abs() < EPS {
            extra.push(edited[j - 1].clone());
            j -= 1;
        } else {
            keep[i - 1] = true;
            i -= 1;
            j -= 1;
        }
    }
    extra.reverse();
    (keep, extra)
}

fn tokens(text: &str) -> Vec<String> {
    text.split_whitespace().map(norm_word).filter(|t| !t.is_empty()).collect()
}

/// Apply an edited file to the document: which words are removed.
pub fn apply(doc: &EditDoc, text: &str) -> Applied {
    // 1. collect the edited text per original line (timestamps choose the line; lines without one
    //    continue the line above, so splitting a line in two is harmless)
    let mut edited: Vec<Option<String>> = vec![None; doc.lines.len()];
    let mut warnings = vec![];
    let mut in_cut = false;
    let mut current: Option<usize> = None;
    let mut last_matched: Option<usize> = None;
    let mut reordered = false;
    for (ln, raw) in text.lines().enumerate() {
        if raw.trim_start().starts_with('#') {
            continue;
        }
        let line = strip_cuts(raw, &mut in_cut);
        if line.trim().is_empty() {
            continue;
        }
        match parse_stamp(&line) {
            Some((t, rest)) => {
                // nearest unused original line within a second
                let p = doc.lines.partition_point(|l| l.start < t);
                let best = (p.saturating_sub(3)..(p + 3).min(doc.lines.len()))
                    .filter(|&i| edited[i].is_none() && (doc.lines[i].start - t).abs() <= 1.0)
                    .min_by(|&a, &b| (doc.lines[a].start - t).abs().total_cmp(&(doc.lines[b].start - t).abs()));
                match best {
                    Some(i) => {
                        let rest = rest.trim_start();
                        // drop the {speaker} label
                        let rest = match rest.strip_prefix('{').and_then(|r| r.split_once('}')) {
                            Some((_, after)) => after,
                            None => rest,
                        };
                        edited[i] = Some(rest.to_string());
                        if last_matched.is_some_and(|l| l > i) {
                            reordered = true;
                        }
                        last_matched = Some(i);
                        current = Some(i);
                    }
                    None => {
                        warnings.push(format!("line {}: no sentence at {} in the transcript (or used twice); ignored", ln + 1, stamp(t)));
                        current = None;
                    }
                }
            }
            None => match current {
                Some(i) => {
                    let e = edited[i].as_mut().unwrap();
                    e.push(' ');
                    e.push_str(&line);
                }
                None => warnings.push(format!("line {}: no [mm:ss.cc] time at the start; ignored", ln + 1)),
            },
        }
    }
    if in_cut {
        warnings.push("a [[ has no closing ]]: everything after it is cut".into());
    }
    if reordered {
        warnings.push("lines were moved around: reordering is not supported, the original order is kept".into());
    }
    // 2. per line: align the remaining words to the original ones
    let mut removed = vec![true; doc.words.len()];
    let (mut kept_lines, mut removed_lines) = (0, 0);
    for (l, e) in doc.lines.iter().zip(&edited) {
        let Some(e) = e else {
            removed_lines += 1;
            continue;
        };
        let orig_norm: Vec<String> = doc.words[l.words.clone()].iter().map(|w| norm_word(&w.text)).collect();
        // words that are only punctuation ("-", "...") follow their neighbour
        let real: Vec<usize> = (0..orig_norm.len()).filter(|&k| !orig_norm[k].is_empty()).collect();
        let orig: Vec<String> = real.iter().map(|&k| orig_norm[k].clone()).collect();
        let (keep, extra) = align(&orig, &tokens(e));
        let mut flags = vec![false; orig_norm.len()];
        for (&k, &kp) in real.iter().zip(&keep) {
            flags[k] = kp;
        }
        for k in 0..flags.len() {
            if orig_norm[k].is_empty() {
                flags[k] = if k > 0 { flags[k - 1] } else { flags.get(1).copied().unwrap_or(true) };
            }
        }
        for (k, f) in flags.iter().enumerate() {
            removed[l.words.start + k] = !f;
        }
        if flags.iter().any(|f| *f) {
            kept_lines += 1;
        } else {
            removed_lines += 1;
        }
        if !extra.is_empty() {
            warnings.push(format!(
                "{}: words that are not in the recording were ignored: {}",
                stamp(l.start),
                extra.iter().take(8).cloned().collect::<Vec<_>>().join(" ")
            ));
        }
    }
    Applied { removed, kept_lines, removed_lines, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::captions::Segment;

    /// Words 0.5 s apart; a segment per entry.
    fn transcript(segments: &[&str]) -> Transcript {
        let mut t = 0.0;
        let segments = segments
            .iter()
            .map(|s| {
                let words: Vec<Word> = s
                    .split_whitespace()
                    .map(|w| {
                        t += 0.5;
                        Word { text: w.into(), start: t, end: t + 0.4, speaker: None }
                    })
                    .collect();
                t += 1.0;
                Segment { start: words[0].start, end: words.last().unwrap().end, text: s.to_string(), speaker: None, words }
            })
            .collect();
        Transcript { language: Some("en".into()), segments }
    }

    fn kept(doc: &EditDoc, a: &Applied) -> String {
        doc.words.iter().zip(&a.removed).filter(|(_, r)| !**r).map(|(w, _)| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }

    fn doc() -> EditDoc {
        EditDoc::new(&transcript(&["Hello and welcome. This is the show.", "Uh, we talk about Rust today, and other things."]))
    }

    #[test]
    fn splits_into_sentences_with_stamps() {
        let d = doc();
        assert_eq!(d.lines.len(), 3);
        let text = d.render(Path::new("a.mp4"), Path::new("a.words.json"), Path::new("a.edit.txt"));
        let body: Vec<&str> = text.lines().filter(|l| l.starts_with('[')).collect();
        assert_eq!(
            body,
            ["[00:00.50] Hello and welcome.", "[00:02.00] This is the show.", "[00:05.00] Uh, we talk about Rust today, and other things."]
        );
        assert_eq!(stamp(3725.456), "[1:02:05.46]");
        assert!((parse_stamp("[1:02:05.46] x").unwrap().0 - 3725.46).abs() < 1e-9);
        assert_eq!(parse_stamp(" [00:02.00]text").unwrap(), (2.0, "text"));
        assert!(parse_stamp("[[00:02.00] x").is_none());
        assert!(parse_stamp("no stamp").is_none());
    }

    #[test]
    fn unchanged_file_keeps_everything() {
        let d = doc();
        let a = apply(&d, &d.render(Path::new("a"), Path::new("b"), Path::new("c")));
        assert!(a.removed.iter().all(|r| !r));
        assert_eq!((a.kept_lines, a.removed_lines), (3, 0));
        assert!(a.warnings.is_empty(), "{:?}", a.warnings);
    }

    #[test]
    fn deleted_lines_words_and_brackets_are_removed() {
        let d = doc();
        let edited = "# a comment [[ not a cut\n\
                      [00:00.50] hello WELCOME\n\
                      [00:05.00] [[Uh,]] we talk about Rust [[today, and other]] things\n";
        let a = apply(&d, edited);
        assert_eq!(kept(&d, &a), "Hello welcome. we talk about Rust things.");
        assert_eq!((a.kept_lines, a.removed_lines), (2, 1));
    }

    #[test]
    fn typo_fixes_keep_words_and_split_lines_continue() {
        let d = doc();
        // "Rust" fixed to "Rusty", the line split in two (second half without a stamp), "show" fixed
        let edited =
            "[00:00.50] Hello and welcome.\n[00:02.00] This is teh show.\n[00:05.00] Uh, we talk about Rusty\ntoday, and other things.\n";
        let a = apply(&d, edited);
        assert!(a.removed.iter().all(|r| !r), "{}", kept(&d, &a));
        // made-up words are reported, and the timestamp may be slightly off
        let a = apply(&d, "[00:00.70] Hello and welcome to flits\n");
        assert_eq!(kept(&d, &a), "Hello and welcome.");
        assert!(a.warnings.iter().any(|w| w.contains("to flits")), "{:?}", a.warnings);
    }

    #[test]
    fn unknown_or_missing_stamps_are_reported() {
        let d = doc();
        let a = apply(&d, "nothing here\n[09:00.00] Hello\n[[ [00:05.00] Uh, we\n");
        assert!(a.removed.iter().all(|r| *r));
        assert_eq!(a.warnings.len(), 3, "{:?}", a.warnings);
    }

    #[test]
    fn speaker_labels_are_written_and_ignored_on_apply() {
        let mut tr = transcript(&["Hi there.", "Hello."]);
        for w in &mut tr.segments[1].words {
            w.speaker = Some("SPEAKER_01".into());
        }
        let d = EditDoc::new(&tr);
        let text = d.render(Path::new("a"), Path::new("b"), Path::new("c"));
        assert!(text.contains("] {SPEAKER_01} Hello."));
        let a = apply(&d, &text);
        assert!(a.removed.iter().all(|r| !r));
        assert!(a.warnings.is_empty());
    }

    #[test]
    fn alignment_prefers_deleting_over_weak_substitution() {
        let o: Vec<String> = ["the", "cat", "sat", "down"].iter().map(|s| s.to_string()).collect();
        let e: Vec<String> = ["the", "sat", "down"].iter().map(|s| s.to_string()).collect();
        assert_eq!(align(&o, &e).0, [true, false, true, true]);
        let e: Vec<String> = ["a", "dog"].iter().map(|s| s.to_string()).collect();
        let (k, extra) = align(&o, &e);
        assert_eq!(k, [false; 4]);
        assert_eq!(extra, ["a", "dog"]);
    }
}
