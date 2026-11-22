//! Alpha-mask operations for clipped CPU framebuffers.

use crate::{Color, Framebuffer, Point, Rect};

/// Multiplies destination alpha by a mask's alpha channel inside the clip.
///
/// RGB values are preserved. Mask coordinates are offset by mask_origin in
/// destination space; pixels outside the mask bounds receive zero coverage.
/// The implementation processes contiguous rows and the overlapping mask span
/// directly, avoiding per-pixel framebuffer bounds checks.
pub fn apply_alpha_mask_clipped(
    target: &mut Framebuffer,
    mask: &Framebuffer,
    mask_origin: Point,
    clip: Rect,
) {
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return;
    };

    let target_x = clip.x as usize;
    let clip_width = clip.width as usize;
    let mask_x_at_clip_start = i64::from(clip.x) - i64::from(mask_origin.x);
    let mask_y_at_clip_start = i64::from(clip.y) - i64::from(mask_origin.y);
    let mask_width = i64::from(mask.width());
    let mask_height = i64::from(mask.height());

    let overlap_start = (-mask_x_at_clip_start).clamp(0, i64::from(clip.width));
    let overlap_end = (mask_width - mask_x_at_clip_start).clamp(0, i64::from(clip.width));
    let has_horizontal_overlap = overlap_start < overlap_end;

    for row_offset in 0..clip.height as usize {
        let target_y = clip.y as usize + row_offset;
        let target_start = target_y * target.width() as usize + target_x;
        let target_row = &mut target.pixels_mut()[target_start..target_start + clip_width];

        let mask_y = mask_y_at_clip_start + row_offset as i64;
        if mask_y < 0 || mask_y >= mask_height || !has_horizontal_overlap {
            for pixel in target_row {
                pixel.a = 0;
            }
            continue;
        }

        let left_end = overlap_start as usize;
        for pixel in &mut target_row[..left_end] {
            pixel.a = 0;
        }

        let overlap_start = overlap_start as usize;
        let overlap_end = overlap_end as usize;
        let mask_x = (mask_x_at_clip_start + overlap_start as i64) as usize;
        let mask_row_start = mask_y as usize * mask.width() as usize + mask_x;
        let mask_row = &mask.pixels()[mask_row_start..mask_row_start + (overlap_end - overlap_start)];
        for (pixel, mask_pixel) in target_row[overlap_start..overlap_end]
            .iter_mut()
            .zip(mask_row.iter())
        {
            pixel.a = ((u16::from(pixel.a) * u16::from(mask_pixel.a) + 127) / 255) as u8;
        }

        for pixel in &mut target_row[overlap_end..] {
            pixel.a = 0;
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
        assert_eq!(target.get_pixel(1, 0), Some(Color::rgba(200, 100, 50, 128)));
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

    #[test]
    fn negative_mask_origin_and_partial_overlap_map_correctly() {
        let mut target = Framebuffer::new(5, 1, Color::rgba(10, 20, 30, 200)).unwrap();
        let mut mask = Framebuffer::new(3, 1, Color::TRANSPARENT).unwrap();
        mask.pixels_mut().copy_from_slice(&[
            Color::rgba(0, 0, 0, 64),
            Color::rgba(0, 0, 0, 128),
            Color::rgba(0, 0, 0, 255),
        ]);

        apply_alpha_mask_clipped(&mut target, &mask, Point::new(-1, 0), Rect::new(0, 0, 5, 1));

        assert_eq!(target.get_pixel(0, 0), Some(Color::rgba(10, 20, 30, 100)));
        assert_eq!(target.get_pixel(1, 0), Some(Color::rgba(10, 20, 30, 200)));
        assert_eq!(target.get_pixel(2, 0), Some(Color::rgba(10, 20, 30, 0)));
        assert_eq!(target.get_pixel(3, 0), Some(Color::rgba(10, 20, 30, 0)));
        assert_eq!(target.get_pixel(4, 0), Some(Color::rgba(10, 20, 30, 0)));
    }

    #[test]
    fn rows_outside_mask_bounds_clear_alpha_without_changing_rgb() {
        let mut target = Framebuffer::new(2, 3, Color::rgba(7, 8, 9, 240)).unwrap();
        let mask = Framebuffer::new(2, 1, Color::WHITE).unwrap();

        apply_alpha_mask_clipped(&mut target, &mask, Point::new(0, 1), Rect::new(0, 0, 2, 3));

        assert_eq!(target.get_pixel(0, 0), Some(Color::rgba(7, 8, 9, 0)));
        assert_eq!(target.get_pixel(0, 1), Some(Color::rgba(7, 8, 9, 240)));
        assert_eq!(target.get_pixel(0, 2), Some(Color::rgba(7, 8, 9, 0)));
    }

    #[test]
    fn disjoint_mask_leaves_only_zero_coverage_in_clip() {
        let mut target = Framebuffer::new(2, 1, Color::rgba(1, 2, 3, 255)).unwrap();
        let mask = Framebuffer::new(1, 1, Color::WHITE).unwrap();

        apply_alpha_mask_clipped(&mut target, &mask, Point::new(10, 10), Rect::new(0, 0, 2, 1));

        assert_eq!(target.get_pixel(0, 0), Some(Color::rgba(1, 2, 3, 0)));
        assert_eq!(target.get_pixel(1, 0), Some(Color::rgba(1, 2, 3, 0)));
    }
}
