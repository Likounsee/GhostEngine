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
        Ok(Self {
            framebuffer,
            damage,
            rendered_frames: 0,
        })
    }

    #[must_use]
    pub const fn framebuffer(&self) -> &Framebuffer {
        &self.framebuffer
    }

    pub fn framebuffer_mut(&mut self) -> &mut Framebuffer {
        &mut self.framebuffer
    }

    #[must_use]
    pub fn damage(&self) -> &DamageRegion {
        &self.damage
    }

    #[must_use]
    pub const fn rendered_frames(&self) -> u64 {
        self.rendered_frames
    }

    /// Marks a changed rectangle for recomputation on the next render call.
    pub fn mark_dirty(&mut self, rectangle: Rect) {
        self.damage.add(rectangle);
    }

    /// Invalidates every pixel, for example after a theme or global style change.
    pub fn mark_dirty_all(&mut self) {
        self.damage.add(Rect::new(
            0,
            0,
            self.framebuffer.width(),
            self.framebuffer.height(),
        ));
    }

    /// Resizes the backing framebuffer and invalidates its complete contents.
    pub fn resize(
        &mut self,
        width: u32,
        height: u32,
        clear: Color,
    ) -> Result<(), FramebufferError> {
        self.framebuffer.resize_and_clear(width, height, clear)?;
        self.damage.reset(width, height);
        self.mark_dirty_all();
        Ok(())
    }

    /// Composites the retained pixels at a new position without rerendering.
    ///
    /// Call render_if_dirty first when content has been invalidated. Moving a
    /// cached page or UI panel only changes this destination offset.
    /// The cached framebuffer is sampled directly and is never copied.
    pub fn composite_onto(
        &self,
        target: &mut Framebuffer,
        x: i32,
        y: i32,
        opacity: u8,
        clip: Rect,
    ) {
        if opacity == 0 {
            return;
        }
        let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
            return;
        };
        let left = i64::from(x).max(i64::from(clip.x));
        let top = i64::from(y).max(i64::from(clip.y));
        let right = (i64::from(x) + i64::from(self.framebuffer.width()))
            .min(i64::from(clip.x) + i64::from(clip.width));
        let bottom = (i64::from(y) + i64::from(self.framebuffer.height()))
            .min(i64::from(clip.y) + i64::from(clip.height));
        if left >= right || top >= bottom {
            return;
        }

        let copy_width = (right - left) as usize;
        let source_width = self.framebuffer.width() as usize;
        let target_width = target.width() as usize;
        let source_x = (left - i64::from(x)) as usize;
        let target_x = left as usize;

        for destination_y in top..bottom {
            let source_y = (destination_y - i64::from(y)) as usize;
            let target_y = destination_y as usize;
            let source_start = source_y * source_width + source_x;
            let target_start = target_y * target_width + target_x;
            let source_row = &self.framebuffer.pixels()[source_start..source_start + copy_width];
            let target_row = &mut target.pixels_mut()[target_start..target_start + copy_width];

            if opacity == 255 {
                if source_row.iter().all(|source| source.a == 255) {
                    target_row.copy_from_slice(source_row);
                    continue;
                }
                for (background, source) in target_row.iter_mut().zip(source_row.iter().copied()) {
                    if source.a == 255 {
                        *background = source;
                    } else if source.a > 0 {
                        *background = source.blend_over(*background);
                    }
                }
            } else {
                for (background, mut source) in
                    target_row.iter_mut().zip(source_row.iter().copied())
                {
                    if source.a == 0 {
                        continue;
                    }
                    source.a = ((u16::from(source.a) * u16::from(opacity) + 127) / 255) as u8;
                    if source.a > 0 {
                        *background = source.blend_over(*background);
                    }
                }
            }
        }
    }

    /// Runs the renderer only when damage exists, then retains the pixels.
    ///
    /// The callback must redraw all pixels covered by the damage region and
    /// preserve pixels outside those rectangles.
    pub fn render_if_dirty<F>(&mut self, render: F) -> bool
    where
        F: FnOnce(&mut Framebuffer, &DamageRegion),
    {
        if self.damage.is_empty() {
            return false;
        }
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
            for rect in damage.rectangles() {
                target.fill_rect(*rect, Color::RED);
            }
        }));
        assert_eq!(cache.framebuffer().get_pixel(2, 3), Some(Color::RED));
        assert_eq!(cache.framebuffer().get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn moving_cached_content_does_not_trigger_a_rerender() {
        let mut cache = CachedLayer::new(2, 1, Color::RED).unwrap();
        cache.render_if_dirty(|target, _| target.clear(Color::RED));
        let mut target = Framebuffer::new(6, 2, Color::BLACK).unwrap();
        cache.composite_onto(&mut target, 0, 0, 255, Rect::new(0, 0, 6, 2));
        cache.composite_onto(&mut target, 3, 0, 255, Rect::new(0, 0, 6, 2));
        assert_eq!(cache.rendered_frames(), 1);
        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(3, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 0), Some(Color::BLACK));
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

    #[test]
    fn repeated_damage_render_and_resize_cycles_keep_cache_state_consistent() {
        let mut cache = CachedLayer::new(4, 4, Color::BLACK).unwrap();

        for iteration in 0..128_u32 {
            let x = iteration % 4;
            let y = (iteration / 4) % 4;
            let color = Color::rgba(
                iteration as u8,
                iteration.wrapping_mul(3) as u8,
                iteration.wrapping_mul(7) as u8,
                255,
            );

            cache.mark_dirty(Rect::new(x, y, 1, 1));
            assert!(cache.render_if_dirty(|target, damage| {
                for rect in damage.rectangles() {
                    target.fill_rect(*rect, color);
                }
            }));
            assert!(cache.damage().is_empty());
            assert_eq!(cache.framebuffer().get_pixel(x, y), Some(color));

            if iteration % 16 == 15 {
                let width = 4 + (iteration / 16) % 5;
                let height = 4 + (iteration / 16) % 3;
                cache.resize(width, height, Color::BLUE).unwrap();
                assert_eq!(cache.framebuffer().width(), width);
                assert_eq!(cache.framebuffer().height(), height);
                assert_eq!(cache.damage().area(), u64::from(width) * u64::from(height));
                assert!(cache.render_if_dirty(|_, _| {}));
                assert!(cache.damage().is_empty());
            }
        }

        assert_eq!(cache.rendered_frames(), 136);
    }

    #[test]
    fn compositing_extreme_offscreen_positions_is_a_noop() {
        let cache = CachedLayer::new(2, 2, Color::RED).unwrap();
        let mut target = Framebuffer::new(3, 3, Color::BLUE).unwrap();
        let original = target.pixels().to_vec();

        cache.composite_onto(&mut target, i32::MIN, i32::MIN, 255, Rect::new(0, 0, 3, 3));
        cache.composite_onto(&mut target, i32::MAX, i32::MAX, 255, Rect::new(0, 0, 3, 3));

        assert_eq!(target.pixels(), original);
    }

    #[test]
    fn compositing_matches_source_over_for_alpha_and_layer_opacity() {
        let mut cache = CachedLayer::new(1, 1, Color::TRANSPARENT).unwrap();
        let backgrounds = [
            Color::TRANSPARENT,
            Color::rgba(17, 83, 149, 91),
            Color::rgb(31, 127, 223),
        ];
        let opacities = [0, 1, 63, 127, 128, 192, 254, 255];

        for source_alpha in 0..=u8::MAX {
            let source = Color::rgba(231, 79, 143, source_alpha);
            cache.framebuffer_mut().set_pixel(0, 0, source);
            for opacity in opacities {
                for background in backgrounds {
                    let mut target = Framebuffer::new(1, 1, background).unwrap();
                    cache.composite_onto(&mut target, 0, 0, opacity, Rect::new(0, 0, 1, 1));

                    let effective_alpha =
                        ((u16::from(source_alpha) * u16::from(opacity) + 127) / 255) as u8;
                    let expected = Color {
                        a: effective_alpha,
                        ..source
                    }
                    .blend_over(background);
                    assert_eq!(
                        target.get_pixel(0, 0),
                        Some(expected),
                        "source alpha={source_alpha}, opacity={opacity}, background={background:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn compositing_clips_negative_offsets_and_preserves_uncovered_pixels() {
        let mut cache = CachedLayer::new(3, 2, Color::TRANSPARENT).unwrap();
        cache.framebuffer_mut().pixels_mut().copy_from_slice(&[
            Color::RED,
            Color::GREEN,
            Color::BLUE,
            Color::YELLOW,
            Color::CYAN,
            Color::MAGENTA,
        ]);
        let mut target = Framebuffer::new(3, 2, Color::BLACK).unwrap();

        cache.composite_onto(&mut target, -1, 0, 255, Rect::new(0, 0, 2, 2));

        assert_eq!(
            target.pixels(),
            &[
                Color::GREEN,
                Color::BLUE,
                Color::BLACK,
                Color::CYAN,
                Color::MAGENTA,
                Color::BLACK
            ]
        );
    }

    #[test]
    fn failed_resize_preserves_framebuffer_and_pending_damage() {
        let mut cache = CachedLayer::new(3, 2, Color::BLACK).unwrap();
        cache.render_if_dirty(|_, _| {});
        cache.mark_dirty(Rect::new(1, 0, 1, 1));
        let original_framebuffer = cache.framebuffer().clone();
        let original_damage = cache.damage().clone();
        let original_frames = cache.rendered_frames();

        assert_eq!(
            cache.resize(0, 4, Color::BLUE),
            Err(FramebufferError::ZeroDimension)
        );
        assert_eq!(cache.framebuffer(), &original_framebuffer);
        assert_eq!(cache.damage(), &original_damage);
        assert_eq!(cache.rendered_frames(), original_frames);
    }

    #[test]
    fn renderer_panic_keeps_damage_pending_for_a_retry() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let mut cache = CachedLayer::new(4, 3, Color::BLACK).unwrap();
        let original_damage = cache.damage().clone();

        let result = catch_unwind(AssertUnwindSafe(|| {
            cache.render_if_dirty(|_, _| panic!("simulated renderer failure"));
        }));

        assert!(result.is_err());
        assert_eq!(cache.damage(), &original_damage);
        assert_eq!(cache.rendered_frames(), 0);

        assert!(cache.render_if_dirty(|target, damage| {
            for rect in damage.rectangles() {
                target.fill_rect(*rect, Color::GREEN);
            }
        }));
        assert!(cache.damage().is_empty());
        assert_eq!(cache.rendered_frames(), 1);
        assert_eq!(cache.framebuffer().get_pixel(3, 2), Some(Color::GREEN));
    }

    #[test]
    fn direct_framebuffer_mutation_requires_explicit_invalidation() {
        let mut cache = CachedLayer::new(2, 2, Color::BLACK).unwrap();
        cache.render_if_dirty(|_, _| {});
        cache.framebuffer_mut().set_pixel(1, 1, Color::RED);

        assert!(!cache.render_if_dirty(|_, _| panic!("clean cache must not render")));
        cache.mark_dirty(Rect::new(1, 1, 1, 1));
        assert!(cache.render_if_dirty(|target, _| {
            target.set_pixel(1, 1, Color::BLUE);
        }));
        assert_eq!(cache.framebuffer().get_pixel(1, 1), Some(Color::BLUE));
    }

}
