//! Sequence ▸ Shorts ▸ Track Model: keeps a cropped pane (a stacked layout's model pane) on the
//! streamer as they move. The part of the picture the pane shows is tracked frame by frame in the
//! background (Progress panel); when the job finishes, the pane's Motion ▸ Anchor Point and Crop
//! get keyframes that follow it, smoothed, and never leave the picture. One undo step.

use std::sync::{Arc, Mutex};

use filmcraft_geom::Vec2;
use filmcraft_project::{ClipId, ItemId, ParamValue, TrackItem, TrackMethod};
use filmcraft_time::Tick;
use serde_json::{Value, json};

use crate::commands::{bad, bool_p, clips_p};
use crate::{EngineError, Result, Session};

/// Analysis width for tracking (the masks tracker uses the same).
const TRACK_WIDTH: f64 = 960.0;
/// Track every n-th source frame.
const STRIDE: i64 = 2;
/// Give up after this many frames in a row without a track.
const MAX_MISSES: usize = 15;
/// The longest stretch one job tracks (source frames).
const MAX_STEPS: usize = 60_000;

/// A tracking job whose result is applied when it finishes.
pub struct PendingModel {
    job: u64,
    seq: ItemId,
    clip: ClipId,
    /// (media time, shift x, shift y) in picture pixels, from the first frame.
    keys: Arc<Mutex<Vec<(Tick, f64, f64)>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn crop_of(it: &TrackItem) -> Option<[f64; 4]> {
    let c = crate::shorts_layout::pane_crop_index(it).and_then(|i| it.effects.get(i))?;
    let v = ["left", "top", "right", "bottom"].map(|k| c.params.get(k).and_then(|p| p.value.as_f64()).unwrap_or(0.0));
    v.iter().any(|x| *x > 0.0).then_some(v)
}

/// The clips to track: selected cropped video clips, or else the model panes under the playhead.
fn targets(s: &Session, p: &Value) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    let cropped = |c: &ClipId| q.video_tracks.iter().find_map(|t| t.item(*c)).is_some_and(|i| crop_of(i).is_some());
    let picked: Vec<ClipId> = clips_p(s, p).into_iter().filter(cropped).collect();
    if !picked.is_empty() {
        return picked;
    }
    let ph = s.playhead();
    q.video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter(|i| i.range().contains(ph) && i.name.ends_with("(Model)") && crop_of(i).is_some())
        .map(|i| i.id)
        .collect()
}

pub(crate) fn track_model(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.trackModel";
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let clips = targets(s, p);
    if clips.is_empty() {
        return Err(bad(CMD, "select the model pane (Stacked Layout makes one), or any cropped clip"));
    }
    let wait = bool_p(p, "wait").unwrap_or(false);
    let mut started = Vec::new();
    for clip in clips {
        if s.model_jobs.iter().any(|j| j.clip == clip) {
            return Err(bad(CMD, "that pane is already being tracked"));
        }
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        let it = q.find_item(clip).map(|(_, i)| i.clone()).ok_or_else(|| bad(CMD, "no such clip"))?;
        if it.reverse || it.frame_hold.is_some() {
            return Err(bad(CMD, "a reversed or held clip can't be tracked"));
        }
        let crop = crop_of(&it).ok_or_else(|| bad(CMD, "the clip has no crop to follow"))?;
        let (sw, sh) =
            filmcraft_render::source_size(&s.project, it.item).map(|(w, h)| (f64::from(w), f64::from(h))).ok_or_else(|| bad(CMD, "the clip has no picture"))?;
        let src = s.source(it.item).ok_or_else(|| EngineError::Other("the clip's media is offline".into()))?;
        let rate = s.project.item(it.item).map(|i| i.frame_rate()).unwrap_or(q.settings.frame_rate);
        let step = Tick(rate.frame_duration().0.max(1).saturating_mul(STRIDE));
        let m0 = it.source_time_at(it.start);
        let m1 = it.source_time_at(Tick(it.end().0.saturating_sub(1)));
        let steps = (((m1.0 - m0.0).max(0) / step.0.max(1)) as usize).min(MAX_STEPS);
        if steps == 0 {
            return Err(bad(CMD, "the clip is too short to track"));
        }
        // the pane's window in picture pixels
        let region = [
            Vec2::new(crop[0] / 100.0 * sw, crop[1] / 100.0 * sh),
            Vec2::new((100.0 - crop[2]) / 100.0 * sw, crop[1] / 100.0 * sh),
            Vec2::new((100.0 - crop[2]) / 100.0 * sw, (100.0 - crop[3]) / 100.0 * sh),
            Vec2::new(crop[0] / 100.0 * sw, (100.0 - crop[3]) / 100.0 * sh),
        ];
        let scale = (TRACK_WIDTH / sw.max(1.0)).min(1.0) as f32;
        let id = s.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
        let job = crate::Job { id, label: format!("Track Model ({})", it.name), progress: Default::default(), result: Default::default() };
        job.progress.total.store(steps as u64, std::sync::atomic::Ordering::Relaxed);
        let keys: Arc<Mutex<Vec<(Tick, f64, f64)>>> = Arc::default();
        let (prog, res, out) = (job.progress.clone(), job.result.clone(), keys.clone());
        let run = move || {
            use std::sync::atomic::Ordering;
            let t0 = web_time::Instant::now();
            let gray = |t: Tick| -> Option<(filmcraft_render::track::Prepared, f64)> {
                let f = src.video_frame(filmcraft_media::FrameRequest { time: t, scale }).ok()?;
                let g = filmcraft_render::track::Gray::from_rgba8(f.width as usize, f.height as usize, &f.to_rgba8());
                Some((filmcraft_render::track::Prepared::new(g), f64::from(f.width) / sw.max(1.0)))
            };
            let (mut dx, mut dy) = (0.0f64, 0.0f64);
            let mut misses = 0usize;
            let mut done = 0u64;
            let mut err: Option<String> = None;
            match gray(m0) {
                None => err = Some("can't decode the clip".into()),
                Some((mut prev, mut k_prev)) => {
                    lock(&out).push((m0, 0.0, 0.0));
                    for n in 1..=steps {
                        if prog.cancel.load(Ordering::Relaxed) {
                            err = Some("stopped".into());
                            break;
                        }
                        let t = Tick(m0.0.saturating_add(step.0.saturating_mul(n as i64)));
                        let Some((next, k)) = gray(t) else {
                            err = Some(format!("can't decode frame {n}"));
                            break;
                        };
                        let pts: Vec<Vec2> = region.iter().map(|p| (*p + Vec2::new(dx, dy)) * k_prev).collect();
                        match filmcraft_render::track::track_step(&prev, &next, &pts, TrackMethod::Position) {
                            Some(st) => {
                                // the window's centre moves with the tracked points
                                let c = pts.iter().fold(Vec2::new(0.0, 0.0), |a, p| a + *p) * 0.25;
                                let moved = st.transform.apply(c);
                                let (ox, oy) = ((moved.x / k) - (c.x / k_prev), (moved.y / k) - (c.y / k_prev));
                                if ox.is_finite() && oy.is_finite() {
                                    dx += ox;
                                    dy += oy;
                                }
                                misses = 0;
                            }
                            None => {
                                misses += 1;
                                if misses > MAX_MISSES {
                                    err = Some(format!("lost the model after {n} frame(s): not enough detail in the pane"));
                                    break;
                                }
                            }
                        }
                        lock(&out).push((t, dx, dy));
                        done += 1;
                        prog.done.store(done, Ordering::Relaxed);
                        *lock(&prog.status) = format!("Frame {n} of {steps}");
                        prev = next;
                        k_prev = k;
                    }
                }
            }
            let secs = t0.elapsed().as_secs_f64();
            *lock(&prog.status) = match &err {
                Some(e) => format!("{e} ({done} frame(s) tracked)"),
                None => format!("Tracked {done} frame(s) in {secs:.1}s"),
            };
            let r = match &err {
                Some(e) if e != "stopped" && done == 0 => Err(e.clone()),
                _ => Ok(filmcraft_export::Report {
                    path: String::new(),
                    frames: done,
                    seconds: secs,
                    bytes: 0,
                    render_fps: done as f64 / secs.max(1e-6),
                    extra_files: Vec::new(),
                }),
            };
            prog.finished.store(true, Ordering::Relaxed);
            *lock(&res) = Some(r);
        };
        let run = crate::export_tools::guard_job(job.progress.clone(), job.result.clone(), run);
        s.jobs.push(job);
        s.model_jobs.push(PendingModel { job: id, seq: seq_id, clip, keys });
        if wait || cfg!(target_arch = "wasm32") {
            run();
        } else {
            std::thread::Builder::new().name("filmcraft-model-track".into()).spawn(run).map_err(|e| EngineError::Other(e.to_string()))?;
        }
        started.push(json!({"job": id, "clip": clip.0, "frames": steps}));
    }
    if wait {
        poll(s);
    }
    Ok(json!({"jobs": started}))
}

/// Moving average over `w` samples on each side.
fn smooth(v: &[(Tick, f64, f64)], w: usize) -> Vec<(Tick, f64, f64)> {
    (0..v.len())
        .filter_map(|i| {
            let a = i.saturating_sub(w);
            let b = i.saturating_add(w).saturating_add(1).min(v.len());
            let win = v.get(a..b)?;
            let n = win.len().max(1) as f64;
            let (sx, sy) = win.iter().fold((0.0, 0.0), |(x, y), k| (x + k.1, y + k.2));
            Some((v.get(i)?.0, sx / n, sy / n))
        })
        .collect()
}

/// Apply finished tracking jobs (call regularly; [`Session::poll_persistence`] does).
pub fn poll(s: &mut Session) {
    use std::sync::atomic::Ordering;
    let mut i = 0;
    while i < s.model_jobs.len() {
        let Some(pj) = s.model_jobs.get(i) else { break };
        let finished = s.jobs.iter().find(|j| j.id == pj.job).is_none_or(|j| j.progress.finished.load(Ordering::Relaxed));
        if !finished {
            i += 1;
            continue;
        }
        let pj = s.model_jobs.remove(i);
        let keys = smooth(&lock(&pj.keys), 3);
        if keys.len() < 2 || s.state.active_sequence != Some(pj.seq) {
            continue;
        }
        let Some((sw, sh)) = s
            .active_sequence()
            .and_then(|q| q.find_item(pj.clip))
            .and_then(|(_, it)| filmcraft_render::source_size(&s.project, it.item))
            .map(|(w, h)| (f64::from(w), f64::from(h)))
        else {
            continue;
        };
        let clip = pj.clip;
        let _ = s.edit_sequence("Track Model", |q, _, _| {
            let Some((_, it)) = q.find_item_mut(clip) else { return Ok(()) };
            let Some(crop) = crop_of(it) else { return Ok(()) };
            let (vw, vh) = ((100.0 - crop[0] - crop[2]) / 100.0 * sw, (100.0 - crop[1] - crop[3]) / 100.0 * sh);
            let (cx0, cy0) = ((crop[0] / 100.0) * sw + vw / 2.0, (crop[1] / 100.0) * sh + vh / 2.0);
            let anchor0 = it.effect("motion").and_then(|m| m.params.get("anchor")).and_then(|a| a.value.as_vec2()).unwrap_or(Vec2::new(cx0, cy0));
            let mut anchors = Vec::with_capacity(keys.len());
            let mut crops = Vec::with_capacity(keys.len());
            for (t, dx, dy) in &keys {
                // the window follows, but stays inside the picture
                let cx = (cx0 + dx).max(vw / 2.0).min(sw - vw / 2.0);
                let cy = (cy0 + dy).max(vh / 2.0).min(sh - vh / 2.0);
                let (sx, sy) = (cx - cx0, cy - cy0);
                anchors.push((*t, Vec2::new(anchor0.x + sx, anchor0.y + sy)));
                let pct = |v: f64, of: f64| (v / of * 100.0).clamp(0.0, 100.0);
                crops.push((*t, [pct(cx - vw / 2.0, sw), pct(cy - vh / 2.0, sh), pct(sw - (cx + vw / 2.0), sw), pct(sh - (cy + vh / 2.0), sh)]));
            }
            if let Some(a) = it.effect_mut("motion").and_then(|m| m.params.get_mut("anchor")) {
                a.keyframes.clear();
                for (t, v) in &anchors {
                    a.put_keyframe(*t, ParamValue::Vec2(*v));
                }
            }
            if let Some(c) = crate::shorts_layout::pane_crop_index(it).and_then(|i| it.effects.get_mut(i)) {
                for (j, k) in ["left", "top", "right", "bottom"].into_iter().enumerate() {
                    if let Some(prm) = c.params.get_mut(k) {
                        prm.keyframes.clear();
                        for (t, v) in &crops {
                            prm.put_keyframe(*t, ParamValue::Float(v.get(j).copied().unwrap_or(0.0)));
                        }
                    }
                }
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_keeps_times_and_averages() {
        let v: Vec<(Tick, f64, f64)> = (0..10).map(|i| (Tick(i), if i == 5 { 10.0 } else { 0.0 }, 0.0)).collect();
        let s = smooth(&v, 2);
        assert_eq!(s.len(), 10);
        assert_eq!(s[5].0, Tick(5));
        assert!((s[5].1 - 2.0).abs() < 1e-9);
        assert!(smooth(&[], 3).is_empty());
    }
}
