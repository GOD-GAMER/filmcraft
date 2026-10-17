//! Sequence ▸ Shorts ▸ Word Captions…: animated captions from the transcript, one graphic clip per
//! spoken word on a **Word Captions** track, every part of the look adjustable.
//!
//! - **Style** of the spoken word: `pop` (it changes colour and grows), `karaoke` (words light up
//!   as they are said), `box` (a coloured box behind it), `oneWord` (one big word at a time) or
//!   `plain` (no highlight).
//! - **Text**: font and style, size, capitals, punctuation, words and characters per line, lines,
//!   word fixes ("nero=Neuro").
//! - **Colours**: text, highlight, highlighted text (box), outline (width and colour), shadow,
//!   background box (colour, opacity).
//! - **Position** (`x`, `y`, fractions of the frame; by default just under the gameplay of a
//!   Shorts layout) and **animation** of each phrase (`pop`, `bounce`, `fade`, `slide`, `none`).
//!
//! Applying again replaces the previous Word Captions (`replace`), and `live` folds repeated
//! applications into one undo step, so the Word Captions… dialog previews as options change.

use filmcraft_edit::shorts::{self as sh, BeatRules};
use filmcraft_edit::transcript::SeqWord;
use filmcraft_geom::Vec2;
use filmcraft_project::graphic::{LayerContent, TEXT_LAYER};
use filmcraft_project::{CharStyle, EffectInstance, Keyframe, LayerExtra, Param, ParamValue, StyleRun, TrackKind};
use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{bad, bool_p, str_p};
use crate::shorts::{after, color_p, new_video_track, num_p, set_static};
use crate::{EngineError, Result, Session};

/// The track Word Captions are placed on (and replaced from).
pub(crate) const TRACK: &str = "Word Captions";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    Pop,
    Karaoke,
    Box,
    OneWord,
    Plain,
}

impl Style {
    fn name(self) -> &'static str {
        match self {
            Style::Pop => "pop",
            Style::Karaoke => "karaoke",
            Style::Box => "box",
            Style::OneWord => "oneWord",
            Style::Plain => "plain",
        }
    }
    fn parse(s: &str) -> Option<Style> {
        Some(match s {
            "pop" => Style::Pop,
            "karaoke" => Style::Karaoke,
            "box" => Style::Box,
            "oneWord" | "one" | "oneword" => Style::OneWord,
            "plain" | "none" => Style::Plain,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anim {
    Pop,
    Bounce,
    Fade,
    Slide,
    None,
}

impl Anim {
    fn name(self) -> &'static str {
        match self {
            Anim::Pop => "pop",
            Anim::Bounce => "bounce",
            Anim::Fade => "fade",
            Anim::Slide => "slide",
            Anim::None => "none",
        }
    }
    fn parse(s: &str) -> Option<Anim> {
        Some(match s {
            "pop" => Anim::Pop,
            "bounce" => Anim::Bounce,
            "fade" => Anim::Fade,
            "slide" => Anim::Slide,
            "none" => Anim::None,
            _ => return None,
        })
    }
}

/// Every option of Word Captions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Opts {
    style: Style,
    max_words: usize,
    max_chars: usize,
    lines: usize,
    font: String,
    font_style: String,
    /// Font size in pixels of a 1080-wide frame.
    size: f64,
    uppercase: bool,
    punctuation: bool,
    color: [f32; 4],
    highlight: [f32; 4],
    highlight_text: [f32; 4],
    /// How much bigger the spoken word is (percent).
    grow: f64,
    /// Outline width as a percentage of the font size (0: none).
    outline: f64,
    outline_color: [f32; 4],
    /// Shadow strength 0..100 (0: none).
    shadow: f64,
    background: bool,
    background_color: [f32; 4],
    background_opacity: f64,
    /// Corner roundness of the highlight and background boxes, percent of the font size.
    radius: f64,
    x: f64,
    y: f64,
    anim: Anim,
    hold: f64,
    speaker_colors: bool,
    /// Display fixes: (normalized wrong word, replacement).
    fixes: Vec<(String, String)>,
}

fn hex(c: [f32; 4]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Caption colours for the second, third and fourth speaker (the first uses `color`).
const SPEAKER_COLORS: [[f32; 4]; 4] = [WHITE, [0.5, 0.86, 1.0, 1.0], [1.0, 0.56, 0.85, 1.0], [0.72, 1.0, 0.48, 1.0]];

impl Opts {
    /// The style's defaults for a `frame`-sized frame; `y` is where captions go by default.
    fn defaults(style: Style, frame: (f64, f64), y: f64) -> Opts {
        let vertical = frame.1 > frame.0;
        let base = Opts {
            style,
            max_words: 3,
            max_chars: if vertical { 16 } else { 28 },
            lines: 1,
            font: "Inter".into(),
            font_style: "Bold".into(),
            size: if vertical { 92.0 } else { 81.0 },
            uppercase: true,
            punctuation: false,
            color: WHITE,
            highlight: [1.0, 0.84, 0.04, 1.0],
            highlight_text: WHITE,
            grow: 12.0,
            outline: 14.0,
            outline_color: BLACK,
            shadow: 60.0,
            background: false,
            background_color: BLACK,
            background_opacity: 55.0,
            radius: 22.0,
            x: 0.5,
            y,
            anim: Anim::Pop,
            hold: 0.25,
            speaker_colors: false,
            fixes: Vec::new(),
        };
        match style {
            Style::Pop => base,
            Style::Karaoke => Opts { max_words: 4, highlight: [0.24, 1.0, 0.43, 1.0], grow: 0.0, ..base },
            Style::Box => Opts { highlight: [0.55, 0.36, 0.96, 1.0], grow: 0.0, outline: 10.0, shadow: 40.0, ..base },
            Style::OneWord => Opts { max_words: 1, size: base.size * 1.4, anim: Anim::Bounce, grow: 0.0, ..base },
            Style::Plain => Opts { grow: 0.0, anim: Anim::Fade, ..base },
        }
    }

    pub(crate) fn from_params(p: &Value, frame: (f64, f64), default_y: f64, default_style: Style, cmd: &str) -> Result<Opts> {
        let style = match str_p(p, "style") {
            None => default_style,
            Some(s) => Style::parse(s).ok_or_else(|| bad(cmd, format!("unknown style `{s}` (pop, karaoke, box, oneWord, plain)")))?,
        };
        let mut o = Opts::defaults(style, frame, default_y);
        if style != Style::OneWord {
            o.max_words = num_p(p, "maxWords", o.max_words as f64, 1.0, 12.0, cmd)?.round() as usize;
        }
        o.max_chars = num_p(p, "maxChars", o.max_chars as f64, 1.0, 200.0, cmd)?.round() as usize;
        o.lines = num_p(p, "lines", o.lines as f64, 1.0, 3.0, cmd)?.round() as usize;
        if let Some(f) = str_p(p, "font").map(str::trim).filter(|f| !f.is_empty()) {
            o.font = f.chars().take(100).collect();
        }
        if let Some(f) = str_p(p, "fontStyle").map(str::trim).filter(|f| !f.is_empty()) {
            o.font_style = f.chars().take(100).collect();
        }
        // `size` is pixels of a 1080-wide frame
        o.size = num_p(p, "size", o.size, 4.0, 2000.0, cmd)?;
        o.uppercase = bool_p(p, "uppercase").unwrap_or(o.uppercase);
        o.punctuation = bool_p(p, "punctuation").unwrap_or(o.punctuation);
        for (k, slot) in [
            ("color", &mut o.color),
            ("highlight", &mut o.highlight),
            ("highlightText", &mut o.highlight_text),
            ("outlineColor", &mut o.outline_color),
            ("backgroundColor", &mut o.background_color),
        ] {
            if let Some(c) = color_p(p, k, cmd)? {
                *slot = c;
            }
        }
        o.grow = num_p(p, "grow", o.grow, 0.0, 100.0, cmd)?;
        o.outline = num_p(p, "outline", o.outline, 0.0, 50.0, cmd)?;
        o.shadow = num_p(p, "shadow", o.shadow, 0.0, 100.0, cmd)?;
        o.background = bool_p(p, "background").unwrap_or(o.background);
        o.background_opacity = num_p(p, "backgroundOpacity", o.background_opacity, 0.0, 100.0, cmd)?;
        o.radius = num_p(p, "radius", o.radius, 0.0, 100.0, cmd)?;
        // the old `position` parameter: [x, y] in frame pixels
        if p.get("position").is_some() {
            let v = crate::graphics::vec2_p(p, "position")
                .filter(|v| v.x.is_finite() && v.y.is_finite())
                .ok_or_else(|| bad(cmd, "`position` must be [x, y] in frame pixels"))?;
            o.x = (v.x / frame.0.max(1.0)).clamp(0.0, 1.0);
            o.y = (v.y / frame.1.max(1.0)).clamp(0.0, 1.0);
        }
        o.x = num_p(p, "x", o.x, 0.0, 1.0, cmd)?;
        o.y = num_p(p, "y", o.y, 0.0, 1.0, cmd)?;
        if let Some(a) = str_p(p, "animation") {
            o.anim = Anim::parse(a).ok_or_else(|| bad(cmd, format!("unknown animation `{a}` (pop, bounce, fade, slide, none)")))?;
        }
        o.hold = num_p(p, "hold", o.hold, 0.0, 5.0, cmd)?;
        o.speaker_colors = bool_p(p, "speakerColors").unwrap_or(o.speaker_colors);
        if let Some(f) = p.get("fixes") {
            let text = match f {
                Value::String(s) => s.clone(),
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(","),
                Value::Null => String::new(),
                _ => return Err(bad(cmd, "`fixes` must be like \"nero=Neuro, kot=cot\"")),
            };
            for pair in sh::split_phrases(&text).into_iter().take(200) {
                let (a, b) = pair.split_once('=').ok_or_else(|| bad(cmd, format!("`{pair}` in `fixes` needs an `=` (wrong=right)")))?;
                let key = sh::normalize_word(a);
                if !key.is_empty() {
                    o.fixes.push((key, b.trim().chars().take(100).collect()));
                }
            }
        }
        Ok(o)
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "style": self.style.name(),
            "maxWords": self.max_words,
            "maxChars": self.max_chars,
            "lines": self.lines,
            "font": self.font,
            "fontStyle": self.font_style,
            "size": self.size,
            "uppercase": self.uppercase,
            "punctuation": self.punctuation,
            "color": hex(self.color),
            "highlight": hex(self.highlight),
            "highlightText": hex(self.highlight_text),
            "grow": self.grow,
            "outline": self.outline,
            "outlineColor": hex(self.outline_color),
            "shadow": self.shadow,
            "background": self.background,
            "backgroundColor": hex(self.background_color),
            "backgroundOpacity": self.background_opacity,
            "radius": self.radius,
            "x": self.x,
            "y": self.y,
            "animation": self.anim.name(),
            "hold": self.hold,
            "speakerColors": self.speaker_colors,
            "fixes": self.fixes.iter().map(|(a, b)| format!("{a}={b}")).collect::<Vec<_>>().join(", "),
        })
    }
}

/// What a word shows as: fixed, without punctuation if asked, in capitals if asked.
fn display_text(w: &str, o: &Opts) -> String {
    let key = sh::normalize_word(w);
    let mut t = match o.fixes.iter().find(|(k, _)| *k == key) {
        Some((_, fix)) => fix.clone(),
        None => w.to_string(),
    };
    if !o.punctuation {
        // keep apostrophes and stars inside a word ("don't", "F***"), drop the rest at the edges
        let keep = |c: char| c.is_alphanumeric() || c == '*';
        let trimmed: String = t.trim_matches(|c: char| !keep(c)).to_string();
        if !trimmed.is_empty() {
            t = trimmed;
        }
    }
    if o.uppercase { t.to_uppercase() } else { t }
}

/// Break a phrase into at most `lines` lines of about `max_chars`: spaces become line breaks
/// (the text keeps its length, so character ranges stay put).
fn break_lines(text: &str, ranges: &[std::ops::Range<usize>], max_chars: usize, lines: usize) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    if lines <= 1 {
        return text.to_string();
    }
    let mut line_start = 0usize;
    let mut used = 1usize;
    for r in ranges.iter().skip(1) {
        if used >= lines {
            break;
        }
        if r.end.saturating_sub(line_start) > max_chars
            && let Some(sp) = r.start.checked_sub(1)
            && chars.get(sp) == Some(&' ')
        {
            if let Some(c) = chars.get_mut(sp) {
                *c = '\n';
            }
            line_start = r.start;
            used += 1;
        }
    }
    chars.into_iter().collect()
}

/// Byte offset of character `i` in `s`.
fn byte_of(s: &str, i: usize) -> usize {
    s.char_indices().nth(i).map_or(s.len(), |(b, _)| b)
}

/// A phrase entrance: keyframes for scale, opacity and position (clip-relative times).
fn entrance(anim: Anim, frame: Tick, at: Vec2, rise: f64) -> (Option<Param>, Option<Param>, Option<Param>) {
    let f = |v: f64| ParamValue::Float(v);
    let k = |n: i64, v: ParamValue| Keyframe::new(after(Tick::ZERO, n, frame), v);
    let ease = |mut kf: Keyframe| {
        kf.interp = filmcraft_project::Interpolation::EaseOut;
        kf
    };
    let param = |keys: Vec<Keyframe>| {
        let mut p = Param::new(keys.last().map_or(f(100.0), |k| k.value.clone()));
        p.keyframes = keys;
        p
    };
    match anim {
        Anim::Pop => (Some(param(vec![k(0, f(75.0)), k(2, f(112.0)), k(4, f(100.0))])), None, None),
        Anim::Bounce => (Some(param(vec![k(0, f(45.0)), k(3, f(120.0)), k(5, f(93.0)), k(7, f(100.0))])), None, None),
        Anim::Fade => (None, Some(param(vec![k(0, f(0.0)), k(4, f(100.0))])), None),
        Anim::Slide => (
            None,
            Some(param(vec![k(0, f(0.0)), k(4, f(100.0))])),
            Some(param(vec![ease(k(0, ParamValue::Vec2(Vec2::new(at.x, at.y + rise)))), k(5, ParamValue::Vec2(at))])),
        ),
        Anim::None => (None, None, None),
    }
}

/// The laid-out bounds of a text layer and, for character range `hl`, the box around it, in
/// layer pixels.
fn measure(layer: &EffectInstance, frame: (u32, u32), hl: Option<std::ops::Range<usize>>) -> Option<([f32; 4], Option<[f32; 4]>)> {
    let spec = filmcraft_project::graphic::eval_layer(layer, Tick::ZERO, frame)?;
    let LayerContent::Text(tp) = &spec.content else { return None };
    let lay = filmcraft_render::graphic_clip::text_layout(tp);
    let boxr = hl.and_then(|r| {
        let rects = lay.selection_rects(byte_of(&tp.text, r.start), byte_of(&tp.text, r.end));
        rects.into_iter().reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
    });
    Some((lay.bounds, boxr))
}

/// The y (fraction of the frame) captions go to by default: just over the bottom edge of the
/// gameplay pane of the last Shorts layout, else 70 % down a vertical frame (86 % otherwise).
fn default_y(s: &Session, frame: (f64, f64)) -> f64 {
    crate::shorts_layout::game_pane_bottom(s).map(|b| (b - 0.005).clamp(0.05, 0.95)).unwrap_or(if frame.1 > frame.0 { 0.70 } else { 0.86 })
}

/// The options the Word Captions… dialog opens with: the last ones applied, else Pop's.
pub fn current(s: &Session) -> Value {
    if let Some(v) = s.state.shorts_captions.as_ref() {
        return v.clone();
    }
    let Some(q) = s.active_sequence() else { return Value::Null };
    let frame = (f64::from(q.settings.width), f64::from(q.settings.height));
    Opts::defaults(Style::Pop, frame, default_y(s, frame)).to_json()
}

pub(crate) fn captions(s: &mut Session, p: &Value, default_style: Style, cmd: &str) -> Result<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let words = crate::transcript::sequence_words(s);
    if words.is_empty() {
        return Err(bad(cmd, "no transcript yet: run Sequence ▸ Transcript ▸ Transcribe… first so FilmCraft knows when each word is spoken"));
    }
    let o = Opts::from_params(p, (fw, fh), default_y(s, (fw, fh)), default_style, cmd)?;
    // words already bleeped show masked ("F***")
    let words = crate::shorts_fx::mask_bleeped(s, words);
    let rules = BeatRules { max_words: o.max_words, max_chars: o.max_chars.saturating_mul(o.lines).saturating_add(o.lines - 1), ..BeatRules::default() };
    let beats = sh::word_beats(&words, &rules);
    let spans = sh::caption_spans(&words, &beats, Tick::from_seconds_f64(o.hold), rate);
    if spans.is_empty() {
        return Err(bad(cmd, "the transcript's words are too short to show"));
    }
    if spans.len() > 20_000 {
        return Err(bad(cmd, "too many words for one caption track (more than 20,000); caption a shorter sequence"));
    }
    // what each word shows as
    let shown: Vec<SeqWord> = words.iter().map(|wd| SeqWord { text: display_text(&wd.text, &o), ..wd.clone() }).collect();
    let mut speakers: Vec<&str> = Vec::new();
    for wd in &words {
        if let Some(sp) = wd.speaker.as_deref()
            && !speakers.contains(&sp)
        {
            speakers.push(sp);
        }
    }
    let by_speaker = o.speaker_colors && speakers.len() > 1;
    let fill_of = |word: usize| -> [f32; 4] {
        if !by_speaker {
            return o.color;
        }
        let k = words.get(word).and_then(|wd| wd.speaker.as_deref()).and_then(|sp| speakers.iter().position(|x| *x == sp)).unwrap_or(0);
        match k % SPEAKER_COLORS.len() {
            0 => o.color,
            i => SPEAKER_COLORS.get(i).copied().unwrap_or(o.color),
        }
    };
    let px = fw / 1080.0;
    let frame_dur = rate.frame_duration().max(Tick(1));
    let at = Vec2::new(o.x * fw, o.y * fh);
    let mut clips: Vec<(Tick, Tick, String, Vec<EffectInstance>)> = Vec::with_capacity(spans.len());
    let mut prev_beat = None;
    for sp in &spans {
        let Some(beat) = beats.get(sp.beat).cloned() else { continue };
        let (text, ranges) = match o.style {
            Style::OneWord => sh::beat_text(&shown, sp.word..sp.word.saturating_add(1), false),
            _ => sh::beat_text(&shown, beat.clone(), false),
        };
        let text = break_lines(&text, &ranges, o.max_chars, o.lines);
        let k = sp.word.saturating_sub(beat.start);
        let base_color = fill_of(beat.start);
        let one_word_color = if sp.word % 3 == 2 { o.highlight } else { base_color };
        let spoken = if o.style == Style::OneWord { None } else { ranges.get(k).cloned() };
        // the styled runs: the spoken word (and for karaoke the words before it)
        let mut size = o.size * px;
        let make = |size: f64| -> EffectInstance {
            let mut layer = filmcraft_project::graphic::new_text_layer(&text, at, size);
            let fill = if o.style == Style::OneWord { one_word_color } else { base_color };
            for (key, v) in [
                ("name", ParamValue::Text(text.replace('\n', " "))),
                ("font", ParamValue::Text(o.font.clone())),
                ("font_style", ParamValue::Text(o.font_style.clone())),
                ("align", ParamValue::Choice(1)),
                ("fill", ParamValue::Bool(true)),
                ("fill_color", ParamValue::Color(fill)),
                ("stroke", ParamValue::Bool(o.outline > 0.0)),
                ("stroke_color", ParamValue::Color(o.outline_color)),
                ("stroke_width", ParamValue::Float(size * o.outline / 100.0)),
                ("stroke_type", ParamValue::Choice(0)),
                ("shadow", ParamValue::Bool(o.shadow > 0.0)),
                ("shadow_opacity", ParamValue::Float(o.shadow)),
                ("shadow_distance", ParamValue::Float(size * 0.06)),
                ("shadow_size", ParamValue::Float(size * 0.05)),
                ("shadow_blur", ParamValue::Float(size * 0.25)),
                ("background", ParamValue::Bool(o.background)),
                ("background_color", ParamValue::Color(o.background_color)),
                ("background_opacity", ParamValue::Float(o.background_opacity)),
                ("background_size", ParamValue::Float(size * 0.3)),
                ("background_radius", ParamValue::Float(size * o.radius / 100.0)),
            ] {
                set_static(&mut layer, key, v);
            }
            let grow = (o.grow > 0.0).then(|| (size * (1.0 + o.grow / 100.0)) as f32);
            let lit: Vec<std::ops::Range<usize>> = match o.style {
                Style::Pop => spoken.clone().into_iter().collect(),
                Style::Karaoke => ranges.iter().take(k.saturating_add(1)).cloned().collect(),
                Style::Box => spoken.clone().into_iter().collect(),
                Style::OneWord | Style::Plain => Vec::new(),
            };
            let runs: Vec<StyleRun> = lit
                .iter()
                .map(|r| {
                    let is_spoken = spoken.as_ref() == Some(r);
                    let fill = match o.style {
                        Style::Box => o.highlight_text,
                        _ => o.highlight,
                    };
                    StyleRun {
                        start: r.start,
                        end: r.end,
                        style: CharStyle { fill: Some(fill), size: if is_spoken { grow } else { None }, ..CharStyle::default() },
                    }
                })
                .collect();
            if !runs.is_empty() {
                layer.layer = Some(Box::new(LayerExtra { runs, ..LayerExtra::default() }));
            }
            layer
        };
        let mut layer = make(size);
        // shrink to fit 92 % of the frame width
        if let Some((b, _)) = measure(&layer, (w, h), None) {
            let width = f64::from(b[2] - b[0]);
            if width > fw * 0.92 && width > 0.0 {
                size = (size * fw * 0.92 / width).max(4.0);
                layer = make(size);
            }
        }
        let hl = if o.style == Style::Box { spoken.clone() } else { None };
        let Some((bounds, word_box)) = measure(&layer, (w, h), hl) else { continue };
        // scale and position around the middle of the text block
        let anchor = Vec2::new(f64::from(bounds[0] + bounds[2]) / 2.0, f64::from(bounds[1] + bounds[3]) / 2.0);
        set_static(&mut layer, "anchor", ParamValue::Vec2(anchor));
        let new_phrase = prev_beat != Some(sp.beat) || o.style == Style::OneWord;
        prev_beat = Some(sp.beat);
        let (scale, opacity, position) = if new_phrase { entrance(o.anim, frame_dur, at, 0.03 * fh) } else { (None, None, None) };
        for (key, prm) in [("scale", &scale), ("opacity", &opacity), ("position", &position)] {
            if let Some(prm) = prm {
                layer.params.insert(key.into(), prm.clone());
            }
        }
        let mut layers = Vec::new();
        // the highlight box goes behind the text and moves with it
        if let Some(r) = word_box {
            let (pad_x, pad_y) = (size * 0.14, size * 0.06);
            let c = Vec2::new(f64::from(r[0] + r[2]) / 2.0, f64::from(r[1] + r[3]) / 2.0);
            let bw = f64::from(r[2] - r[0]) + 2.0 * pad_x;
            let bh = f64::from(r[3] - r[1]) + 2.0 * pad_y;
            let mut boxl = filmcraft_project::graphic::new_shape_layer(0, at, Vec2::new(bw, bh), Vec::new());
            set_static(&mut boxl, "name", ParamValue::Text("Highlight".into()));
            set_static(&mut boxl, "fill", ParamValue::Bool(true));
            set_static(&mut boxl, "fill_color", ParamValue::Color(o.highlight));
            set_static(&mut boxl, "stroke", ParamValue::Bool(false));
            set_static(&mut boxl, "corner_radius", ParamValue::Float(size * o.radius / 100.0));
            set_static(&mut boxl, "anchor", ParamValue::Vec2(Vec2::new(anchor.x - c.x, anchor.y - c.y)));
            for key in ["scale", "opacity", "position"] {
                if let Some(prm) = layer.params.get(key) {
                    boxl.params.insert(key.into(), prm.clone());
                }
            }
            layers.push(boxl);
        }
        layers.push(layer);
        clips.push((sp.start, Tick(sp.end.0.saturating_sub(sp.start.0)), text.replace('\n', " "), layers));
    }
    let replace = bool_p(p, "replace").unwrap_or(true);
    let live = bool_p(p, "live").unwrap_or(false);
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let n = clips.len();
    let opts_json = o.to_json();
    let label = "Word Captions";
    let run = |pr: &mut filmcraft_project::Project, st: &mut crate::EditorState| -> Result<usize> {
        let src = crate::graphics::graphic_source(pr, w, h, rate);
        let mut items = Vec::with_capacity(clips.len());
        for (start, dur, name, layers) in clips {
            let Some(mut ti) = pr.make_track_item(src, TrackKind::Video, start, TimeRange::new(Tick::ZERO, dur), rate) else { continue };
            ti.name = name;
            for mut layer in layers {
                filmcraft_project::resolve_auto_points(&mut layer, (w, h), (w, h));
                ti.effects.push(layer);
            }
            items.push(ti);
        }
        let mut t = new_video_track(pr.alloc_id(), TRACK);
        t.items = items;
        t.sort();
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        if replace {
            // the previous Word Captions: tracks of that name holding only caption graphics
            let is_caption = |it: &filmcraft_project::TrackItem| it.effects.iter().any(|e| e.effect == TEXT_LAYER);
            let at = q.video_tracks.iter().position(|tr| tr.name == TRACK && tr.items.iter().all(is_caption));
            q.video_tracks.retain(|tr| !(tr.name == TRACK && tr.items.iter().all(is_caption)));
            match at {
                Some(i) => q.video_tracks.insert(i.min(q.video_tracks.len()), t),
                None => q.video_tracks.push(t),
            }
        } else {
            q.video_tracks.push(t);
        }
        q.check().map_err(EngineError::Other)?;
        st.selection.clear();
        st.shorts_captions = Some(opts_json.clone());
        Ok(q.video_tracks.iter().position(|tr| tr.name == TRACK).unwrap_or(0))
    };
    let track = if live { s.edit_merged(label, "shorts.captions.live", run)? } else { s.edit(label, run)? };
    Ok(json!({"clips": n, "beats": beats.len(), "track": track, "options": o.to_json()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_break_at_spaces_and_keep_length() {
        let t = "ONE TWO THREE FOUR";
        let r = vec![0..3, 4..7, 8..13, 14..18];
        assert_eq!(break_lines(t, &r, 8, 2), "ONE TWO\nTHREE FOUR");
        assert_eq!(break_lines(t, &r, 8, 1), t);
        assert_eq!(break_lines(t, &r, 4, 3).chars().count(), t.chars().count());
        assert_eq!(break_lines(t, &r, 4, 3).matches('\n').count(), 2);
    }

    #[test]
    fn display_text_fixes_punctuation_and_case() {
        let o = Opts { punctuation: false, fixes: vec![("nero".into(), "Neuro".into())], ..Opts::defaults(Style::Pop, (1080.0, 1920.0), 0.7) };
        assert_eq!(display_text("dinner?", &o), "DINNER");
        assert_eq!(display_text("don't!", &o), "DON'T");
        assert_eq!(display_text("F***!", &o), "F***");
        assert_eq!(display_text("Nero,", &o), "NEURO");
        assert_eq!(display_text("?!", &o), "?!", "nothing left: the word stays as it was");
        let keep = Opts { uppercase: false, ..o };
        assert_eq!(display_text("Hello.", &keep), "Hello");
    }

    #[test]
    fn options_round_trip_and_reject_hostile_values() {
        let frame = (1080.0, 1920.0);
        let p = json!({"style": "box", "size": 120, "color": "#ff0000", "fixes": "nero=Neuro, kot=cot", "lines": 2, "animation": "slide"});
        let o = Opts::from_params(&p, frame, 0.7, Style::Pop, "t").unwrap();
        // the dialog edits `to_json`: it reads back the same (colours as #rrggbb)
        let again = Opts::from_params(&o.to_json(), frame, 0.7, Style::Pop, "t").unwrap();
        assert_eq!(o.to_json(), again.to_json());
        assert_eq!((again.style, again.lines, again.anim, again.size), (Style::Box, 2, Anim::Slide, 120.0));
        assert_eq!(again.fixes, vec![("nero".to_string(), "Neuro".to_string()), ("kot".to_string(), "cot".to_string())]);
        for bad in [
            json!({"style": "wavy"}),
            json!({"size": 0}),
            json!({"lines": 9}),
            json!({"animation": "spin"}),
            json!({"fixes": "noequals"}),
            json!({"fixes": 3}),
            json!({"x": 2}),
            json!({"color": "blue"}),
        ] {
            assert!(Opts::from_params(&bad, frame, 0.7, Style::Pop, "t").is_err(), "{bad}");
        }
    }
}
