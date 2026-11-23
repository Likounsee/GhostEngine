//! Platform-neutral host API contract for GhostEngine applications.
//!
//! Applications target this module rather than depending on winit, minifb,
//! softbuffer, or an operating-system window handle. Platform adapters are
//! responsible for translating native events and surfaces into this contract.
//! The CPU renderer remains independent of every host backend.

use crate::{Framebuffer, InputState};

/// Requested properties for a desktop host window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWindowConfig {
    pub title: String,
    /// Requested client-area width in physical pixels.
    pub width: u32,
    /// Requested client-area height in physical pixels.
    pub height: u32,
    pub resizable: bool,
}

impl HostWindowConfig {
    /// Creates a window configuration with resizing enabled.
    #[must_use]
    pub fn new(title: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            title: title.into(),
            width,
            height,
            resizable: true,
        }
    }

    /// Validates dimensions before a platform backend allocates native resources.
    pub fn validate(&self) -> Result<(), HostError> {
        if self.width == 0 || self.height == 0 {
            return Err(HostError::InvalidDimensions);
        }
        let pixel_count = (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or(HostError::SizeOverflow)?;
        // Validate the actual CPU framebuffer byte footprint, not only its
        // pixel count. This catches dimensions that fit usize as pixels but
        // cannot be represented as an allocation of RGBA Color values.
        pixel_count
            .checked_mul(core::mem::size_of::<crate::Color>())
            .ok_or(HostError::SizeOverflow)?;
        Ok(())
    }
}

/// Identifies the platform adapter selected by the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostPlatform {
    Windows,
    Linux,
    MacOs,
    Headless,
    Other,
}

/// Capabilities exposed by a host backend.
///
/// Capabilities are queried at runtime: applications must not assume that
/// optional services such as clipboard access or IME are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostCapabilities {
    pub windowing: bool,
    pub high_dpi: bool,
    pub clipboard: bool,
    pub text_input: bool,
    pub ime: bool,
    pub resize: bool,
}

/// Normalized events delivered by a host backend.
///
/// Pointer and keyboard details are accumulated in `InputState`; lifecycle
/// and drawable changes are represented explicitly here.
#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    CloseRequested,
    /// Physical drawable dimensions; a zero dimension may indicate minimization.
    /// Backends may retain the last non-zero framebuffer while reporting this state.
    Resized {
        width: u32,
        height: u32,
    },
    ScaleFactorChanged {
        scale_factor: f64,
    },
    FocusChanged {
        focused: bool,
    },
    RedrawRequested,
    Input(InputState),
}

/// Errors shared by host API implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostError {
    InvalidDimensions,
    SizeOverflow,
    UnsupportedCapability,
}

impl core::fmt::Display for HostError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidDimensions => formatter.write_str("window dimensions must be non-zero"),
            Self::SizeOverflow => {
                formatter.write_str("window dimensions overflow addressable memory")
            }
            Self::UnsupportedCapability => formatter.write_str("host capability is not supported"),
        }
    }
}

impl std::error::Error for HostError {}

/// Common interface implemented by platform-specific desktop host backends.
///
/// Implementations own native windows and presentation resources. They must
/// preserve event order, report physical drawable dimensions, and reject or
/// explicitly report unsupported optional capabilities. The renderer only
/// supplies a completed CPU framebuffer; it never receives native handles.
pub trait HostBackend: Sized {
    type BackendError;

    /// Creates a backend using a validated common window configuration.
    fn create(config: HostWindowConfig) -> Result<Self, Self::BackendError>;

    /// Returns whether the host is still accepting events and presenting frames.
    fn is_open(&self) -> bool;

    /// Closes the host and releases its native or in-memory presentation resources.
    fn close(&mut self);

    /// Returns the host platform selected by this backend.
    fn platform(&self) -> HostPlatform;

    /// Returns optional services supported by the current host.
    fn capabilities(&self) -> HostCapabilities;

    /// Returns the current drawable size in physical pixels.
    ///
    /// A zero width or height means the drawable is temporarily unavailable,
    /// for example while a desktop window is minimized.
    fn drawable_size(&self) -> (u32, u32);

    /// Returns the current logical-to-physical scale factor.
    fn scale_factor(&self) -> f64;

    /// Appends pending normalized events without reordering them.
    fn poll_events(&mut self, events: &mut Vec<HostEvent>);

    /// Requests another frame from the native event loop.
    fn request_redraw(&mut self);

    /// Presents a completed CPU framebuffer to the native drawable.
    fn present(&mut self, framebuffer: &Framebuffer) -> Result<(), Self::BackendError>;
}

/// Returns the platform selected by Rust's target configuration.
#[must_use]
pub const fn current_host_platform() -> HostPlatform {
    #[cfg(target_os = "windows")]
    {
        HostPlatform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        HostPlatform::Linux
    }
    #[cfg(target_os = "macos")]
    {
        HostPlatform::MacOs
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        HostPlatform::Other
    }
}

#[cfg(test)]
mod tests {
    use super::{current_host_platform, HostError, HostPlatform, HostWindowConfig};

    #[test]
    fn window_config_accepts_valid_dimensions() {
        let config = HostWindowConfig::new("GhostEngine", 1280, 720);
        assert_eq!(config.validate(), Ok(()));
        assert!(config.resizable);
    }

    #[test]
    fn window_config_rejects_zero_dimensions() {
        let config = HostWindowConfig::new("GhostEngine", 0, 720);
        assert_eq!(config.validate(), Err(HostError::InvalidDimensions));
    }

    #[test]
    fn window_config_rejects_pixel_count_that_overflows_addressable_bytes() {
        let config = HostWindowConfig::new("oversized", u32::MAX, u32::MAX);
        assert_eq!(config.validate(), Err(HostError::SizeOverflow));
    }

    #[test]
    fn current_platform_matches_target() {
        #[cfg(target_os = "windows")]
        assert_eq!(current_host_platform(), HostPlatform::Windows);
        #[cfg(target_os = "linux")]
        assert_eq!(current_host_platform(), HostPlatform::Linux);
        #[cfg(target_os = "macos")]
        assert_eq!(current_host_platform(), HostPlatform::MacOs);
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        assert_eq!(current_host_platform(), HostPlatform::Other);
    }
}
