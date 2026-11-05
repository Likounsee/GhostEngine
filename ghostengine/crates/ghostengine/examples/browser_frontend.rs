use ghostengine::rasterizer::{draw_rect, fill_rect, fill_rounded_rect};
use ghostengine::window::WindowSurface;
use ghostengine::{draw_text, Color, FontFace, Framebuffer, Point, Rect};
use std::{error::Error, fs, path::PathBuf};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 640;

fn main() -> Result<(), Box<dyn Error>> {
    let font_path = std::env::args_os().nth(1).map(PathBuf::from).or_else(find_system_font)
        .ok_or("No system TTF font found. Pass a .ttf path as the first argument.")?;
    let font_data = fs::read(&font_path)?;
    let font = FontFace::from_bytes(&font_data).map_err(|error| format!("Could not load font: {error}"))?;
    let mut window = WindowSurface::new("GhostEngine — Browser frontend", WIDTH, HEIGHT)?;
    let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT, Color::WHITE)?;
    let mut address = String::from("https://welcome.ghostengine.local/");
    let mut address_focused = false;
    let mut previous_mouse_down = false;
    let mut page_title = String::from("Welcome to GhostEngine");
    let mut page_message = String::from("This page is drawn by GhostEngine's CPU renderer.");

    while window.is_open() {
        let input = window.poll_input();
        if input.escape_down { break; }
        let mouse_clicked = input.left_mouse_down && !previous_mouse_down;
        previous_mouse_down = input.left_mouse_down;
        if mouse_clicked {
            if let Some(pointer) = input.mouse_position {
                address_focused = Rect::new(190, 16, 590, 40).contains(pointer);
                if Rect::new(790, 16, 82, 40).contains(pointer) {
                    page_title = "Navigation requested".into();
                    page_message = format!("The frontend received this address: {address}");
                    address_focused = false;
                } else if Rect::new(32, 16, 42, 40).contains(pointer) {
                    page_title = "Back button".into();
                    page_message = "The application received a back-button click.".into();
                } else if Rect::new(82, 16, 42, 40).contains(pointer) {
                    page_title = "Forward button".into();
                    page_message = "The application received a forward-button click.".into();
                } else if Rect::new(132, 16, 42, 40).contains(pointer) {
                    page_title = "Page refreshed".into();
                    page_message = "The demo frontend redrew the current page.".into();
                }
            }
        }
        if address_focused {
            if input.backspace_pressed { address.pop(); }
            for character in input.pressed_characters {
                if !character.is_control() && address.len() < 180 { address.push(character); }
            }
            if input.enter_pressed {
                page_title = "Navigation requested".into();
                page_message = format!("The frontend received this address: {address}");
                address_focused = false;
            }
        }

        framebuffer.clear(Color::rgb(245, 247, 251));
        fill_rect(&mut framebuffer, Rect::new(0, 0, WIDTH, 72), Color::rgb(25, 32, 46));
        fill_rounded_rect(&mut framebuffer, Rect::new(32, 16, 42, 40), 9, Color::rgb(49, 61, 82));
        fill_rounded_rect(&mut framebuffer, Rect::new(82, 16, 42, 40), 9, Color::rgb(49, 61, 82));
        fill_rounded_rect(&mut framebuffer, Rect::new(132, 16, 42, 40), 9, Color::rgb(49, 61, 82));
        draw_text(&mut framebuffer, &font, "<", Point::new(47, 42), 19.0, Color::WHITE);
        draw_text(&mut framebuffer, &font, ">", Point::new(97, 42), 19.0, Color::WHITE);
        draw_text(&mut framebuffer, &font, "R", Point::new(146, 42), 17.0, Color::WHITE);
        fill_rounded_rect(&mut framebuffer, Rect::new(190, 16, 590, 40), 9, if address_focused { Color::WHITE } else { Color::rgb(236, 240, 247) });
        if address_focused { draw_rect(&mut framebuffer, Rect::new(190, 16, 590, 40), Color::rgb(76, 137, 232)); }
        draw_text(&mut framebuffer, &font, &address, Point::new(205, 42), 16.0, Color::rgb(35, 43, 57));
        fill_rounded_rect(&mut framebuffer, Rect::new(790, 16, 82, 40), 9, Color::rgb(55, 117, 220));
        draw_text(&mut framebuffer, &font, "Go", Point::new(817, 42), 16.0, Color::WHITE);
        draw_text(&mut framebuffer, &font, "GhostEngine Browser Preview", Point::new(44, 126), 15.0, Color::rgb(100, 112, 132));
        draw_text(&mut framebuffer, &font, &page_title, Point::new(44, 184), 32.0, Color::rgb(28, 37, 54));
        draw_text(&mut framebuffer, &font, &page_message, Point::new(44, 224), 18.0, Color::rgb(76, 87, 105));
        fill_rounded_rect(&mut framebuffer, Rect::new(44, 270, 872, 240), 16, Color::WHITE);
        draw_rect(&mut framebuffer, Rect::new(44, 270, 872, 240), Color::rgb(224, 229, 237));
        draw_text(&mut framebuffer, &font, "What this demo proves", Point::new(72, 318), 23.0, Color::rgb(32, 44, 64));
        draw_text(&mut framebuffer, &font, "- Native desktop window and framebuffer presentation", Point::new(72, 360), 17.0, Color::rgb(72, 84, 102));
        draw_text(&mut framebuffer, &font, "- Text rasterization using a system TrueType font", Point::new(72, 392), 17.0, Color::rgb(72, 84, 102));
        draw_text(&mut framebuffer, &font, "- Mouse hit-testing, address editing and keyboard input", Point::new(72, 424), 17.0, Color::rgb(72, 84, 102));
        draw_text(&mut framebuffer, &font, "This is an interactive browser shell, not an HTML/web engine yet.", Point::new(72, 472), 15.0, Color::rgb(115, 125, 140));
        draw_text(&mut framebuffer, &font, "Click the address bar, type, then press Enter or Go. Esc closes the window.", Point::new(44, 566), 15.0, Color::rgb(95, 106, 123));
        window.present(&framebuffer)?;
    }
    Ok(())
}

fn find_system_font() -> Option<PathBuf> {
    [
        PathBuf::from("C:/Windows/Fonts/segoeui.ttf"),
        PathBuf::from("C:/Windows/Fonts/arial.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf"),
    ].into_iter().find(|path| path.is_file())
}
