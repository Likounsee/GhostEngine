//! Platform-independent input state shared by native window adapters.
//!
//! This module has no dependency on a particular windowing library, allowing
//! applications to select the minifb or winit adapter independently.

use crate::{
    ClipboardAction, ComponentEvents, ComponentKey, ComponentTree, Point, Rect, TextInputEvent,
};
#[cfg(feature = "native-input")]
use std::time::{Duration, Instant};

/// Input sampled once per application frame.
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
    /// Horizontal wheel movement in pixels; positive values scroll toward later columns.
    pub scroll_delta_x: i32,
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
    pub page_up_pressed: bool,
    pub page_down_pressed: bool,
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
#[cfg(feature = "native-input")]
#[derive(Debug, Default)]
pub(crate) struct ClickTracker {
    last_click: Option<(Instant, Point)>,
    click_count: u8,
}

#[cfg(feature = "native-input")]
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
                // Reject distant points before squaring: the difference between
                // opposite i32 extremes does not fit when squared in i64.
                dx.abs() <= 5 && dy.abs() <= 5 && dx * dx + dy * dy <= 25
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

/// Routes one sampled native input frame into the retained component tree.
///
/// Unicode text and IME composition are supplied through text_events by
/// the host text-input service. The minifb fallback remains a limited physical
/// key mapping; it is not an AZERTY/QWERTY layout implementation.
impl ComponentTree {
    pub fn handle_window_input(&mut self, bounds: Rect, input: &InputState) -> ComponentEvents {
        self.handle_window_input_with_text_font(bounds, input, None)
    }

    /// Handles window input and maps pointer events to text caret positions.
    ///
    /// Supply the same font and pixel size used to draw text inputs. The basic
    /// handle_window_input method remains available for applications without text metrics.
    pub fn handle_window_input_with_text_font(
        &mut self,
        bounds: Rect,
        input: &InputState,
        text_font: Option<(&crate::FontFace, f32)>,
    ) -> ComponentEvents {
        let mut shortcut_events = ComponentEvents::default();
        for key in &input.shortcut_keys {
            if let Some(next) = self.activate_context_menu_shortcut(
                *key,
                input.control_down || input.super_down,
                input.alt_down,
                input.shift_down,
            ) {
                merge_component_events(&mut shortcut_events, next);
                break;
            }
        }
        if input.right_mouse_pressed {
            if let Some(pointer) = input.mouse_position {
                self.open_context_menu_for_at(bounds, pointer);
            }
        }
        let mut events = shortcut_events;
        let pointer_events = if let Some((font, pixel_size)) = text_font {
            self.update_pointer_with_text_input(
                bounds,
                input.mouse_position,
                input.left_mouse_pressed,
                input.left_mouse_released,
                font,
                pixel_size,
                input
                    .left_click_count
                    .max(u8::from(input.left_mouse_pressed)),
            )
        } else {
            self.update_pointer(
                bounds,
                input.mouse_position,
                input.left_mouse_pressed,
                input.left_mouse_released,
            )
        };
        merge_component_events(&mut events, pointer_events);
        if input.scroll_delta_y != 0 {
            if let Some(pointer) = input.mouse_position {
                self.scroll_list_view_at(pointer, input.scroll_delta_y);
            }
        }

        if input.escape_down && self.context_menu_is_open() {
            let next = self.handle_key(bounds, ComponentKey::Escape, input.shift_down);
            merge_component_events(&mut events, next);
        }
        if input.tab_pressed {
            let next = self.handle_key(bounds, ComponentKey::Tab, input.shift_down);
            merge_component_events(&mut events, next);
        }
        if input.enter_pressed {
            let next = self.handle_key(bounds, ComponentKey::Enter, input.shift_down);
            merge_component_events(&mut events, next);
        }
        if input.space_pressed {
            let next = self.handle_key(bounds, ComponentKey::Space, input.shift_down);
            merge_component_events(&mut events, next);
        }
        if input.left_pressed {
            self.handle_key(
                bounds,
                if input.control_down || input.super_down {
                    ComponentKey::WordLeft
                } else {
                    ComponentKey::ArrowLeft
                },
                input.shift_down,
            );
        }
        if input.right_pressed {
            self.handle_key(
                bounds,
                if input.control_down || input.super_down {
                    ComponentKey::WordRight
                } else {
                    ComponentKey::ArrowRight
                },
                input.shift_down,
            );
        }
        if input.home_pressed {
            self.handle_key(bounds, ComponentKey::Home, input.shift_down);
        }
        if input.end_pressed {
            self.handle_key(bounds, ComponentKey::End, input.shift_down);
        }
        if input.page_up_pressed {
            self.handle_key(bounds, ComponentKey::PageUp, input.shift_down);
        }
        if input.page_down_pressed {
            self.handle_key(bounds, ComponentKey::PageDown, input.shift_down);
        }
        if input.backspace_pressed {
            self.handle_key(
                bounds,
                if input.control_down || input.super_down {
                    ComponentKey::DeleteWordBackward
                } else {
                    ComponentKey::Backspace
                },
                input.shift_down,
            );
        }
        if input.delete_pressed {
            self.handle_key(
                bounds,
                if input.control_down || input.super_down {
                    ComponentKey::DeleteWordForward
                } else {
                    ComponentKey::Delete
                },
                input.shift_down,
            );
        }
        if input.select_all_pressed {
            if let Some(id) = self.focused_text_input() {
                self.select_all_text(id);
            }
        }
        if let Some(id) = self.focused_text_input() {
            for event in &input.text_events {
                self.handle_text_input_event(id, event.clone());
            }
            if input.copy_pressed {
                events.clipboard_action = self.copy_selected_text(id).map(ClipboardAction::Copy);
            }
            if input.cut_pressed {
                events.clipboard_action = self.cut_selected_text(id).map(ClipboardAction::Copy);
            }
            if input.paste_pressed {
                if let Some(text) = input.clipboard_text.as_deref() {
                    self.insert_text(id, text);
                } else {
                    events.clipboard_action = Some(ClipboardAction::Paste);
                }
            }
        }

        // Prefer committed Unicode text from the host text service. The legacy
        // character list is only a fallback for adapters that cannot emit text events.
        if input.text_events.is_empty()
            && !input.control_down
            && !input.super_down
            && !input.alt_down
        {
            for character in &input.pressed_characters {
                if *character == ' ' && self.focused_text_input().is_none() {
                    let next = self.handle_key(bounds, ComponentKey::Space, input.shift_down);
                    merge_component_events(&mut events, next);
                } else if self.focused_text_input().is_some() {
                    self.insert_text_focused(&character.to_string());
                }
            }
        }
        events
    }
}

fn merge_component_events(target: &mut ComponentEvents, next: ComponentEvents) {
    if next.activated.is_some() {
        target.activated = next.activated;
    }
    if next.focus_changed.is_some() {
        target.focus_changed = next.focus_changed;
    }
    if next.context_menu_action.is_some() {
        target.context_menu_action = next.context_menu_action;
    }
    if next.context_menu_target.is_some() {
        target.context_menu_target = next.context_menu_target;
    }
}

#[cfg(all(test, feature = "native-input"))]
mod tests {
    #[test]
    fn click_tracker_handles_extreme_coordinates_without_overflow() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let mut tracker = super::ClickTracker::default();
        assert_eq!(
            tracker.record_at(Some(crate::Point::new(i32::MIN, i32::MIN)), start),
            1
        );
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(i32::MAX, i32::MAX)),
                start + Duration::from_millis(100)
            ),
            1
        );
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(i32::MAX - 1, i32::MAX - 1)),
                start + Duration::from_millis(200)
            ),
            2
        );
    }

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

    #[test]
    fn click_tracker_accepts_exact_time_and_distance_boundaries() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let mut tracker = super::ClickTracker::default();
        assert_eq!(tracker.record_at(Some(crate::Point::new(0, 0)), start), 1);
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(3, 4)),
                start + Duration::from_millis(500)
            ),
            2
        );
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(8, 4)),
                start + Duration::from_millis(600)
            ),
            3
        );
    }

    #[test]
    fn click_tracker_rejects_just_outside_time_and_distance_boundaries() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let mut tracker = super::ClickTracker::default();
        assert_eq!(tracker.record_at(Some(crate::Point::new(0, 0)), start), 1);
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(3, 4)),
                start + Duration::from_millis(501)
            ),
            1
        );
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(7, 8)),
                start + Duration::from_millis(600)
            ),
            1
        );
    }

    #[test]
    fn click_tracker_saturates_triple_click_count_and_restarts_after_gap() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let mut tracker = super::ClickTracker::default();
        for (index, expected) in [(0, 1), (1, 2), (2, 3), (3, 3), (4, 3)] {
            assert_eq!(
                tracker.record_at(
                    Some(crate::Point::new(40, 40)),
                    start + Duration::from_millis(index * 100)
                ),
                expected
            );
        }
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(40, 40)),
                start + Duration::from_millis(1_001)
            ),
            1
        );
    }

    #[test]
    fn missing_pointer_position_resets_click_sequence() {
        use std::time::{Duration, Instant};

        let start = Instant::now();
        let mut tracker = super::ClickTracker::default();
        assert_eq!(tracker.record_at(Some(crate::Point::new(12, 18)), start), 1);
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(12, 18)),
                start + Duration::from_millis(100)
            ),
            2
        );
        assert_eq!(
            tracker.record_at(None, start + Duration::from_millis(150)),
            1
        );
        assert_eq!(
            tracker.record_at(
                Some(crate::Point::new(12, 18)),
                start + Duration::from_millis(200)
            ),
            1
        );
    }

}
