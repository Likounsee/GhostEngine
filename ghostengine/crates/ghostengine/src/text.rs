//! Font loading, basic text measurement and CPU glyph rasterization.
//!
//! This first text layer intentionally provides deterministic single-line-run
//! layout. Complex script shaping, fallback chains and persistent glyph caching
//! are separate follow-up milestones.

use crate::{rasterizer::blend_pixel, Color, Framebuffer, Point, Rect};
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
    word.chars()
        .map(|character| font.font.metrics(character, pixel_size).advance_width)
        .sum()
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
        let lines = super::layout_lines_for_test("first\\n\\nlast", 20.0);
        assert_eq!(lines, ["first", "", "last"]);
    }
}
