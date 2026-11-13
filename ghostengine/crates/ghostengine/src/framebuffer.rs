use crate::{Color, Rect};
use core::fmt;

/// Errors returned when creating a CPU framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramebufferError {
    ZeroDimension,
    SizeOverflow,
    AllocationFailed,
    PixelCountMismatch,
}

impl fmt::Display for FramebufferError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDimension => formatter.write_str("framebuffer dimensions must be non-zero"),
            Self::SizeOverflow => formatter.write_str("framebuffer pixel count overflows usize"),
            Self::AllocationFailed => formatter.write_str("could not allocate framebuffer storage"),
            Self::PixelCountMismatch => formatter.write_str("source pixel count does not match framebuffer dimensions"),
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

    /// Copies a complete row-major image into this framebuffer without reallocating.
    ///
    /// The source must contain exactly width() * height() pixels. On a size
    /// mismatch, the framebuffer is left unchanged. This is a convenient
    /// frame-transfer operation for producers such as emulator video cores.
    pub fn copy_pixels_from(&mut self, source: &[Color]) -> Result<(), FramebufferError> {
        if source.len() != self.pixels.len() {
            return Err(FramebufferError::PixelCountMismatch);
        }
        self.pixels.copy_from_slice(source);
        Ok(())
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

    /// Fills the portion of a rectangle that intersects this framebuffer.
    ///
    /// Rows are written as contiguous slices, avoiding one bounds check and
    /// index calculation per pixel. Rectangles extending beyond the target are
    /// safely clipped; empty rectangles are ignored.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let Some(clipped) = rect.intersection(Rect::new(0, 0, self.width, self.height)) else {
            return;
        };
        let left = clipped.x as usize;
        let clipped_width = clipped.width as usize;

        // A full-width rectangle is one contiguous region in row-major storage.
        // Fill it in a single slice operation instead of visiting each row.
        if left == 0 && clipped_width == self.width as usize {
            let start = clipped.y as usize * self.width as usize;
            let end = start + clipped.height as usize * self.width as usize;
            self.pixels[start..end].fill(color);
            return;
        }

        let right = left + clipped_width;
        for y in clipped.y..clipped.y + clipped.height {
            if let Some(row) = self.row_mut(y) {
                row[left..right].fill(color);
            }
        }
    }

    /// Resizes the framebuffer and initializes every pixel to the supplied color.
    ///
    /// The existing pixel allocation is retained whenever its capacity is large
    /// enough. This is useful for applications such as emulators whose output
    /// dimensions can change between frames or display modes.
    ///
    /// If additional storage is required, allocation is attempted before the
    /// dimensions or existing pixels are changed.
    pub fn resize_and_clear(
        &mut self,
        width: u32,
        height: u32,
        clear_color: Color,
    ) -> Result<(), FramebufferError> {
        if width == 0 || height == 0 {
            return Err(FramebufferError::ZeroDimension);
        }

        let pixel_count = (width as usize)
            .checked_mul(height as usize)
            .ok_or(FramebufferError::SizeOverflow)?;

        if pixel_count > self.pixels.len() {
            self.pixels
                .try_reserve_exact(pixel_count - self.pixels.len())
                .map_err(|_| FramebufferError::AllocationFailed)?;
        }

        self.pixels.resize(pixel_count, clear_color);
        self.pixels.fill(clear_color);
        self.width = width;
        self.height = height;
        Ok(())
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
    fn copy_pixels_from_replaces_frame_without_reallocating() {
        let mut framebuffer = Framebuffer::new(2, 2, Color::BLACK).unwrap();
        let allocation = framebuffer.pixels().as_ptr();
        let source = [Color::RED, Color::GREEN, Color::BLUE, Color::WHITE];

        framebuffer.copy_pixels_from(&source).unwrap();

        assert_eq!(framebuffer.pixels(), &source);
        assert_eq!(framebuffer.pixels().as_ptr(), allocation);
    }

    #[test]
    fn copy_pixels_from_rejects_wrong_size_without_mutating_frame() {
        let mut framebuffer = Framebuffer::new(2, 2, Color::RED).unwrap();
        let original = framebuffer.clone();

        assert_eq!(
            framebuffer.copy_pixels_from(&[Color::BLUE]),
            Err(FramebufferError::PixelCountMismatch)
        );
        assert_eq!(framebuffer, original);
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
    fn fill_rect_clips_to_framebuffer_and_preserves_other_pixels() {
        let mut framebuffer = Framebuffer::new(4, 3, Color::BLACK).unwrap();
        framebuffer.fill_rect(crate::Rect::new(2, 1, u32::MAX, 2), Color::RED);

        for y in 0..3 {
            for x in 0..4 {
                let expected = if x >= 2 && y >= 1 {
                    Color::RED
                } else {
                    Color::BLACK
                };
                assert_eq!(framebuffer.get_pixel(x, y), Some(expected));
            }
        }
    }

    #[test]
    fn fill_rect_ignores_empty_and_outside_rectangles() {
        let mut framebuffer = Framebuffer::new(3, 2, Color::BLUE).unwrap();
        framebuffer.fill_rect(crate::Rect::new(1, 1, 0, 1), Color::RED);
        framebuffer.fill_rect(crate::Rect::new(u32::MAX, 0, 2, 2), Color::RED);
        assert!(framebuffer.pixels().iter().all(|pixel| *pixel == Color::BLUE));
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

    #[test]
    fn resize_and_clear_reuses_existing_pixel_allocation() {
        let mut framebuffer = Framebuffer::new(4, 4, Color::RED).unwrap();
        let allocation = framebuffer.pixels().as_ptr();

        framebuffer.resize_and_clear(2, 2, Color::BLUE).unwrap();
        framebuffer.resize_and_clear(4, 4, Color::GREEN).unwrap();

        assert_eq!(framebuffer.width(), 4);
        assert_eq!(framebuffer.height(), 4);
        assert_eq!(framebuffer.pixels().as_ptr(), allocation);
        assert!(framebuffer
            .pixels()
            .iter()
            .all(|pixel| *pixel == Color::GREEN));
    }

    #[test]
    fn failed_resize_keeps_framebuffer_unchanged() {
        let mut framebuffer = Framebuffer::new(2, 2, Color::RED).unwrap();
        let original = framebuffer.clone();

        assert_eq!(
            framebuffer.resize_and_clear(0, 4, Color::BLUE),
            Err(FramebufferError::ZeroDimension)
        );
        assert_eq!(framebuffer, original);
    }
}
