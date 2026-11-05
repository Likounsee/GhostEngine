use ghostengine::{Color, Framebuffer};
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
            let red = ((x * 255) / (width - 1)) as u8;
            let green = ((y * 180) / (height - 1)) as u8;
            framebuffer.set_pixel(x, y, Color::rgb(red, green, 110));
        }
    }

    for y in 55..125 {
        for x in 105..215 {
            framebuffer.set_pixel(x, y, Color::WHITE);
        }
    }

    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in framebuffer.pixels() {
        ppm.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
    }

    fs::write(&output, ppm)?;
    println!("Wrote {}x{} image to {}", width, height, output.display());
    Ok(())
}
