//! Alpha-mask operations for clipped CPU framebuffers.

use crate::{Framebuffer, Point, Rect};

/// Multiplies destination alpha by a mask's alpha channel inside the clip.
///
/// RGB values are preserved. Mask coordinates are offset by mask_origin in
/// destination space; pixels outside the mask bounds receive zero coverage.
pub fn apply_alpha_mask_clipped(
    target: &mut Framebuffer,
    mask: &Framebuffer,
    mask_origin: Point,
    clip: Rect,
) {
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return;
    };
    let right = u64::from(clip.x) + u64::from(clip.width);
    let bottom = u64::from(clip.y) + u64::from(clip.height);
    for y in clip.y..bottom as u32 {
        let mask_y = y as i64 - i64::from(mask_origin.y);
        for x in clip.x..right as u32 {
            let mask_x = x as i64 - i64::from(mask_origin.x);
            let coverage = if mask_x < 0
                || mask_y < 0
                || mask_x >= i64::from(mask.width())
                || mask_y >= i64::from(mask.height())
            {
                0
            } else {
                mask.get_pixel(mask_x as u32, mask_y as u32).unwrap().a
            };
            let pixel = target.get_pixel(x, y).unwrap();
            let alpha = (u16::from(pixel.a) * u16::from(coverage) + 127) / 255;
            target.set_pixel(
                x,
                y,
                crate::Color::rgba(pixel.r, pixel.g, pixel.b, alpha as u8),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    #[test]
    fn mask_multiplies_alpha_and_preserves_rgb() {
        let mut target = Framebuffer::new(2, 1, Color::rgba(200, 100, 50, 255)).unwrap();
        let mask = Framebuffer::new(2, 1, Color::rgba(0, 0, 0, 128)).unwrap();
        apply_alpha_mask_clipped(&mut target, &mask, Point::new(0, 0), Rect::new(0, 0, 2, 1));
        assert_eq!(target.get_pixel(0, 0), Some(Color::rgba(200, 100, 50, 128)));
    }

    #[test]
    fn mask_origin_and_clip_are_respected() {
        let mut target = Framebuffer::new(3, 1, Color::WHITE).unwrap();
        let mask = Framebuffer::new(1, 1, Color::rgba(0, 0, 0, 255)).unwrap();
        apply_alpha_mask_clipped(&mut target, &mask, Point::new(1, 0), Rect::new(0, 0, 2, 1));
        assert_eq!(target.get_pixel(0, 0).unwrap().a, 0);
        assert_eq!(target.get_pixel(1, 0).unwrap().a, 255);
        assert_eq!(target.get_pixel(2, 0).unwrap().a, 255);
    }
}
