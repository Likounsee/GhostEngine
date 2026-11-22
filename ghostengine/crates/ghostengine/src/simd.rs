//! SIMD pixel kernels with a scalar fallback.
//!
//! The renderer calls the same narrow functions regardless of build features.
//! The default build enables the safe `wide` backend; a no-default-features
//! build keeps the CPU renderer functional with scalar blending.

use crate::Color;

#[cfg(feature = "simd")]
use wide::u32x4;

#[cfg(feature = "simd")]
#[inline]
pub(crate) fn blend_opaque_quad(
    pixels: &mut [Color; 4],
    source: Color,
    source_alpha: u32,
    inverse_source_alpha: u32,
) {
    let blend_channel = |source_channel: u32, destinations: [u32; 4]| {
        let numerator = u32x4::splat(source_channel * source_alpha)
            + u32x4::new(destinations) * u32x4::splat(inverse_source_alpha)
            + u32x4::splat(127);
        ((numerator + u32x4::splat(1) + (numerator >> 8_u32)) >> 8_u32).to_array()
    };
    let red = blend_channel(u32::from(source.r), (*pixels).map(|pixel| u32::from(pixel.r)));
    let green = blend_channel(u32::from(source.g), (*pixels).map(|pixel| u32::from(pixel.g)));
    let blue = blend_channel(u32::from(source.b), (*pixels).map(|pixel| u32::from(pixel.b)));
    for index in 0..4 {
        pixels[index] = Color::rgb(red[index] as u8, green[index] as u8, blue[index] as u8);
    }
}

#[cfg(not(feature = "simd"))]
#[inline]
pub(crate) fn blend_opaque_quad(pixels: &mut [Color; 4], source: Color, source_alpha: u32, inverse_source_alpha: u32) {
    let blend_channel = |source: u8, destination: u8| -> u8 {
        ((u32::from(source) * source_alpha
            + u32::from(destination) * inverse_source_alpha
            + 127)
            / 255) as u8
    };
    for pixel in pixels {
        *pixel = Color::rgb(
            blend_channel(source.r, pixel.r),
            blend_channel(source.g, pixel.g),
            blend_channel(source.b, pixel.b),
        );
    }
}

/// Composites four varying source pixels over four opaque destinations.
#[cfg(feature = "simd")]
#[inline]
pub(crate) fn blend_variable_opaque_quad(destinations: &mut [Color; 4], sources: [Color; 4]) {
    let source_alpha = sources.map(|color| u32::from(color.a));
    let inverse_alpha = source_alpha.map(|alpha| 255 - alpha);
    let blend_channel = |source: [u32; 4], destination: [u32; 4]| {
        let numerator = u32x4::new(source) * u32x4::new(source_alpha)
            + u32x4::new(destination) * u32x4::new(inverse_alpha)
            + u32x4::splat(127);
        ((numerator + u32x4::splat(1) + (numerator >> 8_u32)) >> 8_u32).to_array()
    };
    let red = blend_channel(
        sources.map(|color| u32::from(color.r)),
        (*destinations).map(|color| u32::from(color.r)),
    );
    let green = blend_channel(
        sources.map(|color| u32::from(color.g)),
        (*destinations).map(|color| u32::from(color.g)),
    );
    let blue = blend_channel(
        sources.map(|color| u32::from(color.b)),
        (*destinations).map(|color| u32::from(color.b)),
    );
    for index in 0..4 {
        destinations[index] = Color::rgb(red[index] as u8, green[index] as u8, blue[index] as u8);
    }
}

#[cfg(not(feature = "simd"))]
#[inline]
pub(crate) fn blend_variable_opaque_quad(destinations: &mut [Color; 4], sources: [Color; 4]) {
    for (destination, source) in destinations.iter_mut().zip(sources) {
        let alpha = u32::from(source.a);
        let inverse_alpha = 255 - alpha;
        let blend_channel = |source: u8, destination: u8| -> u8 {
            ((u32::from(source) * alpha
                + u32::from(destination) * inverse_alpha
                + 127)
                / 255) as u8
        };
        *destination = Color::rgb(
            blend_channel(source.r, destination.r),
            blend_channel(source.g, destination.g),
            blend_channel(source.b, destination.b),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::Color;

    #[test]
    fn variable_source_simd_blend_matches_scalar_for_all_alpha_values() {
        for alpha in 0..=255_u8 {
            let sources = [
                Color::rgba(255, 0, 17, alpha),
                Color::rgba(10, 220, 30, 255 - alpha),
                Color::rgba(90, 80, 70, alpha / 2),
                Color::rgba(255, 200, 150, alpha.wrapping_mul(73)),
            ];
            let mut actual = [
                Color::rgb(0, 255, 127),
                Color::rgb(255, 0, 128),
                Color::rgb(17, 99, 231),
                Color::rgb(240, 31, 3),
            ];
            let expected = [
                sources[0].blend_over(actual[0]),
                sources[1].blend_over(actual[1]),
                sources[2].blend_over(actual[2]),
                sources[3].blend_over(actual[3]),
            ];
            super::blend_variable_opaque_quad(&mut actual, sources);
            assert_eq!(actual, expected, "base source alpha {alpha}");
        }
    }

    #[test]
    fn simd_opaque_blend_matches_scalar_for_every_source_alpha_and_channel_edges() {
        let source_channels = [(0, 255, 127), (255, 0, 128), (17, 99, 231), (240, 31, 3)];
        let destination_values = [0_u8, 1, 63, 127, 128, 192, 254, 255];

        for alpha in 1..=254 {
            for (red, green, blue) in source_channels {
                let source = Color::rgba(red, green, blue, alpha);
                for offset in 0..destination_values.len() {
                    let mut actual = [
                        Color::rgb(
                            destination_values[offset],
                            destination_values[(offset + 1) % 8],
                            destination_values[(offset + 2) % 8],
                        ),
                        Color::rgb(
                            destination_values[(offset + 3) % 8],
                            destination_values[(offset + 4) % 8],
                            destination_values[(offset + 5) % 8],
                        ),
                        Color::rgb(
                            destination_values[(offset + 6) % 8],
                            destination_values[(offset + 7) % 8],
                            destination_values[offset],
                        ),
                        Color::rgb(
                            destination_values[(offset + 2) % 8],
                            destination_values[(offset + 5) % 8],
                            destination_values[(offset + 7) % 8],
                        ),
                    ];
                    let expected = actual.map(|background| source.blend_over(background));
                    let source_alpha = u32::from(source.a);
                    super::blend_opaque_quad(&mut actual, source, source_alpha, 255 - source_alpha);
                    assert_eq!(
                        actual, expected,
                        "source alpha {alpha}, channels {red}/{green}/{blue}"
                    );
                }
            }
        }
    }
}
