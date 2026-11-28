//! Precomputed texture mip levels and trilinear sampling.

use crate::{Color, Texture, TextureError};

/// A texture and its progressively downsampled levels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MipmapChain {
    levels: Vec<Texture>,
}

#[derive(Clone, Copy)]
struct PreparedTrilinear {
    lower: usize,
    upper: usize,
    fraction: f32,
    lower_inverse_scale: f32,
    upper_inverse_scale: f32,
}

impl PreparedTrilinear {
    fn new(level_count: usize, lod: f32) -> Self {
        let clamped_lod = if lod.is_finite() {
            lod.clamp(0.0, (level_count - 1) as f32)
        } else {
            0.0
        };
        let lower = clamped_lod.floor() as usize;
        let upper = (lower + 1).min(level_count - 1);
        Self {
            lower,
            upper,
            fraction: clamped_lod - lower as f32,
            lower_inverse_scale: 2.0_f32.powi(-(lower as i32)),
            upper_inverse_scale: 2.0_f32.powi(-(upper as i32)),
        }
    }
}

impl MipmapChain {
    pub fn new(base: &Texture) -> Result<Self, TextureError> {
        let mut levels = Vec::new();
        levels
            .try_reserve_exact(1)
            .map_err(|_| TextureError::AllocationFailed)?;
        levels.push(base.try_clone()?);

        while levels
            .last()
            .is_some_and(|level| level.width() > 1 || level.height() > 1)
        {
            let Some(previous) = levels.last() else {
                return Err(TextureError::ZeroDimension);
            };
            let width = previous.width().div_ceil(2);
            let height = previous.height().div_ceil(2);
            let pixel_count = (width as usize)
                .checked_mul(height as usize)
                .ok_or(TextureError::SizeOverflow)?;
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(pixel_count)
                .map_err(|_| TextureError::AllocationFailed)?;

            for y in 0..height {
                for x in 0..width {
                    let x0 = x * 2;
                    let y0 = y * 2;
                    let x1 = (x0 + 1).min(previous.width() - 1);
                    let y1 = (y0 + 1).min(previous.height() - 1);
                    let mut samples = [Color::TRANSPARENT; 4];
                    let mut sample_count = 0;
                    for sample_y in y0..=y1 {
                        for sample_x in x0..=x1 {
                            let index =
                                sample_y as usize * previous.width() as usize + sample_x as usize;
                            samples[sample_count] = previous.pixels()[index];
                            sample_count += 1;
                        }
                    }
                    pixels.push(average_premultiplied(&samples[..sample_count]));
                }
            }
            let level = Texture::try_from_pixels(width, height, &pixels)?;
            levels
                .try_reserve(1)
                .map_err(|_| TextureError::AllocationFailed)?;
            levels.push(level);
        }
        Ok(Self { levels })
    }

    #[must_use]
    pub fn levels(&self) -> &[Texture] {
        &self.levels
    }

    /// Samples between adjacent mip levels using premultiplied-alpha interpolation.
    #[must_use]
    pub fn sample_trilinear(&self, x: f32, y: f32, lod: f32) -> Color {
        if self.levels.is_empty() || !x.is_finite() || !y.is_finite() {
            return Color::TRANSPARENT;
        }
        let prepared = PreparedTrilinear::new(self.levels.len(), lod);
        sample_trilinear_prepared(self, x, y, prepared)
    }
}

/// Draws a mipmapped texture into a destination rectangle.
///
/// The level of detail is selected from the downscale ratio. Sampling and
/// compositing are clipped to the destination framebuffer.
pub fn draw_mipmapped_texture_clipped(
    target: &mut crate::Framebuffer,
    mipmaps: &MipmapChain,
    destination: crate::Rect,
    clip: crate::Rect,
) {
    if destination.width == 0 || destination.height == 0 {
        return;
    }
    let Some(clip) = clip
        .intersection(crate::Rect::new(0, 0, target.width(), target.height()))
        .and_then(|framebuffer_clip| framebuffer_clip.intersection(destination))
    else {
        return;
    };

    let Some(base) = mipmaps.levels.first() else {
        return;
    };

    // Exact 2:1 downscales land on texel centers of mip level 1. Avoid
    // per-pixel LOD calculation and bilinear sampling for this common case.
    if base.width() == destination.width.checked_mul(2).unwrap_or(0)
        && base.height() == destination.height.checked_mul(2).unwrap_or(0)
    {
        if let Some(level) = mipmaps.levels.get(1) {
            if level.width() == destination.width && level.height() == destination.height {
                let target_width = target.width() as usize;
                let source_width = level.width() as usize;
                let copy_width = clip.width as usize;
                let source_x = (clip.x - destination.x) as usize;
                let target_x = clip.x as usize;
                let source_y = (clip.y - destination.y) as usize;

                for row in 0..clip.height as usize {
                    let source_start = (source_y + row) * source_width + source_x;
                    let target_start = (clip.y as usize + row) * target_width + target_x;
                    let source_row = &level.pixels()[source_start..source_start + copy_width];
                    let target_row =
                        &mut target.pixels_mut()[target_start..target_start + copy_width];

                    if level.is_fully_opaque() {
                        target_row.copy_from_slice(source_row);
                    } else {
                        for (destination_pixel, source_pixel) in
                            target_row.iter_mut().zip(source_row.iter().copied())
                        {
                            if source_pixel.a == 255 {
                                *destination_pixel = source_pixel;
                            } else if source_pixel.a > 0 {
                                *destination_pixel = source_pixel.blend_over(*destination_pixel);
                            }
                        }
                    }
                }
                return;
            }
        }
    }

    let scale_x = base.width() as f32 / destination.width as f32;
    let scale_y = base.height() as f32 / destination.height as f32;
    let lod = scale_x.max(scale_y).log2().max(0.0);
    let prepared = PreparedTrilinear::new(mipmaps.levels.len(), lod);
    let lower = &mipmaps.levels[prepared.lower];
    let upper = &mipmaps.levels[prepared.upper];

    // Opaque mip levels are common for game and UI assets. Prepare scanline
    // sampling once per row and write directly, avoiding generic alpha handling
    // and repeated texture-bound checks for every output pixel.
    if lower.is_fully_opaque() && upper.is_fully_opaque() {
        draw_opaque_trilinear_clipped(
            target,
            base,
            lower,
            upper,
            destination,
            clip,
            scale_x,
            scale_y,
            prepared,
        );
        return;
    }

    let target_width = target.width() as usize;
    let target_pixels = target.pixels_mut();

    for y in clip.y..clip.y + clip.height {
        let source_y = ((y as f32 + 0.5 - destination.y as f32) * scale_y - 0.5)
            .clamp(0.0, base.height() as f32 - 1.0);
        for x in clip.x..clip.x + clip.width {
            let source_x = ((x as f32 + 0.5 - destination.x as f32) * scale_x - 0.5)
                .clamp(0.0, base.width() as f32 - 1.0);
            let source = sample_trilinear_prepared(mipmaps, source_x, source_y, prepared);
            if source.a == 0 {
                continue;
            }
            let index = y as usize * target_width + x as usize;
            let destination_pixel = &mut target_pixels[index];
            *destination_pixel = if source.a == 255 {
                source
            } else {
                source.blend_over(*destination_pixel)
            };
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn draw_opaque_trilinear_clipped(
    target: &mut crate::Framebuffer,
    base: &Texture,
    lower: &Texture,
    upper: &Texture,
    destination: crate::Rect,
    clip: crate::Rect,
    scale_x: f32,
    scale_y: f32,
    prepared: PreparedTrilinear,
) {
    let target_width = target.width() as usize;
    let target_pixels = target.pixels_mut();
    let lower_width = lower.width() as usize;
    let lower_height = lower.height() as usize;
    let upper_width = upper.width() as usize;
    let upper_height = upper.height() as usize;
    let lower_pixels = lower.pixels();
    let upper_pixels = upper.pixels();
    let interpolate_levels = prepared.lower != prepared.upper && prepared.fraction > 0.0;
    let clip_left = clip.x as usize;
    let clip_width = clip.width as usize;

    if interpolate_levels {
        for y in clip.y..clip.y + clip.height {
            let source_y = ((y as f32 + 0.5 - destination.y as f32) * scale_y - 0.5)
                .clamp(0.0, base.height() as f32 - 1.0);
            let lower_y = ((source_y + 0.5) * prepared.lower_inverse_scale - 0.5)
                .clamp(0.0, lower_height as f32 - 1.0);
            let lower_y0 = lower_y.floor() as usize;
            let lower_y1 = (lower_y0 + 1).min(lower_height - 1);
            let lower_ty = lower_y - lower_y0 as f32;
            let lower_top = &lower_pixels[lower_y0 * lower_width..(lower_y0 + 1) * lower_width];
            let lower_bottom = &lower_pixels[lower_y1 * lower_width..(lower_y1 + 1) * lower_width];

            let upper_y = ((source_y + 0.5) * prepared.upper_inverse_scale - 0.5)
                .clamp(0.0, upper_height as f32 - 1.0);
            let upper_y0 = upper_y.floor() as usize;
            let upper_y1 = (upper_y0 + 1).min(upper_height - 1);
            let upper_ty = upper_y - upper_y0 as f32;
            let upper_top = &upper_pixels[upper_y0 * upper_width..(upper_y0 + 1) * upper_width];
            let upper_bottom = &upper_pixels[upper_y1 * upper_width..(upper_y1 + 1) * upper_width];

            let row_start = y as usize * target_width + clip_left;
            let target_row = &mut target_pixels[row_start..row_start + clip_width];
            for (column, x) in (clip.x..clip.x + clip.width).enumerate() {
                let source_x = ((x as f32 + 0.5 - destination.x as f32) * scale_x - 0.5)
                    .clamp(0.0, base.width() as f32 - 1.0);

                let lower_x = ((source_x + 0.5) * prepared.lower_inverse_scale - 0.5)
                    .clamp(0.0, lower_width as f32 - 1.0);
                let lower_x0 = lower_x.floor() as usize;
                let lower_x1 = (lower_x0 + 1).min(lower_width - 1);
                let lower_tx = lower_x - lower_x0 as f32;
                let lower_sample = sample_bilinear_opaque_rows(
                    lower_top,
                    lower_bottom,
                    lower_x0,
                    lower_x1,
                    lower_tx,
                    lower_ty,
                );

                let upper_x = ((source_x + 0.5) * prepared.upper_inverse_scale - 0.5)
                    .clamp(0.0, upper_width as f32 - 1.0);
                let upper_x0 = upper_x.floor() as usize;
                let upper_x1 = (upper_x0 + 1).min(upper_width - 1);
                let upper_tx = upper_x - upper_x0 as f32;
                let upper_sample = sample_bilinear_opaque_rows(
                    upper_top,
                    upper_bottom,
                    upper_x0,
                    upper_x1,
                    upper_tx,
                    upper_ty,
                );

                target_row[column] =
                    interpolate_premultiplied(lower_sample, upper_sample, prepared.fraction);
            }
        }
    } else {
        for y in clip.y..clip.y + clip.height {
            let source_y = ((y as f32 + 0.5 - destination.y as f32) * scale_y - 0.5)
                .clamp(0.0, base.height() as f32 - 1.0);
            let lower_y = ((source_y + 0.5) * prepared.lower_inverse_scale - 0.5)
                .clamp(0.0, lower_height as f32 - 1.0);
            let lower_y0 = lower_y.floor() as usize;
            let lower_y1 = (lower_y0 + 1).min(lower_height - 1);
            let lower_ty = lower_y - lower_y0 as f32;
            let lower_top =
                &lower_pixels[lower_y0 * lower_width..(lower_y0 + 1) * lower_width];
            let lower_bottom =
                &lower_pixels[lower_y1 * lower_width..(lower_y1 + 1) * lower_width];

            let row_start = y as usize * target_width + clip_left;
            let target_row = &mut target_pixels[row_start..row_start + clip_width];
            for (column, x) in (clip.x..clip.x + clip.width).enumerate() {
                let source_x = ((x as f32 + 0.5 - destination.x as f32) * scale_x - 0.5)
                    .clamp(0.0, base.width() as f32 - 1.0);
                let lower_x = ((source_x + 0.5) * prepared.lower_inverse_scale - 0.5)
                    .clamp(0.0, lower_width as f32 - 1.0);
                let lower_x0 = lower_x.floor() as usize;
                let lower_x1 = (lower_x0 + 1).min(lower_width - 1);
                let lower_tx = lower_x - lower_x0 as f32;
                target_row[column] = sample_bilinear_opaque_rows(
                    lower_top,
                    lower_bottom,
                    lower_x0,
                    lower_x1,
                    lower_tx,
                    lower_ty,
                );
            }
        }
    }
}

#[inline]
fn sample_bilinear_opaque_rows(
    top_row: &[Color],
    bottom_row: &[Color],
    x0: usize,
    x1: usize,
    tx: f32,
    ty: f32,
) -> Color {
    let top_left = top_row[x0];
    let top_right = top_row[x1];
    let bottom_left = bottom_row[x0];
    let bottom_right = bottom_row[x1];
    let inverse_x = 1.0 - tx;
    let inverse_y = 1.0 - ty;
    let interpolate = |top_left: u8, top_right: u8, bottom_left: u8, bottom_right: u8| {
        let top = f32::from(top_left) * inverse_x + f32::from(top_right) * tx;
        let bottom = f32::from(bottom_left) * inverse_x + f32::from(bottom_right) * tx;
        (top * inverse_y + bottom * ty).round().clamp(0.0, 255.0) as u8
    };

    Color::rgb(
        interpolate(top_left.r, top_right.r, bottom_left.r, bottom_right.r),
        interpolate(top_left.g, top_right.g, bottom_left.g, bottom_right.g),
        interpolate(top_left.b, top_right.b, bottom_left.b, bottom_right.b),
    )
}

#[inline]
fn sample_trilinear_prepared(
    mipmaps: &MipmapChain,
    x: f32,
    y: f32,
    prepared: PreparedTrilinear,
) -> Color {
    let lower_x = (x + 0.5) * prepared.lower_inverse_scale - 0.5;
    let lower_y = (y + 0.5) * prepared.lower_inverse_scale - 0.5;
    let a = sample_bilinear(&mipmaps.levels[prepared.lower], lower_x, lower_y);
    if prepared.lower == prepared.upper || prepared.fraction <= 0.0 {
        return a;
    }

    let upper_x = (x + 0.5) * prepared.upper_inverse_scale - 0.5;
    let upper_y = (y + 0.5) * prepared.upper_inverse_scale - 0.5;
    let b = sample_bilinear(&mipmaps.levels[prepared.upper], upper_x, upper_y);
    interpolate_premultiplied(a, b, prepared.fraction)
}

fn sample_bilinear(texture: &Texture, x: f32, y: f32) -> Color {
    if texture.width() == 0 || texture.height() == 0 || !x.is_finite() || !y.is_finite() {
        return Color::TRANSPARENT;
    }
    let x = x.clamp(0.0, texture.width() as f32 - 1.0);
    let y = y.clamp(0.0, texture.height() as f32 - 1.0);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let pixels = texture.pixels();
    let row_width = texture.width() as usize;
    if x == x0 as f32 && y == y0 as f32 {
        return pixels[y0 as usize * row_width + x0 as usize];
    }
    if texture.is_fully_opaque() {
        return crate::texture::sample_pixels_bilinear_opaque(
            texture.pixels(),
            texture.width(),
            texture.height(),
            x,
            y,
        );
    }
    let x1 = (x0 + 1).min(texture.width() - 1);
    let y1 = (y0 + 1).min(texture.height() - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let top_left = pixels[y0 as usize * row_width + x0 as usize];
    let top_right = pixels[y0 as usize * row_width + x1 as usize];
    let bottom_left = pixels[y1 as usize * row_width + x0 as usize];
    let bottom_right = pixels[y1 as usize * row_width + x1 as usize];
    let top = interpolate_premultiplied(top_left, top_right, tx);
    let bottom = interpolate_premultiplied(bottom_left, bottom_right, tx);
    interpolate_premultiplied(top, bottom, ty)
}

#[inline]
fn average_premultiplied(colors: &[Color]) -> Color {
    debug_assert!(!colors.is_empty());
    let mut alpha_sum = 0u32;
    let mut red_sum = 0u32;
    let mut green_sum = 0u32;
    let mut blue_sum = 0u32;

    for color in colors {
        let alpha = u32::from(color.a);
        alpha_sum += alpha;
        red_sum += u32::from(color.r) * alpha;
        green_sum += u32::from(color.g) * alpha;
        blue_sum += u32::from(color.b) * alpha;
    }

    if alpha_sum == 0 {
        return Color::TRANSPARENT;
    }

    let channel = |sum: u32| ((sum + alpha_sum / 2) / alpha_sum).min(255) as u8;
    let alpha = ((alpha_sum + colors.len() as u32 / 2) / colors.len() as u32).min(255) as u8;
    Color::rgba(
        channel(red_sum),
        channel(green_sum),
        channel(blue_sum),
        alpha,
    )
}

fn interpolate_premultiplied(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let alpha_a = f32::from(a.a);
    let alpha_b = f32::from(b.a);
    let alpha = alpha_a * (1.0 - t) + alpha_b * t;
    if alpha <= f32::EPSILON {
        return Color::TRANSPARENT;
    }

    let channel = |channel_a: u8, channel_b: u8| {
        ((f32::from(channel_a) * alpha_a * (1.0 - t) + f32::from(channel_b) * alpha_b * t) / alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgba(
        channel(a.r, b.r),
        channel(a.g, b.g),
        channel(a.b, b.b),
        alpha.round().clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_complete_mipmap_chain_and_samples_levels() {
        let base = Texture::new(4, 4, vec![Color::WHITE; 16]).unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        assert_eq!(
            chain
                .levels()
                .iter()
                .map(|level| (level.width(), level.height()))
                .collect::<Vec<_>>(),
            vec![(4, 4), (2, 2), (1, 1)]
        );
        assert_eq!(chain.sample_trilinear(1.0, 1.0, 1.5), Color::WHITE);
    }

    #[test]
    fn odd_sized_mipmaps_average_only_existing_edge_texels() {
        let horizontal =
            Texture::new(3, 1, vec![Color::BLACK, Color::BLACK, Color::WHITE]).unwrap();
        let horizontal_chain = MipmapChain::new(&horizontal).unwrap();
        assert_eq!(
            horizontal_chain.levels()[1].get_pixel(1, 0),
            Some(Color::WHITE),
        );

        let vertical = Texture::new(1, 3, vec![Color::BLACK, Color::BLACK, Color::WHITE]).unwrap();
        let vertical_chain = MipmapChain::new(&vertical).unwrap();
        assert_eq!(
            vertical_chain.levels()[1].get_pixel(0, 1),
            Some(Color::WHITE),
        );
    }

    #[test]
    fn odd_sized_mipmaps_keep_partial_alpha_edges_unbiased() {
        let base = Texture::new(
            3,
            1,
            vec![
                Color::TRANSPARENT,
                Color::TRANSPARENT,
                Color::rgba(20, 80, 160, 128),
            ],
        )
        .unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        assert_eq!(
            chain.levels()[1].get_pixel(1, 0),
            Some(Color::rgba(20, 80, 160, 128)),
        );
    }

    #[test]
    fn transparent_texels_do_not_bleed_rgb_into_mip_levels() {
        let base = Texture::new(2, 1, vec![Color::RED, Color::TRANSPARENT]).unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        assert_eq!(
            chain.levels()[1].get_pixel(0, 0),
            Some(Color::rgba(255, 0, 0, 128)),
        );
    }

    #[test]
    fn mipmapped_draw_downscales_and_respects_clipping() {
        let base = Texture::new(
            2,
            2,
            vec![Color::RED, Color::GREEN, Color::BLUE, Color::WHITE],
        )
        .unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        let mut target = crate::Framebuffer::new(3, 2, Color::BLACK).unwrap();
        draw_mipmapped_texture_clipped(
            &mut target,
            &chain,
            crate::Rect::new(1, 0, 1, 1),
            crate::Rect::new(1, 0, 1, 1),
        );
        assert_eq!(target.get_pixel(1, 0), Some(Color::rgb(128, 128, 128)));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn exact_two_to_one_opaque_downscale_copies_mip_rows_with_clipping() {
        let base = Texture::new(
            4,
            2,
            vec![
                Color::RED,
                Color::RED,
                Color::GREEN,
                Color::GREEN,
                Color::BLUE,
                Color::BLUE,
                Color::WHITE,
                Color::WHITE,
            ],
        )
        .unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        let mut actual = crate::Framebuffer::new(4, 3, Color::BLACK).unwrap();
        let mut expected = actual.clone();
        let destination = crate::Rect::new(1, 1, 2, 1);
        let clip = crate::Rect::new(2, 1, 1, 1);

        draw_mipmapped_texture_clipped(&mut actual, &chain, destination, clip);

        let reference = chain.sample_trilinear(2.5, 0.5, 1.0);
        expected.set_pixel(2, 1, reference);
        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn exact_two_to_one_translucent_downscale_blends_instead_of_replacing() {
        let base = Texture::new(
            2,
            2,
            vec![
                Color::rgba(255, 0, 0, 128),
                Color::rgba(255, 0, 0, 128),
                Color::rgba(255, 0, 0, 128),
                Color::rgba(255, 0, 0, 128),
            ],
        )
        .unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        let mut actual = crate::Framebuffer::new(1, 1, Color::BLUE).unwrap();
        let mut expected = actual.clone();

        draw_mipmapped_texture_clipped(
            &mut actual,
            &chain,
            crate::Rect::new(0, 0, 1, 1),
            crate::Rect::new(0, 0, 1, 1),
        );
        let source = chain.levels()[1].get_pixel(0, 0).unwrap();
        expected.set_pixel(0, 0, source.blend_over(Color::BLUE));

        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn prepared_trilinear_sampling_matches_public_sampling_across_lods() {
        let base = Texture::new(
            7,
            5,
            (0..35)
                .map(|index| {
                    let alpha = (index * 37 % 256) as u8;
                    Color::rgba(
                        (index * 11 % 256) as u8,
                        (index * 19 % 256) as u8,
                        (index * 23 % 256) as u8,
                        alpha,
                    )
                })
                .collect(),
        )
        .unwrap();
        let chain = MipmapChain::new(&base).unwrap();

        for lod in [0.0_f32, 0.25, 1.0, 1.5, 2.0, 20.0] {
            let clamped = lod.clamp(0.0, (chain.levels.len() - 1) as f32);
            let lower = clamped.floor() as usize;
            let upper = (lower + 1).min(chain.levels.len() - 1);
            let fraction = clamped - lower as f32;
            for y in [0.0, 1.25, 3.75, 4.0] {
                for x in [0.0, 0.5, 2.25, 6.0] {
                    let prepared = sample_trilinear_prepared(
                        &chain,
                        x,
                        y,
                        PreparedTrilinear {
                            lower,
                            upper,
                            fraction,
                            lower_inverse_scale: 2.0_f32.powi(-(lower as i32)),
                            upper_inverse_scale: 2.0_f32.powi(-(upper as i32)),
                        },
                    );
                    assert_eq!(prepared, chain.sample_trilinear(x, y, lod));
                }
            }
        }
    }

    #[test]
    fn fractional_lod_draw_matches_per_pixel_reference_with_alpha_and_clip() {
        let base = Texture::new(
            9,
            7,
            (0..63)
                .map(|index| {
                    Color::rgba(
                        (index * 17 % 256) as u8,
                        (index * 29 % 256) as u8,
                        (index * 41 % 256) as u8,
                        (index * 13 % 256) as u8,
                    )
                })
                .collect(),
        )
        .unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        let destination = crate::Rect::new(2, 1, 4, 3);
        let clip = crate::Rect::new(3, 2, 2, 2);
        let mut actual = crate::Framebuffer::new(9, 7, Color::rgba(11, 23, 37, 255)).unwrap();
        let mut expected = actual.clone();

        draw_mipmapped_texture_clipped(&mut actual, &chain, destination, clip);

        let scale_x = base.width() as f32 / destination.width as f32;
        let scale_y = base.height() as f32 / destination.height as f32;
        let lod = scale_x.max(scale_y).log2().max(0.0);
        for y in clip.y..clip.y + clip.height {
            let source_y = ((y as f32 + 0.5 - destination.y as f32) * scale_y - 0.5)
                .clamp(0.0, base.height() as f32 - 1.0);
            for x in clip.x..clip.x + clip.width {
                let source_x = ((x as f32 + 0.5 - destination.x as f32) * scale_x - 0.5)
                    .clamp(0.0, base.width() as f32 - 1.0);
                let source = chain.sample_trilinear(source_x, source_y, lod);
                let destination_pixel = expected.get_pixel(x, y).unwrap();
                expected.set_pixel(
                    x,
                    y,
                    if source.a == 255 {
                        source
                    } else {
                        source.blend_over(destination_pixel)
                    },
                );
            }
        }

        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn opaque_fractional_lod_draw_matches_per_pixel_reference_with_clip() {
        let base = Texture::new(
            17,
            11,
            (0..187)
                .map(|index| {
                    Color::rgb(
                        (index * 17 % 256) as u8,
                        (index * 29 % 256) as u8,
                        (index * 41 % 256) as u8,
                    )
                })
                .collect(),
        )
        .unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        assert!(chain.levels().iter().all(Texture::is_fully_opaque));

        let destination = crate::Rect::new(1, 2, 6, 4);
        let clip = crate::Rect::new(2, 2, 4, 3);
        let mut actual = crate::Framebuffer::new(10, 8, Color::rgba(11, 23, 37, 120)).unwrap();
        let mut expected = actual.clone();

        draw_mipmapped_texture_clipped(&mut actual, &chain, destination, clip);

        let scale_x = base.width() as f32 / destination.width as f32;
        let scale_y = base.height() as f32 / destination.height as f32;
        let lod = scale_x.max(scale_y).log2().max(0.0);
        assert!(lod.fract() > 0.0);
        for y in clip.y..clip.y + clip.height {
            let source_y = ((y as f32 + 0.5 - destination.y as f32) * scale_y - 0.5)
                .clamp(0.0, base.height() as f32 - 1.0);
            for x in clip.x..clip.x + clip.width {
                let source_x = ((x as f32 + 0.5 - destination.x as f32) * scale_x - 0.5)
                    .clamp(0.0, base.width() as f32 - 1.0);
                expected.set_pixel(x, y, chain.sample_trilinear(source_x, source_y, lod));
            }
        }

        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn invalid_lod_is_safely_clamped() {
        let base = Texture::new(1, 1, vec![Color::BLUE]).unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        assert_eq!(chain.sample_trilinear(0.0, 0.0, f32::NAN), Color::BLUE);
    }

    #[test]
    fn non_finite_coordinates_return_transparent_without_sampling() {
        let base = Texture::new(2, 2, vec![Color::BLUE; 4]).unwrap();
        let chain = MipmapChain::new(&base).unwrap();

        assert_eq!(
            chain.sample_trilinear(f32::NAN, 0.0, 0.0),
            Color::TRANSPARENT
        );
        assert_eq!(
            chain.sample_trilinear(0.0, f32::INFINITY, 0.0),
            Color::TRANSPARENT
        );
        assert_eq!(
            chain.sample_trilinear(f32::NEG_INFINITY, 1.0, 1.0),
            Color::TRANSPARENT
        );
    }
}
