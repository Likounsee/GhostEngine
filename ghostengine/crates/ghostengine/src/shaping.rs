//! OpenType text shaping primitives.
//!
//! This module computes glyph substitutions and positioning with Rustybuzz.
//! It does not rasterize glyphs: all drawing remains in GhostEngine's CPU renderer.

use std::sync::Arc;

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
        if !pixel_size.is_finite() || pixel_size <= 0.0 {
            return Err(ShapingError::InvalidPixelSize);
        }
        if text.is_empty() {
            return Ok(ShapedText::default());
        }

        let face =
            rustybuzz::Face::from_slice(&self.font_data, 0).ok_or(ShapingError::InvalidFont)?;
        let units_per_em = f32::from(face.units_per_em());
        if !units_per_em.is_finite() || units_per_em <= 0.0 {
            return Err(ShapingError::InvalidFont);
        }
        let scale = pixel_size / units_per_em;

        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
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
    use super::{OpenTypeShaper, ShapedText, ShapingError};

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
