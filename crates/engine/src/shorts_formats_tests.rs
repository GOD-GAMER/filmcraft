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
        // the meme folder can be filled before there is a sequence
        if c.id != "shorts.memeFolder" {
            assert!(Session::default().execute(c.id, json!({})).is_err(), "{} needs a sequence", c.id);
        }
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

#[test]
fn auto_edit_tries_versions_checks_them_and_keeps_the_best() {
    let (mut s, _) = stream(12.0, &[(6.0, -6.0)]);
    let stream_seq = s.state.active_sequence;
    let (undo, seqs) = (s.history.undo.len(), s.project.sequences().count());
    let r = s.execute("shorts.autoEdit", json!({"platform": "tiktok", "style": "formats"})).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    assert_eq!(s.project.sequences().count(), seqs + 1, "the trials leave nothing behind");
    for e in &s.events {
        if let crate::Event::OpenSequence(id) = e {
            assert!(s.project.sequence(*id).is_some(), "no event about a trial's sequence");
        }
    }
    let versions = r["versions"].as_array().unwrap();
    assert!(versions.len() >= 3, "{r}");
    let checks = ["textClear", "modelInView", "screenUsed", "gameplaySize", "fitsClip", "captionClash", "readable"];
    for v in versions {
        let ids: Vec<&str> = v["checks"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert_eq!(ids, checks, "{v}");
        let score = v["score"].as_f64().unwrap();
        assert!((0.0..=100.0).contains(&score));
        let sum: f64 = v["checks"].as_array().unwrap().iter().map(|c| c["points"].as_f64().unwrap()).sum();
        assert!((sum - score).abs() <= 1.0, "the score is the checks' points: {v}");
    }
    let picked = &versions[r["picked"].as_u64().unwrap() as usize];
    let best = versions.iter().map(|v| v["score"].as_f64().unwrap()).fold(0.0, f64::max);
    assert_eq!(picked["score"].as_f64().unwrap(), best);
    // what was made: a TikTok short of the picked format with captions, punch-ins and a loop
    let q = seq(&s);
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    let names = track_names(&s);
    assert!(names.iter().any(|n| n == "Word Captions") && names.iter().any(|n| n == "Progress Bar"), "{names:?}");
    assert!(q.video_tracks.iter().flat_map(|t| t.items.iter()).any(crate::shorts_trim::is_loop_hold));
    let steps: Vec<&str> = r["steps"].as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert!(steps.contains(&"captions") && steps.contains(&"loop") && steps.contains(&"punchIns"), "{steps:?}");
    assert_eq!(s.state.shorts_auto.as_ref().unwrap()["picked"], r["picked"]);
    render(&s, sec(3.0));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.state.active_sequence.map(|id| s.project.sequence(id).is_some()), Some(true));
    assert_eq!(s.project.sequences().count(), seqs);
    let _ = stream_seq;
    // one version picked by hand
    let r = s.execute("shorts.autoEdit", json!({"platform": "youtube", "pick": {"format": "fullFrame"}, "hype": false, "finish": false})).unwrap();
    assert_eq!(r["versions"].as_array().unwrap().len(), 1);
    assert!(track_names(&s).iter().any(|n| n == "Title"));
    assert!(!seq(&s).video_tracks.iter().flat_map(|t| t.items.iter()).any(crate::shorts_trim::is_loop_hold), "no finishing asked for");
    // hostile options
    for p in [
        json!({"platform": "vine"}),
        json!({"length": 1}),
        json!({"length": "forever"}),
        json!({"pick": {"format": "wavy"}}),
        json!({"title": "x".repeat(300)}),
        json!({"style": "wavy"}),
        json!({"memesFolder": 3}),
        json!({"memesFolder": "x".repeat(5000)}),
    ] {
        assert!(s.execute("shorts.autoEdit", p.clone()).is_err(), "{p}");
    }
}

/// A folder of memes for the tests: a still picture named after a word, and `extra` files.
fn meme_folder(name: &str, extra: &[(&str, &[u8])]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("filmcraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let img = image::RgbaImage::from_pixel(64, 48, image::Rgba([250, 30, 200, 255]));
    img.save(dir.join("actually.png")).unwrap();
    for (f, b) in extra {
        std::fs::write(dir.join(f), b).unwrap();
    }
    dir
}

#[test]
fn reaction_split_halves_the_frame_switches_scenes_and_ends_in_black_and_white() {
    let (mut s, _) = stream(24.0, &[(10.0, -6.0)]);
    let memes = meme_folder("reaction", &[]);
    let undo = s.history.undo.len();
    let r = s.execute("shorts.youtube.reaction", json!({"memesFolder": memes.to_string_lossy()})).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1, "one undo step");
    let q = seq(&s);
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    // the gameplay on the top half, the model filling the bottom half
    let (pane, model) = crate::shorts_layout::current_rects(&s).unwrap();
    assert!((pane[3] - 0.5).abs() < 0.01 && pane[1] < 0.01, "{pane:?}");
    let model = model.unwrap();
    assert!((model[1] - 0.5).abs() < 0.01 && model[3] > 0.99 && model[2] - model[0] > 0.99, "{model:?}");
    // the scenes: contiguous from the start to the end, gameplay full screen somewhere, the last
    // line on the face
    let rx = &r["reaction"];
    let scenes = rx["scenes"].as_array().unwrap();
    assert_eq!(scenes[0]["start"], 0.0, "{rx}");
    for w in scenes.windows(2) {
        assert!((w[0]["end"].as_f64().unwrap() - w[1]["start"].as_f64().unwrap()).abs() < 1e-9, "{rx}");
    }
    assert!((scenes.last().unwrap()["end"].as_f64().unwrap() - q.duration().seconds()).abs() < 0.05, "{rx}");
    assert_eq!(scenes.last().unwrap()["scene"], "face", "{rx}");
    assert!(scenes.iter().any(|g| g["scene"] == "game"), "{rx}");
    // where the gameplay fills the frame there is no model; the face covers the whole frame
    let at = |k: &str| scenes.iter().find(|g| g["scene"] == k).map(|g| (g["start"].as_f64().unwrap() + g["end"].as_f64().unwrap()) / 2.0).unwrap();
    let on = |track: &str, t: f64| {
        q.video_tracks.iter().filter(|x| x.name == track).flat_map(|x| x.items.iter()).any(|i| i.start.seconds() <= t && i.end().seconds() > t)
    };
    assert!(!on("Model", at("game")) && on("Video 1", at("game")), "full-screen gameplay");
    assert!(on("Model", at("face")), "full-screen face");
    let face = render(&s, sec(at("face")));
    let split = render(&s, sec(1.0));
    let fw = q.settings.width as usize;
    let row = |px: &[f32], y: usize| px[y * fw * 4..(y + 1) * fw * 4].to_vec();
    let quarter = q.settings.height as usize / 4;
    assert!(diff(&row(&face, quarter), &row(&split, quarter)) > 0.01, "the top half shows the face, not the gameplay");
    let footage = |q: &Sequence| q.video_tracks.iter().find(|t| t.name == "Video 1").map(|t| t.items.iter().map(|i| i.duration.0).sum::<i64>()).unwrap();
    let whole = footage(&q);
    // bass hits on their own track, the black-and-white ending on the grade track, a meme
    let hits = rx["hits"].as_u64().unwrap();
    assert!(hits >= 1, "{rx}");
    assert_eq!(q.audio_tracks.iter().filter(|t| t.name == "Bass Hits").map(|t| t.items.len() as u64).sum::<u64>(), hits);
    assert!(q.video_tracks.iter().filter(|t| t.name == "Grade").flat_map(|t| t.items.iter()).any(|i| i.name == "Black-and-White Ending"), "{rx}");
    assert!(rx["ending"].is_array(), "{rx}");
    let drops = rx["memes"].as_array().unwrap();
    assert_eq!(drops.first().map(|m| m["word"].as_str()), Some(Some("actually")), "{rx}");
    assert!(q.video_tracks.iter().any(|t| t.name == "Memes" && !t.items.is_empty()));
    // captions on the seam (under the face where it fills the frame), no progress bar or loop
    let ys = caption_ys(&s);
    assert!(ys.iter().filter(|y| (*y - 0.5).abs() < 0.03).count() > ys.len() / 2, "{ys:?}");
    assert!(ys.iter().any(|y| *y > 0.7), "under the face: {ys:?}");
    assert_eq!(s.state.shorts_captions.as_ref().unwrap()["style"], "reaction");
    let names = track_names(&s);
    assert_eq!(count(&names, "Progress Bar"), 0, "{names:?}");
    assert!(!q.video_tracks.iter().flat_map(|t| t.items.iter()).any(crate::shorts_trim::is_loop_hold));
    for t in [1.0, at("game"), at("face"), q.duration().seconds() - 0.1] {
        render(&s, sec(t));
    }
    // applied again in place: the same short, nothing doubled, the footage whole
    let r = s.execute("shorts.youtube.reaction", json!({"memesFolder": memes.to_string_lossy()})).unwrap();
    let q = seq(&s);
    assert_eq!(footage(&q), whole, "the footage is whole");
    assert_eq!(r["reaction"]["hits"].as_u64(), Some(hits));
    assert_eq!(q.audio_tracks.iter().filter(|t| t.name == "Bass Hits").map(|t| t.items.len() as u64).sum::<u64>(), hits, "hits not doubled");
    let flashes = q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.name == "Punchline Flash").count();
    assert_eq!(flashes, 1);
    assert_eq!(q.video_tracks.iter().filter(|t| t.name == "Memes").count(), 1);
    assert_eq!(count(&track_names(&s), "Grade"), 1);
    // each part can be left out
    let r = s.execute("shorts.youtube.reaction", json!({"scenes": false, "hits": false, "ending": false, "memes": false})).unwrap();
    let rx = &r["reaction"];
    assert_eq!(rx["scenes"].as_array().unwrap().len(), 1, "{rx}");
    assert_eq!((rx["hits"].as_u64(), rx["ending"].is_null(), rx["memes"].as_array().map(Vec::len)), (Some(0), true, Some(0)), "{rx}");
    let q = seq(&s);
    assert!(!q.audio_tracks.iter().any(|t| t.name == "Bass Hits" && !t.items.is_empty()));
    assert!(!q.video_tracks.iter().any(|t| (t.name == "Grade" || t.name == "Memes") && !t.items.is_empty()));
    assert!(!q.video_tracks.iter().flat_map(|t| t.items.iter()).any(|i| i.name == "Punchline Flash"));
    assert_eq!(footage(&q), whole);
    // another format after it in place: none of the Reaction Split is left
    s.execute("shorts.youtube.gameplay", json!({})).unwrap();
    let q = seq(&s);
    assert_eq!(footage(&q), whole);
    assert!(!q.audio_tracks.iter().any(|t| t.name == "Bass Hits"));
    let _ = std::fs::remove_dir_all(&memes);
}

#[test]
fn auto_edit_makes_the_reaction_split_by_default() {
    let (mut s, _) = stream(16.0, &[(6.0, -6.0)]);
    let r = s.execute("shorts.autoEdit", json!({"platform": "youtube", "memes": false})).unwrap();
    let picked = &r["versions"][r["picked"].as_u64().unwrap() as usize];
    assert_eq!(picked["format"], "reaction", "{r}");
    assert_eq!(r["options"]["style"], "reaction");
    assert!(r["reaction"]["scenes"].is_array(), "{r}");
    let steps: Vec<&str> = r["steps"].as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert!(steps.contains(&"captions") && steps.contains(&"scenes") && steps.contains(&"ending"), "{steps:?}");
    assert!(!steps.contains(&"loop"), "its own ending: {steps:?}");
    assert_eq!(crate::shorts_auto::current(&s)["style"], "reaction");
}

#[test]
fn meme_drops_and_bass_hits_from_the_menu() {
    let (mut s, _) = stream(10.0, &[]);
    // the meme folder: made when missing, its memes and the sound that replaces the bass hit
    let dir = meme_folder("menu", &[]);
    let fresh = dir.join("new");
    let r = s.execute("shorts.memeFolder", json!({"folder": fresh.to_string_lossy()})).unwrap();
    assert!(fresh.is_dir() && r["memes"].as_array().unwrap().is_empty() && r["hitSound"].is_null(), "{r}");
    let r = Session::default().execute("shorts.memeFolder", json!({"folder": dir.to_string_lossy()})).unwrap();
    assert_eq!(r["memes"][0]["words"][0], "actually", "{r}");
    // dropped where its word is said, one undo step; the word's caption is purple
    let undo = s.history.undo.len();
    let r = s.execute("shorts.memeDrops", json!({"folder": dir.to_string_lossy()})).unwrap();
    assert_eq!(s.history.undo.len(), undo + 1);
    let t = r["drops"][0]["seconds"].as_f64().unwrap();
    let words = crate::transcript::sequence_words(&s);
    let w = words.iter().find(|w| w.text == "actually").unwrap();
    assert!((w.start.seconds() - t).abs() < 0.1, "{r}");
    let q = seq(&s);
    let memes = q.video_tracks.iter().find(|t| t.name == "Memes").unwrap();
    assert!(memes.items[0].start.seconds() <= w.start.seconds() && memes.items[0].end().seconds() > w.end.seconds());
    render(&s, w.start + sec(0.1));
    // one file at the playhead
    let r = s.execute("shorts.memeDrop", json!({"path": dir.join("actually.png").to_string_lossy(), "time": sec(8.0).0})).unwrap();
    assert!(r["seconds"].as_f64().unwrap() > 0.5, "{r}");
    // the bass hit: built in, or the folder's own
    let r = s.execute("shorts.bassHit", json!({"time": sec(2.0).0})).unwrap();
    assert_eq!(r["hits"], 1, "{r}");
    let q = seq(&s);
    let hits = q.audio_tracks.iter().find(|t| t.name == "Bass Hits").unwrap();
    assert_eq!(hits.items.len(), 1);
    // hostile or missing files and folders
    for (id, p) in [
        ("shorts.memeDrop", json!({})),
        ("shorts.memeDrop", json!({"path": dir.join("missing.png").to_string_lossy()})),
        ("shorts.memeDrop", json!({"path": dir.join("notes.txt").to_string_lossy()})),
        ("shorts.memeDrops", json!({"folder": dir.join("missing").to_string_lossy()})),
        ("shorts.bassHit", json!({"time": sec(60.0).0})),
    ] {
        assert!(s.execute(id, p.clone()).is_err(), "{id} {p}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
