//! Simple deterministic CPU texture atlas packing.
//!
//! The builder uses shelf packing with configurable transparent padding. It is
//! intended for UI sprites and small immutable assets; callers retain stable
//! region IDs and can rebuild an atlas when their asset set changes.

use crate::{Color, Rect, Texture, TextureError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureAtlasError {
    ZeroDimension,
    DuplicateId,
    AtlasFull,
    Texture(TextureError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasRegion {
    pub id: u64,
    pub rect: Rect,
}

#[derive(Debug)]
struct AtlasItem {
    id: u64,
    texture: Texture,
}

/// Packs textures into one immutable texture using row-based shelf packing.
#[derive(Debug)]
pub struct TextureAtlasBuilder {
    width: u32,
    height: u32,
    padding: u32,
    items: Vec<AtlasItem>,
}

impl TextureAtlasBuilder {
    #[must_use]
    pub fn new(width: u32, height: u32, padding: u32) -> Self {
        Self { width, height, padding, items: Vec::new() }
    }

    pub fn add(&mut self, id: u64, texture: &Texture) -> Result<(), TextureAtlasError> {
        if self.items.iter().any(|item| item.id == id) {
            return Err(TextureAtlasError::DuplicateId);
        }
        self.items.push(AtlasItem { id, texture: texture.clone() });
        Ok(())
    }

    pub fn build(self) -> Result<(Texture, Vec<AtlasRegion>), TextureAtlasError> {
        if self.width == 0 || self.height == 0 {
            return Err(TextureAtlasError::ZeroDimension);
        }
        let count = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or(TextureAtlasError::AtlasFull)?;
        let mut pixels = vec![Color::TRANSPARENT; count];
        let mut regions = Vec::with_capacity(self.items.len());
        let mut x = 0_u32;
        let mut y = 0_u32;
        let mut shelf_height = 0_u32;

        for item in self.items {
            let padded_width = item.texture.width().checked_add(self.padding.saturating_mul(2))
                .ok_or(TextureAtlasError::AtlasFull)?;
            let padded_height = item.texture.height().checked_add(self.padding.saturating_mul(2))
                .ok_or(TextureAtlasError::AtlasFull)?;
            if padded_width > self.width || padded_height > self.height {
                return Err(TextureAtlasError::AtlasFull);
            }
            if x.checked_add(padded_width).is_none_or(|right| right > self.width) {
                x = 0;
                y = y.checked_add(shelf_height).ok_or(TextureAtlasError::AtlasFull)?;
                shelf_height = 0;
            }
            if y.checked_add(padded_height).is_none_or(|bottom| bottom > self.height) {
                return Err(TextureAtlasError::AtlasFull);
            }

            let content_x = x + self.padding;
            let content_y = y + self.padding;
            for row in 0..item.texture.height() as usize {
                let source_start = row * item.texture.width() as usize;
                let destination_start =
                    (content_y as usize + row) * self.width as usize + content_x as usize;
                let source_end = source_start + item.texture.width() as usize;
                let destination_end = destination_start + item.texture.width() as usize;
                pixels[destination_start..destination_end]
                    .copy_from_slice(&item.texture.pixels()[source_start..source_end]);
            }
            regions.push(AtlasRegion {
                id: item.id,
                rect: Rect::new(content_x, content_y, item.texture.width(), item.texture.height()),
            });
            x += padded_width;
            shelf_height = shelf_height.max(padded_height);
        }

        let texture = Texture::new(self.width, self.height, pixels)
            .map_err(TextureAtlasError::Texture)?;
        Ok((texture, regions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_textures_and_preserves_region_pixels() {
        let red = Texture::new(2, 1, vec![Color::RED, Color::GREEN]).unwrap();
        let blue = Texture::new(1, 2, vec![Color::BLUE, Color::WHITE]).unwrap();
        let mut builder = TextureAtlasBuilder::new(8, 4, 1);
        builder.add(10, &red).unwrap();
        builder.add(20, &blue).unwrap();
        let (atlas, regions) = builder.build().unwrap();
        assert_eq!(regions[0].id, 10);
        assert_eq!(atlas.get_pixel(regions[0].rect.x, regions[0].rect.y), Some(Color::RED));
        assert_eq!(atlas.get_pixel(regions[0].rect.x + 1, regions[0].rect.y), Some(Color::GREEN));
        assert_eq!(atlas.get_pixel(regions[1].rect.x, regions[1].rect.y + 1), Some(Color::WHITE));
    }

    #[test]
    fn rejects_duplicate_ids_and_overflow() {
        let texture = Texture::new(1, 1, vec![Color::RED]).unwrap();
        let mut builder = TextureAtlasBuilder::new(2, 2, 0);
        builder.add(1, &texture).unwrap();
        assert_eq!(builder.add(1, &texture), Err(TextureAtlasError::DuplicateId));
        let mut full = TextureAtlasBuilder::new(1, 1, 0);
        full.add(2, &Texture::new(2, 1, vec![Color::RED; 2]).unwrap()).unwrap();
        assert_eq!(full.build().unwrap_err(), TextureAtlasError::AtlasFull);
    }
}
