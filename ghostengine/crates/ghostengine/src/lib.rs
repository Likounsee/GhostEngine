#![forbid(unsafe_code)]

//! GhostEngine's backend-independent foundations and CPU rendering primitives.
//! The crate intentionally has no graphics API or platform dependencies.

pub mod color;
pub mod composition;
pub mod framebuffer;
pub mod geometry;
pub mod layout;
pub mod path;
pub mod rasterizer;
pub mod stroke;
pub mod text;
pub mod texture;
pub mod transform;
pub mod ui;

#[cfg(feature = "window")]
pub mod window;

pub use color::Color;
pub use composition::{compose_layers, Layer};
pub use framebuffer::{Framebuffer, FramebufferError};
pub use geometry::{Point, Rect, Vector};
pub use layout::{Insets, LayoutDirection, LayoutItem, StackLayout};
pub use path::{
    fill_path_even_odd, fill_transformed_path_even_odd, FlattenedPath, FlattenedSubpath, Path,
    PathError,
};
pub use rasterizer::{fill_rect_clipped, Gradient, GradientError, GradientStop};
pub use stroke::{stroke_path, StrokeCap, StrokeError, StrokeStyle};
pub use text::{draw_text, FontFace, TextMetrics};
pub use texture::{draw_texture, Texture, TextureError, TextureFilter};
pub use transform::Transform2D;
pub use ui::{
    draw_button, draw_checkbox, draw_divider, draw_label, draw_panel, draw_progress_bar,
    state_for_pointer, UiTheme, WidgetState,
};
