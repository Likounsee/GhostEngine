//! Small, backend-independent 2D widgets for building real application interfaces.
//!
//! Widgets only draw into a GhostEngine framebuffer. They do not own a window,
//! event loop, or platform state; applications provide interaction state and
//! use the same CPU renderer as every other scene.

use crate::rasterizer::{draw_line_aa, draw_rect, fill_rect, fill_rect_clipped, fill_rounded_rect};
use crate::{draw_text, draw_text_clipped, Color, FontFace, Framebuffer, Point, Rect, TextMetrics};

/// Visual state supplied by an application's hit-testing/input layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidgetState {
    #[default]
    Normal,
    Hovered,
    Pressed,
    Disabled,
}

/// Resolves a widget's visual state from pointer position and button state.
///
/// Disabled widgets never become hovered or pressed. Pointer coordinates use
/// the same pixel space as the target framebuffer.
#[must_use]
pub fn state_for_pointer(
    rect: Rect,
    pointer: Option<Point>,
    pressed: bool,
    disabled: bool,
) -> WidgetState {
    if disabled {
        return WidgetState::Disabled;
    }
    if pointer.is_some_and(|point| rect.contains(point)) {
        if pressed {
            WidgetState::Pressed
        } else {
            WidgetState::Hovered
        }
    } else {
        WidgetState::Normal
    }
}

/// Reusable colors for the built-in application widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiTheme {
    pub panel: Color,
    pub panel_border: Color,
    pub text: Color,
    pub muted_text: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub disabled: Color,
    pub control: Color,
    pub control_border: Color,
    pub track: Color,
}

impl Default for UiTheme {
    fn default() -> Self {
        Self {
            panel: Color::rgb(250, 251, 253),
            panel_border: Color::rgb(220, 225, 233),
            text: Color::rgb(31, 41, 57),
            muted_text: Color::rgb(105, 116, 134),
            accent: Color::rgb(49, 112, 218),
            accent_hover: Color::rgb(38, 96, 196),
            accent_pressed: Color::rgb(30, 79, 165),
            disabled: Color::rgb(165, 173, 185),
            control: Color::WHITE,
            control_border: Color::rgb(177, 187, 201),
            track: Color::rgb(230, 234, 241),
        }
    }
}

/// Draws a filled panel with a one-pixel rectangular outline.
pub fn draw_panel(target: &mut Framebuffer, rect: Rect, radius: u32, theme: &UiTheme) {
    fill_rounded_rect(target, rect, radius, theme.panel);
    if rect.width > 0 && rect.height > 0 {
        draw_rect(target, rect, theme.panel_border);
    }
}

/// Draws a stateful rounded button and centers its label.
/// The caller handles pointer hit-testing and activation.
pub fn draw_button(
    target: &mut Framebuffer,
    font: &FontFace,
    rect: Rect,
    label: &str,
    pixel_size: f32,
    state: WidgetState,
    theme: &UiTheme,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let background = match state {
        WidgetState::Normal => theme.accent,
        WidgetState::Hovered => theme.accent_hover,
        WidgetState::Pressed => theme.accent_pressed,
        WidgetState::Disabled => theme.disabled,
    };
    let foreground = if state == WidgetState::Disabled {
        Color::rgb(242, 244, 247)
    } else {
        Color::WHITE
    };
    let radius = (rect.height / 4).min(10);
    fill_rounded_rect(target, rect, radius, background);
    let metrics = font.measure(label, pixel_size);
    let x = i64::from(rect.x) + (i64::from(rect.width) - metrics.width.round() as i64) / 2;
    let line_height = font.line_height(pixel_size);
    let y = i64::from(rect.y)
        + (i64::from(rect.height) - line_height.round() as i64) / 2
        + (line_height * 0.78).round() as i64;
    if let (Ok(x), Ok(y)) = (i32::try_from(x), i32::try_from(y)) {
        draw_text_clipped(
            target,
            font,
            label,
            Point::new(x, y),
            pixel_size,
            foreground,
            rect,
        );
    }
}

/// Draws a checkbox. The application owns and updates the checked value.
pub fn draw_checkbox(
    target: &mut Framebuffer,
    rect: Rect,
    checked: bool,
    state: WidgetState,
    theme: &UiTheme,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let side = rect.width.min(rect.height).min(24);
    if side == 0 {
        return;
    }
    let box_rect = Rect::new(rect.x, rect.y, side, side);
    let background = match state {
        WidgetState::Disabled if checked => theme.disabled,
        _ if checked => theme.accent,
        _ => theme.control,
    };
    fill_rounded_rect(target, box_rect, (side / 5).max(1), background);
    draw_rect(
        target,
        box_rect,
        if checked {
            theme.accent
        } else {
            theme.control_border
        },
    );
    if checked && side >= 7 {
        let x = rect.x as i64;
        let y = rect.y as i64;
        let s = i64::from(side);
        let point = |px: i64, py: i64| {
            Point::new(
                i32::try_from(px).unwrap_or(i32::MAX),
                i32::try_from(py).unwrap_or(i32::MAX),
            )
        };
        draw_line_aa(
            target,
            point(x + s / 5, y + s / 2),
            point(x + s * 2 / 5, y + s * 7 / 10),
            Color::WHITE,
        );
        draw_line_aa(
            target,
            point(x + s * 2 / 5, y + s * 7 / 10),
            point(x + s * 4 / 5, y + s / 4),
            Color::WHITE,
        );
    }
}

/// Draws a progress bar. Non-finite progress is treated as zero and all other
/// values are clamped to 0..=1.
pub fn draw_progress_bar(target: &mut Framebuffer, rect: Rect, progress: f32, theme: &UiTheme) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let radius = rect.height / 2;
    fill_rounded_rect(target, rect, radius, theme.track);
    let progress = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled_width = (rect.width as f64 * f64::from(progress)).round() as u32;
    if filled_width > 0 {
        fill_rounded_rect(
            target,
            Rect::new(rect.x, rect.y, filled_width.min(rect.width), rect.height),
            radius.min(filled_width / 2),
            theme.accent,
        );
    }
}

/// Draws a one-pixel horizontal divider, clipped by the framebuffer.
pub fn draw_divider(target: &mut Framebuffer, x: u32, y: u32, width: u32, color: Color) {
    fill_rect(target, Rect::new(x, y, width, 1), color);
}

/// Draws a text label at a baseline position.
pub fn draw_label(
    target: &mut Framebuffer,
    font: &FontFace,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
) {
    draw_text(target, font, text, baseline, pixel_size, color);
}

/// Draws a text label while restricting glyph pixels to a viewport rectangle.
pub fn draw_label_clipped(
    target: &mut Framebuffer,
    font: &FontFace,
    text: &str,
    baseline: Point,
    pixel_size: f32,
    color: Color,
    clip: Rect,
) -> TextMetrics {
    draw_text_clipped(target, font, text, baseline, pixel_size, color, clip)
}

/// Editable single-line text state with a UTF-8-safe cursor and selection.
///
/// Cursor and selection positions are byte offsets kept on Unicode scalar
/// boundaries. This model is independent from the platform keyboard backend.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextInputState {
    text: String,
    cursor: usize,
    selection_anchor: Option<usize>,
}

impl TextInputState {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.len();
        Self {
            text,
            cursor,
            selection_anchor: None,
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.selection_anchor
            .map(|anchor| (anchor.min(self.cursor), anchor.max(self.cursor)))
            .filter(|(start, end)| start != end)
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.selection_anchor = None;
    }

    pub fn set_cursor(&mut self, byte_index: usize, extend_selection: bool) {
        let next = previous_boundary(&self.text, byte_index.min(self.text.len()));
        if extend_selection {
            self.selection_anchor.get_or_insert(self.cursor);
        } else {
            self.selection_anchor = None;
        }
        self.cursor = next;
    }

    pub fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.text.len();
    }

    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    pub fn move_left(&mut self, extend_selection: bool) {
        if !extend_selection {
            if let Some((start, _)) = self.selection() {
                self.set_cursor(start, false);
                return;
            }
        }
        let next = previous_boundary(&self.text, self.cursor.saturating_sub(1));
        self.set_cursor(next, extend_selection);
    }

    pub fn move_right(&mut self, extend_selection: bool) {
        if !extend_selection {
            if let Some((_, end)) = self.selection() {
                self.set_cursor(end, false);
                return;
            }
        }
        let next = self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.text.len(), |ch| self.cursor + ch.len_utf8());
        self.set_cursor(next, extend_selection);
    }

    pub fn move_home(&mut self, extend_selection: bool) {
        self.set_cursor(0, extend_selection);
    }

    pub fn move_end(&mut self, extend_selection: bool) {
        self.set_cursor(self.text.len(), extend_selection);
    }

    /// Inserts text at the cursor, replacing any active selection.
    pub fn insert_text(&mut self, inserted: &str) {
        self.delete_selection();
        let inserted: String = inserted
            .chars()
            .filter(|character| *character != '\n' && *character != '\r')
            .collect();
        self.text.insert_str(self.cursor, &inserted);
        self.cursor += inserted.len();
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let previous = previous_boundary(&self.text, self.cursor - 1);
        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            return;
        }
        let Some(next) = self.text[self.cursor..]
            .chars()
            .next()
            .map(|ch| self.cursor + ch.len_utf8())
        else {
            return;
        };
        self.text.replace_range(self.cursor..next, "");
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.selection_anchor = None;
        true
    }
}

fn previous_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Draws a single-line text field, including optional selection and a focused caret.
///
/// This is a deterministic static frame; the application controls caret blinking
/// and keyboard events. Long text is clipped to the field's inner viewport.
pub fn draw_text_input(
    target: &mut Framebuffer,
    font: &FontFace,
    rect: Rect,
    state: &TextInputState,
    placeholder: &str,
    pixel_size: f32,
    focused: bool,
    theme: &UiTheme,
) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    fill_rounded_rect(target, rect, 5, theme.control);
    draw_rect(
        target,
        rect,
        if focused {
            theme.accent
        } else {
            theme.control_border
        },
    );

    let inset = 8_u32.min(rect.width / 2);
    let inner = Rect::new(
        rect.x.saturating_add(inset),
        rect.y,
        rect.width.saturating_sub(inset.saturating_mul(2)),
        rect.height,
    );
    if inner.width == 0 {
        return;
    }

    let display = if state.text.is_empty() {
        placeholder
    } else {
        &state.text
    };
    let color = if state.text.is_empty() {
        theme.muted_text
    } else {
        theme.text
    };
    let line_height = font.line_height(pixel_size);
    let baseline_y = i64::from(rect.y)
        + (i64::from(rect.height) - line_height.round() as i64) / 2
        + (line_height * 0.78).round() as i64;
    let Ok(baseline_y) = i32::try_from(baseline_y) else {
        return;
    };
    let Ok(inner_x) = i32::try_from(inner.x) else {
        return;
    };
    let baseline = Point::new(inner_x, baseline_y);

    if !state.text.is_empty() {
        if let Some((start, end)) = state.selection() {
            let before = font.measure(&state.text[..start], pixel_size).width;
            let selected = font.measure(&state.text[start..end], pixel_size).width;
            let selection_x = i64::from(inner.x) + before.floor() as i64;
            if let Ok(selection_x) = u32::try_from(selection_x) {
                let selection_rect = Rect::new(
                    selection_x,
                    rect.y.saturating_add(3),
                    (selected.ceil() as u32).min(inner.width),
                    rect.height.saturating_sub(6),
                );
                fill_rect_clipped(target, selection_rect, inner, theme.accent_hover);
            }
        }
    }

    draw_text_clipped(target, font, display, baseline, pixel_size, color, inner);

    if focused && !state.text.is_empty() {
        let prefix = font.measure(&state.text[..state.cursor], pixel_size).width;
        let caret_x = i64::from(inner.x) + prefix.round() as i64;
        if let Ok(caret_x) = u32::try_from(caret_x) {
            fill_rect_clipped(
                target,
                Rect::new(
                    caret_x,
                    rect.y.saturating_add(5),
                    1,
                    rect.height.saturating_sub(10),
                ),
                inner,
                theme.accent,
            );
        }
    } else if focused && state.text.is_empty() {
        fill_rect_clipped(
            target,
            Rect::new(
                inner.x,
                rect.y.saturating_add(5),
                1,
                rect.height.saturating_sub(10),
            ),
            inner,
            theme.accent,
        );
    }
}

/// Keyboard and pointer selection model for a vertically scrolling list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ListState {
    selected: Option<usize>,
}

impl ListState {
    #[must_use]
    pub const fn new() -> Self {
        Self { selected: None }
    }

    #[must_use]
    pub const fn selected(self) -> Option<usize> {
        self.selected
    }

    pub fn select(&mut self, index: Option<usize>, item_count: usize) {
        self.selected = index.filter(|index| *index < item_count);
    }

    pub fn move_selection(&mut self, delta: isize, item_count: usize) {
        if item_count == 0 {
            self.selected = None;
            return;
        }
        let Some(current) = self.selected else {
            self.selected = Some(if delta < 0 { item_count - 1 } else { 0 });
            return;
        };
        let current = current.min(item_count - 1) as i128;
        let next = (current + delta as i128).clamp(0, item_count as i128 - 1);
        self.selected = Some(next as usize);
    }

    pub fn select_first(&mut self, item_count: usize) {
        self.selected = (item_count > 0).then_some(0);
    }

    pub fn select_last(&mut self, item_count: usize) {
        self.selected = item_count.checked_sub(1);
    }
}

/// Draws visible rows of a list. Scroll offset is measured in pixels.
///
/// Only rows intersecting the viewport are visited, and both row backgrounds
/// and labels are clipped to the list bounds.
pub fn draw_list(
    target: &mut Framebuffer,
    font: &FontFace,
    bounds: Rect,
    items: &[&str],
    state: ListState,
    scroll_offset: u32,
    row_height: u32,
    pixel_size: f32,
    theme: &UiTheme,
) {
    if bounds.width == 0 || bounds.height == 0 || row_height == 0 {
        return;
    }
    let clip = bounds.intersection(Rect::new(0, 0, target.width(), target.height()));
    let Some(clip) = clip else {
        return;
    };
    let first = (scroll_offset / row_height) as usize;
    let last_exclusive = ((u64::from(scroll_offset) + u64::from(bounds.height))
        .div_ceil(u64::from(row_height)))
    .min(items.len() as u64) as usize;
    let line_height = font.line_height(pixel_size);
    for (index, item) in items
        .iter()
        .enumerate()
        .take(last_exclusive)
        .skip(first.min(items.len()))
    {
        let row_top = i64::from(bounds.y)
            .saturating_add((index as i64).saturating_mul(i64::from(row_height)))
            .saturating_sub(i64::from(scroll_offset));
        if row_top < 0 || row_top > i64::from(u32::MAX) {
            continue;
        }
        let row = Rect::new(
            bounds.x,
            row_top as u32,
            bounds.width,
            row_height.min(u32::MAX.saturating_sub(row_top as u32)),
        );
        if state.selected == Some(index) {
            fill_rect_clipped(target, row, clip, theme.accent_hover);
        }
        let baseline_y = row_top
            + (i64::from(row_height) - line_height.round() as i64) / 2
            + (line_height * 0.78).round() as i64;
        let Ok(x) = i32::try_from(bounds.x.saturating_add(8)) else {
            continue;
        };
        let Ok(y) = i32::try_from(baseline_y) else {
            continue;
        };
        draw_text_clipped(
            target,
            font,
            item,
            Point::new(x, y),
            pixel_size,
            theme.text,
            clip,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        draw_checkbox, draw_divider, draw_panel, draw_progress_bar, state_for_pointer, ListState,
        TextInputState, UiTheme, WidgetState,
    };
    use crate::{Color, Framebuffer, Point, Rect};

    fn target() -> Framebuffer {
        Framebuffer::new(32, 24, Color::BLACK).unwrap()
    }

    #[test]
    fn list_selection_clamps_and_handles_empty_lists() {
        let mut list = ListState::new();
        assert_eq!(list.selected(), None);
        list.select_first(0);
        assert_eq!(list.selected(), None);
        list.move_selection(1, 4);
        assert_eq!(list.selected(), Some(0));
        list.select_first(4);
        assert_eq!(list.selected(), Some(0));
        list.move_selection(-2, 4);
        assert_eq!(list.selected(), Some(0));
        list.move_selection(20, 4);
        assert_eq!(list.selected(), Some(3));
        list.select_last(4);
        assert_eq!(list.selected(), Some(3));
        list.select(Some(8), 4);
        assert_eq!(list.selected(), None);
        list.move_selection(1, 0);
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn text_input_inserts_moves_and_deletes_unicode_safely() {
        let mut input = TextInputState::new("aé🙂");
        input.move_left(false);
        assert_eq!(input.cursor(), 3);
        input.insert_text("界");
        assert_eq!(input.text(), "aé界🙂");
        input.backspace();
        assert_eq!(input.text(), "aé🙂");
        input.move_home(false);
        input.delete_forward();
        assert_eq!(input.text(), "é🙂");
    }

    #[test]
    fn text_input_selection_replacement_and_deletion_work() {
        let mut input = TextInputState::new("hello world");
        input.set_cursor(5, false);
        input.set_cursor(11, true);
        assert_eq!(input.selection(), Some((5, 11)));
        input.move_left(false);
        assert_eq!(input.cursor(), 5);
        assert_eq!(input.selection(), None);
        input.set_cursor(5, false);
        input.set_cursor(11, true);
        input.insert_text("engine");
        assert_eq!(input.text(), "helloengine");
        assert_eq!(input.selection(), None);
        input.select_all();
        input.backspace();
        assert_eq!(input.text(), "");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn text_input_cursor_never_splits_utf8_and_home_end_work() {
        let mut input = TextInputState::new("é🙂x");
        input.set_cursor(2, false);
        assert_eq!(input.cursor(), 2);
        input.set_cursor(1, false);
        assert_eq!(input.cursor(), 0);
        input.move_end(false);
        assert_eq!(input.cursor(), "é🙂x".len());
        input.move_home(false);
        assert_eq!(input.cursor(), 0);
        input.move_right(true);
        assert_eq!(input.selection(), Some((0, 2)));
        input.clear_selection();
        assert_eq!(input.selection(), None);
    }

    #[test]
    fn pointer_state_resolves_hover_press_and_disabled() {
        let rect = Rect::new(4, 5, 10, 8);
        assert_eq!(
            state_for_pointer(rect, Some(Point::new(4, 5)), false, false),
            WidgetState::Hovered
        );
        assert_eq!(
            state_for_pointer(rect, Some(Point::new(8, 8)), true, false),
            WidgetState::Pressed
        );
        assert_eq!(
            state_for_pointer(rect, Some(Point::new(8, 8)), true, true),
            WidgetState::Disabled
        );
        assert_eq!(
            state_for_pointer(rect, Some(Point::new(14, 8)), false, false),
            WidgetState::Normal
        );
        assert_eq!(
            state_for_pointer(rect, None, false, false),
            WidgetState::Normal
        );
    }

    #[test]
    fn panel_fills_and_outlines_its_bounds() {
        let mut image = target();
        let theme = UiTheme::default();
        draw_panel(&mut image, Rect::new(4, 4, 12, 10), 3, &theme);
        assert_eq!(image.get_pixel(8, 8), Some(theme.panel));
        assert_eq!(image.get_pixel(4, 8), Some(theme.panel_border));
        assert_eq!(image.get_pixel(3, 8), Some(Color::BLACK));
    }

    #[test]
    fn progress_bar_clamps_values_and_ignores_nan() {
        let mut image = target();
        let theme = UiTheme::default();
        draw_progress_bar(&mut image, Rect::new(2, 2, 20, 6), 0.5, &theme);
        assert_eq!(image.get_pixel(4, 4), Some(theme.accent));
        assert_eq!(image.get_pixel(19, 4), Some(theme.track));

        image.clear(Color::BLACK);
        draw_progress_bar(&mut image, Rect::new(2, 2, 20, 6), f32::NAN, &theme);
        assert_eq!(image.get_pixel(4, 4), Some(theme.track));
    }

    #[test]
    fn progress_bar_handles_full_and_out_of_range_values() {
        let mut image = target();
        let theme = UiTheme::default();
        draw_progress_bar(&mut image, Rect::new(2, 2, 20, 6), 2.0, &theme);
        assert_eq!(image.get_pixel(20, 4), Some(theme.accent));
        assert_eq!(image.get_pixel(22, 4), Some(Color::BLACK));
    }

    #[test]
    fn checkbox_renders_checked_and_unchecked_states() {
        let mut image = target();
        let theme = UiTheme::default();
        draw_checkbox(
            &mut image,
            Rect::new(2, 2, 16, 16),
            false,
            WidgetState::Normal,
            &theme,
        );
        assert_eq!(image.get_pixel(8, 8), Some(theme.control));
        draw_checkbox(
            &mut image,
            Rect::new(2, 2, 16, 16),
            true,
            WidgetState::Normal,
            &theme,
        );
        assert_eq!(image.get_pixel(8, 8), Some(theme.accent));
    }

    #[test]
    fn divider_is_clipped_and_zero_width_is_a_noop() {
        let mut image = target();
        draw_divider(&mut image, 28, 5, 20, Color::WHITE);
        draw_divider(&mut image, 0, 6, 0, Color::RED);
        assert_eq!(image.get_pixel(31, 5), Some(Color::WHITE));
        assert_eq!(image.get_pixel(0, 6), Some(Color::BLACK));
    }

    #[test]
    fn disabled_checkbox_uses_disabled_fill_when_checked() {
        let mut image = target();
        let theme = UiTheme::default();
        draw_checkbox(
            &mut image,
            Rect::new(2, 2, 16, 16),
            true,
            WidgetState::Disabled,
            &theme,
        );
        assert_eq!(image.get_pixel(8, 8), Some(theme.disabled));
    }
}
