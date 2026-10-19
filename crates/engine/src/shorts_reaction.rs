//! The Reaction Split format: a short cut like the reaction clips of streamers' editors.
//!
//! - **Split screen**: the gameplay on the top half (zoomed in to fill it), the streamer's model
//!   filling the bottom half (its head and shoulders, not shrunk into a box).
//! - **Captions on the seam**, two or three words at a time as they are said, in capitals when
//!   shouted, swears starred ("f*ck") and bleeped ([`crate::shorts_captions`]'s Reaction style).
//! - **Scene switches** ([`plan`]): the gameplay full screen (with a slow push-in) where the
//!   streamer goes quiet or reacts ("huh?", "what?") and the game moves; the streamer's face full
//!   screen for the last line, the punchline.
//! - **Hits**: a bass hit on every switch into a full-screen scene and where a shouting run
//!   starts, a white flash on the punchline, the first full-screen gameplay fading to grey at its
//!   end, and the short ending in black and white, shrinking into a black frame.
//! - **Meme drops** from your own meme folder ([`crate::shorts_memes`]).
//!
//! Every choice is from the transcript (when each word is said, and how loud), and from how much
//! the gameplay changes between keyframes. [`plan`] is a pure function of those, tested on its own.

use filmcraft_edit::shorts::{self as sh, Rect};
use filmcraft_project::{ClipId, ItemKind, Label, Param, ParamValue, TrackKind};
use filmcraft_time::{Tick, TimeRange};
use serde_json::json;

use crate::shorts::{put, set_motion, set_static};
use crate::shorts_layout as ly;
use crate::{EngineError, Result, Session};

/// The tracks the format adds.
pub(crate) const HITS_TRACK: &str = "Bass Hits";
const GRADE_TRACK: &str = "Grade";
const HIT_NAME: &str = "Bass Hit";
/// The white flash before the punchline (named apart from the Flash at the playhead, so a format
/// applied again takes away only its own).
pub(crate) const FLASH_NAME: &str = "Punchline Flash";
/// Words a streamer reacts with: a good moment to show the game.
const REACTIONS: &[&str] = &["huh", "what", "wait", "look", "oh", "no", "bro", "yo", "help", "dude", "wow", "omg", "why", "how", "yes", "nah", "bruh"];

/// What the frame shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scene {
    /// The gameplay on top, the model below.
    Split,
    /// The gameplay full screen.
    Game,
    /// The streamer's model full screen.
    Face,
}

/// A stretch of the short and its scene (seconds).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Segment {
    pub start: f64,
    pub end: f64,
    pub scene: Scene,
}

/// A word as the planner sees it: when (seconds), what, and whether it is shouted.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub loud: bool,
}

/// Everything [`plan`] decides.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct Plan {
    /// Contiguous, from 0 to the end; Split wherever nothing else is.
    pub segments: Vec<Segment>,
    /// Bass hits (seconds).
    pub hits: Vec<f64>,
    /// The white flash.
    pub flash: Option<f64>,
    /// A fade to grey (start, end).
    pub gray: Option<(f64, f64)>,
    /// The black-and-white ending (start, end).
    pub ending: Option<(f64, f64)>,
}

impl Plan {
    /// The stretch of the face-cam ending, if any.
    pub(crate) fn face(&self) -> Option<(f64, f64)> {
        self.segments.iter().rev().find(|s| s.scene == Scene::Face).map(|s| (s.start, s.end))
    }
}

/// What the planner may do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Choices {
    pub scenes: bool,
    pub hits: bool,
    pub ending: bool,
}

fn ends_sentence(t: &str) -> bool {
    t.trim_end().ends_with(['.', '?', '!'])
}

/// The scenes, hits and ending for a `dur`-second short with `words` said in it; `activity`:
/// (seconds, 0..1) how much the gameplay changes around then.
pub(crate) fn plan(words: &[Word], dur: f64, activity: &[(f64, f64)], c: Choices) -> Plan {
    let mut out = Plan::default();
    if !(dur.is_finite() && dur > 0.0) {
        return out;
    }
    let words: Vec<&Word> = words.iter().filter(|w| w.start.is_finite() && w.end.is_finite() && w.start < dur).collect();
    // the punchline: the last sentence, 1.6 to 4.5 s long, full-screen face
    let mut face = None;
    if c.scenes && dur >= 8.0 {
        let starts: Vec<f64> = words
            .iter()
            .enumerate()
            .filter(|(i, w)| *i == 0 || words.get(i - 1).is_some_and(|p| ends_sentence(&p.text) || w.start - p.end >= 0.35))
            .map(|(_, w)| w.start)
            .collect();
        let fits = |t: &f64| *t <= dur - 1.6 && *t >= dur - 4.5;
        let at = starts
            .iter()
            .rev()
            .copied()
            .find(fits)
            .or_else(|| words.iter().rev().map(|w| w.start).find(|t| *t <= dur - 2.0 && *t >= dur - 4.0))
            .unwrap_or(dur - 2.5);
        face = Some((at.max(0.0), dur));
    }
    // full-screen gameplay: where the talk stops or turns into a reaction, and the game moves
    let mut games: Vec<(f64, f64)> = Vec::new();
    if c.scenes {
        let last = face.map_or(dur, |f| f.0) - 0.8;
        let n = ((last / 10.0).floor() as usize).min(4);
        let speech = |a: f64, b: f64| -> f64 { words.iter().map(|w| (w.end.min(b) - w.start.max(a)).max(0.0)).sum::<f64>() };
        let moving = |a: f64, b: f64| -> f64 {
            let v: Vec<f64> = activity.iter().filter(|(t, _)| *t >= a - 1.0 && *t <= b + 1.0).map(|(_, x)| x.clamp(0.0, 1.0)).collect();
            if v.is_empty() { 0.3 } else { v.iter().sum::<f64>() / v.len() as f64 }
        };
        // where a new sentence starts: the cleanest places to switch back
        let sentence_start = |i: usize| i == 0 || words.get(i - 1).zip(words.get(i)).is_some_and(|(p, w)| ends_sentence(&p.text) || w.start - p.end >= 0.35);
        let mut cands: Vec<(f64, f64, f64)> = Vec::new();
        for (i, w) in words.iter().enumerate() {
            let next = words.get(i + 1).map_or(dur, |x| x.start);
            let reaction = REACTIONS.contains(&sh::normalize_word(&w.text).as_str());
            let mut starts = Vec::new();
            if next - w.end >= 0.45 {
                starts.push((w.end, 0.0));
            }
            if reaction {
                starts.push((w.start, 0.6));
            }
            for (t0, bonus) in starts {
                // back to the split 1.6 to 3 s later, on a word (where a sentence starts if one does)
                let target = t0 + 2.2;
                let nearest = |only_sentences: bool| {
                    words
                        .iter()
                        .enumerate()
                        .filter(|(k, x)| x.start >= t0 + 1.6 && x.start <= t0 + 3.0 && (!only_sentences || sentence_start(*k)))
                        .map(|(_, x)| x.start)
                        .min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()))
                };
                let (end, clean) = match nearest(true) {
                    Some(e) => (e, 0.4),
                    None => (nearest(false).unwrap_or(target), 0.0),
                };
                if t0 < 2.0 || end > last {
                    continue;
                }
                let quiet = 1.0 - speech(t0, end) / (end - t0).max(1e-6);
                cands.push((quiet * 2.0 + bonus + clean + moving(t0, end), t0, end));
            }
        }
        cands.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.total_cmp(&b.1)));
        for (_, a, b) in cands {
            if games.len() >= n {
                break;
            }
            if games.iter().all(|(x, y)| b + 3.5 <= *x || a >= *y + 3.5) {
                games.push((a, b));
            }
        }
        games.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    // the segments
    let mut t = 0.0;
    for (a, b) in &games {
        if *a > t {
            out.segments.push(Segment { start: t, end: *a, scene: Scene::Split });
        }
        out.segments.push(Segment { start: *a, end: *b, scene: Scene::Game });
        t = *b;
    }
    let split_end = face.map_or(dur, |f| f.0);
    if split_end > t {
        out.segments.push(Segment { start: t, end: split_end, scene: Scene::Split });
    }
    if let Some((a, b)) = face {
        out.segments.push(Segment { start: a, end: b, scene: Scene::Face });
    }
    // the first full-screen gameplay of 1.8 s or more fades to grey at its end
    out.gray = c.hits.then(|| games.iter().find(|(a, b)| b - a >= 1.8).map(|(_, b)| (b - 0.7, *b))).flatten();
    if c.hits {
        let mut hits: Vec<f64> = games.iter().map(|(a, _)| *a).chain(face.map(|f| f.0)).collect();
        // where a shouting run starts (two loud words in a row, or one after a pause)
        for (i, w) in words.iter().enumerate() {
            let prev = i.checked_sub(1).and_then(|p| words.get(p));
            let next_loud = words.get(i + 1).is_some_and(|n| n.loud);
            let starts_run = w.loud && prev.is_none_or(|p| !p.loud || w.start - p.end >= 0.4) && (next_loud || prev.is_none_or(|p| w.start - p.end >= 0.4));
            if starts_run && w.start >= 1.0 && w.start < split_end {
                hits.push(w.start);
            }
        }
        hits.sort_by(f64::total_cmp);
        let cap = ((dur / 4.0).floor() as usize).max(1);
        let mut kept: Vec<f64> = Vec::new();
        for h in hits {
            if kept.len() < cap && kept.last().is_none_or(|k| h - k >= 2.5) && h < dur - 0.3 {
                kept.push(h);
            }
        }
        out.hits = kept;
        // the flash: on the loudest moment of the punchline
        out.flash =
            face.map(|(a, b)| words.iter().filter(|w| w.loud && w.start >= a && w.start < b - 0.3).map(|w| w.start).next().unwrap_or(a + 0.25).min(b - 0.3));
    }
    if c.ending && dur >= 4.0 {
        let len = face.map_or(1.3, |(a, b)| ((b - a) * 0.5).clamp(0.8, 1.3));
        out.ending = Some((dur - len, dur));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// The clip, as the planner needs it
// ---------------------------------------------------------------------------------------------

/// The planner's words: the transcript with each word's loudness.
pub(crate) fn words_of(s: &Session) -> Vec<Word> {
    let words = crate::transcript::sequence_words(s);
    let levels = crate::shorts_fx::word_levels(s, &words);
    let loud = crate::shorts_fx::shouted(&levels);
    words
        .iter()
        .enumerate()
        .map(|(i, w)| Word { start: w.start.seconds(), end: w.end.seconds(), text: w.text.clone(), loud: loud.get(i).copied().unwrap_or(false) })
        .collect()
}

/// How much the gameplay (`region`, fractions of the recording) changes from keyframe to keyframe
/// of the footage: (sequence seconds, 0..1).
pub(crate) fn activity(s: &Session, region: [f64; 4]) -> Vec<(f64, f64)> {
    use filmcraft_render::SourceProvider;
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let ids = ly::targets(s, &json!({}));
    let clips: Vec<(filmcraft_project::ItemId, Tick, Tick, Tick)> = q
        .video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|i| ids.contains(&i.id) && i.frame_hold.is_none())
        .map(|i| (i.item, i.start, i.end(), i.source_in))
        .collect();
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let mut frames: Vec<(f64, crate::shorts_auto::Frame)> = Vec::new();
    for (item, start, end, source_in) in clips {
        let Some(src) = provider.source(item) else { continue };
        let w = filmcraft_render::source_size(&s.project, item).map_or(1920, |(w, _)| w).max(1);
        let scale = (320.0 / w as f32).clamp(0.05, 1.0);
        let mut t = start;
        let mut last_key: Option<Tick> = None;
        let step = Tick::from_seconds_f64(1.0);
        // one picture per keyframe (they decode on their own), at most 90
        while t < end && frames.len() < 90 {
            let media = Tick(source_in.0.saturating_add(t.0.saturating_sub(start.0)));
            let key = src.keyframe_before(media).filter(|k| *k >= source_in).unwrap_or(media);
            if last_key != Some(key) {
                last_key = Some(key);
                let at = Tick(start.0.saturating_add(key.0.saturating_sub(source_in.0)));
                if let Some(f) = filmcraft_render::render_item(&s.project, item, key, scale, &provider).as_ref().and_then(crate::shorts_auto::Frame::of) {
                    frames.push((at.seconds(), f));
                }
            }
            t = Tick(t.0.saturating_add(step.0));
        }
    }
    frames
        .windows(2)
        .filter_map(|p| match p {
            [(ta, a), (tb, b)] => Some(((ta + tb) / 2.0, ((b.change(a, region) - 0.02) / 0.12).clamp(0.0, 1.0))),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Putting the plan on the timeline
// ---------------------------------------------------------------------------------------------

/// `region` (fractions of a `src`-sized picture) cut to `aspect` (width / height): the full width
/// kept and the bottom cut off (a model's head is at the top), or the sides cut off evenly.
pub(crate) fn top_anchored(region: [f64; 4], src: (f64, f64), aspect: f64) -> [f64; 4] {
    let (w, h) = ((region[2] - region[0]) * src.0, (region[3] - region[1]) * src.1);
    if !(w > 1.0 && h > 1.0 && aspect.is_finite() && aspect > 0.0) {
        return region;
    }
    if w / h < aspect {
        // too tall: keep the top (a little under the region's top edge, past any frame of it)
        let nh = w / aspect;
        let y0 = region[1] + ((h - nh) * 0.1) / src.1;
        [region[0], y0, region[2], y0 + nh / src.1]
    } else {
        let nw = h * aspect;
        let cx = (region[0] + region[2]) / 2.0;
        [cx - nw / 2.0 / src.0, region[1], cx + nw / 2.0 / src.0, region[3]]
    }
}

/// Put `region` of the clip's recording on the whole frame (no crop corners, no border).
fn fill_frame(it: &mut filmcraft_project::TrackItem, region: [f64; 4], storage: (f64, f64), display: (f64, f64), frame: (f64, f64)) -> Option<f64> {
    let pl = sh::place_region(display, Rect::from_fractions(region, display.0, display.1), Rect::new(0.0, 0.0, frame.0, frame.1))?;
    let anchor = (pl.anchor.0 * storage.0 / display.0.max(1.0), pl.anchor.1 * storage.1 / display.1.max(1.0));
    set_motion(it, pl.scale, pl.position, anchor);
    if let Some(e) = it.effects.iter_mut().find(|e| e.effect == "rounded_crop") {
        for (k, v) in ["left", "top", "right", "bottom"].into_iter().zip(pl.crop) {
            set_static(e, k, ParamValue::Float(v));
        }
        set_static(e, "radius", ParamValue::Float(0.0));
        set_static(e, "border", ParamValue::Float(0.0));
    }
    Some(pl.scale)
}

/// Switch the footage to each segment's scene: cut the gameplay and model clips at every switch,
/// then show the gameplay (with a slow push-in) or the model full screen, or leave the split.
pub(crate) fn apply_scenes(s: &mut Session, plan: &Plan, model_full: [f64; 4], game_full: [f64; 4]) -> Result<()> {
    if plan.segments.iter().all(|g| g.scene == Scene::Split) {
        return Ok(());
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = (f64::from(q.settings.width), f64::from(q.settings.height));
    let rate = q.settings.frame_rate;
    let par = q.settings.par;
    let game_ids = ly::targets(s, &json!({}));
    // the tracks holding the gameplay and the model
    let game_track = q.video_tracks.iter().position(|t| t.items.iter().any(|i| game_ids.contains(&i.id)));
    let model_track = q.video_tracks.iter().position(|t| t.name == "Model");
    let tracks: Vec<filmcraft_project::TrackId> = [game_track, model_track].into_iter().flatten().filter_map(|i| q.video_tracks.get(i).map(|t| t.id)).collect();
    let cuts: Vec<Tick> = plan.segments.iter().skip(1).map(|g| rate.snap_nearest(Tick::from_seconds_f64(g.start))).collect();
    let sizes: Vec<(filmcraft_project::ItemId, ((f64, f64), (f64, f64)))> =
        q.video_tracks.iter().flat_map(|t| t.items.iter()).filter_map(|i| crate::shorts::picture_sizes(&s.project, i, par).map(|z| (i.item, z))).collect();
    let frame_dur = rate.frame_duration().max(Tick(1));
    s.edit_sequence("Scenes", |q, ctx, _| {
        for t in &cuts {
            filmcraft_edit::razor(q, &tracks, *t, ctx);
        }
        // a piece's scene is the one around its middle (the cuts are on frames, the plan is not)
        let seg_of = |it: &filmcraft_project::TrackItem| -> Scene {
            let at = (it.start.seconds() + it.end().seconds()) / 2.0;
            plan.segments.iter().find(|g| at >= g.start && at < g.end).map_or(Scene::Split, |g| g.scene)
        };
        let size_of = |item| sizes.iter().find(|(i, _)| *i == item).map(|(_, z)| *z);
        for (ti, keep_scene, region) in [(game_track, Scene::Game, game_full), (model_track, Scene::Face, model_full)] {
            let Some(tr) = ti.and_then(|i| q.video_tracks.get_mut(i)) else { continue };
            // the model goes where the gameplay fills the frame (the gameplay stays under the
            // face, which covers it: the footage stays whole for a format applied again)
            if keep_scene == Scene::Face {
                tr.items.retain(|it| seg_of(it) != Scene::Game);
            }
            for it in tr.items.iter_mut().filter(|it| seg_of(it) == keep_scene) {
                let Some((storage, display)) = size_of(it.item) else { continue };
                let Some(k) = fill_frame(it, region, storage, display, frame) else { continue };
                if keep_scene == Scene::Game {
                    // a slow push-in across the piece
                    let last = Tick(it.end().0.saturating_sub(frame_dur.0));
                    let (a, b) = (it.effect_time_at(it.start), it.effect_time_at(last));
                    if let Some(sc) = it.effect_mut("motion").and_then(|m| m.params.get_mut("scale")) {
                        put(sc, a, k);
                        put(sc, b, k * 1.12);
                    }
                }
            }
        }
        q.check().map_err(EngineError::Other)?;
        Ok(())
    })
}

/// Take away what a Reaction Split added besides its layout and captions: the bass hits, the
/// grades, the memes and the punchline flash (for a format applied again in place). Tracks are
/// removed only when everything on them is the format's own.
pub(crate) fn clear(q: &mut filmcraft_project::Sequence, pr: &filmcraft_project::Project) {
    let graphic_or_layer =
        |it: &filmcraft_project::TrackItem| pr.item(it.item).is_some_and(|x| matches!(x.kind, ItemKind::Graphic { .. } | ItemKind::AdjustmentLayer { .. }));
    let meme = |it: &filmcraft_project::TrackItem| it.name.starts_with(crate::shorts_memes::PREFIX);
    for t in &mut q.video_tracks {
        t.items.retain(|it| !(it.name == FLASH_NAME && graphic_or_layer(it)));
    }
    q.video_tracks.retain(|t| match t.name.as_str() {
        GRADE_TRACK => !t.items.iter().all(graphic_or_layer),
        crate::shorts_memes::TRACK => !t.items.iter().all(meme),
        _ => true,
    });
    q.audio_tracks.retain(|t| match t.name.as_str() {
        HITS_TRACK => !t.items.iter().all(|it| it.name == HIT_NAME),
        crate::shorts_memes::SOUND_TRACK => !t.items.iter().all(meme),
        _ => true,
    });
}

/// Bass hits at `times` (seconds): the built-in synthesised one, or `sound` (a file of the meme
/// folder named `boom`).
pub(crate) fn add_hits(s: &mut Session, times: &[f64], sound: Option<&std::path::Path>) -> Result<usize> {
    if times.is_empty() {
        return Ok(0);
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let st = q.settings.clone();
    let end = q.duration();
    let file_item = match sound {
        Some(p) => crate::shorts_memes::item_for(s, p).ok(),
        None => None,
    };
    let pool = s.media.clone();
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let rate = st.frame_rate;
    s.edit("Bass Hits", |pr, _| {
        let (item, len) = match file_item.and_then(|i| pr.item(i).map(|x| (i, x.duration()))) {
            Some((i, d)) if d > Tick::ZERO => (i, d.min(Tick::from_seconds_f64(3.0))),
            _ => {
                let len = Tick::from_seconds_f64(1.4);
                let existing = pr
                    .items
                    .values()
                    .find(|i| matches!(&i.kind, ItemKind::Media(m) if matches!(m.media, filmcraft_project::MediaRef::Generator(filmcraft_media::Generator::Boom { .. })) && m.duration() >= len))
                    .map(|i| i.id);
                let id = match existing {
                    Some(id) => id,
                    None => {
                        let src = filmcraft_media::generators::GeneratorSource::new(filmcraft_media::Generator::Boom { db: -3.0 }, st.width, st.height, st.frame_rate, len);
                        crate::demo::add_generator(pr, &pool, src, "Bass Hit", Label::Rose, None)
                    }
                };
                (id, len)
            }
        };
        let mut items = Vec::new();
        for t in times {
            let at = rate.snap_nearest(Tick::from_seconds_f64(*t));
            if at >= end {
                continue;
            }
            let d = len.min(Tick(end.0.saturating_sub(at.0)));
            if let Some(mut ti) = pr.make_track_item(item, TrackKind::Audio, at, TimeRange::new(Tick::ZERO, d), rate) {
                ti.name = HIT_NAME.into();
                items.push(ti);
            }
        }
        let n = items.len();
        // hits can overlap a little: as many tracks as it takes
        let mut tracks: Vec<filmcraft_project::Track> = Vec::new();
        for ti in items {
            let r = ti.range();
            match tracks.iter_mut().find(|t| !t.items.iter().any(|i| i.range().overlaps(&r))) {
                Some(t) => t.items.push(ti),
                None => {
                    let mut t = filmcraft_project::Track::new(filmcraft_project::TrackId(pr.alloc_id()), TrackKind::Audio, HITS_TRACK.into());
                    t.items.push(ti);
                    tracks.push(t);
                }
            }
        }
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        for mut t in tracks {
            t.sort();
            q.audio_tracks.push(t);
        }
        q.check().map_err(EngineError::Other)?;
        Ok(n)
    })
}

/// An adjustment layer from `a` to `b` (seconds) on the Grade track (above the footage and the
/// memes, under the captions) with `effects` (keyframe times are from the layer's start).
fn grade(s: &mut Session, label: &str, name: &str, a: f64, b: f64, effects: Vec<filmcraft_project::EffectInstance>) -> Result<()> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let st = q.settings.clone();
    let rate = st.frame_rate;
    let end = q.duration();
    let start = rate.snap_nearest(Tick::from_seconds_f64(a.max(0.0)));
    let stop = rate.snap_nearest(Tick::from_seconds_f64(b)).min(end);
    if stop <= start {
        return Ok(());
    }
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    s.edit(label, |pr, _| {
        let item =
            pr.add_item(name, Label::Lavender, ItemKind::AdjustmentLayer { width: st.width, height: st.height, rate, duration: Tick(stop.0 - start.0) }, None);
        let Some(mut ti) = pr.make_track_item(item, TrackKind::Video, start, TimeRange::new(Tick::ZERO, Tick(stop.0 - start.0)), rate) else {
            return Ok(());
        };
        ti.name = name.into();
        ti.effects.extend(effects);
        let id = pr.alloc_id();
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        let range = ti.range();
        let slot = q.video_tracks.iter().position(|t| t.name == GRADE_TRACK && !t.items.iter().any(|i| i.range().overlaps(&range)));
        let i = match slot {
            Some(i) => i,
            None => {
                // over the footage and the memes, under captions, titles and flashes
                let at = q
                    .video_tracks
                    .iter()
                    .position(|t| t.name == crate::shorts_captions::TRACK || t.name == "Flash" || t.items.iter().any(|i| i.name.starts_with("Flash")))
                    .unwrap_or(q.video_tracks.len());
                q.video_tracks.insert(at, crate::shorts::new_video_track(id, GRADE_TRACK));
                at
            }
        };
        if let Some(t) = q.video_tracks.get_mut(i) {
            t.items.push(ti);
            t.sort();
        }
        q.check().map_err(EngineError::Other)?;
        Ok(())
    })
}

fn keyed(v0: f64, v1: f64, over: Tick) -> Param {
    let mut p = Param::new(ParamValue::Float(v1));
    p.keyframes = vec![filmcraft_project::Keyframe::new(Tick::ZERO, ParamValue::Float(v0)), {
        let mut k = filmcraft_project::Keyframe::new(over, ParamValue::Float(v1));
        k.interp = filmcraft_project::Interpolation::EaseOut;
        k
    }];
    p
}

fn effect(id: &str, params: Vec<(&str, Param)>) -> Option<filmcraft_project::EffectInstance> {
    let mut e = filmcraft_project::find_effect(id)?.instance();
    for (k, v) in params {
        e.params.insert(k.into(), v);
    }
    Some(e)
}

/// The grey fade and the black-and-white ending.
pub(crate) fn add_grades(s: &mut Session, plan: &Plan) -> Result<()> {
    if let Some((a, b)) = plan.gray {
        let over = Tick::from_seconds_f64((b - a).max(0.05));
        if let Some(e) = effect("proc_amp", vec![("saturation", keyed(100.0, 10.0, over))]) {
            grade(s, "Grey Fade", "Grey Fade", a, b, vec![e])?;
        }
    }
    if let Some((a, b)) = plan.ending {
        let over = Tick::from_seconds_f64(((b - a) * 0.4).clamp(0.1, 0.5));
        let fx: Vec<filmcraft_project::EffectInstance> =
            [effect("proc_amp", vec![("saturation", keyed(100.0, 0.0, over))]), effect("transform", vec![("scale_height", keyed(100.0, 86.0, over))])]
                .into_iter()
                .flatten()
                .collect();
        grade(s, "Black-and-White Ending", "Black-and-White Ending", a, b, fx)?;
    }
    Ok(())
}

/// Move the captions said in `stretches` (seconds) to `y` (fraction of the frame height): under
/// the face when it fills the frame.
pub(crate) fn move_captions(s: &mut Session, stretches: &[(f64, f64)], y: f64) -> Result<()> {
    if stretches.is_empty() {
        return Ok(());
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let fh = f64::from(q.settings.height);
    let ids: Vec<ClipId> = q
        .video_tracks
        .iter()
        .filter(|t| t.name == crate::shorts_captions::TRACK)
        .flat_map(|t| t.items.iter())
        .filter(|i| {
            let mid = (i.start.seconds() + i.end().seconds()) / 2.0;
            stretches.iter().any(|(a, b)| mid >= *a && mid < *b)
        })
        .map(|i| i.id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    s.edit_sequence("Captions Under the Face", |q, _, _| {
        for it in q.video_tracks.iter_mut().flat_map(|t| t.items.iter_mut()).filter(|i| ids.contains(&i.id)) {
            for e in &mut it.effects {
                let Some(p) = e.params.get_mut("position") else { continue };
                let shift = |v: &mut ParamValue, to: f64| {
                    if let ParamValue::Vec2(v) = v {
                        v.y = to;
                    }
                };
                let base_y = match &p.value {
                    ParamValue::Vec2(v) => v.y,
                    _ => continue,
                };
                let dy = y * fh - base_y;
                shift(&mut p.value, y * fh);
                for k in &mut p.keyframes {
                    if let ParamValue::Vec2(v) = &mut k.value {
                        v.y += dy;
                    }
                }
            }
        }
        Ok(())
    })
}

/// The model region of the current layout, and the recording's display size.
pub(crate) fn regions(s: &Session) -> Option<([f64; 4], [f64; 4], (f64, f64))> {
    let v = s.state.shorts_layout.as_ref()?;
    let src = ly::source_size(s)?;
    let model = crate::shorts::fractions_p(v, "model", "shorts.reaction").ok().flatten().unwrap_or_else(|| ly::model_region(ly::Side::Right));
    let game = crate::shorts::fractions_p(v, "game", "shorts.reaction").ok().flatten().unwrap_or_else(|| ly::game_region(ly::Side::Right));
    Some((game, model, src))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, start: f64, end: f64, loud: bool) -> Word {
        Word { start, end, text: text.into(), loud }
    }

    const ALL: Choices = Choices { scenes: true, hits: true, ending: true };

    /// A 24 s rant: talk, a pause with a "what?", talk, a "huh?" and "help!", a shouting run, the
    /// punchline.
    fn rant() -> Vec<Word> {
        let mut v = vec![
            w("Why", 0.0, 0.3, false),
            w("are", 0.3, 0.5, false),
            w("people", 0.5, 0.9, false),
            w("screaming", 0.9, 1.5, false),
            w("in", 1.6, 1.7, false),
            w("chat?", 1.7, 2.2, false),
        ];
        v.extend([w("Oh,", 2.4, 2.6, false), w("that's", 2.7, 3.0, false), w("why.", 3.0, 3.2, false), w("What?", 3.4, 3.8, false)]);
        v.extend([
            w("It", 5.6, 5.7, false),
            w("literally", 5.7, 6.2, false),
            w("flashed", 6.3, 6.7, false),
            w("on", 6.7, 6.8, false),
            w("my", 6.8, 6.9, false),
            w("screen.", 6.9, 7.5, false),
        ]);
        v.extend([w("How", 7.8, 7.9, false), w("did", 8.0, 8.1, false), w("you", 8.1, 8.2, false), w("get", 8.2, 8.4, false), w("outside?", 8.4, 9.0, false)]);
        v.extend([w("Huh?", 9.3, 9.8, false), w("Help!", 10.6, 11.0, true), w("help!", 11.2, 11.6, true)]);
        v.extend([
            w("How", 12.6, 12.7, false),
            w("did", 12.8, 12.9, false),
            w("you", 12.9, 13.0, false),
            w("get", 13.0, 13.2, false),
            w("outside?", 13.2, 13.8, false),
        ]);
        for (i, t) in ["HOW", "THE", "HELL", "ARE", "YOU", "GOING", "TO", "GO", "AHEAD", "AND", "HIDE"].iter().enumerate() {
            let a = 15.0 + i as f64 * 0.4;
            v.push(w(t, a, a + 0.35, true));
        }
        v.push(w("map?", 19.6, 20.2, true));
        v.extend([
            w("I", 21.8, 21.9, false),
            w("swear", 21.9, 22.3, true),
            w("to", 22.3, 22.4, false),
            w("God,", 22.4, 22.8, false),
            w("bro!", 22.9, 23.4, true),
        ]);
        v.extend([w("What", 23.6, 23.8, false), w("kind", 23.8, 24.0, false), w("of", 24.0, 24.1, false)]);
        v
    }

    #[test]
    fn a_rant_gets_two_gameplay_moments_a_punchline_and_hits() {
        let p = plan(&rant(), 25.0, &[], ALL);
        let scenes: Vec<Scene> = p.segments.iter().map(|g| g.scene).collect();
        assert_eq!(scenes.iter().filter(|s| **s == Scene::Game).count(), 2, "{p:?}");
        assert_eq!(scenes.last(), Some(&Scene::Face));
        assert_eq!(scenes.first(), Some(&Scene::Split));
        // the segments cover the short without gaps
        assert_eq!(p.segments.first().map(|g| g.start), Some(0.0));
        assert_eq!(p.segments.last().map(|g| g.end), Some(25.0));
        assert!(p.segments.windows(2).all(|g| (g[0].end - g[1].start).abs() < 1e-9 && g[0].scene != g[1].scene || g[0].end == g[1].start));
        // the punchline starts at "I swear"
        assert_eq!(p.face().map(|f| f.0), Some(21.8));
        // the gameplay moments: after "what?" and at "huh?"
        let games: Vec<(f64, f64)> = p.segments.iter().filter(|g| g.scene == Scene::Game).map(|g| (g.start, g.end)).collect();
        assert!(games.iter().any(|(a, _)| (3.4..=4.0).contains(a)), "{games:?}");
        assert!(games.iter().any(|(a, _)| (9.0..=10.8).contains(a)), "{games:?}");
        for (a, b) in &games {
            assert!(b - a >= 1.6 && b - a <= 3.0 + 1e-9, "{a}..{b}");
        }
        // hits on the switches and where the shouting starts, 2.5 s apart at least
        assert!(p.hits.windows(2).all(|h| h[1] - h[0] >= 2.5), "{:?}", p.hits);
        assert!(p.hits.iter().any(|h| (*h - 15.0).abs() < 1e-9), "the shouting run: {:?}", p.hits);
        assert!(p.hits.iter().any(|h| (*h - 21.8).abs() < 1e-9));
        // the flash on the punchline's first shouted word; grey at the end of the first moment
        assert_eq!(p.flash, Some(21.9));
        assert!(p.gray.is_some_and(|(a, b)| (b - a - 0.7).abs() < 1e-9 && games.first().is_some_and(|g| (g.1 - b).abs() < 1e-9)));
        assert!(p.ending.is_some_and(|(a, b)| b == 25.0 && (0.8..=1.3 + 1e-9).contains(&(b - a))));
    }

    #[test]
    fn choices_and_short_clips() {
        let p = plan(&rant(), 25.0, &[], Choices { scenes: false, hits: true, ending: false });
        assert_eq!(p.segments, vec![Segment { start: 0.0, end: 25.0, scene: Scene::Split }]);
        assert!(p.ending.is_none() && p.flash.is_none());
        let p = plan(&rant(), 25.0, &[], Choices { scenes: true, hits: false, ending: true });
        assert!(p.hits.is_empty() && p.gray.is_none() && p.flash.is_none() && p.ending.is_some());
        // a 5 s clip: no scenes, still an ending
        let p = plan(&rant(), 5.0, &[], ALL);
        assert!(p.segments.iter().all(|g| g.scene == Scene::Split));
        // nothing said: a gameplay-free split and a face ending at the last 2.5 s
        let p = plan(&[], 20.0, &[], ALL);
        assert_eq!(p.face(), Some((17.5, 20.0)));
        // hostile lengths
        for d in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(plan(&rant(), d, &[], ALL), Plan::default());
        }
        let odd = vec![w("x", f64::NAN, 1.0, true), w("y", 2.0, f64::INFINITY, true)];
        let _ = plan(&odd, 12.0, &[(f64::NAN, 2.0)], ALL);
    }

    #[test]
    fn a_moving_game_wins_a_quiet_one() {
        // two equal pauses; the gameplay moves only around the second
        let mut v = Vec::new();
        for i in 0..80 {
            let a = i as f64 * 0.3;
            if (4.0..6.4).contains(&a) || (9.0..11.4).contains(&a) {
                continue;
            }
            v.push(w("word", a, a + 0.25, false));
        }
        let activity: Vec<(f64, f64)> = (0..24).map(|s| (s as f64, if (9..=11).contains(&s) { 1.0 } else { 0.0 })).collect();
        // under 20 s before the punchline: one moment, in the pause where the game moves
        let p = plan(&v, 18.0, &activity, ALL);
        let games: Vec<f64> = p.segments.iter().filter(|g| g.scene == Scene::Game).map(|g| g.start).collect();
        assert_eq!(games.len(), 1, "{p:?}");
        assert!((8.5..9.5).contains(&games[0]), "{games:?}");
    }

    #[test]
    fn regions_are_cut_to_shape_from_the_top() {
        let src = (1920.0, 1080.0);
        // a tall model region into a wide box: the top kept
        let r = top_anchored([0.7, 0.36, 1.0, 1.0], src, 1.125);
        let (w, h) = ((r[2] - r[0]) * src.0, (r[3] - r[1]) * src.1);
        assert!((w / h - 1.125).abs() < 1e-6 && r[1] >= 0.36 && r[1] < 0.4 && r[3] < 1.0, "{r:?}");
        // into a tall frame: the sides cut evenly
        let r = top_anchored([0.7, 0.36, 1.0, 1.0], src, 9.0 / 16.0);
        let (w, h) = ((r[2] - r[0]) * src.0, (r[3] - r[1]) * src.1);
        assert!((w / h - 0.5625).abs() < 1e-6 && ((r[0] + r[2]) / 2.0 - 0.85).abs() < 1e-9, "{r:?}");
        // nothing to cut
        assert_eq!(top_anchored([0.0, 0.0, 0.0, 0.0], src, 1.0), [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(top_anchored([0.1, 0.1, 0.5, 0.5], src, f64::NAN), [0.1, 0.1, 0.5, 0.5]);
    }
}
