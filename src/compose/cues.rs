//! Grouping transcript words into captions (subtitle cues) and timing them on screen. Shared by
//! the `captions` layer and the subtitle files `flits transcribe` writes, so both follow the same
//! rules: a sentence end always ends a caption, phrase-aware breaks prefer a comma and never leave
//! a weak word (article, preposition, conjunction) dangling at the end, captions that flash by
//! too briefly merge with a neighbour, and consecutive captions never overlap.

use super::captions::Word;

/// Dutch words a caption should not end on (articles, prepositions, conjunctions).
pub const WEAK_NL: &[&str] = &[
    "de", "het", "een", "voor", "in", "op", "aan", "van", "met", "naar", "en", "maar", "of", "dat", "die", "als", "om", "te", "bij", "tot",
];
/// English words a caption should not end on.
pub const WEAK_EN: &[&str] =
    &["a", "an", "the", "of", "to", "in", "on", "at", "for", "with", "from", "by", "and", "but", "or", "that", "as", "if", "into", "than"];

/// The built-in `noBreakAfter` list for a language code ("nl", "en-US", ...), if there is one.
pub fn weak_words(language: &str) -> Option<&'static [&'static str]> {
    match language.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase().as_str() {
        "nl" => Some(WEAK_NL),
        "en" => Some(WEAK_EN),
        _ => None,
    }
}

/// How words are grouped into captions.
#[derive(Clone, Debug)]
pub struct Rules {
    /// most words per caption
    pub max_words: usize,
    /// most characters per caption (spaces included)
    pub max_chars: usize,
    /// a pause longer than this (s) always starts a new caption
    pub max_gap: f64,
    /// a word ending in one of these ends the caption
    pub sentence_end: &'static [char],
    /// a caption that is more than half full ends after a comma
    pub comma_break: bool,
    /// phrase-aware breaks: a full caption breaks after its last comma, otherwise not after a weak
    /// word; a lone trailing word joins the caption before it; a weak word left at the end moves on
    pub phrase: bool,
    /// lowercase words not to end a caption on (with `phrase`)
    pub no_break_after: Vec<String>,
    /// with `phrase`: characters a lone trailing word may add beyond `max_chars`
    pub lone_slack: usize,
    /// a caption on screen shorter than this (s, first word start to last word end) merges with a
    /// neighbour: not across a sentence end, a pause above `merge_gap` or a new speaker, and only
    /// when the result still fits (lines only; `max_chars` and `max_words` may be exceeded)
    pub min_duration: Option<f64>,
    /// largest pause (s) a `min_duration` merge bridges
    pub merge_gap: f64,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            max_words: usize::MAX,
            max_chars: usize::MAX,
            max_gap: 0.6,
            sentence_end: &['.', '?', '!', ':'],
            comma_break: true,
            phrase: false,
            no_break_after: vec![],
            lone_slack: 8,
            min_duration: None,
            merge_gap: 0.2,
        }
    }
}

/// When captions are on screen.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// seconds a caption appears before its first word
    pub lead: f64,
    /// seconds a caption stays after its last word
    pub hold: f64,
    /// a gap to the next caption shorter than this (s) is closed: the caption stays until the next one
    pub gap_close: f64,
    /// shortest time on screen (s), as far as the next caption allows
    pub min_duration: Option<f64>,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { lead: 0.0, hold: 0.25, gap_close: 0.6, min_duration: None }
    }
}

/// Length of the words as one line of text (spaces included).
pub fn text_len(ws: &[Word]) -> usize {
    ws.iter().map(|w| w.text.chars().count()).sum::<usize>() + ws.len().saturating_sub(1)
}

/// The words joined with spaces.
pub fn text(ws: &[Word]) -> String {
    ws.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

impl Rules {
    fn ends_sentence(&self, w: &Word) -> bool {
        w.text.trim_end_matches(['"', '\'', ')', '\u{201d}', '\u{2019}']).ends_with(self.sentence_end)
    }

    fn weak(&self, w: &Word) -> bool {
        let bare: String = w.text.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        !w.text.ends_with([',', ';', '.', '?', '!', ':']) && self.no_break_after.contains(&bare)
    }

    /// Group `words` (in time order) into captions. `lines_ok` tells whether words fit the
    /// caption's lines (layout); `max_words`/`max_chars` are checked here.
    pub fn group(&self, words: &[Word], lines_ok: &mut dyn FnMut(&[Word]) -> bool) -> Vec<Vec<Word>> {
        let maxw = self.max_words.max(1);
        let mut out: Vec<Vec<Word>> = vec![];
        let mut cur: Vec<Word> = vec![];
        let mut fits = |ws: &[Word]| ws.len() <= maxw && text_len(ws) <= self.max_chars && lines_ok(ws);
        for w in words {
            if let Some(last) = cur.last() {
                let hard = w.start - last.end > self.max_gap
                    || self.ends_sentence(last)
                    || (self.comma_break && last.text.ends_with(',') && cur.len() > maxw / 2)
                    || w.speaker != last.speaker;
                if hard {
                    out.push(std::mem::take(&mut cur));
                } else {
                    let mut with = cur.clone();
                    with.push(w.clone());
                    if !fits(&with) {
                        let k = if self.phrase { self.break_point(&cur, w, &mut fits) } else { cur.len() };
                        let rest = cur.split_off(k);
                        out.push(std::mem::replace(&mut cur, rest));
                    }
                }
            }
            cur.push(w.clone());
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        if self.phrase {
            self.merge_lone(&mut out, lines_ok);
            self.move_weak(&mut out, lines_ok);
        }
        if let Some(min) = self.min_duration {
            self.merge_short(&mut out, min, lines_ok);
        }
        out
    }

    /// Where a full caption `cur` breaks before `next` comes in: after its last comma, else after
    /// the last word that is not weak (and leaves a remainder that fits), else after all of it.
    /// Returns how many words stay in the caption.
    fn break_point(&self, cur: &[Word], next: &Word, fits: &mut dyn FnMut(&[Word]) -> bool) -> usize {
        let mut rest_fits = |i: usize| {
            let mut rest = cur[i + 1..].to_vec();
            rest.push(next.clone());
            fits(&rest)
        };
        // a comma before the last word (a comma on the last word is the plain break anyway)
        if let Some(i) = (1..cur.len().saturating_sub(1)).rev().find(|&i| cur[i].text.ends_with([',', ';'])) {
            if rest_fits(i) {
                return i + 1;
            }
        }
        if !self.weak(&cur[cur.len() - 1]) {
            return cur.len();
        }
        match (1..cur.len()).rev().find(|&i| !self.weak(&cur[i]) && rest_fits(i)) {
            Some(i) => i + 1,
            None => cur.len(),
        }
    }

    /// Captions `a` and `b` (in order) may become one: same speaker, no long pause, `a` doesn't end a sentence.
    fn joinable(&self, a: &[Word], b: &[Word], gap: f64) -> bool {
        let (Some(x), Some(y)) = (a.last(), b.first()) else { return false };
        !self.ends_sentence(x) && x.speaker == y.speaker && y.start - x.end <= gap
    }

    fn merge_lone(&self, out: &mut Vec<Vec<Word>>, lines_ok: &mut dyn FnMut(&[Word]) -> bool) {
        let mut k = 1;
        while k < out.len() {
            if out[k].len() == 1 && self.joinable(&out[k - 1], &out[k], self.max_gap) {
                let mut both = out[k - 1].clone();
                both.push(out[k][0].clone());
                if text_len(&both) <= self.max_chars.saturating_add(self.lone_slack) && lines_ok(&both) {
                    out[k - 1] = both;
                    out.remove(k);
                    continue;
                }
            }
            k += 1;
        }
    }

    fn move_weak(&self, out: &mut [Vec<Word>], lines_ok: &mut dyn FnMut(&[Word]) -> bool) {
        for k in 1..out.len() {
            while out[k - 1].len() > 1 && self.weak(out[k - 1].last().unwrap()) && self.joinable(&out[k - 1], &out[k], self.max_gap) {
                let mut moved = vec![out[k - 1].last().unwrap().clone()];
                moved.extend(out[k].iter().cloned());
                if moved.len() > self.max_words.max(1) || text_len(&moved) > self.max_chars || !lines_ok(&moved) {
                    break;
                }
                out[k - 1].pop();
                out[k] = moved;
            }
        }
    }

    fn merge_short(&self, out: &mut Vec<Vec<Word>>, min: f64, lines_ok: &mut dyn FnMut(&[Word]) -> bool) {
        let dur = |g: &[Word]| g.last().unwrap().end - g[0].start;
        let mut k = 0;
        while k < out.len() {
            if dur(&out[k]) < min {
                let best = [k.checked_sub(1), Some(k + 1)]
                    .into_iter()
                    .flatten()
                    .filter(|&j| j < out.len())
                    .filter(|&j| {
                        let (a, b) = (j.min(k), j.max(k));
                        if !self.joinable(&out[a], &out[b], self.merge_gap) {
                            return false;
                        }
                        let both: Vec<Word> = out[a].iter().chain(&out[b]).cloned().collect();
                        lines_ok(&both)
                    })
                    .min_by_key(|&j| text_len(&out[j]));
                if let Some(j) = best {
                    let (a, b) = (j.min(k), j.max(k));
                    let tail = out.remove(b);
                    out[a].extend(tail);
                    k = a.saturating_sub(1);
                    continue;
                }
            }
            k += 1;
        }
    }
}

impl Timing {
    /// (start, end) on screen for each caption (in order), never past `limit` and never
    /// overlapping: a caption ends at the latest when the next one starts (lead included).
    pub fn apply(&self, groups: &[Vec<Word>], limit: f64) -> Vec<(f64, f64)> {
        let mut starts: Vec<f64> = Vec::with_capacity(groups.len());
        for g in groups {
            let s = (g[0].start - self.lead.max(0.0)).max(if self.lead > 0.0 { 0.0 } else { f64::NEG_INFINITY });
            starts.push(starts.last().map_or(s, |p: &f64| s.max(*p)));
        }
        groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let s = starts[i];
                let next = starts.get(i + 1).copied().unwrap_or(f64::INFINITY);
                let mut e = g.last().unwrap().end + self.hold;
                if let Some(m) = self.min_duration {
                    e = e.max(s + m);
                }
                if next - e < self.gap_close {
                    e = next;
                }
                (s, e.min(next).min(limit))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words from "text@start" with an end 0.05 s before the next word (or +0.3 s for the last).
    fn ws(spec: &str) -> Vec<Word> {
        let raw: Vec<(String, f64)> = spec
            .split_whitespace()
            .map(|t| {
                let (w, s) = t.rsplit_once('@').unwrap();
                (w.to_string(), s.parse().unwrap())
            })
            .collect();
        raw.iter()
            .enumerate()
            .map(|(i, (w, s))| {
                let end = raw.get(i + 1).map(|n| n.1 - 0.05).unwrap_or(s + 0.3);
                Word { text: w.clone(), start: *s, end, speaker: None }
            })
            .collect()
    }

    /// Words spoken evenly, `step` s apart.
    fn even(text: &str, step: f64) -> Vec<Word> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| Word { text: w.into(), start: i as f64 * step, end: i as f64 * step + step * 0.8, speaker: None })
            .collect()
    }

    fn texts(g: &[Vec<Word>]) -> Vec<String> {
        g.iter().map(|g| text(g)).collect()
    }

    fn phrase(max_chars: usize) -> Rules {
        Rules { max_chars, phrase: true, no_break_after: WEAK_NL.iter().map(|s| s.to_string()).collect(), ..Rules::default() }
    }

    #[test]
    fn a_sentence_end_always_ends_a_caption() {
        let g = Rules::default().group(&even("Dit is kort. En dit ook! Klaar?", 0.3), &mut |_| true);
        assert_eq!(texts(&g), ["Dit is kort.", "En dit ook!", "Klaar?"]);
        // phrase rules don't merge a lone word across a sentence end either
        let g = phrase(32).group(&even("Het werkt goed. Ja.", 0.3), &mut |_| true);
        assert_eq!(texts(&g), ["Het werkt goed.", "Ja."]);
    }

    #[test]
    fn full_captions_break_after_a_comma() {
        let words = even("Als je dat wilt, kun je morgen gewoon langskomen bij ons", 0.3);
        let greedy = Rules { max_chars: 32, ..Rules::default() }.group(&words, &mut |_| true);
        assert_eq!(texts(&greedy)[0], "Als je dat wilt, kun je morgen");
        let g = phrase(32).group(&words, &mut |_| true);
        assert_eq!(texts(&g), ["Als je dat wilt,", "kun je morgen gewoon langskomen", "bij ons"]);
    }

    #[test]
    fn no_caption_ends_on_a_weak_word() {
        let words = even("We gaan morgen met de hele groep naar het strand toe vandaag", 0.3);
        let greedy = Rules { max_chars: 24, ..Rules::default() }.group(&words, &mut |_| true);
        assert!(greedy.iter().any(|g| ["de", "het", "naar", "met"].contains(&g.last().unwrap().text.as_str())), "{:?}", texts(&greedy));
        let r = phrase(24);
        let g = r.group(&words, &mut |_| true);
        for c in &g[..g.len() - 1] {
            assert!(!r.weak(c.last().unwrap()), "caption ends on a weak word: {:?}", texts(&g));
        }
        // a lone last word may join with a little slack
        assert!(g.iter().all(|c| text_len(c) <= 24 + r.lone_slack), "{:?}", texts(&g));
        // case-insensitive
        assert!(r.weak(&Word { text: "Het".into(), ..Default::default() }));
        assert!(!r.weak(&Word { text: "het,".into(), ..Default::default() }));
    }

    #[test]
    fn a_lone_trailing_word_joins_the_caption_before_it() {
        let words = even("Dit is een hele mooie zin over niks.", 0.3);
        let g = Rules { max_chars: 32, ..Rules::default() }.group(&words, &mut |_| true);
        assert_eq!(texts(&g), ["Dit is een hele mooie zin over", "niks."]);
        let g = phrase(32).group(&words, &mut |_| true);
        assert_eq!(texts(&g), ["Dit is een hele mooie zin over niks."]);
    }

    #[test]
    fn short_captions_merge_with_a_neighbour() {
        // fast speech: "snel gezegd," lasts 0.35 s
        let words = ws("Dat@0 was@0.3 heel@0.6 mooi,@0.9 snel@1.5 gezegd,@1.65 en@1.9 nu@2.2 verder@2.5 praten.@2.8");
        let base = Rules { max_words: 3, ..Rules::default() };
        let g = base.group(&words, &mut |_| true);
        assert!(g.iter().any(|g| g.last().unwrap().end - g[0].start < 0.9), "{:?}", texts(&g));
        let r = Rules { min_duration: Some(0.9), ..base.clone() };
        let g = r.group(&words, &mut |_| true);
        for c in &g {
            assert!(c.last().unwrap().end - c[0].start >= 0.9, "{:?}", texts(&g));
        }
        // ... but only if the result fits the lines
        let g = r.group(&words, &mut |ws| ws.len() <= 3);
        assert!(g.iter().all(|c| c.len() <= 3), "{:?}", texts(&g));
        // never across a sentence end or a pause
        let words = ws("Ja.@0 Nee@0.3 hoor@0.55");
        let g = Rules { min_duration: Some(0.9), ..Rules::default() }.group(&words, &mut |_| true);
        assert_eq!(texts(&g), ["Ja.", "Nee hoor"]);
        let w = |text: &str, start: f64, end: f64| Word { text: text.into(), start, end, speaker: None };
        let words = [w("eerst", 0.0, 0.4), w("nu", 1.0, 1.15), w("dan", 1.2, 1.5)];
        let g = Rules { min_duration: Some(0.9), max_words: 1, ..Rules::default() }.group(&words, &mut |_| true);
        assert_eq!(texts(&g), ["eerst", "nu dan"]);
    }

    #[test]
    fn captions_never_overlap() {
        // word ends that run into the next word, and leads that reach back before the previous caption
        let mut words = even("een twee drie vier vijf zes zeven acht negen tien elf twaalf", 0.25);
        for w in &mut words {
            w.end += 0.2;
        }
        for t in [
            Timing::default(),
            Timing { lead: 0.05, hold: 0.25, gap_close: 0.6, min_duration: Some(0.9) },
            Timing { lead: 0.4, hold: 1.0, gap_close: 0.0, min_duration: Some(2.0) },
            Timing { lead: 0.0, hold: 0.0, gap_close: 0.0, min_duration: None },
        ] {
            for maxw in [1, 2, 3] {
                let g = Rules { max_words: maxw, ..Rules::default() }.group(&words, &mut |_| true);
                let times = t.apply(&g, 100.0);
                for p in times.windows(2) {
                    assert!(p[0].1 <= p[1].0 + 1e-12, "{t:?} maxWords {maxw}: {p:?}");
                    assert!(p[0].0 <= p[1].0);
                }
            }
        }
    }

    #[test]
    fn timing_leads_holds_and_closes_gaps() {
        let g = vec![ws("a@1.0"), ws("b@1.7"), ws("c@5.0")];
        let t = Timing { lead: 0.05, hold: 0.25, gap_close: 0.6, min_duration: Some(0.9) };
        let times = t.apply(&g, 10.0);
        // a: shown 0.05 s early, held until b (the gap below 0.6 s is closed), b at most until c
        assert!((times[0].0 - 0.95).abs() < 1e-9 && (times[0].1 - 1.65).abs() < 1e-9, "{times:?}");
        assert!((times[1].1 - (1.65 + 0.9)).abs() < 1e-9, "min duration: {times:?}");
        assert!((times[2].1 - 5.85).abs() < 1e-9, "{times:?}");
        // the defaults are the old captions timing: no lead, 0.25 s hold, gaps below 0.6 s closed
        let d = Timing::default().apply(&g, 10.0);
        assert_eq!(d[0].0, 1.0);
        assert!((d[1].1 - 2.25).abs() < 1e-9, "{d:?}");
    }

    #[test]
    fn languages_have_weak_word_lists() {
        assert!(weak_words("nl").unwrap().contains(&"het"));
        assert!(weak_words("en-US").unwrap().contains(&"the"));
        assert!(weak_words("fr").is_none());
    }
}
