//! The Shorts menu: tools for vertical short-form video (YouTube Shorts, TikTok, Reels), built
//! with clips cut from streams in mind.
//!
//! - **Auto Edit…**: tries the formats on the clip, checks each and keeps the best, see
//!   `shorts_auto`.
//! - **Formats** (Formats…, and YouTube Shorts ▸ / TikTok ▸ for each format straight away): a
//!   whole short in one step, see [`crate::shorts_formats`].
//! - **Vertical Short** (Crop to Fill, Follow Subject, Blurred Background): a 1080 x 1920 copy of
//!   the active sequence with its footage reframed for the vertical frame. The original sequence
//!   is left as it was.
//! - **Layout…** and the stacked layouts: gameplay and the streamer's model or camera cut from the
//!   same recording, see [`crate::shorts_layout`].
//! - **Auto-Trim to Length…** and **Loop Ending**, see [`crate::shorts_trim`].
//! - **Punch In**, **Camera Shake** and **Impact Flash** at the playhead.
//! - **Word Captions** (Pop, Karaoke, One Word…): animated captions from the transcript, see
//!   [`crate::shorts_captions`].
//!
//! Every command is one undo step and takes its options as parameters, so agents can drive it
//! too. The geometry and word timing live in [`filmcraft_edit::shorts`].

use std::collections::BTreeMap;

use filmcraft_edit::shorts as sh;
use filmcraft_geom::Vec2;
use filmcraft_project::{ClipId, EffectInstance, ItemKind, Keyframe, Param, ParamValue, Project, Sequence, Track, TrackId, TrackItem, TrackKind};

use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, clips_p, f64_p, has_seq, str_p, time_p};
use crate::{EngineError, Result, Session};

type Run = fn(&mut Session, &Value) -> Result<Value>;

/// The Shorts menu (a menu of its own in the menu bar, after Sequence) and its submenus.
const MENU: &[&str] = &["Shorts"];
const YOUTUBE_MENU: &[&str] = &["Shorts", "YouTube Shorts"];
const TIKTOK_MENU: &[&str] = &["Shorts", "TikTok"];
const REFRAME: &[&str] = &["Shorts", "Reframe"];
const AT_PLAYHEAD: &[&str] = &["Shorts", "Effects at Playhead"];
const AUTOMATIC: &[&str] = &["Shorts", "Automatic Effects"];
const GRAPHICS: &[&str] = &["Shorts", "Graphics"];
const CAPTION_STYLES: &[&str] = &["Shorts", "Caption Styles"];
const SOUND: &[&str] = &["Shorts", "Sound"];
const MEMES: &[&str] = &["Shorts", "Meme Drops"];

fn spec(id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    spec_in(MENU, id, label, params, run)
}

fn spec_in(menu: &'static [&'static str], id: &'static str, label: &'static str, params: &'static str, run: Run) -> CommandSpec {
    CommandSpec { id, label, menu, shortcut: None, params, enabled: has_seq, run, journal: true }
}

const VERTICAL: &str = r#"{"width":u32=1080,"height":u32=1920,"name":str?}"#;
const LAYOUT: &str = r##"{"clips":[id]?,"preset":"stack|split|pip|spotlight|full","side":"right|left","game":[x0,y0,x1,y1]?,"gameWidth":f?,"gameSize":f?,"gamePan":f?,"gameY":f?,"model":[x0,y0,x1,y1]?,"modelWidth":f?,"modelHeight":f?,"modelX":f?,"modelY":f?,"corner":px?,"gameCorner":px?,"border":px?,"borderColor":"#ffffff"?,"showModel":bool?,"background":"blur|black","split":f?,"live":bool?}"##;
const AUTO: &str = r#"{"platform":"youtube|tiktok","style":"reaction|formats"="reaction","length":"auto|keep"|seconds="auto","title":str?,"tryFormats":bool=true,"memes":bool=true,"memesFolder":str?,"captions":bool=true,"hype":bool=true,"finish":bool=true,"pick":{"format":str,"side":"right|left","crop":bool}?}"#;
const FORMAT: &str = r#"{"platform":"youtube|tiktok","format":"reaction|gameplay|fullFrame|meme|spotlight|countdown","title":str?,"side":"right|left"?,"hook":bool?,"captions":bool?,"captionStyle":"auto|pop|karaoke|box|oneWord|plain|reaction"?,"progressBar":bool?,"seconds":f=0,"loop":bool=false,"loopSeconds":f=0.5,"moments":n=3,"momentSeconds":f=8,"background":"blur|black"?,"game":[x0,y0,x1,y1]?,"bleep":bool?,"hype":bool?,"scenes":bool=true,"hits":bool=true,"ending":bool=true,"memes":bool=true,"memesFolder":str?}"#;
const CAPTIONS: &str = r##"{"style":"pop|karaoke|box|oneWord|plain|reaction"?,"maxWords":n=3,"maxChars":n?,"lines":n=1,"font":"Inter"?,"fontStyle":"Bold"?,"size":px?,"uppercase":bool=true,"punctuation":bool=false,"color":"#ffffff"?,"highlight":"#ffd60a"?,"highlightText":"#ffffff"?,"grow":percent?,"outline":percent?,"outlineColor":"#000000"?,"shadow":n?,"background":bool?,"backgroundColor":"#000000"?,"backgroundOpacity":n?,"radius":percent?,"x":f?,"y":f?,"width":f?,"animation":"pop|bounce|fade|slide|none"?,"hold":seconds=0.25,"speakerColors":bool=false,"fixes":"wrong=right, …"?,"shout":bool?,"censor":"stars|vowel"?,"wordColors":"word=#rrggbb, …"?,"replace":bool=true,"live":bool?}"##;

pub(crate) fn commands() -> Vec<CommandSpec> {
    use crate::shorts_captions as cap;
    use crate::shorts_formats::{self as fm, Format};
    use crate::shorts_fx as fx;
    use crate::shorts_layout as ly;
    const WORDS: &str =
        r#"{"words":"bonk, what"|[str],"effect":"punch|shake|glitch|flash"|[..],"shout":bool?,"censor":"stars|vowel"?,"wordColors":"huh=#7a5cfa"?}"#;
    macro_rules! format_spec {
        ($menu:expr, $id:literal, $label:literal, $pf:expr, $f:expr) => {
            spec_in($menu, $id, $label, FORMAT, |s, p| fm::format(s, p, Some(($pf, $f)), $id))
        };
    }
    vec![
        spec("shorts.autoEdit", "Auto Edit…", AUTO, crate::shorts_auto::auto_edit),
        spec("shorts.format", "Formats…", FORMAT, |s, p| fm::format(s, p, None, "shorts.format")),
        format_spec!(YOUTUBE_MENU, "shorts.youtube.reaction", "Reaction Split", fm::YOUTUBE, Format::Reaction),
        format_spec!(YOUTUBE_MENU, "shorts.youtube.gameplay", "Gameplay + Model", fm::YOUTUBE, Format::Gameplay),
        format_spec!(YOUTUBE_MENU, "shorts.youtube.fullFrame", "Full Frame + Blur", fm::YOUTUBE, Format::FullFrame),
        format_spec!(YOUTUBE_MENU, "shorts.youtube.meme", "Meme Caption Bar", fm::YOUTUBE, Format::Meme),
        format_spec!(YOUTUBE_MENU, "shorts.youtube.spotlight", "Model Spotlight", fm::YOUTUBE, Format::Spotlight),
        format_spec!(YOUTUBE_MENU, "shorts.youtube.countdown", "Top Moments Countdown", fm::YOUTUBE, Format::Countdown),
        format_spec!(TIKTOK_MENU, "shorts.tiktok.reaction", "Reaction Split", fm::TIKTOK, Format::Reaction),
        format_spec!(TIKTOK_MENU, "shorts.tiktok.gameplay", "Gameplay + Model", fm::TIKTOK, Format::Gameplay),
        format_spec!(TIKTOK_MENU, "shorts.tiktok.fullFrame", "Full Frame + Blur", fm::TIKTOK, Format::FullFrame),
        format_spec!(TIKTOK_MENU, "shorts.tiktok.meme", "Meme Caption Bar", fm::TIKTOK, Format::Meme),
        format_spec!(TIKTOK_MENU, "shorts.tiktok.spotlight", "Model Spotlight", fm::TIKTOK, Format::Spotlight),
        format_spec!(TIKTOK_MENU, "shorts.tiktok.countdown", "Top Moments Countdown", fm::TIKTOK, Format::Countdown),
        spec("shorts.layout", "Layout…", LAYOUT, |s, p| ly::layout(s, p, (ly::Preset::Stack, ly::Side::Right), "shorts.layout")),
        spec("shorts.captions", "Word Captions…", CAPTIONS, |s, p| cap::captions(s, p, cap::Style::Pop, "shorts.captions")),
        spec("shorts.autoTrim", "Auto-Trim to Length…", r#"{"seconds":f=30}"#, crate::shorts_trim::auto_trim),
        spec("shorts.loopEnding", "Loop Ending", r#"{"seconds":f=0.5}"#, crate::shorts_trim::loop_ending),
        spec_in(REFRAME, "shorts.verticalFill", "Vertical Short: Crop to Fill", VERTICAL, |s, p| vertical(s, p, Reframe::Fill, "shorts.verticalFill")),
        spec_in(REFRAME, "shorts.verticalFollow", "Vertical Short: Follow Subject", VERTICAL, |s, p| vertical(s, p, Reframe::Follow, "shorts.verticalFollow")),
        spec_in(REFRAME, "shorts.verticalBlur", "Vertical Short: Blurred Background", VERTICAL, |s, p| vertical(s, p, Reframe::Blur, "shorts.verticalBlur")),
        spec_in(REFRAME, "shorts.stackRight", "Stacked Layout: Model on Right", LAYOUT, |s, p| {
            ly::layout(s, p, (ly::Preset::Split, ly::Side::Right), "shorts.stackRight")
        }),
        spec_in(REFRAME, "shorts.stackLeft", "Stacked Layout: Model on Left", LAYOUT, |s, p| {
            ly::layout(s, p, (ly::Preset::Split, ly::Side::Left), "shorts.stackLeft")
        }),
        spec_in(REFRAME, "shorts.trackModel", "Track Model", r#"{"clips":[id]?,"wait":bool=false}"#, crate::shorts_track::track_model),
        spec_in(AT_PLAYHEAD, "shorts.punchIn", "Punch In at Playhead", r#"{"clips":[id]?,"time":ticks?,"zoom":percent=125,"seconds":f=1}"#, punch_in),
        spec_in(AT_PLAYHEAD, "shorts.shake", "Camera Shake at Playhead", r#"{"clips":[id]?,"time":ticks?,"amount":px=40,"seconds":f=0.5}"#, shake),
        spec_in(AT_PLAYHEAD, "shorts.glitch", "Glitch Hit at Playhead", r#"{"clips":[id]?,"time":ticks?,"amount":px=40,"seconds":f=0.35}"#, glitch),
        spec_in(AT_PLAYHEAD, "shorts.flash", "Impact Flash at Playhead", r##"{"time":ticks?,"seconds":f=0.3,"color":"#ffffff"?}"##, flash),
        spec_in(
            AT_PLAYHEAD,
            "shorts.speedRamp",
            "Speed Ramp at Playhead",
            r#"{"time":ticks?,"speed":percent=35,"seconds":f=1,"ramp":seconds=0.2}"#,
            fx::speed_ramp,
        ),
        spec_in(
            AT_PLAYHEAD,
            "shorts.freezeFrame",
            "Freeze Frame at Playhead",
            r#"{"time":ticks?,"seconds":f=1.5,"zoom":percent=115,"flash":bool=true}"#,
            fx::freeze_frame,
        ),
        spec_in(
            AUTOMATIC,
            "shorts.hype",
            "Find Hype Moments",
            r#"{"effects":["punch","shake","glitch","flash"]=["punch","shake"],"sensitivity":dB=9,"maxMoments":n=8,"minGap":seconds=2,"markersOnly":bool=false}"#,
            fx::hype,
        ),
        spec_in(
            AUTOMATIC,
            "shorts.chatHighlights",
            "Find Chat Highlights…",
            r#"{"path":str,"clip":id?,"offset":seconds=0,"window":seconds=5,"maxMoments":n=10}"#,
            crate::chat::highlights,
        ),
        spec_in(AUTOMATIC, "shorts.zoomCuts", "Zoom on Every Cut", r#"{"zoom":percent=112}"#, fx::zoom_cuts),
        spec_in(
            AUTOMATIC,
            "shorts.beatSync",
            "Effects on the Beat",
            r#"{"clips":[id]?,"effects":["pulse","flash","shake","punch","glitch"]=["pulse"],"every":n=2,"amount":percent=105}"#,
            fx::beat_sync,
        ),
        spec_in(AUTOMATIC, "shorts.keywordEffects", "Effects on Words…", WORDS, fx::keyword_effects),
        spec_in(
            GRAPHICS,
            "shorts.hookTitle",
            "Hook Title…",
            r##"{"text":str="WAIT FOR IT…","seconds":f=2.5,"color":"#ffd60a"?,"time":ticks?,"y":f?,"width":f=0.9}"##,
            fx::hook_title,
        ),
        spec_in(
            GRAPHICS,
            "shorts.chatOverlay",
            "Chat Overlay…",
            r#"{"path":str,"clip":id?,"offset":seconds=0,"lines":n=6,"maxChars":n=34,"position":"left|right"}"#,
            crate::chat::overlay,
        ),
        spec_in(GRAPHICS, "shorts.progressBar", "Progress Bar", r##"{"color":"#ffd60a"?,"position":"bottom|top","height":px?}"##, fx::progress_bar),
        spec_in(CAPTION_STYLES, "shorts.captionsPop", "Word Captions: Pop", CAPTIONS, |s, p| cap::captions(s, p, cap::Style::Pop, "shorts.captionsPop")),
        spec_in(CAPTION_STYLES, "shorts.captionsKaraoke", "Word Captions: Karaoke", CAPTIONS, |s, p| {
            cap::captions(s, p, cap::Style::Karaoke, "shorts.captionsKaraoke")
        }),
        spec_in(CAPTION_STYLES, "shorts.captionsBox", "Word Captions: Highlight Box", CAPTIONS, |s, p| {
            cap::captions(s, p, cap::Style::Box, "shorts.captionsBox")
        }),
        spec_in(CAPTION_STYLES, "shorts.captionsOneWord", "Word Captions: One Word", CAPTIONS, |s, p| {
            cap::captions(s, p, cap::Style::OneWord, "shorts.captionsOneWord")
        }),
        spec_in(
            SOUND,
            "shorts.bleep",
            "Bleep Swear Words",
            r#"{"words":[str]|str?,"extraWords":[str]|str?,"hz":f=1000,"beepDb":f=-14,"padding":seconds=0.06}"#,
            fx::bleep,
        ),
        spec_in(SOUND, "shorts.duckMusic", "Duck Music Under Speech", r#"{"clips":[id]?,"reduceDb":f=-15}"#, fx::duck_music),
        spec_in(SOUND, "shorts.soundEffect", "Sound Effect at Playhead", r#"{"item":itemId?,"time":ticks?}"#, fx::sound_effect),
        spec_in(SOUND, "shorts.bassHit", "Bass Hit at Playhead", r#"{"time":ticks?,"folder":str?}"#, bass_hit),
        spec_in(MEMES, "shorts.memeDrops", "Drop Memes on Their Words", r#"{"folder":str?}"#, crate::shorts_memes::drop_memes),
        spec_in(MEMES, "shorts.memeDrop", "Drop Meme at Playhead…", r#"{"path":str,"time":ticks?}"#, crate::shorts_memes::drop_meme),
        // the folder can be filled before any sequence exists
        CommandSpec {
            enabled: crate::commands::always,
            ..spec_in(MEMES, "shorts.memeFolder", "Open Meme Folder", r#"{"folder":str?,"create":bool=true}"#, crate::shorts_memes::meme_folder)
        },
    ]
}

// ---------------------------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------------------------

/// A clip that shows a picture of its own (media, a nested sequence, a subclip), as opposed to
/// a graphic or an adjustment layer.
pub(crate) fn is_footage(pr: &Project, it: &TrackItem) -> bool {
    pr.item(it.item).is_some_and(|i| matches!(i.kind, ItemKind::Media(_) | ItemKind::Sequence(_) | ItemKind::Subclip { .. }))
        && filmcraft_render::source_size(pr, it.item).is_some()
}

/// The picture sizes of a clip: storage pixels (for the anchor and crop) and display size in
/// sequence pixels (for the scale).
pub(crate) fn picture_sizes(pr: &Project, it: &TrackItem, par: (u32, u32)) -> Option<((f64, f64), (f64, f64))> {
    let (w, h) = filmcraft_render::source_size(pr, it.item)?;
    let (dw, dh) = pr.conformed_source_size(it.item, par)?;
    let ok = |v: f64| v.is_finite() && v >= 1.0;
    (w >= 1 && h >= 1 && ok(dw) && ok(dh)).then_some(((w as f64, h as f64), (dw, dh)))
}

/// Index of the lowest video track holding footage: the "main picture" track.
pub(crate) fn base_track(pr: &Project, q: &Sequence) -> Option<usize> {
    q.video_tracks
        .iter()
        .position(|t| t.items.iter().any(|i| is_footage(pr, i) && !is_copy(i)))
        .or_else(|| q.video_tracks.iter().position(|t| t.items.iter().any(|i| is_footage(pr, i))))
}

/// A copy a layout made (its model pane or blurred background) or a Loop Ending's held frame, not
/// the main picture.
pub(crate) fn is_copy(it: &TrackItem) -> bool {
    it.name.ends_with(crate::shorts_layout::MODEL_SUFFIX)
        || it.name.ends_with(crate::shorts_layout::BACKGROUND_SUFFIX)
        || it.name.ends_with(" (Blurred)")
        || crate::shorts_trim::is_loop_hold(it)
}

/// A blurred background copy (effects at a moment leave it alone).
fn is_background(it: &TrackItem) -> bool {
    it.name.ends_with(crate::shorts_layout::BACKGROUND_SUFFIX) || it.name.ends_with(" (Blurred)")
}

/// Set a parameter to a static value, dropping its keyframes (adds the parameter if missing).
pub(crate) fn set_static(e: &mut EffectInstance, k: &str, v: ParamValue) {
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
pub(crate) fn set_motion(it: &mut TrackItem, scale: f64, position: (f64, f64), anchor: (f64, f64)) {
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
pub(crate) fn insert_effect(it: &mut TrackItem, id: &str, at: Option<usize>) -> Option<usize> {
    let def = filmcraft_project::find_effect(id)?;
    let slot = standard_slot(it);
    let at = at.unwrap_or(slot).min(slot);
    it.effects.insert(at, def.instance());
    Some(at)
}

/// Put a new instance of effect `id` at the end of the clip's standard effects, with `params` set.
pub(crate) fn add_effect(it: &mut TrackItem, id: &str, params: Vec<(&str, ParamValue)>) {
    if let Some(e) = insert_effect(it, id, None).and_then(|i| it.effects.get_mut(i)) {
        for (k, v) in params {
            set_static(e, k, v);
        }
    }
}

/// A copy of a clip for another track: new id, no links or group, `suffix` added to its name.
pub(crate) fn copy_clip(it: &TrackItem, id: u64, suffix: &str) -> TrackItem {
    let mut c = it.clone();
    c.id = ClipId(id);
    c.link = None;
    c.group = None;
    c.name = format!("{}{suffix}", it.name);
    c
}

pub(crate) fn new_video_track(id: u64, name: &str) -> Track {
    Track::new(TrackId(id), TrackKind::Video, name.to_string())
}

/// A colour parameter: `"#rrggbb"`, `"#rrggbbaa"` or `[r, g, b(, a)]` (0..255), as 0..1 floats.
pub(crate) fn color_p(p: &Value, k: &str, cmd: &str) -> Result<Option<[f32; 4]>> {
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
pub(crate) fn num_p(p: &Value, k: &str, default: f64, lo: f64, hi: f64, cmd: &str) -> Result<f64> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(default),
        Some(_) => match f64_p(p, k) {
            Some(v) if v.is_finite() && v >= lo && v <= hi => Ok(v),
            _ => Err(bad(cmd, format!("`{k}` must be a number from {lo} to {hi}"))),
        },
    }
}

/// Fractions `[x0, y0, x1, y1]` (each 0..1) from a parameter.
pub(crate) fn fractions_p(p: &Value, k: &str, cmd: &str) -> Result<Option<[f64; 4]>> {
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
// Punch In, Camera Shake, Impact Flash
// ---------------------------------------------------------------------------------------------

/// Clips the punch effects apply to at sequence time `t`: the selected video clips under it,
/// or else every footage clip under it.
pub(crate) fn clips_at(s: &Session, p: &Value, t: Tick) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let covers = |c: &ClipId| q.video_tracks.iter().any(|tr| tr.item(*c).is_some_and(|i| i.range().contains(t)));
    let picked: Vec<ClipId> = clips_p(s, p).into_iter().filter(covers).collect();
    if !picked.is_empty() {
        return picked;
    }
    footage_at(s, t)
}

/// Every footage clip under sequence time `t`.
pub(crate) fn footage_at(s: &Session, t: Tick) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    q.video_tracks.iter().flat_map(|tr| tr.items.iter()).filter(|i| i.range().contains(t) && is_footage(&s.project, i)).map(|i| i.id).collect()
}

/// `t` plus `frames` frames of `rate`.
pub(crate) fn after(t: Tick, frames: i64, frame: Tick) -> Tick {
    Tick(t.0.saturating_add(frame.0.saturating_mul(frames)))
}

pub(crate) fn put(prm: &mut Param, t: Tick, v: f64) {
    prm.put_keyframe(t, ParamValue::Float(v));
}

/// A one-shot effect that lands on a moment of the clips under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Hit {
    /// Zoom in over 3 frames, hold, ease back over 4 (`zoom`: 1.25 = 125 %).
    Punch { zoom: f64, hold: Tick },
    /// Camera Shake that hits at the moment and settles to nothing.
    Shake { amount: f64, dur: Tick },
    /// RGB Split and Wave Warp bursts with a small shake.
    Glitch { amount: f64, dur: Tick },
    /// A quick zoom bump that settles back over 5 frames (`amount`: 1.05 = 105 %), for beats.
    Pulse { amount: f64 },
}

/// Apply `hits` at each moment (sequence time, clips under it) as one undo step labelled
/// `label`. Returns how many clip-moments were changed.
pub(crate) fn apply_hits(s: &mut Session, label: &str, moments: &[(Tick, Vec<ClipId>)], hits: &[Hit]) -> Result<usize> {
    let all: Vec<ClipId> = moments.iter().flat_map(|(_, c)| c.iter().copied()).collect();
    let sizes: BTreeMap<ClipId, (f64, f64)> = s
        .active_sequence()
        .map(|q| {
            q.video_tracks
                .iter()
                .flat_map(|tr| tr.items.iter())
                .filter(|i| all.contains(&i.id))
                .filter_map(|i| filmcraft_render::source_size(&s.project, i.item).map(|(w, h)| (i.id, (f64::from(w), f64::from(h)))))
                .collect()
        })
        .unwrap_or_default();
    s.edit_sequence(label, |q, _, _| {
        let rate = q.settings.frame_rate;
        let frame = rate.frame_duration().max(Tick(1));
        let mut n = 0;
        for (t, clips) in moments {
            let t = rate.snap(*t);
            for tr in q.video_tracks.iter_mut() {
                for it in tr.items.iter_mut().filter(|i| clips.contains(&i.id) && i.range().contains(t) && !is_background(i)) {
                    for hit in hits {
                        match *hit {
                            Hit::Punch { zoom, hold } => punch_clip(it, t, zoom, hold, frame, sizes.get(&it.id).copied()),
                            Hit::Shake { amount, dur } => shake_clip(it, t, dur, amount, frame),
                            Hit::Glitch { amount, dur } => glitch_clip(it, t, dur, amount, frame),
                            Hit::Pulse { amount } => pulse_clip(it, t, amount, frame),
                        }
                    }
                    n += 1;
                }
            }
        }
        Ok(n)
    })
}

/// Keyframe Motion ▸ Scale for a punch-in at `t` (and the crop of a stacked pane, which zooms
/// inside its own edges). `size` is the clip's picture size in storage pixels.
fn punch_clip(it: &mut TrackItem, t: Tick, zoom: f64, hold: Tick, frame: Tick, size: Option<(f64, f64)>) {
    let last = Tick(it.end().0.saturating_sub(1));
    // snap in over 3 frames, hold, ease back over 4 (all inside the clip)
    let t1 = after(t, 3, frame).min(last);
    let t2 = Tick(after(t, 3, frame).0.saturating_add(hold.0)).min(last);
    let t3 = after(t2, 4, frame).min(last);
    let mt = [t, t1, t2, t3].map(|x| it.effect_time_at(x));
    let anchor = it.effect("motion").and_then(|m| m.params.get("anchor")).map(|a| a.vec2_at(mt[0]));
    let Some(scale) = it.effect_mut("motion").and_then(|m| m.params.get_mut("scale")) else { return };
    let base = mt.map(|x| scale.f64_at(x));
    put(scale, mt[0], base[0]);
    put(scale, mt[1], base[1] * zoom);
    put(scale, mt[2], base[2] * zoom);
    put(scale, mt[3], base[3]);
    if let (Some(a), Some((w, h))) = (anchor, size)
        && let Some(c) = crate::shorts_layout::pane_crop_index(it).and_then(|i| it.effects.get_mut(i))
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
}

/// A zoom bump on Motion ▸ Scale at `t`: up in one frame, back over four.
fn pulse_clip(it: &mut TrackItem, t: Tick, amount: f64, frame: Tick) {
    let last = Tick(it.end().0.saturating_sub(1));
    let mt = [t, after(t, 1, frame).min(last), after(t, 5, frame).min(last)].map(|x| it.effect_time_at(x));
    let Some(scale) = it.effect_mut("motion").and_then(|m| m.params.get_mut("scale")) else { return };
    let base = mt.map(|x| scale.f64_at(x));
    put(scale, mt[0], base[0]);
    put(scale, mt[1], base[1] * amount);
    put(scale, mt[2], base[2]);
}

/// The clip's first instance of effect `id`, or a new one at the front of its effects (so it
/// works on the picture before any crop) with `statics` set.
pub(crate) fn effect_at_front(it: &mut TrackItem, id: &str, statics: &[(&str, ParamValue)]) -> Option<usize> {
    if let Some(i) = it.effects.iter().position(|e| e.effect == id) {
        return Some(i);
    }
    let i = insert_effect(it, id, Some(0))?;
    if let Some(e) = it.effects.get_mut(i) {
        for (k, v) in statics {
            set_static(e, k, v.clone());
        }
    }
    Some(i)
}

/// Keyframe `param` of effect instance `idx` through `(sequence time, value)` points, with the
/// value back to `rest` the frame before `t` (when the clip has one).
fn burst(it: &mut TrackItem, idx: usize, param: &str, t: Tick, frame: Tick, rest: f64, points: &[(Tick, f64)]) {
    let last = Tick(it.end().0.saturating_sub(1));
    let before = (t > it.start).then(|| it.effect_time_at(Tick(t.0.saturating_sub(frame.0))));
    let pts: Vec<(Tick, f64)> = points.iter().map(|(x, v)| (it.effect_time_at((*x).min(last)), *v)).collect();
    let Some(prm) = it.effects.get_mut(idx).and_then(|e| e.params.get_mut(param)) else { return };
    if let Some(b) = before {
        put(prm, b, rest);
    }
    for (x, v) in pts {
        put(prm, x, v);
    }
}

/// A Camera Shake that hits at `t` and settles to nothing after `dur`.
fn shake_clip(it: &mut TrackItem, t: Tick, dur: Tick, amount: f64, frame: Tick) {
    let statics = [
        ("amount", ParamValue::Float(0.0)),
        ("rotation", ParamValue::Float(0.0)),
        ("zoom", ParamValue::Float(0.0)),
        ("frequency", ParamValue::Float(14.0)),
        ("motion_blur", ParamValue::Float(40.0)),
    ];
    let Some(idx) = effect_at_front(it, "camera_shake", &statics) else { return };
    let end = Tick(t.0.saturating_add(dur.0));
    for (k, peak) in [("amount", amount), ("rotation", (amount / 20.0).min(10.0)), ("zoom", (amount / 6.0).min(30.0))] {
        burst(it, idx, k, t, frame, 0.0, &[(t, peak), (end, 0.0)]);
    }
}

/// A digital glitch at `t`: stuttering RGB Split and Wave Warp, and a small shake.
fn glitch_clip(it: &mut TrackItem, t: Tick, dur: Tick, amount: f64, frame: Tick) {
    let end = Tick(t.0.saturating_add(dur.0));
    let split = [("amount", ParamValue::Float(0.0))];
    if let Some(idx) = effect_at_front(it, "rgb_split", &split) {
        let pts = [(t, amount), (after(t, 2, frame), amount * 0.25), (after(t, 4, frame), amount * 1.2), (after(t, 6, frame), amount * 0.5), (end, 0.0)];
        burst(it, idx, "amount", t, frame, 0.0, &pts);
    }
    let warp = [("height", ParamValue::Float(0.0)), ("width", ParamValue::Float(90.0)), ("speed", ParamValue::Float(12.0))];
    if let Some(idx) = effect_at_front(it, "wave_warp", &warp) {
        let pts = [(t, amount * 0.6), (after(t, 3, frame), 0.0), (after(t, 5, frame), amount * 0.4), (end, 0.0)];
        burst(it, idx, "height", t, frame, 0.0, &pts);
    }
    shake_clip(it, t, dur, amount * 0.6, frame);
}

/// The moment a Shorts command acts at: `time` (or `frame` or `timecode`). Their `seconds` is
/// how long the effect lasts, never when it happens (the generic time parser would read it as
/// a time).
pub(crate) fn at_time(s: &Session, p: &Value) -> Option<Tick> {
    let mut q = p.clone();
    if let Some(m) = q.as_object_mut() {
        m.remove("seconds");
    }
    time_p(s, &q, "")
}

/// Shared by the single-moment commands: the moment (time param or playhead) and its clips.
fn moment(s: &Session, p: &Value, cmd: &str) -> Result<(Tick, Vec<ClipId>)> {
    let t = at_time(s, p).unwrap_or_else(|| s.playhead());
    let clips = clips_at(s, p, t);
    if clips.is_empty() {
        return Err(bad(cmd, "move the playhead over a video clip"));
    }
    Ok((t, clips))
}

fn punch_in(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.punchIn";
    let zoom = num_p(p, "zoom", 125.0, 101.0, 400.0, CMD)? / 100.0;
    let hold = Tick::from_seconds_f64(num_p(p, "seconds", 1.0, 0.1, 60.0, CMD)?);
    let m = moment(s, p, CMD)?;
    let n = apply_hits(s, "Punch In", &[m], &[Hit::Punch { zoom, hold }])?;
    Ok(json!({"clips": n}))
}

fn shake(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.shake";
    let amount = num_p(p, "amount", 40.0, 1.0, 1000.0, CMD)?;
    let dur = Tick::from_seconds_f64(num_p(p, "seconds", 0.5, 0.05, 30.0, CMD)?);
    let m = moment(s, p, CMD)?;
    let n = apply_hits(s, "Camera Shake", &[m], &[Hit::Shake { amount, dur }])?;
    Ok(json!({"clips": n}))
}

fn glitch(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.glitch";
    let amount = num_p(p, "amount", 40.0, 1.0, 500.0, CMD)?;
    let dur = Tick::from_seconds_f64(num_p(p, "seconds", 0.35, 0.05, 10.0, CMD)?);
    let m = moment(s, p, CMD)?;
    let n = apply_hits(s, "Glitch Hit", &[m], &[Hit::Glitch { amount, dur }])?;
    Ok(json!({"clips": n}))
}

/// A full-frame colour shape layer fading out over `dur` (graphic clips start at media time 0,
/// so the keyframe times are clip-relative).
pub(crate) fn flash_layer(w: u32, h: u32, color: [f32; 4], dur: Tick, peak: f64) -> EffectInstance {
    let (fw, fh) = (f64::from(w), f64::from(h));
    let mut layer = filmcraft_project::graphic::new_shape_layer(0, Vec2::new(fw / 2.0, fh / 2.0), Vec2::new(fw + 8.0, fh + 8.0), Vec::new());
    set_static(&mut layer, "name", ParamValue::Text("Flash".into()));
    set_static(&mut layer, "fill", ParamValue::Bool(true));
    set_static(&mut layer, "fill_color", ParamValue::Color(color));
    set_static(&mut layer, "stroke", ParamValue::Bool(false));
    let peak = if peak.is_finite() { peak.clamp(0.0, 100.0) } else { 90.0 };
    let mut fade = Param::new(ParamValue::Float(peak));
    fade.keyframes = vec![
        Keyframe::new(Tick::ZERO, ParamValue::Float(peak)),
        Keyframe { interp: filmcraft_project::Interpolation::EaseOut, ..Keyframe::new(Tick(dur.0 / 3), ParamValue::Float(peak * 0.6)) },
        Keyframe::new(dur, ParamValue::Float(0.0)),
    ];
    layer.params.insert("opacity".into(), fade);
    layer
}

/// Graphic clips to place on one new video track at the top: (start, duration, name, layers).
pub(crate) type GraphicClip = (Tick, Tick, String, Vec<EffectInstance>);

/// Put `clips` (sorted, non-overlapping) on a new top video track named `track` as one undo
/// step. Returns the new track's index.
pub(crate) fn place_graphic_track(s: &mut Session, label: &str, track: &str, clips: Vec<GraphicClip>) -> Result<usize> {
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        (q.settings.width, q.settings.height, q.settings.frame_rate)
    };
    s.edit(label, |pr, st| {
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
        let mut t = new_video_track(pr.alloc_id(), track);
        t.items = items;
        t.sort();
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        q.video_tracks.push(t);
        q.check().map_err(EngineError::Other)?;
        st.selection.clear();
        Ok(q.video_tracks.len().saturating_sub(1))
    })
}

/// Sound ▸ Bass Hit at Playhead: the built-in bass hit (or the meme folder's `boom` sound).
fn bass_hit(s: &mut Session, p: &Value) -> Result<Value> {
    let t = at_time(s, p).unwrap_or_else(|| s.playhead());
    let folder = crate::shorts_memes::folder_p(p);
    let sound = folder.as_deref().and_then(crate::shorts_memes::hit_sound);
    let n = s.edit_group("Bass Hit at Playhead", |s| crate::shorts_reaction::add_hits(s, &[t.seconds()], sound.as_deref()))?;
    if n == 0 {
        return Err(bad("shorts.bassHit", "move the playhead inside the sequence"));
    }
    Ok(json!({"hits": n, "sound": sound.map(|p| p.to_string_lossy().to_string())}))
}

fn flash(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.flash";
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let secs = num_p(p, "seconds", 0.3, 0.05, 10.0, CMD)?;
    let color = color_p(p, "color", CMD)?.unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let dur = rate.snap_nearest(Tick::from_seconds_f64(secs)).max(rate.frame_duration());
    let layer = flash_layer(w, h, color, dur, 90.0);
    let mut placement = json!({});
    if let Some(t) = p.get("time") {
        placement["time"] = t.clone();
    }
    let id = crate::graphics::place_video_clip(s, CMD, "Flash", &placement, "Impact Flash", vec![layer], move |pr, (w, h, rate)| {
        (crate::graphics::graphic_source(pr, w, h, rate), dur)
    })?;
    Ok(json!({"clip": id.0}))
}
