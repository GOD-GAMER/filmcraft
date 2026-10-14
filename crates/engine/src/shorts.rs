//! Sequence ▸ Shorts: tools for vertical short-form video (YouTube Shorts, TikTok, Reels), built
//! with clips cut from streams in mind.
//!
//! - **Vertical Short** (Crop to Fill, Follow Subject, Blurred Background): a 1080 x 1920 copy of
//!   the active sequence with its footage reframed for the vertical frame. The original sequence
//!   is left as it was.
//! - **Stacked Layout** (model on the right or left): gameplay in the top pane and the streamer's
//!   model or camera in the bottom pane, both cut from the same recording with Crop and Motion.
//! - **Punch In**, **Camera Shake** and **Impact Flash** at the playhead.
//! - **Word Captions** (Pop, Karaoke, One Word): animated captions from the transcript, one
//!   graphic clip per spoken word on a new track, the word being spoken highlighted.
//!
//! Every command is one undo step and takes its options as parameters, so agents can drive it
//! too. The geometry and word timing live in [`filmcraft_edit::shorts`].

use std::collections::BTreeMap;

use filmcraft_edit::shorts::{self as sh, BeatRules, Placement, Rect};
use filmcraft_geom::Vec2;
use filmcraft_project::{
    CharStyle, ClipId, EffectInstance, ItemKind, Keyframe, LayerExtra, Param, ParamValue, Project, Sequence, StyleRun, Track, TrackId, TrackItem, TrackKind,
};

use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, bool_p, clips_p, f64_p, has_seq, str_p, time_p};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;

const MENU: &[&str] = &["Sequence", "Shorts"];

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu: MENU, shortcut: None, params, enabled: has_seq, run, journal: true }
}

const VERTICAL: &str = r#"{"width":u32=1080,"height":u32=1920,"name":str?}"#;
const STACK: &str = r#"{"clips":[id]?,"split":f=0.5,"model":[x0,y0,x1,y1]?,"game":[x0,y0,x1,y1]?}"#;
const CAPTIONS: &str = r##"{"maxWords":n=3,"maxChars":n?,"color":"#ffffff"?,"highlight":"#ffd60a"?,"font":"Inter"?,"fontStyle":"Bold"?,"size":f?,"position":[x,y]?,"uppercase":bool=true,"hold":seconds=0.25}"##;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        spec("shorts.verticalFill", "Vertical Short: Crop to Fill", VERTICAL, |s, p| vertical(s, p, Reframe::Fill, "shorts.verticalFill")),
        spec("shorts.verticalFollow", "Vertical Short: Follow Subject", VERTICAL, |s, p| vertical(s, p, Reframe::Follow, "shorts.verticalFollow")),
        spec("shorts.verticalBlur", "Vertical Short: Blurred Background", VERTICAL, |s, p| vertical(s, p, Reframe::Blur, "shorts.verticalBlur")),
        spec("shorts.stackRight", "Stacked Layout: Model on Right", STACK, |s, p| stacked(s, p, Side::Right, "shorts.stackRight")),
        spec("shorts.stackLeft", "Stacked Layout: Model on Left", STACK, |s, p| stacked(s, p, Side::Left, "shorts.stackLeft")),
        spec("shorts.punchIn", "Punch In at Playhead", r#"{"clips":[id]?,"time":ticks?,"zoom":percent=125,"seconds":f=1}"#, punch_in),
        spec("shorts.shake", "Camera Shake at Playhead", r#"{"clips":[id]?,"time":ticks?,"amount":px=40,"seconds":f=0.5}"#, shake),
        spec("shorts.flash", "Impact Flash at Playhead", r##"{"time":ticks?,"seconds":f=0.3,"color":"#ffffff"?}"##, flash),
        spec("shorts.captionsPop", "Word Captions: Pop", CAPTIONS, |s, p| word_captions(s, p, CapStyle::Pop, "shorts.captionsPop")),
        spec("shorts.captionsKaraoke", "Word Captions: Karaoke", CAPTIONS, |s, p| word_captions(s, p, CapStyle::Karaoke, "shorts.captionsKaraoke")),
        spec("shorts.captionsOneWord", "Word Captions: One Word", CAPTIONS, |s, p| word_captions(s, p, CapStyle::OneWord, "shorts.captionsOneWord")),
    ]
}

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

/// A clip that shows a picture of its own (media, a nested sequence, a subclip), as opposed to
/// a graphic or an adjustment layer.
fn is_footage(pr: &Project, it: &TrackItem) -> bool {
    pr.item(it.item).is_some_and(|i| matches!(i.kind, ItemKind::Media(_) | ItemKind::Sequence(_) | ItemKind::Subclip { .. }))
        && filmcraft_render::source_size(pr, it.item).is_some()
}

/// The picture sizes of a clip: storage pixels (for the anchor and crop) and display size in
/// sequence pixels (for the scale).
fn picture_sizes(pr: &Project, it: &TrackItem, par: (u32, u32)) -> Option<((f64, f64), (f64, f64))> {
    let (w, h) = filmcraft_render::source_size(pr, it.item)?;
    let (dw, dh) = pr.conformed_source_size(it.item, par)?;
    let ok = |v: f64| v.is_finite() && v >= 1.0;
    (w >= 1 && h >= 1 && ok(dw) && ok(dh)).then_some(((w as f64, h as f64), (dw, dh)))
}

/// Index of the lowest video track holding footage: the "main picture" track.
fn base_track(pr: &Project, q: &Sequence) -> Option<usize> {
    q.video_tracks.iter().position(|t| t.items.iter().any(|i| is_footage(pr, i)))
}

/// Set a parameter to a static value, dropping its keyframes (adds the parameter if missing).
fn set_static(e: &mut EffectInstance, k: &str, v: ParamValue) {
    match e.params.get_mut(k) {
        Some(prm) => {
            prm.keyframes.clear();
            prm.value = v;
        }
        None => {
            e.params.insert(k.to_string(), Param::new(v));
        }
    }
}

/// Motion that shows the clip with `scale` (percent) and the picture point `anchor` (storage
/// pixels) at frame point `position`. Turns Scale to Frame Size off.
fn set_motion(it: &mut TrackItem, scale: f64, position: (f64, f64), anchor: (f64, f64)) {
    it.scale_to_frame = false;
    if let Some(m) = it.effect_mut("motion") {
        set_static(m, "scale", ParamValue::Float(scale.clamp(0.0, 10_000.0)));
        set_static(m, "uniform_scale", ParamValue::Bool(true));
        set_static(m, "rotation", ParamValue::Float(0.0));
        set_static(m, "position", ParamValue::Vec2(Vec2::new(position.0, position.1)));
        set_static(m, "anchor", ParamValue::Vec2(Vec2::new(anchor.0, anchor.1)));
    }
}

/// Where standard (non-intrinsic) effects go: before Motion, Opacity and Time Remapping.
fn standard_slot(it: &TrackItem) -> usize {
    it.effects.iter().position(|e| e.def().is_some_and(|d| d.intrinsic)).unwrap_or(it.effects.len())
}

/// Put a new instance of effect `id` on the clip at `at` (clamped to the standard effects).
fn insert_effect(it: &mut TrackItem, id: &str, at: Option<usize>) -> Option<usize> {
    let def = filmcraft_project::find_effect(id)?;
    let slot = standard_slot(it);
    let at = at.unwrap_or(slot).min(slot);
    it.effects.insert(at, def.instance());
    Some(at)
}

/// Put a new instance of effect `id` at the end of the clip's standard effects, with `params` set.
fn add_effect(it: &mut TrackItem, id: &str, params: Vec<(&str, ParamValue)>) {
    if let Some(e) = insert_effect(it, id, None).and_then(|i| it.effects.get_mut(i)) {
        for (k, v) in params {
            set_static(e, k, v);
        }
    }
}

/// The clip's Crop effect (added at the end of its standard effects if it has none).
fn crop_mut(it: &mut TrackItem) -> Option<&mut EffectInstance> {
    let idx = match it.effects.iter().position(|e| e.effect == "crop") {
        Some(i) => i,
        None => insert_effect(it, "crop", None)?,
    };
    it.effects.get_mut(idx)
}

fn set_crop(it: &mut TrackItem, crop: [f64; 4]) {
    if let Some(c) = crop_mut(it) {
        for (k, v) in ["left", "top", "right", "bottom"].into_iter().zip(crop) {
            set_static(c, k, ParamValue::Float(v));
        }
        set_static(c, "zoom", ParamValue::Bool(false));
    }
}

/// Apply a placement computed on the display size: the anchor goes back to storage pixels.
fn apply_placement(it: &mut TrackItem, pl: &Placement, storage: (f64, f64), display: (f64, f64)) {
    let anchor = (pl.anchor.0 * storage.0 / display.0, pl.anchor.1 * storage.1 / display.1);
    set_motion(it, pl.scale, pl.position, anchor);
    set_crop(it, pl.crop);
    it.effects.retain(|e| e.effect != "auto_reframe");
}

/// A copy of a clip for another track: new id, no links or group, `suffix` added to its name.
fn copy_clip(it: &TrackItem, id: u64, suffix: &str) -> TrackItem {
    let mut c = it.clone();
    c.id = ClipId(id);
    c.link = None;
    c.group = None;
    c.name = format!("{}{suffix}", it.name);
    c
}

fn new_video_track(id: u64, name: &str) -> Track {
    Track::new(TrackId(id), TrackKind::Video, name.to_string())
}

/// A colour parameter: `"#rrggbb"`, `"#rrggbbaa"` or `[r, g, b(, a)]` (0..255), as 0..1 floats.
fn color_p(p: &Value, k: &str, cmd: &str) -> Result<Option<[f32; 4]>> {
    let Some(v) = p.get(k) else { return Ok(None) };
    let rgba: Option<[u8; 4]> = if let Some(a) = v.as_array() {
        let c: Vec<u8> = a.iter().filter_map(|x| x.as_f64().filter(|f| f.is_finite()).map(|f| f.clamp(0.0, 255.0) as u8)).collect();
        match c.as_slice() {
            [r, g, b] => Some([*r, *g, *b, 255]),
            [r, g, b, a] => Some([*r, *g, *b, *a]),
            _ => None,
        }
    } else {
        v.as_str().and_then(|s| {
            let h = s.trim().trim_start_matches('#');
            let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
            match h.len() {
                6 => Some([b(0)?, b(2)?, b(4)?, 255]),
                8 => Some([b(0)?, b(2)?, b(4)?, b(6)?]),
                _ => None,
            }
        })
    };
    let c = rgba.ok_or_else(|| bad(cmd, format!("`{k}` must be a colour like \"#ffd60a\"")))?;
    Ok(Some(c.map(|x| f32::from(x) / 255.0)))
}

/// A finite number parameter within `lo..=hi` (default when missing).
fn num_p(p: &Value, k: &str, default: f64, lo: f64, hi: f64, cmd: &str) -> Result<f64> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(_) => match f64_p(p, k) {
            Some(v) if v.is_finite() && v >= lo && v <= hi => Ok(v),
            _ => Err(bad(cmd, format!("`{k}` must be a number from {lo} to {hi}"))),
        },
    }
}

/// Fractions `[x0, y0, x1, y1]` (each 0..1) from a parameter.
fn fractions_p(p: &Value, k: &str, cmd: &str) -> Result<Option<[f64; 4]>> {
    let Some(v) = p.get(k) else { return Ok(None) };
    let a = v.as_array().filter(|a| a.len() == 4).ok_or_else(|| bad(cmd, format!("`{k}` must be [x0, y0, x1, y1] fractions of the picture")))?;
    let mut f = [0.0; 4];
    for (o, x) in f.iter_mut().zip(a) {
        *o = x.as_f64().filter(|v| v.is_finite() && (0.0..=1.0).contains(v)).ok_or_else(|| bad(cmd, format!("`{k}` values must be from 0 to 1")))?;
    }
    if f[2] - f[0] < 0.01 || f[3] - f[1] < 0.01 {
        return Err(bad(cmd, format!("`{k}` must have x1 > x0 and y1 > y0")));
    }
    Ok(Some(f))
}

// ---------------------------------------------------------------------------------------------
// Vertical Short
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reframe {
    /// Scale the footage up until it covers the frame; the sides are cut off.
    Fill,
    /// Auto Reframe: the window follows the most interesting part of the picture.
    Follow,
    /// The whole picture fits in the middle, over a blurred, zoomed-in copy of itself.
    Blur,
}

fn vertical(s: &mut Session, p: &Value, mode: Reframe, cmd: &str) -> Result<Value> {
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let src = s.project.sequence(seq_id).ok_or(EngineError::NoSequence)?;
    let w = num_p(p, "width", 1080.0, 16.0, 16384.0, cmd)?.round() as u32;
    let h = num_p(p, "height", 1920.0, 16.0, 16384.0, cmd)?.round() as u32;
    let mut settings = src.settings.clone();
    let from = (settings.width, settings.height);
    settings.width = w;
    settings.height = h;
    settings.preset = format!("Vertical {w}x{h} {}", settings.frame_rate.label());
    settings.validate().map_err(|e| bad(cmd, e))?;
    let par = settings.par;
    let base = base_track(&s.project, src).ok_or_else(|| bad(cmd, "the sequence has no video footage to reframe"))?;
    let sizes: BTreeMap<ClipId, ((f64, f64), (f64, f64))> = src
        .video_tracks
        .get(base)
        .map(|t| t.items.iter().filter(|i| is_footage(&s.project, i)).filter_map(|i| Some((i.id, picture_sizes(&s.project, i, par)?))).collect())
        .unwrap_or_default();
    let base_name = s.project.item(seq_id).map(|i| i.name.clone()).unwrap_or_else(|| "Sequence".into());
    let name = str_p(p, "name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| format!("{base_name} (Vertical)"));
    let seq_label = s.prefs.labels.defaults.sequence;
    let label = match mode {
        Reframe::Fill => "Vertical Short: Crop to Fill",
        Reframe::Follow => "Vertical Short: Follow Subject",
        Reframe::Blur => "Vertical Short: Blurred Background",
    };
    let (fw, fh) = (f64::from(w), f64::from(h));
    let follow_aspect = match mode {
        Reframe::Follow => follow_aspect(fw / fh).ok_or_else(|| bad(cmd, "Follow Subject is for vertical or square frames"))?,
        _ => 0,
    };
    let (id, n) = s.edit(label, |pr, st| {
        let mut q = pr.sequence(seq_id).cloned().ok_or(EngineError::NoSequence)?;
        q.settings = settings.clone();
        // everything keeps its place relative to the frame (titles, overlays, picture-in-picture)
        filmcraft_edit::frame_size::scale_motion(&mut q, from, (w, h));
        // the main picture is reframed
        let mut background: Vec<TrackItem> = Vec::new();
        let mut n = 0usize;
        if let Some(track) = q.video_tracks.get_mut(base) {
            for it in track.items.iter_mut() {
                let Some(&(storage, display)) = sizes.get(&it.id) else { continue };
                let centre = (fw / 2.0, fh / 2.0);
                let anchor = (storage.0 / 2.0, storage.1 / 2.0);
                it.effects.retain(|e| e.effect != "auto_reframe");
                match mode {
                    Reframe::Fill => {
                        let k = sh::frame_scale(display, (fw, fh), true).unwrap_or(100.0);
                        set_motion(it, k, centre, anchor);
                    }
                    Reframe::Follow => {
                        // Auto Reframe moves the picture inside the clip's own frame, so Motion
                        // fills the sequence (as Crop to Fill) and the effect, set to a window at
                        // least as wide as the frame, pans the subject into the middle of it
                        let k = sh::frame_scale(display, (fw, fh), true).unwrap_or(100.0);
                        set_motion(it, k, centre, anchor);
                        add_effect(it, "auto_reframe", vec![("aspect", ParamValue::Choice(follow_aspect))]);
                    }
                    Reframe::Blur => {
                        let fit = sh::frame_scale(display, (fw, fh), false).unwrap_or(100.0);
                        let fill = sh::frame_scale(display, (fw, fh), true).unwrap_or(100.0);
                        set_motion(it, fit, centre, anchor);
                        let mut bg = copy_clip(it, pr.alloc_id(), " (Blurred)");
                        set_motion(&mut bg, fill * 1.08, centre, anchor);
                        let blur = ParamValue::Float((fh / 1920.0 * 90.0).max(10.0));
                        add_effect(&mut bg, "gaussian_blur", vec![("blurriness", blur), ("repeat_edge", ParamValue::Bool(true))]);
                        add_effect(&mut bg, "brightness_contrast", vec![("brightness", ParamValue::Float(-25.0))]);
                        background.push(bg);
                    }
                }
                n += 1;
            }
        }
        if !background.is_empty() {
            let mut t = new_video_track(pr.alloc_id(), "Blurred Background");
            t.items = background;
            t.sort();
            q.video_tracks.insert(base, t);
        }
        q.check().map_err(EngineError::Other)?;
        let id = pr.add_item(&name, seq_label, ItemKind::Sequence(std::sync::Arc::new(q)), None);
        st.active_sequence = Some(id);
        if !st.open_sequences.contains(&id) {
            st.open_sequences.push(id);
        }
        st.selection.clear();
        Ok((id, n))
    })?;
    s.events.push(crate::Event::OpenSequence(id));
    Ok(json!({"sequence": id.0, "reframed": n, "width": w, "height": h}))
}

/// Auto Reframe ▸ Target Aspect for a frame of aspect `ratio` (width / height): the narrowest
/// choice that is at least as wide as the frame, so the window it keeps on the subject always
/// covers what the frame shows. `None` for frames wider than 16:9.
fn follow_aspect(ratio: f64) -> Option<u32> {
    // (choice index, aspect) in the effect's Target Aspect list
    const CHOICES: [(u32, f64); 4] = [(1, 9.0 / 16.0), (3, 4.0 / 5.0), (2, 1.0), (4, 16.0 / 9.0)];
    if !ratio.is_finite() {
        return None;
    }
    CHOICES.iter().find(|(_, r)| *r >= ratio - 1e-6).map(|(i, _)| *i)
}

// ---------------------------------------------------------------------------------------------
// Stacked layout
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Right,
    Left,
}

/// Selected footage clips, or else every footage clip on the main picture track.
fn layout_targets(s: &Session, p: &Value) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let picked: Vec<ClipId> =
        clips_p(s, p).into_iter().filter(|c| q.video_tracks.iter().any(|t| t.item(*c).is_some_and(|i| is_footage(&s.project, i)))).collect();
    if !picked.is_empty() {
        return picked;
    }
    base_track(&s.project, q)
        .and_then(|b| q.video_tracks.get(b))
        .map(|t| t.items.iter().filter(|i| is_footage(&s.project, i)).map(|i| i.id).collect())
        .unwrap_or_default()
}

fn stacked(s: &mut Session, p: &Value, side: Side, cmd: &str) -> Result<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (fw, fh) = (f64::from(q.settings.width), f64::from(q.settings.height));
    let par = q.settings.par;
    let split = num_p(p, "split", 0.5, 0.2, 0.8, cmd)?;
    // where streamers usually sit (a corner-ish model), and the gameplay beside it
    let model = fractions_p(p, "model", cmd)?.unwrap_or(match side {
        Side::Right => [0.60, 0.25, 1.0, 1.0],
        Side::Left => [0.0, 0.25, 0.40, 1.0],
    });
    let game = fractions_p(p, "game", cmd)?.unwrap_or(match side {
        Side::Right => [0.0, 0.0, 0.62, 1.0],
        Side::Left => [0.38, 0.0, 1.0, 1.0],
    });
    let top = Rect::new(0.0, 0.0, fw, (fh * split).round());
    let bottom = Rect::new(0.0, top.h, fw, fh - top.h);
    let targets = layout_targets(s, p);
    // (clip, track index, storage size, display size)
    let plans: Vec<(ClipId, usize, (f64, f64), (f64, f64))> = targets
        .iter()
        .filter_map(|c| {
            let (ti, it) = q.video_tracks.iter().enumerate().find_map(|(ti, t)| t.item(*c).map(|i| (ti, i)))?;
            let (storage, display) = picture_sizes(&s.project, it, par)?;
            Some((*c, ti, storage, display))
        })
        .collect();
    if plans.is_empty() {
        return Err(bad(cmd, "select the stream clip (or put footage on the timeline) first"));
    }
    let above = plans.iter().map(|x| x.1).max().unwrap_or(0).saturating_add(1);
    let label = match side {
        Side::Right => "Stacked Layout: Model on Right",
        Side::Left => "Stacked Layout: Model on Left",
    };
    let out = s.edit_sequence(label, |q, ctx, st| {
        let mut copies = Vec::new();
        let mut out = Vec::new();
        for (c, ti, storage, display) in &plans {
            let (Some(game_pl), Some(model_pl)) = (
                sh::place_region(*display, Rect::from_fractions(game, display.0, display.1), top),
                sh::place_region(*display, Rect::from_fractions(model, display.0, display.1), bottom),
            ) else {
                continue;
            };
            let Some(it) = q.video_tracks.get_mut(*ti).and_then(|t| t.items.iter_mut().find(|i| i.id == *c)) else { continue };
            apply_placement(it, &game_pl, *storage, *display);
            let mut m = copy_clip(it, ctx.alloc(), " (Model)");
            apply_placement(&mut m, &model_pl, *storage, *display);
            out.push(json!({"clip": c.0, "model": m.id.0}));
            copies.push(m);
        }
        if copies.is_empty() {
            return Err(bad(cmd, "the selected clips have no picture to lay out"));
        }
        let mut t = new_video_track(ctx.alloc(), "Model");
        t.items = copies;
        t.sort();
        let at = above.min(q.video_tracks.len());
        q.video_tracks.insert(at, t);
        st.selection = out.iter().filter_map(|o| o["model"].as_u64().map(ClipId)).collect();
        Ok(json!({"clips": out, "track": at}))
    })?;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Punch In, Camera Shake, Impact Flash
// ---------------------------------------------------------------------------------------------

/// Clips the punch effects apply to at sequence time `t`: the selected video clips under it,
/// or else every footage clip under it.
fn clips_at(s: &Session, p: &Value, t: Tick) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let covers = |c: &ClipId| q.video_tracks.iter().any(|tr| tr.item(*c).is_some_and(|i| i.range().contains(t)));
    let picked: Vec<ClipId> = clips_p(s, p).into_iter().filter(covers).collect();
    if !picked.is_empty() {
        return picked;
    }
    q.video_tracks.iter().flat_map(|tr| tr.items.iter()).filter(|i| i.range().contains(t) && is_footage(&s.project, i)).map(|i| i.id).collect()
}

/// `t` plus `frames` frames of `rate`.
fn after(t: Tick, frames: i64, frame: Tick) -> Tick {
    Tick(t.0.saturating_add(frame.0.saturating_mul(frames)))
}

fn put(prm: &mut Param, t: Tick, v: f64) {
    prm.put_keyframe(t, ParamValue::Float(v));
}

fn punch_in(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.punchIn";
    let t = time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let zoom = num_p(p, "zoom", 125.0, 101.0, 400.0, CMD)? / 100.0;
    let hold = Tick::from_seconds_f64(num_p(p, "seconds", 1.0, 0.1, 60.0, CMD)?);
    let clips = clips_at(s, p, t);
    if clips.is_empty() {
        return Err(bad(CMD, "move the playhead over a video clip"));
    }
    let sizes: BTreeMap<ClipId, (f64, f64)> = s
        .active_sequence()
        .map(|q| {
            q.video_tracks
                .iter()
                .flat_map(|tr| tr.items.iter())
                .filter(|i| clips.contains(&i.id))
                .filter_map(|i| filmcraft_render::source_size(&s.project, i.item).map(|(w, h)| (i.id, (f64::from(w), f64::from(h)))))
                .collect()
        })
        .unwrap_or_default();
    let n = s.edit_sequence("Punch In", |q, _, _| {
        let frame = q.settings.frame_rate.frame_duration().max(Tick(1));
        let t = q.settings.frame_rate.snap(t);
        let mut n = 0;
        for tr in q.video_tracks.iter_mut() {
            for it in tr.items.iter_mut().filter(|i| clips.contains(&i.id)) {
                let last = Tick(it.end().0.saturating_sub(1));
                // snap in over 3 frames, hold, ease back over 4 (all inside the clip)
                let seq_times = [t, after(t, 3, frame).min(last), Tick(after(t, 3, frame).0.saturating_add(hold.0)).min(last)];
                let seq_times = [seq_times[0], seq_times[1], seq_times[2], after(seq_times[2], 4, frame).min(last)];
                let mt = seq_times.map(|x| it.effect_time_at(x));
                let anchor = it.effect("motion").and_then(|m| m.params.get("anchor")).map(|a| a.vec2_at(mt[0]));
                let Some(m) = it.effect_mut("motion") else { continue };
                let Some(scale) = m.params.get_mut("scale") else { continue };
                let base = mt.map(|x| scale.f64_at(x));
                put(scale, mt[0], base[0]);
                put(scale, mt[1], base[1] * zoom);
                put(scale, mt[2], base[2] * zoom);
                put(scale, mt[3], base[3]);
                // a cropped pane (stacked layout) zooms inside its own edges
                let size = sizes.get(&it.id).copied();
                if let (Some(a), Some((w, h))) = (anchor, size)
                    && let Some(c) = it.effects.iter_mut().find(|e| e.effect == "crop")
                {
                    let keys = ["left", "top", "right", "bottom"];
                    let crop = keys.map(|k| c.params.get(k).map_or(0.0, |prm| prm.f64_at(mt[0])));
                    let animated = keys.iter().any(|k| c.params.get(*k).is_some_and(Param::is_animated));
                    if !animated && crop.iter().any(|v| *v > 0.0) {
                        let zoomed = sh::zoomed_crop(crop, (a.x / w * 100.0, a.y / h * 100.0), zoom);
                        for ((k, c0), cz) in keys.into_iter().zip(crop).zip(zoomed) {
                            if let Some(prm) = c.params.get_mut(k) {
                                put(prm, mt[0], c0);
                                put(prm, mt[1], cz);
                                put(prm, mt[2], cz);
                                put(prm, mt[3], c0);
                            }
                        }
                    }
                }
                n += 1;
            }
        }
        Ok(n)
    })?;
    Ok(json!({"clips": n}))
}

fn shake(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.shake";
    let t = time_p(s, p, "").unwrap_or_else(|| s.playhead());
    let amount = num_p(p, "amount", 40.0, 1.0, 1000.0, CMD)?;
    let dur = Tick::from_seconds_f64(num_p(p, "seconds", 0.5, 0.05, 30.0, CMD)?);
    let clips = clips_at(s, p, t);
    if clips.is_empty() {
        return Err(bad(CMD, "move the playhead over a video clip"));
    }
    let n = s.edit_sequence("Camera Shake", |q, _, _| {
        let frame = q.settings.frame_rate.frame_duration().max(Tick(1));
        let t = q.settings.frame_rate.snap(t);
        let mut n = 0;
        for tr in q.video_tracks.iter_mut() {
            for it in tr.items.iter_mut().filter(|i| clips.contains(&i.id)) {
                let last = Tick(it.end().0.saturating_sub(1));
                let before = (t > it.start).then(|| it.effect_time_at(Tick(t.0.saturating_sub(frame.0))));
                let (m0, m1) = (it.effect_time_at(t), it.effect_time_at(Tick(t.0.saturating_add(dur.0)).min(last)));
                // the shake moves the picture before any crop, so a stacked pane keeps its edges
                let idx = match it.effects.iter().position(|e| e.effect == "camera_shake") {
                    Some(i) => i,
                    None => {
                        let Some(i) = insert_effect(it, "camera_shake", Some(0)) else { continue };
                        if let Some(e) = it.effects.get_mut(i) {
                            for (k, v) in [("amount", 0.0), ("rotation", 0.0), ("zoom", 0.0), ("frequency", 14.0), ("motion_blur", 40.0)] {
                                set_static(e, k, ParamValue::Float(v));
                            }
                        }
                        i
                    }
                };
                let Some(e) = it.effects.get_mut(idx) else { continue };
                for (k, peak) in [("amount", amount), ("rotation", (amount / 20.0).min(10.0)), ("zoom", (amount / 6.0).min(30.0))] {
                    let Some(prm) = e.params.get_mut(k) else { continue };
                    if let Some(b) = before {
                        put(prm, b, 0.0);
                    }
                    put(prm, m0, peak);
                    put(prm, m1, 0.0);
                }
                n += 1;
            }
        }
        Ok(n)
    })?;
    Ok(json!({"clips": n}))
}

fn flash(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.flash";
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let secs = num_p(p, "seconds", 0.3, 0.05, 10.0, CMD)?;
    let color = color_p(p, "color", CMD)?.unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let dur = rate.snap_nearest(Tick::from_seconds_f64(secs)).max(rate.frame_duration());
    let (fw, fh) = (f64::from(w), f64::from(h));
    let mut layer = filmcraft_project::graphic::new_shape_layer(0, Vec2::new(fw / 2.0, fh / 2.0), Vec2::new(fw + 8.0, fh + 8.0), Vec::new());
    set_static(&mut layer, "name", ParamValue::Text("Flash".into()));
    set_static(&mut layer, "fill", ParamValue::Bool(true));
    set_static(&mut layer, "fill_color", ParamValue::Color(color));
    set_static(&mut layer, "stroke", ParamValue::Bool(false));
    // graphic clips start at media time 0, so keyframe times are clip-relative
    let mut fade = Param::new(ParamValue::Float(90.0));
    fade.keyframes = vec![
        Keyframe::new(Tick::ZERO, ParamValue::Float(90.0)),
        Keyframe { interp: filmcraft_project::Interpolation::EaseOut, ..Keyframe::new(Tick(dur.0 / 3), ParamValue::Float(55.0)) },
        Keyframe::new(dur, ParamValue::Float(0.0)),
    ];
    layer.params.insert("opacity".into(), fade);
    let mut placement = json!({});
    if let Some(t) = p.get("time") {
        placement["time"] = t.clone();
    }
    let id = crate::graphics::place_video_clip(s, CMD, "Flash", &placement, "Impact Flash", vec![layer], move |pr, (w, h, rate)| {
        (crate::graphics::graphic_source(pr, w, h, rate), dur)
    })?;
    Ok(json!({"clip": id.0}))
}

// ---------------------------------------------------------------------------------------------
// Word captions
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum CapStyle {
    /// A few words at a time; the word being spoken changes colour and grows a little.
    Pop,
    /// A few words at a time; words light up as they are spoken.
    Karaoke,
    /// One big word at a time, each popping in.
    OneWord,
}

/// Approximate width of bold capitals, in ems, used to keep a caption inside the frame.
const EM_PER_CHAR: f64 = 0.68;

fn word_captions(s: &mut Session, p: &Value, style: CapStyle, cmd: &str) -> Result<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let vertical = fh > fw;
    let words = crate::transcript::sequence_words(s);
    if words.is_empty() {
        return Err(bad(cmd, "no transcript yet: run Sequence ▸ Transcript ▸ Transcribe… first so FilmCraft knows when each word is spoken"));
    }
    let max_words = match style {
        CapStyle::OneWord => 1,
        _ => num_p(p, "maxWords", 3.0, 1.0, 12.0, cmd)?.round() as usize,
    };
    let max_chars = num_p(p, "maxChars", if vertical { 16.0 } else { 28.0 }, 1.0, 200.0, cmd)?.round() as usize;
    let rules = BeatRules { max_words, max_chars, ..BeatRules::default() };
    let hold = Tick::from_seconds_f64(num_p(p, "hold", 0.25, 0.0, 5.0, cmd)?);
    let fill = color_p(p, "color", cmd)?.unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let highlight = color_p(p, "highlight", cmd)?.unwrap_or(match style {
        CapStyle::Karaoke => [0.24, 1.0, 0.43, 1.0],
        _ => [1.0, 0.84, 0.04, 1.0],
    });
    let font = str_p(p, "font").map(str::trim).filter(|f| !f.is_empty()).unwrap_or("Inter").to_string();
    let font_style = str_p(p, "fontStyle").map(str::trim).filter(|f| !f.is_empty()).unwrap_or("Bold").to_string();
    let default_size = if vertical { fw * 0.085 } else { fh * 0.075 } * if style == CapStyle::OneWord { 1.4 } else { 1.0 };
    let size = num_p(p, "size", default_size, 4.0, 2000.0, cmd)?;
    let uppercase = bool_p(p, "uppercase").unwrap_or(true);
    let position = match p.get("position") {
        None => Vec2::new(fw / 2.0, fh * if vertical { 0.70 } else { 0.86 }),
        Some(_) => crate::graphics::vec2_p(p, "position")
            .filter(|v| v.x.is_finite() && v.y.is_finite())
            .ok_or_else(|| bad(cmd, "`position` must be [x, y] in frame pixels"))?,
    };

    let beats = sh::word_beats(&words, &rules);
    let spans = sh::caption_spans(&words, &beats, hold, rate);
    if spans.is_empty() {
        return Err(bad(cmd, "the transcript's words are too short to show"));
    }
    if spans.len() > 20_000 {
        return Err(bad(cmd, "too many words for one caption track (more than 20,000); caption a shorter sequence"));
    }
    let frame = rate.frame_duration().max(Tick(1));
    // one text layer per span
    let mut clips: Vec<(Tick, Tick, String, EffectInstance)> = Vec::with_capacity(spans.len());
    let mut prev_beat = None;
    for sp in &spans {
        let Some(beat) = beats.get(sp.beat).cloned() else { continue };
        let (text, ranges) = match style {
            CapStyle::OneWord => sh::beat_text(&words, sp.word..sp.word.saturating_add(1), uppercase),
            _ => sh::beat_text(&words, beat.clone(), uppercase),
        };
        let chars = text.chars().count().max(1) as f64;
        let fitted = size.min(fw * 0.92 / (chars * EM_PER_CHAR)).max(4.0);
        let mut layer = filmcraft_project::graphic::new_text_layer(&text, position, fitted);
        for (k, v) in [
            ("name", ParamValue::Text(text.clone())),
            ("font", ParamValue::Text(font.clone())),
            ("font_style", ParamValue::Text(font_style.clone())),
            ("align", ParamValue::Choice(1)),
            ("fill", ParamValue::Bool(true)),
            ("fill_color", ParamValue::Color(if style == CapStyle::OneWord { highlight_or_white(sp.word, fill, highlight) } else { fill })),
            ("stroke", ParamValue::Bool(true)),
            ("stroke_color", ParamValue::Color([0.0, 0.0, 0.0, 1.0])),
            ("stroke_width", ParamValue::Float(fitted * 0.14)),
            ("stroke_type", ParamValue::Choice(0)),
            ("shadow", ParamValue::Bool(true)),
            ("shadow_opacity", ParamValue::Float(60.0)),
            ("shadow_distance", ParamValue::Float(fitted * 0.06)),
            ("shadow_size", ParamValue::Float(fitted * 0.05)),
            ("shadow_blur", ParamValue::Float(fitted * 0.25)),
        ] {
            set_static(&mut layer, k, v);
        }
        // the word being spoken (and, for karaoke, the words before it)
        let k = sp.word.saturating_sub(beat.start);
        let lit: Vec<std::ops::Range<usize>> = match style {
            CapStyle::Pop => ranges.get(k).cloned().into_iter().collect(),
            CapStyle::Karaoke => ranges.iter().take(k.saturating_add(1)).cloned().collect(),
            CapStyle::OneWord => Vec::new(),
        };
        let grow = if style == CapStyle::Pop { Some((fitted * 1.12) as f32) } else { None };
        let runs: Vec<StyleRun> = lit
            .into_iter()
            .map(|r| StyleRun { start: r.start, end: r.end, style: CharStyle { fill: Some(highlight), size: grow, ..CharStyle::default() } })
            .collect();
        if !runs.is_empty() {
            layer.layer = Some(Box::new(LayerExtra { runs, ..LayerExtra::default() }));
        }
        // pop in when a new phrase (or, for One Word, a new word) appears
        let new_phrase = prev_beat != Some(sp.beat) || style == CapStyle::OneWord;
        prev_beat = Some(sp.beat);
        if new_phrase {
            let from = if style == CapStyle::OneWord { 60.0 } else { 75.0 };
            let mut scale = Param::new(ParamValue::Float(100.0));
            scale.keyframes = vec![
                Keyframe::new(Tick::ZERO, ParamValue::Float(from)),
                Keyframe::new(after(Tick::ZERO, 2, frame), ParamValue::Float(112.0)),
                Keyframe::new(after(Tick::ZERO, 4, frame), ParamValue::Float(100.0)),
            ];
            layer.params.insert("scale".into(), scale);
        }
        clips.push((sp.start, Tick(sp.end.0.saturating_sub(sp.start.0)), text, layer));
    }
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let label = match style {
        CapStyle::Pop => "Word Captions: Pop",
        CapStyle::Karaoke => "Word Captions: Karaoke",
        CapStyle::OneWord => "Word Captions: One Word",
    };
    let n = clips.len();
    let track = s.edit(label, |pr, st| {
        let src = crate::graphics::graphic_source(pr, w, h, rate);
        let mut items = Vec::with_capacity(clips.len());
        for (start, dur, text, layer) in clips {
            let Some(mut ti) = pr.make_track_item(src, TrackKind::Video, start, TimeRange::new(Tick::ZERO, dur), rate) else { continue };
            ti.name = text;
            let mut layer = layer;
            filmcraft_project::resolve_auto_points(&mut layer, (w, h), (w, h));
            ti.effects.push(layer);
            items.push(ti);
        }
        let mut t = new_video_track(pr.alloc_id(), "Word Captions");
        t.items = items;
        t.sort();
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        q.video_tracks.push(t);
        q.check().map_err(EngineError::Other)?;
        st.selection.clear();
        Ok(q.video_tracks.len().saturating_sub(1))
    })?;
    Ok(json!({"clips": n, "beats": beats.len(), "track": track}))
}

/// One Word style alternates white and the highlight colour so consecutive words read apart.
fn highlight_or_white(word: usize, fill: [f32; 4], highlight: [f32; 4]) -> [f32; 4] {
    if word % 3 == 2 { highlight } else { fill }
}
