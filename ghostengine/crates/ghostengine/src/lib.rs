#![forbid(unsafe_code)]

//! GhostEngine's backend-independent foundations and CPU rendering primitives.
//! The crate intentionally has no graphics API or platform dependencies.

pub mod color;
pub mod framebuffer;
pub mod geometry;
pub mod path;
pub mod rasterizer;
pub mod text;
pub mod transform;

#[cfg(feature = "window")]
pub mod window;

pub use color::Color;
pub use framebuffer::{Framebuffer, FramebufferError};
pub use geometry::{Point, Rect, Vector};
pub use path::{fill_path_even_odd, fill_transformed_path_even_odd, FlattenedPath, FlattenedSubpath, Path, PathError};
pub use rasterizer::{Gradient, GradientError, GradientStop};
pub use text::{draw_text, FontFace, TextMetrics};
pub use transform::Transform2D;
