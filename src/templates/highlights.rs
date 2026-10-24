//! `flits reel --auto`: find the part of a recording that works best as a reel, from its transcript.
//!
//! The transcript is split into sentences (punctuation, speaker changes, long pauses). A candidate
//! is a run of whole sentences of about the target length (counted with long pauses shortened, as
//! the reel cuts them), so it never starts or ends mid-sentence. Every candidate gets a score made
//! of small, named reasons, the way an editor would judge it:
//!
//! * the **hook** (first sentence): a question, a number, a "the reason / het probleem" cue or a
//!   contrast lift it; starting with a conjunction ("and", "maar", "dus"), a discourse marker ("ja",
//!   "nou", "well") or a pronoun that refers back ("dat is", "die gaan", "it was") sinks it, as does
//!   answering a question that is not in the clip. A leading "Ja," / "Nou," / "So," is trimmed off.
//! * the **ending**: a full stop followed by a pause, ideally a conclusive sentence ("dus", "that's
//!   why", an exclamation); a trailing "..." or a next sentence that continues the thought ("want",
//!   "because") counts against it.
//! * the **body**: one thought (an interviewer's new question halfway means two topics), few long
//!   silences, a lively pace, few filler words, no broken-off sentences.
//! * the **length**: close to the target.
//!
//! Structure (punctuation, pauses, numbers, pace) works for any language; the cue word lists cover
//! English and Dutch.

use super::fillers::{norm_word, DEFAULT_FILLERS};
use crate::compose::captions::Word;
use std::collections::HashSet;

/// A sentence: words `first..=last` of the word list.
#[derive(Clone, Debug)]
pub struct Sentence {
    pub first: usize,
    pub last: usize,
    pub start: f64,
    pub end: f64,
}

/// One candidate part, best first.
#[derive(Clone, Debug)]
pub struct Pick {
    /// source time to cut from / to (in the pauses around the words)
    pub from: f64,
    pub to: f64,
    /// estimated length in the reel (long pauses shortened)
    pub length: f64,
    pub score: f64,
    /// the opening sentence (after a trimmed "Ja," and the like) and the last one
    pub first: String,
    pub last: String,
    /// why: (points, explanation), largest effect first
    pub reasons: Vec<(f64, String)>,
}

/// Sentence-ending punctuation (a trailing quote or bracket allowed).
fn ends_sentence(w: &str) -> bool {
    w.trim_end_matches(['"', '\'', ')', '»', '”']).ends_with(['.', '?', '!', '…'])
}

fn is_question(w: &str) -> bool {
    w.trim_end_matches(['"', '\'', ')', '»', '”']).ends_with('?')
}

/// Broken off: "dat..." / "the-"
fn trails_off(w: &str) -> bool {
    let w = w.trim_end_matches(['"', '\'', ')', '»', '”']);
    w.ends_with("...") || w.ends_with('…') || w.ends_with('-')
}

/// Split words (sorted by time) into sentences: at sentence punctuation, a speaker change or a
/// pause over 2 s; very long runs without punctuation are split at a comma after 40 words.
pub fn sentences(words: &[Word]) -> Vec<Sentence> {
    let mut out: Vec<Sentence> = vec![];
    for (i, w) in words.iter().enumerate() {
        let new = match (i.checked_sub(1).map(|p| &words[p]), out.last()) {
            (Some(prev), Some(s)) => {
                ends_sentence(&prev.text)
                    || prev.speaker != w.speaker
                    || w.start - prev.end > 2.0
                    || (i - s.first >= 40 && prev.text.ends_with([',', ';', ':']))
                    || i - s.first >= 70
            }
            _ => true,
        };
        if new {
            out.push(Sentence { first: i, last: i, start: w.start, end: w.end });
        } else {
            let s = out.last_mut().unwrap();
            s.last = i;
            s.end = s.end.max(w.end);
        }
    }
    out
}

// Cue lists: lower case, compared with `norm_word` (apostrophes split off: "that's" -> "that").

/// A sentence starting with these continues an earlier one.
const CONTINUATIONS: &[&str] = &[
    "and", "but", "so", "because", "or", "then", "also", "plus", "which", "nor", "en", "maar", "dus", "want", "of", "omdat", "toen",
    "daarna", "ook", "waardoor", "zodat",
];
/// Replies and discourse markers: fine inside a clip, weak as its first word.
const MARKERS: &[&str] = &[
    "ja",
    "jaa",
    "nou",
    "nee",
    "oké",
    "oke",
    "okay",
    "ok",
    "yes",
    "yeah",
    "yep",
    "no",
    "well",
    "right",
    "kijk",
    "inderdaad",
    "precies",
    "exactly",
    "sure",
    "oh",
    "tja",
    "goh",
    "hm",
    "uh",
    "uhm",
    "um",
    "ehm",
    "eh",
];
/// Markers that may be trimmed off the start ("Ja, ..." -> "...") when a comma follows.
const TRIMMABLE: &[&str] = &[
    "ja", "jaa", "nou", "oké", "oke", "okay", "ok", "yes", "yeah", "well", "so", "kijk", "right", "uh", "uhm", "um", "ehm", "eh", "en",
    "and", "dus", "tja",
];
/// Words that refer back to something said before ("dat is ...", "that was ..."), when a verb follows.
const BACK_REFS: &[&str] = &["dat", "die", "that", "those", "these", "deze", "dit"];
/// Weaker: "het is" / "it is" is as often a plain opener.
const SOFT_BACK_REFS: &[&str] = &["het", "it"];
/// Refer back whatever follows.
const BACK_ALWAYS: &[&str] = &["daarom", "daar", "therefore", "zo", "hij", "zij", "ze", "he", "she", "they", "hem", "him", "them"];
const VERBS: &[&str] = &[
    "is", "was", "zijn", "waren", "wordt", "werd", "worden", "heeft", "hebben", "had", "heb", "hadden", "gaat", "gaan", "ging", "kan",
    "kunnen", "kon", "moet", "moeten", "wil", "willen", "zal", "zullen", "zou", "zouden", "komt", "komen", "doet", "doen", "deed", "klopt",
    "betekent", "maakt", "zegt", "vind", "vindt", "weet", "ben", "are", "were", "has", "have", "does", "did", "means", "makes", "will",
    "would", "can", "could", "should", "must", "s", "ll", "d", "re", "ve", "sounds", "happened", "gebeurde",
];
/// Hook phrases (matched on whole words anywhere in the first sentence).
const HOOKS: &[&str] = &[
    "the reason",
    "the problem",
    "the secret",
    "the truth",
    "the mistake",
    "the key",
    "the biggest",
    "the best",
    "the worst",
    "the most",
    "most people",
    "nobody",
    "no one",
    "everyone",
    "everybody",
    "never",
    "always",
    "here's",
    "what if",
    "imagine",
    "did you know",
    "the one thing",
    "the thing is",
    "the point is",
    "the question is",
    "the first",
    "the only",
    "important",
    "de reden",
    "het probleem",
    "het geheim",
    "de waarheid",
    "de fout",
    "de sleutel",
    "de grootste",
    "de beste",
    "het beste",
    "de belangrijkste",
    "het belangrijkste",
    "belangrijk",
    "de meeste mensen",
    "veel mensen",
    "niemand",
    "iedereen",
    "nooit",
    "altijd",
    "stel je voor",
    "wist je dat",
    "het punt is",
    "de vraag is",
    "wat als",
    "het enige",
    "de enige",
    "waarom",
    "why",
];
const CONTRASTS: &[&str] = &[
    "but actually",
    "instead",
    "in fact",
    "the opposite",
    "not because",
    "maar eigenlijk",
    "juist",
    "in plaats van",
    "terwijl",
    "het tegendeel",
    "integendeel",
    "niet omdat",
    "aan de andere kant",
    "on the other hand",
];
/// A last sentence that starts with one of these wraps up.
const CONCLUSION_STARTS: &[&str] =
    &["dus", "daarom", "kortom", "uiteindelijk", "so", "therefore", "ultimately", "finally", "overall", "conclusie", "samengevat"];
const CONCLUSION_PHRASES: &[&str] = &[
    "that's why",
    "the bottom line",
    "in the end",
    "that's it",
    "that is why",
    "daar gaat het om",
    "dat is het",
    "zo simpel is het",
    "tot slot",
    "dan denk ik",
    "dus ik denk",
    "daarom denk ik",
    "so i think",
    "the lesson",
    "de les",
    "at the end of the day",
    "en daarom",
    "dat is waarom",
];
const GREETINGS: &[&str] = &[
    "goedemiddag",
    "goedemorgen",
    "goedenavond",
    "hallo",
    "hoi",
    "welkom",
    "dankjewel",
    "dank je wel",
    "bedankt",
    "mijn naam is",
    "hello",
    "hi",
    "welcome",
    "thanks",
    "thank you",
    "my name is",
    "beste",
    "dear",
];
/// "Dus jij vindt ...": a question without a question mark.
const YOU: &[&str] = &["je", "jij", "u", "jullie", "you"];
const CALLS_TO_ACTION: &[&str] = &[
    "kom",
    "volg",
    "abonneer",
    "word lid",
    "doe mee",
    "meld je aan",
    "kijk op",
    "ga naar",
    "luister",
    "stem",
    "follow",
    "subscribe",
    "join",
    "sign up",
    "check out",
    "come",
    "try it",
    "link in",
    "vote",
    "word actief",
];
/// Thanks and goodbyes: a weak last sentence for a reel.
const SIGN_OFFS: &[&str] = &[
    "bedankt",
    "dankjewel",
    "dank je wel",
    "dank u wel",
    "tot ziens",
    "doei",
    "graag gedaan",
    "thanks",
    "thank you",
    "bye",
    "see you",
    "goodbye",
    "cheers",
];
/// Pronouns after "dat"/"that" at the start make it a subordinate clause: "Dat zij er altijd ..."
const PRONOUNS: &[&str] =
    &["ik", "je", "jij", "u", "hij", "zij", "ze", "we", "wij", "jullie", "er", "het", "men", "i", "you", "he", "she", "we", "they"];
/// Soft fillers (besides the hesitation sounds).
const SOFT_FILLERS: &[&str] =
    &["zeg maar", "weet je", "of zo", "enzo", "gewoon", "like", "you know", "i mean", "sort of", "kind of", "basically", "literally"];
const NUMBER_WORDS: &[&str] = &[
    "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "twelve", "twenty", "hundred", "thousand", "million",
    "billion", "percent", "half", "twee", "drie", "vier", "vijf", "zes", "zeven", "acht", "negen", "tien", "twaalf", "twintig", "honderd",
    "duizend", "miljoen", "miljard", "procent", "helft",
];

/// Lower-case tokens of a word list, apostrophes split ("that's" -> "that", "s").
fn tokens(words: &[Word]) -> Vec<String> {
    words
        .iter()
        .flat_map(|w| norm_word(&w.text).split(['\'', '’']).map(str::to_string).collect::<Vec<_>>())
        .filter(|t| !t.is_empty())
        .collect()
}

/// " a b c " for whole-word phrase search.
fn joined(t: &[String]) -> String {
    format!(" {} ", t.join(" "))
}

fn has_phrase(j: &str, phrases: &[&str]) -> Option<String> {
    phrases.iter().find(|p| j.contains(&format!(" {} ", p.replace('\'', " ")))).map(|p| p.to_string())
}

fn quote(words: &[Word]) -> String {
    words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

/// Estimated length once pauses over 0.6 s are cut down (as `flits reel` does).
fn est_length(words: &[Word]) -> f64 {
    let mut t = 0.0;
    for (i, w) in words.iter().enumerate() {
        t += (w.end - w.start).max(0.0);
        if let Some(n) = words.get(i + 1) {
            let g = (n.start - w.end).max(0.0);
            t += if g > 0.6 { 0.35 } else { g };
        }
    }
    t
}

struct Scorer<'a> {
    words: &'a [Word],
    sents: &'a [Sentence],
    total: f64,
    target: f64,
}

impl Scorer<'_> {
    fn pause_after(&self, word: usize) -> Option<f64> {
        self.words.get(word + 1).map(|n| (n.start - self.words[word].end).max(0.0))
    }

    /// Leading markers to drop: "Ja, ..." / "Nou, ..." (at most two, and never the whole sentence).
    fn trim_start(&self, s: &Sentence) -> usize {
        let mut i = s.first;
        while i < s.last.saturating_sub(2) && i < s.first + 2 {
            let w = &self.words[i];
            let marker = TRIMMABLE.contains(&norm_word(&w.text).as_str());
            let hesitation = DEFAULT_FILLERS.contains(&norm_word(&w.text).as_str());
            if !(marker && w.text.ends_with(',') || hesitation) {
                break;
            }
            i += 1;
        }
        i
    }

    /// Whether sentence `a` really starts a sentence: the one before ended with punctuation (or
    /// another speaker spoke), and it isn't written in lower case (whisper capitalises sentences).
    fn starts_sentence(&self, a: usize) -> bool {
        let s = &self.sents[a];
        let first = &self.words[s.first];
        let after_stop = match s.first.checked_sub(1).map(|i| &self.words[i]) {
            None => true,
            Some(p) => ends_sentence(&p.text) || p.speaker != first.speaker,
        };
        after_stop && !first.text.trim_start_matches(|c: char| !c.is_alphanumeric()).starts_with(char::is_lowercase)
    }

    /// Whether sentence `k` asks something: a question mark, or "Dus jij vindt ..." / "So you think ...".
    fn asks(&self, k: usize) -> bool {
        let s = &self.sents[k];
        if is_question(&self.words[s.last].text) {
            return true;
        }
        let t = tokens(&self.words[s.first..=s.last.min(s.first + 2)]);
        t.len() >= 2 && ["dus", "so"].contains(&t[0].as_str()) && YOU.contains(&t[1].as_str())
    }

    /// Whether sentence `b` ends one (punctuation, or the end of the recording).
    fn ends_sentence(&self, b: usize) -> bool {
        let l = self.sents[b].last;
        l + 1 == self.words.len() || ends_sentence(&self.words[l].text)
    }

    /// Score sentences `a..=b`; None when they don't start and end on sentence boundaries.
    fn score(&self, a: usize, b: usize) -> Option<Pick> {
        if !self.starts_sentence(a) || !self.ends_sentence(b) {
            return None;
        }
        let (sa, sb) = (&self.sents[a], &self.sents[b]);
        let first_word = self.trim_start(sa);
        let ws = &self.words[first_word..=sb.last];
        let length = est_length(ws);
        let mut r: Vec<(f64, String)> = vec![];

        // the hook
        let hook = &self.words[first_word..=sa.last];
        let ht = tokens(hook);
        let hj = joined(&ht);
        let w0 = ht.first().cloned().unwrap_or_default();
        let w1 = ht.get(1).cloned().unwrap_or_default();
        if first_word > sa.first {
            r.push((-0.3, format!("drops the leading \"{}\"", quote(&self.words[sa.first..first_word]).trim_end_matches(','))));
        }
        // "En wat ...?" is how interviewers ask; "Maar ...?" / "Waardoor ...?" is the middle of a thought
        let question =
            is_question(&self.words[sa.last].text) && (!CONTINUATIONS.contains(&w0.as_str()) || ["en", "and"].contains(&w0.as_str()));
        if question {
            r.push((if ht.len() >= 5 { 2.0 } else { 1.0 }, "opens with a question".into()));
        }
        if hook.iter().any(|w| w.text.chars().any(|c| c.is_ascii_digit())) || ht.iter().any(|t| NUMBER_WORDS.contains(&t.as_str())) {
            r.push((1.0, "a number in the first sentence".into()));
        }
        if let Some(p) = has_phrase(&hj, HOOKS) {
            r.push((1.5, format!("hook words: \"{p}\"")));
        }
        if let Some(p) = has_phrase(&hj, CONTRASTS) {
            r.push((1.0, format!("a contrast: \"{p}\"")));
        } else if (hj.contains(" niet ") && hj.contains(" maar ")) || (hj.contains(" not ") && hj.contains(" but ")) {
            r.push((1.0, "a contrast: \"not ... but\"".into()));
        }
        if let Some(p) = has_phrase(&hj, GREETINGS) {
            r.push((-1.0, format!("opens with a greeting/introduction (\"{p}\")")));
        }
        if CONTINUATIONS.contains(&w0.as_str()) && question {
            // interviewers start questions with "En ...": less of a problem
            r.push((-1.0, format!("a question starting with \"{w0}\"")));
        } else if CONTINUATIONS.contains(&w0.as_str()) {
            r.push((-3.0, format!("starts with \"{w0}\": continues an earlier thought")));
        } else if MARKERS.contains(&w0.as_str()) {
            r.push((-2.0, format!("starts with \"{w0}\": a reply, not an opening")));
        } else if ["dat", "that", "die"].contains(&w0.as_str()) && PRONOUNS.contains(&w1.as_str()) && !question {
            r.push((-3.0, format!("\"{w0} {w1} ...\" is the second half of a sentence")));
        } else if BACK_REFS.contains(&w0.as_str()) && VERBS.contains(&w1.as_str()) {
            r.push((-2.5, format!("\"{w0} {w1}\" refers back to something not in the clip")));
        } else if SOFT_BACK_REFS.contains(&w0.as_str()) && VERBS.contains(&w1.as_str()) {
            r.push((-1.0, format!("\"{w0} {w1}\" may refer back")));
        } else if BACK_ALWAYS.contains(&w0.as_str()) {
            r.push((-2.0, format!("starts with \"{w0}\": refers back to something not in the clip")));
        } else if VERBS.contains(&w0.as_str()) && !question {
            // "Is het tien jaar ..." without a question mark: the second half of a sentence
            r.push((-2.0, format!("starts with the verb \"{w0}\": the rest of a sentence")));
        }
        let n0 = ht.len();
        if n0 < 4 && !question {
            r.push((-1.5, format!("first sentence too short to hook ({n0} words)")));
        } else if n0 > 35 {
            r.push((-1.0, format!("long-winded first sentence ({n0} words)")));
        }
        let gap_before = if sa.first == 0 { f64::INFINITY } else { sa.start - self.words[sa.first - 1].end };
        // a question just before (one or two sentences, a few seconds): this is (part of) its answer
        let asked = (a.saturating_sub(2)..a).rev().find(|&k| self.asks(k) && sa.start - self.sents[k].end < 8.0);
        if let (Some(k), false) = (asked, question) {
            let what = if k + 1 == a { "answers a question that isn't in the clip" } else { "starts halfway through an answer" };
            r.push((-1.5, format!("{what} (\"{}\")", short(&quote(&self.words[self.sents[k].first..=self.sents[k].last])))));
        } else if gap_before >= 0.7 || (a > 0 && self.words[sa.first - 1].speaker != self.words[sa.first].speaker) {
            r.push((0.5, "starts after a pause: a fresh thought".into()));
        }

        // the ending
        let lw = &self.words[sb.last];
        let lt = tokens(&self.words[sb.first..=sb.last]);
        let lj = joined(&lt);
        if trails_off(&lw.text) {
            r.push((-3.0, "trails off at the end (\"...\")".into()));
        } else if self.asks(b) {
            r.push((-1.5, "ends on a question (the answer is missing)".into()));
        } else if lw.text.ends_with('!') {
            r.push((0.5, "ends on an exclamation".into()));
        }
        match self.pause_after(sb.last) {
            None => r.push((1.0, "ends where the recording ends".into())),
            Some(p) if p >= 1.0 => r.push((1.0, format!("a {p:.1} s pause after the last sentence"))),
            Some(p) if p >= 0.5 => r.push((0.5, format!("a {p:.1} s pause after the last sentence"))),
            // whisper often stretches a word up to the next one: weak evidence, a small penalty
            Some(p) if p < 0.15 => r.push((-0.5, format!("no pause after the last sentence ({p:.2} s)"))),
            _ => {}
        }
        if b > a && lt.first().is_some_and(|t| ["want", "because", "omdat", "cause"].contains(&t.as_str())) {
            r.push((-1.5, format!("ends on a \"{} ...\" that sets up more", lt[0])));
        }
        if let Some(p) = has_phrase(&lj, SIGN_OFFS) {
            r.push((-1.0, format!("ends on a thank-you/goodbye (\"{p}\")")));
        }
        // a call to action in the last two sentences: the natural end of a reel
        let tail = tokens(&self.words[self.sents[b.saturating_sub(1).max(a)].first..=sb.last]);
        let cta = has_phrase(&joined(&tail), CALLS_TO_ACTION);
        if let Some(p) = &cta {
            r.push((1.0, format!("ends on a call to action (\"{p}\")")));
        }
        let conclusive = if cta.is_some() || self.asks(b) {
            cta.is_some()
        } else if lt.first().is_some_and(|t| CONCLUSION_STARTS.contains(&t.as_str())) {
            r.push((1.0, format!("ends on a conclusion (\"{} ...\")", lt[0])));
            true
        } else if let Some(p) = has_phrase(&lj, CONCLUSION_PHRASES) {
            r.push((1.0, format!("ends on a conclusion (\"{p}\")")));
            true
        } else {
            false
        };
        if let Some(next) = self.sents.get(b + 1) {
            let nt = tokens(&self.words[next.first..=next.last]);
            let n0 = nt.first().map(String::as_str).unwrap_or("");
            if ["maar", "but", "dus", "so", "daarom", "therefore", "waardoor"].contains(&n0) {
                // the turn or the conclusion is missing
                r.push((-1.5, format!("the next sentence completes the thought (\"{n0} ...\")")));
            } else if !conclusive && ["want", "because", "omdat", "en", "and", "which"].contains(&n0) {
                r.push((-0.75, format!("the next sentence goes on (\"{n0} ...\")")));
            }
        }
        if b > a && lt.len() <= 2 && !lw.text.ends_with('!') {
            r.push((-1.5, format!("fizzles out on \"{}\"", quote(&self.words[sb.first..=sb.last]))));
        }

        // the body: one thought, pace, fillers, broken sentences
        // the opening question may take more than one sentence ("Do you remember? Where were you?")
        let opening = (a..=b).take_while(|&k| self.asks(k)).count();
        for k in a..b {
            let s = &self.sents[k];
            let said = || short(&quote(&self.words[s.first..=s.last]));
            if k >= a + opening.max(1) && self.asks(k) {
                let next = &self.sents[k + 1];
                let n0 = norm_word(&self.words[next.first].text);
                if MARKERS.contains(&n0.as_str()) || self.words[next.first].speaker != self.words[s.last].speaker {
                    r.push((-1.5, format!("a new question halfway (\"{}\"): two topics", said())));
                }
            }
            if k > a && trails_off(&self.words[s.last].text) {
                r.push((-0.75, format!("a broken-off sentence (\"{}\")", said())));
            }
            if k > a && has_phrase(&joined(&tokens(&self.words[s.first..=s.last])), SIGN_OFFS).is_some() {
                r.push((-1.0, format!("a thank-you/goodbye halfway (\"{}\"): the end of a conversation", said())));
            }
        }
        // short reactions inside the clip ("Gelukkig, ja.", "Prima.", "Yes."): a back-and-forth, not one voice
        let reactions = (a + 1..b).filter(|&k| tokens(&self.words[self.sents[k].first..=self.sents[k].last]).len() <= 3).count();
        if reactions > 1 {
            r.push((-0.75 * (reactions - 1) as f64, format!("{reactions} short reactions: a back-and-forth")));
        }
        let speakers = ws.windows(2).filter(|p| p[0].speaker != p[1].speaker).count();
        if speakers > 2 {
            r.push((-0.5 * (speakers - 2) as f64, format!("{speakers} speaker changes")));
        }
        let long_gaps = ws.windows(2).filter(|p| p[1].start - p[0].end > 1.5).count();
        if long_gaps > 0 {
            r.push(((-0.5 * long_gaps as f64).max(-2.0), format!("{long_gaps} silence(s) over 1.5 s")));
        }
        let tk = tokens(ws);
        let wps = tk.len() as f64 / length.max(1.0);
        if wps < 1.6 {
            r.push((-1.0, format!("slow ({wps:.1} words/s)")));
        } else if wps > 2.6 {
            r.push((0.5, format!("lively pace ({wps:.1} words/s)")));
        }
        let tj = joined(&tk);
        let fillers = tk.iter().filter(|t| DEFAULT_FILLERS.contains(&t.as_str())).count()
            + SOFT_FILLERS.iter().map(|p| tj.matches(&format!(" {p} ")).count()).sum::<usize>();
        let rate = fillers as f64 / tk.len().max(1) as f64 * 100.0;
        if rate > 2.0 {
            r.push((-((rate - 2.0) * 0.4).min(2.5), format!("{rate:.0}% filler words")));
        }
        let n = b - a + 1;
        let shorts = (a..=b).filter(|&k| self.sents[k].last - self.sents[k].first < 2).count();
        let per_sentence = tk.len() as f64 / n as f64;
        if n >= 4 && (shorts * 4 > n || per_sentence < 6.0) {
            r.push((-1.5, format!("short exchanges ({per_sentence:.0} words a sentence, {shorts} of {n} one or two words)")));
        }

        // one voice telling it (with diarization labels): the guest's answer, not a back-and-forth
        let labelled: Vec<&String> = ws.iter().filter_map(|w| w.speaker.as_ref()).collect();
        if !labelled.is_empty() && labelled.len() * 10 >= ws.len() * 8 {
            let mut count: Vec<(&String, usize)> = vec![];
            for s in &labelled {
                match count.iter_mut().find(|(k, _)| k == s) {
                    Some(c) => c.1 += 1,
                    None => count.push((s, 1)),
                }
            }
            let (who, n) = count.iter().max_by_key(|c| c.1).copied().unwrap();
            if n * 100 >= labelled.len() * 85 {
                r.push((0.75, format!("one voice ({who}) tells it")));
            }
        }

        // the length
        let off = (length - self.target).abs() / self.target;
        if off > 0.08 {
            r.push((-(off * 4.0), format!("{length:.0} s for a {:.0} s target", self.target)));
        }

        let score = r.iter().map(|(p, _)| p).sum();
        r.sort_by(|p, q| q.0.abs().total_cmp(&p.0.abs()));
        let fw = &self.words[first_word];
        let prev_end = first_word.checked_sub(1).map(|i| self.words[i].end).unwrap_or(0.0);
        let from = (fw.start - ((fw.start - prev_end) / 2.0).clamp(0.0, 0.3)).max(0.0);
        let next_start = self.words.get(sb.last + 1).map(|w| w.start).unwrap_or(self.total.max(lw.end));
        let to = lw.end + ((next_start - lw.end) / 2.0).clamp(0.0, 0.5);
        Some(Pick { from, to, length, score, first: quote(hook), last: quote(&self.words[sb.first..=sb.last]), reasons: r })
    }
}

fn short(s: &str) -> String {
    if s.chars().count() <= 48 {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(45).collect::<String>().trim_end())
    }
}

/// The best `count` non-overlapping parts of about `target` seconds, best first. `total` is the
/// recording's length (0 = unknown).
pub fn pick(words: &[Word], target: f64, count: usize, total: f64) -> Vec<Pick> {
    let mut words: Vec<Word> = words.iter().filter(|w| !w.text.trim().is_empty()).cloned().collect();
    words.sort_by(|a, b| a.start.total_cmp(&b.start));
    let sents = sentences(&words);
    if sents.is_empty() {
        return vec![];
    }
    let all = est_length(&words);
    // a short recording: anything from most of it up
    let min = (target * 0.6).min(all * 0.8);
    let max = target * 1.35;
    let sc = Scorer { words: &words, sents: &sents, total, target };
    let mut cands: Vec<Pick> = vec![];
    for a in 0..sents.len() {
        for b in a..sents.len() {
            let len = est_length(&words[sents[a].first..=sents[b].last]);
            if len > max {
                break;
            }
            if len >= min {
                cands.extend(sc.score(a, b));
            }
        }
    }
    cands.sort_by(|p, q| q.score.total_cmp(&p.score));
    // several picks should be several stories: apart in time (in a long recording) and about
    // something else (few content words in common)
    let gap = if total > 600.0 { (total * 0.03).min(90.0) } else { 0.0 };
    let topic = |p: &Pick| content_words(words.iter().filter(|w| w.start >= p.from && w.end <= p.to));
    let mut out: Vec<(Pick, HashSet<String>)> = vec![];
    for c in cands {
        if out.len() >= count {
            break;
        }
        if !out.iter().all(|(o, _)| c.to + gap <= o.from || c.from >= o.to + gap) {
            continue;
        }
        let words = topic(&c);
        if out.iter().any(|(_, w)| similarity(&words, w) > MAX_TOPIC_OVERLAP) {
            continue;
        }
        out.push((c, words));
    }
    out.into_iter().map(|(p, _)| p).collect()
}

/// Two picks sharing more than this share of the smaller one's content words tell the same story.
const MAX_TOPIC_OVERLAP: f64 = 0.35;

/// Common long words that say nothing about the topic.
const STOP_WORDS: &[&str] = &[
    "eigenlijk",
    "gewoon",
    "natuurlijk",
    "zeker",
    "echt",
    "heel",
    "allemaal",
    "wordt",
    "worden",
    "hebben",
    "zijn",
    "kunnen",
    "moeten",
    "zouden",
    "willen",
    "gaan",
    "gaat",
    "denk",
    "vind",
    "weet",
    "zeggen",
    "zegt",
    "beetje",
    "misschien",
    "omdat",
    "waarom",
    "wanneer",
    "daarom",
    "daarna",
    "toen",
    "andere",
    "ander",
    "mensen",
    "dingen",
    "about",
    "because",
    "there",
    "their",
    "would",
    "could",
    "should",
    "really",
    "think",
    "people",
    "things",
    "actually",
    "which",
    "where",
    "these",
    "those",
    "being",
    "going",
];

/// The topic of a stretch of words: its content words (5+ letters, not a stop word), lower case.
fn content_words<'a>(words: impl Iterator<Item = &'a Word>) -> HashSet<String> {
    words
        .map(|w| w.text.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|t| t.chars().count() >= 5 && !STOP_WORDS.contains(&t.as_str()))
        .collect()
}

/// Overlap of two word sets: the share of the smaller set the other also has (a short part that
/// repeats a long one's subject counts as the same story).
fn similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    a.intersection(b).count() as f64 / a.len().min(b.len()) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words from sentences, each word 0.3 s with 0.05 s gaps; `|` in the text inserts a 1.2 s pause.
    fn talk(text: &str) -> Vec<Word> {
        let mut t = 0.0;
        let mut out = vec![];
        for tok in text.split_whitespace() {
            if tok == "|" {
                t += 1.2;
                continue;
            }
            out.push(Word { text: tok.into(), start: t, end: t + 0.3, speaker: None });
            t += 0.35;
        }
        out
    }

    #[test]
    fn several_picks_are_several_stories() {
        let a = "Waarom kiezen jongeren voor de waterbeheer politiek? Waterbeheer raakt iedereen die dichtbij rivieren woont.                  Daarom stemmen jongeren vaker voor waterbeheer en dijken. Dijken beschermen onze dorpen tegen rivieren.";
        let b = "Hoe kijken jongeren naar waterbeheer en rivieren? Rivieren en dijken vragen onderhoud van waterbeheer.                  Daarom vinden jongeren waterbeheer belangrijk voor dijken. Dijken en rivieren horen bij onze dorpen.";
        let c = "Wat betekent muziek voor studenten tijdens examens? Muziek helpt studenten zich concentreren tijdens studeren.                  Daarom luisteren studenten vaker naar rustige muziek. Examens voelen dan minder zwaar voor studenten.";
        // a, then the same story told again, then another story; | is a pause
        let w = talk(&format!("{a} | | {b} | | {c}"));
        let picks = pick(&w, 10.0, 3, w.last().unwrap().end + 1.0);
        let firsts: Vec<&str> = picks.iter().map(|p| p.first.as_str()).collect();
        assert!(firsts.iter().any(|f| f.contains("muziek")), "{firsts:?}");
        // the two water stories don't both make it
        assert!(firsts.iter().filter(|f| f.contains("waterbeheer") || f.contains("rivieren")).count() <= 1, "{firsts:?}");
        // the overlap measure itself
        let set = |t: &str| content_words(talk(t).iter());
        assert!(similarity(&set(a), &set(b)) > MAX_TOPIC_OVERLAP);
        assert!(similarity(&set(a), &set(c)) < MAX_TOPIC_OVERLAP);
    }

    #[test]
    fn one_voice_scores_above_a_back_and_forth() {
        let mut solo = talk("Waarom is dit belangrijk voor ons allemaal? Omdat het over onze toekomst gaat. Daarom doen we mee.");
        solo.iter_mut().for_each(|w| w.speaker = Some("SPEAKER_01".into()));
        let mut mixed = solo.clone();
        for (i, w) in mixed.iter_mut().enumerate() {
            if (i / 4) % 2 == 1 {
                w.speaker = Some("SPEAKER_00".into());
            }
        }
        let total = solo.last().unwrap().end + 1.0;
        let s = pick(&solo, 6.0, 1, total);
        assert!(s[0].reasons.iter().any(|(_, why)| why.contains("one voice")), "{:?}", s[0].reasons);
        let m = pick(&mixed, 6.0, 1, total);
        assert!(m.iter().all(|p| !p.reasons.iter().any(|(_, why)| why.contains("one voice"))));
    }

    #[test]
    fn splits_on_punctuation_and_long_pauses() {
        let w = talk("Hello there. How are you? Fine | thanks and you");
        let s = sentences(&w);
        assert_eq!(s.iter().map(|s| (s.first, s.last)).collect::<Vec<_>>(), [(0, 1), (2, 4), (5, 8)]);
    }

    #[test]
    fn never_starts_or_ends_mid_sentence() {
        let w = talk(&"This is one complete sentence that says something. ".repeat(30));
        let picks = pick(&w, 20.0, 1, 0.0);
        let p = &picks[0];
        assert!(p.first.starts_with("This") && p.last.ends_with('.'), "{p:?}");
        assert!((p.length - 20.0).abs() < 4.0, "{}", p.length);
    }

    /// A monologue with a weak stretch (replies, back references) and a strong stretch (a question
    /// as the hook, a number, a conclusion before a pause).
    fn monologue() -> Vec<Word> {
        let filler = "and then we went on with the rest of the day as usual. ";
        let weak = format!("Yeah, that was it. {} That is what he said, uh, you know. ", filler.repeat(5));
        let strong = "Why do most people quit after 3 weeks? The reason is simple: they never plan the boring days. \
            When the first rush is gone, a plan carries you through the dull weeks. \
            Write down what you will do on a bad day, before it comes. \
            Keep it small, so small that you cannot say no to it. \
            That is why the people who plan the boring days keep going. | ";
        let tail = "And another thing about the weather is that it was cold. ".repeat(4);
        talk(&format!("{weak}{strong}{tail}"))
    }

    #[test]
    fn prefers_a_strong_hook_and_a_conclusive_end() {
        let picks = pick(&monologue(), 22.0, 3, 0.0);
        let best = &picks[0];
        assert!(best.first.starts_with("Why do most people"), "{best:#?}");
        assert!(best.last.starts_with("That is why"), "{best:#?}");
        assert!(best.reasons.iter().any(|(p, why)| *p > 0.0 && why.contains("question")));
        // non-overlapping and ranked
        for (i, p) in picks.iter().enumerate() {
            for q in &picks[i + 1..] {
                assert!(p.to <= q.from || p.from >= q.to);
                assert!(p.score >= q.score);
            }
        }
    }

    #[test]
    fn explains_weak_openings() {
        let w = talk("We met at noon. And then it rained all day long. That was the end of it all. Ja, het was een mooie dag vandaag. Dat is wat ik denk.");
        let s = sentences(&w);
        let sc = Scorer { words: &w, sents: &s, total: 0.0, target: 5.0 };
        let why = |a| sc.score(a, a).unwrap().reasons.iter().map(|r| r.1.clone()).collect::<Vec<_>>().join("; ");
        assert!(why(1).contains("starts with \"and\""), "{}", why(1));
        assert!(why(2).contains("\"that was\" refers back"), "{}", why(2));
        // "Ja," is trimmed off; "het was" may refer back
        let p = sc.score(3, 3).unwrap();
        assert!(p.first.starts_with("het was"), "{}", p.first);
        assert!(why(3).contains("drops the leading \"Ja\""), "{}", why(3));
        assert!(why(4).contains("\"dat is\" refers back"), "{}", why(4));
    }

    #[test]
    fn an_answer_without_its_question_and_a_trailing_end_are_penalised() {
        let w = talk("What do you think of it? I think it is great and I would do it again. But then...");
        let s = sentences(&w);
        let sc = Scorer { words: &w, sents: &s, total: 0.0, target: 5.0 };
        let answer = sc.score(1, 1).unwrap();
        assert!(answer.reasons.iter().any(|r| r.1.contains("answers a question")), "{:?}", answer.reasons);
        assert!(answer.reasons.iter().any(|r| r.1.contains("next sentence completes the thought")), "{:?}", answer.reasons);
        let with_question = sc.score(0, 1).unwrap();
        assert!(with_question.score > answer.score);
        let trailing = sc.score(1, 2).unwrap();
        assert!(trailing.reasons.iter().any(|r| r.1.contains("trails off")));
    }

    #[test]
    fn dutch_cues() {
        let w = talk("Het probleem is niet het geld, maar de tijd. Dus begin vandaag! | Nou, en toen?");
        let s = sentences(&w);
        let sc = Scorer { words: &w, sents: &s, total: 0.0, target: 4.0 };
        let p = sc.score(0, 1).unwrap();
        let why = p.reasons.iter().map(|r| r.1.as_str()).collect::<Vec<_>>().join("; ");
        assert!(why.contains("hook words: \"het probleem\""), "{why}");
        assert!(why.contains("contrast"), "{why}");
        assert!(why.contains("ends on a conclusion (\"dus"), "{why}");
        assert!(why.contains("exclamation"), "{why}");
    }

    #[test]
    fn a_new_interview_question_halfway_means_two_topics() {
        let w = talk("Why does this matter so much to you? Because it changed my life completely. How did you start? Yeah, I started small with one thing.");
        let s = sentences(&w);
        let sc = Scorer { words: &w, sents: &s, total: 0.0, target: 6.0 };
        let p = sc.score(0, 3).unwrap();
        assert!(p.reasons.iter().any(|r| r.1.contains("new question halfway")), "{:?}", p.reasons);
    }

    #[test]
    fn a_compound_opening_question_is_one_question() {
        let w = talk("Do you remember that day ten years ago? Where were you then? Yes, I was right there in the front row.");
        let s = sentences(&w);
        let sc = Scorer { words: &w, sents: &s, total: 0.0, target: 6.0 };
        let p = sc.score(0, 2).unwrap();
        assert!(!p.reasons.iter().any(|r| r.1.contains("new question halfway")), "{:?}", p.reasons);
    }

    #[test]
    fn fragments_and_mid_sentence_starts_are_not_openings() {
        // a pause split the sentence: "in the long run" is not a start
        let w = talk("We kept going | | in the long run it paid off. Dat zij er altijd mee bezig zijn. Is het tien jaar al.");
        let s = sentences(&w);
        let sc = Scorer { words: &w, sents: &s, total: 0.0, target: 3.0 };
        assert!(sc.score(1, 1).is_none(), "starts in lower case after a pause split");
        assert!(sc.score(0, 0).is_none(), "doesn't end on a sentence end");
        let why = |a| sc.score(a, a).unwrap().reasons.iter().map(|r| r.1.clone()).collect::<Vec<_>>().join("; ");
        assert!(why(2).contains("second half of a sentence"), "{}", why(2));
        assert!(why(3).contains("starts with the verb \"is\""), "{}", why(3));
    }

    #[test]
    fn endings_calls_to_action_paraphrases_and_goodbyes() {
        let w = talk(
            "Why should you care about this at all? Because it decides your future. Join us and vote next week. \
             So you think everyone should vote. Thanks for having me. Bye.",
        );
        let s = sentences(&w);
        let sc = Scorer { words: &w, sents: &s, total: 0.0, target: 5.0 };
        let why = |a, b| sc.score(a, b).unwrap().reasons.iter().map(|r| r.1.clone()).collect::<Vec<_>>().join("; ");
        assert!(why(0, 2).contains("call to action (\"join\")"), "{}", why(0, 2));
        // "So you think ..." is the interviewer checking, not a conclusion
        assert!(why(0, 3).contains("ends on a question"), "{}", why(0, 3));
        assert!(why(0, 4).contains("thank-you/goodbye"), "{}", why(0, 4));
        assert!(why(0, 5).contains("fizzles out"), "{}", why(0, 5));
    }

    #[test]
    fn short_recordings_still_give_a_pick() {
        let w = talk("This is short. It has just three sentences. Bye now.");
        let p = pick(&w, 45.0, 1, 0.0);
        assert_eq!(p.len(), 1);
    }
}
