//! Small painting layer on top of GhostEngine's CPU primitives.
//! Only functions that GhostEngine exports are used here: no second renderer.

use crate::covers::CoverStore;
use ghostengine::rasterizer::fill_rect;
use ghostengine::{
    draw_mipmapped_texture_clipped, draw_rounded_rect_aa, draw_texture_region_clipped, TextureFilter, draw_text, fill_circle_aa, fill_polygon_aa,
    fill_rect_linear_gradient_parallel, fill_rounded_rect_aa, Color, FontFace, Framebuffer,
    Gradient, GradientStop, Point, Rect, Vector,
};

pub const BG: Color = Color::rgb(0x0E, 0x0C, 0x13);
pub const SIDE: Color = Color::rgb(0x12, 0x0F, 0x19);
pub const PANEL: Color = Color::rgb(0x17, 0x14, 0x1F);
pub const LINE: Color = Color::rgb(0x2A, 0x25, 0x36);
pub const TXT: Color = Color::rgb(0xF1, 0xEE, 0xF7);
pub const MUT: Color = Color::rgb(0xA4, 0x9C, 0xB8);
pub const SOFT: Color = Color::rgb(0xCF, 0xC7, 0xE0);
pub const PALE: Color = Color::rgb(0xDD, 0xD5, 0xEE);
pub const VIO: Color = Color::rgb(0xB9, 0xA2, 0xFF);
pub const VIO_DARK: Color = Color::rgb(0x19, 0x0F, 0x33);
pub const ACT: Color = Color::rgb(0x2A, 0x21, 0x42);
pub const ACT_TXT: Color = Color::rgb(0xD9, 0xCC, 0xFF);
pub const GREEN: Color = Color::rgb(0x5F, 0xDD, 0xB9);
pub const GREEN_DARK: Color = Color::rgb(0x0E, 0x2A, 0x22);
pub const AMBER: Color = Color::rgb(0xF5, 0xC2, 0x6B);
pub const AMBER_DARK: Color = Color::rgb(0x2A, 0x1B, 0x00);
pub const BTN_LINE: Color = Color::rgb(0x4A, 0x41, 0x60);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Store,
    Library,
    Product,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Go(Page),
    StoreFilter(usize),
    LibFilter(usize),
}

/// A clickable region recorded while a page is drawn (page coordinates).
#[derive(Clone, Copy)]
pub struct Hit {
    pub rect: Rect,
    pub radius: u32,
    pub action: Action,
    /// Fixed to the window (sidebar) instead of scrolling with the page.
    pub fixed: bool,
}

/// UI state that changes what the pages show.
#[derive(Clone, Copy, Default)]
pub struct State {
    pub store_filter: usize,
    pub lib_filter: usize,
}

pub fn hex(s: &str) -> Color {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).expect("hex color");
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

#[derive(Clone, Copy)]
pub enum W {
    Reg,
    Med,
    Semi,
    Bold,
    Disp,
}

pub struct Fonts {
    pub reg: FontFace,
    pub med: FontFace,
    pub semi: FontFace,
    pub bold: FontFace,
    pub disp: FontFace,
}

impl Fonts {
    pub fn load(dir: &str) -> Result<Self, String> {
        let load = |name: &str| -> Result<FontFace, String> {
            let path = format!("{dir}/{name}");
            let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
            FontFace::from_bytes(&bytes).map_err(|e| format!("{path}: {e}"))
        };
        Ok(Self {
            reg: load("Inter-Regular.otf")?,
            med: load("Inter-Medium.otf")?,
            semi: load("Inter-SemiBold.otf")?,
            bold: load("Inter-Bold.otf")?,
            disp: load("InterDisplay-Bold.otf")?,
        })
    }
    fn get(&self, w: W) -> &FontFace {
        match w {
            W::Reg => &self.reg,
            W::Med => &self.med,
            W::Semi => &self.semi,
            W::Bold => &self.bold,
            W::Disp => &self.disp,
        }
    }
}

pub fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect::new(x.max(0) as u32, y.max(0) as u32, w.max(0) as u32, h.max(0) as u32)
}

pub struct Painter<'a> {
    pub fb: Framebuffer,
    pub hits: Vec<Hit>,
    fonts: &'a Fonts,
    covers: Option<&'a CoverStore>,
    scratch: Framebuffer,
}

impl<'a> Painter<'a> {
    pub fn new(width: u32, height: u32, fonts: &'a Fonts) -> Self {
        Self {
            fb: Framebuffer::new(width, height, BG).expect("framebuffer"),
            hits: Vec::new(),
            fonts,
            covers: None,
            scratch: Framebuffer::new(1, 1, BG).expect("scratch"),
        }
    }

    pub fn with_covers(mut self, covers: &'a CoverStore) -> Self {
        self.covers = Some(covers);
        self
    }

    /// Draws a decoded cover image into the rectangle (mipmapped when the aspect
    /// matches, otherwise centre-cropped with bilinear sampling). False if no image.
    pub fn draw_cover(&mut self, title: &str, x: i32, y: i32, w: i32, h: i32) -> bool {
        let Some(store) = self.covers else { return false };
        let Some(mips) = store.get(title) else { return false };
        let base = &mips.levels()[0];
        let (bw, bh) = (base.width() as i64, base.height() as i64);
        let dest = rect(x, y, w, h);
        if ((w as i64 * bh) - (h as i64 * bw)).abs() * 20 <= w as i64 * bh {
            draw_mipmapped_texture_clipped(&mut self.fb, mips, dest, dest);
        } else {
            let (sw, sh) = if (bw * h as i64) > (bh * w as i64) { (bh * w as i64 / h as i64, bh) } else { (bw, bw * h as i64 / w as i64) };
            let src = Rect::new(((bw - sw) / 2) as u32, ((bh - sh) / 2) as u32, sw as u32, sh as u32);
            draw_texture_region_clipped(&mut self.fb, base, src, dest, TextureFilter::Bilinear, dest);
        }
        true
    }

    /// Darkens the lower part of a cover so white titles stay readable.
    pub fn scrim(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let band = h * 3 / 5;
        for row in 0..band {
            let t = row as f32 / band as f32;
            let alpha = (t * t * 215.0) as u8;
            self.fill(x, y + h - band + row, w, 1, Color::rgba(8, 6, 14, alpha));
        }
    }

    pub fn hit(&mut self, x: i32, y: i32, w: i32, h: i32, radius: i32, action: Action, fixed: bool) {
        self.hits.push(Hit { rect: rect(x, y, w, h), radius: radius.max(0) as u32, action, fixed });
    }

    // ---- shapes -------------------------------------------------------
    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: Color) {
        if c.a == 255 {
            fill_rect(&mut self.fb, rect(x, y, w, h), c);
        } else {
            let clip = rect(0, 0, self.fb.width() as i32, self.fb.height() as i32);
            ghostengine::blend_rect_clipped(&mut self.fb, rect(x, y, w, h), clip, c);
        }
    }

    pub fn rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: Color) {
        fill_rounded_rect_aa(&mut self.fb, rect(x, y, w, h), r.max(0) as u32, c);
    }

    pub fn rrect_line(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, c: Color) {
        draw_rounded_rect_aa(&mut self.fb, rect(x, y, w, h), r.max(0) as u32, c);
    }

    pub fn circle(&mut self, cx: i32, cy: i32, r: i32, c: Color) {
        fill_circle_aa(&mut self.fb, Point::new(cx, cy), r.max(0) as u32, c);
    }

    pub fn poly(&mut self, pts: &[(f32, f32)], c: Color) {
        let v: Vec<Point> = pts
            .iter()
            .map(|&(x, y)| Point::new(x.round() as i32, y.round() as i32))
            .collect();
        fill_polygon_aa(&mut self.fb, &v, c);
    }

    /// CSS-like linear gradient (`angle` in degrees, 180 = top to bottom).
    pub fn gradient(&mut self, x: i32, y: i32, w: i32, h: i32, angle: f32, stops: &[(f32, Color)]) {
        let a = angle.to_radians();
        let (dx, dy) = (a.sin(), -a.cos());
        let len = (w as f32 * dx).abs() + (h as f32 * dy).abs();
        let (cx, cy) = (x as f32 + w as f32 / 2.0, y as f32 + h as f32 / 2.0);
        let start = Vector { x: cx - dx * len / 2.0, y: cy - dy * len / 2.0 };
        let end = Vector { x: cx + dx * len / 2.0, y: cy + dy * len / 2.0 };
        let stops = stops
            .iter()
            .map(|&(position, color)| GradientStop { position, color })
            .collect();
        let g = Gradient::new(stops).expect("gradient");
        fill_rect_linear_gradient_parallel(&mut self.fb, rect(x, y, w, h), start, end, &g);
    }

    /// Rounds the four corners of an already painted rectangle by blending
    /// the surrounding background colour back over the outside of the arc.
    pub fn round_corners(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, bg: Color) {
        let rf = r as f32;
        for corner in 0..4 {
            let (ox, oy, cx, cy) = match corner {
                0 => (x, y, x + r, y + r),
                1 => (x + w - r, y, x + w - r, y + r),
                2 => (x, y + h - r, x + r, y + h - r),
                _ => (x + w - r, y + h - r, x + w - r, y + h - r),
            };
            for py in oy..oy + r {
                for px in ox..ox + r {
                    let mut inside = 0;
                    for sy in 0..4 {
                        for sx in 0..4 {
                            let fx = px as f32 + (sx as f32 + 0.5) / 4.0 - cx as f32;
                            let fy = py as f32 + (sy as f32 + 0.5) / 4.0 - cy as f32;
                            if fx * fx + fy * fy <= rf * rf {
                                inside += 1;
                            }
                        }
                    }
                    let outside = 16 - inside;
                    if outside == 0 || px < 0 || py < 0 {
                        continue;
                    }
                    if let Some(cur) = self.fb.get_pixel(px as u32, py as u32) {
                        let over = Color::rgba(bg.r, bg.g, bg.b, (outside * 255 / 16) as u8);
                        self.fb.set_pixel(px as u32, py as u32, over.blend_over(cur));
                    }
                }
            }
        }
    }

    // ---- text ---------------------------------------------------------
    pub fn width(&mut self, s: &str, size: f32, w: W) -> f32 {
        let font = self.fonts.get(w);
        draw_text(&mut self.scratch, font, s, Point::new(0, 0), size, Color::WHITE).width
    }

    /// Draws one line inside a line box of height `lh` starting at `y`.
    pub fn text(&mut self, x: i32, y: i32, lh: i32, s: &str, size: f32, w: W, c: Color) -> f32 {
        let baseline = y as f32 + (lh as f32 - 1.21 * size) / 2.0 + 0.969 * size;
        let font = self.fonts.get(w);
        draw_text(&mut self.fb, font, s, Point::new(x, baseline.round() as i32), size, c).width
    }

    pub fn text_right(&mut self, right: i32, y: i32, lh: i32, s: &str, size: f32, w: W, c: Color) {
        let tw = self.width(s, size, w);
        self.text(right - tw.round() as i32, y, lh, s, size, w, c);
    }

    pub fn wrap(&mut self, s: &str, size: f32, w: W, max_w: i32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut cur = String::new();
        for word in s.split_whitespace() {
            let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if !cur.is_empty() && self.width(&trial, size, w) > max_w as f32 {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            } else {
                cur = trial;
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    /// Draws wrapped text and returns its height.
    pub fn paragraph(&mut self, x: i32, y: i32, max_w: i32, lh: i32, s: &str, size: f32, w: W, c: Color, max_lines: usize) -> i32 {
        let lines = self.wrap(s, size, w, max_w);
        let n = lines.len().min(max_lines);
        for (i, line) in lines.iter().take(n).enumerate() {
            self.text(x, y + i as i32 * lh, lh, line, size, w, c);
        }
        n as i32 * lh
    }
}

/// Writes the top `height` rows of a framebuffer as an RGB PNG.
pub fn save_png(fb: &Framebuffer, height: u32, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let (w, h) = (fb.width(), height.min(fb.height()));
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for px in fb.pixels().iter().take((w * h) as usize) {
        rgb.extend_from_slice(&[px.r, px.g, px.b]);
    }
    let mut enc = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(path)?), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgb)?;
    Ok(())
}
