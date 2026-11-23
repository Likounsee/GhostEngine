//! Winit desktop window with direct CPU-framebuffer presentation.
//!
//! This module does not create a GPU context. GhostEngine renders into its
//! framebuffer and softbuffer copies those pixels to the native window.

use crate::{
    input::InputState, Color, Framebuffer, FramebufferError, HostError, HostEvent,
    HostWindowConfig, NativeClipboard, NativeInputAdapter,
};
use softbuffer::{Context, Surface};
use std::{error::Error, fmt, num::NonZeroU32, sync::Arc};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop, OwnedDisplayHandle},
    window::{Window, WindowAttributes, WindowId},
};

/// Errors raised while creating or running a native Winit window.
#[derive(Debug)]
pub enum NativeWindowError {
    InvalidDimensions,
    Host(HostError),
    Framebuffer(FramebufferError),
    EventLoop(String),
    Backend(String),
    Clipboard(String),
    PresentationBufferSizeMismatch { expected: usize, actual: usize },
}

impl fmt::Display for NativeWindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => f.write_str("native window dimensions must be non-zero"),
            Self::Host(error) => write!(f, "invalid native host configuration: {error}"),
            Self::Framebuffer(error) => write!(f, "framebuffer error: {error}"),
            Self::EventLoop(error) => write!(f, "event loop error: {error}"),
            Self::Backend(error) => write!(f, "native presentation error: {error}"),
            Self::Clipboard(error) => write!(f, "native clipboard error: {error}"),
            Self::PresentationBufferSizeMismatch { expected, actual } => write!(
                f,
                "native presentation buffer has {actual} pixels; expected {expected}"
            ),
        }
    }
}

impl Error for NativeWindowError {}

impl From<FramebufferError> for NativeWindowError {
    fn from(value: FramebufferError) -> Self {
        Self::Framebuffer(value)
    }
}

/// Runs a Winit event loop and presents GhostEngine's CPU framebuffer.
///
/// The callback is invoked for each redraw with the accumulated native input
/// frame and a system clipboard handle. It should render into the supplied
/// framebuffer and return `false` to close the window. Clipboard requests
/// produced by ComponentTree can be resolved with `clipboard.resolve(&action)`.
pub struct NativeWindowSurface;

impl NativeWindowSurface {
    /// Creates and runs a native window until it is closed.
    ///
    /// This is an event-loop entry point and should normally be called from
    /// the application's main thread. Winit's event loop can only be run once.
    pub fn run<F>(
        title: &str,
        width: u32,
        height: u32,
        mut draw: F,
    ) -> Result<(), NativeWindowError>
    where
        F: FnMut(&mut Framebuffer, InputState, &mut NativeClipboard) -> bool + 'static,
    {
        Self::run_with_host_events(
            title,
            width,
            height,
            move |framebuffer, events, clipboard| {
                // The legacy callback has no lifecycle-event parameter, so it
                // must still honor the native close button instead of silently
                // discarding CloseRequested and leaving the window alive.
                if legacy_close_requested(&events) {
                    return false;
                }
                let input = events
                    .into_iter()
                    .find_map(|event| match event {
                        HostEvent::Input(input) => Some(input),
                        _ => None,
                    })
                    .unwrap_or_default();
                draw(framebuffer, input, clipboard)
            },
        )
    }

    /// Runs the native window while delivering normalized common host events.
    ///
    /// This entry point exposes resize, scale-factor, focus, close, redraw and
    /// accumulated input events without leaking Winit types to the application.
    /// Returning false from the callback closes the window.
    pub fn run_with_host_events<F>(
        title: &str,
        width: u32,
        height: u32,
        draw: F,
    ) -> Result<(), NativeWindowError>
    where
        F: FnMut(&mut Framebuffer, Vec<HostEvent>, &mut NativeClipboard) -> bool + 'static,
    {
        Self::run_with_config(HostWindowConfig::new(title, width, height), draw)
    }

    /// Runs a native window using the platform-independent host configuration.
    ///
    /// This is the native event-loop entry point for applications that need to
    /// control the initial size and whether the user can resize the window.
    /// The configuration is validated before any native resources are created.
    pub fn run_with_config<F>(config: HostWindowConfig, draw: F) -> Result<(), NativeWindowError>
    where
        F: FnMut(&mut Framebuffer, Vec<HostEvent>, &mut NativeClipboard) -> bool + 'static,
    {
        config.validate().map_err(NativeWindowError::Host)?;
        let title = config.title;
        let width = config.width;
        let height = config.height;
        let event_loop =
            EventLoop::new().map_err(|error| NativeWindowError::EventLoop(error.to_string()))?;
        let context = Context::new(event_loop.owned_display_handle())
            .map_err(|error| NativeWindowError::Backend(error.to_string()))?;
        let framebuffer = Framebuffer::new(width, height, Color::BLACK)?;
        // Clipboard access is optional on several desktop sessions (for
        // example, minimal Wayland/headless environments). Keep the window
        // usable and report an explicit error only if the app requests it.
        let clipboard = NativeClipboard::new().unwrap_or_else(|_| NativeClipboard::unavailable());

        let mut app = NativeApp {
            context,
            title: title.to_owned(),
            initial_size: PhysicalSize::new(width, height),
            resizable: config.resizable,
            framebuffer,
            input: NativeInputAdapter::new(),
            clipboard,
            surface: None,
            surface_size: None,
            drawable_size: (width, height),
            window: None,
            pending_events: Vec::new(),
            failure: None,
            draw,
        };
        event_loop
            .run_app(&mut app)
            .map_err(|error| NativeWindowError::EventLoop(error.to_string()))?;
        if let Some(error) = app.failure.take() {
            return Err(error);
        }
        Ok(())
    }
}

struct NativeApp<F> {
    context: Context<OwnedDisplayHandle>,
    title: String,
    initial_size: PhysicalSize<u32>,
    resizable: bool,
    framebuffer: Framebuffer,
    input: NativeInputAdapter,
    clipboard: NativeClipboard,
    surface: Option<Surface<OwnedDisplayHandle, Arc<Window>>>,
    surface_size: Option<(u32, u32)>,
    /// Last physical drawable size reported by the window system, including zero-sized minimized states.
    drawable_size: (u32, u32),
    window: Option<Arc<Window>>,
    pending_events: Vec<HostEvent>,
    failure: Option<NativeWindowError>,
    draw: F,
}

impl<F> NativeApp<F>
where
    F: FnMut(&mut Framebuffer, Vec<HostEvent>, &mut NativeClipboard) -> bool,
{
    fn resize_framebuffer(&mut self, size: PhysicalSize<u32>) -> Result<(), FramebufferError> {
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        if self.framebuffer.width() == size.width && self.framebuffer.height() == size.height {
            return Ok(());
        }

        self.framebuffer
            .resize_preserving(size.width, size.height, Color::BLACK)?;
        Ok(())
    }

    /// Synchronizes the CPU framebuffer with the actual physical drawable size.
    /// Returns whether the drawable dimensions changed.
    fn sync_drawable_size(&mut self, size: PhysicalSize<u32>) -> Result<bool, FramebufferError> {
        let next_size = (size.width, size.height);
        let changed = drawable_size_changed(self.drawable_size, next_size);
        self.resize_framebuffer(size)?;
        if changed {
            self.drawable_size = next_size;
            self.pending_events.push(HostEvent::Resized {
                width: size.width,
                height: size.height,
            });
        }
        Ok(changed)
    }

    fn present(&mut self) -> Result<(), NativeWindowError> {
        // A minimized window can have a zero-sized drawable. Keep the last
        // framebuffer, but do not ask softbuffer to present into an unavailable
        // surface; the next non-zero resize will resume normal presentation.
        if self.window.as_ref().is_some_and(|window| {
            let size = window.inner_size();
            !drawable_size_is_nonzero(size.width, size.height)
        }) {
            return Ok(());
        }
        let Some(surface) = self.surface.as_mut() else {
            return Ok(());
        };
        let width = self.framebuffer.width();
        let height = self.framebuffer.height();
        let (Some(width_nz), Some(height_nz)) = (NonZeroU32::new(width), NonZeroU32::new(height))
        else {
            return Ok(());
        };
        if self.surface_size != Some((width, height)) {
            surface
                .resize(width_nz, height_nz)
                .map_err(|error| NativeWindowError::Backend(error.to_string()))?;
            self.surface_size = Some((width, height));
        }

        let mut buffer = surface
            .buffer_mut()
            .map_err(|error| NativeWindowError::Backend(error.to_string()))?;
        validate_presentation_buffer_len(self.framebuffer.pixels().len(), buffer.len())?;
        for (destination, source) in buffer.iter_mut().zip(self.framebuffer.pixels()) {
            *destination = pack_softbuffer_pixel(*source);
        }
        if let Some(window) = &self.window {
            window.pre_present_notify();
        }
        buffer
            .present()
            .map_err(|error| NativeWindowError::Backend(error.to_string()))
    }
}

impl<F> ApplicationHandler for NativeApp<F>
where
    F: FnMut(&mut Framebuffer, Vec<HostEvent>, &mut NativeClipboard) -> bool,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title(self.title.clone())
            .with_inner_size(self.initial_size)
            .with_resizable(self.resizable);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => window,
            Err(error) => {
                self.failure = Some(NativeWindowError::Backend(error.to_string()));
                event_loop.exit();
                return;
            }
        };
        let window = Arc::new(window);
        window.set_ime_allowed(true);

        // Window managers and display scaling can adjust the requested client
        // size. The actual physical drawable size is authoritative for both
        // the framebuffer and the first normalized resize event.
        let actual_size = window.inner_size();
        self.drawable_size = (actual_size.width, actual_size.height);
        if let Err(error) = self.resize_framebuffer(actual_size) {
            self.failure = Some(NativeWindowError::Framebuffer(error));
            event_loop.exit();
            return;
        }

        match Surface::new(&self.context, window.clone()) {
            Ok(surface) => {
                self.surface = Some(surface);
                self.window = Some(window.clone());
                self.pending_events.push(HostEvent::Resized {
                    width: actual_size.width,
                    height: actual_size.height,
                });
                self.pending_events.push(HostEvent::ScaleFactorChanged {
                    scale_factor: window.scale_factor(),
                });
                window.request_redraw();
            }
            Err(error) => {
                self.failure = Some(NativeWindowError::Backend(error.to_string()));
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self
            .window
            .as_ref()
            .map_or(true, |window| window.id() != window_id)
        {
            return;
        }
        self.input.process_window_event(&event);
        match event {
            WindowEvent::CloseRequested => {
                self.pending_events.push(HostEvent::CloseRequested);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Resized(size) => {
                if let Err(error) = self.sync_drawable_size(size) {
                    self.failure = Some(NativeWindowError::Framebuffer(error));
                    event_loop.exit();
                    return;
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let input = self.input.take_frame();
                self.pending_events.push(HostEvent::RedrawRequested);
                self.pending_events.push(HostEvent::Input(input));
                let events = std::mem::take(&mut self.pending_events);
                if !(self.draw)(&mut self.framebuffer, events, &mut self.clipboard) {
                    event_loop.exit();
                    return;
                }
                if let Err(error) = self.present() {
                    self.failure = Some(error);
                    event_loop.exit();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.pending_events
                    .push(HostEvent::ScaleFactorChanged { scale_factor });
                // DPI transitions can change the physical drawable even when
                // the platform does not deliver a distinct Resized event.
                if let Some(window) = self.window.clone() {
                    let actual_size = window.inner_size();
                    if let Err(error) = self.sync_drawable_size(actual_size) {
                        self.failure = Some(NativeWindowError::Framebuffer(error));
                        event_loop.exit();
                        return;
                    }
                    window.request_redraw();
                }
            }
            WindowEvent::Focused(focused) => {
                self.pending_events
                    .push(HostEvent::FocusChanged { focused });
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            // Every event that changes accumulated input must schedule a
            // redraw; otherwise wheel and modifier-only updates can remain
            // queued indefinitely when the window is otherwise idle.
            WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::Ime(_)
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// The legacy input-only callback cannot inspect lifecycle events, so
/// native close requests must terminate it automatically.
fn legacy_close_requested(events: &[HostEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, HostEvent::CloseRequested))
}

/// Returns whether a physical drawable has an area available for presentation.
fn drawable_size_is_nonzero(width: u32, height: u32) -> bool {
    width != 0 && height != 0
}

/// Returns whether the window system reported a drawable-size transition.
fn drawable_size_changed(previous: (u32, u32), next: (u32, u32)) -> bool {
    previous != next
}

/// Converts a straight-alpha GhostEngine color to softbuffer's 0x00RRGGBB layout.
fn validate_presentation_buffer_len(
    expected: usize,
    actual: usize,
) -> Result<(), NativeWindowError> {
    if expected == actual {
        Ok(())
    } else {
        Err(NativeWindowError::PresentationBufferSizeMismatch { expected, actual })
    }
}

fn pack_softbuffer_pixel(color: Color) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

#[cfg(test)]
mod tests {
    use super::pack_softbuffer_pixel;
    use crate::{Color, Framebuffer};

    #[test]
    fn resize_preserves_top_left_pixels_and_clears_new_area() {
        let mut source = Framebuffer::new(2, 2, Color::BLACK).unwrap();
        source
            .pixels_mut()
            .copy_from_slice(&[Color::RED, Color::GREEN, Color::BLUE, Color::WHITE]);

        let mut resized = source.try_clone().unwrap();
        resized.resize_preserving(3, 3, Color::BLACK).unwrap();
        assert_eq!(resized.width(), 3);
        assert_eq!(resized.height(), 3);
        assert_eq!(
            resized.pixels(),
            &[
                Color::RED,
                Color::GREEN,
                Color::BLACK,
                Color::BLUE,
                Color::WHITE,
                Color::BLACK,
                Color::BLACK,
                Color::BLACK,
                Color::BLACK,
            ]
        );

        resized.resize_preserving(1, 1, Color::BLACK).unwrap();
        assert_eq!(resized.pixels(), &[Color::RED]);
    }

    #[test]
    fn rejects_invalid_common_host_configuration_before_starting_event_loop() {
        let config = crate::HostWindowConfig::new("invalid", 0, 480);
        let result = super::NativeWindowSurface::run_with_config(config, |_, _, _| true);

        assert!(matches!(
            result,
            Err(super::NativeWindowError::Host(
                crate::HostError::InvalidDimensions
            ))
        ));
    }

    #[test]
    fn legacy_callback_honors_native_close_requests() {
        use crate::HostEvent;

        assert!(super::legacy_close_requested(&[HostEvent::CloseRequested]));
        assert!(!super::legacy_close_requested(&[
            HostEvent::RedrawRequested,
            HostEvent::FocusChanged { focused: true },
        ]));
    }

    #[test]
    fn minimized_window_size_is_not_presentable() {
        assert!(!super::drawable_size_is_nonzero(0, 0));
        assert!(!super::drawable_size_is_nonzero(640, 0));
        assert!(!super::drawable_size_is_nonzero(0, 480));
        assert!(super::drawable_size_is_nonzero(640, 480));
    }

    #[test]
    fn drawable_size_tracks_minimize_and_restore_even_when_framebuffer_size_is_unchanged() {
        let framebuffer_size = (640, 480);
        let mut reported_size = framebuffer_size;

        let minimized = (0, 0);
        assert!(super::drawable_size_changed(reported_size, minimized));
        reported_size = minimized;

        let restored = (640, 480);
        assert!(super::drawable_size_changed(reported_size, restored));
        reported_size = restored;

        // The transition is observable even though the retained framebuffer
        // never changed dimensions while the window was minimized.
        assert_eq!(reported_size, framebuffer_size);
        assert!(!super::drawable_size_changed(
            reported_size,
            framebuffer_size
        ));
    }

    #[test]
    fn rejects_a_native_presentation_buffer_with_the_wrong_pixel_count() {
        assert!(validate_presentation_buffer_len(4, 4).is_ok());
        assert!(matches!(
            validate_presentation_buffer_len(4, 3),
            Err(NativeWindowError::PresentationBufferSizeMismatch {
                expected: 4,
                actual: 3
            })
        ));
    }

    #[test]
    fn converts_rgba_to_softbuffer_rgb_without_alpha() {
        assert_eq!(
            pack_softbuffer_pixel(Color::rgba(0x12, 0x34, 0x56, 0x78)),
            0x0012_3456
        );
    }

    #[test]
    fn converts_black_and_white() {
        assert_eq!(pack_softbuffer_pixel(Color::BLACK), 0);
        assert_eq!(pack_softbuffer_pixel(Color::WHITE), 0x00ff_ffff);
    }
}
