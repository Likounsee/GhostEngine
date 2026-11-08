//! Winit desktop window with direct CPU-framebuffer presentation.
//!
//! This module does not create a GPU context. GhostEngine renders into its
//! framebuffer and softbuffer copies those pixels to the native window.

use crate::{
    window::InputState, Color, Framebuffer, FramebufferError, NativeClipboard, NativeInputAdapter,
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
    Framebuffer(FramebufferError),
    EventLoop(String),
    Backend(String),
    Clipboard(String),
}

impl fmt::Display for NativeWindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => f.write_str("native window dimensions must be non-zero"),
            Self::Framebuffer(error) => write!(f, "framebuffer error: {error}"),
            Self::EventLoop(error) => write!(f, "event loop error: {error}"),
            Self::Backend(error) => write!(f, "native presentation error: {error}"),
            Self::Clipboard(error) => write!(f, "native clipboard error: {error}"),
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
/// framebuffer. Clipboard requests produced by ComponentTree can be resolved
/// with `clipboard.resolve(&action)`.
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
        F: FnMut(&mut Framebuffer, InputState, &mut NativeClipboard) + 'static,
    {
        if width == 0 || height == 0 {
            return Err(NativeWindowError::InvalidDimensions);
        }

        let event_loop =
            EventLoop::new().map_err(|error| NativeWindowError::EventLoop(error.to_string()))?;
        let context = Context::new(event_loop.owned_display_handle())
            .map_err(|error| NativeWindowError::Backend(error.to_string()))?;
        let framebuffer = Framebuffer::new(width, height, Color::BLACK)?;
        let clipboard = NativeClipboard::new()
            .map_err(|error| NativeWindowError::Clipboard(error.to_string()))?;

        let mut app = NativeApp {
            context,
            title: title.to_owned(),
            initial_size: PhysicalSize::new(width, height),
            framebuffer,
            input: NativeInputAdapter::new(),
            clipboard,
            surface: None,
            window: None,
            draw,
        };
        event_loop
            .run_app(&mut app)
            .map_err(|error| NativeWindowError::EventLoop(error.to_string()))
    }
}

struct NativeApp<F> {
    context: Context<OwnedDisplayHandle>,
    title: String,
    initial_size: PhysicalSize<u32>,
    framebuffer: Framebuffer,
    input: NativeInputAdapter,
    clipboard: NativeClipboard,
    surface: Option<Surface<OwnedDisplayHandle, Arc<Window>>>,
    window: Option<Arc<Window>>,
    draw: F,
}

impl<F> NativeApp<F>
where
    F: FnMut(&mut Framebuffer, InputState, &mut NativeClipboard),
{
    fn resize_framebuffer(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        if self.framebuffer.width() == size.width && self.framebuffer.height() == size.height {
            return;
        }
        let Ok(mut resized) = Framebuffer::new(size.width, size.height, Color::BLACK) else {
            return;
        };
        let copy_width = self.framebuffer.width().min(size.width);
        let copy_height = self.framebuffer.height().min(size.height);
        for y in 0..copy_height {
            for x in 0..copy_width {
                if let Some(pixel) = self.framebuffer.get_pixel(x, y) {
                    resized.set_pixel(x, y, pixel);
                }
            }
        }
        self.framebuffer = resized;
    }

    fn present(&mut self) -> Result<(), NativeWindowError> {
        let Some(surface) = self.surface.as_mut() else {
            return Ok(());
        };
        let width = self.framebuffer.width();
        let height = self.framebuffer.height();
        let (Some(width_nz), Some(height_nz)) = (NonZeroU32::new(width), NonZeroU32::new(height))
        else {
            return Ok(());
        };
        surface
            .resize(width_nz, height_nz)
            .map_err(|error| NativeWindowError::Backend(error.to_string()))?;

        let mut buffer = surface
            .buffer_mut()
            .map_err(|error| NativeWindowError::Backend(error.to_string()))?;
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
    F: FnMut(&mut Framebuffer, InputState, &mut NativeClipboard),
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title(self.title.clone())
            .with_inner_size(self.initial_size);
        let Ok(window) = event_loop.create_window(attributes) else {
            event_loop.exit();
            return;
        };
        let window = Arc::new(window);
        match Surface::new(&self.context, window.clone()) {
            Ok(surface) => {
                self.surface = Some(surface);
                self.window = Some(window.clone());
                window.request_redraw();
            }
            Err(_) => event_loop.exit(),
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.resize_framebuffer(size);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let input = self.input.take_frame();
                (self.draw)(&mut self.framebuffer, input, &mut self.clipboard);
                if self.present().is_err() {
                    event_loop.exit();
                }
            }
            WindowEvent::Focused(_)
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::Ime(_)
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// Converts a straight-alpha GhostEngine color to softbuffer's 0x00RRGGBB layout.
fn pack_softbuffer_pixel(color: Color) -> u32 {
    (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
}

#[cfg(test)]
mod tests {
    use super::pack_softbuffer_pixel;
    use crate::Color;

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
