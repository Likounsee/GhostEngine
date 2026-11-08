//! OpenType text shaping primitives.
//!
//! This module computes glyph substitutions and positioning with Rustybuzz.
//! It does not rasterize glyphs: all drawing remains in GhostEngine's CPU renderer.

use std::sync::Arc;

/// Explicit horizontal direction for an OpenType shaping run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDirection {
    #[default]
    LeftToRight,
    RightToLeft,
}

/// A font-backed OpenType shaper.
#[derive(Clone)]
pub struct OpenTypeShaper {
    font_data: Arc<[u8]>,
}

/// A glyph produced by OpenType shaping, with positions expressed in pixels.
///
/// `cluster` is a byte offset into the original UTF-8 input. Y offsets and
/// advances use OpenType's upward-positive coordinate convention.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: f32,
    pub y_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

/// The shaped output for one text run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapedText {
    pub glyphs: Vec<ShapedGlyph>,
    pub advance_x: f32,
    pub advance_y: f32,
}

/// A logical UTF-8 range in the visual order of a bidirectional paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiRun {
    pub byte_range: std::ops::Range<usize>,
    pub direction: TextDirection,
}

/// Resolves Unicode bidi levels and returns directional runs in visual order.
///
/// Ranges always refer to the original UTF-8 string. The function does not
/// reverse UTF-8 bytes or characters; each range must be shaped independently
/// with its returned direction.
pub fn resolve_bidi_runs(text: &str) -> Vec<BidiRun> {
    let bidi = unicode_bidi::BidiInfo::new(text, None);
    let mut output = Vec::new();
    for paragraph in &bidi.paragraphs {
        let (_, runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
        for range in runs {
            let direction = if bidi
                .levels
                .get(range.start)
                .is_some_and(|level| level.is_rtl())
            {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            };
            output.push(BidiRun {
                byte_range: range,
                direction,
            });
        }
    }
    output
}

/// Errors returned while shaping text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapingError {
    InvalidFont,
    InvalidPixelSize,
}

impl OpenTypeShaper {
    /// Creates a shaper from a TrueType/OpenType font.
    ///
    /// The font bytes are retained so shaping can be repeated without requiring
    /// callers to keep their original buffer alive.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ShapingError> {
        if rustybuzz::Face::from_slice(bytes, 0).is_none() {
            return Err(ShapingError::InvalidFont);
        }
        Ok(Self {
            font_data: Arc::from(bytes),
        })
    }

    /// Shapes a UTF-8 run using the font's OpenType layout tables.
    ///
    /// Rustybuzz performs script-property guessing, GSUB substitutions and
    /// GPOS positioning. Returned glyph IDs can be passed to a glyph-index
    /// rasterization path; this method deliberately does not draw pixels.
    pub fn shape(&self, text: &str, pixel_size: f32) -> Result<ShapedText, ShapingError> {
        self.shape_with_direction(text, pixel_size, TextDirection::LeftToRight)
    }

    /// Shapes a run with an explicitly selected horizontal direction.
    ///
    /// Right-to-left shaping changes glyph order and applies direction-sensitive
    /// OpenType substitutions/positioning. Paragraph-level bidi resolution is
    /// handled by the text layout layer rather than guessed here.
    pub fn shape_with_direction(
        &self,
        text: &str,
        pixel_size: f32,
        direction: TextDirection,
    ) -> Result<ShapedText, ShapingError> {
        if !pixel_size.is_finite() || pixel_size <= 0.0 {
            return Err(ShapingError::InvalidPixelSize);
        }
        if text.is_empty() {
            return Ok(ShapedText::default());
        }

        let face =
            rustybuzz::Face::from_slice(&self.font_data, 0).ok_or(ShapingError::InvalidFont)?;
        let units_per_em = face.units_per_em() as f32;
        if !units_per_em.is_finite() || units_per_em <= 0.0 {
            return Err(ShapingError::InvalidFont);
        }
        let scale = pixel_size / units_per_em;

        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        buffer.set_direction(match direction {
            TextDirection::LeftToRight => rustybuzz::Direction::LeftToRight,
            TextDirection::RightToLeft => rustybuzz::Direction::RightToLeft,
        });
        let shaped = rustybuzz::shape(&face, &[], buffer);

        let glyphs = shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions())
            .map(|(info, position)| ShapedGlyph {
                glyph_id: info.glyph_id,
                cluster: info.cluster,
                x_advance: position.x_advance as f32 * scale,
                y_advance: position.y_advance as f32 * scale,
                x_offset: position.x_offset as f32 * scale,
                y_offset: position.y_offset as f32 * scale,
            })
            .collect::<Vec<_>>();

        let advance_x = glyphs.iter().map(|glyph| glyph.x_advance).sum();
        let advance_y = glyphs.iter().map(|glyph| glyph.y_advance).sum();
        Ok(ShapedText {
            glyphs,
            advance_x,
            advance_y,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{OpenTypeShaper, ShapedText, ShapingError, TextDirection};

    #[test]
    fn bidi_runs_keep_original_ranges_and_visual_order() {
        let text = "abc אבג 123";
        let runs = super::resolve_bidi_runs(text);
        assert!(!runs.is_empty());
        for run in &runs {
            assert!(text.get(run.byte_range.clone()).is_some());
        }
        assert!(runs
            .iter()
            .any(|run| run.direction == TextDirection::RightToLeft));
        assert!(runs
            .iter()
            .any(|run| run.direction == TextDirection::LeftToRight));
    }

    #[test]
    fn pure_ltr_text_resolves_to_ltr_runs() {
        let runs = super::resolve_bidi_runs("GhostEngine 123");
        assert!(!runs.is_empty());
        assert!(runs
            .iter()
            .all(|run| run.direction == TextDirection::LeftToRight));
    }

    #[test]
    fn bidi_visual_runs_cover_each_utf8_character_exactly_once() {
        // Include multibyte Latin, Hebrew, Arabic digits, punctuation and emoji.
        // The resolver must return byte ranges, not character indices or reversed
        // UTF-8 fragments, even when the visual order differs from logical order.
        let text = "Café (שלום) ١٢٣ — 👩‍💻";
        let runs = super::resolve_bidi_runs(text);
        assert!(!runs.is_empty());

        let mut coverage = vec![0_u8; text.len()];
        for run in &runs {
            assert!(run.byte_range.start < run.byte_range.end);
            let slice = text
                .get(run.byte_range.clone())
                .expect("every bidi run must be on UTF-8 boundaries");
            assert!(!slice.is_empty());
            for byte in run.byte_range.clone() {
                coverage[byte] = coverage[byte].saturating_add(1);
            }
        }

        for (index, byte) in text.bytes().enumerate() {
            assert_eq!(
                coverage[index], 1,
                "byte {index} ({byte:#04x}) must belong to exactly one visual run"
            );
        }
    }

    #[test]
    fn bidi_visual_run_ranges_never_split_a_unicode_scalar() {
        let text = "Aé אבג العربية 𐐷 Z";
        for run in super::resolve_bidi_runs(text) {
            let slice = text
                .get(run.byte_range.clone())
                .expect("bidi range must start and end at UTF-8 boundaries");
            assert!(slice.chars().count() > 0);
            assert!(run.byte_range.end <= text.len());
        }
    }

    #[test]
    fn empty_bidi_text_has_no_visual_runs() {
        assert!(super::resolve_bidi_runs("").is_empty());
    }

    #[test]
    fn rejects_invalid_font_data() {
        assert_eq!(
            OpenTypeShaper::from_bytes(b"not a font").err(),
            Some(ShapingError::InvalidFont)
        );
    }

    #[test]
    fn shaped_text_default_is_empty_and_zero_advance() {
        assert_eq!(
            ShapedText::default(),
            ShapedText {
                glyphs: Vec::new(),
                advance_x: 0.0,
                advance_y: 0.0,
            }
        );
    }
}
