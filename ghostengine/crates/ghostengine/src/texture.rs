//! CPU-side image storage and scaled texture compositing.

use crate::{Color, Framebuffer, Rect};

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
        let top = interpolate_premultiplied(
            self.get_pixel(x0, y0).unwrap_or(Color::TRANSPARENT),
            self.get_pixel(x1, y0).unwrap_or(Color::TRANSPARENT),
            tx,
        );
        let bottom = interpolate_premultiplied(
            self.get_pixel(x0, y1).unwrap_or(Color::TRANSPARENT),
            self.get_pixel(x1, y1).unwrap_or(Color::TRANSPARENT),
            tx,
        );
        interpolate_premultiplied(top, bottom, ty)
    }
}

fn interpolate_premultiplied(left: Color, right: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    let alpha = f32::from(left.a) + (f32::from(right.a) - f32::from(left.a)) * amount;
    if alpha <= f32::EPSILON {
        return Color::TRANSPARENT;
    }
    let channel = |left_channel: u8, right_channel: u8| {
        let left_value = f32::from(left_channel) * f32::from(left.a) / 255.0;
        let right_value = f32::from(right_channel) * f32::from(right.a) / 255.0;
        ((left_value + (right_value - left_value) * amount) * 255.0 / alpha)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgba(
        channel(left.r, right.r),
        channel(left.g, right.g),
        channel(left.b, right.b),
        alpha.round().clamp(0.0, 255.0) as u8,
    )
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
    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
    let left = destination.x.max(clip.x).min(clip_right);
    let top = destination.y.max(clip.y).min(clip_bottom);
    let right = (u64::from(destination.x) + u64::from(destination.width))
        .min(u64::from(clip_right)) as u32;
    let bottom = (u64::from(destination.y) + u64::from(destination.height))
        .min(u64::from(clip_bottom)) as u32;
    if left >= right || top >= bottom {
        return;
    }

    for y in top..bottom {
        let local_y = y - destination.y;
        let source_y =
            (local_y as f32 + 0.5) * texture.height as f32 / destination.height as f32 - 0.5;
        for x in left..right {
            let local_x = x - destination.x;
            let source_x =
                (local_x as f32 + 0.5) * texture.width as f32 / destination.width as f32 - 0.5;
            let color = match filter {
                TextureFilter::Nearest => texture.sample_nearest(
                    (local_x as f32 + 0.5) * texture.width as f32 / destination.width as f32,
                    (local_y as f32 + 0.5) * texture.height as f32 / destination.height as f32,
                ),
                TextureFilter::Bilinear => texture.sample_bilinear(source_x, source_y),
            };
            if color.a == 0 {
                continue;
            }
            if let Some(background) = target.get_pixel(x, y) {
                target.set_pixel(x, y, color.blend_over(background));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{draw_texture, draw_texture_clipped, Texture, TextureError, TextureFilter};
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
