use ghostengine::Color;

fn reference_source_over(source: Color, destination: Color) -> Color {
    let source_alpha = f64::from(source.a) / 255.0;
    let destination_alpha = f64::from(destination.a) / 255.0;
    let output_alpha =
        source_alpha + destination_alpha * (1.0 - source_alpha);

    if output_alpha == 0.0 {
        return Color::TRANSPARENT;
    }

    let channel = |source_channel: u8, destination_channel: u8| {
        let premultiplied = f64::from(source_channel) * source_alpha
            + f64::from(destination_channel)
                * destination_alpha
                * (1.0 - source_alpha);
        (premultiplied / output_alpha).round() as u8
    };

    Color::rgba(
        channel(source.r, destination.r),
        channel(source.g, destination.g),
        channel(source.b, destination.b),
        (output_alpha * 255.0).round() as u8,
    )
}

#[test]
fn source_over_matches_float_reference_for_every_alpha_pair() {
    let channel_pairs = [
        ([0, 0, 0], [255, 255, 255]),
        ([255, 0, 0], [0, 255, 255]),
        ([17, 129, 241], [231, 63, 5]),
        ([255, 255, 255], [0, 0, 0]),
    ];

    for source_alpha in 0..=u8::MAX {
        for destination_alpha in 0..=u8::MAX {
            for (source_rgb, destination_rgb) in channel_pairs {
                let source = Color::rgba(
                    source_rgb[0],
                    source_rgb[1],
                    source_rgb[2],
                    source_alpha,
                );
                let destination = Color::rgba(
                    destination_rgb[0],
                    destination_rgb[1],
                    destination_rgb[2],
                    destination_alpha,
                );

                assert_eq!(
                    source.blend_over(destination),
                    reference_source_over(source, destination),
                    "source alpha={source_alpha}, destination alpha={destination_alpha}, source={source_rgb:?}, destination={destination_rgb:?}"
                );
            }
        }
    }
}

#[test]
fn source_over_alpha_is_monotonic_and_zero_alpha_is_canonical() {
    for source_alpha in 0..=u8::MAX {
        for destination_alpha in 0..=u8::MAX {
            let source = Color::rgba(90, 170, 230, source_alpha);
            let destination = Color::rgba(30, 80, 140, destination_alpha);
            let result = source.blend_over(destination);

            assert!(result.a >= source_alpha);
            assert!(result.a >= destination_alpha);
            if source_alpha == 0 && destination_alpha == 0 {
                assert_eq!(result, Color::TRANSPARENT);
            }
            if source_alpha == 0 {
                assert_eq!(result, destination);
            }
            if source_alpha == u8::MAX {
                assert_eq!(result, source);
            }
        }
    }
}
