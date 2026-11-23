//! Headless image-decoding, texture-filtering and mipmap example.
//!
//! Run with:
//! cargo run -p ghostengine --features image-decoding --example image_pipeline_demo -- image.png [output.ppm]

use ghostengine::{
    decode_texture, draw_mipmapped_texture_clipped, draw_texture, Color, Framebuffer,
    MipmapChain, Rect, TextureFilter,
};
use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let input = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("pass a BMP, JPEG, PNG or PNM image path")?;
    let output = env::args_os()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ghostengine-image-pipeline.ppm"));

    let encoded = fs::read(&input)?;
    let texture = decode_texture(&encoded)?;
    let mipmaps = MipmapChain::new(&texture)?;
    let mut frame = Framebuffer::new(800, 480, Color::rgb(18, 22, 32))?;

    // Compare nearest and bilinear sampling at two independently sized targets.
    draw_texture(
        &mut frame,
        &texture,
        Rect::new(32, 64, 340, 340),
        TextureFilter::Nearest,
    );
    draw_texture(
        &mut frame,
        &texture,
        Rect::new(428, 64, 340, 340),
        TextureFilter::Bilinear,
    );

    // Exercise the mip chain on a smaller destination and clipped viewport.
    draw_mipmapped_texture_clipped(
        &mut frame,
        &mipmaps,
        Rect::new(340, 360, 120, 90),
        Rect::new(360, 360, 80, 90),
    );

    let mut ppm = format!("P6\n{} {}\n255\n", frame.width(), frame.height()).into_bytes();
    for pixel in frame.pixels() {
        ppm.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
    }
    fs::write(&output, ppm)?;

    println!(
        "Decoded {}x{} image ({} mip levels) from {} and wrote {}",
        texture.width(),
        texture.height(),
        mipmaps.levels().len(),
        input.display(),
        output.display(),
    );
    Ok(())
}