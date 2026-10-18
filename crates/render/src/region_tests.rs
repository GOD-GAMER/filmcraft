//! The compositor's shortcuts must give exactly what the general path gives, bit for bit: an opaque
//! bottom layer taken as the canvas, a layer that is only its non-transparent rectangle, and the
//! recycled float buffers. The reference is the plain loop over `item_layer` + `blend::composite`
//! (the path every clip takes when no shortcut applies).

use super::*;
use filmcraft_frame::{AudioBuffer, Chroma};
use filmcraft_media::generators::GeneratorSource;
use filmcraft_media::{Generator, MediaInfo, MediaSource};
use filmcraft_project::{Label, MediaClip, MediaRef, SequenceSettings, TrackKind, find_effect};
use filmcraft_time::{FrameRate, TICKS_PER_SECOND, TimeRange};

const FPS: FrameRate = FrameRate::FPS_24;

struct Still {
    info: MediaInfo,
    frame: Arc<VideoFrame>,
}

impl MediaSource for Still {
    fn info(&self) -> &MediaInfo {
        &self.info
    }
    fn video_frame(&self, _req: FrameRequest) -> filmcraft_media::Result<Arc<VideoFrame>> {
        Ok(self.frame.clone())
    }
    fn audio(&self, _start: i64, frames: usize, sample_rate: u32) -> filmcraft_media::Result<AudioBuffer> {
        Ok(AudioBuffer::silence(sample_rate, 2, frames))
    }
}

fn frame(w: usize, h: usize, data: PixelData) -> Arc<VideoFrame> {
    Arc::new(VideoFrame { width: w as u32, height: h as u32, data, color: filmcraft_color::ColorInfo::REC709, par: (1, 1), pts: Tick::ZERO })
}

/// A camera-like picture: 8-bit 4:2:0, no alpha.
fn camera(w: usize, h: usize) -> Arc<VideoFrame> {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let y: Vec<u8> = (0..w * h).map(|i| (16 + (i * 7 + i / w * 3) % 219) as u8).collect();
    let u: Vec<u8> = (0..cw * ch).map(|i| (60 + i * 5 % 140) as u8).collect();
    let v: Vec<u8> = (0..cw * ch).map(|i| (50 + i * 11 % 150) as u8).collect();
    frame(w, h, PixelData::Yuv8 { planes: [Arc::new(y), Arc::new(u), Arc::new(v)], chroma: Chroma::C420, alpha: None })
}

/// Alpha 10-bit: 0 outside `rect` (x0, y0, x1, y1), 1023 inside, with a half-transparent one-pixel rim.
fn alpha_in(w: usize, h: usize, rect: (usize, usize, usize, usize)) -> Vec<u16> {
    let (x0, y0, x1, y1) = rect;
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if x < x0 || x >= x1 || y < y0 || y >= y1 {
                0
            } else if x == x0 || y == y0 {
                512
            } else {
                1023
            }
        })
        .collect()
}

/// A ProRes-4444-like picture: 10-bit 4:4:4 with alpha only inside `rect`.
fn banner(w: usize, h: usize, rect: (usize, usize, usize, usize)) -> Arc<VideoFrame> {
    let n = w * h;
    frame(
        w,
        h,
        PixelData::Yuv16 {
            planes: [
                Arc::new((0..n).map(|i| (200 + i * 3 % 600) as u16).collect()),
                Arc::new((0..n).map(|i| (400 + i % 200) as u16).collect()),
                Arc::new((0..n).map(|i| (300 + i * 7 % 300) as u16).collect()),
            ],
            chroma: Chroma::C444,
            bits: 10,
            alpha: Some(Arc::new(alpha_in(w, h, rect))),
        },
    )
}

/// An 8-bit RGBA still with alpha only inside `rect`.
fn sticker(w: usize, h: usize, rect: (usize, usize, usize, usize)) -> Arc<VideoFrame> {
    let a = alpha_in(w, h, rect);
    let px: Vec<u8> = (0..w * h).flat_map(|i| [(i * 13 % 251) as u8, (i * 7 % 241) as u8, (i * 29 % 239) as u8, (a[i] / 4) as u8]).collect();
    frame(w, h, PixelData::Rgba8(Arc::new(px)))
}

struct Spec {
    frame: Arc<VideoFrame>,
    opacity: f64,
    blend: u32,
    motion_scale: Option<f64>,
    effect: Option<&'static str>,
}

impl Spec {
    fn new(frame: Arc<VideoFrame>) -> Self {
        Self { frame, opacity: 100.0, blend: 0, motion_scale: None, effect: None }
    }
}

const W: usize = 96;
const H: usize = 54;

fn scene(specs: &[Spec]) -> (Project, ItemId, SourceMap) {
    scene_sized(W, H, specs)
}

/// [`scene`] in a `w`×`h` sequence.
fn scene_sized(w: usize, h: usize, specs: &[Spec]) -> (Project, ItemId, SourceMap) {
    let mut p = Project::new("regions");
    let mut map = SourceMap::default();
    let seq = p.new_sequence("s", SequenceSettings { width: w as u32, height: h as u32, frame_rate: FPS, ..Default::default() }, specs.len(), 1, None);
    for (track, spec) in specs.iter().enumerate() {
        let g =
            GeneratorSource::new(Generator::ColorMatte { color: [0.0, 0.0, 0.0, 1.0] }, spec.frame.width, spec.frame.height, FPS, Tick(10 * TICKS_PER_SECOND));
        let info = g.info().clone();
        let item = p.add_item(
            &format!("layer{track}"),
            Label::Iris,
            ItemKind::Media(MediaClip {
                media: MediaRef::Generator(g.generator.clone()),
                info: info.clone(),
                interpret: Default::default(),
                mark_in: None,
                mark_out: None,
                markers: vec![],
                offline: false,
                proxy: None,
                identity: None,
            }),
            None,
        );
        let mut clip = p.make_track_item(item, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, FPS.tick_of(48)), FPS).expect("clip");
        clip.scale_to_frame = true;
        {
            let o = clip.effect_mut("opacity").expect("opacity");
            o.params.get_mut("opacity").expect("opacity").value = ParamValue::Float(spec.opacity);
            o.params.get_mut("blend").expect("blend").value = ParamValue::Choice(spec.blend);
        }
        if let Some(s) = spec.motion_scale {
            clip.effect_mut("motion").expect("motion").params.get_mut("scale").expect("scale").value = ParamValue::Float(s);
        }
        if let Some(id) = spec.effect {
            clip.effects.push(find_effect(id).expect("effect").instance());
        }
        if let Some(t) = p.sequence_mut(seq).and_then(|s| s.video_tracks.get_mut(track)) {
            t.items.push(clip);
        }
        map.0.insert(item, Arc::new(Still { info, frame: spec.frame.clone() }));
    }
    (p, seq, map)
}

/// What every clip gets when no shortcut applies: each layer from the general path, mixed into a
/// zeroed canvas.
fn reference(p: &Project, seq: ItemId, t: Tick, opts: RenderOptions, map: &SourceMap) -> Image {
    let s = p.sequence(seq).expect("sequence");
    let (w, h) = output_size(s, opts.scale);
    let mut canvas = Image::new(w, h);
    let tc = format_time(t, s.settings.frame_rate, s.settings.drop_frame, TimeDisplay::Timecode, s.settings.sample_rate as i64);
    for track in &s.video_tracks {
        let Some(item) = track.item_at(t) else { continue };
        if let Some((layer, op, bl)) = item_layer(p, s, item, t, opts, map, &tc) {
            blend::composite(&mut canvas, &layer, op, bl);
        }
    }
    canvas
}

fn same_bits(a: &Image, b: &Image, what: &str) {
    assert_eq!((a.w, a.h, a.px.len()), (b.w, b.h, b.px.len()), "{what}: size");
    if let Some(i) = a.px.iter().zip(&b.px).position(|(x, y)| x.to_bits() != y.to_bits()) {
        panic!("{what}: first difference at pixel {} channel {}: {} vs {}", i / 4, i % 4, a.px[i], b.px[i]);
    }
}

fn check(what: &str, specs: &[Spec], scale: f32) {
    let (p, seq, map) = scene(specs);
    let opts = RenderOptions { scale, ..Default::default() };
    for f in [0, 7, 30] {
        let t = FPS.tick_of(f);
        // twice: the second render starts from recycled buffers
        for round in 0..2 {
            let got = render_sequence(&p, seq, t, opts, &map);
            same_bits(&got, &reference(&p, seq, t, opts, &map), &format!("{what} (frame {f}, render {round})"));
            pool::recycle_f32(got.px);
        }
    }
}

#[test]
fn shortcuts_give_the_general_paths_bits() {
    let bar = (0, H - 9, W, H);
    let corner = (50, 3, 91, 20);
    let full = (0, 0, W, H);
    let cam = || Spec::new(camera(W, H));
    let ban = |rect| Spec::new(banner(W, H, rect));

    check("camera only", &[cam()], 1.0);
    check("camera + bar banner", &[cam(), ban(bar)], 1.0);
    check("camera + corner banner", &[cam(), ban(corner)], 1.0);
    check("camera + banner covering everything", &[cam(), ban(full)], 1.0);
    check("camera + two banners", &[cam(), ban(bar), ban(corner)], 1.0);
    check("banner alone (first layer)", &[ban(bar)], 1.0);
    check("banner under the camera", &[ban(bar), cam()], 1.0);
    check("camera + sticker", &[cam(), Spec::new(sticker(W, H, corner))], 1.0);
    check("camera + transparent banner", &[cam(), ban((0, 0, 0, 0))], 1.0);
    // opacity and blend modes (every mode, over the camera and as the first layer)
    for mode in 0..Blend::ALL.len() as u32 {
        check(&format!("banner blend {mode}"), &[cam(), Spec { opacity: 70.0, blend: mode, ..ban(corner) }], 1.0);
        check(&format!("camera blend {mode} first"), &[Spec { opacity: 100.0, blend: mode, ..cam() }, ban(bar)], 1.0);
    }
    check("camera at half opacity", &[Spec { opacity: 50.0, ..cam() }, ban(bar)], 1.0);
    check("camera at zero opacity", &[Spec { opacity: 0.0, ..cam() }, ban(bar)], 1.0);
    check("banner at zero opacity", &[cam(), Spec { opacity: 0.0, ..ban(bar) }], 1.0);
    // not placed as it is, or with effects: the general path
    check("moved camera", &[Spec { motion_scale: Some(50.0), ..cam() }, ban(bar)], 1.0);
    check("scaled banner", &[cam(), Spec { motion_scale: Some(60.0), ..ban(corner) }], 1.0);
    check("camera with an effect", &[Spec { effect: Some("brightness_contrast"), ..cam() }, ban(bar)], 1.0);
    check("banner with an effect", &[cam(), Spec { effect: Some("gaussian_blur"), ..ban(bar) }], 1.0);
    // a source of another size than the sequence's
    check("bigger banner", &[cam(), Spec::new(banner(W * 2, H * 2, (0, H * 2 - 18, W * 2, H * 2)))], 1.0);
    // reduced-resolution rendering decimates every frame
    check("half scale", &[cam(), ban(bar)], 0.5);
    check("quarter scale", &[cam(), ban(corner)], 0.25);
}

#[test]
fn which_layers_take_a_shortcut() {
    let (p, seq, map) = scene(&[Spec::new(camera(W, H)), Spec::new(banner(W, H, (0, H - 9, W, H)))]);
    let s = p.sequence(seq).expect("sequence");
    let (opts, t) = (RenderOptions::default(), FPS.tick_of(3));
    let layer = |track: usize, allow: bool| item_layer_ex(&p, s, s.video_tracks[track].item_at(t).expect("clip"), t, opts, &map, "", allow);
    // a camera frame has no alpha: opaque, the whole picture
    assert!(matches!(layer(0, true), Some(Layer::Full { opaque: true, .. })));
    // a banner is only its rectangle (rows H-9.. of a 96-wide picture)
    match layer(1, true) {
        Some(Layer::Region { image, x, y, .. }) => assert_eq!((x, y, image.w, image.h), (0, H - 9, W, 9)),
        _ => panic!("the banner should be a region"),
    }
    // without permission every layer is a whole image, and none is known to be opaque
    assert!(matches!(layer(0, false), Some(Layer::Full { opaque: false, .. })));
    assert!(matches!(layer(1, false), Some(Layer::Full { opaque: false, .. })));
    // a transparent banner is an empty region
    let (p, seq, map) = scene(&[Spec::new(banner(W, H, (0, 0, 0, 0)))]);
    let s = p.sequence(seq).expect("sequence");
    match item_layer_ex(&p, s, s.video_tracks[0].item_at(t).expect("clip"), t, opts, &map, "", true) {
        Some(Layer::Region { image, .. }) => assert_eq!((image.w, image.h, image.px.len()), (0, 0, 0)),
        _ => panic!("a transparent banner is an empty region"),
    }
    // an effect keeps the clip on the general path, a whole image
    let (p, seq, map) = scene(&[Spec { effect: Some("brightness_contrast"), ..Spec::new(camera(W, H)) }]);
    let s = p.sequence(seq).expect("sequence");
    assert!(matches!(item_layer_ex(&p, s, s.video_tracks[0].item_at(t).expect("clip"), t, opts, &map, "", true), Some(Layer::Full { opaque: false, .. })));
    // a move is only the rectangle the picture lands on (half the size, in the middle), and a
    // whole image without permission
    let (p, seq, map) = scene(&[Spec { motion_scale: Some(50.0), ..Spec::new(camera(W, H)) }]);
    let s = p.sequence(seq).expect("sequence");
    let it = s.video_tracks[0].item_at(t).expect("clip");
    match item_layer_ex(&p, s, it, t, opts, &map, "", true) {
        Some(Layer::Region { image, x, y, .. }) => {
            assert!(x + 1 >= W / 4 && y + 1 >= H / 4, "{x} {y}");
            assert!(image.w <= W / 2 + 2 && image.h <= H / 2 + 2 && image.w >= W / 2 && image.h >= H / 2, "{}x{}", image.w, image.h);
        }
        _ => panic!("a moved clip should be a region"),
    }
    assert!(matches!(item_layer_ex(&p, s, it, t, opts, &map, "", false), Some(Layer::Full { opaque: false, .. })));
}

#[test]
fn a_rectangle_mixes_like_the_padded_layer_in_every_blend_mode() {
    let (w, h) = (40usize, 30usize);
    let rect = Image { w: 11, h: 7, px: (0..11 * 7 * 4).map(|i| [0.0, 0.2, 0.9, 0.5, 0.01][i % 5] * ((i / 4 % 3) as f32 + 1.0) * 0.3).collect() };
    let base = Image { w, h, px: (0..w * h * 4).map(|i| if i % 4 == 3 { 0.8 } else { ((i * 37 % 100) as f32) / 130.0 }).collect() };
    for mode in Blend::ALL.iter().copied() {
        for (x0, y0) in [(0usize, 0usize), (13, 9), (29, 23), (35, 28)] {
            // the rectangle placed in a transparent layer of the full size
            let mut padded = Image::new(w, h);
            for ry in 0..rect.h {
                for rx in 0..rect.w {
                    if x0 + rx < w && y0 + ry < h {
                        let (s, d) = ((ry * rect.w + rx) * 4, ((y0 + ry) * w + x0 + rx) * 4);
                        padded.px[d..d + 4].copy_from_slice(&rect.px[s..s + 4]);
                    }
                }
            }
            let (mut a, mut b) = (base.clone(), base.clone());
            blend::composite(&mut a, &padded, 0.8, mode);
            blend::composite_at(&mut b, &rect, x0, y0, 0.8, mode);
            same_bits(&a, &b, &format!("{mode:?} at ({x0}, {y0})"));
        }
    }
    // a rectangle that misses the picture, or an empty one, changes nothing
    let mut c = base.clone();
    blend::composite_at(&mut c, &rect, 40, 0, 1.0, Blend::Normal);
    blend::composite_at(&mut c, &rect, 0, 30, 1.0, Blend::Normal);
    blend::composite_at(&mut c, &Image { w: 0, h: 0, px: vec![] }, 3, 3, 1.0, Blend::Normal);
    same_bits(&c, &base, "outside");
}

#[test]
fn a_region_layer_expands_to_the_whole_layer() {
    let image = Image { w: 3, h: 2, px: (0..24).map(|i| i as f32 + 1.0).collect() };
    let (full, op, bl) = Layer::Region { image, x: 2, y: 1, opacity: 0.5, blend: Blend::Multiply }.into_full(6, 4);
    assert_eq!((full.w, full.h, op, bl), (6, 4, 0.5, Blend::Multiply));
    assert_eq!(&full.px[((6 + 2) * 4)..((6 + 5) * 4)], &(1..=12).map(|i| i as f32).collect::<Vec<_>>()[..]);
    assert_eq!(&full.px[((2 * 6 + 2) * 4)..((2 * 6 + 5) * 4)], &(13..=24).map(|i| i as f32).collect::<Vec<_>>()[..]);
    assert_eq!(full.px.iter().filter(|v| **v != 0.0).count(), 24);
    // a region that sticks out is cut, not a panic
    let sticks = Image { w: 4, h: 3, px: vec![1.0; 48] };
    let (full, ..) = Layer::Region { image: sticks, x: 4, y: 2, opacity: 1.0, blend: Blend::Normal }.into_full(6, 4);
    assert_eq!(full.px.len(), 6 * 4 * 4);
    // only the two columns and two rows inside the output are copied, none wrapped into the next row
    assert_eq!(full.px.iter().filter(|v| **v == 1.0).count(), 2 * 2 * 4);
    assert!(full.px[..(2 * 6 + 4) * 4].iter().all(|v| *v == 0.0));
}

/// HDR and wide-gamut Y'CbCr on a Rec. 709 sequence is planned as a GPU layer carrying its input
/// transform, not converted to a float image on the CPU (#965: 4K HLG / PQ playback fell to a
/// third of real time), and the plan still renders what the reference renderer does.
#[test]
fn hdr_media_is_planned_for_the_gpu_with_its_colour_management() {
    use filmcraft_color::{ColorInfo, Matrix, Primaries, Range, Transfer};
    let hdr = |transfer| ColorInfo { matrix: Matrix::Bt2020Ncl, transfer, primaries: Primaries::Bt2020, range: Range::Limited };
    for (name, color) in
        [("HLG", hdr(Transfer::Hlg)), ("PQ", hdr(Transfer::Pq)), ("BT.2020 SDR", ColorInfo { primaries: Primaries::Bt2020, ..ColorInfo::REC709 })]
    {
        let f = Arc::new(VideoFrame { color, ..(*camera(W, H)).clone() });
        let (p, seq, map) = scene(&[Spec::new(f)]);
        let opts = RenderOptions::default();
        let plan = plan::plan_frame(&p, seq, Tick::ZERO, opts, &map);
        let plan::FramePlan::Layers { layers, .. } = &plan else { panic!("{name}: not a layer plan") };
        assert_eq!(layers.len(), 1, "{name}");
        assert!(matches!(layers[0].frame.data, PixelData::Yuv8 { .. }), "{name}: converted on the CPU");
        assert!(layers[0].input.is_some(), "{name}: no colour management for the GPU");
        let reference = render_sequence(&p, seq, Tick::ZERO, opts, &map);
        let got = plan::execute_cpu(&plan);
        let max = got.px.iter().zip(&reference.px).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(max < 1e-4, "{name}: plan differs from the reference by {max}");
    }
}

// ---------------------------------------------------------------------------------------------
// The layout shortcuts (vertical shorts: a blurred backdrop, cropped boxes, keyframed shakes)
// ---------------------------------------------------------------------------------------------

fn fx(id: &str, params: &[(&str, ParamValue)]) -> filmcraft_project::EffectInstance {
    let mut e = find_effect(id).expect("effect").instance();
    for (k, v) in params {
        e.params.get_mut(*k).unwrap_or_else(|| panic!("{id}: no param {k}")).value = v.clone();
    }
    e
}

/// One camera clip of `cw`×`ch` in a `w`×`h` sequence at Motion scale `scale` % (fit to the frame
/// first), with `effects` after its own.
fn camera_scene(w: usize, h: usize, cw: usize, ch: usize, scale: f64, effects: Vec<filmcraft_project::EffectInstance>) -> (Project, ItemId, SourceMap) {
    let (mut p, seq, map) = scene_sized(w, h, &[Spec { motion_scale: Some(scale), ..Spec::new(camera(cw, ch)) }]);
    if let Some(it) = p.sequence_mut(seq).and_then(|s| s.video_tracks.get_mut(0)).and_then(|t| t.items.get_mut(0)) {
        it.effects.extend(effects);
    }
    (p, seq, map)
}

fn clip(p: &Project, seq: ItemId) -> TrackItem {
    p.sequence(seq).and_then(|s| s.video_tracks.first()).and_then(|t| t.items.first()).cloned().expect("clip")
}

/// A Gaussian Blur of 0 changes nothing, and is no colour effect: in front of a clip's effects it
/// keeps every shortcut away (the reference for what the shortcuts must give).
fn no_shortcut() -> filmcraft_project::EffectInstance {
    fx("gaussian_blur", &[("blurriness", ParamValue::Float(0.0))])
}

fn shake_at_rest() -> filmcraft_project::EffectInstance {
    fx(
        "camera_shake",
        &[("amount", ParamValue::Float(0.0)), ("rotation", ParamValue::Float(0.0)), ("zoom", ParamValue::Float(0.0)), ("motion_blur", ParamValue::Float(40.0))],
    )
}

#[test]
fn a_cropped_clip_decodes_only_its_crop_and_gives_the_same_bits() {
    let rc = |l: f64, t: f64, r: f64, b: f64, radius: f64, feather: f64, border: f64| {
        fx(
            "rounded_crop",
            &[
                ("left", ParamValue::Float(l)),
                ("top", ParamValue::Float(t)),
                ("right", ParamValue::Float(r)),
                ("bottom", ParamValue::Float(b)),
                ("radius", ParamValue::Float(radius)),
                ("feather", ParamValue::Float(feather)),
                ("border", ParamValue::Float(border)),
            ],
        )
    };
    let crop = fx(
        "crop",
        &[("left", ParamValue::Float(15.0)), ("right", ParamValue::Float(25.0)), ("top", ParamValue::Float(5.0)), ("feather", ParamValue::Float(4.0))],
    );
    let cases: Vec<(&str, Vec<filmcraft_project::EffectInstance>)> = vec![
        ("rounded box", vec![shake_at_rest(), rc(20.0, 10.0, 30.0, 5.0, 6.0, 0.0, 0.0)]),
        ("feathered box with a border", vec![shake_at_rest(), rc(55.0, 40.0, 3.0, 0.0, 12.0, 3.0, 2.0)]),
        ("crop then a colour effect", vec![crop.clone(), fx("brightness_contrast", &[("brightness", ParamValue::Float(-20.0))])]),
        ("two crops", vec![crop, rc(10.0, 10.0, 10.0, 10.0, 0.0, 0.0, 0.0)]),
        ("a crop past the edges", vec![rc(-20.0, -5.0, 120.0, 0.0, 4.0, 0.0, 0.0)]),
    ];
    for (what, effects) in cases {
        for scale in [80.0, 130.0, 260.0] {
            let (p, seq, map) = camera_scene(W, H, W, H, scale, effects.clone());
            let mut with_ref = effects.clone();
            with_ref.insert(0, no_shortcut());
            let (pr, seqr, mapr) = camera_scene(W, H, W, H, scale, with_ref);
            assert!(crop_keep(&clip(&p, seq), Tick::ZERO).is_some() || what == "a crop past the edges", "{what}: the crop shortcut applies");
            assert!(crop_keep(&clip(&pr, seqr), Tick::ZERO).is_none());
            for f in [0, 9] {
                let t = FPS.tick_of(f);
                let got = render_sequence(&p, seq, t, RenderOptions::default(), &map);
                let want = render_sequence(&pr, seqr, t, RenderOptions::default(), &mapr);
                same_bits(&got, &want, &format!("{what} at {scale} %, frame {f}"));
            }
        }
    }
    // anything that moves pixels, or a masked effect, decodes the whole picture
    let mut shaking = shake_at_rest();
    shaking.params.get_mut("amount").expect("amount").value = ParamValue::Float(10.0);
    let (p, seq, _) = camera_scene(W, H, W, H, 100.0, vec![shaking, fx("crop", &[("left", ParamValue::Float(10.0))])]);
    assert!(crop_keep(&clip(&p, seq), Tick::ZERO).is_none());
    let (p, seq, _) = camera_scene(W, H, W, H, 100.0, vec![fx("crop", &[("left", ParamValue::Float(10.0)), ("zoom", ParamValue::Bool(true))])]);
    assert!(crop_keep(&clip(&p, seq), Tick::ZERO).is_none(), "a zooming crop moves pixels");
    let (p, seq, _) = camera_scene(W, H, W, H, 100.0, vec![fx("brightness_contrast", &[])]);
    assert!(crop_keep(&clip(&p, seq), Tick::ZERO).is_none(), "nothing cropped");
    let (p, seq, _) = camera_scene(W, H, W, H, 100.0, vec![fx("crop", &[("left", ParamValue::Float(f64::NAN))])]);
    assert!(crop_keep(&clip(&p, seq), Tick::ZERO).is_none(), "a NaN crop");
}

#[test]
fn a_camera_shake_at_rest_is_the_plain_picture() {
    for scale in [100.0, 150.0] {
        let (p, seq, map) = camera_scene(W, H, W, H, scale, vec![shake_at_rest()]);
        let (pr, seqr, mapr) = camera_scene(W, H, W, H, scale, vec![]);
        let t = FPS.tick_of(5);
        same_bits(
            &render_sequence(&p, seq, t, RenderOptions::default(), &map),
            &render_sequence(&pr, seqr, t, RenderOptions::default(), &mapr),
            &format!("shake at rest, {scale} %"),
        );
    }
    // a shake that moves still moves the picture
    let mut moving = shake_at_rest();
    moving.params.get_mut("amount").expect("amount").value = ParamValue::Float(12.0);
    let (p, seq, map) = camera_scene(W, H, W, H, 100.0, vec![moving]);
    let (pr, seqr, mapr) = camera_scene(W, H, W, H, 100.0, vec![]);
    let t = FPS.tick_of(5);
    assert_ne!(render_sequence(&p, seq, t, RenderOptions::default(), &map).px, render_sequence(&pr, seqr, t, RenderOptions::default(), &mapr).px);
}

#[test]
fn a_blurred_backdrop_renders_at_a_fraction_and_looks_the_same() {
    let (w, h) = (384, 216);
    let blur = |b: f64| fx("gaussian_blur", &[("blurriness", ParamValue::Float(b)), ("repeat_edge", ParamValue::Bool(true))]);
    let dark = fx("brightness_contrast", &[("brightness", ParamValue::Float(-30.0))]);
    // as the layout makes it: blurred, darkened, scaled to fill
    let (p, seq, map) = camera_scene(w, h, w, h, 180.0, vec![blur(60.0), dark.clone()]);
    let it = clip(&p, seq);
    assert_eq!(blurred_want(&it, Tick::ZERO, 1.0), 0.25, "sigma 30 px: a quarter of the size keeps it 7.5 px wide");
    // the same at full size (an effect after the blur that is not a colour effect)
    let (pr, seqr, mapr) = camera_scene(w, h, w, h, 180.0, vec![blur(60.0), dark.clone(), no_shortcut()]);
    assert_eq!(blurred_want(&clip(&pr, seqr), Tick::ZERO, 1.0), 1.0);
    let t = FPS.tick_of(3);
    let a = render_sequence(&p, seq, t, RenderOptions::default(), &map).over_black_rgba8();
    let b = render_sequence(&pr, seqr, t, RenderOptions::default(), &mapr).over_black_rgba8();
    let worst = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
    let mean = a.iter().zip(&b).map(|(x, y)| f64::from(x.abs_diff(*y))).sum::<f64>() / a.len() as f64;
    assert!(worst <= 3 && mean < 0.6, "8-bit difference: worst {worst}, mean {mean:.3}");
    // no shortcut for a weak blur, a one-way blur, a masked blur, or anything after it that needs detail
    let (p, seq, _) = camera_scene(w, h, w, h, 180.0, vec![blur(8.0)]);
    assert_eq!(blurred_want(&clip(&p, seq), Tick::ZERO, 1.0), 1.0);
    let mut one_way = blur(60.0);
    one_way.params.get_mut("dimensions").expect("dimensions").value = ParamValue::Choice(1);
    let (p, seq, _) = camera_scene(w, h, w, h, 180.0, vec![one_way]);
    assert_eq!(blurred_want(&clip(&p, seq), Tick::ZERO, 1.0), 1.0);
    let (p, seq, _) = camera_scene(w, h, w, h, 180.0, vec![blur(60.0), fx("rounded_crop", &[])]);
    assert_eq!(blurred_want(&clip(&p, seq), Tick::ZERO, 1.0), 1.0);
    let (p, seq, _) = camera_scene(w, h, w, h, 180.0, vec![dark, blur(60.0)]);
    assert_eq!(blurred_want(&clip(&p, seq), Tick::ZERO, 1.0), 1.0, "the blur must come first");
    let mut huge = blur(f64::INFINITY);
    huge.params.get_mut("blurriness").expect("b").value = ParamValue::Float(f64::NAN);
    let (p, seq, _) = camera_scene(w, h, w, h, 180.0, vec![huge]);
    assert_eq!(blurred_want(&clip(&p, seq), Tick::ZERO, 1.0), 1.0);
    // the deepest it goes is an eighth
    let (p, seq, _) = camera_scene(w, h, w, h, 180.0, vec![blur(2000.0)]);
    assert_eq!(blurred_want(&clip(&p, seq), Tick::ZERO, 1.0), 0.125);
}

/// A picture with a transparent margin and some pixels that carry light but no alpha.
fn islands(w: usize, h: usize) -> Image {
    let mut img = Image::new(w, h);
    for y in 7..h - 5 {
        for x in 4..w - 9 {
            let v = ((x * 31 + y * 17) % 97) as f32 / 97.0;
            img.px[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&[v * 0.8, v * 0.5, 0.3, if (x + y) % 11 == 0 { 0.0 } else { 0.9 }]);
        }
    }
    img
}

#[test]
fn a_region_resample_is_the_full_resample_cut_down() {
    let img = islands(60, 40);
    let (w, h) = (90usize, 70usize);
    let mats = [
        Affine::translate(10.0, 5.0),
        Affine::translate(3.3, -2.7).then_apply(&Affine::scale(1.37, 1.37)),
        Affine::translate(-12.0, 4.0).then_apply(&Affine::scale(0.62, 0.8)),
        Affine::translate(45.0, 35.0).then_apply(&Affine::rotate_deg(17.0)).then_apply(&Affine::translate(-30.0, -20.0)),
        Affine::translate(80.0, 60.0).then_apply(&Affine::scale(1.5, 1.5)),
        Affine::translate(-200.0, 0.0),
    ];
    for (k, m) in mats.iter().enumerate() {
        let full = img.transformed(w, h, m);
        let Some((part, x0, y0)) = img.transformed_region(w, h, m) else { panic!("matrix {k}: no region") };
        for y in 0..h {
            for x in 0..w {
                let inside = x >= x0 && x < x0 + part.w && y >= y0 && y < y0 + part.h;
                let f = full.get(x, y);
                if inside {
                    let p = part.get(x - x0, y - y0);
                    assert_eq!(f.map(f32::to_bits), p.map(f32::to_bits), "matrix {k}: pixel ({x}, {y})");
                } else {
                    assert_eq!(f, [0.0; 4], "matrix {k}: ({x}, {y}) is outside the region but not empty");
                }
            }
        }
    }
    // minifying: the full resample's mip path decides
    assert!(img.transformed_region(w, h, &Affine::scale(0.4, 0.4)).is_none());
    assert!(img.transformed_region(w, h, &Affine::scale(0.0, 1.0)).is_none());
    // nothing to draw
    let empty = Image::new(20, 20);
    assert_eq!(empty.content_bounds(), None);
    let (part, _, _) = empty.transformed_region(w, h, &Affine::translate(3.0, 3.0)).expect("region");
    assert_eq!((part.w, part.h), (0, 0));
    assert_eq!(img.content_bounds(), Some((4, 7, 51, 35)));
    assert_eq!(Image { w: 3, h: 3, px: vec![0.0; 5] }.content_bounds(), None, "a short buffer is empty, not a panic");
}

#[test]
fn over_nothing_is_normal_mixing_into_a_blank_frame() {
    let vals = [0.0f32, -0.0, 0.3, 1.0, 1.7, -0.4, f32::NAN, f32::INFINITY];
    let mut img = Image::new(vals.len() * vals.len(), 1);
    for (i, p) in img.px.chunks_exact_mut(4).enumerate() {
        let (a, c) = (vals[i % vals.len()], vals[i / vals.len()]);
        p.copy_from_slice(&[c, c * 0.5, 0.2, a]);
    }
    for opacity in [1.0f32, 0.6, 0.0] {
        let mut canvas = Image::new(img.w, 1);
        blend::composite(&mut canvas, &img, opacity, Blend::Normal);
        let mut mine = img.clone();
        blend::over_nothing(&mut mine, opacity);
        assert_eq!(canvas.px.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), mine.px.iter().map(|v| v.to_bits()).collect::<Vec<_>>(), "opacity {opacity}");
    }
}
