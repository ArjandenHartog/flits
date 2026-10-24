//! Layout checks for `flits check`: text that runs off the canvas, text on top of other text,
//! and text that is not on screen long enough to read. Sampled over the timeline with the
//! transforms the renderer uses (no pixels are drawn), so it costs milliseconds.

use super::draw::{node_fx, node_ts};
use super::{Fill, Kind, Node, Prepared};
use serde::Serialize;
use std::collections::HashMap;
use tiny_skia::{Point, Transform};

/// A layout problem: what, which layer, and when.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    /// `offscreen` | `overlap` | `brief`
    pub kind: &'static str,
    /// the layer (its text, `id` or file name)
    pub layer: String,
    /// the other layer of an `overlap`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    /// scene time range (seconds)
    pub from: f64,
    pub to: f64,
    pub message: String,
}

/// Text partly outside the canvas this long (s) is reported (shorter = an entrance or exit).
const MIN_OFFSCREEN: f64 = 0.5;
/// Overlapping texts this long (s) are reported (shorter = a crossfade or a wipe).
const MIN_OVERLAP: f64 = 0.3;
/// Pixels of slack before text counts as outside the canvas.
const SLACK: f32 = 2.0;
/// Opacity from which a layer counts as visible.
const VISIBLE: f32 = 0.5;

struct Seen<'a> {
    n: &'a Node,
    /// canvas bounding box: x0, y0, x1, y1
    b: [f32; 4],
    op: f32,
    /// under a clipping group or a wipe/iris: going past the canvas is then a mask, not a mistake
    clipped: bool,
}

fn bounds(ts: Transform, w: f32, h: f32) -> [f32; 4] {
    let mut pts = [Point::from_xy(0.0, 0.0), Point::from_xy(w, 0.0), Point::from_xy(0.0, h), Point::from_xy(w, h)];
    ts.map_points(&mut pts);
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in pts {
        b = [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)];
    }
    b
}

/// Opaque solid rects at one moment (draw order id, canvas box): text under them is hidden on purpose.
type Cards = Vec<(usize, [f32; 4])>;

fn visit<'a>(n: &'a Node, t: f64, parent: Transform, parent_op: f32, clipped: bool, out: &mut Vec<Seen<'a>>, cards: &mut Cards) {
    if t < n.start || t >= n.end {
        return;
    }
    let (lt, dur) = (t - n.start, n.end - n.start);
    let fx = node_fx(n, lt, dur);
    let op = parent_op * n.opacity.eval(lt, dur).clamp(0.0, 1.0) as f32 * fx.opacity;
    if op <= 0.002 {
        return;
    }
    let Some(ts) = node_ts(n, lt, dur, parent, &fx) else { return };
    let clipped = clipped || fx.clip.is_some();
    match &n.kind {
        Kind::Group => {
            for c in &n.children {
                visit(c, t, ts, op, clipped || n.clip, out, cards);
            }
        }
        Kind::Text { .. } | Kind::Counter(_) => out.push(Seen { n, b: bounds(ts, n.w, n.h), op, clipped }),
        Kind::Rect(fill) if op >= 0.9 && !matches!(fill, Fill::Solid(c) if c.a < 0.9) => cards.push((n.id, bounds(ts, n.w, n.h))),
        _ => {}
    }
}

/// `text "Some long title…"` / `a counter` for messages.
fn describe(n: &Node) -> String {
    let kind = if matches!(n.kind, Kind::Counter(_)) { "counter" } else { "text" };
    let one = n.label.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.is_empty() {
        return format!("a {kind}");
    }
    let short: String = one.chars().take(40).collect();
    format!("{kind} \"{short}{}\"", if one.chars().count() > 40 { "…" } else { "" })
}

/// The part of a text box the glyphs mostly cover: line boxes include leading above and below,
/// so tightly stacked lines would otherwise count as overlapping.
fn ink(b: &[f32; 4]) -> [f32; 4] {
    let inset = (b[3] - b[1]) * 0.18;
    [b[0], b[1] + inset, b[2], b[3] - inset]
}

/// Times in issues: hundredths of a second.
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Seconds a text of `words` words needs on screen to be read (≈ 0.2 s per word; a single word
/// flashing by for 0.3 s, as in kinetic type, still reads).
pub fn read_time(words: usize) -> f64 {
    0.1 + 0.2 * words as f64
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Off(usize),
    Overlap(usize, usize),
}

struct Run {
    from: f64,
    to: f64,
    /// offscreen: largest overflow per edge (left, top, right, bottom); overlap: largest shared fraction
    worst: [f32; 4],
}

/// Check a prepared scene's layout over its whole timeline.
pub fn lint(p: &Prepared) -> Vec<Issue> {
    let (cw, ch) = (p.width as f32 / p.root_scale, p.height as f32 / p.root_scale);
    let step = (1.0 / p.fps).max(p.duration / 4000.0);
    let samples = (p.duration / step).ceil() as usize;
    let mut nodes: HashMap<usize, &Node> = HashMap::new();
    let mut open: HashMap<Key, Run> = HashMap::new();
    let mut done: Vec<(Key, Run)> = Vec::new();
    let mut visible: HashMap<usize, f64> = HashMap::new();
    let mut seen = Vec::new();
    let mut cards = Cards::new();
    for i in 0..samples {
        let t = i as f64 * step;
        seen.clear();
        cards.clear();
        for n in &p.nodes {
            visit(n, t, Transform::identity(), 1.0, false, &mut seen, &mut cards);
        }
        let mut hits: Vec<(Key, [f32; 4])> = Vec::new();
        for s in &seen {
            nodes.insert(s.n.id, s.n);
            if s.op < VISIBLE {
                continue;
            }
            *visible.entry(s.n.id).or_default() += step;
            let over = [-s.b[0], -s.b[1], s.b[2] - cw, s.b[3] - ch];
            let inside = s.b[2] > 0.0 && s.b[0] < cw && s.b[3] > 0.0 && s.b[1] < ch;
            if !s.clipped && inside && over.iter().any(|&o| o > SLACK) {
                hits.push((Key::Off(s.n.id), over));
            }
        }
        for (i, a) in seen.iter().enumerate() {
            for b in &seen[i + 1..] {
                // text on its own background box drawn over other text (a label over a ticker) covers it on purpose
                let (top, under) = if a.n.id > b.n.id { (a, b) } else { (b, a) };
                if a.op < VISIBLE || b.op < VISIBLE || top.n.backdrop {
                    continue;
                }
                let (ba, bb) = (ink(&a.b), ink(&b.b));
                let shared_box = [ba[0].max(bb[0]), ba[1].max(bb[1]), ba[2].min(bb[2]), ba[3].min(bb[3])];
                let (iw, ih) = (shared_box[2] - shared_box[0], shared_box[3] - shared_box[1]);
                if iw <= 0.0 || ih <= 0.0 {
                    continue;
                }
                // ... and so does an opaque card drawn between the two that covers where they meet
                let covered = cards.iter().any(|(id, c)| {
                    *id > under.n.id
                        && *id < top.n.id
                        && c[0] <= shared_box[0] + SLACK
                        && c[1] <= shared_box[1] + SLACK
                        && c[2] >= shared_box[2] - SLACK
                        && c[3] >= shared_box[3] - SLACK
                });
                if covered {
                    continue;
                }
                let area = |b: &[f32; 4]| ((b[2] - b[0]) * (b[3] - b[1])).max(1.0);
                let shared = iw * ih / area(&ba).min(area(&bb));
                if shared > 0.15 {
                    let (x, y) = (a.n.id.min(b.n.id), a.n.id.max(b.n.id));
                    hits.push((Key::Overlap(x, y), [shared, 0.0, 0.0, 0.0]));
                }
            }
        }
        let active: Vec<Key> = hits.iter().map(|h| h.0).collect();
        let closed: Vec<Key> = open.keys().filter(|k| !active.contains(k)).copied().collect();
        for k in closed {
            done.push((k, open.remove(&k).unwrap()));
        }
        for (k, w) in hits {
            let r = open.entry(k).or_insert(Run { from: t, to: t, worst: [0.0; 4] });
            r.to = t + step;
            for (a, b) in r.worst.iter_mut().zip(w) {
                *a = a.max(b);
            }
        }
    }
    done.extend(open.drain());

    let mut issues = Vec::new();
    for (k, r) in done {
        let len = r.to - r.from;
        match k {
            Key::Off(id) if len >= MIN_OFFSCREEN => {
                let n = nodes[&id];
                let edges: Vec<String> = ["left", "top", "right", "bottom"]
                    .iter()
                    .zip(r.worst)
                    .filter(|(_, o)| *o > SLACK)
                    .map(|(e, o)| format!("{:.0} px past the {e} edge", o))
                    .collect();
                issues.push(Issue {
                    kind: "offscreen",
                    layer: n.label.clone(),
                    other: None,
                    from: r2(r.from),
                    to: r2(r.to.min(p.duration)),
                    message: format!("{} runs {} (move it in, make it smaller or set maxWidth)", describe(n), edges.join(" and ")),
                });
            }
            Key::Overlap(a, b) if len >= MIN_OVERLAP => {
                let (na, nb) = (nodes[&a], nodes[&b]);
                issues.push(Issue {
                    kind: "overlap",
                    layer: na.label.clone(),
                    other: Some(nb.label.clone()),
                    from: r2(r.from),
                    to: r2(r.to.min(p.duration)),
                    message: format!(
                        "{} overlaps {} ({:.0}% of the smaller one; move one, or end one before the other starts)",
                        describe(na),
                        describe(nb),
                        r.worst[0].min(1.0) * 100.0
                    ),
                });
            }
            _ => {}
        }
    }
    let mut texts: Vec<&Node> = nodes.values().copied().filter(|n| matches!(n.kind, Kind::Text { .. })).collect();
    texts.sort_by_key(|n| n.id);
    for n in texts {
        let words = n.label.split_whitespace().count();
        let need = read_time(words);
        let had = visible.get(&n.id).copied().unwrap_or(0.0);
        // the scene's end cuts it off on purpose? still too short to read, so report it
        if words > 0 && had + step < need {
            issues.push(Issue {
                kind: "brief",
                layer: n.label.clone(),
                other: None,
                from: r2(n.start),
                to: r2(n.end),
                message: format!(
                    "{} is readable for only {had:.2} s ({words} word{}: give it at least {need:.1} s)",
                    describe(n),
                    if words == 1 { "" } else { "s" }
                ),
            });
        }
    }
    issues.sort_by(|a, b| a.from.total_cmp(&b.from).then(a.kind.cmp(b.kind)));
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Scene;

    fn issues(layers: serde_json::Value) -> Vec<Issue> {
        let scene = serde_json::json!({ "width": 1920, "height": 1080, "fps": 30, "duration": 4, "layers": layers });
        let (s, _) = Scene::from_value(scene, std::env::temp_dir()).unwrap();
        lint(&Prepared::build(&s, 0.25).unwrap())
    }

    #[test]
    fn finds_text_off_the_canvas_but_not_entrances_or_masks() {
        let off = issues(serde_json::json!([{ "type": "text", "text": "Wide headline here", "x": 1900, "size": 120 }]));
        assert_eq!(off.len(), 1, "{off:?}");
        assert_eq!(off[0].kind, "offscreen");
        assert!(off[0].message.contains("right edge"), "{}", off[0].message);
        // a push-in from outside is motion, not a layout problem
        let push = issues(serde_json::json!([{ "type": "text", "text": "Hello", "size": 120, "in": "push-left 0.4" }]));
        assert!(push.is_empty(), "{push:?}");
        // a clipping group is a mask on purpose
        let masked = issues(serde_json::json!([{ "type": "group", "clip": true, "width": 600, "height": 200,
            "children": [{ "type": "text", "text": "Hello", "size": 120, "x": 2000 }] }]));
        assert!(masked.is_empty(), "{masked:?}");
    }

    #[test]
    fn finds_overlapping_and_brief_text() {
        let both = issues(serde_json::json!([
            { "type": "text", "text": "First line", "size": 100 },
            { "type": "text", "text": "Second line", "size": 100, "y": 560 },
            { "type": "text", "text": "Far away", "size": 60, "y": 100 },
        ]));
        assert_eq!(both.iter().filter(|i| i.kind == "overlap").count(), 1, "{both:?}");
        // a label on its own box over a ticker hides it on purpose; a counter has no text to quote
        let boxed = issues(serde_json::json!([
            { "type": "text", "text": "Breaking news ticker", "size": 60 },
            { "type": "text", "text": "LIVE", "size": 60, "background": { "color": "#c00" } },
            { "type": "counter", "value": 42, "size": 60, "y": 300 },
            { "type": "text", "text": "Label", "size": 60, "y": 310 },
            { "type": "text", "text": "Ticker under a card", "size": 60, "y": 800 },
            { "type": "group", "y": 800, "height": 120, "children": [
                { "type": "rect", "color": "#123" },
                { "type": "text", "text": "TOP STORIES", "size": 60 },
            ] },
        ]));
        assert_eq!(boxed.len(), 1, "{boxed:?}");
        assert!(boxed[0].message.starts_with("a counter overlaps text \"Label\""), "{}", boxed[0].message);
        let brief = issues(serde_json::json!([
            { "type": "text", "text": "one two three four five six seven eight", "duration": 0.8 },
            { "type": "text", "text": "Go", "y": 200, "start": 1, "duration": 1 },
        ]));
        assert_eq!(brief.len(), 1, "{brief:?}");
        assert_eq!((brief[0].kind, brief[0].layer.as_str()), ("brief", "one two three four five six seven eight"));
    }
}
