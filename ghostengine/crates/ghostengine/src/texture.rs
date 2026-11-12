//! CPU-side image storage and scaled texture compositing.

use crate::{Color, Framebuffer, Rect, Transform2D};

/// Errors returned when constructing an in-memory texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureError {
    ZeroDimension,
    SizeOverflow,
    PixelCountMismatch,
}

/// Sampling mode used when a texture or transformed layer is scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFilter {
    /// Selects the closest source texel.
    Nearest,
    /// Interpolates four neighboring texels in premultiplied-alpha space.
    Bilinear,
}

/// An immutable, tightly packed RGBA image suitable for CPU rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    width: u32,
    height: u32,
    pixels: Vec<Color>,
    fully_opaque: bool,
}

impl Texture {
    /// Creates a texture from row-major pixels.
    pub fn new(width: u32, height: u32, pixels: Vec<Color>) -> Result<Self, TextureError> {
        if width == 0 || height == 0 {
            return Err(TextureError::ZeroDimension);
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .ok_or(TextureError::SizeOverflow)?;
        if pixels.len() != expected {
            return Err(TextureError::PixelCountMismatch);
        }
        let fully_opaque = pixels.iter().all(|pixel| pixel.a == 255);
        Ok(Self {
            width,
            height,
            pixels,
            fully_opaque,
        })
    }

    /// Copies the pixels from an existing framebuffer.
    #[must_use]
    pub fn from_framebuffer(framebuffer: &Framebuffer) -> Self {
        Self {
            width: framebuffer.width(),
            height: framebuffer.height(),
            pixels: framebuffer.pixels().to_vec(),
            fully_opaque: framebuffer.pixels().iter().all(|pixel| pixel.a == 255),
        }
    }

    /// Converts an owned framebuffer into a texture without copying its pixels.
    ///
    /// Use this when the framebuffer is no longer needed as a render target.
    /// The pixel allocation is transferred directly into the immutable texture.
    #[must_use]
    pub fn from_framebuffer_owned(framebuffer: Framebuffer) -> Self {
        let (width, height, pixels) = framebuffer.into_parts();
        let fully_opaque = pixels.iter().all(|pixel| pixel.a == 255);
        Self {
            width,
            height,
            pixels,
            fully_opaque,
        }
    }

    /// Returns true when every texel has fully opaque alpha.
    ///
    /// This property is computed once at construction and allows renderers to
    /// avoid per-pixel alpha tests for common opaque assets.
    #[must_use]
    pub const fn is_fully_opaque(&self) -> bool {
        self.fully_opaque
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixels(&self) -> &[Color] {
        &self.pixels
    }

    #[must_use]
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels
            .get((y as usize) * (self.width as usize) + (x as usize))
            .copied()
    }

    fn sample_nearest(&self, x: f32, y: f32) -> Color {
        sample_pixels_nearest(&self.pixels, self.width, self.height, x, y)
    }

    fn sample_bilinear(&self, x: f32, y: f32) -> Color {
        sample_pixels_bilinear(&self.pixels, self.width, self.height, x, y)
    }
}

pub(crate) fn sample_pixels_nearest(
    pixels: &[Color],
    width: u32,
    height: u32,
    x: f32,
    y: f32,
) -> Color {
    if width == 0 || height == 0 {
        return Color::TRANSPARENT;
    }
    let sx = x.floor().clamp(0.0, width as f32 - 1.0) as u32;
    let sy = y.floor().clamp(0.0, height as f32 - 1.0) as u32;
    pixels[sy as usize * width as usize + sx as usize]
}

pub(crate) fn sample_pixels_bilinear(
    pixels: &[Color],
    width: u32,
    height: u32,
    x: f32,
    y: f32,
) -> Color {
    if width == 0 || height == 0 {
        return Color::TRANSPARENT;
    }
    let x = x.clamp(0.0, width as f32 - 1.0);
    let y = y.clamp(0.0, height as f32 - 1.0);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    sample_bilinear_prepared(
        pixels,
        width as usize,
        BilinearCoordinates {
            x0: x0 as usize,
            x1: (x0 + 1).min(width - 1) as usize,
            y0: y0 as usize,
            y1: (y0 + 1).min(height - 1) as usize,
            tx: x - x0 as f32,
            ty: y - y0 as f32,
        },
    )
}

#[derive(Clone, Copy)]
struct BilinearCoordinates {
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
    tx: f32,
    ty: f32,
}

#[inline]
fn sample_bilinear_prepared(
    pixels: &[Color],
    row_width: usize,
    coordinates: BilinearCoordinates,
) -> Color {
    let BilinearCoordinates {
        x0,
        x1,
        y0,
        y1,
        tx,
        ty,
    } = coordinates;
    let top_left = pixels[y0 * row_width + x0];
    let top_right = pixels[y0 * row_width + x1];
    let bottom_left = pixels[y1 * row_width + x0];
    let bottom_right = pixels[y1 * row_width + x1];

    // Interpolate premultiplied channels in two separable passes. This avoids
    // building four bilinear weights and multiplying every texel by each weight.
    let one_minus_x = 1.0 - tx;
    let one_minus_y = 1.0 - ty;
    let weighted_alpha_top_left = f32::from(top_left.a) * one_minus_x;
    let weighted_alpha_top_right = f32::from(top_right.a) * tx;
    let weighted_alpha_bottom_left = f32::from(bottom_left.a) * one_minus_x;
    let weighted_alpha_bottom_right = f32::from(bottom_right.a) * tx;
    let alpha_top = weighted_alpha_top_left + weighted_alpha_top_right;
    let alpha_bottom = weighted_alpha_bottom_left + weighted_alpha_bottom_right;
    let alpha = alpha_top * one_minus_y + alpha_bottom * ty;
    if alpha <= f32::EPSILON {
        return Color::TRANSPARENT;
    }

    // One reciprocal is shared by RGB instead of performing three divisions.
    let inverse_alpha = alpha.recip();
    let interpolate_premultiplied = |top_left_channel: u8,
                                     top_right_channel: u8,
                                     bottom_left_channel: u8,
                                     bottom_right_channel: u8| {
        let top = f32::from(top_left_channel) * weighted_alpha_top_left
            + f32::from(top_right_channel) * weighted_alpha_top_right;
        let bottom = f32::from(bottom_left_channel) * weighted_alpha_bottom_left
            + f32::from(bottom_right_channel) * weighted_alpha_bottom_right;
        (top * one_minus_y + bottom * ty) * inverse_alpha
    };
    let channel = |value: f32| value.round().clamp(0.0, 255.0) as u8;
    Color::rgba(
        channel(interpolate_premultiplied(top_left.r, top_right.r, bottom_left.r, bottom_right.r)),
        channel(interpolate_premultiplied(top_left.g, top_right.g, bottom_left.g, bottom_right.g)),
        channel(interpolate_premultiplied(top_left.b, top_right.b, bottom_left.b, bottom_right.b)),
        channel(alpha),
    )
}

#[inline]
fn sample_bilinear_opaque_prepared(
    pixels: &[Color],
    row_width: usize,
    coordinates: BilinearCoordinates,
) -> Color {
    let BilinearCoordinates {
        x0,
        x1,
        y0,
        y1,
        tx,
        ty,
    } = coordinates;
    let top_left = pixels[y0 * row_width + x0];
    let top_right = pixels[y0 * row_width + x1];
    let bottom_left = pixels[y1 * row_width + x0];
    let bottom_right = pixels[y1 * row_width + x1];

    let inverse_x = 1.0 - tx;
    let inverse_y = 1.0 - ty;
    let interpolate = |top_left: u8, top_right: u8, bottom_left: u8, bottom_right: u8| {
        let top = f32::from(top_left) * inverse_x + f32::from(top_right) * tx;
        let bottom = f32::from(bottom_left) * inverse_x + f32::from(bottom_right) * tx;
        (top * inverse_y + bottom * ty)
            .round()
            .clamp(0.0, 255.0) as u8
    };

    Color::rgb(
        interpolate(top_left.r, top_right.r, bottom_left.r, bottom_right.r),
        interpolate(top_left.g, top_right.g, bottom_left.g, bottom_right.g),
        interpolate(top_left.b, top_right.b, bottom_left.b, bottom_right.b),
    )
}

#[inline]
fn sample_cached_bilinear_opaque(
    top: &[f32; 4],
    bottom: &[f32; 4],
    inverse_y: f32,
    ty: f32,
) -> Color {
    let channel = |index: usize| {
        ((top[index] * inverse_y + bottom[index] * ty) / 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgb(channel(0), channel(1), channel(2))
}

// The cached bilinear path stores horizontal samples as premultiplied RGBA
// floats. Keep the opaque-destination blend in safe Rust because the crate
// explicitly forbids unsafe code.
#[inline]
fn blend_cached_bilinear_over_opaque(
    top: &[f32; 4],
    bottom: &[f32; 4],
    inverse_y: f32,
    ty: f32,
    destination: &mut Color,
) {
    let alpha = top[3] * inverse_y + bottom[3] * ty;
    if alpha <= f32::EPSILON {
        return;
    }

    let inverse_alpha = 255.0 - alpha;
    let channel = |index: usize, background: u8| {
        ((top[index] * inverse_y
            + bottom[index] * ty
            + f32::from(background) * inverse_alpha)
            / 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    destination.r = channel(0, destination.r);
    destination.g = channel(1, destination.g);
    destination.b = channel(2, destination.b);
}

/// Draws a texture into a destination rectangle, scaling and alpha-compositing it.
///
/// The destination is clipped to the framebuffer. A zero-sized destination draws
/// nothing. Bilinear filtering interpolates premultiplied colors to avoid transparent
/// texels introducing colored fringes around image edges.
pub fn draw_texture(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
) {
    let clip = Rect::new(0, 0, target.width(), target.height());
    draw_texture_clipped(target, texture, destination, filter, clip);
}

/// Draws a scaled texture while restricting writes to a framebuffer-space viewport.
///
/// Sampling coordinates remain relative to the full destination rectangle, so
/// clipping an image does not rescale or shift the visible portion.
pub fn draw_texture_clipped(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
) {
    if destination.width == 0 || destination.height == 0 {
        return;
    }
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return;
    };

    // The common untransformed case does not need matrix inversion, corner
    // transforms, or a per-pixel inverse transform. Keep source coordinates
    // relative to the original destination so clipping never shifts sampling.
    let destination_right = u64::from(destination.x) + u64::from(destination.width);
    let destination_bottom = u64::from(destination.y) + u64::from(destination.height);
    let clip_right = u64::from(clip.x) + u64::from(clip.width);
    let clip_bottom = u64::from(clip.y) + u64::from(clip.height);
    let first_x = u64::from(destination.x).max(u64::from(clip.x));
    let first_y = u64::from(destination.y).max(u64::from(clip.y));
    let end_x = destination_right.min(clip_right);
    let end_y = destination_bottom.min(clip_bottom);
    if first_x >= end_x || first_y >= end_y {
        return;
    }

    let source_x_scale = texture.width as f32 / destination.width as f32;
    let source_y_scale = texture.height as f32 / destination.height as f32;
    // Nearest-neighbor horizontal coordinates are invariant across scanlines.
    // Prepare them once instead of repeating float mapping and floor conversion
    // for every destination pixel on every row.
    let nearest_columns = if filter == TextureFilter::Nearest {
        Some(
            (first_x..end_x)
                .map(|x| {
                    let source_x =
                        (x as f32 + 0.5 - destination.x as f32) * source_x_scale;
                    (source_x.floor() as u32).min(texture.width - 1) as usize
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let horizontal_samples = if filter == TextureFilter::Bilinear {
        let row_width = texture.width as usize;
        Some(
            (first_x..end_x)
                .map(|x| {
                    let source_x = (x as f32 + 0.5 - destination.x as f32) * source_x_scale - 0.5;
                    let source_x = source_x.clamp(0.0, texture.width as f32 - 1.0);
                    let x0 = source_x.floor() as usize;
                    (x0, (x0 + 1).min(row_width - 1), source_x - x0 as f32)
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    // Build the optional horizontal cache once per draw, never once per scanline.
    let horizontal_cache = if filter == TextureFilter::Bilinear {
        let samples = horizontal_samples.as_ref().expect("bilinear samples are prepared");
        let cache_len = (texture.height as usize).checked_mul(samples.len());
        let cache_bytes = cache_len.and_then(|len| len.checked_mul(core::mem::size_of::<[f32; 4]>()));
        let use_cache = end_y - first_y >= u64::from(texture.height) * 2
            && cache_bytes.is_some_and(|bytes| bytes <= 64 * 1024 * 1024);
        if use_cache {
            // The cache is fully overwritten during construction. Reserving
            // capacity and pushing rows avoids zero-initializing the entire
            // multi-megabyte buffer before filling it.
            let mut cache = Vec::with_capacity(cache_len.unwrap());
            let row_width = texture.width as usize;
            for source_row in 0..texture.height as usize {
                let source_offset = source_row * row_width;
                for &(x0, x1, tx) in samples {
                    let left = texture.pixels[source_offset + x0];
                    let right = texture.pixels[source_offset + x1];
                    let inverse_x = 1.0 - tx;
                    let left_alpha = f32::from(left.a) * inverse_x;
                    let right_alpha = f32::from(right.a) * tx;
                    cache.push([
                        f32::from(left.r) * left_alpha + f32::from(right.r) * right_alpha,
                        f32::from(left.g) * left_alpha + f32::from(right.g) * right_alpha,
                        f32::from(left.b) * left_alpha + f32::from(right.b) * right_alpha,
                        left_alpha + right_alpha,
                    ]);
                }
            }
            Some(cache)
        } else {
            None
        }
    } else {
        None
    };
    for y in first_y..end_y {
        let source_y = (y as f32 + 0.5 - destination.y as f32) * source_y_scale;
        let Some(row) = target.row_mut(y as u32) else {
            continue;
        };

        if filter == TextureFilter::Nearest {
            // The destination clipping guarantees that samples lie inside the
            // destination rectangle. Clamp only the final integer texel index,
            // avoiding the generic floating-point sampler for every pixel.
            let source_row =
                (source_y.floor() as u32).min(texture.height - 1) as usize * texture.width as usize;
            if texture.fully_opaque {
                for (pixel, &source_column) in row[first_x as usize..end_x as usize]
                    .iter_mut()
                    .zip(nearest_columns.as_ref().expect("nearest columns are prepared"))
                {
                    *pixel = texture.pixels[source_row + source_column];
                }
            } else {
                for (pixel, &source_column) in row[first_x as usize..end_x as usize]
                    .iter_mut()
                    .zip(nearest_columns.as_ref().expect("nearest columns are prepared"))
                {
                    let color = texture.pixels[source_row + source_column];
                    if color.a == 255 {
                        *pixel = color;
                    } else if color.a > 0 {
                        *pixel = color.blend_over(*pixel);
                    }
                }
            }
        } else {
            let source_y = (source_y - 0.5).clamp(0.0, texture.height as f32 - 1.0);
            let y0 = source_y.floor() as usize;
            let y1 = (y0 + 1).min(texture.height as usize - 1);
            let ty = source_y - y0 as f32;
            let row_width = texture.width as usize;
            let samples = horizontal_samples
                .as_ref()
                .expect("bilinear samples are prepared for bilinear filtering");

            if let Some(horizontal_cache) = horizontal_cache.as_ref() {
                let top_offset = y0 * samples.len();
                let bottom_offset = y1 * samples.len();
                let inverse_y = 1.0 - ty;
                for (pixel, (&top, &bottom)) in row[first_x as usize..end_x as usize]
                    .iter_mut()
                    .zip(
                        horizontal_cache[top_offset..top_offset + samples.len()]
                            .iter()
                            .zip(&horizontal_cache[bottom_offset..bottom_offset + samples.len()]),
                    )
                {
                    if texture.fully_opaque {
                        *pixel = sample_cached_bilinear_opaque(&top, &bottom, inverse_y, ty);
                    } else if pixel.a == 255 {
                        blend_cached_bilinear_over_opaque(&top, &bottom, inverse_y, ty, pixel);
                    } else {
                        let alpha = top[3] * inverse_y + bottom[3] * ty;
                        if alpha <= f32::EPSILON {
                            continue;
                        }
                        let inverse_alpha = alpha.recip();
                        let color = Color::rgba(
                            ((top[0] * inverse_y + bottom[0] * ty) * inverse_alpha)
                                .round()
                                .clamp(0.0, 255.0) as u8,
                            ((top[1] * inverse_y + bottom[1] * ty) * inverse_alpha)
                                .round()
                                .clamp(0.0, 255.0) as u8,
                            ((top[2] * inverse_y + bottom[2] * ty) * inverse_alpha)
                                .round()
                                .clamp(0.0, 255.0) as u8,
                            alpha.round().clamp(0.0, 255.0) as u8,
                        );
                        if color.a > 0 {
                            *pixel = color.blend_over(*pixel);
                        }
                    }
                }
            } else {
                for (pixel, &(x0, x1, tx)) in row[first_x as usize..end_x as usize]
                    .iter_mut()
                    .zip(samples.iter())
                {
                    let coordinates = BilinearCoordinates {
                        x0,
                        x1,
                        y0,
                        y1,
                        tx,
                        ty,
                    };
                    if texture.fully_opaque {
                        *pixel = sample_bilinear_opaque_prepared(
                            &texture.pixels,
                            row_width,
                            coordinates,
                        );
                    } else {
                        let color =
                            sample_bilinear_prepared(&texture.pixels, row_width, coordinates);
                        if color.a == 255 {
                            *pixel = color;
                        } else if color.a > 0 {
                            *pixel = color.blend_over(*pixel);
                        }
                    }
                }
            }
        }
    }
}

/// Draws a texture through an affine transform while preserving source sampling.
///
/// The destination rectangle is defined in local coordinates. The inverse transform
/// maps each destination pixel center back into that rectangle, so rotation, skew,
/// non-uniform scale and translation all sample the original image consistently.
/// Returns true when at least one destination pixel center is inside the transformed image.
pub fn draw_texture_transformed_clipped(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    transform: Transform2D,
    filter: TextureFilter,
    clip: Rect,
) -> bool {
    if destination.width == 0 || destination.height == 0 || !transform.is_finite() {
        return false;
    }
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return false;
    };

    // Identity is the common case for display-list texture commands. Route it
    // through the specialized scanline renderer before attempting matrix
    // inversion, corner transforms, or per-pixel inverse mapping.
    if transform == Transform2D::IDENTITY {
        let Some(visible) = destination.intersection(clip) else {
            return false;
        };
        if visible.width == 0 || visible.height == 0 {
            return false;
        }
        draw_texture_clipped(target, texture, destination, filter, clip);
        return true;
    }

    let Some(inverse) = transform.inverse() else {
        return false;
    };

    let left = destination.x as f32;
    let top = destination.y as f32;
    let right = destination.x.saturating_add(destination.width) as f32;
    let bottom = destination.y.saturating_add(destination.height) as f32;
    let corners = [
        transform.transform_point(crate::Vector::new(left, top)),
        transform.transform_point(crate::Vector::new(right, top)),
        transform.transform_point(crate::Vector::new(right, bottom)),
        transform.transform_point(crate::Vector::new(left, bottom)),
    ];
    if corners
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return false;
    }
    let min_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let min_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
    let first_x = min_x.floor().max(clip.x as f32).min(clip_right as f32) as u32;
    let first_y = min_y.floor().max(clip.y as f32).min(clip_bottom as f32) as u32;
    let end_x = max_x.ceil().max(clip.x as f32).min(clip_right as f32) as u32;
    let end_y = max_y.ceil().max(clip.y as f32).min(clip_bottom as f32) as u32;
    if first_x >= end_x || first_y >= end_y {
        return false;
    }

    let local_left = destination.x as f32;
    let local_top = destination.y as f32;
    let local_right = local_left + destination.width as f32;
    let local_bottom = local_top + destination.height as f32;
    let mut visible = false;
    for y in first_y..end_y {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in first_x..end_x {
            let local = inverse.transform_point(crate::Vector::new(x as f32 + 0.5, y as f32 + 0.5));
            if local.x < local_left
                || local.x >= local_right
                || local.y < local_top
                || local.y >= local_bottom
            {
                continue;
            }
            visible = true;
            let source_x = (local.x - local_left) * texture.width as f32 / destination.width as f32;
            let source_y =
                (local.y - local_top) * texture.height as f32 / destination.height as f32;
            let color = match filter {
                TextureFilter::Nearest => texture.sample_nearest(source_x, source_y),
                TextureFilter::Bilinear => texture.sample_bilinear(source_x - 0.5, source_y - 0.5),
            };
            if texture.fully_opaque {
                // Every nearest or bilinear sample is guaranteed opaque, so
                // transformed draws can replace the destination directly.
                row[x as usize] = color;
            } else if color.a == 255 {
                row[x as usize] = color;
            } else if color.a > 0 {
                let index = x as usize;
                row[index] = color.blend_over(row[index]);
            }
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::{
        blend_cached_bilinear_over_opaque, draw_texture, draw_texture_clipped,
        draw_texture_transformed_clipped, sample_bilinear_prepared, BilinearCoordinates, Texture,
        TextureError, TextureFilter,
    };
    use crate::{Color, Framebuffer, Rect};

    fn two_color_texture() -> Texture {
        Texture::new(2, 1, vec![Color::RED, Color::BLUE]).unwrap()
    }

    #[test]
    fn cached_bilinear_opaque_composition_matches_scalar_reference() {
        let cases = [
            (
                [12.5, 2400.0, 1800.0, 220.0],
                [12500.0, 3200.0, 9000.0, 180.0],
                1.0,
                0.0,
                Color::rgb(10, 80, 190),
            ),
            (
                [3200.0, 18000.0, 900.0, 90.0],
                [21000.0, 4000.0, 15000.0, 210.0],
                0.375,
                0.625,
                Color::rgb(250, 30, 70),
            ),
            (
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                0.5,
                0.5,
                Color::WHITE,
            ),
        ];

        for (top, bottom, inverse_y, ty, initial) in cases {
            let alpha = top[3] * inverse_y + bottom[3] * ty;
            let mut expected = initial;
            if alpha > f32::EPSILON {
                let inverse_alpha = 255.0 - alpha;
                let channel = |index: usize, background: u8| {
                    ((top[index] * inverse_y
                        + bottom[index] * ty
                        + f32::from(background) * inverse_alpha)
                        / 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8
                };
                expected.r = channel(0, expected.r);
                expected.g = channel(1, expected.g);
                expected.b = channel(2, expected.b);
            }

            let mut actual = initial;
            blend_cached_bilinear_over_opaque(
                &top,
                &bottom,
                inverse_y,
                ty,
                &mut actual,
            );
            assert_eq!(actual.r, expected.r);
            assert_eq!(actual.g, expected.g);
            assert_eq!(actual.b, expected.b);
            assert_eq!(actual.a, 255);
        }
    }

    #[test]
    fn premultiplied_composition_over_opaque_stays_within_one_channel_value() {
        // The fused floating-point path can differ by at most one 8-bit level
        // from the established unpremultiply-then-source-over rounding path.
        for alpha in [0.5_f32, 1.0, 32.5, 127.5, 180.0, 220.0, 254.5] {
            for premultiplied in [0.0_f32, 1.0, 32.0, 127.0, 255.0, 1024.0, 255.0 * alpha] {
                if premultiplied > 255.0 * alpha {
                    continue;
                }
                for destination in [0_u8, 1, 64, 127, 200, 254, 255] {
                    let source = (premultiplied / alpha)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    let source_alpha = alpha.round().clamp(0.0, 255.0) as u8;
                    let expected = Color::rgba(source, source, source, source_alpha)
                        .blend_over(Color::rgb(destination, destination, destination));
                    let actual = ((premultiplied
                        + f32::from(destination) * (255.0 - alpha))
                        / 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    assert!(actual.abs_diff(expected.r) <= 1);
                }
            }
        }
    }

    #[test]
    fn texture_validates_dimensions_and_pixel_count() {
        assert_eq!(Texture::new(0, 1, vec![]), Err(TextureError::ZeroDimension));
        assert_eq!(
            Texture::new(2, 2, vec![Color::BLACK]),
            Err(TextureError::PixelCountMismatch)
        );
    }

    #[test]
    fn nearest_filter_scales_source_texels() {
        let texture = two_color_texture();
        let mut target = Framebuffer::new(4, 1, Color::BLACK).unwrap();
        draw_texture(
            &mut target,
            &texture,
            Rect::new(0, 0, 4, 1),
            TextureFilter::Nearest,
        );
        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(1, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 0), Some(Color::BLUE));
        assert_eq!(target.get_pixel(3, 0), Some(Color::BLUE));
    }

    #[test]
    fn opaque_bilinear_texture_replaces_destination_in_cached_and_uncached_paths() {
        let texture = Texture::new(
            3,
            2,
            vec![
                Color::rgb(255, 0, 0),
                Color::rgb(0, 255, 0),
                Color::rgb(0, 0, 255),
                Color::rgb(255, 255, 0),
                Color::rgb(0, 255, 255),
                Color::rgb(255, 0, 255),
            ],
        )
        .unwrap();

        for (width, height) in [(7, 3), (7, 5)] {
            let mut target =
                Framebuffer::new(width, height, Color::rgba(20, 40, 60, 120)).unwrap();
            draw_texture(
                &mut target,
                &texture,
                Rect::new(0, 0, width, height),
                TextureFilter::Bilinear,
            );

            let expected = (0..height)
                .flat_map(|y| {
                    let source_y =
                        ((y as f32 + 0.5) * texture.height() as f32 / height as f32 - 0.5)
                            .clamp(0.0, texture.height() as f32 - 1.0);
                    let y0 = source_y.floor() as usize;
                    let y1 = (y0 + 1).min(texture.height() as usize - 1);
                    let ty = source_y - y0 as f32;
                    (0..width).map(move |x| {
                        let source_x =
                            ((x as f32 + 0.5) * texture.width() as f32 / width as f32 - 0.5)
                                .clamp(0.0, texture.width() as f32 - 1.0);
                        let x0 = source_x.floor() as usize;
                        let x1 = (x0 + 1).min(texture.width() as usize - 1);
                        sample_bilinear_prepared(
                            texture.pixels(),
                            texture.width() as usize,
                            BilinearCoordinates {
                                x0,
                                x1,
                                y0,
                                y1,
                                tx: source_x - x0 as f32,
                                ty,
                            },
                        )
                    })
                })
                .collect::<Vec<_>>();

            assert_eq!(target.pixels(), expected);
            assert!(target.pixels().iter().all(|pixel| pixel.a == 255));
        }
    }

    #[test]
    fn bilinear_filter_interpolates_between_texels() {
        let texture = two_color_texture();
        let mut target = Framebuffer::new(3, 1, Color::BLACK).unwrap();
        draw_texture(
            &mut target,
            &texture,
            Rect::new(0, 0, 3, 1),
            TextureFilter::Bilinear,
        );
        assert_eq!(target.get_pixel(1, 0), Some(Color::rgb(128, 0, 128)));
    }

    #[test]
    fn texture_alpha_is_composited_over_destination() {
        let texture = Texture::new(1, 1, vec![Color::rgba(255, 0, 0, 128)]).unwrap();
        let mut target = Framebuffer::new(1, 1, Color::BLUE).unwrap();
        draw_texture(
            &mut target,
            &texture,
            Rect::new(0, 0, 1, 1),
            TextureFilter::Nearest,
        );
        assert_eq!(target.get_pixel(0, 0), Some(Color::rgb(128, 0, 127)));
    }

    #[test]
    fn drawing_is_clipped_to_target_and_zero_area_is_ignored() {
        let texture = Texture::new(1, 1, vec![Color::WHITE]).unwrap();
        let mut target = Framebuffer::new(2, 2, Color::BLACK).unwrap();
        draw_texture(
            &mut target,
            &texture,
            Rect::new(1, 1, u32::MAX, u32::MAX),
            TextureFilter::Nearest,
        );
        draw_texture(
            &mut target,
            &texture,
            Rect::new(0, 0, 0, 2),
            TextureFilter::Bilinear,
        );
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(target.get_pixel(1, 1), Some(Color::WHITE));
    }

    #[test]
    fn texture_blending_skips_transparent_texels_and_replaces_opaque_ones() {
        let texture = Texture::new(
            3,
            1,
            vec![
                Color::TRANSPARENT,
                Color::rgba(255, 0, 0, 128),
                Color::GREEN,
            ],
        )
        .unwrap();
        let mut target = Framebuffer::new(3, 1, Color::BLUE).unwrap();
        draw_texture(
            &mut target,
            &texture,
            Rect::new(0, 0, 3, 1),
            TextureFilter::Nearest,
        );
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLUE));
        assert_eq!(target.get_pixel(1, 0), Some(Color::rgb(128, 0, 127)));
        assert_eq!(target.get_pixel(2, 0), Some(Color::GREEN));
    }

    #[test]
    fn clipped_texture_preserves_original_sampling_and_viewport() {
        let texture = two_color_texture();
        let mut target = Framebuffer::new(6, 2, Color::BLACK).unwrap();
        draw_texture_clipped(
            &mut target,
            &texture,
            Rect::new(1, 0, 4, 1),
            TextureFilter::Nearest,
            Rect::new(2, 0, 2, 1),
        );
        assert_eq!(target.get_pixel(1, 0), Some(Color::BLACK));
        assert_eq!(target.get_pixel(2, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(3, 0), Some(Color::BLUE));
        assert_eq!(target.get_pixel(4, 0), Some(Color::BLACK));
    }

    #[test]
    fn untransformed_fast_path_matches_identity_transform_with_clipping() {
        let texture = Texture::new(
            3,
            2,
            vec![
                Color::rgba(240, 30, 20, 255),
                Color::rgba(20, 220, 80, 180),
                Color::TRANSPARENT,
                Color::rgba(30, 60, 240, 90),
                Color::WHITE,
                Color::rgba(240, 180, 20, 255),
            ],
        )
        .unwrap();
        let destination = Rect::new(1, 1, 5, 3);
        let clip = Rect::new(2, 0, 3, 4);

        for filter in [TextureFilter::Nearest, TextureFilter::Bilinear] {
            let mut fast = Framebuffer::new(7, 5, Color::rgb(12, 24, 36)).unwrap();
            let mut transformed = fast.clone();
            draw_texture_clipped(&mut fast, &texture, destination, filter, clip);
            draw_texture_transformed_clipped(
                &mut transformed,
                &texture,
                destination,
                crate::Transform2D::IDENTITY,
                filter,
                clip,
            );
            for (index, (fast_pixel, transformed_pixel)) in
                fast.pixels().iter().zip(transformed.pixels()).enumerate()
            {
                let channel_delta = |left: u8, right: u8| left.abs_diff(right);
                assert!(
                    channel_delta(fast_pixel.r, transformed_pixel.r) <= 1
                        && channel_delta(fast_pixel.g, transformed_pixel.g) <= 1
                        && channel_delta(fast_pixel.b, transformed_pixel.b) <= 1
                        && fast_pixel.a == transformed_pixel.a,
                    "filter: {filter:?}, pixel {index}: fast={fast_pixel:?}, transformed={transformed_pixel:?}"
                );
            }
        }
    }

    #[test]
    fn identity_texture_transform_uses_the_untransformed_sampling_result() {
        let texture = Texture::new(
            3,
            2,
            vec![
                Color::rgba(240, 30, 20, 255),
                Color::rgba(20, 220, 80, 180),
                Color::TRANSPARENT,
                Color::rgba(30, 60, 240, 90),
                Color::WHITE,
                Color::rgba(240, 180, 20, 255),
            ],
        )
        .unwrap();
        let destination = Rect::new(1, 1, 5, 3);
        let clip = Rect::new(2, 0, 3, 4);

        for filter in [TextureFilter::Nearest, TextureFilter::Bilinear] {
            let mut expected = Framebuffer::new(7, 5, Color::rgb(12, 24, 36)).unwrap();
            let mut actual = expected.clone();
            draw_texture_clipped(&mut expected, &texture, destination, filter, clip);
            assert!(draw_texture_transformed_clipped(
                &mut actual,
                &texture,
                destination,
                crate::Transform2D::IDENTITY,
                filter,
                clip,
            ));
            assert_eq!(actual.pixels(), expected.pixels());
        }
    }

    #[test]
    fn affine_texture_transform_supports_rotation_and_sampling() {
        let texture = two_color_texture();
        let transform = crate::Transform2D::rotation(std::f32::consts::FRAC_PI_2)
            .then(crate::Transform2D::translation(3.0, 1.0));
        let mut target = Framebuffer::new(6, 5, Color::BLACK).unwrap();
        draw_texture_transformed_clipped(
            &mut target,
            &texture,
            Rect::new(0, 0, 2, 1),
            transform,
            TextureFilter::Nearest,
            Rect::new(0, 0, 6, 5),
        );
        assert_eq!(target.get_pixel(2, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 2), Some(Color::BLUE));
        assert_eq!(target.get_pixel(3, 1), Some(Color::BLACK));
        assert_eq!(target.get_pixel(2, 3), Some(Color::BLACK));
    }

    #[test]
    fn transformed_opaque_bilinear_texture_replaces_destination() {
        let texture = Texture::new(2, 1, vec![Color::RED, Color::BLUE]).unwrap();
        let mut target = Framebuffer::new(4, 1, Color::GREEN).unwrap();

        assert!(draw_texture_transformed_clipped(
            &mut target,
            &texture,
            Rect::new(0, 0, 2, 1),
            Transform2D::scale(2.0, 1.0),
            TextureFilter::Bilinear,
            Rect::new(0, 0, 4, 1),
        ));

        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(1, 0), Some(Color::rgb(191, 0, 64)));
        assert_eq!(target.get_pixel(2, 0), Some(Color::rgb(64, 0, 191)));
        assert_eq!(target.get_pixel(3, 0), Some(Color::BLUE));
    }

    #[test]
    fn singular_texture_transform_is_a_noop() {
        let texture = two_color_texture();
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        draw_texture_transformed_clipped(
            &mut target,
            &texture,
            Rect::new(0, 0, 2, 1),
            crate::Transform2D::scale(0.0, 1.0),
            TextureFilter::Nearest,
            Rect::new(0, 0, 4, 4),
        );
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn empty_texture_clip_is_noop() {
        let texture = two_color_texture();
        let mut target = Framebuffer::new(3, 2, Color::BLACK).unwrap();
        draw_texture_clipped(
            &mut target,
            &texture,
            Rect::new(0, 0, 3, 2),
            TextureFilter::Bilinear,
            Rect::new(8, 8, 2, 2),
        );
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn texture_tracks_whether_all_pixels_are_opaque() {
        let opaque = Texture::new(2, 1, vec![Color::RED, Color::BLUE]).unwrap();
        let translucent =
            Texture::new(2, 1, vec![Color::RED, Color::rgba(0, 0, 255, 128)]).unwrap();

        assert!(opaque.is_fully_opaque());
        assert!(!translucent.is_fully_opaque());
    }

    #[test]
    fn framebuffer_can_become_an_immutable_texture() {
        let mut framebuffer = Framebuffer::new(1, 1, Color::GREEN).unwrap();
        let texture = Texture::from_framebuffer(&framebuffer);
        framebuffer.clear(Color::RED);
        assert_eq!(texture.get_pixel(0, 0), Some(Color::GREEN));
    }

    #[test]
    fn owned_framebuffer_conversion_reuses_pixel_allocation() {
        let framebuffer = Framebuffer::new(2, 2, Color::GREEN).unwrap();
        let original_storage = framebuffer.pixels().as_ptr();
        let texture = Texture::from_framebuffer_owned(framebuffer);

        assert_eq!(texture.pixels().as_ptr(), original_storage);
        assert_eq!(texture.width(), 2);
        assert_eq!(texture.height(), 2);
        assert!(texture.pixels().iter().all(|pixel| *pixel == Color::GREEN));
    }
}
