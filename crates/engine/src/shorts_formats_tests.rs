//! Tests of the Shorts menu's formats, Auto-Trim to Length and Loop Ending (`shorts_formats`,
//! `shorts_trim`): each is one undo step, keeps text out of what the apps cover, rejects hostile
//! parameters, and the result renders.

use std::sync::Arc;

use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{DemoScene, Generator};
use filmcraft_project::{ItemId, Label, Marker, MarkerId, MarkerKind, ParamValue, Sequence, Transcript, Word};
use filmcraft_speech::FixedTranscriber;
use filmcraft_time::{FrameRate, Tick, TimeRange};
use serde_json::json;

use crate::Session;
use crate::shorts_formats::{TIKTOK, YOUTUBE};

fn sec(x: f64) -> Tick {
    Tick::from_seconds_f64(x)
}

fn seq(s: &Session) -> Sequence {
    s.active_sequence().unwrap().clone()
}

fn track_names(s: &Session) -> Vec<String> {
    seq(s).video_tracks.iter().map(|t| t.name.clone()).collect()
}

fn count(names: &[String], n: &str) -> usize {
    names.iter().filter(|x| *x == n).count()
}

fn render(s: &Session, t: Tick) -> Vec<f32> {
    let id = s.state.active_sequence.unwrap();
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let img = filmcraft_render::render_sequence(&s.project, id, t, filmcraft_render::RenderOptions::default(), &provider);
    assert!(img.px.iter().all(|v| v.is_finite()));
    img.px
}

fn diff(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| f64::from((x - y).abs())).sum::<f64>() / a.len().max(1) as f64
}

fn tone(s: &mut Session, name: &str, db: f32, secs: f64) -> ItemId {
    let src = GeneratorSource::new(Generator::Tone { hz: 440.0, db }, 1920, 1080, FrameRate::FPS_30, sec(secs));
    let pool = s.media.clone();
    let name = name.to_string();
    s.edit("tone", |pr, _| Ok(crate::demo::add_generator(pr, &pool, src, &name, Label::Rose, None))).unwrap()
}

/// A 16:9, 30 fps stream sequence `secs` long: moving footage on V1 (no sound), a quiet mic bed
/// on A1 with a word spoken every 0.6 s, and half-second bursts on A2 at `bursts` (seconds,
/// dBFS). Returns the session and the burst items.
fn stream(secs: f64, bursts: &[(f64, f32)]) -> (Session, Vec<ItemId>) {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    s.execute("file.newSequence", json!({"name": "Stream", "width": 1920, "height": 1080, "fps": 30.0})).unwrap();
    let pool = s.media.clone();
    let src = GeneratorSource::new(Generator::Demo(DemoScene::Plasma), 1920, 1080, FrameRate::FPS_30, sec(secs));
    let pic = s.edit("pic", |pr, _| Ok(crate::demo::add_generator(pr, &pool, src, "Stream VOD", Label::Iris, None))).unwrap();
    s.execute("timeline.place", json!({"item": pic.0, "track": "V1", "seconds": 0.0, "audio": false})).unwrap();
    let mic = tone(&mut s, "Mic", -30.0, secs);
    s.execute("timeline.place", json!({"item": mic.0, "track": "A1", "seconds": 0.0})).unwrap();
    let mut ids = Vec::new();
    for (at, db) in bursts {
        let b = tone(&mut s, "Burst", *db, 0.5);
        s.execute("timeline.place", json!({"item": b.0, "track": "A2", "seconds": at})).unwrap();
        ids.push(b);
    }
    let mut t = Transcript { language: "en".into(), ..Default::default() };
    let words = ["wait", "no", "way", "that", "actually", "worked"];
    let mut i = 0usize;
    loop {
        let a = 0.3 + i as f64 * 0.6;
        if a + 0.5 > secs {
            break;
        }
        t.words.push(Word::new(words[i % words.len()], sec(a), sec(a + 0.4)));
        i += 1;
    }
    t.normalize();
    s.transcriber = Some(Arc::new(FixedTranscriber { transcript: t, id: "fixed".into() }));
    s.execute("transcript.generate", json!({"items": [mic.0]})).unwrap();
    assert!(!crate::transcript::sequence_words(&s).is_empty());
    (s, ids)
}

/// The y (fraction of the frame) of each caption's text.
fn caption_ys(s: &Session) -> Vec<f64> {
    let q = seq(s);
    let h = f64::from(q.settings.height);
    let Some(t) = q.video_tracks.iter().find(|t| t.name == "Word Captions") else { return Vec::new() };
    t.items
        .iter()
        .filter_map(|i| match &i.effects.last()?.param("position")?.value {
            ParamValue::Vec2(v) => Some(v.y / h),
            _ => None,
        })
        .collect()
}

#[test]
fn every_shorts_command_is_in_the_shorts_menu() {
    for c in crate::commands::command_specs().iter().filter(|c| c.id.starts_with("shorts.")) {
        assert_eq!(c.menu.first().copied(), Some("Shorts"), "{}", c.id);
        assert!(Session::default().execute(c.id, json!({})).is_err(), "{} needs a sequence", c.id);
    }
    let menu = |id: &str| crate::commands::find(id).unwrap().menu.to_vec();
    assert_eq!(menu("shorts.format"), ["Shorts"]);
    assert_eq!(menu("shorts.youtube.countdown"), ["Shorts", "YouTube Shorts"]);
    assert_eq!(menu("shorts.tiktok.meme"), ["Shorts", "TikTok"]);
    assert_eq!(menu("shorts.punchIn"), ["Shorts", "Effects at Playhead"]);
}

#[test]
fn every_format_makes_a_vertical_short_clear_of_the_app() {
    for pf in [YOUTUBE, TIKTOK] {
        for f in ["gameplay", "fullFrame", "meme", "spotlight"] {
            let id = format!("shorts.{}.{f}", pf.id);
            let (mut s, _) = stream(8.0, &[]);
            let src = s.state.active_sequence.unwrap();
            let undo = s.history.undo.len();
            let r = s.execute(&id, json!({})).unwrap();
            assert_eq!(s.history.undo.len(), undo + 1, "{id}: one undo step");
            let q = seq(&s);
            assert_eq!((q.settings.width, q.settings.height), (1080, 1920), "{id}");
            assert_ne!(s.state.active_sequence, Some(src), "{id}: a vertical copy");
            assert!(s.project.item(s.state.active_sequence.unwrap()).unwrap().name.contains(pf.name), "{id}");
            assert_eq!(s.project.sequence(src).unwrap().settings.width, 1920, "{id}: the stream is left alone");
            let names = track_names(&s);
            assert_eq!(names.iter().any(|n| n == "Model"), matches!(f, "gameplay" | "spotlight"), "{id}: {names:?}");
            assert_eq!(names.iter().any(|n| n == "Meme Bar"), f == "meme", "{id}: {names:?}");
            assert_eq!(names.iter().any(|n| n == "Title"), f == "fullFrame", "{id}: {names:?}");
            assert_eq!(names.iter().any(|n| n == "Hook Title"), f == "gameplay", "{id}: {names:?}");
            assert_eq!(count(&names, "Progress Bar"), 1, "{id}: {names:?}");
            assert_eq!(r["preset"], pf.preset);
            assert!(r["warnings"].as_array().unwrap().is_empty(), "{id}: {r}");
            // captions (not on the meme) between the app's bars
            let ys = caption_ys(&s);
            assert_eq!(ys.is_empty(), f == "meme", "{id}");
            for y in ys {
                assert!(y > pf.top() + 0.02 && y < pf.bottom() - 0.02, "{id}: caption at {y}");
            }
            if f != "meme" {
                let w = s.state.shorts_captions.as_ref().and_then(|c| c["width"].as_f64()).unwrap_or(0.92);
                assert!(w <= 0.9, "{id}: captions keep clear of the buttons ({w})");
            }
            render(&s, sec(1.0));
            render(&s, sec(5.0));
            // undo: the vertical copy is gone again
            let made = ItemId(r["sequence"].as_u64().unwrap());
            s.execute("edit.undo", json!({})).unwrap();
            assert!(s.project.item(made).is_none(), "{id}");
        }
    }
}

#[test]
fn spotlight_shows_one_big_word_over_the_model() {
    let (mut s, _) = stream(6.0, &[]);
    s.execute("shorts.tiktok.spotlight", json!({})).unwrap();
    assert_eq!(s.state.shorts_captions.as_ref().unwrap()["style"], "oneWord");
    let (pane, model) = crate::shorts_layout::current_rects(&s).unwrap();
    let model = model.unwrap();
    assert!(model[3] - model[1] > pane[3] - pane[1], "the model is bigger than the gameplay");
    assert!(pane[2] - pane[0] < 0.9, "the gameplay is a card, narrower than the frame");
    assert!(pane[1] > TIKTOK.top(), "below the top bar");
}

#[test]
fn a_format_applied_again_replaces_what_the_last_one_added() {
    let (mut s, _) = stream(8.0, &[]);
    s.execute("shorts.tiktok.gameplay", json!({})).unwrap();
    let vertical = s.state.active_sequence;
    s.execute("shorts.tiktok.meme", json!({"title": "when the plan actually works"})).unwrap();
    assert_eq!(s.state.active_sequence, vertical, "re-done in place");
    let names = track_names(&s);
    for gone in ["Model", "Hook Title", "Word Captions", "Layout Background"] {
        assert_eq!(count(&names, gone), 0, "{gone}: {names:?}");
    }
    assert_eq!(count(&names, "Meme Bar"), 1);
    assert_eq!(count(&names, "Progress Bar"), 1);
    // the bar's text is the title, on a white bar above the picture
    let q = seq(&s);
    let bar = q.video_tracks.iter().find(|t| t.name == "Meme Bar").unwrap();
    assert_eq!(bar.items.len(), 1);
    assert_eq!(bar.items[0].duration, q.duration());
    s.execute("shorts.youtube.gameplay", json!({"hook": false, "progressBar": false})).unwrap();
    let names = track_names(&s);
    assert_eq!((count(&names, "Model"), count(&names, "Meme Bar"), count(&names, "Progress Bar"), count(&names, "Word Captions")), (1, 0, 0, 1), "{names:?}");
    render(&s, sec(2.0));
}

#[test]
fn countdown_cuts_to_the_biggest_moments_and_numbers_them() {
    let (mut s, bursts) = stream(30.0, &[(5.0, -20.0), (15.0, -6.0), (25.0, -14.0)]);
    let r = s.execute("shorts.youtube.countdown", json!({"momentSeconds": 4})).unwrap();
    let ranks: Vec<u64> = r["moments"].as_array().unwrap().iter().map(|m| m["rank"].as_u64().unwrap()).collect();
    assert_eq!(ranks, [3, 2, 1]);
    let db: Vec<f64> = r["moments"].as_array().unwrap().iter().map(|m| m["db"].as_f64().unwrap()).collect();
    assert!(db.windows(2).all(|p| p[0] <= p[1]), "smallest first: {db:?}");
    let q = seq(&s);
    // three parts of up to 4 s (an edge inside a word moves to before it)
    assert!(q.duration().seconds() > 10.5 && q.duration().seconds() <= 12.05, "{}", q.duration().seconds());
    // the loudest burst comes last, the quietest first
    let parts: Vec<(f64, f64)> = r["moments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (Tick(m["start"].as_i64().unwrap()).seconds(), Tick(m["start"].as_i64().unwrap() + m["duration"].as_i64().unwrap()).seconds()))
        .collect();
    let a2 = &q.audio_tracks[1].items;
    let at = |item: ItemId| a2.iter().find(|i| i.item == item).map(|i| i.start.seconds()).unwrap();
    for (part, burst) in parts.iter().zip([bursts[0], bursts[2], bursts[1]]) {
        assert!(at(burst) >= part.0 && at(burst) < part.1, "{parts:?} {a2:?}");
    }
    let badges: Vec<String> = q.video_tracks.iter().find(|t| t.name == "Countdown").unwrap().items.iter().map(|i| i.name.clone()).collect();
    assert_eq!(badges, ["#3", "#2", "#1"]);
    assert_eq!(q.video_tracks.iter().find(|t| t.name == "Countdown Flashes").unwrap().items.len(), 2);
    let hook = q.video_tracks.iter().find(|t| t.name == "Hook Title").unwrap();
    assert_eq!(hook.items[0].name, "TOP 3 MOMENTS");
    render(&s, sec(8.1));
    // a clip with nothing that stands out cannot count down
    let (mut flat, _) = stream(12.0, &[]);
    let (undo, seqs) = (flat.history.undo.len(), flat.project.sequences().count());
    assert!(flat.execute("shorts.tiktok.countdown", json!({})).is_err());
    assert_eq!((flat.history.undo.len(), flat.project.sequences().count()), (undo, seqs), "nothing left behind");
}

#[test]
fn auto_trim_keeps_the_liveliest_part_and_never_runs_long() {
    let (mut s, bursts) = stream(20.0, &[(14.0, -6.0)]);
    let undo = s.history.undo.len();
    let r = s.execute("shorts.autoTrim", json!({"seconds": 6})).unwrap();
    assert_eq!(r["trimmed"], true);
    assert_eq!(s.history.undo.len(), undo + 1);
    let q = seq(&s);
    assert!(q.duration().seconds() <= 6.0 + 1e-6 && q.duration().seconds() > 5.0, "{}", q.duration().seconds());
    let b = q.audio_tracks[1].items.iter().find(|i| i.item == bursts[0]).expect("the scream stays");
    assert!(b.start.seconds() > 0.5 && b.end().seconds() < 6.0, "with time before and after it: {}", b.start.seconds());
    // no word is cut in half at the edges
    for w in crate::transcript::sequence_words(&s) {
        assert!(w.end > w.start);
    }
    s.execute("edit.undo", json!({})).unwrap();
    assert!((seq(&s).duration().seconds() - 20.0).abs() < 0.05);
    // longer than the sequence: nothing to do; hostile values
    assert_eq!(s.execute("shorts.autoTrim", json!({"seconds": 60})).unwrap()["trimmed"], false);
    for p in [json!({"seconds": 1}), json!({"seconds": -3}), json!({"seconds": 1e12}), json!({"seconds": "long"})] {
        assert!(s.execute("shorts.autoTrim", p.clone()).is_err(), "{p}");
    }
    // as part of a format
    let r = s.execute("shorts.tiktok.fullFrame", json!({"seconds": 15})).unwrap();
    assert!(r["seconds"].as_f64().unwrap() <= 15.1, "{r}");
}

#[test]
fn loop_ending_dissolves_into_the_opening_frame() {
    let (mut s, _) = stream(6.0, &[]);
    s.execute("shorts.youtube.gameplay", json!({"captions": false, "progressBar": false, "hook": false})).unwrap();
    let before = seq(&s).duration();
    let undo = s.history.undo.len();
    let r = s.execute("shorts.loopEnding", json!({"seconds": 0.5})).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1);
    // the gameplay, the model and the blurred background each loop
    assert_eq!(r["tracks"], 3, "{r}");
    let q = seq(&s);
    let frame = q.settings.frame_rate.frame_duration();
    assert_eq!(q.duration(), before + frame, "one held frame at the end");
    let holds: Vec<_> = q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| crate::shorts_trim::is_loop_hold(i)).collect();
    assert_eq!(holds.len(), 3);
    for t in q.video_tracks.iter().filter(|t| t.items.iter().any(crate::shorts_trim::is_loop_hold)) {
        let x = t.transitions.iter().find(|x| x.to.is_some_and(|c| holds.iter().any(|h| h.id == c))).expect("a dissolve into the hold");
        assert_eq!(x.end(), before);
        assert!((x.duration.seconds() - 0.5).abs() < 0.02);
    }
    assert!(q.audio_tracks.iter().any(|t| t.transitions.iter().any(|x| x.to.is_none() && x.end() == before)), "the sound fades out");
    // the last frame is the first frame again; half way through the dissolve is in between
    let first = render(&s, Tick::ZERO);
    let last = render(&s, before);
    let earlier = render(&s, before - sec(0.6));
    assert!(diff(&first, &last) < 0.01, "{}", diff(&first, &last));
    assert!(diff(&first, &earlier) > diff(&first, &last) + 0.005, "{} vs {}", diff(&first, &earlier), diff(&first, &last));
    // again: no second set; a new layout keeps the loop, made from it
    s.execute("shorts.loopEnding", json!({})).unwrap();
    let n = |s: &Session| seq(s).video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| crate::shorts_trim::is_loop_hold(i)).count();
    assert_eq!(n(&s), 3);
    s.execute("shorts.layout", json!({"preset": "pip"})).unwrap();
    assert_eq!(n(&s), 3);
    assert_eq!(seq(&s).duration(), before + frame);
    for p in [json!({"seconds": 0}), json!({"seconds": 99}), json!({"seconds": f64::MAX})] {
        assert!(s.execute("shorts.loopEnding", p.clone()).is_err(), "{p}");
    }
}

#[test]
fn keeping_windows_moves_everything_inside_them_and_drops_the_rest() {
    let (mut s, bursts) = stream(10.0, &[(2.0, -6.0), (7.0, -6.0)]);
    let pool_item = s.project.items.values().find(|i| i.name == "Stream VOD").unwrap().id;
    // a linked picture and sound on V2/A3 too
    s.execute("timeline.place", json!({"item": pool_item.0, "track": "V2", "audioTrack": "A3", "seconds": 0.0})).unwrap();
    s.edit_sequence("markers", |q, ctx, _| {
        for at in [1.5, 6.5, 9.5] {
            q.markers.push(Marker {
                id: MarkerId(ctx.alloc()),
                start: sec(at),
                duration: Tick::ZERO,
                name: format!("{at}"),
                comment: String::new(),
                kind: MarkerKind::Comment,
                color: Label::Green,
            });
        }
        Ok(())
    })
    .unwrap();
    let windows = [TimeRange::from_bounds(sec(6.0), sec(8.0)), TimeRange::from_bounds(sec(1.0), sec(3.0))];
    let total = s.edit_sequence("keep", |q, ctx, _| crate::shorts_trim::keep_windows(q, ctx, &windows)).unwrap();
    assert_eq!(total, sec(4.0));
    let q = seq(&s);
    assert_eq!(q.duration(), sec(4.0));
    let marks: Vec<(String, f64)> = q.markers.iter().map(|m| (m.name.clone(), (m.start.seconds() * 10.0).round() / 10.0)).collect();
    assert_eq!(marks, [("6.5".to_string(), 0.5), ("1.5".to_string(), 2.5)]);
    let a2 = &q.audio_tracks[1].items;
    let start_of = |b: ItemId| a2.iter().find(|i| i.item == b).map(|i| (i.start.seconds() * 10.0).round() / 10.0);
    assert_eq!((start_of(bursts[0]), start_of(bursts[1])), (Some(3.0), Some(1.0)));
    let v1: Vec<(f64, f64)> = q.video_tracks[0].items.iter().map(|i| (i.start.seconds(), (i.source_in.seconds() * 10.0).round() / 10.0)).collect();
    assert_eq!(v1, [(0.0, 6.0), (2.0, 1.0)]);
    // every link points at a clip that is still there, with its partner at the same time
    for it in q.video_tracks.iter().chain(&q.audio_tracks).flat_map(|t| t.items.iter()) {
        if let Some(l) = it.link {
            let (_, p) = q.find_item(filmcraft_project::ClipId(l)).expect("partner kept");
            assert_eq!(p.start, it.start);
        }
    }
    // overlapping or empty windows are refused
    let bad = [TimeRange::from_bounds(sec(0.0), sec(2.0)), TimeRange::from_bounds(sec(1.0), sec(3.0))];
    assert!(s.edit_sequence("keep", |q, ctx, _| crate::shorts_trim::keep_windows(q, ctx, &bad)).is_err());
    assert!(s.edit_sequence("keep", |q, ctx, _| crate::shorts_trim::keep_windows(q, ctx, &[])).is_err());
}

#[test]
fn formats_reject_hostile_options_without_leaving_anything_behind() {
    let (mut s, _) = stream(4.0, &[]);
    let undo = s.history.undo.len();
    let seqs = s.project.sequences().count();
    for p in [
        json!({"platform": "vine"}),
        json!({"format": "wavy"}),
        json!({"seconds": 2}),
        json!({"moments": 1}),
        json!({"captionStyle": "comic"}),
        json!({"side": "middle"}),
        json!({"title": "x".repeat(500)}),
        json!({"loopSeconds": -1}),
    ] {
        assert!(s.execute("shorts.format", p.clone()).is_err(), "{p}");
    }
    assert_eq!((s.history.undo.len(), s.project.sequences().count()), (undo, seqs));
    // captions asked for without a transcript
    let mut bare = Session::default();
    bare.execute("file.openDemoProject", json!({})).unwrap();
    let q = bare.active_sequence().unwrap().clone();
    if crate::transcript::sequence_words(&bare).is_empty() && crate::shorts::base_track(&bare.project, &q).is_some() {
        assert!(bare.execute("shorts.format", json!({"captions": true})).is_err());
        // without: fine, and no captions
        let r = bare.execute("shorts.format", json!({})).unwrap();
        assert_eq!(r["captions"], false);
    }
    // a sequence with nothing in it
    let mut empty = Session::default();
    empty.execute("file.newSequence", json!({"name": "Empty", "width": 1920, "height": 1080})).unwrap();
    assert!(empty.execute("shorts.youtube.gameplay", json!({})).is_err());
}
