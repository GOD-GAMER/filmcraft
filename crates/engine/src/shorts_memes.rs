//! Shorts ▸ Meme Drops: your own memes (pictures, GIFs, videos and sounds) dropped into a short
//! when their word is said.
//!
//! FilmCraft ships no memes (they belong to other people); you keep yours in a folder, by default
//! `<FilmCraft data folder>/Memes` (Shorts ▸ Meme Drops ▸ Open Meme Folder). A file's name says
//! when it plays: `huh.mp4` drops on "huh", `what the hell.png` on "what the hell", and
//! `bruh, bro.gif` on either word (a comma or semicolon separates alternatives; dashes and
//! underscores count as spaces). A sound named `boom` (or `bass hit`, `hit`) replaces the built-in
//! bass hit of the Reaction Split format and is never dropped on a word.
//!
//! A picture, GIF or video shows over the middle of the short for as long as it lasts (at most
//! 2.4 s; a still picture 1.3 s), faded in and out, with its sound; a sound file just plays. The
//! word's caption takes the meme colour (purple). Drops are at least 4 s apart, at most one per
//! 5 s of short, and each meme drops once.

use std::path::{Path, PathBuf};

use filmcraft_edit::shorts as sh;
use filmcraft_edit::transcript::SeqWord;
use filmcraft_project::{ItemId, ItemKind, MediaRef, TrackKind};
use filmcraft_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{bad, str_p};
use crate::shorts::new_video_track;
use crate::{EngineError, Result, Session};

/// Names of the clips a drop makes start with this (their sounds are left out of what Shorts
/// measures of the speaker's voice).
pub(crate) const PREFIX: &str = "Meme: ";
pub(crate) const TRACK: &str = "Memes";
pub(crate) const SOUND_TRACK: &str = "Meme Sounds";
/// The colour of a meme word's caption.
pub(crate) const COLOR: &str = "#7a5cfa";
/// Files looked at in a meme folder.
const MAX_FILES: usize = 500;
/// The longest a picture meme shows, and a still one.
const MAX_SECONDS: f64 = 2.4;
const STILL_SECONDS: f64 = 1.3;
/// Stems of a sound that stands in for the bass hit.
const HIT_NAMES: [&str; 4] = ["boom", "basshit", "bass", "hit"];

/// The meme folder used when none is given: `<data folder>/Memes`.
pub fn default_folder() -> Option<PathBuf> {
    crate::autosave::default_data_dir().map(|d| d.join("Memes"))
}

/// What a meme file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A still picture.
    Still,
    /// A GIF or a video (it may have sound).
    Moving,
    /// Only a sound.
    Sound,
}

/// A meme file and the phrases (normalized words) that drop it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Meme {
    pub path: PathBuf,
    pub kind: Kind,
    pub phrases: Vec<Vec<String>>,
}

fn kind_of(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext == "gif" {
        return Some(Kind::Moving);
    }
    if filmcraft_media::STILL_EXTENSIONS.contains(&ext.as_str()) {
        Some(Kind::Still)
    } else if filmcraft_media::AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        Some(Kind::Sound)
    } else if filmcraft_media::VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        Some(Kind::Moving)
    } else {
        None
    }
}

/// The phrases a file name stands for: `bruh, bro` → [["bruh"], ["bro"]], `what-the_hell` →
/// [["what", "the", "hell"]].
pub(crate) fn phrases_of(stem: &str) -> Vec<Vec<String>> {
    stem.split([',', ';'])
        .map(|alt| {
            alt.split(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '.' | '+')).map(sh::normalize_word).filter(|w| !w.is_empty()).collect::<Vec<_>>()
        })
        .filter(|p| !p.is_empty())
        .collect()
}

fn is_hit_name(stem: &str) -> bool {
    let n: String = sh::normalize_word(&stem.replace([' ', '-', '_'], ""));
    HIT_NAMES.contains(&n.as_str())
}

/// The memes in `folder` (not its subfolders), by name; a missing or unreadable folder has none.
pub(crate) fn scan(folder: &Path) -> Vec<Meme> {
    let Ok(dir) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<Meme> = dir
        .filter_map(std::result::Result::ok)
        .take(MAX_FILES)
        .filter_map(|e| {
            let path = e.path();
            if !path.is_file() {
                return None;
            }
            let stem = path.file_stem()?.to_str()?.to_string();
            if stem.starts_with('.') || is_hit_name(&stem) {
                return None;
            }
            let kind = kind_of(&path)?;
            let phrases = phrases_of(&stem);
            (!phrases.is_empty()).then_some(Meme { path, kind, phrases })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// A sound in `folder` named like a bass hit (`boom.wav`), to use instead of the built-in one.
pub(crate) fn hit_sound(folder: &Path) -> Option<PathBuf> {
    let dir = std::fs::read_dir(folder).ok()?;
    let mut found: Vec<PathBuf> = dir
        .filter_map(std::result::Result::ok)
        .take(MAX_FILES)
        .map(|e| e.path())
        .filter(|p| p.is_file() && kind_of(p) == Some(Kind::Sound) && p.file_stem().and_then(|s| s.to_str()).is_some_and(is_hit_name))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Where memes drop: (meme, the word it drops on), in time order. Each meme once, drops at least
/// `min_gap` seconds apart, at most `max` of them, none in `avoid` (the ending).
pub(crate) fn matches(words: &[SeqWord], memes: &[Meme], min_gap: f64, max: usize, avoid: Option<(f64, f64)>) -> Vec<(usize, usize)> {
    let norm: Vec<String> = words.iter().map(|w| sh::normalize_word(&w.text)).collect();
    let mut found: Vec<(usize, usize)> = Vec::new();
    for (wi, _) in words.iter().enumerate() {
        for (mi, m) in memes.iter().enumerate() {
            let hit = m.phrases.iter().any(|p| p.iter().enumerate().all(|(k, pw)| norm.get(wi + k) == Some(pw)));
            if hit {
                found.push((mi, wi));
            }
        }
    }
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut used: Vec<usize> = Vec::new();
    for (mi, wi) in found {
        if out.len() >= max || used.contains(&mi) {
            continue;
        }
        let Some(t) = words.get(wi).map(|w| w.start.seconds()) else { continue };
        if avoid.is_some_and(|(a, b)| t >= a && t < b) {
            continue;
        }
        if out.iter().any(|(_, w)| words.get(*w).is_some_and(|x| (x.start.seconds() - t).abs() < min_gap)) {
            continue;
        }
        used.push(mi);
        out.push((mi, wi));
    }
    out
}

/// The project item of a file: the one already imported from that path, else a new import.
pub(crate) fn item_for(s: &mut Session, path: &Path) -> Result<ItemId> {
    let want = path.to_string_lossy().to_string();
    let existing =
        s.project.items.values().find(|i| matches!(&i.kind, ItemKind::Media(m) if matches!(&m.media, MediaRef::File { path } if *path == want))).map(|i| i.id);
    if let Some(id) = existing {
        return Ok(id);
    }
    let r = s.execute("file.import", json!({ "paths": [want] }))?;
    r["items"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(Value::as_u64)
        .map(ItemId)
        .ok_or_else(|| EngineError::Other(format!("could not import {}", path.display())))
}

/// One drop: `path` at sequence time `t`, centred at `y` (fraction of the frame height). Returns
/// how long it shows.
fn place(s: &mut Session, path: &Path, kind: Kind, t: Tick, y: f64, word: &str, label: &str) -> Result<Tick> {
    let item = item_for(s, path)?;
    let pi = s.project.item(item).ok_or_else(|| bad(label, "the meme could not be read"))?.clone();
    let (has_video, has_audio) = s.project.source_streams(item);
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (fw, fh, rate) = (f64::from(q.settings.width), f64::from(q.settings.height), q.settings.frame_rate);
    let end = q.duration();
    let media = pi.duration();
    let longest = Tick::from_seconds_f64(if kind == Kind::Still { STILL_SECONDS } else { MAX_SECONDS });
    let dur = if kind == Kind::Still || media <= Tick::ZERO { longest } else { media.min(longest) };
    let dur = rate.snap(dur.min(Tick(end.0.saturating_sub(t.0)))).max(rate.frame_duration());
    if t >= end {
        return Ok(Tick::ZERO);
    }
    let size = match &pi.kind {
        ItemKind::Media(m) => m.info.video.as_ref().map(|v| (f64::from(v.width.max(1)), f64::from(v.height.max(1)))),
        _ => None,
    };
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let name = format!("{PREFIX}{word}");
    let frame = rate.frame_duration().max(Tick(1));
    s.edit(label, |pr, _| {
        let q = pr.sequence(seq_id).ok_or(EngineError::NoSequence)?;
        let rate = q.settings.frame_rate;
        let mut video = None;
        if has_video
            && kind != Kind::Sound
            && let Some(mut ti) = pr.make_track_item(item, TrackKind::Video, t, TimeRange::new(Tick::ZERO, dur), rate)
        {
            ti.name = name.clone();
            // inside 62 % of the width and 40 % of the height, over the middle
            let (w, h) = size.unwrap_or((fw, fh));
            let (w, h) = (w.max(1.0), h.max(1.0));
            let k = (fw * 0.62 / w).min(fh * 0.40 / h) * 100.0;
            crate::shorts::set_motion(&mut ti, k, (fw / 2.0, y * fh), (w / 2.0, h / 2.0));
            // faded in and out over three frames, a little see-through
            if let Some(op) = ti.effect_mut("opacity").and_then(|e| e.params.get_mut("opacity")) {
                let last = Tick(dur.0.saturating_sub(frame.0));
                for (at, v) in [
                    (Tick::ZERO, 0.0),
                    (crate::shorts::after(Tick::ZERO, 3, frame).min(last), 92.0),
                    (Tick(last.0.saturating_sub(frame.0.saturating_mul(3))).max(Tick::ZERO), 92.0),
                    (last, 0.0),
                ] {
                    crate::shorts::put(op, at, v);
                }
            }
            video = Some(ti);
        }
        let sound = if has_audio {
            pr.make_track_item(item, TrackKind::Audio, t, TimeRange::new(Tick::ZERO, dur), rate).map(|mut ti| {
                ti.name = name.clone();
                ti
            })
        } else {
            None
        };
        let ids = (pr.alloc_id(), pr.alloc_id());
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        if let Some(ti) = video {
            let range = ti.range();
            // a free slot on a Memes track (under the captions and graphics), else a new one
            let slot = q.video_tracks.iter().position(|tr| tr.name == TRACK && !tr.items.iter().any(|i| i.range().overlaps(&range)));
            let i = match slot {
                Some(i) => i,
                None => {
                    let at = q.video_tracks.iter().position(|tr| tr.name == crate::shorts_captions::TRACK).unwrap_or(q.video_tracks.len());
                    q.video_tracks.insert(at, new_video_track(ids.0, TRACK));
                    at
                }
            };
            if let Some(tr) = q.video_tracks.get_mut(i) {
                tr.items.push(ti);
                tr.sort();
            }
        }
        if let Some(ti) = sound {
            let range = ti.range();
            let slot = q.audio_tracks.iter().position(|tr| tr.name == SOUND_TRACK && !tr.items.iter().any(|i| i.range().overlaps(&range)));
            let i = match slot {
                Some(i) => i,
                None => {
                    q.audio_tracks.push(filmcraft_project::Track::new(filmcraft_project::TrackId(ids.1), TrackKind::Audio, SOUND_TRACK.into()));
                    q.audio_tracks.len() - 1
                }
            };
            if let Some(tr) = q.audio_tracks.get_mut(i) {
                tr.items.push(ti);
                tr.sort();
            }
        }
        q.check().map_err(EngineError::Other)?;
        Ok(())
    })?;
    Ok(dur)
}

/// Drop the memes of `folder` on the words that name them. `avoid`: a stretch (seconds) to keep
/// clear (the ending). Returns the drops (word, file, time) and the words to colour.
pub(crate) fn drop_all(s: &mut Session, folder: &Path, y: f64, avoid: Option<(f64, f64)>, label: &str) -> Result<Vec<(String, String, f64)>> {
    let memes = scan(folder);
    if memes.is_empty() {
        return Ok(Vec::new());
    }
    let words = crate::transcript::sequence_words(s);
    let secs = s.active_sequence().map(|q| q.duration().seconds()).unwrap_or(0.0);
    let max = ((secs / 5.0).floor() as usize).clamp(1, 12);
    let picks = matches(&words, &memes, 4.0, max, avoid);
    let mut out = Vec::new();
    for (mi, wi) in picks {
        let (Some(m), Some(w)) = (memes.get(mi), words.get(wi)) else { continue };
        // a beat early, so the picture is there when the word lands
        let t = Tick(w.start.0.saturating_sub(Tick::from_seconds_f64(0.05).0));
        let word = sh::normalize_word(&w.text);
        // a file that cannot be read is skipped, not the whole short
        if place(s, &m.path, m.kind, t, y, &word, label).is_ok() {
            out.push((word, m.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), t.seconds()));
        }
    }
    Ok(out)
}

/// The meme folder a command names (`folder`), else the default one.
pub(crate) fn folder_p(p: &Value) -> Option<PathBuf> {
    match str_p(p, "folder").map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => Some(PathBuf::from(f)),
        None => default_folder(),
    }
}

/// Shorts ▸ Meme Drops ▸ Drop Memes on Their Words.
pub(crate) fn drop_memes(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.memeDrops";
    let folder = folder_p(p).ok_or_else(|| bad(CMD, "there is no meme folder (give `folder`)"))?;
    if crate::transcript::sequence_words(s).is_empty() {
        return Err(bad(CMD, "no transcript yet: run Sequence ▸ Transcript ▸ Transcribe… first so FilmCraft knows when each word is said"));
    }
    if scan(&folder).is_empty() {
        return Err(bad(CMD, format!("no memes in {} (name each file after its word: huh.mp4, bruh.png…)", folder.display())));
    }
    let drops = s.edit_group("Meme Drops", |s| drop_all(s, &folder, 0.36, None, CMD))?;
    if drops.is_empty() {
        return Err(bad(CMD, "none of the memes' words is said in this sequence"));
    }
    Ok(json!({"drops": drops.iter().map(|(w, f, t)| json!({"word": w, "file": f, "seconds": t})).collect::<Vec<_>>(), "folder": folder.to_string_lossy()}))
}

/// Shorts ▸ Meme Drops ▸ Drop Meme at Playhead…: one file at the playhead (or `time`).
pub(crate) fn drop_meme(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.memeDrop";
    let path = str_p(p, "path").map(PathBuf::from).ok_or_else(|| bad(CMD, "choose the meme file (`path`)"))?;
    let kind = kind_of(&path).ok_or_else(|| bad(CMD, "that is not a picture, GIF, video or sound FilmCraft can read"))?;
    if !path.is_file() {
        return Err(bad(CMD, format!("{} does not exist", path.display())));
    }
    let t = crate::shorts::at_time(s, p).unwrap_or_else(|| s.playhead());
    let word = path.file_stem().and_then(|n| n.to_str()).map(sh::normalize_word).unwrap_or_default();
    let dur = s.edit_group("Drop Meme", |s| place(s, &path, kind, t, 0.36, &word, CMD))?;
    if dur <= Tick::ZERO {
        return Err(bad(CMD, "move the playhead inside the sequence"));
    }
    Ok(json!({"seconds": dur.seconds(), "time": t.0}))
}

/// Shorts ▸ Meme Drops ▸ Open Meme Folder: the folder (made when missing), for the interface to
/// show.
pub(crate) fn meme_folder(_s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.memeFolder";
    let folder = folder_p(p).ok_or_else(|| bad(CMD, "this build has no data folder"))?;
    if crate::commands::bool_p(p, "create").unwrap_or(true) {
        std::fs::create_dir_all(&folder).map_err(|e| bad(CMD, format!("{}: {e}", folder.display())))?;
    }
    let memes = scan(&folder);
    Ok(json!({
        "folder": folder.to_string_lossy(),
        "memes": memes.iter().map(|m| json!({
            "file": m.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            "words": m.phrases.iter().map(|p| p.join(" ")).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "hitSound": hit_sound(&folder).map(|p| p.to_string_lossy().to_string()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, at: f64) -> SeqWord {
        SeqWord {
            text: text.into(),
            start: Tick::from_seconds_f64(at),
            end: Tick::from_seconds_f64(at + 0.2),
            clip: filmcraft_project::ClipId(1),
            item: ItemId(1),
            index: 0,
            track: 0,
            speaker: None,
            confidence: 1.0,
        }
    }

    #[test]
    fn file_names_say_when_a_meme_drops() {
        assert_eq!(phrases_of("huh"), vec![vec!["huh".to_string()]]);
        assert_eq!(phrases_of("what-the_hell"), vec![vec!["what".to_string(), "the".to_string(), "hell".to_string()]]);
        assert_eq!(phrases_of("Bruh, BRO;  "), vec![vec!["bruh".to_string()], vec!["bro".to_string()]]);
        assert!(phrases_of("!!").is_empty());
        assert!(is_hit_name("Boom") && is_hit_name("bass-hit") && is_hit_name("HIT") && !is_hit_name("hitman"));
        assert_eq!(kind_of(Path::new("a.GIF")), Some(Kind::Moving));
        assert_eq!(kind_of(Path::new("a.png")), Some(Kind::Still));
        assert_eq!(kind_of(Path::new("a.wav")), Some(Kind::Sound));
        assert_eq!(kind_of(Path::new("a.mp4")), Some(Kind::Moving));
        assert_eq!(kind_of(Path::new("a.txt")), None);
        assert_eq!(kind_of(Path::new("noext")), None);
    }

    #[test]
    fn drops_are_spread_out_and_each_meme_drops_once() {
        let memes = vec![
            Meme { path: "huh.mp4".into(), kind: Kind::Moving, phrases: phrases_of("huh") },
            Meme { path: "what the hell.png".into(), kind: Kind::Still, phrases: phrases_of("what the hell") },
            Meme { path: "bruh.gif".into(), kind: Kind::Moving, phrases: phrases_of("bruh") },
        ];
        let words = vec![
            w("Huh?", 1.0),
            w("what", 2.0),
            w("the", 2.2),
            w("hell!", 2.4),
            w("huh", 6.0),
            w("bruh", 6.5),
            w("bruh", 12.0),
            w("what", 13.0),
            w("the", 13.2),
            w("hell", 13.4),
        ];
        // huh at 1 s; "what the hell" at 2 s is too close; huh again is the same meme; bruh at 6.5 s
        // is 5.5 s later; what the hell at 13 s
        assert_eq!(matches(&words, &memes, 4.0, 10, None), vec![(0, 0), (2, 5), (1, 7)]);
        assert_eq!(matches(&words, &memes, 4.0, 1, None), vec![(0, 0)]);
        // not in the ending
        assert_eq!(matches(&words, &memes, 4.0, 10, Some((12.5, 20.0))), vec![(0, 0), (2, 5)]);
        assert!(matches(&[], &memes, 4.0, 10, None).is_empty());
        assert!(matches(&words, &[], 4.0, 10, None).is_empty());
    }

    #[test]
    fn a_folder_lists_its_memes_and_its_bass_hit() {
        let dir = std::env::temp_dir().join(format!("filmcraft-memes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for f in ["huh.mp4", "bruh, bro.gif", "boom.wav", "notes.txt", ".hidden.png", "sub/deep.png"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let m = scan(&dir);
        let names: Vec<String> = m.iter().map(|m| m.path.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["bruh, bro.gif", "huh.mp4"]);
        assert_eq!(hit_sound(&dir).and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).as_deref(), Some("boom.wav"));
        assert!(scan(&dir.join("missing")).is_empty() && hit_sound(&dir.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
