Warning: truncated output (original token count: 29633)
Total output lines: 3360

//! Deterministic CPU rasterization primitives.
//!
//! Lines are clipped before rasterization; rectangles use half-open bounds;
//! triangles sample pixel centers and use a top-left edge rule.

use crate::{Color, Framebuffer, Point, Rect};

/// Writes a pixel without blending. Returns false when outside the target.
pub fn draw_pixel(target: &mut Framebuffer, point: Point, color: Color) -> bool {
    if point.x < 0 || point.y < 0 {
        return false;
    }
    target.set_pixel(point.x as u32, point.y as u32, color)
}

/// Composites a pixel over the existing target using source-over alpha.
pub fn blend_pixel(target: &mut Framebuffer, point: Point, color: Color) -> bool {
    if point.x < 0 || point.y < 0 {
        return false;
    }
    let (x, y) = (point.x as u32, point.y as u32);
    let Some(background) = target.get_pixel(x, y) else {
        return false;
    };
    target.set_pixel(x, y, color.blend_over(background))
}

/// Draws a one-pixel-wide line using Bresenham's integer algorithm.
/// The segment is clipped first, so distant off-screen endpoints are inexpensive.
pub fn draw_line(target: &mut Framebuffer, start: Point, end: Point, color: Color) {
    let Some((mut x0, mut y0, mut x1, mut y1)) =
        clip_line(target.width(), target.height(), start, end)
    else {
        return;
    };

    // Axis-aligned lines are common in UI and 2D scenes. Use contiguous
    // framebuffer writes instead of stepping through Bresenham one pixel at a time.
    if y0 == y1 {
        let left = x0.min(x1) as usize;
        let right = x0.max(x1) as usize;
        if let Some(row) = target.row_mut(y0 as u32) {
            row[left..=right].fill(color);
        }
        return;
    }
    if x0 == x1 {
        let x = x0 as usize;
        for y in y0.min(y1)..=y0.max(y1) {
            if let Some(row) = target.row_mut(y as u32) {
                row[x] = color;
            }
        }
        return;
    }

    // Canonical endpoint order makes Bresenham's tie-breaking independent
    // of the order in which the caller supplied the segment.
    if (x0, y0) > (x1, y1) {
        core::mem::swap(&mut x0, &mut x1);
        core::mem::swap(&mut y0, &mut y1);
    }

    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;

    loop {
        target.set_pixel(x0 as u32, y0 as u32, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled_error = 2 * error;
        if doubled_error >= dy {
            error += dy;
            x0 += sx;
        }
        if doubled_error <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn clip_line(width: u32, height: u32, start: Point, end: Point) -> Option<(i64, i64, i64, i64)> {
    if width == 0 || height == 0 {
        return None;
    }
    let xmax = f64::from(width.saturating_sub(1).min(i32::MAX as u32));
    let ymax = f64::from(height.saturating_sub(1).min(i32::MAX as u32));
    let (x0, y0) = (f64::from(start.x), f64::from(start.y));
    let (dx, dy) = (f64::from(end.x) - x0, f64::from(end.y) - y0);

    // Liang-Barsky parametric clipping.
    let p = [-dx, dx, -dy, dy];
    let q = [x0, xmax - x0, y0, ymax - y0];
    let (mut enter, mut leave) = (0.0_f64, 1.0_f64);
    for i in 0..4 {
        if p[i] == 0.0 {
            if q[i] < 0.0 {
                return None;
            }
            continue;
        }
        let ratio = q[i] / p[i];
        if p[i] < 0.0 {
            enter = enter.max(ratio);
        } else {
            leave = leave.min(ratio);
        }
        if enter > leave {
            return None;
        }
    }

    let round = |v: f64, max: f64| v.round().clamp(0.0, max) as i64;
    Some((
        round(x0 + enter * dx, xmax),
        round(y0 + enter * dy, ymax),
        round(x0 + leave * dx, xmax),
        round(y0 + leave * dy, ymax),
    ))
}

/// Draws an anti-aliased line using Xiaolin Wu coverage sampling.
///
/// Coverage is composited over the target, so edge pixels retain smooth alpha.
/// Endpoints are clipped to the framebuffer before rasterization.
pub fn draw_line_aa(target: &mut Framebuffer, start: Point, end: Point, color: Color) {
    let Some((mut x0, mut y0, mut x1, mut y1)) =
        clip_line(target.width(), target.height(), start, end)
    else {
        return;
    };

    // Integer-aligned opaque horizontal and vertical strokes have full
    // coverage. Reuse the span/column path and skip floating-point Wu sampling.
    if color.a == u8::MAX && (y0 == y1 || x0 == x1) {
        draw_line(target, start, end, color);
        return;
    }

    let steep = (y1 - y0).abs() > (x1 - x0).abs();
    if steep {
        core::mem::swap(&mut x0, &mut y0);
        core::mem::swap(&mut x1, &mut y1);
    }
    if x0 > x1 {
        core::mem::swap(&mut x0, &mut x1);
        core::mem::swap(&mut y0, &mut y1);
    }
    if x0 == x1 {
        draw_line(target, start, end, color);
        return;
    }

    let gradient = (y1 - y0) as f64 / (x1 - x0) as f64;
    let mut y = y0 as f64;
    for x in x0..=x1 {
        let lower = y.floor();
        let upper_coverage = y - lower;
        let lower_coverage = 1.0 - upper_coverage;
        let plot = |target: &mut Framebuffer, px: i64, py: i64, coverage: f64| {
            let (px, py) = if steep { (py, px) } else { (px, py) };
            if px < 0
                || py < 0
                || px >= i64::from(target.width())
                || py >= i64::from(target.height())
            {
                return;
            }
            let alpha = (f64::from(color.a) * coverage.clamp(0.0, 1.0)).round() as u8;
            if alpha == 0 {
                return;
            }
            let Some(row) = target.row_mut(py as u32) else {
                return;
            };
            let pixel = &mut row[px as usize];
            let source = Color::rgba(color.r, color.g, color.b, alpha);
            *pixel = if alpha == u8::MAX {
                source
            } else {
                source.blend_over(*pixel)
            };
        };
        plot(target, x, lower as i64, lower_coverage);
        plot(target, x, lower as i64 + 1, upper_coverage);
        y += gradient;
    }
}

/// Fills a rectangle clipped to the framebuffer. Bounds are half-open.
/// Zero-sized rectangles draw nothing.
pub fn fill_rect(target: &mut Framebuffer, rect: Rect, color: Color) {
    let left = u64::from(rect.x).min(u64::from(target.width()));
    let top = u64::from(rect.y).min(u64::from(target.height()));
    let right = (u64::from(rect.x) + u64::from(rect.width)).min(u64::from(target.width()));
    let bottom = (u64::from(rect.y) + u64::from(rect.height)).min(u64::from(target.height()));
    if left >= right || top >= bottom {
        return;
    }
    for y in top..bottom {
        if let Some(row) = target.row_mut(y as u32) {
            row[left as usize..right as usize].fill(color);
        }
    }
}

/// Alpha-composites a clipped rectangle over the framebuffer.
///
/// Fully opaque colors use the fast row-fill path; transparent colors are a no-op.
pub fn blend_rect(target: &mut Framebuffer, rect: Rect, color: Color) {
    if color.a == 0 {
        return;
    }
    if color.a == 255 {
        fill_rect(target, rect, color);
        return;
    }
    let left = u64::from(rect.x).min(u64::from(target.width()));
    let top = u64::from(rect.y).min(u64::from(target.height()));
    let right = (u64::from(rect.x) + u64::from(rect.width)).min(u64::from(target.width()));
    let bottom = (u64::from(rect.y) + u64::from(rect.height)).min(u64::from(target.height()));
    if left >= right || top >= bottom {
        return;
    }
    // Source-dependent terms are constant for the entire rectangle. Compute
    // them once instead of repeating the same conversions and multiplications
    // for every destination pixel.
    let source_alpha = u32::from(color.a);
    let inverse_source_alpha = 255 - source_alpha;
    let source_r = u32::from(color.r) * source_alpha * 255;
    let source_g = u32::from(color.g) * source_alpha * 255;
    let source_b = u32::from(color.b) * source_alpha * 255;

    for y in top..bottom {
        if let Some(row) = target.row_mut(y as u32) {
            for pixel in &mut row[left as usize..right as usize] {
                // Transparent destinations need no channel arithmetic: the
                // straight-alpha source color is preserved exactly.
                if pixel.a == 0 {
                    *pixel = color;
                    continue;
                }

                // Opaque destinations stay opaque, so source-over reduces to
                // three rounded 8-bit channel interpolations.
                if pixel.a == 255 {
                    let blend_channel = |source: u8, destination: u8| -> u8 {
                        ((u32::from(source) * source_alpha
                            + u32::from(destination) * inverse_source_alpha
                            + 127)
                            / 255) as u8
                    };
                    *pixel = Color::rgb(
                        blend_channel(color.r, pixel.r),
                        blend_channel(color.g, pixel.g),
                        blend_channel(color.b, pixel.b),
                    );
                    continue;
                }

                let destination_alpha = u32::from(pixel.a);
                let destination_weight = destination_alpha * inverse_source_alpha;
                let alpha_numerator = source_alpha * 255 + destination_weight;
                let blend_channel = |source: u32, destination: u8| -> u8 {
                    let numerator = source + u32::from(destination) * destination_weight;
                    ((numerator + alpha_numerator / 2) / alpha_numerator) as u8
                };

                *pixel = Color::rgba(
                    blend_channel(source_r, pixel.r),
                    blend_channel(source_g, pixel.g),
                    blend_channel(source_b, pixel.b),
                    ((alpha_numerator + 127) / 255) as u8,
                );
            }
        }
    }
}

/// A color stop in a linear or radial gradient. Positions are normalized to 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    pub position: f32,
    pub color: Color,
}

/// A validated gradient with stops sorted by position.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    stops: Vec<GradientStop>,
}

/// Errors produced while constructing a gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientError {
    TooFewStops,
    InvalidPosition,
}

impl Gradient {
    /// Creates a gradient from at least two finite stops in the inclusive 0..=1 range.
    /// Stops are sorted automatically; duplicate positions are allowed and form a hard edge.
    pub fn new(mut stops: Vec<GradientStop>) -> Result<Self, GradientError> {
        if stops.len() < 2 {
            return Err(GradientError::TooFewStops);
        }
        if stops
            .iter()
            .any(|stop| !stop.position.is_finite() || !(0.0..=1.0).contains(&stop.position))
        {
            return Err(GradientError::InvalidPosition);
        }
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        Ok(Self { stops })
    }

    /// Samples the gradient, clamping positions outside its stop range.
    #[must_use]
    pub fn sample(&self, position: f32) -> Color {
        let position = if position.is_finite() {
            position.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if position <= self.stops[0].position {
            return self.stops[0].color;
        }
        let last = self.stops.len() - 1;
        if position >= self.stops[last].position {
            return self.stops[last].color;
        }
        // Gradients with the common two/three-stop shape are sampled for
        // every covered pixel. Avoid a general binary search for these small,
        // strictly increasing stop arrays while preserving duplicate-stop
        // hard-edge semantics through the general path below.
        let (left, right) =
            if self.stops.len() == 2 && self.stops[0].position < self.stops[1].position {
                (self.stops[0], self.stops[1])
            } else if self.stops.len() == 3
                && self.stops[0].position < self.stops[1].position
                && self.stops[1].position < self.stops[2].position
            {
                if position < self.stops[1].position {
                    (self.stops[0], self.stops[1])
                } else {
                    (self.stops[1], self.stops[2])
                }
            } else {
                let upper = self.stops.partition_point(|stop| stop.position <= position);
                (self.stops[upper - 1], self.stops[upper])
            };
        let span = right.position - left.position;
        if span <= f32::EPSILON {
            return right.color;
        }
        interpolate_color(left.color, right.color, (position - left.position) / span)
    }
}

fn interpolate_color(left: Color, right: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    let channel =
        |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount).round() as u8;
    Color::rgba(
        channel(left.r, right.r),
        channel(left.g, right.g),
        channel(left.b, right.b),
        channel(left.a, right.a),
    )
}

/// Fills a clipped rectangle with a linear gradient between two floating-point points.
/// Sampling occurs at pixel centers. A zero-length axis samples the first color throughout.
pub fn fill_rect_linear_gradient(
    target: &mut Framebuffer,
    rect: Rect,
    start: crate::Vector,
    end: crate::Vector,
    gradient: &Gradient,
) {
    if !start.x.is_finite() || !start.y.is_finite() || !end.x.is_finite() || !end.y.is_finite() {
        return;
    }
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    let left = rect.x.min(target.width());
    let top = rect.y.min(target.height());
    let right = (u64::from(rect.x) + u64::from(rect.width)).min(u64::from(target.width())) as u32;
    let bottom =
        (u64::from(rect.y) + u64::from(rect.height)).min(u64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return;
    }
    let inverse_length_squared = if length_squared <= f32::EPSILON {
        0.0
    } else {
        length_squared.recip()
    };
    let horizontal_step = dx * inverse_length_squared;
    for y in top..bottom {
        let row_position = if inverse_length_squared == 0.0 {
            0.0
        } else {
            ((left as f32 + 0.5 - start.x) * dx + (y as f32 + 0.5 - start.y) * dy)
                * inverse_length_squared
        };
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        let mut position = row_position;
        for pixel in &mut row[left as usize..right as usize] {
            let color = gradient.sample(position.clamp(0.0, 1.0));
            *pixel = if color.a == u8::MAX {
                color
            } else if color.a == 0 {
                *pixel
            } else {
                color.blend_over(*pixel)
            };
            position += horizontal_step;
        }
    }
}

/// Fills a clipped rectangle with a radial gradient sampled by distance from its center.
/// Radius must be finite and positive; samples beyond it use the final stop.
pub fn fill_rect_radial_gradient(
    target: &mut Framebuffer,
    rect: Rect,
    center: crate::Vector,
    radius: f32,
    gradient: &Gradient,
) {
    if !center.x.is_finite() || !center.y.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return;
    }
    let left = rect.x.min(target.width());
    let top = rect.y.min(target.height());
    let right = (u64::from(rect.x) + u64::from(rect.width)).min(u64::from(target.width())) as u32;
    let bottom =
        (u64::from(rect.y) + u64::from(rect.height)).min(u64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return;
    }
    let inverse_radius = radius.recip();
    for y in top..bottom {
        let dy = y as f32 + 0.5 - center.y;
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for (offset, pixel) in row[left as usize..right as usize].iter_mut().enumerate() {
            let dx = left as f32 + offset as f32 + 0.5 - center.x;
            let position = dx.hypot(dy) * inverse_radius;
            let color = gradient.sample(position.clamp(0.0, 1.0));
            *pixel = if color.a == u8::MAX {
                color
            } else if color.a == 0 {
                *pixel
            } else {
                color.blend_over(*pixel)
            };
        }
    }
}

/// Fills an affine-transformed rectangle with a linear gradient.
/// Gradient coordinates are expressed in the rectangle's local coordinate space.
/// Pixel centers are mapped through the inverse transform for stable sampling.
pub fn fill_transformed_rect_linear_gradient_clipped(
    target: &mut Framebuffer,
    rect: Rect,
    transform: crate::Transform2D,
    start: crate::Vector,
    end: crate::Vector,
    gradient: &Gradient,
    clip: Rect,
) -> bool {
    if !transform.is_finite()
        || !start.x.is_finite()
        || !start.y.is_finite()
        || !end.x.is_finite()
        || !end.y.is_finite()
        || rect.width == 0
        || rect.height == 0
    {
        return false;
    }
    let Some(inverse) = transform.inverse() else {
        return false;
    };
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return false;
    };
    let corners = transformed_rect_corners(rect, transform);
    if corners
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return false;
    }
    let Some((left, top, right, bottom)) = transformed_bounds(&corners, clip) else {
        return false;
    };
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    let mut visible = false;
    for y in top..bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            let local = inverse.transform_point(crate::Vector::new(x as f32 + 0.5, y as f32 + 0.5));
            if !rect_contains_point(rect, local) {
                continue;
            }
            visible = true;
            let position = if length_squared <= f32::EPSILON {
                0.0
            } else {
                (((local.x - start.x) * dx + (local.y - start.y) * dy) / length_squared)
                    .clamp(0.0, 1.0)
            };
            let color = gradient.sample(position);
            let pixel = &mut row[x as usize];
            *pixel = if color.a == u8::MAX {
                color
            } else if color.a == 0 {
                *pixel
            } else {
                color.blend_over(*pixel)
            };
        }
    }
    visible
}

/// Fills an affine-transformed rectangle with a radial gradient.
/// The center and radius are expressed in local coordinate space.
pub fn fill_transformed_rect_radial_gradient_clipped(
    target: &mut Framebuffer,
    rect: Rect,
    transform: crate::Transform2D,
    center: crate::Vector,
    radius: f32,
    gradient: &Gradient,
    clip: Rect,
) -> bool {
    if !transform.is_finite()
        || !center.x.is_finite()
        || !center.y.is_finite()
        || !radius.is_finite()
        || radius <= 0.0
        || rect.width == 0
        || rect.height == 0
    {
        return false;
    }
    let Some(inverse) = transform.inverse() else {
        return false;
    };
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return false;
    };
    let corners = transformed_rect_corners(rect, transform);
    if corners
        .iter()
        .any(|point| !point.x.is_finite() || !point.y.is_finite())
    {
        return false;
    }
    let Some((left, top, right, bottom)) = transformed_bounds(&corners, clip) else {
        return false;
    };
    let mut visible = false;
    let inverse_radius = radius.recip();
    for y in top..bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            let local = inverse.transform_point(crate::Vector::new(x as f32 + 0.5, y as f32 + 0.5));
            if !rect_contains_point(rect, local) {
                continue;
            }
            visible = true;
            let position = (local.x - center.x).hypot(local.y - center.y) * inverse_radius;
            let color = gradient.sample(position.clamp(0.0, 1.0));
            let pixel = &mut row[x as usize];
            *pixel = if color.a == u8::MAX {
                color
            } else if color.a == 0 {
                *pixel
            } else {
                color.blend_over(*pixel)
            };
        }
    }
    visible
}

fn transformed_rect_corners(rect: Rect, transform: crate::Transform2D) -> [crate::Vector; 4] {
    let left = rect.x as f32;
    let top = rect.y as f32;
    let right = rect.x.saturating_add(rect.width) as f32;
    let bottom = rect.y.saturating_add(rect.height) as f32;
    [
        transform.transform_point(crate::Vector::new(left, top)),
        transform.transform_point(crate::Vector::new(right, top)),
        transform.transform_point(crate::Vector::new(right, bottom)),
        transform.transform_point(crate::Vector::new(left, bottom)),
    ]
}

fn transformed_bounds(corners: &[crate::Vector; 4], clip: Rect) -> Option<(u32, u32, u32, u32)> {
    let min_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::INFINITY, f32::min);
    let min_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::INFINITY, f32::min);
    let max_x = corners
        .iter()
        .map(|point| point.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let max_y = corners
        .iter()
        .map(|point| point.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let clip_right = clip.x.saturating_add(clip.width);
    let clip_bottom = clip.y.saturating_add(clip.height);
    let left = min_x.floor().max(clip.x as f32).min(clip_right as f32) as u32;
    let top = min_y.floor().max(clip.y as f32).min(clip_bottom as f32) as u32;
    let right = max_x.ceil().max(clip.x as f32).min(clip_right as f32) as u32;
    let bottom = max_y.ceil().max(clip.y as f32).min(clip_bottom as f32) as u32;
    (left < right && top < bottom).then_some((left, top, right, bottom))
}

fn rect_contains_point(rect: Rect, point: crate::Vector) -> bool {
    point.x >= rect.x as f32
        && point.y >= rect.y as f32
        && point.x < rect.x.saturating_add(rect.width) as f32
        && point.y < rect.y.saturating_add(rect.height) as f32
}

/// Draws a one-pixel rectangle outline. The rectangle uses half-open bounds.
pub fn draw_rect(target: &mut Framebuffer, rect: Rect, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let left = i64::from(rect.x);
    let top = i64::from(rect.y);
    let right = left + i64::from(rect.width) - 1;
    let bottom = top + i64::from(rect.height) - 1;
    let point = |x: i64, y: i64| {
        Point::new(
            i32::try_from(x).unwrap_or(if x < 0 { i32::MIN } else { i32::MAX }),
            i32::try_from(y).unwrap_or(if y < 0 { i32::MIN } else { i32::MAX }),
        )
    };
    draw_line(target, point(left, top), point(right, top), color);
    if rect.height > 1 {
        draw_line(target, point(left, bottom), point(right, bottom), color);
    }
    if rect.width > 1 {
        draw_line(target, point(left, top), point(left, bottom), color);
        draw_line(target, point(right, top), poi…17633 tokens truncated…   );
        fill_triangle(
            &mut image,
            Point::new(2, 0),
            Point::new(0, 2),
            Point::new(2, 2),
            Color::GREEN,
        );
        assert!(image.pixels().iter().all(|pixel| *pixel != Color::BLACK));
    }

    #[test]
    fn degenerate_and_offscreen_triangles_draw_nothing() {
        let mut image = target(3, 3);
        fill_triangle(
            &mut image,
            Point::new(0, 0),
            Point::new(1, 1),
            Point::new(2, 2),
            Color::RED,
        );
        fill_triangle(
            &mut image,
            Point::new(-8, -8),
            Point::new(-4, -8),
            Point::new(-8, -4),
            Color::RED,
        );
        assert!(image.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }
}

/// Alpha-composites only the intersection of a rectangle, clip region, and framebuffer.
pub fn blend_rect_clipped(target: &mut Framebuffer, rect: Rect, clip: Rect, color: Color) {
    let Some(visible) = rect.intersection(clip).and_then(|intersection| {
        intersection.intersection(Rect::new(0, 0, target.width(), target.height()))
    }) else {
        return;
    };
    blend_rect(target, visible, color);
}

/// Fills only the intersection of a rectangle, a clip region, and the framebuffer.
/// The clip is expressed in framebuffer coordinates and uses half-open bounds.
pub fn fill_rect_clipped(target: &mut Framebuffer, rect: Rect, clip: Rect, color: Color) {
    let Some(visible) = rect.intersection(clip).and_then(|intersection| {
        intersection.intersection(Rect::new(0, 0, target.width(), target.height()))
    }) else {
        return;
    };
    fill_rect(target, visible, color);
}

#[cfg(test)]
mod clipping_tests {
    use super::fill_rect_clipped;
    use crate::{Color, Framebuffer, Rect};

    #[test]
    fn opaque_axis_aligned_aa_lines_match_integer_line_coverage() {
        let mut expected = Framebuffer::new(8, 6, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        super::draw_line(
            &mut expected,
            crate::Point::new(1, 2),
            crate::Point::new(6, 2),
            Color::WHITE,
        );
        super::draw_line_aa(
            &mut actual,
            crate::Point::new(1, 2),
            crate::Point::new(6, 2),
            Color::WHITE,
        );
        assert_eq!(actual, expected);

        super::draw_line(
            &mut expected,
            crate::Point::new(4, 1),
            crate::Point::new(4, 5),
            Color::RED,
        );
        super::draw_line_aa(
            &mut actual,
            crate::Point::new(4, 1),
            crate::Point::new(4, 5),
            Color::RED,
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn axis_aligned_lines_use_exact_inclusive_endpoints_in_both_directions() {
        let mut horizontal = Framebuffer::new(6, 5, Color::BLACK).unwrap();
        super::draw_line(
            &mut horizontal,
            crate::Point::new(4, 2),
            crate::Point::new(1, 2),
            Color::RED,
        );

        for x in 1..=4 {
            assert_eq!(horizontal.get_pixel(x, 2), Some(Color::RED));
        }
        assert_eq!(horizontal.get_pixel(0, 2), Some(Color::BLACK));
        assert_eq!(horizontal.get_pixel(5, 2), Some(Color::BLACK));

        let mut vertical = Framebuffer::new(6, 5, Color::BLACK).unwrap();
        super::draw_line(
            &mut vertical,
            crate::Point::new(3, 4),
            crate::Point::new(3, 1),
            Color::BLUE,
        );

        for y in 1..=4 {
            assert_eq!(vertical.get_pixel(3, y), Some(Color::BLUE));
        }
        assert_eq!(vertical.get_pixel(3, 0), Some(Color::BLACK));
    }

    #[test]
    fn alpha_blended_rectangle_composites_every_visible_pixel() {
        let mut target = Framebuffer::new(4, 3, Color::BLUE).unwrap();
        super::blend_rect(
            &mut target,
            Rect::new(1, 1, 2, 2),
            Color::rgba(255, 0, 0, 128),
        );
        assert_eq!(target.get_pixel(0, 1), Some(Color::BLUE));
        assert_eq!(target.get_pixel(1, 1), Some(Color::rgb(128, 0, 127)));
        assert_eq!(target.get_pixel(2, 2), Some(Color::rgb(128, 0, 127)));
        assert_eq!(target.get_pixel(3, 2), Some(Color::BLUE));
    }

    #[test]
    fn alpha_blended_rectangle_respects_clip_and_transparent_noop() {
        let mut target = Framebuffer::new(4, 3, Color::BLACK).unwrap();
        super::blend_rect_clipped(
            &mut target,
            Rect::new(0, 0, 4, 3),
            Rect::new(1, 1, 2, 1),
            Color::rgba(0, 255, 0, 128),
        );
        super::blend_rect(&mut target, Rect::new(0, 0, 4, 3), Color::TRANSPARENT);
        assert_eq!(target.get_pixel(0, 1), Some(Color::BLACK));
        assert_eq!(target.get_pixel(1, 1), Some(Color::rgba(0, 128, 0, 255)));
        assert_eq!(target.get_pixel(2, 1), Some(Color::rgba(0, 128, 0, 255)));
        assert_eq!(target.get_pixel(3, 1), Some(Color::BLACK));
    }

    #[test]
    fn opaque_blended_rectangle_matches_fast_fill() {
        let mut blended = Framebuffer::new(3, 2, Color::BLUE).unwrap();
        let mut filled = blended.clone();
        super::blend_rect(&mut blended, Rect::new(1, 0, 2, 2), Color::RED);
        super::fill_rect(&mut filled, Rect::new(1, 0, 2, 2), Color::RED);
        assert_eq!(blended, filled);
    }

    #[test]
    fn optimized_rectangle_blending_matches_per_pixel_source_over() {
        let backgrounds = [
            Color::TRANSPARENT,
            Color::rgba(20, 40, 60, 1),
            Color::rgba(90, 130, 170, 127),
            Color::rgba(200, 100, 50, 254),
            Color::WHITE,
        ];
        for alpha in [1, 17, 64, 128, 200, 254] {
            let source = Color::rgba(231, 73, 149, alpha);
            let mut actual = Framebuffer::new(5, backgrounds.len() as u32, Color::BLACK).unwrap();
            let mut expected = actual.clone();
            for (y, background) in backgrounds.iter().copied().enumerate() {
                for x in 0..5 {
                    actual.set_pixel(x, y as u32, background);
                    expected.set_pixel(x, y as u32, source.blend_over(background));
                }
            }

            super::blend_rect(
                &mut actual,
                Rect::new(0, 0, 5, backgrounds.len() as u32),
                source,
            );
            assert_eq!(actual, expected, "source alpha {alpha}");
        }
    }

    #[test]
    fn clipped_fill_respects_both_shape_and_viewport() {
        let mut target = Framebuffer::new(12, 10, Color::BLACK).unwrap();
        fill_rect_clipped(
            &mut target,
            Rect::new(2, 1, 8, 7),
            Rect::new(5, 3, 4, 4),
            Color::RED,
        );
        assert_eq!(target.get_pixel(5, 3), Some(Color::RED));
        assert_eq!(target.get_pixel(8, 3), Some(Color::RED));
        assert_eq!(target.get_pixel(9, 3), Some(Color::BLACK));
        assert_eq!(target.get_pixel(5, 7), Some(Color::BLACK));
        assert_eq!(target.get_pixel(4, 4), Some(Color::BLACK));
    }

    #[test]
    fn clipped_fill_handles_disjoint_and_offscreen_rectangles() {
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        fill_rect_clipped(
            &mut target,
            Rect::new(8, 8, 2, 2),
            Rect::new(0, 0, 4, 4),
            Color::RED,
        );
        fill_rect_clipped(
            &mut target,
            Rect::new(0, 0, 4, 4),
            Rect::new(20, 20, 3, 3),
            Color::RED,
        );
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn clipped_fill_supports_clip_larger_than_framebuffer() {
        let mut target = Framebuffer::new(3, 2, Color::BLACK).unwrap();
        fill_rect_clipped(
            &mut target,
            Rect::new(0, 0, 8, 8),
            Rect::new(0, 0, 20, 20),
            Color::WHITE,
        );
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::WHITE));
    }
}

#[cfg(test)]
mod rounded_fill_tests {
    use super::{fill_rect, fill_rounded_rect};
    use crate::{Color, Framebuffer, Rect};

    #[test]
    fn rounded_rectangle_fill_covers_interior_and_leaves_corners_clear() {
        let mut target = Framebuffer::new(10, 8, Color::BLACK).unwrap();
        fill_rounded_rect(&mut target, Rect::new(1, 1, 8, 6), 2, Color::WHITE);

        assert_eq!(target.get_pixel(3, 1), Some(Color::WHITE));
        assert_eq!(target.get_pixel(4, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(1, 1), Some(Color::BLACK));
        assert_eq!(target.get_pixel(8, 1), Some(Color::BLACK));
        assert_eq!(target.get_pixel(0, 3), Some(Color::BLACK));
    }

    #[test]
    fn zero_radius_rounded_fill_matches_rectangle_fill() {
        let mut expected = Framebuffer::new(9, 7, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        let rect = Rect::new(1, 1, 7, 5);
        fill_rect(&mut expected, rect, Color::RED);
        fill_rounded_rect(&mut actual, rect, 0, Color::RED);
        assert_eq!(actual, expected);
    }

    #[test]
    fn rounded_rectangle_fill_clips_to_framebuffer() {
        let mut target = Framebuffer::new(5, 4, Color::BLACK).unwrap();
        fill_rounded_rect(&mut target, Rect::new(3, 1, 8, 5), 2, Color::BLUE);
        assert_eq!(target.get_pixel(4, 2), Some(Color::BLUE));
        assert_eq!(target.get_pixel(4, 3), Some(Color::BLUE));
        assert_eq!(target.get_pixel(3, 1), Some(Color::BLACK));
    }

    #[test]
    fn tiny_rounded_rectangle_keeps_its_pixel_coverage() {
        let mut target = Framebuffer::new(2, 2, Color::BLACK).unwrap();
        fill_rounded_rect(&mut target, Rect::new(0, 0, 2, 2), 1, Color::GREEN);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::GREEN));
    }

    #[test]
    fn rounded_rectangle_fill_ignores_empty_rectangles() {
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        fill_rounded_rect(&mut target, Rect::new(1, 1, 0, 3), 2, Color::GREEN);
        fill_rounded_rect(&mut target, Rect::new(1, 1, 3, 0), 2, Color::GREEN);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }
}

#[cfg(test)]
mod rounded_outline_tests {
    use super::{draw_ellipse, draw_rect, draw_rounded_rect, fill_ellipse};
    use crate::{Color, Framebuffer, Point, Rect};

    fn target(width: u32, height: u32) -> Framebuffer {
        Framebuffer::new(width, height, Color::BLACK).unwrap()
    }

    #[test]
    fn ellipse_scan_conversion_handles_maximum_radii_without_overflow() {
        let mut filled = target(3, 3);
        fill_ellipse(
            &mut filled,
            Point::new(1, 1),
            u32::MAX,
            u32::MAX,
            Color::GREEN,
        );
        assert!(filled.pixels().iter().all(|pixel| *pixel == Color::GREEN));

        let mut outlined = target(3, 3);
        draw_ellipse(
            &mut outlined,
            Point::new(1, 1),
            u32::MAX,
            u32::MAX,
            Color::WHITE,
        );
        for y in 0..3 {
            assert_eq!(outlined.get_pixel(0, y), Some(Color::WHITE));
            assert_eq!(outlined.get_pixel(2, y), Some(Color::WHITE));
            assert_eq!(outlined.get_pixel(1, y), Some(Color::BLACK));
        }
    }

    #[test]
    fn rounded_rectangle_outline_draws_only_the_perimeter() {
        let mut target = Framebuffer::new(10, 8, Color::BLACK).unwrap();
        draw_rounded_rect(&mut target, Rect::new(1, 1, 8, 6), 2, Color::WHITE);

        assert_eq!(target.get_pixel(3, 1), Some(Color::WHITE));
        assert_eq!(target.get_pixel(6, 1), Some(Color::WHITE));
        assert_eq!(target.get_pixel(1, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(8, 3), Some(Color::WHITE));
        assert_eq!(target.get_pixel(3, 6), Some(Color::WHITE));
        assert_eq!(target.get_pixel(6, 6), Some(Color::WHITE));
        assert_eq!(target.get_pixel(1, 1), Some(Color::BLACK));
        assert_eq!(target.get_pixel(4, 3), Some(Color::BLACK));
        assert_eq!(target.get_pixel(0, 3), Some(Color::BLACK));
    }

    #[test]
    fn zero_radius_rounded_outline_matches_rectangle_outline() {
        let mut expected = Framebuffer::new(9, 7, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        let rect = Rect::new(1, 1, 7, 5);
        draw_rect(&mut expected, rect, Color::RED);
        draw_rounded_rect(&mut actual, rect, 0, Color::RED);
        assert_eq!(actual, expected);
    }

    #[test]
    fn tiny_rounded_rectangle_outline_keeps_its_perimeter() {
        let mut target = Framebuffer::new(2, 2, Color::BLACK).unwrap();
        draw_rounded_rect(&mut target, Rect::new(0, 0, 2, 2), 1, Color::BLUE);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLUE));
    }

    #[test]
    fn rounded_rectangle_outline_clips_to_framebuffer() {
        let mut target = Framebuffer::new(5, 4, Color::BLACK).unwrap();
        draw_rounded_rect(&mut target, Rect::new(3, 1, 8, 5), 2, Color::BLUE);
        assert_eq!(target.get_pixel(3, 2), Some(Color::BLUE));
        assert_eq!(target.get_pixel(4, 2), Some(Color::BLUE));
        assert_eq!(target.get_pixel(4, 3), Some(Color::BLUE));
    }
}

#[cfg(test)]
mod triangle_aa_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn antialiased_triangle_helpers_match_polygon_rasterizers() {
        let vertices = [Point::new(1, 1), Point::new(7, 2), Point::new(3, 7)];
        let mut expected_fill = Framebuffer::new(9, 9, Color::BLACK).unwrap();
        let mut actual_fill = expected_fill.clone();
        super::fill_polygon_aa(
            &mut expected_fill,
            &vertices,
            Color::rgba(40, 180, 240, 190),
        );
        super::fill_triangle_aa(
            &mut actual_fill,
            vertices[0],
            vertices[1],
            vertices[2],
            Color::rgba(40, 180, 240, 190),
        );
        assert_eq!(actual_fill, expected_fill);

        let mut expected_outline = Framebuffer::new(9, 9, Color::BLACK).unwrap();
        let mut actual_outline = expected_outline.clone();
        super::draw_polygon_aa(&mut expected_outline, &vertices, Color::WHITE);
        super::draw_triangle_aa(
            &mut actual_outline,
            vertices[0],
            vertices[1],
            vertices[2],
            Color::WHITE,
        );
        assert_eq!(actual_outline, expected_outline);
    }

    #[test]
    fn antialiased_triangle_helpers_handle_degenerate_and_clipped_geometry() {
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        super::fill_triangle_aa(
            &mut target,
            Point::new(-8, -8),
            Point::new(8, -8),
            Point::new(0, 8),
            Color::GREEN,
        );
        assert!(target.pixels().iter().any(|pixel| pixel.g > 0));

        let before = target.clone();
        super::fill_triangle_aa(
            &mut target,
            Point::new(1, 1),
            Point::new(2, 2),
            Point::new(3, 3),
            Color::RED,
        );
        super::draw_triangle_aa(
            &mut target,
            Point::new(2, 2),
            Point::new(2, 2),
            Point::new(2, 2),
            Color::RED,
        );
        assert_eq!(target, before);
    }
}

#[cfg(test)]
mod circle_aa_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn antialiased_circle_outline_matches_equal_radius_ellipse() {
        let mut expected = Framebuffer::new(17, 15, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        super::draw_ellipse_aa(
            &mut expected,
            Point::new(8, 7),
            5,
            5,
            Color::rgba(220, 80, 40, 190),
        );
        super::draw_circle_aa(
            &mut actual,
            Point::new(8, 7),
            5,
            Color::rgba(220, 80, 40, 190),
        );
        assert_eq!(actual, expected);
        assert_eq!(actual.get_pixel(8, 7), Some(Color::BLACK));
        assert!(actual.pixels().iter().any(|pixel| pixel.r > 0));
    }

    #[test]
    fn antialiased_circle_outline_handles_zero_radius_and_clipping() {
        let mut expected = Framebuffer::new(5, 5, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        super::draw_line_aa(
            &mut expected,
            Point::new(2, 2),
            Point::new(2, 2),
            Color::WHITE,
        );
        super::draw_circle_aa(&mut actual, Point::new(2, 2), 0, Color::WHITE);
        assert_eq!(actual, expected);

        super::draw_circle_aa(&mut actual, Point::new(-2, 2), 4, Color::GREEN);
        assert!(actual.pixels().iter().any(|pixel| pixel.g > 0));
    }
}

#[cfg(test)]
mod rounded_rectangle_aa_fill_tests {
    use crate::{Color, Framebuffer, Rect};

    #[test]
    fn antialiased_rounded_rectangle_fill_covers_center_and_leaves_corner_clear() {
        let mut target = Framebuffer::new(12, 10, Color::BLACK).unwrap();
        super::fill_rounded_rect_aa(&mut target, Rect::new(2, 2, 8, 6), 3, Color::WHITE);
        assert_eq!(target.get_pixel(5, 4), Some(Color::WHITE));
        assert_eq!(target.get_pixel(2, 2), Some(Color::BLACK));
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
    }

    #[test]
    fn antialiased_rounded_rectangle_fill_clips_and_blends_alpha() {
        let mut target = Framebuffer::new(5, 5, Color::BLACK).unwrap();
        super::fill_rounded_rect_aa(
            &mut target,
            Rect::new(2, 1, 6, 5),
            2,
            Color::rgba(20, 180, 90, 128),
        );
        assert!(target.pixels().iter().any(|pixel| pixel.g > 0));
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| pixel.g > 0 && pixel.g < 180));
        let before = target.clone();
        super::fill_rounded_rect_aa(&mut target, Rect::new(0, 0, 5, 5), 2, Color::TRANSPARENT);
        assert_eq!(target, before);
    }

    #[test]
    fn antialiased_rounded_rectangle_fill_ignores_empty_or_offscreen_shapes() {
        let mut target = Framebuffer::new(4, 4, Color::BLACK).unwrap();
        super::fill_rounded_rect_aa(&mut target, Rect::new(1, 1, 0, 3), 2, Color::WHITE);
        super::fill_rounded_rect_aa(&mut target, Rect::new(10, 10, 3, 3), 1, Color::WHITE);
        assert!(target.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }
}
#[cfg(test)]
mod arc_aa_fill_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn antialiased_arc_fill_covers_sector_without_filling_outside() {
        let mut target = Framebuffer::new(17, 17, Color::BLACK).unwrap();
        super::fill_arc_aa(
            &mut target,
            Point::new(8, 8),
            5,
            0.0,
            core::f32::consts::FRAC_PI_2,
            Color::WHITE,
        );

        assert!(target.get_pixel(8, 8).unwrap().r > 0);
        assert!(target.get_pixel(12, 8).unwrap().r > 0);
        assert!(target.get_pixel(8, 12).unwrap().r > 0);
        assert_eq!(target.get_pixel(4, 8), Some(Color::BLACK));
        assert_eq!(target.get_pixel(8, 4), Some(Color::BLACK));
        assert_eq!(target.get_pixel(14, 8), Some(Color::BLACK));
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
    }

    #[test]
    fn antialiased_arc_fill_wraps_angles_and_blends_alpha() {
        let mut target = Framebuffer::new(17, 17, Color::BLACK).unwrap();
        super::fill_arc_aa(
            &mut target,
            Point::new(8, 8),
            5,
            3.0 * core::f32::consts::FRAC_PI_2,
            core::f32::consts::FRAC_PI_2,
            Color::rgba(30, 180, 90, 128),
        );

        assert!(target.get_pixel(8, 8).unwrap().g > 0);
        assert!(target.get_pixel(8, 3).unwrap().g > 0);
        assert!(target.get_pixel(13, 8).unwrap().g > 0);
        assert!(target.get_pixel(8, 13).unwrap().g > 0);
        assert_eq!(target.get_pixel(3, 8), Some(Color::BLACK));
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| pixel.g > 0 && pixel.g < 180));
    }

    #[test]
    fn antialiased_arc_fill_handles_full_turn_clipping_and_invalid_angles() {
        let mut target = Framebuffer::new(9, 9, Color::BLACK).unwrap();
        super::fill_arc_aa(
            &mut target,
            Point::new(4, 4),
            3,
            0.0,
            core::f32::consts::TAU,
            Color::WHITE,
        );
        assert!(target.get_pixel(4, 4).unwrap().r > 0);
        assert_eq!(target.get_pixel(0, 0), Some(Color::BLACK));

        super::fill_arc_aa(
            &mut target,
            Point::new(-2, 4),
            5,
            0.0,
            core::f32::consts::PI,
            Color::GREEN,
        );
        assert!(target.pixels().iter().any(|pixel| pixel.g > 0));

        let before = target.clone();
        super::fill_arc_aa(&mut target, Point::new(4, 4), 3, 1.0, 1.0, Color::RED);
        super::fill_arc_aa(&mut target, Point::new(4, 4), 3, f32::NAN, 2.0, Color::RED);
        super::fill_arc_aa(
            &mut target,
            Point::new(4, 4),
            3,
            0.0,
            core::f32::consts::TAU,
            Color::TRANSPARENT,
        );
        assert_eq!(target, before);
    }
}

#[cfg(test)]
mod arc_aa_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn antialiased_arc_draws_only_the_requested_quadrant() {
        let mut target = Framebuffer::new(17, 17, Color::BLACK).unwrap();
        super::draw_arc_aa(
            &mut target,
            Point::new(8, 8),
            5,
            0.0,
            core::f32::consts::FRAC_PI_2,
            Color::WHITE,
        );

        assert!(target.get_pixel(13, 8).unwrap().r > 0);
        assert!(target.get_pixel(8, 13).unwrap().r > 0);
        assert_eq!(target.get_pixel(3, 8), Some(Color::BLACK));
        assert_eq!(target.get_pixel(8, 3), Some(Color::BLACK));
        assert_eq!(target.get_pixel(8, 8), Some(Color::BLACK));
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
    }

    #[test]
    fn antialiased_arc_wraps_angles_and_blends_alpha() {
        let mut target = Framebuffer::new(17, 17, Color::BLACK).unwrap();
        super::draw_arc_aa(
            &mut target,
            Point::new(8, 8),
            5,
            3.0 * core::f32::consts::FRAC_PI_2,
            core::f32::consts::FRAC_PI_2,
            Color::rgba(30, 180, 90, 128),
        );

        assert!(target.get_pixel(8, 3).unwrap().g > 0);
        assert!(target.get_pixel(13, 8).unwrap().g > 0);
        assert!(target.get_pixel(8, 13).unwrap().g > 0);
        assert_eq!(target.get_pixel(3, 8), Some(Color::BLACK));
        assert!(target
            .pixels()
            .iter()
            .any(|pixel| pixel.g > 0 && pixel.g < 180));
    }

    #[test]
    fn antialiased_arc_handles_full_turn_degenerate_and_clipped_inputs() {
        let mut full = Framebuffer::new(9, 9, Color::BLACK).unwrap();
        super::draw_arc_aa(
            &mut full,
            Point::new(4, 4),
            3,
            0.0,
            core::f32::consts::TAU,
            Color::WHITE,
        );
        assert!(full.pixels().iter().any(|pixel| pixel.r > 0));
        assert_eq!(full.get_pixel(4, 4), Some(Color::BLACK));

        let before = full.clone();
        super::draw_arc_aa(&mut full, Point::new(4, 4), 3, 1.0, 1.0, Color::RED);
        super::draw_arc_aa(&mut full, Point::new(4, 4), 3, f32::NAN, 2.0, Color::RED);
        assert_eq!(full, before);

        super::draw_arc_aa(
            &mut full,
            Point::new(-2, 4),
            5,
            0.0,
            core::f32::consts::PI,
            Color::GREEN,
        );
        assert!(full.pixels().iter().any(|pixel| pixel.g > 0));
    }
}
