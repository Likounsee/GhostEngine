//! Font loading, basic text measurement and CPU glyph rasterization.
//!
//! Text layout and glyph rasterization for application interfaces.
//! Font fallback, glyph caching, kerning and basic combining-mark placement are
//! supported. The low-level framebuffer drawing functions retain a simple glyph-by-glyph
//! path; the high-level display-list text API prepares OpenType-shaped, bidirectional
//! fallback runs during recording so shaping is not repeated during frame replay.

use crate::{
    rasterizer::blend_pixel, Color, Framebuffer, OpenTypeShaper, Point, Rect, ShapedText,
    ShapingError, TextDirection,
};
use fontdue::Font;

const MAX_CACHED_GLYPHS_PER_FONT: usize = 4096;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A loaded TrueType/OpenType font backed by the fontdue parser and rasterizer.
pub struct FontFace {
    font: Font,
    shaper: OpenTypeShaper,
    glyph_cache: Mutex<HashMap<(u32, char), Arc<CachedGlyph>>>,
    indexed_glyph_cache: Mutex<HashMap<(u32, u16), Arc<CachedGlyph>>>,
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
        let font = Font::from_bytes(bytes, fontdue::FontSettings::default())
            .map_err(|_| "invalid font data")?;
        let shaper =
            OpenTypeShaper::from_bytes(bytes).map_err(|_| "invalid OpenType layout data")?;
        Ok(Self {
            font,
            shaper,
            glyph_cache: Mutex::new(HashMap::new()),
            indexed_glyph_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Shapes a text run with this face's GSUB/GPOS tables.
    pub fn shape_text(&self, text: &str, pixel_size: f32) -> Result<ShapedText, ShapingError> {
        self.shaper.shape(text, pixel_size)
    }

    /// Shapes text in an explicit left-to-right or right-to-left direction.
    pub fn shape_text_with_direction(
        &self,
        text: &str,
        pixel_size: f32,
        direction: TextDirection,
    ) -> Result<ShapedText, ShapingError> {
        self.shaper
            .shape_with_direction(text, pixel_size, direction)
    }

    fn glyph_index(&self, glyph_index: u16, pixel_size: f32) -> Arc<CachedGlyph> {
        let key = (pixel_size.to_bits(), glyph_index);
        {
            let cache = self
                .indexed_glyph_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(glyph) = cache.get(&key) {
                return Arc::clone(glyph);
            }
        }

        // Rasterization can be expensive; do not serialize unrelated cache misses.
        let (metrics, bitmap) = self.font.rasterize_indexed(glyph_index, pixel_size);
        let glyph = Arc::new(CachedGlyph {
            xmin: metrics.xmin,
            ymin: metrics.ymin,
            width: metrics.width,
            height: metrics.height,
            advance_width: metrics.advance_width,
            bitmap: Arc::from(bitmap),
        });

        let mut cache = self
            .indexed_glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = cache.get(&key) {
            return Arc::clone(existing);
        }
        if cache.len() >= MAX_CACHED_GLYPHS_PER_FONT {
            cache.clear();
        }
        cache.insert(key, Arc::clone(&glyph));
        glyph
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

    /// Number of distinct character- and glyph-ID/size pairs retained by this font.
    #[must_use]
    pub fn cached_glyph_count(&self) -> usize {
        let character_count = self
            .glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        let indexed_count = self
            .indexed_glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        character_count + indexed_count
    }

    /// Releases both character and shaped-glyph bitmaps. They are regenerated on demand.
    pub fn clear_glyph_cache(&self) {
        self.glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.indexed_glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    fn glyph(&self, character: char, pixel_size: f32) -> Arc<CachedGlyph> {
        let key = (pixel_size.to_bits(), character);
        {
            let cache = self
                .glyph_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(glyph) = cache.get(&key) {
                return Arc::clone(glyph);
            }
        }

        // Keep expensive rasterization outside the lock so parallel misses can proceed.
        let (metrics, bitmap) = self.font.rasterize(character, pixel_size);
        let glyph = Arc::new(CachedGlyph {
            xmin: metrics.xmin,
            ymin: metrics.ymin,
            width: metrics.width,
            height: metrics.height,
            advance_width: metrics.advance_width,
            bitmap: Arc::from(bitmap),
        });

        let mut cache = self
            .glyph_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = cache.get(&key) {
            return Arc::clone(existing);
        }
        if cache.len() >= MAX_CACHED_GLYPHS_PER_FONT {
            // Bound memory when applications animate font sizes or render arbitrary text.
            cache.clear();
        }
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

/// Returns the next tab stop strictly after a position, measured from line origin.
fn next_tab_stop(position: f64, tab_width: f64) -> Option<f64> {
    if !position.is_finite() || !tab_width.is_finite() || tab_width <= 0.0 {
        return None;
    }
    let stop = (position / tab_width).floor() + 1.0;
    let next = stop * tab_width;
    (next.is_finite() && next >= position).then_some(next)
}

/// Measures text while selecting fallback faces character by character.
pub fn measure_with_fallback(family: &FontFamily, text: &str, pixel_size: f32) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() || family.is_empty() {
        return TextMetrics::default();
    }
    let primary = Arc::clone(&family.faces[0]);
    let mut width = 0.0_f64;
    let mut max_width = 0.0_f64;
    let mut lines = 1_u32;
    let mut line_height = family.line_height(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 {
        return TextMetrics::default();
    }
    let mut previous: Option<(char, usize)> = None;
    let space = family
        .font_for(' ')
        .unwrap_or(primary.as_ref())
        .glyph(' ', pixel_size);
    let tab_width = (f64::from(space.advance_width) * 4.0).max(f64::from(pixel_size));
    if !tab_width.is_finite() || tab_width <= 0.0 || tab_width > f64::from(f32::MAX) {
        return TextMetrics::default();
    }

    for cluster in fallback_text_clusters(text) {
        match cluster {
            "\n" => {
                max_width = max_width.max(width);
                width = 0.0;
                let Some(next_lines) = lines.checked_add(1) else {
                    return TextMetrics::default();
                };
                lines = next_lines;
                previous = None;
            }
            "\t" => {
                let Some(next_width) = next_tab_stop(width, tab_width) else {
                    return TextMetrics::default();
                };
                width = next_width;
                if width > f64::from(f32::MAX) {
                    return TextMetrics::default();
                }
                previous = None;
            }
            _ => {
                if !cluster.chars().any(|character| {
                    !is_combining_mark(character) && !is_shaping_control(character)
                }) {
                    continue;
                }
                let current_face = previous.and_then(|(_, index)| family.faces.get(index));
                let selected = fallback_face_for_cluster(family, current_face, cluster, &primary);
                let face_index = family
                    .faces
                    .iter()
                    .position(|candidate| Arc::ptr_eq(candidate, &selected))
                    .unwrap_or(0);
                let selected_line_height = selected.line_height(pixel_size);
                if !selected_line_height.is_finite() || selected_line_height <= 0.0 {
                    return TextMetrics::default();
                }
                line_height = line_height.max(selected_line_height);
                for character in cluster.chars() {
                    if is_combining_mark(character) || is_shaping_control(character) {
                        continue;
                    }
                    if let Some((previous_character, previous_face)) = previous {
                        if previous_face == face_index {
                            let kerning =
                                selected.kerning(previous_character, character, pixel_size);
                            if !kerning.is_finite() {
                                return TextMetrics::default();
                            }
                            width += f64::from(kerning);
                            if !width.is_finite() || width.abs() > f64::from(f32::MAX) {
                                return TextMetrics::default();
                            }
                        }
                    }
                    let advance = selected.glyph(character, pixel_size).advance_width;
                    if !advance.is_finite() || advance < 0.0 {
                        return TextMetrics::default();
                    }
                    width += f64::from(advance);
                    if !width.is_finite() || width > f64::from(f32::MAX) {
                        return TextMetrics::default();
                    }
                    previous = Some((character, face_index));
                }
            }
        }
    }
    max_width = max_width.max(width);
    if !max_width.is_finite() || max_width < 0.0 || max_width > f64::from(f32::MAX) {
        return TextMetrics::default();
    }
    let Some(height) = checked_text_block_height(line_height, lines as usize) else {
        return TextMetrics::default();
    };
    TextMetrics {
        width: max_width as f32,
        height,
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
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() || family.is_empty() {
        return TextMetrics::default();
    }
    let primary = Arc::clone(&family.faces[0]);
    let metrics = measure_with_fallback(family, text, pixel_size);
    let line_height = family.line_height(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 {
        return TextMetrics::default();
    }
    let mut x = baseline.x as f64;
    let mut y = baseline.y as f64;
    let mut previous: Option<(char, usize)> = None;
    let mut base_anchor: Option<(f32, f32, usize, usize)> = None;
    let mut above_mark_offset = 0.0_f32;
    let mut below_mark_offset = 0.0_f32;
    let space = family
        .font_for(' ')
        .unwrap_or(primary.as_ref())
        .glyph(' ', pixel_size);
    let tab_width = (space.advance_width * 4.0).max(pixel_size);
    if !tab_width.is_finite() || tab_width <= 0.0 {
        return TextMetrics::default();
    }

    for cluster in fallback_text_clusters(text) {
        match cluster {
            "\n" => {
                x = baseline.x as f64;
                previous = None;
                base_anchor = None;
                above_mark_offset = 0.0;
                below_mark_offset = 0.0;
                y += line_height as f64;
                if !y.is_finite() || y < i32::MIN as f64 || y > i32::MAX as f64 {
                    return TextMetrics::default();
                }
            }
            "\t" => {
                let relative_x = x - f64::from(baseline.x);
                let Some(next_x) = next_tab_stop(relative_x, f64::from(tab_width)) else {
                    return TextMetrics::default();
                };
                x = f64::from(baseline.x) + next_x;
                if !x.is_finite() {
                    return TextMetrics::default();
                }
                previous = None;
                base_anchor = None;
                above_mark_offset = 0.0;
                below_mark_offset = 0.0;
            }
            _ => {
                let current_face = previous.and_then(|(_, index)| family.faces.get(index));
                let selected = fallback_face_for_cluster(family, current_face, cluster, &primary);
                let face_index = family
                    .faces
                    .iter()
                    .position(|candidate| Arc::ptr_eq(candidate, &selected))
                    .unwrap_or(0);
                let face = selected.as_ref();

                for character in cluster.chars() {
                    if is_shaping_control(character) {
                        continue;
                    }
                    let glyph = face.glyph(character, pixel_size);
                    let combining = is_combining_mark(character);
                    if !combining {
                        if let Some((previous_character, previous_face)) = previous {
                            if previous_face == face_index {
                                x += face.kerning(previous_character, character, pixel_size) as f64;
                                if !x.is_finite() {
                                    return TextMetrics::default();
                                }
                            }
                        }
                    }
                    let glyph_left_f64 = x + glyph.xmin as f64;
                    let glyph_top_f64 = y - glyph.ymin as f64 - glyph.height as f64;
                    if !glyph_left_f64.is_finite()
                        || !glyph_top_f64.is_finite()
                        || glyph_left_f64 < f32::MIN as f64
                        || glyph_left_f64 > f32::MAX as f64
                        || glyph_top_f64 < f32::MIN as f64
                        || glyph_top_f64 > f32::MAX as f64
                    {
                        return TextMetrics::default();
                    }
                    let mut glyph_left = glyph_left_f64 as f32;
                    let mut glyph_top = glyph_top_f64 as f32;
                    if combining {
                        if let Some(anchor) = base_anchor {
                            let offset = if is_below_combining_mark(character) {
                                below_mark_offset
                            } else {
                                above_mark_offset
                            };
                            (glyph_left, glyph_top) = positioned_mark_origin_stacked(
                                character,
                                glyph.width,
                                glyph.height,
                                anchor,
                                offset,
                            );
                            if is_below_combining_mark(character) {
                                below_mark_offset = next_mark_stack_offset(offset, glyph.height);
                            } else {
                                above_mark_offset = next_mark_stack_offset(offset, glyph.height);
                            }
                        }
                    } else {
                        base_anchor = Some((glyph_left, glyph_top, glyph.width, glyph.height));
                        above_mark_offset = 0.0;
                        below_mark_offset = 0.0;
                    }
                    let draw_x = glyph_left as f64 - glyph.xmin as f64;
                    let draw_y = glyph_top as f64 + glyph.ymin as f64 + glyph.height as f64;
                    if draw_x.is_finite()
                        && draw_y.is_finite()
                        && draw_x >= i32::MIN as f64
                        && draw_x <= i32::MAX as f64
                        && draw_y >= i32::MIN as f64
                        && draw_y <= i32::MAX as f64
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
                        x += glyph.advance_width as f64;
                        if !x.is_finite() {
                            return TextMetrics::default();
                        }
                        previous = Some((character, face_index));
                    }
                }
            }
        }
    }
    metrics
}

/// Draws a shaped glyph run using GhostEngine's own CPU glyph rasterizer.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_shaped_clipped(
    target: &mut Framebuffer,
    font: &FontFace,
    shaped: &ShapedText,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || shaped.glyphs.is_empty() {
        return TextMetrics::default();
    }

    if !shaped_run_is_safe(shaped, baseline) {
        return TextMetrics::default();
    }

    let mut pen_x = f64::from(baseline.x);
    let mut pen_y = f64::from(baseline.y);
    for shaped_glyph in &shaped.glyphs {
        let Ok(glyph_index) = u16::try_from(shaped_glyph.glyph_id) else {
            pen_x += f64::from(shaped_glyph.x_advance);
            pen_y -= f64::from(shaped_glyph.y_advance);
            continue;
        };
        let glyph = font.glyph_index(glyph_index, pixel_size);
        let left = pen_x + f64::from(shaped_glyph.x_offset) + f64::from(glyph.xmin);
        let top =
            pen_y - f64::from(shaped_glyph.y_offset) - f64::from(glyph.ymin) - glyph.height as f64;
        if left.is_finite()
            && top.is_finite()
            && left >= f64::from(i32::MIN)
            && left <= f64::from(i32::MAX)
            && top >= f64::from(i32::MIN)
            && top <= f64::from(i32::MAX)
        {
            draw_cached_glyph_clipped(
                target,
                &glyph,
                left.round() as i32,
                top.round() as i32,
                color,
                clip,
            );
        }
        pen_x += f64::from(shaped_glyph.x_advance);
        pen_y -= f64::from(shaped_glyph.y_advance);
    }
    TextMetrics {
        width: shaped.advance_x.abs(),
        height: font.line_height(pixel_size),
        line_count: 1,
    }
}

/// Checks public shaped-run data before any glyph-cache or framebuffer access.
fn f32_accumulation_exceeds_range(base: f64, delta: f32) -> bool {
    let limit = f64::from(f32::MAX);
    let delta = f64::from(delta);
    (delta > 0.0 && base > limit - delta) || (delta < 0.0 && base < -limit - delta)
}

fn shaped_run_is_safe(shaped: &ShapedText, baseline: Point) -> bool {
    if !shaped.advance_x.is_finite()
        || !shaped.advance_y.is_finite()
        || shaped.glyphs.iter().any(|glyph| {
            !glyph.x_advance.is_finite()
                || !glyph.y_advance.is_finite()
                || !glyph.x_offset.is_finite()
                || !glyph.y_offset.is_finite()
        })
    {
        return false;
    }

    let total_x = shaped
        .glyphs
        .iter()
        .map(|glyph| f64::from(glyph.x_advance))
        .sum::<f64>();
    let total_y = shaped
        .glyphs
        .iter()
        .map(|glyph| f64::from(glyph.y_advance))
        .sum::<f64>();
    if !total_x.is_finite()
        || !total_y.is_finite()
        || total_x.abs() > f64::from(f32::MAX)
        || total_y.abs() > f64::from(f32::MAX)
    {
        return false;
    }

    let mut pen_x = f64::from(baseline.x);
    let mut pen_y = f64::from(baseline.y);
    for glyph in &shaped.glyphs {
        if f32_accumulation_exceeds_range(pen_x, glyph.x_offset)
            || f32_accumulation_exceeds_range(pen_y, -glyph.y_offset)
        {
            return false;
        }
        let glyph_x = pen_x + f64::from(glyph.x_offset);
        let glyph_y = pen_y - f64::from(glyph.y_offset);
        if !glyph_x.is_finite()
            || !glyph_y.is_finite()
            || glyph_x.abs() > f64::from(f32::MAX)
            || glyph_y.abs() > f64::from(f32::MAX)
            || f32_accumulation_exceeds_range(pen_x, glyph.x_advance)
            || f32_accumulation_exceeds_range(pen_y, -glyph.y_advance)
        {
            return false;
        }
        pen_x += f64::from(glyph.x_advance);
        pen_y -= f64::from(glyph.y_advance);
        if !pen_x.is_finite()
            || !pen_y.is_finite()
            || pen_x.abs() > f64::from(f32::MAX)
            || pen_y.abs() > f64::from(f32::MAX)
        {
            return false;
        }
    }
    true
}

fn draw_cached_glyph_clipped(
    target: &mut Framebuffer,
    glyph: &CachedGlyph,
    left: i32,
    top: i32,
    color: Color,
    clip: Rect,
) {
    if glyph.width == 0 || glyph.height == 0 {
        return;
    }
    let x0 = left.max(clip.x.min(i32::MAX as u32) as i32).max(0);
    let y0 = top.max(clip.y.min(i32::MAX as u32) as i32).max(0);
    let clip_right = clip.x.saturating_add(clip.width).min(i32::MAX as u32) as i32;
    let clip_bottom = clip.y.saturating_add(clip.height).min(i32::MAX as u32) as i32;
    let x1 = left
        .saturating_add(glyph.width.min(i32::MAX as usize) as i32)
        .min(clip_right)
        .min(target.width() as i32);
    let y1 = top
        .saturating_add(glyph.height.min(i32::MAX as usize) as i32)
        .min(clip_bottom)
        .min(target.height() as i32);
    for y in y0..y1 {
        for x in x0..x1 {
            let source_x = (x - left) as usize;
            let source_y = (y - top) as usize;
            let coverage = glyph.bitmap[source_y * glyph.width + source_x];
            let alpha = ((u16::from(color.a) * u16::from(coverage) + 127) / 255) as u8;
            if alpha != 0 {
                blend_pixel(
                    target,
                    Point::new(x, y),
                    Color::rgba(color.r, color.g, color.b, alpha),
                );
            }
        }
    }
}

/// Draws a single-font paragraph using Unicode bidirectional visual ordering.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_bidi_clipped(
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

    let line_height = font.line_height(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 {
        return TextMetrics::default();
    }

    let origin_x = f64::from(baseline.x);
    let mut max_width = 0.0_f64;
    let mut line_count = 0_u32;

    for paragraph in text.split('\n') {
        let mut pen_x = origin_x;
        let y = f64::from(baseline.y) + f64::from(line_count) * f64::from(line_height);
        if !y.is_finite() || y < f64::from(i32::MIN) || y > f64::from(i32::MAX) {
            return TextMetrics::default();
        }

        for run in crate::shaping::resolve_bidi_runs(paragraph) {
            let Some(run_text) = paragraph.get(run.byte_range) else {
                continue;
            };
            let Ok(shaped) = font.shape_text_with_direction(run_text, pixel_size, run.direction)
            else {
                continue;
            };
            if !shaped_run_is_safe(&shaped, Point::new(0, 0)) {
                return TextMetrics::default();
            }

            let next_x = pen_x + f64::from(shaped.advance_x.abs());
            if !next_x.is_finite()
                || next_x < f64::from(i32::MIN)
                || next_x > f64::from(i32::MAX)
                || pen_x < f64::from(i32::MIN)
                || pen_x > f64::from(i32::MAX)
            {
                return TextMetrics::default();
            }

            draw_text_shaped_clipped(
                target,
                font,
                &shaped,
                Point::new(pen_x.round() as i32, y.round() as i32),
                pixel_size,
                color,
                clip,
            );
            pen_x = next_x;
        }

        let line_width = pen_x - origin_x;
        if !line_width.is_finite() || line_width < 0.0 || line_width > f64::from(f32::MAX) {
            return TextMetrics::default();
        }
        max_width = max_width.max(line_width);
        let Some(next_line_count) = line_count.checked_add(1) else {
            return TextMetrics::default();
        };
        line_count = next_line_count;
    }

    let Some(height) = checked_text_block_height(line_height, line_count as usize) else {
        return TextMetrics::default();
    };
    if max_width > f64::from(f32::MAX) {
        return TextMetrics::default();
    }

    TextMetrics {
        width: max_width as f32,
        height,
        line_count,
    }
}

/// Returns whether a scalar is a non-rendering Unicode shaping control.
fn is_shaping_control(character: char) -> bool {
    let code = character as u32;
    character == '\u{200D}'
        || (0xFE00..=0xFE0F).contains(&code)
        || (0xE0100..=0xE01EF).contains(&code)
        || (0xE0020..=0xE007F).contains(&code)
}

fn fallback_face_for_cluster(
    family: &FontFamily,
    current_face: Option<&Arc<FontFace>>,
    cluster: &str,
    primary: &Arc<FontFace>,
) -> Arc<FontFace> {
    let characters = cluster.chars().collect::<Vec<_>>();
    let base = characters
        .iter()
        .copied()
        .find(|character| !is_combining_mark(*character) && !is_shaping_control(*character));
    // Shaping controls affect cluster presentation but are not independently
    // rendered glyphs. All other scalars should be covered by one face.
    let required = characters
        .iter()
        .copied()
        .filter(|character| !is_shaping_control(*character))
        .collect::<Vec<_>>();
    if let Some(base) = base {
        family
            .faces
            .iter()
            .find(|face| required.iter().all(|character| face.has_glyph(*character)))
            .or_else(|| family.faces.iter().find(|face| face.has_glyph(base)))
            .cloned()
            .unwrap_or_else(|| Arc::clone(primary))
    } else {
        current_face
            .filter(|face| required.iter().all(|character| face.has_glyph(*character)))
            .cloned()
            .or_else(|| {
                family
                    .faces
                    .iter()
                    .find(|face| required.iter().all(|character| face.has_glyph(*character)))
                    .cloned()
            })
            .or_else(|| current_face.cloned())
            .unwrap_or_else(|| Arc::clone(primary))
    }
}

/// Draws bidirectional OpenType text using an ordered font fallback family.
///
/// Paragraph direction is resolved by Unicode bidi, each visual directional
/// run is shaped independently, and contiguous fallback-font spans are shaped
/// in that run's direction. Wrapping and bidi-aware DisplayList preparation are
/// separate layout operations and are not performed by this function.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_with_fallback_bidi_clipped(
    target: &mut Framebuffer,
    family: &FontFamily,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() || family.is_empty() {
        return TextMetrics::default();
    }

    let line_height = family.line_height(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 {
        return TextMetrics::default();
    }

    enum Segment {
        Glyphs(Arc<FontFace>, String),
        Tab,
    }

    let primary = Arc::clone(&family.faces[0]);
    let space_advance = family
        .font_for(' ')
        .unwrap_or(primary.as_ref())
        .glyph(' ', pixel_size)
        .advance_width;
    let tab_width = (space_advance * 4.0).max(pixel_size);
    if !tab_width.is_finite() || tab_width <= 0.0 {
        return TextMetrics::default();
    }

    let origin_x = f64::from(baseline.x);
    let mut max_width = 0.0_f64;
    let mut line_count = 0_u32;

    for paragraph in text.split('\n') {
        let mut pen_x = origin_x;

        for bidi_run in crate::shaping::resolve_bidi_runs(paragraph) {
            let Some(run_text) = paragraph.get(bidi_run.byte_range) else {
                continue;
            };
            let mut segments = Vec::new();
            let mut current_face: Option<Arc<FontFace>> = None;
            let mut current_text = String::new();

            for cluster in fallback_text_clusters(run_text) {
                if cluster == "\t" {
                    if let Some(face) = current_face.take() {
                        segments.push(Segment::Glyphs(face, std::mem::take(&mut current_text)));
                    }
                    segments.push(Segment::Tab);
                    continue;
                }

                let selected =
                    fallback_face_for_cluster(family, current_face.as_ref(), cluster, &primary);

                if current_face
                    .as_ref()
                    .is_some_and(|face| !Arc::ptr_eq(face, &selected))
                {
                    if let Some(face) = current_face.take() {
                        segments.push(Segment::Glyphs(face, std::mem::take(&mut current_text)));
                    }
                }
                if current_face.is_none() {
                    current_face = Some(selected);
                }
                current_text.push_str(cluster);
            }
            if let Some(face) = current_face {
                segments.push(Segment::Glyphs(face, current_text));
            }

            if bidi_run.direction == TextDirection::RightToLeft {
                segments.reverse();
            }

            for segment in segments {
                match segment {
                    Segment::Tab => {
                        let relative_x = (pen_x - origin_x).max(0.0);
                        let next_x = origin_x
                            + ((relative_x / f64::from(tab_width)).floor() + 1.0)
                                * f64::from(tab_width);
                        if !next_x.is_finite()
                            || next_x < f64::from(i32::MIN)
                            || next_x > f64::from(i32::MAX)
                        {
                            return TextMetrics::default();
                        }
                        pen_x = next_x;
                    }
                    Segment::Glyphs(face, run_text) => {
                        let Ok(shaped) = face.shape_text_with_direction(
                            &run_text,
                            pixel_size,
                            bidi_run.direction,
                        ) else {
                            continue;
                        };
                        if !shaped_run_is_safe(&shaped, Point::new(0, 0)) {
                            return TextMetrics::default();
                        }
                        let next_x = pen_x + f64::from(shaped.advance_x.abs());
                        let y =
                            f64::from(baseline.y) + f64::from(line_count) * f64::from(line_height);
                        if !next_x.is_finite()
                            || next_x < f64::from(i32::MIN)
                            || next_x > f64::from(i32::MAX)
                            || !y.is_finite()
                            || y < f64::from(i32::MIN)
                            || y > f64::from(i32::MAX)
                            || pen_x < f64::from(i32::MIN)
                            || pen_x > f64::from(i32::MAX)
                        {
                            return TextMetrics::default();
                        }
                        draw_text_shaped_clipped(
                            target,
                            &face,
                            &shaped,
                            Point::new(pen_x.round() as i32, y.round() as i32),
                            pixel_size,
                            color,
                            clip,
                        );
                        pen_x = next_x;
                    }
                }
            }
        }

        let line_width = pen_x - origin_x;
        if !line_width.is_finite() || line_width < 0.0 || line_width > f64::from(f32::MAX) {
            return TextMetrics::default();
        }
        max_width = max_width.max(line_width);
        let Some(next_line_count) = line_count.checked_add(1) else {
            return TextMetrics::default();
        };
        line_count = next_line_count;
    }

    let Some(height) = checked_text_block_height(line_height, line_count as usize) else {
        return TextMetrics::default();
    };
    if max_width > f64::from(f32::MAX) {
        return TextMetrics::default();
    }

    TextMetrics {
        width: max_width as f32,
        height,
        line_count,
    }
}

/// Draws OpenType-shaped text across a font fallback family.
///
/// Explicit newlines and four-space tab stops are handled here. Contiguous
/// character runs are shaped with the selected face, so GSUB/GPOS are applied
/// within each fallback run. This is not yet a full Unicode line-break or bidi
/// layout engine; callers needing wrapping should lay out lines before calling.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_with_fallback_shaped_clipped(
    target: &mut Framebuffer,
    family: &FontFamily,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() || family.is_empty() {
        return TextMetrics::default();
    }

    let line_height = family.line_height(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 {
        return TextMetrics::default();
    }
    let primary = &family.faces[0];
    let space_advance = family
        .font_for(' ')
        .unwrap_or(primary.as_ref())
        .glyph(' ', pixel_size)
        .advance_width;
    let tab_width = (space_advance * 4.0).max(pixel_size);
    if !tab_width.is_finite() || tab_width <= 0.0 {
        return TextMetrics::default();
    }

    let origin_x = f64::from(baseline.x);
    let mut pen_x = origin_x;
    let mut pen_y = f64::from(baseline.y);
    let mut max_width = 0.0_f64;
    let mut line_count = 1_u32;
    let mut run_text = String::new();
    let mut run_face: Option<Arc<FontFace>> = None;

    let flush_run = |target: &mut Framebuffer,
                     run_text: &mut String,
                     run_face: &mut Option<Arc<FontFace>>,
                     pen_x: &mut f64,
                     pen_y: f64| {
        if run_text.is_empty() {
            *run_face = None;
            return;
        }
        if let Some(face) = run_face.take() {
            if let Ok(shaped) = face.shape_text(run_text, pixel_size) {
                if !shaped_run_is_safe(&shaped, Point::new(0, 0)) {
                    *pen_x = f64::NAN;
                } else {
                    let next_pen = *pen_x + f64::from(shaped.advance_x);
                    if !next_pen.is_finite()
                        || next_pen < f64::from(i32::MIN)
                        || next_pen > f64::from(i32::MAX)
                        || !pen_y.is_finite()
                        || pen_y < f64::from(i32::MIN)
                        || pen_y > f64::from(i32::MAX)
                        || *pen_x < f64::from(i32::MIN)
                        || *pen_x > f64::from(i32::MAX)
                    {
                        *pen_x = f64::NAN;
                    } else {
                        draw_text_shaped_clipped(
                            target,
                            &face,
                            &shaped,
                            Point::new(pen_x.round() as i32, pen_y.round() as i32),
                            pixel_size,
                            color,
                            clip,
                        );
                        *pen_x = next_pen;
                    }
                }
            } else {
                *pen_x = f64::NAN;
            }
        }
        run_text.clear();
    };

    for cluster in fallback_text_clusters(text) {
        match cluster {
            "\n" => {
                flush_run(target, &mut run_text, &mut run_face, &mut pen_x, pen_y);
                if !pen_x.is_finite() {
                    return TextMetrics::default();
                }
                max_width = max_width.max(pen_x - origin_x);
                pen_x = origin_x;
                pen_y += f64::from(line_height);
                let Some(next_line_count) = line_count.checked_add(1) else {
                    return TextMetrics::default();
                };
                line_count = next_line_count;
                if !pen_y.is_finite() || pen_y < f64::from(i32::MIN) || pen_y > f64::from(i32::MAX)
                {
                    return TextMetrics::default();
                }
            }
            "\t" => {
                flush_run(target, &mut run_text, &mut run_face, &mut pen_x, pen_y);
                if !pen_x.is_finite() {
                    return TextMetrics::default();
                }
                let relative = (pen_x - origin_x).max(0.0);
                pen_x = origin_x
                    + ((relative / f64::from(tab_width)).floor() + 1.0) * f64::from(tab_width);
                if !pen_x.is_finite() || pen_x < f64::from(i32::MIN) || pen_x > f64::from(i32::MAX)
                {
                    return TextMetrics::default();
                }
            }
            _ => {
                if cluster.chars().all(is_shaping_control) {
                    continue;
                }
                let selected =
                    fallback_face_for_cluster(family, run_face.as_ref(), cluster, primary);
                let same_face = run_face
                    .as_ref()
                    .is_some_and(|face| Arc::ptr_eq(face, &selected));
                if !same_face && !run_text.is_empty() {
                    flush_run(target, &mut run_text, &mut run_face, &mut pen_x, pen_y);
                    if !pen_x.is_finite() {
                        return TextMetrics::default();
                    }
                }
                if run_face.is_none() {
                    run_face = Some(selected);
                }
                run_text.push_str(cluster);
            }
        }
    }
    flush_run(target, &mut run_text, &mut run_face, &mut pen_x, pen_y);
    if !pen_x.is_finite() {
        return TextMetrics::default();
    }
    max_width = max_width.max(pen_x - origin_x);
    if !max_width.is_finite() || max_width < 0.0 || max_width > f64::from(f32::MAX) {
        return TextMetrics::default();
    }
    let Some(height) = checked_text_block_height(line_height, line_count as usize) else {
        return TextMetrics::default();
    };

    TextMetrics {
        width: max_width as f32,
        height,
        line_count,
    }
}
/// Measures one line using OpenType shaping for each contiguous fallback-font run.
fn measure_shaped_fallback_line(family: &FontFamily, text: &str, pixel_size: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let Some(primary) = family.faces.first().cloned() else {
        return f32::INFINITY;
    };
    if !pixel_size.is_finite() || pixel_size <= 0.0 {
        return f32::INFINITY;
    }

    enum Segment {
        Glyphs(Arc<FontFace>, String),
        Tab,
    }

    let space = family
        .font_for(' ')
        .unwrap_or(primary.as_ref())
        .glyph(' ', pixel_size)
        .advance_width;
    let tab_width = (space * 4.0).max(pixel_size);
    if !tab_width.is_finite() || tab_width <= 0.0 {
        return f32::INFINITY;
    }
    let mut width = 0.0_f64;

    for bidi_run in crate::shaping::resolve_bidi_runs(text) {
        let Some(run_text) = text.get(bidi_run.byte_range) else {
            return f32::INFINITY;
        };
        let mut segments = Vec::new();
        let mut current_face: Option<Arc<FontFace>> = None;
        let mut current_text = String::new();

        for cluster in fallback_text_clusters(run_text) {
            if cluster == "\t" {
                if let Some(face) = current_face.take() {
                    segments.push(Segment::Glyphs(face, std::mem::take(&mut current_text)));
                }
                segments.push(Segment::Tab);
                continue;
            }

            let selected =
                fallback_face_for_cluster(family, current_face.as_ref(), cluster, &primary);
            if current_face
                .as_ref()
                .is_some_and(|face| !Arc::ptr_eq(face, &selected))
            {
                if let Some(face) = current_face.take() {
                    segments.push(Segment::Glyphs(face, std::mem::take(&mut current_text)));
                }
            }
            if current_face.is_none() {
                current_face = Some(selected);
            }
            current_text.push_str(cluster);
        }

        if let Some(face) = current_face {
            segments.push(Segment::Glyphs(face, current_text));
        }
        if bidi_run.direction == TextDirection::RightToLeft {
            segments.reverse();
        }

        for segment in segments {
            match segment {
                Segment::Tab => {
                    width = ((width / f64::from(tab_width)).floor() + 1.0) * f64::from(tab_width);
                }
                Segment::Glyphs(face, run) => {
                    let Ok(shaped) =
                        face.shape_text_with_direction(&run, pixel_size, bidi_run.direction)
                    else {
                        return f32::INFINITY;
                    };
                    if !shaped_run_is_safe(&shaped, Point::new(0, 0)) {
                        return f32::INFINITY;
                    }
                    width += f64::from(shaped.advance_x.abs());
                }
            }
            if !width.is_finite() || width < 0.0 || width > f64::from(f32::MAX) {
                return f32::INFINITY;
            }
        }
    }

    width as f32
}

/// Splits text at Unicode extended grapheme-cluster boundaries (UAX #29).
///
/// This keeps combining sequences, variation selectors, emoji modifiers,
/// regional-indicator pairs, and ZWJ emoji sequences together during fallback
/// selection and when an oversized word must be wrapped.
fn fallback_text_clusters(text: &str) -> unicode_segmentation::Graphemes<'_> {
    use unicode_segmentation::UnicodeSegmentation;

    UnicodeSegmentation::graphemes(text, true)
}

#[derive(Clone)]
pub(crate) struct PreparedShapedTextRun {
    pub font: Arc<FontFace>,
    pub shaped: ShapedText,
    pub offset_x: f32,
}

#[derive(Clone)]
pub(crate) struct PreparedShapedTextLine {
    pub baseline: Point,
    pub runs: Vec<PreparedShapedTextRun>,
}

#[derive(Clone)]
pub(crate) struct PreparedShapedTextBox {
    pub lines: Vec<PreparedShapedTextLine>,
}

/// Wraps shaped fallback text without normalizing or discarding whitespace.
fn wrap_shaped_fallback_paragraph(
    family: &FontFamily,
    paragraph: &str,
    pixel_size: f32,
    max_width: f32,
) -> Vec<String> {
    wrap_paragraph_preserving_whitespace(paragraph, max_width, |candidate| {
        measure_shaped_fallback_line(family, candidate, pixel_size)
    })
}

/// Generic line wrapper that preserves whitespace and only splits oversized words
/// at Unicode extended grapheme-cluster boundaries.
fn wrap_paragraph_preserving_whitespace<F>(
    paragraph: &str,
    max_width: f32,
    mut measure: F,
) -> Vec<String>
where
    F: FnMut(&str) -> f32,
{
    if paragraph.is_empty() {
        return vec![String::new()];
    }

    let mut output = Vec::new();
    let mut line = String::new();
    let mut token = String::new();
    let mut token_is_whitespace = None;

    let flush_token = |token: &mut String,
                       token_is_whitespace: &mut Option<bool>,
                       line: &mut String,
                       output: &mut Vec<String>,
                       measure: &mut F| {
        if token.is_empty() {
            return;
        }
        let is_whitespace = token_is_whitespace.unwrap_or(false);
        if is_whitespace {
            // Keep spaces and tabs verbatim. If they occur at a wrap boundary,
            // they remain attached to the preceding line instead of disappearing.
            line.push_str(token);
        } else {
            let candidate = format!("{line}{token}");
            if !line.is_empty() && measure(&candidate) > max_width {
                output.push(std::mem::take(line));
            }
            if measure(token) <= max_width {
                line.push_str(token);
            } else {
                for cluster in fallback_text_clusters(token) {
                    let candidate = format!("{line}{cluster}");
                    if !line.is_empty() && measure(&candidate) > max_width {
                        output.push(std::mem::take(line));
                    }
                    line.push_str(cluster);
                }
            }
        }
        token.clear();
        *token_is_whitespace = None;
    };

    for character in paragraph.chars() {
        let whitespace = character.is_whitespace();
        if token_is_whitespace.is_some_and(|previous| previous != whitespace) {
            flush_token(
                &mut token,
                &mut token_is_whitespace,
                &mut line,
                &mut output,
                &mut measure,
            );
        }
        token_is_whitespace = Some(whitespace);
        token.push(character);
    }
    flush_token(
        &mut token,
        &mut token_is_whitespace,
        &mut line,
        &mut output,
        &mut measure,
    );
    output.push(line);
    output
}

/// Builds visual-order shaped runs for one bidi line, including font fallback.
fn prepare_bidi_shaped_line(
    family: &FontFamily,
    line: &str,
    pixel_size: f32,
) -> Result<Vec<PreparedShapedTextRun>, ShapingError> {
    if !pixel_size.is_finite() || pixel_size <= 0.0 {
        return Err(ShapingError::InvalidPixelSize);
    }
    if family.is_empty() {
        return Ok(Vec::new());
    }

    enum Segment {
        Glyphs(Arc<FontFace>, String),
        Tab,
    }

    let primary = Arc::clone(&family.faces[0]);
    let space_advance = family
        .font_for(' ')
        .unwrap_or(primary.as_ref())
        .glyph(' ', pixel_size)
        .advance_width;
    let tab_width = (space_advance * 4.0).max(pixel_size);
    if !tab_width.is_finite() || tab_width <= 0.0 {
        return Ok(Vec::new());
    }

    let mut output = Vec::new();
    let mut pen_x = 0.0_f64;

    for bidi_run in crate::shaping::resolve_bidi_runs(line) {
        let Some(run_text) = line.get(bidi_run.byte_range) else {
            return Ok(Vec::new());
        };
        let mut segments = Vec::new();
        let mut current_face: Option<Arc<FontFace>> = None;
        let mut current_text = String::new();

        for cluster in fallback_text_clusters(run_text) {
            if cluster == "\t" {
                if let Some(face) = current_face.take() {
                    segments.push(Segment::Glyphs(face, std::mem::take(&mut current_text)));
                }
                segments.push(Segment::Tab);
                continue;
            }

            let selected =
                fallback_face_for_cluster(family, current_face.as_ref(), cluster, &primary);

            if current_face
                .as_ref()
                .is_some_and(|face| !Arc::ptr_eq(face, &selected))
            {
                if let Some(face) = current_face.take() {
                    segments.push(Segment::Glyphs(face, std::mem::take(&mut current_text)));
                }
            }
            if current_face.is_none() {
                current_face = Some(selected);
            }
            current_text.push_str(cluster);
        }
        if let Some(face) = current_face {
            segments.push(Segment::Glyphs(face, current_text));
        }

        // The bidi resolver supplies runs in visual order. Within an RTL run,
        // fallback spans must also be emitted in visual order, without reversing
        // the characters inside a span (the shaper handles that).
        if bidi_run.direction == TextDirection::RightToLeft {
            segments.reverse();
        }

        for segment in segments {
            match segment {
                Segment::Tab => {
                    pen_x = ((pen_x / f64::from(tab_width)).floor() + 1.0) * f64::from(tab_width);
                }
                Segment::Glyphs(font, text) => {
                    let shaped =
                        font.shape_text_with_direction(&text, pixel_size, bidi_run.direction)?;
                    if !shaped_run_is_safe(&shaped, Point::new(0, 0)) {
                        return Ok(Vec::new());
                    }
                    let advance = f64::from(shaped.advance_x.abs());
                    let offset_x = pen_x as f32;
                    if !advance.is_finite()
                        || !offset_x.is_finite()
                        || pen_x < f64::from(i32::MIN)
                        || pen_x > f64::from(i32::MAX)
                    {
                        return Ok(Vec::new());
                    }
                    output.push(PreparedShapedTextRun {
                        font,
                        shaped,
                        offset_x,
                    });
                    pen_x += advance;
                }
            }
            if !pen_x.is_finite() || pen_x < 0.0 || pen_x > f64::from(i32::MAX) {
                return Ok(Vec::new());
            }
        }
    }
    Ok(output)
}

fn measure_bidi_shaped_fallback_line(family: &FontFamily, line: &str, pixel_size: f32) -> f32 {
    let Ok(runs) = prepare_bidi_shaped_line(family, line, pixel_size) else {
        return f32::INFINITY;
    };
    let Some(width) = runs.iter().try_fold(0.0_f64, |total, run| {
        let advance = f64::from(run.shaped.advance_x.abs());
        let next = total + advance;
        (advance.is_finite() && next.is_finite() && next <= f64::from(f32::MAX)).then_some(next)
    }) else {
        return f32::INFINITY;
    };
    width as f32
}

/// Shapes, wraps and lays out bidirectional fallback text for display-list replay.
pub(crate) fn prepare_bidi_shaped_text_box(
    family: &FontFamily,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    options: TextLayoutOptions,
) -> Result<PreparedShapedTextBox, ShapingError> {
    if !pixel_size.is_finite() || pixel_size <= 0.0 {
        return Err(ShapingError::InvalidPixelSize);
    }
    if text.is_empty()
        || family.is_empty()
        || !options.line_spacing.is_finite()
        || options.line_spacing <= 0.0
        || options
            .max_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
    {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }

    let line_height = family.line_height(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }

    let mut text_lines = Vec::new();
    for paragraph in text.split('\n') {
        if let Some(max_width) = options.max_width {
            text_lines.extend(wrap_paragraph_preserving_whitespace(
                paragraph,
                max_width,
                |candidate| measure_bidi_shaped_fallback_line(family, candidate, pixel_size),
            ));
        } else {
            text_lines.push(paragraph.to_owned());
        }
    }

    let widths = text_lines
        .iter()
        .map(|line| measure_bidi_shaped_fallback_line(family, line, pixel_size))
        .collect::<Vec<_>>();
    if widths
        .iter()
        .any(|width| !width.is_finite() || *width < 0.0)
    {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }
    let widest = widths.iter().copied().fold(0.0_f32, f32::max);
    let alignment_width = options.max_width.unwrap_or(widest);
    let line_step = f64::from(line_height) * f64::from(options.line_spacing);
    if !line_step.is_finite() || line_step <= 0.0 || !alignment_width.is_finite() {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }

    let mut prepared_lines = Vec::with_capacity(text_lines.len());
    for (index, line) in text_lines.iter().enumerate() {
        let alignment_offset = match options.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => {
                ((f64::from(alignment_width) - f64::from(widths[index])) * 0.5).max(0.0)
            }
            TextAlign::Right => (f64::from(alignment_width) - f64::from(widths[index])).max(0.0),
        };
        let x = f64::from(baseline.x) + alignment_offset;
        let y = f64::from(baseline.y) + index as f64 * line_step;
        if !x.is_finite()
            || !y.is_finite()
            || x < f64::from(i32::MIN)
            || x > f64::from(i32::MAX)
            || y < f64::from(i32::MIN)
            || y > f64::from(i32::MAX)
        {
            return Ok(PreparedShapedTextBox { lines: Vec::new() });
        }
        prepared_lines.push(PreparedShapedTextLine {
            baseline: Point::new(x.round() as i32, y.round() as i32),
            runs: prepare_bidi_shaped_line(family, line, pixel_size)?,
        });
    }

    Ok(PreparedShapedTextBox {
        lines: prepared_lines,
    })
}

/// Draws wrapped, aligned bidirectional text with OpenType shaping and font fallback.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_box_with_fallback_bidi_shaped_clipped(
    target: &mut Framebuffer,
    family: &FontFamily,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
    options: TextLayoutOptions,
) -> TextMetrics {
    let Ok(prepared) = prepare_bidi_shaped_text_box(family, text, baseline, pixel_size, options)
    else {
        return TextMetrics::default();
    };
    let line_height = family.line_height(pixel_size);
    let mut widest = 0.0_f64;
    for line in &prepared.lines {
        let mut width = 0.0_f64;
        for run in &line.runs {
            let offset = f64::from(run.offset_x);
            let advance = f64::from(run.shaped.advance_x.abs());
            let end = offset + advance;
            if !offset.is_finite()
                || !advance.is_finite()
                || !end.is_finite()
                || end > f64::from(f32::MAX)
                || offset < 0.0
                || offset > f64::from(i32::MAX)
            {
                return TextMetrics::default();
            }
            draw_text_shaped_clipped(
                target,
                &run.font,
                &run.shaped,
                Point::new(
                    line.baseline.x.saturating_add(offset.round() as i32),
                    line.baseline.y,
                ),
                pixel_size,
                color,
                clip,
            );
            width = width.max(end);
        }
        widest = widest.max(width);
    }
    let step = f64::from(line_height) * f64::from(options.line_spacing);
    let height = step * prepared.lines.len() as f64;
    let Ok(line_count) = u32::try_from(prepared.lines.len()) else {
        return TextMetrics::default();
    };
    if !widest.is_finite()
        || widest > f64::from(f32::MAX)
        || !height.is_finite()
        || height > f64::from(f32::MAX)
    {
        return TextMetrics::default();
    }
    TextMetrics {
        width: widest as f32,
        height: height as f32,
        line_count,
    }
}

/// Shapes and lays out a fallback-font text block once, for reuse by display lists.
pub(crate) fn prepare_shaped_text_box(
    family: &FontFamily,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    options: TextLayoutOptions,
) -> Result<PreparedShapedTextBox, ShapingError> {
    if !pixel_size.is_finite() || pixel_size <= 0.0 {
        return Err(ShapingError::InvalidPixelSize);
    }
    if text.is_empty()
        || family.is_empty()
        || !options.line_spacing.is_finite()
        || options.line_spacing <= 0.0
        || options
            .max_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
    {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }

    let line_height = family.line_height(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }
    let mut text_lines = Vec::new();
    for paragraph in text.split('\n') {
        if let Some(max_width) = options.max_width {
            text_lines.extend(wrap_shaped_fallback_paragraph(
                family, paragraph, pixel_size, max_width,
            ));
        } else {
            text_lines.push(paragraph.to_owned());
        }
    }

    let widths: Vec<f32> = text_lines
        .iter()
        .map(|line| measure_shaped_fallback_line(family, line, pixel_size))
        .collect();
    if widths
        .iter()
        .any(|width| !width.is_finite() || *width < 0.0)
    {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }
    let widest = widths.iter().copied().fold(0.0_f32, f32::max);
    let alignment_width = options.max_width.unwrap_or(widest);
    let line_step = f64::from(line_height) * f64::from(options.line_spacing);
    if !alignment_width.is_finite() || !line_step.is_finite() || line_step <= 0.0 {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }
    let mut lines = Vec::with_capacity(text_lines.len());
    let primary = Arc::clone(&family.faces[0]);
    let space_advance = family
        .font_for(' ')
        .unwrap_or(primary.as_ref())
        .glyph(' ', pixel_size)
        .advance_width;
    let tab_width = (space_advance * 4.0).max(pixel_size);
    if !tab_width.is_finite() || tab_width <= 0.0 {
        return Ok(PreparedShapedTextBox { lines: Vec::new() });
    }

    for (line_index, line) in text_lines.iter().enumerate() {
        let alignment_offset = match options.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => {
                ((f64::from(alignment_width) - f64::from(widths[line_index])) * 0.5).max(0.0)
            }
            TextAlign::Right => {
                (f64::from(alignment_width) - f64::from(widths[line_index])).max(0.0)
            }
        };
        let x = f64::from(baseline.x) + alignment_offset;
        let y = f64::from(baseline.y) + line_index as f64 * line_step;
        if !x.is_finite()
            || !y.is_finite()
            || x < f64::from(i32::MIN)
            || x > f64::from(i32::MAX)
            || y < f64::from(i32::MIN)
            || y > f64::from(i32::MAX)
        {
            return Ok(PreparedShapedTextBox { lines: Vec::new() });
        }

        let mut runs = Vec::new();
        let mut current_run = String::new();
        let mut current_face: Option<Arc<FontFace>> = None;
        let mut pen_x = 0.0_f64;
        for character in line.chars() {
            if character == '\t' {
                if let Some(face) = current_face.take() {
                    let shaped = face.shape_text(&current_run, pixel_size)?;
                    if !shaped_run_is_safe(&shaped, Point::new(0, 0)) {
                        return Ok(PreparedShapedTextBox { lines: Vec::new() });
                    }
                    let advance = f64::from(shaped.advance_x);
                    let next_pen = pen_x + advance;
                    if !next_pen.is_finite() || next_pen < 0.0 || next_pen > f64::from(f32::MAX) {
                        return Ok(PreparedShapedTextBox { lines: Vec::new() });
                    }
                    runs.push(PreparedShapedTextRun {
                        font: face,
                        offset_x: pen_x as f32,
                        shaped,
                    });
                    pen_x = next_pen;
                    current_run.clear();
                }
                pen_x = ((pen_x / f64::from(tab_width)).floor() + 1.0) * f64::from(tab_width);
                if !pen_x.is_finite() || pen_x > f64::from(f32::MAX) {
                    return Ok(PreparedShapedTextBox { lines: Vec::new() });
                }
                continue;
            }

            let selected = if is_combining_mark(character) {
                current_face
                    .as_ref()
                    .filter(|face| face.has_glyph(character))
                    .cloned()
                    .or_else(|| {
                        family
                            .faces
                            .iter()
                            .find(|face| face.has_glyph(character))
                            .cloned()
                    })
                    .unwrap_or_else(|| Arc::clone(&primary))
            } else {
                family
                    .faces
                    .iter()
                    .find(|face| face.has_glyph(character))
                    .cloned()
                    .unwrap_or_else(|| Arc::clone(&primary))
            };
            if current_face
                .as_ref()
                .is_some_and(|face| !Arc::ptr_eq(face, &selected))
            {
                if let Some(face) = current_face.take() {
                    let shaped = face.shape_text(&current_run, pixel_size)?;
                    if !shaped_run_is_safe(&shaped, Point::new(0, 0)) {
                        return Ok(PreparedShapedTextBox { lines: Vec::new() });
                    }
                    let advance = f64::from(shaped.advance_x);
                    let next_pen = pen_x + advance;
                    if !next_pen.is_finite() || next_pen < 0.0 || next_pen > f64::from(f32::MAX) {
                        return Ok(PreparedShapedTextBox { lines: Vec::new() });
                    }
                    runs.push(PreparedShapedTextRun {
                        font: face,
                        offset_x: pen_x as f32,
                        shaped,
                    });
                    pen_x = next_pen;
                }
                current_run.clear();
            }
            if current_face.is_none() {
                current_face = Some(selected);
            }
            current_run.push(character);
        }
        if let Some(face) = current_face {
            let shaped = face.shape_text(&current_run, pixel_size)?;
            if !shaped_run_is_safe(&shaped, Point::new(0, 0)) {
                return Ok(PreparedShapedTextBox { lines: Vec::new() });
            }
            let advance = f64::from(shaped.advance_x);
            let end = pen_x + advance;
            if !end.is_finite() || end < 0.0 || end > f64::from(f32::MAX) {
                return Ok(PreparedShapedTextBox { lines: Vec::new() });
            }
            runs.push(PreparedShapedTextRun {
                font: face,
                offset_x: pen_x as f32,
                shaped,
            });
        }
        lines.push(PreparedShapedTextLine {
            baseline: Point::new(x.round() as i32, y.round() as i32),
            runs,
        });
    }

    Ok(PreparedShapedTextBox { lines })
}

/// Draws a shaped fallback-font text block with wrapping and per-line alignment.
///
/// Words are kept intact where possible. Overlong words are split only at
/// conservative grapheme-like boundaries (combining sequences, variation
/// selectors, ZWJ sequences and regional-indicator pairs remain together).
/// Explicit newlines, including empty paragraphs, are preserved. This is not
/// yet a complete Unicode line-breaking or bidirectional layout implementation.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_box_with_fallback_shaped_clipped(
    target: &mut Framebuffer,
    family: &FontFamily,
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
        || family.is_empty()
        || !options.line_spacing.is_finite()
        || options.line_spacing <= 0.0
        || options
            .max_width
            .is_some_and(|width| !width.is_finite() || width <= 0.0)
    {
        return TextMetrics::default();
    }
    let line_height = family.line_height(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 {
        return TextMetrics::default();
    }

    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if let Some(max_width) = options.max_width {
            lines.extend(wrap_shaped_fallback_paragraph(
                family, paragraph, pixel_size, max_width,
            ));
        } else {
            lines.push(paragraph.to_owned());
        }
    }

    let widths: Vec<f32> = lines
        .iter()
        .map(|line| measure_shaped_fallback_line(family, line, pixel_size))
        .collect();
    if widths
        .iter()
        .any(|width| !width.is_finite() || *width < 0.0)
    {
        return TextMetrics::default();
    }
    let widest = widths.iter().copied().fold(0.0_f32, f32::max);
    let alignment_width = options.max_width.unwrap_or(widest);
    let step = f64::from(line_height) * f64::from(options.line_spacing);
    let height = step * lines.len() as f64;
    let Ok(line_count) = u32::try_from(lines.len()) else {
        return TextMetrics::default();
    };
    if !alignment_width.is_finite()
        || !step.is_finite()
        || step <= 0.0
        || !height.is_finite()
        || height > f64::from(f32::MAX)
    {
        return TextMetrics::default();
    }
    for (index, line) in lines.iter().enumerate() {
        let offset = match options.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => {
                ((f64::from(alignment_width) - f64::from(widths[index])) * 0.5).max(0.0)
            }
            TextAlign::Right => (f64::from(alignment_width) - f64::from(widths[index])).max(0.0),
        };
        let x = f64::from(baseline.x) + offset;
        let y = f64::from(baseline.y) + index as f64 * step;
        if !x.is_finite()
            || !y.is_finite()
            || x < f64::from(i32::MIN)
            || x > f64::from(i32::MAX)
            || y < f64::from(i32::MIN)
            || y > f64::from(i32::MAX)
        {
            return TextMetrics::default();
        }
        draw_text_with_fallback_shaped_clipped(
            target,
            family,
            line,
            Point::new(x.round() as i32, y.round() as i32),
            pixel_size,
            color,
            clip,
        );
    }
    TextMetrics {
        width: widest,
        height: height as f32,
        line_count,
    }
}

/// Draws a shaped glyph run with a full affine transform and framebuffer clip.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_shaped_transformed_clipped(
    target: &mut Framebuffer,
    font: &FontFace,
    shaped: &ShapedText,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
    transform: crate::Transform2D,
) -> TextMetrics {
    if !pixel_size.is_finite()
        || pixel_size <= 0.0
        || shaped.glyphs.is_empty()
        || !transform.is_finite()
        || !shaped_run_is_safe(shaped, baseline)
    {
        return TextMetrics::default();
    }
    let Some(inverse) = transform.inverse() else {
        return TextMetrics::default();
    };
    let mut pen_x = f64::from(baseline.x);
    let mut pen_y = f64::from(baseline.y);
    for shaped_glyph in &shaped.glyphs {
        let Ok(glyph_index) = u16::try_from(shaped_glyph.glyph_id) else {
            pen_x += f64::from(shaped_glyph.x_advance);
            pen_y -= f64::from(shaped_glyph.y_advance);
            continue;
        };
        let glyph = font.glyph_index(glyph_index, pixel_size);
        let left = pen_x + f64::from(shaped_glyph.x_offset) + f64::from(glyph.xmin);
        let top =
            pen_y - f64::from(shaped_glyph.y_offset) - f64::from(glyph.ymin) - glyph.height as f64;
        if left.is_finite()
            && top.is_finite()
            && left.abs() <= f64::from(f32::MAX)
            && top.abs() <= f64::from(f32::MAX)
        {
            draw_affine_glyph(
                target,
                &glyph,
                left as f32,
                top as f32,
                color,
                clip,
                transform,
                inverse,
            );
        }
        pen_x += f64::from(shaped_glyph.x_advance);
        pen_y -= f64::from(shaped_glyph.y_advance);
    }
    TextMetrics {
        width: shaped.advance_x.abs(),
        height: font.line_height(pixel_size),
        line_count: 1,
    }
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
    if !line_height.is_finite() || line_height <= 0.0 {
        return TextMetrics::default();
    }
    let Some(block_height) = checked_text_block_height(line_height, lines.len()) else {
        return TextMetrics::default();
    };
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
        height: block_height,
        line_count: lines.len().min(u32::MAX as usize) as u32,
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
    let space_metrics = font.glyph(' ', pixel_size);
    let tab_width = (space_metrics.advance_width * 4.0).max(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 || !tab_width.is_finite() || tab_width <= 0.0
    {
        return TextMetrics::default();
    }

    let origin_x = f64::from(baseline.x);
    let mut pen_x = origin_x;
    let mut pen_y = f64::from(baseline.y);
    let mut previous = None;
    let mut base_anchor: Option<(f32, f32, usize, usize)> = None;
    let mut above_mark_offset = 0.0_f32;
    let mut below_mark_offset = 0.0_f32;
    let mut max_width = 0.0_f64;
    let mut line_count = 1_u32;

    for character in text.chars() {
        if character == '\n' {
            max_width = max_width.max(pen_x - origin_x);
            pen_x = origin_x;
            pen_y += f64::from(line_height);
            let Some(next_line_count) = line_count.checked_add(1) else {
                return TextMetrics::default();
            };
            line_count = next_line_count;
            if !pen_y.is_finite() || pen_y < f64::from(i32::MIN) || pen_y > f64::from(i32::MAX) {
                return TextMetrics::default();
            }
            previous = None;
            base_anchor = None;
            above_mark_offset = 0.0;
            below_mark_offset = 0.0;
            continue;
        }
        if character == '\t' {
            pen_x += f64::from(tab_width);
            if !pen_x.is_finite() || pen_x < f64::from(i32::MIN) || pen_x > f64::from(i32::MAX) {
                return TextMetrics::default();
            }
            previous = None;
            base_anchor = None;
            above_mark_offset = 0.0;
            below_mark_offset = 0.0;
            continue;
        }

        if !is_combining_mark(character) {
            if let Some(previous_character) = previous {
                let kerning = font.kerning(previous_character, character, pixel_size);
                if !kerning.is_finite() {
                    return TextMetrics::default();
                }
                pen_x += f64::from(kerning);
            }
        }
        let glyph = font.glyph(character, pixel_size);
        if !glyph.advance_width.is_finite() {
            return TextMetrics::default();
        }
        let mut glyph_left = (pen_x as f32) + glyph.xmin as f32;
        let mut glyph_top = (pen_y as f32) - glyph.ymin as f32 - glyph.height as f32;
        if !glyph_left.is_finite() || !glyph_top.is_finite() {
            return TextMetrics::default();
        }
        let combining = is_combining_mark(character);
        if combining {
            if let Some(anchor) = base_anchor {
                let offset = if is_below_combining_mark(character) {
                    below_mark_offset
                } else {
                    above_mark_offset
                };
                (glyph_left, glyph_top) = positioned_mark_origin_stacked(
                    character,
                    glyph.width,
                    glyph.height,
                    anchor,
                    offset,
                );
                if is_below_combining_mark(character) {
                    below_mark_offset = next_mark_stack_offset(offset, glyph.height);
                } else {
                    above_mark_offset = next_mark_stack_offset(offset, glyph.height);
                }
                if !glyph_left.is_finite()
                    || !glyph_top.is_finite()
                    || !above_mark_offset.is_finite()
                    || !below_mark_offset.is_finite()
                {
                    return TextMetrics::default();
                }
            }
        } else {
            base_anchor = Some((glyph_left, glyph_top, glyph.width, glyph.height));
            above_mark_offset = 0.0;
            below_mark_offset = 0.0;
        }
        for row in 0..glyph.height {
            for column in 0..glyph.width {
                let coverage = glyph.bitmap[row * glyph.width + column];
                if coverage == 0 {
                    continue;
                }
                let x = glyph_left + column as f32;
                let y = glyph_top + row as f32;
                if !x.is_finite()
                    || !y.is_finite()
                    || x < 0.0
                    || y < 0.0
                    || x >= target.width() as f32
                    || y >= target.height() as f32
                {
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
            pen_x += f64::from(glyph.advance_width);
            if !pen_x.is_finite() || pen_x < f64::from(i32::MIN) || pen_x > f64::from(i32::MAX) {
                return TextMetrics::default();
            }
            previous = Some(character);
        }
    }

    max_width = max_width.max(pen_x - origin_x);
    let Some(height) = checked_text_block_height(line_height, line_count as usize) else {
        return TextMetrics::default();
    };
    if !max_width.is_finite() || max_width < 0.0 || max_width > f64::from(f32::MAX) {
        return TextMetrics::default();
    }
    TextMetrics {
        width: max_width as f32,
        height,
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
    if !metrics.width.is_finite()
        || metrics.width < 0.0
        || !metrics.height.is_finite()
        || metrics.height < 0.0
    {
        return TextMetrics::default();
    }
    let Some(visible_clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height()))
    else {
        return metrics;
    };
    let line_height = font.line_height(pixel_size);
    let space_advance = font.glyph(' ', pixel_size).advance_width;
    let tab_width = (space_advance * 4.0).max(pixel_size);
    if !line_height.is_finite() || line_height <= 0.0 || !tab_width.is_finite() || tab_width <= 0.0
    {
        return TextMetrics::default();
    }

    let mut pen_x = f64::from(baseline.x);
    let mut pen_y = f64::from(baseline.y);
    let origin_x = pen_x;
    let mut previous = None;
    let mut base_anchor: Option<(f32, f32, usize, usize)> = None;
    let mut above_mark_offset = 0.0_f32;
    let mut below_mark_offset = 0.0_f32;

    for character in text.chars() {
        match character {
            '\n' => {
                pen_x = origin_x;
                pen_y += f64::from(line_height);
                if !pen_y.is_finite() || pen_y < f64::from(i32::MIN) || pen_y > f64::from(i32::MAX)
                {
                    return TextMetrics::default();
                }
                previous = None;
                base_anchor = None;
                above_mark_offset = 0.0;
                below_mark_offset = 0.0;
            }
            '\t' => {
                pen_x += f64::from(tab_width);
                if !pen_x.is_finite() || pen_x < f64::from(i32::MIN) || pen_x > f64::from(i32::MAX)
                {
                    return TextMetrics::default();
                }
                previous = None;
                base_anchor = None;
                above_mark_offset = 0.0;
                below_mark_offset = 0.0;
            }
            character => {
                if !is_combining_mark(character) {
                    if let Some(previous_character) = previous {
                        let kerning = font.kerning(previous_character, character, pixel_size);
                        if !kerning.is_finite() {
                            return TextMetrics::default();
                        }
                        pen_x += f64::from(kerning);
                    }
                }
                let glyph = font.glyph(character, pixel_size);
                if !glyph.advance_width.is_finite() {
                    return TextMetrics::default();
                }
                let mut left = (pen_x as f32) + glyph.xmin as f32;
                let mut top = (pen_y as f32) - glyph.ymin as f32 - glyph.height as f32;
                if !left.is_finite() || !top.is_finite() {
                    return TextMetrics::default();
                }
                let combining = is_combining_mark(character);
                if combining {
                    if let Some(anchor) = base_anchor {
                        let offset = if is_below_combining_mark(character) {
                            below_mark_offset
                        } else {
                            above_mark_offset
                        };
                        (left, top) = positioned_mark_origin_stacked(
                            character,
                            glyph.width,
                            glyph.height,
                            anchor,
                            offset,
                        );
                        if is_below_combining_mark(character) {
                            below_mark_offset = next_mark_stack_offset(offset, glyph.height);
                        } else {
                            above_mark_offset = next_mark_stack_offset(offset, glyph.height);
                        }
                        if !left.is_finite()
                            || !top.is_finite()
                            || !above_mark_offset.is_finite()
                            || !below_mark_offset.is_finite()
                        {
                            return TextMetrics::default();
                        }
                    }
                } else {
                    base_anchor = Some((left, top, glyph.width, glyph.height));
                    above_mark_offset = 0.0;
                    below_mark_offset = 0.0;
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
                    pen_x += f64::from(glyph.advance_width);
                    if !pen_x.is_finite()
                        || pen_x < f64::from(i32::MIN)
                        || pen_x > f64::from(i32::MAX)
                    {
                        return TextMetrics::default();
                    }
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
        0x0316..=0x0333 | 0x0339..=0x033C | 0x0345
        | 0x05B2..=0x05B5 | 0x05B7 | 0x05B9..=0x05BC | 0x05C5
        | 0x064D..=0x0650 | 0x0656..=0x065F | 0x093C | 0x09BC
        | 0x0A3C | 0x0ABC | 0x0B3C | 0x0C3C | 0x0CBC)
}

/// Positions a mark while keeping earlier marks in the same stack visible.
fn positioned_mark_origin_stacked(
    character: char,
    mark_width: usize,
    mark_height: usize,
    base: (f32, f32, usize, usize),
    stack_offset: f32,
) -> (f32, f32) {
    let (base_left, base_top, base_width, base_height) = base;
    let left = base_left + (base_width as f32 - mark_width as f32) * 0.5;
    let top = if is_below_combining_mark(character) {
        base_top + base_height as f32 - 1.0 + stack_offset
    } else {
        base_top - mark_height as f32 + 1.0 - stack_offset
    };
    (left, top)
}

/// Adds one pixel of separation between vertically stacked marks.
fn next_mark_stack_offset(current: f32, mark_height: usize) -> f32 {
    current + mark_height as f32 + 1.0
}

/// Computes a text block height without allowing floating-point overflow.
fn checked_text_block_height(line_height: f32, line_count: usize) -> Option<f32> {
    if !line_height.is_finite() || line_height < 0.0 {
        return None;
    }
    let height = f64::from(line_height) * line_count as f64;
    if !height.is_finite() || height > f64::from(f32::MAX) {
        return None;
    }
    Some(height as f32)
}

#[cfg(test)]
mod text_block_height_tests {
    use super::checked_text_block_height;

    #[test]
    fn text_block_height_rejects_non_finite_or_overflowing_results() {
        assert_eq!(checked_text_block_height(f32::NAN, 2), None);
        assert_eq!(checked_text_block_height(f32::INFINITY, 2), None);
        assert_eq!(checked_text_block_height(f32::MAX, 2), None);
    }

    #[test]
    fn text_block_height_accepts_finite_results() {
        assert_eq!(checked_text_block_height(12.5, 4), Some(50.0));
        assert_eq!(checked_text_block_height(0.0, 0), Some(0.0));
    }
}

fn layout_metrics(font: &FontFace, text: &str, pixel_size: f32) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() {
        return TextMetrics::default();
    }
    let space = font.glyph(' ', pixel_size);
    let tab_width = (space.advance_width * 4.0).max(pixel_size);
    let line_height = font.line_height(pixel_size);
    if !tab_width.is_finite() || tab_width <= 0.0 || !line_height.is_finite() || line_height <= 0.0
    {
        return TextMetrics::default();
    }

    let mut line_width = 0.0_f64;
    let mut max_width = 0.0_f64;
    let mut line_count = 1_u32;
    let mut previous = None;
    for character in text.chars() {
        match character {
            '\n' => {
                max_width = max_width.max(line_width);
                line_width = 0.0;
                let Some(next_line_count) = line_count.checked_add(1) else {
                    return TextMetrics::default();
                };
                line_count = next_line_count;
                previous = None;
            }
            '\t' => {
                line_width =
                    ((line_width / f64::from(tab_width)).floor() + 1.0) * f64::from(tab_width);
                previous = None;
            }
            character => {
                if !is_combining_mark(character) {
                    if let Some(previous_character) = previous {
                        let kerning = font.kerning(previous_character, character, pixel_size);
                        if !kerning.is_finite() {
                            return TextMetrics::default();
                        }
                        line_width += f64::from(kerning);
                    }
                    let advance = font.glyph(character, pixel_size).advance_width;
                    if !advance.is_finite() || advance < 0.0 {
                        return TextMetrics::default();
                    }
                    line_width += f64::from(advance);
                    previous = Some(character);
                }
            }
        }
        if !line_width.is_finite() || line_width < 0.0 || line_width > f64::from(f32::MAX) {
            return TextMetrics::default();
        }
    }
    max_width = max_width.max(line_width);
    let Some(height) = checked_text_block_height(line_height, line_count as usize) else {
        return TextMetrics::default();
    };
    if !max_width.is_finite() || max_width > f64::from(f32::MAX) {
        return TextMetrics::default();
    }
    TextMetrics {
        width: max_width as f32,
        height,
        line_count,
    }
}

#[cfg(test)]
mod tests {
    use super::{FontFace, MAX_CACHED_GLYPHS_PER_FONT};

    #[test]
    fn clearing_glyph_cache_releases_character_and_indexed_entries() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Hebrew font should load");
        let glyph_index = font.font.lookup_glyph_index('ש');
        assert_ne!(glyph_index, 0);

        let character_glyph = font.glyph('ש', 24.0);
        let indexed_glyph = font.glyph_index(glyph_index, 24.0);
        assert!(!character_glyph.bitmap.is_empty());
        assert!(!indexed_glyph.bitmap.is_empty());
        assert_eq!(font.cached_glyph_count(), 2);

        font.clear_glyph_cache();
        assert_eq!(font.cached_glyph_count(), 0);

        let _ = font.glyph('ש', 24.0);
        assert_eq!(font.cached_glyph_count(), 1);
    }

    #[test]
    fn glyph_caches_remain_bounded_after_many_distinct_entries() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Hebrew font should load");

        for codepoint in 0x1000..(0x1000 + MAX_CACHED_GLYPHS_PER_FONT as u32 + 1) {
            let character = char::from_u32(codepoint).expect("test codepoint should be valid");
            let _ = font.glyph(character, 1.0);
        }
        assert_eq!(font.glyph_cache.lock().unwrap().len(), 1);

        let valid_glyph_index = font.font.lookup_glyph_index('ש');
        assert_ne!(valid_glyph_index, 0);
        for size in 1..=MAX_CACHED_GLYPHS_PER_FONT + 1 {
            let _ = font.glyph_index(valid_glyph_index, size as f32);
        }
        assert_eq!(font.indexed_glyph_cache.lock().unwrap().len(), 1);
        assert_eq!(font.cached_glyph_count(), 2);
    }

    #[test]
    fn concurrent_glyph_cache_misses_share_the_inserted_bitmap() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = Arc::new(FontFace::from_bytes(bytes).expect("Hebrew font should load"));
        let glyph_index = font.font.lookup_glyph_index('ש');
        assert_ne!(glyph_index, 0);

        for indexed in [false, true] {
            font.clear_glyph_cache();
            let workers = 8;
            let barrier = Arc::new(Barrier::new(workers));
            let handles = (0..workers)
                .map(|_| {
                    let font = Arc::clone(&font);
                    let barrier = Arc::clone(&barrier);
                    thread::spawn(move || {
                        barrier.wait();
                        if indexed {
                            font.glyph_index(glyph_index, 28.0)
                        } else {
                            font.glyph('ש', 28.0)
                        }
                    })
                })
                .collect::<Vec<_>>();

            let glyphs = handles
                .into_iter()
                .map(|handle| handle.join().expect("glyph worker should finish"))
                .collect::<Vec<_>>();
            assert!(glyphs.iter().all(|glyph| !glyph.bitmap.is_empty()));
            assert!(
                glyphs.iter().all(|glyph| Arc::ptr_eq(glyph, &glyphs[0])),
                "concurrent misses should return the single cached glyph allocation"
            );
            assert_eq!(font.cached_glyph_count(), 1);
        }
    }

    #[test]
    fn malformed_shaped_metrics_are_rejected_before_rendering() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Hebrew font should load");
        let malformed = crate::ShapedText {
            glyphs: vec![crate::ShapedGlyph {
                glyph_id: 1,
                cluster: 0,
                x_advance: f32::INFINITY,
                y_advance: 0.0,
                x_offset: 0.0,
                y_offset: 0.0,
            }],
            advance_x: 12.0,
            advance_y: 0.0,
        };
        let mut target = crate::Framebuffer::new(32, 32, crate::Color::BLACK).expect("framebuffer");
        let before = target.pixels().to_vec();

        let metrics = super::draw_text_shaped_clipped(
            &mut target,
            &font,
            &malformed,
            crate::Point::new(4, 24),
            16.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 32, 32),
        );

        assert_eq!(metrics, super::TextMetrics::default());
        assert_eq!(target.pixels(), before);
        assert_eq!(font.cached_glyph_count(), 0);
    }

    #[test]
    fn overflowing_shaped_pen_position_is_rejected_before_rendering() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Hebrew font should load");
        let malformed = crate::ShapedText {
            glyphs: vec![
                crate::ShapedGlyph {
                    glyph_id: 1,
                    cluster: 0,
                    x_advance: f32::MAX,
                    y_advance: 0.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                },
                crate::ShapedGlyph {
                    glyph_id: 1,
                    cluster: 1,
                    x_advance: f32::MAX,
                    y_advance: 0.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                },
            ],
            advance_x: f32::MAX,
            advance_y: 0.0,
        };
        let mut target = crate::Framebuffer::new(32, 32, crate::Color::BLACK).expect("framebuffer");
        let before = target.pixels().to_vec();

        let metrics = super::draw_text_shaped_clipped(
            &mut target,
            &font,
            &malformed,
            crate::Point::new(4, 24),
            16.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 32, 32),
        );

        assert_eq!(metrics, super::TextMetrics::default());
        assert_eq!(target.pixels(), before);
        assert_eq!(font.cached_glyph_count(), 0);
    }

    #[test]
    fn shaped_run_rejects_overflowing_intermediate_pen_positions() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Hebrew font should load");
        let malformed = crate::ShapedText {
            glyphs: vec![
                crate::ShapedGlyph {
                    glyph_id: 1,
                    cluster: 0,
                    x_advance: f32::MAX,
                    y_advance: 0.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                },
                crate::ShapedGlyph {
                    glyph_id: 1,
                    cluster: 1,
                    x_advance: -f32::MAX,
                    y_advance: 0.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                },
            ],
            advance_x: 0.0,
            advance_y: 0.0,
        };
        let mut target = crate::Framebuffer::new(32, 32, crate::Color::BLACK).expect("framebuffer");
        let before = target.pixels().to_vec();

        let metrics = super::draw_text_shaped_clipped(
            &mut target,
            &font,
            &malformed,
            crate::Point::new(1, 24),
            16.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 32, 32),
        );

        assert_eq!(metrics, super::TextMetrics::default());
        assert_eq!(target.pixels(), before);
        assert_eq!(font.cached_glyph_count(), 0);
    }

    #[test]
    fn transformed_shaped_renderer_rejects_malformed_metrics_before_rendering() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Hebrew font should load");
        let malformed = crate::ShapedText {
            glyphs: vec![crate::ShapedGlyph {
                glyph_id: 1,
                cluster: 0,
                x_advance: 0.0,
                y_advance: f32::NAN,
                x_offset: 0.0,
                y_offset: 0.0,
            }],
            advance_x: 0.0,
            advance_y: 0.0,
        };
        let mut target = crate::Framebuffer::new(32, 32, crate::Color::BLACK).expect("framebuffer");
        let before = target.pixels().to_vec();

        let metrics = super::draw_text_shaped_transformed_clipped(
            &mut target,
            &font,
            &malformed,
            crate::Point::new(4, 24),
            16.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 32, 32),
            crate::Transform2D::IDENTITY,
        );

        assert_eq!(metrics, super::TextMetrics::default());
        assert_eq!(target.pixels(), before);
        assert_eq!(font.cached_glyph_count(), 0);
    }

    #[test]
    fn licensed_noto_hebrew_font_shapes_and_rasterizes_real_glyphs() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Noto Sans Hebrew should load");
        let shaped = font
            .shape_text_with_direction("שלום", 28.0, crate::TextDirection::RightToLeft)
            .expect("Hebrew run should shape");

        assert!(!shaped.glyphs.is_empty());
        assert!(shaped.advance_x.is_finite());
        assert!(shaped.advance_x.abs() > 0.0);

        let mut target =
            crate::Framebuffer::new(160, 64, crate::Color::BLACK).expect("framebuffer");
        let metrics = super::draw_text_shaped_clipped(
            &mut target,
            &font,
            &shaped,
            crate::Point::new(42, 46),
            28.0,
            crate::Color::WHITE,
            crate::Rect::new(40, 0, 72, 64),
        );

        assert!(metrics.width > 0.0);
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| *pixel != crate::Color::BLACK));
        assert!(target.pixels().iter().enumerate().all(|(index, pixel)| {
            let x = index % target.width() as usize;
            (40..112).contains(&x) || *pixel == crate::Color::BLACK
        }));
    }

    #[test]
    fn licensed_noto_hebrew_rasterization_is_deterministic_across_cache_clears() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Noto Sans Hebrew should load");
        let shaped = font
            .shape_text_with_direction("שלום", 28.0, crate::TextDirection::RightToLeft)
            .expect("Hebrew run should shape");
        let clip = crate::Rect::new(24, 4, 88, 56);
        let baseline = crate::Point::new(48, 44);

        let mut cold_target =
            crate::Framebuffer::new(144, 64, crate::Color::BLACK).expect("framebuffer");
        super::draw_text_shaped_clipped(
            &mut cold_target,
            &font,
            &shaped,
            baseline,
            28.0,
            crate::Color::WHITE,
            clip,
        );

        font.clear_glyph_cache();

        let mut warm_target =
            crate::Framebuffer::new(144, 64, crate::Color::BLACK).expect("framebuffer");
        super::draw_text_shaped_clipped(
            &mut warm_target,
            &font,
            &shaped,
            baseline,
            28.0,
            crate::Color::WHITE,
            clip,
        );

        assert_eq!(
            cold_target.pixels(),
            warm_target.pixels(),
            "glyph cache state must not change rendered text pixels"
        );
        assert!(
            cold_target
                .pixels()
                .iter()
                .any(|pixel| *pixel != crate::Color::BLACK),
            "the licensed fixture must produce visible glyph coverage"
        );
        assert!(cold_target
            .pixels()
            .iter()
            .enumerate()
            .all(|(index, pixel)| {
                let x = index % cold_target.width() as usize;
                let y = index / cold_target.width() as usize;
                (24..112).contains(&x) && (4..60).contains(&y) || *pixel == crate::Color::BLACK
            }));
    }

    #[test]
    fn shaped_text_clipping_and_affine_rasterization_match_pixel_references() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Hebrew font should load");
        let shaped = font
            .shape_text_with_direction("שלום", 28.0, crate::TextDirection::RightToLeft)
            .expect("Hebrew run should shape");

        let background = crate::Color::rgba(18, 27, 36, 91);
        let color = crate::Color::rgba(245, 180, 40, 192);
        let clip = crate::Rect::new(18, 6, 84, 50);
        let baseline = crate::Point::new(42, 46);

        let mut clipped = crate::Framebuffer::new(128, 64, background).expect("framebuffer");
        super::draw_text_shaped_clipped(&mut clipped, &font, &shaped, baseline, 28.0, color, clip);

        let mut unclipped =
            crate::Framebuffer::new(128, 64, background).expect("reference framebuffer");
        super::draw_text_shaped_clipped(
            &mut unclipped,
            &font,
            &shaped,
            baseline,
            28.0,
            color,
            crate::Rect::new(0, 0, 128, 64),
        );
        assert!(clipped.pixels().iter().enumerate().all(|(index, pixel)| {
            let x = index % clipped.width() as usize;
            let y = index / clipped.width() as usize;
            if (18..102).contains(&x) && (6..56).contains(&y) {
                *pixel == unclipped.pixels()[index]
            } else {
                *pixel == background
            }
        }));

        let mut transformed = crate::Framebuffer::new(128, 64, background).expect("framebuffer");
        let transform =
            crate::Transform2D::translation(7.0, -3.0).then(crate::Transform2D::rotation(0.11));
        super::draw_text_shaped_transformed_clipped(
            &mut transformed,
            &font,
            &shaped,
            baseline,
            28.0,
            color,
            clip,
            transform,
        );

        let mut transformed_reference =
            crate::Framebuffer::new(128, 64, background).expect("reference framebuffer");
        super::draw_text_shaped_transformed_clipped(
            &mut transformed_reference,
            &font,
            &shaped,
            baseline,
            28.0,
            color,
            crate::Rect::new(0, 0, 128, 64),
            transform,
        );
        assert!(transformed
            .pixels()
            .iter()
            .enumerate()
            .all(|(index, pixel)| {
                let x = index % transformed.width() as usize;
                let y = index / transformed.width() as usize;
                if (18..102).contains(&x) && (6..56).contains(&y) {
                    *pixel == transformed_reference.pixels()[index]
                } else {
                    *pixel == background
                }
            }));

        assert!(
            clipped.pixels().iter().any(|pixel| *pixel != background),
            "clipped shaped text must produce visible pixels"
        );
        assert!(
            transformed
                .pixels()
                .iter()
                .any(|pixel| *pixel != background),
            "transformed shaped text must produce visible pixels"
        );
    }

    #[test]
    fn transformed_shaped_text_identity_matches_untransformed_pixels_exactly() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Hebrew font should load");
        let shaped = font
            .shape_text_with_direction("שלום", 26.0, crate::TextDirection::RightToLeft)
            .expect("Hebrew run should shape");
        let clip = crate::Rect::new(10, 4, 100, 56);
        let baseline = crate::Point::new(36, 44);
        let color = crate::Color::rgba(90, 190, 245, 173);
        let background = crate::Color::rgba(12, 20, 28, 91);

        let mut direct = crate::Framebuffer::new(128, 64, background).expect("framebuffer");
        let mut identity = crate::Framebuffer::new(128, 64, background).expect("framebuffer");
        super::draw_text_shaped_clipped(&mut direct, &font, &shaped, baseline, 26.0, color, clip);
        super::draw_text_shaped_transformed_clipped(
            &mut identity,
            &font,
            &shaped,
            baseline,
            26.0,
            color,
            clip,
            crate::Transform2D::IDENTITY,
        );

        assert_eq!(
            identity.pixels(),
            direct.pixels(),
            "identity affine transform must preserve every text pixel and alpha value"
        );
        assert!(direct.pixels().iter().any(|pixel| *pixel != background));
    }

    #[test]
    fn fallback_keeps_combining_sequences_on_one_font_face() {
        use std::sync::Arc;

        let hebrew_bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let arabic_bytes = include_bytes!("../tests/fixtures/NotoSansArabic[wdth,wght].ttf");
        let hebrew = Arc::new(FontFace::from_bytes(hebrew_bytes).expect("Hebrew font"));
        let arabic = Arc::new(FontFace::from_bytes(arabic_bytes).expect("Arabic font"));
        let mut family = super::FontFamily::new(Arc::clone(&hebrew));
        family.add_fallback(Arc::clone(&arabic));

        let hebrew_cluster = "שָׁ";
        let selected = super::fallback_face_for_cluster(&family, None, hebrew_cluster, &hebrew);
        assert!(Arc::ptr_eq(&selected, &hebrew));
        assert!(hebrew_cluster
            .chars()
            .all(|character| hebrew.has_glyph(character)));

        let arabic_cluster = "لَا";
        let selected = super::fallback_face_for_cluster(&family, None, arabic_cluster, &hebrew);
        assert!(Arc::ptr_eq(&selected, &arabic));
        assert!(arabic_cluster
            .chars()
            .all(|character| arabic.has_glyph(character)));
    }

    #[test]
    fn simple_and_shaped_fallback_render_combining_clusters() {
        use std::sync::Arc;

        let hebrew_bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let arabic_bytes = include_bytes!("../tests/fixtures/NotoSansArabic[wdth,wght].ttf");
        let hebrew = Arc::new(FontFace::from_bytes(hebrew_bytes).expect("Hebrew font"));
        let arabic = Arc::new(FontFace::from_bytes(arabic_bytes).expect("Arabic font"));
        let mut family = super::FontFamily::new(Arc::clone(&hebrew));
        family.add_fallback(Arc::clone(&arabic));
        let text = "שָׁ لَا";

        let mut simple_target =
            crate::Framebuffer::new(320, 96, crate::Color::BLACK).expect("simple framebuffer");
        let simple_metrics = super::draw_text_with_fallback_clipped(
            &mut simple_target,
            &family,
            text,
            crate::Point::new(12, 64),
            30.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 320, 96),
        );
        assert!(simple_metrics.width.is_finite() && simple_metrics.width > 0.0);
        assert!(simple_target
            .pixels()
            .iter()
            .any(|pixel| *pixel != crate::Color::BLACK));

        let mut shaped_target =
            crate::Framebuffer::new(320, 96, crate::Color::BLACK).expect("shaped framebuffer");
        let shaped_metrics = super::draw_text_with_fallback_shaped_clipped(
            &mut shaped_target,
            &family,
            text,
            crate::Point::new(12, 64),
            30.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 320, 96),
        );
        assert!(shaped_metrics.width.is_finite() && shaped_metrics.width > 0.0);
        assert!(shaped_target
            .pixels()
            .iter()
            .any(|pixel| *pixel != crate::Color::BLACK));
    }

    #[test]
    fn licensed_hebrew_and_arabic_fallback_faces_render_one_bidi_paragraph() {
        use std::sync::Arc;

        let hebrew_bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let arabic_bytes = include_bytes!("../tests/fixtures/NotoSansArabic[wdth,wght].ttf");
        let hebrew = Arc::new(FontFace::from_bytes(hebrew_bytes).expect("Hebrew font"));
        let arabic = Arc::new(FontFace::from_bytes(arabic_bytes).expect("Arabic font"));

        let mut family = super::FontFamily::new(Arc::clone(&hebrew));
        family.add_fallback(Arc::clone(&arabic));

        assert!(std::ptr::eq(
            family
                .font_for('ש')
                .expect("Hebrew face should be selected"),
            hebrew.as_ref()
        ));
        assert!(std::ptr::eq(
            family
                .font_for('ل')
                .expect("Arabic fallback should be selected"),
            arabic.as_ref()
        ));

        let mut target =
            crate::Framebuffer::new(320, 96, crate::Color::BLACK).expect("framebuffer");
        let metrics = super::draw_text_with_fallback_bidi_clipped(
            &mut target,
            &family,
            "שלום  لا",
            crate::Point::new(24, 64),
            30.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 320, 96),
        );

        assert_eq!(metrics.line_count, 1);
        assert!(metrics.width.is_finite() && metrics.width > 0.0);
        assert!(
            target
                .pixels()
                .iter()
                .any(|pixel| *pixel != crate::Color::BLACK),
            "mixed Hebrew and Arabic fallback runs should rasterize into the framebuffer"
        );
    }

    #[test]
    fn licensed_noto_arabic_applies_contextual_forms_and_rasterizes() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansArabic[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Noto Sans Arabic should load");
        let shaped = font
            .shape_text_with_direction("سلام", 30.0, crate::TextDirection::RightToLeft)
            .expect("Arabic run should shape");

        assert!(!shaped.glyphs.is_empty());
        let unjoined = font
            .shape_text_with_direction("سلام", 30.0, crate::TextDirection::LeftToRight)
            .expect("comparison run should shape");
        let joined_ids = shaped
            .glyphs
            .iter()
            .map(|glyph| glyph.glyph_id)
            .collect::<Vec<_>>();
        let unjoined_ids = unjoined
            .glyphs
            .iter()
            .map(|glyph| glyph.glyph_id)
            .collect::<Vec<_>>();
        assert_ne!(
            joined_ids, unjoined_ids,
            "RTL Arabic shaping should select contextual joining forms"
        );
        assert!(shaped.advance_x.is_finite());
        assert!(shaped.advance_x.abs() > 0.0);

        let mut target =
            crate::Framebuffer::new(160, 64, crate::Color::BLACK).expect("framebuffer");
        super::draw_text_shaped_clipped(
            &mut target,
            &font,
            &shaped,
            crate::Point::new(80, 46),
            30.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 160, 64),
        );
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| *pixel != crate::Color::BLACK));
    }

    #[test]
    fn licensed_noto_arabic_shapes_lam_alef_with_rtl_context() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansArabic[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Noto Sans Arabic should load");
        let rtl = font
            .shape_text_with_direction("لا", 30.0, crate::TextDirection::RightToLeft)
            .expect("lam-alef should shape");
        let ltr = font
            .shape_text_with_direction("لا", 30.0, crate::TextDirection::LeftToRight)
            .expect("comparison run should shape");

        let rtl_ids = rtl
            .glyphs
            .iter()
            .map(|glyph| glyph.glyph_id)
            .collect::<Vec<_>>();
        let ltr_ids = ltr
            .glyphs
            .iter()
            .map(|glyph| glyph.glyph_id)
            .collect::<Vec<_>>();

        assert_ne!(
            rtl_ids, ltr_ids,
            "Arabic lam-alef must use direction-aware OpenType substitutions"
        );
        assert!(rtl.advance_x.is_finite());
        assert!(rtl.advance_x.abs() > 0.0);
    }

    #[test]
    fn licensed_noto_hebrew_applies_gpos_mark_to_base_positioning() {
        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = FontFace::from_bytes(bytes).expect("Noto Sans Hebrew should load");
        let shaped = font
            .shape_text_with_direction("שָׁ", 32.0, crate::TextDirection::RightToLeft)
            .expect("Hebrew base and marks should shape");

        assert!(
            shaped.glyphs.len() >= 2,
            "the base and combining marks should remain represented in the shaped run"
        );
        assert!(
            shaped
                .glyphs
                .iter()
                .any(|glyph| glyph.x_offset != 0.0 || glyph.y_offset != 0.0),
            "GPOS mark attachment should produce a positioned mark offset"
        );

        let mut target = crate::Framebuffer::new(96, 64, crate::Color::BLACK).expect("framebuffer");
        super::draw_text_shaped_clipped(
            &mut target,
            &font,
            &shaped,
            crate::Point::new(48, 48),
            32.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 96, 64),
        );
        assert!(
            target
                .pixels()
                .iter()
                .any(|pixel| *pixel != crate::Color::BLACK),
            "positioned Hebrew marks should reach the CPU framebuffer"
        );
    }

    #[test]
    fn combining_mark_ranges_cover_common_latin_and_hebrew_marks() {
        assert!(super::is_combining_mark('\u{0301}'));
        assert!(super::is_combining_mark('\u{0327}'));
        assert!(super::is_combining_mark('\u{05B0}'));
        assert!(super::is_combining_mark('\u{20DD}'));
        assert!(!super::is_combining_mark('A'));
        assert!(!super::is_combining_mark('é'));
    }

    #[test]
    fn combining_mark_position_is_centered_over_base() {
        let origin =
            super::positioned_mark_origin_stacked('\u{0301}', 3, 2, (10.0, 20.0, 8, 12), 0.0);
        assert_eq!(origin, (12.5, 19.0));
        let below =
            super::positioned_mark_origin_stacked('\u{0327}', 3, 2, (10.0, 20.0, 8, 12), 0.0);
        assert_eq!(below, (12.5, 31.0));
    }

    #[test]
    fn multiple_combining_marks_stack_without_overlapping() {
        let base = (10.0, 20.0, 8, 12);
        let first_above = super::positioned_mark_origin_stacked('\u{0301}', 3, 2, base, 0.0);
        let second_above = super::positioned_mark_origin_stacked(
            '\u{0308}',
            3,
            2,
            base,
            super::next_mark_stack_offset(0.0, 2),
        );
        assert_eq!(first_above, (12.5, 19.0));
        assert_eq!(second_above, (12.5, 16.0));

        let first_below = super::positioned_mark_origin_stacked('\u{0327}', 3, 2, base, 0.0);
        let second_below = super::positioned_mark_origin_stacked(
            '\u{0328}',
            3,
            2,
            base,
            super::next_mark_stack_offset(0.0, 2),
        );
        assert_eq!(first_below, (12.5, 31.0));
        assert_eq!(second_below, (12.5, 34.0));
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
    fn shaped_fallback_measurement_handles_empty_and_missing_fonts() {
        let family = super::FontFamily::default();
        assert_eq!(super::measure_shaped_fallback_line(&family, "", 16.0), 0.0);
        assert!(super::measure_shaped_fallback_line(&family, "text", 16.0).is_infinite());
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
    fn tab_stops_advance_to_the_next_four_space_boundary() {
        assert_eq!(super::next_tab_stop(0.0, 4.0), Some(4.0));
        assert_eq!(super::next_tab_stop(1.0, 4.0), Some(4.0));
        assert_eq!(super::next_tab_stop(3.99, 4.0), Some(4.0));
        assert_eq!(super::next_tab_stop(4.0, 4.0), Some(8.0));
        assert_eq!(super::next_tab_stop(8.0, 4.0), Some(12.0));
        assert_eq!(super::next_tab_stop(-1.0, 4.0), Some(0.0));
        assert_eq!(super::next_tab_stop(1.0, 0.0), None);
        assert_eq!(super::next_tab_stop(f64::NAN, 4.0), None);
        assert_eq!(super::next_tab_stop(1.0, f64::INFINITY), None);
    }

    #[test]
    fn fallback_measurement_handles_real_font_tabs_and_multiple_lines() {
        use std::sync::Arc;

        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = Arc::new(FontFace::from_bytes(bytes).expect("Hebrew font should load"));
        let family = super::FontFamily::new(font);

        let single_line = family.measure("שלום", 24.0);
        let multi_line = family.measure("שלום\tשלום\n\nשלום", 24.0);

        assert!(single_line.width.is_finite() && single_line.width > 0.0);
        assert!(single_line.height.is_finite() && single_line.height > 0.0);
        assert_eq!(single_line.line_count, 1);

        assert!(multi_line.width.is_finite() && multi_line.width > single_line.width);
        assert!(multi_line.height.is_finite() && multi_line.height > single_line.height);
        assert_eq!(multi_line.line_count, 3);
    }

    #[test]
    fn fallback_drawing_renders_real_font_tabs_and_blank_lines() {
        use std::sync::Arc;

        let bytes = include_bytes!("../tests/fixtures/NotoSansHebrew[wdth,wght].ttf");
        let font = Arc::new(FontFace::from_bytes(bytes).expect("Hebrew font should load"));
        let family = super::FontFamily::new(font);
        let mut target =
            crate::Framebuffer::new(320, 128, crate::Color::BLACK).expect("framebuffer");

        let metrics = super::draw_text_with_fallback_clipped(
            &mut target,
            &family,
            "שלום\tשלום\n\nשלום",
            crate::Point::new(8, 32),
            24.0,
            crate::Color::WHITE,
            crate::Rect::new(0, 0, 320, 128),
        );

        assert_eq!(metrics.line_count, 3);
        assert!(metrics.width.is_finite() && metrics.width > 0.0);
        assert!(metrics.height.is_finite() && metrics.height > 0.0);

        let count_ink = |start_y: usize, end_y: usize| {
            (start_y..end_y)
                .flat_map(|y| (0..320).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    target.get_pixel(*x as u32, *y as u32) != Some(crate::Color::BLACK)
                })
                .count()
        };

        assert!(count_ink(0, 40) > 0, "first line should render");
        assert_eq!(count_ink(40, 60), 0, "blank second line should stay empty");
        assert!(
            count_ink(60, 100) > 0,
            "third line should render after blank line"
        );
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
    fn shaped_fallback_wrapping_keeps_grapheme_like_sequences_together() {
        assert_eq!(
            super::fallback_text_clusters("a\u{0301}b").collect::<Vec<_>>(),
            ["a\u{0301}", "b"]
        );
        assert_eq!(
            super::fallback_text_clusters("👩‍💻x").collect::<Vec<_>>(),
            ["👩‍💻", "x"]
        );
        assert_eq!(
            super::fallback_text_clusters("🇫🇷!").collect::<Vec<_>>(),
            ["🇫🇷", "!"]
        );
        assert_eq!(
            super::fallback_text_clusters("👍🏽!").collect::<Vec<_>>(),
            ["👍🏽", "!"]
        );
        assert_eq!(
            super::fallback_text_clusters("🏴\u{E0067}\u{E0062}\u{E007F}!").collect::<Vec<_>>(),
            ["🏴\u{E0067}\u{E0062}\u{E007F}", "!"]
        );
        assert_eq!(
            super::fallback_text_clusters("✈️x").collect::<Vec<_>>(),
            ["✈️", "x"]
        );
        assert_eq!(
            super::fallback_text_clusters("👨‍👩‍👧‍👦!").collect::<Vec<_>>(),
            ["👨‍👩‍👧‍👦", "!"]
        );
        assert_eq!(
            super::fallback_text_clusters("1️⃣!").collect::<Vec<_>>(),
            ["1️⃣", "!"]
        );
        assert_eq!(
            super::fallback_text_clusters("क्‍ष!").collect::<Vec<_>>(),
            ["क्‍ष", "!"]
        );
    }

    #[test]
    fn shaped_wrapping_never_splits_extended_grapheme_clusters() {
        let measure = |part: &str| part.chars().count() as f32;
        assert_eq!(
            super::wrap_paragraph_preserving_whitespace("a\u{0301}b", 1.0, measure),
            ["a\u{0301}", "b"]
        );
        assert_eq!(
            super::wrap_paragraph_preserving_whitespace("👨‍👩‍👧‍👦!", 1.0, measure),
            ["👨‍👩‍👧‍👦", "!"]
        );
        assert_eq!(
            super::wrap_paragraph_preserving_whitespace("क्‍ष!", 1.0, measure),
            ["क्‍ष", "!"]
        );
    }

    #[test]
    fn shaped_wrapping_preserves_repeated_leading_and_trailing_whitespace() {
        let lines = super::wrap_paragraph_preserving_whitespace("  a  b  ", 4.0, |part| {
            part.chars().count() as f32
        });
        assert_eq!(lines, ["  a  ", "b  "]);
    }

    #[test]
    fn shaped_wrapping_preserves_tabs_and_blank_paragraphs() {
        let lines = super::wrap_paragraph_preserving_whitespace("a\t\tb", 3.0, |part| {
            part.chars().count() as f32
        });
        assert_eq!(lines, ["a\t\t", "b"]);
        assert_eq!(
            super::wrap_paragraph_preserving_whitespace("", 10.0, |_| 0.0),
            [""]
        );
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
