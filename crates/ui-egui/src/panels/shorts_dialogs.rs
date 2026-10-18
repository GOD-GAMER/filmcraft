//! Shorts ▸ Formats…, Layout… and Word Captions…: windows that re-apply their command live while
//! their options change (folded into one undo step), so the Program monitor previews the result.
//! OK keeps it; Cancel (or Escape) undoes it.
//!
//! Also here: Shorts ▸ Safe Zones (drawn over the Program monitor) and Shorts ▸ Export for
//! YouTube Shorts… / TikTok… (Export mode with the platform's preset).
//!
//! The Layout window shows a frame of the recording with the gameplay (green) and model (pink)
//! areas as boxes to drag: inside a box moves it, near an edge or corner resizes it.

use egui::{Color32, RichText, Stroke};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::FilmcraftApp;

/// Seconds an option must stay unchanged before it is applied.
const DEBOUNCE: f64 = 0.12;

/// An open window: the values shown, the values last applied, and where the undo history was.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub params: Value,
    pub applied: Value,
    pub undo_base: usize,
    pub error: String,
    #[serde(skip)]
    changed_at: Option<f64>,
    /// Layout: the box being dragged in the picker ("game" / "model") and how (edges moved).
    #[serde(skip)]
    drag: Option<(String, [bool; 4])>,
    /// Formats: the sequence the window was opened on (made active again before each new
    /// preview), the safe zone shown before, and the last result.
    pub origin: Option<u64>,
    pub before: String,
    pub result: Value,
}

/// `UiState::shorts`: the open Shorts windows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortsUi {
    pub layout: Option<Draft>,
    pub captions: Option<Draft>,
    pub format: Option<Draft>,
}

fn open(app: &mut FilmcraftApp, current: Value) -> Draft {
    // a fresh undo step for this window (not merged into one from an earlier window)
    app.session.history.merge_key = None;
    Draft { params: current, applied: Value::Null, undo_base: app.session.history.undo.len(), ..Default::default() }
}

/// Menu route: an empty-parameter Formats… / Layout… / Word Captions… opens its window; Export
/// for YouTube Shorts… / TikTok… opens Export mode with the platform's preset.
pub fn route(app: &mut FilmcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if let Some(pf) = match id {
        "shorts.exportYouTube" => Some(filmcraft_engine::shorts_formats::YOUTUBE),
        "shorts.exportTikTok" => Some(filmcraft_engine::shorts_formats::TIKTOK),
        _ => None,
    } {
        return Some(export_for(app, &pf));
    }
    let empty = params.as_object().is_none_or(|m| m.is_empty());
    if !empty || !matches!(id, "shorts.layout" | "shorts.captions" | "shorts.format") {
        return None;
    }
    if let Some(c) = filmcraft_engine::find_command(id)
        && let Err(e) = (c.enabled)(&app.session)
    {
        app.ui.status = e.clone();
        return Some(Err(e));
    }
    if id == "shorts.format" {
        let cur = filmcraft_engine::shorts_formats::current(&app.session);
        let mut d = open(app, cur);
        d.origin = app.session.state.active_sequence.map(|i| i.0);
        d.before = app.ui.program.safe_zone.clone();
        // the preview is made straight away
        d.changed_at = Some(f64::NEG_INFINITY);
        app.ui.shorts.format = Some(d);
        Some(Ok(json!({"dialog": "shortsFormats"})))
    } else if id == "shorts.layout" {
        let cur = filmcraft_engine::shorts_layout::current(&app.session);
        app.ui.shorts.layout = Some(open(app, cur));
        Some(Ok(json!({"dialog": "shortsLayout"})))
    } else {
        if filmcraft_engine::transcript::sequence_words(&app.session).is_empty() {
            let e = tl!("No transcript yet: run Sequence ▸ Transcript ▸ Transcribe… first").to_string();
            app.ui.status = e.clone();
            return Some(Err(e));
        }
        // the font list offers the fonts installed on the computer
        if !filmcraft_text::fonts::system_scanned() {
            filmcraft_text::fonts::scan_system();
        }
        let cur = filmcraft_engine::shorts_captions::current(&app.session);
        app.ui.shorts.captions = Some(open(app, cur));
        Some(Ok(json!({"dialog": "shortsCaptions"})))
    }
}

type Elems = Vec<(String, egui::Rect, String)>;

fn f(p: &Value, k: &str, d: f64) -> f64 {
    p.get(k).and_then(Value::as_f64).unwrap_or(d)
}

fn slider(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str, range: std::ops::RangeInclusive<f64>, pct: bool) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 18.0], egui::Label::new(label));
        let lo = *range.start();
        let mut v = f(p, key, lo);
        let s = if pct {
            egui::Slider::new(&mut v, range)
                .custom_formatter(|x, _| format!("{:.0} %", x * 100.0))
                .custom_parser(|t| t.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|x| x / 100.0))
        } else {
            egui::Slider::new(&mut v, range).max_decimals(1)
        };
        let r = ui.add(s);
        elems.push((format!("{pre}.{key}"), r.rect, label.to_string()));
        if r.changed() {
            p[key] = json!(v);
        }
    });
}

/// A slider of whole numbers.
fn islider(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str, range: std::ops::RangeInclusive<f64>) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 18.0], egui::Label::new(label));
        let mut v = f(p, key, *range.start()).round();
        let r = ui.add(egui::Slider::new(&mut v, range).integer());
        elems.push((format!("{pre}.{key}"), r.rect, label.to_string()));
        if r.changed() {
            p[key] = json!(v.round());
        }
    });
}

/// The installed font families and their styles (rebuilt when fonts are added).
fn families() -> Vec<(String, Vec<String>)> {
    type Cache = std::sync::Mutex<Option<(usize, Vec<(String, Vec<String>)>)>>;
    static CACHE: Cache = std::sync::Mutex::new(None);
    let n = filmcraft_text::fonts::all_faces().len();
    let mut c = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((k, v)) = c.as_ref()
        && *k == n
    {
        return v.clone();
    }
    let v = filmcraft_text::fonts::families();
    *c = Some((n, v.clone()));
    v
}

fn check(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str) {
    let mut v = p.get(key).and_then(Value::as_bool).unwrap_or(false);
    let r = ui.checkbox(&mut v, label);
    elems.push((format!("{pre}.{key}"), r.rect, label.to_string()));
    if r.changed() {
        p[key] = json!(v);
    }
}

fn radios(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str, options: &[(&str, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.add_sized([150.0, 18.0], egui::Label::new(label));
        for (k, l) in options {
            let r = ui.radio(p[key].as_str() == Some(*k), *l);
            elems.push((format!("{pre}.{key}.{k}"), r.rect, l.to_string()));
            if r.clicked() && p[key].as_str() != Some(*k) {
                p[key] = json!(k);
                changed = true;
            }
        }
    });
    changed
}

fn color(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 18.0], egui::Label::new(label));
        let c = p.get(key).and_then(Value::as_str).and_then(filmcraft_color::parse_hex).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let mut rgb = [c[0], c[1], c[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let r = egui::color_picker::color_edit_button_srgb(ui, &mut rgb);
        elems.push((format!("{pre}.{key}"), r.rect, label.to_string()));
        if r.changed() {
            p[key] = json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]));
        }
    });
}

fn text(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value, key: &str, label: &str, hint: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 18.0], egui::Label::new(label));
        let mut v = p.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let r = ui.add(egui::TextEdit::singleline(&mut v).hint_text(hint).desired_width(230.0));
        elems.push((format!("{pre}.{key}"), r.rect, label.to_string()));
        if r.changed() {
            p[key] = json!(v);
        }
    });
}

/// Apply `params` live when they have changed and settled; returns the command's result.
fn live_apply(app: &mut FilmcraftApp, ctx: &egui::Context, command: &str, d: &mut Draft) -> Option<Value> {
    if d.params == d.applied {
        d.changed_at = None;
        return None;
    }
    let now = ctx.input(|i| i.time);
    let since = *d.changed_at.get_or_insert(now);
    if now - since < DEBOUNCE {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(DEBOUNCE));
        return None;
    }
    d.changed_at = None;
    let mut q = d.params.clone();
    if let Some(m) = q.as_object_mut() {
        m.insert("live".into(), json!(true));
    }
    d.applied = d.params.clone();
    match app.session.execute(command, q) {
        Ok(v) => {
            d.error.clear();
            Some(v)
        }
        Err(e) => {
            d.error = e.to_string();
            None
        }
    }
}

/// Undo what the window applied.
fn revert(app: &mut FilmcraftApp, d: &Draft) {
    let mut guard = 0;
    while app.session.history.undo.len() > d.undo_base && guard < 64 {
        if app.session.execute("edit.undo", json!({})).is_err() {
            break;
        }
        guard += 1;
    }
}

/// The windows (call once per frame).
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
    if app.ui.shorts.format.is_some() {
        formats_window(app, ctx);
    }
    if app.ui.shorts.layout.is_some() {
        layout_window(app, ctx);
    }
    if app.ui.shorts.captions.is_some() {
        captions_window(app, ctx);
    }
}

fn footer(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, d: &Draft) -> Option<&'static str> {
    let mut action = None;
    if !d.error.is_empty() {
        ui.label(RichText::new(&d.error).color(ui.visuals().error_fg_color));
    }
    ui.separator();
    ui.horizontal(|ui| {
        let r = ui.button(tl!("Reset"));
        elems.push((format!("{pre}.reset"), r.rect, "Reset".into()));
        if r.clicked() {
            action = Some("reset");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let r = ui.button(tl!("OK"));
            elems.push((format!("{pre}.ok"), r.rect, "OK".into()));
            if r.clicked() {
                action = Some("ok");
            }
            let r = ui.button(tl!("Cancel"));
            elems.push((format!("{pre}.cancel"), r.rect, "Cancel".into()));
            if r.clicked() {
                action = Some("cancel");
            }
        });
    });
    action
}

// -------------------------------------------------------------------------------------------
// Layout
// -------------------------------------------------------------------------------------------

/// The recording the layout arranges, and the media time to show from it.
fn layout_source(app: &FilmcraftApp) -> Option<(filmcraft_project::ItemId, filmcraft_time::Tick)> {
    let q = app.session.active_sequence()?;
    let ph = app.session.playhead();
    let usable = |i: &&filmcraft_project::TrackItem| {
        !i.name.ends_with(" (Model)")
            && !i.name.ends_with(" (Background)")
            && app.session.project.item(i.item).is_some_and(|x| matches!(x.kind, filmcraft_project::ItemKind::Media(_)) && x.has_video())
    };
    let all: Vec<&filmcraft_project::TrackItem> = q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(usable).collect();
    let it = all.iter().find(|i| i.range().contains(ph)).or_else(|| all.first())?;
    let t = if it.range().contains(ph) { it.source_time_at(ph) } else { it.source_time_at(it.start) };
    Some((it.item, t))
}

fn region(p: &Value, key: &str) -> [f32; 4] {
    let a = p.get(key).and_then(Value::as_array);
    let g = |i: usize, d: f32| a.and_then(|a| a.get(i)).and_then(Value::as_f64).map_or(d, |v| v as f32);
    [g(0, 0.0), g(1, 0.0), g(2, 1.0), g(3, 1.0)]
}

/// The frame with the two boxes; drags move or resize them.
fn picker(app: &mut FilmcraftApp, ui: &mut egui::Ui, ctx: &egui::Context, elems: &mut Elems, d: &mut Draft) {
    let Some((item, t)) = layout_source(app) else {
        ui.label(RichText::new(tl!("Put the stream recording on the timeline to see it here.")).weak());
        return;
    };
    let width = 400.0f32;
    let tex = app.thumbnail(ctx, item, t, 480);
    let aspect = tex.map(|(_, s)| s.y / s.x.max(1.0)).unwrap_or(9.0 / 16.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, width * aspect), egui::Sense::drag());
    elems.push(("shortsLayout.picker".into(), rect, "Layout picker".into()));
    let painter = ui.painter_at(rect);
    match tex {
        Some((id, _)) => {
            painter.image(id, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.rect_filled(rect, 4.0, Color32::from_gray(30));
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    let to_screen = |r: [f32; 4]| egui::Rect::from_min_max(rect.lerp_inside(egui::vec2(r[0], r[1])), rect.lerp_inside(egui::vec2(r[2], r[3])));
    let all = [("game", Color32::from_rgb(70, 230, 120), tl!("Gameplay")), ("model", Color32::from_rgb(255, 110, 200), tl!("Model"))];
    let show_model = d.params.get("showModel").and_then(Value::as_bool).unwrap_or(true);
    let boxes: Vec<(&str, Color32, &str)> = all.into_iter().filter(|(k, ..)| *k != "model" || show_model).collect();
    for &(key, col, label) in &boxes {
        let r = to_screen(region(&d.params, key));
        painter.rect_stroke(r, 2.0, Stroke::new(2.0, col), egui::StrokeKind::Inside);
        painter.rect_filled(r, 2.0, col.gamma_multiply(0.12));
        painter.text(r.left_top() + egui::vec2(4.0, 2.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(12.0), col);
    }
    // which box and which edges a drag takes
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let grab = 8.0;
        d.drag = None;
        for (key, ..) in boxes.iter().rev() {
            let r = to_screen(region(&d.params, key));
            if !r.expand(grab).contains(pos) {
                continue;
            }
            let edges = [(pos.x - r.left()).abs() < grab, (pos.y - r.top()).abs() < grab, (pos.x - r.right()).abs() < grab, (pos.y - r.bottom()).abs() < grab];
            let edges = if edges.iter().any(|e| *e) { edges } else { [true; 4] };
            d.drag = Some((key.to_string(), edges));
            break;
        }
    }
    if resp.dragged()
        && let Some((key, edges)) = d.drag.clone()
    {
        let dv = resp.drag_delta();
        let (dx, dy) = (dv.x / rect.width().max(1.0), dv.y / rect.height().max(1.0));
        let mut r = region(&d.params, &key);
        let moving = edges.iter().all(|e| *e);
        if moving {
            let (w, h) = (r[2] - r[0], r[3] - r[1]);
            r[0] = (r[0] + dx).clamp(0.0, 1.0 - w);
            r[1] = (r[1] + dy).clamp(0.0, 1.0 - h);
            r[2] = r[0] + w;
            r[3] = r[1] + h;
        } else {
            let min = 0.05;
            if edges[0] {
                r[0] = (r[0] + dx).clamp(0.0, r[2] - min);
            }
            if edges[1] {
                r[1] = (r[1] + dy).clamp(0.0, r[3] - min);
            }
            if edges[2] {
                r[2] = (r[2] + dx).clamp(r[0] + min, 1.0);
            }
            if edges[3] {
                r[3] = (r[3] + dy).clamp(r[1] + min, 1.0);
            }
        }
        let round = |v: f32| (f64::from(v) * 10_000.0).round() / 10_000.0;
        d.params[key.as_str()] = json!(r.map(round));
    }
    if resp.drag_stopped() {
        d.drag = None;
    }
    ui.label(RichText::new(tl!("Drag a box to move it, or its edges to resize it.")).weak().small());
}

fn layout_window(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.shorts.layout.clone() else { return };
    let pre = "shortsLayout";
    let mut elems: Elems = Vec::new();
    let mut action = None;
    let mut reset_to: Option<Value> = None;
    egui::Window::new(tl!("Layout")).id(egui::Id::new("shorts-layout")).collapsible(false).resizable(false).default_width(430.0).show(ctx, |ui| {
        let p = &mut d.params;
        let presets = [
            ("stack", tl!("Stack")),
            ("split", tl!("Split")),
            ("pip", tl!("Picture in picture")),
            ("spotlight", tl!("Spotlight")),
            ("full", tl!("Full frame")),
        ];
        if radios(ui, &mut elems, pre, p, "preset", tl!("Preset:"), &presets) {
            reset_to = Some(json!({"preset": p["preset"], "side": p["side"]}));
        }
        if radios(ui, &mut elems, pre, p, "side", tl!("Model in the stream:"), &[("right", tl!("Right")), ("left", tl!("Left"))]) {
            reset_to = Some(json!({"preset": p["preset"], "side": p["side"]}));
        }
        ui.separator();
        picker(app, ui, ctx, &mut elems, &mut d);
        let p = &mut d.params;
        ui.separator();
        ui.label(RichText::new(tl!("Gameplay")).strong());
        slider(ui, &mut elems, pre, p, "gameWidth", tl!("Width kept:"), 0.3..=1.0, true);
        slider(ui, &mut elems, pre, p, "gameSize", tl!("Size on screen:"), 0.3..=1.0, true);
        slider(ui, &mut elems, pre, p, "gamePan", tl!("Pan:"), -1.0..=1.0, false);
        slider(ui, &mut elems, pre, p, "gameY", tl!("Down:"), 0.0..=1.0, true);
        slider(ui, &mut elems, pre, p, "gameCorner", tl!("Corner roundness:"), 0.0..=120.0, false);
        ui.label(RichText::new(tl!("Model")).strong());
        check(ui, &mut elems, pre, p, "showModel", tl!("Show the model in its own box"));
        slider(ui, &mut elems, pre, p, "modelWidth", tl!("Box width:"), 0.05..=1.0, true);
        slider(ui, &mut elems, pre, p, "modelHeight", tl!("Box height:"), 0.05..=1.0, true);
        slider(ui, &mut elems, pre, p, "modelX", tl!("Across:"), 0.0..=1.0, true);
        slider(ui, &mut elems, pre, p, "modelY", tl!("Down:"), 0.0..=1.0, true);
        slider(ui, &mut elems, pre, p, "corner", tl!("Corner roundness:"), 0.0..=200.0, false);
        slider(ui, &mut elems, pre, p, "border", tl!("Border:"), 0.0..=40.0, false);
        color(ui, &mut elems, pre, p, "borderColor", tl!("Border colour:"));
        radios(ui, &mut elems, pre, p, "background", tl!("Background:"), &[("blur", tl!("Blurred")), ("black", tl!("Black"))]);
        action = footer(ui, &mut elems, pre, &d);
    });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if crate::widgets::escape_closes(ctx) {
        action = Some("cancel");
    }
    if action == Some("reset") {
        reset_to = Some(json!({"preset": d.params["preset"], "side": d.params["side"]}));
    }
    // a preset (or side) starts from its own values: apply it now and show what it set
    if let Some(start) = reset_to {
        d.params = start;
        d.changed_at = Some(f64::NEG_INFINITY);
    }
    if let Some(v) = live_apply(app, ctx, "shorts.layout", &mut d)
        && let Some(full) = v.get("layout")
    {
        if d.params.get("game").is_none() {
            d.params = full.clone();
        }
        d.applied = d.params.clone();
    }
    match action {
        Some("cancel") => {
            revert(app, &d);
            app.ui.shorts.layout = None;
        }
        Some("ok") => {
            if d.params != d.applied {
                d.changed_at = Some(f64::NEG_INFINITY);
                let _ = live_apply(app, ctx, "shorts.layout", &mut d);
            }
            app.ui.shorts.layout = None;
        }
        _ => app.ui.shorts.layout = Some(d),
    }
}

// -------------------------------------------------------------------------------------------
// Word Captions
// -------------------------------------------------------------------------------------------

fn font_row(ui: &mut egui::Ui, elems: &mut Elems, p: &mut Value) {
    let families = families();
    let cur = p.get("font").and_then(Value::as_str).unwrap_or("Inter").to_string();
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 18.0], egui::Label::new(tl!("Font:")));
        let r = egui::ComboBox::from_id_salt("shortsCaptions.font").width(170.0).selected_text(&cur).show_ui(ui, |ui| {
            for (fam, _) in &families {
                if ui.selectable_label(*fam == cur, fam).clicked() {
                    p["font"] = json!(fam);
                    // keep a bold style when the new font has one
                    let styles = families.iter().find(|(f, _)| f == fam).map(|(_, s)| s.clone()).unwrap_or_default();
                    let want = ["Black", "ExtraBold", "Extra Bold", "Heavy", "Bold"].iter().find(|w| styles.iter().any(|s| s == **w)).map(|w| w.to_string());
                    if let Some(w) = want.or_else(|| styles.first().cloned()) {
                        p["fontStyle"] = json!(w);
                    }
                }
            }
        });
        elems.push(("shortsCaptions.font".into(), r.response.rect, "Font".into()));
        let style = p.get("fontStyle").and_then(Value::as_str).unwrap_or("Bold").to_string();
        let styles = families.iter().find(|(f, _)| *f == cur).map(|(_, s)| s.clone()).unwrap_or_else(|| vec![style.clone()]);
        let r = egui::ComboBox::from_id_salt("shortsCaptions.fontStyle").width(100.0).selected_text(&style).show_ui(ui, |ui| {
            for st in &styles {
                if ui.selectable_label(*st == style, st).clicked() {
                    p["fontStyle"] = json!(st);
                }
            }
        });
        elems.push(("shortsCaptions.fontStyle".into(), r.response.rect, "Font Style".into()));
    });
}

fn captions_window(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.shorts.captions.clone() else { return };
    let pre = "shortsCaptions";
    let mut elems: Elems = Vec::new();
    let mut action = None;
    let mut reset_to: Option<Value> = None;
    egui::Window::new(tl!("Word Captions")).id(egui::Id::new("shorts-captions")).collapsible(false).resizable(false).default_width(430.0).show(ctx, |ui| {
        let p = &mut d.params;
        let styles = [("pop", tl!("Pop")), ("karaoke", tl!("Karaoke")), ("box", tl!("Highlight box")), ("oneWord", tl!("One word")), ("plain", tl!("Plain"))];
        if radios(ui, &mut elems, pre, p, "style", tl!("Style:"), &styles) {
            // a style brings its own colours and sizes, keeping the font and position
            reset_to = Some(json!({"style": p["style"], "font": p["font"], "fontStyle": p["fontStyle"], "x": p["x"], "y": p["y"], "fixes": p["fixes"]}));
        }
        ui.separator();
        egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
            ui.label(RichText::new(tl!("Text")).strong());
            font_row(ui, &mut elems, p);
            slider(ui, &mut elems, pre, p, "size", tl!("Size:"), 20.0..=300.0, false);
            islider(ui, &mut elems, pre, p, "maxWords", tl!("Words at once:"), 1.0..=8.0);
            islider(ui, &mut elems, pre, p, "maxChars", tl!("Letters per line:"), 6.0..=40.0);
            islider(ui, &mut elems, pre, p, "lines", tl!("Lines:"), 1.0..=3.0);
            ui.horizontal(|ui| {
                check(ui, &mut elems, pre, p, "uppercase", tl!("Capitals"));
                check(ui, &mut elems, pre, p, "punctuation", tl!("Punctuation"));
                check(ui, &mut elems, pre, p, "speakerColors", tl!("Colour per speaker"));
            });
            text(ui, &mut elems, pre, p, "fixes", tl!("Fix words:"), "nero=Neuro, kot=cot");
            ui.label(RichText::new(tl!("Colours")).strong());
            color(ui, &mut elems, pre, p, "color", tl!("Text:"));
            color(ui, &mut elems, pre, p, "highlight", tl!("Highlight:"));
            if p["style"] == "box" {
                color(ui, &mut elems, pre, p, "highlightText", tl!("Highlighted text:"));
            }
            slider(ui, &mut elems, pre, p, "grow", tl!("Spoken word bigger by:"), 0.0..=60.0, false);
            slider(ui, &mut elems, pre, p, "outline", tl!("Outline:"), 0.0..=40.0, false);
            color(ui, &mut elems, pre, p, "outlineColor", tl!("Outline colour:"));
            slider(ui, &mut elems, pre, p, "shadow", tl!("Shadow:"), 0.0..=100.0, false);
            check(ui, &mut elems, pre, p, "background", tl!("Box behind the words"));
            if p["background"] == true {
                color(ui, &mut elems, pre, p, "backgroundColor", tl!("Box colour:"));
                slider(ui, &mut elems, pre, p, "backgroundOpacity", tl!("Box opacity:"), 0.0..=100.0, false);
            }
            slider(ui, &mut elems, pre, p, "radius", tl!("Box roundness:"), 0.0..=100.0, false);
            ui.label(RichText::new(tl!("Placement and motion")).strong());
            slider(ui, &mut elems, pre, p, "x", tl!("Across:"), 0.0..=1.0, true);
            slider(ui, &mut elems, pre, p, "y", tl!("Down:"), 0.0..=1.0, true);
            slider(ui, &mut elems, pre, p, "width", tl!("Widest:"), 0.2..=1.0, true);
            let anims = [("pop", tl!("Pop")), ("bounce", tl!("Bounce")), ("fade", tl!("Fade")), ("slide", tl!("Slide up")), ("none", tl!("None"))];
            radios(ui, &mut elems, pre, p, "animation", tl!("Phrases appear:"), &anims);
            slider(ui, &mut elems, pre, p, "hold", tl!("Stay after speech:"), 0.0..=2.0, false);
        });
        action = footer(ui, &mut elems, pre, &d);
    });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if crate::widgets::escape_closes(ctx) {
        action = Some("cancel");
    }
    if action == Some("reset") {
        reset_to = Some(json!({"style": d.params["style"]}));
    }
    if let Some(start) = reset_to {
        d.params = start;
        d.changed_at = Some(f64::NEG_INFINITY);
    }
    if let Some(v) = live_apply(app, ctx, "shorts.captions", &mut d)
        && let Some(full) = v.get("options")
    {
        if d.params.get("size").is_none() {
            d.params = full.clone();
        }
        d.applied = d.params.clone();
    }
    match action {
        Some("cancel") => {
            revert(app, &d);
            app.ui.shorts.captions = None;
        }
        Some("ok") => {
            if d.params != d.applied || d.applied.is_null() {
                d.changed_at = Some(f64::NEG_INFINITY);
                d.applied = Value::Null;
                let _ = live_apply(app, ctx, "shorts.captions", &mut d);
            }
            app.ui.shorts.captions = None;
        }
        _ => app.ui.shorts.captions = Some(d),
    }
}

// -------------------------------------------------------------------------------------------
// Formats
// -------------------------------------------------------------------------------------------

/// Seconds the Formats options must stay unchanged before the preview is made again (a format is
/// a bigger job than a layout).
const FORMAT_DEBOUNCE: f64 = 0.5;

/// A platform's name as the interface shows it.
fn platform_name(pf: &filmcraft_engine::shorts_formats::Platform) -> &'static str {
    match pf.id {
        "tiktok" => tl!("TikTok"),
        _ => tl!("YouTube Shorts"),
    }
}

/// What a short `seconds` long means on `pf`: (note, is a warning).
pub fn length_notes(pf: &filmcraft_engine::shorts_formats::Platform, seconds: f64) -> Vec<(String, bool)> {
    use filmcraft_engine::shorts_formats::clock;
    let name = platform_name(pf);
    let (len, max) = (clock(seconds), clock(pf.max_seconds));
    let mut v = Vec::new();
    if seconds > pf.max_seconds {
        v.push((tlf!("Too long for {name}: it takes up to {max}; this is {len}.", name, max, len), true));
    } else if seconds < pf.min_seconds {
        v.push((tlf!("Too short for {name}: it needs at least {min} seconds.", name, min = pf.min_seconds), true));
    } else {
        v.push((tlf!("{len}: fits {name} (up to {max}).", len, name, max), false));
    }
    if let Some(m) = pf.claimed_music_seconds
        && seconds > m
        && seconds <= pf.max_seconds
    {
        v.push((
            tlf!("Over {limit}, {name} blocks shorts with a copyright claim: keep it under a minute if it uses copyrighted music.", limit = clock(m), name),
            true,
        ));
    }
    v
}

/// Export mode with `pf`'s preset, and a word on the length.
fn export_for(app: &mut FilmcraftApp, pf: &filmcraft_engine::shorts_formats::Platform) -> Result<Value, String> {
    if !crate::panels::export_mode::apply_preset(app, pf.preset) {
        return Err(app.ui.status.clone());
    }
    app.ui.mode = crate::state::Mode::Export;
    let secs = app.session.active_sequence().map(|q| q.duration().seconds()).unwrap_or(0.0);
    let notes = length_notes(pf, secs);
    app.ui.status = notes.iter().find(|n| n.1).or(notes.first()).map(|n| n.0.clone()).unwrap_or_default();
    Ok(json!({"preset": pf.preset, "seconds": secs}))
}

/// What the app covers, in the interface's language.
fn zone_label(label: &str) -> &'static str {
    match label {
        "Search and menu" => tl!("Search and menu"),
        "Title, channel and sound" => tl!("Title, channel and sound"),
        "Like, comment and share" => tl!("Like, comment and share"),
        "Following and For You" => tl!("Following and For You"),
        "Name, caption and sound" => tl!("Name, caption and sound"),
        "Profile, like, comment and share" => tl!("Profile, like, comment and share"),
        _ => "",
    }
}

/// Shorts ▸ Safe Zones: the parts of the picture platform `id`'s app covers, over `pic`. The
/// column of buttons is drawn as plain round buttons (no app's icons), the rest labelled.
pub fn paint_safe_zones(painter: &egui::Painter, pic: egui::Rect, id: &str) {
    let Some(pf) = filmcraft_engine::shorts_formats::platform(id) else { return };
    let painter = painter.with_clip_rect(pic);
    let tint = Color32::from_rgba_unmultiplied(255, 60, 60, 46);
    let edge = Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 110, 110, 200));
    let font = egui::FontId::proportional((pic.width() / 28.0).clamp(8.0, 13.0));
    let at = |x: f64, y: f64| pic.lerp_inside(egui::vec2(x as f32, y as f32));
    for z in pf.zones {
        let r = egui::Rect::from_min_max(at(z.rect[0], z.rect[1]), at(z.rect[2], z.rect[3]));
        painter.rect_filled(r, 0.0, tint);
        painter.rect_stroke(r, 0.0, edge, egui::StrokeKind::Inside);
        if z.rect[2] - z.rect[0] < 0.3 {
            // the button column: four round buttons
            let rad = (r.width() * 0.3).min(r.height() / 10.0);
            for i in 0..4 {
                let c = egui::pos2(r.center().x, r.top() + r.height() * (0.15 + 0.23 * i as f32));
                painter.circle_stroke(c, rad, Stroke::new(1.5, Color32::from_white_alpha(170)));
            }
        } else {
            let below = z.rect[1] > 0.5;
            let pos = if below { r.left_top() + egui::vec2(6.0, 4.0) } else { r.left_bottom() + egui::vec2(6.0, -4.0) };
            let align = if below { egui::Align2::LEFT_TOP } else { egui::Align2::LEFT_BOTTOM };
            painter.text(pos, align, zone_label(z.label), font.clone(), Color32::from_white_alpha(220));
        }
    }
}

/// Make the format's preview again: undo the last one, go back to the sequence the window was
/// opened on, and apply the options (when they have changed and settled, or `now`).
fn format_apply(app: &mut FilmcraftApp, ctx: &egui::Context, d: &mut Draft, now: bool) {
    if d.params == d.applied && !now {
        d.changed_at = None;
        return;
    }
    let t = ctx.input(|i| i.time);
    let since = *d.changed_at.get_or_insert(t);
    if !now && t - since < FORMAT_DEBOUNCE {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(FORMAT_DEBOUNCE));
        return;
    }
    d.changed_at = None;
    revert(app, d);
    restore_origin(app, d);
    d.applied = d.params.clone();
    match app.session.execute("shorts.format", d.params.clone()) {
        Ok(v) => {
            d.error.clear();
            if let Some(pf) = v.get("platform").and_then(Value::as_str) {
                app.ui.program.safe_zone = pf.to_string();
            }
            // the first preview fills in every option the window shows
            if d.params.get("title").is_none()
                && let Some(full) = v.get("options")
            {
                d.params = full.clone();
                d.applied = d.params.clone();
            }
            d.result = v;
        }
        Err(e) => {
            d.error = e.to_string();
            d.result = Value::Null;
        }
    }
}

/// Make the sequence the Formats window was opened on the active one again.
fn restore_origin(app: &mut FilmcraftApp, d: &Draft) {
    let Some(id) = d.origin.map(filmcraft_project::ItemId) else { return };
    if app.session.project.sequence(id).is_none() {
        return;
    }
    let st = &mut app.session.state;
    st.active_sequence = Some(id);
    if !st.open_sequences.contains(&id) {
        st.open_sequences.push(id);
    }
}

/// The length choices of a format (seconds; 0 keeps the length).
fn length_row(ui: &mut egui::Ui, elems: &mut Elems, pre: &str, p: &mut Value) {
    ui.horizontal(|ui| {
        ui.add_sized([150.0, 18.0], egui::Label::new(tl!("Length:")));
        let cur = f(p, "seconds", 0.0);
        for secs in [0.0, 15.0, 30.0, 60.0] {
            let label = if secs == 0.0 { tl!("Keep").to_string() } else { format!("{secs:.0} s") };
            let r = ui.radio((cur - secs).abs() < 0.01, label.as_str());
            elems.push((format!("{pre}.seconds.{secs:.0}"), r.rect, label.clone()));
            if r.clicked() {
                p["seconds"] = json!(secs);
            }
        }
    });
}

fn formats_window(app: &mut FilmcraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.ui.shorts.format.clone() else { return };
    let pre = "shortsFormats";
    let mut elems: Elems = Vec::new();
    let mut action = None;
    let has_words = !filmcraft_engine::transcript::sequence_words(&app.session).is_empty();
    egui::Window::new(tl!("Formats")).id(egui::Id::new("shorts-formats")).collapsible(false).resizable(false).default_width(470.0).show(ctx, |ui| {
        let p = &mut d.params;
        radios(ui, &mut elems, pre, p, "platform", tl!("Platform:"), &[("youtube", tl!("YouTube Shorts")), ("tiktok", tl!("TikTok"))]);
        ui.separator();
        ui.label(RichText::new(tl!("Format:")).strong());
        let formats = [
            ("gameplay", tl!("Gameplay + Model"), tl!("Gameplay on top, your model in its own box below, a hook title and captions.")),
            ("fullFrame", tl!("Full Frame + Blur"), tl!("The whole clip in the middle over a blurred copy, a title above and captions below.")),
            ("meme", tl!("Meme Caption Bar"), tl!("Big black text on a white bar above the clip, like a meme.")),
            ("spotlight", tl!("Model Spotlight"), tl!("Your model big, the gameplay in a smaller card above, one big word at a time.")),
            ("countdown", tl!("Top Moments Countdown"), tl!("The biggest moments of a longer clip as #3, #2, #1, the best one last.")),
        ];
        for (k, name, desc) in formats {
            let r = ui.radio(p["format"].as_str() == Some(k), name);
            elems.push((format!("{pre}.format.{k}"), r.rect, name.to_string()));
            if r.clicked() && p["format"].as_str() != Some(k) {
                p["format"] = json!(k);
                // the new format's own defaults: its title (unless one was typed), captions, hook
                if let Some(m) = p.as_object_mut() {
                    let typed =
                        m.get("title").and_then(Value::as_str).is_some_and(|t| t != "WAIT FOR IT…" && !(t.starts_with("TOP ") && t.ends_with(" MOMENTS")));
                    if !typed {
                        m.remove("title");
                    }
                    m.remove("captions");
                    m.remove("hook");
                }
            }
            ui.indent(format!("{pre}.desc.{k}"), |ui| {
                ui.label(RichText::new(desc).weak().small());
            });
        }
        ui.separator();
        let fmt = p["format"].as_str().unwrap_or("gameplay").to_string();
        let title_label = match fmt.as_str() {
            "fullFrame" => tl!("Title:"),
            "meme" => tl!("Bar text:"),
            "countdown" => tl!("Header:"),
            _ => tl!("Hook title:"),
        };
        text(ui, &mut elems, pre, p, "title", title_label, "WAIT FOR IT…");
        if matches!(fmt.as_str(), "gameplay" | "spotlight") {
            check(ui, &mut elems, pre, p, "hook", tl!("Show the hook title"));
        }
        if !matches!(fmt.as_str(), "fullFrame" | "meme") {
            radios(ui, &mut elems, pre, p, "side", tl!("Model in the stream:"), &[("right", tl!("Right")), ("left", tl!("Left"))]);
        }
        ui.add_enabled_ui(has_words, |ui| check(ui, &mut elems, pre, p, "captions", tl!("Word captions")));
        if !has_words {
            ui.label(RichText::new(tl!("Transcribe the clip first to add captions (Sequence ▸ Transcript ▸ Transcribe…).")).weak().small());
        } else if p["captions"] == true {
            let styles = [
                ("auto", tl!("Automatic")),
                ("pop", tl!("Pop")),
                ("karaoke", tl!("Karaoke")),
                ("box", tl!("Highlight box")),
                ("oneWord", tl!("One word")),
                ("plain", tl!("Plain")),
            ];
            if p.get("captionStyle").is_none() {
                p["captionStyle"] = json!("auto");
            }
            radios(ui, &mut elems, pre, p, "captionStyle", tl!("Caption style:"), &styles);
        }
        check(ui, &mut elems, pre, p, "progressBar", tl!("Progress bar"));
        if fmt == "countdown" {
            islider(ui, &mut elems, pre, p, "moments", tl!("Moments:"), 2.0..=6.0);
            slider(ui, &mut elems, pre, p, "momentSeconds", tl!("Seconds each:"), 3.0..=15.0, false);
        } else {
            length_row(ui, &mut elems, pre, p);
        }
        check(ui, &mut elems, pre, p, "loop", tl!("Loop ending (blends the end into the start)"));
        // what came out
        let pf = d.result["platform"].as_str().and_then(filmcraft_engine::shorts_formats::platform);
        if let (Some(secs), Some(pf)) = (d.result["seconds"].as_f64(), pf) {
            ui.separator();
            for (i, (note, warn)) in length_notes(&pf, secs).into_iter().enumerate() {
                let line = if warn { RichText::new(&note).color(ui.visuals().warn_fg_color) } else { RichText::new(&note) };
                let r = ui.label(line);
                elems.push((format!("{pre}.note.{i}"), r.rect, note));
            }
            let r = ui.button(tlf!("Export for {name}…", name = platform_name(&pf)));
            elems.push((format!("{pre}.export"), r.rect, "Export".into()));
            if r.clicked() {
                action = Some("export");
            }
        }
        if let Some(a) = footer(ui, &mut elems, pre, &d) {
            action = Some(a);
        }
    });
    for (id, r, l) in elems {
        app.auto.add(&id, r, &l);
    }
    if crate::widgets::escape_closes(ctx) {
        action = Some("cancel");
    }
    if action == Some("reset") {
        d.params = json!({"platform": d.params["platform"], "format": d.params["format"]});
        d.changed_at = Some(f64::NEG_INFINITY);
    }
    if action != Some("cancel") {
        let settle = matches!(action, Some("ok" | "export")) && d.params != d.applied;
        format_apply(app, ctx, &mut d, settle);
    }
    match action {
        Some("cancel") => {
            revert(app, &d);
            restore_origin(app, &d);
            app.ui.program.safe_zone = d.before.clone();
            app.ui.shorts.format = None;
        }
        Some("ok") => app.ui.shorts.format = None,
        Some("export") => {
            app.ui.shorts.format = None;
            if let Some(pf) = d.result["platform"].as_str().and_then(filmcraft_engine::shorts_formats::platform) {
                let _ = export_for(app, &pf);
            }
        }
        _ => app.ui.shorts.format = Some(d),
    }
}
