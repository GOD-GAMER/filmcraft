//! Sequence ▸ Shorts ▸ Layout…: arranges a widescreen stream recording in a vertical frame.
//!
//! Three pieces, each adjustable:
//! - the **gameplay pane**: a window on the gameplay (by default everything beside the model),
//!   full frame width, as tall as the part of the gameplay it shows (`gameWidth` = how much of
//!   the gameplay's width is kept: 1 keeps all of it, smaller zooms in and makes the pane taller);
//! - the **model box**: the streamer's model (or camera) cut from the same recording, in a box of
//!   its own size and position, with rounded corners and an optional border;
//! - a **background**: a blurred, darkened copy of the recording behind both (or black).
//!
//! Presets set every value at once: `stack` (gameplay with the model below it), `split` (two
//! full-width panes, the model pane under the gameplay) and `pip` (taller gameplay with the model
//! in a corner over it). Applying again replaces the previous layout of the same clips, so the
//! Layout… dialog can re-apply live while sliders move.

use filmcraft_edit::shorts::{self as sh, Placement, Rect};
use filmcraft_project::{ClipId, ParamValue, TrackItem};
use serde_json::{Value, json};

use crate::commands::{bad, clips_p, str_p};
use crate::shorts::{add_effect, base_track, color_p, copy_clip, insert_effect, is_footage, new_video_track, num_p, picture_sizes, set_motion, set_static};
use crate::{EngineError, Result, Session};

/// Suffixes of the copies a layout makes (found again, and replaced, when it is re-applied).
pub(crate) const MODEL_SUFFIX: &str = " (Model)";
pub(crate) const BACKGROUND_SUFFIX: &str = " (Background)";
/// Track names a layout (or Vertical Short: Blurred Background) puts its copies on.
const MODEL_TRACK: &str = "Model";
const BACKGROUND_TRACK: &str = "Layout Background";
const OLD_BACKGROUND_TRACK: &str = "Blurred Background";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Preset {
    Stack,
    Split,
    Pip,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Right,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Background {
    Blur,
    Black,
}

/// Every value of a layout, in fractions of the recording (regions) or of the frame (boxes).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Spec {
    pub preset: Preset,
    pub side: Side,
    /// The gameplay part of the recording: `[x0, y0, x1, y1]`.
    pub game: [f64; 4],
    /// How much of the gameplay's width the pane keeps (0.3 … 1).
    pub game_width: f64,
    /// Where in the gameplay the kept window sits: −1 left edge … 0 centre … 1 right edge.
    pub game_pan: f64,
    /// The pane's centre, as a fraction of the frame height.
    pub game_y: f64,
    /// The model part of the recording: `[x0, y0, x1, y1]`.
    pub model: [f64; 4],
    /// The model box: width and height (fractions of the frame), centre (fractions).
    pub model_w: f64,
    pub model_h: f64,
    pub model_x: f64,
    pub model_y: f64,
    /// Corner radius and border width of the boxes, frame pixels at 1080 wide.
    pub corner: f64,
    pub game_corner: f64,
    pub border: f64,
    pub border_color: [f32; 4],
    background: Background,
}

/// The model's place in a typical stream layout: the outer 30 % of the width, from a third of
/// the way down (a VTuber model or a camera in a bottom corner, under a chat window).
fn model_region(side: Side) -> [f64; 4] {
    match side {
        Side::Right => [0.70, 0.36, 1.0, 1.0],
        Side::Left => [0.0, 0.36, 0.30, 1.0],
    }
}

/// The gameplay beside the model.
fn game_region(side: Side) -> [f64; 4] {
    match side {
        Side::Right => [0.0, 0.0, 0.70, 1.0],
        Side::Left => [0.30, 0.0, 1.0, 1.0],
    }
}

impl Spec {
    /// The preset's values for a `frame`-sized vertical frame and a `src`-sized recording.
    pub(crate) fn preset(preset: Preset, side: Side, frame: (f64, f64), src: (f64, f64)) -> Spec {
        let (fw, fh) = frame;
        let model = model_region(side);
        let game = game_region(side);
        let (gw, gh) = ((game[2] - game[0]) * src.0, (game[3] - game[1]) * src.1);
        let (mw, mh) = ((model[2] - model[0]) * src.0, (model[3] - model[1]) * src.1);
        let model_aspect = if mh > 0.0 { mw / mh } else { 0.7 };
        let corner_x = |w: f64| match side {
            Side::Right => 1.0 - w / 2.0 - 0.04,
            Side::Left => w / 2.0 + 0.04,
        };
        match preset {
            Preset::Stack => {
                // all of the gameplay, full width; the model below it in a box of its own shape,
                // a third of the frame tall; the two centred together
                let pane_h = (fw * gh / gw.max(1.0) / fh).min(0.6);
                let model_h = 0.34;
                let model_w = (model_h * fh * model_aspect / fw).min(0.9);
                let gap = 0.015;
                let top = ((1.0 - (pane_h + gap + model_h)) / 2.0).max(0.0);
                Spec {
                    preset,
                    side,
                    game,
                    game_width: 1.0,
                    game_pan: 0.0,
                    game_y: top + pane_h / 2.0,
                    model,
                    model_w,
                    model_h,
                    model_x: 0.5,
                    model_y: (top + pane_h + gap + model_h / 2.0).min(1.0 - model_h / 2.0),
                    corner: 28.0,
                    game_corner: 0.0,
                    border: 0.0,
                    border_color: [1.0, 1.0, 1.0, 1.0],
                    background: Background::Blur,
                }
            }
            Preset::Split => {
                // gameplay on the top 60 %, the model across the bottom 40 %
                let split = 0.6;
                let game_width = (fw * gh / (split * fh * gw.max(1.0))).clamp(0.3, 1.0);
                Spec {
                    preset,
                    side,
                    game,
                    game_width,
                    game_pan: 0.0,
                    game_y: split / 2.0,
                    model,
                    model_w: 1.0,
                    model_h: 1.0 - split,
                    model_x: 0.5,
                    model_y: split + (1.0 - split) / 2.0,
                    corner: 0.0,
                    game_corner: 0.0,
                    border: 0.0,
                    border_color: [1.0, 1.0, 1.0, 1.0],
                    background: Background::Blur,
                }
            }
            Preset::Pip => {
                // the gameplay larger (62 % of the height); the model in a bordered box over its
                // lower corner, half in and half out
                let pane = 0.62;
                let game_width = (fw * gh / (pane * fh * gw.max(1.0))).clamp(0.3, 1.0);
                let model_h = 0.3;
                let model_w = (model_h * fh * model_aspect / fw).min(0.6);
                let game_y = 0.45;
                Spec {
                    preset,
                    side,
                    game,
                    game_width,
                    game_pan: 0.0,
                    game_y,
                    model,
                    model_w,
                    model_h,
                    model_x: corner_x(model_w),
                    model_y: game_y + pane / 2.0,
                    corner: 32.0,
                    game_corner: 0.0,
                    border: 6.0,
                    border_color: [1.0, 1.0, 1.0, 1.0],
                    background: Background::Blur,
                }
            }
        }
    }

    /// The preset (from `preset` and `side`) with every other parameter of `p` applied over it.
    pub(crate) fn from_params(p: &Value, frame: (f64, f64), src: (f64, f64), cmd: &str, default: (Preset, Side)) -> Result<Spec> {
        let preset = match str_p(p, "preset") {
            None => default.0,
            Some("stack") => Preset::Stack,
            Some("split") => Preset::Split,
            Some("pip") => Preset::Pip,
            Some(o) => return Err(bad(cmd, format!("unknown preset `{o}` (stack, split, pip)"))),
        };
        let side = match str_p(p, "side") {
            None => default.1,
            Some("right") => Side::Right,
            Some("left") => Side::Left,
            Some(o) => return Err(bad(cmd, format!("`side` must be right or left, not `{o}`"))),
        };
        let mut s = Spec::preset(preset, side, frame, src);
        if let Some(g) = crate::shorts::fractions_p(p, "game", cmd)? {
            s.game = g;
        }
        if let Some(m) = crate::shorts::fractions_p(p, "model", cmd)? {
            s.model = m;
        }
        // the old Stacked Layout parameter: the gameplay pane's share of the height
        if p.get("split").is_some() {
            let split = num_p(p, "split", 0.6, 0.2, 0.85, cmd)?;
            let (gw, gh) = ((s.game[2] - s.game[0]) * src.0, (s.game[3] - s.game[1]) * src.1);
            s.game_width = (frame.0 * gh / (split * frame.1 * gw.max(1.0))).clamp(0.3, 1.0);
            s.game_y = split / 2.0;
            s.model_w = 1.0;
            s.model_h = 1.0 - split;
            s.model_y = split + (1.0 - split) / 2.0;
        }
        s.game_width = num_p(p, "gameWidth", s.game_width, 0.3, 1.0, cmd)?;
        s.game_pan = num_p(p, "gamePan", s.game_pan, -1.0, 1.0, cmd)?;
        s.game_y = num_p(p, "gameY", s.game_y, 0.0, 1.0, cmd)?;
        s.model_w = num_p(p, "modelWidth", s.model_w, 0.05, 1.0, cmd)?;
        s.model_h = num_p(p, "modelHeight", s.model_h, 0.05, 1.0, cmd)?;
        s.model_x = num_p(p, "modelX", s.model_x, 0.0, 1.0, cmd)?;
        s.model_y = num_p(p, "modelY", s.model_y, 0.0, 1.0, cmd)?;
        s.corner = num_p(p, "corner", s.corner, 0.0, 500.0, cmd)?;
        s.game_corner = num_p(p, "gameCorner", s.game_corner, 0.0, 500.0, cmd)?;
        s.border = num_p(p, "border", s.border, 0.0, 100.0, cmd)?;
        if let Some(c) = color_p(p, "borderColor", cmd)? {
            s.border_color = c;
        }
        s.background = match str_p(p, "background") {
            None => s.background,
            Some("blur") => Background::Blur,
            Some("black") => Background::Black,
            Some(o) => return Err(bad(cmd, format!("`background` must be blur or black, not `{o}`"))),
        };
        Ok(s)
    }

    /// The values as command parameters (what the Layout… dialog edits).
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "preset": match self.preset { Preset::Stack => "stack", Preset::Split => "split", Preset::Pip => "pip" },
            "side": match self.side { Side::Right => "right", Side::Left => "left" },
            "game": self.game,
            "gameWidth": self.game_width,
            "gamePan": self.game_pan,
            "gameY": self.game_y,
            "model": self.model,
            "modelWidth": self.model_w,
            "modelHeight": self.model_h,
            "modelX": self.model_x,
            "modelY": self.model_y,
            "corner": self.corner,
            "gameCorner": self.game_corner,
            "border": self.border,
            "borderColor": format!(
                "#{:02x}{:02x}{:02x}",
                (self.border_color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                (self.border_color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                (self.border_color[2].clamp(0.0, 1.0) * 255.0).round() as u8
            ),
            "background": match self.background { Background::Blur => "blur", Background::Black => "black" },
        })
    }

    /// The gameplay window and pane, and the model region and box, in display pixels of a
    /// `src`-sized recording and pixels of a `frame`-sized frame.
    fn geometry(&self, frame: (f64, f64), src: (f64, f64)) -> (Rect, Rect, Rect, Rect) {
        let (fw, fh) = frame;
        let region = Rect::from_fractions(self.game, src.0, src.1);
        // the kept window: `game_width` of the region's width, panned, full region height
        let ww = (region.w * self.game_width).max(1.0);
        let slack = (region.w - ww).max(0.0);
        let wx = region.x + slack * (self.game_pan + 1.0) / 2.0;
        let window = Rect::new(wx, region.y, ww, region.h);
        // the pane: full frame width, the window's shape (never taller than the frame)
        let ph = (fw * window.h / window.w).min(fh);
        let cy = (self.game_y * fh).clamp(ph / 2.0, fh - ph / 2.0);
        let pane = Rect::new(0.0, cy - ph / 2.0, fw, ph);
        let model = Rect::from_fractions(self.model, src.0, src.1);
        let (bw, bh) = ((self.model_w * fw).max(2.0), (self.model_h * fh).max(2.0));
        let bx = (self.model_x * fw).clamp(bw / 2.0, (fw - bw / 2.0).max(bw / 2.0));
        let by = (self.model_y * fh).clamp(bh / 2.0, (fh - bh / 2.0).max(bh / 2.0));
        let boxr = Rect::new(bx - bw / 2.0, by - bh / 2.0, bw, bh);
        (window, pane, model, boxr)
    }
}

/// The pane-cropping effect of a clip: Rounded Crop (layouts) or Crop (older layouts).
pub(crate) fn pane_crop_index(it: &TrackItem) -> Option<usize> {
    it.effects.iter().position(|e| e.effect == "rounded_crop").or_else(|| it.effects.iter().position(|e| e.effect == "crop"))
}

/// Show `pl` on `it` with a Rounded Crop of `corner` / `border` frame pixels (converted to the
/// clip's pixels at its scale). Removes an older Crop and Auto Reframe.
fn apply(it: &mut TrackItem, pl: &Placement, storage: (f64, f64), display: (f64, f64), corner: f64, border: f64, color: [f32; 4], frame_w: f64) {
    let anchor = (pl.anchor.0 * storage.0 / display.0.max(1.0), pl.anchor.1 * storage.1 / display.1.max(1.0));
    set_motion(it, pl.scale, pl.position, anchor);
    it.effects.retain(|e| !matches!(e.effect.as_str(), "crop" | "auto_reframe"));
    let idx = match it.effects.iter().position(|e| e.effect == "rounded_crop") {
        Some(i) => Some(i),
        None => insert_effect(it, "rounded_crop", None),
    };
    // screen pixels (at 1080 wide) to the clip's own pixels
    let to_clip = (frame_w / 1080.0) / (pl.scale / 100.0).max(1e-6) * (storage.0 / display.0.max(1.0));
    if let Some(e) = idx.and_then(|i| it.effects.get_mut(i)) {
        for (k, v) in ["left", "top", "right", "bottom"].into_iter().zip(pl.crop) {
            set_static(e, k, ParamValue::Float(v));
        }
        set_static(e, "radius", ParamValue::Float(corner * to_clip));
        set_static(e, "feather", ParamValue::Float(0.0));
        set_static(e, "border", ParamValue::Float(border * to_clip));
        set_static(e, "border_color", ParamValue::Color(color));
    }
}

/// A blurred, darkened copy of `it` covering the frame.
fn background_copy(it: &TrackItem, id: u64, storage: (f64, f64), display: (f64, f64), frame: (f64, f64)) -> TrackItem {
    let mut bg = copy_clip(it, id, BACKGROUND_SUFFIX);
    bg.effects.retain(|e| !matches!(e.effect.as_str(), "crop" | "rounded_crop" | "auto_reframe" | "camera_shake" | "rgb_split" | "wave_warp"));
    let fill = sh::frame_scale(display, frame, true).unwrap_or(100.0);
    set_motion(&mut bg, fill * 1.08, (frame.0 / 2.0, frame.1 / 2.0), (storage.0 / 2.0, storage.1 / 2.0));
    add_effect(&mut bg, "gaussian_blur", vec![("blurriness", ParamValue::Float((frame.1 / 1920.0 * 90.0).max(10.0))), ("repeat_edge", ParamValue::Bool(true))]);
    add_effect(&mut bg, "brightness_contrast", vec![("brightness", ParamValue::Float(-30.0))]);
    bg
}

/// The clips a layout arranges: selected footage clips, or every footage clip of the main
/// picture track. Copies a layout made are never targets.
fn targets(s: &Session, p: &Value) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let ok = |i: &TrackItem| is_footage(&s.project, i) && !i.name.ends_with(MODEL_SUFFIX) && !i.name.ends_with(BACKGROUND_SUFFIX);
    let picked: Vec<ClipId> = clips_p(s, p).into_iter().filter(|c| q.video_tracks.iter().any(|t| t.item(*c).is_some_and(ok))).collect();
    if !picked.is_empty() {
        return picked;
    }
    let main = q.video_tracks.iter().position(|t| t.items.iter().any(ok)).or_else(|| base_track(&s.project, q));
    main.and_then(|b| q.video_tracks.get(b)).map(|t| t.items.iter().filter(|i| ok(i)).map(|i| i.id).collect()).unwrap_or_default()
}

/// The bottom edge of the gameplay pane of the last layout applied (fraction of the frame height),
/// for captions to sit on.
pub(crate) fn game_pane_bottom(s: &Session) -> Option<f64> {
    let v = s.state.shorts_layout.as_ref()?;
    let q = s.active_sequence()?;
    let frame = (f64::from(q.settings.width), f64::from(q.settings.height));
    let c = targets(s, &json!({})).into_iter().next()?;
    let it = q.video_tracks.iter().find_map(|t| t.item(c))?;
    let (_, display) = picture_sizes(&s.project, it, q.settings.par)?;
    let spec = Spec::from_params(v, frame, display, "shorts.captions", (Preset::Stack, Side::Right)).ok()?;
    let (_, pane, _, _) = spec.geometry(frame, display);
    (frame.1 > 0.0).then(|| (pane.y + pane.h) / frame.1)
}

/// The layout the Layout… dialog opens with: the values last applied in this session, or the
/// `stack` preset for the active sequence.
pub fn current(s: &Session) -> Value {
    if let Some(v) = s.state.shorts_layout.as_ref() {
        return v.clone();
    }
    let Some(q) = s.active_sequence() else { return Value::Null };
    let frame = (f64::from(q.settings.width), f64::from(q.settings.height));
    Spec::preset(Preset::Stack, Side::Right, frame, (1920.0, 1080.0)).to_json()
}

pub(crate) fn layout(s: &mut Session, p: &Value, default: (Preset, Side), cmd: &str) -> Result<Value> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = (f64::from(q.settings.width), f64::from(q.settings.height));
    let par = q.settings.par;
    let ids = targets(s, p);
    // (clip, storage size, display size)
    let plans: Vec<(ClipId, (f64, f64), (f64, f64))> = ids
        .iter()
        .filter_map(|c| {
            let it = q.video_tracks.iter().find_map(|t| t.item(*c))?;
            let (storage, display) = picture_sizes(&s.project, it, par)?;
            Some((*c, storage, display))
        })
        .collect();
    let Some(&(_, _, first_display)) = plans.first() else {
        return Err(bad(cmd, "select the stream clip (or put footage on the timeline) first"));
    };
    let spec = Spec::from_params(p, frame, first_display, cmd, default)?;
    // live re-application from the dialog folds into one undo step
    let live = crate::commands::bool_p(p, "live").unwrap_or(false);
    let label = "Layout";
    let run = |q: &mut filmcraft_project::Sequence, ctx: &mut filmcraft_edit::EditCtx, st: &mut crate::EditorState| -> Result<Value> {
        // the previous layout of these clips goes: its model and background copies
        let shapes: Vec<(filmcraft_project::ItemId, filmcraft_time::Tick, filmcraft_time::Tick)> =
            plans.iter().filter_map(|(c, _, _)| q.find_item(*c).map(|(_, i)| (i.item, i.start, i.duration))).collect();
        for tr in q.video_tracks.iter_mut().filter(|t| matches!(t.name.as_str(), MODEL_TRACK | BACKGROUND_TRACK | OLD_BACKGROUND_TRACK)) {
            tr.items.retain(|i| {
                let copy = i.name.ends_with(MODEL_SUFFIX) || i.name.ends_with(BACKGROUND_SUFFIX) || i.name.ends_with(" (Blurred)");
                !(copy && shapes.contains(&(i.item, i.start, i.duration)))
            });
        }
        q.video_tracks.retain(|t| !(matches!(t.name.as_str(), MODEL_TRACK | BACKGROUND_TRACK | OLD_BACKGROUND_TRACK) && t.items.is_empty()));
        let mut models = Vec::new();
        let mut backs = Vec::new();
        let mut out = Vec::new();
        let mut base = usize::MAX;
        for (c, storage, display) in &plans {
            let (window, pane, region, boxr) = spec.geometry(frame, *display);
            let (Some(game_pl), Some(model_pl)) = (sh::place_region(*display, window, pane), sh::place_region(*display, region, boxr)) else { continue };
            let Some((ti, it)) = q.video_tracks.iter_mut().enumerate().find_map(|(ti, t)| t.items.iter_mut().find(|i| i.id == *c).map(|i| (ti, i))) else {
                continue;
            };
            base = base.min(ti);
            apply(it, &game_pl, *storage, *display, spec.game_corner, 0.0, spec.border_color, frame.0);
            let mut m = copy_clip(it, ctx.alloc(), MODEL_SUFFIX);
            apply(&mut m, &model_pl, *storage, *display, spec.corner, spec.border, spec.border_color, frame.0);
            if spec.background == Background::Blur {
                backs.push(background_copy(it, ctx.alloc(), *storage, *display, frame));
            }
            out.push(json!({"clip": c.0, "model": m.id.0}));
            models.push(m);
        }
        if models.is_empty() {
            return Err(bad(cmd, "the selected clips have no picture to lay out"));
        }
        let base = base.min(q.video_tracks.len());
        let mut mt = new_video_track(ctx.alloc(), MODEL_TRACK);
        mt.items = models;
        mt.sort();
        q.video_tracks.insert((base + 1).min(q.video_tracks.len()), mt);
        if !backs.is_empty() {
            let mut bt = new_video_track(ctx.alloc(), BACKGROUND_TRACK);
            bt.items = backs;
            bt.sort();
            q.video_tracks.insert(base, bt);
        }
        q.check().map_err(EngineError::Other)?;
        st.selection = out.iter().filter_map(|o| o["model"].as_u64().map(ClipId)).collect();
        st.shorts_layout = Some(spec.to_json());
        Ok(json!({"clips": out, "layout": spec.to_json()}))
    };
    if live { s.edit_sequence_as(label, Some("shorts.layout.live"), run) } else { s.edit_sequence(label, run) }
}
