//! Optional native window presentation and input adapter.
//!
//! GhostEngine renders the frame. minifb only creates a host window, forwards
//! input and presents the completed CPU pixel buffer.

use crate::{
    ClipboardAction, Color, ComponentEvents, ComponentKey, ComponentTree, Framebuffer, Point, Rect,
    TextInputEvent,
};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use std::{error::Error, fmt};

/// Errors returned by the optional desktop window adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowError {
    ZeroDimensions,
    SizeOverflow,
    AllocationFailed,
    Backend(String),
    FramebufferSizeMismatch,
}

impl fmt::Display for WindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimensions => formatter.write_str("window dimensions must be non-zero"),
            Self::SizeOverflow => formatter.write_str("window dimensions do not fit this platform"),
            Self::AllocationFailed => formatter.write_str("could not allocate presentation buffer"),
            Self::Backend(message) => write!(formatter, "window backend error: {message}"),
            Self::FramebufferSizeMismatch => {
                formatter.write_str("framebuffer dimensions do not match the window")
            }
        }
    }
}

impl Error for WindowError {}

/// Input sampled once per application frame.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InputState {
    pub mouse_position: Option<Point>,
    pub left_mouse_down: bool,
    /// True only on the frame the left mouse button transitions to down.
    pub left_mouse_pressed: bool,
    /// True only on the frame the left mouse button transitions to up.
    pub left_mouse_released: bool,
    /// Vertical wheel movement in pixels; positive values scroll toward later rows.
    pub scroll_delta_y: i32,
    /// True while either Shift key is held.
    pub shift_down: bool,
    /// True while either Control key is held.
    pub control_down: bool,
    /// True while either Alt key is held.
    pub alt_down: bool,
    /// Committed or composing text delivered by a platform text service.
    pub text_events: Vec<TextInputEvent>,
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

/// A fixed-size host window that presents GhostEngine's CPU framebuffer.
pub struct WindowSurface {
    window: Window,
    width: u32,
    height: u32,
    presentation_buffer: Vec<u32>,
    previous_left_mouse_down: bool,
}

impl WindowSurface {
    /// Creates a window. The renderer remains fully owned by GhostEngine.
    pub fn new(title: &str, width: u32, height: u32) -> Result<Self, WindowError> {
        if width == 0 || height == 0 {
            return Err(WindowError::ZeroDimensions);
        }
        let width_usize = usize::try_from(width).map_err(|_| WindowError::SizeOverflow)?;
        let height_usize = usize::try_from(height).map_err(|_| WindowError::SizeOverflow)?;
        let pixel_count = width_usize
            .checked_mul(height_usize)
            .ok_or(WindowError::SizeOverflow)?;
        let mut presentation_buffer = Vec::new();
        presentation_buffer
            .try_reserve_exact(pixel_count)
            .map_err(|_| WindowError::AllocationFailed)?;
        presentation_buffer.resize(pixel_count, 0);

        let window = Window::new(
            title,
            width_usize,
            height_usize,
            WindowOptions {
                resize: true,
                ..WindowOptions::default()
            },
        )
        .map_err(|error| WindowError::Backend(error.to_string()))?;
        Ok(Self {
            window,
            width,
            height,
            presentation_buffer,
            previous_left_mouse_down: false,
        })
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }
    /// Returns the current drawable size of the native window in pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        let (width, height) = self.window.get_size();
        (
            u32::try_from(width).unwrap_or(u32::MAX),
            u32::try_from(height).unwrap_or(u32::MAX),
        )
    }

    /// Resizes the CPU framebuffer and presentation buffer to the current
    /// native window size. Existing pixels in the overlapping area are kept.
    ///
    /// Returns `true` when a resize was applied. A minimized/zero-sized native
    /// surface is ignored and returns `false`.
    pub fn sync_framebuffer_size(
        &mut self,
        framebuffer: &mut Framebuffer,
    ) -> Result<bool, WindowError> {
        let (native_width, native_height) = self.window.get_size();
        if native_width == 0 || native_height == 0 {
            return Ok(false);
        }

        let width = u32::try_from(native_width).map_err(|_| WindowError::SizeOverflow)?;
        let height = u32::try_from(native_height).map_err(|_| WindowError::SizeOverflow)?;
        if width == self.width && height == self.height {
            return Ok(false);
        }

        let pixel_count = native_width
            .checked_mul(native_height)
            .ok_or(WindowError::SizeOverflow)?;
        let mut presentation_buffer = Vec::new();
        presentation_buffer
            .try_reserve_exact(pixel_count)
            .map_err(|_| WindowError::AllocationFailed)?;
        presentation_buffer.resize(pixel_count, 0);

        let mut resized =
            Framebuffer::new(width, height, Color::BLACK).map_err(|error| match error {
                crate::FramebufferError::ZeroDimension => WindowError::ZeroDimensions,
                crate::FramebufferError::SizeOverflow => WindowError::SizeOverflow,
                crate::FramebufferError::AllocationFailed => WindowError::AllocationFailed,
            })?;
        copy_framebuffer_overlap(framebuffer, &mut resized);

        *framebuffer = resized;
        self.width = width;
        self.height = height;
        self.presentation_buffer = presentation_buffer;
        Ok(true)
    }

    /// Samples pointer and keyboard input for the current frame.
    pub fn poll_input(&mut self) -> InputState {
        let mouse_position = self
            .window
            .get_mouse_pos(MouseMode::Clamp)
            .map(|(x, y)| Point::new(x.floor() as i32, y.floor() as i32));
        let pressed = self.window.get_keys_pressed(KeyRepeat::Yes);
        let left_mouse_down = self.window.get_mouse_down(MouseButton::Left);
        let (left_mouse_pressed, left_mouse_released) =
            button_transition(self.previous_left_mouse_down, left_mouse_down);
        self.previous_left_mouse_down = left_mouse_down;
        let scroll_delta_y = self
            .window
            .get_scroll_wheel()
            .map(|(_, y)| (-y * 48.0).round().clamp(i32::MIN as f32, i32::MAX as f32) as i32)
            .unwrap_or(0);
        let mut input = InputState {
            scroll_delta_y,
            mouse_position,
            left_mouse_down,
            left_mouse_pressed,
            left_mouse_released,
            shift_down: self.window.is_key_down(Key::LeftShift)
                || self.window.is_key_down(Key::RightShift),
            control_down: self.window.is_key_down(Key::LeftCtrl)
                || self.window.is_key_down(Key::RightCtrl),
            alt_down: self.window.is_key_down(Key::LeftAlt)
                || self.window.is_key_down(Key::RightAlt),
            escape_down: self.window.is_key_down(Key::Escape),
            ..InputState::default()
        };

        for key in pressed {
            match key {
                Key::Backspace => input.backspace_pressed = true,
                Key::Delete => input.delete_pressed = true,
                Key::Left => input.left_pressed = true,
                Key::Right => input.right_pressed = true,
                Key::Home => input.home_pressed = true,
                Key::End => input.end_pressed = true,
                Key::Enter => input.enter_pressed = true,
                Key::Tab => input.tab_pressed = true,
                Key::A if input.control_down => input.select_all_pressed = true,
                Key::C if input.control_down => input.copy_pressed = true,
                Key::X if input.control_down => input.cut_pressed = true,
                Key::V if input.control_down => input.paste_pressed = true,
                _ => {
                    if let Some(character) = key_to_character(key, input.shift_down) {
                        input.pressed_characters.push(character);
                    }
                }
            }
        }
        input
    }

    /// Converts and presents one complete GhostEngine framebuffer.
    pub fn present(&mut self, framebuffer: &Framebuffer) -> Result<(), WindowError> {
        if framebuffer.width() != self.width || framebuffer.height() != self.height {
            return Err(WindowError::FramebufferSizeMismatch);
        }
        for (destination, source) in self
            .presentation_buffer
            .iter_mut()
            .zip(framebuffer.pixels())
        {
            *destination = pack_rgb(*source);
        }
        self.window
            .update_with_buffer(
                &self.presentation_buffer,
                self.width as usize,
                self.height as usize,
            )
            .map_err(|error| WindowError::Backend(error.to_string()))
    }
}

/// Routes one sampled native input frame into the retained component tree.
///
/// Unicode text and IME composition are supplied through text_events by
/// the host text-input service. The minifb fallback remains a limited physical
/// key mapping; it is not an AZERTY/QWERTY layout implementation.
impl ComponentTree {
    pub fn handle_window_input(&mut self, bounds: Rect, input: &InputState) -> ComponentEvents {
        let mut events = self.update_pointer(
            bounds,
            input.mouse_position,
            input.left_mouse_pressed,
            input.left_mouse_released,
        );
        if input.scroll_delta_y != 0 {
            if let Some(pointer) = input.mouse_position {
                self.scroll_list_view_at(pointer, input.scroll_delta_y);
            }
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
                if input.control_down {
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
                if input.control_down {
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
        if input.backspace_pressed {
            self.handle_key(
                bounds,
                if input.control_down {
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
                if input.control_down {
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
        if input.text_events.is_empty() && !input.control_down && !input.alt_down {
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
}

/// Resolves the edge events for a button sampled once per frame.
fn button_transition(previous_down: bool, current_down: bool) -> (bool, bool) {
    (
        current_down && !previous_down,
        previous_down && !current_down,
    )
}

/// Copies the top-left overlapping region from one framebuffer to another.
fn copy_framebuffer_overlap(source: &Framebuffer, destination: &mut Framebuffer) {
    let width = source.width().min(destination.width());
    let height = source.height().min(destination.height());
    for y in 0..height {
        for x in 0..width {
            if let Some(pixel) = source.get_pixel(x, y) {
                destination.set_pixel(x, y, pixel);
            }
        }
    }
}

fn pack_rgb(color: Color) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

fn key_to_character(key: Key, shift: bool) -> Option<char> {
    Some(match key {
        Key::A => {
            if shift {
                'A'
            } else {
                'a'
            }
        }
        Key::B => {
            if shift {
                'B'
            } else {
                'b'
            }
        }
        Key::C => {
            if shift {
                'C'
            } else {
                'c'
            }
        }
        Key::D => {
            if shift {
                'D'
            } else {
                'd'
            }
        }
        Key::E => {
            if shift {
                'E'
            } else {
                'e'
            }
        }
        Key::F => {
            if shift {
                'F'
            } else {
                'f'
            }
        }
        Key::G => {
            if shift {
                'G'
            } else {
                'g'
            }
        }
        Key::H => {
            if shift {
                'H'
            } else {
                'h'
            }
        }
        Key::I => {
            if shift {
                'I'
            } else {
                'i'
            }
        }
        Key::J => {
            if shift {
                'J'
            } else {
                'j'
            }
        }
        Key::K => {
            if shift {
                'K'
            } else {
                'k'
            }
        }
        Key::L => {
            if shift {
                'L'
            } else {
                'l'
            }
        }
        Key::M => {
            if shift {
                'M'
            } else {
                'm'
            }
        }
        Key::N => {
            if shift {
                'N'
            } else {
                'n'
            }
        }
        Key::O => {
            if shift {
                'O'
            } else {
                'o'
            }
        }
        Key::P => {
            if shift {
                'P'
            } else {
                'p'
            }
        }
        Key::Q => {
            if shift {
                'Q'
            } else {
                'q'
            }
        }
        Key::R => {
            if shift {
                'R'
            } else {
                'r'
            }
        }
        Key::S => {
            if shift {
                'S'
            } else {
                's'
            }
        }
        Key::T => {
            if shift {
                'T'
            } else {
                't'
            }
        }
        Key::U => {
            if shift {
                'U'
            } else {
                'u'
            }
        }
        Key::V => {
            if shift {
                'V'
            } else {
                'v'
            }
        }
        Key::W => {
            if shift {
                'W'
            } else {
                'w'
            }
        }
        Key::X => {
            if shift {
                'X'
            } else {
                'x'
            }
        }
        Key::Y => {
            if shift {
                'Y'
            } else {
                'y'
            }
        }
        Key::Z => {
            if shift {
                'Z'
            } else {
                'z'
            }
        }
        Key::Key0 => '0',
        Key::Key1 => '1',
        Key::Key2 => '2',
        Key::Key3 => '3',
        Key::Key4 => '4',
        Key::Key5 => '5',
        Key::Key6 => '6',
        Key::Key7 => '7',
        Key::Key8 => '8',
        Key::Key9 => '9',
        Key::Period => {
            if shift {
                '>'
            } else {
                '.'
            }
        }
        Key::Slash => {
            if shift {
                '?'
            } else {
                '/'
            }
        }
        Key::Minus => {
            if shift {
                '_'
            } else {
                '-'
            }
        }
        Key::Space => ' ',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        key_to_character, pack_rgb, ClipboardAction, ComponentTree, InputState, TextInputEvent,
        WindowError,
    };
    use crate::Color;
    use minifb::Key;

    #[test]
    fn framebuffer_resize_preserves_overlapping_pixels_and_clears_new_area() {
        let mut source = crate::Framebuffer::new(2, 2, Color::RED).unwrap();
        source.set_pixel(1, 1, Color::BLUE);
        let mut larger = crate::Framebuffer::new(3, 4, Color::BLACK).unwrap();

        super::copy_framebuffer_overlap(&source, &mut larger);

        assert_eq!(larger.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(larger.get_pixel(1, 1), Some(Color::BLUE));
        assert_eq!(larger.get_pixel(2, 1), Some(Color::BLACK));
        assert_eq!(larger.get_pixel(1, 2), Some(Color::BLACK));
    }

    #[test]
    fn framebuffer_resize_truncates_pixels_outside_new_bounds() {
        let source = crate::Framebuffer::new(4, 3, Color::WHITE).unwrap();
        let mut smaller = crate::Framebuffer::new(2, 1, Color::BLACK).unwrap();

        super::copy_framebuffer_overlap(&source, &mut smaller);

        assert_eq!(smaller.width(), 2);
        assert_eq!(smaller.height(), 1);
        assert!(smaller.pixels().iter().all(|pixel| *pixel == Color::WHITE));
    }

    #[test]
    fn mouse_button_transitions_emit_single_press_and_release_edges() {
        assert_eq!(super::button_transition(false, false), (false, false));
        assert_eq!(super::button_transition(false, true), (true, false));
        assert_eq!(super::button_transition(true, true), (false, false));
        assert_eq!(super::button_transition(true, false), (false, true));
    }

    #[test]
    fn input_state_defaults_mouse_edges_to_false() {
        let input = InputState::default();
        assert!(!input.left_mouse_down);
        assert!(!input.left_mouse_pressed);
        assert!(!input.left_mouse_released);
    }

    #[test]
    fn presentation_uses_rgb_packed_into_u32() {
        assert_eq!(pack_rgb(Color::rgb(0x12, 0x34, 0x56)), 0x0012_3456);
    }

    #[test]
    fn keyboard_maps_basic_url_characters() {
        assert_eq!(key_to_character(Key::A, false), Some('a'));
        assert_eq!(key_to_character(Key::A, true), Some('A'));
        assert_eq!(key_to_character(Key::Slash, true), Some('?'));
        assert_eq!(key_to_character(Key::Minus, true), Some('_'));
        assert_eq!(key_to_character(Key::Key7, false), Some('7'));
        assert_eq!(key_to_character(Key::Period, false), Some('.'));
        assert_eq!(key_to_character(Key::Slash, false), Some('/'));
        assert_eq!(key_to_character(Key::F1, false), None);
    }

    #[test]
    fn input_state_defaults_modifiers_to_released() {
        let input = InputState::default();
        assert!(!input.shift_down);
        assert!(!input.control_down);
        assert!(!input.alt_down);
    }

    #[test]
    fn sampled_window_input_edits_text_and_activates_focused_controls() {
        use crate::{ComponentId, LayoutItem, UiComponent};
        let bounds = crate::Rect::new(0, 0, 120, 40);
        let input_id = ComponentId(70);
        let button_id = ComponentId(71);
        let mut tree = ComponentTree::new(vec![
            UiComponent::text_input(input_id, "Type", "AB").with_layout_item(LayoutItem::fixed(20)),
            UiComponent::button(button_id, "OK").with_layout_item(LayoutItem::fixed(20)),
        ]);

        let tab = InputState {
            tab_pressed: true,
            ..InputState::default()
        };
        assert_eq!(
            tree.handle_window_input(bounds, &tab).focus_changed,
            Some(input_id)
        );

        let type_text = InputState {
            pressed_characters: vec!['h', 'i'],
            ..InputState::default()
        };
        tree.handle_window_input(bounds, &type_text);
        assert!(matches!(
            tree.find(input_id).unwrap().kind(),
            crate::ComponentKind::TextInput { value, .. } if value == "ABhi"
        ));

        let select_and_replace = InputState {
            left_pressed: true,
            shift_down: true,
            pressed_characters: vec!['🙂'],
            ..InputState::default()
        };
        tree.handle_window_input(bounds, &select_and_replace);
        assert!(matches!(
            tree.find(input_id).unwrap().kind(),
            crate::ComponentKind::TextInput { value, .. } if value == "ABh🙂"
        ));

        let select_all = InputState {
            select_all_pressed: true,
            ..InputState::default()
        };
        tree.handle_window_input(bounds, &select_all);
        let replace = InputState {
            pressed_characters: vec!['x'],
            ..InputState::default()
        };
        tree.handle_window_input(bounds, &replace);
        assert!(matches!(
            tree.find(input_id).unwrap().kind(),
            crate::ComponentKind::TextInput { value, .. } if value == "x"
        ));

        let tab = InputState {
            tab_pressed: true,
            ..InputState::default()
        };
        assert_eq!(
            tree.handle_window_input(bounds, &tab).focus_changed,
            Some(button_id)
        );
        let space = InputState {
            space_pressed: true,
            ..InputState::default()
        };
        assert_eq!(
            tree.handle_window_input(bounds, &space).activated,
            Some(button_id)
        );
        let enter = InputState {
            enter_pressed: true,
            ..InputState::default()
        };
        assert_eq!(
            tree.handle_window_input(bounds, &enter).activated,
            Some(button_id)
        );
    }

    #[test]
    fn control_word_shortcuts_move_and_delete_unicode_words() {
        use crate::{ComponentId, LayoutItem, UiComponent};
        let bounds = crate::Rect::new(0, 0, 180, 40);
        let id = ComponentId(91);
        let mut tree = ComponentTree::new(vec![UiComponent::text_input(
            id,
            "Input",
            "hello brave world",
        )
        .with_layout_item(LayoutItem::fixed(40))]);
        tree.handle_window_input(
            bounds,
            &InputState {
                tab_pressed: true,
                ..InputState::default()
            },
        );

        tree.handle_window_input(
            bounds,
            &InputState {
                left_pressed: true,
                control_down: true,
                ..InputState::default()
            },
        );
        assert_eq!(
            tree.text_input_state(id).unwrap().cursor(),
            "hello brave ".len()
        );

        tree.handle_window_input(
            bounds,
            &InputState {
                backspace_pressed: true,
                control_down: true,
                ..InputState::default()
            },
        );
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            crate::ComponentKind::TextInput { value, .. } if value == "hello world"
        ));
    }

    #[test]
    fn host_unicode_text_events_take_precedence_over_legacy_key_fallback() {
        use crate::{ComponentId, LayoutItem, UiComponent};
        let bounds = crate::Rect::new(0, 0, 120, 40);
        let id = ComponentId(92);
        let mut tree = ComponentTree::new(vec![
            UiComponent::text_input(id, "Input", "").with_layout_item(LayoutItem::fixed(40))
        ]);
        tree.handle_window_input(
            bounds,
            &InputState {
                tab_pressed: true,
                ..InputState::default()
            },
        );
        tree.handle_window_input(
            bounds,
            &InputState {
                text_events: vec![TextInputEvent::Insert("é🙂".into())],
                pressed_characters: vec!['x'],
                ..InputState::default()
            },
        );
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            crate::ComponentKind::TextInput { value, .. } if value == "é🙂"
        ));
    }

    #[test]
    fn host_unicode_ime_and_clipboard_events_are_routed_to_focused_input() {
        use crate::{ComponentId, LayoutItem, UiComponent};
        let bounds = crate::Rect::new(0, 0, 120, 40);
        let id = ComponentId(90);
        let mut tree = ComponentTree::new(vec![
            UiComponent::text_input(id, "Input", "").with_layout_item(LayoutItem::fixed(40))
        ]);
        let tab = InputState {
            tab_pressed: true,
            ..InputState::default()
        };
        tree.handle_window_input(bounds, &tab);

        let ime = InputState {
            text_events: vec![TextInputEvent::CompositionUpdate("に".into())],
            ..InputState::default()
        };
        tree.handle_window_input(bounds, &ime);
        assert_eq!(tree.text_input_state(id).unwrap().composition(), Some("に"));

        let commit = InputState {
            text_events: vec![TextInputEvent::CompositionCommit("日本語🙂".into())],
            ..InputState::default()
        };
        tree.handle_window_input(bounds, &commit);
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            crate::ComponentKind::TextInput { value, .. } if value == "日本語🙂"
        ));

        tree.select_all_text(id);
        let copy = InputState {
            copy_pressed: true,
            control_down: true,
            ..InputState::default()
        };
        assert_eq!(
            tree.handle_window_input(bounds, &copy).clipboard_action,
            Some(ClipboardAction::Copy("日本語🙂".into()))
        );

        let cut = InputState {
            cut_pressed: true,
            control_down: true,
            ..InputState::default()
        };
        assert_eq!(
            tree.handle_window_input(bounds, &cut).clipboard_action,
            Some(ClipboardAction::Copy("日本語🙂".into()))
        );
        let paste_request = InputState {
            paste_pressed: true,
            control_down: true,
            ..InputState::default()
        };
        assert_eq!(
            tree.handle_window_input(bounds, &paste_request)
                .clipboard_action,
            Some(ClipboardAction::Paste)
        );
        let paste_result = InputState {
            paste_pressed: true,
            control_down: true,
            clipboard_text: Some("collé 🎮".into()),
            ..InputState::default()
        };
        tree.handle_window_input(bounds, &paste_result);
        assert!(matches!(
            tree.find(id).unwrap().kind(),
            crate::ComponentKind::TextInput { value, .. } if value == "collé 🎮"
        ));
    }

    #[test]
    fn window_errors_have_readable_messages() {
        assert!(WindowError::FramebufferSizeMismatch
            .to_string()
            .contains("dimensions"));
    }
}
