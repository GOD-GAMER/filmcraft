//! The compositor's working image: premultiplied linear-light RGBA f32, row-major.

use filmcraft_geom::Affine;
use rayon::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Image {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, px: vec![0.0; w * h * 4] }
    }
    pub fn filled(w: usize, h: usize, c: [f32; 4]) -> Self {
        let mut px = Vec::with_capacity(w * h * 4);
        for _ in 0..w * h {
            px.extend_from_slice(&c);
        }
        Self { w, h, px }
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> [f32; 4] {
        let i = (y * self.w + x) * 4;
        [self.px[i], self.px[i + 1], self.px[i + 2], self.px[i + 3]]
    }
    /// Clamp-to-edge fetch.
    #[inline]
    pub fn get_clamped(&self, x: isize, y: isize) -> [f32; 4] {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.get(x, y)
    }
    /// Transparent-border fetch.
    #[inline]
    pub fn get_or_clear(&self, x: isize, y: isize) -> [f32; 4] {
        if x < 0 || y < 0 || x >= self.w as isize || y >= self.h as isize { [0.0; 4] } else { self.get(x as usize, y as usize) }
    }
    /// Bilinear sample at continuous pixel coords (pixel centres at +0.5), transparent outside.
    #[inline]
    pub fn sample_bilinear(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = x - 0.5;
        let fy = y - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.get_or_clear(x0, y0);
        let b = self.get_or_clear(x0 + 1, y0);
        let c = self.get_or_clear(x0, y0 + 1);
        let d = self.get_or_clear(x0 + 1, y0 + 1);
        let mut o = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (d[k] - c[k]) * tx;
            o[k] = top + (bot - top) * ty;
        }
        o
    }
    #[inline]
    pub fn sample_bilinear_clamped(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = (x - 0.5).clamp(0.0, self.w as f32 - 1.0);
        let fy = (y - 0.5).clamp(0.0, self.h as f32 - 1.0);
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let (a, b, c, d) = (self.get(x0, y0), self.get(x1, y0), self.get(x0, y1), self.get(x1, y1));
        let mut o = [0.0; 4];
        for k in 0..4 {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bot = c[k] + (d[k] - c[k]) * tx;
            o[k] = top + (bot - top) * ty;
        }
        o
    }

    /// Resample `self` into a `w`×`h` image through `m` (maps source pixels → destination pixels).
    /// Uses bilinear filtering, with a box pre-filter (mip level) when minifying by more than 2×.
    pub fn transformed(&self, w: usize, h: usize, m: &Affine) -> Image {
        let Some(inv) = m.inverse() else { return Image::new(w, h) };
        // minification factor
        let sx = (m.a * m.a + m.b * m.b).sqrt();
        let sy = (m.c * m.c + m.d * m.d).sqrt();
        let minify = 1.0 / sx.min(sy).max(1e-6);
        if minify >= 2.0 && self.w >= 4 && self.h >= 4 {
            let half = self.downsample2();
            let m2 = m.then_apply(&Affine::scale(2.0, 2.0));
            return half.transformed(w, h, &m2);
        }
        let mut out = Image::new(w, h);
        // Destination bounds of the source rect, to skip empty rows/cols.
        let b = m.bounds(&filmcraft_geom::Rect::new(0.0, 0.0, self.w as f64, self.h as f64));
        let y0 = (b.y.floor().max(0.0) as usize).min(h);
        let y1 = (b.bottom().ceil().max(0.0) as usize).min(h);
        let x0 = (b.x.floor().max(0.0) as usize).min(w);
        let x1 = (b.right().ceil().max(0.0) as usize).min(w);
        let axis_aligned = m.b == 0.0 && m.c == 0.0;
        out.px.par_chunks_mut(w * 4).enumerate().skip(y0).take(y1.saturating_sub(y0)).for_each(|(y, row)| {
            let py = y as f64 + 0.5;
            for x in x0..x1 {
                let px = x as f64 + 0.5;
                let (u, v) =
                    if axis_aligned { (inv.a * px + inv.e, inv.d * py + inv.f) } else { (inv.a * px + inv.c * py + inv.e, inv.b * px + inv.d * py + inv.f) };
                if u < -1.0 || v < -1.0 || u > self.w as f64 + 1.0 || v > self.h as f64 + 1.0 {
                    continue;
                }
                let s = self.sample_bilinear(u as f32, v as f32);
                row[x * 4..x * 4 + 4].copy_from_slice(&s);
            }
        });
        out
    }

    /// [`Image::transformed`] for only the part of the `w`×`h` output that `self`'s picture lands
    /// on: the rectangle around its non-transparent pixels, through `m`, clipped to the output.
    /// Returns that part and its top-left corner; every pixel is exactly what `transformed` gives
    /// there, and every pixel outside it is transparent black in `transformed`'s result. `None`
    /// when `transformed` would minify (its mip path), or `m` has no inverse: use `transformed`.
    pub fn transformed_region(&self, w: usize, h: usize, m: &Affine) -> Option<(Image, usize, usize)> {
        let inv = m.inverse()?;
        let sx = (m.a * m.a + m.b * m.b).sqrt();
        let sy = (m.c * m.c + m.d * m.d).sqrt();
        let minify = 1.0 / sx.min(sy).max(1e-6);
        if !minify.is_finite() || minify >= 2.0 && self.w >= 4 && self.h >= 4 {
            return None;
        }
        let Some((cx0, cy0, cx1, cy1)) = self.content_bounds() else { return Some((Image::new(0, 0), 0, 0)) };
        // a bilinear sample is only non-zero within half a pixel of the content: one pixel of margin
        let content = filmcraft_geom::Rect::new(cx0 as f64 - 1.0, cy0 as f64 - 1.0, (cx1 - cx0) as f64 + 2.0, (cy1 - cy0) as f64 + 2.0);
        let whole = m.bounds(&filmcraft_geom::Rect::new(0.0, 0.0, self.w as f64, self.h as f64));
        let b = m.bounds(&content);
        let clip = |v: f64, n: usize| if v.is_finite() { (v.max(0.0) as usize).min(n) } else { 0 };
        let x0 = clip(b.x.floor().max(whole.x.floor()), w);
        let x1 = clip(b.right().ceil().min(whole.right().ceil()), w);
        let y0 = clip(b.y.floor().max(whole.y.floor()), h);
        let y1 = clip(b.bottom().ceil().min(whole.bottom().ceil()), h);
        let (rw, rh) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
        if rw == 0 || rh == 0 {
            return Some((Image::new(rw, rh), x0, y0));
        }
        // a recycled buffer: every pixel is written below (the skipped ones with transparent black)
        let mut out = Image { w: rw, h: rh, px: filmcraft_frame::pool::take_f32_overwritten(rw * rh * 4) };
        let axis_aligned = m.b == 0.0 && m.c == 0.0;
        if axis_aligned && self.px.len() >= self.w * self.h * 4 {
            // a sample's column depends on x only and its row on y only: work out each column's
            // and row's taps once (the same arithmetic as `sample_bilinear`, so the same pixels)
            let tap = |p: f64, n: usize| -> (bool, isize, f32) {
                let skip = p < -1.0 || p > n as f64 + 1.0;
                let f = p as f32 - 0.5;
                let f0 = f.floor();
                (skip, f0 as isize, f - f0)
            };
            let cols: Vec<(bool, isize, f32)> = (0..rw).map(|rx| tap(inv.a * ((x0 + rx) as f64 + 0.5) + inv.e, self.w)).collect();
            let (sw, sh) = (self.w as isize, self.h as isize);
            out.px.par_chunks_mut(rw * 4).enumerate().for_each(|(ry, row)| {
                let (skip_row, y0i, ty) = tap(inv.d * ((y0 + ry) as f64 + 0.5) + inv.f, self.h);
                if skip_row {
                    row.fill(0.0);
                    return;
                }
                let rows_in = y0i >= 0 && y0i + 1 < sh;
                for (rx, &(skip, x0i, tx)) in cols.iter().enumerate() {
                    if skip {
                        row[rx * 4..rx * 4 + 4].fill(0.0);
                        continue;
                    }
                    let i = if rows_in && x0i >= 0 && x0i + 1 < sw { (y0i as usize * self.w + x0i as usize) * 4 } else { usize::MAX };
                    let (a, b, c, d) = match (self.px.get(i..i.saturating_add(8)), self.px.get(i.saturating_add(self.w * 4)..i.saturating_add(self.w * 4 + 8)))
                    {
                        // all four taps inside: straight from the two rows
                        (Some(t), Some(u)) if i != usize::MAX => {
                            ([t[0], t[1], t[2], t[3]], [t[4], t[5], t[6], t[7]], [u[0], u[1], u[2], u[3]], [u[4], u[5], u[6], u[7]])
                        }
                        _ => {
                            (self.get_or_clear(x0i, y0i), self.get_or_clear(x0i + 1, y0i), self.get_or_clear(x0i, y0i + 1), self.get_or_clear(x0i + 1, y0i + 1))
                        }
                    };
                    let o = &mut row[rx * 4..rx * 4 + 4];
                    for k in 0..4 {
                        let top = a[k] + (b[k] - a[k]) * tx;
                        let bot = c[k] + (d[k] - c[k]) * tx;
                        o[k] = top + (bot - top) * ty;
                    }
                }
            });
            return Some((out, x0, y0));
        }
        out.px.par_chunks_mut(rw * 4).enumerate().for_each(|(ry, row)| {
            let py = (y0 + ry) as f64 + 0.5;
            for rx in 0..rw {
                let px = (x0 + rx) as f64 + 0.5;
                let (u, v) =
                    if axis_aligned { (inv.a * px + inv.e, inv.d * py + inv.f) } else { (inv.a * px + inv.c * py + inv.e, inv.b * px + inv.d * py + inv.f) };
                if u < -1.0 || v < -1.0 || u > self.w as f64 + 1.0 || v > self.h as f64 + 1.0 {
                    row[rx * 4..rx * 4 + 4].fill(0.0);
                    continue;
                }
                let s = self.sample_bilinear(u as f32, v as f32);
                row[rx * 4..rx * 4 + 4].copy_from_slice(&s);
            }
        });
        Some((out, x0, y0))
    }

    /// The smallest rectangle `(x0, y0, x1, y1)` (end exclusive) holding every pixel that is not
    /// transparent black (any channel non-zero: premultiplied light with no alpha counts). `None`
    /// for an image that is all transparent black.
    pub fn content_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let (w, h) = (self.w, self.h);
        if w == 0 || h == 0 || self.px.len() < w * h * 4 {
            return None;
        }
        let row_span = |y: usize| -> Option<(usize, usize)> {
            let row = self.px.get(y * w * 4..(y + 1) * w * 4)?;
            let first = row.chunks_exact(4).position(|p| p.iter().any(|v| *v != 0.0))?;
            let last = row.chunks_exact(4).rposition(|p| p.iter().any(|v| *v != 0.0))?;
            Some((first, last + 1))
        };
        let spans: Vec<Option<(usize, usize)>> = (0..h).into_par_iter().map(row_span).collect();
        let y0 = spans.iter().position(Option::is_some)?;
        let y1 = spans.iter().rposition(Option::is_some)? + 1;
        let (x0, x1) = spans.iter().flatten().fold((w, 0), |(a, b), (f, l)| (a.min(*f), b.max(*l)));
        Some((x0, y0, x1, y1))
    }

    /// Half-size box downsample.
    pub fn downsample2(&self) -> Image {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let mut out = Image::new(w, h);
        out.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let a = self.get_clamped(2 * x as isize, 2 * y as isize);
                let b = self.get_clamped(2 * x as isize + 1, 2 * y as isize);
                let c = self.get_clamped(2 * x as isize, 2 * y as isize + 1);
                let d = self.get_clamped(2 * x as isize + 1, 2 * y as isize + 1);
                for k in 0..4 {
                    row[x * 4 + k] = (a[k] + b[k] + c[k] + d[k]) * 0.25;
                }
            }
        });
        out
    }

    /// `self × (1 − w) + other × w` (same size; premultiplied, so this is a proper cross-fade).
    pub fn lerp(mut self, other: &Image, w: f32) -> Image {
        if other.w != self.w || other.h != self.h {
            return self;
        }
        for (a, b) in self.px.iter_mut().zip(&other.px) {
            *a += (*b - *a) * w;
        }
        self
    }

    pub fn scale_alpha(&mut self, a: f32) {
        if (a - 1.0).abs() < 1e-6 {
            return;
        }
        self.px.par_iter_mut().for_each(|v| *v *= a);
    }

    /// Straight-alpha colour of a pixel.
    #[inline]
    pub fn unpremul(p: [f32; 4]) -> [f32; 3] {
        if p[3] <= 1e-6 { [0.0; 3] } else { [p[0] / p[3], p[1] / p[3], p[2] / p[3]] }
    }

    /// Apply a per-pixel function on straight-alpha colour (alpha preserved).
    pub fn map_rgb(&mut self, f: impl Fn([f32; 3], usize, usize) -> [f32; 3] + Sync) {
        let w = self.w;
        self.px.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let p = &mut row[x * 4..x * 4 + 4];
                let a = p[3];
                if a <= 1e-6 {
                    continue;
                }
                let c = f([p[0] / a, p[1] / a, p[2] / a], x, y);
                p[0] = c[0] * a;
                p[1] = c[1] * a;
                p[2] = c[2] * a;
            }
        });
    }

    /// Convert to straight sRGB RGBA8.
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 4];
        out.par_chunks_mut(self.w * 4).zip(self.px.par_chunks(self.w * 4)).for_each(|(o, s)| filmcraft_frame::linear_premul_to_srgb8(s, o));
        out
    }

    /// Opaque version over black (for monitors).
    pub fn over_black_rgba8(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.w * self.h * 4];
        // `linear_to_srgb_u8`, with its table looked up once instead of per channel
        let table = filmcraft_color::linear_to_srgb_u8_table();
        let enc = |v: f32| table.get((v.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize).copied().unwrap_or(255);
        out.par_chunks_mut(self.w * 4).zip(self.px.par_chunks(self.w * 4)).for_each(|(o, s)| {
            for (o, s) in o.as_chunks_mut::<4>().0.iter_mut().zip(s.as_chunks::<4>().0) {
                o[0] = enc(s[0]);
                o[1] = enc(s[1]);
                o[2] = enc(s[2]);
                o[3] = 255;
            }
        });
        out
    }
}
