//! Simple deterministic CPU texture atlas packing.
//!
//! The builder uses shelf packing with configurable edge-extruded padding. It is
//! intended for UI sprites and small immutable assets; callers retain stable
//! region IDs and can rebuild an atlas when their asset set changes.

use crate::{
    draw_texture_region_clipped, Color, Framebuffer, Rect, Texture, TextureError, TextureFilter,
};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureAtlasError {
    ZeroDimension,
    DuplicateId,
    AtlasFull,
    AllocationFailed,
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

/// Draws one atlas entry without sampling pixels from adjacent entries.
pub fn draw_atlas_region_clipped(
    target: &mut Framebuffer,
    atlas: &Texture,
    region: AtlasRegion,
    destination: Rect,
    filter: TextureFilter,
    clip: Rect,
) {
    draw_texture_region_clipped(target, atlas, region.rect, destination, filter, clip);
}

/// A built texture atlas with stable ID-based region lookup.
#[derive(Debug, Clone)]
pub struct TextureAtlas {
    texture: Texture,
    regions: Vec<AtlasRegion>,
    regions_by_id: HashMap<u64, AtlasRegion>,
}

impl TextureAtlas {
    /// Returns the packed atlas texture.
    #[must_use]
    pub const fn texture(&self) -> &Texture {
        &self.texture
    }

    /// Returns all packed regions in packing order.
    #[must_use]
    pub fn regions(&self) -> &[AtlasRegion] {
        &self.regions
    }

    /// Finds a sprite region by its stable application-defined ID.
    #[must_use]
    pub fn region(&self, id: u64) -> Option<AtlasRegion> {
        self.regions_by_id.get(&id).copied()
    }

    /// Draws a sprite by ID. Returns false when the ID is not present.
    pub fn draw_region(
        &self,
        target: &mut Framebuffer,
        id: u64,
        destination: Rect,
        filter: TextureFilter,
        clip: Rect,
    ) -> bool {
        let Some(region) = self.region(id) else {
            return false;
        };
        draw_atlas_region_clipped(target, &self.texture, region, destination, filter, clip);
        true
    }
}

/// Packs textures into one immutable texture using row-based shelf packing.
#[derive(Debug)]
pub struct TextureAtlasBuilder {
    width: u32,
    height: u32,
    padding: u32,
    items: Vec<AtlasItem>,
    ids: HashSet<u64>,
}

impl TextureAtlasBuilder {
    #[must_use]
    pub fn new(width: u32, height: u32, padding: u32) -> Self {
        Self {
            width,
            height,
            padding,
            items: Vec::new(),
            ids: HashSet::new(),
        }
    }

    pub fn add(&mut self, id: u64, texture: &Texture) -> Result<(), TextureAtlasError> {
        if !self.ids.insert(id) {
            return Err(TextureAtlasError::DuplicateId);
        }
        self.items.push(AtlasItem {
            id,
            texture: texture.clone(),
        });
        Ok(())
    }

    pub fn build(self) -> Result<(Texture, Vec<AtlasRegion>), TextureAtlasError> {
        self.build_with_order(false)
    }

    /// Builds an atlas object with direct ID-based sprite lookup.
    pub fn build_atlas(self) -> Result<TextureAtlas, TextureAtlasError> {
        let (texture, regions) = self.build()?;
        let regions_by_id = regions
            .iter()
            .copied()
            .map(|region| (region.id, region))
            .collect();
        Ok(TextureAtlas {
            texture,
            regions,
            regions_by_id,
        })
    }

    /// Optimized packing variant returning an ID-addressable atlas object.
    pub fn build_atlas_optimized(self) -> Result<TextureAtlas, TextureAtlasError> {
        let (texture, regions) = self.build_optimized()?;
        let regions_by_id = regions
            .iter()
            .copied()
            .map(|region| (region.id, region))
            .collect();
        Ok(TextureAtlas {
            texture,
            regions,
            regions_by_id,
        })
    }

    /// Packs larger sprites first to reduce shelf fragmentation.
    ///
    /// Items are sorted by descending height, then width, with ascending IDs as
    /// a deterministic tie-breaker. Returned regions remain keyed by their IDs.
    pub fn build_optimized(self) -> Result<(Texture, Vec<AtlasRegion>), TextureAtlasError> {
        self.build_with_order(true)
    }

    fn build_with_order(
        mut self,
        optimized: bool,
    ) -> Result<(Texture, Vec<AtlasRegion>), TextureAtlasError> {
        if self.width == 0 || self.height == 0 {
            return Err(TextureAtlasError::ZeroDimension);
        }
        let count = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or(TextureAtlasError::AtlasFull)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| TextureAtlasError::AllocationFailed)?;
        pixels.resize(count, Color::TRANSPARENT);
        if optimized {
            self.items.sort_by(|left, right| {
                right
                    .texture
                    .height()
                    .cmp(&left.texture.height())
                    .then_with(|| right.texture.width().cmp(&left.texture.width()))
                    .then_with(|| left.id.cmp(&right.id))
            });
        }
        let mut regions = Vec::with_capacity(self.items.len());
        let mut x = 0_u32;
        let mut y = 0_u32;
        let mut shelf_height = 0_u32;

        for item in self.items {
            let padding = self.padding.saturating_mul(2);
            let padded_width = item
                .texture
                .width()
                .checked_add(padding)
                .ok_or(TextureAtlasError::AtlasFull)?;
            let padded_height = item
                .texture
                .height()
                .checked_add(padding)
                .ok_or(TextureAtlasError::AtlasFull)?;
            if padded_width > self.width || padded_height > self.height {
                return Err(TextureAtlasError::AtlasFull);
            }
            if x.checked_add(padded_width)
                .map_or(true, |right| right > self.width)
            {
                x = 0;
                y = y
                    .checked_add(shelf_height)
                    .ok_or(TextureAtlasError::AtlasFull)?;
                shelf_height = 0;
            }
            if y.checked_add(padded_height)
                .map_or(true, |bottom| bottom > self.height)
            {
                return Err(TextureAtlasError::AtlasFull);
            }

            let content_x = x + self.padding;
            let content_y = y + self.padding;
            // Extrude edge texels into the padding so bilinear sampling near
            // sprite boundaries cannot blend with transparent neighboring space.
            let source_width = item.texture.width() as usize;
            let padding = self.padding as usize;
            for padded_y in 0..padded_height as usize {
                let source_y =
                    (padded_y.saturating_sub(padding)).min(item.texture.height() as usize - 1);
                let source_start = source_y * source_width;
                let source_row = &item.texture.pixels()[source_start..source_start + source_width];

                let destination_start = (y as usize + padded_y) * self.width as usize + x as usize;
                let destination_row =
                    &mut pixels[destination_start..destination_start + padded_width as usize];

                destination_row[..padding].fill(source_row[0]);
                destination_row[padding..padding + source_width].copy_from_slice(source_row);
                destination_row[padding + source_width..].fill(source_row[source_width - 1]);
            }
            regions.push(AtlasRegion {
                id: item.id,
                rect: Rect::new(
                    content_x,
                    content_y,
                    item.texture.width(),
                    item.texture.height(),
                ),
            });
            x += padded_width;
            shelf_height = shelf_height.max(padded_height);
        }

        let texture =
            Texture::new(self.width, self.height, pixels).map_err(TextureAtlasError::Texture)?;
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
        assert_eq!(
            atlas.get_pixel(regions[0].rect.x, regions[0].rect.y),
            Some(Color::RED)
        );
        assert_eq!(
            atlas.get_pixel(regions[0].rect.x + 1, regions[0].rect.y),
            Some(Color::GREEN)
        );
        assert_eq!(
            atlas.get_pixel(regions[1].rect.x, regions[1].rect.y + 1),
            Some(Color::WHITE)
        );
        assert_eq!(
            atlas.get_pixel(regions[0].rect.x - 1, regions[0].rect.y),
            Some(Color::RED)
        );
    }

    #[test]
    fn optimized_packing_avoids_shelf_fragmentation() {
        let items = [
            (1, Texture::new(6, 6, vec![Color::RED; 36]).unwrap()),
            (2, Texture::new(4, 4, vec![Color::GREEN; 16]).unwrap()),
            (3, Texture::new(4, 6, vec![Color::BLUE; 24]).unwrap()),
            (4, Texture::new(6, 4, vec![Color::WHITE; 24]).unwrap()),
        ];

        let mut insertion_order = TextureAtlasBuilder::new(10, 10, 0);
        for (id, texture) in &items {
            insertion_order.add(*id, texture).unwrap();
        }
        assert_eq!(
            insertion_order.build().unwrap_err(),
            TextureAtlasError::AtlasFull
        );

        let mut optimized = TextureAtlasBuilder::new(10, 10, 0);
        for (id, texture) in &items {
            optimized.add(*id, texture).unwrap();
        }
        let (atlas, regions) = optimized.build_optimized().unwrap();

        assert_eq!(regions.len(), 4);
        for (id, texture) in &items {
            let region = regions.iter().find(|region| region.id == *id).unwrap();
            assert_eq!(region.rect.width, texture.width());
            assert_eq!(region.rect.height, texture.height());
            assert_eq!(
                atlas.get_pixel(region.rect.x, region.rect.y),
                texture.get_pixel(0, 0)
            );
        }
    }

    #[test]
    fn atlas_region_draw_uses_region_bounds_and_clip() {
        let red = Texture::new(2, 2, vec![Color::RED; 4]).unwrap();
        let blue = Texture::new(2, 2, vec![Color::BLUE; 4]).unwrap();
        let mut builder = TextureAtlasBuilder::new(8, 4, 1);
        builder.add(10, &red).unwrap();
        builder.add(20, &blue).unwrap();
        let (atlas, regions) = builder.build().unwrap();
        let blue_region = *regions.iter().find(|region| region.id == 20).unwrap();
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();

        draw_atlas_region_clipped(
            &mut target,
            &atlas,
            blue_region,
            Rect::new(0, 0, 4, 4),
            TextureFilter::Bilinear,
            Rect::new(1, 1, 2, 2),
        );

        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(target.get_pixel(1, 1), Some(Color::BLUE));
        assert_eq!(target.get_pixel(2, 2), Some(Color::BLUE));
        assert_eq!(target.get_pixel(3, 3), Some(Color::BLACK));
    }

    #[test]
    fn atlas_region_draw_rejects_invalid_region() {
        let atlas = Texture::new(2, 2, vec![Color::RED; 4]).unwrap();
        let mut target = Framebuffer::new(2, 2, Color::BLACK).unwrap();
        draw_atlas_region_clipped(
            &mut target,
            &atlas,
            AtlasRegion {
                id: 7,
                rect: Rect::new(1, 1, 2, 2),
            },
            Rect::new(0, 0, 2, 2),
            TextureFilter::Nearest,
            Rect::new(0, 0, 2, 2),
        );
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn atlas_object_resolves_and_draws_regions_by_stable_id() {
        let red = Texture::new(2, 2, vec![Color::RED; 4]).unwrap();
        let blue = Texture::new(2, 2, vec![Color::BLUE; 4]).unwrap();
        let mut builder = TextureAtlasBuilder::new(8, 4, 1);
        builder.add(41, &red).unwrap();
        builder.add(99, &blue).unwrap();
        let atlas = builder.build_atlas_optimized().unwrap();

        assert_eq!(atlas.regions().len(), 2);
        assert_eq!(atlas.region(99).unwrap().rect.width, 2);
        assert!(atlas.region(1234).is_none());

        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        assert!(atlas.draw_region(
            &mut target,
            99,
            Rect::new(0, 0, 4, 4),
            TextureFilter::Bilinear,
            Rect::new(0, 0, 4, 4),
        ));
        assert_eq!(target.get_pixel(1, 1), Some(Color::BLUE));
        assert!(!atlas.draw_region(
            &mut target,
            1234,
            Rect::new(0, 0, 4, 4),
            TextureFilter::Nearest,
            Rect::new(0, 0, 4, 4),
        ));
    }

    #[test]
    fn rejects_duplicate_ids_and_overflow() {
        let texture = Texture::new(1, 1, vec![Color::RED]).unwrap();
        let mut builder = TextureAtlasBuilder::new(2, 2, 0);
        builder.add(1, &texture).unwrap();
        assert_eq!(
            builder.add(1, &texture),
            Err(TextureAtlasError::DuplicateId)
        );

        let mut full = TextureAtlasBuilder::new(1, 1, 0);
        full.add(2, &Texture::new(2, 1, vec![Color::RED; 2]).unwrap())
            .unwrap();
        assert_eq!(full.build().unwrap_err(), TextureAtlasError::AtlasFull);
    }
}
