/// An 8-bit-per-channel RGBA color using straight (unassociated) alpha.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const RED: Self = Self::rgb(255, 0, 0);
    pub const GREEN: Self = Self::rgb(0, 255, 0);
    pub const BLUE: Self = Self::rgb(0, 0, 255);

    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Composites this foreground color over a background using source-over alpha.
    #[must_use]
    pub fn blend_over(self, background: Self) -> Self {
        // These two cases dominate opaque drawing and transparent overlays.
        // They are exact source-over results and avoid channel arithmetic/division.
        if self.a == 0 {
            return background;
        }
        if self.a == u8::MAX {
            return self;
        }

        let source_alpha = u32::from(self.a);
        let inverse_source_alpha = 255 - source_alpha;

        // Framebuffers are commonly opaque. In that case source-over reduces
        // to three rounded 8-bit interpolations, avoiding a per-channel
        // division by the more expensive general alpha numerator.
        if background.a == u8::MAX {
            let blend_channel = |source: u8, destination: u8| -> u8 {
                ((u32::from(source) * source_alpha
                    + u32::from(destination) * inverse_source_alpha
                    + 127)
                    / 255) as u8
            };
            return Self::rgb(
                blend_channel(self.r, background.r),
                blend_channel(self.g, background.g),
                blend_channel(self.b, background.b),
            );
        }

        let destination_alpha = u32::from(background.a);
        let alpha_numerator = source_alpha * 255 + destination_alpha * inverse_source_alpha;

        if alpha_numerator == 0 {
            return Self::TRANSPARENT;
        }

        let blend_channel = |source: u8, destination: u8| -> u8 {
            let numerator = u32::from(source) * source_alpha * 255
                + u32::from(destination) * destination_alpha * inverse_source_alpha;
            ((numerator + alpha_numerator / 2) / alpha_numerator) as u8
        };

        Self {
            r: blend_channel(self.r, background.r),
            g: blend_channel(self.g, background.g),
            b: blend_channel(self.b, background.b),
            a: ((alpha_numerator + 127) / 255) as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn rgb_is_opaque() {
        assert_eq!(Color::rgb(1, 2, 3).a, 255);
    }

    #[test]
    fn transparent_foreground_keeps_background() {
        let background = Color::rgba(20, 40, 60, 180);
        assert_eq!(Color::TRANSPARENT.blend_over(background), background);
    }

    #[test]
    fn opaque_foreground_replaces_background() {
        let foreground = Color::rgb(200, 100, 50);
        assert_eq!(foreground.blend_over(Color::rgba(1, 2, 3, 100)), foreground);
    }

    #[test]
    fn half_transparent_red_over_opaque_blue() {
        let result = Color::rgba(255, 0, 0, 128).blend_over(Color::BLUE);
        assert_eq!(result, Color::rgb(128, 0, 127));
    }

    #[test]
    fn opaque_background_fast_blend_matches_general_source_over_rounding() {
        for source_alpha in 1_u32..255 {
            for source in [0_u32, 1, 63, 127, 128, 192, 254, 255] {
                for destination in [0_u32, 1, 63, 127, 128, 192, 254, 255] {
                    let inverse_source_alpha = 255 - source_alpha;
                    let alpha_numerator = source_alpha * 255 + 255 * inverse_source_alpha;
                    let channel_numerator =
                        source * source_alpha * 255 + destination * 255 * inverse_source_alpha;
                    let expected =
                        ((channel_numerator + alpha_numerator / 2) / alpha_numerator) as u8;
                    let actual =
                        Color::rgba(source as u8, source as u8, source as u8, source_alpha as u8)
                            .blend_over(Color::rgb(
                                destination as u8,
                                destination as u8,
                                destination as u8,
                            ));
                    assert_eq!(actual, Color::rgb(expected, expected, expected));
                }
            }
        }
    }
}
