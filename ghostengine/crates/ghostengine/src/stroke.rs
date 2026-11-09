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

/// Shape used to connect adjacent stroked segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokeJoin {
    /// Circular join matching the union of round segment ends.
    Round,
    /// Connects the outer edges with a straight bevel.
    Bevel,
    /// Extends the outer edges to their intersection, subject to the miter limit.
    Miter,
}

/// Stroke parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    pub miter_limit: f32,
}

impl StrokeStyle {
    #[must_use]
    pub const fn new(width: f32) -> Self {
        Self {
            width,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Round,
            miter_limit: 4.0,
        }
    }

    #[must_use]
    pub const fn with_cap(mut self, cap: StrokeCap) -> Self {
        self.cap = cap;
        self
    }

    #[must_use]
    pub const fn with_join(mut self, join: StrokeJoin) -> Self {
        self.join = join;
        self
    }

    /// Sets the maximum miter length divided by the stroke radius.
    #[must_use]
    pub const fn with_miter_limit(mut self, limit: f32) -> Self {
        self.miter_limit = limit;
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
/// tolerance. Open contours honor the selected cap; joins honor the selected style.
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
    if style.join == StrokeJoin::Miter
        && (!style.miter_limit.is_finite() || style.miter_limit < 1.0)
    {
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
        // Miter joins can extend well beyond the half-width of the stroke.
        // Expand the candidate bounds by the configured miter limit so long,
        // valid miters are not accidentally clipped before rasterization.
        let expand = if style.join == StrokeJoin::Miter {
            (radius * style.miter_limit).max(radius) + 1.0
        } else {
            radius + 1.0
        };
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
        // Prepare segment vectors and cap projections once per contour instead
        // of rebuilding them for every pixel and every antialiasing sample.
        let segments = (0..segment_count)
            .map(|i| {
                let first = !contour.closed && i == 0;
                let last = !contour.closed && i + 1 == segment_count;
                let cap = if first || last {
                    style.cap
                } else {
                    StrokeCap::Butt
                };
                StrokeSegment::new(
                    contour.points[i],
                    contour.points[(i + 1) % contour.points.len()],
                    first,
                    last,
                    cap,
                    radius,
                )
            })
            .collect::<Vec<_>>();
        let joins = (0..join_count(&contour.points, contour.closed))
            .filter_map(|index| {
                let (previous, vertex, next) = join_points(&contour.points, contour.closed, index);
                PreparedJoin::new(
                    previous,
                    vertex,
                    next,
                    style.join,
                    radius,
                    style.miter_limit,
                )
            })
            .collect::<Vec<_>>();
        // Sort geometry once so each scanline only visits segments and joins
        // whose vertical bounds are currently active, rather than rescanning
        // the complete contour for every row.
        let mut segments = segments;
        segments.sort_by(|left, right| left.min_y.total_cmp(&right.min_y));
        let mut joins = joins;
        joins.sort_by(|left, right| left.min_y.total_cmp(&right.min_y));
        let radius_squared = radius * radius;
        let mut row_segments: Vec<StrokeSegment> = Vec::with_capacity(segments.len());
        let mut row_joins: Vec<PreparedJoin> = Vec::with_capacity(joins.len());
        let mut next_segment = 0;
        let mut next_join = 0;
        for y in top..bottom {
            // A segment whose vertical bounds do not overlap this pixel row
            // cannot contribute to any of its 4x4 coverage samples.
            let sample_top = y as f32 + 0.125;
            let sample_bottom = y as f32 + 0.875;
            while next_segment < segments.len() && segments[next_segment].min_y <= sample_bottom {
                row_segments.push(segments[next_segment]);
                next_segment += 1;
            }
            row_segments.retain(|segment| segment.max_y >= sample_top);
            while next_join < joins.len() && joins[next_join].min_y <= sample_bottom {
                row_joins.push(joins[next_join]);
                next_join += 1;
            }
            row_joins.retain(|join| join.max_y >= sample_top);
            if row_segments.is_empty() && row_joins.is_empty() {
                continue;
            }
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
                            let mut distance_squared = f32::INFINITY;
                            for segment in &row_segments {
                                if segment.may_cover(sample) {
                                    distance_squared =
                                        distance_squared.min(segment.distance_squared(sample));
                                }
                            }
                            // A segment already covers this sample, so join geometry
                            // cannot increase its coverage.
                            if distance_squared > radius_squared {
                                for join in &row_joins {
                                    if join.contains(sample) {
                                        distance_squared = 0.0;
                                        break;
                                    }
                                }
                            }
                            if distance_squared <= radius_squared {
                                covered += 1;
                            }
                        }
                    }
                    f32::from(covered) / 16.0
                } else {
                    let sample = Vector::new(x as f32 + 0.5, y as f32 + 0.5);
                    let mut distance_squared = f32::INFINITY;
                    for segment in &row_segments {
                        if segment.may_cover(sample) {
                            distance_squared =
                                distance_squared.min(segment.distance_squared(sample));
                        }
                    }
                    if distance_squared > radius_squared {
                        for join in &row_joins {
                            if join.contains(sample) {
                                distance_squared = 0.0;
                                break;
                            }
                        }
                    }
                    // Preserve the legacy pixel-center coverage while delaying the
                    // square root until after the nearest segment has been selected.
                    let distance = distance_squared.sqrt();
                    (radius + 0.5 - distance).clamp(0.0, 1.0)
                };
                let alpha = (f32::from(color.a) * coverage).round() as u8;
                if alpha == 0 {
                    continue;
                }
                let src = Color::rgba(color.r, color.g, color.b, alpha);
                let index = x as usize;
                // Opaque coverage needs no source-over arithmetic.
                row[index] = if alpha == u8::MAX {
                    src
                } else {
                    src.blend_over(row[index])
                };
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct StrokeSegment {
    a: Vector,
    dx: f32,
    dy: f32,
    length_squared: f32,
    inverse_length_squared: f32,
    low: f32,
    high: f32,
    cap: StrokeCap,
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

impl StrokeSegment {
    fn new(a: Vector, b: Vector, first: bool, last: bool, cap: StrokeCap, radius: f32) -> Self {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length_squared = dx * dx + dy * dy;
        let extension = if cap == StrokeCap::Square && length_squared > f32::EPSILON {
            radius / length_squared.sqrt()
        } else {
            0.0
        };
        // A conservative axis-aligned bound avoids distance calculations for
        // samples that cannot intersect this segment's stroked area.
        let padding = if cap == StrokeCap::Square {
            radius * 2.0
        } else {
            radius
        };
        Self {
            a,
            dx,
            dy,
            length_squared,
            inverse_length_squared: if length_squared > f32::EPSILON {
                length_squared.recip()
            } else {
                0.0
            },
            low: if first { -extension } else { 0.0 },
            high: if last { 1.0 + extension } else { 1.0 },
            cap,
            min_x: a.x.min(b.x) - padding,
            min_y: a.y.min(b.y) - padding,
            max_x: a.x.max(b.x) + padding,
            max_y: a.y.max(b.y) + padding,
        }
    }

    fn may_cover(self, point: Vector) -> bool {
        point.x >= self.min_x
            && point.x <= self.max_x
            && point.y >= self.min_y
            && point.y <= self.max_y
    }

    fn distance_squared(self, point: Vector) -> f32 {
        if self.length_squared <= f32::EPSILON {
            let dx = point.x - self.a.x;
            let dy = point.y - self.a.y;
            return dx * dx + dy * dy;
        }
        let mut t = ((point.x - self.a.x) * self.dx + (point.y - self.a.y) * self.dy)
            * self.inverse_length_squared;
        if self.cap == StrokeCap::Butt && !(0.0..=1.0).contains(&t) {
            return f32::INFINITY;
        }
        if self.cap == StrokeCap::Round {
            t = t.clamp(0.0, 1.0);
        } else {
            t = t.clamp(self.low, self.high);
        }
        let nearest_x = self.a.x + t * self.dx;
        let nearest_y = self.a.y + t * self.dy;
        let dx = point.x - nearest_x;
        let dy = point.y - nearest_y;
        dx * dx + dy * dy
    }
}

fn join_count(points: &[Vector], closed: bool) -> usize {
    if closed {
        points.len()
    } else {
        points.len().saturating_sub(2)
    }
}

fn join_points(points: &[Vector], closed: bool, index: usize) -> (Vector, Vector, Vector) {
    if closed {
        let vertex_index = index % points.len();
        (
            points[(vertex_index + points.len() - 1) % points.len()],
            points[vertex_index],
            points[(vertex_index + 1) % points.len()],
        )
    } else {
        (points[index], points[index + 1], points[index + 2])
    }
}

#[derive(Debug, Clone, Copy)]
struct PreparedJoin {
    vertex: Vector,
    radius: f32,
    round: bool,
    bevel: [Vector; 3],
    miter: Option<[Vector; 3]>,
    min_y: f32,
    max_y: f32,
}

impl PreparedJoin {
    fn new(
        previous: Vector,
        vertex: Vector,
        next: Vector,
        join: StrokeJoin,
        radius: f32,
        miter_limit: f32,
    ) -> Option<Self> {
        if join == StrokeJoin::Round {
            return Some(Self {
                vertex,
                radius,
                round: true,
                bevel: [vertex; 3],
                miter: None,
                min_y: vertex.y - radius,
                max_y: vertex.y + radius,
            });
        }
        let incoming = Vector::new(vertex.x - previous.x, vertex.y - previous.y);
        let outgoing = Vector::new(next.x - vertex.x, next.y - vertex.y);
        let in_length = incoming.x.hypot(incoming.y);
        let out_length = outgoing.x.hypot(outgoing.y);
        if in_length <= f32::EPSILON || out_length <= f32::EPSILON {
            return None;
        }
        let incoming = Vector::new(incoming.x / in_length, incoming.y / in_length);
        let outgoing = Vector::new(outgoing.x / out_length, outgoing.y / out_length);
        let cross = incoming.x * outgoing.y - incoming.y * outgoing.x;
        if cross.abs() <= 1.0e-5 {
            return None;
        }
        let side = if cross > 0.0 { -1.0 } else { 1.0 };
        let normal_in = Vector::new(-incoming.y * side, incoming.x * side);
        let normal_out = Vector::new(-outgoing.y * side, outgoing.x * side);
        let outer_in = Vector::new(
            vertex.x + normal_in.x * radius,
            vertex.y + normal_in.y * radius,
        );
        let outer_out = Vector::new(
            vertex.x + normal_out.x * radius,
            vertex.y + normal_out.y * radius,
        );
        let bevel = [vertex, outer_in, outer_out];
        let miter = if join == StrokeJoin::Miter {
            let bisector = Vector::new(normal_in.x + normal_out.x, normal_in.y + normal_out.y);
            let bisector_length = bisector.x.hypot(bisector.y);
            if bisector_length <= f32::EPSILON {
                None
            } else {
                let bisector =
                    Vector::new(bisector.x / bisector_length, bisector.y / bisector_length);
                let denominator = bisector.x * normal_out.x + bisector.y * normal_out.y;
                if denominator <= 1.0e-5 {
                    None
                } else {
                    let miter_length = radius / denominator;
                    if !miter_length.is_finite() || miter_length > radius * miter_limit {
                        None
                    } else {
                        let miter = Vector::new(
                            vertex.x + bisector.x * miter_length,
                            vertex.y + bisector.y * miter_length,
                        );
                        Some([outer_in, miter, outer_out])
                    }
                }
            }
        } else {
            None
        };
        let mut min_y = bevel
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let mut max_y = bevel
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);
        if let Some(triangle) = miter {
            for point in triangle {
                min_y = min_y.min(point.y);
                max_y = max_y.max(point.y);
            }
        }
        Some(Self {
            vertex,
            radius,
            round: false,
            bevel,
            miter,
            min_y,
            max_y,
        })
    }

    fn contains(self, sample: Vector) -> bool {
        if self.round {
            let dx = sample.x - self.vertex.x;
            let dy = sample.y - self.vertex.y;
            return dx * dx + dy * dy <= self.radius * self.radius;
        }
        point_in_triangle(sample, self.bevel[0], self.bevel[1], self.bevel[2])
            || self.miter.is_some_and(|triangle| {
                point_in_triangle(sample, triangle[0], triangle[1], triangle[2])
            })
    }
}

fn point_in_triangle(point: Vector, a: Vector, b: Vector, c: Vector) -> bool {
    let cross = |p1: Vector, p2: Vector, p3: Vector| {
        (p2.x - p1.x) * (p3.y - p1.y) - (p2.y - p1.y) * (p3.x - p1.x)
    };
    let d1 = cross(a, b, point);
    let d2 = cross(b, c, point);
    let d3 = cross(c, a, point);
    let has_negative = d1 < -1.0e-5 || d2 < -1.0e-5 || d3 < -1.0e-5;
    let has_positive = d1 > 1.0e-5 || d2 > 1.0e-5 || d3 > 1.0e-5;
    !(has_negative && has_positive)
}

#[cfg(test)]
mod tests {
    use super::{
        stroke_path, stroke_path_clipped, StrokeCap, StrokeError, StrokeJoin, StrokeStyle,
    };
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
        let mut painted_inside = false;
        for y in 0..12 {
            for x in 0..14 {
                let inside = (5..7).contains(&x) && (4..7).contains(&y);
                let painted = fb.get_pixel(x, y) != Some(Color::BLACK);
                assert!(!painted || inside, "stroke escaped clip at ({x}, {y})");
                painted_inside |= painted;
            }
        }
        assert!(
            painted_inside,
            "stroke should paint at least part of the clip"
        );
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
    fn antialiased_caps_keep_butt_flat_and_extend_round_and_square() {
        let mut path = Path::new();
        path.move_to(Vector::new(4.25, 5.25))
            .line_to(Vector::new(10.25, 5.25));
        let mut butt = Framebuffer::new(16, 12, Color::BLACK).unwrap();
        let mut round = Framebuffer::new(16, 12, Color::BLACK).unwrap();
        let mut square = Framebuffer::new(16, 12, Color::BLACK).unwrap();

        for (target, cap) in [
            (&mut butt, StrokeCap::Butt),
            (&mut round, StrokeCap::Round),
            (&mut square, StrokeCap::Square),
        ] {
            super::stroke_path_aa(
                target,
                &path,
                Transform2D::IDENTITY,
                StrokeStyle::new(4.0).with_cap(cap),
                Color::WHITE,
                0.2,
            )
            .unwrap();
        }

        assert_eq!(butt.get_pixel(3, 5), Some(Color::BLACK));
        assert!(round.get_pixel(3, 5).unwrap().r > 0);
        assert!(square.get_pixel(3, 5).unwrap().r > 0);
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

    fn corner_path() -> Path {
        let mut path = Path::new();
        path.move_to(Vector::new(3.0, 10.0))
            .line_to(Vector::new(10.0, 10.0))
            .line_to(Vector::new(10.0, 3.0));
        path
    }

    #[test]
    fn round_join_fills_outer_corner_with_circular_coverage() {
        let mut fb = Framebuffer::new(16, 16, Color::BLACK).unwrap();
        stroke_path(
            &mut fb,
            &corner_path(),
            Transform2D::IDENTITY,
            StrokeStyle::new(6.0).with_join(StrokeJoin::Round),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        assert_eq!(fb.get_pixel(12, 11), Some(Color::WHITE));
    }

    #[test]
    fn bevel_join_fills_outer_corner_without_round_bulge() {
        let mut fb = Framebuffer::new(16, 16, Color::BLACK).unwrap();
        stroke_path(
            &mut fb,
            &corner_path(),
            Transform2D::IDENTITY,
            StrokeStyle::new(6.0).with_join(StrokeJoin::Bevel),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        assert!(fb.get_pixel(12, 12).unwrap() == Color::BLACK);
        assert!(fb.get_pixel(12, 11).unwrap() == Color::BLACK);
        assert!(fb.get_pixel(10, 12).unwrap() == Color::WHITE);
    }

    #[test]
    fn miter_join_extends_outer_corner_when_within_limit() {
        let mut bevel = Framebuffer::new(16, 16, Color::BLACK).unwrap();
        let mut miter = Framebuffer::new(16, 16, Color::BLACK).unwrap();
        stroke_path(
            &mut bevel,
            &corner_path(),
            Transform2D::IDENTITY,
            StrokeStyle::new(6.0).with_join(StrokeJoin::Bevel),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        stroke_path(
            &mut miter,
            &corner_path(),
            Transform2D::IDENTITY,
            StrokeStyle::new(6.0).with_join(StrokeJoin::Miter),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        assert!(miter.get_pixel(12, 12).unwrap() != bevel.get_pixel(12, 12).unwrap());
    }

    #[test]
    fn miter_join_falls_back_to_bevel_when_limit_is_exceeded() {
        let mut bevel = Framebuffer::new(16, 16, Color::BLACK).unwrap();
        let mut limited = Framebuffer::new(16, 16, Color::BLACK).unwrap();
        stroke_path(
            &mut bevel,
            &corner_path(),
            Transform2D::IDENTITY,
            StrokeStyle::new(6.0).with_join(StrokeJoin::Bevel),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        stroke_path(
            &mut limited,
            &corner_path(),
            Transform2D::IDENTITY,
            StrokeStyle::new(6.0)
                .with_join(StrokeJoin::Miter)
                .with_miter_limit(1.0),
            Color::WHITE,
            0.2,
        )
        .unwrap();
        assert_eq!(limited.pixels(), bevel.pixels());
    }

    #[test]
    fn antialiased_miter_join_honors_style_and_limit() {
        // Fractional geometry ensures the miter edge crosses the supersample grid.
        let mut path = Path::new();
        path.move_to(Vector::new(3.25, 10.25))
            .line_to(Vector::new(10.25, 10.25))
            .line_to(Vector::new(10.25, 3.25));
        let mut bevel = Framebuffer::new(20, 20, Color::BLACK).unwrap();
        let mut miter = Framebuffer::new(20, 20, Color::BLACK).unwrap();
        let mut limited = Framebuffer::new(20, 20, Color::BLACK).unwrap();

        for (target, style) in [
            (
                &mut bevel,
                StrokeStyle::new(6.0).with_join(StrokeJoin::Bevel),
            ),
            (
                &mut miter,
                StrokeStyle::new(6.0).with_join(StrokeJoin::Miter),
            ),
            (
                &mut limited,
                StrokeStyle::new(6.0)
                    .with_join(StrokeJoin::Miter)
                    .with_miter_limit(1.0),
            ),
        ] {
            super::stroke_path_aa(
                target,
                &path,
                Transform2D::IDENTITY,
                style,
                Color::WHITE,
                0.2,
            )
            .unwrap();
        }

        assert_ne!(miter.pixels(), bevel.pixels());
        assert_eq!(limited.pixels(), bevel.pixels());
        assert!(miter
            .pixels()
            .iter()
            .any(|pixel| pixel.r > 0 && pixel.r < 255));
    }

    #[test]
    fn scanline_culling_preserves_segment_and_join_coverage() {
        let points = [
            Vector::new(2.0, 14.0),
            Vector::new(9.0, 5.0),
            Vector::new(14.0, 14.0),
            Vector::new(17.0, 3.0),
        ];
        let radius = 3.0;
        let radius_squared = radius * radius;
        let segments = [
            super::StrokeSegment::new(points[0], points[1], true, false, StrokeCap::Butt, radius),
            super::StrokeSegment::new(points[1], points[2], false, false, StrokeCap::Butt, radius),
            super::StrokeSegment::new(points[2], points[3], false, true, StrokeCap::Square, radius),
        ];
        let joins = [
            super::PreparedJoin::new(
                points[0],
                points[1],
                points[2],
                StrokeJoin::Miter,
                radius,
                4.0,
            )
            .unwrap(),
            super::PreparedJoin::new(
                points[1],
                points[2],
                points[3],
                StrokeJoin::Miter,
                radius,
                4.0,
            )
            .unwrap(),
        ];

        for y in 0..20 {
            let sample_top = y as f32 + 0.125;
            let sample_bottom = y as f32 + 0.875;
            let row_segments = segments
                .iter()
                .filter(|segment| segment.max_y >= sample_top && segment.min_y <= sample_bottom)
                .collect::<Vec<_>>();
            let row_joins = joins
                .iter()
                .filter(|join| join.max_y >= sample_top && join.min_y <= sample_bottom)
                .collect::<Vec<_>>();

            for x in 0..20 {
                for offset_y in [0.125, 0.375, 0.625, 0.875, 0.5] {
                    let sample = Vector::new(x as f32 + 0.5, y as f32 + offset_y);
                    let expected = segments
                        .iter()
                        .any(|segment| segment.distance_squared(sample) <= radius_squared)
                        || joins.iter().any(|join| join.contains(sample));
                    let actual = row_segments.iter().any(|segment| {
                        segment.may_cover(sample)
                            && segment.distance_squared(sample) <= radius_squared
                    }) || row_joins.iter().any(|join| join.contains(sample));

                    assert_eq!(
                        actual, expected,
                        "scanline culling changed coverage at ({}, {})",
                        sample.x, sample.y
                    );
                }
            }
        }
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
