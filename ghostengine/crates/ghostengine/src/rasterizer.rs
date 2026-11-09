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
            if alpha > 0 {
                blend_pixel(
                    target,
                    Point::new(px as i32, py as i32),
                    Color::rgba(color.r, color.g, color.b, alpha),
                );
            }
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
    for y in top..bottom {
        if let Some(row) = target.row_mut(y as u32) {
            for pixel in &mut row[left as usize..right as usize] {
                *pixel = color.blend_over(*pixel);
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
        let upper = self.stops.partition_point(|stop| stop.position <= position);
        let left = self.stops[upper - 1];
        let right = self.stops[upper];
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

fn paint_gradient_pixel(target: &mut Framebuffer, x: u32, y: u32, color: Color) {
    if color.a == 255 {
        target.set_pixel(x, y, color);
    } else {
        blend_pixel(target, Point::new(x as i32, y as i32), color);
    }
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
    for y in top..bottom {
        for x in left..right {
            let position = if length_squared <= f32::EPSILON {
                0.0
            } else {
                (((x as f32 + 0.5 - start.x) * dx + (y as f32 + 0.5 - start.y) * dy)
                    / length_squared)
                    .clamp(0.0, 1.0)
            };
            paint_gradient_pixel(target, x, y, gradient.sample(position));
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
    for y in top..bottom {
        for x in left..right {
            let dx = x as f32 + 0.5 - center.x;
            let dy = y as f32 + 0.5 - center.y;
            let position = (dx.hypot(dy) / radius).clamp(0.0, 1.0);
            paint_gradient_pixel(target, x, y, gradient.sample(position));
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
            paint_gradient_pixel(target, x, y, gradient.sample(position));
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
    for y in top..bottom {
        for x in left..right {
            let local = inverse.transform_point(crate::Vector::new(x as f32 + 0.5, y as f32 + 0.5));
            if !rect_contains_point(rect, local) {
                continue;
            }
            visible = true;
            let position =
                ((local.x - center.x).hypot(local.y - center.y) / radius).clamp(0.0, 1.0);
            paint_gradient_pixel(target, x, y, gradient.sample(position));
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

/// Fills a rounded rectangle; radius is clamped to half its smaller dimension.
pub fn fill_rounded_rect(target: &mut Framebuffer, rect: Rect, radius: u32, color: Color) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let radius = radius.min(rect.width / 2).min(rect.height / 2);
    if radius == 0 {
        fill_rect(target, rect, color);
        return;
    }
    fill_rect(
        target,
        Rect::new(
            rect.x.saturating_add(radius),
            rect.y,
            rect.width - radius * 2,
            rect.height,
        ),
        color,
    );
    fill_rect(
        target,
        Rect::new(
            rect.x,
            rect.y.saturating_add(radius),
            rect.width,
            rect.height - radius * 2,
        ),
        color,
    );

    let Some(lx) = rect.x.checked_add(radius) else {
        return;
    };
    let Some(rx) = rect.x.checked_add(rect.width - radius - 1) else {
        return;
    };
    let Some(ty) = rect.y.checked_add(radius) else {
        return;
    };
    let Some(by) = rect.y.checked_add(rect.height - radius - 1) else {
        return;
    };
    let (Ok(lx), Ok(rx), Ok(ty), Ok(by)) = (
        i32::try_from(lx),
        i32::try_from(rx),
        i32::try_from(ty),
        i32::try_from(by),
    ) else {
        return;
    };
    for center in [
        Point::new(lx, ty),
        Point::new(rx, ty),
        Point::new(lx, by),
        Point::new(rx, by),
    ] {
        fill_circle_clipped(target, center, radius, rect, color);
    }
}

fn fill_circle_clipped(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    clip: Rect,
    color: Color,
) {
    if target.width() == 0 || target.height() == 0 {
        return;
    }
    let radius = i64::from(radius);
    let cx = i64::from(center.x);
    let cy = i64::from(center.y);
    let left = i64::from(clip.x).max(0);
    let top = i64::from(clip.y).max(0);
    let right = (i64::from(clip.x) + i64::from(clip.width) - 1).min(i64::from(target.width()) - 1);
    let bottom =
        (i64::from(clip.y) + i64::from(clip.height) - 1).min(i64::from(target.height()) - 1);
    let first_y = (cy - radius).max(top);
    let last_y = (cy + radius).min(bottom);
    if left > right || first_y > last_y {
        return;
    }

    let radius_squared = (radius as u128) * (radius as u128);
    for y in first_y..=last_y {
        let dy = (y - cy).unsigned_abs() as u128;
        let extent = integer_sqrt(radius_squared - dy * dy) as i64;
        let first_x = (cx - extent).max(left);
        let last_x = (cx + extent).min(right);
        for x in first_x..=last_x {
            target.set_pixel(x as u32, y as u32, color);
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
    let mut intersections = Vec::with_capacity(vertices.len());

    for y in min_y..=max_y {
        intersections.clear();
        let scan_y = f64::from(y) + 0.5;
        for index in 0..vertices.len() {
            let a = vertices[index];
            let b = vertices[(index + 1) % vertices.len()];
            let ay = f64::from(a.y);
            let by = f64::from(b.y);
            if (ay <= scan_y && scan_y < by) || (by <= scan_y && scan_y < ay) {
                let t = (scan_y - ay) / (by - ay);
                intersections.push(f64::from(a.x) + t * (f64::from(b.x) - f64::from(a.x)));
            }
        }
        intersections.sort_by(f64::total_cmp);
        for pair in intersections.chunks_exact(2) {
            let first = (pair[0] - 0.5).ceil().max(0.0) as u32;
            let end = ((pair[1] - 0.5).ceil().max(0.0) as u32).min(target.width());
            for x in first.min(target.width())..end {
                target.set_pixel(x, y, color);
            }
        }
    }
}

/// Fills a polygon with 4x4 supersampled edge coverage and source-over blending.
pub fn fill_polygon_aa(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    if vertices.len() < 3 || color.a == 0 || target.width() == 0 || target.height() == 0 {
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

    for y in top..bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            let mut covered = 0_u16;
            for offset_y in OFFSETS {
                for offset_x in OFFSETS {
                    let sample_x = f64::from(x) + offset_x;
                    let sample_y = f64::from(y) + offset_y;
                    let mut inside = false;
                    for index in 0..vertices.len() {
                        let a = vertices[index];
                        let b = vertices[(index + 1) % vertices.len()];
                        let ay = f64::from(a.y);
                        let by = f64::from(b.y);
                        if (ay > sample_y) != (by > sample_y) {
                            let crossing_x = f64::from(a.x)
                                + (sample_y - ay) * (f64::from(b.x) - f64::from(a.x)) / (by - ay);
                            if sample_x < crossing_x {
                                inside = !inside;
                            }
                        }
                    }
                    covered += u16::from(inside);
                }
            }
            if covered == 0 {
                continue;
            }
            let alpha = (u16::from(color.a) * covered / 16) as u8;
            if alpha == 0 {
                continue;
            }
            let source = Color::rgba(color.r, color.g, color.b, alpha);
            let index = x as usize;
            row[index] = source.blend_over(row[index]);
        }
    }
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

    for y in min_y..=max_y {
        let py = i128::from(y) * 2 + 1;
        for x in min_x..=max_x {
            let px = i128::from(x) * 2 + 1;
            let e0 = edge_doubled(ax, ay, bx, by, px, py);
            let e1 = edge_doubled(bx, by, cx, cy, px, py);
            let e2 = edge_doubled(cx, cy, ax, ay, px, py);
            if (e0 > 0 || (e0 == 0 && tl_ab))
                && (e1 > 0 || (e1 == 0 && tl_bc))
                && (e2 > 0 || (e2 == 0 && tl_ca))
            {
                target.set_pixel(x, y, color);
            }
        }
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

    const SAMPLES: [(f64, f64); 16] = [
        (0.125, 0.125),
        (0.375, 0.125),
        (0.625, 0.125),
        (0.875, 0.125),
        (0.125, 0.375),
        (0.375, 0.375),
        (0.625, 0.375),
        (0.875, 0.375),
        (0.125, 0.625),
        (0.375, 0.625),
        (0.625, 0.625),
        (0.875, 0.625),
        (0.125, 0.875),
        (0.375, 0.875),
        (0.625, 0.875),
        (0.875, 0.875),
    ];
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

    for y in top..bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in left..right {
            let covered = SAMPLES
                .iter()
                .filter(|&&(offset_x, offset_y)| {
                    let dx = (f64::from(x) + offset_x - cx) / rx;
                    let dy = (f64::from(y) + offset_y - cy) / ry;
                    dx * dx + dy * dy <= 1.0
                })
                .count();
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
            target.set_pixel(left as u32, y, color);
            target.set_pixel(right as u32, y, color);
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
            for x in left..=right {
                target.set_pixel(x as u32, y, color);
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
        blend_pixel, draw_circle, draw_ellipse, draw_line, draw_line_aa, draw_pixel, draw_polygon,
        draw_rect, fill_circle, fill_circle_aa, fill_ellipse, fill_ellipse_aa, fill_polygon,
        fill_polygon_aa, fill_rect, fill_rounded_rect, fill_triangle,
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
        assert!(pixels.iter().any(|pixel| pixel.a > 0 && pixel.a < 255));
        assert!(pixels.iter().any(|pixel| *pixel == Color::WHITE));
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
