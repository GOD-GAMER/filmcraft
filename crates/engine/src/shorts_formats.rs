//! Shorts ▸ Formats…, Shorts ▸ YouTube Shorts and Shorts ▸ TikTok: one-click recipes that turn a
//! stream clip into a finished vertical short for one platform.
//!
//! Every format starts the same way: a 16:9 sequence gets a 1080 x 1920 copy (the original is
//! left alone); a vertical one is re-done in place, replacing what an earlier format added. Then:
//!
//! - **Gameplay + Model**: the stacked layout (gameplay above, the model in its own box), a hook
//!   title over the gameplay, word captions on the gameplay's bottom edge.
//! - **Full Frame + Blur**: the whole recording across the frame over a blurred copy, a title
//!   above it for the whole short and captions below it.
//! - **Meme Caption Bar**: a white bar of black text above the whole recording, on black.
//! - **Model Spotlight**: the model large, the gameplay in a smaller card above it, one big word
//!   at a time over the model.
//! - **Top Moments Countdown**: the biggest moments of a longer clip one after the other, smallest
//!   first, each numbered (#3, #2, #1) with a flash between them, under a header.
//!
//! Text is kept out of what the app draws over the video (its buttons, the title and channel
//! name, the tabs at the top): each [`Platform`] lists those zones, and the Program monitor can
//! show them (Shorts ▸ Safe Zones). Options: trim to a length first (Auto-Trim), a loop ending,
//! a progress bar, captions on or off and their style. The whole recipe is one undo step.

use filmcraft_geom::Vec2;
use filmcraft_project::{ClipId, EffectInstance, ItemKind, Keyframe, Param, ParamValue};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{bad, bool_p, str_p};
use crate::shorts::{GraphicClip, after, flash_layer, num_p, place_graphic_track, set_static};
use crate::shorts_captions as cap;
use crate::shorts_layout as ly;
use crate::shorts_trim::Moment;
use crate::{EngineError, Result, Session};

/// Tracks the formats put their graphics on (replaced when a format is applied again).
const TITLE_TRACK: &str = "Title";
const BAR_TRACK: &str = "Meme Bar";
const COUNT_TRACK: &str = "Countdown";
const COUNT_FLASH_TRACK: &str = "Countdown Flashes";
const HOOK_TRACK: &str = "Hook Title";
const PROGRESS_TRACK: &str = "Progress Bar";
const GENERATED: [&str; 7] = [cap::TRACK, HOOK_TRACK, PROGRESS_TRACK, TITLE_TRACK, BAR_TRACK, COUNT_TRACK, COUNT_FLASH_TRACK];
const DEFAULT_TITLE: &str = "WAIT FOR IT…";

// ---------------------------------------------------------------------------------------------
// Platforms
// ---------------------------------------------------------------------------------------------

/// A part of the screen an app covers with its own buttons and text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    pub label: &'static str,
    /// `[x0, y0, x1, y1]`, fractions of a 9:16 frame.
    pub rect: [f64; 4],
}

/// Where a short is posted: what its app covers, how long a short may be, and the export
/// preset that suits it. The zones are approximate (the apps change them, and phones differ):
/// the bar at the top, the text at the bottom, and the column of buttons on the right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Platform {
    pub id: &'static str,
    pub name: &'static str,
    /// Top bar, bottom text, button column (in that order).
    pub zones: [Zone; 3],
    pub max_seconds: f64,
    pub min_seconds: f64,
    /// The longest a short whose music has a copyright claim may be (`None`: no such rule).
    pub claimed_music_seconds: Option<f64>,
    /// The built-in export preset for it.
    pub preset: &'static str,
}

pub const YOUTUBE: Platform = Platform {
    id: "youtube",
    name: "YouTube Shorts",
    zones: [
        Zone { label: "Search and menu", rect: [0.0, 0.0, 1.0, 0.09] },
        Zone { label: "Title, channel and sound", rect: [0.0, 0.80, 1.0, 1.0] },
        Zone { label: "Like, comment and share", rect: [0.86, 0.45, 1.0, 0.80] },
    ],
    max_seconds: 180.0,
    min_seconds: 1.0,
    claimed_music_seconds: Some(60.0),
    preset: "YouTube Shorts 1080×1920",
};

pub const TIKTOK: Platform = Platform {
    id: "tiktok",
    name: "TikTok",
    zones: [
        Zone { label: "Following and For You", rect: [0.0, 0.0, 1.0, 0.09] },
        Zone { label: "Name, caption and sound", rect: [0.0, 0.76, 1.0, 1.0] },
        Zone { label: "Profile, like, comment and share", rect: [0.85, 0.38, 1.0, 0.76] },
    ],
    max_seconds: 600.0,
    min_seconds: 3.0,
    claimed_music_seconds: None,
    preset: "TikTok 1080×1920",
};

pub const PLATFORMS: [Platform; 2] = [YOUTUBE, TIKTOK];

/// A platform by id (`youtube`, `tiktok`).
pub fn platform(id: &str) -> Option<Platform> {
    match id.trim().to_ascii_lowercase().as_str() {
        "youtube" | "youtubeshorts" | "youtube shorts" | "shorts" => Some(YOUTUBE),
        "tiktok" => Some(TIKTOK),
        _ => None,
    }
}

impl Platform {
    /// The bottom of the bar at the top (fraction of the frame height).
    pub fn top(&self) -> f64 {
        self.zones[0].rect[3]
    }
    /// The top of the text at the bottom.
    pub fn bottom(&self) -> f64 {
        self.zones[1].rect[1]
    }
    pub fn buttons(&self) -> [f64; 4] {
        self.zones[2].rect
    }
    /// How wide centred text `half` tall either side of `y` may be (fraction of the frame
    /// width): narrower where the button column is.
    pub fn text_width(&self, y: f64, half: f64) -> f64 {
        let b = self.buttons();
        if y + half > b[1] && y - half < b[3] { (2.0 * (b[0] - 0.5 - 0.015)).clamp(0.3, 0.9) } else { 0.9 }
    }
    /// `y` moved so text `half` tall either side of it stays between the top bar and the text at
    /// the bottom.
    pub fn clamp_y(&self, y: f64, half: f64) -> f64 {
        let (lo, hi) = (self.top() + half + 0.01, self.bottom() - half - 0.01);
        if lo > hi { (lo + hi) / 2.0 } else { y.clamp(lo, hi) }
    }
    /// What to look out for with a short `seconds` long.
    pub fn warnings(&self, seconds: f64) -> Vec<String> {
        let mut v = Vec::new();
        if seconds > self.max_seconds {
            v.push(format!("{} takes shorts up to {}; this one is {}", self.name, clock(self.max_seconds), clock(seconds)));
        }
        if seconds < self.min_seconds {
            v.push(format!("{} needs at least {} seconds", self.name, self.min_seconds));
        }
        if let Some(m) = self.claimed_music_seconds
            && seconds > m
            && seconds <= self.max_seconds
        {
            v.push(format!("{} blocks shorts over {} that have a copyright claim: keep it under a minute if it uses copyrighted music", self.name, clock(m)));
        }
        v
    }
}

/// `m:ss` for a length in seconds.
pub fn clock(seconds: f64) -> String {
    let s = if seconds.is_finite() { seconds.max(0.0).round() as u64 } else { 0 };
    format!("{}:{:02}", s / 60, s % 60)
}

// ---------------------------------------------------------------------------------------------
// Formats
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Gameplay,
    FullFrame,
    Meme,
    Spotlight,
    Countdown,
    /// A reaction clip: split screen, captions on the seam, scene switches, hits, memes, a
    /// black-and-white ending ([`crate::shorts_reaction`]).
    Reaction,
}

impl Format {
    pub const ALL: [Format; 6] = [Format::Reaction, Format::Gameplay, Format::FullFrame, Format::Meme, Format::Spotlight, Format::Countdown];

    pub fn id(self) -> &'static str {
        match self {
            Format::Gameplay => "gameplay",
            Format::FullFrame => "fullFrame",
            Format::Meme => "meme",
            Format::Spotlight => "spotlight",
            Format::Countdown => "countdown",
            Format::Reaction => "reaction",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Gameplay => "Gameplay + Model",
            Format::FullFrame => "Full Frame + Blur",
            Format::Meme => "Meme Caption Bar",
            Format::Spotlight => "Model Spotlight",
            Format::Countdown => "Top Moments Countdown",
            Format::Reaction => "Reaction Split",
        }
    }

    pub fn parse(s: &str) -> Option<Format> {
        Format::ALL.into_iter().find(|f| f.id().eq_ignore_ascii_case(s.trim()))
    }

    fn preset(self) -> ly::Preset {
        match self {
            Format::Gameplay | Format::Countdown => ly::Preset::Stack,
            Format::FullFrame | Format::Meme => ly::Preset::Full,
            Format::Spotlight => ly::Preset::Spotlight,
            Format::Reaction => ly::Preset::Split,
        }
    }
}

/// Every option of a format.
#[derive(Clone, Debug)]
struct Opts {
    platform: Platform,
    format: Format,
    side: Option<String>,
    title: String,
    hook: bool,
    captions: Option<bool>,
    caption_style: Option<cap::Style>,
    progress: bool,
    /// Trim to this many seconds first (0: keep the length).
    seconds: f64,
    loop_end: bool,
    loop_seconds: f64,
    moments: usize,
    each: f64,
    background: Option<String>,
    /// The gameplay part of the recording, when not the layout's (Auto Edit cuts off a stream's
    /// burned-in captions this way).
    game: Option<[f64; 4]>,
    /// Bleep swear words (when the sequence has no bleeps yet).
    bleep: bool,
    /// Punch in and shake on the loud moments.
    hype: bool,
    /// Reaction Split: switch to full-screen gameplay and face, add bass hits and flashes, end in
    /// black and white, drop memes (from `memes_folder`, else the default meme folder).
    scenes: bool,
    hits: bool,
    ending: bool,
    memes: bool,
    memes_folder: Option<String>,
}

impl Opts {
    fn from_params(p: &Value, default: Option<(Platform, Format)>, cmd: &str) -> Result<Opts> {
        let platform = match str_p(p, "platform") {
            Some(id) => platform(id).ok_or_else(|| bad(cmd, format!("unknown platform `{id}` (youtube, tiktok)")))?,
            None => default.map_or(YOUTUBE, |d| d.0),
        };
        let format = match str_p(p, "format") {
            Some(id) => {
                Format::parse(id).ok_or_else(|| bad(cmd, format!("unknown format `{id}` (reaction, gameplay, fullFrame, meme, spotlight, countdown)")))?
            }
            None => default.map_or(Format::Gameplay, |d| d.1),
        };
        let side = match str_p(p, "side") {
            None => None,
            Some(s @ ("right" | "left")) => Some(s.to_string()),
            Some(o) => return Err(bad(cmd, format!("`side` must be right or left, not `{o}`"))),
        };
        let moments = num_p(p, "moments", 3.0, 2.0, 10.0, cmd)?.round() as usize;
        let title = match str_p(p, "title").map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) if t.chars().count() > 200 => return Err(bad(cmd, "keep the title under 200 characters")),
            Some(t) => t.to_string(),
            None if format == Format::Countdown => format!("TOP {moments} MOMENTS"),
            None => DEFAULT_TITLE.to_string(),
        };
        let caption_style = match str_p(p, "captionStyle").map(str::trim) {
            None | Some("" | "auto") => None,
            Some(s) => {
                Some(cap::Style::parse(s).ok_or_else(|| bad(cmd, format!("unknown caption style `{s}` (pop, karaoke, box, oneWord, plain, reaction)")))?)
            }
        };
        let seconds = num_p(p, "seconds", 0.0, 0.0, 600.0, cmd)?;
        if seconds > 0.0 && seconds < 3.0 {
            return Err(bad(cmd, "`seconds` must be 0 (keep the length) or at least 3"));
        }
        let background = match str_p(p, "background") {
            None => None,
            Some(b @ ("blur" | "black")) => Some(b.to_string()),
            Some(o) => return Err(bad(cmd, format!("`background` must be blur or black, not `{o}`"))),
        };
        let reaction = format == Format::Reaction;
        let memes_folder = match p.get("memesFolder") {
            None | Some(Value::Null) => None,
            Some(Value::String(f)) if f.trim().is_empty() => None,
            Some(Value::String(f)) if f.chars().count() <= 4096 => Some(f.trim().to_string()),
            Some(_) => return Err(bad(cmd, "`memesFolder` must be a folder path")),
        };
        Ok(Opts {
            platform,
            format,
            side,
            title,
            hook: bool_p(p, "hook").unwrap_or(matches!(format, Format::Gameplay | Format::Countdown)),
            captions: bool_p(p, "captions"),
            caption_style,
            progress: bool_p(p, "progressBar").unwrap_or(!reaction),
            seconds,
            loop_end: bool_p(p, "loop").unwrap_or(false),
            loop_seconds: num_p(p, "loopSeconds", 0.5, 0.1, 3.0, cmd)?,
            moments,
            each: num_p(p, "momentSeconds", 8.0, 3.0, 30.0, cmd)?,
            background,
            game: crate::shorts::fractions_p(p, "game", cmd)?,
            bleep: bool_p(p, "bleep").unwrap_or(reaction),
            hype: bool_p(p, "hype").unwrap_or(reaction),
            scenes: bool_p(p, "scenes").unwrap_or(true),
            hits: bool_p(p, "hits").unwrap_or(true),
            ending: bool_p(p, "ending").unwrap_or(true),
            memes: bool_p(p, "memes").unwrap_or(true),
            memes_folder,
        })
    }

    fn to_json(&self, captions: bool) -> Value {
        let mut v = json!({
            "platform": self.platform.id,
            "format": self.format.id(),
            "title": self.title,
            "hook": self.hook,
            "captions": captions,
            "progressBar": self.progress,
            "seconds": self.seconds,
            "loop": self.loop_end,
            "loopSeconds": self.loop_seconds,
            "moments": self.moments,
            "momentSeconds": self.each,
            "bleep": self.bleep,
            "hype": self.hype,
        });
        if self.format == Format::Reaction {
            v["scenes"] = json!(self.scenes);
            v["hits"] = json!(self.hits);
            v["ending"] = json!(self.ending);
            v["memes"] = json!(self.memes);
            if let Some(f) = &self.memes_folder {
                v["memesFolder"] = json!(f);
            }
        }
        if let Some(g) = self.game {
            v["game"] = json!(g);
        }
        if let Some(s) = &self.side {
            v["side"] = json!(s);
        }
        if let Some(st) = self.caption_style {
            v["captionStyle"] = json!(st.name());
        }
        if let Some(b) = &self.background {
            v["background"] = json!(b);
        }
        v
    }
}

/// The options the Formats… window opens with: the last ones applied, else Gameplay + Model for
/// YouTube Shorts (captions when there is a transcript).
pub fn current(s: &Session) -> Value {
    if let Some(v) = s.state.shorts_format.as_ref() {
        return v.clone();
    }
    let has_words = !crate::transcript::sequence_words(s).is_empty();
    match Opts::from_params(&json!({}), None, "shorts.format") {
        Ok(mut o) => {
            o.side = Some(layout_side(s));
            o.to_json(has_words)
        }
        Err(_) => Value::Null,
    }
}

/// The side of the recording the model is on, as the last layout had it (else the right).
fn layout_side(s: &Session) -> String {
    s.state.shorts_layout.as_ref().and_then(|v| v.get("side")).and_then(Value::as_str).unwrap_or("right").to_string()
}

// ---------------------------------------------------------------------------------------------
// Applying a format
// ---------------------------------------------------------------------------------------------

/// Where the layout put things, in fractions of the frame (`[x0, y0, x1, y1]`).
struct Placed {
    pane: [f64; 4],
    /// The meme bar.
    bar: Option<[f64; 4]>,
}

pub(crate) fn format(s: &mut Session, p: &Value, default: Option<(Platform, Format)>, cmd: &str) -> Result<Value> {
    let mut o = Opts::from_params(p, default, cmd)?;
    if o.side.is_none() {
        o.side = Some(layout_side(s));
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    if crate::shorts::base_track(&s.project, q).is_none() {
        return Err(bad(cmd, "the sequence has no video footage to make a short from"));
    }
    let vertical = q.settings.height > q.settings.width;
    let seq_name = s.state.active_sequence.and_then(|id| s.project.item(id)).map(|i| i.name.clone()).unwrap_or_else(|| "Sequence".into());
    let has_words = !crate::transcript::sequence_words(s).is_empty();
    let want_captions = match o.captions {
        Some(true) if !has_words => {
            return Err(bad(cmd, "no transcript yet: run Sequence ▸ Transcript ▸ Transcribe… first (or turn captions off)"));
        }
        Some(c) => c,
        None => has_words && o.format != Format::Meme,
    };
    let label = format!("{}: {}", o.platform.name, o.format.name());
    s.state.shorts_reaction = None;
    let moments = s.edit_group(&label, |s| {
        if vertical {
            clear_previous(s)?;
        } else {
            s.execute("shorts.verticalFill", json!({"name": format!("{seq_name} ({})", o.platform.name)}))?;
        }
        let mut moments = Vec::new();
        if o.format == Format::Reaction {
            if o.seconds > 0.0 {
                crate::shorts_trim::trim_to(s, Tick::from_seconds_f64(o.seconds), cmd)?;
            }
            reaction(s, &o, has_words, want_captions, cmd)?;
            return Ok(moments);
        }
        if o.format == Format::Countdown {
            moments = crate::shorts_trim::countdown_cut(s, o.moments, Tick::from_seconds_f64(o.each), cmd)?;
        } else if o.seconds > 0.0 {
            crate::shorts_trim::trim_to(s, Tick::from_seconds_f64(o.seconds), cmd)?;
        }
        let placed = arrange(s, &o, cmd)?;
        // swear words (once: a sequence bleeped before keeps its bleeps); none is fine
        if o.bleep && has_words && !crate::shorts_fx::has_bleeps(s) {
            let _ = s.execute("shorts.bleep", json!({}));
        }
        if o.loop_end {
            crate::shorts_trim::apply_loop(s, o.loop_seconds, cmd)?;
        }
        graphics(s, &o, &placed, &moments)?;
        // the loud moments, about one every 8 seconds; a clip with none stands out is fine too
        if o.hype {
            let secs = s.active_sequence().map(|q| q.duration().seconds()).unwrap_or(0.0);
            let n = (secs / 8.0).round().clamp(1.0, 12.0);
            let _ = s.execute("shorts.hype", json!({"effects": ["punch", "shake"], "maxMoments": n}));
        }
        if want_captions {
            captions(s, &o, &placed, cmd)?;
        }
        if o.progress {
            s.execute("shorts.progressBar", json!({"position": "top"}))?;
        }
        Ok(moments)
    })?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let secs = q.duration().seconds();
    let opts = o.to_json(want_captions);
    s.state.shorts_format = Some(opts.clone());
    let made = s.state.shorts_reaction.clone().unwrap_or(Value::Null);
    Ok(json!({
        "reaction": made,
        "sequence": s.state.active_sequence.map(|i| i.0),
        "platform": o.platform.id,
        "format": o.format.id(),
        "seconds": secs,
        "warnings": o.platform.warnings(secs),
        "preset": o.platform.preset,
        "captions": want_captions,
        "moments": moments.iter().map(|m| json!({"rank": m.rank, "start": m.range.start.0, "duration": m.range.duration.0, "db": m.excess_db})).collect::<Vec<_>>(),
        "options": opts,
    }))
}

/// Take away what an earlier format added to this (vertical) sequence: its graphics tracks and
/// loop ending. The layout is replaced when it is applied again.
fn clear_previous(s: &mut Session) -> Result<()> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let graphics: Vec<ClipId> = q
        .video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|i| s.project.item(i.item).is_some_and(|x| matches!(x.kind, ItemKind::Graphic { .. })))
        .map(|i| i.id)
        .collect();
    let pr = s.project.clone();
    s.edit_sequence("Clear Format", |q, _, _| {
        q.video_tracks.retain(|t| !(GENERATED.contains(&t.name.as_str()) && t.items.iter().all(|i| graphics.contains(&i.id))));
        crate::shorts_trim::remove_loop(q);
        crate::shorts_reaction::clear(q, &pr);
        q.check().map_err(EngineError::Other)?;
        Ok(())
    })
}

/// Lay the footage out for the format, clear of the platform's top bar.
fn arrange(s: &mut Session, o: &Opts, cmd: &str) -> Result<Placed> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h) = (q.settings.width, q.settings.height);
    let frame = (f64::from(w), f64::from(h));
    if frame.0 < 16.0 || frame.1 < 16.0 {
        return Err(bad(cmd, "the sequence frame is too small"));
    }
    let src = ly::source_size(s).ok_or_else(|| bad(cmd, "the sequence has no video footage to lay out"))?;
    let preset = o.format.preset();
    let prev = s.state.shorts_layout.clone().unwrap_or(Value::Null);
    let side = o.side.clone().unwrap_or_else(|| layout_side(s));
    let mut base = json!({"preset": preset.name(), "side": side});
    // the gameplay and model areas picked earlier in Layout… stay (the full-frame formats show
    // the whole recording)
    if preset != ly::Preset::Full {
        for k in ["game", "model"] {
            if let Some(v) = prev.get(k) {
                base[k] = v.clone();
            }
        }
    }
    if let Some(g) = o.game {
        base["game"] = json!(g);
    }
    if o.format == Format::Meme {
        base["background"] = json!("black");
    }
    if o.format == Format::Reaction {
        // two halves, nothing behind them; the model's head and shoulders fill the bottom half
        base["split"] = json!(0.5);
        base["background"] = json!("black");
        let model = base.get("model").and_then(|m| serde_json::from_value::<[f64; 4]>(m.clone()).ok()).unwrap_or_else(|| ly::model_region(side_of(&side)));
        base["model"] = json!(crate::shorts_reaction::top_anchored(model, src, frame.0 / (frame.1 * 0.5)));
    }
    if let Some(b) = &o.background {
        base["background"] = json!(b);
    }
    let mut spec = ly::Spec::from_params(&base, frame, src, cmd, (preset, ly::Side::Right))?;
    let (pane, _) = spec.rects(frame, src);
    let ph = pane.h / frame.1;
    let top = o.platform.top() + 0.01;
    let mut bar = None;
    match o.format {
        // the split of the frame in half, as it is
        Format::Reaction => {}
        Format::Gameplay | Format::Countdown | Format::Spotlight => {
            spec.game_y = top + ph / 2.0;
            spec.model_y = (top + ph + 0.015 + spec.model_h / 2.0).min(1.0 - spec.model_h / 2.0);
        }
        Format::FullFrame => {
            spec.game_y = ((o.platform.top() + o.platform.bottom()) / 2.0).clamp(ph / 2.0, 1.0 - ph / 2.0);
        }
        Format::Meme => {
            let text_h = meme_text(&o.title, (w, h), Vec2::new(frame.0 / 2.0, frame.1 / 2.0)).1 / frame.1;
            let pad = 0.025;
            let bar_h = text_h + 2.0 * pad;
            let centre = (o.platform.top() + o.platform.bottom()) / 2.0;
            // the block (bar and picture) centred between the app's bars, the text clear of the top
            let block_top = (centre - (bar_h + ph) / 2.0).max(o.platform.top() - pad).max(0.0);
            spec.game_y = (block_top + bar_h + ph / 2.0).min(1.0 - ph / 2.0);
            bar = Some([0.0, block_top, 1.0, block_top + bar_h]);
        }
    }
    ly::layout(s, &spec.to_json(), (preset, ly::Side::Right), cmd)?;
    let (pane, _) = spec.rects(frame, src);
    Ok(Placed { pane: [pane.x / frame.0, pane.y / frame.1, (pane.x + pane.w) / frame.0, (pane.y + pane.h) / frame.1], bar })
}

// ---------------------------------------------------------------------------------------------
// Graphics
// ---------------------------------------------------------------------------------------------

/// `text` broken into lines of about `width` characters at spaces (at most `lines` lines; the
/// rest stays on the last one). Lines the writer broke stay broken.
fn wrap(text: &str, width: usize, lines: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width && out.len() + 1 < lines {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if out.len() < lines {
            out.push(line);
        } else if let Some(last) = out.last_mut() {
            last.push(' ');
            last.push_str(&line);
        }
    }
    out.join("\n")
}

/// A centred text layer of Inter Bold at `at`, `size` pixels, with the anchor on the middle of
/// the text (so it scales from there).
fn text_layer(text: &str, at: Vec2, size: f64, fill: [f32; 4], outline: Option<f64>, frame: (u32, u32), name: &str) -> (EffectInstance, f64, f64) {
    let mut layer = filmcraft_project::graphic::new_text_layer(text, at, size);
    for (k, v) in [
        ("name", ParamValue::Text(name.into())),
        ("font", ParamValue::Text("Inter".into())),
        ("font_style", ParamValue::Text("Bold".into())),
        ("align", ParamValue::Choice(1)),
        ("fill", ParamValue::Bool(true)),
        ("fill_color", ParamValue::Color(fill)),
        ("stroke", ParamValue::Bool(outline.is_some())),
        ("stroke_color", ParamValue::Color([0.0, 0.0, 0.0, 1.0])),
        ("stroke_width", ParamValue::Float(size * outline.unwrap_or(0.0) / 100.0)),
        ("stroke_type", ParamValue::Choice(0)),
        ("shadow", ParamValue::Bool(outline.is_some())),
        ("shadow_opacity", ParamValue::Float(60.0)),
        ("shadow_distance", ParamValue::Float(size * 0.06)),
        ("shadow_blur", ParamValue::Float(size * 0.25)),
    ] {
        set_static(&mut layer, k, v);
    }
    let (mut tw, mut th) = (0.0, 0.0);
    if let Some((b, _)) = cap::measure(&layer, frame, None) {
        set_static(&mut layer, "anchor", ParamValue::Vec2(Vec2::new(f64::from(b[0] + b[2]) / 2.0, f64::from(b[1] + b[3]) / 2.0)));
        tw = f64::from(b[2] - b[0]).max(0.0);
        th = f64::from(b[3] - b[1]).max(0.0);
    }
    (layer, tw, th)
}

/// A text layer at most `max_w` x `max_h` pixels (shrunk to fit). Returns it and its height.
fn fitted(
    text: &str,
    at: Vec2,
    size: f64,
    fill: [f32; 4],
    outline: Option<f64>,
    frame: (u32, u32),
    name: &str,
    max_w: f64,
    max_h: f64,
) -> (EffectInstance, f64) {
    let (layer, tw, th) = text_layer(text, at, size, fill, outline, frame, name);
    let k = [if tw > max_w && tw > 0.0 { max_w / tw } else { 1.0 }, if th > max_h && th > 0.0 { max_h / th } else { 1.0 }].into_iter().fold(1.0f64, f64::min);
    if k < 0.999 {
        let (layer, _, th) = text_layer(text, at, (size * k).max(6.0), fill, outline, frame, name);
        return (layer, th);
    }
    (layer, th)
}

/// The meme bar's text (black, as typed) and its height in pixels.
fn meme_text(text: &str, frame: (u32, u32), at: Vec2) -> (EffectInstance, f64) {
    let fw = f64::from(frame.0);
    let px = fw / 1080.0;
    fitted(&wrap(text, 24, 4), at, 64.0 * px, [0.0, 0.0, 0.0, 1.0], None, frame, "Meme Text", fw * 0.9, f64::from(frame.1) * 0.3)
}

/// Scale keyframes for an entrance: `points` of (frame, percent), clip-relative.
fn entrance(frame: Tick, points: &[(i64, f64)]) -> Param {
    let mut p = Param::new(ParamValue::Float(100.0));
    p.keyframes = points.iter().map(|(n, v)| Keyframe::new(after(Tick::ZERO, *n, frame), ParamValue::Float(*v))).collect();
    p
}

fn graphics(s: &mut Session, o: &Opts, placed: &Placed, moments: &[Moment]) -> Result<()> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let px = fw / 1080.0;
    let frame = rate.frame_duration().max(Tick(1));
    let dur = rate.snap(q.duration());
    if dur < frame {
        return Ok(());
    }
    let pf = &o.platform;
    match o.format {
        Format::Meme => {
            if let Some(b) = placed.bar {
                let centre = Vec2::new(fw / 2.0, (b[1] + b[3]) / 2.0 * fh);
                let mut bar = filmcraft_project::graphic::new_shape_layer(0, centre, Vec2::new(fw + 8.0, (b[3] - b[1]) * fh), Vec::new());
                set_static(&mut bar, "name", ParamValue::Text("Bar".into()));
                set_static(&mut bar, "fill", ParamValue::Bool(true));
                set_static(&mut bar, "fill_color", ParamValue::Color([1.0, 1.0, 1.0, 1.0]));
                set_static(&mut bar, "stroke", ParamValue::Bool(false));
                let (text, _) = meme_text(&o.title, (w, h), centre);
                place_graphic_track(s, "Meme Caption Bar", BAR_TRACK, vec![(Tick::ZERO, dur, o.title.clone(), vec![bar, text])])?;
            }
        }
        Format::FullFrame => {
            // the title in the space between the top bar and the picture, for the whole short
            let (lo, hi) = (pf.top() + 0.01, (placed.pane[1] - 0.01).max(pf.top() + 0.06));
            let at = Vec2::new(fw / 2.0, (lo + hi) / 2.0 * fh);
            let (mut title, _) = fitted(&wrap(&o.title, 16, 3), at, 84.0 * px, [1.0, 1.0, 1.0, 1.0], Some(14.0), (w, h), "Title", fw * 0.9, (hi - lo) * fh);
            title.params.insert("scale".into(), entrance(frame, &[(0, 75.0), (2, 112.0), (4, 100.0)]));
            place_graphic_track(s, "Title", TITLE_TRACK, vec![(Tick::ZERO, dur, o.title.clone(), vec![title])])?;
        }
        Format::Countdown => {
            let mut badges: Vec<GraphicClip> = Vec::new();
            let mut flashes: Vec<GraphicClip> = Vec::new();
            let flash_dur = rate.snap_nearest(Tick::from_seconds_f64(0.25)).max(frame);
            // under the header, over the top left of the gameplay
            let y = (placed.pane[1] + 0.13).max(pf.top() + 0.155);
            for (i, m) in moments.iter().enumerate() {
                let start = rate.snap(m.range.start);
                let len = rate.snap(m.range.duration).max(frame);
                let text = format!("#{}", m.rank);
                let (mut badge, _, _) = text_layer(&text, Vec2::new(0.17 * fw, y * fh), 150.0 * px, [1.0, 0.84, 0.04, 1.0], Some(12.0), (w, h), "Rank");
                badge.params.insert("scale".into(), entrance(frame, &[(0, 40.0), (3, 115.0), (6, 100.0)]));
                badges.push((start, len, text, vec![badge]));
                if i > 0 {
                    flashes.push((start, flash_dur, "Flash".into(), vec![flash_layer(w, h, [1.0, 1.0, 1.0, 1.0], flash_dur, 70.0)]));
                }
            }
            if !badges.is_empty() {
                place_graphic_track(s, "Top Moments Countdown", COUNT_TRACK, badges)?;
            }
            if !flashes.is_empty() {
                place_graphic_track(s, "Top Moments Countdown", COUNT_FLASH_TRACK, flashes)?;
            }
            hook(s, o)?;
        }
        Format::Gameplay | Format::Spotlight | Format::Reaction => {
            if o.hook {
                hook(s, o)?;
            }
        }
    }
    Ok(())
}

fn side_of(side: &str) -> ly::Side {
    if side == "left" { ly::Side::Left } else { ly::Side::Right }
}

/// The Reaction Split, after any trim: the layout, bleeps, the planned scenes, memes, hits and
/// grades, punch-ins and captions (see [`crate::shorts_reaction`]). What was made goes to
/// `shorts_reaction` in the editor state.
fn reaction(s: &mut Session, o: &Opts, has_words: bool, want_captions: bool, cmd: &str) -> Result<()> {
    use crate::shorts_reaction as rx;
    // the model region before it is cut to the half's shape (the face scene cuts its own): one
    // picked in Layout… for another layout, else the usual corner (a split's own region is
    // already cut)
    let side = o.side.clone().unwrap_or_else(|| layout_side(s));
    let uncut_model = s
        .state
        .shorts_layout
        .as_ref()
        .filter(|v| v.get("preset").and_then(Value::as_str) != Some("split"))
        .and_then(|v| v.get("model"))
        .and_then(|m| serde_json::from_value::<[f64; 4]>(m.clone()).ok())
        .unwrap_or_else(|| ly::model_region(side_of(&side)));
    let placed = arrange(s, o, cmd)?;
    if o.bleep && has_words && !crate::shorts_fx::has_bleeps(s) {
        let _ = s.execute("shorts.bleep", json!({}));
    }
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (fw, fh) = (f64::from(q.settings.width), f64::from(q.settings.height));
    let dur = q.duration().seconds();
    let words = if has_words { rx::words_of(s) } else { Vec::new() };
    let (game, _, src) = rx::regions(s).ok_or_else(|| bad(cmd, "the sequence has no video footage to lay out"))?;
    let activity = if o.scenes { rx::activity(s, game) } else { Vec::new() };
    let plan = rx::plan(&words, dur, &activity, rx::Choices { scenes: o.scenes, hits: o.hits, ending: o.ending });
    let face_region = rx::top_anchored(uncut_model, src, fw / fh.max(1.0));
    rx::apply_scenes(s, &plan, face_region, game)?;
    // memes, from the folder named or the default one
    let folder = o.memes_folder.as_ref().map(std::path::PathBuf::from).or_else(crate::shorts_memes::default_folder);
    let mut drops = Vec::new();
    if o.memes
        && has_words
        && let Some(f) = &folder
    {
        drops = crate::shorts_memes::drop_all(s, f, 0.36, plan.ending, cmd)?;
    }
    let hit_sound = folder.as_deref().and_then(crate::shorts_memes::hit_sound);
    let hits = if o.hits { rx::add_hits(s, &plan.hits, hit_sound.as_deref())? } else { 0 };
    rx::add_grades(s, &plan)?;
    if let Some(t) = plan.flash {
        let r = s.execute("shorts.flash", json!({"time": Tick::from_seconds_f64(t).0, "seconds": 0.35}))?;
        if let Some(id) = r.get("clip").and_then(Value::as_u64).map(ClipId) {
            s.edit_sequence("Punchline Flash", |q, _, _| {
                if let Some(it) = q.video_tracks.iter_mut().flat_map(|t| t.items.iter_mut()).find(|i| i.id == id) {
                    it.name = rx::FLASH_NAME.into();
                }
                Ok(())
            })?;
        }
    }
    // punch-ins on the loud moments, about one every 8 seconds (none standing out is fine)
    if o.hype {
        let n = (dur / 8.0).round().clamp(1.0, 12.0);
        let _ = s.execute("shorts.hype", json!({"effects": ["punch"], "maxMoments": n}));
    }
    if want_captions {
        let mut p = s.state.shorts_captions.clone().filter(|v| v.get("style").and_then(Value::as_str) == Some("reaction")).unwrap_or_else(|| json!({}));
        p["style"] = json!(o.caption_style.map_or("reaction", |st| st.name()));
        // on the seam between the halves
        let seam = (placed.pane[3]).clamp(0.2, 0.8);
        let half = 0.035;
        p["x"] = json!(0.5);
        p["y"] = json!(o.platform.clamp_y(seam, half));
        p["width"] = json!(o.platform.text_width(seam, half));
        p["replace"] = json!(true);
        let colors: Vec<String> = drops.iter().map(|(w, _, _)| format!("{w}={}", crate::shorts_memes::COLOR)).collect();
        p["wordColors"] = json!(colors.join(", "));
        if let Some(m) = p.as_object_mut() {
            m.remove("live");
            m.remove("position");
        }
        cap::captions(s, &p, cap::Style::Reaction, cmd)?;
        // under the face where it fills the frame
        if let Some(face) = plan.face() {
            let y = o.platform.clamp_y(o.platform.bottom() - 0.07, half);
            rx::move_captions(s, &[face], y)?;
        }
    }
    if o.hook {
        hook(s, o)?;
    }
    if o.progress {
        s.execute("shorts.progressBar", json!({"position": "top"}))?;
    }
    let scenes: Vec<Value> = plan
        .segments
        .iter()
        .map(|g| json!({"start": g.start, "end": g.end, "scene": match g.scene { rx::Scene::Split => "split", rx::Scene::Game => "game", rx::Scene::Face => "face" }}))
        .collect();
    s.state.shorts_reaction = Some(json!({
        "scenes": scenes,
        "hits": hits,
        "flash": plan.flash,
        "gray": plan.gray.map(|(a, b)| [a, b]),
        "ending": plan.ending.map(|(a, b)| [a, b]),
        "memes": drops.iter().map(|(w, f, t)| json!({"word": w, "file": f, "seconds": t})).collect::<Vec<_>>(),
        "memesFolder": folder.map(|f| f.to_string_lossy().to_string()),
        "shouted": words.iter().filter(|w| w.loud).count(),
    }));
    Ok(())
}

/// The hook title (the countdown's header) under the platform's top bar.
fn hook(s: &mut Session, o: &Opts) -> Result<()> {
    s.execute("shorts.hookTitle", json!({"text": o.title, "seconds": 2.5, "y": o.platform.top() + 0.065, "width": 0.9}))?;
    Ok(())
}

/// Word Captions where the format wants them, inside the platform's clear area, with the options
/// last used in Word Captions… (and their style, unless the format was given one or is the
/// spotlight).
fn captions(s: &mut Session, o: &Opts, placed: &Placed, cmd: &str) -> Result<()> {
    let mut p = s.state.shorts_captions.clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
    // the spotlight's look is one big word at a time
    let style = o.caption_style.or_else(|| (o.format == Format::Spotlight).then_some(cap::Style::OneWord));
    if let Some(st) = style {
        p["style"] = json!(st.name());
    }
    let lines = p.get("lines").and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(1.0).clamp(1.0, 3.0);
    let half = 0.03 * lines + 0.005;
    let y = match o.format {
        Format::Gameplay | Format::Countdown => placed.pane[3] - 0.005,
        Format::Reaction => placed.pane[3],
        // where the card meets the model: over the model's face would hide it
        Format::Spotlight => placed.pane[3] + 0.01,
        Format::FullFrame | Format::Meme => placed.pane[3] + 0.055,
    };
    let y = o.platform.clamp_y(y, half);
    p["x"] = json!(0.5);
    p["y"] = json!(y);
    p["width"] = json!(o.platform.text_width(y, half));
    p["replace"] = json!(true);
    if let Some(m) = p.as_object_mut() {
        m.remove("live");
        m.remove("position");
    }
    cap::captions(s, &p, style.unwrap_or(cap::Style::Pop), cmd)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_spaces_within_the_line_count() {
        assert_eq!(wrap("when chat finds out what happened", 12, 4), "when chat\nfinds out\nwhat\nhappened");
        assert_eq!(wrap("one two three four five six", 7, 2), "one two\nthree four five six");
        assert_eq!(wrap("kept\nbreak", 40, 3), "kept\nbreak");
        assert_eq!(wrap("", 10, 2), "");
        assert_eq!(wrap("aVeryLongWordWithNoSpaces", 5, 3), "aVeryLongWordWithNoSpaces");
    }

    #[test]
    fn platforms_keep_text_out_of_the_app() {
        for pf in PLATFORMS {
            // between the bars, and narrower beside the buttons
            assert!(pf.top() > 0.05 && pf.bottom() < 0.85);
            let y = pf.clamp_y(0.99, 0.03);
            assert!(y + 0.03 < pf.bottom());
            assert!(pf.clamp_y(0.0, 0.03) - 0.03 > pf.top());
            let beside = pf.text_width((pf.buttons()[1] + pf.buttons()[3]) / 2.0, 0.03);
            assert!(0.5 + beside / 2.0 < pf.buttons()[0], "{}: {beside}", pf.id);
            assert_eq!(pf.text_width(0.2, 0.03), 0.9);
            // a band too thin for the text: the middle of it, not a panic
            assert!(pf.clamp_y(0.5, 0.6).is_finite());
        }
        assert_eq!(platform("TikTok"), Some(TIKTOK));
        assert_eq!(platform("youtube"), Some(YOUTUBE));
        assert_eq!(platform("myspace"), None);
    }

    #[test]
    fn length_warnings() {
        assert!(YOUTUBE.warnings(45.0).is_empty());
        assert_eq!(YOUTUBE.warnings(75.0).len(), 1, "a minute and a bit: the music claim rule");
        assert!(YOUTUBE.warnings(200.0).iter().any(|w| w.contains("3:00")));
        assert!(TIKTOK.warnings(2.0).iter().any(|w| w.contains("at least 3")));
        assert!(TIKTOK.warnings(120.0).is_empty());
        assert_eq!(clock(75.4), "1:15");
        assert_eq!(clock(f64::NAN), "0:00");
    }

    #[test]
    fn options_read_back_and_reject_hostile_values() {
        let o = Opts::from_params(&json!({"platform": "tiktok", "format": "countdown", "moments": 4}), None, "t").unwrap();
        assert_eq!((o.platform, o.format, o.moments), (TIKTOK, Format::Countdown, 4));
        assert_eq!(o.title, "TOP 4 MOMENTS");
        let again = Opts::from_params(&o.to_json(true), None, "t").unwrap();
        assert_eq!(again.to_json(true), o.to_json(true));
        for bad in [
            json!({"platform": "vine"}),
            json!({"format": "wavy"}),
            json!({"side": "up"}),
            json!({"seconds": 1}),
            json!({"seconds": -5}),
            json!({"seconds": 1e9}),
            json!({"moments": 50}),
            json!({"momentSeconds": 0}),
            json!({"captionStyle": "comic"}),
            json!({"background": "plaid"}),
            json!({"title": "x".repeat(300)}),
        ] {
            assert!(Opts::from_params(&bad, None, "t").is_err(), "{bad}");
        }
    }
}
