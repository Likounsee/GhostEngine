use ghostengine::{
    AlphaMode, Color, ColorSpace, Framebuffer, PixelFormat, SurfaceDescriptor, SurfaceError,
};

const FORMATS: [PixelFormat; 4] = [
    PixelFormat::Rgb8,
    PixelFormat::Bgr8,
    PixelFormat::Rgba8,
    PixelFormat::Bgra8,
];
const ALPHA_MODES: [AlphaMode; 3] = [
    AlphaMode::Straight,
    AlphaMode::Premultiplied,
    AlphaMode::Opaque,
];

fn expected_channels(format: PixelFormat, rgba: [u8; 4]) -> Vec<u8> {
    let [r, g, b, a] = rgba;
    match format {
        PixelFormat::Rgb8 => vec![r, g, b],
        PixelFormat::Bgr8 => vec![b, g, r],
        PixelFormat::Rgba8 => vec![r, g, b, a],
        PixelFormat::Bgra8 => vec![b, g, r, a],
    }
}

#[test]
fn export_matrix_covers_every_pixel_format_and_alpha_mode() {
    let source = Color::rgba(100, 50, 20, 128);
    let frame = Framebuffer::new(1, 1, source).unwrap();

    for format in FORMATS {
        for alpha_mode in ALPHA_MODES {
            let descriptor =
                SurfaceDescriptor::tightly_packed(1, 1, format, alpha_mode, ColorSpace::Srgb);
            let mut bytes = vec![0xCD; format.bytes_per_pixel()];
            frame.write_to_surface(&mut bytes, descriptor).unwrap();

            let alpha = match (format, alpha_mode) {
                (PixelFormat::Rgb8 | PixelFormat::Bgr8, _) | (_, AlphaMode::Opaque) => 255,
                (_, AlphaMode::Straight) => 128,
                (_, AlphaMode::Premultiplied) => 128,
            };
            let channels = match alpha_mode {
                AlphaMode::Straight | AlphaMode::Opaque => [100, 50, 20, alpha],
                AlphaMode::Premultiplied if format.bytes_per_pixel() == 4 => [50, 25, 10, alpha],
                AlphaMode::Premultiplied => [100, 50, 20, 255],
            };
            assert_eq!(
                bytes,
                expected_channels(format, channels),
                "format={format:?}, alpha={alpha_mode:?}"
            );
        }
    }
}

#[test]
fn import_matrix_covers_channel_order_alpha_and_zero_alpha() {
    for format in FORMATS {
        for alpha_mode in ALPHA_MODES {
            let has_alpha = matches!(format, PixelFormat::Rgba8 | PixelFormat::Bgra8);
            let descriptor =
                SurfaceDescriptor::tightly_packed(1, 1, format, alpha_mode, ColorSpace::Srgb);
            let rgba = match (has_alpha, alpha_mode) {
                (true, AlphaMode::Premultiplied) => [50, 25, 10, 128],
                (true, AlphaMode::Straight) => [100, 50, 20, 128],
                (true, AlphaMode::Opaque) => [100, 50, 20, 0],
                (false, _) => [100, 50, 20, 0],
            };
            let bytes = expected_channels(format, rgba);
            let mut frame = Framebuffer::new(1, 1, Color::MAGENTA).unwrap();
            frame.read_from_surface(&bytes, descriptor).unwrap();

            let expected_alpha = if has_alpha && alpha_mode != AlphaMode::Opaque {
                128
            } else {
                255
            };
            assert_eq!(
                frame.get_pixel(0, 0),
                Some(Color::rgba(100, 50, 20, expected_alpha)),
                "format={format:?}, alpha={alpha_mode:?}"
            );
        }
    }

    for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
        let descriptor = SurfaceDescriptor::tightly_packed(
            1,
            1,
            format,
            AlphaMode::Premultiplied,
            ColorSpace::Srgb,
        );
        let bytes = expected_channels(format, [0, 0, 0, 0]);
        let mut frame = Framebuffer::new(1, 1, Color::WHITE).unwrap();
        frame.read_from_surface(&bytes, descriptor).unwrap();
        assert_eq!(frame.get_pixel(0, 0), Some(Color::TRANSPARENT));
    }
}

#[test]
fn padded_rows_preserve_export_padding_and_ignore_import_padding() {
    let descriptor = SurfaceDescriptor {
        width: 2,
        height: 2,
        row_stride: 12,
        pixel_format: PixelFormat::Rgba8,
        alpha_mode: AlphaMode::Straight,
        color_space: ColorSpace::Srgb,
    };
    let mut frame = Framebuffer::new(2, 2, Color::TRANSPARENT).unwrap();
    frame.set_pixel(0, 0, Color::rgba(1, 2, 3, 4));
    frame.set_pixel(1, 0, Color::rgba(5, 6, 7, 8));
    frame.set_pixel(0, 1, Color::rgba(9, 10, 11, 12));
    frame.set_pixel(1, 1, Color::rgba(13, 14, 15, 16));

    let mut output = vec![0xA5; 20];
    frame.write_to_surface(&mut output, descriptor).unwrap();
    assert_eq!(&output[8..12], &[0xA5; 4]);
    assert_eq!(&output[16..20], &[0xA5; 4]);

    let mut imported = Framebuffer::new(2, 2, Color::BLACK).unwrap();
    imported.read_from_surface(&output, descriptor).unwrap();
    assert_eq!(imported, frame);

    let mut source_with_poisoned_padding = output;
    source_with_poisoned_padding[8..12].fill(0xFF);
    let mut imported_again = Framebuffer::new(2, 2, Color::BLACK).unwrap();
    imported_again
        .read_from_surface(&source_with_poisoned_padding, descriptor)
        .unwrap();
    assert_eq!(imported_again, frame);
}

#[test]
fn every_validation_failure_is_transactional_for_import_and_export() {
    let mut frame = Framebuffer::new(2, 1, Color::rgba(10, 20, 30, 40)).unwrap();
    let before = frame.clone();
    let valid = SurfaceDescriptor::tightly_packed(
        2,
        1,
        PixelFormat::Rgba8,
        AlphaMode::Straight,
        ColorSpace::Srgb,
    );

    let mut short_output = [0xCC; 7];
    assert_eq!(
        frame.write_to_surface(&mut short_output, valid),
        Err(SurfaceError::BufferTooSmall)
    );
    assert_eq!(short_output, [0xCC; 7]);

    let invalid_stride = SurfaceDescriptor {
        row_stride: 7,
        ..valid
    };
    let mut output = [0xCC; 8];
    assert_eq!(
        frame.write_to_surface(&mut output, invalid_stride),
        Err(SurfaceError::StrideTooSmall)
    );
    assert_eq!(output, [0xCC; 8]);

    assert_eq!(
        frame.read_from_surface(&[1, 2, 3], valid),
        Err(SurfaceError::BufferTooSmall)
    );
    assert_eq!(frame, before);

    let mismatch = SurfaceDescriptor { width: 1, ..valid };
    assert_eq!(
        frame.read_from_surface(&[1, 2, 3, 4], mismatch),
        Err(SurfaceError::DimensionMismatch)
    );
    assert_eq!(frame, before);

    let zero = SurfaceDescriptor { width: 0, ..valid };
    assert_eq!(
        frame.read_from_surface(&[], zero),
        Err(SurfaceError::ZeroDimension)
    );
    assert_eq!(frame, before);
}
