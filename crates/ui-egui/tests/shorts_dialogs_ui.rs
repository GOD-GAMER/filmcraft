//! Headless UI tests of Sequence ▸ Shorts ▸ Layout… and Word Captions…: the windows open from
//! the menu, preview their command live (one undo step), Cancel undoes the preview, OK keeps it,
//! and the Layout picker's boxes can be dragged.

use std::sync::mpsc::{Sender, channel};

use egui_kittest::Harness;
use filmcraft_engine::Session;
use filmcraft_ui_egui::FilmcraftApp;
use filmcraft_ui_egui::control::ControlRequest;
use serde_json::{Value, json};

struct Driver {
    harness: Harness<'static, FilmcraftApp>,
    tx: Sender<ControlRequest>,
}

impl Driver {
    fn new() -> Self {
        let mut s = Session::default();
        s.execute("file.openDemoProject", json!({})).unwrap();
        s.execute("shorts.verticalFill", json!({})).unwrap();
        let (tx, rx) = channel();
        let app = FilmcraftApp::new(s).with_control(rx);
        let harness = Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).with_max_steps(10_000).build_eframe(move |_cc| app);
        let mut d = Driver { harness, tx };
        d.frames(4);
        d
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            let ctx = self.harness.ctx.clone();
            let mut raw = std::mem::take(self.harness.input_mut());
            eframe::App::raw_input_hook(self.harness.state_mut(), &ctx, &mut raw);
            *self.harness.input_mut() = raw;
            self.harness.step();
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        let (req, reply) = ControlRequest::new(method, params.clone());
        self.tx.send(req).unwrap();
        for _ in 0..600 {
            self.frames(1);
            if let Ok(v) = reply.try_recv() {
                return v;
            }
        }
        panic!("no reply to {method} {params}");
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.call(method, params.clone());
        assert_eq!(v["ok"], json!(true), "{method} {params} failed: {v}");
        v["result"].clone()
    }

    fn exec(&mut self, command: &str, params: Value) -> Value {
        self.ok("engine.execute", json!({"command": command, "params": params}))
    }

    fn click(&mut self, id: &str) {
        self.ok("ui.click", json!({"id": id}));
        self.frames(3);
    }

    /// Long enough for a live preview to be applied.
    fn settle(&mut self) {
        self.frames(40);
    }

    fn app(&mut self) -> &mut FilmcraftApp {
        self.harness.state_mut()
    }

    fn tracks(&mut self) -> Vec<String> {
        self.app().session.active_sequence().unwrap().video_tracks.iter().map(|t| t.name.clone()).collect()
    }

    fn undo_len(&mut self) -> usize {
        self.app().session.history.undo.len()
    }

    /// A transcript for the first A1 clip's media: "one two three four five six".
    fn transcribe(&mut self) {
        let q = self.app().session.active_sequence().unwrap().clone();
        let a = &q.audio_tracks[0].items[0];
        let words: Vec<Value> = ["one", "two", "three", "four", "five", "six"]
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let t = a.source_in.0 + (i as i64) * 254_016_000_000 / 3;
                json!({"text": w, "start": t, "end": t + 254_016_000_000 / 4})
            })
            .collect();
        self.exec("transcript.set", json!({"item": a.item.0, "transcript": {"language": "en", "words": words}}));
    }
}

#[test]
fn layout_window_previews_live_and_cancel_undoes_it() {
    let mut d = Driver::new();
    let (tracks, undo) = (d.tracks(), d.undo_len());
    d.ok("ui.menu.invoke", json!({"id": "shorts.layout"}));
    d.settle();
    assert!(d.app().ui.shorts.layout.is_some(), "the window is open");
    let now = d.tracks();
    assert!(now.contains(&"Model".to_string()) && now.contains(&"Layout Background".to_string()), "previewed on open: {now:?}");
    // drag the model box (pink) to the left in the picker
    let before = d.app().session.state.shorts_layout.clone().unwrap()["model"][0].as_f64().unwrap();
    d.ok("ui.drag", json!({"from": {"id": "shortsLayout.picker", "fx": 0.85, "fy": 0.7}, "to": {"id": "shortsLayout.picker", "fx": 0.7, "fy": 0.7}}));
    d.settle();
    let after = d.app().session.state.shorts_layout.clone().unwrap()["model"][0].as_f64().unwrap();
    assert!((before - after - 0.15).abs() < 0.03, "{before} → {after}");
    assert_eq!(d.undo_len(), undo + 1, "every preview is one undo step");
    d.click("shortsLayout.cancel");
    assert!(d.app().ui.shorts.layout.is_none());
    assert_eq!(d.tracks(), tracks, "Cancel undoes the preview");
    assert_eq!(d.undo_len(), undo);
}

#[test]
fn layout_window_presets_and_ok() {
    let mut d = Driver::new();
    d.ok("ui.menu.invoke", json!({"id": "shorts.layout"}));
    d.settle();
    d.click("shortsLayout.preset.pip");
    d.settle();
    assert_eq!(d.app().session.state.shorts_layout.clone().unwrap()["preset"], "pip");
    let border = d.app().ui.shorts.layout.as_ref().unwrap().params["border"].as_f64().unwrap();
    assert!(border > 0.0, "the window shows the preset's own values");
    d.click("shortsLayout.ok");
    assert!(d.app().ui.shorts.layout.is_none());
    assert!(d.tracks().contains(&"Model".to_string()), "OK keeps it");
}

#[test]
fn captions_window_needs_a_transcript_then_previews_each_style() {
    let mut d = Driver::new();
    let v = d.call("ui.menu.invoke", json!({"id": "shorts.captions"}));
    assert_ne!(v["ok"], json!(true), "no transcript yet: {v}");
    assert!(d.app().ui.shorts.captions.is_none());
    d.transcribe();
    let undo = d.undo_len();
    d.ok("ui.menu.invoke", json!({"id": "shorts.captions"}));
    d.settle();
    assert!(d.tracks().contains(&"Word Captions".to_string()));
    d.click("shortsCaptions.style.box");
    d.settle();
    let q = d.app().session.active_sequence().unwrap().clone();
    let caps = q.video_tracks.iter().find(|t| t.name == "Word Captions").unwrap();
    assert!(
        caps.items.iter().any(|i| i.effects.iter().any(|e| e.effect == filmcraft_engine::project::graphic::SHAPE_LAYER)),
        "a highlight box behind the word"
    );
    assert_eq!(q.video_tracks.iter().filter(|t| t.name == "Word Captions").count(), 1, "replaced, not stacked");
    d.click("shortsCaptions.ok");
    assert!(d.app().ui.shorts.captions.is_none());
    assert_eq!(d.undo_len(), undo + 1);
    assert_eq!(d.app().session.state.shorts_captions.clone().unwrap()["style"], "box");
}
