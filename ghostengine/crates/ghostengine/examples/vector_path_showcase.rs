//! Headless vector-path showcase: Bézier fills, transformed geometry, strokes,
//! clipping and deterministic PPM export.
//!
//! Run with: cargo run -p ghostengine --example vector_path_showcase -- output.ppm

use ghostengine::rasterizer::fill_rect_linear_gradient;
use ghostengine::{
    fill_path_even_odd_aa, fill_path_non_zero_aa, fill_path_non_zero_aa_clipped,
    fill_transformed_path_non_zero_aa, stroke_path_aa, Color, Framebuffer, Gradient, GradientStop,
    Path, Rect, StrokeCap, StrokeJoin, StrokeStyle, Transform2D, Vector,
};
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ghostengine-vector-showcase.ppm"));
    let mut framebuffer = Framebuffer::new(720, 420, Color::rgb(12, 15, 26))?;

    let background = Gradient::new(vec![
        GradientStop {
            position: 0.0,
            color: Color::rgb(20, 25, 46),
        },
        GradientStop {
            position: 0.55,
            color: Color::rgb(35, 25, 62),
        },
        GradientStop {
            position: 1.0,
            color: Color::rgb(10, 45, 58),
        },
    ])
    .map_err(|error| format!("invalid showcase gradient: {error:?}"))?;
    fill_rect_linear_gradient(
        &mut framebuffer,
        Rect::new(0, 0, 720, 420),
        Vector::new(0.0, 0.0),
        Vector::new(720.0, 420.0),
        &background,
    );

    let mut ribbon = Path::new();
    ribbon
        .move_to(Vector::new(65.0, 165.0))
        .cubic_to(
            Vector::new(105.0, 35.0),
            Vector::new(250.0, 35.0),
            Vector::new(300.0, 150.0),
        )
        .cubic_to(
            Vector::new(350.0, 265.0),
            Vector::new(205.0, 345.0),
            Vector::new(65.0, 245.0),
        )
        .close();
    fill_path_non_zero_aa(
        &mut framebuffer,
        &ribbon,
        Color::rgba(112, 91, 225, 220),
        0.35,
    )
    .map_err(|error| format!("ribbon fill failed: {error:?}"))?;
    stroke_path_aa(
        &mut framebuffer,
        &ribbon,
        Transform2D::IDENTITY,
        StrokeStyle::new(5.0)
            .with_cap(StrokeCap::Round)
            .with_join(StrokeJoin::Round),
        Color::rgb(225, 218, 255),
        0.35,
    )
    .map_err(|error| format!("ribbon stroke failed: {error:?}"))?;

    let mut star = Path::new();
    for index in 0..10 {
        let angle = -std::f32::consts::FRAC_PI_2 + index as f32 * std::f32::consts::PI / 5.0;
        let radius = if index % 2 == 0 { 82.0 } else { 36.0 };
        let point = Vector::new(angle.cos() * radius, angle.sin() * radius);
        if index == 0 {
            star.move_to(point);
        } else {
            star.line_to(point);
        }
    }
    star.close();
    let transform = Transform2D::rotation(0.22).then(Transform2D::translation(510.0, 165.0));
    fill_transformed_path_non_zero_aa(
        &mut framebuffer,
        &star,
        transform,
        Color::rgb(245, 177, 76),
        0.3,
    )
    .map_err(|error| format!("transformed star fill failed: {error:?}"))?;
    stroke_path_aa(
        &mut framebuffer,
        &star,
        transform,
        StrokeStyle::new(4.0).with_join(StrokeJoin::Bevel),
        Color::rgb(255, 235, 190),
        0.3,
    )
    .map_err(|error| format!("transformed star stroke failed: {error:?}"))?;

    // Two contours demonstrate even-odd filling: the inner contour stays hollow.
    let mut ring = Path::new();
    ring.move_to(Vector::new(430.0, 300.0))
        .cubic_to(
            Vector::new(430.0, 255.0),
            Vector::new(500.0, 255.0),
            Vector::new(500.0, 300.0),
        )
        .cubic_to(
            Vector::new(500.0, 345.0),
            Vector::new(430.0, 345.0),
            Vector::new(430.0, 300.0),
        )
        .close();
    ring.move_to(Vector::new(448.0, 300.0))
        .cubic_to(
            Vector::new(448.0, 275.0),
            Vector::new(482.0, 275.0),
            Vector::new(482.0, 300.0),
        )
        .cubic_to(
            Vector::new(482.0, 325.0),
            Vector::new(448.0, 325.0),
            Vector::new(448.0, 300.0),
        )
        .close();
    fill_path_even_odd_aa(&mut framebuffer, &ring, Color::rgb(75, 205, 174), 0.25)
        .map_err(|error| format!("even-odd ring fill failed: {error:?}"))?;

    let mut clipped = Path::new();
    clipped
        .move_to(Vector::new(545.0, 275.0))
        .line_to(Vector::new(700.0, 275.0))
        .line_to(Vector::new(700.0, 385.0))
        .line_to(Vector::new(545.0, 385.0))
        .close();
    fill_path_non_zero_aa_clipped(
        &mut framebuffer,
        &clipped,
        Color::rgba(60, 155, 210, 230),
        0.25,
        Rect::new(575, 300, 95, 60),
    )
    .map_err(|error| format!("clipped path fill failed: {error:?}"))?;

    let mut ppm = b"P6\n720 420\n255\n".to_vec();
    for pixel in framebuffer.pixels() {
        ppm.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
    }
    fs::write(&output, ppm)?;
    println!("Wrote 720x420 vector showcase to {}", output.display());
    Ok(())
}
