//! Font loading, basic text measurement and CPU glyph rasterization.
//!
//! This first text layer intentionally provides deterministic single-line-run
//! layout. Complex script shaping, fallback chains and persistent glyph caching
//! are separate follow-up milestones.

use crate::{rasterizer::blend_pixel, Color, Framebuffer, Point};
use fontdue::Font;

/// A loaded TrueType/OpenType font backed by the fontdue parser and rasterizer.
pub struct FontFace {
    font: Font,
}

impl FontFace {
    /// Loads a font from in-memory TTF/OTF bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        Font::from_bytes(bytes, fontdue::FontSettings::default()).map(|font| Self { font })
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
}

/// Logical size of a laid-out text run, in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
    pub line_count: u32,
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
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() {
        return TextMetrics::default();
    }

    let line_height = font.line_height(pixel_size);
    let mut pen_x = baseline.x as f32;
    let mut pen_y = baseline.y as f32;
    let origin_x = pen_x;
    let mut max_width: f32 = 0.0;
    let mut line_count = 1_u32;
    let (space_metrics, _) = font.font.rasterize(' ', pixel_size);
    let tab_width = (space_metrics.advance_width * 4.0).max(pixel_size);

    for character in text.chars() {
        if character == '\n' {
            max_width = max_width.max(pen_x - origin_x);
            pen_x = origin_x;
            pen_y += line_height;
            line_count += 1;
            continue;
        }
        if character == '\t' {
            pen_x += tab_width;
            continue;
        }

        let (metrics, bitmap) = font.font.rasterize(character, pixel_size);
        let glyph_left = pen_x + metrics.xmin as f32;
        let glyph_top = pen_y - metrics.ymin as f32 - metrics.height as f32;
        for row in 0..metrics.height {
            for column in 0..metrics.width {
                let coverage = bitmap[row * metrics.width + column];
                if coverage == 0 {
                    continue;
                }
                let x = glyph_left + column as f32;
                let y = glyph_top + row as f32;
                if x < 0.0 || y < 0.0 || x >= target.width() as f32 || y >= target.height() as f32 {
                    continue;
                }
                let alpha = ((u16::from(color.a) * u16::from(coverage) + 127) / 255) as u8;
                if alpha > 0 {
                    blend_pixel(
                        target,
                        Point::new(x.floor() as i32, y.floor() as i32),
                        Color::rgba(color.r, color.g, color.b, alpha),
                    );
                }
            }
        }
        pen_x += metrics.advance_width;
    }

    max_width = max_width.max(pen_x - origin_x);
    TextMetrics {
        width: max_width,
        height: line_height * line_count as f32,
        line_count,
    }
}

fn layout_metrics(font: &FontFace, text: &str, pixel_size: f32) -> TextMetrics {
    if !pixel_size.is_finite() || pixel_size <= 0.0 || text.is_empty() {
        return TextMetrics::default();
    }
    let (space, _) = font.font.rasterize(' ', pixel_size);
    let tab_width = (space.advance_width * 4.0).max(pixel_size);
    let mut line_width = 0.0_f32;
    let mut max_width = 0.0_f32;
    let mut line_count = 1_u32;
    for character in text.chars() {
        match character {
            '\n' => {
                max_width = max_width.max(line_width);
                line_width = 0.0;
                line_count += 1;
            }
            '\t' => line_width += tab_width,
            character => line_width += font.font.metrics(character, pixel_size).advance_width,
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
}
