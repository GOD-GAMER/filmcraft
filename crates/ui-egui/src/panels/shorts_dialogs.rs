//! Sequence ▸ Shorts ▸ Layout… and Word Captions…: windows that re-apply their command live
//! while their options change (folded into one undo step), so the Program monitor previews the
//! result. OK keeps it; Cancel (or Escape) undoes it.
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
}

/// `UiState::shorts`: the open Shorts windows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortsUi {
    pub layout: Option<Draft>,
    pub captions: Option<Draft>,
}

fn open(app: &mut FilmcraftApp, current: Value) -> Draft {
    // a fresh undo step for this window (not merged into one from an earlier window)
    app.session.history.merge_key = None;
    Draft { params: current, applied: Value::Null, undo_base: app.session.history.undo.len(), ..Default::default() }
}

/// Menu route: an empty-parameter Layout… / Word Captions… opens its window.
pub fn route(app: &mut FilmcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let empty = params.as_object().is_none_or(|m| m.is_empty());
    if !empty || !matches!(id, "shorts.layout" | "shorts.captions") {
        return None;
    }
    if let Some(c) = filmcraft_engine::find_command(id)
        && let Err(e) = (c.enabled)(&app.session)
    {
        app.ui.status = e.clone();
        return Some(Err(e));
    }
    if id == "shorts.layout" {
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

/// Both windows (call once per frame).
pub fn show(app: &mut FilmcraftApp, ctx: &egui::Context) {
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
    let boxes = [("game", Color32::from_rgb(70, 230, 120), tl!("Gameplay")), ("model", Color32::from_rgb(255, 110, 200), tl!("Model"))];
    for (key, col, label) in boxes {
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
        let presets = [("stack", tl!("Stack")), ("split", tl!("Split")), ("pip", tl!("Picture in picture"))];
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
        slider(ui, &mut elems, pre, p, "gamePan", tl!("Pan:"), -1.0..=1.0, false);
        slider(ui, &mut elems, pre, p, "gameY", tl!("Down:"), 0.0..=1.0, true);
        slider(ui, &mut elems, pre, p, "gameCorner", tl!("Corner roundness:"), 0.0..=120.0, false);
        ui.label(RichText::new(tl!("Model")).strong());
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
