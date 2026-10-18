//! Shorts ▸ Auto-Trim to Length…, Loop Ending, and the cutting behind Top Moments Countdown.
//!
//! - **Auto-Trim to Length**: keeps the stretch of the sequence (15, 30, 60 s…) with the most
//!   going on: the loud reactions Find Hype Moments looks for, weighted by how far they stand out,
//!   with a little weight on how busy the sound is so a quiet clip still keeps its liveliest part.
//!   The edges move off words, so nobody is cut off mid-word, and the result is never longer than
//!   asked (a 60-second short stays under a minute).
//! - **Top moments**: the biggest moments, each with the seconds leading up to it, put one after
//!   the other from the smallest to the biggest, for a countdown.
//! - **Loop Ending**: the last half second dissolves into the opening frame on every picture track
//!   (a one-frame hold of it ends the sequence) and the sound fades out, so the short replays
//!   without a jump.
//!
//! [`keep_windows`] does the cutting for both: every track is cut at the window edges and only
//! the clips inside the windows stay, laid end to end in the order given. Clips keep their ids,
//! effects and keyframes; links, transitions, captions and markers inside the windows move along.

use filmcraft_edit::EditCtx;
use filmcraft_edit::shorts::{self as sh, HypeRules, LoudMoment};
use filmcraft_edit::transcript::SeqWord;
use filmcraft_project::{ClipId, Sequence, Track, TrackItem, Transition, TransitionAlign, TransitionId};
use filmcraft_time::{FrameRate, Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::bad;
use crate::shorts::num_p;
use crate::shorts_fx::{HOP_S, mix_levels};
use crate::{EngineError, Result, Session};

/// Suffix of the held first frame Loop Ending puts after each picture track's last clip.
pub(crate) const LOOP_SUFFIX: &str = " (Loop)";
/// The longest sequence these commands analyse.
const MAX_SECONDS: f64 = 3.0 * 3600.0;

// ---------------------------------------------------------------------------------------------
// Keeping windows of a sequence
// ---------------------------------------------------------------------------------------------

/// Keep only `windows` of the sequence (in the order given; they must not overlap) and lay them
/// end to end from the start. Returns the new duration.
pub(crate) fn keep_windows(q: &mut Sequence, ctx: &mut EditCtx, windows: &[TimeRange]) -> Result<Tick> {
    if windows.is_empty() || windows.iter().any(|w| w.duration <= Tick::ZERO || w.start < Tick::ZERO) {
        return Err(EngineError::Other("nothing to keep".into()));
    }
    let mut sorted = windows.to_vec();
    sorted.sort_by_key(|w| w.start);
    if sorted.windows(2).any(|p| p.first().zip(p.get(1)).is_some_and(|(a, b)| a.end() > b.start)) {
        return Err(EngineError::Other("the parts to keep overlap".into()));
    }
    // cut every track at every window edge (locked tracks too: they are unlocked meanwhile)
    let locks: Vec<bool> = q.all_tracks().map(|t| t.locked).collect();
    for t in q.all_tracks_mut() {
        t.locked = false;
    }
    for w in windows {
        filmcraft_edit::razor(q, &[], w.start, ctx);
        filmcraft_edit::razor(q, &[], w.end(), ctx);
    }
    let mut dest = Vec::with_capacity(windows.len());
    let mut at = Tick::ZERO;
    for w in windows {
        dest.push(at);
        at = Tick(at.0.saturating_add(w.duration.0));
    }
    // (window index, new start) of a time inside a window
    let place = |t: Tick| -> Option<(usize, Tick)> {
        let i = windows.iter().position(|w| w.contains(t))?;
        let (w, d) = (windows.get(i)?, dest.get(i)?);
        Some((i, Tick(d.0.saturating_add(t.0.saturating_sub(w.start.0)))))
    };
    let mut kept: Vec<ClipId> = Vec::new();
    for tr in q.all_tracks_mut() {
        let items = std::mem::take(&mut tr.items);
        for mut it in items {
            let Some((i, start)) = place(it.start) else { continue };
            // after the cuts a clip lies inside one window; never let one run past it
            if let Some(w) = windows.get(i)
                && it.end() > w.end()
            {
                it.duration = Tick(w.end().0.saturating_sub(it.start.0));
            }
            if it.duration <= Tick::ZERO {
                continue;
            }
            it.start = start;
            kept.push(it.id);
            tr.items.push(it);
        }
    }
    for tr in q.all_tracks_mut() {
        for it in tr.items.iter_mut() {
            if it.link.is_some_and(|l| !kept.contains(&ClipId(l))) {
                it.link = None;
            }
        }
        // transitions inside one window move with it (when their clips stayed); the rest go
        let ids: Vec<ClipId> = tr.items.iter().map(|i| i.id).collect();
        let transitions = std::mem::take(&mut tr.transitions);
        tr.transitions = transitions
            .into_iter()
            .filter_map(|mut x| {
                let i = windows.iter().position(|w| x.start >= w.start && x.end() <= w.end())?;
                let ok = |c: Option<ClipId>| c.is_none_or(|c| ids.contains(&c));
                if !(ok(x.from) && ok(x.to)) || (x.from.is_none() && x.to.is_none()) {
                    return None;
                }
                x.start = Tick(dest.get(i)?.0.saturating_add(x.start.0.saturating_sub(windows.get(i)?.start.0)));
                Some(x)
            })
            .collect();
        tr.sort();
    }
    for ct in q.caption_tracks.iter_mut() {
        let caps = std::mem::take(&mut ct.captions);
        ct.captions = caps
            .into_iter()
            .filter_map(|mut c| {
                let (i, start) = place(c.start)?;
                let end = Tick(c.start.0.saturating_add(c.duration.0)).min(windows.get(i)?.end());
                c.duration = Tick(end.0.saturating_sub(c.start.0));
                c.start = start;
                (c.duration > Tick::ZERO).then_some(c)
            })
            .collect();
        ct.captions.sort_by_key(|c| c.start);
    }
    let markers = std::mem::take(&mut q.markers);
    q.markers = markers
        .into_iter()
        .filter_map(|mut m| {
            m.start = place(m.start)?.1;
            Some(m)
        })
        .collect();
    q.markers.sort_by_key(|m| m.start);
    q.mark_in = None;
    q.mark_out = None;
    q.split = Default::default();
    for (t, l) in q.all_tracks_mut().zip(locks) {
        t.locked = l;
    }
    q.check().map_err(EngineError::Other)?;
    Ok(at)
}

// ---------------------------------------------------------------------------------------------
// Finding the best part
// ---------------------------------------------------------------------------------------------

/// What the choice of windows is made from: the mix's level curve (dBFS every [`HOP_S`]), the
/// moments that stand out of it, and the spoken words.
pub(crate) struct Analysis {
    levels: Vec<f64>,
    duration: Tick,
    rate: FrameRate,
    words: Vec<SeqWord>,
}

pub(crate) fn analyse(s: &Session, cmd: &str) -> Result<Analysis> {
    let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let duration = q.duration();
    let rate = q.settings.frame_rate;
    if duration < Tick::from_seconds_f64(1.0) {
        return Err(bad(cmd, "the sequence is shorter than a second"));
    }
    if duration.seconds() > MAX_SECONDS {
        return Err(bad(cmd, "that is more than 3 hours: cut the part you want first"));
    }
    let levels = mix_levels(s, Tick::ZERO, duration)?;
    Ok(Analysis { levels, duration, rate, words: crate::transcript::sequence_words(s) })
}

fn hops(secs: f64) -> usize {
    if secs.is_finite() && secs > 0.0 { (secs / HOP_S).round() as usize } else { 0 }
}

fn at_hop(i: usize) -> Tick {
    Tick::from_seconds_f64(i as f64 * HOP_S)
}

impl Analysis {
    fn moments(&self, sensitivity: f64, max: usize, gap_s: f64) -> Vec<LoudMoment> {
        let rules = HypeRules { sensitivity_db: sensitivity, max_moments: max, min_gap_s: gap_s, ..HypeRules::default() };
        sh::loud_moments(&self.levels, HOP_S, &rules)
    }

    /// The `len`-long stretch with the most going on.
    fn best_window(&self, len: Tick) -> TimeRange {
        let n = self.levels.len();
        let lh = hops(len.seconds()).max(1);
        if lh >= n {
            return TimeRange::new(Tick::ZERO, self.duration.min(len));
        }
        let mut weight = vec![0.0f64; n];
        for m in self.moments(5.0, 500, 0.5) {
            if let Some(x) = weight.get_mut(m.at) {
                *x += m.excess_db.max(0.0);
            }
        }
        // how busy the sound is, 0..40 per hop, for windows that are otherwise alike
        let busy: Vec<f64> = self.levels.iter().map(|l| if l.is_finite() { (l + 50.0).clamp(0.0, 40.0) } else { 0.0 }).collect();
        let prefix = |v: &[f64]| {
            let mut p = Vec::with_capacity(v.len() + 1);
            p.push(0.0);
            let mut acc = 0.0;
            for x in v {
                acc += x;
                p.push(acc);
            }
            p
        };
        let indexed: Vec<f64> = weight.iter().enumerate().map(|(i, w)| w * i as f64).collect();
        let (pw, pb, pi) = (prefix(&weight), prefix(&busy), prefix(&indexed));
        let sum = |p: &[f64], a: usize, b: usize| p.get(b).copied().unwrap_or(0.0) - p.get(a).copied().unwrap_or(0.0);
        // a moment counts when the window holds a little of what led to it and of what followed
        let (lead, tail) = (hops(0.6).min(lh / 4), hops(0.8).min(lh / 4));
        let moments = |i: usize| sum(&pw, i + lead, i + lh - tail);
        let most = (0..=(n - lh)).map(moments).fold(f64::NEG_INFINITY, f64::max);
        // of the windows holding as much, the one with its moments 60 % of the way in (some
        // build-up before them and the reaction after), then the busiest
        let off_centre = |i: usize| {
            let w = moments(i);
            if w <= 0.0 {
                return 0.0;
            }
            let centre = sum(&pi, i + lead, i + lh - tail) / w;
            (centre - (i as f64 + 0.6 * lh as f64)).abs()
        };
        let mut best: Option<(f64, f64, usize)> = None;
        for i in 0..=(n - lh) {
            if moments(i) < most - 1e-6 {
                continue;
            }
            let (off, quiet) = (off_centre(i), -sum(&pb, i, i + lh));
            if best.is_none_or(|b| off < b.0 - 1e-9 || (off <= b.0 + 1e-9 && quiet < b.1 - 1e-9)) {
                best = Some((off, quiet, i));
            }
        }
        let best = (0.0, best.map_or(0, |b| b.2));
        TimeRange::new(at_hop(best.1), len)
    }

    /// `w` with its edges off words (a start inside a word moves to just before it, an end inside
    /// one to just before that word), never longer than it was, on frame boundaries.
    fn tidy(&self, w: TimeRange) -> TimeRange {
        let mut a = w.start.max(Tick::ZERO);
        let mut b = w.end().min(self.duration);
        let pad = Tick::from_seconds_f64(0.08);
        if let Some(word) = self.words.iter().find(|x| x.start < a && a < x.end) {
            let moved = Tick(word.start.0.saturating_sub(pad.0)).max(Tick::ZERO);
            // keep the length: the end comes back as much as the start did
            b = Tick(b.0.saturating_sub(a.0.saturating_sub(moved.0)));
            a = moved;
        }
        if let Some(word) = self.words.iter().find(|x| x.start < b && b < x.end && x.start > a) {
            b = Tick(word.start.0.saturating_sub(Tick::from_seconds_f64(0.03).0)).max(a);
        }
        let a = self.rate.snap(a);
        let b = self.rate.snap(b);
        let frame = self.rate.frame_duration().max(Tick(1));
        TimeRange::from_bounds(a, b.max(Tick(a.0.saturating_add(frame.0))).min(self.duration.max(Tick(a.0.saturating_add(frame.0)))))
    }

    /// The `n` biggest moments as windows `each` long (most of it leading up to the moment), in
    /// countdown order: smallest first, biggest last. Each comes with how far it stood out (dB).
    fn top(&self, n: usize, each: Tick) -> Vec<(TimeRange, f64)> {
        let gap = each.seconds() + 0.5;
        let mut found = self.moments(6.0, n, gap);
        if found.len() < n {
            found = self.moments(3.0, n, gap);
        }
        let lead = Tick((each.0 as f64 * 0.6) as i64);
        let mut picks: Vec<(TimeRange, f64)> = Vec::new();
        // biggest first, so a later one that would overlap is the one dropped
        found.sort_by(|a, b| b.excess_db.total_cmp(&a.excess_db));
        for m in found {
            let t = at_hop(m.at);
            let b = Tick(t.0.saturating_sub(lead.0).max(0).saturating_add(each.0)).min(self.duration);
            let a = Tick(b.0.saturating_sub(each.0)).max(Tick::ZERO);
            let w = self.tidy(TimeRange::from_bounds(a, b));
            if w.duration > Tick::ZERO && !picks.iter().any(|(p, _)| p.overlaps(&w)) {
                picks.push((w, m.excess_db));
            }
        }
        picks.reverse();
        picks
    }
}

// ---------------------------------------------------------------------------------------------
// Auto-Trim to Length
// ---------------------------------------------------------------------------------------------

pub(crate) fn auto_trim(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.autoTrim";
    let secs = num_p(p, "seconds", 30.0, 3.0, 600.0, CMD)?;
    s.edit_group("Auto-Trim to Length", |s| trim_to(s, Tick::from_seconds_f64(secs), CMD))
}

/// Trim the active sequence to its best `len` (see [`Analysis::best_window`]).
pub(crate) fn trim_to(s: &mut Session, len: Tick, cmd: &str) -> Result<Value> {
    let a = analyse(s, cmd)?;
    if a.duration <= Tick(len.0.saturating_add(a.rate.frame_duration().0)) {
        return Ok(json!({"trimmed": false, "seconds": a.duration.seconds()}));
    }
    let w = a.tidy(a.best_window(len));
    let total = s.edit_sequence("Auto-Trim to Length", |q, ctx, _| keep_windows(q, ctx, &[w]))?;
    Ok(json!({"trimmed": true, "from": w.start.0, "to": w.end().0, "seconds": total.seconds()}))
}

/// A part of a countdown: where it now is in the sequence and its place (3, 2, 1).
pub(crate) struct Moment {
    pub range: TimeRange,
    pub rank: usize,
    pub excess_db: f64,
}

/// Cut the active sequence down to its `n` biggest moments, `each` long, smallest first.
pub(crate) fn countdown_cut(s: &mut Session, n: usize, each: Tick, cmd: &str) -> Result<Vec<Moment>> {
    let a = analyse(s, cmd)?;
    let picks = a.top(n, each);
    if picks.len() < 2 {
        return Err(bad(
            cmd,
            format!(
                "found {} moment{} that stand{} out from the rest of the sound; a countdown needs at least 2 (try Auto-Trim to Length instead)",
                picks.len(),
                if picks.len() == 1 { "" } else { "s" },
                if picks.len() == 1 { "s" } else { "" }
            ),
        ));
    }
    let windows: Vec<TimeRange> = picks.iter().map(|(w, _)| *w).collect();
    s.edit_sequence("Top Moments", |q, ctx, _| keep_windows(q, ctx, &windows))?;
    let k = picks.len();
    let mut at = Tick::ZERO;
    Ok(picks
        .iter()
        .enumerate()
        .map(|(i, (w, x))| {
            let m = Moment { range: TimeRange::new(at, w.duration), rank: k - i, excess_db: *x };
            at = Tick(at.0.saturating_add(w.duration.0));
            m
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Loop Ending
// ---------------------------------------------------------------------------------------------

pub(crate) fn loop_ending(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "shorts.loopEnding";
    let secs = num_p(p, "seconds", 0.5, 0.1, 3.0, CMD)?;
    let n = apply_loop(s, secs, CMD)?;
    Ok(json!({"tracks": n}))
}

/// The clips of the active sequence that show footage (or a layout's copy of it).
fn footage_ids(s: &Session) -> Vec<ClipId> {
    let Some(q) = s.active_sequence() else { return Vec::new() };
    q.video_tracks.iter().flat_map(|t| t.items.iter()).filter(|i| crate::shorts::is_footage(&s.project, i)).map(|i| i.id).collect()
}

/// Remove an earlier Loop Ending: its held frames, the dissolves into them, and its fades of the
/// sound at the very start and end.
pub(crate) fn remove_loop(q: &mut Sequence) {
    let mut any = false;
    for tr in q.video_tracks.iter_mut() {
        let holds: Vec<ClipId> = tr.items.iter().filter(|i| is_loop_hold(i)).map(|i| i.id).collect();
        if holds.is_empty() {
            continue;
        }
        any = true;
        tr.items.retain(|i| !holds.contains(&i.id));
        tr.transitions.retain(|x| !x.to.is_some_and(|c| holds.contains(&c)));
    }
    if !any {
        return;
    }
    let end = q.duration();
    for tr in q.audio_tracks.iter_mut() {
        tr.transitions.retain(|x| {
            let fade = x.effect.effect == "constant_power" && (x.from.is_none() != x.to.is_none());
            !(fade && (x.start == Tick::ZERO || x.end() == end))
        });
    }
}

/// How long the dissolve of the sequence's Loop Ending is (seconds), when it has one.
pub(crate) fn loop_seconds(q: &Sequence) -> Option<f64> {
    q.video_tracks.iter().find_map(|tr| {
        let hold = tr.items.iter().find(|i| is_loop_hold(i))?;
        tr.transitions.iter().find(|x| x.to == Some(hold.id)).map(|x| x.duration.seconds())
    })
}

/// Dissolve the last `secs` of every picture track into its first frame (held for one frame at
/// the end) and fade the sound out (and in over two frames), as one undo step. Returns how many
/// picture tracks loop.
pub(crate) fn apply_loop(s: &mut Session, secs: f64, cmd: &str) -> Result<usize> {
    let footage = footage_ids(s);
    if footage.is_empty() {
        return Err(bad(cmd, "the sequence has no video footage"));
    }
    s.edit_sequence("Loop Ending", |q, ctx, _| {
        remove_loop(q);
        let n = add_loop(q, ctx, &footage, secs);
        if n == 0 {
            return Err(bad(cmd, "no picture track runs from the start to the end of the sequence"));
        }
        q.check().map_err(EngineError::Other)?;
        Ok(n)
    })
}

/// The edit behind [`apply_loop`]: `footage` are the clips that count as picture. Returns how
/// many picture tracks loop.
pub(crate) fn add_loop(q: &mut Sequence, ctx: &mut EditCtx, footage: &[ClipId], secs: f64) -> usize {
    let rate = q.settings.frame_rate;
    let frame = rate.frame_duration().max(Tick(1));
    let end = q.duration();
    let secs = if secs.is_finite() { secs.clamp(0.0, 10.0) } else { 0.5 };
    let d = rate.snap_nearest(Tick::from_seconds_f64(secs)).max(frame);
    let mut n = 0;
    for tr in q.video_tracks.iter_mut() {
        let first = tr.items.iter().find(|i| i.start == Tick::ZERO && footage.contains(&i.id)).cloned();
        let last = tr.items.iter().filter(|i| footage.contains(&i.id)).max_by_key(|i| i.end()).cloned();
        let (Some(first), Some(last)) = (first, last) else { continue };
        if last.end() != end || last.duration <= d {
            continue;
        }
        let hold_at = first.source_time_at(first.start);
        let mut hold = first.clone();
        hold.id = ClipId(ctx.alloc());
        hold.name = format!("{}{LOOP_SUFFIX}", first.name);
        hold.start = end;
        hold.duration = frame;
        hold.source_in = hold_at;
        hold.frame_hold = Some(hold_at);
        hold.link = None;
        hold.group = None;
        hold.markers.clear();
        let dissolve = Transition {
            id: TransitionId(ctx.alloc()),
            effect: crate::presets::effect_instance("cross_dissolve"),
            start: Tick(end.0.saturating_sub(d.0)),
            duration: d,
            from: Some(last.id),
            to: Some(hold.id),
            align: TransitionAlign::EndAtCut,
            reverse: false,
        };
        put_transition(tr, dissolve);
        tr.items.push(hold);
        tr.sort();
        n += 1;
    }
    if n == 0 {
        return 0;
    }
    let fade_in = Tick(frame.0.saturating_mul(2));
    for tr in q.audio_tracks.iter_mut() {
        let last = tr.items.iter().filter(|i| i.end() == end && i.duration > d).map(|i| i.id).next();
        if let Some(c) = last {
            let x = Transition {
                id: TransitionId(ctx.alloc()),
                effect: crate::presets::effect_instance("constant_power"),
                start: Tick(end.0.saturating_sub(d.0)),
                duration: d,
                from: Some(c),
                to: None,
                align: TransitionAlign::EndAtCut,
                reverse: false,
            };
            put_transition(tr, x);
        }
        let first = tr.items.iter().find(|i| i.start == Tick::ZERO && i.duration > fade_in).map(|i| i.id);
        if let Some(c) = first {
            let x = Transition {
                id: TransitionId(ctx.alloc()),
                effect: crate::presets::effect_instance("constant_power"),
                start: Tick::ZERO,
                duration: fade_in,
                from: None,
                to: Some(c),
                align: TransitionAlign::StartAtCut,
                reverse: false,
            };
            put_transition(tr, x);
        }
        tr.sort();
    }
    n
}

/// Add `x` to the track, replacing a transition already over that time.
fn put_transition(tr: &mut Track, x: Transition) {
    tr.transitions.retain(|o| !o.range().overlaps(&x.range()));
    tr.transitions.push(x);
}

/// Whether `it` is a held frame Loop Ending added.
pub(crate) fn is_loop_hold(it: &TrackItem) -> bool {
    it.name.ends_with(LOOP_SUFFIX) && it.frame_hold.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analysis(levels: Vec<f64>, words: Vec<SeqWord>) -> Analysis {
        let duration = Tick::from_seconds_f64(levels.len() as f64 * HOP_S);
        Analysis { levels, duration, rate: FrameRate::FPS_30, words }
    }

    fn word(text: &str, a: f64, b: f64) -> SeqWord {
        SeqWord {
            text: text.into(),
            start: Tick::from_seconds_f64(a),
            end: Tick::from_seconds_f64(b),
            clip: ClipId(1),
            item: filmcraft_project::ItemId(1),
            index: 0,
            track: 0,
            speaker: None,
            confidence: 1.0,
        }
    }

    #[test]
    fn the_best_window_holds_the_loud_moments() {
        // 60 s of talking at -30 dB, two screams at 40 s and 44 s, a laugh at 10 s
        let mut lv = vec![-30.0; 1200];
        for at in [800usize, 880] {
            for v in lv.iter_mut().skip(at).take(10) {
                *v = -6.0;
            }
        }
        for v in lv.iter_mut().skip(200).take(6) {
            *v = -18.0;
        }
        let a = analysis(lv, Vec::new());
        let w = a.best_window(Tick::from_seconds_f64(15.0));
        assert!(w.start.seconds() <= 39.5 && w.end().seconds() >= 45.0, "{:?}..{:?}", w.start.seconds(), w.end().seconds());
        assert_eq!(w.duration, Tick::from_seconds_f64(15.0));
        // longer than the sequence: all of it
        let all = a.best_window(Tick::from_seconds_f64(100.0));
        assert_eq!(all.start, Tick::ZERO);
        assert_eq!(all.duration, a.duration);
    }

    #[test]
    fn edges_move_off_words_and_never_lengthen() {
        let a = analysis(vec![-30.0; 400], vec![word("hello", 4.9, 5.4), word("there", 9.8, 10.4)]);
        let w = a.tidy(TimeRange::from_bounds(Tick::from_seconds_f64(5.0), Tick::from_seconds_f64(10.0)));
        assert!(w.start.seconds() < 4.9, "starts before the word: {}", w.start.seconds());
        assert!(w.end().seconds() <= 9.8, "ends before the cut-off word: {}", w.end().seconds());
        assert!(w.duration.seconds() <= 5.0 + 1e-6);
        // on frame boundaries
        assert_eq!(FrameRate::FPS_30.snap(w.start), w.start);
    }

    #[test]
    fn top_moments_count_down_to_the_biggest() {
        let mut lv = vec![-30.0; 1200];
        for (at, db) in [(200usize, -20.0), (600, -6.0), (1000, -14.0)] {
            for v in lv.iter_mut().skip(at).take(8) {
                *v = db;
            }
        }
        let a = analysis(lv, Vec::new());
        let top = a.top(3, Tick::from_seconds_f64(6.0));
        assert_eq!(top.len(), 3);
        let order: Vec<f64> = top.iter().map(|(w, _)| w.start.seconds()).collect();
        // smallest (10 s), then 50 s, then the biggest (30 s)
        assert!(order[0] < 10.0 && order[1] > 40.0 && (25.0..30.0).contains(&order[2]), "{order:?}");
        assert!(top.windows(2).all(|p| p[0].1 <= p[1].1));
        for (w, _) in &top {
            assert!((w.duration.seconds() - 6.0).abs() < 0.05);
        }
    }

    #[test]
    fn hostile_levels_do_not_panic() {
        let a = analysis(vec![f64::NAN, f64::INFINITY, -f64::INFINITY, 0.0], Vec::new());
        let _ = a.best_window(Tick::from_seconds_f64(0.1));
        let _ = a.top(5, Tick::from_seconds_f64(0.05));
        let empty = analysis(Vec::new(), Vec::new());
        let _ = empty.best_window(Tick::from_seconds_f64(10.0));
        assert!(empty.top(3, Tick::from_seconds_f64(5.0)).is_empty());
    }
}
