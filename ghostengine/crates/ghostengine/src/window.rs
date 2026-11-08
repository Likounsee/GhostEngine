//! Optional native window presentation and input adapter.
//!
//! GhostEngine renders the frame. minifb only creates a host window, forwards
//! input and presents the completed CPU pixel buffer.

use crate::{Color, Framebuffer, Point};
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
    /// True while either Shift key is held.
    pub shift_down: bool,
    /// True while either Control key is held.
    pub control_down: bool,
    /// True while either Alt key is held.
    pub alt_down: bool,
    pub pressed_characters: Vec<char>,
    pub backspace_pressed: bool,
    pub delete_pressed: bool,
    pub left_pressed: bool,
    pub right_pressed: bool,
    pub home_pressed: bool,
    pub end_pressed: bool,
    pub enter_pressed: bool,
    pub tab_pressed: bool,
    pub escape_down: bool,
}

/// A fixed-size host window that presents GhostEngine's CPU framebuffer.
pub struct WindowSurface {
    window: Window,
    width: u32,
    height: u32,
    presentation_buffer: Vec<u32>,
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

        let window = Window::new(title, width_usize, height_usize, WindowOptions::default())
            .map_err(|error| WindowError::Backend(error.to_string()))?;
        Ok(Self {
            window,
            width,
            height,
            presentation_buffer,
        })
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    /// Samples pointer and keyboard input for the current frame.
    pub fn poll_input(&mut self) -> InputState {
        let mouse_position = self
            .window
            .get_mouse_pos(MouseMode::Clamp)
            .map(|(x, y)| Point::new(x.floor() as i32, y.floor() as i32));
        let pressed = self.window.get_keys_pressed(KeyRepeat::Yes);
        let mut input = InputState {
            mouse_position,
            left_mouse_down: self.window.get_mouse_down(MouseButton::Left),
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

fn pack_rgb(color: Color) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

fn key_to_character(key: Key, shift: bool) -> Option<char> {
    Some(match key {
        Key::A => if shift { 'A' } else { 'a' },
        Key::B => if shift { 'B' } else { 'b' },
        Key::C => if shift { 'C' } else { 'c' },
        Key::D => if shift { 'D' } else { 'd' },
        Key::E => if shift { 'E' } else { 'e' },
        Key::F => if shift { 'F' } else { 'f' },
        Key::G => if shift { 'G' } else { 'g' },
        Key::H => if shift { 'H' } else { 'h' },
        Key::I => if shift { 'I' } else { 'i' },
        Key::J => if shift { 'J' } else { 'j' },
        Key::K => if shift { 'K' } else { 'k' },
        Key::L => if shift { 'L' } else { 'l' },
        Key::M => if shift { 'M' } else { 'm' },
        Key::N => if shift { 'N' } else { 'n' },
        Key::O => if shift { 'O' } else { 'o' },
        Key::P => if shift { 'P' } else { 'p' },
        Key::Q => if shift { 'Q' } else { 'q' },
        Key::R => if shift { 'R' } else { 'r' },
        Key::S => if shift { 'S' } else { 's' },
        Key::T => if shift { 'T' } else { 't' },
        Key::U => if shift { 'U' } else { 'u' },
        Key::V => if shift { 'V' } else { 'v' },
        Key::W => if shift { 'W' } else { 'w' },
        Key::X => if shift { 'X' } else { 'x' },
        Key::Y => if shift { 'Y' } else { 'y' },
        Key::Z => if shift { 'Z' } else { 'z' },
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
        Key::Period => if shift { '>' } else { '.' },
        Key::Slash => if shift { '?' } else { '/' },
        Key::Minus => if shift { '_' } else { '-' },
        Key::Space => ' ',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{key_to_character, pack_rgb, InputState, WindowError};
    use crate::Color;
    use minifb::Key;

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
    fn window_errors_have_readable_messages() {
        assert!(WindowError::FramebufferSizeMismatch
            .to_string()
            .contains("dimensions"));
    }
}
