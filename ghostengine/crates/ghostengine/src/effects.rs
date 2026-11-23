//! Reusable CPU image effects.

use crate::{Color, Framebuffer, FramebufferError};

/// Configuration for a CPU drop shadow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropShadowOptions {
    /// Horizontal displacement from the source image.
    pub offset_x: i32,
    /// Vertical displacement from the source image.
    pub offset_y: i32,
    /// Gaussian blur half-width in pixels.
    pub blur_radius: u32,
    /// Shadow tint, including its maximum opacity.
    pub color: Color,
}

impl Default for DropShadowOptions {
    fn default() -> Self {
        Self {
            offset_x: 0,
            offset_y: 2,
            blur_radius: 4,
            color: Color::rgba(0, 0, 0, 128),
        }
    }
}

/// Retained temporary storage for separable blur passes.
#[derive(Debug, Default)]
pub struct EffectScratch {
    source: Vec<Color>,
    pixels: Vec<Color>,
    weights: Vec<f64>,
    shadow: Option<Framebuffer>,
}

/// Applies a separable box blur in-place using premultiplied-alpha arithmetic.
///
/// The scratch allocation is retained between calls. Runtime is linear in the
/// number of pixels and independent of blur radius after initialization.
pub fn apply_box_blur(target: &mut Framebuffer, radius: u32, scratch: &mut EffectScratch) {
    if radius == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }
    let width = target.width() as usize;
    let height = target.height() as usize;
    let radius_x = (radius as usize).min(width.saturating_sub(1));
    let radius_y = (radius as usize).min(height.saturating_sub(1));
    scratch
        .source
        .resize(target.pixels().len(), Color::TRANSPARENT);
    scratch.source.copy_from_slice(target.pixels());
    scratch
        .pixels
        .resize(scratch.source.len(), Color::TRANSPARENT);

    for y in 0..height {
        let mut sums = [0_u64; 4];
        let count = radius_x * 2 + 1;
        for offset in 0..count {
            let x = offset.saturating_sub(radius_x).min(width - 1);
            add_pixel(&mut sums, scratch.source[y * width + x], 1);
        }
        for x in 0..width {
            scratch.pixels[y * width + x] = color_from_sums(sums, count as u64);
            let remove_x = x.saturating_sub(radius_x);
            let add_x = (x + radius_x + 1).min(width - 1);
            add_pixel(&mut sums, scratch.source[y * width + remove_x], -1);
            add_pixel(&mut sums, scratch.source[y * width + add_x], 1);
        }
    }

    for x in 0..width {
        let mut sums = [0_u64; 4];
        let count = radius_y * 2 + 1;
        for offset in 0..count {
            let y = offset.saturating_sub(radius_y).min(height - 1);
            add_pixel(&mut sums, scratch.pixels[y * width + x], 1);
        }
        for y in 0..height {
            target.pixels_mut()[y * width + x] = color_from_sums(sums, count as u64);
            let remove_y = y.saturating_sub(radius_y);
            let add_y = (y + radius_y + 1).min(height - 1);
            add_pixel(&mut sums, scratch.pixels[remove_y * width + x], -1);
            add_pixel(&mut sums, scratch.pixels[add_y * width + x], 1);
        }
    }
}

/// Applies a separable Gaussian blur in-place using premultiplied-alpha arithmetic.
///
/// The radius is the kernel half-width; standard deviation is max(radius / 2, 0.5).
/// Edge kernels are clipped to the image bounds and renormalized. Scratch allocations persist.
pub fn apply_gaussian_blur(target: &mut Framebuffer, radius: u32, scratch: &mut EffectScratch) {
    if radius == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }
    let width = target.width() as usize;
    let height = target.height() as usize;
    let radius = (radius as usize).min(width.max(height).saturating_sub(1));
    if radius == 0 {
        return;
    }

    let sigma = (radius as f64 * 0.5).max(0.5);
    let denominator = 2.0 * sigma * sigma;
    scratch.weights.resize(radius + 1, 0.0);
    for (distance, weight) in scratch.weights.iter_mut().enumerate() {
        let distance = distance as f64;
        *weight = (-(distance * distance) / denominator).exp();
    }

    scratch
        .source
        .resize(target.pixels().len(), Color::TRANSPARENT);
    scratch.source.copy_from_slice(target.pixels());
    scratch
        .pixels
        .resize(scratch.source.len(), Color::TRANSPARENT);

    let radius_x = radius.min(width - 1);
    for y in 0..height {
        for x in 0..width {
            let start = x.saturating_sub(radius_x);
            let end = (x + radius_x + 1).min(width);
            let mut sums = [0.0_f64; 4];
            let mut total_weight = 0.0_f64;
            for sample_x in start..end {
                let weight = scratch.weights[x.abs_diff(sample_x)];
                add_weighted_pixel(&mut sums, scratch.source[y * width + sample_x], weight);
                total_weight += weight;
            }
            scratch.pixels[y * width + x] = weighted_color(sums, total_weight);
        }
    }

    let radius_y = radius.min(height - 1);
    for y in 0..height {
        let start = y.saturating_sub(radius_y);
        let end = (y + radius_y + 1).min(height);
        for x in 0..width {
            let mut sums = [0.0_f64; 4];
            let mut total_weight = 0.0_f64;
            for sample_y in start..end {
                let weight = scratch.weights[y.abs_diff(sample_y)];
                add_weighted_pixel(&mut sums, scratch.pixels[sample_y * width + x], weight);
                total_weight += weight;
            }
            target.pixels_mut()[y * width + x] = weighted_color(sums, total_weight);
        }
    }
}

/// Draws a source image over a tinted, blurred copy of its alpha mask.
///
/// The shadow is placed behind the source at (x + offset_x, y + offset_y).
/// The padded surface and blur buffers are retained in scratch for reuse.
/// A zero blur radius produces a hard-edged shadow. Pixels are clipped safely.
pub fn draw_drop_shadow(
    target: &mut Framebuffer,
    source: &Framebuffer,
    x: i32,
    y: i32,
    options: DropShadowOptions,
    scratch: &mut EffectScratch,
) -> Result<(), FramebufferError> {
    let DropShadowOptions {
        offset_x,
        offset_y,
        blur_radius,
        color,
    } = options;
    if color.a != 0 {
        let padding = blur_radius
            .checked_mul(2)
            .ok_or(FramebufferError::SizeOverflow)?;
        let shadow_width = source
            .width()
            .checked_add(padding)
            .ok_or(FramebufferError::SizeOverflow)?;
        let shadow_height = source
            .height()
            .checked_add(padding)
            .ok_or(FramebufferError::SizeOverflow)?;

        let mut shadow = match scratch.shadow.take() {
            Some(mut cached) => {
                if let Err(error) =
                    cached.resize_and_clear(shadow_width, shadow_height, Color::TRANSPARENT)
                {
                    scratch.shadow = Some(cached);
                    return Err(error);
                }
                cached
            }
            None => Framebuffer::new(shadow_width, shadow_height, Color::TRANSPARENT)?,
        };

        let inset = blur_radius as usize;
        let shadow_width_usize = shadow_width as usize;
        for source_y in 0..source.height() as usize {
            let source_start = source_y * source.width() as usize;
            let shadow_start = (source_y + inset) * shadow_width_usize + inset;
            for source_x in 0..source.width() as usize {
                let alpha = source.pixels()[source_start + source_x].a;
                shadow.pixels_mut()[shadow_start + source_x] =
                    Color::rgba(color.r, color.g, color.b, alpha);
            }
        }

        apply_gaussian_blur(&mut shadow, blur_radius, scratch);
        for pixel in shadow.pixels_mut() {
            pixel.a = ((u16::from(pixel.a) * u16::from(color.a) + 127) / 255) as u8;
        }

        let shadow_x = i64::from(x) + i64::from(offset_x) - i64::from(blur_radius);
        let shadow_y = i64::from(y) + i64::from(offset_y) - i64::from(blur_radius);
        composite_framebuffer(target, &shadow, shadow_x, shadow_y);
        scratch.shadow = Some(shadow);
    }

    composite_framebuffer(target, source, i64::from(x), i64::from(y));
    Ok(())
}

fn composite_framebuffer(target: &mut Framebuffer, source: &Framebuffer, x: i64, y: i64) {
    let left = x.max(0);
    let top = y.max(0);
    let right = (x + i64::from(source.width())).min(i64::from(target.width()));
    let bottom = (y + i64::from(source.height())).min(i64::from(target.height()));
    if left >= right || top >= bottom {
        return;
    }

    let width = (right - left) as usize;
    let source_x = (left - x) as usize;
    let target_x = left as usize;
    let source_width = source.width() as usize;
    let target_width = target.width() as usize;

    for destination_y in top..bottom {
        let source_y = (destination_y - y) as usize;
        let source_start = source_y * source_width + source_x;
        let target_start = destination_y as usize * target_width + target_x;
        let source_row = &source.pixels()[source_start..source_start + width];
        let target_row = &mut target.pixels_mut()[target_start..target_start + width];
        for (foreground, background) in source_row.iter().copied().zip(target_row) {
            if foreground.a == 255 {
                *background = foreground;
            } else if foreground.a != 0 {
                *background = foreground.blend_over(*background);
            }
        }
    }
}

fn add_weighted_pixel(sums: &mut [f64; 4], color: Color, weight: f64) {
    let alpha = f64::from(color.a) / 255.0;
    sums[0] += f64::from(color.r) * alpha * weight;
    sums[1] += f64::from(color.g) * alpha * weight;
    sums[2] += f64::from(color.b) * alpha * weight;
    sums[3] += alpha * weight;
}

fn weighted_color(sums: [f64; 4], total_weight: f64) -> Color {
    if total_weight <= 0.0 || sums[3] <= 0.0 {
        return Color::TRANSPARENT;
    }
    let channel = |value: f64| (value / sums[3]).round().clamp(0.0, 255.0) as u8;
    let alpha = (sums[3] / total_weight * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::rgba(channel(sums[0]), channel(sums[1]), channel(sums[2]), alpha)
}

fn add_pixel(sums: &mut [u64; 4], color: Color, direction: i8) {
    let alpha = u64::from(color.a);
    let values = [
        u64::from(color.r) * alpha,
        u64::from(color.g) * alpha,
        u64::from(color.b) * alpha,
        alpha,
    ];
    for (sum, value) in sums.iter_mut().zip(values) {
        if direction > 0 {
            *sum += value;
        } else {
            *sum -= value;
        }
    }
}

fn color_from_sums(sums: [u64; 4], count: u64) -> Color {
    if sums[3] == 0 {
        return Color::TRANSPARENT;
    }
    let channel = |premultiplied: u64| ((premultiplied + sums[3] / 2) / sums[3]).min(255) as u8;
    let alpha = ((sums[3] + count / 2) / count).min(255) as u8;
    Color::rgba(channel(sums[0]), channel(sums[1]), channel(sums[2]), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Framebuffer;

    #[test]
    fn drop_shadow_is_drawn_behind_source_and_reuses_surface() {
        let source = Framebuffer::new(1, 1, Color::WHITE).unwrap();
        let mut target = Framebuffer::new(5, 3, Color::BLACK).unwrap();
        let mut scratch = EffectScratch::default();

        draw_drop_shadow(
            &mut target,
            &source,
            1,
            1,
            DropShadowOptions {
                offset_x: 2,
                offset_y: 0,
                blur_radius: 0,
                color: Color::RED,
            },
            &mut scratch,
        )
        .unwrap();

        assert_eq!(target.get_pixel(1, 1), Some(Color::WHITE));
        assert_eq!(target.get_pixel(3, 1), Some(Color::RED));
        let allocation = scratch.shadow.as_ref().unwrap().pixels().as_ptr();

        target.clear(Color::BLACK);
        draw_drop_shadow(
            &mut target,
            &source,
            1,
            1,
            DropShadowOptions {
                offset_x: 2,
                offset_y: 0,
                blur_radius: 0,
                color: Color::RED,
            },
            &mut scratch,
        )
        .unwrap();
        assert_eq!(
            scratch.shadow.as_ref().unwrap().pixels().as_ptr(),
            allocation
        );
        assert_eq!(target.get_pixel(3, 1), Some(Color::RED));
    }

    #[test]
    fn blurred_drop_shadow_spreads_alpha_without_colored_halos() {
        let source = Framebuffer::new(1, 1, Color::WHITE).unwrap();
        let mut target = Framebuffer::new(5, 5, Color::BLACK).unwrap();

        draw_drop_shadow(
            &mut target,
            &source,
            2,
            2,
            DropShadowOptions {
                offset_x: 0,
                offset_y: 0,
                blur_radius: 1,
                color: Color::rgba(255, 0, 0, 200),
            },
            &mut EffectScratch::default(),
        )
        .unwrap();

        assert_eq!(target.get_pixel(2, 2), Some(Color::WHITE));
        let adjacent = target.get_pixel(1, 2).unwrap();
        assert!(adjacent.r > 0);
        assert_eq!(adjacent.g, 0);
        assert_eq!(adjacent.b, 0);
        assert!(adjacent.a == 255);
    }

    #[test]
    fn transparent_shadow_color_still_draws_source() {
        let source = Framebuffer::new(1, 1, Color::WHITE).unwrap();
        let mut target = Framebuffer::new(2, 1, Color::BLACK).unwrap();

        draw_drop_shadow(
            &mut target,
            &source,
            1,
            0,
            DropShadowOptions {
                offset_x: 0,
                offset_y: 0,
                blur_radius: 2,
                color: Color::TRANSPARENT,
            },
            &mut EffectScratch::default(),
        )
        .unwrap();

        assert_eq!(target.get_pixel(1, 0), Some(Color::WHITE));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn blur_preserves_uniform_images() {
        let mut target = Framebuffer::new(5, 3, Color::rgba(20, 80, 160, 200)).unwrap();
        apply_box_blur(&mut target, 2, &mut EffectScratch::default());
        assert!(target
            .pixels()
            .iter()
            .all(|pixel| *pixel == Color::rgba(20, 80, 160, 200)));
    }

    #[test]
    fn blur_softens_edges_without_transparent_rgb_halos() {
        let mut target = Framebuffer::new(3, 1, Color::TRANSPARENT).unwrap();
        target.set_pixel(1, 0, Color::RED);
        apply_box_blur(&mut target, 1, &mut EffectScratch::default());
        assert_eq!(target.get_pixel(1, 0).unwrap().a, 85);
        assert_eq!(target.get_pixel(0, 0).unwrap().a, 85);
        assert_eq!(target.get_pixel(2, 0).unwrap().a, 85);
        assert!(target
            .pixels()
            .iter()
            .all(|pixel| pixel.r == 255 && pixel.g == 0 && pixel.b == 0));
    }

    #[test]
    fn gaussian_blur_preserves_uniform_images_and_alpha() {
        let color = Color::rgba(20, 80, 160, 200);
        let mut target = Framebuffer::new(7, 5, color).unwrap();
        apply_gaussian_blur(&mut target, 3, &mut EffectScratch::default());
        assert!(target.pixels().iter().all(|pixel| *pixel == color));
    }

    #[test]
    fn gaussian_blur_softens_edges_without_transparent_rgb_halos() {
        let mut target = Framebuffer::new(5, 1, Color::TRANSPARENT).unwrap();
        target.set_pixel(2, 0, Color::RED);
        let mut scratch = EffectScratch::default();
        apply_gaussian_blur(&mut target, 2, &mut scratch);
        assert!(target.pixels()[2].a > target.pixels()[1].a);
        assert!(target.pixels()[1].a > target.pixels()[0].a);
        assert!(target
            .pixels()
            .iter()
            .all(|pixel| pixel.r == 255 && pixel.g == 0 && pixel.b == 0));
        let capacity = scratch.weights.capacity();
        apply_gaussian_blur(&mut target, 2, &mut scratch);
        assert_eq!(scratch.weights.capacity(), capacity);
    }

    #[test]
    fn gaussian_blur_handles_single_pixel_axes_and_zero_radius() {
        let mut target = Framebuffer::new(1, 3, Color::TRANSPARENT).unwrap();
        target.set_pixel(0, 1, Color::BLUE);
        let original = target.clone();
        apply_gaussian_blur(&mut target, 0, &mut EffectScratch::default());
        assert_eq!(target, original);
        apply_gaussian_blur(&mut target, 4, &mut EffectScratch::default());
        assert!(target.pixels().iter().all(|pixel| pixel.b == 255));
        assert!(target.pixels()[1].a > target.pixels()[0].a);
    }

    #[test]
    fn zero_radius_is_a_noop() {
        let mut target = Framebuffer::new(1, 1, Color::BLUE).unwrap();
        apply_box_blur(&mut target, 0, &mut EffectScratch::default());
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLUE));
    }
}


#[cfg(test)]
mod box_blur_reference_tests {
    use super::{apply_box_blur, EffectScratch};
    use crate::{Color, Framebuffer};

    fn reference_axis(source: &[Color], width: usize, height: usize, radius: usize, horizontal: bool) -> Vec<Color> {
        let mut output = vec![Color::TRANSPARENT; source.len()];
        for y in 0..height {
            for x in 0..width {
                let mut sums = [0_u64; 4];
                let count = radius * 2 + 1;
                for offset in 0..count {
                    let (sample_x, sample_y) = if horizontal {
                        ((x as isize + offset as isize - radius as isize).clamp(0, width as isize - 1) as usize, y)
                    } else {
                        (x, (y as isize + offset as isize - radius as isize).clamp(0, height as isize - 1) as usize)
                    };
                    let pixel = source[sample_y * width + sample_x];
                    let alpha = u64::from(pixel.a);
                    sums[0] += u64::from(pixel.r) * alpha;
                    sums[1] += u64::from(pixel.g) * alpha;
                    sums[2] += u64::from(pixel.b) * alpha;
                    sums[3] += alpha;
                }
                let alpha = ((sums[3] + count as u64 / 2) / count as u64).min(255) as u8;
                let channel = |value: u64| {
                    if sums[3] == 0 { 0 } else { ((value + sums[3] / 2) / sums[3]).min(255) as u8 }
                };
                output[y * width + x] = if sums[3] == 0 {
                    Color::TRANSPARENT
                } else {
                    Color::rgba(channel(sums[0]), channel(sums[1]), channel(sums[2]), alpha)
                };
            }
        }
        output
    }

    #[test]
    fn sliding_box_blur_matches_clamped_reference_for_small_surfaces_and_large_radii() {
        for width in 1..=7 {
            for height in 1..=6 {
                let mut frame = Framebuffer::new(width as u32, height as u32, Color::TRANSPARENT).unwrap();
                for (index, pixel) in frame.pixels_mut().iter_mut().enumerate() {
                    let value = (index * 47 + width * 19 + height * 31) as u8;
                    *pixel = Color::rgba(value, value.wrapping_mul(3), value.wrapping_add(91), value.wrapping_mul(7));
                }
                for radius in [1_u32, 2, 4, 9] {
                    let radius_x = (radius as usize).min(width - 1);
                    let radius_y = (radius as usize).min(height - 1);
                    let horizontal = reference_axis(frame.pixels(), width, height, radius_x, true);
                    let expected = reference_axis(&horizontal, width, height, radius_y, false);
                    let mut actual = frame.clone();
                    apply_box_blur(&mut actual, radius, &mut EffectScratch::default());
                    assert_eq!(actual.pixels(), expected, "size={width}x{height}, radius={radius}");
                }
            }
        }
    }
}
