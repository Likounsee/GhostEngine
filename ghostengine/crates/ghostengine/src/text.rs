//! Font loading, basic text measurement and CPU glyph rasterization.
//!
//! Text layout and glyph rasterization for application interfaces.
//! Complex script shaping, fallback chains and persistent glyph caching remain
//! separate follow-up milestones.

use crate::{rasterizer::blend_pixel, Color, Framebuffer, Point, Rect};
use fontdue::Font;

const MAX_CACHED_GLYPHS_PER_FONT: usize = 4096;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A loaded TrueType/OpenType font backed by the fontdue parser and rasterizer.
pub struct FontFace {
    font: Font,
    glyph_cache: Mutex<HashMap<(u32, char), Arc<CachedGlyph>>>,
}

#[derive(Debug)]
struct CachedGlyph {
    xmin: i32,
    ymin: i32,
    width: usize,
    height: usize,
    advance_width: f32,
    bitmap: Arc<[u8]>,
}

impl FontFace {
    /// Loads a font from in-memory TTF/OTF bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        Font::from_bytes(bytes, fontdue::FontSettings::default()).map(|font| Self {
            font,
            glyph_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Returns whether this face contains a glyph for the character.
    #[must_use]
    pub fn has_glyph(&self, character: char) -> bool {
        self.font.lookup_glyph_index(character) != 0
    }

    /// Returns the horizontal kerning adjustment for an adjacent character pair.
    /// Positive values increase spacing; negative values bring glyphs closer.
    #[must_use]
    pub fn kerning(&self, left: char, right: char, pixel_size: f32) -> f32 {
        if !pixel_size.is_finite() || pixel_size <= 0.0 {
            return 0.0;
        }
        self.font
            .horizontal_kern(left, right, pixel_size)
            .unwrap_or(0.0)
    }

    /// Returns the recommended baseline-to-baseline distance at this size.
    #[must_use]
    pub fn line_height(&self, pixel_size: f32) -> f32 {
        self.font
            .horizontal_line_metrics(pixel_size)
            .map_or(pixel_size.max(1.0), |metrics| metrics.new_line_size)
    }

    /// Measures a UTF-8 string using advances and four-space tab stops.
    #[must_use]
    pub fn measure(&self, text: &str, pixel_size: f32) -> TextMetrics {
        layout_metrics(self, text, pixel_size)
    }

    /// Number of distinct glyph/size pairs currently retained by this font.
    #[must_use]
    pub fn cached_glyph_count(&self) -> usize {
        self.glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Releases cached glyph bitmaps. They are regenerated transparently on demand.
    pub fn clear_glyph_cache(&self) {
        self.glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn glyph(&self, character: char, pixel_size: f32) -> Arc<CachedGlyph> {
        let key = (pixel_size.to_bits(), character);
        let mut cache = self
            .glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(glyph) = cache.get(&key) {
            return Arc::clone(glyph);
        }
        if cache.len() >= MAX_CACHED_GLYPHS_PER_FONT {
            // Bound memory when applications animate font sizes or render arbitrary text.
            cache.clear();
        }
        let (metrics, bitmap) = self.font.rasterize(character, pixel_size);
        let glyph = Arc::new(CachedGlyph {
            xmin: metrics.xmin,
            ymin: metrics.ymin,
            width: metrics.width,
            height: metrics.height,
            advance_width: metrics.advance_width,
            bitmap: Arc::from(bitmap),
        });
        cache.insert(key, Arc::clone(&glyph));
        glyph
    }
}

/// Ordered font fallback chain used when a primary face lacks a character.
#[derive(Clone, Default)]
pub struct FontFamily {
    faces: Vec<Arc<FontFace>>,
}

impl FontFamily {
    /// Creates a family with one primary font.
    #[must_use]
    pub fn new(primary: Arc<FontFace>) -> Self {
        Self {
            faces: vec![primary],
        }
    }

    /// Appends a fallback face. Duplicate face pointers are ignored.
    pub fn add_fallback(&mut self, fallback: Arc<FontFace>) {
        if !self.faces.iter().any(|face| Arc::ptr_eq(face, &fallback)) {
            self.faces.push(fallback);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.faces.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// Maximum natural line height across all faces in the family.
    #[must_use]
    pub fn line_height(&self, pixel_size: f32) -> f32 {
        self.faces
            .iter()
            .map(|face| face.line_height(pixel_size))
            .fold(0.0_f32, f32::max)
    }

    #[must_use]
    pub fn primary(&self) -> Option<&FontFace> {
        self.faces.first().map(Arc::as_ref)
    }

    /// Selects the first face containing a glyph; otherwise uses the primary face.
    #[must_use]
    pub fn font_for(&self, character: char) -> Option<&FontFace> {
        self.faces
            .iter()
            .find(|face| face.has_glyph(character))
            .map(Arc::as_ref)
            .or_else(|| self.primary())
    }

    #[must_use]
    pub fn measure(&self, text: &str, pixel_size: f32) -> TextMetrics {
        measure_with_fallback(self, text, pixel_size)
    }
}

/// Measures text while selecting fallback faces character by character.
pub fn measure_with_fallback(family: &FontFamily, text: &str, pixel_size: f32) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() {
        return TextMetrics::default();
    }
    let Some(primary) = family.primary() else {
        return TextMetrics::default();
    };
    let mut width = 0.0_f32;
    let mut max_width = 0.0_f32;
    let mut lines = 1_u32;
    let mut line_height = family.line_height(pixel_size);
    let mut previous: Option<(char, usize)> = None;
    let space = family
        .font_for(' ')
        .unwrap_or(primary)
        .glyph(' ', pixel_size);
    let tab_width = (space.advance_width * 4.0).max(pixel_size);
    for character in text.chars() {
        match character {
            '\n' => {
                max_width = max_width.max(width);
                width = 0.0;
                lines += 1;
                previous = None;
            }
            '\t' => {
                width += tab_width;
                previous = None;
            }
            character => {
                if is_combining_mark(character) {
                    continue;
                }
                let face = family.font_for(character).unwrap_or(primary);
                let face_index = family
                    .faces
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate.as_ref(), face))
                    .unwrap_or(0);
                if let Some((previous_character, previous_face)) = previous {
                    if previous_face == face_index {
                        width += face.kerning(previous_character, character, pixel_size);
                    }
                }
                line_height = line_height.max(face.line_height(pixel_size));
                width += face.glyph(character, pixel_size).advance_width;
                previous = Some((character, face_index));
            }
        }
    }
    max_width = max_width.max(width);
    TextMetrics {
        width: max_width,
        height: line_height * lines as f32,
        line_count: lines,
    }
}

/// Draws text with a fallback chain. Glyphs remain unshaped and use simple advances.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_with_fallback_clipped(
    target: &mut Framebuffer,
    family: &FontFamily,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() {
        return TextMetrics::default();
    }
    let Some(primary) = family.primary() else {
        return TextMetrics::default();
    };
    let metrics = measure_with_fallback(family, text, pixel_size);
    let mut x = baseline.x as f32;
    let mut y = baseline.y;
    let mut previous: Option<(char, usize)> = None;
    let mut base_anchor: Option<(f32, f32, usize, usize)> = None;
    let space = family
        .font_for(' ')
        .unwrap_or(primary)
        .glyph(' ', pixel_size);
    let tab_width = (space.advance_width * 4.0).max(pixel_size);
    for character in text.chars() {
        match character {
            '\n' => {
                x = baseline.x as f32;
                previous = None;
                base_anchor = None;
                let next_y = y as f32 + family.line_height(pixel_size);
                if !next_y.is_finite() || next_y < i32::MIN as f32 || next_y > i32::MAX as f32 {
                    break;
                }
                y = next_y.round() as i32;
            }
            '\t' => {
                x += tab_width;
                previous = None;
                base_anchor = None;
            }
            character => {
                let face = family.font_for(character).unwrap_or(primary);
                let face_index = family
                    .faces
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate.as_ref(), face))
                    .unwrap_or(0);
                let glyph = face.glyph(character, pixel_size);
                let combining = is_combining_mark(character);
                if !combining {
                    if let Some((previous_character, previous_face)) = previous {
                        if previous_face == face_index {
                            x += face.kerning(previous_character, character, pixel_size);
                        }
                    }
                }
                let mut glyph_left = x + glyph.xmin as f32;
                let mut glyph_top = y as f32 - glyph.ymin as f32 - glyph.height as f32;
                if combining {
                    if let Some(anchor) = base_anchor {
                        (glyph_left, glyph_top) =
                            positioned_mark_origin(character, glyph.width, glyph.height, anchor);
                    }
                } else {
                    base_anchor = Some((glyph_left, glyph_top, glyph.width, glyph.height));
                }
                let draw_x = glyph_left - glyph.xmin as f32;
                let draw_y = glyph_top + glyph.ymin as f32 + glyph.height as f32;
                if draw_x.is_finite()
                    && draw_y.is_finite()
                    && draw_x >= i32::MIN as f32
                    && draw_x <= i32::MAX as f32
                    && draw_y >= i32::MIN as f32
                    && draw_y <= i32::MAX as f32
                {
                    draw_text_clipped(
                        target,
                        face,
                        &character.to_string(),
                        Point::new(draw_x.round() as i32, draw_y.round() as i32),
                        pixel_size,
                        color,
                        clip,
                    );
                }
                if !combining {
                    x += glyph.advance_width;
                    previous = Some((character, face_index));
                }
            }
        }
    }
    metrics
}

/// Horizontal alignment used for each line in a constrained text run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Layout constraints for application text blocks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextLayoutOptions {
    /// Optional maximum line width. Long words are split when necessary.
    pub max_width: Option<f32>,
    pub align: TextAlign,
    /// Multiplier applied to the font's natural line height.
    pub line_spacing: f32,
}

impl Default for TextLayoutOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            align: TextAlign::Left,
            line_spacing: 1.0,
        }
    }
}

/// Logical size of a laid-out text run, in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    pub line_count: u32,
}

/// Draws a wrapped and aligned text block with a framebuffer-space clip.
///
/// `baseline` is the baseline of the first line. `max_width` controls word
/// wrapping and the alignment box. Newlines are always respected; words wider than the
/// box are split at character boundaries. The returned width is the widest rendered line.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_box_clipped(
    target: &mut Framebuffer,
    font: &FontFace,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
    options: TextLayoutOptions,
) -> TextMetrics {
    if !pixel_size.is_finite()
        || pixel_size <= 0.0
        || text.is_empty()
        || !options.line_spacing.is_finite()
        || options.line_spacing <= 0.0
        || options
            .max_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
    {
        return TextMetrics::default();
    }

    let lines = layout_lines(font, text, pixel_size, options.max_width);
    if lines.is_empty() {
        return TextMetrics::default();
    }
    let line_height = font.line_height(pixel_size) * options.line_spacing;
    let box_width = options.max_width.unwrap_or_else(|| {
        lines
            .iter()
            .map(|line| font.measure(line, pixel_size).width)
            .fold(0.0_f32, f32::max)
    });
    let mut max_width = 0.0_f32;
    for (index, line) in lines.iter().enumerate() {
        let width = font.measure(line, pixel_size).width;
        max_width = max_width.max(width);
        let offset = match options.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => ((box_width - width) * 0.5).max(0.0),
            TextAlign::Right => (box_width - width).max(0.0),
        };
        let y = baseline.y as f32 + index as f32 * line_height;
        if !y.is_finite() || y < i32::MIN as f32 || y > i32::MAX as f32 {
            continue;
        }
        let x = baseline.x as f32 + offset;
        if !x.is_finite() || x < i32::MIN as f32 || x > i32::MAX as f32 {
            continue;
        }
        draw_text_clipped(
            target,
            font,
            line,
            Point::new(x.round() as i32, y.round() as i32),
            pixel_size,
            color,
            clip,
        );
    }

    TextMetrics {
        width: max_width,
        height: line_height * lines.len() as f32,
        line_count: lines.len() as u32,
    }
}

fn layout_lines(
    font: &FontFace,
    text: &str,
    pixel_size: f32,
    max_width: Option<f32>,
) -> Vec<String> {
    let Some(max_width) = max_width else {
        return text.split('\n').map(str::to_owned).collect();
    };
    text.split('\n')
        .flat_map(|paragraph| {
            wrap_paragraph(paragraph, max_width, |part| {
                measure_word(font, part, pixel_size)
            })
        })
        .collect()
}

fn wrap_paragraph<F>(paragraph: &str, max_width: f32, mut measure: F) -> Vec<String>
where
    F: FnMut(&str) -> f32,
{
    let mut output = Vec::new();
    let mut line = String::new();
    let mut line_width = 0.0_f32;
    for word in paragraph.split_whitespace() {
        let word_width = measure(word);
        let separator_width = if line.is_empty() { 0.0 } else { measure(" ") };
        if !line.is_empty() && line_width + separator_width + word_width > max_width {
            output.push(std::mem::take(&mut line));
            line_width = 0.0;
        }
        if word_width <= max_width {
            if !line.is_empty() {
                line.push(' ');
                line_width += separator_width;
            }
            line.push_str(word);
            line_width += word_width;
        } else {
            for character in word.chars() {
                let character_text = character.to_string();
                let advance = measure(&character_text);
                if !line.is_empty() && line_width + advance > max_width {
                    output.push(std::mem::take(&mut line));
                    line_width = 0.0;
                }
                line.push(character);
                line_width += advance;
            }
        }
    }
    output.push(line);
    output
}

fn measure_word(font: &FontFace, word: &str, pixel_size: f32) -> f32 {
    let mut previous = None;
    let mut width = 0.0;
    for character in word.chars() {
        if let Some(previous_character) = previous {
            width += font.kerning(previous_character, character, pixel_size);
        }
        width += font.glyph(character, pixel_size).advance_width;
        previous = Some(character);
    }
    width
}

/// Draws text with the supplied baseline origin and returns its measured bounds.
/// Glyph coverage is composited with source-over alpha and clipped by the target.
pub fn draw_text(
    target: &mut Framebuffer,
    font: &FontFace,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
) -> TextMetrics {
    let target_bounds = Rect::new(0, 0, target.width(), target.height());
    draw_text_clipped(
        target,
        font,
        text,
        baseline,
        pixel_size,
        color,
        target_bounds,
    )
}

/// Draws text while restricting glyph pixels to a framebuffer-coordinate clip rectangle.
///
/// Text measurement is independent of clipping, so the returned metrics describe the
/// complete run even when only a small part of it is visible.
pub fn draw_text_clipped(
    target: &mut Framebuffer,
    font: &FontFace,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() {
        return TextMetrics::default();
    }

    let visible_clip = clip.intersection(Rect::new(0, 0, target.width(), target.height()));
    let line_height = font.line_height(pixel_size);
    let mut pen_x = baseline.x as f32;
    let mut pen_y = baseline.y as f32;
    let origin_x = pen_x;
    let mut previous = None;
    let mut base_anchor: Option<(f32, f32, usize, usize)> = None;
    let mut max_width: f32 = 0.0;
    let mut line_count = 1_u32;
    let space_metrics = font.glyph(' ', pixel_size);
    let tab_width = (space_metrics.advance_width * 4.0).max(pixel_size);

    for character in text.chars() {
        if character == '\n' {
            max_width = max_width.max(pen_x - origin_x);
            pen_x = origin_x;
            pen_y += line_height;
            line_count += 1;
            previous = None;
            base_anchor = None;
            continue;
        }
        if character == '\t' {
            pen_x += tab_width;
            previous = None;
            base_anchor = None;
            continue;
        }

        if let Some(previous_character) = previous {
            pen_x += font.kerning(previous_character, character, pixel_size);
        }
        let glyph = font.glyph(character, pixel_size);
        let mut glyph_left = pen_x + glyph.xmin as f32;
        let mut glyph_top = pen_y - glyph.ymin as f32 - glyph.height as f32;
        let combining = is_combining_mark(character);
        if combining {
            if let Some(anchor) = base_anchor {
                (glyph_left, glyph_top) =
                    positioned_mark_origin(character, glyph.width, glyph.height, anchor);
            }
        } else {
            base_anchor = Some((glyph_left, glyph_top, glyph.width, glyph.height));
        }
        for row in 0..glyph.height {
            for column in 0..glyph.width {
                let coverage = glyph.bitmap[row * glyph.width + column];
                if coverage == 0 {
                    continue;
                }
                let x = glyph_left + column as f32;
                let y = glyph_top + row as f32;
                if x < 0.0 || y < 0.0 || x >= target.width() as f32 || y >= target.height() as f32 {
                    continue;
                }
                let pixel = Point::new(x.floor() as i32, y.floor() as i32);
                if !visible_clip.is_some_and(|bounds| bounds.contains(pixel)) {
                    continue;
                }
                let alpha = ((u16::from(color.a) * u16::from(coverage) + 127) / 255) as u8;
                if alpha > 0 {
                    blend_pixel(target, pixel, Color::rgba(color.r, color.g, color.b, alpha));
                }
            }
        }
        if !combining {
            pen_x += glyph.advance_width;
            previous = Some(character);
        }
    }

    max_width = max_width.max(pen_x - origin_x);
    TextMetrics {
        width: max_width,
        height: line_height * line_count as f32,
        line_count,
    }
}

/// Draws a text run with a full affine transform applied to every glyph.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_transformed_clipped(
    target: &mut Framebuffer,
    font: &FontFace,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
    transform: crate::Transform2D,
) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() || !transform.is_finite() {
        return TextMetrics::default();
    }
    let Some(inverse) = transform.inverse() else {
        return TextMetrics::default();
    };
    let metrics = layout_metrics(font, text, pixel_size);
    let Some(visible_clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height()))
    else {
        return metrics;
    };
    let line_height = font.line_height(pixel_size);
    let mut pen_x = baseline.x as f32;
    let mut pen_y = baseline.y as f32;
    let origin_x = pen_x;
    let mut previous = None;
    let mut base_anchor: Option<(f32, f32, usize, usize)> = None;
    let tab_width = (font.glyph(' ', pixel_size).advance_width * 4.0).max(pixel_size);
    for character in text.chars() {
        match character {
            '\n' => {
                pen_x = origin_x;
                pen_y += line_height;
                previous = None;
                base_anchor = None;
            }
            '\t' => {
                pen_x += tab_width;
                previous = None;
                base_anchor = None;
            }
            character => {
                if let Some(previous_character) = previous {
                    pen_x += font.kerning(previous_character, character, pixel_size);
                }
                let glyph = font.glyph(character, pixel_size);
                let mut left = pen_x + glyph.xmin as f32;
                let mut top = pen_y - glyph.ymin as f32 - glyph.height as f32;
                let combining = is_combining_mark(character);
                if combining {
                    if let Some(anchor) = base_anchor {
                        (left, top) =
                            positioned_mark_origin(character, glyph.width, glyph.height, anchor);
                    }
                } else {
                    base_anchor = Some((left, top, glyph.width, glyph.height));
                }
                draw_affine_glyph(
                    target,
                    &glyph,
                    left,
                    top,
                    color,
                    visible_clip,
                    transform,
                    inverse,
                );
                if !combining {
                    pen_x += glyph.advance_width;
                    previous = Some(character);
                }
            }
        }
    }
    metrics
}

#[allow(clippy::too_many_arguments)]
fn draw_affine_glyph(
    target: &mut Framebuffer,
    glyph: &CachedGlyph,
    left: f32,
    top: f32,
    color: Color,
    clip: Rect,
    transform: crate::Transform2D,
    inverse: crate::Transform2D,
) {
    if glyph.width == 0 || glyph.height == 0 {
        return;
    }
    let corners = [
        crate::Vector::new(left, top),
        crate::Vector::new(left + glyph.width as f32, top),
        crate::Vector::new(left, top + glyph.height as f32),
        crate::Vector::new(left + glyph.width as f32, top + glyph.height as f32),
    ]
    .map(|point| transform.transform_point(point));
    if corners
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return;
    }
    let min_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min)
        .floor();
    let max_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil();
    let min_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min)
        .floor();
    let max_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil();
    if min_x < i32::MIN as f32
        || max_x > i32::MAX as f32
        || min_y < i32::MIN as f32
        || max_y > i32::MAX as f32
    {
        return;
    }
    let x0 = (min_x as i32).max(clip.x as i32);
    let y0 = (min_y as i32).max(clip.y as i32);
    let x1 = (max_x as i32).min(clip.x.saturating_add(clip.width).min(i32::MAX as u32) as i32);
    let y1 = (max_y as i32).min(clip.y.saturating_add(clip.height).min(i32::MAX as u32) as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let point = inverse.transform_point(crate::Vector::new(x as f32 + 0.5, y as f32 + 0.5));
            let sx = point.x - left - 0.5;
            let sy = point.y - top - 0.5;
            if !sx.is_finite() || !sy.is_finite() {
                continue;
            }
            let ix = sx.floor() as i32;
            let iy = sy.floor() as i32;
            let fx = sx - ix as f32;
            let fy = sy - iy as f32;
            let sample = |px: i32, py: i32| -> f32 {
                if px < 0 || py < 0 || px >= glyph.width as i32 || py >= glyph.height as i32 {
                    0.0
                } else {
                    f32::from(glyph.bitmap[py as usize * glyph.width + px as usize])
                }
            };
            let top_coverage = sample(ix, iy) * (1.0 - fx) + sample(ix + 1, iy) * fx;
            let bottom_coverage = sample(ix, iy + 1) * (1.0 - fx) + sample(ix + 1, iy + 1) * fx;
            let coverage = (top_coverage * (1.0 - fy) + bottom_coverage * fy)
                .round()
                .clamp(0.0, 255.0) as u8;
            let alpha = ((u16::from(color.a) * u16::from(coverage) + 127) / 255) as u8;
            if alpha > 0 {
                blend_pixel(
                    target,
                    Point::new(x, y),
                    Color::rgba(color.r, color.g, color.b, alpha),
                );
            }
        }
    }
}

/// Draws a wrapped text block with the same affine transform applied to every glyph.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_box_transformed_clipped(
    target: &mut Framebuffer,
    font: &FontFace,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
    options: TextLayoutOptions,
    transform: crate::Transform2D,
) -> TextMetrics {
    if !pixel_size.is_finite()
        || pixel_size <= 0.0
        || text.is_empty()
        || !options.line_spacing.is_finite()
        || options.line_spacing <= 0.0
        || options
            .max_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
    {
        return TextMetrics::default();
    }
    let lines = layout_lines(font, text, pixel_size, options.max_width);
    let line_height = font.line_height(pixel_size) * options.line_spacing;
    let box_width = options.max_width.unwrap_or_else(|| {
        lines
            .iter()
            .map(|line| font.measure(line, pixel_size).width)
            .fold(0.0_f32, f32::max)
    });
    let mut max_width = 0.0_f32;
    for (index, line) in lines.iter().enumerate() {
        let width = font.measure(line, pixel_size).width;
        max_width = max_width.max(width);
        let offset = match options.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => ((box_width - width) * 0.5).max(0.0),
            TextAlign::Right => (box_width - width).max(0.0),
        };
        let x = baseline.x as f32 + offset;
        let y = baseline.y as f32 + index as f32 * line_height;
        if x.is_finite()
            && y.is_finite()
            && x >= i32::MIN as f32
            && x <= i32::MAX as f32
            && y >= i32::MIN as f32
            && y <= i32::MAX as f32
        {
            draw_text_transformed_clipped(
                target,
                font,
                line,
                Point::new(x.round() as i32, y.round() as i32),
                pixel_size,
                color,
                clip,
                transform,
            );
        }
    }
    TextMetrics {
        width: max_width,
        height: line_height * lines.len() as f32,
        line_count: lines.len() as u32,
    }
}


/// Returns whether a Unicode scalar is a combining mark in common scripts.
/// This is a positioning aid, not a full Unicode grapheme or shaping algorithm.
fn is_combining_mark(character: char) -> bool {
    matches!(character as u32,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x05BF
        | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670
        | 0x06D6..=0x06ED | 0x0711 | 0x0730..=0x074A
        | 0x07A6..=0x07B0 | 0x07EB..=0x07F3
        | 0x0816..=0x082D | 0x0859..=0x085B | 0x08D3..=0x0903
        | 0x093A..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963
        | 0x0981..=0x0983 | 0x09BC..=0x09CD | 0x09D7
        | 0x0A01..=0x0A03 | 0x0A3C..=0x0A4D | 0x0A70..=0x0A71
        | 0x0ABC..=0x0ACD | 0x0B01..=0x0B03 | 0x0B3C..=0x0B4D
        | 0x0C00..=0x0C04 | 0x0C3E..=0x0C56 | 0x0D00..=0x0D03
        | 0x0D3B..=0x0D4D | 0x0E31..=0x0E4E | 0x0F18..=0x0FBC
        | 0x102B..=0x103E | 0x135D..=0x135F | 0x1712..=0x1715
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF
        | 0xFE20..=0xFE2F | 0xE0100..=0xE01EF)
}

/// Marks that normally attach below their base glyph (cedilla/ogonek families).
fn is_below_combining_mark(character: char) -> bool {
    matches!(character as u32,
        0x0316..=0x0333 | 0x0339..=0x033C | 0x0345 | 0x0327..=0x0328
        | 0x05B2..=0x05B5 | 0x05B7 | 0x05B9..=0x05BC | 0x05C5
        | 0x064D..=0x0650 | 0x0656..=0x065F | 0x093C | 0x09BC
        | 0x0A3C | 0x0ABC | 0x0B3C | 0x0C3C | 0x0CBC)
}

fn positioned_mark_origin(
    character: char,
    mark_width: usize,
    mark_height: usize,
    base: (f32, f32, usize, usize),
) -> (f32, f32) {
    let (base_left, base_top, base_width, base_height) = base;
    let left = base_left + (base_width as f32 - mark_width as f32) * 0.5;
    let top = if is_below_combining_mark(character) {
        base_top + base_height as f32 - 1.0
    } else {
        base_top - mark_height as f32 + 1.0
    };
    (left, top)
}

fn layout_metrics(font: &FontFace, text: &str, pixel_size: f32) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() {
        return TextMetrics::default();
    }
    let space = font.glyph(' ', pixel_size);
    let tab_width = (space.advance_width * 4.0).max(pixel_size);
    let mut line_width = 0.0_f32;
    let mut max_width = 0.0_f32;
    let mut line_count = 1_u32;
    let mut previous = None;
    for character in text.chars() {
        match character {
            '\n' => {
                max_width = max_width.max(line_width);
                line_width = 0.0;
                line_count += 1;
                previous = None;
            }
            '\t' => {
                line_width += tab_width;
                previous = None;
            }
            character => {
                if !is_combining_mark(character) {
                    if let Some(previous_character) = previous {
                        line_width += font.kerning(previous_character, character, pixel_size);
                    }
                    line_width += font.glyph(character, pixel_size).advance_width;
                    previous = Some(character);
                }
            }
        }
    }
    max_width = max_width.max(line_width);
    TextMetrics {
        width: max_width,
        height: font.line_height(pixel_size) * line_count as f32,
        line_count,
    }
}

#[cfg(test)]
mod tests {
    use super::FontFace;

    // A compact embedded test font is deliberately avoided; loading behavior is
    // exercised by callers with their own licensed TTF/OTF asset.
    #[test]
    fn combining_mark_ranges_cover_common_latin_and_hebrew_marks() {
        assert!(super::is_combining_mark('\\u{0301}'));
        assert!(super::is_combining_mark('\\u{0327}'));
        assert!(super::is_combining_mark('\\u{05B0}'));
        assert!(super::is_combining_mark('\\u{20DD}'));
        assert!(!super::is_combining_mark('A'));
        assert!(!super::is_combining_mark('é'));
    }

    #[test]
    fn combining_mark_position_is_centered_over_base() {
        let origin = super::positioned_mark_origin(
            '\\u{0301}',
            3,
            2,
            (10.0, 20.0, 8, 12),
        );
        assert_eq!(origin, (12.5, 19.0));
        let below = super::positioned_mark_origin(
            '\\u{0327}',
            3,
            2,
            (10.0, 20.0, 8, 12),
        );
        assert_eq!(below, (12.5, 31.0));
    }

    #[test]
    fn affine_glyph_renderer_preserves_identity_coverage() {
        let glyph = super::CachedGlyph {
            xmin: 0,
            ymin: 1,
            width: 2,
            height: 1,
            advance_width: 2.0,
            bitmap: std::sync::Arc::from([255_u8, 128_u8]),
        };
        let mut target = crate::Framebuffer::new(5, 4, crate::Color::BLACK).unwrap();
        super::draw_affine_glyph(
            &mut target,
            &glyph,
            1.0,
            1.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 5, 4),
            crate::Transform2D::IDENTITY,
            crate::Transform2D::IDENTITY,
        );
        assert_eq!(target.get_pixel(1, 1), Some(crate::Color::WHITE));
        assert_ne!(target.get_pixel(2, 1), Some(crate::Color::BLACK));
        assert_eq!(target.get_pixel(3, 1), Some(crate::Color::BLACK));
    }

    #[test]
    fn affine_glyph_renderer_rotates_glyph_bitmap() {
        let glyph = super::CachedGlyph {
            xmin: 0,
            ymin: 1,
            width: 1,
            height: 1,
            advance_width: 1.0,
            bitmap: std::sync::Arc::from([255_u8]),
        };
        let transform = crate::Transform2D::rotation(std::f32::consts::FRAC_PI_2)
            .then(crate::Transform2D::translation(4.0, 1.0));
        let mut target = crate::Framebuffer::new(6, 6, crate::Color::BLACK).unwrap();
        super::draw_affine_glyph(
            &mut target,
            &glyph,
            1.0,
            1.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 6, 6),
            transform,
            transform.inverse().unwrap(),
        );
        assert_eq!(target.get_pixel(2, 2), Some(crate::Color::WHITE));
    }

    #[test]
    fn empty_font_family_is_safe_and_reports_empty_metrics() {
        let family = super::FontFamily::default();
        assert!(family.is_empty());
        assert_eq!(family.len(), 0);
        assert!(family.primary().is_none());
        assert!(family.font_for('A').is_none());
        assert_eq!(family.measure("Hello", 16.0), super::TextMetrics::default());
        assert_eq!(
            super::measure_with_fallback(&family, "Hello", 16.0),
            super::TextMetrics::default()
        );
    }

    #[test]
    fn fallback_measurement_rejects_invalid_sizes() {
        let family = super::FontFamily::default();
        assert_eq!(
            family.measure("Hello", f32::NAN),
            super::TextMetrics::default()
        );
        assert_eq!(family.measure("Hello", 0.0), super::TextMetrics::default());
    }

    #[test]
    fn invalid_font_data_is_rejected() {
        assert!(FontFace::from_bytes(b"not a font").is_err());
    }

    #[test]
    fn empty_text_has_zero_metrics() {
        let metrics = super::TextMetrics::default();
        assert_eq!(metrics.width, 0.0);
        assert_eq!(metrics.height, 0.0);
        assert_eq!(metrics.line_count, 0);
    }

    fn layout_lines_for_test(text: &str, max_width: f32) -> Vec<String> {
        text.split('\n')
            .flat_map(|paragraph| {
                super::wrap_paragraph(paragraph, max_width, |part| part.len() as f32)
            })
            .collect()
    }

    #[test]
    fn text_wrapping_keeps_words_together_when_they_fit() {
        let lines = super::wrap_paragraph("one two three", 7.0, |part| part.len() as f32);
        assert_eq!(lines, ["one two", "three"]);
    }

    #[test]
    fn text_wrapping_splits_words_wider_than_the_line() {
        let lines = super::wrap_paragraph("abcdefgh", 3.0, |part| part.len() as f32);
        assert_eq!(lines, ["abc", "def", "gh"]);
    }

    #[test]
    fn text_wrapping_preserves_empty_paragraphs() {
        let lines = layout_lines_for_test("first\n\nlast", 20.0);
        assert_eq!(lines, ["first", "", "last"]);
    }
}
