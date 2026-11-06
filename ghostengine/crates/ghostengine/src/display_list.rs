//! Ordered CPU display lists.
//!
//! Applications record drawing commands first, then replay them into a
//! framebuffer. Each command captures the active clip rectangle, so later
//! clip-stack changes cannot affect commands that were already recorded.

use crate::rasterizer::{draw_line, fill_rect_clipped};
use crate::{Color, Framebuffer, Point, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Clear(Color),
    FillRect { rect: Rect, color: Color, clip: Rect },
    StrokeRect { rect: Rect, color: Color, clip: Rect },
    Line { start: Point, end: Point, color: Color, clip: Rect },
}

/// Summary of commands executed by one display-list replay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub commands_submitted: usize,
    pub commands_executed: usize,
    pub commands_skipped: usize,
}

/// A reusable, ordered list of CPU drawing operations.
///
/// The list owns no framebuffer or platform resources. Recording and replay
/// are separate, which makes command generation testable without opening a
/// window and allows applications to rebuild only the lists that changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayList {
    commands: Vec<Command>,
    clips: Vec<Rect>,
}

impl DisplayList {
    /// Creates an empty list with no explicit clip restriction.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
            clips: Vec::new(),
        }
    }

    /// Removes all commands and clip state while retaining allocated capacity.
    pub fn clear(&mut self) {
        self.commands.clear();
        self.clips.clear();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Adds a clip rectangle. Nested clips are intersected with their parent.
    /// An empty intersection remains empty until the matching pop.
    pub fn push_clip(&mut self, clip: Rect) {
        let effective = self.clips.last().map_or(Some(clip), |parent| parent.intersection(clip));
        self.clips.push(effective.unwrap_or_default());
    }

    /// Removes the most recently pushed clip. Returns false for an empty stack.
    pub fn pop_clip(&mut self) -> bool {
        self.clips.pop().is_some()
    }

    /// Records a full-framebuffer clear. Clear deliberately ignores clip state.
    pub fn clear_color(&mut self, color: Color) {
        self.commands.push(Command::Clear(color));
    }

    /// Records a filled rectangle using the clip active at recording time.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::FillRect { rect, color, clip });
        }
    }

    /// Records a one-pixel rectangle outline.
    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::StrokeRect { rect, color, clip });
        }
    }

    /// Records a one-pixel line. Endpoints are clipped during replay.
    pub fn line(&mut self, start: Point, end: Point, color: Color) {
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::Line { start, end, color, clip });
        }
    }

    /// Executes commands in submission order and returns basic replay counters.
    pub fn replay(&self, target: &mut Framebuffer) -> RenderStats {
        let mut stats = RenderStats {
            commands_submitted: self.commands.len(),
            ..RenderStats::default()
        };

        for command in &self.commands {
            let executed = match *command {
                Command::Clear(color) => {
                    target.clear(color);
                    true
                }
                Command::FillRect { rect, color, clip } => {
                    if rect.intersection(clip).is_some() {
                        fill_rect_clipped(target, rect, clip, color);
                        true
                    } else {
                        false
                    }
                }
                Command::StrokeRect { rect, color, clip } => {
                    draw_stroke_rect(target, rect, clip, color);
                    rect.width > 0 && rect.height > 0 && rect.intersection(clip).is_some()
                }
                Command::Line { start, end, color, clip } => {
                    if let Some((start, end)) = clip_line(start, end, clip) {
                        draw_line(target, start, end, color);
                        true
                    } else {
                        false
                    }
                }
            };
            if executed {
                stats.commands_executed += 1;
            } else {
                stats.commands_skipped += 1;
            }
        }
        stats
    }

    fn active_clip(&self) -> Option<Rect> {
        self.clips.last().copied()
    }
}

fn draw_stroke_rect(target: &mut Framebuffer, rect: Rect, clip: Rect, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let top = Rect::new(rect.x, rect.y, rect.width, 1);
    let bottom = Rect::new(
        rect.x,
        rect.y.saturating_add(rect.height - 1),
        rect.width,
        1,
    );
    let left = Rect::new(rect.x, rect.y, 1, rect.height);
    let right = Rect::new(
        rect.x.saturating_add(rect.width - 1),
        rect.y,
        1,
        rect.height,
    );
    fill_rect_clipped(target, top, clip, color);
    if rect.height > 1 {
        fill_rect_clipped(target, bottom, clip, color);
    }
    fill_rect_clipped(target, left, clip, color);
    if rect.width > 1 {
        fill_rect_clipped(target, right, clip, color);
    }
}

fn clip_line(start: Point, end: Point, clip: Rect) -> Option<(Point, Point)> {
    if clip.width == 0 || clip.height == 0 {
        return None;
    }
    let left = f64::from(clip.x);
    let top = f64::from(clip.y);
    let right = (u64::from(clip.x) + u64::from(clip.width) - 1) as f64;
    let bottom = (u64::from(clip.y) + u64::from(clip.height) - 1) as f64;
    let (x0, y0) = (f64::from(start.x), f64::from(start.y));
    let (dx, dy) = (f64::from(end.x) - x0, f64::from(end.y) - y0);
    let p = [-dx, dx, -dy, dy];
    let q = [x0 - left, right - x0, y0 - top, bottom - y0];
    let (mut enter, mut leave) = (0.0_f64, 1.0_f64);

    for index in 0..4 {
        if p[index] == 0.0 {
            if q[index] < 0.0 {
                return None;
            }
            continue;
        }
        let ratio = q[index] / p[index];
        if p[index] < 0.0 {
            enter = enter.max(ratio);
        } else {
            leave = leave.min(ratio);
        }
        if enter > leave {
            return None;
        }
    }

    let point = |amount: f64| {
        Point::new(
            (x0 + amount * dx).round().clamp(left, right) as i32,
            (y0 + amount * dy).round().clamp(top, bottom) as i32,
        )
    };
    Some((point(enter), point(leave)))
}

#[cfg(test)]
mod tests {
    use super::DisplayList;
    use crate::{Color, Framebuffer, Point, Rect};

    #[test]
    fn commands_replay_in_submission_order() {
        let mut list = DisplayList::new();
        list.clear_color(Color::BLACK);
        list.fill_rect(Rect::new(1, 1, 4, 4), Color::RED);
        list.fill_rect(Rect::new(2, 2, 2, 2), Color::BLUE);

        let mut target = Framebuffer::new(8, 8, Color::WHITE).unwrap();
        let stats = list.replay(&mut target);

        assert_eq!(target.get_pixel(1, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 2), Some(Color::BLUE));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(stats.commands_submitted, 3);
        assert_eq!(stats.commands_executed, 3);
        assert_eq!(stats.commands_skipped, 0);
    }

    #[test]
    fn nested_clips_intersect_and_are_captured_per_command() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(1, 1, 6, 6));
        list.fill_rect(Rect::new(0, 0, 8, 8), Color::RED);
        list.push_clip(Rect::new(3, 3, 2, 2));
        list.fill_rect(Rect::new(0, 0, 8, 8), Color::BLUE);
        assert!(list.pop_clip());
        list.fill_rect(Rect::new(0, 0, 8, 8), Color::GREEN);

        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        list.replay(&mut target);

        assert_eq!(target.get_pixel(1, 1), Some(Color::GREEN));
        assert_eq!(target.get_pixel(2, 2), Some(Color::GREEN));
        assert_eq!(target.get_pixel(3, 3), Some(Color::GREEN));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(target.get_pixel(7, 7), Some(Color::BLACK));
    }

    #[test]
    fn empty_nested_clip_skips_drawing() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(1, 1, 2, 2));
        list.push_clip(Rect::new(6, 6, 2, 2));
        list.fill_rect(Rect::new(0, 0, 8, 8), Color::RED);

        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);

        assert_eq!(stats.commands_submitted, 0);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn line_is_clipped_to_active_viewport() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(2, 2, 3, 3));
        list.line(Point::new(0, 3), Point::new(7, 3), Color::WHITE);

        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);

        assert_eq!(target.get_pixel(1, 3), Some(Color::BLACK));
        assert_eq!(target.get_pixel(2, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(4, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(5, 3), Some(Color::BLACK));
        assert_eq!(stats.commands_executed, 1);
    }

    #[test]
    fn rectangle_outline_respects_clip_and_avoids_duplicate_edges() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(3, 0, 2, 5));
        list.stroke_rect(Rect::new(1, 1, 6, 3), Color::RED);

        let mut target = Framebuffer::new(8, 5, Color::BLACK).unwrap();
        list.replay(&mut target);

        assert_eq!(target.get_pixel(3, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(4, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(3, 2), Some(Color::BLACK));
        assert_eq!(target.get_pixel(4, 2), Some(Color::BLACK));
        assert_eq!(target.get_pixel(3, 3), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 1), Some(Color::BLACK));
    }

    #[test]
    fn clear_resets_clip_independently_and_list_can_be_reused() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(1, 1, 2, 2));
        list.clear_color(Color::BLUE);
        list.fill_rect(Rect::new(0, 0, 4, 4), Color::RED);

        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        list.replay(&mut target);
        assert_eq!(target.get_pixel(0, 0), Some(Color.BLUE));
        assert_eq!(target.get_pixel(1, 1), Some(Color.RED));

        list.clear();
        assert!(list.is_empty());
        list.clear_color(Color.GREEN);
        list.replay(&mut target);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color.GREEN));
    }
}
