//! Native animation demo using the callback-driven host runner.
//!
//! Run with: cargo run -p ghostengine --example native_animation_demo --features native-window

use ghostengine::rasterizer::{draw_circle, fill_rect};
use ghostengine::{Color, HostEvent, HostRunControl, NativeWindowSurface, Point, Rect};
use std::{error::Error, time::Duration};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 480;
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

fn main() -> Result<(), Box<dyn Error>> {
    let mut frame = 0_u32;

    NativeWindowSurface::run_with_host_events_control(
        "GhostEngine — native animation",
        WIDTH,
        HEIGHT,
        move |framebuffer, events, _clipboard| {
            if events
                .iter()
                .any(|event| matches!(event, HostEvent::CloseRequested))
            {
                return HostRunControl::Exit;
            }

            framebuffer.clear(Color::rgb(16, 20, 30));
            let travel = framebuffer.width().saturating_sub(96).max(1);
            let x = frame.wrapping_mul(4) % travel;
            let center_y = framebuffer.height() / 2;
            fill_rect(
                framebuffer,
                Rect::new(x, center_y.saturating_sub(24), 64, 48),
                Color::rgb(70, 170, 245),
            );
            draw_circle(
                framebuffer,
                Point::new(
                    (framebuffer.width() / 2) as i32,
                    (framebuffer.height() / 2) as i32,
                ),
                18,
                Color::rgb(245, 185, 75),
            );

            frame = frame.wrapping_add(1);
            HostRunControl::RedrawAfter(FRAME_INTERVAL)
        },
    )?;

    Ok(())
}
