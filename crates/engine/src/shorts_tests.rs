//! Tests of the Shorts menu (`shorts`): each command edits as one undo step, rejects hostile
//! parameters, and the result renders.

use std::sync::Arc;

use filmcraft_project::{ClipId, ItemId, ParamValue, Sequence, TrackItem, Transcript, Word};
use filmcraft_speech::FixedTranscriber;
use filmcraft_time::Tick;
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

fn f(it: &TrackItem, effect: &str, param: &str) -> f64 {
    it.effect(effect).unwrap().param(param).unwrap().value.as_f64().unwrap()
}

/// The first footage clip on V1 and the middle of it.
fn v1_clip(s: &Session) -> (ClipId, Tick) {
    let q = s.active_sequence().unwrap();
    let it = q
        .video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .find(|i| matches!(s.project.item(i.item).map(|x| &x.kind), Some(filmcraft_project::ItemKind::Media(_))))
        .unwrap();
    (it.id, it.start + Tick(it.duration.0 / 2))
}

/// Render the active sequence at `t` (RGBA floats); the frame must have the sequence size.
fn render(s: &Session, t: Tick) -> Vec<f32> {
    let id = s.state.active_sequence.unwrap();
    let provider = s.media.full_res_provider(s.project.clone(), s.services.clone());
    let img = filmcraft_render::render_sequence(&s.project, id, t, filmcraft_render::RenderOptions::default(), &provider);
    let q = s.active_sequence().unwrap();
    assert_eq!((img.w, img.h), (q.settings.width as usize, q.settings.height as usize));
    assert_eq!(img.px.len(), img.w * img.h * 4);
    assert!(img.px.iter().all(|v| v.is_finite()));
    img.px
}

/// Whether the top and bottom rows of a rendered frame show picture (no letterbox bars).
fn fills_top_and_bottom(s: &Session, px: &[f32]) -> bool {
    let q = s.active_sequence().unwrap();
    let (w, h) = (q.settings.width as usize, q.settings.height as usize);
    let lit_row = |y: usize| px[y * w * 4..(y + 1) * w * 4].chunks(4).filter(|p| p[0] + p[1] + p[2] > 0.02).count() > w / 2;
    lit_row(2) && lit_row(h - 3)
}

fn menu(id: &str) -> Vec<&'static str> {
    crate::commands::find(id).unwrap().menu.to_vec()
}

#[test]
fn every_command_is_in_sequence_shorts() {
    for id in [
        "shorts.verticalFill",
        "shorts.verticalFollow",
        "shorts.verticalBlur",
        "shorts.stackRight",
        "shorts.stackLeft",
        "shorts.punchIn",
        "shorts.shake",
        "shorts.flash",
        "shorts.captionsPop",
        "shorts.captionsKaraoke",
        "shorts.captionsOneWord",
    ] {
        assert_eq!(menu(id).first().copied(), Some("Shorts"), "{id}");
        assert!(Session::default().execute(id, json!({})).is_err(), "{id} needs a sequence");
    }
}

#[test]
fn vertical_crop_to_fill_makes_a_new_sequence_and_keeps_the_original() {
    let mut s = demo();
    let original = s.state.active_sequence.unwrap();
    let before = seq(&s);
    let (c, t) = v1_clip(&s);
    let r = s.execute("shorts.verticalFill", json!({})).unwrap();
    let new = ItemId(r["sequence"].as_u64().unwrap());
    assert_ne!(new, original);
    assert_eq!(s.state.active_sequence, Some(new), "the vertical copy opens");
    let q = seq(&s);
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    assert_eq!(q.settings.frame_rate, before.settings.frame_rate);
    let it = clip(&s, c);
    // a 1920 x 1080 picture covers 1080 x 1920 at 1920/1080 = 177.8 %
    assert!((f(&it, "motion", "scale") - 1920.0 / 1080.0 * 100.0).abs() < 0.01, "{}", f(&it, "motion", "scale"));
    assert!(!it.scale_to_frame);
    assert_eq!(s.project.sequence(original).unwrap(), &before, "the original sequence is untouched");
    let px = render(&s, t);
    assert!(fills_top_and_bottom(&s, &px), "the reframed picture covers the frame");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.sequence(new).is_none(), "undo removes the copy");
    q.check().unwrap();
}

#[test]
fn vertical_follow_subject_uses_auto_reframe() {
    let mut s = demo();
    let (c, t) = v1_clip(&s);
    s.execute("shorts.verticalFollow", json!({"width": 720, "height": 1280})).unwrap();
    let it = clip(&s, c);
    assert_eq!((seq(&s).settings.width, seq(&s).settings.height), (720, 1280));
    // Motion fills the frame; the effect pans inside a 9:16 window
    assert_eq!(it.effect("auto_reframe").unwrap().param("aspect").unwrap().value, ParamValue::Choice(1));
    assert!((f(&it, "motion", "scale") - 1280.0 / 1080.0 * 100.0).abs() < 0.01);
    let px = render(&s, t);
    assert!(fills_top_and_bottom(&s, &px), "no black bars");
    // square and 4:5 frames pick their own windows; a wide frame is refused
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("shorts.verticalFollow", json!({"width": 1080, "height": 1350})).unwrap();
    assert_eq!(clip(&s, c).effect("auto_reframe").unwrap().param("aspect").unwrap().value, ParamValue::Choice(3));
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.execute("shorts.verticalFollow", json!({"width": 2560, "height": 1080})).is_err());
}

#[test]
fn vertical_blurred_background_adds_a_blurred_copy_underneath() {
    let mut s = demo();
    let (c, t) = v1_clip(&s);
    let tracks = seq(&s).video_tracks.len();
    s.execute("shorts.verticalBlur", json!({})).unwrap();
    let q = seq(&s);
    assert_eq!(q.video_tracks.len(), tracks + 1);
    let (ti, fg) = q.video_tracks.iter().enumerate().find_map(|(i, t)| t.item(c).map(|x| (i, x.clone()))).unwrap();
    // the foreground fits: 1080 / 1920 = 56.25 %
    assert!((f(&fg, "motion", "scale") - 56.25).abs() < 0.01);
    let bg_track = &q.video_tracks[ti - 1];
    assert_eq!(bg_track.name, "Blurred Background");
    let bg = bg_track.items.iter().find(|i| i.start == fg.start && i.duration == fg.duration).unwrap();
    assert_ne!(bg.id, fg.id);
    assert!(bg.link.is_none());
    assert!(bg.effect("gaussian_blur").is_some() && bg.effect("brightness_contrast").is_some());
    assert!(f(bg, "motion", "scale") > 177.0);
    q.check().unwrap();
    render(&s, t);
}

#[test]
fn vertical_rejects_hostile_sizes() {
    let mut s = demo();
    for p in [json!({"width": 0}), json!({"height": -5}), json!({"width": 1e12}), json!({"width": "wide"}), json!({"height": f64::MAX})] {
        assert!(s.execute("shorts.verticalFill", p.clone()).is_err(), "{p}");
    }
}

/// Crop (Rounded Crop) of a clip as [left, top, right, bottom].
fn crop(it: &TrackItem) -> [f64; 4] {
    ["left", "top", "right", "bottom"].map(|k| f(it, "rounded_crop", k))
}

/// The size on screen of what a cropped, scaled clip shows (a 1920 x 1080 source).
fn shown(it: &TrackItem) -> (f64, f64) {
    let c = crop(it);
    let k = f(it, "motion", "scale") / 100.0;
    ((100.0 - c[0] - c[2]) / 100.0 * 1920.0 * k, (100.0 - c[1] - c[3]) / 100.0 * 1080.0 * k)
}

fn pos(it: &TrackItem) -> (f64, f64) {
    let v = it.effect("motion").unwrap().param("position").unwrap().value.as_vec2().unwrap();
    (v.x, v.y)
}

#[test]
fn split_layout_puts_gameplay_on_top_and_the_model_below() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let (c, t) = v1_clip(&s);
    let tracks = seq(&s).video_tracks.len();
    let r = s.execute("shorts.stackRight", json!({"clips": [c.0]})).unwrap();
    let model = ClipId(r["clips"][0]["model"].as_u64().unwrap());
    let q = seq(&s);
    assert_eq!(q.video_tracks.len(), tracks + 2, "a model track and a background track");
    let (game, m) = (clip(&s, c), clip(&s, model));
    // gameplay across the top 60 % (1080 x 1152), the model across the bottom 40 % (1080 x 768)
    assert_eq!(pos(&game), (540.0, 576.0));
    assert_eq!(pos(&m), (540.0, 1536.0));
    let (gw, gh) = shown(&game);
    assert!((gw - 1080.0).abs() < 0.5 && (gh - 1152.0).abs() < 0.5, "{gw} x {gh}");
    let (mw, mh) = shown(&m);
    assert!((mw - 1080.0).abs() < 0.5 && (mh - 768.0).abs() < 0.5, "{mw} x {mh}");
    for it in [&game, &m] {
        assert!(crop(it).iter().all(|v| (0.0..=100.0).contains(v)), "{:?}", crop(it));
        assert!(it.effect("crop").is_none(), "Rounded Crop replaces Crop");
    }
    // the model is cut from the right side of the recording, the gameplay from the rest
    assert!(m.effect("motion").unwrap().param("anchor").unwrap().value.as_vec2().unwrap().x > 1100.0);
    assert!(crop(&game)[2] > 20.0, "the model's side is cut off the gameplay");
    assert!(m.link.is_none());
    q.check().unwrap();
    render(&s, t);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(seq(&s).video_tracks.len(), tracks);
    assert!(seq(&s).find_item(model).is_none());
}

#[test]
fn stack_layout_keeps_all_the_gameplay_and_boxes_the_model() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let (c, t) = v1_clip(&s);
    let r = s.execute("shorts.layout", json!({})).unwrap();
    let model = ClipId(r["clips"][0]["model"].as_u64().unwrap());
    let (game, m) = (clip(&s, c), clip(&s, model));
    // the whole gameplay region beside the model, nothing more cut off
    let g = crop(&game);
    assert!(g[0].abs() < 0.01 && g[1].abs() < 0.01 && (g[2] - 30.0).abs() < 0.01 && g[3].abs() < 0.01, "{g:?}");
    assert!((shown(&game).0 - 1080.0).abs() < 0.5, "full frame width");
    // the model region whole, at its own shape
    let mc = crop(&m);
    assert!((mc[0] - 70.0).abs() < 0.01 && (mc[1] - 36.0).abs() < 0.01 && mc[2].abs() < 0.01 && mc[3].abs() < 0.01, "{mc:?}");
    let (mw, mh) = shown(&m);
    assert!((mw / mh - (0.3 * 1920.0) / (0.64 * 1080.0)).abs() < 0.01, "{mw} x {mh}");
    assert!(mh < 1920.0 * 0.4, "the model takes a third of the frame, not half");
    assert!(f(&m, "rounded_crop", "radius") > 0.0, "rounded corners");
    // the two do not overlap and both fit in the frame
    assert!(pos(&game).1 + shown(&game).1 / 2.0 <= pos(&m).1 - mh / 2.0 + 0.5);
    assert!(pos(&m).1 + mh / 2.0 <= 1920.0 + 0.5);
    render(&s, t);
    assert_eq!(s.state.shorts_layout.as_ref().unwrap()["preset"], "stack");
}

#[test]
fn layout_reapplies_in_place_and_live_changes_are_one_undo_step() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let (c, _) = v1_clip(&s);
    s.execute("shorts.layout", json!({})).unwrap();
    let tracks = seq(&s).video_tracks.len();
    let count_models = |s: &Session| seq(s).video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.name.ends_with(" (Model)")).count();
    let models = count_models(&s);
    assert!(models >= 1);
    let undo = s.history.undo.len();
    s.history.merge_key = None;
    for w in [0.9, 0.8, 0.7] {
        s.execute("shorts.layout", json!({"preset": "stack", "gameWidth": w, "live": true})).unwrap();
    }
    assert_eq!(s.history.undo.len(), undo + 1, "live changes fold into one step");
    assert_eq!(seq(&s).video_tracks.len(), tracks, "the old model and background copies were replaced");
    assert_eq!(count_models(&s), models, "one model pane per clip, however often it is applied");
    assert!((shown(&clip(&s, c)).0 - 1080.0).abs() < 0.5);
    // picture in picture: the model box overlaps the gameplay pane's lower corner
    let r = s.execute("shorts.layout", json!({"preset": "pip", "border": 8, "background": "black"})).unwrap();
    let m = clip(&s, ClipId(r["clips"][0]["model"].as_u64().unwrap()));
    assert!(pos(&m).0 > 540.0);
    assert!(f(&m, "rounded_crop", "border") > 0.0);
    assert!(!seq(&s).video_tracks.iter().any(|t| t.name == "Layout Background"), "black background: no blurred copy");
    seq(&s).check().unwrap();
}

#[test]
fn stacked_layout_rejects_hostile_regions() {
    let mut s = demo();
    for p in [
        json!({"split": 2.0}),
        json!({"split": "half"}),
        json!({"model": [0.5, 0.5, 0.5, 0.5]}),
        json!({"model": [0.0, 0.0, 1.5, 1.0]}),
        json!({"game": [0.0, 0.0, 1.0]}),
        json!({"game": "left"}),
    ] {
        assert!(s.execute("shorts.stackLeft", p.clone()).is_err(), "{p}");
    }
    assert!(s.execute("shorts.stackLeft", json!({"split": 0.6, "model": [0.0, 0.5, 0.3, 1.0]})).is_ok());
}

#[test]
fn seconds_is_how_long_an_effect_lasts_not_when_it_happens() {
    // regression: the generic time parser read `seconds` (an effect's length) as the time to put
    // it at, so a Hook Title of 2.5 s started 2.5 s in and a punch of 1 s landed at 1 s
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let (c, t) = v1_clip(&s);
    s.set_playhead(t);
    s.execute("shorts.punchIn", json!({"clips": [c.0], "seconds": 1.0})).unwrap();
    let scale = clip(&s, c).effect("motion").unwrap().param("scale").unwrap().clone();
    let first = scale.keyframes.first().unwrap().time;
    assert_eq!(first, clip(&s, c).effect_time_at(seq(&s).settings.frame_rate.snap(s.playhead())), "at the playhead");
    let r = s.execute("shorts.hookTitle", json!({"text": "hook", "seconds": 2.5})).unwrap();
    let hook = &seq(&s).video_tracks[r["track"].as_u64().unwrap() as usize].items[0];
    assert_eq!((hook.start, hook.duration), (Tick::ZERO, seq(&s).settings.frame_rate.snap_nearest(Tick::from_seconds_f64(2.5))));
}

#[test]
fn punch_in_zooms_and_comes_back_and_zooms_cropped_panes_inside_their_edges() {
    let mut s = demo();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    let (c, t) = v1_clip(&s);
    let base = f(&clip(&s, c), "motion", "scale");
    s.execute("shorts.punchIn", json!({"clips": [c.0], "time": t.0, "zoom": 150, "seconds": 0.5})).unwrap();
    let it = clip(&s, c);
    let scale = it.effect("motion").unwrap().param("scale").unwrap();
    assert_eq!(scale.keyframes.len(), 4);
    let peak = scale.keyframes.iter().map(|k| k.value.as_f64().unwrap()).fold(0.0, f64::max);
    assert!((peak - base * 1.5).abs() < 1e-6);
    assert!((scale.keyframes[3].value.as_f64().unwrap() - base).abs() < 1e-6, "back to where it was");
    render(&s, t);
    s.execute("edit.undo", json!({})).unwrap();
    assert!(clip(&s, c).effect("motion").unwrap().param("scale").unwrap().keyframes.is_empty());

    // in a stacked layout the crop zooms with the picture, so the pane keeps its size
    s.execute("shorts.stackRight", json!({"clips": [c.0]})).unwrap();
    let left_before = f(&clip(&s, c), "rounded_crop", "left");
    s.execute("shorts.punchIn", json!({"clips": [c.0], "time": t.0})).unwrap();
    let crop = clip(&s, c).effect("rounded_crop").unwrap().param("left").unwrap().clone();
    assert_eq!(crop.keyframes.len(), 4);
    assert!(crop.keyframes[1].value.as_f64().unwrap() > left_before);

    for p in [json!({"zoom": 50}), json!({"zoom": "big"}), json!({"seconds": -1}), json!({"seconds": 1e9})] {
        assert!(s.execute("shorts.punchIn", p.clone()).is_err(), "{p}");
    }
}

#[test]
fn camera_shake_ramps_down_and_sits_before_the_crop() {
    let mut s = demo();
    let (c, t) = v1_clip(&s);
    s.execute("shorts.stackLeft", json!({"clips": [c.0]})).unwrap();
    // with nothing selected, everything under the playhead shakes (both panes; the blurred
    // background is left alone)
    s.state.selection.clear();
    let r = s.execute("shorts.shake", json!({"time": t.0, "amount": 60})).unwrap();
    assert_eq!(r["clips"], 2, "{r}");
    let it = clip(&s, c);
    let shake = it.effects.iter().position(|e| e.effect == "camera_shake").unwrap();
    let crop = it.effects.iter().position(|e| e.effect == "rounded_crop").unwrap();
    assert!(shake < crop, "the picture shakes inside the pane's crop");
    let amount = it.effects[shake].param("amount").unwrap();
    let values: Vec<f64> = amount.keyframes.iter().map(|k| k.value.as_f64().unwrap()).collect();
    assert_eq!(values, [0.0, 60.0, 0.0]);
    render(&s, t);
    render(&s, t + Tick::from_seconds_f64(0.1));
    // a second shake reuses the effect
    s.execute("shorts.shake", json!({"time": (t + Tick::from_seconds_f64(1.0)).0})).unwrap();
    assert_eq!(clip(&s, c).effects.iter().filter(|e| e.effect == "camera_shake").count(), 1);
    assert!(s.execute("shorts.shake", json!({"amount": 0})).is_err());
    // nothing under the playhead
    assert!(s.execute("shorts.shake", json!({"time": Tick::from_seconds_f64(100_000.0).0})).is_err());
}

#[test]
fn impact_flash_is_a_fading_white_graphic() {
    let mut s = demo();
    let (_, t) = v1_clip(&s);
    let r = s.execute("shorts.flash", json!({"time": t.0, "color": "#ff2244"})).unwrap();
    let it = clip(&s, ClipId(r["clip"].as_u64().unwrap()));
    assert_eq!(it.start, seq(&s).settings.frame_rate.snap(t));
    let layer = it.effects.iter().find(|e| e.effect == filmcraft_project::graphic::SHAPE_LAYER).unwrap();
    let fade = layer.param("opacity").unwrap();
    assert_eq!(fade.keyframes.last().unwrap().value.as_f64(), Some(0.0));
    assert_eq!(layer.param("fill_color").unwrap().value, ParamValue::Color([1.0, 34.0 / 255.0, 68.0 / 255.0, 1.0]));
    let lit = render(&s, it.start);
    assert!(lit.chunks(4).filter(|p| p[0] > 0.6).count() > lit.len() / 8, "the flash covers the frame");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(seq(&s).find_item(it.id).is_none());
    for p in [json!({"color": "#12"}), json!({"color": [1, 2]}), json!({"seconds": 0})] {
        assert!(s.execute("shorts.flash", p.clone()).is_err(), "{p}");
    }
}

/// The demo project in a vertical sequence with a transcript of its first A1 clip:
/// "Hello there friends. This is a test of word captions" (two sentences).
fn transcribed() -> Session {
    let mut s = demo();
    let q = s.active_sequence().unwrap();
    let a = &q.audio_tracks[0].items[0];
    let (item, sin) = (a.item, a.source_in);
    let sec = |x: f64| sin + Tick::from_seconds_f64(x);
    let mut t = Transcript { language: "en".into(), ..Default::default() };
    let words = [
        ("Hello", 0.2, 0.5),
        ("there", 0.5, 0.8),
        ("friends.", 0.8, 1.2),
        ("This", 1.4, 1.6),
        ("is", 1.6, 1.7),
        ("a", 1.7, 1.75),
        ("test", 1.75, 2.1),
        ("of", 2.1, 2.2),
        ("word", 2.2, 2.5),
        ("captions", 2.5, 3.0),
    ];
    for (text, a, b) in words {
        t.words.push(Word::new(text, sec(a), sec(b)));
    }
    t.normalize();
    s.transcriber = Some(Arc::new(FixedTranscriber { transcript: t, id: "fixed".into() }));
    s.execute("transcript.generate", json!({"items": [item.0]})).unwrap();
    s.execute("shorts.verticalFill", json!({})).unwrap();
    s
}

fn caption_track(s: &Session, r: &Value) -> Vec<TrackItem> {
    seq(s).video_tracks[r["track"].as_u64().unwrap() as usize].items.clone()
}

fn text_of(it: &TrackItem) -> String {
    match &it.effects.iter().find(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER).unwrap().param("text").unwrap().value {
        ParamValue::Text(t) => t.clone(),
        v => panic!("{v:?}"),
    }
}

#[test]
fn word_captions_pop_highlights_each_spoken_word() {
    let mut s = transcribed();
    let tracks = seq(&s).video_tracks.len();
    let r = s.execute("shorts.captionsPop", json!({})).unwrap();
    let items = caption_track(&s, &r);
    assert_eq!(seq(&s).video_tracks.len(), tracks + 1);
    assert_eq!(items.len(), 10, "one clip per word: {r}");
    // 16 characters fit across a vertical frame: "HELLO THERE FRIENDS" is two phrases (and
    // punctuation is off by default)
    assert_eq!(text_of(&items[0]), "HELLO THERE");
    assert_eq!(text_of(&items[1]), "HELLO THERE");
    assert_eq!(text_of(&items[2]), "FRIENDS");
    assert_eq!(text_of(&items[3]), "THIS IS A", "a sentence ends a phrase");
    for p in items.windows(2) {
        assert!(p[0].end() <= p[1].start, "no overlaps");
    }
    // the second clip highlights the second word only
    let layer = items[1].effects.iter().find(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER).unwrap();
    let runs = &layer.layer.as_ref().unwrap().runs;
    assert_eq!((runs.len(), runs[0].start, runs[0].end), (1, 6, 11));
    // a phrase pops in; the next word of the same phrase does not
    assert_eq!(items[0].effects.last().unwrap().param("scale").unwrap().keyframes.len(), 3);
    assert!(items[1].effects.last().unwrap().param("scale").unwrap().keyframes.is_empty());
    seq(&s).check().unwrap();
    render(&s, items[1].start);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(seq(&s).video_tracks.len(), tracks);
}

#[test]
fn word_captions_karaoke_and_one_word() {
    let mut s = transcribed();
    let r = s.execute("shorts.captionsKaraoke", json!({"maxWords": 4, "maxChars": 40, "highlight": "#00ffff", "uppercase": false})).unwrap();
    let items = caption_track(&s, &r);
    let runs =
        |it: &TrackItem| it.effects.iter().find(|e| e.effect == filmcraft_project::graphic::TEXT_LAYER).unwrap().layer.as_ref().map_or(0, |l| l.runs.len());
    assert_eq!(text_of(&items[0]), "Hello there friends");
    // karaoke lights the words spoken so far
    assert_eq!((runs(&items[0]), runs(&items[1]), runs(&items[2])), (1, 2, 3));
    render(&s, items[2].start);

    let r = s.execute("shorts.captionsOneWord", json!({})).unwrap();
    let items = caption_track(&s, &r);
    assert_eq!(items.iter().map(text_of).collect::<Vec<_>>()[..3], ["HELLO", "THERE", "FRIENDS"]);
    assert!(items.iter().all(|i| i.effects.last().unwrap().param("scale").unwrap().keyframes.len() >= 3), "every word pops");
    render(&s, items[0].start);
}

#[test]
fn word_captions_need_a_transcript_and_take_hostile_options() {
    let mut s = demo();
    let e = s.execute("shorts.captionsPop", json!({})).unwrap_err().to_string();
    assert!(e.contains("Transcribe"), "{e}");
    let mut s = transcribed();
    for p in [
        json!({"maxWords": 0}),
        json!({"maxWords": 1e9}),
        json!({"size": -1}),
        json!({"color": "red"}),
        json!({"highlight": [1]}),
        json!({"position": [1]}),
        json!({"position": "middle"}),
        json!({"hold": -2}),
    ] {
        assert!(s.execute("shorts.captionsPop", p.clone()).is_err(), "{p}");
    }
    // a huge size is fitted inside the frame
    let r = s.execute("shorts.captionsPop", json!({"size": 1999})).unwrap();
    let items = caption_track(&s, &r);
    let size = items[0].effects.last().unwrap().param("size").unwrap().value.as_f64().unwrap();
    assert!(size < 200.0, "{size}");
}
