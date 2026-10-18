//! Short-form (vertical video) helpers, used by the engine's Shorts menu commands:
//!
//! - [`word_beats`] groups transcript words into the short phrases shown on screen together, and
//!   [`caption_spans`] gives each spoken word of a phrase its own frame-aligned display span (the
//!   span during which that word is highlighted);
//! - [`place_region`] works out the Crop and Motion that show one region of a picture inside one
//!   rectangle of the frame (a vertical crop of a widescreen recording, or one pane of a stacked
//!   layout).
//!
//! Everything here is pure: no project edits, no I/O. Every input is treated as hostile.

use std::ops::Range;

use filmcraft_time::{FrameRate, Tick};

use crate::transcript::SeqWord;

/// How words are grouped into the short phrases ("beats") shown on screen together.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeatRules {
    /// Most words on screen at once (0 counts as 1).
    pub max_words: usize,
    /// Most characters on screen at once, spaces included. A single longer word still gets a
    /// beat of its own.
    pub max_chars: usize,
    /// A silence at least this long between two words starts a new beat.
    pub break_pause: Tick,
}

impl Default for BeatRules {
    fn default() -> Self {
        Self { max_words: 3, max_chars: 16, break_pause: Tick::from_seconds_f64(0.5) }
    }
}

/// Group `words` (in time order, as [`crate::transcript::sequence_words`] returns them) into
/// beats. A beat ends when it is full (words or characters), at a pause of at least
/// `break_pause`, after a word ending a sentence (`.`, `?`, `!`) and when the speaker changes.
/// The beats cover every word, in order, and none is empty.
pub fn word_beats(words: &[SeqWord], rules: &BeatRules) -> Vec<Range<usize>> {
    let max_words = rules.max_words.max(1);
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut chars = 0usize;
    for (i, w) in words.iter().enumerate() {
        let n = w.text.chars().count();
        if i > start
            && let Some(prev) = i.checked_sub(1).and_then(|j| words.get(j))
        {
            let full = i - start >= max_words || chars.saturating_add(1).saturating_add(n) > rules.max_chars;
            let pause = w.start.0.saturating_sub(prev.end.0) >= rules.break_pause.0;
            let sentence = prev.text.trim_end_matches(['"', '\'', ')', '»', '”']).ends_with(['.', '?', '!', '。', '？', '！']);
            let speaker = prev.speaker != w.speaker;
            if full || pause || sentence || speaker {
                out.push(start..i);
                start = i;
            }
        }
        chars = if i == start { n } else { chars.saturating_add(1).saturating_add(n) };
    }
    if start < words.len() {
        out.push(start..words.len());
    }
    out
}

/// One word's turn on screen: during `start..end` (sequence time, frame-aligned) beat `beat` is
/// shown with word `word` (an index into the words) as the one being spoken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WordSpan {
    pub beat: usize,
    pub word: usize,
    pub start: Tick,
    pub end: Tick,
}

/// Display spans for every beat, in order and never overlapping, each at least one frame long.
///
/// Within a beat the spans are back to back: a word is highlighted from its start until the next
/// word starts. The beat's last word stays up until `hold` after the beat's last word ends, but
/// never past the start of the next beat. Times snap to the nearest frame; a word that would
/// start in the same frame as the previous one gets no span of its own (the earlier word stays
/// highlighted).
pub fn caption_spans(words: &[SeqWord], beats: &[Range<usize>], hold: Tick, rate: FrameRate) -> Vec<WordSpan> {
    let frame = rate.frame_duration().max(Tick(1));
    let hold = hold.max(Tick::ZERO);
    let first_start = |b: &Range<usize>| words.get(b.start).map(|w| rate.snap_nearest(w.start));
    let mut out = Vec::new();
    // the end of the previous beat: nothing may start before it
    let mut cursor: Option<Tick> = None;
    for (bi, beat) in beats.iter().enumerate() {
        // the frame each word of the beat takes over
        let mut starts: Vec<(usize, Tick)> = Vec::new();
        for i in beat.clone() {
            let Some(w) = words.get(i) else { break };
            let mut t = rate.snap_nearest(w.start);
            if let Some(c) = cursor {
                t = t.max(c);
            }
            if starts.last().is_some_and(|&(_, prev)| t <= prev) {
                continue;
            }
            starts.push((i, t));
        }
        let Some(&(_, last_start)) = starts.last() else { continue };
        let spoken_end = beat.clone().filter_map(|i| words.get(i)).map(|w| w.end).max().unwrap_or(last_start);
        let mut end = rate.snap_nearest(Tick(spoken_end.0.saturating_add(hold.0)));
        if let Some(next) = beats.get(bi.saturating_add(1)).and_then(first_start) {
            end = end.min(next);
        }
        end = end.max(Tick(last_start.0.saturating_add(frame.0)));
        for (k, &(word, start)) in starts.iter().enumerate() {
            let until = starts.get(k.saturating_add(1)).map_or(end, |&(_, t)| t);
            out.push(WordSpan { beat: bi, word, start, end: until });
        }
        cursor = Some(end);
    }
    out
}

/// The text of a beat (its words joined by single spaces) and, for each word, its character
/// range in that text (character indices, as graphic text style runs use).
pub fn beat_text(words: &[SeqWord], beat: Range<usize>, uppercase: bool) -> (String, Vec<Range<usize>>) {
    let mut text = String::new();
    let mut ranges = Vec::new();
    let mut at = 0usize;
    for i in beat {
        let Some(w) = words.get(i) else { break };
        let t = if uppercase { w.text.to_uppercase() } else { w.text.clone() };
        if !text.is_empty() {
            text.push(' ');
            at = at.saturating_add(1);
        }
        let n = t.chars().count();
        ranges.push(at..at.saturating_add(n));
        at = at.saturating_add(n);
        text.push_str(&t);
    }
    (text, ranges)
}

/// A rectangle in pixels (`x`, `y` = top-left corner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// The rectangle from fractions of a `w` x `h` area: `[x0, y0, x1, y1]`, each 0..1.
    pub fn from_fractions(f: [f64; 4], w: f64, h: f64) -> Self {
        let c = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let (x0, y0, x1, y1) = (c(f[0]), c(f[1]), c(f[2]), c(f[3]));
        Self::new(x0.min(x1) * w, y0.min(y1) * h, (x1 - x0).abs() * w, (y1 - y0).abs() * h)
    }

    fn finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.w.is_finite() && self.h.is_finite()
    }
}

/// Crop and Motion values that show part of a picture inside part of the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Motion ▸ Scale, percent.
    pub scale: f64,
    /// Motion ▸ Position (frame pixels): the centre of the pane.
    pub position: (f64, f64),
    /// Motion ▸ Anchor Point (picture pixels): the centre of what is shown.
    pub anchor: (f64, f64),
    /// Crop ▸ Left, Top, Right, Bottom (percent of the picture), so nothing outside the pane
    /// shows.
    pub crop: [f64; 4],
}

/// Show `region` of a picture `src` pixels in size inside `pane` of the frame, covering the
/// whole pane: the region is scaled just enough to fill the pane (cutting off whichever sides of
/// it don't fit), the shown part is kept inside the picture (so no empty edge ever shows), and
/// the rest is cropped away. A region smaller than one pixel (or outside the picture) stands for
/// the whole picture. `None` for an empty pane or picture, or any non-finite number.
pub fn place_region(src: (f64, f64), region: Rect, pane: Rect) -> Option<Placement> {
    let (sw, sh) = src;
    if !(sw.is_finite() && sh.is_finite() && region.finite() && pane.finite()) || sw < 1.0 || sh < 1.0 || pane.w < 1.0 || pane.h < 1.0 {
        return None;
    }
    let x0 = region.x.max(0.0).min(sw);
    let y0 = region.y.max(0.0).min(sh);
    let x1 = (region.x + region.w).max(0.0).min(sw);
    let y1 = (region.y + region.h).max(0.0).min(sh);
    let (x0, y0, x1, y1) = if x1 - x0 < 1.0 || y1 - y0 < 1.0 { (0.0, 0.0, sw, sh) } else { (x0, y0, x1, y1) };
    let (rw, rh) = (x1 - x0, y1 - y0);
    // cover the pane with the region, and never ask for more picture than there is
    let k = (pane.w / rw).max(pane.h / rh).max(pane.w / sw).max(pane.h / sh);
    let (vw, vh) = ((pane.w / k).min(sw), (pane.h / k).min(sh));
    // centred on the region, moved back inside the picture
    let cx = ((x0 + x1) / 2.0).max(vw / 2.0).min(sw - vw / 2.0);
    let cy = ((y0 + y1) / 2.0).max(vh / 2.0).min(sh - vh / 2.0);
    let pct = |v: f64, of: f64| (v / of * 100.0).clamp(0.0, 100.0);
    let crop = [pct(cx - vw / 2.0, sw), pct(cy - vh / 2.0, sh), pct(sw - (cx + vw / 2.0), sw), pct(sh - (cy + vh / 2.0), sh)];
    Some(Placement { scale: k * 100.0, position: (pane.x + pane.w / 2.0, pane.y + pane.h / 2.0), anchor: (cx, cy), crop })
}

/// Motion ▸ Scale (percent) that makes a `src` picture fill (cover) or fit inside a `frame`.
/// `None` for an empty or non-finite size.
pub fn frame_scale(src: (f64, f64), frame: (f64, f64), fill: bool) -> Option<f64> {
    let ok = |v: f64| v.is_finite() && v >= 1.0;
    if !(ok(src.0) && ok(src.1) && ok(frame.0) && ok(frame.1)) {
        return None;
    }
    let (sx, sy) = (frame.0 / src.0, frame.1 / src.1);
    Some(if fill { sx.max(sy) } else { sx.min(sy) } * 100.0)
}

/// Where the zoom of a punch-in leaves a crop: zooming by `zoom` (1.25 = 125%) around the anchor
/// `anchor_pct` (percent of the picture) keeps a cropped pane the same size on screen by moving
/// each crop edge towards the anchor. Values stay within 0..100.
pub fn zoomed_crop(crop: [f64; 4], anchor_pct: (f64, f64), zoom: f64) -> [f64; 4] {
    if !zoom.is_finite() || zoom <= 0.0 || !anchor_pct.0.is_finite() || !anchor_pct.1.is_finite() {
        return crop;
    }
    let (ax, ay) = anchor_pct;
    let c = |v: f64| if v.is_finite() { v.clamp(0.0, 100.0) } else { 0.0 };
    // left / top edges sit at `crop`; right / bottom edges sit at `100 - crop`
    let left = ax - (ax - crop[0]) / zoom;
    let top = ay - (ay - crop[1]) / zoom;
    let right = ax + ((100.0 - crop[2]) - ax) / zoom;
    let bottom = ay + ((100.0 - crop[3]) - ay) / zoom;
    [c(left), c(top), c(100.0 - right), c(100.0 - bottom)]
}

/// How [`loud_moments`] finds the loud reactions in a level curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HypeRules {
    /// How far above the surrounding level (the median of the `window` around it), in dB, a
    /// moment must be.
    pub sensitivity_db: f64,
    /// Nothing quieter than this (dBFS) counts, however much it stands out.
    pub floor_db: f64,
    /// Width of the surrounding window, in seconds.
    pub window_s: f64,
    /// Moments closer than this (seconds) are one moment; the louder one wins.
    pub min_gap_s: f64,
    /// At most this many moments, the most striking ones (0 counts as 1).
    pub max_moments: usize,
}

impl Default for HypeRules {
    fn default() -> Self {
        Self { sensitivity_db: 9.0, floor_db: -32.0, window_s: 6.0, min_gap_s: 2.0, max_moments: 8 }
    }
}

/// A loud moment: where it starts (index into the level curve) and how far above its
/// surroundings it peaks (dB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudMoment {
    pub at: usize,
    pub excess_db: f64,
}

/// The loud reactions (screams, laughs, hype) in `levels` (dBFS, one value every `hop_s`
/// seconds): runs of levels at least `sensitivity_db` above the median of the surrounding
/// window and above `floor_db`. The most striking ones are kept, at least `min_gap_s` apart, in
/// time order. Non-finite levels count as silence.
pub fn loud_moments(levels: &[f64], hop_s: f64, rules: &HypeRules) -> Vec<LoudMoment> {
    if levels.is_empty() || !(hop_s.is_finite() && hop_s > 0.0) {
        return Vec::new();
    }
    let lv: Vec<f64> = levels.iter().map(|v| if v.is_finite() { v.max(-120.0) } else { -120.0 }).collect();
    let n = lv.len();
    let span = |secs: f64| if secs.is_finite() && secs > 0.0 { ((secs / hop_s).round() as usize).min(n.max(1)) } else { 0 };
    let half = span(rules.window_s / 2.0).max(1);
    // the surrounding level: a sliding median
    let mut base = Vec::with_capacity(n);
    let mut win: Vec<f64> = Vec::with_capacity(half.saturating_mul(2).saturating_add(1));
    for i in 0..n {
        win.clear();
        win.extend(lv.iter().take(i.saturating_add(half).saturating_add(1).min(n)).skip(i.saturating_sub(half)).copied());
        win.sort_by(f64::total_cmp);
        base.push(win.get(win.len() / 2).copied().unwrap_or(-120.0));
    }
    // runs of loud hops (a gap of up to two hops does not break a run)
    let loud = |i: usize| -> Option<f64> {
        let (l, b) = (*lv.get(i)?, *base.get(i)?);
        (l >= rules.floor_db && l - b >= rules.sensitivity_db).then_some(l - b)
    };
    let mut runs: Vec<LoudMoment> = Vec::new();
    let mut quiet = usize::MAX;
    for i in 0..n {
        match loud(i) {
            Some(x) => {
                match runs.last_mut() {
                    Some(r) if quiet <= 2 => r.excess_db = r.excess_db.max(x),
                    _ => runs.push(LoudMoment { at: i, excess_db: x }),
                }
                quiet = 0;
            }
            None => quiet = quiet.saturating_add(1),
        }
    }
    // the most striking first, keeping them apart
    let gap = span(rules.min_gap_s);
    runs.sort_by(|a, b| b.excess_db.total_cmp(&a.excess_db).then(a.at.cmp(&b.at)));
    let mut out: Vec<LoudMoment> = Vec::new();
    for r in runs {
        if out.len() >= rules.max_moments.max(1) {
            break;
        }
        if out.iter().all(|o| o.at.abs_diff(r.at) >= gap) {
            out.push(r);
        }
    }
    out.sort_by_key(|m| m.at);
    out
}

/// A word as it is matched: lower case, letters, digits and apostrophes only.
pub fn normalize_word(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric() || *c == '\'' || *c == '’').flat_map(char::to_lowercase).filter(|c| *c != '\'' && *c != '’').collect()
}

/// Bleeped when the word contains one of these.
const SWEAR_STEMS: &[&str] = &["fuck", "shit", "bitch", "cunt", "motherf"];
/// Bleeped when the word is exactly one of these (so "class" and "pass" are left alone).
const SWEAR_WORDS: &[&str] = &[
    "ass",
    "asses",
    "asshole",
    "assholes",
    "dumbass",
    "jackass",
    "dick",
    "dicks",
    "dickhead",
    "pussy",
    "bastard",
    "bastards",
    "slut",
    "sluts",
    "whore",
    "whores",
    "wtf",
    "stfu",
    "goddamn",
    "goddamnit",
];

/// Whether `word` should be bleeped: it matches the built-in list of strong swear words, or one
/// of `extra` (whole words, compared after [`normalize_word`]).
pub fn is_swear(word: &str, extra: &[String]) -> bool {
    // already censored by the transcriber ("f***", "sh*t"): a letter, then a star
    let t = word.trim();
    if t.chars().next().is_some_and(char::is_alphabetic) && t.contains('*') {
        return true;
    }
    let w = normalize_word(word);
    if w.is_empty() {
        return false;
    }
    SWEAR_STEMS.iter().any(|s| w.contains(s)) || SWEAR_WORDS.contains(&w.as_str()) || extra.iter().any(|e| normalize_word(e) == w)
}

/// `word` with every letter after the first one replaced by `*` ("Fucking!" → "F******!"). The
/// number of characters stays the same, so styled runs over a caption keep their place.
pub fn mask_word(word: &str) -> String {
    let mut first = true;
    word.chars()
        .map(|c| {
            if !c.is_alphanumeric() {
                c
            } else if first {
                first = false;
                c
            } else {
                '*'
            }
        })
        .collect()
}

/// Every place a phrase from `phrases` is said: the index of its first word in `words`. A phrase
/// is one or more words, compared after [`normalize_word`] ("oh my god" matches "Oh, my God!").
/// Empty phrases are ignored. Matches are in time order and never overlap.
pub fn phrase_matches(words: &[SeqWord], phrases: &[String]) -> Vec<usize> {
    let norm: Vec<String> = words.iter().map(|w| normalize_word(&w.text)).collect();
    let pats: Vec<Vec<String>> =
        phrases.iter().map(|p| p.split_whitespace().map(normalize_word).filter(|w| !w.is_empty()).collect::<Vec<_>>()).filter(|p| !p.is_empty()).collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < norm.len() {
        let hit = pats.iter().filter(|p| norm.get(i..i.saturating_add(p.len())).is_some_and(|w| w == p.as_slice())).map(Vec::len).max();
        match hit {
            Some(len) => {
                out.push(i);
                i = i.saturating_add(len.max(1));
            }
            None => i += 1,
        }
    }
    out
}

/// The phrases of a comma- (or newline-) separated list, trimmed, empties dropped.
pub fn split_phrases(list: &str) -> Vec<String> {
    list.split([',', '\n', ';']).map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{ClipId, ItemId};

    fn w(text: &str, a: f64, b: f64) -> SeqWord {
        SeqWord {
            text: text.into(),
            start: Tick::from_seconds_f64(a),
            end: Tick::from_seconds_f64(b),
            clip: ClipId(1),
            item: ItemId(1),
            index: 0,
            track: 0,
            speaker: None,
            confidence: 1.0,
        }
    }

    fn rules(max_words: usize, max_chars: usize) -> BeatRules {
        BeatRules { max_words, max_chars, break_pause: Tick::from_seconds_f64(0.5) }
    }

    #[test]
    fn beats_break_on_count_chars_pause_sentence_and_speaker() {
        let words = vec![w("one", 0.0, 0.2), w("two", 0.2, 0.4), w("three", 0.4, 0.6), w("four", 0.6, 0.8)];
        assert_eq!(word_beats(&words, &rules(3, 100)), vec![0..3, 3..4]);
        // "one two" is 7 chars; "one two three" would be 13
        assert_eq!(word_beats(&words, &rules(10, 10)), vec![0..2, 2..4]);
        let paused = vec![w("a", 0.0, 0.2), w("b", 1.0, 1.2)];
        assert_eq!(word_beats(&paused, &rules(5, 100)), vec![0..1, 1..2]);
        let sentence = vec![w("done.", 0.0, 0.2), w("next", 0.2, 0.4)];
        assert_eq!(word_beats(&sentence, &rules(5, 100)), vec![0..1, 1..2]);
        let mut talk = vec![w("hi", 0.0, 0.2), w("yo", 0.2, 0.4)];
        talk[1].speaker = Some("B".into());
        assert_eq!(word_beats(&talk, &rules(5, 100)), vec![0..1, 1..2]);
    }

    #[test]
    fn beats_cover_every_word_and_take_hostile_rules() {
        let words: Vec<SeqWord> = (0..20).map(|i| w("supercalifragilistic", i as f64 * 0.1, i as f64 * 0.1 + 0.1)).collect();
        for r in [rules(0, 0), rules(usize::MAX, usize::MAX), rules(1, 1)] {
            let b = word_beats(&words, &r);
            assert!(b.iter().all(|x| !x.is_empty()));
            assert_eq!(b.iter().map(|x| x.len()).sum::<usize>(), words.len());
        }
        assert!(word_beats(&[], &BeatRules::default()).is_empty());
    }

    #[test]
    fn spans_are_back_to_back_frame_aligned_and_never_overlap() {
        let rate = FrameRate::FPS_30;
        let words = vec![w("we", 0.0, 0.2), w("did", 0.21, 0.4), w("it", 0.5, 0.7), w("again", 2.0, 2.4)];
        let beats = word_beats(&words, &rules(3, 100));
        assert_eq!(beats, vec![0..3, 3..4]);
        let spans = caption_spans(&words, &beats, Tick::from_seconds_f64(0.3), rate);
        assert_eq!(spans.len(), 4);
        for s in &spans {
            assert!(s.end > s.start, "{s:?}");
            assert_eq!(rate.snap(s.start), s.start);
            assert_eq!(rate.snap(s.end), s.end);
        }
        for p in spans.windows(2) {
            assert!(p[0].end <= p[1].start, "{p:?}");
        }
        // inside a beat, each word runs until the next one starts
        assert_eq!(spans[0].end, spans[1].start);
        assert_eq!(spans[1].end, spans[2].start);
        // the last word of the first beat holds 0.3 s after "it" ends (0.7 + 0.3 = 1.0 s)
        assert_eq!(spans[2].end, Tick::from_seconds_f64(1.0));
        assert_eq!(spans[3].beat, 1);
    }

    #[test]
    fn spans_skip_words_inside_one_frame_and_hold_stops_at_the_next_beat() {
        let rate = FrameRate::FPS_30;
        // "b" starts 5 ms after "a": same frame
        let words = vec![w("a", 0.0, 0.1), w("b", 0.005, 0.1), w("next", 0.2, 0.3)];
        let beats = vec![0..2, 2..3];
        let spans = caption_spans(&words, &beats, Tick::from_seconds_f64(10.0), rate);
        assert_eq!(spans.iter().map(|s| s.word).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(spans[0].end, spans[1].start, "a long hold is cut at the next beat");
        // overlapping, unsorted and tiny words still give clean spans
        let messy = vec![w("x", 1.0, 0.5), w("y", 0.9, 0.95), w("z", 0.9, 0.9)];
        let spans = caption_spans(&messy, &[0..1, 1..2, 2..3, 7..9], Tick(-5), rate);
        for p in spans.windows(2) {
            assert!(p[0].end <= p[1].start);
        }
        assert!(spans.iter().all(|s| s.end > s.start));
    }

    #[test]
    fn beat_text_ranges_are_in_characters() {
        let words = vec![w("héllo", 0.0, 0.1), w("wörld!", 0.1, 0.2)];
        let (t, r) = beat_text(&words, 0..2, true);
        assert_eq!(t, "HÉLLO WÖRLD!");
        assert_eq!(r, vec![0..5, 6..12]);
        assert_eq!(beat_text(&words, 1..9, false).1, vec![0..6]);
    }

    #[test]
    fn a_widescreen_picture_fills_a_vertical_frame() {
        let p = place_region((1920.0, 1080.0), Rect::new(0.0, 0.0, 1920.0, 1080.0), Rect::new(0.0, 0.0, 1080.0, 1920.0)).unwrap();
        assert!((p.scale - 1920.0 / 1080.0 * 100.0).abs() < 1e-9);
        assert_eq!(p.position, (540.0, 960.0));
        assert_eq!(p.anchor, (960.0, 540.0));
        // a centred 1080 x 1920 window of the 3413 x 1920 picture: the sides are cropped evenly
        assert!((p.crop[0] - p.crop[2]).abs() < 1e-9 && p.crop[0] > 30.0);
        assert!(p.crop[1].abs() < 1e-9 && p.crop[3].abs() < 1e-9);
    }

    #[test]
    fn a_corner_region_covers_its_pane_and_stays_inside_the_picture() {
        let src = (1920.0, 1080.0);
        let pane = Rect::new(0.0, 960.0, 1080.0, 960.0);
        let region = Rect::from_fractions([0.6, 0.25, 1.0, 1.0], src.0, src.1);
        let p = place_region(src, region, pane).unwrap();
        // the shown window is the pane's size at this scale, and lies inside the picture
        let k = p.scale / 100.0;
        let (vw, vh) = (pane.w / k, pane.h / k);
        assert!(p.anchor.0 + vw / 2.0 <= src.0 + 1e-6 && p.anchor.1 + vh / 2.0 <= src.1 + 1e-6);
        assert!(vw <= region.w + 1e-6 || vh <= region.h + 1e-6, "the region covers the pane");
        assert_eq!(p.position, (540.0, 1440.0));
        // crop leaves exactly the window
        let shown_w = (100.0 - p.crop[0] - p.crop[2]) / 100.0 * src.0;
        let shown_h = (100.0 - p.crop[1] - p.crop[3]) / 100.0 * src.1;
        assert!((shown_w - vw).abs() < 1e-6 && (shown_h - vh).abs() < 1e-6);
    }

    #[test]
    fn placement_rejects_hostile_sizes() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(place_region((0.0, 1080.0), r, r).is_none());
        assert!(place_region((f64::NAN, 1080.0), r, r).is_none());
        assert!(place_region((1920.0, 1080.0), Rect::new(f64::INFINITY, 0.0, 1.0, 1.0), r).is_none());
        assert!(place_region((1920.0, 1080.0), r, Rect::new(0.0, 0.0, 0.0, 10.0)).is_none());
        // a region outside the picture means the whole picture
        let p = place_region((100.0, 100.0), Rect::new(500.0, 500.0, 10.0, 10.0), Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        assert_eq!(p.anchor, (50.0, 50.0));
        assert!(Rect::from_fractions([f64::NAN, 2.0, -1.0, 0.5], 100.0, 100.0).finite());
        assert_eq!(frame_scale((1920.0, 1080.0), (1080.0, 1920.0), false).map(|v| v.round()), Some(56.0));
        assert!(frame_scale((0.0, 1.0), (1.0, 1.0), true).is_none());
    }

    #[test]
    fn zooming_a_cropped_pane_moves_the_crop_towards_the_anchor() {
        let c = zoomed_crop([20.0, 0.0, 20.0, 0.0], (50.0, 50.0), 2.0);
        assert_eq!(c, [35.0, 25.0, 35.0, 25.0]);
        assert_eq!(zoomed_crop([20.0, 0.0, 20.0, 0.0], (50.0, 50.0), 1.0), [20.0, 0.0, 20.0, 0.0]);
        assert_eq!(zoomed_crop([1.0, 2.0, 3.0, 4.0], (50.0, 50.0), f64::NAN), [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn loud_moments_stand_out_from_their_surroundings() {
        // 20 s of talking at -30 dB with a scream at 5 s and a laugh at 12 s, 50 ms hops
        let hop = 0.05;
        let mut lv = vec![-30.0; 400];
        for v in lv.iter_mut().skip(100).take(10) {
            *v = -8.0;
        }
        for v in lv.iter_mut().skip(240).take(6) {
            *v = -15.0;
        }
        let m = loud_moments(&lv, hop, &HypeRules::default());
        assert_eq!(m.iter().map(|x| x.at).collect::<Vec<_>>(), vec![100, 240]);
        assert!(m[0].excess_db > m[1].excess_db);
        // only the strongest when one is asked for
        let one = loud_moments(&lv, hop, &HypeRules { max_moments: 1, ..HypeRules::default() });
        assert_eq!(one.iter().map(|x| x.at).collect::<Vec<_>>(), vec![100]);
        // too close together: one moment
        let mut close = vec![-30.0; 400];
        close[100] = -5.0;
        close[120] = -6.0;
        assert_eq!(loud_moments(&close, hop, &HypeRules::default()).len(), 1);
        // a quiet stretch that stands out but stays under the floor is not hype
        let mut whisper = vec![-80.0; 400];
        whisper[100] = -50.0;
        assert!(loud_moments(&whisper, hop, &HypeRules::default()).is_empty());
    }

    #[test]
    fn loud_moments_take_hostile_input() {
        let rules = HypeRules { sensitivity_db: f64::NAN, floor_db: -32.0, window_s: f64::INFINITY, min_gap_s: -1.0, max_moments: 0 };
        let lv = vec![f64::NAN, f64::INFINITY, -10.0, f64::NEG_INFINITY, 0.0];
        let _ = loud_moments(&lv, 0.05, &rules);
        assert!(loud_moments(&lv, 0.0, &HypeRules::default()).is_empty());
        assert!(loud_moments(&lv, f64::NAN, &HypeRules::default()).is_empty());
        assert!(loud_moments(&[], 0.05, &HypeRules::default()).is_empty());
    }

    #[test]
    fn swears_are_matched_and_masked_without_false_alarms() {
        for w in ["Fuck", "fucking!", "MOTHERFUCKER", "shit,", "bullshit", "ass", "WTF?", "dickhead"] {
            assert!(is_swear(w, &[]), "{w}");
        }
        for w in ["class", "pass", "assume", "Dickens", "shift", "", "!!"] {
            assert!(!is_swear(w, &[]), "{w}");
        }
        assert!(is_swear("Bonk!", &["bonk".into()]));
        // censored by the transcriber
        assert!(is_swear("f***ing", &[]) && is_swear("S**t!", &[]) && !is_swear("*", &[]) && !is_swear("**", &[]));
        assert_eq!(mask_word("Fucking!"), "F******!");
        assert_eq!(mask_word("SHIT"), "S***");
        assert_eq!(mask_word("ünïcödé"), "ü******");
        assert_eq!(mask_word("Fucking!").chars().count(), "Fucking!".chars().count());
    }

    #[test]
    fn phrases_match_words_in_order() {
        let words = vec![w("Oh,", 0.0, 0.1), w("my", 0.1, 0.2), w("God!", 0.2, 0.3), w("bonk", 0.4, 0.5), w("BONK.", 0.6, 0.7), w("my", 0.8, 0.9)];
        assert_eq!(phrase_matches(&words, &["bonk".into()]), vec![3, 4]);
        assert_eq!(phrase_matches(&words, &["oh my god".into(), "my".into()]), vec![0, 5]);
        assert!(phrase_matches(&words, &["".into(), "  ".into(), "?!".into()]).is_empty());
        assert_eq!(split_phrases("bonk, what?! ,, oh my god\nnice"), vec!["bonk", "what?!", "oh my god", "nice"]);
    }
}
