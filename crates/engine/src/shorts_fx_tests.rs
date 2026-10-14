//! Tests of Sequence ▸ Shorts, part two (`shorts_fx`): each command edits as one undo step,
//! rejects hostile parameters, and the result renders.

use std::sync::Arc;

use filmcraft_media::Generator;
use filmcraft_media::generators::GeneratorSource;
use filmcraft_project::{ClipId, ItemId, Label, ParamValue, Sequence, TrackItem, Transcript, Word};
use filmcraft_speech::FixedTranscriber;
use filmcraft_time::{FrameRate, Tick};
use serde_json::{Value, json};

use crate::Session;

fn demo() -> Session {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s
}

fn seq(s: &Session) -> Sequence {
    s.active_sequence().unwrap().clone()
}

fn clip(s: &Session, id: ClipId) -> TrackItem {
    s.active_sequence().unwrap().find_item(id).unwrap().1.clone()
}

fn sec(x: f64) -> Tick {
    Tick::from_seconds_f64(x)
}

fn render(s: &Session, t: Tick) {
    let id = s.state.active_sequence.unwrap();
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let img = filmcraft_render::render_sequence(&s.project, id, t, filmcraft_render::RenderOptions::default(), &provider);
    assert!(img.px.iter().all(|v| v.is_finite()));
}

/// A tone item of `secs` seconds at `db` dBFS.
fn tone(s: &mut Session, name: &str, db: f32, secs: f64) -> ItemId {
    let src = GeneratorSource::new(Generator::Tone { hz: 440.0, db }, 1920, 1080, FrameRate::FPS_30, sec(secs));
    let pool = s.media.clone();
    let name = name.to_string();
    s.edit("tone", |pr, _| Ok(crate::demo::add_generator(pr, &pool, src, &name, Label::Rose, None))).unwrap()
}

/// The longest demo item with a picture.
fn footage_item(s: &Session) -> ItemId {
    s.project
        .items
        .values()
        .filter(|i| i.as_media().is_some_and(|m| m.info.has_video()))
        .max_by_key(|i| i.as_media().map(|m| m.duration()).unwrap_or_default())
        .unwrap()
        .id
}

/// A 30 fps 1080 x 1920 sequence: demo footage on V1 (video only) for 10 s, a quiet tone bed on
/// A1 and a loud half-second tone on A2 at 5 s.
fn hype_session() -> (Session, ClipId) {
    let mut s = demo();
    s.execute("file.newSequence", json!({"name": "Hype", "width": 1080, "height": 1920, "fps": 30.0})).unwrap();
    let pic = footage_item(&s);
    let r = s.execute("timeline.place", json!({"item": pic.0, "track": "V1", "seconds": 0.0, "audio": false, "duration": sec(10.0).0})).unwrap();
    let v1 = ClipId(r["clips"][0].as_u64().unwrap());
    let bed = tone(&mut s, "Bed", -30.0, 10.0);
    let hit = tone(&mut s, "Scream", -6.0, 0.5);
    s.execute("timeline.place", json!({"item": bed.0, "track": "A1", "seconds": 0.0})).unwrap();
    s.execute("timeline.place", json!({"item": hit.0, "track": "A2", "seconds": 5.0})).unwrap();
    (s, v1)
}

fn menu(id: &str) -> Vec<&'static str> {
    crate::commands::find(id).unwrap().menu.to_vec()
}

#[test]
fn every_new_command_is_in_sequence_shorts() {
    for id in [
        "shorts.glitch",
        "shorts.freezeFrame",
        "shorts.hype",
        "shorts.zoomCuts",
        "shorts.keywordEffects",
        "shorts.hookTitle",
        "shorts.progressBar",
        "shorts.bleep",
        "shorts.duckMusic",
        "shorts.soundEffect",
    ] {
        assert_eq!(menu(id), ["Sequence", "Shorts"], "{id}");
        assert!(Session::default().execute(id, json!({})).is_err(), "{id} needs a sequence");
    }
}

#[test]
fn hype_finds_the_scream_and_punches_in_with_a_marker() {
    let (mut s, v1) = hype_session();
    let undo = s.history.undo.len();
    let r = s.execute("shorts.hype", json!({})).unwrap();
    let moments: Vec<i64> = r["moments"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    assert_eq!(moments.len(), 1, "{r}");
    assert!((Tick(moments[0]).seconds() - 5.0).abs() < 0.1, "{r}");
    let q = seq(&s);
    assert_eq!(q.markers.iter().filter(|m| m.name == "Hype").count(), 1);
    let it = clip(&s, v1);
    assert_eq!(it.effect("motion").unwrap().param("scale").unwrap().keyframes.len(), 4, "punched in");
    assert!(it.effect("camera_shake").is_some(), "and shaken");
    render(&s, sec(5.1));
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(seq(&s).markers.is_empty());
    assert!(clip(&s, v1).effect("camera_shake").is_none());

    // markers only, flashes, and hostile options
    let r = s.execute("shorts.hype", json!({"markersOnly": true})).unwrap();
    assert_eq!(r["clips"], 0);
    assert!(clip(&s, v1).effect("motion").unwrap().param("scale").unwrap().keyframes.is_empty());
    s.execute("edit.undo", json!({})).unwrap();
    let r = s.execute("shorts.hype", json!({"effects": ["flash", "glitch"]})).unwrap();
    assert_eq!(r["flashes"], 1);
    assert!(clip(&s, v1).effect("rgb_split").is_some());
    for p in [json!({"effects": ["boom"]}), json!({"effects": []}), json!({"sensitivity": 0}), json!({"maxMoments": -1}), json!({"effects": 5})] {
        assert!(s.execute("shorts.hype", p.clone()).is_err(), "{p}");
    }
    // nothing stands out: a clear error rather than random punches
    assert!(s.execute("shorts.hype", json!({"sensitivity": 40})).is_err());
}

/// The demo project with a transcript of its first A1 clip, in a vertical copy:
/// "What the fuck is this shit bonk" (Speaker 1), then "bonk again" (Speaker 2).
fn transcribed() -> Session {
    let mut s = demo();
    let q = s.active_sequence().unwrap();
    let a = &q.audio_tracks[0].items[0];
    let (item, sin) = (a.item, a.source_in);
    let at = |x: f64| sin + sec(x);
    let mut t = Transcript { language: "en".into(), ..Default::default() };
    for (text, a, b, sp) in [
        ("What", 0.2, 0.4, 0),
        ("the", 0.4, 0.5, 0),
        ("fuck", 0.5, 0.8, 0),
        ("is", 0.9, 1.0, 0),
        ("this", 1.0, 1.2, 0),
        ("shit?", 1.2, 1.5, 0),
        ("bonk", 2.5, 2.8, 1),
        ("again", 2.8, 3.2, 1),
    ] {
        let mut w = Word::new(text, at(a), at(b));
        w.speaker = Some(sp);
        t.words.push(w);
    }
    t.normalize();
    s.transcriber = Some(Arc::new(FixedTranscriber { transcript: t, id: "fixed".into() }));
    s.execute("transcript.generate", json!({"items": [item.0]})).unwrap();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    s
}

fn words(s: &Session) -> Vec<filmcraft_edit::transcript::SeqWord> {
    crate::transcript::sequence_words(s)
}

fn caption_texts(s: &Session, r: &Value) -> Vec<String> {
    seq(s).video_tracks[r["track"].as_u64().unwrap() as usize]
        .items
        .iter()
        .map(|i| match &i.effects.last().unwrap().param("text").unwrap().value {
            ParamValue::Text(t) => t.clone(),
            v => panic!("{v:?}"),
        })
        .collect()
}

fn bleeps(s: &Session) -> Vec<TrackItem> {
    seq(s).audio_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.name == "Bleep").cloned().collect()
}

#[test]
fn bleep_mutes_the_words_adds_beeps_and_masks_captions() {
    let mut s = transcribed();
    // captions made before the bleep get masked by it
    let before = s.execute("shorts.captionsPop", json!({"maxChars": 40})).unwrap();
    let audio_tracks = seq(&s).audio_tracks.len();
    let w = words(&s).into_iter().find(|w| w.text == "fuck").unwrap();
    let level_at = |s: &Session, t: Tick| {
        let a = seq(s).find_item(w.clip).map(|(_, i)| i.clone()).unwrap();
        a.effect("volume").unwrap().param("level").unwrap().f64_at(a.source_time_at(t))
    };
    let untouched = level_at(&s, w.start - sec(0.2));
    let r = s.execute("shorts.bleep", json!({})).unwrap();
    assert_eq!(r["bleeped"], 2, "{r}");
    assert_eq!(bleeps(&s).len(), 2);
    assert_eq!(seq(&s).audio_tracks.len(), audio_tracks + 1);
    // the word's own sound is silent in the middle of the word, and as it was elsewhere
    let mid = w.start + Tick((w.end.0 - w.start.0) / 2);
    assert!(level_at(&s, mid) <= -96.0, "{}", level_at(&s, mid));
    assert!((level_at(&s, w.start - sec(0.2)) - untouched).abs() < 1e-9);
    let texts = caption_texts(&s, &before);
    assert!(texts.iter().any(|t| t.contains("F***")), "{texts:?}");
    assert!(!texts.iter().any(|t| t.to_lowercase().contains("fuck")), "{texts:?}");
    seq(&s).check().unwrap();

    // captions made after the bleep come out masked too
    let after = s.execute("shorts.captionsKaraoke", json!({"maxChars": 40})).unwrap();
    let texts = caption_texts(&s, &after);
    assert!(texts.iter().any(|t| t.contains("S***?")), "{texts:?}");

    // a word list of your own
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    let r = s.execute("shorts.bleep", json!({"words": "bonk"})).unwrap();
    assert_eq!(r["bleeped"], 1);
    assert!(s.execute("shorts.bleep", json!({"words": "zebra"})).is_err(), "nothing to bleep");
    for p in [json!({"hz": 0}), json!({"beepDb": 3}), json!({"words": 7})] {
        assert!(s.execute("shorts.bleep", p.clone()).is_err(), "{p}");
    }
}

#[test]
fn captions_color_each_speaker() {
    let mut s = transcribed();
    let r = s.execute("shorts.captionsPop", json!({"maxChars": 40})).unwrap();
    let q = seq(&s);
    let items = &q.video_tracks[r["track"].as_u64().unwrap() as usize].items;
    let fill = |i: &TrackItem| i.effects.last().unwrap().param("fill_color").unwrap().value.clone();
    let first = fill(&items[0]);
    let last = fill(items.last().unwrap());
    assert_ne!(first, last, "speaker 2 has another colour");
    s.execute("edit.undo", json!({})).unwrap();
    let r = s.execute("shorts.captionsPop", json!({"maxChars": 40, "speakerColors": false})).unwrap();
    let q = seq(&s);
    let items = &q.video_tracks[r["track"].as_u64().unwrap() as usize].items;
    assert_eq!(fill(&items[0]), fill(items.last().unwrap()));
}

#[test]
fn effects_on_words_hit_every_match() {
    let mut s = transcribed();
    let bonk = words(&s).into_iter().find(|w| w.text == "bonk").unwrap();
    let r = s.execute("shorts.keywordEffects", json!({"words": "bonk, oh my god", "effect": "punch"})).unwrap();
    assert_eq!(r["matches"], 1, "{r}");
    let footage: Vec<TrackItem> = seq(&s).video_tracks[0].items.iter().filter(|i| i.range().contains(bonk.start)).cloned().collect();
    assert!(footage.iter().all(|i| i.effect("motion").unwrap().param("scale").unwrap().keyframes.len() == 4));
    s.execute("edit.undo", json!({})).unwrap();
    let tracks = seq(&s).video_tracks.len();
    let r = s.execute("shorts.keywordEffects", json!({"words": ["what", "bonk"], "effect": ["flash", "shake"]})).unwrap();
    assert_eq!((r["matches"].as_u64(), r["flashes"].as_u64()), (Some(2), Some(2)));
    assert_eq!(seq(&s).video_tracks.len(), tracks + 1);
    render(&s, bonk.start);
    for p in [json!({}), json!({"words": ""}), json!({"words": "bonk", "effect": "explode"}), json!({"words": 3}), json!({"words": "zebra"})] {
        assert!(s.execute("shorts.keywordEffects", p.clone()).is_err(), "{p}");
    }
}

#[test]
fn glitch_bursts_sit_before_the_crop() {
    let mut s = demo();
    let q = seq(&s);
    let c = q.video_tracks[0].items[0].clone();
    let t = c.start + Tick(c.duration.0 / 2);
    s.execute("shorts.stackRight", json!({"clips": [c.id.0]})).unwrap();
    s.state.selection.clear();
    let r = s.execute("shorts.glitch", json!({"time": t.0})).unwrap();
    assert_eq!(r["clips"], 2);
    let it = clip(&s, c.id);
    let pos = |id: &str| it.effects.iter().position(|e| e.effect == id).unwrap();
    assert!(pos("rgb_split") < pos("crop") && pos("wave_warp") < pos("crop") && pos("camera_shake") < pos("crop"));
    let amount = it.effect("rgb_split").unwrap().param("amount").unwrap();
    assert!(amount.keyframes.len() >= 5 && amount.keyframes.last().unwrap().value.as_f64() == Some(0.0));
    render(&s, t + sec(0.1));
    assert!(s.execute("shorts.glitch", json!({"amount": 0})).is_err());
}

#[test]
fn zoom_on_every_cut_alternates() {
    let mut s = demo();
    let c = seq(&s).video_tracks[0].items[0].clone();
    let third = Tick(c.duration.0 / 3);
    // two cuts: three pieces of the same clip
    s.edit_sequence("cut", |q, ctx, _| {
        filmcraft_edit::razor_items(q, &[c.id], c.start + third, ctx);
        let right = q.video_tracks[0].items.iter().find(|i| i.start == c.start + third).unwrap().id;
        filmcraft_edit::razor_items(q, &[right], c.start + third + third, ctx);
        Ok(())
    })
    .unwrap();
    let pieces: Vec<TrackItem> =
        seq(&s).video_tracks[0].items.iter().filter(|i| i.item == c.item && i.start >= c.start && i.end() <= c.end()).cloned().collect();
    assert_eq!(pieces.len(), 3);
    let scale = |s: &Session, id: ClipId| clip(s, id).effect("motion").unwrap().param("scale").unwrap().value.as_f64().unwrap();
    let before: Vec<f64> = pieces.iter().map(|p| scale(&s, p.id)).collect();
    s.execute("shorts.zoomCuts", json!({"zoom": 120})).unwrap();
    let after: Vec<f64> = pieces.iter().map(|p| scale(&s, p.id)).collect();
    assert_eq!(after[0], before[0]);
    assert!((after[1] - before[1] * 1.2).abs() < 1e-9);
    assert_eq!(after[2], before[2]);
    assert!(s.execute("shorts.zoomCuts", json!({"zoom": 50})).is_err());
}

#[test]
fn freeze_frame_holds_desaturates_and_ripples() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let c = seq(&s).video_tracks[0].items[0].clone();
    let t = FrameRate::FPS_30.snap(c.start + Tick(c.duration.0 / 2));
    s.execute("shorts.stackLeft", json!({"clips": [c.id.0]})).unwrap();
    let dur_before = seq(&s).duration();
    let undo = s.history.undo.len();
    let r = s.execute("shorts.freezeFrame", json!({"time": t.0, "seconds": 1.0})).unwrap();
    let seg = clip(&s, ClipId(r["clip"].as_u64().unwrap()));
    assert!(seg.frame_hold.is_some());
    assert!(seg.effect("black_white").is_some());
    assert_eq!(seg.effect("motion").unwrap().param("scale").unwrap().keyframes.len(), 2, "pushes in");
    let q = seq(&s);
    assert!(q.duration() > dur_before, "later clips moved right");
    // the stacked model pane holds as well (no hole in the bottom half)
    let holds = q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.start == seg.start && i.frame_hold.is_some()).count();
    assert_eq!(holds, 2);
    // the flash
    assert!(q.video_tracks.iter().flat_map(|t| t.items.iter()).any(|i| i.name == "Flash" && i.start == seg.start));
    q.check().unwrap();
    render(&s, seg.start + sec(0.5));
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(seq(&s).duration(), dur_before);
    assert!(s.execute("shorts.freezeFrame", json!({"time": c.start.0})).is_err(), "not on a clip's first frame");
}

#[test]
fn hook_title_and_progress_bar() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let r = s.execute("shorts.hookTitle", json!({"text": "he really did that"})).unwrap();
    let q = seq(&s);
    let it = &q.video_tracks[r["track"].as_u64().unwrap() as usize].items[0];
    assert_eq!(it.start, Tick::ZERO);
    let layer = it.effects.last().unwrap();
    assert_eq!(layer.param("text").unwrap().value, ParamValue::Text("he really did that".into()));
    assert_eq!(layer.param("opacity").unwrap().keyframes.last().unwrap().value.as_f64(), Some(0.0));
    render(&s, sec(0.5));
    let r = s.execute("shorts.hookTitle", json!({})).unwrap();
    assert_eq!(r["text"], "WAIT FOR IT…");
    assert!(s.execute("shorts.hookTitle", json!({"text": "x".repeat(500)})).is_err());

    let dur = seq(&s).duration();
    let r = s.execute("shorts.progressBar", json!({"position": "top", "color": "#ff0000"})).unwrap();
    let q = seq(&s);
    let it = &q.video_tracks[r["track"].as_u64().unwrap() as usize].items[0];
    assert_eq!(it.duration, q.settings.frame_rate.snap(dur));
    let bar = it.effects.iter().rev().find(|e| e.effect == filmcraft_project::graphic::SHAPE_LAYER).unwrap();
    let size = bar.param("size").unwrap();
    assert_eq!(size.keyframes.first().unwrap().value.as_vec2().unwrap().x, 0.0);
    assert_eq!(size.keyframes.last().unwrap().value.as_vec2().unwrap().x, 1080.0);
    render(&s, Tick(it.duration.0 / 2));
    for p in [json!({"position": "middle"}), json!({"height": 0}), json!({"color": "nope"})] {
        assert!(s.execute("shorts.progressBar", p.clone()).is_err(), "{p}");
    }
}

#[test]
fn duck_music_tags_and_ducks_under_speech() {
    let (mut s, _) = hype_session();
    let a2: Vec<ClipId> = seq(&s).audio_tracks[1].items.iter().map(|i| i.id).collect();
    let bed = seq(&s).audio_tracks[0].items[0].id;
    // the bed is the music; the loud tone on A2 plays the part of speech
    let r = s.execute("shorts.duckMusic", json!({"clips": [bed.0]})).unwrap();
    assert_eq!((r["music"].as_u64(), r["dialogue"].as_u64()), (Some(1), Some(a2.len() as u64)), "{r}");
    let music = clip(&s, bed);
    assert!(music.essential.is_some());
    assert!(!music.effect("volume").unwrap().param("level").unwrap().keyframes.is_empty(), "ducked");
    s.state.selection.clear();
    assert!(s.execute("shorts.duckMusic", json!({})).is_err(), "nothing selected");
}

#[test]
fn sound_effect_drops_the_selected_sound_at_the_playhead() {
    let (mut s, _) = hype_session();
    let boom = tone(&mut s, "Boom", -10.0, 1.0);
    s.state.project_selection = vec![boom];
    let r = s.execute("shorts.soundEffect", json!({"time": sec(5.0).0})).unwrap();
    let id = ClipId(r["clips"][0].as_u64().unwrap());
    let q = seq(&s);
    let (track, it) = q.audio_tracks.iter().find_map(|t| t.item(id).map(|i| (t.id, i.clone()))).unwrap();
    assert_eq!(it.start, sec(5.0));
    // the loud tone already sits on A2 at 5 s, so it went elsewhere
    assert_ne!(track, q.audio_tracks[1].id);
    q.check().unwrap();
    s.state.project_selection.clear();
    assert!(s.execute("shorts.soundEffect", json!({})).is_err());
}

// ---------------------------------------------------------------------------------------------
// Batch 2: speed ramps, beats, chat, model tracking
// ---------------------------------------------------------------------------------------------

#[test]
fn speed_ramp_slows_the_moment_and_keeps_sync_after_it() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let c = seq(&s).video_tracks[0].items[0].clone();
    let rate = seq(&s).settings.frame_rate;
    let t = rate.snap(c.start + Tick(c.duration.0 / 4));
    let m0 = c.moving_source_time_at(t);
    let dur_before = seq(&s).duration();
    let undo = s.history.undo.len();
    let r = s.execute("shorts.speedRamp", json!({"time": t.0, "speed": 40, "seconds": 1.0})).unwrap();
    let added = Tick(r["added"].as_i64().unwrap());
    assert!(added > sec(0.5), "{r}");
    assert_eq!(seq(&s).duration(), dur_before + added, "later clips moved right by the extra time");
    let mid = clip(&s, ClipId(r["clips"][0].as_u64().unwrap()));
    assert_eq!(mid.start, t);
    assert_eq!(mid.effect("time_remap").unwrap().param("speed").unwrap().keyframes.len(), 4);
    // the picture starts where it was and lands on the next piece's first frame
    let len = rate.snap_nearest(sec(1.0));
    assert!((mid.picture_time_at(t).0 - m0.0).abs() < rate.frame_duration().0);
    let landing = mid.picture_time_at(mid.end());
    assert!((landing.0 - (m0 + len).0).abs() <= rate.frame_duration().0, "{landing:?} vs {:?}", m0 + len);
    // in the middle it moves slowly
    let a = mid.picture_time_at(mid.start + Tick(mid.duration.0 / 2));
    let b = mid.picture_time_at(mid.start + Tick(mid.duration.0 / 2) + rate.frame_duration());
    assert!(((b - a).0 as f64) < rate.frame_duration().0 as f64 * 0.5, "slow motion");
    let next = seq(&s).video_tracks[0].items.iter().find(|i| i.start == mid.end()).cloned().unwrap();
    assert_eq!(next.source_in, m0 + len, "the clip continues where the moment ended");
    seq(&s).check().unwrap();
    render(&s, mid.start + Tick(mid.duration.0 / 2));
    assert_eq!(s.history.undo.len(), undo + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(seq(&s).duration(), dur_before);
    for p in [json!({"speed": 100}), json!({"speed": 1}), json!({"seconds": 0}), json!({"time": c.start.0})] {
        assert!(s.execute("shorts.speedRamp", p.clone()).is_err(), "{p}");
    }
}

/// A 16-bit stereo WAV of clicks at `bpm` for `secs` seconds.
fn click_track(dir: &std::path::Path, bpm: f64, secs: f64) -> String {
    let sr = 44_100u32;
    let n = (secs * f64::from(sr)) as usize;
    let period = (60.0 / bpm * f64::from(sr)) as usize;
    let mut pcm: Vec<i16> = Vec::with_capacity(n * 2);
    let mut seed = 12345u32;
    for i in 0..n {
        let k = i % period;
        let v = if k < 600 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let noise = ((seed >> 16) as f64 / 32768.0) - 1.0;
            noise * (1.0 - k as f64 / 600.0) * 0.8
        } else {
            0.0
        };
        let s = (v * 32767.0) as i16;
        pcm.push(s);
        pcm.push(s);
    }
    let data: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + data.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(sr.to_le_bytes());
    wav.extend((sr * 4).to_le_bytes());
    wav.extend(4u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((data.len() as u32).to_le_bytes());
    wav.extend(data);
    let path = dir.join("clicks.wav");
    std::fs::write(&path, wav).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn effects_on_the_beat_pulse_with_the_music() {
    let (mut s, v1) = hype_session();
    let dir = crate::media_test_util::tmp_dir("beats");
    let path = click_track(&dir, 120.0, 10.0);
    let r = s.execute("file.import", json!({"paths": [path]})).unwrap();
    let item = r["items"][0].as_u64().map(ItemId).or_else(|| s.state.project_selection.first().copied()).unwrap();
    let r = s.execute("timeline.place", json!({"item": item.0, "track": "A3", "seconds": 0.0})).unwrap();
    let music = ClipId(r["clips"][0].as_u64().unwrap());
    let r = s.execute("shorts.beatSync", json!({"clips": [music.0], "every": 1})).unwrap();
    let bpm = r["bpm"].as_f64().unwrap();
    assert!((bpm - 120.0).abs() < 3.0, "{r}");
    assert!(r["beats"].as_u64().unwrap() >= 15, "{r}");
    let keys = clip(&s, v1).effect("motion").unwrap().param("scale").unwrap().keyframes.len();
    assert!(keys >= 45, "three keyframes per beat: {keys}");
    render(&s, sec(2.0));
    s.execute("edit.undo", json!({})).unwrap();
    let r = s.execute("shorts.beatSync", json!({"clips": [music.0], "effects": ["flash"], "every": 4})).unwrap();
    assert!(r["flashes"].as_u64().unwrap() >= 3, "{r}");
    s.state.selection.clear();
    assert!(s.execute("shorts.beatSync", json!({})).is_err());
    assert!(s.execute("shorts.beatSync", json!({"clips": [music.0], "every": 0})).is_err());
}

/// A TwitchDownloader chat file for the demo's first V1 clip: a message a second through the
/// clip's media, and a burst of 30 at its middle.
fn chat_file(s: &Session) -> (String, TrackItem) {
    let c = seq(s).video_tracks[0].items[0].clone();
    let (a, b) = (c.source_in.seconds(), c.source_out().seconds());
    let mut comments = Vec::new();
    // a quiet minute either side of the clip, a message a second
    let mut t = a - 60.0;
    let mut i = 0;
    while t < b + 60.0 {
        comments
            .push(json!({"content_offset_seconds": t, "commenter": {"display_name": format!("viewer{}", i % 7)}, "message": {"body": format!("message {i}")}}));
        t += 1.0;
        i += 1;
    }
    let mid = (a + b) / 2.0;
    for k in 0..30 {
        comments.push(json!({"content_offset_seconds": mid + k as f64 * 0.1, "commenter": {"display_name": "hype"}, "message": {"body": "KEKW KEKW", "user_color": "#00FF7F"}}));
    }
    let dir = crate::media_test_util::tmp_dir("chat");
    let path = dir.join("chat.json");
    std::fs::write(&path, serde_json::to_string(&json!({"comments": comments})).unwrap()).unwrap();
    (path.to_string_lossy().into_owned(), c)
}

#[test]
fn chat_overlay_shows_the_latest_messages() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let (path, c) = chat_file(&s);
    let tracks = seq(&s).video_tracks.len();
    let r = s.execute("shorts.chatOverlay", json!({"path": path, "lines": 4})).unwrap();
    assert!(r["messages"].as_u64().unwrap() >= 30, "{r}");
    assert_eq!(seq(&s).video_tracks.len(), tracks + 1);
    let q = seq(&s);
    let items = &q.video_tracks[r["track"].as_u64().unwrap() as usize].items;
    for p in items.windows(2) {
        assert!(p[0].end() <= p[1].start);
    }
    let last = items.last().unwrap().effects.last().unwrap();
    let ParamValue::Text(text) = &last.param("text").unwrap().value else { panic!() };
    assert!(text.lines().count() <= 4, "{text}");
    assert!(text.contains(": "), "{text}");
    assert!(!last.layer.as_ref().unwrap().runs.is_empty(), "names are coloured");
    render(&s, items[items.len() / 2].start);
    let _ = c;
    for p in [json!({}), json!({"path": "/no/such/chat.json"}), json!({"path": path, "lines": 0}), json!({"path": path, "position": "middle"})] {
        assert!(s.execute("shorts.chatOverlay", p.clone()).is_err(), "{p}");
    }
    // the chat of another part of the recording does not land anywhere
    assert!(s.execute("shorts.chatOverlay", json!({"path": path, "offset": 86_000})).is_err());
}

#[test]
fn chat_highlights_mark_the_burst() {
    let mut s = demo();
    let (path, c) = chat_file(&s);
    let r = s.execute("shorts.chatHighlights", json!({"path": path})).unwrap();
    assert_eq!(r["markers"], 1, "{r}");
    let q = seq(&s);
    let m = q.markers.iter().find(|m| m.name == "Chat Spike").unwrap();
    let mid = c.start + Tick(c.duration.0 / 2);
    assert!((m.start.seconds() - mid.seconds()).abs() < 6.0, "{} vs {}", m.start.seconds(), mid.seconds());
    assert!(m.comment.contains("KEKW"), "{}", m.comment);
}

#[test]
fn track_model_keyframes_the_pane_inside_the_picture() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let c = seq(&s).video_tracks[0].items[0].clone();
    let r = s.execute("shorts.stackRight", json!({"clips": [c.id.0]})).unwrap();
    let model = ClipId(r["clips"][0]["model"].as_u64().unwrap());
    let r = s.execute("shorts.trackModel", json!({"clips": [model.0], "wait": true})).unwrap();
    assert_eq!(r["jobs"].as_array().unwrap().len(), 1, "{r}");
    assert!(s.model_jobs.is_empty(), "applied");
    let it = clip(&s, model);
    let anchor = it.effect("motion").unwrap().param("anchor").unwrap();
    assert!(anchor.keyframes.len() >= 2, "{}", anchor.keyframes.len());
    let crop = it.effect("crop").unwrap();
    for k in ["left", "top", "right", "bottom"] {
        assert!(crop.param(k).unwrap().keyframes.iter().all(|kf| (0.0..=100.0).contains(&kf.value.as_f64().unwrap())), "{k}");
    }
    render(&s, it.start + Tick(it.duration.0 / 2));
    // nothing selected and no model pane under the playhead
    s.state.selection.clear();
    let seq_id = s.state.active_sequence.unwrap();
    s.state.playheads.insert(seq_id, Tick(i64::MAX / 4));
    assert!(s.execute("shorts.trackModel", json!({})).is_err());
}
