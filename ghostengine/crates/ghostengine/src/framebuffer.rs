use crate::Color;
use core::fmt;

/// Errors returned when creating a CPU framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramebufferError {
    ZeroDimension,
    SizeOverflow,
    AllocationFailed,
}

impl fmt::Display for FramebufferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimension => formatter.write_str("framebuffer dimensions must be non-zero"),
            Self::SizeOverflow => formatter.write_str("framebuffer pixel count overflows usize"),
            Self::AllocationFailed => formatter.write_str("could not allocate framebuffer storage"),
        }
    }
}

impl std::error::Error for FramebufferError {}

/// A tightly packed, row-major CPU framebuffer storing straight-alpha RGBA pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framebuffer {
    width: u32,
    height: u32,
    pixels: Vec<Color>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32, clear_color: Color) -> Result<Self, FramebufferError> {
        if width == 0 || height == 0 {
            return Err(FramebufferError::ZeroDimension);
        }

        let pixel_count = (width as usize)
            .checked_mul(height as usize)
            .ok_or(FramebufferError::SizeOverflow)?;

        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(pixel_count)
            .map_err(|_| FramebufferError::AllocationFailed)?;
        pixels.resize(pixel_count, clear_color);

        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixels(&self) -> &[Color] {
        &self.pixels
    }

    /// Returns mutable access to the tightly packed row-major pixel storage.
    ///
    /// This is useful for applications that produce pixels outside GhostEngine
    /// (for example, an emulator framebuffer) and want to copy them without
    /// issuing one bounds-checked set_pixel call per pixel.
    ///
    /// The slice always contains exactly width() * height() pixels.
    pub fn pixels_mut(&mut self) -> &mut [Color] {
        &mut self.pixels
    }

    /// Consumes the framebuffer and returns its dimensions and pixel allocation.
    ///
    /// This crate-private transfer lets other owned image types reuse the same
    /// allocation without copying the framebuffer contents.
    pub(crate) fn into_parts(self) -> (u32, u32, Vec<Color>) {
        (self.width, self.height, self.pixels)
    }

    pub fn clear(&mut self, color: Color) {
        self.pixels.fill(color);
    }

    /// Writes a pixel. Returns false when the coordinates are outside the target.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color) -> bool {
        let Some(index) = self.pixel_index(x, y) else {
            return false;
        };
        self.pixels[index] = color;
        true
    }

    /// Reads a pixel, returning None when the coordinates are outside the target.
    #[must_use]
    pub fn get_pixel(&self, x: u32, y: u32) -> Option<Color> {
        self.pixel_index(x, y).map(|index| self.pixels[index])
    }

    /// Returns a mutable slice for one framebuffer row.
    ///
    /// This crate-private primitive lets rasterizers write contiguous spans without
    /// repeating per-pixel coordinate checks.
    pub(crate) fn row_mut(&mut self, y: u32) -> Option<&mut [Color]> {
        if y >= self.height {
            return None;
        }
        let width = self.width as usize;
        let start = (y as usize).checked_mul(width)?;
        self.pixels.get_mut(start..start.checked_add(width)?)
    }

    fn pixel_index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }

        (y as usize)
            .checked_mul(self.width as usize)?
            .checked_add(x as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::{Framebuffer, FramebufferError};
    use crate::Color;

    #[test]
    fn creates_and_initializes_framebuffer() {
        let framebuffer = Framebuffer::new(3, 2, Color::BLUE).unwrap();
        assert_eq!(framebuffer.width(), 3);
        assert_eq!(framebuffer.height(), 2);
        assert_eq!(framebuffer.pixels().len(), 6);
        assert!(framebuffer
            .pixels()
            .iter()
            .all(|pixel| *pixel == Color::BLUE));
    }

    #[test]
    fn rejects_zero_dimensions() {
        assert_eq!(
            Framebuffer::new(0, 10, Color::BLACK),
            Err(FramebufferError::ZeroDimension)
        );
        assert_eq!(
            Framebuffer::new(10, 0, Color::BLACK),
            Err(FramebufferError::ZeroDimension)
        );
    }

    #[test]
    fn reads_and_writes_pixels_with_bounds_checks() {
        let mut framebuffer = Framebuffer::new(2, 2, Color::BLACK).unwrap();
        assert!(framebuffer.set_pixel(1, 1, Color::RED));
        assert_eq!(framebuffer.get_pixel(1, 1), Some(Color::RED));
        assert!(!framebuffer.set_pixel(2, 1, Color::RED));
        assert_eq!(framebuffer.get_pixel(2, 1), None);
    }

    #[test]
    fn mutable_pixel_slice_updates_row_major_storage() {
        let mut framebuffer = Framebuffer::new(2, 2, Color::BLACK).unwrap();
        framebuffer.pixels_mut().copy_from_slice(&[
            Color::RED,
            Color::GREEN,
            Color::BLUE,
            Color::WHITE,
        ]);

        assert_eq!(framebuffer.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(framebuffer.get_pixel(1, 0), Some(Color::GREEN));
        assert_eq!(framebuffer.get_pixel(0, 1), Some(Color::BLUE));
        assert_eq!(framebuffer.get_pixel(1, 1), Some(Color::WHITE));
    }

    #[test]
    fn clear_replaces_every_pixel() {
        let mut framebuffer = Framebuffer::new(2, 3, Color::RED).unwrap();
        framebuffer.clear(Color::WHITE);
        assert!(framebuffer
            .pixels()
            .iter()
            .all(|pixel| *pixel == Color::WHITE));
    }
}
