//! Display-free application example using the public HostBackend contract.
//!
//! Run with: cargo run -p ghostengine --example headless_host_demo

use ghostengine::rasterizer::{draw_circle, fill_rect};
use ghostengine::{
    Color, Framebuffer, HeadlessHost, HostBackend, HostWindowConfig, Point, Rect,
};
use std::error::Error;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;

fn main() -> Result<(), Box<dyn Error>> {
    let mut host = HeadlessHost::new(
        HostWindowConfig::new("GhostEngine headless host", WIDTH, HEIGHT)
            .with_resizable(false),
    )?;
    let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT, Color::rgb(18, 24, 36))?;

    fill_rect(
        &mut framebuffer,
        Rect::new(48, 48, 220, 96),
        Color::rgb(52, 126, 220),
    );
    draw_circle(
        &mut framebuffer,
        Point::new(390, 150),
        58,
        Color::rgb(242, 174, 76),
    );

    host.request_redraw();
    let mut events = Vec::new();
    host.poll_events(&mut events);
    host.present(&framebuffer)?;

    let presented = host
        .take_presented_frame()
        .expect("a successful headless presentation retains its framebuffer");
    println!(
        "platform={:?}, drawable={:?}, events={}, pixels={}",
        host.platform(),
        host.drawable_size(),
        events.len(),
        presented.pixels().len(),
    );
    host.close();

    Ok(())
}
