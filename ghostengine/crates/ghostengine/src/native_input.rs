//! Native keyboard, text, IME and clipboard input for desktop applications.
//!
//! This adapter consumes events from winit, which delegates keyboard layout,
//! text input and IME handling to the active Windows or Linux window system.
//! It does not render or create a graphics context: applications continue to
//! render into GhostEngine's CPU framebuffer.
//!
//! Enable with the `native-input` Cargo feature. Applications using this
//! adapter should forward each `WindowEvent` for their window to
//! `process_window_event`, then pass `take_frame()` to
//! `ComponentTree::handle_window_input`.

use crate::{window::InputState, ClipboardAction, Point, TextInputEvent};
use arboard::Clipboard;
use std::{error::Error, fmt, mem};
use winit::{
    event::{ElementState, Ime, MouseButton, WindowEvent},
    keyboard::{KeyCode, ModifiersState, PhysicalKey},
};

/// Stateful adapter from native window events to GhostEngine input frames.
#[derive(Debug, Default)]
pub struct NativeInputAdapter {
    frame: InputState,
    mouse_position: Option<Point>,
    left_mouse_down: bool,
    shift_down: bool,
    control_down: bool,
    alt_down: bool,
    escape_down: bool,
    ime_enabled: bool,
}

impl NativeInputAdapter {
    /// Creates an empty adapter. Call this once per native window.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one event received for the application's winit window.
    ///
    /// Events are accumulated until `take_frame` is called. Modifier and
    /// pointer state persist between frames; key edges and text events do not.
    pub fn process_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.update_modifiers(modifiers.state());
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_position = Some(Point::new(
                    clamp_coordinate(position.x),
                    clamp_coordinate(position.y),
                ));
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse_position = None;
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let down = *state == ElementState::Pressed;
                if down != self.left_mouse_down {
                    if down {
                        self.frame.left_mouse_pressed = true;
                    } else {
                        self.frame.left_mouse_released = true;
                    }
                }
                self.left_mouse_down = down;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.process_keyboard_event(event);
            }
            WindowEvent::Ime(ime_event) => match ime_event {
                Ime::Enabled => self.ime_enabled = true,
                Ime::Preedit(text, _) => self
                    .frame
                    .text_events
                    .push(TextInputEvent::CompositionUpdate(text.clone())),
                Ime::Commit(text) => self
                    .frame
                    .text_events
                    .push(TextInputEvent::CompositionCommit(text.clone())),
                Ime::Disabled => {
                    self.ime_enabled = false;
                    self.frame
                        .text_events
                        .push(TextInputEvent::CompositionCancel);
                }
            },
            WindowEvent::Focused(false) => {
                self.shift_down = false;
                self.control_down = false;
                self.alt_down = false;
                self.escape_down = false;
                if self.left_mouse_down {
                    self.left_mouse_down = false;
                    self.frame.left_mouse_released = true;
                }
                self.frame
                    .text_events
                    .push(TextInputEvent::CompositionCancel);
            }
            _ => {}
        }
    }

    /// Returns the accumulated frame and resets transient events.
    #[must_use]
    pub fn take_frame(&mut self) -> InputState {
        let mut frame = mem::take(&mut self.frame);
        frame.mouse_position = self.mouse_position;
        frame.left_mouse_down = self.left_mouse_down;
        frame.shift_down = self.shift_down;
        frame.control_down = self.control_down;
        frame.alt_down = self.alt_down;
        frame.escape_down = self.escape_down;
        frame
    }

    fn update_modifiers(&mut self, modifiers: ModifiersState) {
        self.shift_down = modifiers.shift_key();
        self.control_down = modifiers.control_key();
        self.alt_down = modifiers.alt_key();
    }

    fn process_keyboard_event(&mut self, event: &winit::event::KeyEvent) {
        let pressed = event.state == ElementState::Pressed;
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };

        if code == KeyCode::Escape {
            self.escape_down = pressed;
        }
        if !pressed {
            return;
        }

        apply_key_code(code, self.control_down, self.shift_down, &mut self.frame);

        // KeyEvent::text is produced using the active OS keyboard layout. It
        // therefore supports AZERTY/QWERTY and composed characters, unlike
        // translating physical key positions into ASCII characters.
        if !self.ime_enabled && !self.control_down && !self.alt_down {
            if let Some(text) = event.text.as_ref().filter(|text| !text.is_empty()) {
                self.frame
                    .text_events
                    .push(TextInputEvent::Insert(text.to_string()));
            }
        }
    }
}

fn clamp_coordinate(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    value
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

fn apply_key_code(code: KeyCode, control: bool, shift: bool, input: &mut InputState) {
    match code {
        KeyCode::Backspace => input.backspace_pressed = true,
        KeyCode::Delete => input.delete_pressed = true,
        KeyCode::ArrowLeft => input.left_pressed = true,
        KeyCode::ArrowRight => input.right_pressed = true,
        KeyCode::Home => input.home_pressed = true,
        KeyCode::End => input.end_pressed = true,
        KeyCode::Enter | KeyCode::NumpadEnter => input.enter_pressed = true,
        KeyCode::Space => input.space_pressed = true,
        KeyCode::Tab => input.tab_pressed = true,
        KeyCode::KeyA if control => input.select_all_pressed = true,
        KeyCode::KeyC if control => input.copy_pressed = true,
        KeyCode::KeyX if control => input.cut_pressed = true,
        KeyCode::KeyV if control => input.paste_pressed = true,
        _ => {}
    }
    input.shift_down = shift;
    input.control_down = control;
}

/// System clipboard access for a native desktop application.
///
/// Clipboard access is deliberately kept outside the renderer and input
/// adapter so applications can choose when to resolve a Copy/Paste request.
pub struct NativeClipboard {
    clipboard: Clipboard,
}

impl NativeClipboard {
    /// Opens the current user's system clipboard.
    pub fn new() -> Result<Self, NativeClipboardError> {
        Clipboard::new()
            .map(|clipboard| Self { clipboard })
            .map_err(NativeClipboardError::from)
    }

    /// Executes a component-tree clipboard request.
    ///
    /// Copy writes the supplied text and returns `Ok(None)`. Paste reads the
    /// current clipboard and returns `Ok(Some(text))` for insertion into the
    /// focused text input.
    pub fn resolve(
        &mut self,
        action: &ClipboardAction,
    ) -> Result<Option<String>, NativeClipboardError> {
        match action {
            ClipboardAction::Copy(text) => {
                self.clipboard
                    .set_text(text.clone())
                    .map_err(NativeClipboardError::from)?;
                Ok(None)
            }
            ClipboardAction::Paste => self
                .clipboard
                .get_text()
                .map(Some)
                .map_err(NativeClipboardError::from),
        }
    }
}

/// Error returned when the native system clipboard cannot be opened or used.
#[derive(Debug)]
pub struct NativeClipboardError(arboard::Error);

impl fmt::Display for NativeClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "native clipboard error: {}", self.0)
    }
}

impl Error for NativeClipboardError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl From<arboard::Error> for NativeClipboardError {
    fn from(error: arboard::Error) -> Self {
        Self(error)
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_key_code, clamp_coordinate, NativeInputAdapter};
    use crate::{window::InputState, TextInputEvent};
    use winit::keyboard::KeyCode;

    #[test]
    fn native_key_mapping_preserves_shortcuts_and_navigation() {
        let mut input = InputState::default();
        apply_key_code(KeyCode::ArrowLeft, true, true, &mut input);
        apply_key_code(KeyCode::Backspace, true, true, &mut input);
        apply_key_code(KeyCode::KeyA, true, true, &mut input);
        apply_key_code(KeyCode::KeyC, true, true, &mut input);
        apply_key_code(KeyCode::Space, false, false, &mut input);

        assert!(input.left_pressed);
        assert!(input.backspace_pressed);
        assert!(input.select_all_pressed);
        assert!(input.copy_pressed);
        assert!(input.space_pressed);
        assert!(input.shift_down);
        assert!(input.control_down);
    }

    #[test]
    fn native_input_frames_clear_edges_but_preserve_modifiers_and_pointer() {
        let mut adapter = NativeInputAdapter::new();
        adapter.mouse_position = Some(crate::Point::new(12, 34));
        adapter.left_mouse_down = true;
        adapter.shift_down = true;
        adapter.frame.enter_pressed = true;

        let first = adapter.take_frame();
        assert!(first.enter_pressed);
        assert!(first.left_mouse_down);
        assert!(first.shift_down);
        assert_eq!(first.mouse_position, Some(crate::Point::new(12, 34)));

        let second = adapter.take_frame();
        assert!(!second.enter_pressed);
        assert!(second.left_mouse_down);
        assert!(second.shift_down);
    }

    #[test]
    fn ime_events_are_kept_as_composition_events() {
        let mut adapter = NativeInputAdapter::new();
        adapter
            .frame
            .text_events
            .push(TextInputEvent::CompositionUpdate("に".to_owned()));
        let frame = adapter.take_frame();
        assert_eq!(
            frame.text_events,
            vec![TextInputEvent::CompositionUpdate("に".to_owned())]
        );
    }

    #[test]
    fn native_pointer_coordinates_are_clamped_and_non_finite_values_are_safe() {
        assert_eq!(clamp_coordinate(42.4), 42);
        assert_eq!(clamp_coordinate(f64::INFINITY), 0);
        assert_eq!(clamp_coordinate(f64::NAN), 0);
        assert_eq!(clamp_coordinate(f64::MAX), i32::MAX);
    }
}
