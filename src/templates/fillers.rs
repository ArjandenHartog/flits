//! Filler words per language and repeated-word (stutter) detection on word-timed transcripts.
//! Both return the indices of words to remove; `cut` turns them into time ranges.

use crate::compose::captions::Word;

/// Filler lists per language (ISO 639-1). Only hesitation sounds: words like "like",
/// "you know" or "nou" carry meaning too often to remove blindly (use `--fillers-list`).
const FILLERS: &[(&str, &[&str])] = &[
    ("en", &["uh", "uhh", "um", "umm", "uhm", "er", "erm", "ah", "ahh", "hm", "hmm", "mm", "mmm", "mhm"]),
    ("nl", &["uh", "uhh", "uhm", "uhmm", "eh", "ehh", "ehm", "euh", "eeh", "hm", "hmm", "mm", "mmm", "ah"]),
    ("de", &["äh", "ähh", "ähm", "öh", "öhm", "eh", "ehm", "hm", "hmm", "mhm", "mm", "mmm"]),
    ("fr", &["euh", "euhh", "euhm", "heu", "hum", "hm", "hmm", "mmh", "mm", "bah"]),
    ("es", &["eh", "ehh", "ehm", "em", "emm", "mmm", "mm", "hmm", "hm", "ah"]),
    ("it", &["eh", "ehh", "ehm", "uhm", "mmm", "mm", "hmm", "hm"]),
    ("pt", &["hum", "hã", "ãh", "ahn", "éé", "hmm", "hm", "mm"]),
];

/// Used when the language is unknown or not in the table.
pub const DEFAULT_FILLERS: &[&str] =
    &["uh", "uhm", "um", "umm", "eh", "ehm", "euh", "uhh", "hm", "hmm", "mm", "ah", "eeh", "äh", "ähm", "er", "erm"];

/// Languages with a built-in filler list.
pub fn languages() -> Vec<&'static str> {
    FILLERS.iter().map(|(l, _)| *l).collect()
}

/// Filler list for a language code ("nl", "en-US", "German"...); unknown = `DEFAULT_FILLERS`.
pub fn for_language(lang: Option<&str>) -> Vec<String> {
    let code = lang.map(|l| l.trim().to_ascii_lowercase()).unwrap_or_default();
    let code = match code.as_str() {
        "dutch" => "nl",
        "english" => "en",
        "german" => "de",
        "french" => "fr",
        "spanish" => "es",
        "italian" => "it",
        "portuguese" => "pt",
        c => c.get(..2).unwrap_or(c),
    };
    let list = FILLERS.iter().find(|(l, _)| *l == code).map(|(_, f)| *f).unwrap_or(DEFAULT_FILLERS);
    list.iter().map(|s| s.to_string()).collect()
}

/// Whisper tends to leave hesitations out of its transcript. A prompt that contains them makes
/// it write them down (so they can be cut). Only used when the language is given explicitly.
pub fn verbatim_prompt(lang: &str) -> Option<&'static str> {
    Some(match lang.get(..2)? {
        "en" => "Umm, let me think like, hmm... Okay, here's what I'm, like, thinking.",
        "nl" => "Ehm, nou, even denken, uh... Oké, ehm, dit is wat ik, uh, bedoel.",
        "de" => "Ähm, also, lass mich mal überlegen, äh... Okay, ähm, das ist, äh, was ich meine.",
        "fr" => "Euh, bon, laisse-moi réfléchir, euh... Bon, hum, c'est ce que, euh, je veux dire.",
        "es" => "Eh, bueno, déjame pensar, em... Vale, eh, es lo que, mmm, quiero decir.",
        "it" => "Ehm, allora, fammi pensare, eh... Ok, ehm, è quello che, eh, voglio dire.",
        "pt" => "Hum, bom, deixa eu pensar, ãh... Tá, hum, é o que, ãh, eu quero dizer.",
        _ => return None,
    })
}

/// Parse a user list like "uh, um, you know" (phrases allowed).
pub fn parse_list(s: &str) -> Vec<String> {
    s.split([',', ';', '\n']).map(norm_phrase).filter(|p| !p.is_empty()).collect()
}

/// Lower case, punctuation trimmed from both ends ("Uhm," -> "uhm"; inner hyphens stay).
pub fn norm_word(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn norm_phrase(p: &str) -> String {
    p.split_whitespace().map(norm_word).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Indices of words that are fillers. Entries may be phrases ("you know"); the longest match wins.
pub fn find_fillers(words: &[Word], fillers: &[String]) -> Vec<usize> {
    let mut phrases: Vec<Vec<String>> = fillers.iter().map(|f| norm_phrase(f).split(' ').map(str::to_string).collect()).collect();
    phrases.retain(|p| !p.is_empty() && !p[0].is_empty());
    phrases.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let norm: Vec<String> = words.iter().map(|w| norm_word(&w.text)).collect();
    let mut out = vec![];
    let mut i = 0;
    while i < words.len() {
        match phrases.iter().find(|p| i + p.len() <= norm.len() && p.iter().zip(&norm[i..]).all(|(a, b)| a == b)) {
            Some(p) => {
                out.extend(i..i + p.len());
                i += p.len();
            }
            None => i += 1,
        }
    }
    out
}

fn ends_sentence(w: &str) -> bool {
    w.trim_end_matches(['"', '\'', ')', '»', '”']).ends_with(['.', '?', '!', '…'])
}

/// Indices of repeated words and stutters to remove: "I I I think" drops the first two "I"s,
/// "I think I think we" drops the first "I think", and a cut-off fragment like "th- the" drops
/// "th-". The last take is kept. Repeats across a sentence end or a pause over a second are
/// left alone ("No. No, ...").
pub fn find_repeats(words: &[Word]) -> Vec<usize> {
    const MAX_GAP: f64 = 1.0;
    let norm: Vec<String> = words.iter().map(|w| norm_word(&w.text)).collect();
    let mut out = vec![];
    let mut i = 0;
    'outer: while i < words.len() {
        if norm[i].is_empty() {
            i += 1;
            continue;
        }
        // a cut-off fragment ("th-", "wa—") followed by the word it starts
        let raw = words[i].text.trim_end();
        if (raw.ends_with('-') || raw.ends_with('—') || raw.ends_with('–'))
            && i + 1 < words.len()
            && norm[i + 1].starts_with(&norm[i])
            && norm[i + 1] != norm[i]
            && words[i + 1].start - words[i].end < MAX_GAP
        {
            out.push(i);
            i += 1;
            continue;
        }
        // an n-gram (longest first) immediately repeated
        for n in (1..=3).rev() {
            if i + 2 * n > words.len() {
                continue;
            }
            let (a, b) = (&norm[i..i + n], &norm[i + n..i + 2 * n]);
            if a != b || a.iter().any(String::is_empty) {
                continue;
            }
            let last = &words[i + n - 1];
            if ends_sentence(&last.text) || words[i + n].start - last.end > MAX_GAP {
                continue;
            }
            out.extend(i..i + n);
            i += n;
            continue 'outer;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(text: &str) -> Vec<Word> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| Word { text: w.into(), start: i as f64 * 0.4, end: i as f64 * 0.4 + 0.3, speaker: None })
            .collect()
    }

    #[test]
    fn picks_the_list_for_the_language() {
        assert!(for_language(Some("nl")).contains(&"ehm".to_string()));
        assert!(for_language(Some("de")).contains(&"ähm".to_string()));
        assert!(for_language(Some("fr")).contains(&"euh".to_string()));
        assert!(for_language(Some("en-US")).contains(&"erm".to_string()));
        assert!(for_language(Some("Spanish")).contains(&"emm".to_string()));
        assert_eq!(for_language(Some("xx")).len(), DEFAULT_FILLERS.len());
        assert_eq!(for_language(None).len(), DEFAULT_FILLERS.len());
        for l in ["nl", "en", "de", "fr", "es"] {
            assert!(languages().contains(&l));
        }
    }

    #[test]
    fn finds_fillers_and_phrases() {
        let w = words("So, uhm, I think, you know, it's ähm fine. Uh.");
        let f = find_fillers(&w, &parse_list("uh, uhm, You Know"));
        assert_eq!(f, vec![1, 4, 5, 9]);
        let de = find_fillers(&w, &for_language(Some("de")));
        assert_eq!(de, vec![7]);
        assert_eq!(norm_word(" Uhm,"), "uhm");
        assert_eq!(norm_word("Mm-hmm."), "mm-hmm");
    }

    #[test]
    fn finds_repeats_and_stutters() {
        assert_eq!(find_repeats(&words("I I I think so")), vec![0, 1]);
        assert_eq!(find_repeats(&words("I think I think we should")), vec![0, 1]);
        assert_eq!(find_repeats(&words("we th- the plan")), vec![1]);
        assert_eq!(find_repeats(&words("The the plan")), vec![0]);
        // a sentence end between them: deliberate
        assert!(find_repeats(&words("No. No, that is fine")).is_empty());
        assert!(find_repeats(&words("this is fine")).is_empty());
        // a long pause between them: not a stutter
        let mut w = words("well well done");
        w[1].start += 2.0;
        w[1].end += 2.0;
        assert!(find_repeats(&w).is_empty());
    }
}
