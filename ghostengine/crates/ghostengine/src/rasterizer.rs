//! Deterministic CPU rasterization primitives.
//!
//! Lines are clipped before rasterization; rectangles use half-open bounds;
//! triangles sample pixel centers and use a top-left edge rule.

use crate::{
    simd::{blend_opaque_quad, blend_variable_opaque_quad},
    Color, Framebuffer, Point, Rect,
};

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
        let width = target.width() as usize;
        let pixels = target.pixels_mut();
        for y in y0.min(y1)..=y0.max(y1) {
            let index = y as usize * width + x;
            pixels[index] = color;
        }
        return;
    }

    // Exact 45-degree lines have one pixel per step and need no error
    // accumulator. Write directly into the packed framebuffer in either slope.
    if (x1 - x0).abs() == (y1 - y0).abs() {
        if x0 > x1 {
            core::mem::swap(&mut x0, &mut x1);
            core::mem::swap(&mut y0, &mut y1);
        }
        let step_y = if y0 <= y1 { 1 } else { -1 };
        let width = target.width() as usize;
        let pixels = target.pixels_mut();
        for offset in 0..=(x1 - x0) {
            let x = (x0 + offset) as usize;
            let y = (y0 + offset * step_y) as usize;
            pixels[y * width + x] = color;
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
    let width = target.width() as usize;
    let pixels = target.pixels_mut();

    // Clipping guarantees every Bresenham sample lies inside the framebuffer.
    // Borrow the packed storage once instead of repeating set_pixel's bounds
    // checks and index calculation for every point on the segment.
    loop {
        let index = y0 as usize * width + x0 as usize;
        pixels[index] = color;
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
    let max_x = i64::from(width.saturating_sub(1).min(i32::MAX as u32));
    let max_y = i64::from(height.saturating_sub(1).min(i32::MAX as u32));

    // Axis-aligned segments can be clipped exactly with integer comparisons.
    // This avoids setting up the floating-point Liang-Barsky calculation for
    // the most common line shapes while preserving the caller's endpoint order.
    if start.y == end.y {
        let y = i64::from(start.y);
        if y < 0 || y > max_y {
            return None;
        }
        let (x0, x1) = (i64::from(start.x), i64::from(end.x));
        if x0.max(x1) < 0 || x0.min(x1) > max_x {
            return None;
        }
        return Some((x0.clamp(0, max_x), y, x1.clamp(0, max_x), y));
    }
    if start.x == end.x {
        let x = i64::from(start.x);
        if x < 0 || x > max_x {
            return None;
        }
        let (y0, y1) = (i64::from(start.y), i64::from(end.y));
        if y0.max(y1) < 0 || y0.min(y1) > max_y {
            return None;
        }
        return Some((x, y0.clamp(0, max_y), x, y1.clamp(0, max_y)));
    }

    let xmax = max_x as f64;
    let ymax = max_y as f64;
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

    // Integer-aligned horizontal strokes have full coverage for every
    // source alpha. Composite the contiguous row in four-pixel SIMD batches.
    if y0 == y1 {
        if color.a == u8::MAX {
            draw_line(target, start, end, color);
            return;
        }
        if color.a == 0 {
            return;
        }
        let left = x0.min(x1) as usize;
        let right = x0.max(x1) as usize;
        if let Some(row) = target.row_mut(y0 as u32) {
            let pixels = &mut row[left..=right];
            let source_alpha = u32::from(color.a);
            let inverse_source_alpha = 255 - source_alpha;
            let mut index = 0;
            while index < pixels.len() {
                if pixels[index].a == 0 {
                    let run_start = index;
                    while index < pixels.len() && pixels[index].a == 0 {
                        index += 1;
                    }
                    pixels[run_start..index].fill(color);
                    continue;
                }
                if pixels[index].a != u8::MAX {
                    pixels[index] = color.blend_over(pixels[index]);
                    index += 1;
                    continue;
                }

                // SIMD batches are formed from complete opaque runs, so
                // isolated translucent pixels do not disable vectorization
                // for neighboring opaque destinations.
                let run_start = index;
                while index < pixels.len() && pixels[index].a == u8::MAX {
                    index += 1;
                }
                let mut quads = pixels[run_start..index].chunks_exact_mut(4);
                for quad in &mut quads {
                    blend_opaque_quad(
                        quad.try_into().expect("four destination pixels"),
                        color,
                        source_alpha,
                        inverse_source_alpha,
                    );
                }
                for pixel in quads.into_remainder() {
                    *pixel = color.blend_over(*pixel);
                }
            }
        }
        return;
    }

    // Integer-aligned vertical strokes have full coverage for every
    // source alpha, just like horizontal strokes. Avoid the general Wu path
    // (floating-point stepping and two coverage plots) for this common case.
    if x0 == x1 {
        if color.a == 0 {
            return;
        }
        let x = x0 as usize;
        let width = target.width() as usize;
        let pixels = target.pixels_mut();
        for y in y0.min(y1)..=y0.max(y1) {
            let index = y as usize * width + x;
            let pixel = &mut pixels[index];
            *pixel = if color.a == u8::MAX || pixel.a == 0 {
                color
            } else {
                color.blend_over(*pixel)
            };
        }
        return;
    }

    // Exact 45-degree segments land on pixel centers throughout their
    // length. Wu's two-sample coverage loop therefore collapses to one
    // fully covered pixel per step; draw those pixels directly.
    if (x1 - x0).abs() == (y1 - y0).abs() {
        if color.a == 0 {
            return;
        }
        if x0 > x1 {
            core::mem::swap(&mut x0, &mut x1);
            core::mem::swap(&mut y0, &mut y1);
        }
        let step_y = if y0 <= y1 { 1 } else { -1 };
        let width = target.width() as usize;
        let pixels = target.pixels_mut();
        for offset in 0..=(x1 - x0) {
            let x = (x0 + offset) as usize;
            let y = (y0 + offset * step_y) as usize;
            let index = y * width + x;
            let pixel = &mut pixels[index];
            *pixel = if color.a == u8::MAX || pixel.a == 0 {
                color
            } else {
                color.blend_over(*pixel)
            };
        }
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

    let width = target.width() as usize;
    let height = target.height() as usize;
    let pixels = target.pixels_mut();
    let gradient = (y1 - y0) as f64 / (x1 - x0) as f64;
    let mut y = y0 as f64;

    // The general Wu path touches two pixels for each major-axis step.
    // Borrow the packed pixel buffer once and calculate flat indices directly,
    // avoiding repeated row-slice setup and bounds checks through row_mut().
    let mut plot = |px: i64, py: i64, coverage: f64| {
        let (px, py) = if steep { (py, px) } else { (px, py) };
        if px < 0 || py < 0 || px >= width as i64 || py >= height as i64 {
            return;
        }
        let alpha = (f64::from(color.a) * coverage.clamp(0.0, 1.0)).round() as u8;
        if alpha == 0 {
            return;
        }
        let index = py as usize * width + px as usize;
        let Some(pixel) = pixels.get_mut(index) else {
            return;
        };
        let source = Color::rgba(color.r, color.g, color.b, alpha);
        *pixel = if alpha == u8::MAX {
            source
        } else {
            source.blend_over(*pixel)
        };
    };

    for x in x0..=x1 {
        let lower = y.floor();
        let upper_coverage = y - lower;
        let lower_coverage = 1.0 - upper_coverage;
        plot(x, lower as i64, lower_coverage);
        plot(x, lower as i64 + 1, upper_coverage);
        y += gradient;
    }
}

/// Draws an antialiased line while clipping its coverage to a framebuffer region.
///
/// Keeping the original line equation preserves Wu coverage at clip boundaries;
/// clipping rounded endpoints first can turn smooth edges into hard pixels.
pub fn draw_line_aa_clipped(
    target: &mut Framebuffer,
    start: Point,
    end: Point,
    clip: Rect,
    color: Color,
) -> bool {
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return false;
    };
    if color.a == 0 {
        return true;
    }

    let (mut x0, mut y0, mut x1, mut y1) = (
        i64::from(start.x),
        i64::from(start.y),
        i64::from(end.x),
        i64::from(end.y),
    );
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
        let (px, py) = if steep { (y0, x0) } else { (x0, y0) };
        if px < i64::from(clip.x)
            || py < i64::from(clip.y)
            || px >= i64::from(clip.x) + i64::from(clip.width)
            || py >= i64::from(clip.y) + i64::from(clip.height)
        {
            return false;
        }
        let index = py as usize * target.width() as usize + px as usize;
        if let Some(pixel) = target.pixels_mut().get_mut(index) {
            *pixel = if color.a == u8::MAX || pixel.a == 0 {
                color
            } else {
                color.blend_over(*pixel)
            };
        }
        return true;
    }

    let (major_min, major_max, minor_min, minor_max) = if steep {
        (
            i64::from(clip.y),
            i64::from(clip.y) + i64::from(clip.height) - 1,
            i64::from(clip.x),
            i64::from(clip.x) + i64::from(clip.width) - 1,
        )
    } else {
        (
            i64::from(clip.x),
            i64::from(clip.x) + i64::from(clip.width) - 1,
            i64::from(clip.y),
            i64::from(clip.y) + i64::from(clip.height) - 1,
        )
    };
    let first_x = x0.max(major_min);
    let last_x = x1.min(major_max);
    if first_x > last_x {
        return false;
    }

    let target_width = target.width() as usize;
    let target_height = target.height() as i64;
    let pixels = target.pixels_mut();
    let gradient = (y1 - y0) as f64 / (x1 - x0) as f64;
    let mut y = y0 as f64 + gradient * (first_x - x0) as f64;
    let mut plot = |major: i64, minor: i64, coverage: f64| {
        if minor < minor_min || minor > minor_max {
            return;
        }
        let (px, py) = if steep {
            (minor, major)
        } else {
            (major, minor)
        };
        if px < 0 || py < 0 || px >= target_width as i64 || py >= target_height {
            return;
        }
        let alpha = (f64::from(color.a) * coverage.clamp(0.0, 1.0)).round() as u8;
        if alpha == 0 {
            return;
        }
        let index = py as usize * target_width + px as usize;
        if let Some(pixel) = pixels.get_mut(index) {
            let source = Color::rgba(color.r, color.g, color.b, alpha);
            *pixel = if alpha == u8::MAX {
                source
            } else {
                source.blend_over(*pixel)
            };
        }
    };

    for x in first_x..=last_x {
        let lower = y.floor();
        let upper_coverage = y - lower;
        plot(x, lower as i64, 1.0 - upper_coverage);
        plot(x, lower as i64 + 1, upper_coverage);
        y += gradient;
    }
    true
}

/// Fills a batch of same-color rectangles under one shared clip.
///
/// Use this for repeated UI or sprite primitives that share a color and clip.
/// Rectangle coverage is preserved exactly, including overlapping shapes.
pub fn fill_rects_clipped(target: &mut Framebuffer, rectangles: &[Rect], color: Color, clip: Rect) {
    let Some(clip) = clip.intersection(Rect::new(0, 0, target.width(), target.height())) else {
        return;
    };
    for rectangle in rectangles {
        if let Some(rectangle) = rectangle.intersection(clip) {
            fill_rect(target, rectangle, color);
        }
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

fn process_rows_mut<F>(rows: &mut [Color], row_width: usize, thread_count: usize, process: F)
where
    F: Fn(&mut [Color]) + Sync,
{
    let row_count = rows.len() / row_width;
    let worker_count = thread_count.max(1).min(row_count);
    if worker_count <= 1 {
        process(rows);
        return;
    }

    // A small number of row bands amortizes scoped-thread scheduling while
    // preserving contiguous writes for the CPU cache and LLVM's optimizers.
    let rows_per_worker = row_count.div_ceil(worker_count);
    let pixels_per_worker = rows_per_worker * row_width;
    std::thread::scope(|scope| {
        for chunk in rows.chunks_mut(pixels_per_worker) {
            let process = &process;
            scope.spawn(move || process(chunk));
        }
    });
}

fn clipped_rect_bounds(target: &Framebuffer, rect: Rect) -> Option<(usize, usize, usize, usize)> {
    let left = u64::from(rect.x).min(u64::from(target.width()));
    let top = u64::from(rect.y).min(u64::from(target.height()));
    let right = (u64::from(rect.x) + u64::from(rect.width)).min(u64::from(target.width()));
    let bottom = (u64::from(rect.y) + u64::from(rect.height)).min(u64::from(target.height()));
    (left < right && top < bottom).then_some((
        left as usize,
        top as usize,
        right as usize,
        bottom as usize,
    ))
}

/// Processes disjoint rows while preserving each row's absolute Y coordinate.
fn process_rows_with_y_mut<F>(
    rows: &mut [Color],
    row_width: usize,
    first_y: usize,
    thread_count: usize,
    process: F,
) where
    F: Fn(&mut [Color], usize) + Sync,
{
    let row_count = rows.len() / row_width;
    let worker_count = thread_count.max(1).min(row_count);
    if worker_count <= 1 {
        for (offset, row) in rows.chunks_mut(row_width).enumerate() {
            process(row, first_y + offset);
        }
        return;
    }

    let rows_per_worker = row_count.div_ceil(worker_count);
    let pixels_per_worker = rows_per_worker * row_width;
    std::thread::scope(|scope| {
        for (chunk_index, chunk) in rows.chunks_mut(pixels_per_worker).enumerate() {
            let process = &process;
            let chunk_first_y = first_y + chunk_index * rows_per_worker;
            scope.spawn(move || {
                for (offset, row) in chunk.chunks_mut(row_width).enumerate() {
                    process(row, chunk_first_y + offset);
                }
            });
        }
    });
}

#[inline]
fn automatic_worker_count() -> usize {
    #[cfg(feature = "parallel-rendering")]
    {
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
    }
    #[cfg(not(feature = "parallel-rendering"))]
    {
        1
    }
}

fn parallel_thread_count_for_rect(target: &Framebuffer, rect: Rect) -> usize {
    let area = clipped_rect_bounds(target, rect)
        .map(|(left, top, right, bottom)| (right - left) as u64 * (bottom - top) as u64)
        .unwrap_or(0);
    if area < 16_384 {
        return 1;
    }
    automatic_worker_count()
}

/// Fills a clipped rectangle using the requested number of safe workers.
///
/// A thread count of zero or one selects the serial path. Rows are partitioned
/// into exclusive mutable slices, so no locks, atomics, or unsafe code are used.
pub fn fill_rect_parallel_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    color: Color,
    thread_count: usize,
) {
    let Some((left, top, right, bottom)) = clipped_rect_bounds(target, rect) else {
        return;
    };
    let width = target.width() as usize;
    let start = top * width;
    let end = bottom * width;
    let rows = &mut target.pixels_mut()[start..end];
    process_rows_mut(rows, width, thread_count, |band| {
        for row in band.chunks_mut(width) {
            row[left..right].fill(color);
        }
    });
}

/// Alpha-composites a clipped rectangle over the framebuffer.
///
/// Fully opaque colors use the fast row-fill path; transparent colors are a no-op.
pub fn blend_rect(target: &mut Framebuffer, rect: Rect, color: Color) {
    blend_rect_with_threads(target, rect, color, 1);
}

/// Alpha-composites a clipped rectangle using safe, disjoint row bands.
///
/// This parallel variant is deterministic and pixel-identical to blend_rect.
/// It uses the same exact integer rounding and opaque-destination lookup path.
pub fn blend_rect_parallel(target: &mut Framebuffer, rect: Rect, color: Color) {
    blend_rect_with_threads(
        target,
        rect,
        color,
        parallel_thread_count_for_rect(target, rect),
    );
}

/// Alpha-composites a clipped rectangle using the requested number of workers.
///
/// A thread count of zero or one selects the serial path. The source-over
/// arithmetic and rounding are identical to blend_rect.
pub fn blend_rect_parallel_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    color: Color,
    thread_count: usize,
) {
    blend_rect_with_threads(target, rect, color, thread_count);
}

fn blend_rect_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    color: Color,
    thread_count: usize,
) {
    if color.a == 0 {
        return;
    }
    if color.a == 255 {
        fill_rect(target, rect, color);
        return;
    }
    let Some((left, top, right, bottom)) = clipped_rect_bounds(target, rect) else {
        return;
    };
    let width = target.width() as usize;
    let start = top * width;
    let end = bottom * width;
    let area = (right - left) as u64 * (bottom - top) as u64;
    let rows = &mut target.pixels_mut()[start..end];

    // Tiny rectangles are cheaper with the general scalar blend and do not
    // justify constructing the opaque-channel lookup tables.
    const LOOKUP_TABLE_MIN_PIXELS: u64 = 256;
    if area < LOOKUP_TABLE_MIN_PIXELS {
        process_rows_mut(rows, width, thread_count, |band| {
            for row in band.chunks_mut(width) {
                for pixel in &mut row[left..right] {
                    *pixel = color.blend_over(*pixel);
                }
            }
        });
        return;
    }

    let source_alpha = u32::from(color.a);
    let inverse_source_alpha = 255 - source_alpha;
    let source_r = u32::from(color.r) * source_alpha * 255;
    let source_g = u32::from(color.g) * source_alpha * 255;
    let source_b = u32::from(color.b) * source_alpha * 255;

    // Precompute exact rounded results for opaque destinations once. The
    // tables are immutable and safely shared by every worker.
    let opaque_channel = |source: u8| {
        let mut table = [0_u8; 256];
        for (destination, output) in table.iter_mut().enumerate() {
            *output = ((u32::from(source) * source_alpha
                + destination as u32 * inverse_source_alpha
                + 127)
                / 255) as u8;
        }
        table
    };
    let opaque_r = opaque_channel(color.r);
    let opaque_g = opaque_channel(color.g);
    let opaque_b = opaque_channel(color.b);
    let context = BlendRectPixelContext {
        color,
        source_alpha,
        inverse_source_alpha,
        source_r,
        source_g,
        source_b,
        opaque_r: &opaque_r,
        opaque_g: &opaque_g,
        opaque_b: &opaque_b,
    };

    process_rows_mut(rows, width, thread_count, |band| {
        for row in band.chunks_mut(width) {
            let pixels = &mut row[left..right];
            let mut index = 0;
            while index < pixels.len() {
                if pixels[index].a == 0 {
                    // Transparent destinations need no channel arithmetic:
                    // source-over is exactly the source color. Fill the whole
                    // contiguous run instead of dispatching one pixel at a time.
                    let run_start = index;
                    while index < pixels.len() && pixels[index].a == 0 {
                        index += 1;
                    }
                    pixels[run_start..index].fill(color);
                    continue;
                }

                if pixels[index].a != 255 {
                    blend_rect_pixel(&mut pixels[index], &context);
                    index += 1;
                    continue;
                }

                // Keep SIMD batches aligned to opaque runs, not arbitrary
                // groups of four. A single translucent destination therefore
                // no longer forces its three opaque neighbors through scalar
                // compositing.
                let run_start = index;
                while index < pixels.len() && pixels[index].a == 255 {
                    index += 1;
                }
                let run = &mut pixels[run_start..index];
                let mut quads = run.chunks_exact_mut(4);
                for quad in &mut quads {
                    let quad: &mut [Color; 4] = quad
                        .try_into()
                        .expect("chunks_exact_mut always yields four pixels");
                    blend_opaque_quad(quad, color, source_alpha, inverse_source_alpha);
                }
                for pixel in quads.into_remainder() {
                    blend_rect_pixel(pixel, &context);
                }
            }
        }
    });
}

struct BlendRectPixelContext<'a> {
    color: Color,
    source_alpha: u32,
    inverse_source_alpha: u32,
    source_r: u32,
    source_g: u32,
    source_b: u32,
    opaque_r: &'a [u8; 256],
    opaque_g: &'a [u8; 256],
    opaque_b: &'a [u8; 256],
}

#[inline]
fn blend_rect_pixel(pixel: &mut Color, context: &BlendRectPixelContext<'_>) {
    if pixel.a == 0 {
        *pixel = context.color;
    } else if pixel.a == 255 {
        *pixel = Color::rgb(
            context.opaque_r[pixel.r as usize],
            context.opaque_g[pixel.g as usize],
            context.opaque_b[pixel.b as usize],
        );
    } else {
        let destination_alpha = u32::from(pixel.a);
        let destination_weight = destination_alpha * context.inverse_source_alpha;
        let alpha_numerator = context.source_alpha * 255 + destination_weight;
        let blend_channel = |source: u32, destination: u8| -> u8 {
            let numerator = source + u32::from(destination) * destination_weight;
            ((numerator + alpha_numerator / 2) / alpha_numerator) as u8
        };
        *pixel = Color::rgba(
            blend_channel(context.source_r, pixel.r),
            blend_channel(context.source_g, pixel.g),
            blend_channel(context.source_b, pixel.b),
            ((alpha_numerator + 127) / 255) as u8,
        );
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
    // Fallibly allocated lookup tables accelerate per-pixel raster sampling.
    // They are only used when stops are strictly increasing, so hard edges remain exact.
    lookup: Option<Vec<Color>>,
    // Radial gradients can index directly with normalized squared distance,
    // avoiding a square root for every destination pixel.
    radial_lookup: Option<Vec<Color>>,
}

/// Errors produced by fallible rasterizer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterizerError {
    AllocationFailed,
}

/// Errors produced while constructing a gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientError {
    TooFewStops,
    InvalidPosition,
    AllocationFailed,
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
            let mut lookup = Vec::new();
            lookup
                .try_reserve_exact(LOOKUP_SIZE)
                .map_err(|_| GradientError::AllocationFailed)?;
            for index in 0..LOOKUP_SIZE {
                let position = index as f32 / (LOOKUP_SIZE - 1) as f32;
                lookup.push(sample_gradient_stops(&stops, position));
            }
            Some(lookup)
        } else {
            None
        };
        let radial_lookup = if lookup.is_some() {
            // Uniform squared-distance steps need a denser table near the
            // center, where sqrt changes fastest. 65,536 entries keep the
            // resulting color error within approximately one 8-bit channel.
            const RADIAL_LOOKUP_SIZE: usize = 65_536;
            let mut radial_lookup = Vec::new();
            radial_lookup
                .try_reserve_exact(RADIAL_LOOKUP_SIZE)
                .map_err(|_| GradientError::AllocationFailed)?;
            for index in 0..RADIAL_LOOKUP_SIZE {
                let squared_position = index as f32 / (RADIAL_LOOKUP_SIZE - 1) as f32;
                radial_lookup.push(sample_gradient_stops(&stops, squared_position.sqrt()));
            }
            Some(radial_lookup)
        } else {
            None
        };
        Ok(Self {
            stops,
            lookup,
            radial_lookup,
        })
    }

    /// Samples the gradient, clamping positions outside its stop range.
    #[must_use]
    pub fn sample(&self, position: f32) -> Color {
        sample_gradient_stops(&self.stops, position)
    }

    /// Samples a finite, non-negative squared distance from the radial
    /// rasterizer. Its caller has already rejected non-finite values and
    /// bounded the value to the radius (with a tiny edge tolerance), so the
    /// hot path only needs the final table-index bound.
    #[inline]
    fn sample_radial_squared_in_range(&self, squared_position: f32) -> Color {
        if let Some(lookup) = &self.radial_lookup {
            let index = (squared_position * (lookup.len() - 1) as f32 + 0.5) as usize;
            lookup[index.min(lookup.len() - 1)]
        } else {
            self.sample_raster(squared_position.sqrt())
        }
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
    fill_rect_linear_gradient_with_threads(target, rect, start, end, gradient, 1);
}

/// Fills a clipped linear gradient using safe parallel row bands.
///
/// Small rectangles remain serial to avoid thread scheduling overhead. Sampling
/// and alpha rounding are identical to fill_rect_linear_gradient.
pub fn fill_rect_linear_gradient_parallel(
    target: &mut Framebuffer,
    rect: Rect,
    start: crate::Vector,
    end: crate::Vector,
    gradient: &Gradient,
) {
    let thread_count = parallel_thread_count_for_rect(target, rect);
    fill_rect_linear_gradient_with_threads(target, rect, start, end, gradient, thread_count);
}

/// Fills a clipped linear gradient using the requested number of safe workers.
pub fn fill_rect_linear_gradient_parallel_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    start: crate::Vector,
    end: crate::Vector,
    gradient: &Gradient,
    thread_count: usize,
) {
    fill_rect_linear_gradient_with_threads(target, rect, start, end, gradient, thread_count);
}

fn fill_rect_linear_gradient_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    start: crate::Vector,
    end: crate::Vector,
    gradient: &Gradient,
    thread_count: usize,
) {
    if !start.x.is_finite() || !start.y.is_finite() || !end.x.is_finite() || !end.y.is_finite() {
        return;
    }
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let length_squared = dx * dx + dy * dy;
    let left = rect.x.min(target.width()) as usize;
    let top = rect.y.min(target.height()) as usize;
    let right = (u64::from(rect.x) + u64::from(rect.width)).min(u64::from(target.width())) as usize;
    let bottom =
        (u64::from(rect.y) + u64::from(rect.height)).min(u64::from(target.height())) as usize;
    if left >= right || top >= bottom {
        return;
    }
    let inverse_length_squared = if length_squared <= f32::EPSILON {
        0.0
    } else {
        length_squared.recip()
    };
    let horizontal_step = dx * inverse_length_squared;
    let width = target.width() as usize;
    let start_index = top * width;
    let end_index = bottom * width;
    let rows = &mut target.pixels_mut()[start_index..end_index];

    process_rows_with_y_mut(rows, width, top, thread_count, |row, y| {
        let row_position = if inverse_length_squared == 0.0 {
            0.0
        } else {
            ((left as f32 + 0.5 - start.x) * dx + (y as f32 + 0.5 - start.y) * dy)
                * inverse_length_squared
        };
        let pixels = &mut row[left..right];
        let vectorized_len = pixels.len() / 4 * 4;
        let (quads, remainder) = pixels.split_at_mut(vectorized_len);
        let mut position = row_position;

        for quad in quads.chunks_exact_mut(4) {
            let mut sources = [Color::TRANSPARENT; 4];
            for source in &mut sources {
                *source = gradient.sample_raster(position);
                position += horizontal_step;
            }
            if quad.iter().all(|pixel| pixel.a == u8::MAX) {
                let mut destinations = [quad[0], quad[1], quad[2], quad[3]];
                blend_variable_opaque_quad(&mut destinations, sources);
                quad.copy_from_slice(&destinations);
            } else {
                for (pixel, color) in quad.iter_mut().zip(sources) {
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

        for pixel in remainder {
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
    });
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
    fill_rect_radial_gradient_with_threads(target, rect, center, radius, gradient, 1);
}

/// Fills a clipped radial gradient using safe parallel row bands.
///
/// Small rectangles remain serial to avoid thread scheduling overhead. The
/// radial lookup and floating-point edge handling match the serial path.
pub fn fill_rect_radial_gradient_parallel(
    target: &mut Framebuffer,
    rect: Rect,
    center: crate::Vector,
    radius: f32,
    gradient: &Gradient,
) {
    let thread_count = parallel_thread_count_for_rect(target, rect);
    fill_rect_radial_gradient_with_threads(target, rect, center, radius, gradient, thread_count);
}

/// Fills a clipped radial gradient using the requested number of safe workers.
pub fn fill_rect_radial_gradient_parallel_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    center: crate::Vector,
    radius: f32,
    gradient: &Gradient,
    thread_count: usize,
) {
    fill_rect_radial_gradient_with_threads(target, rect, center, radius, gradient, thread_count);
}

fn fill_rect_radial_gradient_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    center: crate::Vector,
    radius: f32,
    gradient: &Gradient,
    thread_count: usize,
) {
    if !center.x.is_finite() || !center.y.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return;
    }
    let left = rect.x.min(target.width()) as usize;
    let top = rect.y.min(target.height()) as usize;
    let right = (u64::from(rect.x) + u64::from(rect.width)).min(u64::from(target.width())) as usize;
    let bottom =
        (u64::from(rect.y) + u64::from(rect.height)).min(u64::from(target.height())) as usize;
    if left >= right || top >= bottom {
        return;
    }
    let inverse_radius = radius.recip();
    let radius_squared = radius * radius;
    let inverse_radius_squared = radius_squared.recip();
    // Keep a small floating-point safety margin so samples at the radius
    // resolve to the final stop instead of a rounded inner lookup entry.
    let outside_radius_squared = radius_squared * (1.0 + 4.0 * f32::EPSILON);
    let outside_color = gradient.sample_raster(1.0);
    let width = target.width() as usize;
    let start_index = top * width;
    let end_index = bottom * width;
    let rows = &mut target.pixels_mut()[start_index..end_index];

    process_rows_with_y_mut(rows, width, top, thread_count, |row, y| {
        let dy = y as f32 + 0.5 - center.y;
        let dy_squared = dy * dy;
        let mut dx = left as f32 + 0.5 - center.x;
        let pixels = &mut row[left..right];
        let vectorized_len = pixels.len() / 4 * 4;
        let (quads, remainder) = pixels.split_at_mut(vectorized_len);

        for quad in quads.chunks_exact_mut(4) {
            let mut sources = [Color::TRANSPARENT; 4];
            for source in &mut sources {
                let distance_squared = dx * dx + dy_squared;
                *source = if radius_squared.is_finite() && distance_squared.is_finite() {
                    if distance_squared >= outside_radius_squared {
                        outside_color
                    } else {
                        gradient.sample_radial_squared_in_range(
                            distance_squared * inverse_radius_squared,
                        )
                    }
                } else {
                    let distance = dx.hypot(dy);
                    gradient.sample_raster(distance * inverse_radius)
                };
                dx += 1.0;
            }
            if quad.iter().all(|pixel| pixel.a == u8::MAX) {
                let mut destinations = [quad[0], quad[1], quad[2], quad[3]];
                blend_variable_opaque_quad(&mut destinations, sources);
                quad.copy_from_slice(&destinations);
            } else {
                for (pixel, color) in quad.iter_mut().zip(sources) {
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

        for pixel in remainder {
            let distance_squared = dx * dx + dy_squared;
            let color = if radius_squared.is_finite() && distance_squared.is_finite() {
                if distance_squared >= outside_radius_squared {
                    outside_color
                } else {
                    gradient
                        .sample_radial_squared_in_range(distance_squared * inverse_radius_squared)
                }
            } else {
                let distance = dx.hypot(dy);
                gradient.sample_raster(distance * inverse_radius)
            };
            *pixel = if color.a == u8::MAX {
                color
            } else if color.a == 0 {
                *pixel
            } else {
                color.blend_over(*pixel)
            };
            dx += 1.0;
        }
    });
}

/// Fills an affine-transformed rectangle with a linear gradient.
/// Gradient coordinates are expressed in the rectangle's local coordinate space.
/// Pixel centers are mapped through the inverse transform for stable sampling.
/// Fills a clipped rectangle with a conic gradient rotating clockwise from `start_angle`.
///
/// Angles are expressed in radians in framebuffer coordinates (positive Y points
/// down). The angle is normalized to one full turn; the gradient's first and last
/// stops meet at the angular seam. Non-finite centers or angles draw nothing.
pub fn fill_rect_conic_gradient(
    target: &mut Framebuffer,
    rect: Rect,
    center: crate::Vector,
    start_angle: f32,
    gradient: &Gradient,
) {
    fill_rect_conic_gradient_with_threads(target, rect, center, start_angle, gradient, 1);
}

/// Fills a conic gradient using automatically selected row workers for large regions.
pub fn fill_rect_conic_gradient_parallel(
    target: &mut Framebuffer,
    rect: Rect,
    center: crate::Vector,
    start_angle: f32,
    gradient: &Gradient,
) {
    let thread_count = parallel_thread_count_for_rect(target, rect);
    fill_rect_conic_gradient_with_threads(target, rect, center, start_angle, gradient, thread_count);
}

/// Fills a conic gradient using the requested number of safe row workers.
pub fn fill_rect_conic_gradient_parallel_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    center: crate::Vector,
    start_angle: f32,
    gradient: &Gradient,
    thread_count: usize,
) {
    fill_rect_conic_gradient_with_threads(target, rect, center, start_angle, gradient, thread_count);
}

fn fill_rect_conic_gradient_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    center: crate::Vector,
    start_angle: f32,
    gradient: &Gradient,
    thread_count: usize,
) {
    if !center.x.is_finite()
        || !center.y.is_finite()
        || !start_angle.is_finite()
    {
        return;
    }
    let left = rect.x.min(target.width()) as usize;
    let top = rect.y.min(target.height()) as usize;
    let right = (u64::from(rect.x) + u64::from(rect.width))
        .min(u64::from(target.width())) as usize;
    let bottom = (u64::from(rect.y) + u64::from(rect.height))
        .min(u64::from(target.height())) as usize;
    if left >= right || top >= bottom {
        return;
    }

    const TAU: f32 = core::f32::consts::TAU;
    let width = target.width() as usize;
    let rows = &mut target.pixels_mut()[top * width..bottom * width];
    process_rows_with_y_mut(rows, width, top, thread_count, |row, y| {
        let dy = y as f32 + 0.5 - center.y;
        let pixels = &mut row[left..right];
        let vectorized_len = pixels.len() / 4 * 4;
        let (quads, remainder) = pixels.split_at_mut(vectorized_len);
        let mut x = left as f32 + 0.5;

        for quad in quads.chunks_exact_mut(4) {
            let mut sources = [Color::TRANSPARENT; 4];
            for source in &mut sources {
                let dx = x - center.x;
                let angle = (dy.atan2(dx) - start_angle).rem_euclid(TAU) / TAU;
                *source = gradient.sample_raster(angle);
                x += 1.0;
            }
            if quad.iter().all(|pixel| pixel.a == u8::MAX) {
                let mut destinations = [quad[0], quad[1], quad[2], quad[3]];
                blend_variable_opaque_quad(&mut destinations, sources);
                quad.copy_from_slice(&destinations);
            } else {
                for (pixel, color) in quad.iter_mut().zip(sources) {
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

        for pixel in remainder {
            let dx = x - center.x;
            let angle = (dy.atan2(dx) - start_angle).rem_euclid(TAU) / TAU;
            let color = gradient.sample_raster(angle);
            *pixel = if color.a == u8::MAX {
                color
            } else if color.a == 0 {
                *pixel
            } else {
                color.blend_over(*pixel)
            };
            x += 1.0;
        }
    });
}

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
        let mut x = left;
        while x + 4 <= right {
            let mut sources = [Color::TRANSPARENT; 4];
            let mut inside = [false; 4];
            for lane in 0_usize..4 {
                let local = inverse.transform_point(crate::Vector::new(
                    (x as usize + lane) as f32 + 0.5,
                    y as f32 + 0.5,
                ));
                inside[lane] = rect_contains_point(rect, local);
                if inside[lane] {
                    visible = true;
                    let position = if length_squared <= f32::EPSILON {
                        0.0
                    } else {
                        (((local.x - start.x) * dx + (local.y - start.y) * dy) / length_squared)
                            .clamp(0.0, 1.0)
                    };
                    sources[lane] = gradient.sample_raster(position);
                }
            }

            let destinations = &mut row[x as usize..x as usize + 4];
            if inside.iter().all(|is_inside| *is_inside)
                && destinations.iter().all(|pixel| pixel.a == u8::MAX)
            {
                let mut quad = [
                    destinations[0],
                    destinations[1],
                    destinations[2],
                    destinations[3],
                ];
                blend_variable_opaque_quad(&mut quad, sources);
                destinations.copy_from_slice(&quad);
            } else {
                for lane in 0_usize..4 {
                    if !inside[lane] {
                        continue;
                    }
                    let color = sources[lane];
                    let pixel = &mut destinations[lane];
                    *pixel = if color.a == u8::MAX {
                        color
                    } else if color.a == 0 {
                        *pixel
                    } else {
                        color.blend_over(*pixel)
                    };
                }
            }
            x += 4;
        }

        for x in x..right {
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
        let mut x = left;
        while x + 4 <= right {
            let mut sources = [Color::TRANSPARENT; 4];
            let mut inside = [false; 4];
            for lane in 0_usize..4 {
                let local = inverse.transform_point(crate::Vector::new(
                    (x as usize + lane) as f32 + 0.5,
                    y as f32 + 0.5,
                ));
                inside[lane] = rect_contains_point(rect, local);
                if inside[lane] {
                    visible = true;
                    let position = (local.x - center.x).hypot(local.y - center.y) * inverse_radius;
                    sources[lane] = gradient.sample_raster(position);
                }
            }

            let destinations = &mut row[x as usize..x as usize + 4];
            if inside.iter().all(|is_inside| *is_inside)
                && destinations.iter().all(|pixel| pixel.a == u8::MAX)
            {
                let mut quad = [
                    destinations[0],
                    destinations[1],
                    destinations[2],
                    destinations[3],
                ];
                blend_variable_opaque_quad(&mut quad, sources);
                destinations.copy_from_slice(&quad);
            } else {
                for lane in 0_usize..4 {
                    if !inside[lane] {
                        continue;
                    }
                    let color = sources[lane];
                    let pixel = &mut destinations[lane];
                    *pixel = if color.a == u8::MAX {
                        color
                    } else if color.a == 0 {
                        *pixel
                    } else {
                        color.blend_over(*pixel)
                    };
                }
            }
            x += 4;
        }

        for x in x..right {
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
    let mut scratch = RasterizerScratch::default();
    draw_rect_aa_with_scratch(target, rect, color, &mut scratch);
}

/// Draws an antialiased rectangle outline using reusable polygon buffers.
pub fn draw_rect_aa_with_scratch(
    target: &mut Framebuffer,
    rect: Rect,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
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
    draw_polygon_aa_with_scratch(
        target,
        &[
            point(left, top),
            point(right, top),
            point(right, bottom),
            point(left, bottom),
        ],
        color,
        scratch,
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

    let coverage_colors: [Color; 17] = core::array::from_fn(|covered| {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        Color::rgba(color.r, color.g, color.b, alpha)
    });

    for y in clip_top..clip_bottom {
        let sample_ys = SAMPLE_OFFSETS.map(|offset| f64::from(y) + offset - center_y);
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        for x in clip_left..clip_right {
            let sample_xs = SAMPLE_OFFSETS.map(|offset| f64::from(x) + offset - center_x);
            let mut covered = 0_usize;
            for py in sample_ys {
                for px in sample_xs {
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
            if covered != 0 {
                let source = coverage_colors[covered];
                if source.a != 0 {
                    row[x as usize] = source.blend_over(row[x as usize]);
                }
            }
        }
    }
}

/// Fills a rounded rectangle with 4x4 supersampled edge coverage.
///
/// The rectangle uses half-open bounds, clamps the corner radius to its dimensions,
/// clips work to the framebuffer, and composites translucent coverage source-over.
#[inline]
fn rounded_rect_aa_coverage(
    x: u32,
    y: usize,
    center_x: f64,
    center_y: f64,
    inner_width: f64,
    inner_height: f64,
    corner_radius: f64,
) -> usize {
    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let sample_xs = SAMPLE_OFFSETS.map(|offset| f64::from(x) + offset - center_x);
    let sample_ys = SAMPLE_OFFSETS.map(|offset| y as f64 + offset - center_y);
    let mut covered = 0;
    for py in sample_ys {
        for px in sample_xs {
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
    covered
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn blend_rounded_rect_aa_row(
    row: &mut [Color],
    y: usize,
    clip_left: u32,
    clip_right: u32,
    center_x: f64,
    center_y: f64,
    inner_width: f64,
    inner_height: f64,
    corner_radius: f64,
    coverage_colors: &[Color; 17],
) {
    let mut x = clip_left;
    while clip_right - x >= 4 {
        let mut sources = [Color::TRANSPARENT; 4];
        for (lane, source) in sources.iter_mut().enumerate() {
            let covered = rounded_rect_aa_coverage(
                x + lane as u32,
                y,
                center_x,
                center_y,
                inner_width,
                inner_height,
                corner_radius,
            );
            *source = coverage_colors[covered];
        }

        let start = x as usize;
        let destinations = &mut row[start..start + 4];
        if sources.iter().all(|source| source.a == 0) {
            // Fully uncovered SIMD groups need no framebuffer writes.
        } else if destinations.iter().all(|pixel| pixel.a == 255) {
            let mut quad = [
                destinations[0],
                destinations[1],
                destinations[2],
                destinations[3],
            ];
            crate::simd::blend_variable_opaque_quad(&mut quad, sources);
            destinations.copy_from_slice(&quad);
        } else {
            for (destination, source) in destinations.iter_mut().zip(sources) {
                if source.a != 0 {
                    *destination = source.blend_over(*destination);
                }
            }
        }
        x += 4;
    }

    for x in x..clip_right {
        let covered = rounded_rect_aa_coverage(
            x,
            y,
            center_x,
            center_y,
            inner_width,
            inner_height,
            corner_radius,
        );
        let source = coverage_colors[covered];
        if source.a != 0 {
            let pixel = &mut row[x as usize];
            *pixel = source.blend_over(*pixel);
        }
    }
}

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

    let center_x = (left + right - 1) as f64 * 0.5;
    let center_y = (top + bottom - 1) as f64 * 0.5;
    let half_width = (f64::from(rect.width) - 1.0) * 0.5;
    let half_height = (f64::from(rect.height) - 1.0) * 0.5;
    let corner_radius = f64::from(radius.min(rect.width / 2).min(rect.height / 2));
    let inner_width = half_width - corner_radius;
    let inner_height = half_height - corner_radius;

    let coverage_colors: [Color; 17] = core::array::from_fn(|covered| {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        Color::rgba(color.r, color.g, color.b, alpha)
    });

    for y in clip_top..clip_bottom {
        let Some(row) = target.row_mut(y) else {
            continue;
        };
        blend_rounded_rect_aa_row(
            row,
            y as usize,
            clip_left,
            clip_right,
            center_x,
            center_y,
            inner_width,
            inner_height,
            corner_radius,
            &coverage_colors,
        );
    }
}

/// Fills an anti-aliased rounded rectangle using disjoint scanline bands.
///
/// The result is deterministic and pixel-identical to [`fill_rounded_rect_aa`].
/// A thread count of zero or one selects the serial execution path.
pub fn fill_rounded_rect_aa_parallel_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    radius: u32,
    color: Color,
    thread_count: usize,
) {
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

    let center_x = (left + right - 1) as f64 * 0.5;
    let center_y = (top + bottom - 1) as f64 * 0.5;
    let half_width = (f64::from(rect.width) - 1.0) * 0.5;
    let half_height = (f64::from(rect.height) - 1.0) * 0.5;
    let corner_radius = f64::from(radius.min(rect.width / 2).min(rect.height / 2));
    let inner_width = half_width - corner_radius;
    let inner_height = half_height - corner_radius;
    let coverage_colors: [Color; 17] = core::array::from_fn(|covered| {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        Color::rgba(color.r, color.g, color.b, alpha)
    });

    let row_width = target.width() as usize;
    let start = clip_top as usize * row_width;
    let end = clip_bottom as usize * row_width;
    process_rows_with_y_mut(
        &mut target.pixels_mut()[start..end],
        row_width,
        clip_top as usize,
        thread_count,
        |row, y| {
            blend_rounded_rect_aa_row(
                row,
                y,
                clip_left,
                clip_right,
                center_x,
                center_y,
                inner_width,
                inner_height,
                corner_radius,
                &coverage_colors,
            );
        },
    );
}

/// Automatically selects a parallel worker count for sufficiently large rounded rectangles.
pub fn fill_rounded_rect_aa_parallel(
    target: &mut Framebuffer,
    rect: Rect,
    radius: u32,
    color: Color,
) {
    let area = clipped_rect_bounds(target, rect)
        .map(|(left, top, right, bottom)| (right - left) as u64 * (bottom - top) as u64)
        .unwrap_or(0);
    let thread_count = if area < 16_384 {
        1
    } else {
        automatic_worker_count()
    };
    fill_rounded_rect_aa_parallel_with_threads(target, rect, radius, color, thread_count);
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

/// Fills a rounded rectangle in disjoint scanline bands using safe worker threads.
///
/// The output is pixel-identical to [`fill_rounded_rect`]. A thread count of
/// zero or one selects serial execution.
pub fn fill_rounded_rect_parallel_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    radius: u32,
    color: Color,
    thread_count: usize,
) {
    if rect.width == 0 || rect.height == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }
    let Some((left, top, right, bottom)) = clipped_rect_bounds(target, rect) else {
        return;
    };
    let row_width = target.width() as usize;
    let start = top * row_width;
    let end = bottom * row_width;
    process_rows_with_y_mut(
        &mut target.pixels_mut()[start..end],
        row_width,
        top,
        thread_count,
        |row, y| {
            let Some((first_x, last_x)) = rounded_rect_scanline_span(rect, radius, y as i64) else {
                return;
            };
            let first_x = (first_x.max(left as i64)) as usize;
            let last_x = (last_x.min(right as i64 - 1)) as usize;
            if first_x <= last_x {
                row[first_x..=last_x].fill(color);
            }
        },
    );
}

/// Automatically parallelizes large rounded-rectangle fills.
pub fn fill_rounded_rect_parallel(target: &mut Framebuffer, rect: Rect, radius: u32, color: Color) {
    let thread_count = parallel_thread_count_for_rect(target, rect);
    fill_rounded_rect_parallel_with_threads(target, rect, radius, color, thread_count);
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

/// Draws a rounded-rectangle outline using independent scanline workers.
///
/// Rows are disjoint, so the implementation is safe without locks or unsafe code.
pub fn draw_rounded_rect_parallel_with_threads(
    target: &mut Framebuffer,
    rect: Rect,
    radius: u32,
    color: Color,
    thread_count: usize,
) {
    if rect.width == 0 || rect.height == 0 || target.width() == 0 || target.height() == 0 {
        return;
    }
    let radius = radius.min(rect.width / 2).min(rect.height / 2);
    let Some((left, top, right, bottom)) = clipped_rect_bounds(target, rect) else {
        return;
    };
    let row_width = target.width() as usize;
    let start = top * row_width;
    let end = bottom * row_width;
    let rect_top = i64::from(rect.y);
    let rect_bottom = rect_top + i64::from(rect.height) - 1;
    let horizontal_start = (i64::from(rect.x) + i64::from(radius)).max(left as i64);
    let horizontal_end =
        (i64::from(rect.x) + i64::from(rect.width) - 1 - i64::from(radius)).min(right as i64 - 1);
    let has_horizontal = horizontal_start <= horizontal_end;

    process_rows_with_y_mut(
        &mut target.pixels_mut()[start..end],
        row_width,
        top,
        thread_count,
        |row, y| {
            let y = y as i64;
            if y == rect_top || y == rect_bottom {
                if has_horizontal {
                    row[horizontal_start as usize..=horizontal_end as usize].fill(color);
                } else if let Some((first_x, last_x)) = rounded_rect_scanline_span(rect, radius, y)
                {
                    let first_x = first_x.max(left as i64) as usize;
                    let last_x = last_x.min(right as i64 - 1) as usize;
                    if first_x <= last_x {
                        row[first_x..=last_x].fill(color);
                    }
                }
                return;
            }
            if y <= rect_top || y >= rect_bottom {
                return;
            }
            let Some((first_x, last_x)) = rounded_rect_scanline_span(rect, radius, y) else {
                return;
            };
            let first_x = first_x.max(left as i64) as usize;
            let last_x = last_x.min(right as i64 - 1) as usize;
            if first_x <= last_x {
                row[first_x] = color;
                row[last_x] = color;
            }
        },
    );
}

/// Automatically selects a worker count for large rounded-rectangle outlines.
pub fn draw_rounded_rect_parallel(target: &mut Framebuffer, rect: Rect, radius: u32, color: Color) {
    let thread_count = parallel_thread_count_for_rect(target, rect);
    draw_rounded_rect_parallel_with_threads(target, rect, radius, color, thread_count);
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
    let mut scratch = RasterizerScratch::default();
    draw_polygon_aa_with_scratch(target, vertices, color, &mut scratch);
}

/// Draws an antialiased polygon outline while reusing caller-owned buffers.
pub fn draw_polygon_aa_with_scratch(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    scratch: &mut RasterizerScratch,
) {
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

    let segments = &mut scratch.aa_outline_segments;
    segments.clear();
    segments.extend((0..vertices.len()).map(|index| {
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
    }));

    const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];

    let coverage = &mut scratch.aa_coverage;
    coverage.resize((right - left) as usize, 0);
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

type PolygonScanEdge = (f64, f64, f64, f64, f64, f64);

/// Reusable temporary storage for CPU rasterization operations.
///
/// Keep one scratch value alongside a renderer and pass it to repeated polygon
/// fills to reuse edge and intersection allocations across frames.
type AaOutlineSegment = (f64, f64, f64, f64, f64, f64, f64, f64, f64);

#[derive(Debug, Default)]
pub struct RasterizerScratch {
    polygon_edges: Vec<PolygonScanEdge>,
    active_edges: Vec<usize>,
    intersections: Vec<f64>,
    aa_edges: Vec<PolygonEdge>,
    aa_edge_order: Vec<usize>,
    aa_active_edges: Vec<usize>,
    aa_intersections: Vec<f64>,
    aa_coverage: Vec<u8>,
    aa_outline_segments: Vec<AaOutlineSegment>,
    ellipse_aa_coverage: Vec<u8>,
    arc_aa_sources: Vec<Color>,
    ellipse_aa_sources: Vec<Color>,
}

/// Fills a polygon using the even-odd scanline rule at pixel centers.
pub fn fill_polygon(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    let mut scratch = RasterizerScratch::default();
    fill_polygon_with_scratch(target, vertices, color, &mut scratch);
}

/// Fills a polygon using independent scanline workers.
///
/// Each worker computes intersections for its own rows, preserving the
/// even-odd scan conversion used by `fill_polygon`.
pub fn fill_polygon_parallel_with_threads(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    thread_count: usize,
) {
    try_fill_polygon_parallel_with_threads(target, vertices, color, thread_count)
        .expect("failed to allocate parallel polygon scratch storage");
}

/// Fallibly fills a polygon using independent scanline workers.
///
/// All edge and worker-local intersection buffers are reserved before the
/// framebuffer is modified. An allocation failure therefore leaves the
/// target unchanged.
pub fn try_fill_polygon_parallel_with_threads(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    thread_count: usize,
) -> Result<(), RasterizerError> {
    if vertices.len() < 3 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }

    let min_y = vertices.iter().map(|point| point.y).min().unwrap().max(0) as usize;
    let max_y = vertices.iter().map(|point| point.y).max().unwrap();
    if max_y < 0 || min_y >= target.height() as usize {
        return Ok(());
    }
    let max_y = (max_y as usize).min(target.height() as usize - 1);
    if min_y > max_y {
        return Ok(());
    }

    let mut edges = Vec::new();
    edges
        .try_reserve_exact(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;
    for index in 0..vertices.len() {
        let a = vertices[index];
        let b = vertices[(index + 1) % vertices.len()];
        let (ax, ay, bx, by) = (
            f64::from(a.x),
            f64::from(a.y),
            f64::from(b.x),
            f64::from(b.y),
        );
        if ay != by {
            edges.push((ay.min(by), ay.max(by), ax, ay, bx, by));
        }
    }
    if edges.is_empty() {
        return Ok(());
    }

    let row_count = max_y - min_y + 1;
    let worker_count = thread_count.max(1).min(row_count);
    let mut worker_intersections = Vec::new();
    worker_intersections
        .try_reserve_exact(worker_count)
        .map_err(|_| RasterizerError::AllocationFailed)?;
    for _ in 0..worker_count {
        let mut intersections = Vec::new();
        intersections
            .try_reserve_exact(edges.len())
            .map_err(|_| RasterizerError::AllocationFailed)?;
        worker_intersections.push(intersections);
    }

    let row_width = target.width() as usize;
    let start = min_y * row_width;
    let end = (max_y + 1) * row_width;
    let rows = &mut target.pixels_mut()[start..end];
    let rows_per_worker = row_count.div_ceil(worker_count);
    let pixels_per_worker = rows_per_worker * row_width;

    std::thread::scope(|scope| {
        for (chunk_index, (chunk, mut intersections)) in rows
            .chunks_mut(pixels_per_worker)
            .zip(worker_intersections)
            .enumerate()
        {
            let first_y = min_y + chunk_index * rows_per_worker;
            let edges = &edges;
            scope.spawn(move || {
                for (row_offset, row) in chunk.chunks_mut(row_width).enumerate() {
                    let scan_y = (first_y + row_offset) as f64 + 0.5;
                    intersections.clear();
                    for &(edge_min_y, edge_max_y, ax, ay, bx, by) in edges {
                        if edge_min_y <= scan_y && edge_max_y > scan_y {
                            let t = (scan_y - ay) / (by - ay);
                            intersections.push(ax + t * (bx - ax));
                        }
                    }
                    intersections.sort_unstable_by(f64::total_cmp);
                    for pair in intersections.chunks_exact(2) {
                        let first = (pair[0] - 0.5).ceil().max(0.0) as usize;
                        let end = ((pair[1] - 0.5).ceil().max(0.0) as usize).min(row_width);
                        let start = first.min(row_width);
                        if start < end {
                            row[start..end].fill(color);
                        }
                    }
                }
            });
        }
    });
    Ok(())
}

/// Automatically selects a worker count for polygon filling.
pub fn fill_polygon_parallel(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    let area = target.width() as u64 * target.height() as u64;
    let thread_count = if vertices.len() < 3 || area < 16_384 {
        1
    } else {
        automatic_worker_count()
    };
    fill_polygon_parallel_with_threads(target, vertices, color, thread_count);
}

/// Fills a polygon using caller-owned reusable scratch storage.
///
/// The output is pixel-identical to fill_polygon. Reusing the same scratch
/// value avoids allocating edge, active-edge, and intersection vectors for
/// every draw call.
pub fn try_fill_polygon_with_scratch(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    scratch: &mut RasterizerScratch,
) -> Result<(), RasterizerError> {
    if vertices.len() < 3 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    let min_y = vertices.iter().map(|p| p.y).min().unwrap().max(0) as u32;
    let max_y = vertices.iter().map(|p| p.y).max().unwrap();
    if max_y < 0 || min_y >= target.height() {
        return Ok(());
    }
    let max_y = (max_y as u32).min(target.height() - 1);

    let RasterizerScratch {
        polygon_edges: edges,
        active_edges,
        intersections,
        ..
    } = scratch;

    // Reserve every buffer before touching the framebuffer. If any reservation
    // fails, the operation returns without drawing a partial polygon.
    edges
        .try_reserve(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;
    active_edges
        .try_reserve(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;
    intersections
        .try_reserve(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;

    // Refill existing buffers without discarding their allocations.
    edges.clear();
    for index in 0..vertices.len() {
        let a = vertices[index];
        let b = vertices[(index + 1) % vertices.len()];
        let (ax, ay, bx, by) = (
            f64::from(a.x),
            f64::from(a.y),
            f64::from(b.x),
            f64::from(b.y),
        );
        if ay != by {
            edges.push((ay.min(by), ay.max(by), ax, ay, bx, by));
        }
    }
    if edges.is_empty() {
        return Ok(());
    }

    // Activate only edges crossing the current scanline. Sorting by their
    // first covered y avoids revisiting every polygon edge on every row.
    edges.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));
    active_edges.clear();
    intersections.clear();
    let mut next_edge = 0;
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
        for &index in active_edges.iter() {
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
    Ok(())
}
/// Fills a polygon using caller-owned reusable scratch storage.
///
/// This compatibility wrapper preserves the original infallible API.
pub fn fill_polygon_with_scratch(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    try_fill_polygon_with_scratch(target, vertices, color, scratch)
        .expect("failed to allocate polygon rasterizer scratch storage");
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

#[inline]
fn accumulate_polygon_span_edges(
    coverage: &mut [u8],
    row_left: u32,
    start_x: u32,
    end_x: u32,
    span_left: f64,
    span_right: f64,
) {
    const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    if start_x >= end_x {
        return;
    }

    // Pixels in this interval contain all four horizontal samples. Count them
    // with a contiguous coverage update and reserve scalar tests for the edges.
    let interior_start =
        ((span_left - OFFSETS[0]).ceil() as i64).clamp(i64::from(start_x), i64::from(end_x)) as u32;
    let interior_end = (((span_right - OFFSETS[3]).ceil() as i64)
        .clamp(i64::from(start_x), i64::from(end_x)) as u32)
        .max(interior_start);

    for x in start_x..interior_start {
        let base = f64::from(x);
        let mut covered = 0;
        for offset in OFFSETS {
            let sample_x = base + offset;
            covered += u8::from(sample_x >= span_left && sample_x < span_right);
        }
        coverage[(x - row_left) as usize] += covered;
    }

    for covered in
        &mut coverage[(interior_start - row_left) as usize..(interior_end - row_left) as usize]
    {
        *covered += 4;
    }

    for x in interior_end..end_x {
        let base = f64::from(x);
        let mut covered = 0;
        for offset in OFFSETS {
            let sample_x = base + offset;
            covered += u8::from(sample_x >= span_left && sample_x < span_right);
        }
        coverage[(x - row_left) as usize] += covered;
    }
}

/// Processes a disjoint band of antialiased polygon rows with reusable worker-local buffers.
// This worker kernel receives its row partition, geometry, and reusable buffers separately.
#[allow(clippy::too_many_arguments)]
fn fill_polygon_aa_parallel_rows(
    rows: &mut [Color],
    row_width: usize,
    first_y: usize,
    left: u32,
    right: u32,
    edges: &[PolygonEdge],
    coverage_colors: &[Color; 17],
    coverage: &mut [u8],
    intersections: &mut Vec<f64>,
) {
    const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];

    for (row_offset, full_row) in rows.chunks_mut(row_width).enumerate() {
        let y = first_y + row_offset;
        coverage.fill(0);
        for offset_y in OFFSETS {
            let sample_y = y as f64 + offset_y;
            intersections.clear();
            for edge in edges {
                if edge.min_y <= sample_y && edge.max_y > sample_y {
                    intersections.push(edge.intersection_x(sample_y));
                }
            }
            intersections.sort_unstable_by(f64::total_cmp);
            for pair in intersections.chunks_exact(2) {
                let span_left = pair[0].max(f64::from(left));
                let span_right = pair[1].min(f64::from(right));
                if span_left < span_right {
                    accumulate_polygon_span_edges(
                        coverage,
                        left,
                        span_left.floor() as u32,
                        span_right.ceil() as u32,
                        span_left,
                        span_right,
                    );
                }
            }
        }

        let row_pixels = &mut full_row[left as usize..right as usize];
        let mut index = 0;
        while index < row_pixels.len() {
            if coverage[index] == 0 {
                index += 1;
                continue;
            }
            if row_pixels[index].a != u8::MAX {
                let source = coverage_colors[coverage[index] as usize];
                if source.a != 0 {
                    row_pixels[index] = source.blend_over(row_pixels[index]);
                }
                index += 1;
                continue;
            }

            let run_start = index;
            while index < row_pixels.len() && row_pixels[index].a == u8::MAX && coverage[index] != 0
            {
                index += 1;
            }
            let run_end = index;
            let mut offset = run_start;
            while offset + 4 <= run_end {
                let sources =
                    core::array::from_fn(|lane| coverage_colors[coverage[offset + lane] as usize]);
                let mut destinations = [
                    row_pixels[offset],
                    row_pixels[offset + 1],
                    row_pixels[offset + 2],
                    row_pixels[offset + 3],
                ];
                blend_variable_opaque_quad(&mut destinations, sources);
                row_pixels[offset..offset + 4].copy_from_slice(&destinations);
                offset += 4;
            }
            while offset < run_end {
                let source = coverage_colors[coverage[offset] as usize];
                row_pixels[offset] = source.blend_over(row_pixels[offset]);
                offset += 1;
            }
        }
    }
}

/// Fills an antialiased polygon using independent scanline workers.
///
/// Every worker computes the four vertical samples for its own rows. The
/// even-odd coverage rule and source-over compositing match the serial API.
pub fn fill_polygon_aa_parallel_with_threads(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    thread_count: usize,
) {
    try_fill_polygon_aa_parallel_with_threads(target, vertices, color, thread_count)
        .expect("failed to allocate parallel antialiased polygon scratch storage");
}

/// Fallibly fills an antialiased polygon using independent scanline workers.
///
/// Edge, coverage, and intersection storage for every worker is reserved
/// before the framebuffer is modified. Allocation failure leaves the target
/// unchanged.
pub fn try_fill_polygon_aa_parallel_with_threads(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    thread_count: usize,
) -> Result<(), RasterizerError> {
    if vertices.len() < 3 || color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }

    let mut coverage_colors = [Color::TRANSPARENT; 17];
    for (covered, slot) in coverage_colors.iter_mut().enumerate().skip(1) {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        *slot = Color::rgba(color.r, color.g, color.b, alpha);
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
        return Ok(());
    }

    let mut edges = Vec::new();
    edges
        .try_reserve_exact(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;
    for index in 0..vertices.len() {
        if let Some(edge) =
            PolygonEdge::new(vertices[index], vertices[(index + 1) % vertices.len()])
        {
            edges.push(edge);
        }
    }
    if edges.is_empty() {
        return Ok(());
    }

    let row_width = target.width() as usize;
    let row_count = (bottom - top) as usize;
    let worker_count = thread_count.max(1).min(row_count);
    let rows_per_worker = row_count.div_ceil(worker_count);
    let actual_worker_count = row_count.div_ceil(rows_per_worker);
    let span_width = (right - left) as usize;

    let mut worker_scratch = Vec::new();
    worker_scratch
        .try_reserve_exact(actual_worker_count)
        .map_err(|_| RasterizerError::AllocationFailed)?;
    for _ in 0..actual_worker_count {
        let mut coverage = Vec::new();
        coverage
            .try_reserve_exact(span_width)
            .map_err(|_| RasterizerError::AllocationFailed)?;
        coverage.resize(span_width, 0);

        let mut intersections = Vec::new();
        intersections
            .try_reserve_exact(edges.len())
            .map_err(|_| RasterizerError::AllocationFailed)?;
        worker_scratch.push((coverage, intersections));
    }

    let start = top as usize * row_width;
    let end = bottom as usize * row_width;
    let rows = &mut target.pixels_mut()[start..end];
    let pixels_per_worker = rows_per_worker * row_width;
    std::thread::scope(|scope| {
        for (chunk_index, (chunk, (mut coverage, mut intersections))) in rows
            .chunks_mut(pixels_per_worker)
            .zip(worker_scratch)
            .enumerate()
        {
            let first_y = top as usize + chunk_index * rows_per_worker;
            let edges = &edges;
            let coverage_colors = &coverage_colors;
            scope.spawn(move || {
                fill_polygon_aa_parallel_rows(
                    chunk,
                    row_width,
                    first_y,
                    left,
                    right,
                    edges,
                    coverage_colors,
                    &mut coverage,
                    &mut intersections,
                );
            });
        }
    });
    Ok(())
}

/// Automatically selects a worker count for antialiased polygon filling.
pub fn fill_polygon_aa_parallel(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    let area = target.width() as u64 * target.height() as u64;
    let thread_count = if vertices.len() < 3 || area < 16_384 {
        1
    } else {
        automatic_worker_count()
    };
    fill_polygon_aa_parallel_with_threads(target, vertices, color, thread_count);
}

/// Fills a polygon with 4x4 supersampled edge coverage and source-over blending.
pub fn fill_polygon_aa(target: &mut Framebuffer, vertices: &[Point], color: Color) {
    let mut scratch = RasterizerScratch::default();
    fill_polygon_aa_with_scratch(target, vertices, color, &mut scratch);
}

/// Fills an antialiased polygon while reusing caller-owned edge and scanline buffers.
///
/// Reusing the same scratch value avoids allocating polygon edges, edge ordering,
/// active edges, intersections, and per-row coverage for every draw call.
pub fn try_fill_polygon_aa_with_scratch(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    scratch: &mut RasterizerScratch,
) -> Result<(), RasterizerError> {
    if vertices.len() < 3 || color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }

    const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    // Cache the 17 possible source colors produced by 4x4 coverage sampling.
    // Opaque destinations are composited four pixels at a time by the SIMD backend.
    let mut coverage_colors = [Color::TRANSPARENT; 17];
    for (covered, slot) in coverage_colors.iter_mut().enumerate().skip(1) {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        *slot = Color::rgba(color.r, color.g, color.b, alpha);
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
        return Ok(());
    }

    let RasterizerScratch {
        aa_edges: edges,
        aa_edge_order: edge_order,
        aa_active_edges: active_edges,
        aa_intersections: intersections,
        aa_coverage: coverage,
        ..
    } = scratch;

    // Reserve all scratch storage before modifying the target. A failed
    // reservation leaves the framebuffer untouched.
    edges
        .try_reserve(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;
    edge_order
        .try_reserve(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;
    active_edges
        .try_reserve(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;
    intersections
        .try_reserve(vertices.len())
        .map_err(|_| RasterizerError::AllocationFailed)?;
    coverage
        .try_reserve((right - left) as usize)
        .map_err(|_| RasterizerError::AllocationFailed)?;

    edges.clear();
    edges.extend((0..vertices.len()).filter_map(|index| {
        PolygonEdge::new(vertices[index], vertices[(index + 1) % vertices.len()])
    }));
    if edges.is_empty() {
        return Ok(());
    }

    // Keep edges ordered by their first covered sub-scanline. The four
    // vertical samples advance monotonically, so an active list avoids
    // checking every polygon edge for every sample.
    edge_order.clear();
    edge_order.extend(0..edges.len());
    edge_order.sort_unstable_by(|&a, &b| edges[a].min_y.total_cmp(&edges[b].min_y));
    active_edges.clear();
    intersections.clear();
    let row_width = (right - left) as usize;
    coverage.resize(row_width, 0);
    let mut next_edge = 0;
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

            // Convert each even-odd crossing pair into a horizontal span.
            // Interior pixels receive all four horizontal samples at once;
            // only the two boundary regions need individual sample tests.
            for pair in intersections.chunks_exact(2) {
                let span_left = pair[0].max(f64::from(left));
                let span_right = pair[1].min(f64::from(right));
                if span_left >= span_right {
                    continue;
                }

                let first = span_left.floor() as u32;
                let end = span_right.ceil() as u32;
                let interior_start = (span_left - OFFSETS[0]).ceil() as u32;
                let interior_end = (span_right - OFFSETS[3]).ceil() as u32;
                let interior_start = interior_start.clamp(first, end);
                let interior_end = interior_end.clamp(first, end);

                if interior_start < interior_end {
                    accumulate_polygon_span_edges(
                        coverage,
                        left,
                        first,
                        interior_start,
                        span_left,
                        span_right,
                    );
                    coverage[(interior_start - left) as usize..(interior_end - left) as usize]
                        .iter_mut()
                        .for_each(|covered| *covered += 4);
                    accumulate_polygon_span_edges(
                        coverage,
                        left,
                        interior_end,
                        end,
                        span_left,
                        span_right,
                    );
                } else {
                    accumulate_polygon_span_edges(
                        coverage, left, first, end, span_left, span_right,
                    );
                }
            }
        }

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        let row_pixels = &mut row[left as usize..right as usize];
        let mut index = 0;
        while index < row_pixels.len() {
            if coverage[index] == 0 {
                index += 1;
                continue;
            }
            if row_pixels[index].a != u8::MAX {
                let covered = coverage[index];
                let source = coverage_colors[covered as usize];
                let pixel = &mut row_pixels[index];
                *pixel = if source.a == u8::MAX {
                    source
                } else if source.a == 0 {
                    *pixel
                } else {
                    source.blend_over(*pixel)
                };
                index += 1;
                continue;
            }

            // Vectorize only covered opaque runs. Uncovered spans are skipped
            // without loading or blending source colors.
            let run_start = index;
            while index < row_pixels.len() && row_pixels[index].a == u8::MAX && coverage[index] != 0
            {
                index += 1;
            }
            let run_end = index;
            let mut offset = run_start;
            while offset + 4 <= run_end {
                let sources =
                    core::array::from_fn(|lane| coverage_colors[coverage[offset + lane] as usize]);
                let mut quad = [
                    row_pixels[offset],
                    row_pixels[offset + 1],
                    row_pixels[offset + 2],
                    row_pixels[offset + 3],
                ];
                blend_variable_opaque_quad(&mut quad, sources);
                row_pixels[offset..offset + 4].copy_from_slice(&quad);
                offset += 4;
            }

            // Scalar remainder of this opaque run.
            while offset < run_end {
                let covered = coverage[offset];
                if covered != 0 {
                    let source = coverage_colors[covered as usize];
                    let pixel = &mut row_pixels[offset];
                    *pixel = if source.a == u8::MAX {
                        source
                    } else if source.a == 0 {
                        *pixel
                    } else {
                        source.blend_over(*pixel)
                    };
                }
                offset += 1;
            }
        }
    }
    Ok(())
}

/// Fills an antialiased polygon using reusable scratch storage.
///
/// This compatibility wrapper preserves the original infallible API.
pub fn fill_polygon_aa_with_scratch(
    target: &mut Framebuffer,
    vertices: &[Point],
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    try_fill_polygon_aa_with_scratch(target, vertices, color, scratch)
        .expect("failed to allocate antialiased polygon rasterizer scratch storage");
}

/// Draws an anti-aliased triangle outline with 4x4 coverage sampling.
pub fn draw_triangle_aa(target: &mut Framebuffer, a: Point, b: Point, c: Point, color: Color) {
    let mut scratch = RasterizerScratch::default();
    draw_triangle_aa_with_scratch(target, a, b, c, color, &mut scratch);
}

/// Draws an antialiased triangle outline using reusable rasterizer storage.
pub fn draw_triangle_aa_with_scratch(
    target: &mut Framebuffer,
    a: Point,
    b: Point,
    c: Point,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    draw_polygon_aa_with_scratch(target, &[a, b, c], color, scratch);
}

/// Fills a triangle with 4x4 coverage sampling and source-over alpha blending.
pub fn fill_triangle_aa(target: &mut Framebuffer, a: Point, b: Point, c: Point, color: Color) {
    let mut scratch = RasterizerScratch::default();
    fill_triangle_aa_with_scratch(target, a, b, c, color, &mut scratch);
}

/// Fills an antialiased triangle while reusing caller-owned polygon buffers.
pub fn fill_triangle_aa_with_scratch(
    target: &mut Framebuffer,
    a: Point,
    b: Point,
    c: Point,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    fill_polygon_aa_with_scratch(target, &[a, b, c], color, scratch);
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
#[inline]
fn accumulate_ellipse_span(
    coverage: &mut [u8],
    row_left: u32,
    start_x: u32,
    end_x: u32,
    span_left: f64,
    span_right: f64,
) {
    const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    if start_x >= end_x {
        return;
    }

    let interior_start =
        ((span_left - OFFSETS[0]).ceil() as i64).clamp(i64::from(start_x), i64::from(end_x)) as u32;
    let interior_end = ((span_right - OFFSETS[3]).floor() as i64 + 1)
        .clamp(i64::from(start_x), i64::from(end_x)) as u32;

    if interior_start < interior_end {
        for x in start_x..interior_start {
            let base = f64::from(x);
            for offset in OFFSETS {
                let sample_x = base + offset;
                if sample_x >= span_left && sample_x <= span_right {
                    coverage[(x - row_left) as usize] += 1;
                }
            }
        }
        for covered in
            &mut coverage[(interior_start - row_left) as usize..(interior_end - row_left) as usize]
        {
            *covered += 4;
        }
        for x in interior_end..end_x {
            let base = f64::from(x);
            for offset in OFFSETS {
                let sample_x = base + offset;
                if sample_x >= span_left && sample_x <= span_right {
                    coverage[(x - row_left) as usize] += 1;
                }
            }
        }
    } else {
        for x in start_x..end_x {
            let base = f64::from(x);
            for offset in OFFSETS {
                let sample_x = base + offset;
                if sample_x >= span_left && sample_x <= span_right {
                    coverage[(x - row_left) as usize] += 1;
                }
            }
        }
    }
}

/// Fills an ellipse with 4x4 supersampled edge coverage and source-over blending.
///
/// The ellipse is clipped to the framebuffer. A zero radius produces an anti-aliased
/// line along the remaining axis; a point-sized ellipse draws one pixel.
/// Processes an ellipse scanline band with one reusable coverage buffer per worker.
#[allow(clippy::too_many_arguments)]
fn fill_ellipse_aa_parallel_rows(
    rows: &mut [Color],
    row_width: usize,
    first_y: usize,
    left: u32,
    right: u32,
    cx: f64,
    cy: f64,
    rx: f64,
    inverse_ry: f64,
    coverage_colors: &[Color; 17],
    coverage: &mut [u8],
) {
    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    for (row_offset, row) in rows.chunks_mut(row_width).enumerate() {
        let y = first_y + row_offset;
        coverage.fill(0);
        for offset_y in SAMPLE_OFFSETS {
            let normalized_y = (y as f64 + offset_y - cy) * inverse_ry;
            let remaining = 1.0 - normalized_y * normalized_y;
            if remaining < 0.0 {
                continue;
            }
            let half_width = rx * remaining.sqrt();
            let span_left = cx - half_width;
            let span_right = cx + half_width;
            let first = ((span_left - SAMPLE_OFFSETS[3]).ceil() as i64)
                .clamp(i64::from(left), i64::from(right)) as u32;
            let end = ((span_right - SAMPLE_OFFSETS[0]).floor() as i64 + 1)
                .clamp(i64::from(left), i64::from(right)) as u32;
            accumulate_ellipse_span(coverage, left, first, end, span_left, span_right);
        }

        let row_pixels = &mut row[left as usize..right as usize];
        let mut index = 0;
        while index < row_pixels.len() {
            let covered = coverage[index];
            if covered == 0 {
                index += 1;
                continue;
            }
            if row_pixels[index].a != u8::MAX {
                let source = coverage_colors[usize::from(covered)];
                if source.a != 0 {
                    row_pixels[index] = if row_pixels[index].a == 0 {
                        source
                    } else {
                        source.blend_over(row_pixels[index])
                    };
                }
                index += 1;
                continue;
            }
            let run_start = index;
            while index < row_pixels.len() && row_pixels[index].a == u8::MAX && coverage[index] != 0
            {
                index += 1;
            }
            let run_end = index;
            let mut offset = run_start;
            while offset + 4 <= run_end {
                let sources = core::array::from_fn(|lane| {
                    coverage_colors[usize::from(coverage[offset + lane])]
                });
                let destinations: &mut [Color; 4] = (&mut row_pixels[offset..offset + 4])
                    .try_into()
                    .expect("four destination pixels");
                blend_variable_opaque_quad(destinations, sources);
                offset += 4;
            }
            while offset < run_end {
                let source = coverage_colors[usize::from(coverage[offset])];
                row_pixels[offset] = source.blend_over(row_pixels[offset]);
                offset += 1;
            }
        }
    }
}

/// Fills an antialiased ellipse using disjoint scanline bands and worker-local coverage buffers.
pub fn fill_ellipse_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
    thread_count: usize,
) {
    try_fill_ellipse_aa_parallel_with_threads(
        target,
        center,
        radius_x,
        radius_y,
        color,
        thread_count,
    )
    .expect("failed to allocate parallel antialiased ellipse scratch storage");
}

/// Fallibly fills an antialiased ellipse using disjoint scanline workers.
///
/// Every worker's coverage buffer is reserved before the framebuffer is
/// modified. Allocation failure leaves the target unchanged.
pub fn try_fill_ellipse_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
    thread_count: usize,
) -> Result<(), RasterizerError> {
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    if radius_x == 0 || radius_y == 0 || thread_count <= 1 {
        fill_ellipse_aa(target, center, radius_x, radius_y, color);
        return Ok(());
    }

    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    let rx = f64::from(radius_x);
    let ry = f64::from(radius_y);
    let left = (cx - rx - 1.0).floor().max(0.0) as u32;
    let top = (cy - ry - 1.0).floor().max(0.0) as u32;
    let right = (cx + rx + 1.0).ceil().min(f64::from(target.width())) as u32;
    let bottom = (cy + ry + 1.0).ceil().min(f64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return Ok(());
    }

    let mut coverage_colors = [Color::TRANSPARENT; 17];
    for (covered, slot) in coverage_colors.iter_mut().enumerate().skip(1) {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        *slot = Color::rgba(color.r, color.g, color.b, alpha);
    }

    let row_width = target.width() as usize;
    let row_count = (bottom - top) as usize;
    let worker_count = thread_count.max(1).min(row_count);
    let rows_per_worker = row_count.div_ceil(worker_count);
    let actual_worker_count = row_count.div_ceil(rows_per_worker);
    let span_width = (right - left) as usize;

    let mut worker_coverage = Vec::new();
    worker_coverage
        .try_reserve_exact(actual_worker_count)
        .map_err(|_| RasterizerError::AllocationFailed)?;
    for _ in 0..actual_worker_count {
        let mut coverage = Vec::new();
        coverage
            .try_reserve_exact(span_width)
            .map_err(|_| RasterizerError::AllocationFailed)?;
        coverage.resize(span_width, 0);
        worker_coverage.push(coverage);
    }

    let start = top as usize * row_width;
    let end = bottom as usize * row_width;
    let rows = &mut target.pixels_mut()[start..end];
    let pixels_per_worker = rows_per_worker * row_width;
    std::thread::scope(|scope| {
        for (chunk_index, (chunk, mut coverage)) in rows
            .chunks_mut(pixels_per_worker)
            .zip(worker_coverage)
            .enumerate()
        {
            let first_y = top as usize + chunk_index * rows_per_worker;
            let colors = &coverage_colors;
            scope.spawn(move || {
                fill_ellipse_aa_parallel_rows(
                    chunk,
                    row_width,
                    first_y,
                    left,
                    right,
                    cx,
                    cy,
                    rx,
                    ry.recip(),
                    colors,
                    &mut coverage,
                );
            });
        }
    });
    Ok(())
}

/// Automatically selects workers for sufficiently large antialiased ellipses.
pub fn fill_ellipse_aa_parallel(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
) {
    let area = u64::from(radius_x.saturating_mul(2).saturating_add(2))
        .saturating_mul(u64::from(radius_y.saturating_mul(2).saturating_add(2)));
    let workers = if area < 16_384 {
        1
    } else {
        automatic_worker_count()
    };
    fill_ellipse_aa_parallel_with_threads(target, center, radius_x, radius_y, color, workers);
}

pub fn fill_ellipse_aa(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
) {
    let mut scratch = RasterizerScratch::default();
    fill_ellipse_aa_with_scratch(target, center, radius_x, radius_y, color, &mut scratch);
}

/// Fills an antialiased ellipse while reusing caller-owned scanline coverage storage.
///
/// Reusing the scratch value avoids allocating the coverage row for every ellipse.
pub fn try_fill_ellipse_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
    scratch: &mut RasterizerScratch,
) -> Result<(), RasterizerError> {
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
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
        return Ok(());
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
        return Ok(());
    }

    let inverse_ry = ry.recip();

    // Coverage has only 17 possible values (0..=16). Cache their source
    // colors so opaque destination pixels can be composited four at a time.
    let mut coverage_colors = [Color::TRANSPARENT; 17];
    for (covered, slot) in coverage_colors.iter_mut().enumerate().skip(1) {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        *slot = Color::rgba(color.r, color.g, color.b, alpha);
    }

    let coverage = &mut scratch.ellipse_aa_coverage;
    coverage
        .try_reserve((right - left) as usize)
        .map_err(|_| RasterizerError::AllocationFailed)?;
    coverage.resize((right - left) as usize, 0);
    for y in top..bottom {
        coverage.fill(0);
        for offset_y in SAMPLE_OFFSETS {
            let normalized_y = (f64::from(y) + offset_y - cy) * inverse_ry;
            let remaining = 1.0 - normalized_y * normalized_y;
            if remaining < 0.0 {
                continue;
            }

            // At a fixed sub-scanline, the ellipse is a single horizontal span.
            // Count its interior four samples at once and inspect only edge pixels.
            let half_width = rx * remaining.sqrt();
            let span_left = cx - half_width;
            let span_right = cx + half_width;
            let first = ((span_left - SAMPLE_OFFSETS[3]).ceil() as i64)
                .clamp(i64::from(left), i64::from(right)) as u32;
            let end = ((span_right - SAMPLE_OFFSETS[0]).floor() as i64 + 1)
                .clamp(i64::from(left), i64::from(right)) as u32;
            accumulate_ellipse_span(coverage, left, first, end, span_left, span_right);
        }

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        let row_pixels = &mut row[left as usize..right as usize];
        let mut index = 0;
        while index < row_pixels.len() {
            let covered = coverage[index];
            if covered == 0 {
                index += 1;
                continue;
            }
            if row_pixels[index].a != u8::MAX {
                let source = coverage_colors[usize::from(covered)];
                if source.a != 0 {
                    row_pixels[index] = if row_pixels[index].a == 0 {
                        source
                    } else {
                        source.blend_over(row_pixels[index])
                    };
                }
                index += 1;
                continue;
            }

            // Form SIMD batches from contiguous opaque pixels that actually
            // receive coverage, instead of being limited to fixed row quads.
            let run_start = index;
            while index < row_pixels.len() && row_pixels[index].a == u8::MAX && coverage[index] != 0
            {
                index += 1;
            }
            let run_end = index;
            let mut offset = run_start;
            while offset + 4 <= run_end {
                let sources = core::array::from_fn(|lane| {
                    coverage_colors[usize::from(coverage[offset + lane])]
                });
                let destination_quad: &mut [Color; 4] = (&mut row_pixels[offset..offset + 4])
                    .try_into()
                    .expect("four destination pixels");
                blend_variable_opaque_quad(destination_quad, sources);
                offset += 4;
            }
            while offset < run_end {
                let source = coverage_colors[usize::from(coverage[offset])];
                row_pixels[offset] = source.blend_over(row_pixels[offset]);
                offset += 1;
            }
        }
    }
    Ok(())
}

/// Fills an antialiased ellipse while reusing caller-owned scanline storage.
///
/// This compatibility wrapper preserves the original infallible API.
pub fn fill_ellipse_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    try_fill_ellipse_aa_with_scratch(target, center, radius_x, radius_y, color, scratch)
        .expect("failed to allocate antialiased ellipse rasterizer scratch storage");
}

/// Draws an antialiased ellipse outline using disjoint row bands.
pub fn draw_ellipse_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
    thread_count: usize,
) {
    try_draw_ellipse_aa_parallel_with_threads(
        target,
        center,
        radius_x,
        radius_y,
        color,
        thread_count,
    )
    .expect("failed to allocate parallel antialiased ellipse outline scratch storage");
}

/// Fallibly draws an antialiased ellipse outline using disjoint scanline workers.
///
/// Every worker's source buffer is reserved before the framebuffer is
/// modified. Allocation failure leaves the target unchanged.
pub fn try_draw_ellipse_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
    thread_count: usize,
) -> Result<(), RasterizerError> {
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
    }
    if radius_x == 0 || radius_y == 0 || thread_count <= 1 {
        draw_ellipse_aa(target, center, radius_x, radius_y, color);
        return Ok(());
    }

    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    let rx = f64::from(radius_x);
    let ry = f64::from(radius_y);
    let left = (cx - rx - 1.0).floor().max(0.0) as u32;
    let top = (cy - ry - 1.0).floor().max(0.0) as u32;
    let right = (cx + rx + 1.0).ceil().min(f64::from(target.width())) as u32;
    let bottom = (cy + ry + 1.0).ceil().min(f64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return Ok(());
    }

    let inverse_rx_squared = 1.0 / (rx * rx);
    let inverse_ry_squared = 1.0 / (ry * ry);
    let row_width = target.width() as usize;
    let row_count = (bottom - top) as usize;
    let worker_count = thread_count.max(1).min(row_count);
    let rows_per_worker = row_count.div_ceil(worker_count);
    let actual_worker_count = row_count.div_ceil(rows_per_worker);
    let span_width = (right - left) as usize;

    let mut worker_sources = Vec::new();
    worker_sources
        .try_reserve_exact(actual_worker_count)
        .map_err(|_| RasterizerError::AllocationFailed)?;
    for _ in 0..actual_worker_count {
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(span_width)
            .map_err(|_| RasterizerError::AllocationFailed)?;
        sources.resize(span_width, Color::TRANSPARENT);
        worker_sources.push(sources);
    }

    let start = top as usize * row_width;
    let end = bottom as usize * row_width;
    let rows = &mut target.pixels_mut()[start..end];
    let pixels_per_worker = rows_per_worker * row_width;
    std::thread::scope(|scope| {
        for (chunk_index, (chunk, mut sources)) in rows
            .chunks_mut(pixels_per_worker)
            .zip(worker_sources)
            .enumerate()
        {
            let first_y = top as usize + chunk_index * rows_per_worker;
            scope.spawn(move || {
                draw_ellipse_aa_parallel_rows(
                    chunk,
                    row_width,
                    first_y,
                    left,
                    right,
                    cx,
                    cy,
                    inverse_rx_squared,
                    inverse_ry_squared,
                    color,
                    &mut sources,
                );
            });
        }
    });
    Ok(())
}

/// Automatically selects workers for sufficiently large antialiased ellipse outlines.
pub fn draw_ellipse_aa_parallel(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
) {
    let area = u64::from(radius_x.saturating_mul(2).saturating_add(2))
        .saturating_mul(u64::from(radius_y.saturating_mul(2).saturating_add(2)));
    let workers = if area < 16_384 {
        1
    } else {
        automatic_worker_count()
    };
    draw_ellipse_aa_parallel_with_threads(target, center, radius_x, radius_y, color, workers);
}

#[allow(clippy::too_many_arguments)]
fn draw_ellipse_aa_parallel_rows(
    rows: &mut [Color],
    row_width: usize,
    first_y: usize,
    left: u32,
    right: u32,
    cx: f64,
    cy: f64,
    inverse_rx_squared: f64,
    inverse_ry_squared: f64,
    color: Color,
    sources: &mut [Color],
) {
    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let mut coverage_colors = [Color::TRANSPARENT; 17];
    for (covered, slot) in coverage_colors.iter_mut().enumerate().skip(1) {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        *slot = Color::rgba(color.r, color.g, color.b, alpha);
    }

    for (row_index, row_pixels) in rows.chunks_mut(row_width).enumerate() {
        let y = first_y + row_index;
        sources.fill(Color::TRANSPARENT);
        for (index, source) in sources.iter_mut().enumerate() {
            let x = left + index as u32;
            let mut covered = 0_u32;
            for offset_y in SAMPLE_OFFSETS {
                let dy = y as f64 + offset_y - cy;
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
                    if ((radius - 1.0) / gradient.max(f64::EPSILON)).abs() <= 0.5 {
                        covered += 1;
                    }
                }
            }
            if covered != 0 {
                *source = coverage_colors[covered as usize];
            }
        }

        let destinations = &mut row_pixels[left as usize..right as usize];
        let mut index = 0;
        while index < destinations.len() {
            if sources[index].a == 0 {
                index += 1;
                continue;
            }
            if destinations[index].a != u8::MAX {
                destinations[index] = sources[index].blend_over(destinations[index]);
                index += 1;
                continue;
            }
            let run_start = index;
            while index < destinations.len()
                && destinations[index].a == u8::MAX
                && sources[index].a != 0
            {
                index += 1;
            }
            let mut quad_start = run_start;
            while quad_start + 4 <= index {
                let source_quad: [Color; 4] = sources[quad_start..quad_start + 4]
                    .try_into()
                    .expect("four source pixels");
                let destination_quad: &mut [Color; 4] = (&mut destinations
                    [quad_start..quad_start + 4])
                    .try_into()
                    .expect("four destination pixels");
                blend_variable_opaque_quad(destination_quad, source_quad);
                quad_start += 4;
            }
            for pixel_index in quad_start..index {
                destinations[pixel_index] =
                    sources[pixel_index].blend_over(destinations[pixel_index]);
            }
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
    let mut scratch = RasterizerScratch::default();
    draw_ellipse_aa_with_scratch(target, center, radius_x, radius_y, color, &mut scratch);
}

/// Draws an antialiased ellipse outline while reusing caller-owned scanline color storage.
pub fn try_draw_ellipse_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
    scratch: &mut RasterizerScratch,
) -> Result<(), RasterizerError> {
    if color.a == 0 || target.width() == 0 || target.height() == 0 {
        return Ok(());
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
        return Ok(());
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
        return Ok(());
    }

    let inverse_rx_squared = 1.0 / (rx * rx);
    let inverse_ry_squared = 1.0 / (ry * ry);
    let sources = &mut scratch.ellipse_aa_sources;
    let row_width = (right - left) as usize;
    sources
        .try_reserve(row_width.saturating_sub(sources.len()))
        .map_err(|_| RasterizerError::AllocationFailed)?;
    sources.resize(row_width, Color::TRANSPARENT);
    for y in top..bottom {
        sources.fill(Color::TRANSPARENT);
        for (index, source) in sources.iter_mut().enumerate() {
            let x = left + index as u32;
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
                    if ((radius - 1.0) / gradient.max(f64::EPSILON)).abs() <= 0.5 {
                        covered += 1;
                    }
                }
            }
            if covered != 0 {
                let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
                *source = Color::rgba(color.r, color.g, color.b, alpha);
            }
        }

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        let destinations = &mut row[left as usize..right as usize];
        let mut index = 0;
        while index < destinations.len() {
            if sources[index].a == 0 {
                index += 1;
                continue;
            }
            if destinations[index].a != u8::MAX {
                destinations[index] = sources[index].blend_over(destinations[index]);
                index += 1;
                continue;
            }

            // Batch complete groups from opaque destination runs. A single
            // translucent destination no longer blocks adjacent SIMD groups.
            let run_start = index;
            while index < destinations.len()
                && destinations[index].a == u8::MAX
                && sources[index].a != 0
            {
                index += 1;
            }
            let mut quad_start = run_start;
            while quad_start + 4 <= index {
                let source_quad: [Color; 4] = sources[quad_start..quad_start + 4]
                    .try_into()
                    .expect("four source pixels");
                let destination_quad: &mut [Color; 4] = (&mut destinations
                    [quad_start..quad_start + 4])
                    .try_into()
                    .expect("four destination pixels");
                blend_variable_opaque_quad(destination_quad, source_quad);
                quad_start += 4;
            }
            for pixel_index in quad_start..index {
                destinations[pixel_index] =
                    sources[pixel_index].blend_over(destinations[pixel_index]);
            }
        }
    }
    Ok(())
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
    let mut scratch = RasterizerScratch::default();
    draw_arc_aa_with_scratch(
        target,
        center,
        radius,
        start_angle,
        end_angle,
        color,
        &mut scratch,
    );
}

/// Draws an antialiased arc while reusing caller-owned scanline color storage.
pub fn try_draw_arc_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
    scratch: &mut RasterizerScratch,
) -> Result<(), RasterizerError> {
    use core::f64::consts::TAU;

    if color.a == 0
        || target.width() == 0
        || target.height() == 0
        || !start_angle.is_finite()
        || !end_angle.is_finite()
    {
        return Ok(());
    }

    let raw_sweep = f64::from(end_angle) - f64::from(start_angle);
    if raw_sweep == 0.0 {
        return Ok(());
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
        return Ok(());
    }

    if radius == 0.0 {
        if full_circle || sweep > 0.0 {
            draw_pixel(target, center, color);
        }
        return Ok(());
    }

    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let sources = &mut scratch.arc_aa_sources;
    let row_width = (right - left) as usize;
    sources
        .try_reserve(row_width.saturating_sub(sources.len()))
        .map_err(|_| RasterizerError::AllocationFailed)?;
    sources.resize(row_width, Color::TRANSPARENT);
    for y in top..bottom {
        sources.fill(Color::TRANSPARENT);
        for (index, source) in sources.iter_mut().enumerate() {
            let x = left + index as u32;
            let mut covered = 0_u32;
            for offset_y in SAMPLE_OFFSETS {
                let dy = f64::from(y) + offset_y - cy;
                for offset_x in SAMPLE_OFFSETS {
                    let dx = f64::from(x) + offset_x - cx;
                    if (dx.hypot(dy) - radius).abs() > 0.5 {
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
            if covered != 0 {
                let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
                *source = Color::rgba(color.r, color.g, color.b, alpha);
            }
        }

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        let destinations = &mut row[left as usize..right as usize];
        let mut index = 0;
        while index < destinations.len() {
            if sources[index].a == 0 {
                index += 1;
                continue;
            }
            if destinations[index].a != u8::MAX {
                destinations[index] = sources[index].blend_over(destinations[index]);
                index += 1;
                continue;
            }
            let run_start = index;
            while index < destinations.len()
                && destinations[index].a == u8::MAX
                && sources[index].a != 0
            {
                index += 1;
            }
            let mut quad_start = run_start;
            while quad_start + 4 <= index {
                let source_quad: [Color; 4] = sources[quad_start..quad_start + 4]
                    .try_into()
                    .expect("four source pixels");
                let destination_quad: &mut [Color; 4] = (&mut destinations
                    [quad_start..quad_start + 4])
                    .try_into()
                    .expect("four destination pixels");
                blend_variable_opaque_quad(destination_quad, source_quad);
                quad_start += 4;
            }
            for pixel_index in quad_start..index {
                destinations[pixel_index] =
                    sources[pixel_index].blend_over(destinations[pixel_index]);
            }
        }
    }
    Ok(())
}

/// Draws an antialiased arc using reusable caller-owned storage.
pub fn draw_arc_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    try_draw_arc_aa_with_scratch(
        target,
        center,
        radius,
        start_angle,
        end_angle,
        color,
        scratch,
    )
    .expect("failed to allocate antialiased arc scratch storage");
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
    let mut scratch = RasterizerScratch::default();
    fill_arc_aa_with_scratch(
        target,
        center,
        radius,
        start_angle,
        end_angle,
        color,
        &mut scratch,
    );
}

/// Fills an antialiased arc sector while reusing caller-owned scanline color storage.
pub fn try_fill_arc_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
    scratch: &mut RasterizerScratch,
) -> Result<(), RasterizerError> {
    use core::f64::consts::TAU;

    if color.a == 0
        || target.width() == 0
        || target.height() == 0
        || !start_angle.is_finite()
        || !end_angle.is_finite()
    {
        return Ok(());
    }

    let raw_sweep = f64::from(end_angle) - f64::from(start_angle);
    if raw_sweep == 0.0 {
        return Ok(());
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
        return Ok(());
    }

    // Compute one source row, then composite contiguous opaque destination
    // runs. This lets SIMD groups cross the old four-pixel chunk boundaries.
    const SAMPLE_OFFSETS: [f64; 4] = [-0.375, -0.125, 0.125, 0.375];
    let sources = &mut scratch.arc_aa_sources;
    let row_width = (right - left) as usize;
    sources
        .try_reserve(row_width.saturating_sub(sources.len()))
        .map_err(|_| RasterizerError::AllocationFailed)?;
    sources.resize(row_width, Color::TRANSPARENT);
    for y in top..bottom {
        sources.fill(Color::TRANSPARENT);
        for (index, source) in sources.iter_mut().enumerate() {
            let x = left + index as u32;
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
            if covered != 0 {
                let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
                *source = Color::rgba(color.r, color.g, color.b, alpha);
            }
        }

        let Some(row) = target.row_mut(y) else {
            continue;
        };
        let destinations = &mut row[left as usize..right as usize];
        let mut index = 0;
        while index < destinations.len() {
            if sources[index].a == 0 {
                index += 1;
                continue;
            }
            if destinations[index].a != u8::MAX {
                destinations[index] = sources[index].blend_over(destinations[index]);
                index += 1;
                continue;
            }

            let run_start = index;
            while index < destinations.len()
                && destinations[index].a == u8::MAX
                && sources[index].a != 0
            {
                index += 1;
            }
            let mut quad_start = run_start;
            while quad_start + 4 <= index {
                let source_quad: [Color; 4] = sources[quad_start..quad_start + 4]
                    .try_into()
                    .expect("four source pixels");
                let destination_quad: &mut [Color; 4] = (&mut destinations
                    [quad_start..quad_start + 4])
                    .try_into()
                    .expect("four destination pixels");
                blend_variable_opaque_quad(destination_quad, source_quad);
                quad_start += 4;
            }
            for pixel_index in quad_start..index {
                destinations[pixel_index] =
                    sources[pixel_index].blend_over(destinations[pixel_index]);
            }
        }
    }
    Ok(())
}

/// Fills an antialiased arc sector using reusable caller-owned storage.
pub fn fill_arc_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    try_fill_arc_aa_with_scratch(
        target,
        center,
        radius,
        start_angle,
        end_angle,
        color,
        scratch,
    )
    .expect("failed to allocate antialiased arc scratch storage");
}

/// Draws an antialiased circular arc using disjoint framebuffer row bands.
pub fn draw_arc_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
    thread_count: usize,
) {
    try_draw_arc_aa_parallel_with_threads(
        target,
        center,
        radius,
        start_angle,
        end_angle,
        color,
        thread_count,
    )
    .expect("failed to allocate parallel antialiased arc scratch storage");
}

/// Fallibly draws an antialiased circular arc with disjoint scanline workers.
///
/// Worker source buffers are allocated before drawing; allocation failure
/// leaves the framebuffer unchanged.
pub fn try_draw_arc_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
    thread_count: usize,
) -> Result<(), RasterizerError> {
    use core::f64::consts::TAU;

    if color.a == 0
        || target.width() == 0
        || target.height() == 0
        || !start_angle.is_finite()
        || !end_angle.is_finite()
    {
        return Ok(());
    }
    let raw_sweep = f64::from(end_angle) - f64::from(start_angle);
    if raw_sweep == 0.0 {
        return Ok(());
    }
    if radius == 0 || thread_count <= 1 {
        draw_arc_aa(target, center, radius, start_angle, end_angle, color);
        return Ok(());
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
        return Ok(());
    }

    let row_width = target.width() as usize;
    let row_count = (bottom - top) as usize;
    let worker_count = thread_count.min(row_count).max(1);
    let rows_per_worker = row_count.div_ceil(worker_count);
    let actual_worker_count = row_count.div_ceil(rows_per_worker);
    let span_width = (right - left) as usize;
    let mut worker_sources = Vec::new();
    worker_sources
        .try_reserve_exact(actual_worker_count)
        .map_err(|_| RasterizerError::AllocationFailed)?;
    for _ in 0..actual_worker_count {
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(span_width)
            .map_err(|_| RasterizerError::AllocationFailed)?;
        sources.resize(span_width, Color::TRANSPARENT);
        worker_sources.push(sources);
    }

    let start_index = top as usize * row_width;
    let end_index = bottom as usize * row_width;
    let rows = &mut target.pixels_mut()[start_index..end_index];
    let pixels_per_worker = rows_per_worker * row_width;
    std::thread::scope(|scope| {
        for (chunk_index, (chunk, mut sources)) in rows
            .chunks_mut(pixels_per_worker)
            .zip(worker_sources)
            .enumerate()
        {
            let first_y = top as usize + chunk_index * rows_per_worker;
            scope.spawn(move || {
                draw_arc_aa_parallel_rows(
                    chunk,
                    row_width,
                    first_y,
                    left,
                    right,
                    cx,
                    cy,
                    radius,
                    start,
                    sweep,
                    full_circle,
                    color,
                    &mut sources,
                );
            });
        }
    });
    Ok(())
}

/// Automatically selects workers for sufficiently large antialiased arcs.
pub fn draw_arc_aa_parallel(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
) {
    let diameter = u64::from(radius).saturating_mul(2).saturating_add(2);
    let workers = if diameter.saturating_mul(diameter) < 16_384 {
        1
    } else {
        automatic_worker_count()
    };
    draw_arc_aa_parallel_with_threads(
        target,
        center,
        radius,
        start_angle,
        end_angle,
        color,
        workers,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_arc_aa_parallel_rows(
    rows: &mut [Color],
    row_width: usize,
    first_y: usize,
    left: u32,
    right: u32,
    cx: f64,
    cy: f64,
    radius: f64,
    start: f64,
    sweep: f64,
    full_circle: bool,
    color: Color,
    sources: &mut [Color],
) {
    use core::f64::consts::TAU;
    const SAMPLE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
    let mut coverage_colors = [Color::TRANSPARENT; 17];
    for (covered, slot) in coverage_colors.iter_mut().enumerate().skip(1) {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        *slot = Color::rgba(color.r, color.g, color.b, alpha);
    }

    for (row_index, row_pixels) in rows.chunks_mut(row_width).enumerate() {
        let y = first_y + row_index;
        sources.fill(Color::TRANSPARENT);
        for (index, source) in sources.iter_mut().enumerate() {
            let x = left + index as u32;
            let mut covered = 0_u32;
            for offset_y in SAMPLE_OFFSETS {
                let dy = y as f64 + offset_y - cy;
                for offset_x in SAMPLE_OFFSETS {
                    let dx = f64::from(x) + offset_x - cx;
                    if (dx.hypot(dy) - radius).abs() > 0.5 {
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
            if covered != 0 {
                *source = coverage_colors[covered as usize];
            }
        }

        let destinations = &mut row_pixels[left as usize..right as usize];
        let mut index = 0;
        while index < destinations.len() {
            if sources[index].a == 0 {
                index += 1;
                continue;
            }
            if destinations[index].a != u8::MAX {
                destinations[index] = sources[index].blend_over(destinations[index]);
                index += 1;
                continue;
            }
            let run_start = index;
            while index < destinations.len()
                && destinations[index].a == u8::MAX
                && sources[index].a != 0
            {
                index += 1;
            }
            let mut quad_start = run_start;
            while quad_start + 4 <= index {
                let source_quad: [Color; 4] = sources[quad_start..quad_start + 4]
                    .try_into()
                    .expect("four source pixels");
                let destination_quad: &mut [Color; 4] = (&mut destinations
                    [quad_start..quad_start + 4])
                    .try_into()
                    .expect("four destination pixels");
                blend_variable_opaque_quad(destination_quad, source_quad);
                quad_start += 4;
            }
            for pixel_index in quad_start..index {
                destinations[pixel_index] =
                    sources[pixel_index].blend_over(destinations[pixel_index]);
            }
        }
    }
}

/// Fills an antialiased circular sector using disjoint framebuffer row bands.
pub fn fill_arc_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
    thread_count: usize,
) {
    try_fill_arc_aa_parallel_with_threads(
        target,
        center,
        radius,
        start_angle,
        end_angle,
        color,
        thread_count,
    )
    .expect("failed to allocate parallel antialiased sector scratch storage");
}

/// Fallibly fills an antialiased circular sector with disjoint scanline workers.
///
/// Worker source buffers are allocated before drawing; allocation failure
/// leaves the framebuffer unchanged.
pub fn try_fill_arc_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
    thread_count: usize,
) -> Result<(), RasterizerError> {
    use core::f64::consts::TAU;

    if color.a == 0
        || target.width() == 0
        || target.height() == 0
        || !start_angle.is_finite()
        || !end_angle.is_finite()
    {
        return Ok(());
    }
    let raw_sweep = f64::from(end_angle) - f64::from(start_angle);
    if raw_sweep == 0.0 {
        return Ok(());
    }
    if radius == 0 || thread_count <= 1 {
        fill_arc_aa(target, center, radius, start_angle, end_angle, color);
        return Ok(());
    }

    let full_circle = raw_sweep.abs() >= TAU;
    let start = f64::from(start_angle).rem_euclid(TAU);
    let sweep = if full_circle {
        TAU
    } else {
        raw_sweep.rem_euclid(TAU)
    };
    let radius_f = f64::from(radius);
    let cx = f64::from(center.x);
    let cy = f64::from(center.y);
    let left = (cx - radius_f - 1.0).floor().max(0.0) as u32;
    let top = (cy - radius_f - 1.0).floor().max(0.0) as u32;
    let right = (cx + radius_f + 1.0).ceil().min(f64::from(target.width())) as u32;
    let bottom = (cy + radius_f + 1.0).ceil().min(f64::from(target.height())) as u32;
    if left >= right || top >= bottom {
        return Ok(());
    }

    let row_width = target.width() as usize;
    let row_count = (bottom - top) as usize;
    let worker_count = thread_count.min(row_count).max(1);
    let rows_per_worker = row_count.div_ceil(worker_count);
    let actual_worker_count = row_count.div_ceil(rows_per_worker);
    let span_width = (right - left) as usize;
    let mut worker_sources = Vec::new();
    worker_sources
        .try_reserve_exact(actual_worker_count)
        .map_err(|_| RasterizerError::AllocationFailed)?;
    for _ in 0..actual_worker_count {
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(span_width)
            .map_err(|_| RasterizerError::AllocationFailed)?;
        sources.resize(span_width, Color::TRANSPARENT);
        worker_sources.push(sources);
    }

    let start_index = top as usize * row_width;
    let end_index = bottom as usize * row_width;
    let rows = &mut target.pixels_mut()[start_index..end_index];
    let pixels_per_worker = rows_per_worker * row_width;
    std::thread::scope(|scope| {
        for (chunk_index, (chunk, mut sources)) in rows
            .chunks_mut(pixels_per_worker)
            .zip(worker_sources)
            .enumerate()
        {
            let first_y = top as usize + chunk_index * rows_per_worker;
            scope.spawn(move || {
                fill_arc_aa_parallel_rows(
                    chunk,
                    row_width,
                    first_y,
                    left,
                    right,
                    cx,
                    cy,
                    radius_f,
                    start,
                    sweep,
                    full_circle,
                    color,
                    &mut sources,
                );
            });
        }
    });
    Ok(())
}

/// Automatically selects workers for sufficiently large antialiased sectors.
pub fn fill_arc_aa_parallel(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    start_angle: f32,
    end_angle: f32,
    color: Color,
) {
    let diameter = u64::from(radius).saturating_mul(2).saturating_add(2);
    let workers = if diameter.saturating_mul(diameter) < 16_384 {
        1
    } else {
        automatic_worker_count()
    };
    fill_arc_aa_parallel_with_threads(
        target,
        center,
        radius,
        start_angle,
        end_angle,
        color,
        workers,
    );
}

#[allow(clippy::too_many_arguments)]
fn fill_arc_aa_parallel_rows(
    rows: &mut [Color],
    row_width: usize,
    first_y: usize,
    left: u32,
    right: u32,
    cx: f64,
    cy: f64,
    radius: f64,
    start: f64,
    sweep: f64,
    full_circle: bool,
    color: Color,
    sources: &mut [Color],
) {
    use core::f64::consts::TAU;
    const SAMPLE_OFFSETS: [f64; 4] = [-0.375, -0.125, 0.125, 0.375];
    let mut coverage_colors = [Color::TRANSPARENT; 17];
    for (covered, slot) in coverage_colors.iter_mut().enumerate().skip(1) {
        let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
        *slot = Color::rgba(color.r, color.g, color.b, alpha);
    }

    for (row_index, row_pixels) in rows.chunks_mut(row_width).enumerate() {
        let y = first_y + row_index;
        sources.fill(Color::TRANSPARENT);
        for (index, source) in sources.iter_mut().enumerate() {
            let x = left + index as u32;
            let mut covered = 0_u32;
            for offset_y in SAMPLE_OFFSETS {
                let dy = y as f64 + offset_y - cy;
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
            if covered != 0 {
                *source = coverage_colors[covered as usize];
            }
        }

        let destinations = &mut row_pixels[left as usize..right as usize];
        let mut index = 0;
        while index < destinations.len() {
            if sources[index].a == 0 {
                index += 1;
                continue;
            }
            if destinations[index].a != u8::MAX {
                destinations[index] = sources[index].blend_over(destinations[index]);
                index += 1;
                continue;
            }
            let run_start = index;
            while index < destinations.len()
                && destinations[index].a == u8::MAX
                && sources[index].a != 0
            {
                index += 1;
            }
            let mut quad_start = run_start;
            while quad_start + 4 <= index {
                let source_quad: [Color; 4] = sources[quad_start..quad_start + 4]
                    .try_into()
                    .expect("four source pixels");
                let destination_quad: &mut [Color; 4] = (&mut destinations
                    [quad_start..quad_start + 4])
                    .try_into()
                    .expect("four destination pixels");
                blend_variable_opaque_quad(destination_quad, source_quad);
                quad_start += 4;
            }
            for pixel_index in quad_start..index {
                destinations[pixel_index] =
                    sources[pixel_index].blend_over(destinations[pixel_index]);
            }
        }
    }
}

/// Fills a circle with anti-aliased edge coverage.
pub fn fill_circle_aa(target: &mut Framebuffer, center: Point, radius: u32, color: Color) {
    fill_ellipse_aa(target, center, radius, radius, color);
}

/// Fills an antialiased circle while reusing caller-owned ellipse coverage storage.
pub fn try_fill_circle_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    color: Color,
    scratch: &mut RasterizerScratch,
) -> Result<(), RasterizerError> {
    try_fill_ellipse_aa_with_scratch(target, center, radius, radius, color, scratch)
}

/// Fills an antialiased circle while reusing caller-owned ellipse coverage storage.
pub fn fill_circle_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    try_fill_circle_aa_with_scratch(target, center, radius, color, scratch)
        .expect("failed to allocate antialiased circle rasterizer scratch storage");
}

/// Draws an anti-aliased circle outline with the ellipse coverage rasterizer.
pub fn draw_circle_aa(target: &mut Framebuffer, center: Point, radius: u32, color: Color) {
    draw_ellipse_aa(target, center, radius, radius, color);
}

/// Draws an antialiased ellipse outline using reusable caller-owned storage.
///
/// This compatibility wrapper preserves the original infallible API.
pub fn draw_ellipse_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius_x: u32,
    radius_y: u32,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    try_draw_ellipse_aa_with_scratch(target, center, radius_x, radius_y, color, scratch)
        .expect("failed to allocate antialiased ellipse outline scratch storage");
}

/// Draws an antialiased circle outline while reusing caller-owned ellipse scanline storage.
pub fn try_draw_circle_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    color: Color,
    scratch: &mut RasterizerScratch,
) -> Result<(), RasterizerError> {
    try_draw_ellipse_aa_with_scratch(target, center, radius, radius, color, scratch)
}

/// Draws an antialiased circle outline while reusing caller-owned ellipse scanline storage.
pub fn draw_circle_aa_with_scratch(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    color: Color,
    scratch: &mut RasterizerScratch,
) {
    try_draw_circle_aa_with_scratch(target, center, radius, color, scratch)
        .expect("failed to allocate antialiased circle outline scratch storage");
}

/// Fills an antialiased circle using the parallel ellipse rasterizer.
pub fn fill_circle_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    color: Color,
    thread_count: usize,
) {
    try_fill_circle_aa_parallel_with_threads(target, center, radius, color, thread_count)
        .expect("failed to allocate parallel antialiased circle fill scratch storage");
}

/// Fallibly fills an antialiased circle using parallel scanline workers.
pub fn try_fill_circle_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    color: Color,
    thread_count: usize,
) -> Result<(), RasterizerError> {
    try_fill_ellipse_aa_parallel_with_threads(target, center, radius, radius, color, thread_count)
}

/// Automatically selects workers for sufficiently large antialiased circle fills.
pub fn fill_circle_aa_parallel(target: &mut Framebuffer, center: Point, radius: u32, color: Color) {
    fill_ellipse_aa_parallel(target, center, radius, radius, color);
}

/// Draws an antialiased circle outline using the parallel ellipse rasterizer.
pub fn draw_circle_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    color: Color,
    thread_count: usize,
) {
    try_draw_circle_aa_parallel_with_threads(target, center, radius, color, thread_count)
        .expect("failed to allocate parallel antialiased circle outline scratch storage");
}

/// Fallibly draws an antialiased circle outline using parallel scanline workers.
pub fn try_draw_circle_aa_parallel_with_threads(
    target: &mut Framebuffer,
    center: Point,
    radius: u32,
    color: Color,
    thread_count: usize,
) -> Result<(), RasterizerError> {
    try_draw_ellipse_aa_parallel_with_threads(target, center, radius, radius, color, thread_count)
}

/// Automatically selects workers for sufficiently large antialiased circle outlines.
pub fn draw_circle_aa_parallel(target: &mut Framebuffer, center: Point, radius: u32, color: Color) {
    draw_ellipse_aa_parallel(target, center, radius, radius, color);
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

    #[test]
    fn parallel_antialiased_ellipse_fill_matches_serial_for_worker_counts() {
        for (center, radius_x, radius_y, color, background) in [
            (
                Point::new(20, 18),
                12,
                8,
                Color::rgba(230, 70, 120, 177),
                Color::rgba(12, 20, 28, 255),
            ),
            (
                Point::new(42, 25),
                19,
                11,
                Color::rgba(40, 190, 210, 129),
                Color::rgba(30, 50, 70, 120),
            ),
            (
                Point::new(3, 7),
                8,
                6,
                Color::rgba(170, 90, 30, 211),
                Color::TRANSPARENT,
            ),
        ] {
            let mut expected = Framebuffer::new(67, 43, background).unwrap();
            super::fill_ellipse_aa(&mut expected, center, radius_x, radius_y, color);
            for workers in [0, 1, 2, 4, 8, 64] {
                let mut actual = Framebuffer::new(67, 43, background).unwrap();
                super::fill_ellipse_aa_parallel_with_threads(
                    &mut actual,
                    center,
                    radius_x,
                    radius_y,
                    color,
                    workers,
                );
                assert_eq!(
                    actual, expected,
                    "worker count: {workers}, center: {center:?}, radii: {radius_x}/{radius_y}"
                );
                let mut expected_outline = Framebuffer::new(67, 43, background).unwrap();
                super::draw_ellipse_aa(&mut expected_outline, center, radius_x, radius_y, color);
                let mut fallible = Framebuffer::new(67, 43, background).unwrap();
                super::try_draw_ellipse_aa_parallel_with_threads(
                    &mut fallible,
                    center,
                    radius_x,
                    radius_y,
                    color,
                    workers,
                )
                .unwrap();
                assert_eq!(
                    fallible, expected_outline,
                    "fallible outline API, worker count: {workers}, center: {center:?}"
                );
                let mut fallible = Framebuffer::new(67, 43, background).unwrap();
                super::try_fill_ellipse_aa_parallel_with_threads(
                    &mut fallible,
                    center,
                    radius_x,
                    radius_y,
                    color,
                    workers,
                )
                .unwrap();
                assert_eq!(
                    fallible, expected,
                    "fallible API, worker count: {workers}, center: {center:?}"
                );
            }
            let mut automatic = Framebuffer::new(67, 43, background).unwrap();
            super::fill_ellipse_aa_parallel(&mut automatic, center, radius_x, radius_y, color);
            assert_eq!(automatic, expected);
        }
    }

    #[test]
    fn vertical_antialiased_lines_match_full_coverage_compositing() {
        let background = Color::rgba(20, 40, 60, 255);
        for alpha in [0_u8, 1, 64, 128, 254, 255] {
            let color = Color::rgba(230, 70, 120, alpha);
            let mut actual = Framebuffer::new(5, 7, background).unwrap();
            draw_line_aa(&mut actual, Point::new(2, 1), Point::new(2, 5), color);

            for y in 0..7 {
                let expected = if (1..=5).contains(&y) {
                    color.blend_over(background)
                } else {
                    background
                };
                assert_eq!(
                    actual.get_pixel(2, y),
                    Some(expected),
                    "alpha={alpha}, y={y}"
                );
            }
        }
    }

    #[test]
    fn parallel_antialiased_ellipse_outline_matches_serial_for_worker_counts() {
        for (center, radius_x, radius_y, color, background) in [
            (
                Point::new(20, 18),
                12,
                8,
                Color::rgba(230, 70, 120, 177),
                Color::rgba(12, 20, 28, 255),
            ),
            (
                Point::new(42, 25),
                19,
                11,
                Color::rgba(40, 190, 210, 129),
                Color::rgba(30, 50, 70, 120),
            ),
            (
                Point::new(3, 7),
                8,
                6,
                Color::rgba(170, 90, 30, 211),
                Color::TRANSPARENT,
            ),
        ] {
            let mut expected = Framebuffer::new(67, 43, background).unwrap();
            super::draw_ellipse_aa(&mut expected, center, radius_x, radius_y, color);
            for workers in [0, 1, 2, 4, 8, 64] {
                let mut actual = Framebuffer::new(67, 43, background).unwrap();
                super::draw_ellipse_aa_parallel_with_threads(
                    &mut actual,
                    center,
                    radius_x,
                    radius_y,
                    color,
                    workers,
                );
                assert_eq!(
                    actual, expected,
                    "worker count: {workers}, center: {center:?}, radii: {radius_x}/{radius_y}"
                );
            }
            let mut automatic = Framebuffer::new(67, 43, background).unwrap();
            super::draw_ellipse_aa_parallel(&mut automatic, center, radius_x, radius_y, color);
            assert_eq!(automatic, expected);
        }
    }

    #[test]
    fn ellipse_aa_reuses_scratch_coverage_across_draws() {
        let mut scratch = super::RasterizerScratch::default();
        let mut actual = Framebuffer::new(67, 43, Color::TRANSPARENT).unwrap();
        let mut expected = actual.clone();

        for (center, radius_x, radius_y, color) in [
            (Point::new(20, 18), 12, 8, Color::rgba(230, 70, 120, 177)),
            (Point::new(42, 25), 19, 11, Color::rgba(40, 190, 210, 129)),
            (Point::new(3, 7), 8, 6, Color::rgba(170, 90, 30, 211)),
        ] {
            super::fill_ellipse_aa_with_scratch(
                &mut actual,
                center,
                radius_x,
                radius_y,
                color,
                &mut scratch,
            );
            super::fill_ellipse_aa(&mut expected, center, radius_x, radius_y, color);
        }

        assert_eq!(actual, expected);
        assert!(scratch.ellipse_aa_coverage.capacity() >= 39);
        let capacity = scratch.ellipse_aa_coverage.capacity();

        super::fill_ellipse_aa_with_scratch(
            &mut actual,
            Point::new(30, 20),
            5,
            4,
            Color::WHITE,
            &mut scratch,
        );
        assert_eq!(scratch.ellipse_aa_coverage.capacity(), capacity);
    }

    #[test]
    fn polygon_span_fast_path_matches_four_sample_reference() {
        const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
        let row_left = 3_u32;
        let start_x = 5_u32;
        let end_x = 24_u32;
        for left_step in 0..32 {
            for width_step in 1..28 {
                let span_left = 4.0 + f64::from(left_step) * 0.137;
                let span_right = span_left + f64::from(width_step) * 0.113;
                let mut actual = vec![2_u8; 32];
                let mut expected = actual.clone();

                super::accumulate_polygon_span_edges(
                    &mut actual,
                    row_left,
                    start_x,
                    end_x,
                    span_left,
                    span_right,
                );

                for x in start_x..end_x {
                    let mut covered = 0_u8;
                    for offset in OFFSETS {
                        let sample_x = f64::from(x) + offset;
                        covered += u8::from(sample_x >= span_left && sample_x < span_right);
                    }
                    expected[(x - row_left) as usize] += covered;
                }
                assert_eq!(actual, expected, "span [{span_left}, {span_right})");
            }
        }
    }

    #[test]
    fn small_blend_rect_matches_pixel_source_over_for_all_destination_alpha_classes() {
        let initial = [
            Color::TRANSPARENT,
            Color::rgba(20, 40, 60, 1),
            Color::rgba(80, 100, 120, 127),
            Color::rgba(140, 160, 180, 254),
            Color::WHITE,
            Color::BLACK,
            Color::rgba(12, 34, 56, 200),
            Color::rgba(210, 180, 150, 90),
            Color::rgb(5, 15, 25),
            Color::rgba(255, 0, 128, 128),
            Color::rgba(0, 255, 64, 32),
            Color::rgb(240, 230, 220),
            Color::TRANSPARENT,
            Color::rgba(1, 2, 3, 64),
            Color::rgb(100, 110, 120),
            Color::rgba(200, 210, 220, 240),
        ];
        let source = Color::rgba(235, 91, 47, 137);
        let mut actual = target(4, 4);
        actual.copy_pixels_from(&initial).unwrap();

        let mut expected = initial;
        for y in 1..3 {
            for x in 1..3 {
                let index = y * 4 + x;
                expected[index] = source.blend_over(expected[index]);
            }
        }

        super::blend_rect(&mut actual, Rect::new(1, 1, 2, 2), source);

        assert_eq!(actual.pixels(), &expected);
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
    fn radial_gradient_squared_lookup_stays_within_one_channel_of_exact_sampling() {
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
            let squared_position = index as f32 / 10_000.0;
            let exact = gradient.sample(squared_position.sqrt());
            let fast = gradient.sample_radial_squared_in_range(squared_position);
            for (a, b) in [
                (exact.r, fast.r),
                (exact.g, fast.g),
                (exact.b, fast.b),
                (exact.a, fast.a),
            ] {
                assert!(
                    a.abs_diff(b) <= 1,
                    "squared position {squared_position}: {exact:?} != {fast:?}"
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
    fn fallible_antialiased_ellipse_outline_scratch_matches_compatibility_api() {
        let mut expected = target(20, 18);
        let mut actual = target(20, 18);
        let mut expected_scratch = super::RasterizerScratch::default();
        let mut actual_scratch = super::RasterizerScratch::default();
        super::draw_ellipse_aa_with_scratch(
            &mut expected,
            Point { x: 9, y: 8 },
            7,
            5,
            Color::rgba(220, 90, 40, 180),
            &mut expected_scratch,
        );
        super::try_draw_ellipse_aa_with_scratch(
            &mut actual,
            Point { x: 9, y: 8 },
            7,
            5,
            Color::rgba(220, 90, 40, 180),
            &mut actual_scratch,
        )
        .unwrap();
        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn fallible_antialiased_circle_scratch_matches_compatibility_api() {
        let mut expected = target(18, 18);
        let mut actual = target(18, 18);
        let mut expected_scratch = super::RasterizerScratch::default();
        let mut actual_scratch = super::RasterizerScratch::default();
        super::fill_circle_aa_with_scratch(
            &mut expected,
            Point { x: 8, y: 8 },
            6,
            Color::rgba(40, 160, 90, 200),
            &mut expected_scratch,
        );
        super::try_fill_circle_aa_with_scratch(
            &mut actual,
            Point { x: 8, y: 8 },
            6,
            Color::rgba(40, 160, 90, 200),
            &mut actual_scratch,
        )
        .unwrap();
        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn fallible_antialiased_ellipse_scratch_matches_compatibility_api() {
        let mut expected = target(18, 16);
        let mut actual = target(18, 16);
        let mut expected_scratch = super::RasterizerScratch::default();
        let mut actual_scratch = super::RasterizerScratch::default();
        super::fill_ellipse_aa_with_scratch(
            &mut expected,
            Point { x: 8, y: 7 },
            6,
            4,
            Color::rgba(60, 140, 220, 190),
            &mut expected_scratch,
        );
        super::try_fill_ellipse_aa_with_scratch(
            &mut actual,
            Point { x: 8, y: 7 },
            6,
            4,
            Color::rgba(60, 140, 220, 190),
            &mut actual_scratch,
        )
        .unwrap();
        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn fallible_antialiased_polygon_scratch_matches_compatibility_api() {
        let vertices = [
            Point { x: 1, y: 1 },
            Point { x: 7, y: 2 },
            Point { x: 4, y: 7 },
            Point { x: 2, y: 5 },
        ];
        let mut expected = target(10, 10);
        let mut actual = target(10, 10);
        let mut expected_scratch = super::RasterizerScratch::default();
        let mut actual_scratch = super::RasterizerScratch::default();
        super::fill_polygon_aa_with_scratch(
            &mut expected,
            &vertices,
            Color::rgba(200, 80, 40, 180),
            &mut expected_scratch,
        );
        super::try_fill_polygon_aa_with_scratch(
            &mut actual,
            &vertices,
            Color::rgba(200, 80, 40, 180),
            &mut actual_scratch,
        )
        .unwrap();
        assert_eq!(actual.pixels(), expected.pixels());
    }

    #[test]
    fn fallible_polygon_scratch_matches_compatibility_api() {
        let vertices = [
            Point { x: 1, y: 1 },
            Point { x: 7, y: 2 },
            Point { x: 4, y: 7 },
            Point { x: 2, y: 5 },
        ];
        let mut expected = target(10, 10);
        let mut actual = target(10, 10);
        super::fill_polygon(&mut expected, &vertices, Color::RED);
        let mut scratch = super::RasterizerScratch::default();
        super::try_fill_polygon_with_scratch(&mut actual, &vertices, Color::RED, &mut scratch)
            .unwrap();
        assert_eq!(actual.pixels(), expected.pixels());
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
    fn gradient_lookup_tables_keep_expected_sizes() {
        let gradient = test_gradient();
        assert_eq!(gradient.lookup.as_ref().map(Vec::len), Some(4096));
        assert_eq!(gradient.radial_lookup.as_ref().map(Vec::len), Some(65_536));
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
    fn conic_gradient_samples_clockwise_and_clips_to_rectangle() {
        let gradient = super::Gradient::new(vec![
            super::GradientStop { position: 0.0, color: Color::RED },
            super::GradientStop { position: 0.25, color: Color::GREEN },
            super::GradientStop { position: 0.5, color: Color::BLUE },
            super::GradientStop { position: 0.75, color: Color::WHITE },
            super::GradientStop { position: 1.0, color: Color::RED },
        ]).unwrap();
        let mut image = target(5, 5);
        super::fill_rect_conic_gradient(
            &mut image,
            Rect::new(1, 1, 3, 3),
            Vector::new(2.5, 2.5),
            0.0,
            &gradient,
        );

        assert_eq!(image.get_pixel(3, 2), Some(Color::RED));
        assert_eq!(image.get_pixel(2, 3), Some(Color::GREEN));
        assert_eq!(image.get_pixel(1, 2), Some(Color::BLUE));
        assert_eq!(image.get_pixel(2, 1), Some(Color::WHITE));
        assert_eq!(image.get_pixel(0, 0), Some(Color::BLACK));
    }

    #[test]
    fn parallel_conic_gradient_matches_serial_exactly() {
        let gradient = test_gradient();
        let mut serial = patterned_frame();
        let mut parallel = serial.clone();
        let rect = Rect::new(5, 4, 166, 111);
        let center = Vector::new(83.5, 56.25);
        let start_angle = 0.375;

        super::fill_rect_conic_gradient(&mut serial, rect, center, start_angle, &gradient);
        super::fill_rect_conic_gradient_parallel_with_threads(
            &mut parallel,
            rect,
            center,
            start_angle,
            &gradient,
            4,
        );
        assert_eq!(parallel, serial);
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
    fn fallible_parallel_circle_apis_match_ellipse_apis() {
        let center = Point::new(13, 11);
        let radius = 8;
        let color = Color::rgba(190, 75, 130, 171);
        for workers in [0, 1, 2, 4, 8] {
            let mut expected_fill = Framebuffer::new(31, 27, Color::rgba(12, 20, 28, 255)).unwrap();
            let mut actual_fill = expected_fill.clone();
            super::try_fill_ellipse_aa_parallel_with_threads(
                &mut expected_fill,
                center,
                radius,
                radius,
                color,
                workers,
            )
            .unwrap();
            super::try_fill_circle_aa_parallel_with_threads(
                &mut actual_fill,
                center,
                radius,
                color,
                workers,
            )
            .unwrap();
            assert_eq!(actual_fill, expected_fill, "fill, workers={workers}");

            let mut expected_draw = Framebuffer::new(31, 27, Color::rgba(12, 20, 28, 255)).unwrap();
            let mut actual_draw = expected_draw.clone();
            super::try_draw_ellipse_aa_parallel_with_threads(
                &mut expected_draw,
                center,
                radius,
                radius,
                color,
                workers,
            )
            .unwrap();
            super::try_draw_circle_aa_parallel_with_threads(
                &mut actual_draw,
                center,
                radius,
                color,
                workers,
            )
            .unwrap();
            assert_eq!(actual_draw, expected_draw, "outline, workers={workers}");
        }
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
    fn parallel_antialiased_rounded_rectangle_fill_matches_serial_for_worker_counts() {
        for (rect, radius, color) in [
            (Rect::new(3, 2, 25, 17), 6, Color::rgba(220, 70, 35, 173)),
            (Rect::new(24, 5, 19, 14), 9, Color::rgba(25, 180, 120, 211)),
            (Rect::new(0, 0, 2, 2), 1, Color::WHITE),
        ] {
            let mut expected = Framebuffer::new(32, 24, Color::rgba(12, 20, 28, 255)).unwrap();
            super::fill_rounded_rect_aa(&mut expected, rect, radius, color);
            for workers in [0, 1, 2, 4, 8] {
                let mut actual = Framebuffer::new(32, 24, Color::rgba(12, 20, 28, 255)).unwrap();
                super::fill_rounded_rect_aa_parallel_with_threads(
                    &mut actual,
                    rect,
                    radius,
                    color,
                    workers,
                );
                assert_eq!(actual, expected, "worker count: {workers}, rect: {rect:?}");
            }
            let mut automatic = Framebuffer::new(32, 24, Color::rgba(12, 20, 28, 255)).unwrap();
            super::fill_rounded_rect_aa_parallel(&mut automatic, rect, radius, color);
            assert_eq!(automatic, expected);
        }
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

#[cfg(test)]
mod parallel_raster_tests {
    use crate::{Color, Framebuffer, Rect};

    #[test]
    fn parallel_rectangle_fill_matches_serial_for_clipped_and_empty_regions() {
        let mut serial = Framebuffer::new(131, 97, Color::rgba(12, 34, 56, 78)).unwrap();
        let mut parallel = serial.clone();
        let cases = [
            (Rect::new(3, 7, 119, 83), Color::rgba(200, 40, 80, 120)),
            (Rect::new(0, 0, u32::MAX, 2), Color::WHITE),
            (Rect::new(127, 95, 20, 20), Color::GREEN),
            (Rect::new(20, 20, 0, 10), Color::RED),
            (Rect::new(u32::MAX, 0, 2, 2), Color::BLUE),
        ];

        for (rect, color) in cases {
            super::fill_rect(&mut serial, rect, color);
            super::fill_rect_parallel_with_threads(&mut parallel, rect, color, 4);
            assert_eq!(parallel, serial);
        }
    }

    #[test]
    fn parallel_rectangle_blend_matches_serial_for_every_alpha_class() {
        let mut serial = Framebuffer::new(129, 91, Color::TRANSPARENT).unwrap();
        for (index, pixel) in serial.pixels_mut().iter_mut().enumerate() {
            let alpha = match index % 3 {
                0 => 0,
                1 => 255,
                _ => (index % 254 + 1) as u8,
            };
            *pixel = Color::rgba(
                (index % 251) as u8,
                (index % 197) as u8,
                (index % 149) as u8,
                alpha,
            );
        }
        let mut parallel = serial.clone();
        let cases = [
            (Rect::new(2, 3, 124, 85), Color::rgba(210, 70, 35, 127)),
            (Rect::new(0, 0, 129, 91), Color::rgba(10, 200, 90, 1)),
            (Rect::new(20, 20, 4, 4), Color::rgba(80, 120, 240, 180)),
            (Rect::new(0, 0, 40, 30), Color::TRANSPARENT),
            (Rect::new(40, 40, 50, 40), Color::WHITE),
        ];

        for (rect, color) in cases {
            super::blend_rect(&mut serial, rect, color);
            super::blend_rect_parallel_with_threads(&mut parallel, rect, color, 4);
            assert_eq!(parallel, serial);
        }
    }

    #[test]
    fn parallel_raster_operations_fall_back_to_serial_for_one_worker() {
        let mut serial = Framebuffer::new(32, 24, Color::BLACK).unwrap();
        let mut parallel = serial.clone();
        let rect = Rect::new(2, 3, 28, 19);
        super::fill_rect(&mut serial, rect, Color::RED);
        super::fill_rect_parallel_with_threads(&mut parallel, rect, Color::RED, 1);
        assert_eq!(parallel, serial);

        super::blend_rect(&mut serial, rect, Color::rgba(20, 180, 90, 128));
        super::blend_rect_parallel_with_threads(
            &mut parallel,
            rect,
            Color::rgba(20, 180, 90, 128),
            0,
        );
        assert_eq!(parallel, serial);
    }
}

#[cfg(test)]
mod parallel_gradient_tests {
    use crate::{Color, Framebuffer, Gradient, GradientStop, Rect, Vector};

    fn test_gradient() -> Gradient {
        Gradient::new(vec![
            GradientStop {
                position: 0.0,
                color: Color::rgba(20, 80, 230, 255),
            },
            GradientStop {
                position: 0.45,
                color: Color::rgba(230, 70, 40, 170),
            },
            GradientStop {
                position: 1.0,
                color: Color::rgba(40, 210, 120, 0),
            },
        ])
        .unwrap()
    }

    fn patterned_frame() -> Framebuffer {
        let mut target = Framebuffer::new(173, 119, Color::BLACK).unwrap();
        for (index, pixel) in target.pixels_mut().iter_mut().enumerate() {
            *pixel = Color::rgba(
                (index % 251) as u8,
                (index % 193) as u8,
                (index % 157) as u8,
                (index % 256) as u8,
            );
        }
        target
    }

    #[test]
    fn parallel_linear_gradient_matches_serial_exactly() {
        let gradient = test_gradient();
        let mut serial = patterned_frame();
        let mut parallel = serial.clone();
        let rect = Rect::new(7, 9, 159, 103);
        let start = Vector::new(2.25, 5.5);
        let end = Vector::new(168.0, 111.75);

        super::fill_rect_linear_gradient(&mut serial, rect, start, end, &gradient);
        super::fill_rect_linear_gradient_parallel_with_threads(
            &mut parallel,
            rect,
            start,
            end,
            &gradient,
            4,
        );
        assert_eq!(parallel, serial);
    }

    #[test]
    fn parallel_radial_gradient_matches_serial_exactly() {
        let gradient = test_gradient();
        let mut serial = patterned_frame();
        let mut parallel = serial.clone();
        let rect = Rect::new(5, 4, 166, 111);
        let center = Vector::new(83.5, 56.25);
        let radius = 62.75;

        super::fill_rect_radial_gradient(&mut serial, rect, center, radius, &gradient);
        super::fill_rect_radial_gradient_parallel_with_threads(
            &mut parallel,
            rect,
            center,
            radius,
            &gradient,
            4,
        );
        assert_eq!(parallel, serial);
    }

    #[test]
    fn simd_gradient_compositing_matches_scalar_source_over() {
        let gradient = test_gradient();
        let rect = Rect::new(2, 1, 17, 6);
        let start = Vector::new(1.25, 0.5);
        let end = Vector::new(18.0, 6.75);
        let mut actual = patterned_frame();
        let mut expected = actual.clone();

        super::fill_rect_linear_gradient(&mut actual, rect, start, end, &gradient);
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let inverse_length_squared = (dx * dx + dy * dy).recip();
        for y in 1..7 {
            let mut position =
                ((2.5 - start.x) * dx + (y as f32 + 0.5 - start.y) * dy) * inverse_length_squared;
            for x in 2..19 {
                let source = gradient.sample_raster(position);
                let pixel = expected.get_pixel(x, y).unwrap();
                expected.set_pixel(x, y, source.blend_over(pixel));
                position += dx * inverse_length_squared;
            }
        }
        assert_eq!(actual, expected);

        let center = Vector::new(9.25, 4.0);
        let radius = 8.5;
        let mut actual = patterned_frame();
        let mut expected = actual.clone();
        super::fill_rect_radial_gradient(&mut actual, rect, center, radius, &gradient);
        let radius_squared = radius * radius;
        let inverse_radius_squared = radius_squared.recip();
        let outside_radius_squared = radius_squared * (1.0 + 4.0 * f32::EPSILON);
        let outside_color = gradient.sample_raster(1.0);
        for y in 1..7 {
            let dy = y as f32 + 0.5 - center.y;
            let dy_squared = dy * dy;
            let mut dx = 2.5 - center.x;
            for x in 2..19 {
                let distance_squared = dx * dx + dy_squared;
                let source = if distance_squared >= outside_radius_squared {
                    outside_color
                } else {
                    gradient
                        .sample_radial_squared_in_range(distance_squared * inverse_radius_squared)
                };
                let pixel = expected.get_pixel(x, y).unwrap();
                expected.set_pixel(x, y, source.blend_over(pixel));
                dx += 1.0;
            }
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn parallel_gradient_operations_keep_invalid_inputs_as_no_ops() {
        let gradient = test_gradient();
        let mut serial = patterned_frame();
        let mut parallel = serial.clone();
        let before = serial.clone();

        super::fill_rect_linear_gradient(
            &mut serial,
            Rect::new(0, 0, 80, 80),
            Vector::new(f32::NAN, 0.0),
            Vector::new(40.0, 40.0),
            &gradient,
        );
        super::fill_rect_linear_gradient_parallel_with_threads(
            &mut parallel,
            Rect::new(0, 0, 80, 80),
            Vector::new(f32::NAN, 0.0),
            Vector::new(40.0, 40.0),
            &gradient,
            3,
        );
        super::fill_rect_radial_gradient(
            &mut serial,
            Rect::new(0, 0, 80, 80),
            Vector::new(40.0, 40.0),
            0.0,
            &gradient,
        );
        super::fill_rect_radial_gradient_parallel_with_threads(
            &mut parallel,
            Rect::new(0, 0, 80, 80),
            Vector::new(40.0, 40.0),
            0.0,
            &gradient,
            3,
        );

        assert_eq!(serial, before);
        assert_eq!(parallel, before);
    }
}

#[cfg(test)]
mod rasterizer_scratch_tests {
    use crate::{Color, Framebuffer, Point, RasterizerScratch};

    #[test]
    fn polygon_fill_with_reused_scratch_is_pixel_identical() {
        let polygons: [&[Point]; 3] = [
            &[
                Point::new(3, 2),
                Point::new(24, 5),
                Point::new(18, 19),
                Point::new(7, 23),
                Point::new(1, 12),
            ],
            &[
                Point::new(2, 2),
                Point::new(27, 2),
                Point::new(27, 22),
                Point::new(2, 22),
            ],
            &[
                Point::new(4, 4),
                Point::new(24, 24),
                Point::new(4, 24),
                Point::new(24, 4),
            ],
        ];
        let colors = [Color::RED, Color::rgba(20, 120, 240, 190), Color::WHITE];
        let mut scratch = RasterizerScratch::default();

        for (vertices, color) in polygons.iter().zip(colors) {
            let mut expected = Framebuffer::new(31, 27, Color::BLACK).unwrap();
            let mut actual = expected.clone();
            super::fill_polygon(&mut expected, vertices, color);
            super::fill_polygon_with_scratch(&mut actual, vertices, color, &mut scratch);
            assert_eq!(actual, expected);
        }

        let edge_capacity = scratch.polygon_edges.capacity();
        let active_capacity = scratch.active_edges.capacity();
        let intersection_capacity = scratch.intersections.capacity();
        let mut target = Framebuffer::new(31, 27, Color::BLACK).unwrap();
        for _ in 0..8 {
            target.clear(Color::BLACK);
            super::fill_polygon_with_scratch(&mut target, polygons[0], Color::GREEN, &mut scratch);
        }
        assert_eq!(scratch.polygon_edges.capacity(), edge_capacity);
        assert_eq!(scratch.active_edges.capacity(), active_capacity);
        assert_eq!(scratch.intersections.capacity(), intersection_capacity);
    }

    #[test]
    fn antialiased_polygon_fill_with_reused_scratch_is_pixel_identical() {
        let polygons: [&[Point]; 3] = [
            &[
                Point::new(3, 2),
                Point::new(24, 5),
                Point::new(18, 19),
                Point::new(7, 23),
                Point::new(1, 12),
            ],
            &[
                Point::new(2, 2),
                Point::new(27, 2),
                Point::new(27, 22),
                Point::new(2, 22),
            ],
            &[
                Point::new(4, 4),
                Point::new(24, 24),
                Point::new(4, 24),
                Point::new(24, 4),
            ],
        ];
        let colors = [
            Color::rgba(220, 40, 80, 190),
            Color::rgba(20, 120, 240, 255),
            Color::rgba(180, 210, 30, 127),
        ];
        let mut scratch = RasterizerScratch::default();

        for (vertices, color) in polygons.iter().zip(colors) {
            let mut expected = Framebuffer::new(31, 27, Color::BLACK).unwrap();
            let mut actual = expected.clone();
            super::fill_polygon_aa(&mut expected, vertices, color);
            super::fill_polygon_aa_with_scratch(&mut actual, vertices, color, &mut scratch);
            assert_eq!(actual, expected);
        }

        let capacities = (
            scratch.aa_edges.capacity(),
            scratch.aa_edge_order.capacity(),
            scratch.aa_active_edges.capacity(),
            scratch.aa_intersections.capacity(),
            scratch.aa_coverage.capacity(),
        );
        let mut target = Framebuffer::new(31, 27, Color::BLACK).unwrap();
        for _ in 0..8 {
            target.clear(Color::BLACK);
            super::fill_polygon_aa_with_scratch(
                &mut target,
                polygons[0],
                Color::rgba(90, 180, 240, 210),
                &mut scratch,
            );
        }
        assert_eq!(
            capacities,
            (
                scratch.aa_edges.capacity(),
                scratch.aa_edge_order.capacity(),
                scratch.aa_active_edges.capacity(),
                scratch.aa_intersections.capacity(),
                scratch.aa_coverage.capacity(),
            )
        );
    }

    #[test]
    fn invalid_polygon_does_not_corrupt_reusable_scratch() {
        let mut scratch = RasterizerScratch::default();
        let mut target = Framebuffer::new(16, 16, Color::BLACK).unwrap();
        let triangle = [Point::new(2, 2), Point::new(13, 3), Point::new(7, 14)];
        super::fill_polygon_with_scratch(&mut target, &triangle, Color::RED, &mut scratch);
        let expected = target.clone();

        super::fill_polygon_with_scratch(&mut target, &triangle[..2], Color::GREEN, &mut scratch);
        assert_eq!(target, expected);
    }
}

#[cfg(test)]
mod transformed_radial_gradient_simd_tests {
    use crate::{Color, Framebuffer, Gradient, GradientStop, Rect, Transform2D, Vector};

    #[test]
    fn transformed_radial_gradient_simd_matches_scalar_for_opaque_and_translucent_targets() {
        let gradient = Gradient::new(vec![
            GradientStop {
                position: 0.0,
                color: Color::rgba(230, 30, 70, 255),
            },
            GradientStop {
                position: 0.4,
                color: Color::rgba(20, 190, 230, 173),
            },
            GradientStop {
                position: 1.0,
                color: Color::rgba(240, 210, 30, 35),
            },
        ])
        .unwrap();
        let rect = Rect::new(2, 3, 19, 13);
        let transform = Transform2D::translation(13.0, 7.0)
            .then(Transform2D::rotation(0.23))
            .then(Transform2D::scale(1.2, 0.9));
        let center = Vector::new(11.5, 8.0);
        let radius = 9.75;
        let inverse = transform.inverse().unwrap();

        for translucent in [false, true] {
            let mut actual = Framebuffer::new(64, 48, Color::BLACK).unwrap();
            for (index, pixel) in actual.pixels_mut().iter_mut().enumerate() {
                let alpha = if translucent {
                    (index % 256) as u8
                } else {
                    255
                };
                *pixel = Color::rgba(
                    (index % 251) as u8,
                    (index % 193) as u8,
                    (index % 157) as u8,
                    alpha,
                );
            }
            let mut expected = actual.clone();

            let visible = super::fill_transformed_rect_radial_gradient_clipped(
                &mut actual,
                rect,
                transform,
                center,
                radius,
                &gradient,
                Rect::new(0, 0, 64, 48),
            );
            assert!(visible);

            for y in 0..expected.height() {
                for x in 0..expected.width() {
                    let local =
                        inverse.transform_point(Vector::new(x as f32 + 0.5, y as f32 + 0.5));
                    if local.x < rect.x as f32
                        || local.y < rect.y as f32
                        || local.x >= rect.x.saturating_add(rect.width) as f32
                        || local.y >= rect.y.saturating_add(rect.height) as f32
                    {
                        continue;
                    }
                    let position = (local.x - center.x).hypot(local.y - center.y) / radius;
                    let source = gradient.sample_raster(position);
                    let destination = expected.get_pixel(x, y).unwrap();
                    expected.set_pixel(x, y, source.blend_over(destination));
                }
            }
            assert_eq!(actual, expected, "translucent destination: {translucent}");
        }
    }
}

#[cfg(test)]
mod ellipse_outline_simd_tests {
    use crate::{Color, Framebuffer, Point};

    fn scalar_reference(
        target: &mut Framebuffer,
        center: Point,
        radius_x: u32,
        radius_y: u32,
        color: Color,
    ) {
        if color.a == 0 || radius_x == 0 || radius_y == 0 {
            return;
        }
        const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
        let cx = f64::from(center.x);
        let cy = f64::from(center.y);
        let rx = f64::from(radius_x);
        let ry = f64::from(radius_y);
        let left = (cx - rx - 1.0).floor().max(0.0) as u32;
        let top = (cy - ry - 1.0).floor().max(0.0) as u32;
        let right = (cx + rx + 1.0).ceil().min(f64::from(target.width())) as u32;
        let bottom = (cy + ry + 1.0).ceil().min(f64::from(target.height())) as u32;
        let inverse_rx_squared = 1.0 / (rx * rx);
        let inverse_ry_squared = 1.0 / (ry * ry);

        for y in top..bottom {
            let row = target.row_mut(y).unwrap();
            for x in left..right {
                let mut covered = 0_u32;
                for offset_y in OFFSETS {
                    let dy = f64::from(y) + offset_y - cy;
                    let normalized_y = dy * dy * inverse_ry_squared;
                    for offset_x in OFFSETS {
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
                        if ((radius - 1.0) / gradient.max(f64::EPSILON)).abs() <= 0.5 {
                            covered += 1;
                        }
                    }
                }
                let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
                if alpha != 0 {
                    row[x as usize] =
                        Color::rgba(color.r, color.g, color.b, alpha).blend_over(row[x as usize]);
                }
            }
        }
    }

    fn scalar_fill_reference(
        target: &mut Framebuffer,
        center: Point,
        radius_x: u32,
        radius_y: u32,
        color: Color,
    ) {
        if color.a == 0 || radius_x == 0 || radius_y == 0 {
            return;
        }
        const OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
        let cx = f64::from(center.x);
        let cy = f64::from(center.y);
        let rx = f64::from(radius_x);
        let ry = f64::from(radius_y);
        let left = (cx - rx - 1.0).floor().max(0.0) as u32;
        let top = (cy - ry - 1.0).floor().max(0.0) as u32;
        let right = (cx + rx + 1.0).ceil().min(f64::from(target.width())) as u32;
        let bottom = (cy + ry + 1.0).ceil().min(f64::from(target.height())) as u32;

        for y in top..bottom {
            let row = target.row_mut(y).unwrap();
            for x in left..right {
                let mut covered = 0_u32;
                for offset_y in OFFSETS {
                    let dy = (f64::from(y) + offset_y - cy) / ry;
                    for offset_x in OFFSETS {
                        let dx = (f64::from(x) + offset_x - cx) / rx;
                        if dx * dx + dy * dy <= 1.0 {
                            covered += 1;
                        }
                    }
                }
                let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
                if alpha != 0 {
                    let source = Color::rgba(color.r, color.g, color.b, alpha);
                    let destination = &mut row[x as usize];
                    *destination = if destination.a == 0 {
                        source
                    } else {
                        source.blend_over(*destination)
                    };
                }
            }
        }
    }

    #[test]
    fn simd_ellipse_fill_matches_scalar_for_clipping_and_alpha_modes() {
        let cases = [
            (Point::new(18, 14), 12, 8),
            (Point::new(2, 13), 9, 7),
            (Point::new(37, 26), 11, 9),
            (Point::new(18, 14), 1, 1),
        ];
        for target_alpha in [255, 117] {
            for source_alpha in [255, 173, 1] {
                for (center, radius_x, radius_y) in cases {
                    let mut actual = Framebuffer::new(43, 31, Color::BLACK).unwrap();
                    for (index, pixel) in actual.pixels_mut().iter_mut().enumerate() {
                        *pixel = Color::rgba(
                            (index % 251) as u8,
                            (index % 193) as u8,
                            (index % 157) as u8,
                            if target_alpha == 255 {
                                255
                            } else {
                                (index * 37 % 256) as u8
                            },
                        );
                    }
                    let mut expected = actual.clone();
                    let color = Color::rgba(210, 75, 130, source_alpha);
                    super::fill_ellipse_aa(&mut actual, center, radius_x, radius_y, color);
                    scalar_fill_reference(&mut expected, center, radius_x, radius_y, color);
                    assert_eq!(
                        actual, expected,
                        "target alpha {target_alpha}, source alpha {source_alpha}, radii {radius_x}x{radius_y}"
                    );
                }
            }
        }
    }

    #[test]
    fn simd_ellipse_outline_matches_scalar_for_opaque_and_translucent_targets() {
        for target_alpha in [255, 117] {
            for source_alpha in [255, 173, 1] {
                let mut actual = Framebuffer::new(43, 31, Color::BLACK).unwrap();
                for (index, pixel) in actual.pixels_mut().iter_mut().enumerate() {
                    *pixel = Color::rgba(
                        (index % 251) as u8,
                        (index % 193) as u8,
                        (index % 157) as u8,
                        if target_alpha == 255 {
                            255
                        } else {
                            (index % 256) as u8
                        },
                    );
                }
                let mut expected = actual.clone();
                let color = Color::rgba(210, 75, 130, source_alpha);
                super::draw_ellipse_aa(&mut actual, Point::new(18, 15), 13, 9, color);
                scalar_reference(&mut expected, Point::new(18, 15), 13, 9, color);
                assert_eq!(
                    actual, expected,
                    "target alpha class: {target_alpha}, source alpha: {source_alpha}"
                );
            }
        }
    }

    #[test]
    fn simd_ellipse_outline_handles_clipping_and_scalar_tail() {
        let mut actual = Framebuffer::new(19, 13, Color::rgba(20, 30, 40, 255)).unwrap();
        let mut expected = actual.clone();
        let color = Color::rgba(230, 80, 40, 190);
        super::draw_ellipse_aa(&mut actual, Point::new(1, 6), 8, 5, color);
        scalar_reference(&mut expected, Point::new(1, 6), 8, 5, color);
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod arc_simd_tests {
    use core::f64::consts::TAU;

    use crate::{Color, Framebuffer, Point};

    const FILL_OFFSETS: [f64; 4] = [-0.375, -0.125, 0.125, 0.375];
    const OUTLINE_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];

    fn scalar_arc(
        target: &mut Framebuffer,
        center: Point,
        radius: u32,
        start_angle: f32,
        end_angle: f32,
        color: Color,
        outline: bool,
    ) {
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

        if outline && radius == 0.0 {
            if full_circle || sweep > 0.0 {
                super::draw_pixel(target, center, color);
            }
            return;
        }

        let offsets = if outline {
            &OUTLINE_OFFSETS
        } else {
            &FILL_OFFSETS
        };
        for y in top..bottom {
            let row = target.row_mut(y).unwrap();
            for x in left..right {
                let mut covered = 0_u32;
                for &offset_y in offsets {
                    let dy = f64::from(y) + offset_y - cy;
                    for &offset_x in offsets {
                        let dx = f64::from(x) + offset_x - cx;
                        let distance = dx.hypot(dy);
                        let inside = if outline {
                            (distance - radius).abs() <= 0.5
                        } else {
                            distance <= radius
                        };
                        if !inside {
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
                let alpha = (u16::from(color.a) * covered as u16 / 16) as u8;
                if alpha != 0 {
                    let source = Color::rgba(color.r, color.g, color.b, alpha);
                    row[x as usize] = source.blend_over(row[x as usize]);
                }
            }
        }
    }

    fn initialized_target(alpha_mode: u8) -> Framebuffer {
        let mut target = Framebuffer::new(37, 29, Color::BLACK).unwrap();
        for (index, pixel) in target.pixels_mut().iter_mut().enumerate() {
            let alpha = if alpha_mode == 255 {
                255
            } else {
                (index * 37 % 256) as u8
            };
            *pixel = Color::rgba(
                (index * 13 % 256) as u8,
                (index * 29 % 256) as u8,
                (index * 47 % 256) as u8,
                alpha,
            );
        }
        target
    }

    #[test]
    fn simd_arc_fill_matches_scalar_for_clipping_wrap_and_full_circle() {
        let cases = [
            (Point::new(18, 14), 10, 0.2, 2.4),
            (Point::new(2, 14), 9, 5.2, 1.1),
            (Point::new(18, 14), 11, -1.0, 8.0),
            (Point::new(18, 14), 0, 0.0, 1.0),
        ];
        for alpha_mode in [255, 127] {
            for source_alpha in [255, 173, 1] {
                for (center, radius, start, end) in cases {
                    let mut actual = initialized_target(alpha_mode);
                    let mut expected = actual.clone();
                    let color = Color::rgba(215, 71, 139, source_alpha);
                    super::fill_arc_aa(&mut actual, center, radius, start, end, color);
                    scalar_arc(&mut expected, center, radius, start, end, color, false);
                    assert_eq!(
                        actual, expected,
                        "fill alpha mode {alpha_mode}, source alpha {source_alpha}, radius {radius}"
                    );
                }
            }
        }
    }

    #[test]
    fn simd_arc_outline_matches_scalar_for_clipping_wrap_and_full_circle() {
        let cases = [
            (Point::new(18, 14), 10, 0.2, 2.4),
            (Point::new(2, 14), 9, 5.2, 1.1),
            (Point::new(18, 14), 11, -1.0, 8.0),
            (Point::new(18, 14), 0, 0.0, 1.0),
        ];
        for alpha_mode in [255, 127] {
            for source_alpha in [255, 173, 1] {
                for (center, radius, start, end) in cases {
                    let mut actual = initialized_target(alpha_mode);
                    let mut expected = actual.clone();
                    let color = Color::rgba(215, 71, 139, source_alpha);
                    super::draw_arc_aa(&mut actual, center, radius, start, end, color);
                    scalar_arc(&mut expected, center, radius, start, end, color, true);
                    assert_eq!(
                        actual, expected,
                        "outline alpha mode {alpha_mode}, source alpha {source_alpha}, radius {radius}"
                    );
                }
            }
        }
    }

    #[test]
    fn simd_arcs_ignore_zero_and_non_finite_sweeps() {
        for (start, end) in [(1.0, 1.0), (f32::NAN, 2.0), (0.0, f32::INFINITY)] {
            let mut fill = initialized_target(255);
            let mut outline = fill.clone();
            let before = fill.clone();
            super::fill_arc_aa(&mut fill, Point::new(18, 14), 8, start, end, Color::WHITE);
            super::draw_arc_aa(
                &mut outline,
                Point::new(18, 14),
                8,
                start,
                end,
                Color::WHITE,
            );
            assert_eq!(fill, before);
            assert_eq!(outline, before);
        }
    }
}

#[cfg(test)]
mod horizontal_line_simd_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn simd_horizontal_aa_line_matches_scalar_source_over() {
        let cases = [
            (Point::new(2, 5), Point::new(30, 5)),
            (Point::new(0, 0), Point::new(18, 0)),
            (Point::new(7, 11), Point::new(35, 11)),
        ];
        for destination_alpha in [255, 127] {
            for source_alpha in [255, 173, 1, 0] {
                for (start, end) in cases {
                    let mut actual = Framebuffer::new(37, 13, Color::BLACK).unwrap();
                    for (index, pixel) in actual.pixels_mut().iter_mut().enumerate() {
                        *pixel = Color::rgba(
                            (index * 13 % 256) as u8,
                            (index * 29 % 256) as u8,
                            (index * 47 % 256) as u8,
                            if destination_alpha == 255 {
                                255
                            } else {
                                (index * 17 % 256) as u8
                            },
                        );
                    }
                    let mut expected = actual.clone();
                    let color = Color::rgba(211, 83, 149, source_alpha);
                    let y = start.y as u32;
                    let left = start.x.min(end.x).max(0) as u32;
                    let right = start.x.max(end.x).min(36) as u32;
                    if start.y >= 0 && start.y < 13 {
                        let row = expected.row_mut(y).unwrap();
                        for pixel in &mut row[left as usize..=right as usize] {
                            *pixel = color.blend_over(*pixel);
                        }
                    }

                    super::draw_line_aa(&mut actual, start, end, color);
                    assert_eq!(
                        actual, expected,
                        "destination alpha {destination_alpha}, source alpha {source_alpha}"
                    );
                }
            }
        }
    }

    #[test]
    fn simd_horizontal_aa_line_preserves_clipping_and_tail_pixels() {
        let mut actual = Framebuffer::new(11, 3, Color::rgb(20, 30, 40)).unwrap();
        let mut expected = actual.clone();
        let color = Color::rgba(230, 90, 40, 127);
        for pixel in expected.row_mut(1).unwrap() {
            *pixel = color.blend_over(*pixel);
        }
        super::draw_line_aa(&mut actual, Point::new(-8, 1), Point::new(18, 1), color);
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod blend_rect_opaque_run_tests {
    use crate::{Color, Framebuffer, Rect};

    #[test]
    fn large_blend_rect_matches_scalar_with_fragmented_opaque_runs() {
        let mut initial = Framebuffer::new(43, 17, Color::BLACK).unwrap();
        for (index, pixel) in initial.pixels_mut().iter_mut().enumerate() {
            *pixel = Color::rgba(
                (index * 19 % 256) as u8,
                (index * 37 % 256) as u8,
                (index * 53 % 256) as u8,
                match index % 11 {
                    0 | 1 | 5 => 255,
                    2 => 0,
                    _ => (index * 23 % 254 + 1) as u8,
                },
            );
        }
        let source = Color::rgba(219, 83, 147, 139);
        let rect = Rect::new(3, 2, 37, 13);

        let mut expected = initial.clone();
        for y in 2..15 {
            for x in 3..40 {
                let pixel = expected.get_pixel(x, y).unwrap();
                expected.set_pixel(x, y, source.blend_over(pixel));
            }
        }

        let mut serial = initial.clone();
        super::blend_rect(&mut serial, rect, source);
        assert_eq!(serial, expected);

        for thread_count in [0, 1, 2, 4, 32] {
            let mut parallel = initial.clone();
            super::blend_rect_parallel_with_threads(&mut parallel, rect, source, thread_count);
            assert_eq!(parallel, expected, "thread count {thread_count}");
        }
    }

    #[test]
    fn large_blend_rect_clips_opaque_runs_and_preserves_untouched_pixels() {
        let base = Color::rgb(17, 29, 41);
        let mut actual = Framebuffer::new(19, 9, base).unwrap();
        let mut expected = actual.clone();
        let source = Color::rgba(230, 70, 110, 177);
        let rect = Rect::new(4, 2, 40, 20);

        for y in 2..9 {
            for x in 4..19 {
                let old = expected.get_pixel(x, y).unwrap();
                expected.set_pixel(x, y, source.blend_over(old));
            }
        }

        super::blend_rect(&mut actual, rect, source);
        assert_eq!(actual, expected);
        for y in 0..9 {
            for x in 0..19 {
                if x < 4 || y < 2 {
                    assert_eq!(actual.get_pixel(x, y), Some(base));
                }
            }
        }
    }
}

#[cfg(test)]
mod polygon_aa_opaque_run_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn polygon_aa_simd_opaque_runs_match_scalar_source_over() {
        let width = 41;
        let height = 23;
        let vertices = [
            Point::new(2, 3),
            Point::new(35, 1),
            Point::new(38, 17),
            Point::new(7, 21),
        ];
        let source = Color::rgba(231, 74, 159, 173);

        let mut initial = Framebuffer::new(width, height, Color::TRANSPARENT).unwrap();
        for (index, pixel) in initial.pixels_mut().iter_mut().enumerate() {
            *pixel = Color::rgba(
                (index * 17 % 256) as u8,
                (index * 43 % 256) as u8,
                (index * 71 % 256) as u8,
                match index % 9 {
                    0 | 1 | 4 => 255,
                    2 => 0,
                    _ => (index * 29 % 254 + 1) as u8,
                },
            );
        }

        // A transparent target exposes the exact coverage-adjusted source
        // alpha produced by the rasterizer, giving us an independent scalar
        // source-over reference for every destination alpha class.
        let mut coverage = Framebuffer::new(width, height, Color::TRANSPARENT).unwrap();
        super::fill_polygon_aa(&mut coverage, &vertices, source);

        let mut expected = initial.clone();
        for (destination, covered_source) in expected
            .pixels_mut()
            .iter_mut()
            .zip(coverage.pixels().iter().copied())
        {
            if covered_source.a != 0 {
                *destination = covered_source.blend_over(*destination);
            }
        }

        let mut actual = initial;
        super::fill_polygon_aa(&mut actual, &vertices, source);
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod polygon_aa_parallel_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn parallel_aa_polygon_matches_serial_for_clipping_alpha_and_threads() {
        let polygons = [
            vec![
                Point::new(4, 5),
                Point::new(55, 8),
                Point::new(39, 47),
                Point::new(9, 42),
            ],
            vec![Point::new(18, 12), Point::new(65, 19), Point::new(48, 53)],
            vec![Point::new(-8, 20), Point::new(26, -4), Point::new(71, 39)],
            vec![
                Point::new(-30, -20),
                Point::new(120, -15),
                Point::new(40, 90),
            ],
        ];
        let backgrounds = [Color::rgba(11, 23, 37, 255), Color::rgba(11, 23, 37, 91)];
        let colors = [Color::rgba(190, 80, 220, 173), Color::rgba(255, 140, 20, 1)];

        for vertices in &polygons {
            for background in backgrounds {
                for color in colors {
                    let mut expected = Framebuffer::new(79, 63, background).unwrap();
                    super::fill_polygon_aa(&mut expected, vertices, color);
                    for threads in [0, 1, 2, 4, 8] {
                        let mut actual = Framebuffer::new(79, 63, background).unwrap();
                        super::fill_polygon_aa_parallel_with_threads(
                            &mut actual,
                            vertices,
                            color,
                            threads,
                        );
                        assert_eq!(
                            actual, expected,
                            "threads {threads}, background {background:?}, color {color:?}"
                        );
                        let mut fallible = Framebuffer::new(79, 63, background).unwrap();
                        super::try_fill_polygon_aa_parallel_with_threads(
                            &mut fallible,
                            vertices,
                            color,
                            threads,
                        )
                        .unwrap();
                        assert_eq!(
                            fallible, expected,
                            "fallible API, threads {threads}, background {background:?}, color {color:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn automatic_parallel_aa_polygon_matches_serial() {
        let vertices = [
            Point::new(-12, 8),
            Point::new(181, 14),
            Point::new(154, 117),
            Point::new(23, 101),
        ];
        let mut expected = Framebuffer::new(192, 128, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        let color = Color::rgba(40, 180, 220, 190);
        super::fill_polygon_aa(&mut expected, &vertices, color);
        super::fill_polygon_aa_parallel(&mut actual, &vertices, color);
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod polygon_parallel_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn parallel_polygon_matches_serial_for_clipping_and_thread_counts() {
        let polygons = [
            vec![
                Point::new(4, 5),
                Point::new(55, 8),
                Point::new(39, 47),
                Point::new(9, 42),
            ],
            vec![Point::new(18, 12), Point::new(65, 19), Point::new(48, 53)],
            vec![Point::new(-8, 20), Point::new(26, -4), Point::new(71, 39)],
            vec![Point::new(12, 12), Point::new(12, 12), Point::new(12, 12)],
            vec![
                Point::new(-30, -20),
                Point::new(120, -15),
                Point::new(40, 90),
            ],
        ];
        for vertices in &polygons {
            let mut expected = Framebuffer::new(79, 63, Color::rgba(11, 23, 37, 255)).unwrap();
            super::fill_polygon(&mut expected, vertices, Color::rgba(190, 80, 220, 173));
            for threads in [0, 1, 2, 4, 8] {
                let mut actual = Framebuffer::new(79, 63, Color::rgba(11, 23, 37, 255)).unwrap();
                super::fill_polygon_parallel_with_threads(
                    &mut actual,
                    vertices,
                    Color::rgba(190, 80, 220, 173),
                    threads,
                );
                assert_eq!(actual, expected, "threads {threads}, vertices {vertices:?}");

                let mut fallible = Framebuffer::new(79, 63, Color::rgba(11, 23, 37, 255)).unwrap();
                super::try_fill_polygon_parallel_with_threads(
                    &mut fallible,
                    vertices,
                    Color::rgba(190, 80, 220, 173),
                    threads,
                )
                .unwrap();
                assert_eq!(
                    fallible, expected,
                    "fallible API, threads {threads}, vertices {vertices:?}"
                );
            }
        }
    }

    #[test]
    fn automatic_parallel_polygon_matches_serial() {
        let vertices = [
            Point::new(-12, 8),
            Point::new(181, 14),
            Point::new(154, 117),
            Point::new(23, 101),
        ];
        let mut expected = Framebuffer::new(192, 128, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        super::fill_polygon(&mut expected, &vertices, Color::WHITE);
        super::fill_polygon_parallel(&mut actual, &vertices, Color::WHITE);
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod rounded_rect_outline_parallel_tests {
    use crate::{Color, Framebuffer, Rect};

    #[test]
    fn parallel_outline_matches_serial_for_clipped_and_degenerate_rectangles() {
        let cases = [
            (Rect::new(3, 4, 71, 39), 12),
            (Rect::new(0, 7, 39, 31), 10),
            (Rect::new(62, 48, 35, 29), 8),
            (Rect::new(14, 13, 1, 1), 0),
            (Rect::new(0, 0, 2, 2), 1),
            (Rect::new(0, 0, 96, 64), 0),
        ];
        for (rect, radius) in cases {
            let mut expected = Framebuffer::new(96, 64, Color::TRANSPARENT).unwrap();
            super::draw_rounded_rect(&mut expected, rect, radius, Color::WHITE);
            for threads in [0, 1, 2, 4, 8] {
                let mut actual = Framebuffer::new(96, 64, Color::TRANSPARENT).unwrap();
                super::draw_rounded_rect_parallel_with_threads(
                    &mut actual,
                    rect,
                    radius,
                    Color::WHITE,
                    threads,
                );
                assert_eq!(actual, expected, "threads {threads}, rect {rect:?}");
            }
        }
    }

    #[test]
    fn automatic_parallel_outline_matches_serial() {
        let rect = Rect::new(0, 8, 175, 96);
        let mut expected = Framebuffer::new(192, 128, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        super::draw_rounded_rect(&mut expected, rect, 27, Color::WHITE);
        super::draw_rounded_rect_parallel(&mut actual, rect, 27, Color::WHITE);
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod rounded_rect_parallel_tests {
    use crate::{Color, Framebuffer, Rect};

    #[test]
    fn parallel_rounded_rect_matches_serial_for_clipping_and_thread_counts() {
        let cases = [
            (Rect::new(3, 4, 71, 39), 12),
            (Rect::new(0, 7, 39, 31), 10),
            (Rect::new(62, 48, 35, 29), 8),
            (Rect::new(14, 13, 1, 1), 0),
            (Rect::new(0, 0, 96, 64), 32),
        ];
        for (rect, radius) in cases {
            let mut expected = Framebuffer::new(96, 64, Color::rgba(19, 31, 47, 255)).unwrap();
            super::fill_rounded_rect(&mut expected, rect, radius, Color::rgba(211, 87, 43, 173));
            for threads in [0, 1, 2, 4, 8] {
                let mut actual = Framebuffer::new(96, 64, Color::rgba(19, 31, 47, 255)).unwrap();
                super::fill_rounded_rect_parallel_with_threads(
                    &mut actual,
                    rect,
                    radius,
                    Color::rgba(211, 87, 43, 173),
                    threads,
                );
                assert_eq!(actual, expected, "thread count {threads}, rect {rect:?}");
            }
        }
    }

    #[test]
    fn automatic_parallel_rounded_rect_matches_serial() {
        let rect = Rect::new(0, 8, 175, 96);
        let color = Color::rgba(40, 180, 220, 201);
        let mut expected = Framebuffer::new(192, 128, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        super::fill_rounded_rect(&mut expected, rect, 27, color);
        super::fill_rounded_rect_parallel(&mut actual, rect, 27, color);
        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod blend_rect_transparent_run_tests {
    use crate::{
        rasterizer::{
            draw_circle_aa_parallel, draw_circle_aa_parallel_with_threads, draw_ellipse_aa,
            fill_circle_aa_parallel, fill_circle_aa_parallel_with_threads, fill_ellipse_aa,
        },
        Color, Framebuffer, Point, Rect,
    };

    #[test]
    fn transparent_destination_runs_match_scalar_source_over() {
        let mut initial = Framebuffer::new(47, 11, Color::BLACK).unwrap();
        for (index, pixel) in initial.pixels_mut().iter_mut().enumerate() {
            let alpha = match index % 13 {
                0..=4 => 0,
                5..=9 => 255,
                _ => (index * 31 % 254 + 1) as u8,
            };
            *pixel = Color::rgba(
                (index * 17 % 256) as u8,
                (index * 37 % 256) as u8,
                (index * 59 % 256) as u8,
                alpha,
            );
        }

        let source = Color::rgba(213, 91, 147, 137);
        let rect = Rect::new(2, 1, 43, 9);
        let mut expected = initial.clone();
        for y in 1..10 {
            for x in 2..45 {
                let destination = expected.get_pixel(x, y).unwrap();
                expected.set_pixel(x, y, source.blend_over(destination));
            }
        }

        let mut actual = initial;
        super::blend_rect(&mut actual, rect, source);
        assert_eq!(actual, expected);
    }
    #[test]
    fn fallible_arc_scratch_apis_match_compatibility_apis() {
        let center = Point::new(15, 12);
        let color = Color::rgba(180, 70, 35, 190);
        let background = Color::rgba(17, 29, 43, 255);
        let mut expected_draw = Framebuffer::new(32, 28, background).unwrap();
        let mut actual_draw = Framebuffer::new(32, 28, background).unwrap();
        let mut expected_fill = Framebuffer::new(32, 28, background).unwrap();
        let mut actual_fill = Framebuffer::new(32, 28, background).unwrap();
        let mut expected_scratch = super::RasterizerScratch::default();
        let mut actual_scratch = super::RasterizerScratch::default();

        super::draw_arc_aa_with_scratch(
            &mut expected_draw,
            center,
            10,
            0.3,
            2.4,
            color,
            &mut expected_scratch,
        );
        super::try_draw_arc_aa_with_scratch(
            &mut actual_draw,
            center,
            10,
            0.3,
            2.4,
            color,
            &mut actual_scratch,
        )
        .unwrap();
        super::fill_arc_aa_with_scratch(
            &mut expected_fill,
            center,
            10,
            0.3,
            2.4,
            color,
            &mut expected_scratch,
        );
        super::try_fill_arc_aa_with_scratch(
            &mut actual_fill,
            center,
            10,
            0.3,
            2.4,
            color,
            &mut actual_scratch,
        )
        .unwrap();

        assert_eq!(actual_draw.pixels(), expected_draw.pixels());
        assert_eq!(actual_fill.pixels(), expected_fill.pixels());
    }

    #[test]
    fn arc_aa_reuses_scratch_scanline_across_draws() {
        let mut scratch = super::RasterizerScratch::default();
        let mut actual = Framebuffer::new(67, 43, Color::TRANSPARENT).unwrap();
        let mut expected = actual.clone();

        for (center, radius, start, end, color) in [
            (
                Point::new(20, 18),
                12,
                0.2,
                2.7,
                Color::rgba(230, 70, 120, 177),
            ),
            (
                Point::new(42, 25),
                19,
                1.1,
                5.4,
                Color::rgba(40, 190, 210, 129),
            ),
            (Point::new(3, 7), 8, 4.8, 7.2, Color::rgba(170, 90, 30, 211)),
        ] {
            super::draw_arc_aa_with_scratch(
                &mut actual,
                center,
                radius,
                start,
                end,
                color,
                &mut scratch,
            );
            super::draw_arc_aa(&mut expected, center, radius, start, end, color);
            super::fill_arc_aa_with_scratch(
                &mut actual,
                center,
                radius,
                start,
                end,
                color,
                &mut scratch,
            );
            super::fill_arc_aa(&mut expected, center, radius, start, end, color);
        }

        assert_eq!(actual, expected);
        assert!(scratch.arc_aa_sources.capacity() >= 39);
        let capacity = scratch.arc_aa_sources.capacity();
        super::draw_arc_aa_with_scratch(
            &mut actual,
            Point::new(30, 20),
            10,
            0.0,
            1.5,
            Color::WHITE,
            &mut scratch,
        );
        assert_eq!(scratch.arc_aa_sources.capacity(), capacity);
    }

    #[test]
    fn ellipse_outline_aa_reuses_scratch_scanline_across_draws() {
        let mut scratch = super::RasterizerScratch::default();
        let mut actual = Framebuffer::new(67, 43, Color::TRANSPARENT).unwrap();
        let mut expected = actual.clone();

        for (center, radius_x, radius_y, color) in [
            (Point::new(20, 18), 12, 8, Color::rgba(230, 70, 120, 177)),
            (Point::new(42, 25), 19, 11, Color::rgba(40, 190, 210, 129)),
            (Point::new(3, 7), 8, 6, Color::rgba(170, 90, 30, 211)),
        ] {
            super::draw_ellipse_aa_with_scratch(
                &mut actual,
                center,
                radius_x,
                radius_y,
                color,
                &mut scratch,
            );
            super::draw_ellipse_aa(&mut expected, center, radius_x, radius_y, color);
        }

        assert_eq!(actual, expected);
        assert!(scratch.ellipse_aa_sources.capacity() >= 39);
        let capacity = scratch.ellipse_aa_sources.capacity();
        super::draw_ellipse_aa_with_scratch(
            &mut actual,
            Point::new(30, 20),
            10,
            6,
            Color::WHITE,
            &mut scratch,
        );
        assert_eq!(scratch.ellipse_aa_sources.capacity(), capacity);
    }

    #[test]
    fn circle_aa_scratch_variants_match_compatibility_wrappers() {
        let mut scratch = super::RasterizerScratch::default();
        let mut actual = Framebuffer::new(61, 47, Color::TRANSPARENT).unwrap();
        let mut expected = actual.clone();

        for (center, radius, color) in [
            (Point::new(18, 17), 12, Color::rgba(230, 70, 120, 177)),
            (Point::new(42, 25), 19, Color::rgba(40, 190, 210, 129)),
            (Point::new(3, 7), 8, Color::rgba(170, 90, 30, 211)),
        ] {
            super::fill_circle_aa_with_scratch(&mut actual, center, radius, color, &mut scratch);
            super::fill_circle_aa(&mut expected, center, radius, color);
            super::draw_circle_aa_with_scratch(&mut actual, center, radius, color, &mut scratch);
            super::draw_circle_aa(&mut expected, center, radius, color);
        }
        assert_eq!(actual, expected);
        assert!(scratch.ellipse_aa_coverage.capacity() >= 41);
        assert!(scratch.ellipse_aa_sources.capacity() >= 41);
    }

    #[test]
    fn polygon_outline_aa_reuses_scratch_buffers() {
        let mut scratch = super::RasterizerScratch::default();
        let mut actual = Framebuffer::new(73, 59, Color::TRANSPARENT).unwrap();
        let mut expected = actual.clone();
        let polygons = [
            vec![
                Point::new(4, 5),
                Point::new(55, 8),
                Point::new(39, 47),
                Point::new(9, 42),
            ],
            vec![Point::new(18, 12), Point::new(65, 19), Point::new(48, 53)],
            vec![Point::new(-8, 20), Point::new(26, -4), Point::new(71, 39)],
        ];
        for vertices in &polygons {
            super::draw_polygon_aa_with_scratch(
                &mut actual,
                vertices,
                Color::rgba(190, 80, 220, 173),
                &mut scratch,
            );
            super::draw_polygon_aa(&mut expected, vertices, Color::rgba(190, 80, 220, 173));
        }
        assert_eq!(actual, expected);
        assert!(scratch.aa_outline_segments.capacity() >= 4);
        assert!(scratch.aa_coverage.capacity() >= 73);
    }

    #[test]
    fn triangle_and_rect_outline_aa_scratch_match_wrappers() {
        let mut scratch = super::RasterizerScratch::default();
        let mut actual = Framebuffer::new(83, 67, Color::TRANSPARENT).unwrap();
        let mut expected = actual.clone();
        for rect in [
            Rect::new(4, 6, 31, 24),
            Rect::new(48, 8, 29, 43),
            Rect::new(0, 39, 17, 35),
        ] {
            super::draw_rect_aa_with_scratch(
                &mut actual,
                rect,
                Color::rgba(30, 190, 120, 181),
                &mut scratch,
            );
            super::draw_rect_aa(&mut expected, rect, Color::rgba(30, 190, 120, 181));
        }
        for (a, b, c) in [
            (Point::new(8, 9), Point::new(71, 17), Point::new(33, 59)),
            (Point::new(-7, 32), Point::new(39, 2), Point::new(82, 63)),
        ] {
            super::draw_triangle_aa_with_scratch(
                &mut actual,
                a,
                b,
                c,
                Color::rgba(210, 80, 40, 137),
                &mut scratch,
            );
            super::draw_triangle_aa(&mut expected, a, b, c, Color::rgba(210, 80, 40, 137));
        }
        assert_eq!(actual, expected);
        assert!(scratch.aa_outline_segments.capacity() >= 4);
        assert!(scratch.aa_coverage.capacity() >= 83);
    }

    #[test]
    fn fill_triangle_aa_scratch_matches_compatibility_wrapper() {
        let mut scratch = super::RasterizerScratch::default();
        let mut actual = Framebuffer::new(79, 63, Color::TRANSPARENT).unwrap();
        let mut expected = actual.clone();
        for (a, b, c, color) in [
            (
                Point::new(7, 8),
                Point::new(68, 14),
                Point::new(31, 55),
                Color::rgba(230, 80, 40, 177),
            ),
            (
                Point::new(-9, 26),
                Point::new(43, -5),
                Point::new(77, 61),
                Color::rgba(40, 170, 220, 129),
            ),
            (
                Point::new(12, 49),
                Point::new(54, 49),
                Point::new(33, 18),
                Color::rgba(120, 210, 70, 203),
            ),
        ] {
            super::fill_triangle_aa_with_scratch(&mut actual, a, b, c, color, &mut scratch);
            super::fill_triangle_aa(&mut expected, a, b, c, color);
        }
        assert_eq!(actual, expected);
        assert!(scratch.aa_edges.capacity() >= 3);
        assert!(scratch.aa_edge_order.capacity() >= 3);
        assert!(scratch.aa_coverage.capacity() >= 79);
    }

    #[test]
    fn parallel_antialiased_arc_outline_matches_serial_for_worker_counts() {
        for background in [
            Color::TRANSPARENT,
            Color::BLACK,
            Color::rgba(30, 70, 120, 113),
        ] {
            for (center, radius, start, end, color) in [
                (
                    Point::new(31, 24),
                    17,
                    0.2,
                    2.7,
                    Color::rgba(230, 70, 120, 177),
                ),
                (
                    Point::new(42, 25),
                    19,
                    5.4,
                    1.1,
                    Color::rgba(40, 190, 210, 129),
                ),
                (Point::new(3, 7), 8, 4.8, 7.2, Color::rgba(170, 90, 30, 211)),
                (Point::new(31, 24), 17, -1.0, 8.0, Color::WHITE),
            ] {
                let initial = Framebuffer::new(67, 49, background).unwrap();
                let mut expected = initial.clone();
                super::draw_arc_aa(&mut expected, center, radius, start, end, color);
                for workers in [0, 1, 2, 4, 8, 64] {
                    let mut actual = initial.clone();
                    super::draw_arc_aa_parallel_with_threads(
                        &mut actual,
                        center,
                        radius,
                        start,
                        end,
                        color,
                        workers,
                    );
                    assert_eq!(actual, expected, "worker_count={workers}");
                    let mut fallible = initial.clone();
                    super::try_draw_arc_aa_parallel_with_threads(
                        &mut fallible,
                        center,
                        radius,
                        start,
                        end,
                        color,
                        workers,
                    )
                    .unwrap();
                    assert_eq!(
                        fallible, expected,
                        "fallible outline, worker_count={workers}"
                    );
                }
                let mut automatic = initial;
                super::draw_arc_aa_parallel(&mut automatic, center, radius, start, end, color);
                assert_eq!(automatic, expected);
            }
        }
    }

    #[test]
    fn parallel_antialiased_arc_sector_matches_serial_for_worker_counts() {
        for background in [
            Color::TRANSPARENT,
            Color::BLACK,
            Color::rgba(30, 70, 120, 113),
        ] {
            for (center, radius, start, end, color) in [
                (
                    Point::new(31, 24),
                    17,
                    0.2,
                    2.7,
                    Color::rgba(230, 70, 120, 177),
                ),
                (
                    Point::new(42, 25),
                    19,
                    5.4,
                    1.1,
                    Color::rgba(40, 190, 210, 129),
                ),
                (Point::new(3, 7), 8, 4.8, 7.2, Color::rgba(170, 90, 30, 211)),
                (Point::new(31, 24), 17, -1.0, 8.0, Color::WHITE),
                (Point::new(31, 24), 0, 0.0, 1.5, Color::WHITE),
            ] {
                let initial = Framebuffer::new(67, 49, background).unwrap();
                let mut expected = initial.clone();
                super::fill_arc_aa(&mut expected, center, radius, start, end, color);
                for workers in [0, 1, 2, 4, 8, 64] {
                    let mut actual = initial.clone();
                    super::fill_arc_aa_parallel_with_threads(
                        &mut actual,
                        center,
                        radius,
                        start,
                        end,
                        color,
                        workers,
                    );
                    assert_eq!(actual, expected, "worker_count={workers}");
                    let mut fallible = initial.clone();
                    super::try_fill_arc_aa_parallel_with_threads(
                        &mut fallible,
                        center,
                        radius,
                        start,
                        end,
                        color,
                        workers,
                    )
                    .unwrap();
                    assert_eq!(
                        fallible, expected,
                        "fallible sector, worker_count={workers}"
                    );
                }
                let mut automatic = initial;
                super::fill_arc_aa_parallel(&mut automatic, center, radius, start, end, color);
                assert_eq!(automatic, expected);
            }
        }
    }

    #[test]
    fn parallel_antialiased_circle_apis_match_serial_ellipse_apis() {
        for background in [
            Color::TRANSPARENT,
            Color::BLACK,
            Color::rgba(30, 70, 120, 113),
        ] {
            for (center, radius, color) in [
                (Point::new(31, 24), 17, Color::rgba(230, 70, 120, 177)),
                (Point::new(3, 7), 8, Color::rgba(40, 190, 210, 129)),
                (Point::new(72, 51), 19, Color::WHITE),
                (Point::new(31, 24), 0, Color::rgba(170, 90, 30, 211)),
            ] {
                let initial = Framebuffer::new(67, 49, background).unwrap();
                let mut expected_fill = initial.clone();
                fill_ellipse_aa(&mut expected_fill, center, radius, radius, color);
                for workers in [0, 1, 2, 4, 8, 64] {
                    let mut actual = initial.clone();
                    fill_circle_aa_parallel_with_threads(
                        &mut actual,
                        center,
                        radius,
                        color,
                        workers,
                    );
                    assert_eq!(actual, expected_fill, "fill worker_count={workers}");
                }
                let mut automatic_fill = initial.clone();
                fill_circle_aa_parallel(&mut automatic_fill, center, radius, color);
                assert_eq!(automatic_fill, expected_fill);

                let mut expected_outline = initial.clone();
                draw_ellipse_aa(&mut expected_outline, center, radius, radius, color);
                for workers in [0, 1, 2, 4, 8, 64] {
                    let mut actual = initial.clone();
                    draw_circle_aa_parallel_with_threads(
                        &mut actual,
                        center,
                        radius,
                        color,
                        workers,
                    );
                    assert_eq!(actual, expected_outline, "outline worker_count={workers}");
                }
                let mut automatic_outline = initial;
                draw_circle_aa_parallel(&mut automatic_outline, center, radius, color);
                assert_eq!(automatic_outline, expected_outline);
            }
        }
    }
    #[test]
    fn diagonal_aa_fast_path_preserves_alpha_and_endpoint_order() {
        for background in [
            Color::TRANSPARENT,
            Color::BLACK,
            Color::rgba(30, 70, 120, 113),
        ] {
            for color in [
                Color::rgba(230, 70, 120, 177),
                Color::rgba(40, 190, 210, 0),
                Color::WHITE,
            ] {
                let mut forward = Framebuffer::new(24, 24, background).unwrap();
                let mut reverse = forward.clone();
                super::draw_line_aa(&mut forward, Point::new(3, 5), Point::new(18, 20), color);
                super::draw_line_aa(&mut reverse, Point::new(18, 20), Point::new(3, 5), color);
                assert_eq!(forward, reverse);
                for offset in 0..=15 {
                    let expected = if color.a == 0 {
                        background
                    } else if background.a == 0 || color.a == u8::MAX {
                        color
                    } else {
                        color.blend_over(background)
                    };
                    assert_eq!(
                        forward.get_pixel(3 + offset, 5 + offset),
                        Some(expected),
                        "offset={offset}, alpha={}",
                        color.a
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod axis_aligned_clip_tests {
    use super::clip_line;
    use crate::Point;

    #[test]
    fn horizontal_clipping_is_exact_and_preserves_endpoint_order() {
        assert_eq!(
            clip_line(5, 4, Point::new(-10, 2), Point::new(10, 2)),
            Some((0, 2, 4, 2))
        );
        assert_eq!(
            clip_line(5, 4, Point::new(10, 2), Point::new(-10, 2)),
            Some((4, 2, 0, 2))
        );
    }

    #[test]
    fn vertical_clipping_is_exact_and_preserves_endpoint_order() {
        assert_eq!(
            clip_line(5, 4, Point::new(3, -10), Point::new(3, 10)),
            Some((3, 0, 3, 3))
        );
        assert_eq!(
            clip_line(5, 4, Point::new(3, 10), Point::new(3, -10)),
            Some((3, 3, 3, 0))
        );
    }

    #[test]
    fn axis_aligned_clipping_rejects_segments_outside_the_framebuffer() {
        assert_eq!(clip_line(5, 4, Point::new(-10, 1), Point::new(-2, 1)), None);
        assert_eq!(clip_line(5, 4, Point::new(2, 8), Point::new(2, 12)), None);
        assert_eq!(clip_line(0, 4, Point::new(0, 0), Point::new(0, 2)), None);
    }
}

#[cfg(test)]
mod diagonal_line_fast_path_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn diagonal_fast_path_matches_expected_pixels_for_both_slopes_and_orders() {
        for (start, end) in [
            (Point::new(2, 3), Point::new(12, 13)),
            (Point::new(2, 13), Point::new(12, 3)),
            (Point::new(12, 13), Point::new(2, 3)),
            (Point::new(12, 3), Point::new(2, 13)),
        ] {
            let mut framebuffer = Framebuffer::new(16, 16, Color::BLACK).unwrap();
            super::draw_line(&mut framebuffer, start, end, Color::WHITE);

            let (left, right) = if start.x <= end.x {
                (start, end)
            } else {
                (end, start)
            };
            let step_y = if left.y <= right.y { 1 } else { -1 };
            for offset in 0..=10 {
                let x = left.x + offset;
                let y = left.y + step_y * offset;
                assert_eq!(
                    framebuffer.get_pixel(x as u32, y as u32),
                    Some(Color::WHITE)
                );
            }
            assert_eq!(
                framebuffer
                    .pixels()
                    .iter()
                    .filter(|pixel| **pixel == Color::WHITE)
                    .count(),
                11
            );
        }
    }

    #[test]
    fn diagonal_fast_path_handles_clipped_segments() {
        let mut framebuffer = Framebuffer::new(8, 8, Color::BLACK).unwrap();
        super::draw_line(
            &mut framebuffer,
            Point::new(-5, -5),
            Point::new(10, 10),
            Color::RED,
        );

        for coordinate in 0..8 {
            assert_eq!(
                framebuffer.get_pixel(coordinate, coordinate),
                Some(Color::RED)
            );
        }
        assert_eq!(
            framebuffer
                .pixels()
                .iter()
                .filter(|pixel| **pixel == Color::RED)
                .count(),
            8
        );
    }
}

#[cfg(test)]
mod bresenham_direct_buffer_tests {
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn general_bresenham_lines_preserve_pixels_in_both_endpoint_orders() {
        for (start, end) in [
            (Point::new(1, 2), Point::new(13, 8)),
            (Point::new(13, 8), Point::new(1, 2)),
            (Point::new(2, 13), Point::new(8, 1)),
            (Point::new(8, 1), Point::new(2, 13)),
        ] {
            let mut forward = Framebuffer::new(16, 16, Color::BLACK).unwrap();
            let mut reverse = forward.clone();
            super::draw_line(&mut forward, start, end, Color::GREEN);
            super::draw_line(&mut reverse, end, start, Color::GREEN);
            assert_eq!(forward, reverse);
            assert!(forward.pixels().contains(&Color::GREEN));
        }
    }

    #[test]
    fn general_bresenham_line_is_safe_when_clipped_at_frame_edges() {
        let mut framebuffer = Framebuffer::new(9, 7, Color::BLACK).unwrap();
        super::draw_line(
            &mut framebuffer,
            Point::new(-100, 2),
            Point::new(100, 5),
            Color::BLUE,
        );

        assert!(framebuffer.pixels().contains(&Color::BLUE));
        assert_eq!(framebuffer.width(), 9);
        assert_eq!(framebuffer.height(), 7);
    }
}

#[cfg(test)]
mod oblique_line_clipping_symmetry_tests {
    use super::clip_line;
    use crate::{Color, Framebuffer, Point};

    #[test]
    fn oblique_clipping_reverses_endpoints_symmetrically() {
        let cases = [
            (Point::new(-17, 3), Point::new(29, 19)),
            (Point::new(29, 19), Point::new(-17, 3)),
            (Point::new(4, -23), Point::new(18, 31)),
            (Point::new(18, 31), Point::new(4, -23)),
            (Point::new(-19, 27), Point::new(24, -11)),
            (Point::new(24, -11), Point::new(-19, 27)),
            (Point::new(-8, -13), Point::new(31, 28)),
            (Point::new(31, 28), Point::new(-8, -13)),
        ];

        for (start, end) in cases {
            let forward = clip_line(13, 11, start, end);
            let reverse = clip_line(13, 11, end, start);
            assert_eq!(
                forward.map(|(x0, y0, x1, y1)| (x1, y1, x0, y0)),
                reverse,
                "start={start:?}, end={end:?}"
            );
        }
    }

    #[test]
    fn clipped_oblique_rasterization_is_independent_of_endpoint_order() {
        let cases = [
            (Point::new(-17, 3), Point::new(29, 19)),
            (Point::new(4, -23), Point::new(18, 31)),
            (Point::new(-19, 27), Point::new(24, -11)),
            (Point::new(-8, -13), Point::new(31, 28)),
        ];

        for (start, end) in cases {
            let mut forward = Framebuffer::new(13, 11, Color::BLACK).unwrap();
            let mut reverse = forward.clone();
            super::draw_line(&mut forward, start, end, Color::WHITE);
            super::draw_line(&mut reverse, end, start, Color::WHITE);
            assert_eq!(forward, reverse, "start={start:?}, end={end:?}");
            assert_eq!(forward.width(), 13);
            assert_eq!(forward.height(), 11);
        }
    }
}

/// Fills a clipped rectangle using per-operation CPU rendering options.
///
/// The options control the worker limit and the minimum area at which
/// parallel execution is worthwhile. The result is pixel-identical to
/// `fill_rect`.
pub fn fill_rect_with_options(
    target: &mut Framebuffer,
    rect: Rect,
    color: Color,
    options: crate::RenderOptions,
) {
    let area = clipped_rect_bounds(target, rect)
        .map(|(left, top, right, bottom)| (right - left) as u64 * (bottom - top) as u64)
        .unwrap_or(0);
    let threads = options.worker_count(area);
    if threads <= 1 {
        fill_rect(target, rect, color);
    } else {
        fill_rect_parallel_with_threads(target, rect, color, threads);
    }
}

/// Alpha-composites a clipped rectangle using per-operation CPU rendering options.
///
/// Fully transparent colors remain a no-op. The configured worker limit and
/// area threshold are applied after clipping to the framebuffer.
pub fn blend_rect_with_options(
    target: &mut Framebuffer,
    rect: Rect,
    color: Color,
    options: crate::RenderOptions,
) {
    let area = clipped_rect_bounds(target, rect)
        .map(|(left, top, right, bottom)| (right - left) as u64 * (bottom - top) as u64)
        .unwrap_or(0);
    let threads = options.worker_count(area);
    blend_rect_with_threads(target, rect, color, threads);
}

#[cfg(test)]
mod render_options_tests {
    use super::{blend_rect, blend_rect_with_options, fill_rect, fill_rect_with_options};
    use crate::{Color, Framebuffer, Rect, RenderOptions};

    #[test]
    fn configured_fill_matches_serial_output() {
        let mut expected = Framebuffer::new(256, 128, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        let rect = Rect::new(4, 8, 240, 112);
        fill_rect(&mut expected, rect, Color::rgba(20, 40, 60, 255));
        fill_rect_with_options(
            &mut actual,
            rect,
            Color::rgba(20, 40, 60, 255),
            RenderOptions::new()
                .with_max_threads(2)
                .with_parallel_threshold(0),
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn configured_blend_matches_serial_output() {
        let mut expected = Framebuffer::new(256, 128, Color::BLUE).unwrap();
        let mut actual = expected.clone();
        let rect = Rect::new(4, 8, 240, 112);
        blend_rect(&mut expected, rect, Color::rgba(200, 20, 40, 127));
        blend_rect_with_options(
            &mut actual,
            rect,
            Color::rgba(200, 20, 40, 127),
            RenderOptions::new()
                .with_max_threads(2)
                .with_parallel_threshold(0),
        );
        assert_eq!(actual, expected);
    }

    #[test]
    fn configured_fill_respects_serial_mode() {
        let mut expected = Framebuffer::new(32, 32, Color::BLACK).unwrap();
        let mut actual = expected.clone();
        let rect = Rect::new(0, 0, 32, 32);
        fill_rect(&mut expected, rect, Color::WHITE);
        fill_rect_with_options(&mut actual, rect, Color::WHITE, RenderOptions::serial());
        assert_eq!(actual, expected);
    }
}
