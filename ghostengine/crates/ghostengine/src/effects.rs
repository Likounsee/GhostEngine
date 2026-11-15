//! Reusable CPU image effects.

use crate::{Color, Framebuffer};

/// Retained temporary storage for separable blur passes.
#[derive(Debug, Default)]
pub struct EffectScratch {
    source: Vec<Color>,
    pixels: Vec<Color>,
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
            if x >= radius_x {
                add_pixel(&mut sums, scratch.source[y * width + remove_x], -1);
            }
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
    fn zero_radius_is_a_noop() {
        let mut target = Framebuffer::new(1, 1, Color::BLUE).unwrap();
        apply_box_blur(&mut target, 0, &mut EffectScratch::default());
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLUE));
    }
}
