use ghostengine::rasterizer::{draw_circle, draw_line, fill_ellipse, fill_rect, fill_triangle};
use ghostengine::{Color, Framebuffer, Point, Rect};
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ghostengine-first-image.ppm"));

    let width = 320;
    let height = 180;
    let mut framebuffer = Framebuffer::new(width, height, Color::rgb(18, 22, 32))?;

    for y in 0..height {
        for x in 0..width {
            let red = ((x * 90) / (width - 1)) as u8;
            let green = ((y * 100) / (height - 1)) as u8;
            framebuffer.set_pixel(x, y, Color::rgb(red, green, 70));
        }
    }

    fill_rect(
        &mut framebuffer,
        Rect::new(24, 24, 72, 42),
        Color::rgb(30, 190, 130),
    );
    fill_triangle(
        &mut framebuffer,
        Point::new(160, 18),
        Point::new(112, 112),
        Point::new(208, 112),
        Color::rgb(245, 190, 55),
    );
    draw_circle(&mut framebuffer, Point::new(260, 52), 25, Color::WHITE);
    fill_ellipse(
        &mut framebuffer,
        Point::new(260, 132),
        38,
        17,
        Color::rgb(100, 145, 235),
    );
    draw_line(
        &mut framebuffer,
        Point::new(20, 150),
        Point::new(300, 125),
        Color::WHITE,
    );
    draw_line(
        &mut framebuffer,
        Point::new(-40, 80),
        Point::new(360, 80),
        Color::rgb(230, 80, 100),
    );

    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in framebuffer.pixels() {
        ppm.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
    }
    fs::write(&output, ppm)?;
    println!("Wrote {}x{} image to {}", width, height, output.display());
    Ok(())
}
