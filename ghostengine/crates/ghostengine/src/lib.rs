#![forbid(unsafe_code)]

//! GhostEngine's backend-independent foundations and CPU rendering primitives.
//! The crate intentionally has no graphics API or platform dependencies.

pub mod color;
pub mod framebuffer;
pub mod geometry;
pub mod rasterizer;\npub mod path;\npub mod transform;
pub mod text;

#[cfg(feature = "window")]
pub mod window;

pub use color::Color;
pub use framebuffer::{Framebuffer, FramebufferError};
pub use geometry::{Point, Rect, Vector};
pub use rasterizer::{Gradient, GradientError, GradientStop};\npub use path::{fill_path_even_odd, FlattenedPath, FlattenedSubpath, Path, PathError};\npub use transform::Transform2D;
pub use text::{draw_text, FontFace, TextMetrics};
