//! Deterministic stack layout for application UI.
//!
//! Layout is independent of the window system. Applications describe a vertical
//! or horizontal stack and receive pixel rectangles for each child.

use crate::Rect;

/// Direction in which stack children are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutDirection {
    #[default]
    Vertical,
    Horizontal,
}

/// Insets applied inside a layout's outer bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Insets {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Insets {
    #[must_use]
    pub const fn all(value: u32) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    #[must_use]
    pub const fn symmetric(horizontal: u32, vertical: u32) -> Self {
        Self {
            left: horizontal,
            top: vertical,
            right: horizontal,
            bottom: vertical,
        }
    }
}

/// Requested size for one child along the stack's main axis.
/// A missing size distributes remaining space equally between flexible children.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutItem {
    pub main_size: Option<u32>,
}

impl LayoutItem {
    #[must_use]
    pub const fn fixed(main_size: u32) -> Self {
        Self {
            main_size: Some(main_size),
        }
    }

    #[must_use]
    pub const fn flexible() -> Self {
        Self { main_size: None }
    }
}

/// Configuration for a simple vertical or horizontal stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StackLayout {
    pub direction: LayoutDirection,
    pub padding: Insets,
    pub gap: u32,
}

impl StackLayout {
    #[must_use]
    pub const fn new(direction: LayoutDirection) -> Self {
        Self {
            direction,
            padding: Insets {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            gap: 0,
        }
    }

    #[must_use]
    pub const fn with_padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    #[must_use]
    pub const fn with_gap(mut self, gap: u32) -> Self {
        self.gap = gap;
        self
    }

    /// Computes child rectangles in input order.
    ///
    /// Fixed children keep their requested main-axis size (clipped to available
    /// space); flexible children share remaining space. Division remainder is
    /// assigned one pixel at a time to earlier flexible children.
    #[must_use]
    pub fn layout(self, bounds: Rect, items: &[LayoutItem]) -> Vec<Rect> {
        if items.is_empty() {
            return Vec::new();
        }

        let (origin_x, origin_y, main_available, cross_available) = match self.direction {
            LayoutDirection::Vertical => (
                bounds.x.saturating_add(self.padding.left),
                bounds.y.saturating_add(self.padding.top),
                bounds
                    .height
                    .saturating_sub(self.padding.top.saturating_add(self.padding.bottom)),
                bounds
                    .width
                    .saturating_sub(self.padding.left.saturating_add(self.padding.right)),
            ),
            LayoutDirection::Horizontal => (
                bounds.x.saturating_add(self.padding.left),
                bounds.y.saturating_add(self.padding.top),
                bounds
                    .width
                    .saturating_sub(self.padding.left.saturating_add(self.padding.right)),
                bounds
                    .height
                    .saturating_sub(self.padding.top.saturating_add(self.padding.bottom)),
            ),
        };

        let gaps = self
            .gap
            .saturating_mul(items.len().saturating_sub(1) as u32);
        let available = main_available.saturating_sub(gaps);
        let fixed_total = items.iter().fold(0_u32, |sum, item| {
            sum.saturating_add(item.main_size.unwrap_or(0))
        });
        let flexible_count = items.iter().filter(|item| item.main_size.is_none()).count() as u32;
        let remaining = available.saturating_sub(fixed_total);
        let flexible_base = remaining.checked_div(flexible_count).unwrap_or_default();
        let mut remainder = remaining.checked_rem(flexible_count).unwrap_or_default();
        let mut consumed = 0_u32;
        let mut result = Vec::with_capacity(items.len());

        for (index, item) in items.iter().enumerate() {
            let requested = match item.main_size {
                Some(size) => size,
                None => {
                    let extra = u32::from(remainder > 0);
                    remainder = remainder.saturating_sub(extra);
                    flexible_base.saturating_add(extra)
                }
            };
            let gaps_after = self
                .gap
                .saturating_mul(items.len().saturating_sub(index + 1) as u32);
            let maximum = main_available.saturating_sub(consumed.saturating_add(gaps_after));
            let main_size = requested.min(maximum);
            let (x, y, width, height) = match self.direction {
                LayoutDirection::Vertical => (
                    origin_x,
                    origin_y.saturating_add(consumed),
                    cross_available,
                    main_size,
                ),
                LayoutDirection::Horizontal => (
                    origin_x.saturating_add(consumed),
                    origin_y,
                    main_size,
                    cross_available,
                ),
            };
            result.push(Rect::new(x, y, width, height));
            consumed = consumed.saturating_add(main_size);
            if index + 1 < items.len() {
                consumed = consumed.saturating_add(self.gap);
            }
        }
        result
    }
}

/// A scrollable viewport over content larger than its visible bounds.
///
/// The viewport rectangle is also the clip rectangle used by rendering calls.
/// Content coordinates start at the viewport's top-left before the scroll offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollViewport {
    bounds: Rect,
    direction: LayoutDirection,
    content_extent: u32,
    offset: u32,
}

impl ScrollViewport {
    #[must_use]
    pub const fn new(bounds: Rect, direction: LayoutDirection, content_extent: u32) -> Self {
        Self {
            bounds,
            direction,
            content_extent,
            offset: 0,
        }
    }

    #[must_use]
    pub const fn bounds(self) -> Rect {
        self.bounds
    }

    /// Returns the viewport bounds to pass to clipped drawing operations.
    #[must_use]
    pub const fn clip(self) -> Rect {
        self.bounds
    }

    #[must_use]
    pub const fn offset(self) -> u32 {
        self.offset
    }

    #[must_use]
    pub fn max_offset(self) -> u32 {
        self.content_extent.saturating_sub(self.viewport_extent())
    }

    #[must_use]
    pub fn is_scrollable(self) -> bool {
        self.max_offset() > 0
    }

    /// Returns the content origin in signed framebuffer coordinates.
    #[must_use]
    pub fn content_origin(self) -> (i64, i64) {
        let x = i64::from(self.bounds.x);
        let y = i64::from(self.bounds.y);
        match self.direction {
            LayoutDirection::Vertical => (x, y - i64::from(self.offset)),
            LayoutDirection::Horizontal => (x - i64::from(self.offset), y),
        }
    }

    pub fn set_offset(&mut self, offset: u32) {
        self.offset = offset.min(self.max_offset());
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let next = i64::from(self.offset) + i64::from(delta);
        self.offset = next.clamp(0, i64::from(self.max_offset())) as u32;
    }

    /// Updates content length and clamps the current offset if content shrinks.
    pub fn set_content_extent(&mut self, content_extent: u32) {
        self.content_extent = content_extent;
        self.set_offset(self.offset);
    }

    /// Updates viewport bounds and clamps the current offset to the new size.
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.set_offset(self.offset);
    }

    fn viewport_extent(self) -> u32 {
        match self.direction {
            LayoutDirection::Vertical => self.bounds.height,
            LayoutDirection::Horizontal => self.bounds.width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Insets, LayoutDirection, LayoutItem, ScrollViewport, StackLayout};
    use crate::Rect;

    #[test]
    fn vertical_stack_applies_padding_and_gaps() {
        let layout = StackLayout::new(LayoutDirection::Vertical)
            .with_padding(Insets::symmetric(4, 6))
            .with_gap(3);
        let result = layout.layout(
            Rect::new(10, 20, 100, 80),
            &[LayoutItem::fixed(10), LayoutItem::fixed(20)],
        );
        assert_eq!(result[0], Rect::new(14, 26, 92, 10));
        assert_eq!(result[1], Rect::new(14, 39, 92, 20));
    }

    #[test]
    fn flexible_children_share_remaining_space() {
        let layout = StackLayout::new(LayoutDirection::Vertical).with_gap(2);
        let result = layout.layout(
            Rect::new(0, 0, 20, 17),
            &[
                LayoutItem::fixed(3),
                LayoutItem::flexible(),
                LayoutItem::flexible(),
            ],
        );
        assert_eq!(result[0], Rect::new(0, 0, 20, 3));
        assert_eq!(result[1], Rect::new(0, 5, 20, 5));
        assert_eq!(result[2], Rect::new(0, 12, 20, 5));
    }

    #[test]
    fn horizontal_stack_uses_width_as_main_axis() {
        let layout = StackLayout::new(LayoutDirection::Horizontal)
            .with_padding(Insets::all(2))
            .with_gap(1);
        let result = layout.layout(
            Rect::new(5, 7, 30, 12),
            &[LayoutItem::fixed(8), LayoutItem::flexible()],
        );
        assert_eq!(result[0], Rect::new(7, 9, 8, 8));
        assert_eq!(result[1], Rect::new(16, 9, 17, 8));
    }

    #[test]
    fn fixed_children_are_clipped_when_the_stack_overflows() {
        let layout = StackLayout::new(LayoutDirection::Vertical).with_gap(1);
        let result = layout.layout(
            Rect::new(0, 0, 10, 5),
            &[LayoutItem::fixed(4), LayoutItem::fixed(4)],
        );
        assert_eq!(result[0].height, 4);
        assert_eq!(result[1].height, 0);
    }

    #[test]
    fn vertical_scroll_viewport_clamps_offsets_and_exposes_clip() {
        let bounds = Rect::new(4, 6, 20, 10);
        let mut viewport = ScrollViewport::new(bounds, LayoutDirection::Vertical, 40);
        assert_eq!(viewport.clip(), bounds);
        assert!(viewport.is_scrollable());
        assert_eq!(viewport.max_offset(), 30);
        viewport.set_offset(50);
        assert_eq!(viewport.offset(), 30);
        assert_eq!(viewport.content_origin(), (4, -24));
        viewport.scroll_by(-8);
        assert_eq!(viewport.offset(), 22);
        viewport.scroll_by(-100);
        assert_eq!(viewport.offset(), 0);
    }

    #[test]
    fn horizontal_scroll_viewport_uses_width_and_signed_origin() {
        let bounds = Rect::new(12, 5, 8, 14);
        let mut viewport = ScrollViewport::new(bounds, LayoutDirection::Horizontal, 30);
        viewport.scroll_by(7);
        assert_eq!(viewport.max_offset(), 22);
        assert_eq!(viewport.content_origin(), (5, 5));
        assert_eq!(viewport.clip(), bounds);
    }

    #[test]
    fn scroll_viewport_clamps_when_content_or_bounds_shrink() {
        let mut viewport =
            ScrollViewport::new(Rect::new(0, 0, 10, 10), LayoutDirection::Vertical, 50);
        viewport.set_offset(40);
        viewport.set_content_extent(18);
        assert_eq!(viewport.offset(), 8);
        viewport.set_bounds(Rect::new(0, 0, 10, 20));
        assert_eq!(viewport.offset(), 0);
        assert!(!viewport.is_scrollable());
    }

    #[test]
    fn scroll_viewport_handles_empty_content_and_extreme_deltas() {
        let mut viewport =
            ScrollViewport::new(Rect::new(0, 0, 10, 10), LayoutDirection::Vertical, 0);
        assert!(!viewport.is_scrollable());
        viewport.scroll_by(i32::MAX);
        assert_eq!(viewport.offset(), 0);
        viewport.set_content_extent(u32::MAX);
        viewport.scroll_by(i32::MAX);
        assert_eq!(viewport.offset(), i32::MAX as u32);
        viewport.scroll_by(i32::MIN);
        assert_eq!(viewport.offset(), 0);
    }

    #[test]
    fn empty_stack_and_zero_bounds_are_safe() {
        let layout = StackLayout::new(LayoutDirection::Vertical);
        assert!(layout.layout(Rect::new(0, 0, 0, 0), &[]).is_empty());
        let result = layout.layout(
            Rect::new(0, 0, 0, 0),
            &[LayoutItem::flexible(), LayoutItem::flexible()],
        );
        assert_eq!(result, vec![Rect::new(0, 0, 0, 0); 2]);
    }
}
