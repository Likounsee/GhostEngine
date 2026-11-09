//! CPU rasterization of stroked vector paths.
//!
//! The implementation evaluates stroke coverage at pixel centers and blends each
//! covered pixel once. This avoids dark seams where adjacent segments overlap.

use crate::{Color, Framebuffer, Path, PathError, Rect, Transform2D, Vector};

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
        Self {
            width,
            cap: StrokeCap::Butt,
        }
    }

    #[must_use]
    pub const fn with_cap(mut self, cap: StrokeCap) -> Self {
        self.cap = cap;
        self
    }
}

#[derive(Debug, Clone, Copy)]
struct StrokeOptions {
    clip: Rect,
    antialias: bool,
}

/// Errors returned by path stroking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeError {
    InvalidWidth,
    InvalidTransform,
    Path(PathError),
}

impl From<PathError> for StrokeError {
    fn from(value: PathError) -> Self {
        Self::Path(value)
    }
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
    let clip = Rect::new(0, 0, target.width(), target.height());
    stroke_path_clipped(target, path, transform, style, color, tolerance, clip)
}

/// Strokes a path while restricting pixel writes to a framebuffer-space viewport.
///
/// Geometry and stroke width are evaluated exactly as in the regular stroke API;
/// the viewport only limits rasterization, without changing cap or join geometry.
pub fn stroke_path_clipped(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    style: StrokeStyle,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), StrokeError> {
    stroke_path_impl(
        target,
        path,
        transform,
        style,
        color,
        tolerance,
        StrokeOptions {
            clip,
            antialias: false,
        },
    )
}

/// Strokes a path with 4x4 supersampled edge coverage.
pub fn stroke_path_aa(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    style: StrokeStyle,
    color: Color,
    tolerance: f32,
) -> Result<(), StrokeError> {
    let clip = Rect::new(0, 0, target.width(), target.height());
    stroke_path_aa_clipped(target, path, transform, style, color, tolerance, clip)
}

/// Antialiased path stroke restricted to a framebuffer-space viewport.
pub fn stroke_path_aa_clipped(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    style: StrokeStyle,
    color: Color,
    tolerance: f32,
    clip: Rect,
) -> Result<(), StrokeError> {
    stroke_path_impl(
        target,
        path,
        transform,
        style,
        color,
        tolerance,
        StrokeOptions {
            clip,
            antialias: true,
        },
    )
}

fn stroke_path_impl(
    target: &mut Framebuffer,
    path: &Path,
    transform: Transform2D,
    style: StrokeStyle,
    color: Color,
    tolerance: f32,
    options: StrokeOptions,
) -> Result<(), StrokeError> {
    let StrokeOptions { clip, antialias } = options;
    if !style.width.is_finite() || style.width <= 0.0 {
        return Err(StrokeError::InvalidWidth);
    }
    if !transform.is_finite() {
        return Err(StrokeError::InvalidTransform);
    }
    let flattened = path.flatten(tolerance)?.transformed(transform)?;
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return Ok(());
    };
    if target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
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
        let left = (min_x - expand)
            .floor()
            .max(clip.x as f32)
            .min(clip_right as f32) as u32;
        let top = (min_y - expand)
            .floor()
            .max(clip.y as f32)
            .min(clip_bottom as f32) as u32;
        let right = (max_x + expand)
            .ceil()
            .max(clip.x as f32)
            .min(clip_right as f32) as u32;
        let bottom = (max_y + expand)
            .ceil()
            .max(clip.y as f32)
            .min(clip_bottom as f32) as u32;
        if left >= right || top >= bottom {
            continue;
        }

        let segment_count = if contour.closed {
            contour.points.len()
        } else {
            contour.points.len() - 1
        };
        for y in top..bottom {
            let Some(row) = target.row_mut(y) else {
                continue;
            };
            for x in left..right {
                let coverage = if antialias {
                    const OFFSETS: [f32; 4] = [0.125, 0.375, 0.625, 0.875];
                    let mut covered = 0_u16;
                    for offset_y in OFFSETS {
                        for offset_x in OFFSETS {
                            let sample = Vector::new(x as f32 + offset_x, y as f32 + offset_y);
                            let mut distance = f32::INFINITY;
                            for i in 0..segment_count {
                                let a = contour.points[i];
                                let b = contour.points[(i + 1) % contour.points.len()];
                                let first = !contour.closed && i == 0;
                                let last = !contour.closed && i + 1 == segment_count;
                                distance = distance.min(segment_distance(
                                    sample, a, b, first, last, style.cap, radius,
                                ));
                            }
                            if distance <= radius {
                                covered += 1;
                            }
                        }
                    }
                    f32::from(covered) / 16.0
                } else {
                    let sample = Vector::new(x as f32 + 0.5, y as f32 + 0.5);
                    let mut distance = f32::INFINITY;
                    for i in 0..segment_count {
                        let a = contour.points[i];
                        let b = contour.points[(i + 1) % contour.points.len()];
                        let first = !contour.closed && i == 0;
                        let last = !contour.closed && i + 1 == segment_count;
                        distance = distance.min(segment_distance(
                            sample, a, b, first, last, style.cap, radius,
                        ));
                    }
                    // Legacy pixel-center coverage keeps the existing stroke appearance.
                    (radius + 0.5 - distance).clamp(0.0, 1.0)
                };
                let alpha = (f32::from(color.a) * coverage).round() as u8;
                if alpha == 0 {
                    continue;
                }
                let src = Color::rgba(color.r, color.g, color.b, alpha);
                let index = x as usize;
                row[index] = src.blend_over(row[index]);
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
    let extension = if cap == StrokeCap::Square {
        radius / length
    } else {
        0.0
    };
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
    use super::{stroke_path, stroke_path_clipped, StrokeCap, StrokeError, StrokeStyle};
    use crate::{Color, Framebuffer, Path, Rect, Transform2D, Vector};

    fn line() -> Path {
        let mut path = Path::new();
        path.move_to(Vector::new(3.0, 5.0))
            .line_to(Vector::new(9.0, 5.0));
        path
    }

    #[test]
    fn strokes_horizontal_path_with_requested_width() {
        let mut fb = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        stroke_path(
            &mut fb,
            &line(),
            Transform2D::IDENTITY,
            StrokeStyle::new(4.0),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        assert_eq!(fb.get_pixel(6, 4), Some(Color::WHITE));
        assert_eq!(fb.get_pixel(6, 5), Some(Color::WHITE));
        assert_eq!(fb.get_pixel(6, 6), Some(Color::WHITE));
        assert_eq!(fb.get_pixel(6, 2), Some(Color::BLACK));
    }

    #[test]
    fn round_cap_reaches_beyond_endpoint() {
        let mut butt = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        let mut round = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        stroke_path(
            &mut butt,
            &line(),
            Transform2D::IDENTITY,
            StrokeStyle::new(4.0),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        stroke_path(
            &mut round,
            &line(),
            Transform2D::IDENTITY,
            StrokeStyle::new(4.0).with_cap(StrokeCap::Round),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        assert_eq!(butt.get_pixel(2, 5), Some(Color::BLACK));
        assert_eq!(round.get_pixel(2, 5), Some(Color::WHITE));
    }

    #[test]
    fn transformed_stroke_is_clipped_to_target() {
        let mut fb = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        let transform = Transform2D::translation(2.0, 1.0);
        stroke_path(
            &mut fb,
            &line(),
            transform,
            StrokeStyle::new(2.0),
            Color::RED,
            0.2,
        )
        .unwrap();
        assert!(fb.pixels().contains(&Color::RED));
    }

    #[test]
    fn clipped_stroke_never_writes_outside_viewport() {
        let mut fb = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        stroke_path_clipped(
            &mut fb,
            &line(),
            Transform2D::IDENTITY,
            StrokeStyle::new(4.0).with_cap(StrokeCap::Round),
            Color::RED,
            0.2,
            Rect::new(5, 4, 2, 3),
        )
        .unwrap();
        for y in 0..12 {
            for x in 0..14 {
                let inside = (5..7).contains(&x) && y == 3;
                assert_eq!(fb.get_pixel(x, y) != Some(Color::BLACK), inside);
            }
        }
    }

    #[test]
    fn empty_stroke_clip_is_noop() {
        let mut fb = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        stroke_path_clipped(
            &mut fb,
            &line(),
            Transform2D::IDENTITY,
            StrokeStyle::new(2.0),
            Color::RED,
            0.2,
            Rect::new(8, 8, 2, 2),
        )
        .unwrap();
        assert!(fb.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn antialiased_stroke_blends_partial_edge_coverage() {
        let mut path = Path::new();
        path.move_to(Vector::new(2.0, 3.25))
            .line_to(Vector::new(10.0, 3.25));
        let mut fb = Framebuffer::new(14, 9, Color::BLACK).unwrap();
        super::stroke_path_aa(
            &mut fb,
            &path,
            Transform2D::IDENTITY,
            StrokeStyle::new(2.0),
            Color::WHITE,
            0.2,
        )
        .unwrap();

        assert!(fb.pixels().iter().any(|pixel| pixel.r > 0 && pixel.r < 255));
        assert!(fb.pixels().contains(&Color::WHITE));
    }

    #[test]
    fn antialiased_stroke_respects_clip_and_source_alpha() {
        let mut path = Path::new();
        path.move_to(Vector::new(3.0, 5.25))
            .line_to(Vector::new(9.0, 5.25));
        let mut fb = Framebuffer::new(14, 12, Color::BLACK).unwrap();
        super::stroke_path_aa_clipped(
            &mut fb,
            &path,
            Transform2D::IDENTITY,
            StrokeStyle::new(4.0).with_cap(StrokeCap::Round),
            Color::rgba(255, 0, 0, 128),
            0.2,
            Rect::new(5, 3, 2, 1),
        )
        .unwrap();

        for y in 0..12 {
            for x in 0..14 {
                let pixel = fb.get_pixel(x, y).unwrap();
                let inside = (5..7).contains(&x) && y == 3;
                if !inside {
                    assert_eq!(pixel, Color::BLACK);
                } else {
                    assert!(pixel.r <= 128);
                }
            }
        }
        assert!(fb.pixels().iter().any(|pixel| pixel.r > 0 && pixel.r < 128));
    }

    #[test]
    fn invalid_width_is_rejected() {
        let mut fb = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        assert_eq!(
            stroke_path(
                &mut fb,
                &line(),
                Transform2D::IDENTITY,
                StrokeStyle::new(f32::NAN),
                Color::WHITE,
                0.2,
            ),
            Err(StrokeError::InvalidWidth)
        );
    }
}
