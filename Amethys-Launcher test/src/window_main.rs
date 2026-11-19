//! Interactive Amethyst Launcher test window (GhostEngine native window).
//!
//!   cargo run --release --features window --bin amethyst-window [FONT_DIR]
//!   cargo run --release --features window --bin amethyst-window -- --snapshot OUT_DIR [FONT_DIR]
//!
//! Wheel = scroll, hover highlights clickable areas, click navigates (sidebar, cards,
//! filters, "Fiche"), the search bar is GhostEngine's text input. Esc quits.
//! `--snapshot` replays a scripted session headlessly and saves PNG frames (no display needed).

use amethyst_preview::app::{App, WIN_H, WIN_W};
use amethyst_preview::gfx::{save_png, Fonts, BG};
use ghostengine::window::InputState;
use ghostengine::{Framebuffer, NativeWindowSurface, Point};
use std::path::PathBuf;

const DEFAULT_FONTS: &str = "/usr/share/fonts/opentype/inter";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let snapshot = args.iter().position(|a| a == "--snapshot").and_then(|i| args.get(i + 1)).cloned();
    let font_dir = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && args.get(i.wrapping_sub(1)).map_or(true, |p| p != "--snapshot"))
        .map(|(_, a)| a.clone())
        .next()
        .unwrap_or_else(|| DEFAULT_FONTS.to_string());
    // The window closure must be 'static: leak the fonts for the process lifetime.
    let fonts: &'static Fonts = Box::leak(Box::new(Fonts::load(&font_dir)?));
    let mut app = App::new(fonts);

    if let Some(dir) = snapshot {
        return replay(&mut app, PathBuf::from(dir));
    }

    NativeWindowSurface::run("Amethyst Launcher — GhostEngine test", WIN_W, WIN_H, move |fb, input, clipboard| {
        let out = app.frame(fb, &input);
        if let Some(action) = out.clipboard {
            if let Ok(Some(text)) = clipboard.resolve(&action) {
                app.paste(text);
            }
        }
        out.keep_running
    })?;
    Ok(())
}

fn at(x: i32, y: i32) -> InputState {
    InputState { mouse_position: Some(Point::new(x, y)), ..Default::default() }
}

fn press(x: i32, y: i32) -> InputState {
    InputState { left_mouse_down: true, left_mouse_pressed: true, ..at(x, y) }
}

fn release(x: i32, y: i32) -> InputState {
    InputState { left_mouse_released: true, ..at(x, y) }
}

/// Scripted session using the same `App::frame` the window calls.
fn replay(app: &mut App, dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(&dir)?;
    let mut fb = Framebuffer::new(WIN_W, WIN_H, BG)?;
    let mut shot = |app: &mut App, name: &str, input: InputState| -> Result<(), Box<dyn std::error::Error>> {
        app.frame(&mut fb, &input);
        save_png(&fb, WIN_H, &dir.join(format!("{name}.png")))?;
        println!("{name}: page={:?} scroll={}/{} store_filter={} lib_filter={}", app.page, app.scroll.offset(), app.scroll.max_offset(), app.st.store_filter, app.st.lib_filter);
        Ok(())
    };
    shot(app, "w0-boutique", at(800, 300))?;
    shot(app, "w1-survol-carte", at(370, 760))?;
    shot(app, "w2-defilement", InputState { scroll_delta_y: 700, ..at(800, 500) })?;
    shot(app, "w3-defilement-conserve", at(800, 500))?; // pointer only: scrolled position kept
    shot(app, "w4-clic-bibliotheque-press", press(100, 140))?;
    shot(app, "w4-clic-bibliotheque", release(100, 140))?;
    shot(app, "w5-bibliotheque-steam-press", press(562, 228))?;
    shot(app, "w5-bibliotheque-steam", release(562, 228))?;
    shot(app, "w6-fiche-press", press(350, 400))?;
    shot(app, "w6-fiche", release(350, 400))?;
    shot(app, "w7-retour-press", press(285, 32))?;
    shot(app, "w7-retour-boutique", release(285, 32))?;
    shot(app, "w8-recherche-press", press(400, 46))?;
    shot(app, "w8-recherche-focus", release(400, 46))?;
    Ok(())
}
