use ghostengine::Color;

// Independent high-precision reference for the straight-alpha source-over equation.
fn reference_source_over(source: Color, destination: Color) -> Color {
    let source_alpha = u128::from(source.a);
    let destination_alpha = u128::from(destination.a);
    let inverse_source_alpha = 255 - source_alpha;
    let alpha_numerator =
        source_alpha * 255 + destination_alpha * inverse_source_alpha;

    if alpha_numerator == 0 {
        return Color::TRANSPARENT;
    }

    let channel = |source_channel: u8, destination_channel: u8| {
        let numerator = u128::from(source_channel) * source_alpha * 255
            + u128::from(destination_channel)
                * destination_alpha
                * inverse_source_alpha;
        ((numerator + alpha_numerator / 2) / alpha_numerator) as u8
    };

    Color::rgba(
        channel(source.r, destination.r),
        channel(source.g, destination.g),
        channel(source.b, destination.b),
        ((alpha_numerator + 127) / 255) as u8,
    )
}

#[test]
fn source_over_matches_reference_for_every_alpha_pair() {
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
