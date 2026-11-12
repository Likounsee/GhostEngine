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
    // A high-resolution lookup table accelerates per-pixel raster sampling.
    // It is only used when stops are strictly increasing, so hard edges remain exact.
    lookup: Option<Box<[Color]>>,
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
        let lookup = if stops
            .windows(2)
            .all(|pair| pair[0].position < pair[1].position)
        {
            const LOOKUP_SIZE: usize = 4096;
            Some(
                (0..LOOKUP_SIZE)
                    .map(|index| {
                        let position = index as f32 / (LOOKUP_SIZE - 1) as f32;
                        sample_gradient_stops(&stops, position)
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            )
        } else {
            None
        };
        Ok(Self { stops, lookup })
    }

    /// Samples the gradient, clamping positions outside its stop range.
    #[must_use]
    pub fn sample(&self, position: f32) -> Color {
        sample_gradient_stops(&self.stops, position)
    }

    /// Fast per-pixel sampler. The 4096-entry table introduces at most a
    /// sub-channel interpolation error for smooth gradients while avoiding
    /// repeated stop lookup and floating-point channel interpolation.
    #[inline]
    fn sample_raster(&self, position: f32) -> Color {
        let position = if position.is_finite() {
            position.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if let Some(lookup) = &self.lookup {
            let index = (position * (lookup.len() - 1) as f32 + 0.5) as usize;
            lookup[index.min(lookup.len() - 1)]
        } else {
            sample_gradient_stops(&self.stops, position)
        }
    }
}

fn sample_gradient_stops(stops: &[GradientStop], position: f32) -> Color {
    let position = if position.is_finite() {
        position.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if position <= stops[0].position {
        return stops[0].color;
    }
    let last = stops.len() - 1;
    if position >= stops[last].position {
        return stops[last].color;
    }
    let (left, right) = if stops.len() == 2 && stops[0].position < stops[1].position {
        (stops[0], stops[1])
    } else if stops.len() == 3
        && stops[0].position < stops[1].position
        && stops[1].position < stops[2].position
    {
        if position < stops[1].position {
            (stops[0], stops[1])
        } else {
            (stops[1], stops[2])
        }
    } else {
        let upper = stops.partition_point(|stop| stop.position <= position);
        (stops[upper - 1], stops[upper])
    };
    let span = right.position - left.position;
    if span <= f32::EPSILON {
        return right.color;
    }
    interpolate_color(left.color, right.color, (position - left.position) / span)
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
            let color = gradient.sample_raster(position);
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
    let radius_squared = radius * radius;
    // Keep a small floating-point safety margin so pixels close to the radius
    // still use the original square-root calculation.
    let outside_radius_squared = radius_squared * (1.0 + 4.0 * f32::EPSILON);
    for y in top..bottom {
        let dy = y as f32 + 0.5 - center.y;
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for (offset, pixel) in row[left as usize..right as usize].iter_mut().enumerate() {
            let dx = left as f32 + offset as f32 + 0.5 - center.x;
            // Framebuffer coordinates are bounded by u32, so the squared
            // distance is finite for ordinary scene coordinates. Prefer the
            // cheaper square-root path and retain hypot's overflow-safe
            // behavior for unusually large finite gradient centers.
            let distance_squared = dx * dx + dy * dy;
            // Samples outside the radius always resolve to the final stop;
            // avoid a square root for those pixels.
            let position =
                if radius_squared.is_finite() && distance_squared >= outside_radius_squared {
                    1.0
                } else {
                    let distance = if distance_squared.is_finite() {
                        distance_squared.sqrt()
                    } else {
                        dx.hypot(dy)
                    };
                    distance * inverse_radius
                };
            let color = gradient.sample_raster(position);
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
            let color = gradient.sample_raster(position);
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
            let color = gradient.sample_raster(position);
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
        draw_line(target, point(right, top), point(right, bottom), color);
    }
}

/// Draws an anti-aliased rectangle outline with 4x4 supersampled coverage.
pub fn draw_rect_aa(target: &mut Framebuffer, rect: Rect, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let point = |x: i64, y: i64| {
        Point::new(
            i32::try_from(x).unwrap_or(if x < 0 { i32::MIN } else { i32::MAX }),
            i32::try_from(y).unwrap_or(if y < 0 { i32::MIN } else { i32::MAX }),
        )
    };
    let left = i64::from(rect.x);
    let top = i64::from(rect.y);
    let right = left + i64::from(rect.width) - 1;
    let bottom = top + i64::from(rect.height) - 1;
    draw_polygon_aa(
        target,
        &[
            point(left, top),
            point(right, top),
            point(right, bottom),
            point(left, bottom),
        ],
        color,
    );
}

/// Draws an anti-aliased rounded-rectangle outline using signed-distance coverage.
pub fn draw_rounded_rect_aa(target: &mut Framebuffer, rect: Rect, radius: u32, color: Color) {
    if rect.width == 0 || rect.height == 0 || color.a == 0 {
        return;
    }

    let left = i64::from(rect.x);
    let top = i64::from(rect.y);
    let right = left + i64::from(rect.width);
    let bottom = top + i64::from(rect.height);
    let clip_left = left.saturating_sub(1).max(0) as u32;
    let clip_top = top.saturating_sub(1).max(0) as u32;
    let clip_right = (right + 1).clamp(0, i64::from(target.width())) as u32;
    let clip_bottom = (bottom + 1).clamp(0, i64::from(target.height())) as u32;
    if clip_left >= clip_right || clip_top >= clip_bottom {
        return;
    }

    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let center_x = (left + right - 1) as f64 * 0.5;
    let center_y = (top + bottom - 1) as f64 * 0.5;
    let half_width = (f64::from(rect.width) - 1.0) * 0.5;
    let half_height = (f64::from(rect.height) - 1.0) * 0.5;
    let corner_radius = f64::from(radius.min(rect.width / 2).min(rect.height / 2));
    let inner_width = half_width - corner_radius;
    let inner_height = half_height - corner_radius;

    for y in clip_top..clip_bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in clip_left..clip_right {
            let mut covered = 0_u16;
            for offset_y in SAMPLE_OFFSETS {
                let py = f64::from(y) + offset_y - center_y;
                for offset_x in SAMPLE_OFFSETS {
                    let px = f64::from(x) + offset_x - center_x;
                    let qx = px.abs() - inner_width;
                    let qy = py.abs() - inner_height;
                    let outside_x = qx.max(0.0);
                    let outside_y = qy.max(0.0);
                    let distance = outside_x.hypot(outside_y) + qx.max(qy).min(0.0) - corner_radius;
                    if distance.abs() <= 0.5 {
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * covered / 16) as u8;
            if alpha != 0 {
                row[x as usize] =
                    Color::rgba(color.r, color.g, color.b, alpha).blend_over(row[x as usize]);
            }
        }
    }
}

/// Fills a rounded rectangle with 4x4 supersampled edge coverage.
///
/// The rectangle uses half-open bounds, clamps the corner radius to its dimensions,
/// clips work to the framebuffer, and composites translucent coverage source-over.
pub fn fill_rounded_rect_aa(target: &mut Framebuffer, rect: Rect, radius: u32, color: Color) {
    if rect.width == 0
        || rect.height == 0
        || color.a == 0
        || target.width() == 0
        || target.height() == 0
    {
        return;
    }

    let left = i64::from(rect.x);
    let top = i64::from(rect.y);
    let right = left + i64::from(rect.width);
    let bottom = top + i64::from(rect.height);
    let clip_left = left.saturating_sub(1).max(0) as u32;
    let clip_top = top.saturating_sub(1).max(0) as u32;
    let clip_right = (right + 1).clamp(0, i64::from(target.width())) as u32;
    let clip_bottom = (bottom + 1).clamp(0, i64::from(target.height())) as u32;
    if clip_left >= clip_right || clip_top >= clip_bottom {
        return;
    }

    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let center_x = (left + right - 1) as f64 * 0.5;
    let center_y = (top + bottom - 1) as f64 * 0.5;
    let half_width = (f64::from(rect.width) - 1.0) * 0.5;
    let half_height = (f64::from(rect.height) - 1.0) * 0.5;
    let corner_radius = f64::from(radius.min(rect.width / 2).min(rect.height / 2));
    let inner_width = half_width - corner_radius;
    let inner_height = half_height - corner_radius;

    for y in clip_top..clip_bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in clip_left..clip_right {
            let mut covered = 0_u16;
            for offset_y in SAMPLE_OFFSETS {
                let py = f64::from(y) + offset_y - center_y;
                for offset_x in SAMPLE_OFFSETS {
                    let px = f64::from(x) + offset_x - center_x;
                    let qx = px.abs() - inner_width;
                    let qy = py.abs() - inner_height;
                    let outside_x = qx.max(0.0);
                    let outside_y = qy.max(0.0);
                    let distance = outside_x.hypot(outside_y) + qx.max(qy).min(0.0) - corner_radius;
                    if distance <= 0.0 {
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * covered / 16) as u8;
            if alpha != 0 {
                row[x as usize] =
                    Color::rgba(color.r, color.g, color.b, alpha).blend_over(row[x as usize]);
            }
        }
    }
}

/// Returns the inclusive horizontal pixel span for one rounded-rectangle scanline.
///
/// Geometry is evaluated at pixel centers in doubled integer coordinates,
/// avoiding floating-point drift and preserving very small rounded rectangles.
fn rounded_rect_scanline_span(rect: Rect, radius: u32, y: i64) -> Option<(i64, i64)> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }

    let radius = radius.min(rect.width / 2).min(rect.height / 2);
    if radius == 0 {
        return Some((
            i64::from(rect.x),
            i64::from(rect.x) + i64::from(rect.width) - 1,
        ));
    }

    let left = i64::from(rect.x);
    let right = left + i64::from(rect.width) - 1;
    let top = i64::from(rect.y);
    let bottom = top + i64::from(rect.height) - 1;
    if y < top || y > bottom {
        return None;
    }

    let radius_twice = i64::from(radius) * 2;
    let radius_squared = (radius_twice as u128) * (radius_twice as u128);
    let pixel_center_y = y * 2 + 1;
    let top_center = top * 2 + radius_twice;
    let bottom_center = (bottom + 1) * 2 - radius_twice;
    let dy = if pixel_center_y < top_center {
        top_center - pixel_center_y
    } else if pixel_center_y > bottom_center {
        pixel_center_y - bottom_center
    } else {
        0
    };
    let dy_squared = (dy as u128) * (dy as u128);
    if dy_squared > radius_squared {
        return None;
    }

    let extent = integer_sqrt(radius_squared - dy_squared) as i64;
    let left_center = left * 2 + radius_twice;
    let right_center = (right + 1) * 2 - radius_twice;
    let left_boundary = left_center - extent;
    let right_boundary = right_center + extent;

    // Pixel centers are at 2*x+1. Convert the continuous boundaries to
    // inclusive integer pixel coordinates, then clamp to the rectangle.
    let first_x =
        (left_boundary - 1).div_euclid(2) + i64::from((left_boundary - 1).rem_euclid(2) != 0);
    let last_x = (right_boundary - 1).div_euclid(2);
    let first_x = first_x.max(left);
    let last_x = last_x.min(right);
    (first_x <= last_x).then_some((first_x, last_x))
}

/// Fills a rounded rectangle using clipped horizontal scanline spans.
///
/// The radius is clamped to half the smaller dimension. A zero radius is
/// equivalent to a regular rectangle fill.
pub fn fill_rounded_rect(target: &mut Framebuffer, rect: Rect, radius: u32, color: Color) {
    if rect.width == 0 || rect.height == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }
    let first_y = i64::from(rect.y).max(0);
    let last_y =
        (i64::from(rect.y) + i64::from(rect.height) - 1).min(i64::from(target.height()) - 1);
    if first_y > last_y {
        return;
    }

    let framebuffer_right = i64::from(target.width()) - 1;
    for y in first_y..=last_y {
        let Some((first_x, last_x)) = rounded_rect_scanline_span(rect, radius, y) else {
            continue;
        };
        let first_x = first_x.max(0);
        let last_x = last_x.min(framebuffer_right);
        if first_x > last_x {
            continue;
        }
        if let Some(row) = target.row_mut(y as u32) {
            row[first_x as usize..=last_x as usize].fill(color);
        }
    }
}

/// Draws the outline of a rounded rectangle with clipped scanline endpoints.
///
/// The radius is clamped to half the smaller dimension. Unlike drawing four
/// complete circles, this emits only the visible quarter-circle corners and
/// the straight side sections.
pub fn draw_rounded_rect(target: &mut Framebuffer, rect: Rect, radius: u32, color: Color) {
    if rect.width == 0 || rect.height == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }

    let radius = radius.min(rect.width / 2).min(rect.height / 2);
    if radius == 0 {
        draw_rect(target, rect, color);
        return;
    }

    let left = i64::from(rect.x);
    let right = left + i64::from(rect.width) - 1;
    let top = i64::from(rect.y);
    let bottom = top + i64::from(rect.height) - 1;
    let framebuffer_right = i64::from(target.width()) - 1;

    // Rounded corners join two straight horizontal sections. Draw those
    // sections explicitly; scanline endpoints are reserved for the curved
    // sides so the top and bottom edges do not collapse to two pixels.
    let horizontal_start = (left + i64::from(radius)).max(0);
    let horizontal_end = (right - i64::from(radius)).min(framebuffer_right);
    if horizontal_start <= horizontal_end {
        for y in [top, bottom] {
            if y >= 0 && y < i64::from(target.height()) {
                if let Some(row) = target.row_mut(y as u32) {
                    row[horizontal_start as usize..=horizontal_end as usize].fill(color);
                }
            }
        }
    } else {
        // When the corner arcs meet (for example, a 2x2 rectangle), there
        // is no straight top or bottom segment. Keep the complete tiny
        // perimeter rather than dropping its only scanlines.
        for y in [top, bottom] {
            if y >= 0 && y < i64::from(target.height()) {
                if let Some((first_x, last_x)) = rounded_rect_scanline_span(rect, radius, y) {
                    let first_x = first_x.max(0);
                    let last_x = last_x.min(framebuffer_right);
                    if first_x <= last_x {
                        if let Some(row) = target.row_mut(y as u32) {
                            row[first_x as usize..=last_x as usize].fill(color);
                        }
                    }
                }
            }
        }
    }

    let first_y = (top + 1).max(0);
    let last_y = (bottom - 1).min(i64::from(target.height()) - 1);
    if first_y > last_y {
        return;
    }

    for y in first_y..=last_y {
        let Some((first_x, last_x)) = rounded_rect_scanline_span(rect, radius, y) else {
            continue;
        };
        let first_x = first_x.max(0);
        let last_x = last_x.min(framebuffer_right);
        if first_x > last_x {
            continue;
        }
        if let Some(row) = target.row_mut(y as u32) {
            row[first_x as usize] = color;
            row[last_x as usize] = color;
        }
    }
}

/// Draws a closed polygon outline. Fewer than two vertices draw nothing.
pub fn draw_polygon(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    if vertices.len() < 2 {
        return;
    }
    for index in 0..vertices.len() {
        draw_line(
            target,
            vertices[index],
            vertices[(index + 1) % vertices.len()],
            color,
        );
    }
}

/// Draws a closed polygon outline with 4x4 supersampled coverage.
///
/// Segment coverage is accumulated as a union, so shared vertices are blended
/// only once and translucent outlines do not become darker at their corners.
pub fn draw_polygon_aa(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    if vertices.len() < 2 || color.a == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }
    if vertices.len() == 2 {
        draw_line_aa(target, vertices[0], vertices[1], color);
        return;
    }
    if vertices.iter().all(|point| *point == vertices[0]) {
        return;
    }

    const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];

    let min_x = vertices.iter().map(|point| point.x).min().unwrap();
    let min_y = vertices.iter().map(|point| point.y).min().unwrap();
    let max_x = vertices.iter().map(|point| point.x).max().unwrap();
    let max_y = vertices.iter().map(|point| point.y).max().unwrap();
    let left = (i64::from(min_x) - 1).max(0) as u32;
    let top = (i64::from(min_y) - 1).max(0) as u32;
    let right = (i64::from(max_x) + 2).clamp(0, i64::from(target.width())) as u32;
    let bottom = (i64::from(max_y) + 2).clamp(0, i64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return;
    }

    let segments = (0..vertices.len())
        .map(|index| {
            let a = vertices[index];
            let b = vertices[(index + 1) % vertices.len()];
            let (ax, ay) = (f64::from(a.x), f64::from(a.y));
            let (dx, dy) = (f64::from(b.x) - ax, f64::from(b.y) - ay);
            (
                ax,
                ay,
                dx,
                dy,
                dx * dx + dy * dy,
                ax.min(f64::from(b.x)) - 0.5,
                ax.max(f64::from(b.x)) + 0.5,
                ay.min(f64::from(b.y)) - 0.5,
                ay.max(f64::from(b.y)) + 0.5,
            )
        })
        .collect::<Vec<_>>();

    let mut coverage = vec![0_u8; (right - left) as usize];
    for y in top..bottom {
        coverage.fill(0);
        for offset_y in OFFSETS {
            let sample_y = f64::from(y) + offset_y;
            for x in left..right {
                let sample_x = f64::from(x);
                let mut covered = 0;
                for offset_x in OFFSETS {
                    let (px, py) = (sample_x + offset_x, sample_y);
                    let on_outline = segments.iter().any(
                        |&(ax, ay, dx, dy, length_squared, min_x, max_x, min_y, max_y)| {
                            if px < min_x || px > max_x || py < min_y || py > max_y {
                                return false;
                            }
                            let t = if length_squared == 0.0 {
                                0.0
                            } else {
                                (((px - ax) * dx + (py - ay) * dy) / length_squared).clamp(0.0, 1.0)
                            };
                            let (nearest_x, nearest_y) = (ax + t * dx, ay + t * dy);
                            let distance_x = px - nearest_x;
                            let distance_y = py - nearest_y;
                            distance_x * distance_x + distance_y * distance_y <= 0.25
                        },
                    );
                    if on_outline {
                        covered += 1;
                    }
                }
                coverage[(x - left) as usize] += covered;
            }
        }
        if let Some(row) = target.row_mut(y) {
            for (pixel, covered) in row[left as usize..right as usize]
                .iter_mut()
                .zip(coverage.iter().copied())
            {
                if covered == 0 {
                    continue;
                }
                let alpha = (u16::from(color.a) * u16::from(covered) / 16) as u8;
                if alpha != 0 {
                    *pixel = Color::rgba(color.r, color.g, color.b, alpha).blend_over(*pixel);
                }
            }
        }
    }
}

/// Fills a polygon using the even-odd scanline rule at pixel centers.
pub fn fill_polygon(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    if vertices.len() < 3 || target.width() == 0 || target.height() == 0 {
        return;
    }
    let min_y = vertices.iter().map(|p| p.y).min().unwrap().max(0) as u32;
    let max_y = vertices.iter().map(|p| p.y).max().unwrap();
    if max_y < 0 || min_y >= target.height() {
        return;
    }
    let max_y = (max_y as u32).min(target.height() - 1);
    // Convert vertices and discard horizontal edges once. Scan conversion
    // then works on compact f64 edge records without repeated indexing,
    // modulo operations, or integer-to-float conversions on every row.
    let mut edges = (0..vertices.len())
        .filter_map(|index| {
            let a = vertices[index];
            let b = vertices[(index + 1) % vertices.len()];
            let (ax, ay, bx, by) = (
                f64::from(a.x),
                f64::from(a.y),
                f64::from(b.x),
                f64::from(b.y),
            );
            (ay != by).then_some((ay.min(by), ay.max(by), ax, ay, bx, by))
        })
        .collect::<Vec<_>>();
    if edges.is_empty() {
        return;
    }
    // Activate only edges crossing the current scanline. Sorting by their
    // first covered y avoids revisiting every polygon edge on every row.
    edges.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));
    let mut active_edges: Vec<usize> = Vec::with_capacity(edges.len());
    let mut next_edge = 0;
    let mut intersections = Vec::with_capacity(edges.len());
    let target_width = target.width();

    for y in min_y..=max_y {
        intersections.clear();
        let scan_y = f64::from(y) + 0.5;
        active_edges.retain(|&index| edges[index].1 > scan_y);
        while next_edge < edges.len() && edges[next_edge].0 <= scan_y {
            if edges[next_edge].1 > scan_y {
                active_edges.push(next_edge);
            }
            next_edge += 1;
        }
        for &index in &active_edges {
            let (_, _, ax, ay, bx, by) = edges[index];
            let t = (scan_y - ay) / (by - ay);
            intersections.push(ax + t * (bx - ax));
        }
        intersections.sort_unstable_by(f64::total_cmp);
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for pair in intersections.chunks_exact(2) {
            let first = (pair[0] - 0.5).ceil().max(0.0) as u32;
            let end = ((pair[1] - 0.5).ceil().max(0.0) as u32).min(target_width);
            let start = first.min(target_width);
            if start < end {
                row[start as usize..end as usize].fill(color);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PolygonEdge {
    min_y: f64,
    max_y: f64,
    min_x: f64,
    inverse_slope: f64,
}

impl PolygonEdge {
    fn new(a: Point, b: Point) -> Option<Self> {
        let (ax, ay) = (f64::from(a.x), f64::from(a.y));
        let (bx, by) = (f64::from(b.x), f64::from(b.y));
        if (ay - by).abs() <= f64::EPSILON {
            return None;
        }
        let (min_x, min_y, max_y, upper_x) = if ay < by {
            (ax, ay, by, bx)
        } else {
            (bx, by, ay, ax)
        };
        Some(Self {
            min_y,
            max_y,
            min_x,
            inverse_slope: (upper_x - min_x) / (max_y - min_y),
        })
    }

    fn intersection_x(self, sample_y: f64) -> f64 {
        self.min_x + (sample_y - self.min_y) * self.inverse_slope
    }
}

/// Fills a polygon with 4x4 supersampled edge coverage and source-over blending.
pub fn fill_polygon_aa(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    if vertices.len() < 3 || color.a == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }

    const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    // Cache the 17 possible source colors produced by 4x4 coverage sampling.
    let mut coverage_colors = [Color::TRANSPARENT; 17];
    for (covered, source) in coverage_colors.iter_mut().enumerate().skip(1) {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        *source = Color::rgba(color.r, color.g, color.b, alpha);
    }
    let min_x = vertices.iter().map(|point| point.x).min().unwrap();
    let min_y = vertices.iter().map(|point| point.y).min().unwrap();
    let max_x = vertices.iter().map(|point| point.x).max().unwrap();
    let max_y = vertices.iter().map(|point| point.y).max().unwrap();
    let left = (i64::from(min_x) - 1).max(0) as u32;
    let top = (i64::from(min_y) - 1).max(0) as u32;
    let right = (i64::from(max_x) + 2).clamp(0, i64::from(target.width())) as u32;
    let bottom = (i64::from(max_y) + 2).clamp(0, i64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return;
    }

    let edges = (0..vertices.len())
        .filter_map(|index| {
            PolygonEdge::new(vertices[index], vertices[(index + 1) % vertices.len()])
        })
        .collect::<Vec<_>>();
    if edges.is_empty() {
        return;
    }

    // Keep edges ordered by their first covered sub-scanline. The four
    // vertical samples advance monotonically, so an active list avoids
    // checking every polygon edge for every sample.
    let mut edge_order = (0..edges.len()).collect::<Vec<_>>();
    edge_order.sort_unstable_by(|&a, &b| edges[a].min_y.total_cmp(&edges[b].min_y));
    let mut active_edges: Vec<usize> = Vec::with_capacity(edges.len());
    let mut next_edge = 0;
    let row_width = (right - left) as usize;
    let mut coverage = vec![0_u8; row_width];
    // Reuse one intersection buffer for every sub-scanline.
    let mut intersections = Vec::with_capacity(edges.len());
    for y in top..bottom {
        coverage.fill(0);
        for offset_y in OFFSETS {
            let sample_y = f64::from(y) + offset_y;
            active_edges.retain(|&index| edges[index].max_y > sample_y);
            while next_edge < edge_order.len() && edges[edge_order[next_edge]].min_y <= sample_y {
                let index = edge_order[next_edge];
                if edges[index].max_y > sample_y {
                    active_edges.push(index);
                }
                next_edge += 1;
            }
            intersections.clear();
            intersections.extend(
                active_edges
                    .iter()
                    .map(|&index| edges[index].intersection_x(sample_y)),
            );
            intersections.sort_unstable_by(f64::total_cmp);

            // Samples are visited from left to right, so advance through the
            // sorted crossings once instead of binary-searching for every
            // horizontal subpixel sample.
            let mut crossing = 0;
            let mut inside = false;
            for x in left..right {
                for offset_x in OFFSETS {
                    let sample_x = f64::from(x) + offset_x;
                    while crossing < intersections.len() && intersections[crossing] <= sample_x {
                        crossing += 1;
                        inside = !inside;
                    }
                    if inside {
                        coverage[(x - left) as usize] += 1;
                    }
                }
            }
        }

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for (pixel, covered) in row[left as usize..right as usize]
            .iter_mut()
            .zip(coverage.iter().copied())
        {
            if covered == 0 {
                continue;
            }
            let source = coverage_colors[covered as usize];
            *pixel = if source.a == u8::MAX {
                source
            } else if source.a > 0 {
                source.blend_over(*pixel)
            } else {
                *pixel
            };
        }
    }
}

/// Draws an anti-aliased triangle outline with 4x4 coverage sampling.
pub fn draw_triangle_aa(target: &mut Framebuffer, a: Point, b: Point, c: Point, color: Color) {
    draw_polygon_aa(target, &[a, b, c], color);
}

/// Fills a triangle with 4x4 coverage sampling and source-over alpha blending.
pub fn fill_triangle_aa(target: &mut Framebuffer, a: Point, b: Point, c: Point, color: Color) {
    fill_polygon_aa(target, &[a, b, c], color);
}

/// Fills a triangle using integer edge functions, pixel-center sampling and
/// the top-left rule. Degenerate triangles draw nothing; vertex order is free.
pub fn fill_triangle(target: &mut Framebuffer, a: Point, b: Point, c: Point, color: Color) {
    let mut vertices = [a, b, c];
    let area = edge(vertices[0], vertices[1], vertices[2]);
    if area == 0 {
        return;
    }
    if area < 0 {
        vertices.swap(1, 2);
    }
    let [a, b, c] = vertices;

    let min_x = a.x.min(b.x).min(c.x).max(0) as u32;
    let min_y = a.y.min(b.y).min(c.y).max(0) as u32;
    let max_x = a.x.max(b.x).max(c.x);
    let max_y = a.y.max(b.y).max(c.y);
    if max_x < 0 || max_y < 0 || min_x >= target.width() || min_y >= target.height() {
        return;
    }
    let max_x = (max_x as u32).min(target.width() - 1);
    let max_y = (max_y as u32).min(target.height() - 1);

    let tl_ab = is_top_left(a, b);
    let tl_bc = is_top_left(b, c);
    let tl_ca = is_top_left(c, a);
    let (ax, ay) = (i128::from(a.x) * 2, i128::from(a.y) * 2);
    let (bx, by) = (i128::from(b.x) * 2, i128::from(b.y) * 2);
    let (cx, cy) = (i128::from(c.x) * 2, i128::from(c.y) * 2);

    // Edge functions are linear across the pixel grid. Step their values
    // incrementally instead of multiplying coordinates three times per pixel.
    let px_start = i128::from(min_x) * 2 + 1;
    let py_start = i128::from(min_y) * 2 + 1;
    let step_x0 = 2 * (by - ay);
    let step_x1 = 2 * (cy - by);
    let step_x2 = 2 * (ay - cy);
    let step_y0 = -2 * (bx - ax);
    let step_y1 = -2 * (cx - bx);
    let step_y2 = -2 * (ax - cx);
    let (mut row_e0, mut row_e1, mut row_e2) = (
        edge_doubled(ax, ay, bx, by, px_start, py_start),
        edge_doubled(bx, by, cx, cy, px_start, py_start),
        edge_doubled(cx, cy, ax, ay, px_start, py_start),
    );

    for y in min_y..=max_y {
        if let Some(row) = target.row_mut(y) {
            let (mut e0, mut e1, mut e2) = (row_e0, row_e1, row_e2);
            for x in min_x..=max_x {
                if (e0 > 0 || (e0 == 0 && tl_ab))
                    && (e1 > 0 || (e1 == 0 && tl_bc))
                    && (e2 > 0 || (e2 == 0 && tl_ca))
                {
                    row[x as usize] = color;
                }
                e0 += step_x0;
                e1 += step_x1;
                e2 += step_x2;
            }
        }
        row_e0 += step_y0;
        row_e1 += step_y1;
        row_e2 += step_y2;
    }
}

fn edge(a: Point, b: Point, p: Point) -> i128 {
    (i128::from(p.x) - i128::from(a.x)) * (i128::from(b.y) - i128::from(a.y))
        - (i128::from(p.y) - i128::from(a.y)) * (i128::from(b.x) - i128::from(a.x))
}

fn edge_doubled(ax: i128, ay: i128, bx: i128, by: i128, px: i128, py: i128) -> i128 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

fn is_top_left(a: Point, b: Point) -> bool {
    let dx = i64::from(b.x) - i64::from(a.x);
    let dy = i64::from(b.y) - i64::from(a.y);
    dy > 0 || (dy == 0 && dx < 0)
}

/// Draws the outline of a circle using an integer ellipse-span algorithm.
/// The radius is measured in pixels; a zero radius draws the center pixel.
pub fn draw_circle(target: &mut Framebuffer, center: Point, radius: u32, color: Color) {
    draw_ellipse(target, center, radius, radius, color);
}

/// Fills a circle, clipping its scanlines to the framebuffer.
pub fn fill_circle(target: &mut Framebuffer, center: Point, radius: u32, color: Color) {
    fill_ellipse(target, center, radius, radius, color);
}

/// Fills an ellipse with 4x4 supersampled edge coverage and source-over blending.
///
/// The ellipse is clipped to the framebuffer. A zero radius produces an anti-aliased
/// line along the remaining axis; a point-sized ellipse draws one pixel.
pub fn fill_ellipse_aa(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
) {
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }
    if radius_x == 0 || radius_y == 0 {
        let rx = i64::from(radius_x);
        let ry = i64::from(radius_y);
        let start = Point::new(
            (i64::from(center.x) - rx).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            (i64::from(center.y) - ry).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        );
        let end = Point::new(
            (i64::from(center.x) + rx).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            (i64::from(center.y) + ry).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        );
        draw_line_aa(target, start, end, color);
        return;
    }

    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    let rx = f64::from(radius_x);
    let ry = f64::from(radius_y);
    let left = (cx - rx - 1.0).floor().max(0.0) as u32;
    let top = (cy - ry - 1.0).floor().max(0.0) as u32;
    let right = (cx + rx + 1.0).ceil().min(f64::from(target.width())) as u32;
    let bottom = (cy + ry + 1.0).ceil().min(f64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return;
    }

    let inverse_rx = rx.recip();
    let inverse_ry = ry.recip();
    for y in top..bottom {
        let normalized_y = SAMPLE_OFFSETS.map(|offset| (f64::from(y) + offset - cy) * inverse_ry);
        let normalized_y_squared = normalized_y.map(|value| value * value);
        let min_y_squared = normalized_y_squared
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let max_y_squared = normalized_y_squared
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            let normalized_x =
                SAMPLE_OFFSETS.map(|offset| (f64::from(x) + offset - cx) * inverse_rx);
            let mut covered = 0_u32;
            for dx in normalized_x {
                let dx_squared = dx * dx;
                if dx_squared + max_y_squared <= 1.0 {
                    // All four vertical samples are inside this ellipse column.
                    covered += 4;
                } else if dx_squared + min_y_squared <= 1.0 {
                    // Only edge columns need the individual sample comparisons.
                    for dy_squared in normalized_y_squared {
                        if dx_squared + dy_squared <= 1.0 {
                            covered += 1;
                        }
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
            if alpha == 0 {
                continue;
            }
            let source = Color::rgba(color.r, color.g, color.b, alpha);
            let index = x as usize;
            row[index] = source.blend_over(row[index]);
        }
    }
}

/// Draws a one-pixel anti-aliased ellipse outline using 4x4 coverage sampling.
pub fn draw_ellipse_aa(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
) {
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }
    if radius_x == 0 || radius_y == 0 {
        let rx = i64::from(radius_x);
        let ry = i64::from(radius_y);
        let start = Point::new(
            (i64::from(center.x) - rx).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            (i64::from(center.y) - ry).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        );
        let end = Point::new(
            (i64::from(center.x) + rx).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            (i64::from(center.y) + ry).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        );
        draw_line_aa(target, start, end, color);
        return;
    }

    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    let rx = f64::from(radius_x);
    let ry = f64::from(radius_y);
    let left = (cx - rx - 1.0).floor().max(0.0) as u32;
    let top = (cy - ry - 1.0).floor().max(0.0) as u32;
    let right = (cx + rx + 1.0).ceil().min(f64::from(target.width())) as u32;
    let bottom = (cy + ry + 1.0).ceil().min(f64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return;
    }

    let inverse_rx_squared = 1.0 / (rx * rx);
    let inverse_ry_squared = 1.0 / (ry * ry);
    for y in top..bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            let mut covered = 0_u32;
            for offset_y in SAMPLE_OFFSETS {
                let dy = f64::from(y) + offset_y - cy;
                let normalized_y = dy * dy * inverse_ry_squared;
                for offset_x in SAMPLE_OFFSETS {
                    let dx = f64::from(x) + offset_x - cx;
                    let normalized_x = dx * dx * inverse_rx_squared;
                    let radius = (normalized_x + normalized_y).sqrt();
                    if radius == 0.0 {
                        continue;
                    }
                    let gradient = (dx * dx * inverse_rx_squared * inverse_rx_squared
                        + dy * dy * inverse_ry_squared * inverse_ry_squared)
                        .sqrt()
                        / radius;
                    let distance = (radius - 1.0) / gradient.max(f64::EPSILON);
                    if distance.abs() <= 0.5 {
                        covered += 1;
                    }
                }
            }
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
            if alpha == 0 {
                continue;
            }
            let source = Color::rgba(color.r, color.g, color.b, alpha);
            let index = x as usize;
            row[index] = source.blend_over(row[index]);
        }
    }
}

/// Draws an anti-aliased circular arc.
///
/// Angles are expressed in radians from the positive X axis. Positive sweeps
/// follow the framebuffer's clockwise direction; when `end_angle` is less
/// than `start_angle`, the arc wraps through zero. A sweep of at least one
/// full turn draws a complete circle. The arc is sampled at 4x4 subpixel
/// positions and composited with source-over alpha.
pub fn draw_arc_aa(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
) {
    use core::f64::consts::TAU;

    if color.a == 0
        || target.width() == 0
        || target.height() == 0
        || !start_angle.is_finite()
        || !end_angle.is_finite()
    {
        return;
    }

    let raw_sweep = f64::from(end_angle) - f64::from(start_angle);
    if raw_sweep == 0.0 {
        return;
    }
    let full_circle = raw_sweep.abs() >= TAU;
    let start = f64::from(start_angle).rem_euclid(TAU);
    let sweep = if full_circle {
        TAU
    } else {
        raw_sweep.rem_euclid(TAU)
    };
    let radius = f64::from(radius);
    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    let left = (cx - radius - 1.0).floor().max(0.0) as u32;
    let top = (cy - radius - 1.0).floor().max(0.0) as u32;
    let right = (cx + radius + 1.0).ceil().min(f64::from(target.width())) as u32;
    let bottom = (cy + radius + 1.0).ceil().min(f64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return;
    }

    if radius == 0.0 {
        if full_circle || sweep > 0.0 {
            draw_pixel(target, center, color);
        }
        return;
    }

    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    for y in top..bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            let mut covered = 0_u32;
            for offset_y in SAMPLE_OFFSETS {
                let dy = f64::from(y) + offset_y - cy;
                for offset_x in SAMPLE_OFFSETS {
                    let dx = f64::from(x) + offset_x - cx;
                    let distance = dx.hypot(dy);
                    if (distance - radius).abs() > 0.5 {
                        continue;
                    }
                    if !full_circle {
                        let angle = dy.atan2(dx).rem_euclid(TAU);
                        if (angle - start).rem_euclid(TAU) > sweep {
                            continue;
                        }
                    }
                    covered += 1;
                }
            }
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
            if alpha == 0 {
                continue;
            }
            let source = Color::rgba(color.r, color.g, color.b, alpha);
            let index = x as usize;
            row[index] = source.blend_over(row[index]);
        }
    }
}

/// Fills an anti-aliased circular sector (pie slice).
///
/// Angles are radians measured from the positive X axis. Positive sweeps are
/// clockwise in framebuffer coordinates; an end angle below the start angle
/// wraps through zero. A sweep of at least one full turn fills a full circle.
/// The circular and radial edges use 4x4 subpixel coverage.
pub fn fill_arc_aa(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
) {
    use core::f64::consts::TAU;

    if color.a == 0
        || target.width() == 0
        || target.height() == 0
        || !start_angle.is_finite()
        || !end_angle.is_finite()
    {
        return;
    }

    let raw_sweep = f64::from(end_angle) - f64::from(start_angle);
    if raw_sweep == 0.0 {
        return;
    }
    let full_circle = raw_sweep.abs() >= TAU;
    let start = f64::from(start_angle).rem_euclid(TAU);
    let sweep = if full_circle {
        TAU
    } else {
        raw_sweep.rem_euclid(TAU)
    };
    let radius = f64::from(radius);
    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    let left = (cx - radius - 1.0).floor().max(0.0) as u32;
    let top = (cy - radius - 1.0).floor().max(0.0) as u32;
    let right = (cx + radius + 1.0).ceil().min(f64::from(target.width())) as u32;
    let bottom = (cy + radius + 1.0).ceil().min(f64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return;
    }

    // Arc points use integer pixel coordinates as their centers. Keep the
    // coverage samples centered on each destination pixel as well.
    const SAMPLE_OFFSETS: [f64; 4] = [-0.375, -0.125, 0.125, 0.375];
    for y in top..bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            let mut covered = 0_u32;
            for offset_y in SAMPLE_OFFSETS {
                let dy = f64::from(y) + offset_y - cy;
                for offset_x in SAMPLE_OFFSETS {
                    let dx = f64::from(x) + offset_x - cx;
                    if dx.hypot(dy) > radius {
                        continue;
                    }
                    if !full_circle {
                        let angle = dy.atan2(dx).rem_euclid(TAU);
                        if (angle - start).rem_euclid(TAU) > sweep {
                            continue;
                        }
                    }
                    covered += 1;
                }
            }
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
            if alpha == 0 {
                continue;
            }
            let source = Color::rgba(color.r, color.g, color.b, alpha);
            let index = x as usize;
            row[index] = source.blend_over(row[index]);
        }
    }
}

/// Fills a circle with anti-aliased edge coverage.
pub fn fill_circle_aa(target: &mut Framebuffer, center: Point, radius: u32, color: Color) {
    fill_ellipse_aa(target, center, radius, radius, color);
}

/// Draws an anti-aliased circle outline with the ellipse coverage rasterizer.
pub fn draw_circle_aa(target: &mut Framebuffer, center: Point, radius: u32, color: Color) {
    draw_ellipse_aa(target, center, radius, radius, color);
}

/// Draws an ellipse outline from its center and horizontal/vertical radii.
/// The result is symmetric and clipped, including when the center is off-screen.
pub fn draw_ellipse(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
) {
    for_each_ellipse_span(
        target.width(),
        target.height(),
        center,
        radius_x,
        radius_y,
        |y, left, right| {
            if let Some(row) = target.row_mut(y) {
                row[left as usize] = color;
                row[right as usize] = color;
            }
        },
    );
}

/// Fills an ellipse using integer scanline spans, clipped to the framebuffer.
/// A zero horizontal or vertical radius produces a line.
pub fn fill_ellipse(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
) {
    for_each_ellipse_span(
        target.width(),
        target.height(),
        center,
        radius_x,
        radius_y,
        |y, left, right| {
            if let Some(row) = target.row_mut(y) {
                row[left as usize..=right as usize].fill(color);
            }
        },
    );
}

fn for_each_ellipse_span(
    width: u32,
    height: u32,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    mut draw_span: impl FnMut(u32, i64, i64),
) {
    if width == 0 || height == 0 {
        return;
    }

    let cx = i64::from(center.x);
    let cy = i64::from(center.y);
    let rx = u128::from(radius_x);
    let ry = u128::from(radius_y);
    let x_limit = i64::from(width - 1);
    let y_limit = i64::from(height - 1);

    if radius_y == 0 {
        if cy < 0 || cy > y_limit {
            return;
        }
        let left = (cx - i64::from(radius_x)).max(0);
        let right = (cx + i64::from(radius_x)).min(x_limit);
        if left <= right {
            draw_span(cy as u32, left, right);
        }
        return;
    }

    let first_y = (cy - i64::from(radius_y)).max(0);
    let last_y = (cy + i64::from(radius_y)).min(y_limit);
    if first_y > last_y {
        return;
    }

    if radius_x == 0 {
        if cx >= 0 && cx <= x_limit {
            for y in first_y..=last_y {
                draw_span(y as u32, cx, cx);
            }
        }
        return;
    }

    let rx_squared = rx * rx;
    let ry_squared = ry * ry;
    for y in first_y..=last_y {
        let dy = (i128::from(y) - i128::from(center.y)).unsigned_abs();
        let dy = dy.min(ry);
        let remaining = ry_squared - dy * dy;
        let x_squared = rx_squared * remaining / ry_squared;
        let extent = integer_sqrt(x_squared) as i64;
        let left = (cx - extent).max(0);
        let right = (cx + extent).min(x_limit);
        if left <= right {
            draw_span(y as u32, left, right);
        }
    }
}

fn integer_sqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut low = 1_u128;
    let mut high = u64::MAX as u128 + 1;
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if middle * middle <= value {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

#[cfg(test)]
mod tests {
    use super::{
        blend_pixel, draw_circle, draw_ellipse, draw_ellipse_aa, draw_line, draw_line_aa,
        draw_pixel, draw_polygon, draw_polygon_aa, draw_rect, fill_circle, fill_circle_aa,
        fill_ellipse, fill_ellipse_aa, fill_polygon, fill_polygon_aa, fill_rect, fill_rounded_rect,
        fill_triangle,
    };
    use crate::{Color, Framebuffer, Point, Rect};

    fn target(width: u32, height: u32) -> Framebuffer {
        Framebuffer::new(width, height, Color::BLACK).unwrap()
    }

    fn test_gradient() -> super::Gradient {
        super::Gradient::new(vec![
            super::GradientStop {
                position: 1.0,
                color: Color::BLUE,
            },
            super::GradientStop {
                position: 0.0,
                color: Color::RED,
            },
        ])
        .unwrap()
    }

    #[test]
    fn raster_gradient_lookup_stays_within_one_channel_of_exact_sampling() {
        let gradient = super::Gradient::new(vec![
            super::GradientStop {
                position: 0.0,
                color: Color::rgb(35, 110, 230),
            },
            super::GradientStop {
                position: 0.5,
                color: Color::rgb(50, 210, 160),
            },
            super::GradientStop {
                position: 1.0,
                color: Color::rgb(245, 130, 45),
            },
        ])
        .unwrap();

        for index in 0..=10_000 {
            let position = index as f32 / 10_000.0;
            let exact = gradient.sample(position);
            let fast = gradient.sample_raster(position);
            for (a, b) in [
                (exact.r, fast.r),
                (exact.g, fast.g),
                (exact.b, fast.b),
                (exact.a, fast.a),
            ] {
                assert!(
                    a.abs_diff(b) <= 1,
                    "position {position}: {exact:?} != {fast:?}"
                );
            }
        }
    }

    #[test]
    fn raster_gradient_lookup_preserves_duplicate_stop_hard_edges() {
        let gradient = super::Gradient::new(vec![
            super::GradientStop {
                position: 0.0,
                color: Color::RED,
            },
            super::GradientStop {
                position: 0.5,
                color: Color::GREEN,
            },
            super::GradientStop {
                position: 0.5,
                color: Color::BLUE,
            },
            super::GradientStop {
                position: 1.0,
                color: Color::WHITE,
            },
        ])
        .unwrap();

        assert!(gradient.lookup.is_none());
        for position in [0.0, 0.25, 0.4999, 0.5, 0.5001, 0.75, 1.0] {
            assert_eq!(gradient.sample_raster(position), gradient.sample(position));
        }
    }

    #[test]
    fn gradient_sorts_stops_and_interpolates_midpoint() {
        let gradient = test_gradient();
        assert_eq!(gradient.sample(0.0), Color::RED);
        assert_eq!(gradient.sample(0.5), Color::rgb(128, 0, 128));
        assert_eq!(gradient.sample(1.0), Color::BLUE);
        assert_eq!(gradient.sample(-2.0), Color::RED);
        assert_eq!(gradient.sample(2.0), Color::BLUE);
    }

    #[test]
    fn gradient_rejects_invalid_stop_definitions() {
        use super::{Gradient, GradientError, GradientStop};
        assert_eq!(Gradient::new(vec![]), Err(GradientError::TooFewStops));
        assert_eq!(
            Gradient::new(vec![
                GradientStop {
                    position: 0.0,
                    color: Color::BLACK,
                },
                GradientStop {
                    position: f32::NAN,
                    color: Color::WHITE,
                },
            ]),
            Err(GradientError::InvalidPosition)
        );
    }

    #[test]
    fn linear_gradient_fills_clipped_rectangle_at_pixel_centers() {
        use super::fill_rect_linear_gradient;
        use crate::Vector;
        let mut image = target(4, 1);
        fill_rect_linear_gradient(
            &mut image,
            Rect::new(0, 0, 4, 1),
            Vector::new(0.0, 0.5),
            Vector::new(4.0, 0.5),
            &test_gradient(),
        );
        assert_eq!(image.get_pixel(0, 0), Some(Color::rgb(223, 0, 32)));
        assert_eq!(image.get_pixel(3, 0), Some(Color::rgb(32, 0, 223)));
    }

    #[test]
    fn linear_gradient_blends_transparent_stops_over_existing_pixels() {
        use super::{fill_rect_linear_gradient, Gradient, GradientStop};
        use crate::Vector;
        let gradient = Gradient::new(vec![
            GradientStop {
                position: 0.0,
                color: Color::rgba(255, 0, 0, 0),
            },
            GradientStop {
                position: 1.0,
                color: Color::rgba(255, 0, 0, 255),
            },
        ])
        .unwrap();
        let mut image = target(2, 1);
        fill_rect_linear_gradient(
            &mut image,
            Rect::new(0, 0, 2, 1),
            Vector::new(0.0, 0.0),
            Vector::new(2.0, 0.0),
            &gradient,
        );
        assert_eq!(image.get_pixel(0, 0), Some(Color::rgb(64, 0, 0)));
        assert_eq!(image.get_pixel(1, 0), Some(Color::rgb(191, 0, 0)));
    }

    #[test]
    fn radial_gradient_uses_distance_and_clips_to_rectangle() {
        use super::fill_rect_radial_gradient;
        use crate::Vector;
        let mut image = target(5, 5);
        fill_rect_radial_gradient(
            &mut image,
            Rect::new(1, 1, 3, 3),
            Vector::new(2.5, 2.5),
            2.0,
            &test_gradient(),
        );
        assert_eq!(image.get_pixel(2, 2), Some(Color::rgb(255, 0, 0)));
        assert_eq!(image.get_pixel(1, 1), Some(Color::rgb(75, 0, 180)));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn invalid_gradient_geometry_draws_nothing() {
        use super::fill_rect_radial_gradient;
        use crate::Vector;
        let mut image = target(3, 3);
        fill_rect_radial_gradient(
            &mut image,
            Rect::new(0, 0, 3, 3),
            Vector::new(1.0, 1.0),
            0.0,
            &test_gradient(),
        );
        super::fill_rect_linear_gradient(
            &mut image,
            Rect::new(0, 0, 3, 3),
            Vector::new(f32::NAN, 0.0),
            Vector::new(2.0, 2.0),
            &test_gradient(),
        );
        assert!(image.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn circle_outline_is_symmetric_and_radius_zero_is_one_pixel() {
        let mut image = target(9, 9);
        draw_circle(&mut image, Point::new(4, 4), 3, Color::RED);
        assert_eq!(image.get_pixel(4, 1), Some(Color::RED));
        assert_eq!(image.get_pixel(4, 7), Some(Color::RED));
        assert_eq!(image.get_pixel(1, 4), Some(Color::RED));
        assert_eq!(image.get_pixel(7, 4), Some(Color::RED));
        assert_eq!(image.get_pixel(4, 4), Some(Color::BLACK));

        draw_circle(&mut image, Point::new(0, 0), 0, Color::GREEN);
        assert_eq!(image.get_pixel(0, 0), Some(Color::GREEN));
    }

    #[test]
    fn filled_circle_has_center_and_clipped_edges() {
        let mut image = target(5, 5);
        fill_circle(&mut image, Point::new(0, 0), 2, Color::BLUE);
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLUE));
        assert_eq!(image.get_pixel(2, 0), Some(Color::BLUE));
        assert_eq!(image.get_pixel(0, 2), Some(Color::BLUE));
        assert_eq!(image.get_pixel(3, 0), Some(Color::BLACK));
        assert_eq!(image.get_pixel(4, 4), Some(Color::BLACK));
    }

    #[test]
    fn ellipse_respects_independent_radii() {
        let mut image = target(11, 9);
        draw_ellipse(&mut image, Point::new(5, 4), 4, 2, Color::RED);
        assert_eq!(image.get_pixel(1, 4), Some(Color::RED));
        assert_eq!(image.get_pixel(9, 4), Some(Color::RED));
        assert_eq!(image.get_pixel(5, 2), Some(Color::RED));
        assert_eq!(image.get_pixel(5, 6), Some(Color::RED));
        assert_eq!(image.get_pixel(5, 4), Some(Color::BLACK));
    }

    #[test]
    fn filled_ellipse_covers_interior_and_is_clipped() {
        let mut image = target(6, 5);
        fill_ellipse(&mut image, Point::new(1, 2), 4, 2, Color::GREEN);
        assert_eq!(image.get_pixel(1, 2), Some(Color::GREEN));
        assert_eq!(image.get_pixel(5, 2), Some(Color::GREEN));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(image.get_pixel(1, 4), Some(Color::GREEN));
        assert_eq!(image.get_pixel(5, 4), Some(Color::BLACK));
    }

    #[test]
    fn antialiased_polygon_outline_two_vertices_matches_line_fallback() {
        let mut expected = target(10, 7);
        let mut actual = target(10, 7);
        draw_line_aa(
            &mut expected,
            Point::new(-3, 1),
            Point::new(8, 5),
            Color::rgba(40, 180, 240, 160),
        );
        draw_polygon_aa(
            &mut actual,
            &[Point::new(-3, 1), Point::new(8, 5)],
            Color::rgba(40, 180, 240, 160),
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn antialiased_polygon_outline_clips_large_coordinates_and_ignores_transparency() {
        let mut image = target(5, 4);
        draw_polygon_aa(
            &mut image,
            &[
                Point::new(-100, -100),
                Point::new(100, -100),
                Point::new(100, 2),
                Point::new(-100, 2),
            ],
            Color::TRANSPARENT,
        );
        assert!(image.pixels().iter().all(|pixel| *pixel == Color::BLACK));

        draw_polygon_aa(
            &mut image,
            &[
                Point::new(-100, -100),
                Point::new(100, -100),
                Point::new(100, 2),
                Point::new(-100, 2),
            ],
            Color::GREEN,
        );
        assert!(image
            .pixels()
            .iter()
            .any(|pixel| pixel.g > 0 && pixel.g < 255));
        assert_eq!(image.pixels().len(), 20);
    }

    #[test]
    fn antialiased_polygon_outline_handles_repeated_and_degenerate_vertices() {
        let mut image = target(8, 8);
        draw_polygon_aa(
            &mut image,
            &[
                Point::new(2, 2),
                Point::new(2, 2),
                Point::new(5, 2),
                Point::new(5, 5),
                Point::new(2, 5),
                Point::new(2, 2),
            ],
            Color::BLUE,
        );
        assert!(image.pixels().iter().any(|pixel| pixel.b > 0));
        assert_eq!(image.get_pixel(3, 3), Some(Color::BLACK));

        let mut degenerate = target(8, 8);
        draw_polygon_aa(
            &mut degenerate,
            &[Point::new(3, 3), Point::new(3, 3), Point::new(3, 3)],
            Color::RED,
        );
        assert!(degenerate
            .pixels()
            .iter()
            .all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn antialiased_rectangle_outline_is_clipped_and_does_not_fill_interior() {
        let mut image = target(7, 6);
        super::draw_rect_aa(&mut image, Rect::new(4, 1, 7, 4), Color::WHITE);
        assert_eq!(image.get_pixel(5, 2), Some(Color::BLACK));
        assert!(image
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
        assert!(image.pixels().iter().any(|pixel| pixel.r > 0));

        let before = image.clone();
        super::draw_rect_aa(&mut image, Rect::new(1, 1, 0, 4), Color::RED);
        super::draw_rect_aa(&mut image, Rect::new(1, 1, 4, 0), Color::RED);
        assert_eq!(image, before);
    }

    #[test]
    fn antialiased_rounded_rectangle_outline_preserves_center_and_clips() {
        let mut image = target(10, 8);
        super::draw_rounded_rect_aa(&mut image, Rect::new(2, 1, 12, 6), 2, Color::WHITE);
        assert_eq!(image.get_pixel(5, 4), Some(Color::BLACK));
        assert!(image
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
        assert!(image.pixels().iter().any(|pixel| pixel.r > 0));
    }

    #[test]
    fn antialiased_rounded_rectangle_outline_handles_zero_size_and_transparency() {
        let mut image = target(6, 6);
        let before = image.clone();
        super::draw_rounded_rect_aa(&mut image, Rect::new(1, 1, 0, 4), 2, Color::WHITE);
        super::draw_rounded_rect_aa(&mut image, Rect::new(1, 1, 4, 0), 2, Color::WHITE);
        super::draw_rounded_rect_aa(&mut image, Rect::new(1, 1, 4, 4), 2, Color::TRANSPARENT);
        assert_eq!(image, before);
    }

    #[test]
    fn antialiased_polygon_outline_draws_edges_without_filling_center() {
        let mut image = target(12, 10);
        draw_polygon_aa(
            &mut image,
            &[
                Point::new(2, 2),
                Point::new(9, 2),
                Point::new(9, 7),
                Point::new(2, 7),
            ],
            Color::WHITE,
        );
        let edge = image.get_pixel(5, 2).unwrap();
        assert!(edge.r > 0 && edge.r < 255);
        assert_eq!(image.get_pixel(5, 4), Some(Color::BLACK));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
        assert!(image
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
    }

    #[test]
    fn antialiased_polygon_outline_blends_shared_vertices_only_once() {
        let mut image = target(12, 10);
        draw_polygon_aa(
            &mut image,
            &[
                Point::new(2, 2),
                Point::new(9, 2),
                Point::new(9, 7),
                Point::new(2, 7),
            ],
            Color::rgba(255, 0, 0, 128),
        );
        let corner = image.get_pixel(2, 2).unwrap();
        assert!(corner.r > 0);
        assert!(corner.r < 255);
        assert_eq!(image.get_pixel(5, 4), Some(Color::BLACK));
    }

    #[test]
    fn antialiased_polygon_blends_partial_edge_coverage() {
        let mut image = target(8, 8);
        fill_polygon_aa(
            &mut image,
            &[Point::new(1, 1), Point::new(6, 2), Point::new(3, 6)],
            Color::WHITE,
        );
        let pixels = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter_map(|(x, y)| image.get_pixel(x, y))
            .collect::<Vec<_>>();
        assert!(pixels.iter().any(|pixel| pixel.r > 0 && pixel.r < 255));
        assert!(pixels.contains(&Color::WHITE));
    }

    #[test]
    fn antialiased_polygon_is_clipped_and_respects_source_alpha() {
        let mut image = target(4, 4);
        fill_polygon_aa(
            &mut image,
            &[
                Point::new(-4, -4),
                Point::new(3, -4),
                Point::new(3, 3),
                Point::new(-4, 3),
            ],
            Color::rgba(255, 0, 0, 128),
        );
        assert_eq!(image.get_pixel(3, 3), Some(Color::BLACK));
        assert!(image.get_pixel(0, 0).unwrap().r > 0);
        assert!(image.get_pixel(0, 0).unwrap().r < 255);
    }

    #[test]
    fn antialiased_polygon_uses_even_odd_fill_for_concave_shapes() {
        let mut image = target(8, 8);
        fill_polygon_aa(
            &mut image,
            &[
                Point::new(1, 1),
                Point::new(6, 1),
                Point::new(6, 3),
                Point::new(3, 3),
                Point::new(3, 6),
                Point::new(1, 6),
            ],
            Color::WHITE,
        );
        assert_eq!(image.get_pixel(2, 4), Some(Color::WHITE));
        assert_eq!(image.get_pixel(5, 5), Some(Color::BLACK));
    }

    #[test]
    fn antialiased_ellipse_blends_partial_edge_coverage() {
        let mut image = target(13, 11);
        fill_ellipse_aa(&mut image, Point::new(6, 5), 4, 3, Color::WHITE);
        assert_eq!(image.get_pixel(6, 5), Some(Color::WHITE));
        assert!(image
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn antialiased_ellipse_outline_draws_edges_without_filling_center() {
        let mut image = target(17, 13);
        draw_ellipse_aa(&mut image, Point::new(8, 6), 5, 3, Color::WHITE);

        assert_eq!(image.get_pixel(8, 6), Some(Color::BLACK));
        assert!(image.pixels().iter().any(|pixel| pixel.r > 0));
        assert!(image.pixels().iter().any(|pixel| {
            pixel.r > 0 && pixel.r < 255 && pixel.g == pixel.r && pixel.b == pixel.r
        }));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn antialiased_ellipse_outline_blends_partial_coverage_over_existing_pixels() {
        let mut image = target(19, 15);
        fill_rect(&mut image, Rect::new(0, 0, 19, 15), Color::BLUE);
        draw_ellipse_aa(
            &mut image,
            Point::new(9, 7),
            6,
            4,
            Color::rgba(255, 0, 0, 192),
        );

        assert_eq!(image.get_pixel(9, 7), Some(Color::BLUE));
        assert!(image
            .pixels()
            .iter()
            .any(|pixel| { pixel.r > 0 && pixel.b > 0 && pixel.r < 255 && pixel.b < 255 }));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLUE));
    }

    #[test]
    fn antialiased_ellipse_outline_handles_degenerate_radii() {
        let mut expected = target(9, 7);
        let mut actual = target(9, 7);
        draw_line_aa(
            &mut expected,
            Point::new(2, 3),
            Point::new(6, 3),
            Color::GREEN,
        );
        draw_ellipse_aa(&mut actual, Point::new(4, 3), 2, 0, Color::GREEN);
        assert_eq!(actual, expected);

        let mut expected = target(9, 7);
        let mut actual = target(9, 7);
        draw_line_aa(
            &mut expected,
            Point::new(4, 0),
            Point::new(4, 6),
            Color::GREEN,
        );
        draw_ellipse_aa(&mut actual, Point::new(4, 3), 0, 3, Color::GREEN);
        assert_eq!(actual, expected);

        let mut expected = target(9, 7);
        let mut actual = target(9, 7);
        draw_line_aa(
            &mut expected,
            Point::new(4, 3),
            Point::new(4, 3),
            Color::GREEN,
        );
        draw_ellipse_aa(&mut actual, Point::new(4, 3), 0, 0, Color::GREEN);
        assert_eq!(actual, expected);
    }

    #[test]
    fn antialiased_ellipse_outline_is_clipped_and_respects_transparency() {
        let mut image = target(5, 4);
        draw_ellipse_aa(
            &mut image,
            Point::new(0, 1),
            4,
            3,
            Color::rgba(255, 0, 0, 0),
        );
        assert!(image.pixels().iter().all(|pixel| *pixel == Color::BLACK));

        draw_ellipse_aa(&mut image, Point::new(0, 1), 4, 3, Color::BLUE);
        assert!(image.pixels().iter().any(|pixel| pixel.b > 0));
        assert_eq!(image.pixels().len(), 20);
    }

    #[test]
    fn antialiased_ellipse_is_clipped_and_respects_source_alpha() {
        let mut image = target(5, 5);
        fill_circle_aa(&mut image, Point::new(0, 0), 3, Color::rgba(255, 0, 0, 128));
        assert!(image.pixels().iter().any(|pixel| pixel.r > 0));
        assert!(image.pixels().iter().all(|pixel| pixel.b == 0));
        assert_eq!(image.get_pixel(4, 4), Some(Color::BLACK));
    }

    #[test]
    fn antialiased_degenerate_ellipse_uses_line_fallback() {
        let mut image = target(5, 3);
        fill_ellipse_aa(&mut image, Point::new(2, 1), 2, 0, Color::WHITE);
        assert_eq!(image.get_pixel(0, 1), Some(Color::WHITE));
        assert_eq!(image.get_pixel(4, 1), Some(Color::WHITE));
        assert_eq!(image.get_pixel(2, 0), Some(Color::BLACK));
    }

    #[test]
    fn zero_radius_ellipse_axes_are_lines() {
        let mut horizontal = target(5, 3);
        fill_ellipse(&mut horizontal, Point::new(2, 1), 2, 0, Color::RED);
        for x in 0..5 {
            assert_eq!(horizontal.get_pixel(x, 1), Some(Color::RED));
        }

        let mut vertical = target(3, 5);
        fill_ellipse(&mut vertical, Point::new(1, 2), 0, 2, Color::BLUE);
        for y in 0..5 {
            assert_eq!(vertical.get_pixel(1, y), Some(Color::BLUE));
        }
    }

    #[test]
    fn huge_ellipse_radii_are_clipped_without_coordinate_overflow() {
        let mut image = target(3, 3);
        fill_ellipse(
            &mut image,
            Point::new(1, 1),
            u32::MAX,
            u32::MAX,
            Color::WHITE,
        );
        assert!(image.pixels().iter().all(|pixel| *pixel == Color::WHITE));
    }

    #[test]
    fn rectangle_outline_respects_half_open_bounds() {
        let mut image = target(5, 5);
        draw_rect(&mut image, Rect::new(1, 1, 3, 2), Color::RED);
        assert_eq!(image.get_pixel(1, 1), Some(Color::RED));
        assert_eq!(image.get_pixel(3, 2), Some(Color::RED));
        assert_eq!(image.get_pixel(2, 3), Some(Color::BLACK));
    }

    #[test]
    fn rounded_rectangle_stays_inside_its_bounds() {
        let mut image = target(8, 8);
        fill_rounded_rect(&mut image, Rect::new(1, 1, 6, 6), 2, Color::GREEN);
        assert_eq!(image.get_pixel(3, 1), Some(Color::GREEN));
        assert_eq!(image.get_pixel(1, 1), Some(Color::BLACK));
        assert_eq!(image.get_pixel(5, 5), Some(Color::GREEN));
        assert_eq!(image.get_pixel(6, 6), Some(Color::BLACK));
    }

    #[test]
    fn rounded_rectangle_corner_circles_are_clipped_to_the_rectangle() {
        let mut image = target(8, 8);
        fill_rounded_rect(&mut image, Rect::new(1, 2, 6, 2), 1, Color::BLUE);
        assert_eq!(image.get_pixel(0, 2), Some(Color::BLACK));
        assert_eq!(image.get_pixel(1, 1), Some(Color::BLACK));
        assert_eq!(image.get_pixel(1, 2), Some(Color::BLUE));
        assert_eq!(image.get_pixel(6, 3), Some(Color::BLUE));
        assert_eq!(image.get_pixel(6, 4), Some(Color::BLACK));
    }

    #[test]
    fn rounded_rectangle_radius_is_clamped_and_zero_is_plain_rect() {
        let mut rounded = target(4, 4);
        fill_rounded_rect(&mut rounded, Rect::new(0, 0, 4, 2), u32::MAX, Color::BLUE);
        assert_eq!(rounded.get_pixel(0, 0), Some(Color::BLUE));
        assert_eq!(rounded.get_pixel(1, 0), Some(Color::BLUE));
        let mut plain = target(3, 3);
        fill_rounded_rect(&mut plain, Rect::new(0, 0, 2, 2), 0, Color::RED);
        assert_eq!(plain.get_pixel(0, 0), Some(Color::RED));
    }

    #[test]
    fn polygon_fill_uses_even_odd_scanlines() {
        let mut image = target(6, 6);
        fill_polygon(
            &mut image,
            &[Point::new(1, 1), Point::new(5, 1), Point::new(3, 5)],
            Color::RED,
        );
        assert_eq!(image.get_pixel(3, 1), Some(Color::RED));
        assert_eq!(image.get_pixel(3, 3), Some(Color::RED));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
        assert_eq!(image.get_pixel(3, 5), Some(Color::BLACK));
    }

    #[test]
    fn polygon_outline_closes_and_fill_ignores_too_few_vertices() {
        let mut image = target(4, 4);
        draw_polygon(
            &mut image,
            &[Point::new(1, 1), Point::new(2, 1), Point::new(2, 2)],
            Color::WHITE,
        );
        assert_eq!(image.get_pixel(1, 1), Some(Color::WHITE));
        assert_eq!(image.get_pixel(2, 2), Some(Color::WHITE));
        fill_polygon(
            &mut image,
            &[Point::new(0, 0), Point::new(3, 3)],
            Color::RED,
        );
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn pixel_write_checks_bounds() {
        let mut image = target(2, 2);
        assert!(draw_pixel(&mut image, Point::new(1, 1), Color::RED));
        assert!(!draw_pixel(&mut image, Point::new(-1, 0), Color::RED));
        assert!(!draw_pixel(&mut image, Point::new(2, 0), Color::RED));
        assert_eq!(image.get_pixel(1, 1), Some(Color::RED));
    }

    #[test]
    fn blended_pixel_uses_source_over_alpha() {
        let mut image = target(1, 1);
        assert!(blend_pixel(
            &mut image,
            Point::new(0, 0),
            Color::rgba(255, 0, 0, 128)
        ));
        assert_eq!(image.get_pixel(0, 0), Some(Color::rgba(128, 0, 0, 255)));
        assert!(!blend_pixel(&mut image, Point::new(1, 0), Color::WHITE));
    }

    #[test]
    fn horizontal_and_vertical_lines_include_endpoints() {
        let mut image = target(4, 4);
        draw_line(&mut image, Point::new(0, 1), Point::new(3, 1), Color::RED);
        draw_line(&mut image, Point::new(2, 0), Point::new(2, 3), Color::GREEN);
        for x in 0..4 {
            assert_eq!(
                image.get_pixel(x, 1),
                Some(if x == 2 { Color::GREEN } else { Color::RED })
            );
        }
        for y in 0..4 {
            assert_eq!(image.get_pixel(2, y), Some(Color::GREEN));
        }
    }

    #[test]
    fn diagonal_line_is_symmetric_when_reversed() {
        let mut forward = target(5, 5);
        let mut reverse = target(5, 5);
        draw_line(
            &mut forward,
            Point::new(0, 0),
            Point::new(4, 3),
            Color::WHITE,
        );
        draw_line(
            &mut reverse,
            Point::new(4, 3),
            Point::new(0, 0),
            Color::WHITE,
        );
        assert_eq!(forward, reverse);
    }

    #[test]
    fn antialiased_line_blends_edge_coverage() {
        let mut image = target(8, 8);
        draw_line_aa(&mut image, Point::new(1, 1), Point::new(6, 4), Color::WHITE);
        assert!(image.pixels().iter().any(|pixel| pixel.r == 255));
        assert!(image
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
    }

    #[test]
    fn antialiased_line_is_clipped_and_reversible() {
        let mut forward = target(6, 4);
        let mut reverse = target(6, 4);
        draw_line_aa(
            &mut forward,
            Point::new(-20, 0),
            Point::new(20, 3),
            Color::RED,
        );
        draw_line_aa(
            &mut reverse,
            Point::new(20, 3),
            Point::new(-20, 0),
            Color::RED,
        );
        assert_eq!(forward, reverse);
        assert!(forward.pixels().iter().any(|pixel| pixel.r > 0));
    }

    #[test]
    fn line_is_clipped_to_framebuffer() {
        let mut image = target(5, 3);
        draw_line(
            &mut image,
            Point::new(-100, 1),
            Point::new(100, 1),
            Color::BLUE,
        );
        for x in 0..5 {
            assert_eq!(image.get_pixel(x, 1), Some(Color::BLUE));
        }
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn line_outside_target_draws_nothing() {
        let mut image = target(4, 4);
        draw_line(
            &mut image,
            Point::new(-5, -2),
            Point::new(-1, -8),
            Color::WHITE,
        );
        assert!(image.pixels().iter().all(|pixel| *pixel == Color::BLACK));
    }

    #[test]
    fn rectangle_uses_half_open_bounds() {
        let mut image = target(4, 4);
        fill_rect(&mut image, Rect::new(1, 1, 2, 2), Color::RED);
        assert_eq!(image.get_pixel(1, 1), Some(Color::RED));
        assert_eq!(image.get_pixel(2, 2), Some(Color::RED));
        assert_eq!(image.get_pixel(3, 2), Some(Color::BLACK));
        assert_eq!(image.get_pixel(2, 3), Some(Color::BLACK));
    }

    #[test]
    fn rectangle_is_clipped_and_zero_area_is_ignored() {
        let mut image = target(3, 3);
        fill_rect(
            &mut image,
            Rect::new(2, 2, u32::MAX, u32::MAX),
            Color::GREEN,
        );
        fill_rect(&mut image, Rect::new(0, 0, 0, 3), Color::RED);
        assert_eq!(image.get_pixel(2, 2), Some(Color::GREEN));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn triangle_is_independent_of_vertex_order() {
        let mut forward = target(3, 3);
        let mut reverse = target(3, 3);
        fill_triangle(
            &mut forward,
            Point::new(0, 0),
            Point::new(0, 2),
            Point::new(2, 0),
            Color::RED,
        );
        fill_triangle(
            &mut reverse,
            Point::new(0, 0),
            Point::new(2, 0),
            Point::new(0, 2),
            Color::RED,
        );
        assert_eq!(forward, reverse);
        assert_eq!(forward.get_pixel(0, 0), Some(Color::RED));
        assert_eq!(forward.get_pixel(2, 2), Some(Color::BLACK));
    }

    #[test]
    fn adjacent_triangles_cover_square_without_cracks() {
        let mut image = target(2, 2);
        fill_triangle(
            &mut image,
            Point::new(0, 0),
            Point::new(0, 2),
            Point::new(2, 0),
            Color::RED,
        );
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
