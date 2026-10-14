//! Sequence ▸ Shorts, part two: automated effects for stream clips.
//!
//! - **Find Hype Moments**: the loud reactions in the sequence's mix (screams, laughs) get a
//!   punch-in, a shake and a marker.
//! - **Zoom on Every Cut**: every other clip of the main picture is zoomed in, the jump-cut look.
//! - **Effects on Words…**: a punch-in, shake, glitch or flash wherever chosen words are said.
//! - **Freeze Frame at Playhead**: a held, desaturated, slowly zooming frame with a flash and a
//!   moment of silence.
//! - **Hook Title…** and **Progress Bar**: graphics for the first seconds and the whole short.
//! - **Bleep Swear Words**: muted words with a beep over them, masked in Word Captions.
//! - **Duck Music Under Speech** and **Sound Effect at Playhead**.
//!
//! The command specs are registered by [`crate::shorts::commands`] so the menu keeps one order.
//! Every command is one undo step.

use filmcraft_edit::shorts::{self as sh, HypeRules};
use filmcraft_edit::transcript::SeqWord;
use filmcraft_geom::Vec2;
use filmcraft_media::Generator;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_project::{
    ClipId, EffectInstance, ItemId, ItemKind, Keyframe, Label, Marker, MarkerId, MarkerKind, MediaRef, Param, ParamValue, Track, TrackId, TrackItem, TrackKind,
};
use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{bad, bool_p, str_p, time_p};
use crate::shorts::{GraphicClip, Hit, after, apply_hits, color_p, flash_layer, footage_at, is_footage, num_p, place_graphic_track, put, set_static};
use crate::{EngineError, Result, Session};

/// The longest stretch of audio Find Hype Moments analyses (mark In/Out for longer sequences).
const MAX_HYPE_SECONDS: f64 = 3.0 * 3600.0;
/// Level curve resolution for Find Hype Moments.
const HOP_S: f64 = 0.05;
/// The name every bleep clip gets; Word Captions masks the words under one.
const BLEEP: &str = "Bleep";

// ---------------------------------------------------------------------------------------------
// Find Hype Moments
// ---------------------------------------------------------------------------------------------

/// Which one-shot effects a batch command adds (from `"effects": ["punch", "shake", …]`).
#[derive(Clone, Copy, Default)]
struct Picks {
    punch: bool,
    shake: bool,
    glitch: bool,
    flash: bool,
    pulse: bool,
}

fn picks_p(p: &Value, key: &str, default: &[&str], cmd: &str) -> Result<Picks> {
    let names: Vec<String> = match p.get(key) {
        None | Some(Value::Null) => default.iter().map(|s| (*s).to_string()).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => {
            a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad(cmd, format!("`{key}` must list effect names")))).collect::<Result<_>>()?
        }
        Some(_) => return Err(bad(cmd, format!("`{key}` must be an effect name or a list of them"))),
    };
    let mut k = Picks::default();
    for n in &names {
        match n.trim().to_ascii_lowercase().as_str() {
            "punch" | "punchin" | "zoom" => k.punch = true,
            "shake" => k.shake = true,
            "glitch" => k.glitch = true,
            "flash" => k.flash = true,
            "pulse" | "bump" => k.pulse = true,
            other => return Err(bad(cmd, format!("unknown effect `{other}` (punch, shake, glitch, flash, pulse)"))),
        }
    }
    if !(k.punch || k.shake || k.glitch || k.flash || k.pulse) {
        return Err(bad(cmd, format!("`{key}` names no effect")));
    }
    Ok(k)
}

fn hits_of(k: Picks, pulse: f64) -> Vec<Hit> {
    let mut v = Vec::new();
    if k.pulse {
        v.push(Hit::Pulse { amount: pulse });
    }
    if k.punch {
        v.push(Hit::Punch { zoom: 1.2, hold: Tick::from_seconds_f64(0.5) });
    }
    if k.shake {
        v.push(Hit::Shake { amount: 35.0, dur: Tick::from_seconds_f64(0.4) });
    }
    if k.glitch {
        v.push(Hit::Glitch { amount: 30.0, dur: Tick::from_seconds_f64(0.3) });
    }
    v
}

/// Apply the picked effects at `times` (sequence time) as one undo step: the hits on the
/// footage under each moment, the flashes on a new track. Returns (moments used, flashes).
fn effects_at(s: &mut Session, label: &str, times: &[Tick], k: Picks, flash_track: &str) -> Result<(usize, usize)> {
    effects_at_with(s, label, times, k, flash_track, 1.05, 90.0)
}

/// [`effects_at`] with the pulse size and the flash's peak opacity (percent).
fn effects_at_with(s: &mut Session, label: &str, times: &[Tick], k: Picks, flash_track: &str, pulse: f64, flash_peak: f64) -> Result<(usize, usize)> {
    let moments: Vec<(Tick, Vec<ClipId>)> = times.iter().map(|t| (*t, footage_at(s, *t))).filter(|(_, c)| !c.is_empty()).collect();
    let hits = hits_of(k, pulse);
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let flash_dur = rate.snap_nearest(Tick::from_seconds_f64(0.25)).max(rate.frame_duration());
    let mut flashes: Vec<GraphicClip> = Vec::new();
    if k.flash {
        for t in times {
            let t = rate.snap(*t);
            if flashes.last().is_none_or(|(prev, d, _, _)| t >= Tick(prev.0.saturating_add(d.0))) {
                flashes.push((t, flash_dur, "Flash".into(), vec![flash_layer(w, h, [1.0, 1.0, 1.0, 1.0], flash_dur, flash_peak)]));
            }
        }
    }
    let nf = flashes.len();
    let used = moments.len();
    s.edit_group(label, |s| {
        if !hits.is_empty() && !moments.is_empty() {
            apply_hits(s, label, &moments, &hits)?;
        }
        if !flashes.is_empty() {
            place_graphic_track(s, label, flash_track, flashes)?;
        }
        Ok(())
    })?;
    Ok((used, nf))
}

/// The level curve (dBFS every [`HOP_S`]) of the active sequence's mix over `from..to`.
fn mix_levels(s: &Session, from: Tick, to: Tick) -> Result<Vec<f64>> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let sr = q.settings.sample_rate.max(8000);
    let start = from.to_units_floor(i64::from(sr));
    let end = to.to_units_floor(i64::from(sr));
    let hop = ((f64::from(sr) * HOP_S).round() as usize).max(1);
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let mut levels = Vec::new();
    let (mut acc, mut n) = (0.0f64, 0usize);
    let mut pos = start;
    while pos < end {
        let len = (end - pos).min(i64::from(sr)) as usize;
        let b = filmcraft_render::audio::mix_sequence(&s.project, q, pos, len, &provider);
        for i in 0..len {
            let l = b.channels.first().and_then(|c| c.get(i)).copied().unwrap_or(0.0);
            let r = b.channels.get(1).and_then(|c| c.get(i)).copied().unwrap_or(l);
            let v = f64::from((l + r) * 0.5);
            acc += if v.is_finite() { v * v } else { 0.0 };
            n += 1;
            if n == hop {
                levels.push(10.0 * (acc / n as f64).max(1e-12).log10());
                (acc, n) = (0.0, 0);
            }
        }
        pos += len as i64;
    }
    Ok(levels)
}

pub(crate) fn hype(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.hype";
    let picks = picks_p(p, "effects", &["punch", "shake"], CMD)?;
    let rules = HypeRules {
        sensitivity_db: num_p(p, "sensitivity", 9.0, 2.0, 40.0, CMD)?,
        max_moments: num_p(p, "maxMoments", 8.0, 1.0, 200.0, CMD)?.round() as usize,
        min_gap_s: num_p(p, "minGap", 2.0, 0.3, 60.0, CMD)?,
        ..HypeRules::default()
    };
    let markers_only = bool_p(p, "markersOnly").unwrap_or(false);
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let from = q.mark_in.unwrap_or(Tick::ZERO);
    let to = q.mark_out.unwrap_or_else(|| q.duration());
    if to <= from {
        return Err(bad(CMD, "the sequence (or its In/Out range) is empty"));
    }
    if (to - from).seconds() > MAX_HYPE_SECONDS {
        return Err(bad(CMD, "that is more than 3 hours of audio: mark In and Out around the part to search"));
    }
    let levels = mix_levels(s, from, to)?;
    let found = sh::loud_moments(&levels, HOP_S, &rules);
    let times: Vec<Tick> = found.iter().map(|m| Tick(from.0.saturating_add(Tick::from_seconds_f64(m.at as f64 * HOP_S).0))).collect();
    if times.is_empty() {
        return Err(bad(CMD, "no moment stands out from the rest of the audio (try a lower sensitivity)"));
    }
    let (n, flashes) = s.edit_group("Find Hype Moments", |s| {
        let notes: Vec<(Tick, String)> = found.iter().zip(&times).map(|(m, t)| (*t, format!("+{:.0} dB over its surroundings", m.excess_db))).collect();
        add_markers(s, "Hype", Label::Mango, &notes)?;
        if markers_only { Ok((0, 0)) } else { effects_at(s, "Find Hype Moments", &times, picks, "Hype Flashes") }
    })?;
    Ok(json!({"moments": times.iter().map(|t| t.0).collect::<Vec<_>>(), "clips": n, "flashes": flashes}))
}

/// Sequence markers named `name` at `(time, comment)`, one undo step.
fn add_markers(s: &mut Session, name: &str, color: Label, at: &[(Tick, String)]) -> Result<()> {
    s.edit_sequence("Add Markers", |q, ctx, _| {
        for (t, comment) in at {
            q.markers.push(Marker {
                id: MarkerId(ctx.alloc()),
                start: *t,
                duration: Tick::ZERO,
                name: name.to_string(),
                comment: comment.clone(),
                kind: MarkerKind::Comment,
                color,
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })
}

// ---------------------------------------------------------------------------------------------
// Zoom on Every Cut
// ---------------------------------------------------------------------------------------------

pub(crate) fn zoom_cuts(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.zoomCuts";
    let zoom = num_p(p, "zoom", 112.0, 101.0, 300.0, CMD)? / 100.0;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let base = crate::shorts::base_track(&s.project, q).ok_or_else(|| bad(CMD, "the sequence has no video footage"))?;
    let mut clips: Vec<(Tick, ClipId)> =
        q.video_tracks.get(base).map(|t| t.items.iter().filter(|i| is_footage(&s.project, i)).map(|i| (i.start, i.id)).collect()).unwrap_or_default();
    clips.sort();
    if clips.len() < 2 {
        return Err(bad(CMD, "there are no cuts yet: cut the clip first (Transcript ▸ Remove Pauses makes jump cuts)"));
    }
    // every other clip, starting with the second, is the zoomed one
    let zoomed: Vec<ClipId> = clips.iter().skip(1).step_by(2).map(|(_, c)| *c).collect();
    let n = zoomed.len();
    s.edit_sequence("Zoom on Every Cut", |q, _, _| {
        for it in q.video_tracks.iter_mut().flat_map(|t| t.items.iter_mut()).filter(|i| zoomed.contains(&i.id)) {
            if let Some(scale) = it.effect_mut("motion").and_then(|m| m.params.get_mut("scale")) {
                scale.map_values(|v| match v {
                    ParamValue::Float(x) => ParamValue::Float((x * zoom).min(10_000.0)),
                    other => other,
                });
            }
            // a stacked pane zooms inside its crop
            if let Some(c) = it.effects.iter_mut().find(|e| e.effect == "crop") {
                let keys = ["left", "top", "right", "bottom"];
                if !keys.iter().any(|k| c.params.get(*k).is_some_and(Param::is_animated)) {
                    let crop = keys.map(|k| c.params.get(k).and_then(|x| x.value.as_f64()).unwrap_or(0.0));
                    if crop.iter().any(|v| *v > 0.0) {
                        crop_zoom(c, crop, zoom);
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({"zoomed": n, "clips": clips.len()}))
}

/// Move a static crop in for a zoom around the middle of what it shows (where a pane placed by
/// [`crate::shorts`] has its anchor), so the pane keeps its size on screen.
fn crop_zoom(c: &mut EffectInstance, crop: [f64; 4], zoom: f64) {
    let ax = (crop[0] + (100.0 - crop[2])) / 2.0;
    let ay = (crop[1] + (100.0 - crop[3])) / 2.0;
    let zoomed = sh::zoomed_crop(crop, (ax, ay), zoom);
    for (k, v) in ["left", "top", "right", "bottom"].into_iter().zip(zoomed) {
        set_static(c, k, ParamValue::Float(v));
    }
}

// ---------------------------------------------------------------------------------------------
// Effects on Words
// ---------------------------------------------------------------------------------------------

/// Phrases from a `"words"` parameter: a comma-separated string or a list of strings.
fn phrases_p(p: &Value, key: &str, cmd: &str) -> Result<Option<Vec<String>>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(sh::split_phrases(s))),
        Some(Value::Array(a)) => {
            let mut out = Vec::new();
            for v in a {
                let s = v.as_str().ok_or_else(|| bad(cmd, format!("`{key}` must be a list of words")))?;
                out.extend(sh::split_phrases(s));
            }
            Ok(Some(out))
        }
        Some(_) => Err(bad(cmd, format!("`{key}` must be words separated by commas"))),
    }
}

fn transcript_words(s: &Session, cmd: &str) -> Result<Vec<SeqWord>> {
    let words = crate::transcript::sequence_words(s);
    if words.is_empty() {
        return Err(bad(cmd, "no transcript yet: run Sequence ▸ Transcript ▸ Transcribe… first so FilmCraft knows when each word is spoken"));
    }
    Ok(words)
}

pub(crate) fn keyword_effects(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.keywordEffects";
    let phrases = phrases_p(p, "words", CMD)?.unwrap_or_default();
    if phrases.is_empty() {
        return Err(bad(CMD, "type the words to react to, separated by commas"));
    }
    if phrases.len() > 200 {
        return Err(bad(CMD, "at most 200 words at once"));
    }
    let picks = picks_p(p, "effect", &["punch"], CMD)?;
    let words = transcript_words(s, CMD)?;
    let hits = sh::phrase_matches(&words, &phrases);
    let mut times: Vec<Tick> = Vec::new();
    for i in hits {
        let Some(w) = words.get(i) else { continue };
        // one reaction per half second
        if times.last().is_none_or(|t| w.start.0.saturating_sub(t.0) >= Tick::from_seconds_f64(0.5).0) {
            times.push(w.start);
        }
    }
    if times.is_empty() {
        return Err(bad(CMD, format!("none of those words is in the transcript ({})", phrases.join(", "))));
    }
    times.truncate(500);
    let (n, flashes) = effects_at(s, "Effects on Words", &times, picks, "Word Flashes")?;
    Ok(json!({"matches": times.len(), "clips": n, "flashes": flashes}))
}

// ---------------------------------------------------------------------------------------------
// Freeze Frame
// ---------------------------------------------------------------------------------------------

pub(crate) fn freeze_frame(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.freezeFrame";
    let secs = num_p(p, "seconds", 1.5, 0.2, 30.0, CMD)?;
    let zoom = num_p(p, "zoom", 115.0, 100.0, 300.0, CMD)? / 100.0;
    let with_flash = bool_p(p, "flash").unwrap_or(true);
    let t = time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let rate = q.settings.frame_rate;
    let t = rate.snap(t);
    // the main picture under the playhead (the lowest footage track)
    let base = q
        .video_tracks
        .iter()
        .find_map(|tr| tr.items.iter().find(|i| i.range().contains(t) && i.start < t && is_footage(&s.project, i)))
        .map(|i| i.id)
        .ok_or_else(|| bad(CMD, "move the playhead into a video clip (not onto its first frame)"))?;
    let (w, h) = (q.settings.width, q.settings.height);
    let others: Vec<ClipId> = footage_at(s, t).into_iter().filter(|c| *c != base).collect();
    s.edit_group("Freeze Frame", |s| {
        let r = s.execute("clip.insertFrameHoldSegment", json!({"clip": base.0, "time": t.0, "seconds": secs}))?;
        let seg = ClipId(r["clip"].as_u64().ok_or_else(|| bad(CMD, "the hold was not inserted"))?);
        let dur = Tick(r["duration"].as_i64().unwrap_or(0));
        let end = Tick(t.0.saturating_add(dur.0));
        s.edit_sequence("Freeze Frame", |q, ctx, _| {
            // other footage cut at the same moment (a stacked layout's model pane) holds too
            for tr in q.video_tracks.iter_mut() {
                if tr.items.iter().any(|i| i.id == seg) {
                    continue;
                }
                let left = tr.items.iter().find(|i| i.end() == t && i.start < t).cloned();
                let right_starts = tr.items.iter().any(|i| i.start == end);
                let free = !tr.items.iter().any(|i| i.range().overlaps(&TimeRange::new(t, dur)));
                if let (Some(l), true, true) = (left.filter(|l| others.contains(&l.id)), right_starts, free) {
                    let hold = l.moving_source_time_at(Tick(t.0.saturating_sub(1)));
                    let mut twin = l.clone();
                    twin.id = ClipId(ctx.alloc());
                    twin.start = t;
                    twin.duration = dur;
                    twin.source_in = hold;
                    twin.frame_hold = Some(hold);
                    twin.link = None;
                    twin.markers.clear();
                    tr.items.push(twin);
                    tr.sort();
                }
            }
            // every held frame: black and white, slowly pushing in
            for it in q.video_tracks.iter_mut().flat_map(|tr| tr.items.iter_mut()).filter(|i| i.start == t && i.duration == dur && i.frame_hold.is_some()) {
                crate::shorts::add_effect(it, "black_white", vec![]);
                let last = Tick(it.end().0.saturating_sub(1));
                let (m0, m1) = (it.effect_time_at(t), it.effect_time_at(last));
                if let Some(scale) = it.effect_mut("motion").and_then(|m| m.params.get_mut("scale")) {
                    let b = scale.f64_at(m0);
                    put(scale, m0, b);
                    put(scale, m1, b * zoom);
                    if let Some(k) = scale.keyframes.iter_mut().find(|k| k.time == m0) {
                        k.interp = filmcraft_project::Interpolation::EaseOut;
                    }
                }
            }
            q.check().map_err(EngineError::Other)?;
            Ok(())
        })?;
        if with_flash {
            let rate_dur = rate.snap_nearest(Tick::from_seconds_f64(0.25)).max(rate.frame_duration());
            let layer = flash_layer(w, h, [1.0, 1.0, 1.0, 1.0], rate_dur, 90.0);
            crate::graphics::place_video_clip(s, CMD, "Flash", &json!({"time": t.0}), "Freeze Frame", vec![layer], move |pr, (w, h, rate)| {
                (crate::graphics::graphic_source(pr, w, h, rate), rate_dur)
            })?;
        }
        Ok(json!({"clip": seg.0, "duration": dur.0}))
    })
}

// ---------------------------------------------------------------------------------------------
// Hook Title, Progress Bar
// ---------------------------------------------------------------------------------------------

/// Approximate width of bold capitals, in ems.
const EM_PER_CHAR: f64 = 0.68;

pub(crate) fn hook_title(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.hookTitle";
    let text = str_p(p, "text").map(str::trim).filter(|t| !t.is_empty()).unwrap_or("WAIT FOR IT…").to_string();
    if text.chars().count() > 200 {
        return Err(bad(CMD, "keep the hook under 200 characters"));
    }
    let secs = num_p(p, "seconds", 2.5, 0.5, 30.0, CMD)?;
    let color = color_p(p, "color", CMD)?.unwrap_or([1.0, 0.84, 0.04, 1.0]);
    let start = time_p(s, p, "").unwrap_or(Tick::ZERO);
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let dur = rate.snap_nearest(Tick::from_seconds_f64(secs)).max(rate.frame_duration());
    let frame = rate.frame_duration().max(Tick(1));
    let chars = text.chars().count().max(1) as f64;
    let size = (fw.min(fh) * 0.11).min(fw * 0.9 / (chars * EM_PER_CHAR)).max(8.0);
    let y = if fh > fw { fh * 0.2 } else { fh * 0.16 };
    let mut layer = filmcraft_project::graphic::new_text_layer(&text, Vec2::new(fw / 2.0, y), size);
    for (k, v) in [
        ("name", ParamValue::Text("Hook".into())),
        ("font", ParamValue::Text("Inter".into())),
        ("font_style", ParamValue::Text("Bold".into())),
        ("align", ParamValue::Choice(1)),
        ("fill_color", ParamValue::Color(color)),
        ("stroke", ParamValue::Bool(true)),
        ("stroke_color", ParamValue::Color([0.0, 0.0, 0.0, 1.0])),
        ("stroke_width", ParamValue::Float(size * 0.15)),
        ("shadow", ParamValue::Bool(true)),
        ("shadow_opacity", ParamValue::Float(70.0)),
        ("shadow_distance", ParamValue::Float(size * 0.07)),
        ("shadow_blur", ParamValue::Float(size * 0.3)),
    ] {
        set_static(&mut layer, k, v);
    }
    // slams in, wobbles while it is up, fades at the end
    let key = |t: Tick, v: f64| Keyframe::new(t, ParamValue::Float(v));
    let mut scale = Param::new(ParamValue::Float(100.0));
    scale.keyframes = vec![key(Tick::ZERO, 40.0), key(after(Tick::ZERO, 3, frame), 115.0), key(after(Tick::ZERO, 6, frame), 100.0)];
    layer.params.insert("scale".into(), scale);
    let mut rot = Param::new(ParamValue::Float(-3.0));
    let half = Tick::from_seconds_f64(0.5);
    let mut tk = Tick::ZERO;
    let mut sign = -1.0;
    while tk < dur {
        rot.keyframes.push(key(tk, 3.0 * sign));
        sign = -sign;
        tk = Tick(tk.0.saturating_add(half.0));
    }
    layer.params.insert("rotation".into(), rot);
    let fade_at = Tick(dur.0.saturating_sub(Tick::from_seconds_f64(0.25).0).max(0));
    let mut fade = Param::new(ParamValue::Float(100.0));
    fade.keyframes = vec![key(fade_at, 100.0), key(dur, 0.0)];
    layer.params.insert("opacity".into(), fade);
    let track = place_graphic_track(s, "Hook Title", "Hook Title", vec![(rate.snap(start), dur, text.clone(), vec![layer])])?;
    Ok(json!({"track": track, "text": text}))
}

pub(crate) fn progress_bar(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.progressBar";
    let color = color_p(p, "color", CMD)?.unwrap_or([1.0, 0.84, 0.04, 1.0]);
    let top = match str_p(p, "position").unwrap_or("bottom") {
        "top" => true,
        "bottom" => false,
        other => return Err(bad(CMD, format!("`position` must be top or bottom, not `{other}`"))),
    };
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let thick = num_p(p, "height", (fh.max(fw) / 1920.0 * 16.0).round().max(4.0), 1.0, 400.0, CMD)?;
    let dur = rate.snap(q.duration());
    if dur < rate.frame_duration() {
        return Err(bad(CMD, "the sequence is empty"));
    }
    let y = if top { thick / 2.0 } else { fh - thick / 2.0 };
    // a faint full-width track behind the bar
    let mut back = filmcraft_project::graphic::new_shape_layer(0, Vec2::new(fw / 2.0, y), Vec2::new(fw, thick), Vec::new());
    set_static(&mut back, "name", ParamValue::Text("Track".into()));
    set_static(&mut back, "fill_color", ParamValue::Color([0.0, 0.0, 0.0, 0.35]));
    // the bar grows from the left edge: its size and centre move together
    let mut bar = filmcraft_project::graphic::new_shape_layer(0, Vec2::new(0.0, y), Vec2::new(0.0, thick), Vec::new());
    set_static(&mut bar, "name", ParamValue::Text("Progress".into()));
    set_static(&mut bar, "fill_color", ParamValue::Color(color));
    let v2 = |t: Tick, x: f64, yy: f64| Keyframe::new(t, ParamValue::Vec2(Vec2::new(x, yy)));
    let mut size = Param::new(ParamValue::Vec2(Vec2::new(0.0, thick)));
    size.keyframes = vec![v2(Tick::ZERO, 0.0, thick), v2(dur, fw, thick)];
    bar.params.insert("size".into(), size);
    let mut pos = Param::new(ParamValue::Vec2(Vec2::new(0.0, y)));
    pos.keyframes = vec![v2(Tick::ZERO, 0.0, y), v2(dur, fw / 2.0, y)];
    bar.params.insert("position".into(), pos);
    let track = place_graphic_track(s, "Progress Bar", "Progress Bar", vec![(Tick::ZERO, dur, "Progress Bar".into(), vec![back, bar])])?;
    Ok(json!({"track": track, "duration": dur.0}))
}

// ---------------------------------------------------------------------------------------------
// Bleep Swear Words
// ---------------------------------------------------------------------------------------------

/// Whether a clip is one of the beeps Bleep Swear Words made.
fn is_bleep(pr: &filmcraft_project::Project, it: &TrackItem) -> bool {
    it.name == BLEEP
        && pr.item(it.item).is_some_and(|i| matches!(&i.kind, ItemKind::Media(m) if matches!(m.media, MediaRef::Generator(Generator::Tone { .. }))))
}

/// `words` with the ones under a bleep masked ("F***"), for Word Captions.
pub(crate) fn mask_bleeped(s: &Session, mut words: Vec<SeqWord>) -> Vec<SeqWord> {
    let Some(q) = s.active_sequence() else { return words };
    let bleeps: Vec<TimeRange> = q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| is_bleep(&s.project, i)).map(TrackItem::range).collect();
    if bleeps.is_empty() {
        return words;
    }
    for w in &mut words {
        let mid = Tick(w.start.0.saturating_add(w.end.0.saturating_sub(w.start.0) / 2));
        if bleeps.iter().any(|r| r.contains(mid)) {
            w.text = sh::mask_word(&w.text);
        }
    }
    words
}

/// Merge overlapping or touching ranges (sorted by start).
fn merge(mut v: Vec<TimeRange>) -> Vec<TimeRange> {
    v.sort_by_key(|r| r.start);
    let mut out: Vec<TimeRange> = Vec::new();
    for r in v {
        match out.last_mut() {
            Some(last) if r.start <= last.end() => {
                let end = last.end().max(r.end());
                *last = TimeRange::new(last.start, Tick(end.0.saturating_sub(last.start.0)));
            }
            _ => out.push(r),
        }
    }
    out
}

pub(crate) fn bleep(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.bleep";
    let custom = phrases_p(p, "words", CMD)?;
    let extra = phrases_p(p, "extraWords", CMD)?.unwrap_or_default();
    let hz = num_p(p, "hz", 1000.0, 50.0, 8000.0, CMD)? as f32;
    let db = num_p(p, "beepDb", -14.0, -60.0, 0.0, CMD)? as f32;
    let words = transcript_words(s, CMD)?;
    // `words` replaces the built-in list; `extraWords` adds to whichever list is used
    let target = |text: &str| -> bool {
        match &custom {
            Some(list) => {
                let w = sh::normalize_word(text);
                !w.is_empty() && list.iter().chain(&extra).any(|x| sh::normalize_word(x) == w)
            }
            None => sh::is_swear(text, &extra),
        }
    };
    let hit = |w: &SeqWord| target(&w.text);
    let pad = Tick::from_seconds_f64(0.03);
    let found: Vec<&SeqWord> = words.iter().filter(|w| hit(w)).collect();
    if found.is_empty() {
        return Err(bad(CMD, "no words to bleep in the transcript"));
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let rate = q.settings.frame_rate;
    let seq_end = q.duration();
    let ranges = merge(
        found
            .iter()
            .map(|w| {
                let a = Tick(w.start.0.saturating_sub(pad.0).max(0));
                let b = Tick(w.end.0.saturating_add(pad.0)).min(seq_end.max(w.end));
                TimeRange::new(a, Tick(b.0.saturating_sub(a.0).max(1)))
            })
            .collect(),
    );
    // the clips that carry each word's sound: every audio clip of the same media under it
    let items: Vec<ItemId> = found.iter().map(|w| w.item).collect();
    let tone_secs = ranges.iter().map(|r| r.duration.seconds()).fold(1.0, f64::max).ceil() + 1.0;
    let st = q.settings.clone();
    let pool = s.media.clone();
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let fade = Tick::from_seconds_f64(0.01);
    let n = ranges.len();
    s.edit("Bleep Swear Words", |pr, _| {
        // the beep: a tone item, reused while it is long enough
        let tone = pr
            .items
            .values()
            .find(|i| {
                matches!(&i.kind, ItemKind::Media(m) if matches!(m.media, MediaRef::Generator(Generator::Tone { hz: h, db: d }) if h == hz && d == db) && m.duration().seconds() >= tone_secs)
            })
            .map(|i| i.id);
        let tone = match tone {
            Some(t) => t,
            None => {
                let src = GeneratorSource::new(Generator::Tone { hz, db }, st.width, st.height, st.frame_rate, Tick::from_seconds_f64(tone_secs));
                crate::demo::add_generator(pr, &pool, src, "Bleep Tone", Label::Rose, None)
            }
        };
        let mut beeps = Vec::new();
        for r in &ranges {
            let Some(mut ti) = pr.make_track_item(tone, TrackKind::Audio, r.start, TimeRange::new(Tick::ZERO, r.duration), rate) else { continue };
            ti.name = BLEEP.into();
            beeps.push(ti);
        }
        let track_id = TrackId(pr.alloc_id());
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        // mute the words: Volume dips to silence under each range
        for it in q.audio_tracks.iter_mut().flat_map(|t| t.items.iter_mut()).filter(|i| items.contains(&i.item)) {
            let last = Tick(it.end().0.saturating_sub(1));
            let span = it.range();
            for r in ranges.iter().filter(|r| r.overlaps(&span)) {
                let a = r.start.max(it.start);
                let b = r.end().min(last);
                let pts = [
                    (Tick(a.0.saturating_sub(fade.0)).max(it.start), None),
                    (a, Some(-96.0)),
                    (b, Some(-96.0)),
                    (Tick(b.0.saturating_add(fade.0)).min(last), None),
                ];
                let mts: Vec<(Tick, Option<f64>)> = pts.iter().map(|(t, v)| (it.source_time_at(*t), *v)).collect();
                let Some(level) = it.effect_mut("volume").and_then(|e| e.params.get_mut("level")) else { continue };
                let around: Vec<f64> = mts.iter().map(|(t, _)| level.f64_at(*t)).collect();
                for ((t, v), keep) in mts.into_iter().zip(around) {
                    put(level, t, v.unwrap_or(keep));
                }
            }
        }
        // the beeps on a new audio track
        let mut t = Track::new(track_id, TrackKind::Audio, "Bleeps".into());
        t.items = beeps;
        t.sort();
        q.audio_tracks.push(t);
        // Word Captions already made: mask the bleeped words in every clip of a phrase that is
        // on screen while one is bleeped (a phrase is consecutive clips showing the same text)
        let text_of = |it: &TrackItem| -> Option<String> {
            let v: Vec<&str> = it
                .effects
                .iter()
                .filter(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER)
                .filter_map(|e| match e.params.get("text").map(|p| &p.value) {
                    Some(ParamValue::Text(t)) => Some(t.as_str()),
                    _ => None,
                })
                .collect();
            (!v.is_empty()).then(|| v.join("\n"))
        };
        let mask = |txt: &str| txt.split(' ').map(|tok| if target(tok) { sh::mask_word(tok) } else { tok.to_string() }).collect::<Vec<_>>().join(" ");
        for tr in q.video_tracks.iter_mut() {
            let mut hit: Vec<bool> = vec![false; tr.items.len()];
            let mut k = 0usize;
            while k < tr.items.len() {
                let Some(key) = tr.items.get(k).and_then(text_of) else {
                    k += 1;
                    continue;
                };
                let mut j = k.saturating_add(1);
                while let (Some(prev), Some(next)) = (tr.items.get(j.saturating_sub(1)), tr.items.get(j)) {
                    if next.start != prev.end() || text_of(next).as_deref() != Some(key.as_str()) {
                        break;
                    }
                    j += 1;
                }
                let on = tr.items.get(k..j).is_some_and(|g| g.iter().any(|i| ranges.iter().any(|r| r.overlaps(&i.range()))));
                for h in hit.iter_mut().take(j).skip(k) {
                    *h = on;
                }
                k = j;
            }
            for (it, _) in tr.items.iter_mut().zip(hit).filter(|(_, h)| *h) {
                for e in it.effects.iter_mut().filter(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER) {
                    if let Some(Param { value: ParamValue::Text(txt), .. }) = e.params.get_mut("text") {
                        *txt = mask(txt);
                    }
                }
                it.name = mask(&it.name);
            }
        }
        q.check().map_err(EngineError::Other)?;
        Ok(())
    })?;
    Ok(json!({"bleeped": found.len(), "beeps": n}))
}

// ---------------------------------------------------------------------------------------------
// Duck Music Under Speech, Sound Effect at Playhead
// ---------------------------------------------------------------------------------------------

pub(crate) fn duck_music(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.duckMusic";
    let reduce = num_p(p, "reduceDb", -15.0, -40.0, 0.0, CMD)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let picked = crate::commands::clips_p(s, p);
    let on_audio = |c: &ClipId| q.audio_tracks.iter().any(|t| t.item(*c).is_some());
    // the selected clips' sound (a picked video clip brings its linked audio)
    let mut music: Vec<ClipId> = picked.iter().copied().filter(on_audio).collect();
    for c in &picked {
        if let Some(link) = q.find_item(*c).and_then(|(_, i)| i.link) {
            music.extend(q.audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.link == Some(link)).map(|i| i.id));
        }
    }
    music.sort();
    music.dedup();
    if music.is_empty() {
        return Err(bad(CMD, "select the music clip(s) first"));
    }
    let spans: Vec<TimeRange> = music.iter().filter_map(|c| q.find_item(*c).map(|(_, i)| i.range())).collect();
    let speech: Vec<ClipId> = q
        .audio_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|i| !music.contains(&i.id) && i.essential.is_none() && !is_bleep(&s.project, i) && spans.iter().any(|r| r.overlaps(&i.range())))
        .map(|i| i.id)
        .collect();
    let ids = |v: &[ClipId]| json!(v.iter().map(|c| c.0).collect::<Vec<_>>());
    let r = s.edit_group("Duck Music Under Speech", |s| {
        s.execute("essentialSound.setType", json!({"clips": ids(&music), "type": "music"}))?;
        if !speech.is_empty() {
            s.execute("essentialSound.setType", json!({"clips": ids(&speech), "type": "dialogue"}))?;
        }
        s.execute("essentialSound.set", json!({"clips": ids(&music), "values": {"ducking.enabled": true, "ducking.reduceDb": reduce}}))?;
        s.execute("essentialSound.generateDucking", json!({"clips": ids(&music)}))
    })?;
    Ok(json!({"music": music.len(), "dialogue": speech.len(), "ducking": r}))
}

pub(crate) fn sound_effect(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.soundEffect";
    let t = time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let items: Vec<ItemId> = match crate::commands::item_p(p, "item") {
        Some(i) => vec![i],
        None => s.state.project_selection.clone(),
    };
    let items: Vec<(ItemId, Tick)> = items
        .into_iter()
        .filter(|i| s.project.source_streams(*i).1)
        .filter_map(|i| s.project.item(i).map(|x| (i, x.duration())))
        .filter(|(_, d)| *d > Tick::ZERO)
        .collect();
    if items.is_empty() {
        return Err(bad(CMD, "select a sound file in the Project panel first (import your sound effects with File ▸ Import)"));
    }
    if items.len() > 32 {
        return Err(bad(CMD, "select at most 32 sounds at once"));
    }
    s.edit_group("Sound Effect at Playhead", |s| {
        let mut placed = Vec::new();
        for (item, dur) in &items {
            let range = TimeRange::new(t, *dur);
            let free = |s: &Session| -> Option<TrackId> {
                s.active_sequence()?.audio_tracks.iter().find(|tr| !tr.locked && !tr.items.iter().any(|i| i.range().overlaps(&range))).map(|tr| tr.id)
            };
            let track = match free(s) {
                Some(id) => id,
                None => {
                    s.execute("sequence.addTracks", json!({"video": 0, "audio": 1}))?;
                    free(s).ok_or_else(|| bad(CMD, "could not add an audio track"))?
                }
            };
            let ids = crate::commands::place_item(s, *item, TimeRange::new(Tick::ZERO, *dur), t, None, Some(track), false, "Sound Effect at Playhead", None)?;
            placed.extend(ids.into_iter().map(|c| c.0));
        }
        Ok(json!({"clips": placed}))
    })
}

// ---------------------------------------------------------------------------------------------
// Speed Ramp
// ---------------------------------------------------------------------------------------------

/// Speed Ramp at Playhead: the next `seconds` of the picture slow down to `speed` % and back with
/// Time Remapping, so the moment lasts longer. Later clips move right by the extra time; the
/// moment's sound plays at normal speed and then pauses, so everything after it stays in sync.
/// A stacked layout's panes ramp together.
pub(crate) fn speed_ramp(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.speedRamp";
    let slow = num_p(p, "speed", 35.0, 5.0, 95.0, CMD)? / 100.0;
    let secs = num_p(p, "seconds", 1.0, 0.2, 30.0, CMD)?;
    let ramp = num_p(p, "ramp", 0.2, 0.0, 5.0, CMD)?;
    let t = time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let rate = q.settings.frame_rate;
    let frame = rate.frame_duration().max(Tick(1));
    let t = rate.snap(t);
    // the footage under the playhead (every pane of a stacked layout), not on its first frame
    let under: Vec<TrackItem> =
        q.video_tracks.iter().flat_map(|tr| tr.items.iter()).filter(|i| i.start < t && t < i.end() && is_footage(&s.project, i)).cloned().collect();
    if under.is_empty() {
        return Err(bad(CMD, "move the playhead into a video clip (not onto its first frame)"));
    }
    if under.iter().any(|i| i.reverse || i.frame_hold.is_some()) {
        return Err(bad(CMD, "a reversed or held clip can't be ramped"));
    }
    // how much of the moment there is (timeline length at the clips' own speed)
    let room = under.iter().map(|i| i.end() - t).min().unwrap_or(Tick::ZERO);
    let len = rate.snap_nearest(Tick::from_seconds_f64(secs)).min(rate.snap(room));
    if len < Tick(frame.0.saturating_mul(2)) {
        return Err(bad(CMD, "there is too little of the clip left after the playhead"));
    }
    // ramp down over `a`, hold, ramp up over `a`: the picture covers `len` of the clip in `dur`
    let lm = len.0 as f64;
    let a0 = (Tick::from_seconds_f64(ramp).0 as f64).min(lm / (1.0 + slow));
    let hold0 = (lm - a0 * (1.0 + slow)) / slow;
    let dur = rate.snap_nearest(Tick((2.0 * a0 + hold0).round() as i64)).max(Tick(len.0.saturating_add(frame.0)));
    let d = dur.0 as f64;
    // after snapping the duration, pick the ramp length that makes the picture land exactly
    let a = ((lm - d * slow) / (1.0 - slow)).clamp(0.0, d / 2.0);
    let extra = dur - len;
    let ids: Vec<ClipId> = under.iter().map(|i| i.id).collect();
    let r = s.edit_sequence("Speed Ramp", |q, ctx, _| {
        // cut the moment out of each pane: [t, t + len)
        let mut middles: Vec<ClipId> = filmcraft_edit::razor_items(q, &ids, t, ctx);
        let rights = filmcraft_edit::razor_items(q, &middles, Tick(t.0.saturating_add(len.0)), ctx);
        let _ = rights;
        middles.sort();
        // everything after the moment moves right by the extra time (the sound pauses there)
        let at = Tick(t.0.saturating_add(len.0));
        let mut links = std::collections::HashMap::new();
        for tr in q.all_tracks_mut() {
            if !tr.locked && (tr.sync_lock || tr.items.iter().any(|i| middles.contains(&i.id))) {
                filmcraft_edit::insert_track_gap(tr, at, extra, ctx, &mut links);
            }
        }
        for ct in q.caption_tracks.iter_mut().filter(|c| !c.locked && c.sync_lock) {
            filmcraft_edit::captions::insert_gap(ct, at, extra, ctx);
        }
        // the moment fills the extra time, slowed by Time Remapping
        for it in q.video_tracks.iter_mut().flat_map(|tr| tr.items.iter_mut()).filter(|i| middles.contains(&i.id)) {
            it.duration = dur;
            let sp = it.speed.abs();
            let m0 = it.source_in;
            let nominal = |x: f64| Tick(m0.0.saturating_add((x * sp).round() as i64));
            let Some(speed) = it.effect_mut("time_remap").and_then(|e| e.params.get_mut("speed")) else { continue };
            speed.keyframes.clear();
            speed.value = ParamValue::Float(100.0);
            for (x, v) in [(0.0, 100.0), (a, slow * 100.0), (d - a, slow * 100.0), (d, 100.0)] {
                put(speed, nominal(x), v);
            }
        }
        q.check().map_err(EngineError::Other)?;
        Ok(middles)
    })?;
    Ok(json!({"clips": r.iter().map(|c| c.0).collect::<Vec<_>>(), "duration": dur.0, "added": extra.0}))
}

// ---------------------------------------------------------------------------------------------
// Effects on the Beat
// ---------------------------------------------------------------------------------------------

/// The longest music Effects on the Beat analyses.
const MAX_BEAT_SECONDS: f64 = 15.0 * 60.0;
/// Analysis sample rate for beat tracking.
const BEAT_SR: u32 = 22_050;

/// Effects on the Beat: finds the beats of the selected music clip and adds a pulse (or flash,
/// shake…) to the footage on every `every`-th beat.
pub(crate) fn beat_sync(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.beatSync";
    let picks = picks_p(p, "effects", &["pulse"], CMD)?;
    let every = num_p(p, "every", 2.0, 1.0, 16.0, CMD)?.round() as usize;
    let amount = num_p(p, "amount", 105.0, 101.0, 150.0, CMD)? / 100.0;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let picked = crate::commands::clips_p(s, p);
    // the music: a selected audio clip (a picked video clip brings its linked sound)
    let music = picked
        .iter()
        .find_map(|c| {
            q.audio_tracks.iter().find_map(|t| t.item(*c)).or_else(|| {
                let link = q.find_item(*c).and_then(|(_, i)| i.link)?;
                q.audio_tracks.iter().flat_map(|t| t.items.iter()).find(|i| i.link == Some(link))
            })
        })
        .cloned()
        .ok_or_else(|| bad(CMD, "select the music clip first"))?;
    if music.reverse || music.speed.abs() < 1e-6 {
        return Err(bad(CMD, "the music clip plays reversed or frozen"));
    }
    let media_len = Tick((music.duration.0 as f64 * music.speed.abs()).round() as i64);
    if media_len.seconds() > MAX_BEAT_SECONDS {
        return Err(bad(CMD, "the music clip is longer than 15 minutes"));
    }
    let src = s.source(music.item).ok_or_else(|| bad(CMD, "the music clip's media is offline"))?;
    let start = music.source_in.to_units_floor(i64::from(BEAT_SR));
    let frames = usize::try_from(media_len.to_units_floor(i64::from(BEAT_SR))).unwrap_or(0);
    let buf = src.audio(start, frames, BEAT_SR).map_err(|e| bad(CMD, format!("can't read the music: {e}")))?;
    let chans: Vec<&[f32]> = buf.channels.iter().map(Vec::as_slice).collect();
    let analysis = filmcraft_audio_dsp::remix::analyze(&chans, BEAT_SR).map_err(|e| {
        bad(
            CMD,
            match e {
                filmcraft_audio_dsp::remix::RemixError::TooShort => "the music clip is too short to find its beat (it needs ten beats or more)".to_string(),
                _ => "no steady beat found in that clip".to_string(),
            },
        )
    })?;
    let lead = filmcraft_audio_dsp::remix::beat_lead(BEAT_SR);
    let (from, to) = (q.mark_in.unwrap_or(Tick::ZERO), q.mark_out.unwrap_or(Tick(i64::MAX)));
    let times: Vec<Tick> = analysis
        .beats
        .iter()
        .step_by(every)
        .filter_map(|b| {
            // the attack (beats sit just before it), from media samples to the timeline
            let m = Tick::from_seconds_f64(b.saturating_add(lead) as f64 / f64::from(BEAT_SR));
            let t = Tick(music.start.0.saturating_add((m.0 as f64 / music.speed.abs()).round() as i64));
            (t >= music.start && t < music.end() && t >= from && t < to).then_some(t)
        })
        .take(4000)
        .collect();
    if times.is_empty() {
        return Err(bad(CMD, "no beats fall inside the clip (or the In/Out range)"));
    }
    let (n, flashes) = effects_at_with(s, "Effects on the Beat", &times, picks, "Beat Flashes", amount, 35.0)?;
    Ok(json!({"bpm": analysis.bpm, "beats": times.len(), "clips": n, "flashes": flashes}))
}
