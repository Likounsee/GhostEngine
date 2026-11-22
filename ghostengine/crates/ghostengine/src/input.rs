//! Platform-independent input state shared by native window adapters.
//!
//! This module has no dependency on a particular windowing library, allowing
//! applications to select the minifb or winit adapter independently.

use crate::{Point, TextInputEvent};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InputState {
    pub mouse_position: Option<Point>,
    pub left_mouse_down: bool,
    /// True only on the frame the left mouse button transitions to down.
    pub left_mouse_pressed: bool,
    /// True only on the frame the left mouse button transitions to up.
    pub left_mouse_released: bool,
    /// Number of consecutive clicks reported by the host on the press frame.
    /// Use 2 or more for word selection; 0 is treated as a single click.
    pub left_click_count: u8,
    pub right_mouse_down: bool,
    pub right_mouse_pressed: bool,
    /// Vertical wheel movement in pixels; positive values scroll toward later rows.
    pub scroll_delta_y: i32,
    /// True while either Shift key is held.
    pub shift_down: bool,
    /// True while either Control key is held.
    pub control_down: bool,
    /// True while either Command/Super key is held (macOS/Linux window managers).
    pub super_down: bool,
    /// True while either Alt key is held.
    pub alt_down: bool,
    /// Committed or composing text delivered by a platform text service.
    pub text_events: Vec<TextInputEvent>,
    /// Printable key identities captured for configurable command shortcuts.
    pub shortcut_keys: Vec<char>,
    /// Legacy printable-key fallback for the minifb adapter (limited layout support).
    pub pressed_characters: Vec<char>,
    pub backspace_pressed: bool,
    pub delete_pressed: bool,
    pub left_pressed: bool,
    pub right_pressed: bool,
    pub home_pressed: bool,
    pub end_pressed: bool,
    pub enter_pressed: bool,
    pub space_pressed: bool,
    pub tab_pressed: bool,
    /// True on the key-down edge of Ctrl+A.
    pub select_all_pressed: bool,
    pub copy_pressed: bool,
    pub cut_pressed: bool,
    pub paste_pressed: bool,
    /// Clipboard text supplied by the host after a paste request.
    pub clipboard_text: Option<String>,
    pub escape_down: bool,
}

/// Resolves the edge events for a button sampled once per frame.
#[derive(Debug, Default)]
pub(crate) struct ClickTracker {
    last_click: Option<(Instant, Point)>,
    click_count: u8,
}

impl ClickTracker {
    pub(crate) fn record(&mut self, position: Option<Point>) -> u8 {
        self.record_at(position, Instant::now())
    }

    fn record_at(&mut self, position: Option<Point>, now: Instant) -> u8 {
        let Some(position) = position else {
            self.last_click = None;
            self.click_count = 1;
            return self.click_count;
        };
        let is_double_click = self.last_click.is_some_and(|(last_time, last_position)| {
            now.duration_since(last_time) <= Duration::from_millis(500) && {
                let dx = i64::from(position.x) - i64::from(last_position.x);
                let dy = i64::from(position.y) - i64::from(last_position.y);
                dx * dx + dy * dy <= 25
            }
        });
        self.click_count = if is_double_click {
            self.click_count.saturating_add(1).min(3)
        } else {
            1
        };
        self.last_click = Some((now, position));
        self.click_count
    }
}

#[cfg(test)]
mod tests {
    use super::ClickTracker;

    #[test]
    fn click_tracker_requires_time_and_spatial_proximity() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let mut tracker = super::ClickTracker::default();
        assert_eq!(tracker.record_at(Some(crate::Point::new(10, 10)), start), 1);
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(12, 12)),
                start + Duration::from_millis(300)
            ),
            2
        );
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(40, 40)),
                start + Duration::from_millis(400)
            ),
            1
        );
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(40, 40)),
                start + Duration::from_millis(1_000)
            ),
            1
        );
        assert_eq!(
            tracker.record_at(None, start + Duration::from_millis(1_100)),
            1
        );
    }

}
