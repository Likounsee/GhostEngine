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
        for x in left..right {
            target.set_pixel(x as u32, y as u32, color);
        }
    }
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
        fill_circle(target, center, radius, color);
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
        blend_pixel, draw_circle, draw_ellipse, draw_line, draw_pixel, draw_polygon, draw_rect,
        fill_circle, fill_ellipse, fill_polygon, fill_rect, fill_rounded_rect, fill_triangle,
    };
    use crate::{Color, Framebuffer, Point, Rect};

    fn target(width: u32, height: u32) -> Framebuffer {
        Framebuffer::new(width, height, Color::BLACK).unwrap()
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
        assert_eq!(image.get_pixel(6, 6), Some(Color::GREEN));
        assert_eq!(image.get_pixel(7, 6), Some(Color::BLACK));
    }

    #[test]
    fn rounded_rectangle_radius_is_clamped_and_zero_is_plain_rect() {
        let mut rounded = target(4, 4);
        fill_rounded_rect(&mut rounded, Rect::new(0, 0, 4, 2), u32::MAX, Color::BLUE);
        assert_eq!(rounded.get_pixel(0, 0), Some(Color::BLACK));
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
        assert_eq!(image.get_pixel(1, 2), Some(Color::WHITE));
        fill_polygon(
            &mut image,
            &[Point::new(0, 0), Point::new(3, 3)],
            Color::RED,
        );
        assert_eq!(image.get_pixel(0, 0), Some(Color::WHITE));
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
