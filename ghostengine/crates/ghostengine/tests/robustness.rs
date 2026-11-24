use ghostengine::{
    AlphaMode, CachedLayer, Color, ColorSpace, DisplayList, FontFace, Framebuffer, Path,
    PathGeometryCache, PixelFormat, Point, Rect, SurfaceDescriptor, SurfaceError, Texture,
    TextureCache, TextureCacheError, Transform2D, Vector,
};

/// Small deterministic generator: reproducible property-style coverage without
/// introducing a runtime dependency or nondeterministic CI failures.
struct Generator(u64);

impl Generator {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 32) as u32
    }

    fn range(&mut self, upper: u32) -> u32 {
        self.next() % upper
    }
}

#[test]
fn rectangle_intersections_obey_half_open_bounds_for_adversarial_coordinates() {
    let mut rng = Generator::new(0x4748_4f53_5446_5352);
    let mut rectangles = vec![
        Rect::new(0, 0, u32::MAX, u32::MAX),
        Rect::new(u32::MAX, u32::MAX, u32::MAX, u32::MAX),
        Rect::new(u32::MAX - 1, 0, u32::MAX, 1),
        Rect::new(0, u32::MAX - 1, 1, u32::MAX),
        Rect::new(0, 0, 0, u32::MAX),
    ];
    for _ in 0..512 {
        rectangles.push(Rect::new(rng.next(), rng.next(), rng.next(), rng.next()));
    }

    for (index, left) in rectangles.iter().copied().enumerate() {
        let right = rectangles[(index.wrapping_mul(37) + 11) % rectangles.len()];
        if let Some(intersection) = left.intersection(right) {
            assert!(intersection.width > 0 && intersection.height > 0);
            let ix0 = u64::from(intersection.x);
            let iy0 = u64::from(intersection.y);
            let ix1 = ix0 + u64::from(intersection.width);
            let iy1 = iy0 + u64::from(intersection.height);
            for source in [left, right] {
                let sx0 = u64::from(source.x);
                let sy0 = u64::from(source.y);
                let sx1 = sx0 + u64::from(source.width);
                let sy1 = sy0 + u64::from(source.height);
                assert!(ix0 >= sx0 && iy0 >= sy0 && ix1 <= sx1 && iy1 <= sy1);
            }
        }
    }
}

#[test]
fn framebuffer_preserving_resize_matches_a_reference_grid_across_many_shapes() {
    let mut rng = Generator::new(0x5245_5349_5a45_4653);
    for _ in 0..256 {
        let old_width = 1 + rng.range(12);
        let old_height = 1 + rng.range(12);
        let new_width = 1 + rng.range(12);
        let new_height = 1 + rng.range(12);
        let clear = Color::rgba(9, 17, 33, 65);
        let mut frame = Framebuffer::new(old_width, old_height, clear).unwrap();
        for y in 0..old_height {
            for x in 0..old_width {
                let value = (x + y * old_width) as u8;
                assert!(frame.set_pixel(x, y, Color::rgba(value, !value, value / 2, 255)));
            }
        }
        let before = frame.clone();
        let keep_width = old_width.min(new_width);
        let keep_height = old_height.min(new_height);

        frame
            .resize_preserving(new_width, new_height, clear)
            .unwrap();

        assert_eq!(frame.width(), new_width);
        assert_eq!(frame.height(), new_height);
        assert_eq!(frame.pixels().len(), (new_width * new_height) as usize);
        for y in 0..new_height {
            for x in 0..new_width {
                let expected = if x < keep_width && y < keep_height {
                    before.get_pixel(x, y).unwrap()
                } else {
                    clear
                };
                assert_eq!(frame.get_pixel(x, y), Some(expected));
            }
        }
    }
}

#[test]
fn display_list_adversarial_clips_and_rectangles_never_write_outside_clip() {
    let mut rng = Generator::new(0x4449_5350_4c41_5953);
    let mut frame = Framebuffer::new(31, 23, Color::BLACK).unwrap();
    let mut list = DisplayList::new();
    let mut pixels_allowed_by_a_clip = vec![false; 31 * 23];

    for _ in 0..512 {
        let x = rng.range(48) as i32 - 8;
        let y = rng.range(40) as i32 - 8;
        let width = match rng.range(8) {
            0 => 0,
            1 => u32::MAX,
            _ => rng.range(24),
        };
        let height = match rng.range(8) {
            0 => 0,
            1 => u32::MAX,
            _ => rng.range(24),
        };
        let clip = Rect::new(rng.range(40), rng.range(32), rng.range(36), rng.range(28));
        if let Some(visible) = clip.intersection(Rect::new(0, 0, 31, 23)) {
            for y in visible.y..visible.y + visible.height {
                for x in visible.x..visible.x + visible.width {
                    pixels_allowed_by_a_clip[(y * 31 + x) as usize] = true;
                }
            }
        }
        list.push_clip(clip);
        list.fill_rect(
            Rect::new(x.max(0) as u32, y.max(0) as u32, width, height),
            Color::rgba(rng.next() as u8, rng.next() as u8, rng.next() as u8, 255),
        );
        assert!(list.pop_clip());
    }

    let stats = list.replay(&mut frame);
    assert!(stats.commands_submitted <= 512);
    assert_eq!(
        stats.commands_executed + stats.commands_skipped,
        stats.commands_submitted
    );
    // Every command had its own clip. Replay must remain memory-safe even when
    // the requested geometry and clip extend beyond the target dimensions.
    assert_eq!(frame.pixels().len(), 31 * 23);
    for (index, (pixel, allowed)) in frame
        .pixels()
        .iter()
        .zip(pixels_allowed_by_a_clip)
        .enumerate()
    {
        if !allowed {
            assert_eq!(*pixel, Color::BLACK, "pixel {index} escaped every clip");
        }
    }
}

#[test]
fn transforms_and_path_flattening_reject_non_finite_inputs_without_panicking() {
    let invalid = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
    for value in invalid {
        let transform = Transform2D::new(value, 0.0, 0.0, 1.0, 0.0, 0.0);
        assert!(!transform.is_finite());
        assert!(transform.inverse().is_none());

        let mut path = Path::new();
        path.move_to(Vector::new(0.0, 0.0))
            .line_to(Vector::new(value, 1.0));
        assert!(path.flatten(0.25).is_err());
    }

    let mut rng = Generator::new(0x5041_5448_464c_4154);
    for _ in 0..48 {
        let mut path = Path::new();
        path.move_to(Vector::new(
            rng.range(2_000) as f32 - 1_000.0,
            rng.range(20_000) as f32 - 10_000.0,
        ));
        for _ in 0..3 {
            path.cubic_to(
                Vector::new(
                    rng.range(20_000) as f32 - 10_000.0,
                    rng.range(20_000) as f32 - 10_000.0,
                ),
                Vector::new(
                    rng.range(20_000) as f32 - 10_000.0,
                    rng.range(20_000) as f32 - 10_000.0,
                ),
                Vector::new(
                    rng.range(20_000) as f32 - 10_000.0,
                    rng.range(20_000) as f32 - 10_000.0,
                ),
            );
        }
        let flattened = path.flatten(4.0).unwrap();
        assert_eq!(flattened.subpaths.len(), 1);
        assert!(flattened.subpaths[0]
            .points
            .iter()
            .all(|point| point.x.is_finite() && point.y.is_finite()));
    }

    let mut malformed = Path::new();
    malformed.line_to(Vector::new(1.0, 1.0));
    assert!(malformed.flatten(0.25).is_err());
    assert!(malformed.flatten(0.0).is_err());
    assert!(malformed.flatten(f32::NAN).is_err());

    let mut list = DisplayList::new();
    assert!(!list.push_transform(Transform2D::scale(f32::INFINITY, 1.0)));
    assert!(!list.pop_transform());
    assert!(list
        .fill_triangle(
            Point::new(i32::MIN, i32::MAX),
            Point::new(i32::MAX, i32::MIN),
            Point::new(0, 0),
            Color::WHITE,
        )
        .is_ok());
}

#[test]
fn texture_cache_randomized_lifecycle_keeps_accounting_and_budget_consistent() {
    let mut rng = Generator::new(0x5445_5854_5552_4543);
    let mut cache = TextureCache::new(64);
    let mut handles = Vec::new();

    for step in 0..2_048 {
        let candidate =
            (!handles.is_empty()).then(|| handles[rng.range(handles.len() as u32) as usize]);
        match rng.range(6) {
            0 | 1 => {
                let color = Color::rgba(rng.next() as u8, rng.next() as u8, step as u8, 255);
                if let Ok(handle) = cache.insert(Texture::new(1, 1, vec![color]).unwrap()) {
                    handles.push(handle);
                }
            }
            2 => {
                if let Some(handle) = candidate {
                    let _ = cache.get(handle);
                }
            }
            3 => {
                if let Some(handle) = candidate {
                    let _ = cache.retain(handle);
                }
            }
            4 => {
                if let Some(handle) = candidate {
                    let _ = cache.release(handle);
                }
            }
            _ => {
                let budget = (rng.range(17) as usize) * 4;
                let result = cache.set_budget_bytes(budget);
                if matches!(result, Err(TextureCacheError::CacheFullyRetained)) {
                    assert!(cache.stats().used_bytes > budget);
                }
            }
        }

        let stats = cache.stats();
        assert!(stats.used_bytes <= stats.budget_bytes);
        assert_eq!(stats.used_bytes, stats.entries * 4);
        assert_eq!(stats.entries, cache.len());
        for handle in handles.iter().copied().rev().take(8) {
            if let Some(retain_count) = cache.retain_count(handle) {
                assert!(cache.peek(handle).is_some());
                if retain_count > 0 {
                    assert!(cache.release(handle));
                }
            } else {
                assert!(cache.peek(handle).is_none());
            }
        }
    }
}

#[test]
fn surface_stride_and_buffer_boundaries_are_validated_without_partial_writes() {
    let mut rng = Generator::new(0x5355_5246_4143_4553);
    for width in 1..=8 {
        for height in 1..=6 {
            let frame = Framebuffer::new(width, height, Color::rgba(31, 63, 127, 191)).unwrap();
            for format in [
                PixelFormat::Rgb8,
                PixelFormat::Bgr8,
                PixelFormat::Rgba8,
                PixelFormat::Bgra8,
            ] {
                let minimum = width as usize * format.bytes_per_pixel();
                let stride = minimum + rng.range(9) as usize;
                let descriptor = SurfaceDescriptor {
                    width,
                    height,
                    row_stride: stride,
                    pixel_format: format,
                    alpha_mode: AlphaMode::Straight,
                    color_space: ColorSpace::Srgb,
                };
                let required = descriptor.required_buffer_len().unwrap();
                assert_eq!(required, stride * (height as usize - 1) + minimum);

                let mut exact = vec![0xD7; required];
                frame.write_to_surface(&mut exact, descriptor).unwrap();
                for row in 0..height as usize {
                    let padding_start = row * stride + minimum;
                    let padding_end = ((row + 1) * stride).min(required);
                    assert!(exact[padding_start..padding_end]
                        .iter()
                        .all(|byte| *byte == 0xD7));
                }

                if required > 0 {
                    let mut short = vec![0xA9; required - 1];
                    let before = short.clone();
                    assert_eq!(
                        frame.write_to_surface(&mut short, descriptor),
                        Err(SurfaceError::BufferTooSmall)
                    );
                    assert_eq!(short, before);
                }
            }
        }
    }

    let overflow = SurfaceDescriptor {
        width: u32::MAX,
        height: u32::MAX,
        row_stride: usize::MAX,
        pixel_format: PixelFormat::Rgba8,
        alpha_mode: AlphaMode::Straight,
        color_space: ColorSpace::Srgb,
    };
    assert_eq!(
        overflow.required_buffer_len(),
        Err(SurfaceError::SizeOverflow)
    );
}

#[test]
fn texture_constructors_reject_malformed_dimensions_and_pixel_counts() {
    use ghostengine::TextureError;

    assert_eq!(
        Texture::new(0, 1, Vec::new()).unwrap_err(),
        TextureError::ZeroDimension
    );
    assert_eq!(
        Texture::new(1, 0, Vec::new()).unwrap_err(),
        TextureError::ZeroDimension
    );
    assert!(matches!(
        Texture::new(u32::MAX, u32::MAX, Vec::new()),
        Err(TextureError::SizeOverflow | TextureError::PixelCountMismatch)
    ));
    assert_eq!(
        Texture::try_from_pixels(3, 2, &[Color::BLACK; 5]).unwrap_err(),
        TextureError::PixelCountMismatch
    );
}

#[cfg(feature = "image-decoding")]
#[test]
fn arbitrary_truncated_image_inputs_fail_cleanly_without_panicking() {
    use ghostengine::{decode_texture_with_options, ImageDecodeOptions};

    let mut rng = Generator::new(0x494d_4147_4546_555a);
    for length in 0..384 {
        let mut bytes = vec![0; length];
        for byte in &mut bytes {
            *byte = rng.next() as u8;
        }
        let result = std::panic::catch_unwind(|| {
            decode_texture_with_options(&bytes, ImageDecodeOptions::default())
        });
        assert!(result.is_ok(), "decoder panicked for {length} bytes");
        assert!(
            result.unwrap().is_err(),
            "random bytes unexpectedly decoded"
        );
    }
}

#[test]
fn display_list_fallible_capacity_apis_report_overflow_without_mutating_state() {
    use ghostengine::{DisplayList, DisplayListError};

    assert_eq!(
        DisplayList::try_with_capacity(usize::MAX).unwrap_err(),
        DisplayListError::AllocationFailed
    );

    let mut list = DisplayList::new();
    list.clear();
    let original_capacity = list.command_capacity();
    assert_eq!(
        list.try_reserve_commands(usize::MAX),
        Err(DisplayListError::AllocationFailed)
    );
    assert_eq!(list.len(), 0);
    assert_eq!(list.command_capacity(), original_capacity);

    let mut reserved = DisplayList::try_with_capacity(32).unwrap();
    assert!(reserved.command_capacity() >= 32);
    reserved.try_reserve_commands(64).unwrap();
    assert!(reserved.command_capacity() >= 64);
}


#[test]
fn unified_cache_lifecycle_stays_bounded_under_repeated_churn() {
    let mut texture_cache = TextureCache::new(256);
    let mut path_cache = PathGeometryCache::new(8);
    let mut layer = CachedLayer::new(8, 8, Color::BLACK).unwrap();
    let font = FontFace::from_bytes(include_bytes!("fixtures/NotoSansArabic[wdth,wght].ttf"))
        .expect("bundled Arabic font fixture must load");

    let mut path = Path::new();
    path.move_to(Vector::new(0.0, 0.0))
        .line_to(Vector::new(12.0, 0.0))
        .line_to(Vector::new(6.0, 12.0))
        .close();

    for cycle in 0..2_000u32 {
        let color = Color::rgba(cycle as u8, (cycle >> 3) as u8, 127, 255);
        if let Ok(handle) = texture_cache.insert(Texture::new(2, 2, vec![color; 4]).unwrap()) {
            assert!(texture_cache.peek(handle).is_some());
        }
        assert!(texture_cache.stats().used_bytes <= texture_cache.stats().budget_bytes);

        let tolerance = 0.25 + (cycle % 4) as f32 * 0.125;
        assert!(!path_cache
            .get_or_flatten(u64::from(cycle % 32), &path, tolerance)
            .unwrap()
            .subpaths
            .is_empty());
        assert!(path_cache.stats().entries <= path_cache.stats().capacity);

        let pixel_x = cycle % layer.framebuffer().width();
        let pixel_y = cycle % layer.framebuffer().height();
        layer.mark_dirty(Rect::new(pixel_x, pixel_y, 1, 1));
        assert!(layer.render_if_dirty(|frame, _damage| {
            frame.set_pixel(pixel_x, pixel_y, color);
        }));
        if cycle % 17 == 0 {
            let width = 4 + cycle % 9;
            let height = 4 + (cycle / 17) % 9;
            layer.resize(width, height, Color::BLACK).unwrap();
            assert!(layer.render_if_dirty(|frame, _damage| frame.clear(color)));
        }
    }

    // The glyph cache deliberately clears in bounded batches instead of growing
    // without limit when applications encounter many size/glyph combinations.
    for index in 0..5_000 {
        let pixel_size = 12.0 + index as f32 / 5_000.0;
        let measured = font.measure("ا", pixel_size);
        assert!(measured.width.is_finite());
    }
    assert!(font.cached_glyph_count() <= 4_096);
    assert!(font.cached_glyph_count() > 0);

    assert!(path_cache.stats().entries <= 8);
    assert!(texture_cache.stats().used_bytes <= 256);
    assert!(layer.damage().is_empty());
}
