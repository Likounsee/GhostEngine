//! Ordered CPU display lists.
//!
//! Applications record drawing commands first, then replay them into a
//! framebuffer. Each command captures the active clip rectangle, so later
//! clip-stack changes cannot affect commands that were already recorded.

use crate::path::{fill_transformed_path_even_odd_clipped, Path, PathError};
use crate::rasterizer::{
    draw_line, fill_transformed_rect_linear_gradient_clipped,
    fill_transformed_rect_radial_gradient_clipped, Gradient,
};
use crate::shaping::{ShapedText, ShapingError};
use crate::stroke::{stroke_path_clipped, StrokeError, StrokeStyle};
use crate::text::{
    draw_text_box_transformed_clipped, draw_text_shaped_transformed_clipped,
    prepare_bidi_shaped_text_box, prepare_shaped_text_box, FontFace, FontFamily, TextLayoutOptions,
};
use crate::texture::{draw_texture_transformed_clipped, Texture, TextureFilter};
use crate::{Color, Framebuffer, Point, Rect, Transform2D};
use std::sync::Arc;

#[derive(Clone)]
struct SharedFont(Arc<FontFace>);

impl std::fmt::Debug for SharedFont {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SharedFont(..)")
    }
}

impl PartialEq for SharedFont {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Clear(Color),
    StrokeRect {
        path: Path,
        transform: Transform2D,
        color: Color,
        clip: Rect,
    },
    Line {
        start: Point,
        end: Point,
        color: Color,
        clip: Rect,
    },
    FillPath {
        path: Path,
        transform: Transform2D,
        color: Color,
        tolerance: f32,
        clip: Rect,
    },
    StrokePath {
        path: Path,
        transform: Transform2D,
        style: StrokeStyle,
        color: Color,
        tolerance: f32,
        clip: Rect,
    },
    DrawText {
        font: SharedFont,
        text: String,
        baseline: Point,
        transform: Transform2D,
        pixel_size: f32,
        color: Color,
        options: TextLayoutOptions,
        clip: Rect,
    },
    DrawShapedText {
        font: SharedFont,
        shaped: ShapedText,
        baseline: Point,
        transform: Transform2D,
        pixel_size: f32,
        color: Color,
        clip: Rect,
    },
    DrawTexture {
        texture: Arc<Texture>,
        destination: Rect,
        transform: Transform2D,
        filter: TextureFilter,
        clip: Rect,
    },
    LinearGradient {
        rect: Rect,
        start: crate::Vector,
        end: crate::Vector,
        transform: Transform2D,
        gradient: Gradient,
        clip: Rect,
    },
    RadialGradient {
        rect: Rect,
        center: crate::Vector,
        radius: f32,
        transform: Transform2D,
        gradient: Gradient,
        clip: Rect,
    },
}

/// Summary of commands executed by one display-list replay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub commands_submitted: usize,
    pub commands_executed: usize,
    pub commands_skipped: usize,
}

/// A reusable, ordered list of CPU drawing operations.
///
/// The list owns no framebuffer or platform resources. Recording and replay
/// are separate, which makes command generation testable without opening a
/// window and allows applications to rebuild only the lists that changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    commands: Vec<Command>,
    clips: Vec<Rect>,
    transforms: Vec<Transform2D>,
}

impl DisplayList {
    /// Creates an empty list with no explicit clip restriction.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            commands: Vec::new(),
            clips: Vec::new(),
            transforms: Vec::new(),
        }
    }

    /// Creates an empty display list with room for at least the requested
    /// number of drawing commands.
    #[must_use]
    pub fn with_capacity(command_capacity: usize) -> Self {
        Self {
            commands: Vec::with_capacity(command_capacity),
            clips: Vec::new(),
            transforms: Vec::new(),
        }
    }

    /// Reserves room for at least the requested number of additional drawing
    /// commands without reallocating the command buffer.
    ///
    /// Call this before recording a frame when the approximate command count
    /// is known. The reservation is retained by clear() for later frames.
    pub fn reserve_commands(&mut self, additional: usize) {
        self.commands.reserve(additional);
    }

    /// Returns the number of drawing commands that can be recorded before
    /// the command buffer needs to grow.
    #[must_use]
    pub fn command_capacity(&self) -> usize {
        self.commands.capacity()
    }

    /// Removes all commands and clip state while retaining allocated capacity.
    pub fn clear(&mut self) {
        self.commands.clear();
        self.clips.clear();
        self.transforms.clear();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Adds a clip rectangle. Nested clips are intersected with their parent.
    /// An empty intersection remains empty until the matching pop.
    pub fn push_clip(&mut self, clip: Rect) {
        let effective = self
            .clips
            .last()
            .map_or(Some(clip), |parent| parent.intersection(clip));
        self.clips.push(effective.unwrap_or_default());
    }

    /// Removes the most recently pushed clip. Returns false for an empty stack.
    pub fn pop_clip(&mut self) -> bool {
        self.clips.pop().is_some()
    }

    /// Pushes an affine transform for subsequent geometric drawing commands.
    /// Nested transforms are composed in push order:
    /// the existing transform is applied first, then the newly pushed transform.
    /// Non-finite matrices are rejected and leave the stack unchanged.
    pub fn push_transform(&mut self, transform: Transform2D) -> bool {
        if !transform.is_finite() {
            return false;
        }
        let combined = self
            .transforms
            .last()
            .copied()
            .unwrap_or(Transform2D::IDENTITY)
            .then(transform);
        if !combined.is_finite() {
            return false;
        }
        self.transforms.push(combined);
        true
    }

    /// Removes the most recently pushed transform. Returns false for an empty stack.
    pub fn pop_transform(&mut self) -> bool {
        self.transforms.pop().is_some()
    }

    fn active_transform(&self) -> Transform2D {
        self.transforms
            .last()
            .copied()
            .unwrap_or(Transform2D::IDENTITY)
    }

    /// Records a full-framebuffer clear. Clear deliberately ignores clip state.
    pub fn clear_color(&mut self, color: Color) {
        self.commands.push(Command::Clear(color));
    }

    /// Records a filled rectangle as a transformed vector contour.
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let _ = self.fill_path(rectangle_path(rect), Transform2D::IDENTITY, color, 0.25);
    }

    /// Records a one-pixel rectangle outline using the active affine transform.
    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::StrokeRect {
                path: stroke_rectangle_path(rect),
                transform: self.active_transform(),
                color,
                clip,
            });
        }
    }

    /// Records a one-pixel line. Its endpoints are transformed when recorded.
    pub fn line(&mut self, start: Point, end: Point, color: Color) {
        if let Some(clip) = self.active_clip() {
            let transform = self.active_transform();
            let start =
                transform.transform_point(crate::Vector::new(start.x as f32, start.y as f32));
            let end = transform.transform_point(crate::Vector::new(end.x as f32, end.y as f32));
            self.commands.push(Command::Line {
                start: Point::new(start.x.round() as i32, start.y.round() as i32),
                end: Point::new(end.x.round() as i32, end.y.round() as i32),
                color,
                clip,
            });
        }
    }

    /// Records a filled vector path. Geometry and transform are validated before
    /// the command is accepted, so replay does not silently discard invalid paths.
    pub fn fill_path(
        &mut self,
        path: Path,
        transform: Transform2D,
        color: Color,
        tolerance: f32,
    ) -> Result<(), PathError> {
        let transform = transform.then(self.active_transform());
        path.flatten(tolerance)?.transformed(transform)?;
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::FillPath {
                path,
                transform,
                color,
                tolerance,
                clip,
            });
        }
        Ok(())
    }

    /// Records a stroked vector path. Stroke width is measured in framebuffer
    /// pixels after the combined local and active transforms are applied.
    pub fn stroke_path(
        &mut self,
        path: Path,
        transform: Transform2D,
        style: StrokeStyle,
        color: Color,
        tolerance: f32,
    ) -> Result<(), StrokeError> {
        if !style.width.is_finite() || style.width <= 0.0 {
            return Err(StrokeError::InvalidWidth);
        }
        let transform = transform.then(self.active_transform());
        if !transform.is_finite() {
            return Err(StrokeError::InvalidTransform);
        }
        path.flatten(tolerance)?.transformed(transform)?;
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::StrokePath {
                path,
                transform,
                style,
                color,
                tolerance,
                clip,
            });
        }
        Ok(())
    }

    /// Records a text run using a shared font face. The baseline origin is transformed
    /// when recorded; glyphs remain screen-aligned until affine glyph rendering is added.
    /// Empty strings and invalid pixel sizes are ignored. Multiple commands can share one
    /// loaded font without copying its internal font data.
    pub fn draw_text(
        &mut self,
        font: Arc<FontFace>,
        text: impl Into<String>,
        baseline: Point,
        pixel_size: f32,
        color: Color,
    ) {
        self.draw_text_box(
            font,
            text,
            baseline,
            pixel_size,
            color,
            TextLayoutOptions::default(),
        );
    }

    /// Records a wrapped and aligned text block. Its first baseline is transformed
    /// when recorded; glyphs themselves remain screen-aligned.
    pub fn draw_text_box(
        &mut self,
        font: Arc<FontFace>,
        text: impl Into<String>,
        baseline: Point,
        pixel_size: f32,
        color: Color,
        options: TextLayoutOptions,
    ) {
        let text = text.into();
        if text.is_empty()
            || !pixel_size.is_finite()
            || pixel_size <= 0.0
            || !options.line_spacing.is_finite()
            || options.line_spacing <= 0.0
            || options
                .max_width
                .is_some_and(|width| !width.is_finite() || width <= 0.0)
        {
            return;
        }
        if let Some(clip) = self.active_clip() {
            let transform = self.active_transform();
            if transform.is_finite() && transform.inverse().is_some() {
                self.commands.push(Command::DrawText {
                    font: SharedFont(font),
                    text,
                    baseline,
                    transform,
                    pixel_size,
                    color,
                    options,
                    clip,
                });
            }
        }
    }

    /// Shapes and records a single OpenType text run.
    ///
    /// Shaping is performed once, while recording; replay only rasterizes the
    /// resulting glyph IDs. Newlines and wrapping are intentionally handled by
    /// the text-layout API rather than this single-run method.
    pub fn draw_shaped_text(
        &mut self,
        font: Arc<FontFace>,
        text: &str,
        baseline: Point,
        pixel_size: f32,
        color: Color,
    ) -> Result<(), ShapingError> {
        if !pixel_size.is_finite() || pixel_size <= 0.0 {
            return Err(ShapingError::InvalidPixelSize);
        }
        if text.is_empty() {
            return Ok(());
        }
        let shaped = font.shape_text(text, pixel_size)?;
        if shaped.glyphs.is_empty() {
            return Ok(());
        }
        if let Some(clip) = self.active_clip() {
            let transform = self.active_transform();
            if transform.is_finite() && transform.inverse().is_some() {
                self.commands.push(Command::DrawShapedText {
                    font: SharedFont(font),
                    shaped,
                    baseline,
                    transform,
                    pixel_size,
                    color,
                    clip,
                });
            }
        }
        Ok(())
    }

    /// Shapes, wraps and records a fallback-font text block once.
    ///
    /// Each fallback run is converted to glyph IDs while recording. Replaying
    /// the display list only rasterizes those glyph IDs and never reshapes text.
    pub fn draw_shaped_text_box(
        &mut self,
        family: Arc<FontFamily>,
        text: &str,
        baseline: Point,
        pixel_size: f32,
        color: Color,
        options: TextLayoutOptions,
    ) -> Result<(), ShapingError> {
        let prepared = prepare_shaped_text_box(&family, text, baseline, pixel_size, options)?;
        if prepared.lines.is_empty() {
            return Ok(());
        }
        if let Some(clip) = self.active_clip() {
            let transform = self.active_transform();
            if !transform.is_finite() || transform.inverse().is_none() {
                return Ok(());
            }
            for line in prepared.lines {
                for run in line.runs {
                    if run.shaped.glyphs.is_empty() {
                        continue;
                    }
                    let run_transform = Transform2D::translation(run.offset_x, 0.0).then(transform);
                    self.commands.push(Command::DrawShapedText {
                        font: SharedFont(run.font),
                        shaped: run.shaped,
                        baseline: line.baseline,
                        transform: run_transform,
                        pixel_size,
                        color,
                        clip,
                    });
                }
            }
        }
        Ok(())
    }

    /// Shapes, bidi-orders, wraps and records a fallback-font text block once.
    ///
    /// Directional runs and fallback-font spans are resolved while recording.
    /// Replay only rasterizes the prepared glyph IDs.
    pub fn draw_bidi_shaped_text_box(
        &mut self,
        family: Arc<FontFamily>,
        text: &str,
        baseline: Point,
        pixel_size: f32,
        color: Color,
        options: TextLayoutOptions,
    ) -> Result<(), ShapingError> {
        let prepared = prepare_bidi_shaped_text_box(&family, text, baseline, pixel_size, options)?;
        if prepared.lines.is_empty() {
            return Ok(());
        }
        if let Some(clip) = self.active_clip() {
            let transform = self.active_transform();
            if !transform.is_finite() || transform.inverse().is_none() {
                return Ok(());
            }
            for line in prepared.lines {
                for run in line.runs {
                    if run.shaped.glyphs.is_empty() {
                        continue;
                    }
                    let run_transform = Transform2D::translation(run.offset_x, 0.0).then(transform);
                    self.commands.push(Command::DrawShapedText {
                        font: SharedFont(run.font),
                        shaped: run.shaped,
                        baseline: line.baseline,
                        transform: run_transform,
                        pixel_size,
                        color,
                        clip,
                    });
                }
            }
        }
        Ok(())
    }

    /// Records an image command with its sampling mode, active transform and clip.
    ///
    /// The texture is moved into a shared allocation. Use draw_shared_texture
    /// when the same image is submitted by multiple commands or display lists.
    pub fn draw_texture(&mut self, texture: Texture, destination: Rect, filter: TextureFilter) {
        self.draw_shared_texture(Arc::new(texture), destination, filter);
    }

    /// Records an image command using shared texture storage.
    ///
    /// Cloning the Arc is constant-time and does not copy the pixel buffer.
    /// This is useful for applications that keep a texture asset alive and
    /// reference it from many frames or display lists.
    pub fn draw_shared_texture(
        &mut self,
        texture: Arc<Texture>,
        destination: Rect,
        filter: TextureFilter,
    ) {
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::DrawTexture {
                texture,
                destination,
                transform: self.active_transform(),
                filter,
                clip,
            });
        }
    }

    /// Records a linear-gradient rectangle. The rectangle and gradient coordinates
    /// are local to the active affine transform; clipping does not alter sampling.
    pub fn fill_linear_gradient(
        &mut self,
        rect: Rect,
        start: crate::Vector,
        end: crate::Vector,
        gradient: Gradient,
    ) {
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::LinearGradient {
                rect,
                start,
                end,
                transform: self.active_transform(),
                gradient,
                clip,
            });
        }
    }

    /// Records a radial-gradient rectangle. The center and radius are local to the
    /// active affine transform. The radius must be finite and positive;
    /// invalid radii are ignored during replay, matching the rasterizer contract.
    pub fn fill_radial_gradient(
        &mut self,
        rect: Rect,
        center: crate::Vector,
        radius: f32,
        gradient: Gradient,
    ) {
        if let Some(clip) = self.active_clip() {
            self.commands.push(Command::RadialGradient {
                rect,
                center,
                radius,
                transform: self.active_transform(),
                gradient,
                clip,
            });
        }
    }

    /// Executes commands in submission order and returns basic replay counters.
    pub fn replay(&self, target: &mut Framebuffer) -> RenderStats {
        let mut stats = RenderStats {
            commands_submitted: self.commands.len(),
            ..RenderStats::default()
        };

        for command in &self.commands {
            let executed = match command {
                Command::Clear(color) => {
                    target.clear(*color);
                    true
                }
                Command::StrokeRect {
                    path,
                    transform,
                    color,
                    clip,
                } => stroke_path_clipped(
                    target,
                    path,
                    *transform,
                    StrokeStyle::new(1.0),
                    *color,
                    0.25,
                    *clip,
                )
                .is_ok(),
                Command::Line {
                    start,
                    end,
                    color,
                    clip,
                } => {
                    if let Some((start, end)) = clip_line(*start, *end, *clip) {
                        draw_line(target, start, end, *color);
                        true
                    } else {
                        false
                    }
                }
                Command::FillPath {
                    path,
                    transform,
                    color,
                    tolerance,
                    clip,
                } => {
                    let result = fill_transformed_path_even_odd_clipped(
                        target, path, *transform, *color, *tolerance, *clip,
                    );
                    result.is_ok()
                }
                Command::StrokePath {
                    path,
                    transform,
                    style,
                    color,
                    tolerance,
                    clip,
                } => {
                    stroke_path_clipped(target, path, *transform, *style, *color, *tolerance, *clip)
                        .is_ok()
                }
                Command::DrawText {
                    font,
                    text,
                    baseline,
                    transform,
                    pixel_size,
                    color,
                    options,
                    clip,
                } => {
                    draw_text_box_transformed_clipped(
                        target,
                        &font.0,
                        text,
                        *baseline,
                        *pixel_size,
                        *color,
                        *clip,
                        *options,
                        *transform,
                    )
                    .line_count
                        > 0
                }
                Command::DrawShapedText {
                    font,
                    shaped,
                    baseline,
                    transform,
                    pixel_size,
                    color,
                    clip,
                } => {
                    draw_text_shaped_transformed_clipped(
                        target,
                        &font.0,
                        shaped,
                        *baseline,
                        *pixel_size,
                        *color,
                        *clip,
                        *transform,
                    )
                    .line_count
                        > 0
                }
                Command::DrawTexture {
                    texture,
                    destination,
                    transform,
                    filter,
                    clip,
                } => draw_texture_transformed_clipped(
                    target,
                    texture,
                    *destination,
                    *transform,
                    *filter,
                    *clip,
                ),
                Command::LinearGradient {
                    rect,
                    start,
                    end,
                    transform,
                    gradient,
                    clip,
                } => fill_transformed_rect_linear_gradient_clipped(
                    target, *rect, *transform, *start, *end, gradient, *clip,
                ),
                Command::RadialGradient {
                    rect,
                    center,
                    radius,
                    transform,
                    gradient,
                    clip,
                } => fill_transformed_rect_radial_gradient_clipped(
                    target, *rect, *transform, *center, *radius, gradient, *clip,
                ),
            };
            if executed {
                stats.commands_executed += 1;
            } else {
                stats.commands_skipped += 1;
            }
        }
        stats
    }

    fn active_clip(&self) -> Option<Rect> {
        match self.clips.last().copied() {
            Some(clip) if clip.width == 0 || clip.height == 0 => None,
            Some(clip) => Some(clip),
            None => Some(Rect::new(0, 0, u32::MAX, u32::MAX)),
        }
    }
}

fn rectangle_path(rect: Rect) -> Path {
    let left = rect.x as f32;
    let top = rect.y as f32;
    let right = rect.x.saturating_add(rect.width) as f32;
    let bottom = rect.y.saturating_add(rect.height) as f32;
    let mut path = Path::new();
    path.move_to(crate::Vector::new(left, top))
        .line_to(crate::Vector::new(right, top))
        .line_to(crate::Vector::new(right, bottom))
        .line_to(crate::Vector::new(left, bottom))
        .close();
    path
}

fn stroke_rectangle_path(rect: Rect) -> Path {
    let left = rect.x as f32 + 0.5;
    let top = rect.y as f32 + 0.5;
    let right = rect.x.saturating_add(rect.width) as f32 - 0.5;
    let bottom = rect.y.saturating_add(rect.height) as f32 - 0.5;
    let mut path = Path::new();
    path.move_to(crate::Vector::new(left, top))
        .line_to(crate::Vector::new(right, top))
        .line_to(crate::Vector::new(right, bottom))
        .line_to(crate::Vector::new(left, bottom))
        .close();
    path
}

fn clip_line(start: Point, end: Point, clip: Rect) -> Option<(Point, Point)> {
    if clip.width == 0 || clip.height == 0 {
        return None;
    }
    let left = f64::from(clip.x);
    let top = f64::from(clip.y);
    let right = (u64::from(clip.x) + u64::from(clip.width) - 1) as f64;
    let bottom = (u64::from(clip.y) + u64::from(clip.height) - 1) as f64;
    let (x0, y0) = (f64::from(start.x), f64::from(start.y));
    let (dx, dy) = (f64::from(end.x) - x0, f64::from(end.y) - y0);
    let p = [-dx, dx, -dy, dy];
    let q = [x0 - left, right - x0, y0 - top, bottom - y0];
    let (mut enter, mut leave) = (0.0_f64, 1.0_f64);

    for index in 0..4 {
        if p[index] == 0.0 {
            if q[index] < 0.0 {
                return None;
            }
            continue;
        }
        let ratio = q[index] / p[index];
        if p[index] < 0.0 {
            enter = enter.max(ratio);
        } else {
            leave = leave.min(ratio);
        }
        if enter > leave {
            return None;
        }
    }

    let point = |amount: f64| {
        Point::new(
            (x0 + amount * dx).round().clamp(left, right) as i32,
            (y0 + amount * dy).round().clamp(top, bottom) as i32,
        )
    };
    Some((point(enter), point(leave)))
}

#[cfg(test)]
mod tests {
    use super::DisplayList;
    use crate::{Color, Framebuffer, Point, Rect};

    #[test]
    fn display_list_capacity_is_reused_between_frames() {
        let mut list = DisplayList::with_capacity(8);
        assert!(list.command_capacity() >= 8);
        let initial_capacity = list.command_capacity();

        for index in 0..8 {
            list.fill_rect(Rect::new(index, 0, 1, 1), Color::WHITE);
        }
        assert_eq!(list.len(), 8);
        assert_eq!(list.command_capacity(), initial_capacity);

        list.clear();
        assert_eq!(list.len(), 0);
        assert_eq!(list.command_capacity(), initial_capacity);

        list.reserve_commands(16);
        assert!(list.command_capacity() >= 16);
    }

    #[test]
    fn commands_replay_in_submission_order() {
        let mut list = DisplayList::new();
        list.clear_color(Color::BLACK);
        list.fill_rect(Rect::new(1, 1, 4, 4), Color::RED);
        list.fill_rect(Rect::new(2, 2, 2, 2), Color::BLUE);

        let mut target = Framebuffer::new(8, 8, Color::WHITE).unwrap();
        let stats = list.replay(&mut target);

        assert_eq!(target.get_pixel(1, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 2), Some(Color::BLUE));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(stats.commands_submitted, 3);
        assert_eq!(stats.commands_executed, 3);
        assert_eq!(stats.commands_skipped, 0);
    }

    #[test]
    fn nested_clips_intersect_and_are_captured_per_command() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(1, 1, 6, 6));
        list.fill_rect(Rect::new(0, 0, 8, 8), Color::RED);
        list.push_clip(Rect::new(3, 3, 2, 2));
        list.fill_rect(Rect::new(0, 0, 8, 8), Color::BLUE);
        assert!(list.pop_clip());
        list.fill_rect(Rect::new(0, 0, 8, 8), Color::GREEN);

        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        list.replay(&mut target);

        assert_eq!(target.get_pixel(1, 1), Some(Color::GREEN));
        assert_eq!(target.get_pixel(2, 2), Some(Color::GREEN));
        assert_eq!(target.get_pixel(3, 3), Some(Color::GREEN));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(target.get_pixel(7, 7), Some(Color::BLACK));
    }

    #[test]
    fn empty_nested_clip_skips_drawing() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(1, 1, 2, 2));
        list.push_clip(Rect::new(6, 6, 2, 2));
        list.fill_rect(Rect::new(0, 0, 8, 8), Color::RED);

        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);

        assert_eq!(stats.commands_submitted, 0);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn line_is_clipped_to_active_viewport() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(2, 2, 3, 3));
        list.line(Point::new(0, 3), Point::new(7, 3), Color::WHITE);

        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);

        assert_eq!(target.get_pixel(1, 3), Some(Color::BLACK));
        assert_eq!(target.get_pixel(2, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(4, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(5, 3), Some(Color::BLACK));
        assert_eq!(stats.commands_executed, 1);
    }

    #[test]
    fn rectangle_outline_respects_clip_and_avoids_duplicate_edges() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(3, 0, 2, 5));
        list.stroke_rect(Rect::new(1, 1, 6, 3), Color::RED);

        let mut target = Framebuffer::new(8, 5, Color::BLACK).unwrap();
        list.replay(&mut target);

        assert_eq!(target.get_pixel(3, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(4, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(3, 2), Some(Color::BLACK));
        assert_eq!(target.get_pixel(4, 2), Some(Color::BLACK));
        assert_eq!(target.get_pixel(3, 3), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 1), Some(Color::BLACK));
    }

    #[test]
    fn filled_rectangle_uses_active_rotation_transform() {
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::rotation(std::f32::consts::FRAC_PI_2)));
        assert!(list.push_transform(crate::Transform2D::translation(5.0, 1.0)));
        list.fill_rect(Rect::new(0, 0, 3, 2), Color::GREEN);
        let mut target = Framebuffer::new(10, 10, Color::BLACK).unwrap();
        list.replay(&mut target);
        assert_eq!(target.get_pixel(3, 2), Some(Color::GREEN));
        assert_eq!(target.get_pixel(6, 2), Some(Color::BLACK));
        assert_eq!(target.get_pixel(4, 3), Some(Color::GREEN));
        assert_eq!(target.get_pixel(4, 4), Some(Color::BLACK));
    }

    #[test]
    fn rectangle_outline_uses_active_transform() {
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::translation(3.0, 2.0)));
        list.stroke_rect(Rect::new(0, 0, 4, 3), Color::RED);
        let mut target = Framebuffer::new(9, 8, Color::BLACK).unwrap();
        list.replay(&mut target);
        assert_eq!(target.get_pixel(3, 2), Some(Color::RED));
        assert_eq!(target.get_pixel(5, 2), Some(Color::RED));
        assert_eq!(target.get_pixel(4, 3), Some(Color::BLACK));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn line_uses_active_transform_when_recorded() {
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::translation(2.0, 1.0)));
        list.line(Point::new(1, 2), Point::new(5, 2), Color::WHITE);
        assert!(list.pop_transform());
        let mut target = Framebuffer::new(9, 6, Color::BLACK).unwrap();
        list.replay(&mut target);
        assert_eq!(target.get_pixel(3, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(1, 2), Some(Color::BLACK));
    }

    #[test]
    fn vector_path_is_replayed_with_transform_and_clip() {
        let mut path = crate::Path::new();
        path.move_to(crate::Vector::new(0.0, 0.0))
            .line_to(crate::Vector::new(6.0, 0.0))
            .line_to(crate::Vector::new(6.0, 6.0))
            .line_to(crate::Vector::new(0.0, 6.0))
            .close();
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(4, 3, 2, 2));
        list.fill_path(
            path,
            crate::Transform2D::translation(2.0, 1.0),
            Color::GREEN,
            0.25,
        )
        .unwrap();
        let mut target = Framebuffer::new(10, 10, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);
        assert_eq!(target.get_pixel(4, 3), Some(Color::GREEN));
        assert_eq!(target.get_pixel(3, 3), Some(Color::BLACK));
        assert_eq!(target.get_pixel(6, 3), Some(Color::BLACK));
        assert_eq!(stats.commands_executed, 1);
    }

    #[test]
    fn invalid_path_is_rejected_before_recording() {
        let mut path = crate::Path::new();
        path.line_to(crate::Vector::new(1.0, 1.0));
        let mut list = DisplayList::new();
        assert_eq!(
            list.fill_path(path, crate::Transform2D::IDENTITY, Color::RED, 0.25),
            Err(crate::PathError::MissingCurrentPoint),
        );
        assert!(list.is_empty());
    }

    #[test]
    fn stroked_vector_path_uses_active_transform_and_clip() {
        let mut path = crate::Path::new();
        path.move_to(crate::Vector::new(1.0, 2.0))
            .line_to(crate::Vector::new(6.0, 2.0));
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::translation(2.0, 1.0)));
        list.push_clip(Rect::new(3, 1, 3, 3));
        list.stroke_path(
            path,
            crate::Transform2D::IDENTITY,
            crate::StrokeStyle::new(2.0),
            Color::WHITE,
            0.25,
        )
        .unwrap();
        let mut target = Framebuffer::new(9, 6, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);
        assert_eq!(target.get_pixel(2, 2), Some(Color::BLACK));
        assert_eq!(target.get_pixel(3, 2), Some(Color::WHITE));
        assert_eq!(target.get_pixel(5, 2), Some(Color::WHITE));
        assert_eq!(target.get_pixel(6, 2), Some(Color::BLACK));
        assert_eq!(stats.commands_executed, 1);
    }

    #[test]
    fn invalid_stroke_is_rejected_before_recording() {
        let mut path = crate::Path::new();
        path.move_to(crate::Vector::new(0.0, 0.0))
            .line_to(crate::Vector::new(2.0, 0.0));
        let mut list = DisplayList::new();
        assert_eq!(
            list.stroke_path(
                path,
                crate::Transform2D::IDENTITY,
                crate::StrokeStyle::new(0.0),
                Color::RED,
                0.25,
            ),
            Err(crate::StrokeError::InvalidWidth),
        );
        assert!(list.is_empty());
    }

    #[test]
    fn texture_command_keeps_sampling_and_clip_in_order() {
        let texture = crate::Texture::new(2, 1, vec![Color::RED, Color::BLUE]).unwrap();
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(2, 0, 2, 1));
        list.draw_texture(
            texture,
            Rect::new(1, 0, 4, 1),
            crate::TextureFilter::Nearest,
        );
        let mut target = Framebuffer::new(6, 1, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);
        assert_eq!(target.get_pixel(1, 0), Some(Color::BLACK));
        assert_eq!(target.get_pixel(2, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(3, 0), Some(Color::BLUE));
        assert_eq!(target.get_pixel(4, 0), Some(Color::BLACK));
        assert_eq!(stats.commands_executed, 1);
    }

    #[test]
    fn shared_texture_commands_reuse_the_same_pixel_storage() {
        let texture = std::sync::Arc::new(
            crate::Texture::new(2, 1, vec![Color::RED, Color::BLUE]).unwrap(),
        );
        let mut list = DisplayList::new();
        list.draw_shared_texture(
            std::sync::Arc::clone(&texture),
            Rect::new(0, 0, 2, 1),
            crate::TextureFilter::Nearest,
        );
        list.draw_shared_texture(
            std::sync::Arc::clone(&texture),
            Rect::new(2, 0, 2, 1),
            crate::TextureFilter::Nearest,
        );

        let first = match &list.commands[0] {
            super::Command::DrawTexture { texture, .. } => texture,
            _ => panic!("expected first texture command"),
        };
        let second = match &list.commands[1] {
            super::Command::DrawTexture { texture, .. } => texture,
            _ => panic!("expected second texture command"),
        };
        assert!(std::sync::Arc::ptr_eq(first, second));
        assert!(std::sync::Arc::ptr_eq(first, &texture));

        let mut target = Framebuffer::new(4, 1, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);
        assert_eq!(target.pixels(), &[Color::RED, Color::BLUE, Color::RED, Color::BLUE]);
        assert_eq!(stats.commands_executed, 2);
    }

    #[test]
    fn texture_uses_active_transform_when_recorded() {
        let texture = crate::Texture::new(2, 1, vec![Color::RED, Color::BLUE]).unwrap();
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::translation(2.0, 1.0)));
        list.draw_texture(
            texture,
            Rect::new(0, 0, 2, 1),
            crate::TextureFilter::Nearest,
        );
        assert!(list.pop_transform());
        let mut target = Framebuffer::new(6, 4, Color::BLACK).unwrap();
        list.replay(&mut target);
        assert_eq!(target.get_pixel(2, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(3, 1), Some(Color::BLUE));
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn singular_texture_transform_is_counted_as_skipped() {
        let texture = crate::Texture::new(1, 1, vec![Color::WHITE]).unwrap();
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::scale(0.0, 1.0)));
        list.draw_texture(
            texture,
            Rect::new(0, 0, 1, 1),
            crate::TextureFilter::Nearest,
        );
        let mut target = Framebuffer::new(3, 3, Color::BLACK).unwrap();
        let stats = list.replay(&mut target);
        assert_eq!(stats.commands_submitted, 1);
        assert_eq!(stats.commands_executed, 0);
        assert_eq!(stats.commands_skipped, 1);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn nested_transforms_are_composed_and_captured_when_recording() {
        let mut path = crate::Path::new();
        path.move_to(crate::Vector::new(0.0, 0.0))
            .line_to(crate::Vector::new(2.0, 0.0))
            .line_to(crate::Vector::new(2.0, 2.0))
            .line_to(crate::Vector::new(0.0, 2.0))
            .close();

        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::translation(2.0, 1.0)));
        assert!(list.push_transform(crate::Transform2D::translation(1.0, 2.0)));
        list.fill_path(
            path.clone(),
            crate::Transform2D::IDENTITY,
            Color::GREEN,
            0.25,
        )
        .unwrap();
        assert!(list.pop_transform());
        assert!(list.pop_transform());
        list.fill_path(path, crate::Transform2D::IDENTITY, Color::RED, 0.25)
            .unwrap();

        let mut target = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        list.replay(&mut target);
        assert_eq!(target.get_pixel(3, 3), Some(Color::GREEN));
        assert_eq!(target.get_pixel(1, 1), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 1), Some(Color::BLACK));
    }

    #[test]
    fn invalid_transform_is_rejected_without_changing_stack() {
        let mut list = DisplayList::new();
        let invalid = crate::Transform2D::new(f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0);
        assert!(!list.push_transform(invalid));
        assert!(!list.pop_transform());
        assert!(list.push_transform(crate::Transform2D::translation(2.0, 0.0)));
        assert!(list.pop_transform());
    }

    #[test]
    fn clear_resets_transform_stack() {
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::translation(4.0, 0.0)));
        list.clear();
        assert!(!list.pop_transform());
    }

    fn black_to_white_gradient() -> crate::Gradient {
        crate::Gradient::new(vec![
            crate::GradientStop {
                position: 0.0,
                color: Color::BLACK,
            },
            crate::GradientStop {
                position: 1.0,
                color: Color::WHITE,
            },
        ])
        .unwrap()
    }

    #[test]
    fn linear_gradient_replays_with_original_coordinates_and_clip() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(2, 0, 2, 1));
        list.fill_linear_gradient(
            Rect::new(0, 0, 4, 1),
            crate::Vector::new(0.0, 0.5),
            crate::Vector::new(4.0, 0.5),
            black_to_white_gradient(),
        );
        let mut target = Framebuffer::new(4, 1, Color::RED).unwrap();
        let stats = list.replay(&mut target);

        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(1, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(2, 0), Some(Color::rgb(159, 159, 159)));
        assert_eq!(target.get_pixel(3, 0), Some(Color::rgb(223, 223, 223)));
        assert_eq!(stats.commands_executed, 1);
    }

    #[test]
    fn radial_gradient_respects_nested_clip() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(0, 0, 4, 4));
        list.push_clip(Rect::new(1, 1, 2, 2));
        list.fill_radial_gradient(
            Rect::new(0, 0, 4, 4),
            crate::Vector::new(1.5, 1.5),
            2.0,
            black_to_white_gradient(),
        );
        let mut target = Framebuffer::new(4, 4, Color::RED).unwrap();
        list.replay(&mut target);

        assert_eq!(target.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(target.get_pixel(1, 1), Some(Color::BLACK));
        assert_eq!(target.get_pixel(2, 2), Some(Color::rgb(180, 180, 180)));
        assert_eq!(target.get_pixel(3, 3), Some(Color::RED));
    }

    #[test]
    fn linear_gradient_uses_active_affine_transform() {
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::rotation(std::f32::consts::FRAC_PI_2,)));
        assert!(list.push_transform(crate::Transform2D::translation(4.0, 1.0)));
        list.fill_linear_gradient(
            Rect::new(0, 0, 4, 1),
            crate::Vector::new(0.0, 0.5),
            crate::Vector::new(4.0, 0.5),
            black_to_white_gradient(),
        );
        let mut target = Framebuffer::new(6, 6, Color::RED).unwrap();
        let stats = list.replay(&mut target);
        assert_eq!(target.get_pixel(3, 1), Some(Color::rgb(32, 32, 32)));
        assert_eq!(target.get_pixel(3, 4), Some(Color::rgb(223, 223, 223)));
        assert_eq!(target.get_pixel(4, 1), Some(Color::RED));
        assert_eq!(stats.commands_executed, 1);
    }

    #[test]
    fn radial_gradient_uses_active_affine_transform_and_clip() {
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::translation(2.0, 1.0)));
        list.push_clip(Rect::new(2, 1, 3, 3));
        list.fill_radial_gradient(
            Rect::new(0, 0, 4, 4),
            crate::Vector::new(2.0, 2.0),
            2.0,
            black_to_white_gradient(),
        );
        let mut target = Framebuffer::new(7, 7, Color::RED).unwrap();
        let stats = list.replay(&mut target);
        assert_eq!(target.get_pixel(4, 3), Some(Color::rgb(90, 90, 90)));
        assert_eq!(target.get_pixel(2, 1), Some(Color::WHITE));
        assert_eq!(target.get_pixel(5, 3), Some(Color::RED));
        assert_eq!(stats.commands_executed, 1);
    }

    #[test]
    fn singular_gradient_transform_is_skipped() {
        let mut list = DisplayList::new();
        assert!(list.push_transform(crate::Transform2D::scale(0.0, 1.0)));
        list.fill_linear_gradient(
            Rect::new(0, 0, 4, 4),
            crate::Vector::new(0.0, 0.0),
            crate::Vector::new(4.0, 4.0),
            black_to_white_gradient(),
        );
        let mut target = Framebuffer::new(4, 4, Color::RED).unwrap();
        let stats = list.replay(&mut target);
        assert_eq!(stats.commands_executed, 0);
        assert_eq!(stats.commands_skipped, 1);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::RED));
    }

    #[test]
    fn clear_resets_clip_independently_and_list_can_be_reused() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(1, 1, 2, 2));
        list.clear_color(Color::BLUE);
        list.fill_rect(Rect::new(0, 0, 4, 4), Color::RED);

        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        list.replay(&mut target);
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLUE));
        assert_eq!(target.get_pixel(1, 1), Some(Color::RED));

        list.clear();
        assert!(list.is_empty());
        list.clear_color(Color::GREEN);
        list.replay(&mut target);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::GREEN));
    }
}
