//! Shorts ▸ Auto Edit…: makes a short from a stream clip on its own, and checks its own work.
//!
//! 1. **Looks at the clip.** Frames sampled across the recording (at keyframes, which decode
//!    quickest, when the file says where they are) show which bottom corner the
//!    streamer's model is in (the corner whose colours stay the same while the game around it
//!    changes), whether the stream burned its own captions in under the gameplay (a band of
//!    text-like edges that stays in place), and how much the gameplay changes; the transcript
//!    says how much of the clip is talk.
//! 2. **Tries every version.** Each format that fits (Gameplay + Model, Model Spotlight, Full
//!    Frame + Blur; the countdown for long clips with enough loud moments), with the model on the
//!    side found (both sides when unsure), with and without the burned-in captions cut off, is
//!    made for real, checked, and taken back.
//! 3. **Checks each one** (0–100): text clear of what the app covers, the model's face in view,
//!    how much of the visible screen shows the clip, how big the gameplay is, whether the format
//!    suits the clip, no clash with the stream's own captions, and how readable the captions are
//!    (how busy the picture is behind them, measured on the sampled frames as each version shows
//!    them).
//! 4. **Applies the best** as one undo step, with captions and a hook title, punch-ins on the loud
//!    moments, bleeped swears, the length trimmed and a loop ending, and reports every version's
//!    score and checks.
//!
//! The checks are measurements, not taste: they catch what is easy to get wrong (text under the
//! buttons, a covered face, captions over the stream's own captions, a tiny picture), and the
//! report says what each version got for each, so a different one can be picked.

use filmcraft_project::{ItemId, ParamValue, Project};
use filmcraft_render::{Image, SourceProvider};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{bad, bool_p, str_p};
use crate::shorts_formats::{self as fm, Format, Platform};
use crate::shorts_layout::{self as ly, Side};
use crate::{EngineError, Result, Session};

/// Frames looked at across the recording.
const SAMPLES: usize = 10;

// ---------------------------------------------------------------------------------------------
// Looking at the stream
// ---------------------------------------------------------------------------------------------

/// What Auto Edit learned about the recording.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Look {
    /// The corner the model is in, if one was found.
    pub model: Option<Side>,
    /// Whether that was clear (otherwise both sides are tried).
    pub sure: bool,
    /// The stream's own captions: `[y0, y1]`, fractions of the recording's height.
    pub burned: Option<[f64; 2]>,
    /// How much the gameplay changes, 0 (still) … 1 (a lot).
    pub motion: f64,
    /// How much of the clip is talk (or singing), 0 … 1.
    pub talk: f64,
}

/// A frame as luma and RGB (0..1), `w` x `h`.
pub(crate) struct Frame {
    w: usize,
    h: usize,
    rgb: Vec<[f32; 3]>,
}

impl Frame {
    pub(crate) fn of(img: &Image) -> Option<Frame> {
        if img.w < 8 || img.h < 8 || img.px.len() < img.w * img.h * 4 {
            return None;
        }
        let rgb = img
            .px
            .chunks_exact(4)
            .map(|p| {
                let a = p.get(3).copied().unwrap_or(1.0);
                // over black, as shown
                let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
                let c = |i: usize| {
                    let v = p.get(i).copied().unwrap_or(0.0) * a;
                    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
                };
                [c(0), c(1), c(2)]
            })
            .collect();
        Some(Frame { w: img.w, h: img.h, rgb })
    }

    fn luma(&self, x: usize, y: usize) -> f32 {
        self.rgb.get(y * self.w + x).map_or(0.0, |c| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2])
    }

    /// Pixel bounds of `r` (fractions), never empty.
    fn span(&self, r: [f64; 4]) -> (usize, usize, usize, usize) {
        let px = |f: f64, n: usize| ((f.clamp(0.0, 1.0) * n as f64).round() as usize).min(n);
        let (x0, y0) = (px(r[0], self.w), px(r[1], self.h));
        let (x1, y1) = (px(r[2], self.w).max(x0 + 1).min(self.w), px(r[3], self.h).max(y0 + 1).min(self.h));
        (x0.min(x1.saturating_sub(1)), y0.min(y1.saturating_sub(1)), x1, y1)
    }

    /// A 4 x 4 x 4 colour histogram of region `r`, summing to 1.
    fn histogram(&self, r: [f64; 4]) -> [f32; 64] {
        let (x0, y0, x1, y1) = self.span(r);
        let mut h = [0f32; 64];
        let mut n = 0f32;
        for y in y0..y1 {
            for x in x0..x1 {
                let Some(c) = self.rgb.get(y * self.w + x) else { continue };
                let b = |v: f32| ((v * 4.0) as usize).min(3);
                if let Some(slot) = h.get_mut(b(c[0]) * 16 + b(c[1]) * 4 + b(c[2])) {
                    *slot += 1.0;
                }
                n += 1.0;
            }
        }
        if n > 0.0 {
            for v in h.iter_mut() {
                *v /= n;
            }
        }
        h
    }

    /// Mean absolute luma difference from `o` over region `r`.
    pub(crate) fn change(&self, o: &Frame, r: [f64; 4]) -> f64 {
        if (self.w, self.h) != (o.w, o.h) {
            return 0.0;
        }
        let (x0, y0, x1, y1) = self.span(r);
        let (mut sum, mut n) = (0f64, 0f64);
        for y in y0..y1 {
            for x in x0..x1 {
                sum += f64::from((self.luma(x, y) - o.luma(x, y)).abs());
                n += 1.0;
            }
        }
        if n > 0.0 { sum / n } else { 0.0 }
    }

    /// For each row of region `r`: the share of its pixels where the brightness jumps sharply
    /// to the next one (text is dense with such jumps).
    fn row_edges(&self, r: [f64; 4]) -> Vec<f64> {
        let (x0, y0, x1, y1) = self.span(r);
        (y0..y1)
            .map(|y| {
                let n = x1.saturating_sub(x0).saturating_sub(1).max(1);
                let jumps = (x0..x1.saturating_sub(1)).filter(|&x| (self.luma(x + 1, y) - self.luma(x, y)).abs() > 0.18).count();
                jumps as f64 / n as f64
            })
            .collect()
    }

    /// The share of pixels of region `r` where the brightness jumps (how busy it is).
    fn busyness(&self, r: [f64; 4]) -> f64 {
        let rows = self.row_edges(r);
        if rows.is_empty() { 0.0 } else { rows.iter().sum::<f64>() / rows.len() as f64 }
    }
}

/// The footage the short is made from: (item, first and last media time used) of each clip of
/// the main picture.
fn recordings(s: &Session) -> Vec<(ItemId, Tick, Tick)> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let ids = ly::targets(s, &json!({}));
    q.video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|i| ids.contains(&i.id) && i.frame_hold.is_none())
        .map(|i| (i.item, i.source_in, i.source_out()))
        .collect()
}

/// Sample [`SAMPLES`] frames across the recording, at about 480 pixels wide: each at the keyframe
/// before its place when the file says where that is (one picture to decode instead of a run).
fn sample_frames(s: &Session) -> Vec<Frame> {
    let clips = recordings(s);
    let total: i64 = clips.iter().map(|(_, a, b)| b.0.saturating_sub(a.0).max(0)).sum();
    if total <= 0 {
        return Vec::new();
    }
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let mut out = Vec::new();
    let mut seen: Vec<(ItemId, Tick)> = Vec::new();
    for k in 0..SAMPLES {
        // the middle of each tenth of the footage
        let mut at = (total as f64 * (k as f64 + 0.5) / SAMPLES as f64) as i64;
        for (item, a, b) in &clips {
            let len = b.0.saturating_sub(a.0).max(0);
            if at < len {
                let t = Tick(a.0.saturating_add(at));
                let t = provider.source(*item).and_then(|src| src.keyframe_before(t)).filter(|k| *k >= *a && *k < *b).unwrap_or(t);
                if seen.contains(&(*item, t)) {
                    break;
                }
                seen.push((*item, t));
                let w = filmcraft_render::source_size(&s.project, *item).map_or(1920, |(w, _)| w).max(1);
                let scale = (480.0 / w as f32).clamp(0.05, 1.0);
                if let Some(f) = filmcraft_render::render_item(&s.project, *item, t, scale, &provider).as_ref().and_then(Frame::of) {
                    out.push(f);
                }
                break;
            }
            at -= len;
        }
    }
    out
}

/// How much region `r`'s colours change from frame to frame (0 … 2).
fn instability(frames: &[Frame], r: [f64; 4]) -> f64 {
    let hs: Vec<[f32; 64]> = frames.iter().map(|f| f.histogram(r)).collect();
    let d: Vec<f64> = hs.windows(2).map(|p| p[0].iter().zip(&p[1]).map(|(a, b)| f64::from((a - b).abs())).sum()).collect();
    if d.is_empty() { 0.0 } else { d.iter().sum::<f64>() / d.len() as f64 }
}

/// Which corner the model is in: the corner that changes least compared with the middle of the
/// stream. (side, sure), or `None` when neither corner stands out.
fn find_model(frames: &[Frame], fallback: Side) -> (Option<Side>, bool) {
    if frames.len() < 3 {
        return (Some(fallback), false);
    }
    let mid = instability(frames, [0.30, 0.0, 0.70, 1.0]);
    let right = instability(frames, ly::model_region(Side::Right));
    let left = instability(frames, ly::model_region(Side::Left));
    if mid < 0.08 {
        // the whole picture hardly changes: nothing to tell the corners apart by
        return (Some(fallback), false);
    }
    let (r, l) = (right / mid, left / mid);
    if r < 0.6 && r < l * 0.75 {
        (Some(Side::Right), true)
    } else if l < 0.6 && l < r * 0.75 {
        (Some(Side::Left), true)
    } else if r.min(l) < 0.85 {
        (Some(if r <= l { Side::Right } else { Side::Left }), false)
    } else {
        (None, true)
    }
}

/// The stream's own captions in the bottom of the gameplay `game` (fractions of the recording):
/// rows much denser with sharp edges than the picture's usual, in the same place in at least a
/// third of the frames, forming a band 2–15 % of the height. `[y0, y1]` of that band.
fn find_burned(frames: &[Frame], game: [f64; 4]) -> Option<[f64; 2]> {
    let first = frames.first()?;
    // the middle of the gameplay's width, where captions sit
    let w = game[2] - game[0];
    let band = [game[0] + w * 0.1, game[1], game[2] - w * 0.1, game[3]];
    let (_, y0, _, y1) = first.span(band);
    let rows = y1.saturating_sub(y0);
    if rows < 8 {
        return None;
    }
    let mut present = vec![0usize; rows];
    for f in frames {
        let e = f.row_edges(band);
        let mut sorted = e.clone();
        sorted.sort_by(f64::total_cmp);
        let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
        let bar = (median * 3.0).max(0.08);
        for (i, v) in e.iter().enumerate() {
            // only the lower part of the gameplay
            let y = (y0 + i) as f64 / f.h as f64;
            if *v > bar
                && y > 0.6
                && let Some(p) = present.get_mut(i)
            {
                *p += 1;
            }
        }
    }
    let need = (frames.len() as f64 / 3.0).ceil() as usize;
    let on: Vec<bool> = present.iter().map(|n| *n >= need.max(1)).collect();
    // the longest run of rows that are on (a gap of one row is bridged)
    let mut best: Option<(usize, usize)> = None;
    let mut i = 0;
    while i < on.len() {
        if !on.get(i).copied().unwrap_or(false) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut j = i + 1;
        while j < on.len() {
            if on.get(j).copied().unwrap_or(false) {
                end = j;
            } else if !on.get(j + 1).copied().unwrap_or(false) {
                break;
            }
            j += 1;
        }
        if best.is_none_or(|(a, b)| end - start > b - a) {
            best = Some((start, end));
        }
        i = end + 1;
    }
    let (a, b) = best?;
    let h = first.h as f64;
    let (ya, yb) = ((y0 + a) as f64 / h, (y0 + b + 1) as f64 / h);
    (yb - ya >= 0.02 && yb - ya <= 0.15).then_some([ya, yb])
}

/// How much of the sequence has words spoken (or sung) over it.
fn talk_share(s: &Session) -> f64 {
    let words = crate::transcript::sequence_words(s);
    let dur = s.active_sequence().map(|q| q.duration().seconds()).unwrap_or(0.0);
    if dur <= 0.0 || words.is_empty() {
        return 0.0;
    }
    // merged spans of speech, with the short pauses between words counted as talk
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for w in &words {
        let (a, b) = (w.start.seconds(), w.end.seconds().max(w.start.seconds()) + 0.3);
        match spans.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => spans.push((a, b)),
        }
    }
    (spans.iter().map(|(a, b)| b - a).sum::<f64>() / dur).clamp(0.0, 1.0)
}

/// What Auto Edit learned about the recording, and the frames it looked at.
pub(crate) fn look(s: &Session) -> (Look, Vec<Frame>) {
    let fallback = match s.state.shorts_layout.as_ref().and_then(|v| v.get("side")).and_then(Value::as_str) {
        Some("left") => Side::Left,
        _ => Side::Right,
    };
    let frames = sample_frames(s);
    let (model, sure) = find_model(&frames, fallback);
    let game = match model {
        Some(side) => ly::game_region(side),
        None => [0.0, 0.0, 1.0, 1.0],
    };
    let burned = find_burned(&frames, game);
    let changes: Vec<f64> = frames.windows(2).map(|p| p[1].change(&p[0], game)).collect();
    let change = if changes.is_empty() { 0.0 } else { changes.iter().sum::<f64>() / changes.len() as f64 };
    (Look { model, sure, burned, motion: ((change - 0.03) / 0.12).clamp(0.0, 1.0), talk: talk_share(s) }, frames)
}

// ---------------------------------------------------------------------------------------------
// Versions and their checks
// ---------------------------------------------------------------------------------------------

/// One version Auto Edit tries.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Version {
    pub format: Format,
    pub side: Side,
    /// The stream's own captions cut off the gameplay.
    pub crop: bool,
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Right => "right",
        Side::Left => "left",
    }
}

impl Version {
    fn to_json(&self) -> Value {
        json!({"format": self.format.id(), "side": side_name(self.side), "crop": self.crop})
    }
}

/// One check of a version: points of `max`, and what was found.
#[derive(Clone, Debug)]
struct Check {
    id: &'static str,
    /// Which of the check's findings this is (for the interface's wording).
    kind: &'static str,
    points: f64,
    max: f64,
    /// The measurement the note is about (a share 0..1, a size, seconds…).
    value: f64,
    note: String,
}

impl Check {
    fn to_json(&self) -> Value {
        json!({"id": self.id, "kind": self.kind, "points": (self.points * 10.0).round() / 10.0, "max": self.max, "value": (self.value * 1000.0).round() / 1000.0, "ok": self.points >= self.max * 0.7, "note": self.note})
    }
}

fn area(r: [f64; 4]) -> f64 {
    (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0)
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> f64 {
    area([a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])])
}

/// Share of `r` under the app's zones.
fn covered(pf: &Platform, r: [f64; 4]) -> f64 {
    let a = area(r);
    if a <= 0.0 {
        return 0.0;
    }
    (pf.zones.iter().map(|z| overlap(r, z.rect)).sum::<f64>() / a).clamp(0.0, 1.0)
}

/// Frame rectangles (fractions) of the text on the version's graphics tracks: every hook, title,
/// bar and badge, and up to 40 captions spread over the short.
fn text_rects(s: &Session) -> Vec<(String, [f64; 4])> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let (w, h) = (q.settings.width, q.settings.height);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let mut out = Vec::new();
    for t in &q.video_tracks {
        let what = match t.name.as_str() {
            crate::shorts_captions::TRACK => "captions",
            "Hook Title" => "hook title",
            "Title" => "title",
            "Meme Bar" => "bar text",
            "Countdown" => "countdown numbers",
            _ => continue,
        };
        let step = (t.items.len() / 40).max(1);
        for it in t.items.iter().step_by(step) {
            let Some(layer) = it.effects.iter().rev().find(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER) else { continue };
            let Some((b, _)) = crate::shorts_captions::measure(layer, (w, h), None) else { continue };
            let v2 = |k: &str| match layer.params.get(k).map(|p| &p.value) {
                Some(ParamValue::Vec2(v)) if v.x.is_finite() && v.y.is_finite() => Some(*v),
                _ => None,
            };
            let (dx, dy) = match (v2("position"), v2("anchor")) {
                (Some(p), Some(a)) => (p.x - a.x, p.y - a.y),
                _ => (0.0, 0.0),
            };
            let r = [(f64::from(b[0]) + dx) / fw, (f64::from(b[1]) + dy) / fh, (f64::from(b[2]) + dx) / fw, (f64::from(b[3]) + dy) / fh];
            if area(r) > 0.0 {
                out.push((what.to_string(), r));
            }
        }
    }
    out
}

/// How busy the picture is behind `cap` (fractions of the frame), measured on the sampled frames
/// of the recording as the version shows them: through the gameplay pane and the model box (the
/// blurred background behind them is calm).
fn busy_behind(frames: &[Frame], cap: [f64; 4], geo: &ly::Geometry) -> f64 {
    let total = area(cap);
    if total <= 0.0 || frames.is_empty() {
        return 0.0;
    }
    let mut parts: Vec<([f64; 4], [f64; 4])> = vec![(geo.pane, geo.window)];
    if let Some(m) = geo.model {
        parts.push(m);
    }
    let mut busy = 0.0;
    for (on, src) in parts {
        let i = [cap[0].max(on[0]), cap[1].max(on[1]), cap[2].min(on[2]), cap[3].min(on[3])];
        let a = area(i);
        if a <= 0.0 || on[2] <= on[0] || on[3] <= on[1] {
            continue;
        }
        let map = |fx: f64, fy: f64| (src[0] + (fx - on[0]) / (on[2] - on[0]) * (src[2] - src[0]), src[1] + (fy - on[1]) / (on[3] - on[1]) * (src[3] - src[1]));
        let (x0, y0) = map(i[0], i[1]);
        let (x1, y1) = map(i[2], i[3]);
        let b = frames.iter().map(|f| f.busyness([x0, y0, x1, y1])).sum::<f64>() / frames.len() as f64;
        busy += b * a;
    }
    (busy / total).clamp(0.0, 1.0)
}

/// Score the version now in the sequence.
fn check(s: &Session, v: &Version, pf: &Platform, look: &Look, frames: &[Frame], captions: bool) -> Vec<Check> {
    let mut out = Vec::new();
    let rects = ly::current_rects(s);
    let (pane, model) = rects.map_or(([0.0; 4], None), |(p, m)| (p, m));
    // 1. text clear of what the app covers
    let texts = text_rects(s);
    let total: f64 = texts.iter().map(|(_, r)| area(*r)).sum();
    let under: f64 = texts.iter().map(|(_, r)| covered(pf, *r) * area(*r)).sum();
    let share = if total > 0.0 { under / total } else { 0.0 };
    let worst = texts.iter().max_by(|a, b| covered(pf, a.1).total_cmp(&covered(pf, b.1))).filter(|(_, r)| covered(pf, *r) > 0.0).map(|(n, _)| n.clone());
    out.push(Check {
        id: "textClear",
        kind: if worst.is_some() { "under" } else { "clear" },
        points: 25.0 * (1.0 - share * 5.0).clamp(0.0, 1.0),
        max: 25.0,
        value: share,
        note: match worst {
            Some(n) => format!("{:.0} % of the text is under the app's buttons or bars (the {n})", share * 100.0),
            None => "all text is clear of the app's buttons and bars".into(),
        },
    });
    // 2. the model's face (the top 60 % of its box) in view
    let (kind, points, value, note) = match (look.model, model) {
        (Some(_), Some(b)) => {
            let head = [b[0], b[1], b[2], b[1] + (b[3] - b[1]) * 0.6];
            let seen = 1.0 - covered(pf, head);
            ("face", 20.0 * ((seen - 0.5) / 0.45).clamp(0.0, 1.0), seen, format!("{:.0} % of the model's face area is in view", seen * 100.0))
        }
        (Some(_), None) => ("small", 8.0, 0.0, "the model is only as big as it is in the stream".into()),
        (None, Some(_)) => ("missing", 0.0, 0.0, "no model was found in the stream, so the model box may show gameplay".into()),
        (None, None) => ("none", 20.0, 1.0, "no model in the stream: the whole picture is shown".into()),
    };
    out.push(Check { id: "modelInView", kind, points, max: 20.0, value, note });
    // 3. how much of the visible screen shows the clip
    let visible = 1.0 - pf.zones.iter().map(|z| area(z.rect)).sum::<f64>();
    let mut content = area(pane) - pf.zones.iter().map(|z| overlap(pane, z.rect)).sum::<f64>();
    if let Some(b) = model {
        content += area(b) - pf.zones.iter().map(|z| overlap(b, z.rect)).sum::<f64>() - overlap(pane, b);
    }
    let used = if visible > 0.0 { (content / visible).clamp(0.0, 1.0) } else { 0.0 };
    out.push(Check {
        id: "screenUsed",
        kind: "share",
        points: 15.0 * (used / 0.75).clamp(0.0, 1.0),
        max: 15.0,
        value: used,
        note: format!("the clip fills {:.0} % of the screen the app leaves", used * 100.0),
    });
    // 4. how big the gameplay is (its pixels on screen per pixel of the stream)
    let q = s.active_sequence();
    let fw = q.map_or(1080.0, |q| f64::from(q.settings.width));
    let src_w = ly::source_size(s).map_or(1920.0, |(w, _)| w);
    let game_w = match v.format {
        Format::FullFrame | Format::Meme => 1.0,
        _ => {
            let g = ly::game_region(v.side);
            g[2] - g[0]
        }
    };
    let size = ((pane[2] - pane[0]) * fw / (game_w * src_w).max(1.0)).max(0.0);
    out.push(Check {
        id: "gameplaySize",
        kind: "share",
        points: 10.0 * (size / 0.8).clamp(0.0, 1.0),
        max: 10.0,
        value: size,
        note: format!("the gameplay is shown at {:.0} % of its size in the stream", size * 100.0),
    });
    // 5. the format suits the clip: lots of game action wants big gameplay, a quiet game under a
    //    lot of talk (or singing) wants the model big
    let calm_talk = look.talk * (1.0 - look.motion);
    let (kind, fit, why) = match v.format {
        Format::Spotlight if look.model.is_some() => ("spotlight", 4.0 + 6.0 * (calm_talk / 0.5).clamp(0.0, 1.0), "talk over a quiet game suits the spotlight"),
        Format::Spotlight => ("noModel", 0.0, "the spotlight needs a model"),
        Format::Gameplay | Format::Countdown => ("gameplay", 6.0 + 4.0 * look.motion, "game action suits big gameplay"),
        Format::FullFrame | Format::Meme => ("full", 5.0 + 3.0 * look.motion, "the whole picture, for clips where everything matters"),
        Format::Reaction if look.model.is_some() => {
            ("reaction", (6.0 + 2.5 * look.talk + 2.0 * look.motion).min(10.0), "a streamer talking over the game suits the reaction split")
        }
        Format::Reaction => ("splitNoModel", 2.0, "the reaction split needs a model"),
    };
    out.push(Check {
        id: "fitsClip",
        kind,
        points: fit,
        max: 10.0,
        value: look.motion,
        note: format!("{why} (game action {:.0} %, talk {:.0} %)", look.motion * 100.0, look.talk * 100.0),
    });
    // 6. no clash with the stream's own captions
    let clash = captions && look.burned.is_some() && !v.crop;
    out.push(Check {
        id: "captionClash",
        kind: match (look.burned, clash) {
            (None, _) => "none",
            (Some(_), true) => "clash",
            (Some(_), false) => "cut",
        },
        points: if clash { 0.0 } else { 10.0 },
        max: 10.0,
        value: if clash { 1.0 } else { 0.0 },
        note: match (look.burned, clash) {
            (None, _) => "the stream has no captions of its own".into(),
            (Some(_), true) => "the stream's own captions show under the new ones".into(),
            (Some(_), false) => "the stream's own captions are cut off".into(),
        },
    });
    // 7. how busy the picture is behind the captions
    let cap = s.state.shorts_captions.as_ref().filter(|_| captions);
    let readable = cap.map(|c| {
        let (x, y, w) = (c["x"].as_f64().unwrap_or(0.5), c["y"].as_f64().unwrap_or(0.7), c["width"].as_f64().unwrap_or(0.9));
        let r = [x - w / 2.0, y - 0.035, x + w / 2.0, y + 0.035];
        ly::current_geometry(s).map_or(0.0, |g| busy_behind(frames, r, &g))
    });
    out.push(match readable {
        Some(busy) => Check {
            id: "readable",
            kind: "busy",
            points: 10.0 * (1.0 - ((busy - 0.04) / 0.2).clamp(0.0, 1.0)),
            max: 10.0,
            value: busy,
            note: format!("the picture behind the captions is {:.0} % busy", busy * 100.0),
        },
        None => Check { id: "readable", kind: "none", points: 10.0, max: 10.0, value: 0.0, note: "no captions".into() },
    });
    out
}

fn total(checks: &[Check]) -> f64 {
    checks.iter().map(|c| c.points).sum::<f64>().clamp(0.0, 100.0)
}

// ---------------------------------------------------------------------------------------------
// Auto Edit
// ---------------------------------------------------------------------------------------------

/// What a trial changed, put back exactly (the project, the editor state, the undo history, and
/// the events and journal entries it added).
struct Saved {
    project: std::sync::Arc<Project>,
    state: crate::EditorState,
    history: crate::History,
    events: usize,
    journal: usize,
}

impl Saved {
    fn take(s: &Session) -> Saved {
        Saved { project: s.project.clone(), state: s.state.clone(), history: s.history.clone(), events: s.events.len(), journal: s.journal.len() }
    }
    fn restore(&self, s: &mut Session) {
        s.project = self.project.clone();
        s.state = self.state.clone();
        s.history = self.history.clone();
        s.events.truncate(self.events);
        s.journal.truncate(self.journal);
        s.bump();
    }
}

/// The length to trim to: `length` is `auto`, `keep` or seconds.
fn target_seconds(p: &Value, pf: &Platform, dur: f64, cmd: &str) -> Result<f64> {
    match p.get("length") {
        None | Some(Value::Null) => Ok(auto_length(pf, dur)),
        Some(Value::String(v)) if v == "auto" => Ok(auto_length(pf, dur)),
        Some(Value::String(v)) if v == "keep" => Ok(0.0),
        Some(v) => match v.as_f64() {
            Some(x) if x == 0.0 || (3.0..=600.0).contains(&x) => Ok(x),
            _ => Err(bad(cmd, "`length` must be \"auto\", \"keep\" or 3 to 600 seconds")),
        },
    }
}

/// Auto length: under a minute on YouTube Shorts (Shorts over a minute with a copyright claim are
/// blocked), a minute on TikTok; shorter clips keep their length.
fn auto_length(pf: &Platform, dur: f64) -> f64 {
    let cap = if pf.claimed_music_seconds.is_some() { 55.0 } else { 60.0 };
    if dur > cap + 4.0 { cap } else { 0.0 }
}

/// The options the Auto Edit… window opens with: the last ones used, else every step on, for the
/// platform last used.
pub fn current(s: &Session) -> Value {
    if let Some(o) = s.state.shorts_auto.as_ref().and_then(|r| r.get("options")) {
        return o.clone();
    }
    let platform = s.state.shorts_format.as_ref().and_then(|f| f.get("platform")).and_then(Value::as_str).unwrap_or("youtube");
    json!({"platform": platform, "style": "reaction", "length": "auto", "title": "", "tryFormats": true, "captions": true, "hype": true, "finish": true, "memes": true, "memesFolder": ""})
}

pub(crate) fn auto_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.autoEdit";
    let pf = match str_p(p, "platform") {
        Some(id) => fm::platform(id).ok_or_else(|| bad(CMD, format!("unknown platform `{id}` (youtube, tiktok)")))?,
        None => fm::YOUTUBE,
    };
    let try_formats = bool_p(p, "tryFormats").unwrap_or(true);
    // Reaction Split (the default), or the best of the other formats
    let reaction = match str_p(p, "style") {
        None | Some("reaction") => true,
        Some("formats") => false,
        Some(o) => return Err(bad(CMD, format!("`style` must be reaction or formats, not `{o}`"))),
    };
    let memes = bool_p(p, "memes").unwrap_or(true);
    let memes_folder = match p.get("memesFolder") {
        None | Some(Value::Null) => None,
        Some(Value::String(f)) if f.trim().is_empty() => None,
        Some(Value::String(f)) if f.chars().count() <= 4096 => Some(f.trim().to_string()),
        Some(_) => return Err(bad(CMD, "`memesFolder` must be a folder path")),
    };
    let want_captions = bool_p(p, "captions").unwrap_or(true);
    let hype = bool_p(p, "hype").unwrap_or(true);
    let finish = bool_p(p, "finish").unwrap_or(true);
    let title = match str_p(p, "title").map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) if t.chars().count() > 200 => return Err(bad(CMD, "keep the title under 200 characters")),
        t => t.map(str::to_string),
    };
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if crate::shorts::base_track(&s.project, q).is_none() {
        return Err(bad(CMD, "the sequence has no video footage to make a short from"));
    }
    let dur = q.duration().seconds();
    let seconds = if finish { target_seconds(p, &pf, dur, CMD)? } else { 0.0 };
    let has_words = !crate::transcript::sequence_words(s).is_empty();
    let captions = want_captions && has_words;
    let pick = p.get("pick").filter(|v| v.is_object());
    let prev_game: Option<[f64; 4]> =
        s.state.shorts_layout.as_ref().and_then(|l| l.get("game")).and_then(|g| serde_json::from_value::<[f64; 4]>(g.clone()).ok());

    // 1. the clip
    let (look, frames) = look(s);
    // 2. the versions to try
    let mut versions: Vec<Version> = Vec::new();
    // the side found (when unsure, the more likely one: the other scores the same, the checks
    // cannot see a model; Layout… moves it)
    let sides: Vec<Side> = vec![look.model.unwrap_or(Side::Right)];
    let mut formats = vec![Format::Gameplay];
    if try_formats {
        formats.push(Format::Spotlight);
        formats.push(Format::FullFrame);
    }
    if reaction {
        formats = vec![Format::Reaction];
    }
    let crops: Vec<bool> = if look.burned.is_some() { vec![false, true] } else { vec![false] };
    for f in &formats {
        for side in &sides {
            for crop in &crops {
                versions.push(Version { format: *f, side: *side, crop: *crop });
            }
        }
    }
    if let Some(pk) = pick {
        let format = pk.get("format").and_then(Value::as_str).and_then(Format::parse).ok_or_else(|| bad(CMD, "`pick` needs a format"))?;
        let side = if pk.get("side").and_then(Value::as_str) == Some("left") { Side::Left } else { Side::Right };
        let crop = pk.get("crop").and_then(Value::as_bool).unwrap_or(false) && look.burned.is_some();
        versions = vec![Version { format, side, crop }];
    }
    let params_of = |v: &Version, full: bool| -> Value {
        let mut q = json!({
            "platform": pf.id,
            "format": v.format.id(),
            "side": side_name(v.side),
            "captions": captions,
            "hook": want_captions && !matches!(v.format, Format::FullFrame | Format::Reaction),
            "progressBar": full && v.format != Format::Reaction,
            "seconds": if full { seconds } else { 0.0 },
            "loop": full && finish && v.format != Format::Reaction,
            "bleep": full && finish,
            "hype": full && hype,
        });
        if v.format == Format::Reaction {
            // the trials check the layout and captions; the scenes, hits, memes and ending come
            // with the real one
            q["scenes"] = json!(full);
            q["hits"] = json!(full && hype);
            q["ending"] = json!(full && finish);
            q["memes"] = json!(full && memes);
            if let Some(f) = &memes_folder {
                q["memesFolder"] = json!(f);
            }
        }
        if let Some(t) = &title {
            q["title"] = json!(t);
        }
        if v.crop
            && let Some(b) = look.burned
        {
            let mut g = match v.format {
                Format::FullFrame | Format::Meme => [0.0, 0.0, 1.0, 1.0],
                _ => prev_game.unwrap_or_else(|| ly::game_region(v.side)),
            };
            // the gameplay ends just above the stream's captions (and keeps at least half the height)
            g[3] = (b[0] - 0.02).max(g[1] + 0.5).min(g[3]);
            q["game"] = json!(g);
        }
        q
    };
    // 3. try each, check it, take it back
    let saved = Saved::take(s);
    let mut tried: Vec<(Version, f64, Vec<Check>, Option<String>)> = Vec::new();
    for v in &versions {
        let r = fm::format(s, &params_of(v, false), Some((pf, v.format)), CMD);
        match r {
            Ok(_) => {
                let checks = check(s, v, &pf, &look, &frames, captions);
                let score = total(&checks);
                tried.push((v.clone(), score, checks, None));
            }
            Err(e) => tried.push((v.clone(), 0.0, Vec::new(), Some(e.to_string()))),
        }
        saved.restore(s);
    }
    // the best (ties go to the earlier, simpler version)
    let best = tried
        .iter()
        .enumerate()
        .filter(|(_, t)| t.3.is_none())
        .max_by(|a, b| a.1.1.total_cmp(&b.1.1).then(b.0.cmp(&a.0)))
        .map(|(i, _)| i)
        .ok_or_else(|| bad(CMD, tried.first().and_then(|t| t.3.clone()).unwrap_or_else(|| "no version could be made".into())))?;
    let chosen = tried.get(best).map(|t| t.0.clone()).ok_or_else(|| bad(CMD, "no version could be made"))?;
    // 4. the best one, for real, with everything
    let made = s.edit_group("Auto Edit", |s| fm::format(s, &params_of(&chosen, true), Some((pf, chosen.format)), CMD))?;
    let mut steps: Vec<Value> = Vec::new();
    if let Some(m) = made.get("moments").and_then(Value::as_array).filter(|m| !m.is_empty()) {
        steps.push(json!({"id": "countdown", "count": m.len()}));
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let len = q.duration().seconds();
    if seconds > 0.0 && len < dur - 0.5 {
        steps.push(json!({"id": "trimmed", "seconds": len}));
    }
    let bleeps = q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.name.starts_with("Bleep")).count();
    if finish && bleeps > 0 {
        steps.push(json!({"id": "bleeped", "count": bleeps}));
    }
    let hype_marks = q.markers.iter().filter(|m| m.name == "Hype").count();
    if hype && hype_marks > 0 {
        steps.push(json!({"id": "punchIns", "count": hype_marks}));
    }
    if captions {
        steps.push(json!({"id": "captions"}));
    }
    let made_reaction = s.state.shorts_reaction.clone().filter(|_| chosen.format == Format::Reaction);
    if let Some(r) = &made_reaction {
        let scenes = r["scenes"].as_array().cloned().unwrap_or_default();
        let count = |k: &str| scenes.iter().filter(|g| g["scene"] == k).count();
        if count("game") + count("face") > 0 {
            steps.push(json!({"id": "scenes", "game": count("game"), "face": count("face")}));
        }
        if let Some(n) = r["hits"].as_u64().filter(|n| *n > 0) {
            steps.push(json!({"id": "hits", "count": n}));
        }
        if let Some(m) = r["memes"].as_array().filter(|m| !m.is_empty()) {
            steps.push(json!({"id": "memes", "count": m.len()}));
        }
        if !r["ending"].is_null() {
            steps.push(json!({"id": "ending"}));
        }
    } else if finish {
        steps.push(json!({"id": "loop"}));
    }
    let report = json!({
        "platform": pf.id,
        "picked": best,
        "versions": tried.iter().map(|(v, score, checks, err)| {
            let mut o = v.to_json();
            o["score"] = json!(score.round());
            o["checks"] = json!(checks.iter().map(Check::to_json).collect::<Vec<_>>());
            if let Some(e) = err {
                o["error"] = json!(e);
            }
            o
        }).collect::<Vec<_>>(),
        "look": {
            "model": look.model.map(side_name),
            "sure": look.sure,
            "burnedCaptions": look.burned,
            "motion": (look.motion * 100.0).round() / 100.0,
            "talk": (look.talk * 100.0).round() / 100.0,
        },
        "steps": steps,
        "options": {
            "platform": pf.id,
            "length": p.get("length").cloned().unwrap_or(json!("auto")),
            "title": title.clone().unwrap_or_default(),
            "style": if reaction { "reaction" } else { "formats" },
            "tryFormats": try_formats,
            "captions": want_captions,
            "hype": hype,
            "finish": finish,
            "memes": memes,
            "memesFolder": memes_folder.clone().unwrap_or_default(),
        },
        "reaction": made_reaction.unwrap_or(Value::Null),
        "seconds": len,
        "warnings": pf.warnings(len),
        "sequence": made.get("sequence").cloned().unwrap_or(Value::Null),
    });
    s.state.shorts_auto = Some(report.clone());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` x `h` frame from a function of (x, y) fractions.
    fn frame(w: usize, h: usize, f: impl Fn(f64, f64) -> [f32; 3]) -> Frame {
        let rgb = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x as f64 / w as f64, y as f64 / h as f64)).collect();
        Frame { w, h, rgb }
    }

    /// A stream: gameplay whose colours change every frame, a steady pink model in a bottom
    /// corner, and (optionally) a line of white-on-black text near the bottom of the gameplay.
    fn stream(n: usize, model: Option<Side>, text: bool) -> Vec<Frame> {
        (0..n)
            .map(|k| {
                let g = (k as f32 * 0.37).fract();
                frame(160, 90, |x, y| {
                    let in_model = match model {
                        Some(Side::Right) => x >= 0.72 && y >= 0.4,
                        Some(Side::Left) => x < 0.28 && y >= 0.4,
                        None => false,
                    };
                    if in_model {
                        return [1.0, 0.6, 0.8];
                    }
                    if text && (0.86..0.92).contains(&y) && (0.15..0.55).contains(&x) {
                        // letters: alternating columns
                        return if ((x * 160.0) as usize / 2).is_multiple_of(2) { [1.0, 1.0, 1.0] } else { [0.0, 0.0, 0.0] };
                    }
                    [g, (g + 0.3 * y as f32).fract(), 1.0 - g]
                })
            })
            .collect()
    }

    #[test]
    fn finds_the_model_in_its_corner() {
        assert_eq!(find_model(&stream(8, Some(Side::Right), false), Side::Left), (Some(Side::Right), true));
        assert_eq!(find_model(&stream(8, Some(Side::Left), false), Side::Right), (Some(Side::Left), true));
        // no model: neither corner stands out
        assert_eq!(find_model(&stream(8, None, false), Side::Right).0, None);
        // too few frames, or a still picture: the fallback, unsure
        assert_eq!(find_model(&stream(2, Some(Side::Left), false), Side::Right), (Some(Side::Right), false));
        let still: Vec<Frame> = (0..6).map(|_| frame(64, 36, |_, _| [0.5, 0.5, 0.5])).collect();
        assert_eq!(find_model(&still, Side::Right), (Some(Side::Right), false));
    }

    #[test]
    fn finds_burned_in_captions_under_the_gameplay() {
        let b = find_burned(&stream(8, Some(Side::Right), true), ly::game_region(Side::Right)).expect("a band of text");
        assert!(b[0] > 0.8 && b[1] < 0.96, "{b:?}");
        assert_eq!(find_burned(&stream(8, Some(Side::Right), false), ly::game_region(Side::Right)), None);
        assert_eq!(find_burned(&[], ly::game_region(Side::Right)), None);
    }

    #[test]
    fn coverage_and_overlap() {
        assert!((overlap([0.0, 0.0, 1.0, 1.0], [0.5, 0.5, 2.0, 2.0]) - 0.25).abs() < 1e-9);
        assert_eq!(overlap([0.0, 0.0, 0.1, 0.1], [0.5, 0.5, 1.0, 1.0]), 0.0);
        let pf = fm::TIKTOK;
        assert!(covered(&pf, [0.0, 0.9, 1.0, 1.0]) > 0.99, "the bottom is the app's");
        assert_eq!(covered(&pf, [0.2, 0.3, 0.6, 0.5]), 0.0);
        assert!(auto_length(&fm::YOUTUBE, 90.0) < 60.0);
        assert_eq!(auto_length(&fm::TIKTOK, 40.0), 0.0);
    }

    #[test]
    fn hostile_frames_do_not_panic() {
        let tiny = Image { w: 2, h: 2, px: vec![f32::NAN; 16] };
        assert!(Frame::of(&tiny).is_none());
        let short = Image { w: 100, h: 100, px: vec![0.0; 10] };
        assert!(Frame::of(&short).is_none());
        let f = frame(16, 16, |_, _| [0.0, 2.0, -1.0]);
        let _ = f.histogram([0.0, 0.0, 1.0, 1.0]);
        assert!(f.busyness([-1.0, -1.0, 5.0, 5.0]).is_finite());
        let (x0, y0, x1, y1) = f.span([0.9, 0.9, 0.1, 0.1]);
        assert!(x1 > x0 && y1 > y0 && x1 <= 16 && y1 <= 16, "inverted rectangles are clamped, not panicked on");
        let nan = Image { w: 16, h: 16, px: vec![f32::NAN; 16 * 16 * 4] };
        let f = Frame::of(&nan).unwrap();
        assert!(f.rgb.iter().all(|c| c.iter().all(|v| v.is_finite())));
    }
}
