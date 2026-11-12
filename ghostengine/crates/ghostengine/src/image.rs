//! Optional PNG and JPEG decoding into GhostEngine's CPU texture format.

use std::{error::Error, fmt, io::Cursor};

use ::image::{io::Limits, ImageFormat};

use crate::{Color, Texture, TextureError};

const MAX_IMAGE_DIMENSION: u32 = 4096;
const MAX_DECODER_ALLOCATION: u64 = 64 * 1024 * 1024;

/// Errors returned while decoding an image into a GhostEngine texture.
#[derive(Debug)]
pub enum ImageDecodeError {
    /// The encoded bytes did not identify a supported image format.
    UnknownFormat,
    /// The format is recognized but its decoder is not enabled by this feature.
    UnsupportedFormat(ImageFormat),
    /// The reader could not inspect or decode the encoded bytes.
    Reader(std::io::Error),
    Decode(::image::ImageError),
    Texture(TextureError),
}

impl fmt::Display for ImageDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFormat => formatter.write_str("could not identify image format"),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "image format {format:?} is not enabled")
            }
            Self::Reader(error) => write!(formatter, "could not read image header: {error}"),
            Self::Decode(error) => write!(formatter, "could not decode image: {error}"),
            Self::Texture(error) => write!(formatter, "invalid decoded texture: {error:?}"),
        }
    }
}

impl Error for ImageDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Reader(error) => Some(error),
            Self::Decode(error) => Some(error),
            Self::UnknownFormat | Self::UnsupportedFormat(_) | Self::Texture(_) => None,
        }
    }
}

/// Decodes PNG or JPEG bytes into an immutable, row-major RGBA texture.
///
/// This adapter is available only with the `image-decoding` feature. The decoder
/// rejects dimensions above 4096 pixels and applies a 64 MiB decoder allocation
/// limit before converting the image to GhostEngine's `Color` storage.
pub fn decode_texture(encoded: &[u8]) -> Result<Texture, ImageDecodeError> {
    let reader = ::image::io::Reader::new(Cursor::new(encoded))
        .with_guessed_format()
        .map_err(ImageDecodeError::Reader)?;
    let format = reader.format().ok_or(ImageDecodeError::UnknownFormat)?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
        return Err(ImageDecodeError::UnsupportedFormat(format));
    }

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODER_ALLOCATION);

    let mut reader = reader;
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(ImageDecodeError::Decode)?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = rgba
        .pixels()
        .map(|pixel| Color::rgba(pixel[0], pixel[1], pixel[2], pixel[3]))
        .collect();

    Texture::new(width, height, pixels).map_err(ImageDecodeError::Texture)
}

#[cfg(test)]
mod tests {
    use super::{decode_texture, ImageDecodeError};
    use crate::Color;
    use ::image::ImageEncoder;

    fn encode_png() -> Vec<u8> {
        let mut encoded = Vec::new();
        ::image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(
                &[255, 20, 10, 255, 5, 40, 230, 128],
                2,
                1,
                ::image::ColorType::Rgba8.into(),
            )
            .unwrap();
        encoded
    }

    #[test]
    fn decodes_png_pixels_and_alpha_into_texture_storage() {
        let texture = decode_texture(&encode_png()).unwrap();

        assert_eq!((texture.width(), texture.height()), (2, 1));
        assert_eq!(texture.get_pixel(0, 0), Some(Color::rgba(255, 20, 10, 255)));
        assert_eq!(texture.get_pixel(1, 0), Some(Color::rgba(5, 40, 230, 128)));
    }

    #[test]
    fn decodes_jpeg_into_opaque_texture_pixels() {
        let mut encoded = Vec::new();
        ::image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, 100)
            .encode(
                &[250, 30, 20, 10, 40, 230],
                2,
                1,
                ::image::ColorType::Rgb8.into(),
            )
            .unwrap();

        let texture = decode_texture(&encoded).unwrap();

        assert_eq!((texture.width(), texture.height()), (2, 1));
        assert_eq!(texture.get_pixel(0, 0).unwrap().a, 255);
        assert_eq!(texture.get_pixel(1, 0).unwrap().a, 255);
    }

    #[test]
    fn rejects_unknown_and_recognized_unsupported_formats() {
        assert!(matches!(
            decode_texture(b"not an image"),
            Err(ImageDecodeError::UnknownFormat)
        ));

        let ppm = b"P3\n1 1\n255\n0 0 0\n";
        assert!(matches!(
            decode_texture(ppm),
            Err(ImageDecodeError::UnsupportedFormat(
                ::image::ImageFormat::Pnm
            ))
        ));
    }

    #[test]
    fn rejects_images_over_the_dimension_limit() {
        let mut encoded = Vec::new();
        let width = super::MAX_IMAGE_DIMENSION + 1;
        let pixels = vec![0; width as usize * 4];
        ::image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(&pixels, width, 1, ::image::ColorType::Rgba8.into())
            .unwrap();

        assert!(matches!(
            decode_texture(&encoded),
            Err(ImageDecodeError::Decode(::image::ImageError::Limits(_)))
        ));
    }
}
