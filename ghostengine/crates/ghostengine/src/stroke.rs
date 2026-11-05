//! CPU rasterization of stroked vector paths.
 //!
 //! The implementation evaluates stroke coverage at pixel centers and blends each
 //! covered pixel once. This avoids dark seams where adjacent segments overlap.

use crate::{Color, Framebuffer, Path, PathError, Transform2D, Vector};

/// Shape used to terminate an open stroked contour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeCap {
    /// Ends exactly at the contour endpoint.
    Butt,
    /// Extends the contour by half the stroke width.
    Square,
    /// Adds a semicircular end with radius half the stroke width.
    Round,
}

/// Stroke parameters. Joins are round in this first stable implementation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: StrokeCap,
}

impl StrokeStyle {
    #[must_use]
    pub const fn new(width: f32) -> Self {
        Self { width, cap: StrokeCap::Butt }
    }

    #[must_use]
    pub const fn with_cap(mut self, cap: StrokeCap) -> Self {
        self.cap = cap;
        self
    }
}

/// Errors returned by path stroking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeError {
    InvalidWidth,
    InvalidTransform,
    Path(PathError),
}

impl From<PathError> for StrokeError {
    fn from(value: PathError) -> Self { Self::Path(value) }
}

/// Flattens and strokes a path using a transform and pixel-space stroke width.
///
/// Width is measured after transformation. Curves use the supplied flattening
/// tolerance. Open contours honor the selected cap; all joins are round.
/// Pixels receive one source-over blend even when multiple segments overlap.
pub fn stroke_path(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    style: StrokeStyle,
    color: Color,
    tolerance: f32,
) -> Result<(), StrokeError> {
    if !style.width.is_finite() || style.width <= 0.0 {
        return Err(StrokeError::InvalidWidth);
    }
    if !transform.is_finite() {
        return Err(StrokeError::InvalidTransform);
    }
    if target.width() == 0 || target.height() == 0 {
        return Ok(());
    }

    let flattened = path.flatten(tolerance)?.transformed(transform)?;
    let radius = style.width * 0.5;
    if !radius.is_finite() {
        return Err(StrokeError::InvalidWidth);
    }

    for contour in flattened.subpaths {
        if contour.points.len() < 2 {
            continue;
        }
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for p in &contour.points {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
        let expand = radius + 1.0;
        let left = (min_x - expand).floor().max(0.0).min(target.width() as f32) as u32;
        let top = (min_y - expand).floor().max(0.0).min(target.height() as f32) as u32;
        let right = (max_x + expand).ceil().max(0.0).min(target.width() as f32) as u32;
        let bottom = (max_y + expand).ceil().max(0.0).min(target.height() as f32) as u32;
        if left >= right || top >= bottom { continue; }

        let segment_count = if contour.closed { contour.points.len() } else { contour.points.len() - 1 };
        for y in top..bottom {
            for x in left..right {
                let sample = Vector::new(x as f32 + 0.5, y as f32 + 0.5);
                let mut distance = f32::INFINITY;
                for i in 0..segment_count {
                    let a = contour.points[i];
                    let b = contour.points[(i + 1) % contour.points.len()];
                    let first = !contour.closed && i == 0;
                    let last = !contour.closed && i + 1 == segment_count;
                    distance = distance.min(segment_distance(sample, a, b, first, last, style.cap, radius));
                }
                // Round joins are the union of the round segment ends.
                let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
                let alpha = (f32::from(color.a) * coverage).round() as u8;
                if alpha == 0 { continue; }
                let src = Color::rgba(color.r, color.g, color.b, alpha);
                if let Some(dst) = target.get_pixel(x, y) {
                    target.set_pixel(x, y, src.blend_over(dst));
                }
            }
        }
    }
    Ok(())
}

fn segment_distance(
    p: Vector,
    a: Vector,
    b: Vector,
    first: bool,
    last: bool,
    cap: StrokeCap,
    radius: f32,
) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_squared = dx * dx + dy * dy;
    if length_squared <= f32::EPSILON {
        return (p.x - a.x).hypot(p.y - a.y);
    }
    let length = length_squared.sqrt();
    let mut t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / length_squared;
    let extension = if cap == StrokeCap::Square { radius / length } else { 0.0 };
    let low = if first { -extension } else { 0.0 };
    let high = if last { 1.0 + extension } else { 1.0 };
    if cap == StrokeCap::Butt && ((first && t < 0.0) || (last && t > 1.0)) {
        return f32::INFINITY;
    }
    if cap == StrokeCap::Round {
        t = t.clamp(0.0, 1.0);
    } else {
        t = t.clamp(low, high);
    }
    let nearest_x = a.x + t * dx;
    let nearest_y = a.y + t * dy;
    (p.x - nearest_x).hypot(p.y - nearest_y)
}

#[cfg(test)]
mod tests {
    use super::{stroke_path, StrokeCap, StrokeError, StrokeStyle};
    use crate::{Color, Framebuffer, Path, Transform2D, Vector};

    fn line() -> Path {
        let mut path = Path::new();
        path.move_to(Vector::new(3.0, 5.0)).line_to(Vector::new(9.0, 5.0));
        path
    }

    #[test]
    fn strokes_horizontal_path_with_requested_width() {
        let mut fb = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        stroke_path(&mut fb, &line(), Transform2D::IDENTITY, StrokeStyle::new(4.0), Color::WHITE, 0.2).unwrap();
        assert_eq!(fb.get_pixel(6, 4), Some(Color::WHITE));
        assert_eq!(fb.get_pixel(6, 5), Some(Color::WHITE));
        assert_eq!(fb.get_pixel(6, 6), Some(Color::WHITE));
        assert_eq!(fb.get_pixel(6, 2), Some(Color::BLACK));
    }

    #[test]
    fn round_cap_reaches_beyond_endpoint() {
        let mut butt = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        let mut round = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        stroke_path(&mut butt, &line(), Transform2D::IDENTITY, StrokeStyle::new(4.0), Color::WHITE, 0.2).unwrap();
        stroke_path(&mut round, &line(), Transform2D::IDENTITY, StrokeStyle::new(4.0).with_cap(StrokeCap::Round), Color::WHITE, 0.2).unwrap();
        assert_eq!(butt.get_pixel(2, 5), Some(Color::BLACK));
        assert_eq!(round.get_pixel(2, 5), Some(Color::WHITE));
    }

    #[test]
    fn transformed_stroke_is_clipped_to_target() {
        let mut fb = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        let transform = Transform2D::translation(2.0, 1.0);
        stroke_path(&mut fb, &line(), transform, StrokeStyle::new(2.0), Color::RED, 0.2).unwrap();
        assert!(fb.pixels().iter().any(|p| *p == Color::RED));
    }

    #[test]
    fn invalid_width_is_rejected() {
        let mut fb = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        assert_eq!(
            stroke_path(&mut fb, &line(), Transform2D::IDENTITY, StrokeStyle::new(f32::NAN), Color::WHITE, 0.2),
            Err(StrokeError::InvalidWidth)
        );
    }
}
