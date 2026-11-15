//! Explicitly invalidated framebuffer cache for retained-mode rendering.

use crate::{Color, DamageRegion, Framebuffer, FramebufferError, Rect};

/// A reusable framebuffer whose pixels are recomputed only in marked regions.
///
/// The cache never guesses whether scene data changed: callers must mark the
/// affected bounds dirty after changing content. Rendering receives the
/// accumulated damage rectangles and can redraw only those areas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedLayer {
    framebuffer: Framebuffer,
    damage: DamageRegion,
    rendered_frames: u64,
}

impl CachedLayer {
    /// Creates a cache and marks the complete framebuffer dirty for first render.
    pub fn new(width: u32, height: u32, clear: Color) -> Result<Self, FramebufferError> {
        let framebuffer = Framebuffer::new(width, height, clear)?;
        let mut damage = DamageRegion::new(width, height);
        damage.add(Rect::new(0, 0, width, height));
        Ok(Self { framebuffer, damage, rendered_frames: 0 })
    }

    #[must_use]
    pub const fn framebuffer(&self) -> &Framebuffer { &self.framebuffer }

    pub fn framebuffer_mut(&mut self) -> &mut Framebuffer { &mut self.framebuffer }

    #[must_use]
    pub fn damage(&self) -> &DamageRegion { &self.damage }

    #[must_use]
    pub const fn rendered_frames(&self) -> u64 { self.rendered_frames }

    /// Marks a changed rectangle for recomputation on the next render call.
    pub fn mark_dirty(&mut self, rectangle: Rect) { self.damage.add(rectangle); }

    /// Invalidates every pixel, for example after a theme or global style change.
    pub fn mark_dirty_all(&mut self) {
        self.damage.add(Rect::new(0, 0, self.framebuffer.width(), self.framebuffer.height()));
    }

    /// Resizes the backing framebuffer and invalidates its complete contents.
    pub fn resize(&mut self, width: u32, height: u32, clear: Color) -> Result<(), FramebufferError> {
        self.framebuffer.resize_and_clear(width, height, clear)?;
        self.damage.reset(width, height);
        self.mark_dirty_all();
        Ok(())
    }

    /// Runs the renderer only when damage exists, then retains the pixels.
    ///
    /// The callback must redraw all pixels covered by the damage region and
    /// preserve pixels outside those rectangles.
    pub fn render_if_dirty<F>(&mut self, render: F) -> bool
    where
        F: FnOnce(&mut Framebuffer, &DamageRegion),
    {
        if self.damage.is_empty() { return false; }
        render(&mut self.framebuffer, &self.damage);
        self.damage.clear();
        self.rendered_frames = self.rendered_frames.saturating_add(1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_render_when_cached_pixels_are_still_valid() {
        let mut cache = CachedLayer::new(8, 8, Color::BLACK).unwrap();
        let mut calls = 0;
        assert!(cache.render_if_dirty(|target, damage| {
            calls += 1;
            assert_eq!(damage.area(), 64);
            target.clear(Color::WHITE);
        }));
        assert!(!cache.render_if_dirty(|_, _| calls += 1));
        assert_eq!(calls, 1);
        assert_eq!(cache.rendered_frames(), 1);
        assert_eq!(cache.framebuffer().get_pixel(0, 0), Some(Color::WHITE));
    }

    #[test]
    fn redraws_only_marked_damage_and_preserves_other_pixels() {
        let mut cache = CachedLayer::new(8, 8, Color::BLACK).unwrap();
        cache.render_if_dirty(|target, _| target.clear(Color::BLACK));
        cache.mark_dirty(Rect::new(2, 3, 2, 2));
        assert!(cache.render_if_dirty(|target, damage| {
            assert_eq!(damage.area(), 4);
            for rect in damage.rectangles() { target.fill_rect(*rect, Color::RED); }
        }));
        assert_eq!(cache.framebuffer().get_pixel(2, 3), Some(Color::RED));
        assert_eq!(cache.framebuffer().get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn resize_invalidates_the_new_framebuffer() {
        let mut cache = CachedLayer::new(2, 2, Color::BLACK).unwrap();
        cache.render_if_dirty(|_, _| {});
        cache.resize(4, 3, Color::BLUE).unwrap();
        assert_eq!(cache.damage().area(), 12);
        assert!(cache.render_if_dirty(|_, _| {}));
        assert_eq!(cache.framebuffer().width(), 4);
        assert_eq!(cache.framebuffer().height(), 3);
    }
}
