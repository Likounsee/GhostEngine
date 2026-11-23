//! Deterministic, display-server-free implementation of the common host API.
//!
//! The headless backend is intended for tests, servers, off-screen rendering,
//! image generation and applications that provide their own presentation path.

use crate::{
    Framebuffer, FramebufferError, HostBackend, HostCapabilities, HostError, HostEvent,
    HostPlatform, HostWindowConfig,
};
use std::{collections::VecDeque, error::Error, fmt};

/// Errors returned by the headless host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlessHostError {
    Configuration(HostError),
    Framebuffer(FramebufferError),
    DimensionMismatch,
    DrawableUnavailable,
    ResizeDisabled,
    Closed,
}

impl fmt::Display for HeadlessHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(error) => write!(formatter, "headless host configuration: {error}"),
            Self::Framebuffer(error) => write!(formatter, "headless framebuffer copy: {error}"),
            Self::DimensionMismatch => {
                formatter.write_str("framebuffer dimensions do not match the headless drawable")
            }
            Self::DrawableUnavailable => {
                formatter.write_str("headless drawable is minimized and cannot present a frame")
            }
            Self::ResizeDisabled => formatter.write_str("headless drawable is not resizable"),
            Self::Closed => formatter.write_str("headless host is closed"),
        }
    }
}

impl Error for HeadlessHostError {}

impl From<HostError> for HeadlessHostError {
    fn from(value: HostError) -> Self {
        Self::Configuration(value)
    }
}

impl From<FramebufferError> for HeadlessHostError {
    fn from(value: FramebufferError) -> Self {
        Self::Framebuffer(value)
    }
}

/// In-memory host implementation with deterministic event delivery.
#[derive(Debug)]
pub struct HeadlessHost {
    config: HostWindowConfig,
    events: VecDeque<HostEvent>,
    presented_frame: Option<Framebuffer>,
    last_nonzero_drawable_size: (u32, u32),
    open: bool,
}

impl HeadlessHost {
    /// Creates a headless drawable using the requested physical dimensions.
    pub fn new(config: HostWindowConfig) -> Result<Self, HeadlessHostError> {
        config.validate()?;
        let last_nonzero_drawable_size = (config.width, config.height);
        Ok(Self {
            config,
            events: VecDeque::new(),
            presented_frame: None,
            last_nonzero_drawable_size,
            open: true,
        })
    }

    /// Queues an event to be returned by the next event poll.
    pub fn queue_event(&mut self, event: HostEvent) {
        self.events.push_back(event);
    }

    /// Changes the drawable size and queues the matching resize event.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), HeadlessHostError> {
        if !self.open {
            return Err(HeadlessHostError::Closed);
        }
        if (width, height) == self.drawable_size() {
            return Ok(());
        }
        let is_minimized = width == 0 || height == 0;
        if !self.config.resizable
            && !is_minimized
            && (width, height) != self.last_nonzero_drawable_size
        {
            return Err(HeadlessHostError::ResizeDisabled);
        }
        // A zero-sized drawable represents a minimized window. Initial
        // configuration still requires non-zero dimensions, but runtime
        // resize events must be able to model the native window lifecycle.
        if !is_minimized {
            let mut candidate = self.config.clone();
            candidate.width = width;
            candidate.height = height;
            candidate.validate()?;
        }
        self.config.width = width;
        self.config.height = height;
        if !is_minimized {
            self.last_nonzero_drawable_size = (width, height);
        }
        self.queue_event(HostEvent::Resized { width, height });
        Ok(())
    }

    /// Takes ownership of the most recently presented frame, if any.
    #[must_use]
    pub fn take_presented_frame(&mut self) -> Option<Framebuffer> {
        self.presented_frame.take()
    }
}

impl HostBackend for HeadlessHost {
    type BackendError = HeadlessHostError;

    fn create(config: HostWindowConfig) -> Result<Self, Self::BackendError> {
        Self::new(config)
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn close(&mut self) {
        if self.open {
            self.open = false;
            self.presented_frame = None;
            self.queue_event(HostEvent::CloseRequested);
        }
    }

    fn platform(&self) -> HostPlatform {
        HostPlatform::Headless
    }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            windowing: false,
            high_dpi: false,
            clipboard: false,
            text_input: false,
            ime: false,
            resize: self.config.resizable,
        }
    }

    fn drawable_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn scale_factor(&self) -> f64 {
        1.0
    }

    fn poll_events(&mut self, events: &mut Vec<HostEvent>) {
        events.extend(self.events.drain(..));
    }

    fn request_redraw(&mut self) {
        if self.open {
            self.queue_event(HostEvent::RedrawRequested);
        }
    }

    fn present(&mut self, framebuffer: &Framebuffer) -> Result<(), Self::BackendError> {
        if !self.open {
            return Err(HeadlessHostError::Closed);
        }
        let (width, height) = self.drawable_size();
        if width == 0 || height == 0 {
            return Err(HeadlessHostError::DrawableUnavailable);
        }
        if (framebuffer.width(), framebuffer.height()) != (width, height) {
            return Err(HeadlessHostError::DimensionMismatch);
        }
        if let Some(presented) = self.presented_frame.as_mut() {
            presented.copy_pixels_from(framebuffer.pixels())?;
        } else {
            self.presented_frame = Some(framebuffer.try_clone()?);
        }
        Ok(())
    }
}

impl Default for HeadlessHost {
    fn default() -> Self {
        Self::new(HostWindowConfig::new("GhostEngine headless", 1, 1))
            .expect("the built-in 1x1 headless configuration is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::{HeadlessHost, HeadlessHostError};
    use crate::{
        Color, Framebuffer, HostBackend, HostError, HostEvent, HostPlatform, HostWindowConfig,
    };

    #[test]
    fn delivers_queued_events_in_order_and_clears_them() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 64, 32)).unwrap();
        host.queue_event(HostEvent::FocusChanged { focused: false });
        host.request_redraw();

        let mut events = Vec::new();
        host.poll_events(&mut events);
        assert_eq!(
            events,
            [
                HostEvent::FocusChanged { focused: false },
                HostEvent::RedrawRequested
            ]
        );

        events.clear();
        host.poll_events(&mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn resize_updates_drawable_and_queues_event() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 64, 32)).unwrap();
        host.resize(128, 96).unwrap();

        assert_eq!(host.drawable_size(), (128, 96));
        let mut events = Vec::new();
        host.poll_events(&mut events);
        assert_eq!(
            events,
            [HostEvent::Resized {
                width: 128,
                height: 96
            }]
        );
    }

    #[test]
    fn resizing_to_the_current_drawable_does_not_emit_duplicate_events() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 64, 32)).unwrap();

        host.resize(64, 32).unwrap();
        host.resize(0, 0).unwrap();
        host.resize(0, 0).unwrap();

        let mut events = Vec::new();
        host.poll_events(&mut events);
        assert_eq!(
            events,
            [HostEvent::Resized {
                width: 0,
                height: 0,
            }]
        );
    }

    #[test]
    fn rejects_invalid_resize_without_changing_drawable() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 64, 32)).unwrap();
        assert_eq!(
            host.resize(u32::MAX, u32::MAX),
            Err(HeadlessHostError::Configuration(HostError::SizeOverflow))
        );
        assert_eq!(host.drawable_size(), (64, 32));
    }

    #[test]
    fn resize_models_minimize_and_restore_with_zero_sized_drawables() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 64, 32)).unwrap();

        host.resize(0, 0).unwrap();
        assert_eq!(host.drawable_size(), (0, 0));

        host.resize(64, 32).unwrap();
        assert_eq!(host.drawable_size(), (64, 32));

        let mut events = Vec::new();
        host.poll_events(&mut events);
        assert_eq!(
            events,
            [
                HostEvent::Resized {
                    width: 0,
                    height: 0,
                },
                HostEvent::Resized {
                    width: 64,
                    height: 32,
                },
            ]
        );
    }

    #[test]
    fn resize_accepts_zero_in_one_drawable_dimension() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 64, 32)).unwrap();
        host.resize(0, 32).unwrap();
        assert_eq!(host.drawable_size(), (0, 32));
        host.resize(64, 32).unwrap();
        assert_eq!(host.drawable_size(), (64, 32));
    }

    #[test]
    fn minimized_drawable_rejects_presentation_with_a_specific_error() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 2, 2)).unwrap();
        let frame = Framebuffer::new(2, 2, Color::BLACK).unwrap();

        host.resize(0, 0).unwrap();

        assert_eq!(
            host.present(&frame),
            Err(HeadlessHostError::DrawableUnavailable)
        );
    }

    #[test]
    fn fixed_size_host_rejects_resize_and_reports_capability() {
        let mut config = HostWindowConfig::new("fixed", 64, 32);
        config.resizable = false;
        let mut host = HeadlessHost::new(config).unwrap();

        assert!(!host.capabilities().resize);
        assert_eq!(host.resize(128, 96), Err(HeadlessHostError::ResizeDisabled));
        assert_eq!(host.drawable_size(), (64, 32));
        let mut events = Vec::new();
        host.poll_events(&mut events);
        assert!(events.is_empty());
    }

    #[test]
    fn fixed_size_host_can_minimize_and_restore_without_enabling_user_resize() {
        let mut config = HostWindowConfig::new("fixed", 64, 32);
        config.resizable = false;
        let mut host = HeadlessHost::new(config).unwrap();

        host.resize(0, 0).unwrap();
        assert_eq!(host.drawable_size(), (0, 0));
        host.resize(64, 32).unwrap();
        assert_eq!(host.drawable_size(), (64, 32));
        assert_eq!(host.resize(128, 96), Err(HeadlessHostError::ResizeDisabled));
        assert_eq!(host.drawable_size(), (64, 32));
        assert!(!host.capabilities().resize);
    }

    #[test]
    fn presents_an_independent_frame_copy() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 2, 1)).unwrap();
        let mut frame = Framebuffer::new(2, 1, Color::BLACK).unwrap();
        frame.set_pixel(0, 0, Color::RED);

        host.present(&frame).unwrap();
        frame.set_pixel(0, 0, Color::BLUE);

        let presented = host.take_presented_frame().unwrap();
        assert_eq!(presented.get_pixel(0, 0), Some(Color::RED));
        assert!(host.take_presented_frame().is_none());
    }

    #[test]
    fn repeated_presentation_reuses_the_owned_frame_allocation() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 2, 1)).unwrap();
        let mut frame = Framebuffer::new(2, 1, Color::BLACK).unwrap();
        host.present(&frame).unwrap();
        let allocation = host.presented_frame.as_ref().unwrap().pixels().as_ptr();

        frame.pixels_mut().copy_from_slice(&[Color::RED, Color::BLUE]);
        host.present(&frame).unwrap();

        let presented = host.presented_frame.as_ref().unwrap();
        assert_eq!(presented.pixels(), &[Color::RED, Color::BLUE]);
        assert_eq!(presented.pixels().as_ptr(), allocation);
    }

    #[test]
    fn close_releases_the_last_presented_frame() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 1, 1)).unwrap();
        let frame = Framebuffer::new(1, 1, Color::RED).unwrap();
        host.present(&frame).unwrap();
        assert!(host.presented_frame.is_some());

        host.close();

        assert!(host.presented_frame.is_none());
    }

    #[test]
    fn close_is_idempotent_and_closed_hosts_do_not_schedule_redraws() {
        let mut host = HeadlessHost::default();

        host.close();
        host.close();
        host.request_redraw();

        let mut events = Vec::new();
        host.poll_events(&mut events);
        assert_eq!(events, [HostEvent::CloseRequested]);
        assert!(!host.is_open());
    }

    #[test]
    fn rejects_frame_with_wrong_dimensions() {
        let mut host = HeadlessHost::new(HostWindowConfig::new("test", 2, 1)).unwrap();
        let frame = Framebuffer::new(1, 1, Color::BLACK).unwrap();
        assert_eq!(
            host.present(&frame),
            Err(HeadlessHostError::DimensionMismatch)
        );
    }

    #[test]
    fn lifecycle_can_be_created_and_closed_through_common_trait() {
        let mut host =
            <HeadlessHost as HostBackend>::create(HostWindowConfig::new("lifecycle", 16, 16))
                .unwrap();
        assert!(host.is_open());
        host.close();
        assert!(!host.is_open());
        let mut events = Vec::new();
        host.poll_events(&mut events);
        assert_eq!(events, [HostEvent::CloseRequested]);
    }

    #[test]
    fn closed_host_rejects_resize_and_presentation() {
        let mut host = HeadlessHost::default();
        host.close();
        assert_eq!(host.resize(2, 2), Err(HeadlessHostError::Closed));
        let frame = Framebuffer::new(1, 1, Color::BLACK).unwrap();
        assert_eq!(host.present(&frame), Err(HeadlessHostError::Closed));
    }

    #[test]
    fn reports_headless_capabilities() {
        let host = HeadlessHost::default();
        assert_eq!(host.platform(), HostPlatform::Headless);
        assert!(!host.capabilities().windowing);
        assert!(!host.capabilities().clipboard);
        assert_eq!(host.scale_factor(), 1.0);
    }
}
