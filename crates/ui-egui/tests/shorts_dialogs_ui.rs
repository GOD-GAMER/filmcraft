//! Headless UI tests of the Shorts menu: Formats…, Layout… and Word Captions… open from the
//! menu, preview their command live (one undo step), Cancel undoes the preview, OK keeps it, and
//! the Layout picker's boxes can be dragged; Safe Zones and Export for… work from the menu.

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
        Self::with(s)
    }

    fn with(s: Session) -> Self {
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

#[test]
fn shorts_is_a_menu_of_its_own_with_safe_zones_and_export() {
    use filmcraft_ui_egui::menus::{MENUS, menu_items};
    let mut d = Driver::new();
    let pos = |m: &str| MENUS.iter().position(|x| *x == m).unwrap();
    assert_eq!(pos("Shorts"), pos("Sequence") + 1, "next to Sequence in the menu bar");
    let items = menu_items(d.app());
    assert!(!items.iter().any(|i| i.path.first().map(String::as_str) == Some("Sequence") && i.id.starts_with("shorts.")), "nothing left under Sequence");
    let ids: Vec<String> = items.iter().filter(|i| i.path.first().map(String::as_str) == Some("Shorts")).map(|i| i.id.clone()).collect();
    assert_eq!(ids.iter().take(2).map(String::as_str).collect::<Vec<_>>(), ["shorts.autoEdit", "shorts.format"], "Auto Edit… and Formats… first");
    let at = |id: &str| ids.iter().position(|x| x == id).unwrap_or_else(|| panic!("{id} in {ids:?}"));
    assert!(at("view.safeZones.tiktok") > at("shorts.loopEnding") && at("shorts.exportTikTok") < at("shorts.verticalFill"), "{ids:?}");
    // Safe Zones: one at a time, drawn over the Program monitor
    let checked = |d: &mut Driver, id: &str| menu_items(d.app()).into_iter().find(|i| i.id == id).and_then(|i| i.checked);
    d.ok("ui.menu.invoke", json!({"id": "view.safeZones.tiktok"}));
    assert_eq!(d.app().ui.program.safe_zone, "tiktok");
    assert_eq!((checked(&mut d, "view.safeZones.tiktok"), checked(&mut d, "view.safeZones.youtube")), (Some(true), Some(false)));
    d.frames(3);
    d.ok("ui.menu.invoke", json!({"id": "view.safeZones.off"}));
    assert!(d.app().ui.program.safe_zone.is_empty());
    assert_eq!(checked(&mut d, "view.safeZones.off"), Some(true));
    // Export for TikTok…: Export mode with its preset
    d.ok("ui.menu.invoke", json!({"id": "shorts.exportTikTok"}));
    assert_eq!(format!("{:?}", d.app().ui.mode), "Export");
    assert_eq!(d.app().ui.export.preset, "TikTok 1080×1920");
    d.frames(3);
    let notes: Vec<String> = d.call("ui.elements", json!({})).to_string().matches("export.platformNote").map(str::to_string).collect();
    assert!(!notes.is_empty(), "the length is checked against TikTok's");
}

#[test]
fn formats_window_previews_each_choice_and_cancel_puts_the_stream_back() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let mut d = Driver::with(s);
    let stream = d.app().session.state.active_sequence;
    let undo = d.undo_len();
    d.ok("ui.menu.invoke", json!({"id": "shorts.format"}));
    d.settle();
    d.settle();
    assert!(d.app().ui.shorts.format.is_some(), "the window is open");
    let q = d.app().session.active_sequence().unwrap().clone();
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920), "a vertical short is previewed");
    assert_ne!(d.app().session.state.active_sequence, stream);
    assert_eq!(d.app().ui.program.safe_zone, "youtube", "with YouTube's safe zones");
    assert_eq!(d.undo_len(), undo + 1, "one undo step");
    // another platform and format: still one step, made again from the stream
    d.click("shortsFormats.platform.tiktok");
    d.click("shortsFormats.format.meme");
    d.settle();
    d.settle();
    let r = d.app().ui.shorts.format.clone().unwrap().result;
    assert_eq!((r["platform"].as_str(), r["format"].as_str()), (Some("tiktok"), Some("meme")), "{r}");
    assert_eq!(d.app().ui.program.safe_zone, "tiktok");
    assert!(d.tracks().contains(&"Meme Bar".to_string()));
    assert_eq!(d.undo_len(), undo + 1);
    let shorts = d.app().session.project.sequences().filter(|i| i.name.contains("TikTok") || i.name.contains("YouTube")).count();
    assert_eq!(shorts, 1, "previews replace each other");
    // Cancel: the stream again, nothing left behind
    d.click("shortsFormats.cancel");
    assert!(d.app().ui.shorts.format.is_none());
    assert_eq!(d.app().session.state.active_sequence, stream);
    assert_eq!(d.undo_len(), undo);
    assert!(d.app().ui.program.safe_zone.is_empty());
    // OK keeps it; Export for… goes on to Export mode with the preset
    d.ok("ui.menu.invoke", json!({"id": "shorts.format"}));
    d.settle();
    d.settle();
    d.click("shortsFormats.export");
    assert!(d.app().ui.shorts.format.is_none());
    assert_eq!(d.undo_len(), undo + 1);
    assert_eq!(format!("{:?}", d.app().ui.mode), "Export");
    assert_eq!(d.app().ui.export.preset, "TikTok 1080×1920", "the platform last used");
}

#[test]
fn auto_trim_asks_for_the_length() {
    let mut d = Driver::new();
    d.ok("ui.menu.invoke", json!({"id": "shorts.autoTrim"}));
    d.frames(3);
    let els = d.call("ui.elements", json!({})).to_string();
    assert!(els.contains("shortsTrim.seconds") && els.contains("shortsTrim.preset.15"), "the dialog is open");
}

#[test]
fn auto_edit_window_runs_reports_and_can_switch_versions() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let mut d = Driver::with(s);
    let stream = d.app().session.state.active_sequence;
    let undo = d.undo_len();
    d.ok("ui.menu.invoke", json!({"id": "shorts.autoEdit"}));
    d.frames(3);
    assert!(d.app().ui.shorts.auto.is_some(), "the window is open");
    assert_eq!(d.undo_len(), undo, "nothing happens until asked");
    d.click("shortsAuto.platform.tiktok");
    // the other formats (the Reaction Split, the default, is one version)
    d.click("shortsAuto.style.formats");
    d.click("shortsAuto.run");
    d.settle();
    let r = d.app().ui.shorts.auto.clone().unwrap().result;
    assert_eq!(r["platform"], "tiktok", "{r}");
    let versions = r["versions"].as_array().unwrap().clone();
    assert!(versions.len() >= 3, "{r}");
    assert_eq!(d.undo_len(), undo + 1, "one undo step");
    assert_eq!(d.app().ui.program.safe_zone, "tiktok");
    let els = d.call("ui.elements", json!({})).to_string();
    assert!(els.contains("shortsAuto.check.0") && els.contains("shortsAuto.use."), "the report is shown");
    // another version from the report: still one step
    let picked = r["picked"].as_u64().unwrap() as usize;
    let other = (0..versions.len()).find(|i| *i != picked).unwrap();
    d.click(&format!("shortsAuto.use.{other}"));
    d.settle();
    let r2 = d.app().ui.shorts.auto.clone().unwrap().result;
    assert_eq!(r2["versions"][0]["format"], versions[other]["format"]);
    assert_eq!(d.undo_len(), undo + 1);
    // Cancel puts the stream back
    d.click("shortsAuto.cancel");
    assert!(d.app().ui.shorts.auto.is_none());
    assert_eq!(d.undo_len(), undo);
    assert_eq!(d.app().session.state.active_sequence, stream);
    // OK straight away runs it with the options last used, and keeps it
    d.ok("ui.menu.invoke", json!({"id": "shorts.autoEdit"}));
    d.frames(3);
    d.click("shortsAuto.ok");
    assert!(d.app().ui.shorts.auto.is_none());
    assert_eq!(d.undo_len(), undo + 1);
    assert_eq!(d.app().session.state.shorts_auto.clone().unwrap()["platform"], "tiktok");
}

#[test]
fn auto_edit_makes_the_reaction_split_by_default() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let mut d = Driver::with(s);
    let undo = d.undo_len();
    d.ok("ui.menu.invoke", json!({"id": "shorts.autoEdit"}));
    d.frames(3);
    let els = d.call("ui.elements", json!({})).to_string();
    assert!(els.contains("shortsAuto.style.reaction") && els.contains("shortsAuto.memes"), "the style and meme drops are offered");
    assert_eq!(d.app().ui.shorts.auto.clone().unwrap().params["style"], "reaction", "the Reaction Split is the default");
    assert!(!els.contains("shortsAuto.tryFormats"), "trying formats is for the other style");
    // a meme folder of its own
    let memes = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("auto-edit-memes");
    d.app().ui.shorts.auto.as_mut().unwrap().params["memesFolder"] = json!(memes.to_string_lossy());
    d.frames(2);
    d.click("shortsAuto.openMemes");
    assert!(memes.is_dir(), "Open Meme Folder makes the folder");
    assert!(d.app().ui.extras.opened.iter().any(|o| std::path::Path::new(o) == memes), "and shows it");
    d.click("shortsAuto.run");
    d.settle();
    let r = d.app().ui.shorts.auto.clone().unwrap().result;
    assert_eq!(r["versions"][0]["format"], "reaction", "{r}");
    assert_eq!(d.undo_len(), undo + 1, "one undo step");
    let q = d.app().session.active_sequence().unwrap().clone();
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920));
    assert!(r["reaction"]["scenes"].is_array(), "{r}");
    assert!(d.call("ui.elements", json!({})).to_string().contains("shortsAuto.export"), "the report is shown");
    snapshot(&mut d, "auto-edit-reaction.png");
    d.click("shortsAuto.ok");
    assert!(d.app().ui.shorts.auto.is_none());
}

/// A picture of the window, when `FILMCRAFT_UI_SNAPSHOT_DIR` names a folder for it.
fn snapshot(d: &mut Driver, name: &str) {
    if let Some(dir) = std::env::var_os("FILMCRAFT_UI_SNAPSHOT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        d.harness.render().unwrap().save(dir.join(name)).unwrap();
    }
}

#[test]
fn formats_window_offers_the_reaction_split_and_its_options() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let mut d = Driver::with(s);
    d.ok("ui.menu.invoke", json!({"id": "shorts.format"}));
    d.settle();
    d.click("shortsFormats.format.reaction");
    d.settle();
    d.settle();
    let r = d.app().ui.shorts.format.clone().unwrap().result;
    assert_eq!(r["format"], "reaction", "{r}");
    let p = d.app().ui.shorts.format.clone().unwrap().params;
    assert_eq!((p["bleep"].as_bool(), p["progressBar"].as_bool()), (Some(true), Some(false)), "its own defaults, not the last format's: {p}");
    let els = d.call("ui.elements", json!({})).to_string();
    for id in ["shortsFormats.scenes", "shortsFormats.hits", "shortsFormats.ending", "shortsFormats.bleep", "shortsFormats.memes"] {
        assert!(els.contains(id), "{id}");
    }
    assert!(!els.contains("shortsFormats.loop\""), "its own ending, not a loop");
    snapshot(&mut d, "formats-reaction.png");
    // the black-and-white ending off: made again without it
    d.click("shortsFormats.ending");
    d.settle();
    d.settle();
    let r = d.app().ui.shorts.format.clone().unwrap().result;
    assert!(r["reaction"]["ending"].is_null(), "{r}");
    d.click("shortsFormats.ok");
}
