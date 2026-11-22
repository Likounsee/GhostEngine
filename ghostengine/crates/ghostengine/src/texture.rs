//! CPU-side image storage and scaled texture compositing.

use crate::{simd::blend_variable_opaque_quad, Color, Framebuffer, Rect, Transform2D};
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEXTURE_ID: AtomicU64 = AtomicU64::new(1);

#[inline]
fn next_texture_id() -> u64 {
    NEXT_TEXTURE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Errors returned when constructing an in-memory texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureError {
    ZeroDimension,
    SizeOverflow,
    PixelCountMismatch,
    AllocationFailed,
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
#[derive(Debug, Clone)]
pub struct Texture {
    id: u64,
    width: u32,
    height: u32,
    pixels: Vec<Color>,
    fully_opaque: bool,
}

impl PartialEq for Texture {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.pixels == other.pixels
            && self.fully_opaque == other.fully_opaque
    }
}

impl Eq for Texture {}

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
            id: next_texture_id(),
            width,
            height,
            pixels,
            fully_opaque,
        })
    }

    /// Creates a texture by copying a borrowed row-major pixel slice.
    ///
    /// This method reports allocation failure through TextureError.
    pub fn try_from_pixels(
        width: u32,
        height: u32,
        source: &[Color],
    ) -> Result<Self, TextureError> {
        if width == 0 || height == 0 {
            return Err(TextureError::ZeroDimension);
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .ok_or(TextureError::SizeOverflow)?;
        if source.len() != expected {
            return Err(TextureError::PixelCountMismatch);
        }

        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(expected)
            .map_err(|_| TextureError::AllocationFailed)?;
        pixels.extend_from_slice(source);
        let fully_opaque = pixels.iter().all(|pixel| pixel.a == 255);
        Ok(Self {
            id: next_texture_id(),
            width,
            height,
            pixels,
            fully_opaque,
        })
    }

    /// Creates an independent texture copy while reporting allocation failure.
    pub fn try_clone(&self) -> Result<Self, TextureError> {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(self.pixels.len())
            .map_err(|_| TextureError::AllocationFailed)?;
        pixels.extend_from_slice(&self.pixels);
        Ok(Self {
            id: next_texture_id(),
            width: self.width,
            height: self.height,
            pixels,
            fully_opaque: self.fully_opaque,
        })
    }

    /// Copies framebuffer pixels into a texture while reporting allocation failure.
    pub fn try_from_framebuffer(framebuffer: &Framebuffer) -> Result<Self, TextureError> {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(framebuffer.pixels().len())
            .map_err(|_| TextureError::AllocationFailed)?;
        pixels.extend_from_slice(framebuffer.pixels());
        let fully_opaque = pixels.iter().all(|pixel| pixel.a == 255);
        Ok(Self {
            id: next_texture_id(),
            width: framebuffer.width(),
            height: framebuffer.height(),
            pixels,
            fully_opaque,
        })
    }

    /// Copies the pixels from an existing framebuffer.
    #[must_use]
    pub fn from_framebuffer(framebuffer: &Framebuffer) -> Self {
        Self {
            id: next_texture_id(),
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
            id: next_texture_id(),
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

    #[cfg(test)]
    fn sample_nearest(&self, x: f32, y: f32) -> Color {
        sample_pixels_nearest(&self.pixels, self.width, self.height, x, y)
    }

    #[cfg(test)]
    fn sample_bilinear(&self, x: f32, y: f32) -> Color {
        sample_pixels_bilinear(&self.pixels, self.width, self.height, x, y)
    }
}

#[cfg(test)]
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

/// Bilinear sampling for images known to contain only opaque texels.
pub(crate) fn sample_pixels_bilinear_opaque(
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
    sample_bilinear_opaque_prepared(
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
struct BilinearHorizontalSample {
    x0: usize,
    x1: usize,
    tx: f32,
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
        channel(interpolate_premultiplied(
            top_left.r,
            top_right.r,
            bottom_left.r,
            bottom_right.r,
        )),
        channel(interpolate_premultiplied(
            top_left.g,
            top_right.g,
            bottom_left.g,
            bottom_right.g,
        )),
        channel(interpolate_premultiplied(
            top_left.b,
            top_right.b,
            bottom_left.b,
            bottom_right.b,
        )),
        channel(alpha),
    )
}

#[inline]
fn sample_bilinear_opaque_rows(
    top_row: &[Color],
    bottom_row: &[Color],
    sample: BilinearHorizontalSample,
    ty: f32,
) -> Color {
    let top_left = top_row[sample.x0];
    let top_right = top_row[sample.x1];
    let bottom_left = bottom_row[sample.x0];
    let bottom_right = bottom_row[sample.x1];

    let inverse_x = 1.0 - sample.tx;
    let inverse_y = 1.0 - ty;
    let interpolate = |top_left: u8, top_right: u8, bottom_left: u8, bottom_right: u8| {
        let top = f32::from(top_left) * inverse_x + f32::from(top_right) * sample.tx;
        let bottom = f32::from(bottom_left) * inverse_x + f32::from(bottom_right) * sample.tx;
        (top * inverse_y + bottom * ty).round().clamp(0.0, 255.0) as u8
    };

    Color::rgb(
        interpolate(top_left.r, top_right.r, bottom_left.r, bottom_right.r),
        interpolate(top_left.g, top_right.g, bottom_left.g, bottom_right.g),
        interpolate(top_left.b, top_right.b, bottom_left.b, bottom_right.b),
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
        (top * inverse_y + bottom * ty).round().clamp(0.0, 255.0) as u8
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
    let red = (top[0] * inverse_y + bottom[0] * ty + f32::from(destination.r) * inverse_alpha)
        * (1.0 / 255.0);
    let green = (top[1] * inverse_y + bottom[1] * ty + f32::from(destination.g) * inverse_alpha)
        * (1.0 / 255.0);
    let blue = (top[2] * inverse_y + bottom[2] * ty + f32::from(destination.b) * inverse_alpha)
        * (1.0 / 255.0);
    destination.r = red.round().clamp(0.0, 255.0) as u8;
    destination.g = green.round().clamp(0.0, 255.0) as u8;
    destination.b = blue.round().clamp(0.0, 255.0) as u8;
}

// Parallel scanlines cannot share the sequential renderer's persistent
// horizontal cache. Build the two premultiplied horizontal samples in-place,
// then composite directly over opaque destinations. This avoids converting
// the interpolated sample back to straight-alpha bytes only to blend it again.
#[inline]
fn blend_bilinear_rows_over_opaque(
    top_row: &[Color],
    bottom_row: &[Color],
    sample: BilinearHorizontalSample,
    ty: f32,
    destination: &mut Color,
) {
    let top_left = top_row[sample.x0];
    let top_right = top_row[sample.x1];
    let bottom_left = bottom_row[sample.x0];
    let bottom_right = bottom_row[sample.x1];

    let inverse_x = 1.0 - sample.tx;
    let weighted_top_left_alpha = f32::from(top_left.a) * inverse_x;
    let weighted_top_right_alpha = f32::from(top_right.a) * sample.tx;
    let weighted_bottom_left_alpha = f32::from(bottom_left.a) * inverse_x;
    let weighted_bottom_right_alpha = f32::from(bottom_right.a) * sample.tx;
    let top = [
        f32::from(top_left.r) * weighted_top_left_alpha
            + f32::from(top_right.r) * weighted_top_right_alpha,
        f32::from(top_left.g) * weighted_top_left_alpha
            + f32::from(top_right.g) * weighted_top_right_alpha,
        f32::from(top_left.b) * weighted_top_left_alpha
            + f32::from(top_right.b) * weighted_top_right_alpha,
        weighted_top_left_alpha + weighted_top_right_alpha,
    ];
    let bottom = [
        f32::from(bottom_left.r) * weighted_bottom_left_alpha
            + f32::from(bottom_right.r) * weighted_bottom_right_alpha,
        f32::from(bottom_left.g) * weighted_bottom_left_alpha
            + f32::from(bottom_right.g) * weighted_bottom_right_alpha,
        f32::from(bottom_left.b) * weighted_bottom_left_alpha
            + f32::from(bottom_right.b) * weighted_bottom_right_alpha,
        weighted_bottom_left_alpha + weighted_bottom_right_alpha,
    ];
    let inverse_y = 1.0 - ty;
    blend_cached_bilinear_over_opaque(&top, &bottom, inverse_y, ty, destination);
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
    draw_texture_clipped_with_scratch(
        target,
        texture,
        destination,
        filter,
        clip,
        &mut TextureDrawScratch::default(),
    );
}

/// Draws a texture using an automatic CPU strategy.
///
/// Large destination regions use the parallel row-band compositor; smaller
/// regions stay on the sequential path to avoid thread scheduling overhead.
pub fn draw_texture_auto(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
) {
    let clip = Rect::new(0, 0, target.width(), target.height());
    draw_texture_clipped_auto(target, texture, destination, filter, clip);
}

/// Draws a clipped texture using the automatic CPU strategy.
pub fn draw_texture_clipped_auto(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
) {
    let threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    draw_texture_clipped_parallel_with_threads(target, texture, destination, filter, clip, threads);
}

/// Draws a rectangular region of a texture or atlas into a destination rectangle.
///
/// The source region is validated against the texture bounds. Bilinear sampling
/// is clamped to the selected region so neighboring atlas entries cannot bleed
/// into the sprite.
pub fn draw_texture_region_clipped(
    target: &mut Framebuffer,
    texture: &Texture,
    source: Rect,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
) {
    draw_texture_region_clipped_with_scratch(
        target,
        texture,
        source,
        destination,
        filter,
        clip,
        &mut TextureDrawScratch::default(),
    );
}

/// Draws a texture region while reusing horizontal bilinear samples across scanlines.
///
/// Keep the scratch value between frames to avoid repeating allocations for atlas sprites.
pub fn draw_texture_region_clipped_with_scratch(
    target: &mut Framebuffer,
    texture: &Texture,
    source: Rect,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
    scratch: &mut TextureDrawScratch,
) {
    if source.width == 0
        || source.height == 0
        || destination.width == 0
        || destination.height == 0
        || u64::from(destination.x) + u64::from(destination.width) > u64::from(u32::MAX)
        || u64::from(destination.y) + u64::from(destination.height) > u64::from(u32::MAX)
        || u64::from(source.x) + u64::from(source.width) > u64::from(texture.width())
        || u64::from(source.y) + u64::from(source.height) > u64::from(texture.height())
    {
        return;
    }
    let Some(clip) = clip
        .intersection(Rect::new(0, 0, target.width(), target.height()))
        .and_then(|framebuffer_clip| framebuffer_clip.intersection(destination))
    else {
        return;
    };
    let source_right = source.x + source.width - 1;
    let source_bottom = source.y + source.height - 1;
    let target_width = target.width() as usize;
    let texture_width = texture.width() as usize;
    let source_pixels = texture.pixels();
    let target_pixels = target.pixels_mut();

    match filter {
        TextureFilter::Nearest => {
            // Use an integer error accumulator for horizontal nearest-neighbor
            // coordinates. It is equivalent to floor(offset * source / dest),
            // but removes a floating-point division from every destination pixel.
            let destination_width = u64::from(destination.width);
            let source_width = u64::from(source.width);
            let first_offset = u64::from(clip.x - destination.x);
            let first_scaled = first_offset * source_width;
            let source_offset = first_scaled / destination_width;
            let error = first_scaled % destination_width;
            let step_whole = source_width / destination_width;
            let step_remainder = source_width % destination_width;

            for y in clip.y..clip.y + clip.height {
                let relative_y = (y - destination.y) as f32 / destination.height as f32;
                let sy = (source.y + (relative_y * source.height as f32).floor() as u32)
                    .min(source_bottom) as usize;
                let target_row = y as usize * target_width;
                let source_row = sy * texture_width;
                let mut sx_offset = source_offset;
                let mut x_error = error;

                for x in clip.x..clip.x + clip.width {
                    let sx =
                        (u64::from(source.x) + sx_offset).min(u64::from(source_right)) as usize;
                    let sampled = source_pixels[source_row + sx];
                    if sampled.a == 255 {
                        target_pixels[target_row + x as usize] = sampled;
                    } else if sampled.a != 0 {
                        let pixel = &mut target_pixels[target_row + x as usize];
                        *pixel = sampled.blend_over(*pixel);
                    }

                    sx_offset += step_whole;
                    x_error += step_remainder;
                    if x_error >= destination_width {
                        sx_offset += 1;
                        x_error -= destination_width;
                    }
                }
            }
        }
        TextureFilter::Bilinear => {
            let clip_area = u64::from(clip.width) * u64::from(clip.height);
            let source_area = u64::from(source.width) * u64::from(source.height);
            // Atlas sprites are often opaque even when the atlas allocation has
            // transparent unused space. Detect that once, then use the cheaper
            // opaque interpolation path instead of alpha-premultiplied sampling
            // and source-over branching for every output pixel.
            let source_region_is_opaque = texture.is_fully_opaque()
                || (source_area <= clip_area
                    && (source.y..source.y + source.height).all(|row| {
                        let start = row as usize * texture_width + source.x as usize;
                        source_pixels[start..start + source.width as usize]
                            .iter()
                            .all(|pixel| pixel.a == 255)
                    }));

            let horizontal_step = source.width as f32 / destination.width as f32;
            let first_relative_x =
                ((clip.x - destination.x) as f32 + 0.5) / destination.width as f32;
            let first_sx = source.x as f32 + first_relative_x * source.width as f32 - 0.5;

            // Horizontal source coordinates are identical on every scanline.
            // Resolve them once instead of repeating float mapping, clamping and
            // floor conversion for every row of an atlas sprite.
            scratch.horizontal_samples.clear();
            let visible_width = clip.width as usize;
            if scratch.horizontal_samples.capacity() < visible_width {
                scratch
                    .horizontal_samples
                    .reserve(visible_width - scratch.horizontal_samples.len());
            }
            let mut raw_sx = first_sx;
            for _ in 0..clip.width {
                let sx = raw_sx.clamp(source.x as f32, source_right as f32);
                let x0 = sx.floor() as usize;
                scratch.horizontal_samples.push((
                    x0,
                    (x0 + 1).min(source_right as usize),
                    sx - x0 as f32,
                ));
                raw_sx += horizontal_step;
            }

            let cache_len = (source.height as usize).checked_mul(clip.width as usize);
            let cache_bytes =
                cache_len.and_then(|len| len.checked_mul(core::mem::size_of::<[f32; 4]>()));
            let use_region_cache = clip.height >= source.height.saturating_mul(2)
                && cache_bytes.is_some_and(|bytes| bytes <= 64 * 1024 * 1024);
            if use_region_cache {
                let required = cache_len.expect("cache length validated with byte size");
                let cache_key = RegionBilinearCacheKey {
                    texture_id: texture.id,
                    source,
                    destination_x: destination.x,
                    destination_width: destination.width,
                    clip_x: clip.x,
                    clip_width: clip.width,
                };
                let cache_is_valid = scratch.cached_region_bilinear_key == Some(cache_key)
                    && scratch.region_horizontal_cache.len() == required;
                if !cache_is_valid {
                    scratch.region_horizontal_cache.clear();
                    if scratch.region_horizontal_cache.capacity() < required {
                        scratch
                            .region_horizontal_cache
                            .reserve(required - scratch.region_horizontal_cache.len());
                    }
                    for source_y in source.y..=source_bottom {
                        let row_offset = source_y as usize * texture_width;
                        for &(x0, x1, tx) in &scratch.horizontal_samples {
                            let left = source_pixels[row_offset + x0];
                            let right = source_pixels[row_offset + x1];
                            let inverse_x = 1.0 - tx;
                            let left_alpha = f32::from(left.a) * inverse_x;
                            let right_alpha = f32::from(right.a) * tx;
                            scratch.region_horizontal_cache.push([
                                f32::from(left.r) * left_alpha + f32::from(right.r) * right_alpha,
                                f32::from(left.g) * left_alpha + f32::from(right.g) * right_alpha,
                                f32::from(left.b) * left_alpha + f32::from(right.b) * right_alpha,
                                left_alpha + right_alpha,
                            ]);
                        }
                    }
                    scratch.cached_region_bilinear_key = Some(cache_key);
                }
            } else {
                scratch.region_horizontal_cache.clear();
                scratch.cached_region_bilinear_key = None;
            }
            for y in clip.y..clip.y + clip.height {
                let relative_y = ((y - destination.y) as f32 + 0.5) / destination.height as f32;
                let sy = (source.y as f32 + relative_y * source.height as f32 - 0.5)
                    .clamp(source.y as f32, source_bottom as f32);
                let y0 = sy.floor() as usize;
                let y1 = (y0 + 1).min(source_bottom as usize);
                let ty = sy - y0 as f32;
                let target_row = y as usize * target_width;
                if use_region_cache {
                    let top_start = (y0 - source.y as usize) * clip.width as usize;
                    let bottom_start = (y1 - source.y as usize) * clip.width as usize;
                    let top_samples = &scratch.region_horizontal_cache
                        [top_start..top_start + clip.width as usize];
                    let bottom_samples = &scratch.region_horizontal_cache
                        [bottom_start..bottom_start + clip.width as usize];
                    for (column, ((top, bottom), &(x0, x1, tx))) in top_samples
                        .iter()
                        .zip(bottom_samples)
                        .zip(&scratch.horizontal_samples)
                        .enumerate()
                    {
                        let sample = BilinearHorizontalSample { x0, x1, tx };
                        let pixel = &mut target_pixels[target_row + clip.x as usize + column];
                        if pixel.a == 255 {
                            blend_cached_bilinear_over_opaque(top, bottom, 1.0 - ty, ty, pixel);
                        } else {
                            let sampled = sample_bilinear_prepared(
                                source_pixels,
                                texture_width,
                                BilinearCoordinates {
                                    x0: sample.x0,
                                    x1: sample.x1,
                                    y0,
                                    y1,
                                    tx: sample.tx,
                                    ty,
                                },
                            );
                            if sampled.a == 255 {
                                *pixel = sampled;
                            } else if sampled.a != 0 {
                                *pixel = sampled.blend_over(*pixel);
                            }
                        }
                    }
                } else if source_region_is_opaque {
                    // Resolve the two source rows once per destination row.
                    // The previous per-pixel helper repeated both row-offset
                    // multiplications for every sample in the horizontal run.
                    let top_offset = y0 * texture_width;
                    let bottom_offset = y1 * texture_width;
                    let top_row = &source_pixels[top_offset..top_offset + texture_width];
                    let bottom_row = &source_pixels[bottom_offset..bottom_offset + texture_width];

                    for (column, &(x0, x1, tx)) in
                        scratch.horizontal_samples.iter().enumerate()
                    {
                        let sampled = sample_bilinear_opaque_rows(
                            top_row,
                            bottom_row,
                            BilinearHorizontalSample { x0, x1, tx },
                            ty,
                        );
                        target_pixels[target_row + clip.x as usize + column] = sampled;
                    }
                } else {
                    let top_offset = y0 * texture_width;
                    let bottom_offset = y1 * texture_width;
                    let top_row = &source_pixels[top_offset..top_offset + texture_width];
                    let bottom_row = &source_pixels[bottom_offset..bottom_offset + texture_width];

                    for (column, &(x0, x1, tx)) in
                        scratch.horizontal_samples.iter().enumerate()
                    {
                        let sample = BilinearHorizontalSample { x0, x1, tx };
                        let pixel = &mut target_pixels[target_row + clip.x as usize + column];

                        // Match the main texture renderer: when the destination is
                        // opaque, composite the premultiplied bilinear sample directly.
                        // This avoids converting to straight-alpha bytes and blending
                        // those bytes again for every atlas pixel.
                        if pixel.a == 255 {
                            blend_bilinear_rows_over_opaque(top_row, bottom_row, sample, ty, pixel);
                        } else {
                            let sampled = sample_bilinear_prepared(
                                source_pixels,
                                texture_width,
                                BilinearCoordinates {
                                    x0: sample.x0,
                                    x1: sample.x1,
                                    y0,
                                    y1,
                                    tx: sample.tx,
                                    ty,
                                },
                            );
                            if sampled.a == 255 {
                                *pixel = sampled;
                            } else if sampled.a != 0 {
                                *pixel = sampled.blend_over(*pixel);
                            }
                        }
                    }
                }
            }
        }
    }
}
/// Fixed-size border widths for nine-slice image scaling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NineSliceMargins {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl NineSliceMargins {
    #[must_use]
    pub const fn new(left: u32, top: u32, right: u32, bottom: u32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }
}

/// Draws a nine-slice image, preserving corner dimensions while stretching edges
/// and the center. If the destination is smaller than the combined borders, the
/// opposing destination borders are proportionally reduced to fit.
///
/// Invalid source rectangles or margins larger than the source are ignored.
pub fn draw_texture_nine_slice_clipped(
    target: &mut Framebuffer,
    texture: &Texture,
    source: Rect,
    destination: Rect,
    margins: NineSliceMargins,
    filter: TextureFilter,
    clip: Rect,
) {
    draw_texture_nine_slice_clipped_with_scratch(
        target,
        texture,
        source,
        destination,
        margins,
        filter,
        clip,
        &mut TextureDrawScratch::default(),
    );
}

/// Draws a nine-slice image while reusing texture sampling storage across all
/// nine patches. Keep the scratch value between frames to avoid up to nine
/// independent temporary allocations per scaled panel.
pub fn draw_texture_nine_slice_clipped_with_scratch(
    target: &mut Framebuffer,
    texture: &Texture,
    source: Rect,
    destination: Rect,
    margins: NineSliceMargins,
    filter: TextureFilter,
    clip: Rect,
    scratch: &mut TextureDrawScratch,
) {
    if source.width == 0
        || source.height == 0
        || destination.width == 0
        || destination.height == 0
        || u64::from(destination.x) + u64::from(destination.width) > u64::from(u32::MAX)
        || u64::from(destination.y) + u64::from(destination.height) > u64::from(u32::MAX)
        || u64::from(source.x) + u64::from(source.width) > u64::from(texture.width())
        || u64::from(source.y) + u64::from(source.height) > u64::from(texture.height())
        || u64::from(margins.left) + u64::from(margins.right) > u64::from(source.width)
        || u64::from(margins.top) + u64::from(margins.bottom) > u64::from(source.height)
    {
        return;
    }

    let fit_borders = |first: u32, second: u32, available: u32| {
        let total = u64::from(first) + u64::from(second);
        if total <= u64::from(available) {
            (first, second)
        } else {
            let fitted_first = (u64::from(available) * u64::from(first) / total) as u32;
            (fitted_first, available - fitted_first)
        }
    };
    let (left_dst, right_dst) = fit_borders(margins.left, margins.right, destination.width);
    let (top_dst, bottom_dst) = fit_borders(margins.top, margins.bottom, destination.height);

    let source_x = [
        source.x,
        source.x + margins.left,
        source.x + source.width - margins.right,
    ];
    let source_y = [
        source.y,
        source.y + margins.top,
        source.y + source.height - margins.bottom,
    ];
    let source_widths = [
        margins.left,
        source.width - margins.left - margins.right,
        margins.right,
    ];
    let source_heights = [
        margins.top,
        source.height - margins.top - margins.bottom,
        margins.bottom,
    ];
    let destination_x = [
        destination.x,
        destination.x + left_dst,
        destination.x + destination.width - right_dst,
    ];
    let destination_y = [
        destination.y,
        destination.y + top_dst,
        destination.y + destination.height - bottom_dst,
    ];
    let destination_widths = [
        left_dst,
        destination.width - left_dst - right_dst,
        right_dst,
    ];
    let destination_heights = [
        top_dst,
        destination.height - top_dst - bottom_dst,
        bottom_dst,
    ];

    for row in 0..3 {
        for column in 0..3 {
            if source_widths[column] == 0
                || source_heights[row] == 0
                || destination_widths[column] == 0
                || destination_heights[row] == 0
            {
                continue;
            }
            draw_texture_region_clipped_with_scratch(
                target,
                texture,
                Rect::new(
                    source_x[column],
                    source_y[row],
                    source_widths[column],
                    source_heights[row],
                ),
                Rect::new(
                    destination_x[column],
                    destination_y[row],
                    destination_widths[column],
                    destination_heights[row],
                ),
                filter,
                clip,
                scratch,
            );
        }
    }
}
/// Errors returned by fallible texture drawing operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureDrawError {
    /// Temporary sampling storage could not be allocated.
    AllocationFailed,
}

/// Reusable temporary storage for repeated texture draws.
#[derive(Debug, Default)]
pub struct TextureDrawScratch {
    nearest_columns: Vec<usize>,
    horizontal_samples: Vec<(usize, usize, f32)>,
    horizontal_cache: Vec<[f32; 4]>,
    region_horizontal_cache: Vec<[f32; 4]>,
    cached_bilinear_key: Option<BilinearCacheKey>,
    cached_region_bilinear_key: Option<RegionBilinearCacheKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BilinearCacheKey {
    texture_id: u64,
    texture_width: u32,
    texture_height: u32,
    destination_x: u32,
    destination_width: u32,
    first_x: u64,
    end_x: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegionBilinearCacheKey {
    texture_id: u64,
    source: Rect,
    destination_x: u32,
    destination_width: u32,
    clip_x: u32,
    clip_width: u32,
}

/// Draws a clipped texture while reporting temporary-buffer allocation failures.
///
/// The destination is left unchanged if the required scratch storage cannot be
/// reserved. Existing scratch allocations are reused whenever they are large enough.
pub fn try_draw_texture_clipped_with_scratch(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
    scratch: &mut TextureDrawScratch,
) -> Result<(), TextureDrawError> {
    if destination.width == 0 || destination.height == 0 {
        return Ok(());
    }
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };
    let first_x = u64::from(destination.x).max(u64::from(clip.x));
    let end_x = (u64::from(destination.x) + u64::from(destination.width))
        .min(u64::from(clip.x) + u64::from(clip.width));
    let first_y = u64::from(destination.y).max(u64::from(clip.y));
    let end_y = (u64::from(destination.y) + u64::from(destination.height))
        .min(u64::from(clip.y) + u64::from(clip.height));
    if first_x >= end_x || first_y >= end_y {
        return Ok(());
    }

    // The opaque 1:1 fast path performs row copies and needs no scratch storage.
    let direct_copy = filter == TextureFilter::Nearest
        && texture.fully_opaque
        && destination.width == texture.width
        && destination.height == texture.height;
    if !direct_copy {
        let visible_width = (end_x - first_x) as usize;
        match filter {
            TextureFilter::Nearest => {
                if scratch.nearest_columns.capacity() < visible_width {
                    scratch
                        .nearest_columns
                        .try_reserve_exact(
                            visible_width.saturating_sub(scratch.nearest_columns.len()),
                        )
                        .map_err(|_| TextureDrawError::AllocationFailed)?;
                }
            }
            TextureFilter::Bilinear => {
                if scratch.horizontal_samples.capacity() < visible_width {
                    scratch
                        .horizontal_samples
                        .try_reserve_exact(
                            visible_width.saturating_sub(scratch.horizontal_samples.len()),
                        )
                        .map_err(|_| TextureDrawError::AllocationFailed)?;
                }
                let cache_len = (texture.height as usize).checked_mul(visible_width);
                let cache_bytes =
                    cache_len.and_then(|len| len.checked_mul(core::mem::size_of::<[f32; 4]>()));
                let should_cache = end_y - first_y >= u64::from(texture.height) * 2
                    && cache_bytes.is_some_and(|bytes| bytes <= 64 * 1024 * 1024);
                let key = BilinearCacheKey {
                    texture_id: texture.id,
                    texture_width: texture.width,
                    texture_height: texture.height,
                    destination_x: destination.x,
                    destination_width: destination.width,
                    first_x,
                    end_x,
                };
                if should_cache && scratch.cached_bilinear_key != Some(key) {
                    let required = cache_len.expect("cache length checked with cache byte size");
                    if scratch.horizontal_cache.capacity() < required {
                        scratch
                            .horizontal_cache
                            .try_reserve_exact(
                                required.saturating_sub(scratch.horizontal_cache.len()),
                            )
                            .map_err(|_| TextureDrawError::AllocationFailed)?;
                    }
                }
            }
        }
    }

    draw_texture_clipped_with_scratch(target, texture, destination, filter, clip, scratch);
    Ok(())
}

/// Draws a texture while reusing temporary storage between calls.
pub fn draw_texture_clipped_with_scratch(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
    scratch: &mut TextureDrawScratch,
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

    // A 1:1 opaque nearest-neighbor draw is a pair of contiguous row
    // copies. Avoid building sampling coordinates and visiting pixels one by
    // one; the clipped offsets still refer to the original destination.
    if filter == TextureFilter::Nearest
        && texture.fully_opaque
        && destination.width == texture.width
        && destination.height == texture.height
    {
        let source_x = (first_x - u64::from(destination.x)) as usize;
        let copy_width = (end_x - first_x) as usize;
        for y in first_y..end_y {
            let source_y = (y - u64::from(destination.y)) as usize;
            let source_offset = source_y * texture.width as usize + source_x;
            let Some(row) = target.row_mut(y as u32) else {
                continue;
            };
            row[first_x as usize..end_x as usize]
                .copy_from_slice(&texture.pixels[source_offset..source_offset + copy_width]);
        }
        return;
    }

    let source_x_scale = texture.width as f32 / destination.width as f32;
    let source_y_scale = texture.height as f32 / destination.height as f32;
    // Nearest-neighbor horizontal coordinates are invariant across scanlines.
    // Prepare them once instead of repeating float mapping and floor conversion
    // for every destination pixel on every row.
    if filter == TextureFilter::Nearest {
        scratch.nearest_columns.clear();
        scratch.nearest_columns.reserve((end_x - first_x) as usize);
        scratch.nearest_columns.extend((first_x..end_x).map(|x| {
            let source_x = (x as f32 + 0.5 - destination.x as f32) * source_x_scale;
            (source_x.floor() as u32).min(texture.width - 1) as usize
        }));
    }
    if filter == TextureFilter::Bilinear {
        let row_width = texture.width as usize;
        scratch.horizontal_samples.clear();
        scratch.horizontal_samples.extend((first_x..end_x).map(|x| {
            let source_x = (x as f32 + 0.5 - destination.x as f32) * source_x_scale - 0.5;
            let source_x = source_x.clamp(0.0, texture.width as f32 - 1.0);
            let x0 = source_x.floor() as usize;
            (x0, (x0 + 1).min(row_width - 1), source_x - x0 as f32)
        }));
    }
    let use_horizontal_cache = if filter == TextureFilter::Bilinear {
        let cache_len = (texture.height as usize).checked_mul(scratch.horizontal_samples.len());
        let cache_bytes =
            cache_len.and_then(|len| len.checked_mul(core::mem::size_of::<[f32; 4]>()));
        let should_cache = end_y - first_y >= u64::from(texture.height) * 2
            && cache_bytes.is_some_and(|bytes| bytes <= 64 * 1024 * 1024);
        if should_cache {
            let key = BilinearCacheKey {
                texture_id: texture.id,
                texture_width: texture.width,
                texture_height: texture.height,
                destination_x: destination.x,
                destination_width: destination.width,
                first_x,
                end_x,
            };
            let cache_is_valid = scratch.cached_bilinear_key == Some(key);
            if !cache_is_valid {
                scratch.horizontal_cache.clear();
                scratch.horizontal_cache.reserve(cache_len.unwrap());
                let row_width = texture.width as usize;
                for source_row in 0..texture.height as usize {
                    let source_offset = source_row * row_width;
                    for &(x0, x1, tx) in &scratch.horizontal_samples {
                        let left = texture.pixels[source_offset + x0];
                        let right = texture.pixels[source_offset + x1];
                        let inverse_x = 1.0 - tx;
                        let left_alpha = f32::from(left.a) * inverse_x;
                        let right_alpha = f32::from(right.a) * tx;
                        scratch.horizontal_cache.push([
                            f32::from(left.r) * left_alpha + f32::from(right.r) * right_alpha,
                            f32::from(left.g) * left_alpha + f32::from(right.g) * right_alpha,
                            f32::from(left.b) * left_alpha + f32::from(right.b) * right_alpha,
                            left_alpha + right_alpha,
                        ]);
                    }
                }
                scratch.cached_bilinear_key = Some(key);
            }
        } else {
            scratch.cached_bilinear_key = None;
            scratch.horizontal_cache.clear();
        }
        should_cache
    } else {
        false
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
            if texture.fully_opaque && destination.width > texture.width {
                // During horizontal upscaling, several adjacent destination
                // pixels commonly select the same source texel. Fill each run
                // as a slice rather than repeating a source lookup per pixel.
                let visible_row = &mut row[first_x as usize..end_x as usize];
                let mut run_start = 0;
                while run_start < scratch.nearest_columns.len() {
                    let source_column = scratch.nearest_columns[run_start];
                    let mut run_end = run_start + 1;
                    while run_end < scratch.nearest_columns.len()
                        && scratch.nearest_columns[run_end] == source_column
                    {
                        run_end += 1;
                    }
                    visible_row[run_start..run_end]
                        .fill(texture.pixels[source_row + source_column]);
                    run_start = run_end;
                }
            } else if texture.fully_opaque {
                for (pixel, &source_column) in row[first_x as usize..end_x as usize]
                    .iter_mut()
                    .zip(&scratch.nearest_columns)
                {
                    *pixel = texture.pixels[source_row + source_column];
                }
            } else {
                let visible_row = &mut row[first_x as usize..end_x as usize];
                let vectorized_len = visible_row.len() / 4 * 4;
                let (vectorized_pixels, remainder_pixels) =
                    visible_row.split_at_mut(vectorized_len);
                let (vectorized_columns, remainder_columns) =
                    scratch.nearest_columns.split_at(vectorized_len);
                for (destinations, columns) in vectorized_pixels
                    .chunks_exact_mut(4)
                    .zip(vectorized_columns.chunks_exact(4))
                {
                    if destinations.iter().all(|pixel| pixel.a == 255) {
                        let sources = [
                            texture.pixels[source_row + columns[0]],
                            texture.pixels[source_row + columns[1]],
                            texture.pixels[source_row + columns[2]],
                            texture.pixels[source_row + columns[3]],
                        ];
                        let destinations: &mut [Color; 4] = destinations
                            .try_into()
                            .expect("chunks_exact_mut always yields four pixels");
                        blend_variable_opaque_quad(destinations, sources);
                    } else {
                        for (pixel, &source_column) in destinations.iter_mut().zip(columns) {
                            let color = texture.pixels[source_row + source_column];
                            if color.a == 255 {
                                *pixel = color;
                            } else if color.a > 0 {
                                *pixel = color.blend_over(*pixel);
                            }
                        }
                    }
                }
                for (pixel, &source_column) in remainder_pixels.iter_mut().zip(remainder_columns) {
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
            let samples = &scratch.horizontal_samples;

            if use_horizontal_cache {
                let horizontal_cache = &scratch.horizontal_cache;
                let top_offset = y0 * samples.len();
                let bottom_offset = y1 * samples.len();
                let inverse_y = 1.0 - ty;
                for (pixel, (&top, &bottom)) in
                    row[first_x as usize..end_x as usize].iter_mut().zip(
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
            } else if texture.fully_opaque {
                for (pixel, &(x0, x1, tx)) in row[first_x as usize..end_x as usize]
                    .iter_mut()
                    .zip(samples.iter())
                {
                    *pixel = sample_bilinear_opaque_prepared(
                        &texture.pixels,
                        row_width,
                        BilinearCoordinates {
                            x0,
                            x1,
                            y0,
                            y1,
                            tx,
                            ty,
                        },
                    );
                }
            } else {
                let visible_row = &mut row[first_x as usize..end_x as usize];
                let vectorized_len = visible_row.len() / 4 * 4;
                let (vectorized_pixels, remainder_pixels) =
                    visible_row.split_at_mut(vectorized_len);
                let (vectorized_samples, remainder_samples) = samples.split_at(vectorized_len);
                for (pixels, coordinates) in vectorized_pixels
                    .chunks_exact_mut(4)
                    .zip(vectorized_samples.chunks_exact(4))
                {
                    if pixels.iter().all(|pixel| pixel.a == 255) {
                        let sample = |&(x0, x1, tx): &(usize, usize, f32)| {
                            sample_bilinear_prepared(
                                &texture.pixels,
                                row_width,
                                BilinearCoordinates {
                                    x0,
                                    x1,
                                    y0,
                                    y1,
                                    tx,
                                    ty,
                                },
                            )
                        };
                        let sources = [
                            sample(&coordinates[0]),
                            sample(&coordinates[1]),
                            sample(&coordinates[2]),
                            sample(&coordinates[3]),
                        ];
                        let pixels: &mut [Color; 4] = pixels
                            .try_into()
                            .expect("chunks_exact_mut always yields four pixels");
                        blend_variable_opaque_quad(pixels, sources);
                    } else {
                        for (pixel, &(x0, x1, tx)) in pixels.iter_mut().zip(coordinates) {
                            let color = sample_bilinear_prepared(
                                &texture.pixels,
                                row_width,
                                BilinearCoordinates {
                                    x0,
                                    x1,
                                    y0,
                                    y1,
                                    tx,
                                    ty,
                                },
                            );
                            if color.a == 255 {
                                *pixel = color;
                            } else if color.a > 0 {
                                *pixel = color.blend_over(*pixel);
                            }
                        }
                    }
                }
                for (pixel, &(x0, x1, tx)) in remainder_pixels.iter_mut().zip(remainder_samples) {
                    let color = sample_bilinear_prepared(
                        &texture.pixels,
                        row_width,
                        BilinearCoordinates {
                            x0,
                            x1,
                            y0,
                            y1,
                            tx,
                            ty,
                        },
                    );
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

/// Draws a clipped texture using per-operation CPU rendering options.
///
/// The worker limit and configured area threshold are resolved for the
/// destination region. The texture renderer retains its conservative internal
/// cutoff for parallel work to avoid thread overhead on small blits.
pub fn draw_texture_clipped_with_options(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
    options: crate::RenderOptions,
) {
    let area = u64::from(destination.width) * u64::from(destination.height);
    let threads = options.worker_count(area);
    if threads <= 1 {
        draw_texture_clipped(target, texture, destination, filter, clip);
    } else {
        draw_texture_clipped_parallel_with_threads(
            target,
            texture,
            destination,
            filter,
            clip,
            threads,
        );
    }
}

/// Draws a clipped texture using independent row bands on scoped worker threads.
///
/// The operation preserves the sequential sampler's pixel results. It splits the
/// destination storage into disjoint mutable slices, so workers never share mutable
/// pixels and no framebuffer copy is made. Small workloads and a single requested
/// worker use the established sequential implementation.
/// Draws a clipped texture using parallel workers.
///
/// This compatibility wrapper panics only if temporary allocation fails.
/// Use `try_draw_texture_clipped_parallel_with_threads` when allocation
/// failure must be reported to the caller.
pub fn draw_texture_clipped_parallel_with_threads(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
    threads: usize,
) {
    try_draw_texture_clipped_parallel_with_threads(
        target,
        texture,
        destination,
        filter,
        clip,
        threads,
    )
    .expect("parallel texture drawing scratch allocation failed");
}

/// Fallible parallel texture drawing.
///
/// All temporary lookup buffers are reserved before framebuffer pixels are
/// changed. If allocation fails, the framebuffer is left untouched.
pub fn try_draw_texture_clipped_parallel_with_threads(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
    threads: usize,
) -> Result<(), TextureDrawError> {
    if threads <= 1 || destination.width.saturating_mul(destination.height) < 131_072 {
        draw_texture_clipped(target, texture, destination, filter, clip);
        return Ok(());
    }
    if destination.width == 0 || destination.height == 0 {
        return Ok(());
    }
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };

    let destination_right = u64::from(destination.x) + u64::from(destination.width);
    let destination_bottom = u64::from(destination.y) + u64::from(destination.height);
    let clip_right = u64::from(clip.x) + u64::from(clip.width);
    let clip_bottom = u64::from(clip.y) + u64::from(clip.height);
    let first_x = u64::from(destination.x).max(u64::from(clip.x));
    let first_y = u64::from(destination.y).max(u64::from(clip.y));
    let end_x = destination_right.min(clip_right);
    let end_y = destination_bottom.min(clip_bottom);
    if first_x >= end_x || first_y >= end_y {
        return Ok(());
    }

    if filter == TextureFilter::Nearest
        && texture.fully_opaque
        && destination.width == texture.width
        && destination.height == texture.height
    {
        let source_x = (first_x - u64::from(destination.x)) as usize;
        let copy_width = (end_x - first_x) as usize;
        let width = target.width() as usize;
        let height = target.height() as usize;
        let left = first_x as usize;
        let right = end_x as usize;
        let worker_count = threads.min(height).max(1);
        let rows_per_worker = height.div_ceil(worker_count);
        let chunk_len = rows_per_worker * width;
        std::thread::scope(|scope| {
            for (worker_index, pixels) in target.pixels_mut().chunks_mut(chunk_len).enumerate() {
                let row_start = worker_index * rows_per_worker;
                let row_end = (row_start + pixels.len() / width).min(height);
                let first_row = (first_y as usize).max(row_start);
                let last_row = (end_y as usize).min(row_end);
                if first_row >= last_row {
                    continue;
                }
                scope.spawn(move || {
                    for y in first_row..last_row {
                        let source_y = y - destination.y as usize;
                        let source_offset = source_y * texture.width as usize + source_x;
                        let local_row = (y - row_start) * width;
                        let row = &mut pixels[local_row..local_row + width];
                        row[left..right].copy_from_slice(
                            &texture.pixels[source_offset..source_offset + copy_width],
                        );
                    }
                });
            }
        });
        return Ok(());
    }

    let width = target.width() as usize;
    let height = target.height() as usize;
    let worker_count = threads.min(height).max(1);
    let rows_per_worker = height.div_ceil(worker_count);
    let chunk_len = rows_per_worker * width;
    let source_x_scale = texture.width as f32 / destination.width as f32;
    let source_y_scale = texture.height as f32 / destination.height as f32;
    let left = first_x as usize;
    let right = end_x as usize;
    // Horizontal bilinear coordinates are constant for every scanline.
    // Compute them once, then let all workers reuse the immutable lookup table.
    let mut horizontal_samples = Vec::new();
    if filter == TextureFilter::Bilinear {
        let sample_count =
            usize::try_from(end_x - first_x).map_err(|_| TextureDrawError::AllocationFailed)?;
        horizontal_samples
            .try_reserve_exact(sample_count)
            .map_err(|_| TextureDrawError::AllocationFailed)?;
        for x in first_x..end_x {
            let source_x = (x as f32 + 0.5 - destination.x as f32) * source_x_scale - 0.5;
            let source_x = source_x.clamp(0.0, texture.width as f32 - 1.0);
            let x0 = source_x.floor() as usize;
            horizontal_samples.push(BilinearHorizontalSample {
                x0,
                x1: (x0 + 1).min(texture.width as usize - 1),
                tx: source_x - x0 as f32,
            });
        }
    }

    // Share precomputed premultiplied horizontal samples across worker threads.
    // Bound memory use and keep the direct sampler for short or very wide draws.
    let horizontal_cache: Vec<[f32; 4]> = if filter == TextureFilter::Bilinear {
        let cache_len = (texture.height as usize).checked_mul(horizontal_samples.len());
        let cache_bytes =
            cache_len.and_then(|len| len.checked_mul(core::mem::size_of::<[f32; 4]>()));
        if end_y - first_y >= u64::from(texture.height) * 2
            && cache_bytes.is_some_and(|bytes| bytes <= 64 * 1024 * 1024)
        {
            let mut cache = Vec::new();
            cache
                .try_reserve_exact(cache_len.unwrap())
                .map_err(|_| TextureDrawError::AllocationFailed)?;
            let row_width = texture.width as usize;
            for source_row in 0..texture.height as usize {
                let source_offset = source_row * row_width;
                for sample in &horizontal_samples {
                    let left = texture.pixels[source_offset + sample.x0];
                    let right = texture.pixels[source_offset + sample.x1];
                    let inverse_x = 1.0 - sample.tx;
                    let left_alpha = f32::from(left.a) * inverse_x;
                    let right_alpha = f32::from(right.a) * sample.tx;
                    cache.push([
                        f32::from(left.r) * left_alpha + f32::from(right.r) * right_alpha,
                        f32::from(left.g) * left_alpha + f32::from(right.g) * right_alpha,
                        f32::from(left.b) * left_alpha + f32::from(right.b) * right_alpha,
                        left_alpha + right_alpha,
                    ]);
                }
            }
            cache
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let use_horizontal_cache = !horizontal_cache.is_empty();
    let horizontal_cache = &horizontal_cache;

    std::thread::scope(|scope| {
        for (worker_index, pixels) in target.pixels_mut().chunks_mut(chunk_len).enumerate() {
            let row_start = worker_index * rows_per_worker;
            let row_end = (row_start + pixels.len() / width).min(height);
            let first_row = (first_y as usize).max(row_start);
            let last_row = (end_y as usize).min(row_end);
            if first_row >= last_row {
                continue;
            }
            let horizontal_samples = &horizontal_samples;
            scope.spawn(move || {
                for y in first_row..last_row {
                    let local_row = (y - row_start) * width;
                    let row = &mut pixels[local_row..local_row + width];
                    let source_y = (y as f32 + 0.5 - destination.y as f32) * source_y_scale;
                    if filter == TextureFilter::Nearest {
                        let source_row = (source_y.floor() as u32).min(texture.height - 1) as usize
                            * texture.width as usize;
                        for (x, pixel) in row.iter_mut().enumerate().take(right).skip(left) {
                            let source_x = (x as f32 + 0.5 - destination.x as f32) * source_x_scale;
                            let source_column =
                                (source_x.floor() as u32).min(texture.width - 1) as usize;
                            let color = texture.pixels[source_row + source_column];
                            if color.a == 255 {
                                *pixel = color;
                            } else if color.a > 0 {
                                *pixel = color.blend_over(*pixel);
                            }
                        }
                    } else {
                        let sample_y = (source_y - 0.5).clamp(0.0, texture.height as f32 - 1.0);
                        let y0 = sample_y.floor() as usize;
                        let y1 = (y0 + 1).min(texture.height as usize - 1);
                        let ty = sample_y - y0 as f32;
                        let top_offset = y0 * texture.width as usize;
                        let bottom_offset = y1 * texture.width as usize;
                        let top_row =
                            &texture.pixels[top_offset..top_offset + texture.width as usize];
                        let bottom_row =
                            &texture.pixels[bottom_offset..bottom_offset + texture.width as usize];
                        if use_horizontal_cache {
                            let samples_per_row = horizontal_samples.len();
                            let top_offset = y0 * samples_per_row;
                            let bottom_offset = y1 * samples_per_row;
                            let inverse_y = 1.0 - ty;
                            for (pixel, (&top, &bottom)) in row[left..right].iter_mut().zip(
                                horizontal_cache[top_offset..top_offset + samples_per_row]
                                    .iter()
                                    .zip(
                                        &horizontal_cache
                                            [bottom_offset..bottom_offset + samples_per_row],
                                    ),
                            ) {
                                if texture.fully_opaque {
                                    *pixel =
                                        sample_cached_bilinear_opaque(&top, &bottom, inverse_y, ty);
                                } else if pixel.a == 255 {
                                    blend_cached_bilinear_over_opaque(
                                        &top, &bottom, inverse_y, ty, pixel,
                                    );
                                } else {
                                    let alpha = top[3] * inverse_y + bottom[3] * ty;
                                    if alpha <= f32::EPSILON {
                                        continue;
                                    }
                                    let inverse_alpha = alpha.recip();
                                    let color = Color::rgba(
                                        ((top[0] * inverse_y + bottom[0] * ty) * inverse_alpha)
                                            .round()
                                            .clamp(0.0, 255.0)
                                            as u8,
                                        ((top[1] * inverse_y + bottom[1] * ty) * inverse_alpha)
                                            .round()
                                            .clamp(0.0, 255.0)
                                            as u8,
                                        ((top[2] * inverse_y + bottom[2] * ty) * inverse_alpha)
                                            .round()
                                            .clamp(0.0, 255.0)
                                            as u8,
                                        alpha.round().clamp(0.0, 255.0) as u8,
                                    );
                                    if color.a > 0 {
                                        *pixel = color.blend_over(*pixel);
                                    }
                                }
                            }
                        } else {
                            for (pixel, sample) in
                                row[left..right].iter_mut().zip(horizontal_samples.iter())
                            {
                                if texture.fully_opaque {
                                    *pixel = sample_bilinear_opaque_rows(
                                        top_row, bottom_row, *sample, ty,
                                    );
                                } else if pixel.a == 255 {
                                    blend_bilinear_rows_over_opaque(
                                        top_row, bottom_row, *sample, ty, pixel,
                                    );
                                } else {
                                    let color = sample_bilinear_prepared(
                                        &texture.pixels,
                                        texture.width as usize,
                                        BilinearCoordinates {
                                            x0: sample.x0,
                                            x1: sample.x1,
                                            y0,
                                            y1,
                                            tx: sample.tx,
                                            ty,
                                        },
                                    );
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
            });
        }
    });
    Ok(())
}

// Returns a conservative horizontal interval for one inverse-mapped scanline.
// The final per-pixel bounds check remains authoritative; the one-pixel padding
// here prevents floating-point rounding at transformed edges from dropping pixels.
#[inline]
fn conservative_scanline_axis_range(
    first_x: u32,
    end_x: u32,
    reciprocal: f32,
    offset: f32,
    minimum: f32,
    maximum: f32,
) -> Option<(u32, u32)> {
    if reciprocal == 0.0 {
        return (offset >= minimum && offset < maximum).then_some((first_x, end_x));
    }

    let first_intersection = (minimum - offset) * reciprocal;
    let second_intersection = (maximum - offset) * reciprocal;
    let lower = first_intersection.min(second_intersection);
    let upper = first_intersection.max(second_intersection);
    let start = ((lower - 0.5).floor() as i64 - 1).max(i64::from(first_x));
    let end = ((upper - 0.5).ceil() as i64 + 2).min(i64::from(end_x));
    (start < end).then_some((start as u32, end as u32))
}

#[derive(Clone, Copy)]
struct TransformedScanlineBounds {
    inverse: Transform2D,
    reciprocal_m11: f32,
    reciprocal_m21: f32,
    first_x: u32,
    end_x: u32,
    local_left: f32,
    local_top: f32,
    local_right: f32,
    local_bottom: f32,
}

#[inline]
fn transformed_scanline_x_range(
    bounds: TransformedScanlineBounds,
    pixel_center_y: f32,
) -> Option<(u32, u32)> {
    let x_offset = bounds.inverse.m12 * pixel_center_y + bounds.inverse.tx;
    let y_offset = bounds.inverse.m22 * pixel_center_y + bounds.inverse.ty;
    let (x_start, x_end) = conservative_scanline_axis_range(
        bounds.first_x,
        bounds.end_x,
        bounds.reciprocal_m11,
        x_offset,
        bounds.local_left,
        bounds.local_right,
    )?;
    let (y_start, y_end) = conservative_scanline_axis_range(
        bounds.first_x,
        bounds.end_x,
        bounds.reciprocal_m21,
        y_offset,
        bounds.local_top,
        bounds.local_bottom,
    )?;
    let start = x_start.max(y_start);
    let end = x_end.min(y_end);
    (start < end).then_some((start, end))
}

/// Draws an affine-transformed texture using independent destination row bands.
///
/// This is the parallel counterpart of draw_texture_transformed_clipped.
/// It preserves the inverse-mapped pixel-center sampling and blends each pixel
/// exactly once. Workers own disjoint framebuffer slices; the source texture and
/// transform are shared read-only.
pub fn draw_texture_transformed_clipped_parallel_with_threads(
    target: &mut Framebuffer,
    texture: &Texture,
    destination: Rect,
    transform: Transform2D,
    filter: TextureFilter,
    clip: Rect,
    threads: usize,
) -> bool {
    if threads <= 1 || destination.width.saturating_mul(destination.height) < 131_072 {
        return draw_texture_transformed_clipped(
            target,
            texture,
            destination,
            transform,
            filter,
            clip,
        );
    }
    if destination.width == 0 || destination.height == 0 || !transform.is_finite() {
        return false;
    }
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return false;
    };
    if transform == Transform2D::IDENTITY {
        let Some(visible) = destination.intersection(clip) else {
            return false;
        };
        if visible.width == 0 || visible.height == 0 {
            return false;
        }
        draw_texture_clipped_parallel_with_threads(
            target,
            texture,
            destination,
            filter,
            clip,
            threads,
        );
        return true;
    }
    let Some(inverse) = transform.inverse() else {
        return false;
    };
    let reciprocal_m11 = if inverse.m11 == 0.0 {
        0.0
    } else {
        inverse.m11.recip()
    };
    let reciprocal_m21 = if inverse.m21 == 0.0 {
        0.0
    } else {
        inverse.m21.recip()
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
    let scanline_bounds = TransformedScanlineBounds {
        inverse,
        reciprocal_m11,
        reciprocal_m21,
        first_x,
        end_x,
        local_left,
        local_top,
        local_right,
        local_bottom,
    };
    let source_scale_x = texture.width as f32 / destination.width as f32;
    let source_scale_y = texture.height as f32 / destination.height as f32;
    let texture_width = texture.width as usize;
    let texture_height = texture.height as usize;
    let width = target.width() as usize;
    let height = target.height() as usize;
    let worker_count = threads.min(height).max(1);
    let rows_per_worker = height.div_ceil(worker_count);
    let chunk_len = rows_per_worker * width;
    // The inverse affine transform separates into an X contribution and a
    // Y contribution. Cache the X terms once for the whole draw so workers
    // only need two additions per destination pixel instead of repeating
    // the two X multiplications and pixel-center conversion on every row.
    // Keeping the same multiplication and addition order preserves the
    // previous per-pixel coordinates exactly.
    let horizontal_terms: Vec<(f32, f32)> = (first_x..end_x)
        .map(|x| {
            let pixel_center_x = x as f32 + 0.5;
            (inverse.m11 * pixel_center_x, inverse.m21 * pixel_center_x)
        })
        .collect();
    let mut any_visible = false;

    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for (worker_index, pixels) in target.pixels_mut().chunks_mut(chunk_len).enumerate() {
            let row_start = worker_index * rows_per_worker;
            let row_end = (row_start + pixels.len() / width).min(height);
            let first_row = (first_y as usize).max(row_start);
            let last_row = (end_y as usize).min(row_end);
            if first_row >= last_row {
                continue;
            }
            let horizontal_terms = horizontal_terms.as_slice();
            handles.push(scope.spawn(move || {
                let mut worker_visible = false;
                for y in first_row..last_row {
                    let pixel_center_y = y as f32 + 0.5;
                    let local_row = (y - row_start) * width;
                    let row = &mut pixels[local_row..local_row + width];
                    let vertical_x = inverse.m12 * pixel_center_y;
                    let vertical_y = inverse.m22 * pixel_center_y;
                    let Some((row_first_x, row_end_x)) =
                        transformed_scanline_x_range(scanline_bounds, pixel_center_y)
                    else {
                        continue;
                    };
                    for x in row_first_x..row_end_x {
                        let (horizontal_x, horizontal_y) = horizontal_terms[(x - first_x) as usize];
                        // Preserve the sequential renderer's left-to-right
                        // floating-point addition order exactly.
                        let local_x = (horizontal_x + vertical_x) + inverse.tx;
                        let local_y = (horizontal_y + vertical_y) + inverse.ty;
                        if local_x < local_left
                            || local_x >= local_right
                            || local_y < local_top
                            || local_y >= local_bottom
                        {
                            continue;
                        }
                        worker_visible = true;
                        let source_x = (local_x - local_left) * source_scale_x;
                        let source_y = (local_y - local_top) * source_scale_y;
                        let color = match filter {
                            TextureFilter::Nearest => {
                                let sample_x =
                                    source_x.floor().clamp(0.0, texture.width as f32 - 1.0)
                                        as usize;
                                let sample_y =
                                    source_y.floor().clamp(0.0, texture.height as f32 - 1.0)
                                        as usize;
                                texture.pixels[sample_y * texture_width + sample_x]
                            }
                            TextureFilter::Bilinear => {
                                let sample_x =
                                    (source_x - 0.5).clamp(0.0, texture.width as f32 - 1.0);
                                let sample_y =
                                    (source_y - 0.5).clamp(0.0, texture.height as f32 - 1.0);
                                let x0 = sample_x.floor() as usize;
                                let y0 = sample_y.floor() as usize;
                                let coordinates = BilinearCoordinates {
                                    x0,
                                    x1: (x0 + 1).min(texture_width - 1),
                                    y0,
                                    y1: (y0 + 1).min(texture_height - 1),
                                    tx: sample_x - x0 as f32,
                                    ty: sample_y - y0 as f32,
                                };
                                if texture.fully_opaque {
                                    sample_bilinear_opaque_prepared(
                                        &texture.pixels,
                                        texture_width,
                                        coordinates,
                                    )
                                } else if row[x as usize].a == 255 {
                                    let top_offset = y0 * texture_width;
                                    let bottom_offset = coordinates.y1 * texture_width;
                                    let sample = BilinearHorizontalSample {
                                        x0,
                                        x1: (x0 + 1).min(texture_width - 1),
                                        tx: sample_x - x0 as f32,
                                    };
                                    blend_bilinear_rows_over_opaque(
                                        &texture.pixels[top_offset..top_offset + texture_width],
                                        &texture.pixels
                                            [bottom_offset..bottom_offset + texture_width],
                                        sample,
                                        coordinates.ty,
                                        &mut row[x as usize],
                                    );
                                    continue;
                                } else {
                                    sample_bilinear_prepared(
                                        &texture.pixels,
                                        texture_width,
                                        coordinates,
                                    )
                                }
                            }
                        };
                        if texture.fully_opaque || color.a == 255 {
                            row[x as usize] = color;
                        } else if color.a > 0 {
                            row[x as usize] = color.blend_over(row[x as usize]);
                        }
                    }
                }
                worker_visible
            }));
        }
        for handle in handles {
            any_visible |= handle.join().unwrap_or(false);
        }
    });
    any_visible
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
    let reciprocal_m11 = if inverse.m11 == 0.0 {
        0.0
    } else {
        inverse.m11.recip()
    };
    let reciprocal_m21 = if inverse.m21 == 0.0 {
        0.0
    } else {
        inverse.m21.recip()
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
    let scanline_bounds = TransformedScanlineBounds {
        inverse,
        reciprocal_m11,
        reciprocal_m21,
        first_x,
        end_x,
        local_left,
        local_top,
        local_right,
        local_bottom,
    };
    let mut visible = false;
    let source_scale_x = texture.width as f32 / destination.width as f32;
    let source_scale_y = texture.height as f32 / destination.height as f32;
    let texture_width = texture.width as usize;
    let texture_height = texture.height as usize;
    // The inverse affine transform separates into horizontal and vertical
    // contributions. Cache the X terms once for the whole draw, preserving
    // the original left-to-right floating-point addition order per pixel.
    let horizontal_terms: Vec<(f32, f32)> = (first_x..end_x)
        .map(|x| {
            let pixel_center_x = x as f32 + 0.5;
            (inverse.m11 * pixel_center_x, inverse.m21 * pixel_center_x)
        })
        .collect();
    for y in first_y..end_y {
        let pixel_center_y = y as f32 + 0.5;
        let Some((row_first_x, row_end_x)) =
            transformed_scanline_x_range(scanline_bounds, pixel_center_y)
        else {
            continue;
        };
        let vertical_x = inverse.m12 * pixel_center_y;
        let vertical_y = inverse.m22 * pixel_center_y;
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in row_first_x..row_end_x {
            let (horizontal_x, horizontal_y) = horizontal_terms[(x - first_x) as usize];
            let local_x = (horizontal_x + vertical_x) + inverse.tx;
            let local_y = (horizontal_y + vertical_y) + inverse.ty;
            if local_x < local_left
                || local_x >= local_right
                || local_y < local_top
                || local_y >= local_bottom
            {
                continue;
            }

            visible = true;
            let source_x = (local_x - local_left) * source_scale_x;
            let source_y = (local_y - local_top) * source_scale_y;
            // Sample directly after validating the transformed source bounds,
            // using the opaque bilinear kernel whenever possible.
            let color = match filter {
                TextureFilter::Nearest => {
                    let sample_x = source_x.floor().clamp(0.0, texture.width as f32 - 1.0) as usize;
                    let sample_y =
                        source_y.floor().clamp(0.0, texture.height as f32 - 1.0) as usize;
                    texture.pixels[sample_y * texture_width + sample_x]
                }
                TextureFilter::Bilinear => {
                    let sample_x = (source_x - 0.5).clamp(0.0, texture.width as f32 - 1.0);
                    let sample_y = (source_y - 0.5).clamp(0.0, texture.height as f32 - 1.0);
                    let x0 = sample_x.floor() as usize;
                    let y0 = sample_y.floor() as usize;
                    let coordinates = BilinearCoordinates {
                        x0,
                        x1: (x0 + 1).min(texture_width - 1),
                        y0,
                        y1: (y0 + 1).min(texture_height - 1),
                        tx: sample_x - x0 as f32,
                        ty: sample_y - y0 as f32,
                    };
                    if texture.fully_opaque {
                        sample_bilinear_opaque_prepared(&texture.pixels, texture_width, coordinates)
                    } else if row[x as usize].a == 255 {
                        let top_offset = y0 * texture_width;
                        let bottom_offset = coordinates.y1 * texture_width;
                        let sample = BilinearHorizontalSample {
                            x0,
                            x1: (x0 + 1).min(texture_width - 1),
                            tx: sample_x - x0 as f32,
                        };
                        blend_bilinear_rows_over_opaque(
                            &texture.pixels[top_offset..top_offset + texture_width],
                            &texture.pixels[bottom_offset..bottom_offset + texture_width],
                            sample,
                            coordinates.ty,
                            &mut row[x as usize],
                        );
                        continue;
                    } else {
                        sample_bilinear_prepared(&texture.pixels, texture_width, coordinates)
                    }
                }
            };
            if texture.fully_opaque || color.a == 255 {
                row[x as usize] = color;
            } else if color.a > 0 {
                row[x as usize] = color.blend_over(row[x as usize]);
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
    use crate::{
        draw_texture_clipped_with_scratch, draw_texture_region_clipped_with_scratch, Color,
        Framebuffer, Rect, TextureDrawScratch, Transform2D,
    };

    fn two_color_texture() -> Texture {
        Texture::new(2, 1, vec![Color::RED, Color::BLUE]).unwrap()
    }

    #[test]
    fn nine_slice_preserves_corners_and_stretches_center() {
        use super::{draw_texture_nine_slice_clipped, NineSliceMargins};

        let colors = [
            Color::RED,
            Color::GREEN,
            Color::BLUE,
            Color::rgb(255, 255, 0),
            Color::WHITE,
            Color::rgb(255, 0, 255),
            Color::rgb(0, 255, 255),
            Color::rgb(90, 100, 110),
            Color::rgb(120, 130, 140),
        ];
        let texture = Texture::new(3, 3, colors.to_vec()).unwrap();
        let mut target = Framebuffer::new(7, 7, Color::BLACK).unwrap();
        draw_texture_nine_slice_clipped(
            &mut target,
            &texture,
            Rect::new(0, 0, 3, 3),
            Rect::new(0, 0, 7, 7),
            NineSliceMargins::new(1, 1, 1, 1),
            TextureFilter::Nearest,
            Rect::new(0, 0, 7, 7),
        );

        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(6, 0), Some(Color::BLUE));
        assert_eq!(target.get_pixel(0, 6), Some(colors[6]));
        assert_eq!(target.get_pixel(6, 6), Some(colors[8]));
        assert_eq!(target.get_pixel(3, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(3, 0), Some(Color::GREEN));
        assert_eq!(target.get_pixel(0, 3), Some(colors[3]));
    }

    #[test]
    fn nine_slice_reuses_scratch_and_matches_stateless_rendering() {
        use super::{
            draw_texture_nine_slice_clipped, draw_texture_nine_slice_clipped_with_scratch,
            NineSliceMargins,
        };

        let pixels = (0..16)
            .map(|index| {
                Color::rgba(
                    (index * 13) as u8,
                    (index * 7) as u8,
                    (255 - index * 9) as u8,
                    if index % 3 == 0 { 128 } else { 255 },
                )
            })
            .collect();
        let texture = Texture::new(4, 4, pixels).unwrap();
        let source = Rect::new(0, 0, 4, 4);
        let destination = Rect::new(1, 1, 13, 11);
        let margins = NineSliceMargins::new(1, 1, 1, 1);
        let clip = Rect::new(0, 0, 15, 13);

        let mut expected = Framebuffer::new(15, 13, Color::rgb(20, 30, 40)).unwrap();
        draw_texture_nine_slice_clipped(
            &mut expected,
            &texture,
            source,
            destination,
            margins,
            TextureFilter::Bilinear,
            clip,
        );

        let mut actual = Framebuffer::new(15, 13, Color::rgb(20, 30, 40)).unwrap();
        let mut scratch = TextureDrawScratch::default();
        draw_texture_nine_slice_clipped_with_scratch(
            &mut actual,
            &texture,
            source,
            destination,
            margins,
            TextureFilter::Bilinear,
            clip,
            &mut scratch,
        );
        assert_eq!(actual.pixels(), expected.pixels());
        assert!(!scratch.horizontal_samples.is_empty());
        let samples_ptr = scratch.horizontal_samples.as_ptr();
        let samples_capacity = scratch.horizontal_samples.capacity();

        actual.clear(Color::rgb(20, 30, 40));
        draw_texture_nine_slice_clipped_with_scratch(
            &mut actual,
            &texture,
            source,
            destination,
            margins,
            TextureFilter::Bilinear,
            clip,
            &mut scratch,
        );
        assert_eq!(actual.pixels(), expected.pixels());
        assert_eq!(scratch.horizontal_samples.as_ptr(), samples_ptr);
        assert_eq!(scratch.horizontal_samples.capacity(), samples_capacity);
    }

    #[test]
    fn nine_slice_compresses_borders_when_destination_is_too_small() {
        use super::{draw_texture_nine_slice_clipped, NineSliceMargins};

        let texture = Texture::new(
            3,
            3,
            vec![
                Color::RED,
                Color::GREEN,
                Color::BLUE,
                Color::rgb(255, 255, 0),
                Color::WHITE,
                Color::rgb(255, 0, 255),
                Color::rgb(0, 255, 255),
                Color::rgb(90, 100, 110),
                Color::rgb(120, 130, 140),
            ],
        )
        .unwrap();
        let mut target = Framebuffer::new(1, 1, Color::BLACK).unwrap();
        draw_texture_nine_slice_clipped(
            &mut target,
            &texture,
            Rect::new(0, 0, 3, 3),
            Rect::new(0, 0, 1, 1),
            NineSliceMargins::new(1, 1, 1, 1),
            TextureFilter::Nearest,
            Rect::new(0, 0, 1, 1),
        );
        assert_ne!(target.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn nine_slice_rejects_invalid_margins_without_writing() {
        use super::{draw_texture_nine_slice_clipped, NineSliceMargins};

        let texture = two_color_texture();
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        draw_texture_nine_slice_clipped(
            &mut target,
            &texture,
            Rect::new(0, 0, 2, 1),
            Rect::new(0, 0, 4, 4),
            NineSliceMargins::new(2, 0, 1, 0),
            TextureFilter::Nearest,
            Rect::new(0, 0, 4, 4),
        );
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
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
            blend_cached_bilinear_over_opaque(&top, &bottom, inverse_y, ty, &mut actual);
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
                    let source = (premultiplied / alpha).round().clamp(0.0, 255.0) as u8;
                    let source_alpha = alpha.round().clamp(0.0, 255.0) as u8;
                    let expected = Color::rgba(source, source, source, source_alpha)
                        .blend_over(Color::rgb(destination, destination, destination));
                    let actual = ((premultiplied + f32::from(destination) * (255.0 - alpha))
                        / 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    assert!(actual.abs_diff(expected.r) <= 1);
                }
            }
        }
    }

    #[test]
    fn parallel_texture_draw_matches_sequential_for_both_filters_and_alpha() {
        use super::{
            draw_texture_clipped_parallel_with_threads,
            try_draw_texture_clipped_parallel_with_threads,
        };

        let texture = Texture::new(
            257,
            193,
            (0..257 * 193)
                .map(|index| {
                    Color::rgba(
                        (index * 17) as u8,
                        (index * 31) as u8,
                        (index * 47) as u8,
                        (index * 13) as u8,
                    )
                })
                .collect(),
        )
        .unwrap();

        for filter in [TextureFilter::Nearest, TextureFilter::Bilinear] {
            for background in [Color::rgb(12, 34, 56), Color::rgba(12, 34, 56, 190)] {
                let mut sequential = Framebuffer::new(640, 480, background).unwrap();
                let mut parallel = sequential.clone();
                let destination = Rect::new(23, 17, 580, 430);
                let clip = Rect::new(41, 29, 511, 401);

                draw_texture_clipped(&mut sequential, &texture, destination, filter, clip);
                draw_texture_clipped_parallel_with_threads(
                    &mut parallel,
                    &texture,
                    destination,
                    filter,
                    clip,
                    4,
                );

                let mut fallible_parallel = Framebuffer::new(640, 480, background).unwrap();
                try_draw_texture_clipped_parallel_with_threads(
                    &mut fallible_parallel,
                    &texture,
                    destination,
                    filter,
                    clip,
                    4,
                )
                .unwrap();

                assert_eq!(parallel.pixels(), sequential.pixels());
                assert_eq!(fallible_parallel.pixels(), sequential.pixels());
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
            let mut target = Framebuffer::new(width, height, Color::rgba(20, 40, 60, 120)).unwrap();
            draw_texture(
                &mut target,
                &texture,
                Rect::new(0, 0, width, height),
                TextureFilter::Bilinear,
            );

            let source_width = texture.width();
            let source_height = texture.height();
            let source_pixels = texture.pixels();
            let expected = (0..height)
                .flat_map(|y| {
                    let source_y = ((y as f32 + 0.5) * source_height as f32 / height as f32 - 0.5)
                        .clamp(0.0, source_height as f32 - 1.0);
                    let y0 = source_y.floor() as usize;
                    let y1 = (y0 + 1).min(source_height as usize - 1);
                    let ty = source_y - y0 as f32;
                    (0..width).map(move |x| {
                        let source_x = ((x as f32 + 0.5) * source_width as f32 / width as f32
                            - 0.5)
                            .clamp(0.0, source_width as f32 - 1.0);
                        let x0 = source_x.floor() as usize;
                        let x1 = (x0 + 1).min(source_width as usize - 1);
                        sample_bilinear_prepared(
                            source_pixels,
                            source_width as usize,
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
    fn opaque_one_to_one_nearest_copy_respects_clipped_source_offsets() {
        let texture = Texture::new(
            4,
            3,
            (0..12)
                .map(|index| Color::rgb((index * 17) as u8, (index * 29) as u8, (index * 41) as u8))
                .collect(),
        )
        .unwrap();
        let background = Color::rgb(3, 5, 7);
        let mut target = Framebuffer::new(6, 5, background).unwrap();

        draw_texture_clipped(
            &mut target,
            &texture,
            Rect::new(1, 1, 4, 3),
            TextureFilter::Nearest,
            Rect::new(2, 0, 2, 4),
        );

        for y in 0..5 {
            for x in 0..6 {
                let expected = if (2..4).contains(&x) && (1..4).contains(&y) {
                    texture.get_pixel(x - 1, y - 1).unwrap()
                } else {
                    background
                };
                assert_eq!(target.get_pixel(x, y), Some(expected));
            }
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
    fn incremental_affine_sampling_matches_direct_inverse_mapping() {
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
        let destination = Rect::new(1, 1, 5, 4);
        let clip = Rect::new(0, 0, 12, 10);
        let transforms = [
            Transform2D::rotation(0.37).then(Transform2D::translation(4.0, 2.0)),
            Transform2D::scale(1.7, 0.8).then(Transform2D::translation(2.0, 1.0)),
            Transform2D::new(1.0, 0.25, -0.2, 1.0, 3.0, 2.0),
        ];

        for transform in transforms {
            for filter in [TextureFilter::Nearest, TextureFilter::Bilinear] {
                let mut actual = Framebuffer::new(12, 10, Color::rgb(12, 24, 36)).unwrap();
                let mut expected = actual.clone();
                assert!(draw_texture_transformed_clipped(
                    &mut actual,
                    &texture,
                    destination,
                    transform,
                    filter,
                    clip,
                ));

                let inverse = transform.inverse().unwrap();
                for y in 0..expected.height() {
                    for x in 0..expected.width() {
                        let local = inverse
                            .transform_point(crate::Vector::new(x as f32 + 0.5, y as f32 + 0.5));
                        if local.x < destination.x as f32
                            || local.x >= (destination.x + destination.width) as f32
                            || local.y < destination.y as f32
                            || local.y >= (destination.y + destination.height) as f32
                        {
                            continue;
                        }
                        let source_x = (local.x - destination.x as f32) * texture.width as f32
                            / destination.width as f32;
                        let source_y = (local.y - destination.y as f32) * texture.height as f32
                            / destination.height as f32;
                        let color = match filter {
                            TextureFilter::Nearest => texture.sample_nearest(source_x, source_y),
                            TextureFilter::Bilinear => {
                                texture.sample_bilinear(source_x - 0.5, source_y - 0.5)
                            }
                        };
                        if color.a == 255 {
                            expected.set_pixel(x, y, color);
                        } else if color.a > 0 {
                            let background = expected.get_pixel(x, y).unwrap();
                            expected.set_pixel(x, y, color.blend_over(background));
                        }
                    }
                }

                for (index, (actual, expected)) in
                    actual.pixels().iter().zip(expected.pixels()).enumerate()
                {
                    assert!(
                        actual.r.abs_diff(expected.r) <= 1
                            && actual.g.abs_diff(expected.g) <= 1
                            && actual.b.abs_diff(expected.b) <= 1
                            && actual.a == expected.a,
                        "transform: {transform:?}, filter: {filter:?}, pixel {index}: actual={actual:?}, expected={expected:?}"
                    );
                }
            }
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
    fn parallel_transformed_texture_matches_sequential_for_both_filters() {
        use super::draw_texture_transformed_clipped_parallel_with_threads;

        let texture = Texture::new(
            256,
            192,
            (0..256 * 192)
                .map(|index| {
                    Color::rgba(
                        (index * 7) as u8,
                        (index * 19) as u8,
                        (index * 37) as u8,
                        (index * 11) as u8,
                    )
                })
                .collect(),
        )
        .unwrap();
        let transform = Transform2D::translation(35.0, 18.0)
            .then(Transform2D::rotation(0.12))
            .then(Transform2D::scale(1.8, 1.7));
        for filter in [TextureFilter::Nearest, TextureFilter::Bilinear] {
            let mut sequential = Framebuffer::new(640, 480, Color::rgba(21, 43, 65, 177)).unwrap();
            let mut parallel = sequential.clone();
            let destination = Rect::new(10, 12, 400, 350);
            let clip = Rect::new(5, 7, 620, 460);
            let expected_visible = draw_texture_transformed_clipped(
                &mut sequential,
                &texture,
                destination,
                transform,
                filter,
                clip,
            );
            let actual_visible = draw_texture_transformed_clipped_parallel_with_threads(
                &mut parallel,
                &texture,
                destination,
                transform,
                filter,
                clip,
                4,
            );
            assert_eq!(actual_visible, expected_visible);
            assert_eq!(parallel.pixels(), sequential.pixels());
        }
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
    fn atlas_region_draw_does_not_sample_neighboring_sprites() {
        let texture =
            Texture::new(4, 1, vec![Color::RED, Color::RED, Color::BLUE, Color::BLUE]).unwrap();
        let mut target = Framebuffer::new(4, 1, Color::BLACK).unwrap();
        super::draw_texture_region_clipped(
            &mut target,
            &texture,
            Rect::new(0, 0, 2, 1),
            Rect::new(0, 0, 4, 1),
            TextureFilter::Bilinear,
            Rect::new(0, 0, 4, 1),
        );
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::RED));
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
    fn fallible_framebuffer_conversion_copies_pixels_into_independent_texture() {
        let framebuffer = Framebuffer::new(2, 1, Color::GREEN).unwrap();
        let original_storage = framebuffer.pixels().as_ptr();
        let texture = Texture::try_from_framebuffer(&framebuffer).unwrap();

        assert_ne!(texture.pixels().as_ptr(), original_storage);
        assert_eq!(texture.width(), 2);
        assert_eq!(texture.height(), 1);
        assert!(texture.is_fully_opaque());
        assert!(texture.pixels().iter().all(|pixel| *pixel == Color::GREEN));
    }

    #[test]
    fn fallible_texture_clone_copies_pixels_into_independent_storage() {
        let original = Texture::new(2, 1, vec![Color::RED, Color::rgba(0, 0, 255, 128)]).unwrap();
        let copied = original.try_clone().unwrap();

        assert_eq!(copied.width(), original.width());
        assert_eq!(copied.height(), original.height());
        assert_eq!(copied.pixels(), original.pixels());
        assert_ne!(copied.pixels().as_ptr(), original.pixels().as_ptr());
        assert_eq!(copied.is_fully_opaque(), original.is_fully_opaque());
    }

    #[test]
    fn fallible_texture_creation_copies_borrowed_pixels() {
        let source = [Color::RED, Color::GREEN, Color::BLUE, Color::WHITE];
        let texture = Texture::try_from_pixels(2, 2, &source).unwrap();

        assert_eq!(texture.width(), 2);
        assert_eq!(texture.height(), 2);
        assert_eq!(texture.pixels(), &source);
        assert!(texture.is_fully_opaque());
        assert_ne!(texture.pixels().as_ptr(), source.as_ptr());
    }

    #[test]
    fn fallible_texture_creation_validates_dimensions_and_pixel_count() {
        assert!(matches!(
            Texture::try_from_pixels(0, 1, &[]),
            Err(super::TextureError::ZeroDimension)
        ));
        assert!(matches!(
            Texture::try_from_pixels(2, 2, &[Color::RED]),
            Err(super::TextureError::PixelCountMismatch)
        ));
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
    #[test]
    fn opaque_nearest_upscale_run_fill_matches_reference_when_clipped() {
        let texture = Texture::new(
            3,
            2,
            vec![
                Color::RED,
                Color::GREEN,
                Color::BLUE,
                Color::WHITE,
                Color::rgb(20, 40, 60),
                Color::rgb(80, 100, 120),
            ],
        )
        .unwrap();
        let destination = Rect::new(1, 1, 11, 5);
        let clip = Rect::new(3, 0, 8, 6);
        let mut expected = Framebuffer::new(13, 7, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        let mut scratch = super::TextureDrawScratch::default();

        // Independent scalar reference: map each visible destination pixel
        // back to its source texel using pixel-center nearest-neighbor sampling.
        for y in 1..6 {
            let source_y = (((y as f32 + 0.5 - destination.y as f32) * texture.height() as f32)
                / destination.height as f32)
                .floor()
                .min(texture.height() as f32 - 1.0) as u32;
            for x in 3..11 {
                let source_x = (((x as f32 + 0.5 - destination.x as f32) * texture.width() as f32)
                    / destination.width as f32)
                    .floor()
                    .min(texture.width() as f32 - 1.0) as u32;
                expected.set_pixel(x, y, texture.get_pixel(source_x, source_y).unwrap());
            }
        }

        super::draw_texture_clipped_with_scratch(
            &mut actual,
            &texture,
            destination,
            TextureFilter::Nearest,
            clip,
            &mut scratch,
        );

        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn fallible_texture_scratch_draw_matches_standard_draw_for_both_filters() {
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
        let destination = Rect::new(1, 1, 6, 6);
        let clip = Rect::new(2, 0, 5, 7);
        let mut scratch = TextureDrawScratch::default();

        for filter in [TextureFilter::Nearest, TextureFilter::Bilinear] {
            let mut expected = Framebuffer::new(9, 8, Color::rgb(12, 24, 36)).unwrap();
            let mut actual = expected.clone();
            draw_texture_clipped(&mut expected, &texture, destination, filter, clip);
            super::try_draw_texture_clipped_with_scratch(
                &mut actual,
                &texture,
                destination,
                filter,
                clip,
                &mut scratch,
            )
            .unwrap();
            assert_eq!(actual.pixels(), expected.pixels(), "filter: {filter:?}");
        }
    }

    #[test]
    fn reusable_texture_scratch_matches_standard_draw_for_both_filters() {
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
        let destination = Rect::new(1, 1, 6, 6);
        let clip = Rect::new(2, 0, 5, 7);
        let mut scratch = TextureDrawScratch::default();

        for filter in [TextureFilter::Nearest, TextureFilter::Bilinear] {
            let mut expected = Framebuffer::new(9, 8, Color::rgb(12, 24, 36)).unwrap();
            let mut actual = expected.clone();
            draw_texture_clipped(&mut expected, &texture, destination, filter, clip);
            draw_texture_clipped_with_scratch(
                &mut actual,
                &texture,
                destination,
                filter,
                clip,
                &mut scratch,
            );
            assert_eq!(actual.pixels(), expected.pixels(), "filter: {filter:?}");
        }
    }

    #[test]
    fn atlas_bilinear_scratch_reuses_horizontal_samples_and_invalidates_geometry() {
        let texture = Texture::new(
            4,
            2,
            vec![
                Color::rgba(240, 30, 20, 180),
                Color::rgba(20, 220, 80, 220),
                Color::rgba(30, 60, 240, 120),
                Color::rgba(240, 180, 20, 200),
                Color::rgba(20, 40, 200, 160),
                Color::rgba(220, 80, 30, 210),
                Color::rgba(40, 200, 120, 130),
                Color::rgba(180, 40, 220, 190),
            ],
        )
        .unwrap();
        let source = Rect::new(1, 0, 2, 2);
        let destination = Rect::new(0, 0, 8, 8);
        let clip = Rect::new(0, 0, 8, 8);
        let mut scratch = TextureDrawScratch::default();
        let mut first = Framebuffer::new(8, 8, Color::BLACK).unwrap();

        draw_texture_region_clipped_with_scratch(
            &mut first,
            &texture,
            source,
            destination,
            TextureFilter::Bilinear,
            clip,
            &mut scratch,
        );
        let first_key = scratch.cached_region_bilinear_key;
        let first_ptr = scratch.region_horizontal_cache.as_ptr();
        let first_pixels = first.pixels().to_vec();

        assert!(first_key.is_some());
        assert_eq!(scratch.region_horizontal_cache.len(), 16);

        let mut second = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        draw_texture_region_clipped_with_scratch(
            &mut second,
            &texture,
            source,
            destination,
            TextureFilter::Bilinear,
            clip,
            &mut scratch,
        );
        assert_eq!(second.pixels(), first_pixels);
        assert_eq!(scratch.cached_region_bilinear_key, first_key);
        assert_eq!(scratch.region_horizontal_cache.as_ptr(), first_ptr);

        let resized_destination = Rect::new(0, 0, 7, 8);
        let mut resized = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        draw_texture_region_clipped_with_scratch(
            &mut resized,
            &texture,
            source,
            resized_destination,
            TextureFilter::Bilinear,
            clip,
            &mut scratch,
        );
        assert_ne!(scratch.cached_region_bilinear_key, first_key);
    }

    #[test]
    fn reusable_texture_scratch_retains_allocations_between_draws() {
        let texture = Texture::new(
            2,
            2,
            vec![Color::RED, Color::GREEN, Color::BLUE, Color::WHITE],
        )
        .unwrap();
        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        let mut scratch = TextureDrawScratch::default();
        let clip = Rect::new(0, 0, 8, 8);
        let destination = Rect::new(0, 0, 8, 8);

        draw_texture_clipped_with_scratch(
            &mut target,
            &texture,
            destination,
            TextureFilter::Nearest,
            clip,
            &mut scratch,
        );
        let nearest_capacity = scratch.nearest_columns.capacity();
        let nearest_ptr = scratch.nearest_columns.as_ptr();

        draw_texture_clipped_with_scratch(
            &mut target,
            &texture,
            destination,
            TextureFilter::Nearest,
            clip,
            &mut scratch,
        );
        assert_eq!(scratch.nearest_columns.capacity(), nearest_capacity);
        assert_eq!(scratch.nearest_columns.as_ptr(), nearest_ptr);

        draw_texture_clipped_with_scratch(
            &mut target,
            &texture,
            destination,
            TextureFilter::Bilinear,
            clip,
            &mut scratch,
        );
        let samples_capacity = scratch.horizontal_samples.capacity();
        let samples_ptr = scratch.horizontal_samples.as_ptr();
        let cache_capacity = scratch.horizontal_cache.capacity();
        let cache_ptr = scratch.horizontal_cache.as_ptr();

        draw_texture_clipped_with_scratch(
            &mut target,
            &texture,
            destination,
            TextureFilter::Bilinear,
            clip,
            &mut scratch,
        );
        assert_eq!(scratch.horizontal_samples.capacity(), samples_capacity);
        assert_eq!(scratch.horizontal_samples.as_ptr(), samples_ptr);
        assert_eq!(scratch.horizontal_cache.capacity(), cache_capacity);
        assert_eq!(scratch.horizontal_cache.as_ptr(), cache_ptr);
    }
}

#[cfg(test)]
mod render_options_tests {
    use crate::{
        draw_texture, draw_texture_clipped, draw_texture_clipped_with_options,
        draw_texture_region_clipped, Color, Framebuffer, Rect, RenderOptions, Texture,
        TextureFilter,
    };

    #[test]
    fn configured_texture_draw_matches_serial_output() {
        let texture = Texture::new(
            16,
            16,
            (0..256)
                .map(|index| {
                    Color::rgba(
                        index as u8,
                        (index * 3) as u8,
                        (index * 7) as u8,
                        if index % 3 == 0 { 128 } else { 255 },
                    )
                })
                .collect(),
        )
        .unwrap();
        let clip = Rect::new(32, 16, 160, 96);
        let destination = Rect::new(0, 0, 192, 128);
        let mut expected = Framebuffer::new(192, 128, Color::BLACK).unwrap();
        let mut actual = expected.clone();

        draw_texture_clipped(
            &mut expected,
            &texture,
            destination,
            TextureFilter::Bilinear,
            clip,
        );
        draw_texture_clipped_with_options(
            &mut actual,
            &texture,
            destination,
            TextureFilter::Bilinear,
            clip,
            RenderOptions::serial(),
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn configured_texture_draw_keeps_parallel_output_deterministic() {
        let texture = Texture::new(
            128,
            128,
            (0..128 * 128)
                .map(|index| {
                    let value = index as u8;
                    Color::rgba(value, value.wrapping_mul(3), value.wrapping_mul(7), 255)
                })
                .collect(),
        )
        .unwrap();
        let destination = Rect::new(0, 0, 512, 256);
        let clip = Rect::new(0, 0, 512, 256);
        let mut expected = Framebuffer::new(512, 256, Color::BLACK).unwrap();
        let mut actual = expected.clone();

        draw_texture_clipped(
            &mut expected,
            &texture,
            destination,
            TextureFilter::Nearest,
            clip,
        );
        draw_texture_clipped_with_options(
            &mut actual,
            &texture,
            destination,
            TextureFilter::Nearest,
            clip,
            RenderOptions::new()
                .with_max_threads(2)
                .with_parallel_threshold(0),
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn atlas_bilinear_matches_regular_texture_draw_over_opaque_destination() {
        let source_pixels = vec![
            Color::WHITE,
            Color::rgba(220, 40, 30, 180),
            Color::rgba(20, 210, 80, 220),
            Color::BLACK,
            Color::rgba(30, 60, 230, 150),
            Color::rgba(240, 180, 20, 200),
            Color::WHITE,
            Color::rgba(120, 40, 200, 90),
            Color::BLACK,
        ];
        let atlas = Texture::new(3, 3, source_pixels.clone()).unwrap();
        let region_pixels = vec![
            source_pixels[4],
            source_pixels[5],
            source_pixels[7],
            source_pixels[8],
        ];
        let region_texture = Texture::new(2, 2, region_pixels).unwrap();
        let destination = Rect::new(1, 1, 7, 6);
        let clip = Rect::new(0, 0, 10, 9);
        let mut atlas_target = Framebuffer::new(10, 9, Color::BLACK).unwrap();
        let mut texture_target = Framebuffer::new(10, 9, Color::BLACK).unwrap();

        draw_texture_region_clipped(
            &mut atlas_target,
            &atlas,
            Rect::new(1, 1, 2, 2),
            destination,
            TextureFilter::Bilinear,
            clip,
        );
        draw_texture(
            &mut texture_target,
            &region_texture,
            destination,
            TextureFilter::Bilinear,
        );

        assert_eq!(atlas_target.pixels(), texture_target.pixels());
    }
}
