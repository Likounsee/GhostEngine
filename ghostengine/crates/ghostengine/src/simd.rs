//! Architecture-independent SIMD entry points for CPU raster kernels.
//!
//! Keep vector types and backend-specific operations inside this module. The
//! rasterizer calls small pixel-oriented functions, allowing the implementation
//! to move from `wide` to Rust Portable SIMD (or another backend) without
//! spreading SIMD types through the renderer's public or internal APIs.

use crate::Color;
use wide::u32x4;

#[inline]
pub(crate) fn blend_opaque_quad(
    pixels: &mut [Color; 4],
    source: Color,
    source_alpha: u32,
    inverse_source_alpha: u32,
) {
    // Each SIMD lane represents one pixel channel value across four pixels.
    // The rounded divide by 255 is expressed with shifts and additions:
    // floor((n + 127) / 255) == (m + 1 + (m >> 8)) >> 8, m = n + 127.
    let blend_channel = |source_channel: u32, destinations: [u32; 4]| {
        let numerator = u32x4::splat(source_channel * source_alpha)
            + u32x4::new(destinations) * u32x4::splat(inverse_source_alpha)
            + u32x4::splat(127);
        ((numerator + u32x4::splat(1) + (numerator >> 8_u32)) >> 8_u32).to_array()
    };

    let red = blend_channel(
        u32::from(source.r),
        (*pixels).map(|pixel| u32::from(pixel.r)),
    );
    let green = blend_channel(
        u32::from(source.g),
        (*pixels).map(|pixel| u32::from(pixel.g)),
    );
    let blue = blend_channel(
        u32::from(source.b),
        (*pixels).map(|pixel| u32::from(pixel.b)),
    );

    for index in 0..4 {
        pixels[index] = Color::rgb(red[index] as u8, green[index] as u8, blue[index] as u8);
    }
}

#[cfg(test)]
mod tests {
    use crate::Color;

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
                    super::blend_opaque_quad(
                        &mut actual,
                        source,
                        source_alpha,
                        255 - source_alpha,
                    );
                    assert_eq!(
                        actual, expected,
                        "source alpha {alpha}, channels {red}/{green}/{blue}"
                    );
                }
            }
        }
    }
}
