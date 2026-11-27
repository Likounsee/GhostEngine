//! Keyboard focus traversal for application-level UI controls.
//!
//! The focus manager stores only a control index. Applications keep ownership
//! of widgets and input events, and provide the enabled state in stable tab order.

/// Tracks the currently focused control in an application's tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusManager {
    focused: Option<usize>,
}

impl FocusManager {
    /// Creates a focus manager with no focused control.
    #[must_use]
    pub const fn new() -> Self {
        Self { focused: None }
    }

    /// Returns the focused control index, if one is currently selected.
    #[must_use]
    pub const fn focused(self) -> Option<usize> {
        self.focused
    }

    /// Focuses an enabled control. Invalid or disabled indices clear focus.
    pub fn set_focus(&mut self, index: Option<usize>, enabled: &[bool]) {
        self.focused = index.filter(|index| enabled.get(*index).copied().unwrap_or(false));
    }

    /// Removes keyboard focus from every control.
    pub fn clear(&mut self) {
        self.focused = None;
    }

    /// Keeps focus valid after controls are removed, reordered, or disabled.
    ///
    /// If the focused control is no longer available, focus moves to the first
    /// enabled control. An empty or fully disabled interface has no focus.
    pub fn reconcile(&mut self, enabled: &[bool]) {
        if self
            .focused
            .is_some_and(|index| !enabled.get(index).copied().unwrap_or(false))
        {
            self.focused = enabled.iter().position(|is_enabled| *is_enabled);
        }
    }

    /// Moves focus one step through the tab order, wrapping at either end.
    ///
    /// Disabled controls are skipped. With no current focus, forward traversal
    /// starts at the first enabled control and reverse traversal at the last.
    pub fn focus_next(&mut self, enabled: &[bool], reverse: bool) {
        let count = enabled.len();
        if count == 0 || !enabled.iter().any(|is_enabled| *is_enabled) {
            self.focused = None;
            return;
        }

        let start = match self.focused.filter(|index| *index < count) {
            Some(index) if reverse => (index + count - 1) % count,
            Some(index) => (index + 1) % count,
            None if reverse => count - 1,
            None => 0,
        };

        for step in 0..count {
            let index = if reverse {
                (start + count - (step % count)) % count
            } else {
                (start + step) % count
            };
            if enabled[index] {
                self.focused = Some(index);
                return;
            }
        }
    }

    /// Focuses the first enabled control in tab order.
    pub fn focus_first(&mut self, enabled: &[bool]) {
        self.focused = enabled.iter().position(|is_enabled| *is_enabled);
    }

    /// Focuses the last enabled control in tab order.
    pub fn focus_last(&mut self, enabled: &[bool]) {
        self.focused = enabled.iter().rposition(|is_enabled| *is_enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::FocusManager;

    #[test]
    fn traversal_skips_disabled_controls_and_wraps() {
        let enabled = [true, false, true, false, true];
        let mut focus = FocusManager::new();

        focus.focus_next(&enabled, false);
        assert_eq!(focus.focused(), Some(0));
        focus.focus_next(&enabled, false);
        assert_eq!(focus.focused(), Some(2));
        focus.focus_next(&enabled, false);
        assert_eq!(focus.focused(), Some(4));
        focus.focus_next(&enabled, false);
        assert_eq!(focus.focused(), Some(0));

        focus.focus_next(&enabled, true);
        assert_eq!(focus.focused(), Some(4));
        focus.focus_next(&enabled, true);
        assert_eq!(focus.focused(), Some(2));
    }

    #[test]
    fn reverse_traversal_without_focus_starts_at_last_enabled_control() {
        let mut focus = FocusManager::new();
        focus.focus_next(&[true, false, true], true);
        assert_eq!(focus.focused(), Some(2));
    }

    #[test]
    fn setting_and_reconciling_focus_rejects_disabled_or_removed_controls() {
        let mut focus = FocusManager::new();
        let enabled = [false, true, true];
        focus.set_focus(Some(0), &enabled);
        assert_eq!(focus.focused(), None);
        focus.set_focus(Some(2), &enabled);
        assert_eq!(focus.focused(), Some(2));

        focus.reconcile(&[false, true, false]);
        assert_eq!(focus.focused(), Some(1));
        focus.reconcile(&[false, false]);
        assert_eq!(focus.focused(), None);
    }

    #[test]
    fn empty_and_fully_disabled_interfaces_never_gain_focus() {
        let mut focus = FocusManager::new();
        focus.focus_first(&[]);
        assert_eq!(focus.focused(), None);
        focus.focus_next(&[false, false], false);
        assert_eq!(focus.focused(), None);
        focus.focus_last(&[false, false]);
        assert_eq!(focus.focused(), None);
    }

    #[test]
    fn clearing_focus_allows_traversal_to_restart_at_first_control() {
        let mut focus = FocusManager::new();
        focus.set_focus(Some(2), &[true, true, true]);
        focus.clear();
        focus.focus_next(&[true, true, true], false);
        assert_eq!(focus.focused(), Some(0));
    }

    #[test]
    fn traversal_matches_reference_for_every_small_enabled_mask() {
        for count in 0..=8 {
            for mask in 0..(1usize << count) {
                let enabled: Vec<bool> = (0..count).map(|index| mask & (1 << index) != 0).collect();

                for start in 0..=count {
                    if start < count && !enabled[start] {
                        continue;
                    }
                    for reverse in [false, true] {
                        let mut focus = FocusManager::new();
                        if start < count {
                            focus.set_focus(Some(start), &enabled);
                        }

                        let expected = if start < count {
                            (1..=count)
                                .map(|step| {
                                    if reverse {
                                        (start + count - (step % count)) % count
                                    } else {
                                        (start + step) % count
                                    }
                                })
                                .find(|index| enabled[*index])
                        } else if reverse {
                            (0..count).rev().find(|index| enabled[*index])
                        } else {
                            (0..count).find(|index| enabled[*index])
                        };

                        focus.focus_next(&enabled, reverse);
                        assert_eq!(
                            focus.focused(),
                            expected,
                            "count={count}, mask={mask:#010b}, start={start}, reverse={reverse}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn reconciliation_preserves_focus_when_enabled_order_changes_around_it() {
        let mut focus = FocusManager::new();
        focus.set_focus(Some(3), &[true, false, true, true, false]);

        focus.reconcile(&[false, true, true, true, true, true]);
        assert_eq!(focus.focused(), Some(3));

        focus.reconcile(&[true, true, false, false, true]);
        assert_eq!(focus.focused(), Some(0));
    }
}
