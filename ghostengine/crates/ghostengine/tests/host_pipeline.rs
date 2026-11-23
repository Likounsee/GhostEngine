use ghostengine::{
    AlphaMode, Color, ColorSpace, Framebuffer, HeadlessHost, HeadlessHostError, HostBackend,
    HostEvent, HostWindowConfig, PixelFormat, SurfaceDescriptor,
};

#[test]
fn headless_render_present_and_surface_export_work_through_public_api() {
    let mut host = HeadlessHost::new(HostWindowConfig::new("pipeline", 2, 2)).unwrap();
    let mut frame = Framebuffer::new(2, 2, Color::BLACK).unwrap();
    frame.pixels_mut().copy_from_slice(&[
        Color::rgba(255, 0, 0, 128),
        Color::rgba(0, 255, 0, 255),
        Color::rgba(0, 0, 255, 64),
        Color::rgba(255, 255, 255, 0),
    ]);

    host.present(&frame).unwrap();
    let presented = host
        .take_presented_frame()
        .expect("the headless host must retain the presented frame");

    let descriptor = SurfaceDescriptor {
        width: 2,
        height: 2,
        row_stride: 12,
        pixel_format: PixelFormat::Bgra8,
        alpha_mode: AlphaMode::Straight,
        color_space: ColorSpace::Srgb,
    };
    // Two 8-byte rows with four bytes of padding between them. No padding is
    // required after the final row, so the exact required length is 20 bytes.
    let mut output = [0xCD; 20];
    presented
        .write_to_surface(&mut output, descriptor)
        .unwrap();

    assert_eq!(&output[0..8], &[0, 0, 255, 128, 0, 255, 0, 255]);
    assert_eq!(&output[8..12], &[0xCD; 4]);
    assert_eq!(&output[12..20], &[255, 0, 0, 64, 255, 255, 255, 0]);
}

#[test]
fn headless_resize_minimize_restore_and_event_delivery_are_consistent() {
    let mut host = HeadlessHost::new(HostWindowConfig::new("lifecycle", 4, 3)).unwrap();

    host.resize(0, 0).unwrap();
    assert_eq!(
        host.present(&Framebuffer::new(4, 3, Color::BLACK).unwrap()),
        Err(HeadlessHostError::DrawableUnavailable)
    );

    host.resize(4, 3).unwrap();
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
                width: 4,
                height: 3,
            },
        ]
    );

    host.present(&Framebuffer::new(4, 3, Color::BLACK).unwrap())
        .unwrap();
}

#[test]
fn surface_export_matrix_preserves_padding_and_handles_every_pixel_layout() {
    let mut frame = Framebuffer::new(2, 2, Color::TRANSPARENT).unwrap();
    frame.pixels_mut().copy_from_slice(&[
        Color::rgba(200, 100, 50, 128),
        Color::rgba(10, 20, 30, 255),
        Color::rgba(90, 60, 30, 0),
        Color::rgba(1, 2, 3, 64),
    ]);

    for pixel_format in [
        PixelFormat::Rgb8,
        PixelFormat::Bgr8,
        PixelFormat::Rgba8,
        PixelFormat::Bgra8,
    ] {
        let bytes_per_pixel = pixel_format.bytes_per_pixel();
        let minimum_row_bytes = 2 * bytes_per_pixel;
        let row_stride = minimum_row_bytes + 3;

        for alpha_mode in [AlphaMode::Straight, AlphaMode::Premultiplied, AlphaMode::Opaque] {
            let descriptor = SurfaceDescriptor {
                width: 2,
                height: 2,
                row_stride,
                pixel_format,
                alpha_mode,
                color_space: ColorSpace::Srgb,
            };
            let required_len = descriptor.required_buffer_len().unwrap();
            assert_eq!(required_len, row_stride + minimum_row_bytes);

            let mut destination = vec![0xA5; required_len];
            frame
                .write_to_surface(&mut destination, descriptor)
                .unwrap();

            let mut expected = vec![0xA5; required_len];
            let pixels = frame.pixels();
            for y in 0..2 {
                for x in 0..2 {
                    let pixel = pixels[y * 2 + x];
                    let has_alpha = matches!(pixel_format, PixelFormat::Rgba8 | PixelFormat::Bgra8);
                    let alpha = if has_alpha && alpha_mode != AlphaMode::Opaque {
                        pixel.a
                    } else {
                        255
                    };
                    let mut rgb = [pixel.r, pixel.g, pixel.b];
                    if has_alpha && alpha_mode == AlphaMode::Premultiplied {
                        for channel in &mut rgb {
                            *channel = ((u16::from(*channel) * u16::from(alpha) + 127) / 255) as u8;
                        }
                    }
                    let ordered = match pixel_format {
                        PixelFormat::Rgb8 | PixelFormat::Rgba8 => rgb,
                        PixelFormat::Bgr8 | PixelFormat::Bgra8 => [rgb[2], rgb[1], rgb[0]],
                    };
                    let offset = y * row_stride + x * bytes_per_pixel;
                    expected[offset..offset + 3].copy_from_slice(&ordered);
                    if has_alpha {
                        expected[offset + 3] = alpha;
                    }
                }
            }

            assert_eq!(
                destination, expected,
                "format {pixel_format:?}, alpha mode {alpha_mode:?}"
            );
        }
    }
}

