//! Native desktop UI demo using GhostEngine's retained components.
//!
//! Run with: cargo run -p ghostengine --example native_ui_demo --features native-window

use ghostengine::{
    ComponentId, ComponentTree, FontFace, Insets, LayoutDirection, LayoutItem, NativeWindowSurface,
    Rect, StackLayout, UiComponent, UiTheme,
};
use std::{error::Error, fs, path::PathBuf};

const WIDTH: u32 = 900;
const HEIGHT: u32 = 620;
const TITLE: ComponentId = ComponentId(1);
const ADDRESS: ComponentId = ComponentId(2);
const ENABLED: ComponentId = ComponentId(3);
const ACTION: ComponentId = ComponentId(4);
const STATUS: ComponentId = ComponentId(5);

fn main() -> Result<(), Box<dyn Error>> {
    let font_path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(find_system_font)
        .ok_or("No system TTF font found; pass a .ttf path as the first argument")?;
    let font_data = fs::read(font_path)?;
    let font = FontFace::from_bytes(&font_data)
        .map_err(|error| format!("Could not load font: {error}"))?;

    let panel = UiComponent::panel(
        ComponentId(0),
        StackLayout::new(LayoutDirection::Vertical)
            .with_padding(Insets::all(28))
            .with_gap(14),
    )
    .with_layout_item(LayoutItem::flexible())
    .with_children(vec![
        UiComponent::label(TITLE, "GhostEngine native UI").with_layout_item(LayoutItem::fixed(42)),
        UiComponent::label(
            ComponentId(6),
            "CPU-rendered components, native input, IME and system clipboard",
        )
        .with_layout_item(LayoutItem::fixed(32)),
        UiComponent::text_input(ADDRESS, "Type text here…", "Hello from GhostEngine")
            .with_layout_item(LayoutItem::fixed(52)),
        UiComponent::checkbox(ENABLED, "Enable this option", true)
            .with_layout_item(LayoutItem::fixed(42)),
        UiComponent::button(ACTION, "Apply").with_layout_item(LayoutItem::fixed(48)),
        UiComponent::label(
            STATUS,
            "Click the input, type, then try Ctrl+A / Ctrl+C / Ctrl+V.",
        )
        .with_layout_item(LayoutItem::flexible()),
    ]);
    let mut tree = ComponentTree::new(vec![panel]);
    let theme = UiTheme::default();

    NativeWindowSurface::run(
        "GhostEngine — Native component demo",
        WIDTH,
        HEIGHT,
        move |framebuffer, input, clipboard| {
            if input.escape_down {
                return false;
            }

            let bounds = Rect::new(0, 0, framebuffer.width(), framebuffer.height());
            let events = tree.handle_window_input(bounds, &input);
            if let Some(action) = events.clipboard_action {
                match clipboard.resolve(&action) {
                    Ok(Some(text)) => {
                        let paste = ghostengine::window::InputState {
                            paste_pressed: true,
                            clipboard_text: Some(text),
                            ..Default::default()
                        };
                        tree.handle_window_input(bounds, &paste);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if let Some(status) = tree.find_mut(STATUS) {
                            status.set_text(format!("Clipboard error: {error}"));
                        }
                    }
                }
            }
            if events.activated == Some(ACTION) {
                let value = match tree.find(ENABLED).map(UiComponent::kind) {
                    Some(ghostengine::ComponentKind::Checkbox { checked: true, .. }) => "enabled",
                    _ => "disabled",
                };
                if let Some(status) = tree.find_mut(STATUS) {
                    status.set_text(format!("Applied: option {value}."));
                }
            }

            framebuffer.clear(ghostengine::Color::rgb(239, 242, 248));
            tree.render(framebuffer, bounds, &font, 19.0, &theme);
            true
        },
    )?;
    Ok(())
}

fn find_system_font() -> Option<PathBuf> {
    [
        PathBuf::from("C:/Windows/Fonts/segoeui.ttf"),
        PathBuf::from("C:/Windows/Fonts/arial.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}
