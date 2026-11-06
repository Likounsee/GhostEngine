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
pub use layout::{Insets, LayoutDirection, LayoutItem, ScrollViewport, StackLayout};
pub use path::{
    fill_path_even_odd, fill_path_even_odd_clipped, fill_transformed_path_even_odd,
    fill_transformed_path_even_odd_clipped, FlattenedPath, FlattenedSubpath, Path, PathError,
};
pub use rasterizer::{fill_rect_clipped, Gradient, GradientError, GradientStop};
pub use stroke::{stroke_path, stroke_path_clipped, StrokeCap, StrokeError, StrokeStyle};
pub use text::{draw_text, draw_text_clipped, FontFace, TextMetrics};
pub use texture::{draw_texture, draw_texture_clipped, Texture, TextureError, TextureFilter};
pub use transform::Transform2D;
pub use ui::{
    draw_button, draw_checkbox, draw_divider, draw_label, draw_label_clipped, draw_panel,
    draw_progress_bar, state_for_pointer, UiTheme, WidgetState,
};
