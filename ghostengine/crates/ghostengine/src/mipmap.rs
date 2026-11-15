//! Precomputed texture mip levels and trilinear sampling.

use crate::{Color, Texture, TextureError};

/// A texture and its progressively downsampled levels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MipmapChain {
    levels: Vec<Texture>,
}

impl MipmapChain {
    pub fn new(base: &Texture) -> Result<Self, TextureError> {
        let mut levels = vec![base.clone()];
        while levels
            .last()
            .is_some_and(|level| level.width() > 1 || level.height() > 1)
        {
            let previous = levels.last().expect("base mip level always exists");
            let width = previous.width().div_ceil(2);
            let height = previous.height().div_ceil(2);
            let mut pixels = Vec::with_capacity(width as usize * height as usize);

            for y in 0..height {
                for x in 0..width {
                    let x0 = x * 2;
                    let y0 = y * 2;
                    let x1 = (x0 + 1).min(previous.width() - 1);
                    let y1 = (y0 + 1).min(previous.height() - 1);
                    pixels.push(average_premultiplied([
                        previous.get_pixel(x0, y0).unwrap(),
                        previous.get_pixel(x1, y0).unwrap(),
                        previous.get_pixel(x0, y1).unwrap(),
                        previous.get_pixel(x1, y1).unwrap(),
                    ]));
                }
            }
            levels.push(Texture::new(width, height, pixels)?);
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
        if self.levels.is_empty() {
            return Color::TRANSPARENT;
        }
        let lod = if lod.is_finite() {
            lod.clamp(0.0, (self.levels.len() - 1) as f32)
        } else {
            0.0
        };
        let lower = lod.floor() as usize;
        let upper = (lower + 1).min(self.levels.len() - 1);
        let fraction = lod - lower as f32;
        let lower_scale = (1_u32 << lower.min(31)) as f32;
        let a = sample_bilinear(&self.levels[lower], x / lower_scale, y / lower_scale);
        if lower == upper || fraction <= 0.0 {
            return a;
        }

        let upper_scale = (1_u32 << upper.min(31)) as f32;
        let b = sample_bilinear(&self.levels[upper], x / upper_scale, y / upper_scale);
        interpolate_premultiplied(a, b, fraction)
    }
}

fn sample_bilinear(texture: &Texture, x: f32, y: f32) -> Color {
    let x = x.clamp(0.0, texture.width() as f32 - 1.0);
    let y = y.clamp(0.0, texture.height() as f32 - 1.0);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(texture.width() - 1);
    let y1 = (y0 + 1).min(texture.height() - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let top = interpolate_premultiplied(
        texture.get_pixel(x0, y0).unwrap(),
        texture.get_pixel(x1, y0).unwrap(),
        tx,
    );
    let bottom = interpolate_premultiplied(
        texture.get_pixel(x0, y1).unwrap(),
        texture.get_pixel(x1, y1).unwrap(),
        tx,
    );
    interpolate_premultiplied(top, bottom, ty)
}

fn average_premultiplied(colors: [Color; 4]) -> Color {
    let alpha = colors.iter().map(|color| f32::from(color.a)).sum::<f32>() * 0.25;
    if alpha <= f32::EPSILON {
        return Color::TRANSPARENT;
    }

    let channel = |select: fn(Color) -> u8| {
        let premultiplied = colors
            .iter()
            .map(|color| f32::from(select(*color)) * f32::from(color.a))
            .sum::<f32>()
            * 0.25;
        (premultiplied / alpha).round().clamp(0.0, 255.0) as u8
    };
    Color::rgba(
        channel(|color| color.r),
        channel(|color| color.g),
        channel(|color| color.b),
        alpha.round().clamp(0.0, 255.0) as u8,
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
        ((f32::from(channel_a) * alpha_a * (1.0 - t) + f32::from(channel_b) * alpha_b * t)
            / alpha)
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
    fn transparent_texels_do_not_bleed_rgb_into_mip_levels() {
        let base = Texture::new(2, 1, vec![Color::RED, Color::TRANSPARENT]).unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        assert_eq!(chain.levels()[1].get_pixel(0, 0), Some(Color::RED));
        assert_eq!(chain.levels()[1].get_pixel(0, 0).unwrap().a, 128);
    }

    #[test]
    fn invalid_lod_is_safely_clamped() {
        let base = Texture::new(1, 1, vec![Color::BLUE]).unwrap();
        let chain = MipmapChain::new(&base).unwrap();
        assert_eq!(chain.sample_trilinear(0.0, 0.0, f32::NAN), Color::BLUE);
    }
}
