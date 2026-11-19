//! Renders the Amethyst Launcher mockups headlessly with GhostEngine's CPU renderer.
//!
//! Usage: cargo run -p amethyst-preview --release -- [OUT_DIR] [FONT_DIR]
//! Writes, per page, `<name>.png` (the 1600x900 / 16:9 launcher window) and
//! `<name>-full.png` (the whole scrollable page). Fonts: Inter (OTF) from FONT_DIR.

use amethyst_preview::gfx::{save_png, Fonts, Painter, State};
use amethyst_preview::pages::{self, Active, PageFn};
use std::{path::Path, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "amethyst-preview-out".into());
    let font_dir = std::env::args().nth(2).unwrap_or_else(|| "/usr/share/fonts/opentype/inter".into());
    std::fs::create_dir_all(&out)?;
    let fonts = Fonts::load(&font_dir)?;

    let st = State::default();
    let list: [(&str, Active, PageFn); 4] = [
        ("1-boutique", Active::Store, pages::store),
        ("2-bibliotheque", Active::Library, pages::library),
        ("3-fiche-cyberpunk-2077", Active::Store, pages::product),
        ("4-parametres", Active::Settings, pages::settings),
    ];
    for (name, active, page) in list {
        // 16:9 launcher window (content below the fold is clipped, as in the app)
        let t = Instant::now();
        let mut p = Painter::new(1600, 900, &fonts);
        pages::sidebar(&mut p, active, 900);
        let content_h = page(&mut p, 900, &st);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        save_png(&p.fb, 900, &Path::new(&out).join(format!("{name}.png")))?;

        // full scrollable page
        let full_h = content_h.max(900);
        let mut f = Painter::new(1600, full_h as u32, &fonts);
        pages::sidebar(&mut f, active, full_h);
        page(&mut f, full_h, &st);
        save_png(&f.fb, full_h as u32, &Path::new(&out).join(format!("{name}-full.png")))?;
        println!("{name}: window 1600x900 rendered in {ms:.1} ms (CPU, headless); full page 1600x{full_h}");
    }
    Ok(())
}
