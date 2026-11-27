//! Generates placeholder cover art (PNG files) with GhostEngine's own rasteriser.
//!
//!   cargo run --release --bin gen-covers [OUT_DIR]      (default: assets/covers)
//!
//! The launcher loads these files like any real cover. Replace one by dropping a real
//! PNG/JPG with the same name (slug of the title) in `assets/covers/`.

use amethyst_preview::covers::slug;
use amethyst_preview::data::cover;
use amethyst_preview::gfx::{hex, save_png, Fonts, Painter};
use ghostengine::Color;
use std::path::PathBuf;

const W: i32 = 480;
const H: i32 = 640;

fn mix(a: Color, b: Color, t: f32) -> Color {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color::rgb(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

fn alpha(c: Color, a: u8) -> Color {
    Color::rgba(c.r, c.g, c.b, a)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "assets/covers".into()));
    std::fs::create_dir_all(&out)?;
    let fonts_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts");
    let fonts = Fonts::load(fonts_dir)?;

    // (title, motif)
    let titles = [
        ("Cyberpunk 2077", 5),
        ("The Witcher 3", 1),
        ("Elden Ring", 2),
        ("Alan Wake 2", 3),
        ("Forza Horizon 5", 0),
        ("Hades", 2),
        ("Baldur's Gate 3", 1),
        ("Balatro", 4),
        ("Celeste", 1),
        ("Stardew Valley", 0),
        ("Disco Elysium", 3),
        ("Hollow Knight", 4),
    ];
    for (title, motif) in titles {
        let (top, bottom) = cover(title);
        let (top, bottom) = (hex(top), hex(bottom));
        let light = mix(top, Color::WHITE, 0.6);
        let mid = mix(top, Color::WHITE, 0.3);
        let mut p = Painter::new(W as u32, H as u32, &fonts);
        p.gradient(0, 0, W, H, 165.0, &[(0.0, mix(top, Color::WHITE, 0.12)), (1.0, bottom)]);
        match motif {
            0 => {
                // low sun over layered bands
                p.circle(240, 250, 150, alpha(light, 70));
                p.circle(240, 250, 105, alpha(light, 110));
                p.circle(240, 250, 62, alpha(Color::WHITE, 170));
                for i in 0..7 {
                    let y = 330 + i * 34;
                    p.fill(0, y, W, 14 + i * 3, alpha(mix(bottom, mid, 0.25 + i as f32 * 0.08), 190));
                }
            }
            1 => {
                // mountain range
                p.circle(350, 170, 56, alpha(light, 150));
                let peaks: [(f32, f32, f32, u8); 3] = [(120.0, 300.0, 260.0, 120), (340.0, 250.0, 300.0, 160), (230.0, 360.0, 220.0, 210)];
                for (cx, top_y, half, a) in peaks {
                    p.poly(&[(cx - half, H as f32), (cx, top_y), (cx + half, H as f32)], alpha(mix(bottom, mid, 0.35), a));
                }
                p.poly(&[(0.0, H as f32), (160.0, 470.0), (330.0, H as f32)], alpha(bottom, 235));
            }
            2 => {
                // concentric rings
                for (i, r) in [200, 160, 122, 86, 52].iter().enumerate() {
                    let c = if i % 2 == 0 { alpha(light, 60 + i as u8 * 22) } else { alpha(bottom, 150) };
                    p.circle(240, 270, *r, c);
                }
                p.circle(240, 270, 22, alpha(Color::WHITE, 200));
            }
            3 => {
                // diagonal slashes
                for i in 0..6 {
                    let x = -80.0 + i as f32 * 110.0;
                    let a = 50 + i as u8 * 22;
                    p.poly(&[(x + 140.0, 0.0), (x + 215.0, 0.0), (x + 15.0, H as f32), (x - 60.0, H as f32)], alpha(if i % 2 == 0 { light } else { mid }, a));
                }
            }
            4 => {
                // grid of dots
                for gy in 0..9 {
                    for gx in 0..6 {
                        let r = 8 + ((gx * 3 + gy * 5) % 4) * 5;
                        let c = if (gx + gy) % 3 == 0 { alpha(Color::WHITE, 170) } else { alpha(light, 95) };
                        p.circle(50 + gx * 76, 70 + gy * 62, r, c);
                    }
                }
            }
            _ => {
                // city skyline with lit windows
                p.circle(380, 150, 70, alpha(light, 120));
                let towers = [(20, 300), (110, 220), (200, 340), (290, 180), (380, 280)];
                for (x, top_y) in towers {
                    p.fill(x, top_y, 82, H - top_y, alpha(mix(bottom, Color::BLACK, 0.35), 235));
                    for wy in (top_y + 16..H - 30).step_by(30) {
                        for wx in 0..3 {
                            if (wx * 7 + wy / 30 + x / 10) % 3 != 0 {
                                p.fill(x + 12 + wx * 24, wy, 12, 14, alpha(light, 190));
                            }
                        }
                    }
                }
            }
        }
        save_png(&p.fb, H as u32, &out.join(format!("{}.png", slug(title))))?;
        println!("{}", slug(title));
    }
    Ok(())
}
