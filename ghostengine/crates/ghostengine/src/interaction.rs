//! Backend-independent pointer interaction and focus dispatch for UI widgets.
//!
//! Widgets are identified by their stable index in the caller-provided bounds
//! and enabled arrays. The caller owns widget data and drawing; this module
//! owns hover, pointer capture, click activation, and keyboard focus transitions.

use crate::{FocusManager, Point, Rect};

/// Events produced by one UI interaction update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InteractionEvents {
    /// Widget activated by a press followed by a release inside the same widget.
    pub clicked: Option<usize>,
    /// Widget that received focus during this update.
    pub focus_changed: Option<usize>,
}

/// Stateful interaction model shared by buttons, checkboxes, menus, and other controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiInteraction {
    hovered: Option<usize>,
    captured: Option<usize>,
    focus: FocusManager,
}

impl UiInteraction {
    /// Creates an interaction model with no hovered, captured, or focused widget.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            hovered: None,
            captured: None,
            focus: FocusManager::new(),
        }
    }

    #[must_use]
    pub const fn hovered(self) -> Option<usize> {
        self.hovered
    }

    #[must_use]
    pub const fn captured(self) -> Option<usize> {
        self.captured
    }

    #[must_use]
    pub const fn focused(self) -> Option<usize> {
        self.focus.focused()
    }

    #[must_use]
    pub const fn is_hovered(self, index: usize) -> bool {
        matches!(self.hovered, Some(current) if current == index)
    }

    /// A captured widget remains pressed even when the pointer leaves its bounds.
    #[must_use]
    pub const fn is_pressed(self, index: usize) -> bool {
        matches!(self.captured, Some(current) if current == index)
    }

    #[must_use]
    pub const fn is_focused(self, index: usize) -> bool {
        matches!(self.focus.focused(), Some(current) if current == index)
    }

    /// Updates pointer interaction for one frame.
    ///
    /// Bounds and enabled flags must describe the same stable widget ordering.
    /// When rectangles overlap, the last enabled widget is considered topmost.
    /// A click is emitted only when a captured widget is released inside itself.
    pub fn update(
        &mut self,
        pointer: Option<Point>,
        pressed: bool,
        released: bool,
        enabled: &[bool],
        bounds: &[Rect],
    ) -> InteractionEvents {
        let previous_focus = self.focus.focused();
        self.focus.reconcile(enabled);
        if self.captured.is_some_and(|index| !is_enabled(index, enabled) || index >= bounds.len()) {
            self.captured = None;
        }

        self.hovered = pointer.and_then(|point| hit_test(point, enabled, bounds));

        if pressed {
            self.captured = self.hovered;
            if let Some(index) = self.hovered {
                self.focus.set_focus(Some(index), enabled);
            }
        }

        let mut clicked = None;
        if released {
            if let (Some(index), Some(point)) = (self.captured, pointer) {
                if is_enabled(index, enabled)
                    && bounds.get(index).is_some_and(|rect| rect.contains(point))
                {
                    clicked = Some(index);
                }
            }
            self.captured = None;
        }

        InteractionEvents {
            clicked,
            focus_changed: (self.focus.focused() != previous_focus)
                .then_some(self.focus.focused())
                .flatten(),
        }
    }

    /// Moves keyboard focus through enabled widgets in tab order.
    pub fn focus_next(&mut self, enabled: &[bool], reverse: bool) -> InteractionEvents {
        let previous = self.focus.focused();
        self.focus.focus_next(enabled, reverse);
        InteractionEvents {
            clicked: None,
            focus_changed: (self.focus.focused() != previous)
                .then_some(self.focus.focused())
                .flatten(),
        }
    }

    /// Clears pointer capture and keyboard focus, for example when the window loses focus.
    pub fn clear(&mut self) {
        self.hovered = None;
        self.captured = None;
        self.focus.clear();
    }
}

fn is_enabled(index: usize, enabled: &[bool]) -> bool {
    enabled.get(index).copied().unwrap_or(false)
}

fn hit_test(pointer: Point, enabled: &[bool], bounds: &[Rect]) -> Option<usize> {
    bounds
        .iter()
        .enumerate()
        .rev()
        .find(|(index, rect)| is_enabled(*index, enabled) && rect.contains(pointer))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::UiInteraction;
    use crate::{Point, Rect};

    fn controls() -> ([bool; 3], [Rect; 3]) {
        (
            [true, true, false],
            [
                Rect::new(0, 0, 20, 20),
                Rect::new(10, 10, 20, 20),
                Rect::new(40, 0, 20, 20),
            ],
        )
    }

    #[test]
    fn pointer_hover_uses_topmost_enabled_widget() {
        let (enabled, bounds) = controls();
        let mut ui = UiInteraction::new();
        ui.update(Some(Point::new(12, 12)), false, false, &enabled, &bounds);
        assert_eq!(ui.hovered(), Some(1));
        assert!(!ui.is_hovered(0));
        assert!(ui.is_hovered(1));
    }

    #[test]
    fn release_inside_captured_widget_emits_one_click_and_focuses_it() {
        let (enabled, bounds) = controls();
        let mut ui = UiInteraction::new();
        ui.update(Some(Point::new(2, 2)), true, false, &enabled, &bounds);
        assert!(ui.is_pressed(0));
        let events = ui.update(Some(Point::new(3, 3)), false, true, &enabled, &bounds);
        assert_eq!(events.clicked, Some(0));
        assert_eq!(events.focus_changed, Some(0));
        assert!(!ui.is_pressed(0));
        assert_eq!(
            ui.update(Some(Point::new(3, 3)), false, false, &enabled, &bounds).clicked,
            None
        );
    }

    #[test]
    fn capture_survives_pointer_exit_but_release_outside_does_not_click() {
        let (enabled, bounds) = controls();
        let mut ui = UiInteraction::new();
        ui.update(Some(Point::new(2, 2)), true, false, &enabled, &bounds);
        ui.update(Some(Point::new(30, 30)), false, false, &enabled, &bounds);
        assert_eq!(ui.captured(), Some(0));
        assert!(!ui.is_hovered(0));
        assert!(ui.is_pressed(0));
        let events = ui.update(Some(Point::new(30, 30)), false, true, &enabled, &bounds);
        assert_eq!(events.clicked, None);
        assert_eq!(ui.captured(), None);
    }

    #[test]
    fn disabled_widgets_are_not_hovered_captured_or_focused() {
        let (enabled, bounds) = controls();
        let mut ui = UiInteraction::new();
        ui.update(Some(Point::new(42, 2)), true, false, &enabled, &bounds);
        assert_eq!(ui.hovered(), None);
        assert_eq!(ui.captured(), None);
        assert_eq!(ui.focused(), None);
    }

    #[test]
    fn disabling_a_captured_widget_cancels_capture() {
        let (_, bounds) = controls();
        let mut ui = UiInteraction::new();
        ui.update(Some(Point::new(2, 2)), true, false, &[true, true, false], &bounds);
        ui.update(Some(Point::new(2, 2)), false, false, &[false, true, false], &bounds);
        assert_eq!(ui.captured(), None);
        assert_eq!(ui.focused(), Some(1));
    }

    #[test]
    fn disabling_focused_widget_reports_reconciled_focus() {
        let (_, bounds) = controls();
        let mut ui = UiInteraction::new();
        ui.update(Some(Point::new(2, 2)), true, false, &[true, true, false], &bounds);
        let events = ui.update(
            Some(Point::new(30, 30)),
            false,
            false,
            &[false, true, false],
            &bounds,
        );
        assert_eq!(events.focus_changed, Some(1));
        assert_eq!(ui.focused(), Some(1));
    }

    #[test]
    fn tab_navigation_skips_disabled_controls_and_wraps() {
        let mut ui = UiInteraction::new();
        assert_eq!(ui.focus_next(&[true, false, true], false).focus_changed, Some(0));
        assert_eq!(ui.focus_next(&[true, false, true], false).focus_changed, Some(2));
        assert_eq!(ui.focus_next(&[true, false, true], false).focus_changed, Some(0));
        assert_eq!(ui.focus_next(&[true, false, true], true).focus_changed, Some(2));
    }

    #[test]
    fn clear_releases_capture_hover_and_focus() {
        let (enabled, bounds) = controls();
        let mut ui = UiInteraction::new();
        ui.update(Some(Point::new(2, 2)), true, false, &enabled, &bounds);
        ui.clear();
        assert_eq!(ui.hovered(), None);
        assert_eq!(ui.captured(), None);
        assert_eq!(ui.focused(), None);
    }
}
