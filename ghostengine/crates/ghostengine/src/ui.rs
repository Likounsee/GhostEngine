//! Small, backend-independent 2D widgets for building real application interfaces.
//!
//! Widgets only draw into a GhostEngine framebuffer. They do not own a window,
//! event loop, or platform state; applications provide interaction state and
//! use the same CPU renderer as every other scene.

use crate::rasterizer::{draw_line_aa, draw_rect, fill_rect, fill_rounded_rect};
use crate::{draw_text, Color, FontFace, Framebuffer, Point, Rect};

/// Visual state supplied by an application's hit-testing/input layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidgetState {
    #[default]
    Normal,
    Hovered,
    Pressed,
    Disabled,
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
        draw_text(
            target,
            font,
            label,
            Point::new(x, y),
            pixel_size,
            foreground,
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

#[cfg(test)]
mod tests {
    use super::{draw_checkbox, draw_divider, draw_panel, draw_progress_bar, UiTheme, WidgetState};
    use crate::{Color, Framebuffer, Rect};

    fn target() -> Framebuffer {
        Framebuffer::new(32, 24, Color::BLACK).unwrap()
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
        assert_eq!(image.get_pixel(21, 4), Some(Color::BLACK));
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
