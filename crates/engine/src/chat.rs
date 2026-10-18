//! Chat replays from stream archives, for Shorts ▸ Chat Overlay… and Find Chat
//! Highlights….
//!
//! Reads the common chat-replay downloads:
//! - TwitchDownloader JSON (`{"comments": [{"content_offset_seconds", "commenter": {"display_name"},
//!   "message": {"body", "user_color"}}]}`);
//! - chat-downloader JSON (`[{"time_in_seconds", "author": {"name"}, "message", "colour"?}]`);
//! - yt-dlp YouTube live chat (`.live_chat.json`, one JSON object per line with
//!   `replayChatItemAction.videoOffsetTimeMsec` and `liveChatTextMessageRenderer`s).
//!
//! Chat times are seconds from the start of the recording. They reach the timeline through the
//! clips of the recording's media item, the way transcript words do.

use filmcraft_geom::Vec2;
use filmcraft_project::{CharStyle, ClipId, ItemId, Label, LayerExtra, ParamValue, StyleRun};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{bad, str_p};
use crate::shorts::{GraphicClip, is_footage, num_p, place_graphic_track, set_static};
use crate::{EngineError, Result, Session};

/// The most messages read from one file.
const MAX_MESSAGES: usize = 500_000;
/// Longest message kept (characters); longer ones are cut.
const MAX_TEXT: usize = 300;
/// Biggest chat file read (bytes).
const MAX_FILE: u64 = 512 * 1024 * 1024;

/// One chat message.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    /// Seconds from the start of the recording.
    pub t: f64,
    pub name: String,
    pub text: String,
    /// The chatter's colour, when the file has one.
    pub color: Option<[f32; 4]>,
}

fn hex_color(s: &str) -> Option<[f32; 4]> {
    let h = s.trim().trim_start_matches('#');
    let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok());
    (h.len() == 6).then(|| Some([f32::from(b(0)?) / 255.0, f32::from(b(2)?) / 255.0, f32::from(b(4)?) / 255.0, 1.0])).flatten()
}

fn clean(s: &str) -> String {
    let one_line: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = one_line.trim();
    if t.chars().count() > MAX_TEXT { t.chars().take(MAX_TEXT).collect() } else { t.to_string() }
}

fn num(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok())).filter(|x| x.is_finite())
}

/// TwitchDownloader / chat-downloader message objects.
fn from_object(o: &Value) -> Option<ChatMessage> {
    // TwitchDownloader
    if let Some(t) = num(o.get("content_offset_seconds")) {
        let c = o.get("commenter");
        let name = c.and_then(|c| c.get("display_name").or_else(|| c.get("name"))).and_then(Value::as_str).unwrap_or("");
        let m = o.get("message");
        let text = m.and_then(|m| m.get("body")).and_then(Value::as_str).unwrap_or("");
        let color = m.and_then(|m| m.get("user_color")).and_then(Value::as_str).and_then(hex_color);
        return Some(ChatMessage { t, name: clean(name), text: clean(text), color });
    }
    // chat-downloader
    if let Some(t) = num(o.get("time_in_seconds")) {
        let a = o.get("author");
        let name = a.and_then(|a| a.get("display_name").or_else(|| a.get("name"))).and_then(Value::as_str).unwrap_or("");
        let text = o.get("message").and_then(Value::as_str).unwrap_or("");
        let color = o.get("colour").or_else(|| a.and_then(|a| a.get("colour"))).and_then(Value::as_str).and_then(hex_color);
        return Some(ChatMessage { t, name: clean(name), text: clean(text), color });
    }
    None
}

/// The text of a YouTube message's runs (emoji as their shortcut).
fn runs_text(runs: &Value) -> String {
    runs.as_array()
        .map(|a| {
            a.iter()
                .map(|r| match r.get("text").and_then(Value::as_str) {
                    Some(t) => t.to_string(),
                    None => r
                        .get("emoji")
                        .and_then(|e| e.get("shortcuts").and_then(|s| s.get(0)).or_else(|| e.get("emojiId")))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One line of a yt-dlp live chat file.
fn from_youtube(o: &Value) -> Vec<ChatMessage> {
    let Some(replay) = o.get("replayChatItemAction") else { return Vec::new() };
    let Some(ms) = num(replay.get("videoOffsetTimeMsec")) else { return Vec::new() };
    let mut out = Vec::new();
    for a in replay.get("actions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        let Some(item) = a.get("addChatItemAction").and_then(|x| x.get("item")) else { continue };
        let r = item.get("liveChatTextMessageRenderer").or_else(|| item.get("liveChatPaidMessageRenderer"));
        let Some(r) = r else { continue };
        let name = r.get("authorName").and_then(|n| n.get("simpleText")).and_then(Value::as_str).unwrap_or("");
        let text = r.get("message").and_then(|m| m.get("runs")).map(runs_text).unwrap_or_default();
        out.push(ChatMessage { t: ms / 1000.0, name: clean(name), text: clean(&text), color: None });
    }
    out
}

/// Read a chat replay (any of the supported formats), sorted by time. Messages without text are
/// dropped.
pub fn parse_chat(text: &str) -> std::result::Result<Vec<ChatMessage>, String> {
    let body = text.trim_start_matches('\u{feff}').trim();
    let mut out: Vec<ChatMessage> = Vec::new();
    match serde_json::from_str::<Value>(body) {
        Ok(v) => {
            let list = v.get("comments").and_then(Value::as_array).or_else(|| v.as_array());
            match list {
                Some(items) => out.extend(items.iter().take(MAX_MESSAGES).filter_map(from_object)),
                None => out.extend(from_youtube(&v)),
            }
        }
        // one JSON object per line (yt-dlp live chat)
        Err(_) => {
            for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if out.len() >= MAX_MESSAGES {
                    break;
                }
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                out.extend(from_youtube(&v));
                out.extend(from_object(&v));
            }
        }
    }
    out.retain(|m| !m.text.is_empty() && m.t.is_finite());
    if out.is_empty() {
        return Err("no chat messages found (TwitchDownloader JSON, chat-downloader JSON and yt-dlp live_chat.json are supported)".into());
    }
    out.truncate(MAX_MESSAGES);
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    Ok(out)
}

/// Busy stretches of chat: windows of `window` seconds holding at least three times the usual
/// number of messages (and at least 5), the busiest first, at least `gap` seconds apart, at most
/// `max`, returned in time order as (start second, messages in the window).
pub fn chat_spikes(msgs: &[ChatMessage], window: f64, gap: f64, max: usize) -> Vec<(f64, usize)> {
    let (Some(first), Some(last)) = (msgs.first(), msgs.last()) else { return Vec::new() };
    if !(window.is_finite() && window > 0.0) {
        return Vec::new();
    }
    let span = (last.t - first.t).max(0.0);
    let steps = ((span / 1.0).ceil() as usize).saturating_add(1).min(4_000_000);
    // messages per window, starting every second
    let mut counts = Vec::with_capacity(steps);
    let (mut lo, mut hi) = (0usize, 0usize);
    for k in 0..steps {
        let a = first.t + k as f64;
        while lo < msgs.len() && msgs.get(lo).is_some_and(|m| m.t < a) {
            lo += 1;
        }
        hi = hi.max(lo);
        while hi < msgs.len() && msgs.get(hi).is_some_and(|m| m.t < a + window) {
            hi += 1;
        }
        counts.push(hi.saturating_sub(lo));
    }
    let mut sorted = counts.clone();
    sorted.sort_unstable();
    let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0);
    let need = (median.saturating_mul(3)).max(5);
    let mut cands: Vec<(usize, usize)> = counts.iter().enumerate().filter(|(_, c)| **c >= need).map(|(k, c)| (k, *c)).collect();
    cands.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut out: Vec<(f64, usize)> = Vec::new();
    for (k, c) in cands {
        if out.len() >= max.max(1) {
            break;
        }
        let at = first.t + k as f64;
        if out.iter().all(|(t, _)| (t - at).abs() >= gap) {
            out.push((at, c));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// The most common words of the messages (for marker notes), at most `n`.
fn top_words(msgs: &[&ChatMessage], n: usize) -> Vec<String> {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for m in msgs {
        let mut seen = std::collections::BTreeSet::new();
        for w in m.text.split_whitespace() {
            let k = w.to_string();
            if k.chars().count() >= 2 && seen.insert(k.clone()) {
                *counts.entry(k).or_default() += 1;
            }
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().filter(|(_, c)| *c >= 2).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().take(n).map(|(w, _)| w).collect()
}

fn read_chat(p: &Value, cmd: &str) -> Result<Vec<ChatMessage>> {
    let path = str_p(p, "path").ok_or_else(|| bad(cmd, "need `path` (a chat replay .json file)"))?;
    let meta = std::fs::metadata(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    if meta.len() > MAX_FILE {
        return Err(bad(cmd, "the chat file is larger than 512 MB"));
    }
    let bytes = std::fs::read(path).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    parse_chat(&String::from_utf8_lossy(&bytes)).map_err(|e| bad(cmd, e))
}

/// The recording the chat belongs to: `clip`'s media, else a selected footage clip's, else the
/// footage under the playhead (lowest track), else the first footage on the timeline.
fn recording(s: &Session, p: &Value, cmd: &str) -> Result<ItemId> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let footage = |c: &ClipId| q.video_tracks.iter().find_map(|t| t.item(*c)).filter(|i| is_footage(&s.project, i)).map(|i| i.item);
    if let Some(c) = crate::commands::clip_p(p, "clip") {
        return footage(&c).ok_or_else(|| bad(cmd, "`clip` is not a video clip"));
    }
    if let Some(i) = s.state.selection.iter().find_map(footage) {
        return Ok(i);
    }
    let ph = s.playhead();
    let all = || q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| is_footage(&s.project, i));
    all()
        .find(|i| i.range().contains(ph))
        .or_else(|| all().min_by_key(|i| i.start))
        .map(|i| i.item)
        .ok_or_else(|| bad(cmd, "put the stream recording on the timeline first"))
}

/// Timeline time of recording time `secs` (seconds, after the `offset`), through the clips of
/// `item` on the video tracks (the first that shows it).
fn to_timeline(s: &Session, item: ItemId, secs: f64) -> Option<Tick> {
    let q = s.active_sequence()?;
    let m = Tick::from_seconds_f64(secs);
    q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| i.item == item && !i.reverse && i.frame_hold.is_none()).find_map(|i| {
        let sp = i.speed.abs();
        if sp <= 0.0 || m < i.source_in || m >= i.source_out() {
            return None;
        }
        Some(Tick(i.start.0.saturating_add(((m.0 - i.source_in.0) as f64 / sp).round() as i64)))
    })
}

/// A readable colour for a chatter without one: from a small palette by name.
fn name_color(name: &str) -> [f32; 4] {
    const P: [[f32; 4]; 8] = [
        [1.0, 0.45, 0.45, 1.0],
        [0.45, 0.8, 1.0, 1.0],
        [0.55, 1.0, 0.55, 1.0],
        [1.0, 0.8, 0.35, 1.0],
        [0.85, 0.55, 1.0, 1.0],
        [1.0, 0.6, 0.85, 1.0],
        [0.4, 1.0, 0.9, 1.0],
        [1.0, 0.65, 0.4, 1.0],
    ];
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    P.get(h as usize % P.len()).copied().unwrap_or([1.0, 1.0, 1.0, 1.0])
}

/// Chat Overlay…: the chat of the recording on screen, the newest `lines` messages in a box, on a
/// new track.
pub(crate) fn overlay(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.chatOverlay";
    let msgs = read_chat(p, CMD)?;
    let offset = num_p(p, "offset", 0.0, -86_400.0, 86_400.0, CMD)?;
    let lines = num_p(p, "lines", 6.0, 1.0, 30.0, CMD)?.round() as usize;
    let max_chars = num_p(p, "maxChars", 34.0, 8.0, 200.0, CMD)?.round() as usize;
    let side = str_p(p, "position").unwrap_or("left");
    if !matches!(side, "left" | "right") {
        return Err(bad(CMD, format!("`position` must be left or right, not `{side}`")));
    }
    let item = recording(s, p, CMD)?;
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (w, h, rate) = (q.settings.width, q.settings.height, q.settings.frame_rate);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let vertical = fh > fw;
    // each message at its time on the timeline (the parts of the recording that are used)
    let mut placed: Vec<(Tick, &ChatMessage)> = msgs.iter().filter_map(|m| to_timeline(s, item, m.t - offset).map(|t| (rate.snap(t), m))).collect();
    placed.sort_by_key(|(t, _)| *t);
    if placed.is_empty() {
        return Err(bad(CMD, "none of the chat falls in the part of the recording on the timeline (check the offset)"));
    }
    let size = if vertical { fw * 0.032 } else { fh * 0.03 };
    let line_h = size * 1.3;
    let bottom = if vertical { fh * 0.6 } else { fh * 0.9 };
    let x = if side == "left" { fw * 0.05 } else { fw * 0.95 };
    let idle = Tick::from_seconds_f64(10.0);
    let seq_end = q.duration();
    // one clip per state of the box: from one message to the next (or 10 s of quiet)
    let mut clips: Vec<GraphicClip> = Vec::new();
    let mut shown: Vec<&ChatMessage> = Vec::new();
    let mut k = 0usize;
    while k < placed.len() && clips.len() < 5000 {
        let Some(&(t, _)) = placed.get(k) else { break };
        while let Some(&(_, m)) = placed.get(k).filter(|(tt, _)| *tt == t) {
            shown.push(m);
            k += 1;
        }
        if shown.len() > lines {
            shown.drain(..shown.len() - lines);
        }
        let next = placed.get(k).map(|(tn, _)| *tn).unwrap_or(seq_end);
        let end = next.min(Tick(t.0.saturating_add(idle.0))).min(seq_end.max(t));
        if end <= t {
            continue;
        }
        let mut text = String::new();
        let mut runs = Vec::new();
        for (i, m) in shown.iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            let at = text.chars().count();
            let name: String = m.name.chars().take(24).collect();
            text.push_str(&name);
            runs.push(StyleRun {
                start: at,
                end: at + name.chars().count(),
                style: CharStyle { fill: Some(m.color.unwrap_or_else(|| name_color(&m.name))), font_style: Some("Bold".into()), ..CharStyle::default() },
            });
            text.push_str(": ");
            let room = max_chars.saturating_sub(name.chars().count() + 2).max(4);
            if m.text.chars().count() > room {
                text.extend(m.text.chars().take(room.saturating_sub(1)));
                text.push('…');
            } else {
                text.push_str(&m.text);
            }
        }
        let n = shown.len().max(1) as f64;
        let mut layer = filmcraft_project::graphic::new_text_layer(&text, Vec2::new(x, bottom - (n - 1.0) * line_h), size);
        for (key, v) in [
            ("name", ParamValue::Text("Chat".into())),
            ("font", ParamValue::Text("Inter".into())),
            ("font_style", ParamValue::Text("Medium".into())),
            ("align", ParamValue::Choice(if side == "left" { 0 } else { 2 })),
            ("fill_color", ParamValue::Color([1.0, 1.0, 1.0, 1.0])),
            ("background", ParamValue::Bool(true)),
            ("background_color", ParamValue::Color([0.0, 0.0, 0.0, 1.0])),
            ("background_opacity", ParamValue::Float(55.0)),
            ("background_size", ParamValue::Float(size * 0.45)),
            ("background_radius", ParamValue::Float(size * 0.35)),
        ] {
            set_static(&mut layer, key, v);
        }
        layer.layer = Some(Box::new(LayerExtra { runs, ..LayerExtra::default() }));
        clips.push((t, end - t, "Chat".into(), vec![layer]));
    }
    let n = clips.len();
    let track = place_graphic_track(s, "Chat Overlay", "Chat", clips)?;
    Ok(json!({"messages": placed.len(), "clips": n, "track": track}))
}

/// Find Chat Highlights…: markers where chat suddenly gets busy (the moments viewers reacted to).
pub(crate) fn highlights(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.chatHighlights";
    let msgs = read_chat(p, CMD)?;
    let offset = num_p(p, "offset", 0.0, -86_400.0, 86_400.0, CMD)?;
    let window = num_p(p, "window", 5.0, 1.0, 120.0, CMD)?;
    let max = num_p(p, "maxMoments", 10.0, 1.0, 500.0, CMD)?.round() as usize;
    let item = recording(s, p, CMD)?;
    let spikes = chat_spikes(&msgs, window, 30.0, max.saturating_mul(4));
    let mut notes: Vec<(Tick, String)> = Vec::new();
    for (at, count) in &spikes {
        if notes.len() >= max {
            break;
        }
        // react a little before chat does: viewers type after the moment
        let Some(t) = to_timeline(s, item, at - offset - 3.0).or_else(|| to_timeline(s, item, at - offset)) else { continue };
        let inside: Vec<&ChatMessage> = msgs.iter().filter(|m| m.t >= *at && m.t < at + window).collect();
        let words = top_words(&inside, 3);
        let note =
            if words.is_empty() { format!("{count} messages in {window:.0} s") } else { format!("{count} messages in {window:.0} s: {}", words.join(" ")) };
        notes.push((t, note));
    }
    if notes.is_empty() {
        return Err(bad(CMD, "chat never gets much busier than usual in the part of the recording on the timeline"));
    }
    let n = notes.len();
    s.edit_sequence("Find Chat Highlights", |q, ctx, _| {
        for (t, comment) in &notes {
            q.markers.push(filmcraft_project::Marker {
                id: filmcraft_project::MarkerId(ctx.alloc()),
                start: *t,
                duration: Tick::ZERO,
                name: "Chat Spike".into(),
                comment: comment.clone(),
                kind: filmcraft_project::MarkerKind::Comment,
                color: Label::Rose,
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    Ok(json!({"markers": n, "spikes": spikes.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_twitch_downloader_json() {
        let j = r##"{"video":{"title":"x"},"comments":[
            {"content_offset_seconds": 12.5, "commenter": {"display_name": "Kappa_Fan"}, "message": {"body": "LUL", "user_color": "#FF4500"}},
            {"content_offset_seconds": 3, "commenter": {"name": "anon"}, "message": {"body": "first"}},
            {"content_offset_seconds": 4, "commenter": {"name": "anon"}, "message": {"body": "   "}}
        ]}"##;
        let m = parse_chat(j).unwrap();
        assert_eq!(m.len(), 2, "empty messages are dropped");
        assert_eq!((m[0].t, m[0].text.as_str()), (3.0, "first"));
        assert_eq!(m[1].name, "Kappa_Fan");
        assert_eq!(m[1].color, Some([1.0, 69.0 / 255.0, 0.0, 1.0]));
    }

    #[test]
    fn reads_chat_downloader_and_youtube_live_chat() {
        let j = r##"[{"time_in_seconds": 1.5, "author": {"name": "a"}, "message": "hi", "colour": "#00ff00"}, {"time_in_seconds": "2", "author": {}, "message": "yo"}]"##;
        let m = parse_chat(j).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].t, 2.0);
        let yt = concat!(
            r#"{"replayChatItemAction":{"actions":[{"addChatItemAction":{"item":{"liveChatTextMessageRenderer":{"message":{"runs":[{"text":"so "},{"emoji":{"shortcuts":[":good:"]}}]},"authorName":{"simpleText":"Viewer"}}}}}],"videoOffsetTimeMsec":"65000"}}"#,
            "\n",
            "not json\n",
            r#"{"replayChatItemAction":{"actions":[],"videoOffsetTimeMsec":"70000"}}"#
        );
        let m = parse_chat(yt).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].t, m[0].name.as_str(), m[0].text.as_str()), (65.0, "Viewer", "so :good:"));
    }

    #[test]
    fn rejects_files_without_chat_and_hostile_values() {
        assert!(parse_chat("").is_err());
        assert!(parse_chat("{}").is_err());
        assert!(parse_chat("[1, 2, null]").is_err());
        let j = r#"[{"time_in_seconds": "soon", "author": {"name": "a"}, "message": "x"}, {"time_in_seconds": 1, "author": {"name": "\u0007b"}, "message": "ok\nnow"}]"#;
        let m = parse_chat(j).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].text, "ok now");
        let long = format!(r#"[{{"time_in_seconds": 0, "author": {{"name": "a"}}, "message": "{}"}}]"#, "x".repeat(5000));
        assert_eq!(parse_chat(&long).unwrap()[0].text.chars().count(), MAX_TEXT);
    }

    #[test]
    fn spikes_are_bursts_of_chat() {
        // one message a second for 200 s, and 40 extra messages around 100 s and 20 around 150 s
        let mut msgs: Vec<ChatMessage> = (0..200).map(|i| ChatMessage { t: i as f64, name: "a".into(), text: "hi".into(), color: None }).collect();
        msgs.extend((0..40).map(|i| ChatMessage { t: 100.0 + i as f64 * 0.1, name: "b".into(), text: "W".into(), color: None }));
        msgs.extend((0..20).map(|i| ChatMessage { t: 150.0 + i as f64 * 0.2, name: "c".into(), text: "LUL".into(), color: None }));
        msgs.sort_by(|a, b| a.t.total_cmp(&b.t));
        let s = chat_spikes(&msgs, 5.0, 30.0, 5);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!((s[0].0 - 100.0).abs() <= 4.0 && (s[1].0 - 150.0).abs() <= 4.0, "{s:?}");
        assert!(chat_spikes(&[], 5.0, 30.0, 5).is_empty());
        assert!(chat_spikes(&msgs, f64::NAN, 30.0, 5).is_empty());
    }
}
