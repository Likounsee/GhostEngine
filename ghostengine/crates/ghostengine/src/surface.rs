use crate::{Color, Framebuffer};

/// Byte layout used when exporting a CPU framebuffer to an external surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat { Rgb8, Bgr8, Rgba8, Bgra8 }

impl PixelFormat {
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self { Self::Rgb8 | Self::Bgr8 => 3, Self::Rgba8 | Self::Bgra8 => 4 }
    }
    const fn has_alpha(self) -> bool { matches!(self, Self::Rgba8 | Self::Bgra8) }
}

/// Interpretation of the alpha channel in an exported surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlphaMode { Straight, Premultiplied, Opaque }

/// Transfer function applied to RGB channels during export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpace { Srgb, LinearSrgb }

/// Describes a caller-owned byte surface. Row stride is measured in bytes and
/// may exceed the minimum row size to accommodate native alignment/padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SurfaceDescriptor {
    pub width: u32,
    pub height: u32,
    pub row_stride: usize,
    pub pixel_format: PixelFormat,
    pub alpha_mode: AlphaMode,
    pub color_space: ColorSpace,
}

impl SurfaceDescriptor {
    #[must_use]
    pub const fn tightly_packed(width: u32, height: u32, pixel_format: PixelFormat, alpha_mode: AlphaMode, color_space: ColorSpace) -> Self {
        Self { width, height, row_stride: width as usize * pixel_format.bytes_per_pixel(), pixel_format, alpha_mode, color_space }
    }

    pub fn minimum_row_bytes(self) -> Result<usize, SurfaceError> {
        (self.width as usize).checked_mul(self.pixel_format.bytes_per_pixel()).ok_or(SurfaceError::SizeOverflow)
    }

    pub fn required_buffer_len(self) -> Result<usize, SurfaceError> {
        if self.width == 0 || self.height == 0 { return Err(SurfaceError::ZeroDimension); }
        if self.row_stride < self.minimum_row_bytes()? { return Err(SurfaceError::StrideTooSmall); }
        self.row_stride.checked_mul(self.height as usize).ok_or(SurfaceError::SizeOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceError { ZeroDimension, SizeOverflow, StrideTooSmall, BufferTooSmall, DimensionMismatch }

impl core::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZeroDimension => f.write_str("surface dimensions must be non-zero"),
            Self::SizeOverflow => f.write_str("surface byte size overflows usize"),
            Self::StrideTooSmall => f.write_str("surface row stride is smaller than one row"),
            Self::BufferTooSmall => f.write_str("destination buffer is smaller than the surface"),
            Self::DimensionMismatch => f.write_str("surface and framebuffer dimensions differ"),
        }
    }
}
impl std::error::Error for SurfaceError {}

impl Framebuffer {
    /// Exports pixels to caller-owned RGB/BGR or RGBA/BGRA memory.
    /// Supports padded rows, alpha conversion and sRGB/linear-sRGB output.
    /// Padding bytes are preserved. Validation errors leave the destination unchanged.
    pub fn write_to_surface(&self, destination: &mut [u8], descriptor: SurfaceDescriptor) -> Result<(), SurfaceError> {
        let required_len = descriptor.required_buffer_len()?;
        if descriptor.width != self.width() || descriptor.height != self.height() { return Err(SurfaceError::DimensionMismatch); }
        if destination.len() < required_len { return Err(SurfaceError::BufferTooSmall); }

        let bpp = descriptor.pixel_format.bytes_per_pixel();
        for (y, row) in self.pixels().chunks_exact(self.width() as usize).enumerate() {
            let row_start = y * descriptor.row_stride;
            for (x, pixel) in row.iter().copied().enumerate() {
                let [r, g, b, a] = convert_pixel(pixel, descriptor);
                let offset = row_start + x * bpp;
                match descriptor.pixel_format {
                    PixelFormat::Rgb8 => destination[offset..offset + 3].copy_from_slice(&[r, g, b]),
                    PixelFormat::Bgr8 => destination[offset..offset + 3].copy_from_slice(&[b, g, r]),
                    PixelFormat::Rgba8 => destination[offset..offset + 4].copy_from_slice(&[r, g, b, a]),
                    PixelFormat::Bgra8 => destination[offset..offset + 4].copy_from_slice(&[b, g, r, a]),
                }
            }
        }
        Ok(())
    }
}

fn convert_pixel(pixel: Color, descriptor: SurfaceDescriptor) -> [u8; 4] {
    let mut r = pixel.r; let mut g = pixel.g; let mut b = pixel.b;
    let a = if descriptor.alpha_mode == AlphaMode::Opaque || !descriptor.pixel_format.has_alpha() { 255 } else { pixel.a };
    if descriptor.alpha_mode == AlphaMode::Premultiplied && descriptor.pixel_format.has_alpha() {
        r = multiply_alpha(r, a); g = multiply_alpha(g, a); b = multiply_alpha(b, a);
    }
    if descriptor.color_space == ColorSpace::LinearSrgb {
        r = srgb_to_linear(r); g = srgb_to_linear(g); b = srgb_to_linear(b);
    }
    [r, g, b, a]
}

fn multiply_alpha(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}

fn srgb_to_linear(channel: u8) -> u8 {
    let value = f64::from(channel) / 255.0;
    let linear = if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) };
    (linear.mul_add(255.0, 0.5).floor() as u8)
}

#[cfg(test)]
mod tests {
    use super::{AlphaMode, ColorSpace, PixelFormat, SurfaceDescriptor, SurfaceError};
    use crate::{Color, Framebuffer};

    #[test]
    fn exports_bgra_with_padded_rows_without_touching_padding() {
        let mut frame = Framebuffer::new(1, 2, Color::TRANSPARENT).unwrap();
        frame.set_pixel(0, 0, Color::rgba(10, 20, 30, 40));
        frame.set_pixel(0, 1, Color::rgba(50, 60, 70, 80));
        let descriptor = SurfaceDescriptor { width: 1, height: 2, row_stride: 8, pixel_format: PixelFormat::Bgra8, alpha_mode: AlphaMode::Straight, color_space: ColorSpace::Srgb };
        let mut bytes = [0xCC; 16];
        frame.write_to_surface(&mut bytes, descriptor).unwrap();
        assert_eq!(&bytes[..4], &[30, 20, 10, 40]);
        assert_eq!(&bytes[4..8], &[0xCC; 4]);
        assert_eq!(&bytes[8..12], &[70, 60, 50, 80]);
        assert_eq!(&bytes[12..16], &[0xCC; 4]);
    }

    #[test]
    fn exports_premultiplied_alpha() {
        let frame = Framebuffer::new(1, 1, Color::rgba(200, 100, 50, 128)).unwrap();
        let descriptor = SurfaceDescriptor::tightly_packed(1, 1, PixelFormat::Rgba8, AlphaMode::Premultiplied, ColorSpace::Srgb);
        let mut bytes = [0; 4];
        frame.write_to_surface(&mut bytes, descriptor).unwrap();
        assert_eq!(bytes, [100, 50, 25, 128]);
    }

    #[test]
    fn opaque_rgb_export_discards_alpha() {
        let frame = Framebuffer::new(1, 1, Color::rgba(1, 2, 3, 4)).unwrap();
        let descriptor = SurfaceDescriptor::tightly_packed(1, 1, PixelFormat::Rgb8, AlphaMode::Straight, ColorSpace::Srgb);
        let mut bytes = [0; 3];
        frame.write_to_surface(&mut bytes, descriptor).unwrap();
        assert_eq!(bytes, [1, 2, 3]);
    }

    #[test]
    fn rejects_invalid_stride_and_short_buffer_before_writing() {
        let frame = Framebuffer::new(2, 1, Color::RED).unwrap();
        let invalid = SurfaceDescriptor { width: 2, height: 1, row_stride: 5, pixel_format: PixelFormat::Rgba8, alpha_mode: AlphaMode::Straight, color_space: ColorSpace::Srgb };
        let mut bytes = [0xAA; 8];
        assert_eq!(frame.write_to_surface(&mut bytes, invalid), Err(SurfaceError::StrideTooSmall));
        assert_eq!(bytes, [0xAA; 8]);
        let valid = SurfaceDescriptor::tightly_packed(2, 1, PixelFormat::Rgba8, AlphaMode::Straight, ColorSpace::Srgb);
        assert_eq!(frame.write_to_surface(&mut bytes[..7], valid), Err(SurfaceError::BufferTooSmall));
        assert_eq!(bytes, [0xAA; 8]);
    }

    #[test]
    fn rejects_dimension_mismatch_without_writing() {
        let frame = Framebuffer::new(1, 1, Color::RED).unwrap();
        let descriptor = SurfaceDescriptor::tightly_packed(2, 1, PixelFormat::Rgba8, AlphaMode::Straight, ColorSpace::Srgb);
        let mut bytes = [0xAA; 8];
        assert_eq!(frame.write_to_surface(&mut bytes, descriptor), Err(SurfaceError::DimensionMismatch));
        assert_eq!(bytes, [0xAA; 8]);
    }
}
