//! Optional common-format decoding into GhostEngine's CPU texture format.

use std::{error::Error, fmt, io::Cursor};

use ::image::{io::Limits, ImageFormat};

use crate::{Color, Texture, TextureCache, TextureCacheError, TextureError, TextureHandle};

const MAX_IMAGE_DIMENSION: u32 = 4096;
const MAX_DECODER_ALLOCATION: u64 = 64 * 1024 * 1024;

/// Resource limits applied while decoding an image.
///
/// Width and height limits are strict. The decoder allocation limit is a
/// best-effort bound provided by the selected codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDecodeOptions {
    pub max_image_width: Option<u32>,
    pub max_image_height: Option<u32>,
    pub max_decoder_allocation_bytes: Option<u64>,
}

impl Default for ImageDecodeOptions {
    fn default() -> Self {
        Self {
            max_image_width: Some(MAX_IMAGE_DIMENSION),
            max_image_height: Some(MAX_IMAGE_DIMENSION),
            max_decoder_allocation_bytes: Some(MAX_DECODER_ALLOCATION),
        }
    }
}

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
    /// The image codec panicked while decoding the supplied bytes.
    ///
    /// This can be recovered only when the process uses unwinding panics.
    DecoderPanicked,
    /// The decoded pixel buffer could not be allocated.
    AllocationFailed,
    Texture(TextureError),
    Cache(TextureCacheError),
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
            Self::DecoderPanicked => {
                formatter.write_str("image decoder panicked while processing encoded input")
            }
            Self::AllocationFailed => {
                formatter.write_str("could not allocate decoded image pixel storage")
            }
            Self::Texture(error) => write!(formatter, "invalid decoded texture: {error:?}"),
            Self::Cache(error) => write!(formatter, "could not cache decoded texture: {error:?}"),
        }
    }
}

impl Error for ImageDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Reader(error) => Some(error),
            Self::Decode(error) => Some(error),
            Self::UnknownFormat
            | Self::UnsupportedFormat(_)
            | Self::DecoderPanicked
            | Self::AllocationFailed
            | Self::Texture(_)
            | Self::Cache(_) => None,
        }
    }
}

/// Decodes BMP, JPEG, PNG, or PNM bytes into an immutable, row-major RGBA texture.
///
/// This adapter is available only with the `image-decoding` feature. The decoder
/// uses the default decode limits before converting the image to GhostEngine's
/// `Color` storage.
pub fn decode_texture(encoded: &[u8]) -> Result<Texture, ImageDecodeError> {
    decode_texture_with_options(encoded, ImageDecodeOptions::default())
}

/// Decodes an image and inserts it into a bounded texture cache.
pub fn decode_texture_into_cache(
    encoded: &[u8],
    cache: &mut TextureCache,
) -> Result<TextureHandle, ImageDecodeError> {
    decode_texture_with_options_into_cache(encoded, ImageDecodeOptions::default(), cache)
}

/// Decodes an image with custom limits and inserts it into a bounded texture cache.
///
/// If the decoded texture exceeds the cache budget, insertion fails without
/// evicting existing cached textures.
pub fn decode_texture_with_options_into_cache(
    encoded: &[u8],
    options: ImageDecodeOptions,
    cache: &mut TextureCache,
) -> Result<TextureHandle, ImageDecodeError> {
    let texture = decode_texture_with_options(encoded, options)?;
    cache.insert(texture).map_err(ImageDecodeError::Cache)
}

/// Decodes image bytes with limits supplied by the calling application.
///
/// Set any limit to `None` to leave that dimension or allocation unbounded.
/// Codec panics are converted to `ImageDecodeError::DecoderPanicked` when the
/// process uses unwinding panics; `panic=abort` builds cannot recover from panics.
pub fn decode_texture_with_options(
    encoded: &[u8],
    options: ImageDecodeOptions,
) -> Result<Texture, ImageDecodeError> {
    let reader = ::image::io::Reader::new(Cursor::new(encoded))
        .with_guessed_format()
        .map_err(ImageDecodeError::Reader)?;
    let format = reader.format().ok_or(ImageDecodeError::UnknownFormat)?;
    if !matches!(
        format,
        ImageFormat::Bmp | ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Pnm
    ) {
        return Err(ImageDecodeError::UnsupportedFormat(format));
    }

    let mut limits = Limits::default();
    limits.max_image_width = options.max_image_width;
    limits.max_image_height = options.max_image_height;
    limits.max_alloc = options.max_decoder_allocation_bytes;

    let mut reader = reader;
    reader.limits(limits);
    let rgba = decode_with_panic_guard(|| {
        reader.decode().map(|decoded| decoded.into_rgba8())
    })?;
    let (width, height) = rgba.dimensions();
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(ImageDecodeError::AllocationFailed)?;
    let raw_pixels = rgba.into_raw();
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(pixel_count)
        .map_err(|_| ImageDecodeError::AllocationFailed)?;
    pixels.extend(
        raw_pixels
            .chunks_exact(4)
            .map(|pixel| Color::rgba(pixel[0], pixel[1], pixel[2], pixel[3])),
    );

    Texture::new(width, height, pixels).map_err(ImageDecodeError::Texture)
}

fn decode_with_panic_guard<T>(
    decode: impl FnOnce() -> Result<T, ::image::ImageError>,
) -> Result<T, ImageDecodeError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(decode))
        .map_err(|_| ImageDecodeError::DecoderPanicked)?
        .map_err(ImageDecodeError::Decode)
}

#[cfg(test)]
mod tests {
    use super::{
        decode_texture, decode_texture_with_options, ImageDecodeError, ImageDecodeOptions,
    };
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
    fn codec_panics_are_returned_as_typed_decode_errors() {
        let result = super::decode_with_panic_guard(
            || -> Result<(), ::image::ImageError> {
                std::panic::resume_unwind(Box::new("simulated codec panic"))
            },
        );

        assert!(matches!(result, Err(ImageDecodeError::DecoderPanicked)));
    }

    #[test]
    fn decodes_png_pixels_and_alpha_into_texture_storage() {
        let texture = decode_texture(&encode_png()).unwrap();

        assert_eq!((texture.width(), texture.height()), (2, 1));
        assert_eq!(texture.get_pixel(0, 0), Some(Color::rgba(255, 20, 10, 255)));
        assert_eq!(texture.get_pixel(1, 0), Some(Color::rgba(5, 40, 230, 128)));
    }

    #[test]
    fn decodes_png_directly_into_texture_cache() {
        let mut cache = crate::TextureCache::new(64);
        let handle = super::decode_texture_into_cache(&encode_png(), &mut cache).unwrap();

        let texture = cache.get(handle).unwrap();
        assert_eq!((texture.width(), texture.height()), (2, 1));
        assert_eq!(texture.get_pixel(0, 0), Some(Color::rgba(255, 20, 10, 255)));
        assert_eq!(cache.stats().entries, 1);
        assert_eq!(cache.stats().used_bytes, 8);
    }

    #[test]
    fn cache_budget_error_does_not_evict_existing_textures() {
        let mut cache = crate::TextureCache::new(7);
        let existing = cache
            .insert(crate::Texture::new(1, 1, vec![Color::RED]).unwrap())
            .unwrap();

        assert!(matches!(
            super::decode_texture_into_cache(&encode_png(), &mut cache),
            Err(ImageDecodeError::Cache(
                crate::TextureCacheError::TextureExceedsBudget { .. }
            ))
        ));
        assert!(cache.peek(existing).is_some());
        assert_eq!(cache.stats().entries, 1);
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
    fn decodes_bmp_pixels() {
        let mut encoded = Vec::new();
        ::image::codecs::bmp::BmpEncoder::new(&mut encoded)
            .encode(&[25, 50, 225], 1, 1, ::image::ColorType::Rgb8.into())
            .unwrap();

        let texture = decode_texture(&encoded).unwrap();

        assert_eq!(texture.get_pixel(0, 0), Some(Color::rgb(25, 50, 225)));
    }

    #[test]
    fn decodes_portable_pixmap_pixels() {
        let texture = decode_texture(b"P3\n1 1\n255\n12 34 56\n").unwrap();

        assert_eq!(texture.get_pixel(0, 0), Some(Color::rgb(12, 34, 56)));
    }

    #[test]
    fn rejects_unknown_and_recognized_unsupported_formats() {
        assert!(matches!(
            decode_texture(b"not an image"),
            Err(ImageDecodeError::UnknownFormat)
        ));

        assert!(matches!(
            decode_texture(b"GIF89a"),
            Err(ImageDecodeError::UnsupportedFormat(
                ::image::ImageFormat::Gif
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

    #[test]
    fn custom_limits_can_allow_a_larger_image() {
        let mut encoded = Vec::new();
        let width = super::MAX_IMAGE_DIMENSION + 1;
        let pixels = vec![0; width as usize * 4];
        ::image::codecs::png::PngEncoder::new(&mut encoded)
            .write_image(&pixels, width, 1, ::image::ColorType::Rgba8.into())
            .unwrap();

        let texture = decode_texture_with_options(
            &encoded,
            ImageDecodeOptions {
                max_image_width: Some(width),
                ..ImageDecodeOptions::default()
            },
        )
        .unwrap();

        assert_eq!(texture.width(), width);
    }
}
