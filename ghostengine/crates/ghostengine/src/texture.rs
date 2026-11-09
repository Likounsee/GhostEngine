//! CPU-side image storage and scaled texture compositing.

use crate::{Color, Framebuffer, Rect, Transform2D};

/// Errors returned when constructing an in-memory texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureError {
    ZeroDimension,
    SizeOverflow,
    PixelCountMismatch,
}

/// Sampling mode used when a texture is scaled.
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
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Copies the pixels from an existing framebuffer.
    #[must_use]
    pub fn from_framebuffer(framebuffer: &Framebuffer) -> Self {
        Self {
            width: framebuffer.width(),
            height: framebuffer.height(),
            pixels: framebuffer.pixels().to_vec(),
        }
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
        let sx = x.floor().clamp(0.0, self.width as f32 - 1.0) as u32;
        let sy = y.floor().clamp(0.0, self.height as f32 - 1.0) as u32;
        self.get_pixel(sx, sy).unwrap_or(Color::TRANSPARENT)
    }

    fn sample_bilinear(&self, x: f32, y: f32) -> Color {
        let x = x.clamp(0.0, self.width as f32 - 1.0);
        let y = y.clamp(0.0, self.height as f32 - 1.0);
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;

        // Fetch directly from the validated row-major storage. The coordinates
        // have already been clamped, so four Option-based get_pixel calls are
        // unnecessary in this inner loop.
        let width = self.width as usize;
        let top_left = self.pixels[y0 as usize * width + x0 as usize];
        let top_right = self.pixels[y0 as usize * width + x1 as usize];
        let bottom_left = self.pixels[y1 as usize * width + x0 as usize];
        let bottom_right = self.pixels[y1 as usize * width + x1 as usize];

        // Bilinear interpolation is linear, so combine all four premultiplied
        // samples and unpremultiply once instead of doing three intermediate
        // color conversions and roundings.
        let one_minus_x = 1.0 - tx;
        let one_minus_y = 1.0 - ty;
        let weights = [
            one_minus_x * one_minus_y,
            tx * one_minus_y,
            one_minus_x * ty,
            tx * ty,
        ];
        let samples = [top_left, top_right, bottom_left, bottom_right];
        let mut alpha = 0.0_f32;
        let mut red = 0.0_f32;
        let mut green = 0.0_f32;
        let mut blue = 0.0_f32;

        for (sample, weight) in samples.into_iter().zip(weights) {
            let weighted_alpha = f32::from(sample.a) * weight;
            alpha += weighted_alpha;
            red += f32::from(sample.r) * weighted_alpha;
            green += f32::from(sample.g) * weighted_alpha;
            blue += f32::from(sample.b) * weighted_alpha;
        }

        if alpha <= f32::EPSILON {
            return Color::TRANSPARENT;
        }

        let channel = |value: f32| (value / alpha).round().clamp(0.0, 255.0) as u8;
        Color::rgba(
            channel(red),
            channel(green),
            channel(blue),
            alpha.round().clamp(0.0, 255.0) as u8,
        )
    }
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
    for y in first_y..end_y {
        let source_y = (y as f32 + 0.5 - destination.y as f32) * source_y_scale;
        let Some(row) = target.row_mut(y as u32) else {
            continue;
        };
        for x in first_x..end_x {
            let source_x = (x as f32 + 0.5 - destination.x as f32) * source_x_scale;
            let color = match filter {
                TextureFilter::Nearest => texture.sample_nearest(source_x, source_y),
                TextureFilter::Bilinear => texture.sample_bilinear(source_x - 0.5, source_y - 0.5),
            };
            let index = x as usize;
            if color.a == 255 {
                row[index] = color;
            } else if color.a > 0 {
                row[index] = color.blend_over(row[index]);
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
    let Some(inverse) = transform.inverse() else {
        return false;
    };
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
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
            if color.a == 255 {
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
        draw_texture, draw_texture_clipped, draw_texture_transformed_clipped, Texture,
        TextureError, TextureFilter,
    };
    use crate::{Color, Framebuffer, Rect};

    fn two_color_texture() -> Texture {
        Texture::new(2, 1, vec![Color::RED, Color::BLUE]).unwrap()
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
    fn framebuffer_can_become_an_immutable_texture() {
        let mut framebuffer = Framebuffer::new(1, 1, Color::GREEN).unwrap();
        let texture = Texture::from_framebuffer(&framebuffer);
        framebuffer.clear(Color::RED);
        assert_eq!(texture.get_pixel(0, 0), Some(Color::GREEN));
    }
}
